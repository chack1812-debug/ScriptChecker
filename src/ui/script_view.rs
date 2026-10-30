use std::ops::Range;

use eframe::egui::{self, Color32, RichText};

use crate::{
    charmap::{char_index_at, char_spans},
    script::{SceneHead, ScriptItem, ScriptLine, Seg},
    timeline::{current_line, Timeline},
};

#[derive(Clone, Copy)]
pub enum LineState {
    Past,
    Current { char: Option<usize> },
    Future,
}

pub fn wrap_tokens(widths: &[f32], max_width: f32) -> Vec<Range<usize>> {
    if widths.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut width = 0.0;
    for (index, token_width) in widths.iter().enumerate() {
        if index > start && width + token_width > max_width {
            lines.push(start..index);
            start = index;
            width = 0.0;
        }
        width += token_width;
    }
    lines.push(start..widths.len());
    lines
}

pub fn draw_line(
    ui: &mut egui::Ui,
    line: &ScriptLine,
    state: LineState,
    show_ruby: bool,
) -> egui::Response {
    let mut response = ui.allocate_response(egui::Vec2::ZERO, egui::Sense::click());
    let frame = matches!(state, LineState::Current { .. }).then(|| {
        egui::Frame::new().fill(Color32::from_rgb(50, 75, 105))
    });
    let draw = |ui: &mut egui::Ui, response: &mut egui::Response| {
        ui.horizontal_wrapped(|ui| {
            let speaker = ui.add_sized(
                [90.0, 0.0],
                egui::Label::new(RichText::new(format!("{}:", line.cast)).strong()),
            );
            *response = response.union(speaker);
            let mut index = 0;
            for seg in &line.segs {
                match seg {
                    Seg::Plain(text) => {
                        for character in text.chars() {
                            let part = ui.add(egui::Label::new(character_text(
                                character.to_string(),
                                state,
                                index,
                            )));
                            *response = response.union(part);
                            index += 1;
                        }
                    }
                    Seg::Ruby { base, reading } => {
                        let base_count = base.chars().count();
                        let highlighted = matches!(state, LineState::Current { char: Some(current) } if (index..index + base_count).contains(&current));
                        let ruby = ui.vertical(|ui| {
                            if show_ruby {
                                ui.label(RichText::new(reading).size(8.0));
                            }
                            ui.horizontal(|ui| {
                                for character in base.chars() {
                                    let part = ui.add(egui::Label::new(character_text(
                                        character.to_string(),
                                        if highlighted {
                                            LineState::Current { char: Some(index) }
                                        } else {
                                            state
                                        },
                                        index,
                                    )));
                                    *response = response.union(part);
                                    index += 1;
                                }
                            });
                        });
                        *response = response.union(ruby.response);
                    }
                }
            }
        });
    };
    if let Some(frame) = frame {
        frame.show(ui, |ui| draw(ui, &mut response));
    } else {
        draw(ui, &mut response);
    }
    response
}

pub fn draw_script(
    ui: &mut egui::Ui,
    items: &[ScriptItem],
    timeline: &Timeline,
    pos_ms: u64,
    follow: bool,
    show_ruby: bool,
) -> Option<u64> {
    let current = current_line(timeline, pos_ms);
    let id = ui.id().with("script-current-line");
    let changed = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<Option<usize>>(id);
        data.insert_temp(id, current);
        previous != Some(current)
    });
    let mut seek_to = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for item in items {
            match item {
                ScriptItem::Scene(SceneHead { id, name }) => {
                    ui.separator();
                    ui.heading(format!("{id} {name}"));
                }
                ScriptItem::Line(line) => {
                    let timeline_index = timeline
                        .lines
                        .iter()
                        .position(|entry| entry.id == line.id && !entry.missing);
                    let state = match (timeline_index, current) {
                        (Some(index), Some(active)) if index == active => {
                            let active_line = &timeline.lines[index];
                            let spans = char_spans(&line.segs, active_line.start_ms, active_line.end_ms);
                            LineState::Current {
                                char: char_index_at(&spans, pos_ms),
                            }
                        }
                        (Some(index), Some(active)) if index < active => LineState::Past,
                        _ => LineState::Future,
                    };
                    let response = draw_line(ui, line, state, show_ruby);
                    if response.clicked()
                        && let Some(index) = timeline_index
                    {
                        seek_to = Some(timeline.lines[index].start_ms);
                    }
                    if follow && changed && matches!(state, LineState::Current { .. }) {
                        ui.scroll_to_rect(response.rect, None);
                    }
                }
            }
        }
    });
    seek_to
}

fn character_text(text: String, state: LineState, index: usize) -> RichText {
    match state {
        LineState::Past => RichText::new(text).color(Color32::DARK_GRAY),
        LineState::Future => RichText::new(text),
        LineState::Current { char: Some(current) } if index < current => {
            RichText::new(text).color(Color32::WHITE)
        }
        LineState::Current { char: Some(current) } if index == current => RichText::new(text)
            .color(Color32::WHITE)
            .underline()
            .background_color(Color32::from_rgb(100, 140, 190)),
        LineState::Current { .. } => RichText::new(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_tokens_without_stalling_on_wide_tokens() {
        assert_eq!(wrap_tokens(&[10.0; 7], 30.0), vec![0..3, 3..6, 6..7]);
        assert_eq!(wrap_tokens(&[40.0, 10.0], 30.0), vec![0..1, 1..2]);
    }
}
