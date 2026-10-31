use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};

use crate::meter::MeterFrame;

pub const METER_WINDOW_S: f32 = 0.3;
pub const METER_PANEL_WIDTH: f32 = 136.0;
pub const NUMERIC_DISPLAY_INTERVAL_S: f64 = 0.05;
const TICK_LABEL_PADDING: f32 = 6.0;

#[derive(Default)]
pub struct MeterViewState {
    peak_hold_dbfs: Vec<f32>,
    displayed_levels: Vec<crate::meter::ChannelMeter>,
    displayed_hold_dbfs: Vec<f32>,
    last_numeric_update_s: Option<f64>,
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

    pub fn reset_for_audio(&mut self) {
        self.reset_peak_hold();
        self.displayed_levels.clear();
        self.last_numeric_update_s = None;
    }

    pub fn reset_peak_hold(&mut self) {
        self.peak_hold_dbfs.clear();
        self.displayed_hold_dbfs.clear();
    }

    pub fn hold(&self) -> &[f32] {
        &self.peak_hold_dbfs
    }

    pub fn update_numeric_display(&mut self, meter: &MeterFrame, now_s: f64) -> bool {
        self.update_numeric_display_when(meter, now_s, false)
    }

    pub fn refresh_numeric_display(&mut self, meter: &MeterFrame, now_s: f64) {
        let _ = self.update_numeric_display_when(meter, now_s, true);
    }

    fn update_numeric_display_when(&mut self, meter: &MeterFrame, now_s: f64, force: bool) -> bool {
        self.update(meter);
        if !force && !should_update_numeric_display(self.last_numeric_update_s, now_s) {
            return false;
        }
        self.displayed_levels.clone_from(&meter.channels);
        self.displayed_hold_dbfs.clone_from(&self.peak_hold_dbfs);
        self.last_numeric_update_s = now_s.is_finite().then_some(now_s);
        true
    }

    pub fn displayed_levels(&self) -> &[crate::meter::ChannelMeter] {
        &self.displayed_levels
    }

    fn displayed_hold(&self) -> &[f32] {
        &self.displayed_hold_dbfs
    }
}

pub fn should_update_numeric_display(last_update_s: Option<f64>, now_s: f64) -> bool {
    now_s.is_finite()
        && last_update_s
            .is_none_or(|last_update_s| now_s - last_update_s >= NUMERIC_DISPLAY_INTERVAL_S)
}

pub fn meter_y_fraction(dbfs: f32) -> f32 {
    ((dbfs + 60.0) / 60.0).clamp(0.0, 1.0)
}

struct MeterLayout {
    rect: Rect,
    bar_rect: Rect,
    number_rect: Rect,
}

fn meter_layout(available: Rect) -> MeterLayout {
    let width = available.width().min(METER_PANEL_WIDTH);
    let rect = Rect::from_min_size(available.min, egui::vec2(width, available.height()));
    let number_height = 58.0_f32.min(rect.height());
    let number_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - number_height),
        rect.right_bottom(),
    );
    let bar_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + TICK_LABEL_PADDING),
        Pos2::new(rect.right(), number_rect.top() - TICK_LABEL_PADDING),
    );
    MeterLayout {
        rect,
        bar_rect,
        number_rect,
    }
}

fn level_text(dbfs: f32) -> String {
    if dbfs <= -100.0 {
        "-∞".to_owned()
    } else {
        format!("{dbfs:+.1}")
    }
}

fn number_cell_rect(number_rect: Rect, row: usize, column: usize) -> Rect {
    let label_width = 31.0;
    let row_height = number_rect.height() / 3.0;
    let value_width = (number_rect.width() - label_width).max(0.0) / 2.0;
    Rect::from_min_size(
        Pos2::new(
            label_width + number_rect.left() + value_width * column as f32,
            number_rect.top() + row_height * row as f32,
        ),
        egui::vec2(value_width, row_height),
    )
}

pub fn draw_meter(ui: &mut egui::Ui, state: &mut MeterViewState, meter: &MeterFrame, now_s: f64) {
    let _ = draw_meter_with_report(ui, state, meter, now_s);
}

#[allow(dead_code)] // Production ignores this test-only rendering report.
struct MeterDrawReport {
    numeric_text_rects: Vec<Rect>,
    tick_text_rects: Vec<Rect>,
    number_rect: Rect,
    outer_rect: Rect,
}

