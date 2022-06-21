use crate::{
    audio::stream_handle::StreamHandle,
    state::{app_action::AppAction, app_state::AppState},
};
use common::granular_synthesizer::GranularSynthesizer;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream,
};
use gloo_net::http::Request;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use yew::UseReducerHandle;

use super::{current_status::CurrentStatus, decode_bytes};

const NUM_CHANNELS: usize = 3;
const GRAIN_LEN_MIN_IN_MS: usize = 10;
const GRAIN_LEN_MAX_IN_MS: usize = 1000;

/// Converts default mp3 file to raw audio sample data
async fn load_default_buffer(app_state_handle: UseReducerHandle<AppState>) -> Arc<Vec<f32>> {
    let audio_context =
        web_sys::AudioContext::new().expect("Browser should have AudioContext implemented");

    // audio files are copied into static director for web (same directory as source wasm file)
    // fetch a default audio file at initialization time
    let mp3_file_bytes = Request::get("./ecce_nova_3.mp3")
        .send()
        .await
        .unwrap()
        .binary()
        .await
        .unwrap();

    let audio_buffer = decode_bytes::decode_bytes(&audio_context, &mp3_file_bytes).await;
    let mp3_source_data = Arc::new(audio_buffer.get_channel_data(0).unwrap());
    app_state_handle.dispatch(AppAction::SetBuffer(Arc::clone(&mp3_source_data)));

    mp3_source_data
}

/// This function is called periodically to write audio data into an audio output buffer
fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f32, f32))
where
    T: cpal::Sample,
{
    for frame in output.chunks_mut(channels) {
        let (left_sample, right_sample) = next_sample();
        let left_sample = cpal::Sample::from::<f32>(&left_sample);
        let right_sample = cpal::Sample::from::<f32>(&right_sample);

        // assume a 2-channel system and just map to evens and odds if there are more channels
        for (i, sample) in frame.iter_mut().enumerate() {
            if i % 2 == 0 {
                *sample = left_sample;
            } else {
                *sample = right_sample;
            }
        }
    }
}

fn init_granular_synth<const C: usize>(mp3_source_data: Arc<Vec<f32>>, sample_rate: u32) -> GranularSynthesizer<C> {
    let mut granular_synth: GranularSynthesizer<C> =
        GranularSynthesizer::new(mp3_source_data, sample_rate);

    // this data does not need to be current (for now)
    granular_synth
        .set_grain_len_min(GRAIN_LEN_MIN_IN_MS)
        .set_grain_len_max(GRAIN_LEN_MAX_IN_MS);

    granular_synth
}

/// Setup all audio data and processes and begin playing
pub async fn run<T>(
    app_state_handle: UseReducerHandle<AppState>,
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
) -> Result<Stream, anyhow::Error>
where
    T: cpal::Sample,
{
    let sample_rate = stream_config.sample_rate.0;
    let channels = stream_config.channels as usize;

    // only load new buffer if current one is empty
    let existing_buffer_data = app_state_handle.buffer.get_data();
    let mp3_source_data = if existing_buffer_data.is_empty() {
        let app_state_handle = app_state_handle.clone();
        load_default_buffer(app_state_handle).await
    } else {
        existing_buffer_data
    };

    let mut prev_buffer = Arc::clone(&mp3_source_data);
    let mut granular_synth = init_granular_synth::<NUM_CHANNELS>(mp3_source_data, sample_rate);

    let buffer_selection_handle = app_state_handle.buffer_selection_handle.clone();
    let gain_handle = app_state_handle.gain_handle.clone();
    let status = app_state_handle.current_status_handle.clone();
    let buffer_handle = app_state_handle.buffer.clone();

    // Called for every audio frame to generate appropriate sample
    let mut next_value = move || {
        // if paused, do not process any audio, just return silence
        if let CurrentStatus::PAUSE = status.get() {
            return (0.0, 0.0);
        }

        // check if buffer has changed:
        // if so, replace granular synth with one that has the udpated buffer
        let new_buffer = buffer_handle.get_data();
        if !new_buffer.is_empty() && new_buffer != prev_buffer {
            prev_buffer = Arc::clone(&new_buffer);
            granular_synth = init_granular_synth::<NUM_CHANNELS>(new_buffer, sample_rate);
        }

        // always keep granular_synth up-to-date with buffer selection from UI
        let (selection_start, selection_end) = buffer_selection_handle.get_buffer_start_and_end();
        granular_synth
            .set_selection_start(selection_start)
            .set_selection_end(selection_end);

        let frame = granular_synth.next_frame();

        // mix frame channels down to 2 channels (spacialize from left to right)
        let mut left = 0.0;
        let mut right = 0.0;
        for (i, channel_value) in frame.iter().enumerate() {
            // earlier indexes to later indexes == left to right spacialization
            let left_spatialization_percent = 1.0 - (i as f32) / (frame.len() as f32);
            let right_spatialization_percent = (i as f32) / (frame.len() as f32);

            // division by 0 will happen below if num of channels is less than 2
            debug_assert!(NUM_CHANNELS >= 2);

            // logarithmically scaling the volume seems to work well for very large numbers of voices
            let left_value_to_add =
                (channel_value * left_spatialization_percent) / (NUM_CHANNELS as f32).log(2.0);
            let right_value_to_add =
                (channel_value * right_spatialization_percent) / (NUM_CHANNELS as f32).log(2.0);

            left += left_value_to_add;
            right += right_value_to_add;
        }

        let gain = gain_handle.get();
        let (left, right) = (left * gain, right * gain);

        (left, right)
    };

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value)
        },
        err_fn,
    )?;

    stream.play()?;

    Ok(stream)
}

pub async fn initialize_audio(app_state_handle: UseReducerHandle<AppState>) -> StreamHandle {
    app_state_handle.dispatch(AppAction::SetAudioInitialized(false));
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("failed to find a default output device");
    let config = device.default_output_config().unwrap();
    let sample_format = config.sample_format();

    StreamHandle::new(match sample_format {
        cpal::SampleFormat::F32 => run::<f32>(app_state_handle, &device, &config.into())
            .await
            .unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(app_state_handle, &device, &config.into())
            .await
            .unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(app_state_handle, &device, &config.into())
            .await
            .unwrap(),
    })
}
