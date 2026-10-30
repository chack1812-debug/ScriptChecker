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
}

impl Transport {
    pub fn new(total_frames: u64, sample_rate: u32) -> Self {
        Self {
            pos_frames: 0.0,
            total_frames,
            sample_rate,
            mode: Mode::Stopped,
            seek_epoch: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn position_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            0
        } else {
            (self.pos_frames * 1_000.0 / f64::from(self.sample_rate)) as u64
        }
    }

    pub fn seek_ms(&mut self, ms: i64) {
        let requested = ms.max(0) as f64 * f64::from(self.sample_rate) / 1_000.0;
        let clamped = requested.min(self.total_frames as f64);
        if ms < 0 || requested >= self.total_frames as f64 {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
