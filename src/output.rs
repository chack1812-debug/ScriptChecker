use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{audio::AudioData, transport::Transport};

pub struct Output {
    _stream: cpal::Stream,
    error: Arc<AtomicBool>,
    error_message: Arc<Mutex<Option<String>>>,
}

impl Output {
    pub fn has_error(&self) -> bool {
        self.error.load(Ordering::Relaxed)
    }

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
        .find(|config| {
            config.min_sample_rate().0 <= audio.sample_rate
                && audio.sample_rate <= config.max_sample_rate().0
        })
        .map(|config| config.with_sample_rate(cpal::SampleRate(audio.sample_rate)))
        .ok_or_else(|| format!("出力デバイスは {}Hz に対応していません", audio.sample_rate))?;

    let config = supported.config();
    let device_channels = config.channels;
    let error = Arc::new(AtomicBool::new(false));
    let error_message = Arc::new(Mutex::new(None));
    let error_callback = {
        let error = Arc::clone(&error);
        let error_message = Arc::clone(&error_message);
        move |stream_error: cpal::StreamError| {
            error.store(true, Ordering::Relaxed);
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
        format => return Err(format!("未対応の出力サンプル形式: {format:?}")),
    }
    .map_err(|error| format!("出力ストリームを作成できません: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("出力ストリームを開始できません: {error}"))?;
    Ok(Output {
        _stream: stream,
        error,
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
}
