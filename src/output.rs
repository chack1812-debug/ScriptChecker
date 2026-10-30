use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
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

pub fn start_output(
    audio: AudioData,
    transport: Arc<Mutex<Transport>>,
) -> Result<Output, String> {
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
    device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            let mut rendered = vec![0.0; out.len() / usize::from(device_channels) * usize::from(audio_channels)];
            if let Ok(mut transport) = transport.try_lock() {
                transport.render(&samples, audio_channels, &mut rendered);
            }
            for (frame, destination) in out.chunks_exact_mut(usize::from(device_channels)).enumerate() {
                let source = &rendered[frame * usize::from(audio_channels)..][..usize::from(audio_channels)];
                for (channel, sample) in destination.iter_mut().enumerate() {
                    *sample = convert(source[channel.min(source.len() - 1)]);
                }
            }
        },
        error_callback,
        None,
    )
}
