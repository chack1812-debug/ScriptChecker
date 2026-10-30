use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{audio::AudioData, transport::Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OutputCandidate {
    sample_format: cpal::SampleFormat,
    min_sample_rate: u32,
    max_sample_rate: u32,
}

fn select_output_format(
    candidates: &[OutputCandidate],
    sample_rate: u32,
) -> Option<cpal::SampleFormat> {
    const PREFERENCE: [cpal::SampleFormat; 5] = [
        cpal::SampleFormat::F32,
        cpal::SampleFormat::I16,
        cpal::SampleFormat::I32,
        cpal::SampleFormat::U16,
        cpal::SampleFormat::U8,
    ];
    PREFERENCE.into_iter().find(|format| {
        candidates.iter().any(|candidate| {
            candidate.sample_format == *format
                && candidate.min_sample_rate <= sample_rate
                && sample_rate <= candidate.max_sample_rate
        })
    })
}

fn convert_i32(sample: f32) -> i32 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i32::MIN
    } else if sample >= 1.0 {
        i32::MAX
    } else {
        (sample * i32::MAX as f32).round() as i32
    }
}

fn convert_u8(sample: f32) -> u8 {
    ((sample.clamp(-1.0, 1.0) + 1.0) * 0.5 * u8::MAX as f32).round() as u8
}

pub struct Output {
    _stream: cpal::Stream,
    error_message: Arc<Mutex<Option<String>>>,
}

impl Output {
    pub fn error_message(&self) -> Option<String> {
        self.error_message.lock().ok()?.clone()
    }
}

pub fn start_output(audio: AudioData, transport: Arc<Mutex<Transport>>) -> Result<Output, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "既定の出力デバイスがありません".to_owned())?;
    let supported = device
        .supported_output_configs()
        .map_err(|error| format!("出力設定を取得できません: {error}"))?
        .collect::<Vec<_>>();
    let candidates = supported
        .iter()
        .map(|config| OutputCandidate {
            sample_format: config.sample_format(),
            min_sample_rate: config.min_sample_rate().0,
            max_sample_rate: config.max_sample_rate().0,
        })
        .collect::<Vec<_>>();
    let selected_format = select_output_format(&candidates, audio.sample_rate)
        .ok_or_else(|| format!("出力デバイスは {}Hz に対応していません", audio.sample_rate))?;
    let supported = supported
        .into_iter()
        .find(|config| {
            config.sample_format() == selected_format
                && config.min_sample_rate().0 <= audio.sample_rate
                && audio.sample_rate <= config.max_sample_rate().0
        })
        .expect("selected output format must have a supported configuration")
        .with_sample_rate(cpal::SampleRate(audio.sample_rate));

    let config = supported.config();
    let device_channels = config.channels;
    let error_message = Arc::new(Mutex::new(None));
    let error_callback = {
        let error_message = Arc::clone(&error_message);
        move |stream_error: cpal::StreamError| {
            if let Ok(mut message) = error_message.lock() {
                *message = Some(stream_error.to_string());
            }
        }
    };
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build_stream(
            &device,
            &config,
            audio,
            transport,
            device_channels,
            error_callback,
            |sample| sample,
        ),
        cpal::SampleFormat::I16 => build_stream(
            &device,
            &config,
            audio,
            transport,
            device_channels,
            error_callback,
            |sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16,
        ),
        cpal::SampleFormat::U16 => build_stream(
            &device,
            &config,
            audio,
            transport,
            device_channels,
            error_callback,
            |sample| ((sample.clamp(-1.0, 1.0) + 1.0) * 0.5 * f32::from(u16::MAX)).round() as u16,
        ),
        cpal::SampleFormat::I32 => build_stream(
            &device,
            &config,
            audio,
            transport,
            device_channels,
            error_callback,
            convert_i32,
        ),
        cpal::SampleFormat::U8 => build_stream(
            &device,
            &config,
            audio,
            transport,
            device_channels,
            error_callback,
            convert_u8,
        ),
        format => return Err(format!("未対応の出力サンプル形式: {format:?}")),
    }
    .map_err(|error| format!("出力ストリームを作成できません: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("出力ストリームを開始できません: {error}"))?;
    Ok(Output {
        _stream: stream,
        error_message,
    })
}

fn build_stream<T: cpal::SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    audio: AudioData,
    transport: Arc<Mutex<Transport>>,
    device_channels: u16,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
    convert: impl Fn(f32) -> T + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let audio_channels = audio.channels;
    let samples = Arc::clone(&audio.samples);
    let mut rendered = Vec::new();
    let mut mixed = Vec::new();
    device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            let required_samples =
                out.len() / usize::from(device_channels) * usize::from(audio_channels);
            if rendered.len() < required_samples {
                rendered.resize(required_samples, 0.0);
            }
            let rendered = &mut rendered[..required_samples];
            rendered.fill(0.0);
            if let Ok(mut transport) = transport.try_lock() {
                transport.render(&samples, audio_channels, rendered);
            }
            if mixed.len() < out.len() {
                mixed.resize(out.len(), 0.0);
            }
            for (frame, destination) in out
                .chunks_exact_mut(usize::from(device_channels))
                .enumerate()
            {
                let source =
                    &rendered[frame * usize::from(audio_channels)..][..usize::from(audio_channels)];
                let mixed_frame = &mut mixed[frame * usize::from(device_channels)
                    ..(frame + 1) * usize::from(device_channels)];
                for (channel, sample) in mix_frame_into(source, mixed_frame).iter().enumerate() {
                    destination[channel] = convert(*sample);
                }
            }
        },
        error_callback,
        None,
    )
}

