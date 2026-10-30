/// Fixed lower bound used for meter readings in dBFS.
pub const METER_FLOOR_DBFS: f32 = -120.0;

/// RMS and peak measurements for one audio channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMeter {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
}

/// Meter measurements for all channels in a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterFrame {
    pub channels: Vec<ChannelMeter>,
}

fn dbfs(amplitude: f32) -> f32 {
    if amplitude.is_finite() && amplitude > 0.0 {
        (20.0 * amplitude.log10()).max(METER_FLOOR_DBFS)
    } else {
        METER_FLOOR_DBFS
    }
}

/// Copied and adapted from スペアナⅡ's `crates/sp2-dsp/src/meter.rs`.
pub fn analyze_meter(
    interleaved: &[f32],
    channel_count: u16,
    sample_rate_hz: u32,
    center_time_s: f32,
    window_duration_s: f32,
) -> MeterFrame {
    let channel_total = usize::from(channel_count);
    if channel_total == 0 || sample_rate_hz == 0 {
        return MeterFrame {
            channels: Vec::new(),
        };
    }

    let frame_count = interleaved.len() / channel_total;
    if frame_count == 0 {
        return MeterFrame {
            channels: Vec::new(),
        };
    }

    let requested_window = if window_duration_s.is_finite() && window_duration_s > 0.0 {
        (window_duration_s * sample_rate_hz as f32).round() as usize
    } else {
        1
    };
    let window_frames = requested_window.max(1).min(frame_count);
    let max_start = frame_count - window_frames;
    let center_frame = if center_time_s.is_finite() && center_time_s > 0.0 {
        (center_time_s * sample_rate_hz as f32).round() as usize
    } else {
        0
    };
    let start = center_frame
        .saturating_sub(window_frames / 2)
        .min(max_start);

    let channels = (0..channel_total)
        .map(|channel| {
            let mut sum_squares = 0.0_f32;
            let mut peak = 0.0_f32;
            let mut has_non_finite = false;
            for frame in start..start + window_frames {
                let sample = interleaved[frame * channel_total + channel];
                if !sample.is_finite() {
                    has_non_finite = true;
                } else {
                    let magnitude = sample.abs();
                    peak = peak.max(magnitude);
                    sum_squares += sample * sample;
                }
            }
            if has_non_finite {
                return ChannelMeter {
                    rms_dbfs: METER_FLOOR_DBFS,
                    peak_dbfs: METER_FLOOR_DBFS,
                };
            }
            let rms = (sum_squares / window_frames as f32).sqrt();
            ChannelMeter {
                rms_dbfs: dbfs(rms),
                peak_dbfs: dbfs(peak),
            }
        })
        .collect();
    MeterFrame { channels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_peak_silence_and_mono_channels() {
        let full_scale = analyze_meter(&[1.0, 1.0, 1.0, 1.0], 2, 48_000, 0.0, 0.05);
        assert_eq!(full_scale.channels.len(), 2);
        assert!(full_scale.channels[0].peak_dbfs.abs() < 0.01);

        let silence = analyze_meter(&[0.0, 0.0], 2, 48_000, 0.0, 0.05);
        assert_eq!(silence.channels[0].peak_dbfs, METER_FLOOR_DBFS);

        let mono = analyze_meter(&[0.5, -0.5], 1, 48_000, 0.0, 0.05);
        assert_eq!(mono.channels.len(), 1);
    }
}
