use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub const FAST_RATE: f64 = 4.0;
pub const SKIP_MS: u64 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Stopped,
    Play,
    Forward,
    Rewind,
}

pub struct Transport {
    pos_frames: f64,
    total_frames: u64,
    sample_rate: u32,
    mode: Mode,
    seek_epoch: u64,
    output_latency_frames: Arc<AtomicU64>,
}

impl Transport {
    pub fn new(total_frames: u64, sample_rate: u32) -> Self {
        Self {
            pos_frames: 0.0,
            total_frames,
            sample_rate,
            mode: Mode::Stopped,
            seek_epoch: 0,
            output_latency_frames: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Stopped && self.mode != Mode::Stopped {
            let audible = self.audible_position_frames();
            if self.pos_frames != audible {
                self.pos_frames = audible;
                self.seek_epoch = self.seek_epoch.wrapping_add(1);
            }
        }
        self.mode = mode;
    }

    pub fn position_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            0
        } else {
            (self.audible_position_frames() * 1_000.0 / f64::from(self.sample_rate)) as u64
        }
    }

    pub fn seek_ms(&mut self, ms: i64) {
        let requested = ms.max(0) as f64 * f64::from(self.sample_rate) / 1_000.0;
        let at_or_past_boundary = ms < 0 || requested >= self.total_frames as f64;
        let clamped = if at_or_past_boundary {
            requested.min(self.total_frames as f64)
        } else {
            self.raw_position_for_audible_position(requested)
        };
        if at_or_past_boundary {
            self.mode = Mode::Stopped;
        }
        if self.pos_frames != clamped {
            self.pos_frames = clamped;
            self.seek_epoch = self.seek_epoch.wrapping_add(1);
        }
    }

    pub fn seek_epoch(&self) -> u64 {
        self.seek_epoch
    }

    pub(crate) fn output_latency_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.output_latency_frames)
    }

    #[cfg(test)]
    fn set_output_latency_frames(&self, frames: u64) {
        self.output_latency_frames.store(frames, Ordering::Relaxed);
    }

    pub fn skip_ms(&mut self, delta_ms: i64) {
        let target = self.position_ms().saturating_add_signed(delta_ms);
        self.seek_ms(target as i64);
    }

    pub fn render(&mut self, samples: &[f32], channels: u16, out: &mut [f32]) {
        out.fill(0.0);
        let channels = usize::from(channels);
        if channels == 0 || out.len() < channels || matches!(self.mode, Mode::Stopped) {
            return;
        }

        for output_frame in out.chunks_exact_mut(channels) {
            if self.at_boundary() {
                self.mode = Mode::Stopped;
                self.clamp_position();
                break;
            }
            let source_frame = if matches!(self.mode, Mode::Rewind)
                && self.pos_frames >= self.total_frames as f64
            {
                self.total_frames.saturating_sub(1) as usize
            } else {
                self.pos_frames.floor() as usize
            };
            let source_start = source_frame * channels;
            let Some(source) = samples.get(source_start..source_start + channels) else {
                self.mode = Mode::Stopped;
                self.clamp_position();
                break;
            };
            output_frame.copy_from_slice(source);
            self.advance();
            if self.at_boundary() {
                self.mode = Mode::Stopped;
                self.clamp_position();
            }
        }
    }

    fn at_boundary(&self) -> bool {
        match self.mode {
            Mode::Stopped => true,
            Mode::Rewind => self.pos_frames <= 0.0,
            Mode::Play | Mode::Forward => self.pos_frames >= self.total_frames as f64,
        }
    }

    fn advance(&mut self) {
        match self.mode {
            Mode::Play => self.pos_frames += 1.0,
            Mode::Forward => self.pos_frames += FAST_RATE,
            Mode::Rewind => self.pos_frames -= FAST_RATE,
            Mode::Stopped => {}
        }
    }

    fn clamp_position(&mut self) {
        self.pos_frames = self.pos_frames.clamp(0.0, self.total_frames as f64);
    }

    fn audible_position_frames(&self) -> f64 {
        let latency = self.output_latency_frames.load(Ordering::Relaxed) as f64;
        let compensated = match self.mode {
            Mode::Play => self.pos_frames - latency,
            Mode::Forward => self.pos_frames - latency * FAST_RATE,
            Mode::Rewind => self.pos_frames + latency * FAST_RATE,
            Mode::Stopped => self.pos_frames,
        };
        compensated.clamp(0.0, self.total_frames as f64)
    }

    fn raw_position_for_audible_position(&self, audible_position: f64) -> f64 {
        let latency = self.output_latency_frames.load(Ordering::Relaxed) as f64;
        let rendered = match self.mode {
            Mode::Play => audible_position + latency,
            Mode::Forward => audible_position + latency * FAST_RATE,
            Mode::Rewind => audible_position - latency * FAST_RATE,
            Mode::Stopped => audible_position,
        };
        rendered.clamp(0.0, self.total_frames as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Timeline, TimelineLine, current_line};

    fn stereo_frames(frame_count: usize) -> Vec<f32> {
        (0..frame_count)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect()
    }

    #[test]
    fn play_outputs_source_frames_and_advances_position() {
        let samples = stereo_frames(1_000);
        let mut transport = Transport::new(1_000, 1_000);
        transport.set_mode(Mode::Play);
        let mut output = vec![0.0; 200];
        transport.render(&samples, 2, &mut output);
        assert_eq!(transport.position_ms(), 100);
        assert_eq!(output, samples[..200]);
    }

    #[test]
    fn fast_forward_and_rewind_use_four_times_rate_and_reverse_samples() {
        let samples = stereo_frames(1_000);
        let mut transport = Transport::new(1_000, 1_000);
        transport.set_mode(Mode::Forward);
        let mut output = vec![0.0; 20];
        transport.render(&samples, 2, &mut output);
        assert_eq!(transport.position_ms(), 40);

        transport.seek_ms(500);
        transport.set_mode(Mode::Rewind);
        transport.render(&samples, 2, &mut output);
        assert_eq!(output[..4], [500.0, -500.0, 496.0, -496.0]);
        assert_eq!(transport.position_ms(), 460);
    }

    #[test]
    fn stops_and_clamps_at_transport_boundaries() {
        let samples = stereo_frames(20_000);
        let mut transport = Transport::new(20_000, 1_000);
        let mut output = vec![1.0; 20];

        transport.set_mode(Mode::Rewind);
        transport.render(&samples, 2, &mut output);
        assert_eq!(transport.mode(), Mode::Stopped);
        assert_eq!(transport.position_ms(), 0);
        assert!(output.iter().all(|sample| *sample == 0.0));

        transport.seek_ms(20_000);
        transport.set_mode(Mode::Play);
        transport.render(&samples, 2, &mut output);
        assert_eq!(transport.mode(), Mode::Stopped);
        assert_eq!(transport.position_ms(), 20_000);
        assert!(output.iter().all(|sample| *sample == 0.0));

        transport.seek_ms(-5);
        assert_eq!(transport.position_ms(), 0);
        assert_eq!(transport.mode(), Mode::Stopped);
        transport.seek_ms(20_001);
        assert_eq!(transport.position_ms(), 20_000);

        transport.skip_ms(-10_000);
        assert_eq!(transport.position_ms(), 10_000);
        transport.skip_ms(10_000);
        assert_eq!(transport.position_ms(), 20_000);

        transport.set_mode(Mode::Rewind);
        transport.render(&samples, 2, &mut output);
        assert_eq!(output[..4], [19_999.0, -19_999.0, 19_996.0, -19_996.0]);
        assert_eq!(transport.mode(), Mode::Rewind);
        assert_eq!(transport.position_ms(), 19_960);

        transport.set_mode(Mode::Stopped);
        transport.render(&samples, 2, &mut output);
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(transport.position_ms(), 19_960);
    }

    fn two_line_timeline() -> Timeline {
        Timeline {
            script_sha256: String::new(),
            duration_ms: 2_000,
            sample_rate: 1_000,
            lines: vec![
                TimelineLine {
                    id: "L001".to_owned(),
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "first".to_owned(),
                    missing: false,
                },
                TimelineLine {
                    id: "L002".to_owned(),
                    start_ms: 1_000,
                    end_ms: 2_000,
                    text: "second".to_owned(),
                    missing: false,
                },
            ],
        }
    }

    #[test]
    fn stopping_at_an_output_latency_boundary_keeps_the_audible_line() {
        let timeline = two_line_timeline();
        let mut transport = Transport::new(2_000, 1_000);

        transport.seek_ms(1_050);
        transport.set_output_latency_frames(100);
        transport.set_mode(Mode::Play);
        assert_eq!(current_line(&timeline, transport.position_ms()), Some(0));

        transport.set_mode(Mode::Stopped);
        assert_eq!(transport.position_ms(), 950);
        assert_eq!(current_line(&timeline, transport.position_ms()), Some(0));
    }

    #[test]
    fn stopping_rewind_uses_the_opposite_latency_direction_and_clamps() {
        let timeline = two_line_timeline();
        let mut transport = Transport::new(2_000, 1_000);

        transport.seek_ms(650);
        transport.set_output_latency_frames(100);
        transport.set_mode(Mode::Rewind);
        transport.set_mode(Mode::Stopped);
        assert_eq!(transport.position_ms(), 1_050);
        assert_eq!(current_line(&timeline, transport.position_ms()), Some(1));

        transport.seek_ms(50);
        transport.set_mode(Mode::Play);
        transport.set_mode(Mode::Stopped);
        assert_eq!(transport.position_ms(), 0);

        transport.seek_ms(1_950);
        transport.set_mode(Mode::Rewind);
        transport.set_mode(Mode::Stopped);
        assert_eq!(transport.position_ms(), 2_000);
    }

    #[test]
    fn seek_and_skip_use_the_audible_position() {
        let mut transport = Transport::new(30_000, 1_000);

        transport.seek_ms(15_000);
        transport.set_output_latency_frames(1_000);
        transport.set_mode(Mode::Play);
        assert_eq!(transport.position_ms(), 14_000);

        transport.skip_ms(-(SKIP_MS as i64));
        assert_eq!(transport.position_ms(), 4_000);

        transport.seek_ms(10_000);
        assert_eq!(transport.position_ms(), 10_000);

        transport.seek_ms(-1);
        assert_eq!(transport.position_ms(), 0);

        transport.set_mode(Mode::Play);
        transport.seek_ms(30_001);
        assert_eq!(transport.position_ms(), 30_000);
    }
}
