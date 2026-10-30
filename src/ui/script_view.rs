use std::{collections::HashMap, ops::Range};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

use crate::{
    charmap::{char_index_at, char_spans},
    script::{SceneHead, ScriptItem, ScriptLine, Seg},
    timeline::{Timeline, current_line},
};

const SPEAKER_WIDTH: f32 = 90.0;
const RUBY_SCALE: f32 = 0.55;
const KINSOKU_LINE_START: &str = "、。，．）」』】!?！？";

#[derive(Clone, Copy)]
pub enum LineState {
    Past,
    Current { char: Option<usize> },
    Future,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapToken {
    width: f32,
    prohibit_line_start: bool,
}

impl WrapToken {
    pub fn plain(width: f32) -> Self {
        Self {
            width,
            prohibit_line_start: false,
        }
    }

    pub fn prohibit_line_start(width: f32) -> Self {
        Self {
            width,
            prohibit_line_start: true,
        }
    }
}

pub fn wrap_tokens(tokens: &[WrapToken], max_width: f32) -> Vec<Range<usize>> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut width = 0.0;
    for (index, token) in tokens.iter().enumerate() {
        if index > start && width + token.width > max_width && !token.prohibit_line_start {
            lines.push(start..index);
            start = index;
            width = 0.0;
        }
        width += token.width;
    }
    lines.push(start..tokens.len());
    lines
}

#[derive(Clone)]
struct ScriptToken {
    base: String,
    ruby: Option<String>,
    width: f32,
    ruby_width: f32,
    char_start: usize,
    char_count: usize,
    prohibit_line_start: bool,
}

#[derive(Clone)]
struct LineLayout {
    tokens: Vec<ScriptToken>,
    rows: Vec<Range<usize>>,
    height: f32,
    base_size: f32,
    ruby_size: f32,
    ruby_top: f32,
    row_height: f32,
}

#[derive(Clone, Default)]
struct LayoutCache {
    entries: HashMap<(String, u32, bool), LineLayout>,
}

pub fn draw_line(
    ui: &mut egui::Ui,
    line: &ScriptLine,
    state: LineState,
    show_ruby: bool,
) -> egui::Response {
    let layout = build_layout(ui, line, content_width(ui), show_ruby);
    paint_line(ui, line, &layout, state)
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
    let current_id = ui.id().with("script-current-line");
    let follow_id = ui.id().with("script-follow");
    let (changed, follow_started) = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<Option<usize>>(current_id);
        data.insert_temp(current_id, current);
        let previous_follow = data.get_temp::<bool>(follow_id).unwrap_or(false);
        data.insert_temp(follow_id, follow);
        (previous != Some(current), follow && !previous_follow)
    });
    let cache_id = ui.id().with("script-layout-cache");
    let mut cache: LayoutCache = ui
        .ctx()
        .data_mut(|data| data.get_temp(cache_id).unwrap_or_default());
    let mut seek_to = None;
    egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
        for item in items {
            match item {
                ScriptItem::Scene(SceneHead { id, name }) => {
                    ui.separator();
                    ui.heading(format!("{id} {name}"));
                }
                ScriptItem::Line(line) => {
                    let key = (line.id.clone(), content_width(ui).to_bits(), show_ruby);
                    let layout = cache
                        .entries
                        .entry(key)
                        .or_insert_with(|| build_layout(ui, line, content_width(ui), show_ruby))
                        .clone();
                    let timeline_index = timeline
                        .lines
                        .iter()
                        .position(|entry| entry.id == line.id && !entry.missing);
                    let state = line_state(line, timeline, timeline_index, current, pos_ms);
                    let next_rect = Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(ui.available_width(), layout.height),
                    );
                    let response = if next_rect.intersects(viewport) {
                        paint_line(ui, line, &layout, state)
                    } else {
                        ui.allocate_rect(next_rect, Sense::click())
                    };
                    if response.clicked()
                        && let Some(index) = timeline_index
                    {
                        seek_to = Some(timeline.lines[index].start_ms);
                    }
                    if follow
                        && (changed || follow_started)
                        && matches!(state, LineState::Current { .. })
                    {
                        ui.scroll_to_rect(response.rect, None);
                    }
                }
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(cache_id, cache));
    seek_to
}

fn line_state(
    line: &ScriptLine,
    timeline: &Timeline,
    timeline_index: Option<usize>,
    current: Option<usize>,
    pos_ms: u64,
) -> LineState {
    match (timeline_index, current) {
        (Some(index), Some(active)) if index == active => {
            let active_line = &timeline.lines[index];
            let spans = char_spans(&line.segs, active_line.start_ms, active_line.end_ms);
            LineState::Current {
                char: char_index_at(&spans, pos_ms),
            }
        }
        (Some(index), Some(active)) if index < active => LineState::Past,
        _ => LineState::Future,
    }
}

fn content_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - SPEAKER_WIDTH).max(1.0)
}