fn draw_meter_with_report(
    ui: &mut egui::Ui,
    state: &mut MeterViewState,
    meter: &MeterFrame,
    now_s: f64,
) -> MeterDrawReport {
    state.update_numeric_display(meter, now_s);
    let height = ui.available_height().max(120.0);
    let available = Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    let layout = meter_layout(available);
    let (rect, _) = ui.allocate_exact_size(layout.rect.size(), egui::Sense::hover());
    let offset = rect.min - layout.rect.min;
    let bar_rect = layout.bar_rect.translate(offset);
    let number_rect = layout.number_rect.translate(offset);
    let painter = ui.painter_at(rect);
    let tick_width = 32.0;
    let bar_width = ((bar_rect.width() - tick_width) / 2.0).max(0.0);
    let mut tick_text_rects = Vec::new();
    for dbfs in [-60.0, -48.0, -36.0, -24.0, -12.0, 0.0] {
        let y = bar_rect.bottom() - meter_y_fraction(dbfs) * bar_rect.height();
        tick_text_rects.push(painter.text(
            Pos2::new(rect.left(), y),
            Align2::LEFT_CENTER,
            format!("{dbfs:.0}"),
            FontId::proportional(9.0),
            ui.visuals().text_color(),
        ));
        painter.line_segment(
            [
                Pos2::new(rect.left() + tick_width, y),
                Pos2::new(bar_rect.right(), y),
            ],
            Stroke::new(0.5, Color32::DARK_GRAY),
        );
    }
    for channel in 0..2 {
        let left = rect.left() + tick_width + channel as f32 * bar_width;
        let bar = Rect::from_min_max(
            Pos2::new(left, bar_rect.top()),
            Pos2::new(left + bar_width, bar_rect.bottom()),
        );
        painter.rect_filled(bar, 0.0, Color32::from_gray(30));
        if let Some(level) = meter.channels.get(channel) {
            let rms_y = bar.bottom() - meter_y_fraction(level.rms_dbfs) * bar.height();
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(bar.left(), rms_y), bar.right_bottom()),
                0.0,
                Color32::from_rgb(60, 190, 100),
            );
            draw_line(
                &painter,
                bar,
                level.peak_dbfs,
                Color32::from_rgb(80, 150, 255),
            );
            if let Some(hold) = state.hold().get(channel) {
                draw_line(&painter, bar, *hold, Color32::RED);
            }
        }
    }
    let mut text_rects = Vec::new();
    for (row, label) in ["RMS", "Peak", "Hold"].into_iter().enumerate() {
        let y = number_rect.top() + number_rect.height() * (row as f32 + 0.5) / 3.0;
        text_rects.push(painter.text(
            Pos2::new(number_rect.left(), y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(9.0),
            ui.visuals().text_color(),
        ));
        for channel in 0..2 {
            let value = state.displayed_levels().get(channel);
            let dbfs = match row {
                0 => value.map_or(f32::NEG_INFINITY, |level| level.rms_dbfs),
                1 => value.map_or(f32::NEG_INFINITY, |level| level.peak_dbfs),
                _ => state
                    .displayed_hold()
                    .get(channel)
                    .copied()
                    .unwrap_or(f32::NEG_INFINITY),
            };
            let cell = number_cell_rect(number_rect, row, channel);
            let text_rect = painter.text(
                cell.center(),
                Align2::CENTER_CENTER,
                level_text(dbfs),
                FontId::proportional(9.0),
                ui.visuals().text_color(),
            );
            text_rects.push(text_rect);
        }
    }
    MeterDrawReport {
        numeric_text_rects: text_rects,
        tick_text_rects,
        number_rect,
        outer_rect: rect,
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

    fn test_luminance(color: Color32) -> f32 {
        let linear = |component: u8| {
            let component = component as f32 / 255.0;
            if component <= 0.04045 {
                component / 12.92
            } else {
                ((component + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn test_contrast_ratio(left: Color32, right: Color32) -> f32 {
        let (left, right) = (test_luminance(left), test_luminance(right));
        (left.max(right) + 0.05) / (left.min(right) + 0.05)
    }

    fn drawn_text_colours(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32)> {
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<(String, Color32)>) {
            match shape {
                egui::epaint::Shape::Text(text) => texts.push((
                    text.galley.job.text.clone(),
                    text.override_text_color.unwrap_or(text.fallback_color),
                )),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts);
                    }
                }
                _ => {}
            }
        }

        let mut texts = Vec::new();
        for clipped in shapes {
            collect(&clipped.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn numeric_display_updates_only_after_fifty_milliseconds() {
        assert!(should_update_numeric_display(None, 0.0));
        assert!(!should_update_numeric_display(Some(1.0), 1.049));
        assert!(should_update_numeric_display(Some(1.0), 1.05));

        let first = MeterFrame {
            channels: vec![ChannelMeter {
                rms_dbfs: -20.0,
                peak_dbfs: -10.0,
            }],
        };
        let second = MeterFrame {
            channels: vec![ChannelMeter {
                rms_dbfs: -40.0,
                peak_dbfs: -30.0,
            }],
        };
        let mut state = MeterViewState::default();
        assert!(state.update_numeric_display(&first, 0.0));
        assert!(!state.update_numeric_display(&second, 0.049));
        assert_eq!(state.displayed_levels()[0].rms_dbfs, -20.0);
        assert!(state.update_numeric_display(&second, 0.05));
        assert_eq!(state.displayed_levels()[0].rms_dbfs, -40.0);
    }

    #[test]
    fn reloading_audio_and_reset_peak_clear_peak_hold() {
        let meter = MeterFrame {
            channels: vec![ChannelMeter {
                rms_dbfs: -12.0,
                peak_dbfs: -3.0,
            }],
        };
        let mut state = MeterViewState::default();

        state.update_numeric_display(&meter, 0.0);
        state.reset_peak_hold();
        assert!(state.hold().is_empty());
        assert!(state.displayed_hold().is_empty());

        state.update_numeric_display(&meter, 0.05);
        state.reset_for_audio();
        assert!(state.hold().is_empty());
        assert!(state.displayed_levels().is_empty());
    }

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
        state.reset_peak_hold();
        assert!(state.hold().is_empty());
    }

    #[test]
    fn meter_layout_keeps_bars_and_peak_numbers_inside_available_rect() {
        let outer = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(74.0, 200.0));
        let layout = meter_layout(outer);
        assert!(layout.rect.width() <= outer.width());
        assert!(outer.contains_rect(layout.bar_rect));
        assert!(outer.contains_rect(layout.number_rect));
        assert!(layout.bar_rect.bottom() < layout.number_rect.top());
        assert!(layout.bar_rect.top() > layout.rect.top());
    }

    #[test]
    fn formats_floor_and_lays_out_all_numeric_cells_inside_panel() {
        assert_eq!(level_text(-120.0), "-∞");
        assert_eq!(level_text(-9.94), "-9.9");
        let number = Rect::from_min_size(Pos2::ZERO, egui::vec2(136.0, 58.0));
        for row in 0..3 {
            for column in 0..2 {
                assert!(number.contains_rect(number_cell_rect(number, row, column)));
            }
        }
    }

    #[test]
    fn headless_meter_draws_nine_numeric_texts_inside_its_number_rect() {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(METER_PANEL_WIDTH, 240.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut state = MeterViewState::default();
                    let meter = MeterFrame {
                        channels: vec![
                            ChannelMeter {
                                rms_dbfs: 0.0,
                                peak_dbfs: -120.0,
                            },
                            ChannelMeter {
                                rms_dbfs: -12.0,
                                peak_dbfs: -3.0,
                            },
                        ],
                    };
                    let report = draw_meter_with_report(ui, &mut state, &meter, 0.0);
                    assert_eq!(report.numeric_text_rects.len(), 9);
                    assert!(
                        report
                            .numeric_text_rects
                            .iter()
                            .all(|rect| report.number_rect.contains_rect(*rect))
                    );
                    assert_eq!(report.tick_text_rects.len(), 6);
                    assert!(
                        report
                            .tick_text_rects
                            .iter()
                            .all(|rect| report.outer_rect.contains_rect(*rect))
                    );
                    let bar = Rect::from_min_max(Pos2::new(32.0, 0.0), Pos2::new(84.0, 100.0));
                    assert_eq!(
                        bar.top(),
                        bar.bottom() - meter_y_fraction(0.0) * bar.height()
                    );
                });
            },
        );
    }

    #[test]
    fn headless_meter_tick_and_row_label_text_meet_wcag_contrast_in_both_themes() {
        let required = [
            "-60", "-48", "-36", "-24", "-12", "0", "RMS", "Peak", "Hold",
        ];
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let panel_background = visuals.panel_fill;
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(METER_PANEL_WIDTH, 240.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut state = MeterViewState::default();
                        draw_meter(
                            ui,
                            &mut state,
                            &MeterFrame {
                                channels: vec![ChannelMeter {
                                    rms_dbfs: -12.0,
                                    peak_dbfs: -3.0,
                                }],
                            },
                            0.0,
                        );
                    });
                },
            );
            let drawn = drawn_text_colours(&output.shapes);
            for label in required {
                let foreground = drawn
                    .iter()
                    .find_map(|(text, color)| (text == label).then_some(*color))
                    .unwrap_or_else(|| panic!("{label:?} must be drawn"));
                assert!(
                    test_contrast_ratio(foreground, panel_background) >= 4.5,
                    "{label:?} contrast={} foreground={foreground:?} background={panel_background:?}",
                    test_contrast_ratio(foreground, panel_background)
                );
            }
        }
    }
}
