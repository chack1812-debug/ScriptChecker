use crate::transport::{Mode, SKIP_MS, Transport};
use eframe::egui;

const MIN_SLIDER_WIDTH: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlState {
    pub position_ms: u64,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    SetMode(Mode),
    Skip(i64),
    Seek(u64),
}

pub fn apply_command(transport: &mut Transport, command: ControlCommand) {
    match command {
        ControlCommand::SetMode(mode) => transport.set_mode(mode),
        ControlCommand::Skip(delta_ms) => transport.skip_ms(delta_ms),
        ControlCommand::Seek(position_ms) => transport.seek_ms(position_ms as i64),
    }
}

pub fn draw_controls(
    ui: &mut egui::Ui,
    state: ControlState,
    total_ms: u64,
    playback_enabled: bool,
) -> Option<ControlCommand> {
    let mut command = None;
    ui.add_enabled_ui(playback_enabled, |ui| {
        ui.horizontal(|ui| {
            command = mode_button(ui, "⏪", Mode::Rewind, state.mode);
            if ui.button("◀ 10s").clicked() {
                command = Some(ControlCommand::Skip(-(SKIP_MS as i64)));
            }
            if let Some(next) = mode_button(ui, "■", Mode::Stopped, state.mode) {
                command = Some(next);
            }
            if let Some(next) = mode_button(ui, "▶", Mode::Play, state.mode) {
                command = Some(next);
            }
            if ui.button("10s ▶").clicked() {
                command = Some(ControlCommand::Skip(SKIP_MS as i64));
            }
            if let Some(next) = mode_button(ui, "⏩", Mode::Forward, state.mode) {
                command = Some(next);
            }
        });

        let (seek_command, _) = draw_seek_row(ui, state.position_ms, total_ms);
        command = command.or(seek_command);
    });
    command
}

fn draw_seek_row(
    ui: &mut egui::Ui,
    initial_position: u64,
    total_ms: u64,
) -> (Option<ControlCommand>, f32) {
    let mut position = initial_position;
    let left = format_time(position);
    let right = format_time(total_ms);
    let mut command = None;
    let mut drawn_width = 0.0;
    ui.horizontal(|ui| {
        let available = ui.available_width();
        let (left_width, right_width) = ui.fonts(|fonts| {
            let font = egui::FontId::proportional(egui::TextStyle::Body.resolve(ui.style()).size);
            (
                fonts
                    .layout_no_wrap(left.clone(), font.clone(), ui.visuals().text_color())
                    .rect
                    .width(),
                fonts
                    .layout_no_wrap(right.clone(), font, ui.visuals().text_color())
                    .rect
                    .width(),
            )
        });
        let width = slider_width(
            available,
            left_width,
            right_width,
            ui.spacing().item_spacing.x,
        );
        ui.label(left);
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = width;
            let response = ui.add(
                egui::Slider::new(&mut position, 0..=total_ms)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            drawn_width = response.rect.width();
            if response.dragged() || response.changed() {
                command = Some(ControlCommand::Seek(position));
            }
        });
        ui.label(right);
    });
    (command, drawn_width)
}

pub fn slider_width(
    available_width: f32,
    left_label_width: f32,
    right_label_width: f32,
    item_spacing: f32,
) -> f32 {
    (available_width - left_label_width - right_label_width - item_spacing * 2.0)
        .max(MIN_SLIDER_WIDTH)
        .min(available_width)
}

fn mode_button(
    ui: &mut egui::Ui,
    label: &str,
    mode: Mode,
    current: Mode,
) -> Option<ControlCommand> {
    ui.add(egui::Button::new(label).selected(current == mode))
        .clicked()
        .then_some(ControlCommand::SetMode(mode))
}

fn format_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}.{}",
        ms / 60_000,
        (ms / 1_000) % 60,
        (ms / 100) % 10
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_uses_remaining_width_with_a_small_window_clamp() {
        assert_eq!(slider_width(500.0, 50.0, 50.0, 8.0), 384.0);
        assert_eq!(slider_width(110.0, 50.0, 50.0, 8.0), 24.0);
        assert_eq!(slider_width(12.0, 50.0, 50.0, 8.0), 12.0);
    }

    #[test]
    fn applies_control_commands_to_transport_after_drawing() {
        let mut transport = Transport::new(60_000, 1_000);
        apply_command(&mut transport, ControlCommand::SetMode(Mode::Play));
        assert_eq!(transport.mode(), Mode::Play);
        apply_command(&mut transport, ControlCommand::Seek(12_345));
        assert_eq!(transport.position_ms(), 12_345);
        apply_command(&mut transport, ControlCommand::Skip(-(SKIP_MS as i64)));
        assert_eq!(transport.position_ms(), 2_345);
    }

    #[test]
    fn headless_slider_uses_configured_width_at_each_window_size() {
        for width in [1000.0, 600.0, 300.0, 200.0, 140.0] {
            let ctx = egui::Context::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 120.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_width();
                        let left = format_time(0);
                        let right = format_time(60_000);
                        let (left_width, right_width) = ui.fonts(|fonts| {
                            let font = egui::FontId::proportional(
                                egui::TextStyle::Body.resolve(ui.style()).size,
                            );
                            (
                                fonts
                                    .layout_no_wrap(left, font.clone(), ui.visuals().text_color())
                                    .rect
                                    .width(),
                                fonts
                                    .layout_no_wrap(right, font, ui.visuals().text_color())
                                    .rect
                                    .width(),
                            )
                        });
                        let expected = slider_width(
                            available,
                            left_width,
                            right_width,
                            ui.spacing().item_spacing.x,
                        );
                        let (_, actual) = draw_seek_row(ui, 0, 60_000);
                        assert!(
                            (actual - expected).abs() < 0.01,
                            "width={width}: {actual} != {expected}"
                        );
                    });
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }
}
