use std::{path::Path, sync::Arc};

pub struct AudioData {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Arc<Vec<f32>>,
}

impl AudioData {
    pub fn frame_count(&self) -> u64 {
        u64::try_from(self.samples.len() / usize::from(self.channels)).unwrap_or(u64::MAX)
    }

    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            0
        } else {
            self.frame_count() * 1_000 / u64::from(self.sample_rate)
        }
    }
}

pub fn load_wav(path: &Path) -> Result<AudioData, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|error| format!("WAV を開けません {}: {error}", path.display()))?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>(),
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<Vec<_>, _>>()
        }
    }
    .map_err(|error| format!("WAV を読み取れません {}: {error}", path.display()))?;
    Ok(AudioData {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples: Arc::new(samples),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn temp_wav(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("scriptchecker-{name}-{}.wav", std::process::id()))
    }

    #[test]
    fn loads_float_and_integer_stereo_wav_as_f32_frames() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let float_path = temp_wav("float");
        let mut writer = WavWriter::create(&float_path, spec).expect("create float wav");
        for frame in 0..24_000 {
            let sample = (std::f32::consts::TAU * frame as f32 / 480.0).sin();
            writer.write_sample(sample).expect("write left");
            writer.write_sample(-sample).expect("write right");
        }
        writer.finalize().expect("finalize float wav");
        let float_audio = load_wav(&float_path).expect("load float wav");
        assert_eq!(float_audio.channels, 2);
        assert_eq!(float_audio.frame_count(), 24_000);
        assert_eq!(float_audio.duration_ms(), 500);
        assert!(float_audio.samples[2] > 0.0);
        std::fs::remove_file(&float_path).expect("remove float wav");

        let integer_path = temp_wav("integer");
        let integer_spec = WavSpec {
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
            ..spec
        };
        let mut writer =
            WavWriter::create(&integer_path, integer_spec).expect("create integer wav");
        writer
            .write_sample(16_384_i16)
            .expect("write integer sample");
        writer
            .write_sample(-16_384_i16)
            .expect("write integer sample");
        writer.finalize().expect("finalize integer wav");
        let integer_audio = load_wav(&integer_path).expect("load integer wav");
        assert!((integer_audio.samples[0] - 0.5).abs() < 0.001);
        assert!((integer_audio.samples[1] + 0.5).abs() < 0.001);
        std::fs::remove_file(&integer_path).expect("remove integer wav");
    }

    #[test]
    fn reports_missing_wav_path() {
        let path = temp_wav("missing");
        let error = match load_wav(&path) {
            Err(error) => error,
            Ok(_) => panic!("missing wav must fail"),
        };
        assert!(error.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    #[ignore]
    fn loads_episode_60_extensible_wav() {
        let audio = load_wav(std::path::Path::new(
            r"D:\UDS\YouTube\AI道具箱\第60回\full_dialogue.wav",
        ))
        .expect("hound supports episode 60 WAVE_FORMAT_EXTENSIBLE");
        assert!((audio.duration_ms() as i64 - 936_710).abs() <= 50);
    }
}
