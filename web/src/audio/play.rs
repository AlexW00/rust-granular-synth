use crate::{
    audio::stream_handle::StreamHandle,
    state::{app_action::AppAction, app_state::AppState},
};
use common::granular_synthesizer::GranularSynthesizer;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream,
};
use log::*;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use yew::UseReducerHandle;

const NUM_CHANNELS: usize = 5;
const GRAIN_LEN_MIN_IN_MS: usize = 1;
const GRAIN_LEN_MAX_IN_MS: usize = 100;

/// Converts default mp3 file to raw audio sample data
async fn load_default_buffer(app_state_handle: UseReducerHandle<AppState>) -> Arc<Vec<f32>> {
    let audio_context =
        web_sys::AudioContext::new().expect("Browser should have AudioContext implemented");

    info!(
        "Sample Rate of Audio Context = {}",
        audio_context.sample_rate()
    );

    // get audio file data at compile time
    let mp3_file_bytes = include_bytes!("..\\..\\..\\audio\\pater_emon.mp3");

    // this action is "unsafe" because it's creating a JavaScript view into wasm linear memory,
    // but there's no risk in this case, because `mp3_file_bytes` is an array that is statically compiled
    // into the wasm binary itself and will not be reallocated at runtime
    let mp3_u_int8_array = unsafe { js_sys::Uint8Array::view(mp3_file_bytes) };

    // this data must be copied, because decodeAudioData() claims the ArrayBuffer it receives
    let mp3_u_int8_array = mp3_u_int8_array.slice(0, mp3_u_int8_array.length());

    let decoded_audio_result = audio_context
        .decode_audio_data(&mp3_u_int8_array.buffer())
        .expect("Should succeed at decoding audio data");

    let audio_buffer: web_sys::AudioBuffer =
        wasm_bindgen_futures::JsFuture::from(decoded_audio_result)
            .await
            .expect("Should convert decode_audio_data Promise into Future")
            .dyn_into()
            .expect("decode_audio_data should return a buffer of data on success");

    info!(
        "Sample Rate of Default Audio File = {}",
        audio_buffer.sample_rate()
    );

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
    let mp3_source_data = if app_state_handle.buffer.data.is_empty() {
        let app_state_handle = app_state_handle.clone();
        load_default_buffer(app_state_handle).await
    } else {
        Arc::clone(&app_state_handle.buffer.data)
    };

    let mut granular_synth: GranularSynthesizer<NUM_CHANNELS> =
        GranularSynthesizer::new(mp3_source_data, sample_rate);

    // this data does not need to be current (for now)
    granular_synth
        .set_grain_len_min(GRAIN_LEN_MIN_IN_MS)
        .set_grain_len_max(GRAIN_LEN_MAX_IN_MS);

    let buffer_selection = Arc::clone(&app_state_handle.buffer_handle.buffer_selection);

    // Called for every audio frame to generate appropriate sample
    let mut next_value = move || {
        // always keep granular_synth up-to-date with buffer selection from UI
        let (selection_start, selection_end) =
            buffer_selection.lock().unwrap().get_buffer_start_and_end();
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

pub async fn play(app_state_handle: UseReducerHandle<AppState>) -> StreamHandle {
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
