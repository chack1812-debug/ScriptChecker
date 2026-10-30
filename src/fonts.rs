use std::sync::Arc;

use eframe::egui;

pub fn install_japanese_font(ctx: &egui::Context) -> Result<(), String> {
    let candidates = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"];
    let (name, bytes) = candidates
        .iter()
        .find_map(|name| {
            std::fs::read(format!(r"C:\Windows\Fonts\{name}"))
                .ok()
                .map(|bytes| (*name, bytes))
        })
        .ok_or_else(|| format!("日本語フォントが見つかりません: {}", candidates.join(", ")))?;
    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        name.to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .insert(0, name.to_owned());
    }
    ctx.set_fonts(definitions);
    Ok(())
}