fn build_layout(ui: &egui::Ui, line: &ScriptLine, max_width: f32, show_ruby: bool) -> LineLayout {
    let base_size = egui::TextStyle::Body.resolve(ui.style()).size;
    let ruby_size = base_size * RUBY_SCALE;
    let base_font = FontId::proportional(base_size);
    let ruby_font = FontId::proportional(ruby_size);
    let mut tokens = Vec::new();
    let mut char_start = 0;
    for seg in &line.segs {
        match seg {
            Seg::Plain(text) => {
                for character in text.chars() {
                    let base = character.to_string();
                    tokens.push(ScriptToken {
                        width: text_width(ui, &base, &base_font),
                        prohibit_line_start: KINSOKU_LINE_START.contains(character),
                        base,
                        ruby: None,
                        ruby_width: 0.0,
                        char_start,
                        char_count: 1,
                    });
                    char_start += 1;
                }
            }
            Seg::Ruby { base, reading } => {
                let base_width = text_width(ui, base, &base_font);
                let ruby_width = if show_ruby {
                    text_width(ui, reading, &ruby_font)
                } else {
                    0.0
                };
                let char_count = base.chars().count();
                tokens.push(ScriptToken {
                    base: base.clone(),
                    ruby: show_ruby.then(|| reading.clone()),
                    width: base_width.max(ruby_width),
                    ruby_width,
                    char_start,
                    char_count,
                    prohibit_line_start: false,
                });
                char_start += char_count;
            }
        }
    }
    let wrap_specs = tokens
        .iter()
        .map(|token| WrapToken {
            width: token.width,
            prohibit_line_start: token.prohibit_line_start,
        })
        .collect::<Vec<_>>();
    let rows = wrap_tokens(&wrap_specs, max_width);
    let ruby_top = if show_ruby && tokens.iter().any(|token| token.ruby.is_some()) {
        ruby_size + 2.0
    } else {
        0.0
    };
    let row_height = ruby_top + base_size * 1.35;
    LineLayout {
        height: row_height * rows.len() as f32,
        tokens,
        rows,
        base_size,
        ruby_size,
        ruby_top,
        row_height,
    }
}

fn text_width(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .rect
        .width()
}

fn paint_line(
    ui: &mut egui::Ui,
    line: &ScriptLine,
    layout: &LineLayout,
    state: LineState,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), layout.height),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);
    if matches!(state, LineState::Current { .. }) {
        painter.rect_filled(rect, 0.0, Color32::from_rgb(50, 75, 105));
    }
    painter.text(
        Pos2::new(rect.left(), rect.top() + layout.ruby_top),
        Align2::LEFT_TOP,
        format!("{}:", line.cast),
        FontId::proportional(layout.base_size),
        Color32::WHITE,
    );
    for (row_index, range) in layout.rows.iter().enumerate() {
        let mut x = rect.left() + SPEAKER_WIDTH;
        let base_y = rect.top() + row_index as f32 * layout.row_height + layout.ruby_top;
        for token in &layout.tokens[range.clone()] {
            let color = token_color(state, token);
            let base_rect = Rect::from_min_size(
                Pos2::new(x, base_y),
                egui::vec2(token.width, layout.base_size * 1.35),
            );
            if is_current_token(state, token) {
                painter.rect_filled(base_rect, 0.0, Color32::from_rgb(100, 140, 190));
                painter.line_segment(
                    [
                        Pos2::new(base_rect.left(), base_rect.bottom()),
                        Pos2::new(base_rect.right(), base_rect.bottom()),
                    ],
                    Stroke::new(1.0, Color32::WHITE),
                );
            }
            if let Some(ruby) = &token.ruby {
                painter.text(
                    Pos2::new(
                        x + (token.width - token.ruby_width) * 0.5,
                        base_y - layout.ruby_top,
                    ),
                    Align2::LEFT_TOP,
                    ruby,
                    FontId::proportional(layout.ruby_size),
                    color,
                );
            }
            painter.text(
                Pos2::new(x, base_y),
                Align2::LEFT_TOP,
                &token.base,
                FontId::proportional(layout.base_size),
                color,
            );
            x += token.width;
        }
    }
    response
}

fn is_current_token(state: LineState, token: &ScriptToken) -> bool {
    matches!(state, LineState::Current { char: Some(current) }
        if (token.char_start..token.char_start + token.char_count).contains(&current))
}

fn token_color(state: LineState, token: &ScriptToken) -> Color32 {
    match state {
        LineState::Past => Color32::DARK_GRAY,
        LineState::Current {
            char: Some(current),
        } if token.char_start < current => Color32::WHITE,
        _ => Color32::WHITE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_tokens_without_stalling_on_wide_tokens() {
        assert_eq!(
            wrap_tokens(&[WrapToken::plain(10.0); 7], 30.0),
            vec![0..3, 3..6, 6..7]
        );
        assert_eq!(
            wrap_tokens(&[WrapToken::plain(40.0), WrapToken::plain(10.0)], 30.0),
            vec![0..1, 1..2]
        );
    }

    #[test]
    fn keeps_prohibited_punctuation_off_the_start_of_a_wrapped_line() {
        let tokens = [
            WrapToken::plain(20.0),
            WrapToken::plain(20.0),
            WrapToken::prohibit_line_start(10.0),
            WrapToken::plain(20.0),
        ];
        assert_eq!(wrap_tokens(&tokens, 40.0), vec![0..3, 3..4]);
    }
}
