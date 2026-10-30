use crate::transport::{Mode, Transport, SKIP_MS};
use eframe::egui;

pub fn draw_controls(ui: &mut egui::Ui, transport: &mut Transport, total_ms: u64) {
    ui.horizontal(|ui| {
        mode_button(ui, "⏪", Mode::Rewind, transport);
        if ui.button("◀ 10s").clicked() {
            transport.skip_ms(-(SKIP_MS as i64));
        }
        mode_button(ui, "■", Mode::Stopped, transport);
        mode_button(ui, "▶", Mode::Play, transport);
        if ui.button("10s ▶").clicked() {
            transport.skip_ms(SKIP_MS as i64);
        }
        mode_button(ui, "⏩", Mode::Forward, transport);
    });

    let mut position = transport.position_ms();
    ui.horizontal(|ui| {
        ui.label(format_time(position));
        let response = ui.add(
            egui::Slider::new(&mut position, 0..=total_ms)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        if response.dragged() || response.changed() {
            transport.seek_ms(position as i64);
        }
        ui.label(format_time(total_ms));
    });
}

fn mode_button(ui: &mut egui::Ui, label: &str, mode: Mode, transport: &mut Transport) {
    let response = ui.add(egui::Button::new(label).selected(transport.mode() == mode));
    if response.clicked() {
        transport.set_mode(mode);
    }
}

fn format_time(ms: u64) -> String {
    format!("{:02}:{:02}.{}", ms / 60_000, (ms / 1_000) % 60, (ms / 100) % 10)
}