fn mix_frame_into<'a>(source: &[f32], destination: &'a mut [f32]) -> &'a [f32] {
    if source.is_empty() {
        destination.fill(0.0);
    } else if destination.len() == 1 {
        destination[0] = source.iter().sum::<f32>() / source.len() as f32;
    } else {
        for (channel, sample) in destination.iter_mut().enumerate() {
            *sample = source[channel.min(source.len() - 1)];
        }
    }
    destination
}

#[cfg(test)]
fn mix_frame(source: &[f32], device_channels: usize) -> Vec<f32> {
    let mut destination = vec![0.0; device_channels];
    mix_frame_into(source, &mut destination);
    destination
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_audio_frames_for_device_channel_counts() {
        let stereo = [0.25, -0.75];
        assert_eq!(mix_frame(&stereo, 2), vec![0.25, -0.75]);
        assert_eq!(mix_frame(&stereo, 1), vec![-0.25]);
        assert_eq!(mix_frame(&[0.5], 2), vec![0.5, 0.5]);
        assert_eq!(mix_frame(&stereo, 4), vec![0.25, -0.75, -0.75, -0.75]);
    }

    #[test]
    fn selects_preferred_supported_format_regardless_of_candidate_order() {
        let candidates = [
            OutputCandidate {
                sample_format: cpal::SampleFormat::U8,
                min_sample_rate: 48_000,
                max_sample_rate: 48_000,
            },
            OutputCandidate {
                sample_format: cpal::SampleFormat::I32,
                min_sample_rate: 48_000,
                max_sample_rate: 48_000,
            },
            OutputCandidate {
                sample_format: cpal::SampleFormat::I16,
                min_sample_rate: 48_000,
                max_sample_rate: 48_000,
            },
            OutputCandidate {
                sample_format: cpal::SampleFormat::F32,
                min_sample_rate: 48_000,
                max_sample_rate: 48_000,
            },
        ];
        assert_eq!(
            select_output_format(&candidates, 48_000),
            Some(cpal::SampleFormat::F32)
        );
        let mut reversed = candidates;
        reversed.reverse();
        assert_eq!(
            select_output_format(&reversed, 48_000),
            Some(cpal::SampleFormat::F32)
        );
        assert_eq!(select_output_format(&candidates, 44_100), None);
    }

    #[test]
    fn converts_i32_and_u8_samples_with_full_scale_endpoints() {
        assert_eq!(convert_i32(-1.0), i32::MIN);
        assert_eq!(convert_i32(1.0), i32::MAX);
        assert_eq!(convert_u8(-1.0), 0);
        assert_eq!(convert_u8(0.0), 128);
        assert_eq!(convert_u8(1.0), u8::MAX);
    }
}
