use common::{grain::Grain, grain_sample::GrainSample, utils};
use log::*;
use rand::Rng;
use wasm_bindgen::{prelude::*, JsCast};
// use minimp3::{Decoder};

// When the `wee_alloc` feature is enabled, this uses `wee_alloc` as the global
// allocator.
//
// If you don't want to use `wee_alloc`, you can safely delete this.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

// This is like the `main` function, except for JavaScript.
#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    // This provides better error messages in debug mode.
    // It's disabled in release mode, so it doesn't bloat up the file size.
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();

    // enables using info!() macros
    wasm_logger::init(wasm_logger::Config::default());

    Ok(())
}

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;

#[wasm_bindgen]
pub struct Handle(Stream);

#[wasm_bindgen]
pub async fn beep() -> Handle {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("failed to find a default output device");
    let config = device.default_output_config().unwrap();

    Handle(match config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into()).await.unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into()).await.unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into()).await.unwrap(),
    })
}

pub async fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
) -> Result<Stream, anyhow::Error>
where
    T: cpal::Sample,
{
    const NUM_CHANNELS: usize = 3;
    const ENVELOPE_LEN_MS_MIN: f32 = 1.0;
    const ENVELOPE_LEN_MS_MAX: f32 = 100.0;

    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    let envelope_len_samples_min = (sample_rate / (1000.0 / ENVELOPE_LEN_MS_MIN)) as usize;
    let envelope_len_samples_max = (sample_rate / (1000.0 / ENVELOPE_LEN_MS_MAX)) as usize;

    let audio_context =
        web_sys::AudioContext::new().expect("Browser should have AudioContext implemented");

    // get audio file data at compile time
    let mp3_file_bytes = include_bytes!("..\\..\\audio\\pater_emon.mp3");

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

    let mp3_source_data = audio_buffer.get_channel_data(0).unwrap();

    // associates each grain with the amplitude of it's envelope
    // instantiated here to prevent allocations during audio calculations
    let mut frame_samples_and_envelopes = Vec::with_capacity(NUM_CHANNELS);
    for _ in 0..NUM_CHANNELS {
        frame_samples_and_envelopes.push(GrainSample::default());
    }

    // keeps track of where each grain should be in the buffer
    let mut channels_grains: Vec<Grain> = Vec::with_capacity(NUM_CHANNELS);
    for _ in 0..NUM_CHANNELS {
        channels_grains.push(Grain::default());
    }

    // Called for every audio frame to generate appropriate sample
    let mut next_value = move || {
        let mut rng = rand::thread_rng();

        // grain length should not exceed max mp3 source data length
        debug_assert!(mp3_source_data.len() > envelope_len_samples_max);

        // create new grains for any that are finished
        for grain in channels_grains.iter_mut() {
            if grain.finished {
                let envolope_len_samples =
                    rng.gen_range(envelope_len_samples_min..envelope_len_samples_max);
                let max_index = mp3_source_data.len() - envolope_len_samples;
                let start_frame = rng.gen_range(0..max_index);
                let end_frame = start_frame + envolope_len_samples;

                debug_assert!(start_frame > 0);
                debug_assert!(end_frame > 0);
                debug_assert!(start_frame < mp3_source_data.len());
                debug_assert!(end_frame < mp3_source_data.len());

                let new_grain = Grain::new(start_frame, end_frame);
                *grain = new_grain;
            }
        }

        debug_assert_eq!(channels_grains.len(), NUM_CHANNELS);

        // get value of each grain's current index in the buffer for each channel
        channels_grains
            .iter_mut()
            .enumerate()
            .for_each(|(i, grain)| {
                debug_assert_eq!(grain.finished, false);

                let envelope_percent =
                    ((grain.current_frame - grain.start_frame) as f32) / (grain.len as f32);
                debug_assert!(envelope_percent >= 0.0, "{}", envelope_percent);
                debug_assert!(envelope_percent < 1.0, "{}", envelope_percent);

                let envelope_value =
                    utils::generate_triangle_envelope_value_from_percent(envelope_percent);
                let frame_index = grain.current_frame;
                let sample_value = mp3_source_data[frame_index];

                frame_samples_and_envelopes[i].sample_value = sample_value;
                frame_samples_and_envelopes[i].envelope_value = envelope_value;

                grain.get_next_frame();
            });

        // mix frame channels down to 2 channels (spacialize from left to right)
        let mut left = 0.0;
        let mut right = 0.0;
        for (i, grain_sample) in frame_samples_and_envelopes.iter().enumerate() {
            // earlier indexes to later indexes == left to right spacialization
            let left_spatialization_percent =
                1.0 - (i as f32) / (frame_samples_and_envelopes.len() as f32);
            let right_spatialization_percent =
                (i as f32) / (frame_samples_and_envelopes.len() as f32);

            // division by 0 will happen below if num of channels is less than 2
            debug_assert!(NUM_CHANNELS >= 2);

            // logarithmically scaling the volume seems to work well for very large numbers of voices
            let left_value_to_add = (grain_sample.sample_value
                * grain_sample.envelope_value
                * left_spatialization_percent)
                / (NUM_CHANNELS as f32).log(2.0);
            let right_value_to_add = (grain_sample.sample_value
                * grain_sample.envelope_value
                * right_spatialization_percent)
                / (NUM_CHANNELS as f32).log(2.0);

            left += left_value_to_add;
            right += right_value_to_add;
        }

        (left, right)
    };

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value)
        },
        err_fn,
    )?;

    stream.play()?;

    Ok(stream)
}

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
