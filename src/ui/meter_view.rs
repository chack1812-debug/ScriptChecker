use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};

use crate::meter::MeterFrame;

pub const METER_WINDOW_S: f32 = 0.05;

#[derive(Default)]
pub struct MeterViewState {
    peak_hold_dbfs: Vec<f32>,
}

impl MeterViewState {
    pub fn update(&mut self, meter: &MeterFrame) {
        if self.peak_hold_dbfs.len() < meter.channels.len() {
            self.peak_hold_dbfs
                .resize(meter.channels.len(), f32::NEG_INFINITY);
        }
        for (hold, channel) in self.peak_hold_dbfs.iter_mut().zip(&meter.channels) {
            *hold = hold.max(channel.peak_dbfs);
        }
    }

    pub fn reset(&mut self) {
        self.peak_hold_dbfs.clear();
    }

    pub fn hold(&self) -> &[f32] {
        &self.peak_hold_dbfs
    }
}

pub fn meter_y_fraction(dbfs: f32) -> f32 {
    ((dbfs + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn draw_meter(ui: &mut egui::Ui, state: &mut MeterViewState, meter: &MeterFrame) {
    state.update(meter);
    let height = ui.available_height().max(120.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let tick_width = 32.0;
    let bar_width = (rect.width() - tick_width) / 2.0;
    for dbfs in [-60.0, -48.0, -36.0, -24.0, -12.0, 0.0] {
        let y = rect.bottom() - meter_y_fraction(dbfs) * rect.height();
        painter.text(
            Pos2::new(rect.left(), y),
            Align2::LEFT_CENTER,
            format!("{dbfs:.0}"),
            FontId::proportional(9.0),
            Color32::LIGHT_GRAY,
        );
        painter.line_segment(
            [Pos2::new(rect.left() + tick_width, y), Pos2::new(rect.right(), y)],
            Stroke::new(0.5, Color32::DARK_GRAY),
        );
    }
    for channel in 0..2 {
        let left = rect.left() + tick_width + channel as f32 * bar_width;
        let bar = Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(left + bar_width, rect.bottom()));
        painter.rect_filled(bar, 0.0, Color32::from_gray(30));
        if let Some(level) = meter.channels.get(channel) {
            let rms_y = rect.bottom() - meter_y_fraction(level.rms_dbfs) * rect.height();
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(bar.left(), rms_y), bar.right_bottom()),
                0.0,
                Color32::from_rgb(60, 190, 100),
            );
            draw_line(&painter, bar, level.peak_dbfs, Color32::from_rgb(80, 150, 255));
            if let Some(hold) = state.hold().get(channel) {
                draw_line(&painter, bar, *hold, Color32::RED);
            }
            painter.text(
                Pos2::new(bar.center().x, rect.bottom() + 2.0),
                Align2::CENTER_TOP,
                format!("{:.1}", level.peak_dbfs),
                FontId::proportional(8.0),
                Color32::WHITE,
            );
        }
    }
}

fn draw_line(painter: &egui::Painter, bar: Rect, dbfs: f32, color: Color32) {
    let y = bar.bottom() - meter_y_fraction(dbfs) * bar.height();
    painter.line_segment(
        [Pos2::new(bar.left(), y), Pos2::new(bar.right(), y)],
        Stroke::new(1.5, color),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::{ChannelMeter, MeterFrame};

    #[test]
    fn maps_dbfs_and_holds_channel_peaks() {
        assert_eq!(meter_y_fraction(-60.0), 0.0);
        assert_eq!(meter_y_fraction(0.0), 1.0);
        assert_eq!(meter_y_fraction(-30.0), 0.5);
        assert_eq!(meter_y_fraction(-90.0), 0.0);
        assert_eq!(meter_y_fraction(6.0), 1.0);

        let mut state = MeterViewState::default();
        state.update(&MeterFrame {
            channels: vec![ChannelMeter {
                rms_dbfs: -20.0,
                peak_dbfs: -10.0,
            }],
        });
        state.update(&MeterFrame {
            channels: vec![ChannelMeter {
                rms_dbfs: -30.0,
                peak_dbfs: -20.0,
            }],
        });
        assert_eq!(state.hold(), &[-10.0]);
        state.reset();
        assert!(state.hold().is_empty());
    }
}
