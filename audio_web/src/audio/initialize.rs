use super::{
    buffer_selection_action::BufferSelectionAction, decode, gain_action::GainAction,
    play_status::PlayStatus, play_status_action::PlayStatusAction,
};
use crate::{
    audio::stream_handle::StreamHandle,
    components::controls_select_buffer::DEFAULT_AUDIO_FILE,
    state::{app_action::AppAction, app_state::AppState}, utils::download,
};
use audio_common::{granular_synthesizer_action::GranularSynthesizerAction, mixdown::mixdown};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream,
};
use gloo_net::http::Request;
use hound::{WavWriter};
use std::{
    io::{Cursor, BufWriter},
    sync::{Arc},
};
use yew::UseReducerHandle;

/// Converts default mp3 file to raw audio sample data
async fn load_default_buffer(app_state_handle: UseReducerHandle<AppState>) -> Arc<Vec<f32>> {
    let audio_context =
        web_sys::AudioContext::new().expect("Browser should have AudioContext implemented");

    // audio files are copied into static director for web (same directory as source wasm file)
    // fetch a default audio file at initialization time
    let mp3_file_bytes = Request::get(&format!("./{}", DEFAULT_AUDIO_FILE))
        .send()
        .await
        .unwrap()
        .binary()
        .await
        .unwrap();

    let audio_buffer = decode::decode_bytes(&audio_context, &mp3_file_bytes).await;
    let mp3_source_data = Arc::new(audio_buffer.get_channel_data(0).unwrap());
    app_state_handle.dispatch(AppAction::SetBuffer(Arc::clone(&mp3_source_data)));

    mp3_source_data
}

/// This function is called periodically to write audio data into an audio output buffer
fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> Vec<f32>)
where
    T: cpal::Sample,
{
    for frame in output.chunks_mut(channels) {
        let output_samples = next_sample();

        for (i, sample) in frame.iter_mut().enumerate() {
            *sample = cpal::Sample::from::<f32>(&output_samples[i]);
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
    // this is the config of the output audio
    let output_sample_rate = stream_config.sample_rate.0;
    let output_num_channels = stream_config.channels as usize;

    // only load if buffer hasn't been loaded
    if app_state_handle.buffer_handle.get_data().is_empty() {
        load_default_buffer(app_state_handle.clone()).await;
    }

    let buffer_selection_handle = app_state_handle.buffer_selection_handle.clone();
    let gain_handle = app_state_handle.gain_handle.clone();
    let status = app_state_handle.play_status_handle.clone();
    let mut granular_synthesizer_handle = app_state_handle.granular_synthesizer_handle.clone();

    // make sure granular synthesizer's internal state is current with audio context state
    granular_synthesizer_handle.set_sample_rate(output_sample_rate);

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    let mut buf_writer = BufWriter::new( &mut cursor);
    let mut writer = WavWriter::new(&mut buf_writer, spec).unwrap();
    for t in (0 .. 44100).map(|x| x as f32 / 44100.0) {
        let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
        let amplitude = i16::MAX as f32;
        writer.write_sample((sample * amplitude) as i16).unwrap();
    }
    println!("duration = {:?}", writer.duration());
    println!("spec = {:?}", writer.spec());
    writer.finalize().unwrap();
    std::mem::drop(buf_writer);

    download::download_bytes(&cursor.get_ref(), "recording.wav");

    // Called for every audio frame to generate appropriate sample
    let mut next_value = move || {
        // if paused, do not process any audio, just return silence
        if let PlayStatus::Pause = status.get() {
            return vec![0.0; output_num_channels];
        }

        // always keep granular_synth up-to-date with buffer selection from UI
        let (selection_start, selection_end) = buffer_selection_handle.get_buffer_start_and_end();
        granular_synthesizer_handle
            .set_selection_start(selection_start)
            .set_selection_end(selection_end);

        // get next frame from granular synth
        let frame = granular_synthesizer_handle.next_frame();

        // mix multi-channel down to number of outputs
        let output_frame = mixdown(&frame, output_num_channels as u32);

        // gate final output with global gain
        let gain = gain_handle.get();
        let output_frame: Vec<f32> = output_frame
            .into_iter()
            .map(|output| output * gain)
            .collect();

        // for sample in output_frame {
        //     writer.write_sample(sample).unwrap();
        // }
        // current_write_count += 1;

        // if current_write_count == max_write_count {
            // writer.finalize().unwrap();
            // let mut blob_property_bag = BlobPropertyBag::new();
            // blob_property_bag.type_("'audio/wav; codecs=0'");
            // let u8_view = unsafe { js_sys::Uint8Array::view(&cursor.get_ref()[..]) };
            // let blob =
            //     Blob::new_with_blob_sequence_and_options(&u8_view.as_ref(), &blob_property_bag)
            //         .unwrap();
            // let url = Url::create_object_url_with_blob(&blob).unwrap();
            // let window = web_sys::window().unwrap();
            // let document = window.document().unwrap();
            // let body = document.body().unwrap();
            // let a: HtmlAnchorElement = document.create_element("a").unwrap().dyn_into().unwrap();
            // body.append_child(&a).unwrap();
            // a.set_href(&url);
            // a.set_download("recording.wav");
            // a.click();
            // Url::revoke_object_url(&url).unwrap();
        // }

        output_frame
    };

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, output_num_channels, &mut next_value)
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
    let sample_rate = config.sample_rate().0;
    app_state_handle.dispatch(AppAction::SetSampleRate(sample_rate));

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
