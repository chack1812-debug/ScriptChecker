use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelectionPoint {
    pub line: usize,
    pub character: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: SelectionPoint,
    pub end: SelectionPoint,
}

impl SelectionRange {
    pub fn new(first: SelectionPoint, second: SelectionPoint) -> Self {
        if first <= second {
            Self {
                start: first,
                end: second,
            }
        } else {
            Self {
                start: second,
                end: first,
            }
        }
    }

    fn is_empty(self) -> bool {
        self.start == self.end
    }
}

pub fn selection_text(lines: &[ScriptLine], selection: SelectionRange) -> String {
    selection_text_refs(&lines.iter().collect::<Vec<_>>(), selection)
}

fn selection_text_refs(lines: &[&ScriptLine], selection: SelectionRange) -> String {
    if selection.is_empty() {
        return String::new();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(line_index, _)| (selection.start.line..=selection.end.line).contains(line_index))
        .map(|(line_index, line)| selected_line_text(line, line_index, selection))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Default)]
pub struct ScriptViewState {
    selection: Option<SelectionRange>,
    drag_start: Option<(SelectionPoint, Pos2)>,
    pressed_timeline: Option<Option<usize>>,
}

impl ScriptViewState {
    pub fn selection(&self) -> Option<SelectionRange> {
        self.selection
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.drag_start = None;
        self.pressed_timeline = None;
    }
}

fn selected_line_text(line: &ScriptLine, line_index: usize, selection: SelectionRange) -> String {
    let first = if line_index == selection.start.line {
        selection.start.character
    } else {
        0
    };
    let last = if line_index == selection.end.line {
        selection.end.character
    } else {
        usize::MAX
    };
    let mut text = String::new();
    let mut character = 0;
    for segment in &line.segs {
        match segment {
            Seg::Plain(plain) => {
                for value in plain.chars() {
                    if (first..=last).contains(&character) {
                        text.push(value);
                    }
                    character += 1;
                }
            }
            Seg::Ruby { base, reading } => {
                let count = base.chars().count();
                if character <= last && character + count > first {
                    text.push('\'');
                    text.push_str(base);
                    text.push('|');
                    text.push_str(reading);
                    text.push('\'');
                }
                character += count;
            }
        }
    }
    if text.is_empty() {
        String::new()
    } else {
        format!("[{}] {}: {text}", line.id, line.cast)
    }
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
    base_width: f32,
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
    width_bits: u32,
    entries: HashMap<(String, bool), Arc<LineLayout>>,
}

pub fn draw_line(
    ui: &mut egui::Ui,
    line: &ScriptLine,
    state: LineState,
    show_ruby: bool,
) -> egui::Response {
    let layout = build_layout(ui, line, content_width(ui), show_ruby);
    paint_line(ui, line, &layout, state, None, 0).0
}

pub fn draw_script(
    ui: &mut egui::Ui,
    items: &[ScriptItem],
    timeline: &Timeline,
    pos_ms: u64,
    follow: bool,
    show_ruby: bool,
    view_state: &mut ScriptViewState,
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
    let cache = ui.ctx().data_mut(|data| {
        data.get_temp::<Arc<Mutex<LayoutCache>>>(cache_id)
            .unwrap_or_else(|| {
                let cache = Arc::new(Mutex::new(LayoutCache::default()));
                data.insert_temp(cache_id, Arc::clone(&cache));
                cache
            })
    });
    let mut seek_to = None;
    let mut hit_lines = Vec::new();
    let mut line_index = 0;
    egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
        let viewport = viewport.translate(ui.min_rect().min.to_vec2());
        for item in items {
            match item {
                ScriptItem::Scene(SceneHead { id, name }) => {
                    ui.separator();
                    ui.heading(format!("{id} {name}"));
                }
                ScriptItem::Line(line) => {
                    let this_line_index = line_index;
                    line_index += 1;
                    let width = content_width(ui);
                    let layout = {
                        let mut cache = cache.lock().expect("script layout cache poisoned");
                        if cache.width_bits != width.to_bits() {
                            cache.width_bits = width.to_bits();
                            cache.entries.clear();
                        }
                        Arc::clone(
                            cache
                                .entries
                                .entry((format!("{}:{:?}", line.id, line.segs), show_ruby))
                                .or_insert_with(|| {
                                    Arc::new(build_layout(ui, line, width, show_ruby))
                                }),
                        )
                    };
                    let timeline_index = timeline
                        .lines
                        .iter()
                        .position(|entry| entry.id == line.id && !entry.missing);
                    let state = line_state(line, timeline, timeline_index, current, pos_ms);
                    let next_rect = Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(ui.available_width(), layout.height),
                    );
                    let (response, hits) = if next_rect.intersects(viewport) {
                        let (response, hits) = paint_line(
                            ui,
                            line,
                            &layout,
                            state,
                            view_state.selection,
                            this_line_index,
                        );
                        (response, hits)
                    } else {
                        (
                            ui.allocate_rect(next_rect, Sense::click_and_drag()),
                            Vec::new(),
                        )
                    };
                    hit_lines.push(HitLine {
                        line: this_line_index,
                        timeline_index,
                        rect: response.rect,
                        hits,
                    });
                    response.context_menu(|ui| {
                        if ui.button("コピー").clicked() {
                            copy_selection(ui.ctx(), items, view_state.selection);
                            ui.close_menu();
                        }
                        if ui.button("この行をコピー").clicked() {
                            let all = SelectionRange::new(
                                SelectionPoint {
                                    line: 0,
                                    character: 0,
                                },
                                SelectionPoint {
                                    line: 0,
                                    character: usize::MAX,
                                },
                            );
                            let lines = [line];
                            let text = selection_text_refs(&lines, all);
                            if !text.is_empty() {
                                ui.ctx().copy_text(text);
                            }
                            ui.close_menu();
                        }
                    });
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
    if ui
        .ctx()
        .input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        && let Some(hit) = line_at_pointer(ui.ctx(), &hit_lines)
    {
        view_state.selection = None;
        view_state.drag_start = None;
        view_state.pressed_timeline = Some(hit.timeline_index);
        if let Some((point, pointer)) = pointer_hit(ui.ctx(), &hit_lines) {
            view_state.drag_start = Some((point, pointer));
        }
    }
    if let Some((start, origin)) = view_state.drag_start
        && let Some((end, pointer)) = pointer_hit(ui.ctx(), &hit_lines)
        && pointer.distance(origin) >= 3.0
    {
        view_state.selection = Some(SelectionRange::new(start, end));
    }
    if ui
        .ctx()
        .input(|input| input.pointer.button_released(egui::PointerButton::Primary))
    {
        view_state.drag_start = None;
        if view_state.selection.is_none()
            && let Some(Some(index)) = view_state.pressed_timeline.take()
        {
            seek_to = Some(timeline.lines[index].start_ms);
        }
    }
    if ui
        .ctx()
        .input(|input| input.modifiers.command && input.key_pressed(egui::Key::C))
    {
        copy_selection(ui.ctx(), items, view_state.selection);
    }
    seek_to
}

struct HitToken {
    rect: Rect,
    character: usize,
}

struct HitLine {
    line: usize,
    timeline_index: Option<usize>,
    rect: Rect,
    hits: Vec<HitToken>,
}

fn line_at_pointer<'a>(ctx: &egui::Context, hit_lines: &'a [HitLine]) -> Option<&'a HitLine> {
    let pointer = ctx.input(|input| input.pointer.interact_pos())?;
    hit_lines.iter().find(|line| line.rect.contains(pointer))
}

fn pointer_hit(ctx: &egui::Context, hit_lines: &[HitLine]) -> Option<(SelectionPoint, Pos2)> {
    let pointer = ctx.input(|input| input.pointer.interact_pos())?;
    selection_point_at(hit_lines, pointer).map(|point| (point, pointer))
}

fn selection_point_at(hit_lines: &[HitLine], pointer: Pos2) -> Option<SelectionPoint> {
    hit_lines.iter().find_map(|line| {
        line.hits.iter().find_map(|hit| {
            hit.rect.contains(pointer).then_some(SelectionPoint {
                line: line.line,
                character: hit.character,
            })
        })
    })
}

fn copy_selection(ctx: &egui::Context, items: &[ScriptItem], selection: Option<SelectionRange>) {
    let Some(selection) = selection else {
        return;
    };
    let lines = items
        .iter()
        .filter_map(|item| match item {
            ScriptItem::Line(line) => Some(line),
            ScriptItem::Scene(_) => None,
        })
        .collect::<Vec<_>>();
    let text = selection_text_refs(&lines, selection);
    if !text.is_empty() {
        ctx.copy_text(text);
    }
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
                    let base_width = text_width(ui, &base, &base_font);
                    tokens.push(ScriptToken {
                        width: base_width,
                        prohibit_line_start: KINSOKU_LINE_START.contains(character),
                        base,
                        ruby: None,
                        ruby_width: 0.0,
                        base_width,
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
                    base_width,
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
    selection: Option<SelectionRange>,
    line_index: usize,
) -> (egui::Response, Vec<HitToken>) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), layout.height),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    if matches!(state, LineState::Current { .. }) {
        painter.rect_filled(rect, 0.0, current_background(ui.visuals()));
    }
    painter.text(
        Pos2::new(rect.left(), rect.top() + layout.ruby_top),
        Align2::LEFT_TOP,
        format!("{}:", line.cast),
        FontId::proportional(layout.base_size),
        ui.visuals().strong_text_color(),
    );
    let mut hits = Vec::new();
    for (row_index, range) in layout.rows.iter().enumerate() {
        let mut x = rect.left() + SPEAKER_WIDTH;
        let base_y = rect.top() + row_index as f32 * layout.row_height + layout.ruby_top;
        for token in &layout.tokens[range.clone()] {
            let color = token_color(ui.visuals(), state, token);
            let base_rect = Rect::from_min_size(
                Pos2::new(x, base_y),
                egui::vec2(token.width, layout.base_size * 1.35),
            );
            if token_is_selected(selection, line_index, token) {
                painter.rect_filled(base_rect, 0.0, selection_background(ui.visuals()));
            }
            if is_current_token(state, token) {
                painter.rect_filled(base_rect, 0.0, current_token_background(ui.visuals()));
                painter.line_segment(
                    [
                        Pos2::new(base_rect.left(), base_rect.bottom()),
                        Pos2::new(base_rect.right(), base_rect.bottom()),
                    ],
                    Stroke::new(1.0, current_text_color(ui.visuals())),
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
                Pos2::new(x + (token.width - token.base_width) * 0.5, base_y),
                Align2::LEFT_TOP,
                &token.base,
                FontId::proportional(layout.base_size),
                color,
            );
            hits.push(HitToken {
                rect: base_rect,
                character: token.char_start,
            });
            x += token.width;
        }
    }
    (response, hits)
}

fn token_is_selected(
    selection: Option<SelectionRange>,
    line_index: usize,
    token: &ScriptToken,
) -> bool {
    let Some(selection) = selection else {
        return false;
    };
    if !(selection.start.line..=selection.end.line).contains(&line_index) {
        return false;
    }
    let first = if line_index == selection.start.line {
        selection.start.character
    } else {
        0
    };
    let last = if line_index == selection.end.line {
        selection.end.character
    } else {
        usize::MAX
    };
    token.char_start <= last && token.char_start + token.char_count > first
}

fn is_current_token(state: LineState, token: &ScriptToken) -> bool {
    matches!(state, LineState::Current { char: Some(current) }
        if (token.char_start..token.char_start + token.char_count).contains(&current))
}

fn token_color(visuals: &egui::Visuals, state: LineState, token: &ScriptToken) -> Color32 {
    match state {
        LineState::Past => visuals.weak_text_color(),
        LineState::Current {
            char: Some(current),
        } if token.char_start < current => current_text_color(visuals),
        _ => visuals.text_color(),
    }
}

fn luminance(color: Color32) -> f32 {
    let linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn current_background(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(50, 75, 105)
    } else {
        Color32::from_rgb(185, 215, 245)
    }
}
fn current_token_background(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(100, 140, 190)
    } else {
        Color32::from_rgb(50, 85, 125)
    }
}
fn selection_background(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(90, 70, 30)
    } else {
        Color32::from_rgb(255, 230, 145)
    }
}
fn current_text_color(visuals: &egui::Visuals) -> Color32 {
    let bg = current_token_background(visuals);
    if contrast_ratio(Color32::WHITE, bg) >= contrast_ratio(Color32::BLACK, bg) {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_lines() -> Vec<ScriptLine> {
        vec![
            ScriptLine {
                id: "S01-L001".to_owned(),
                scene: "S01".to_owned(),
                cast: "蒼真".to_owned(),
                segs: vec![
                    Seg::Plain("前".to_owned()),
                    Seg::Ruby {
                        base: "東京".to_owned(),
                        reading: "とうきょう".to_owned(),
                    },
                    Seg::Plain("後".to_owned()),
                ],
            },
            ScriptLine {
                id: "S01-L002".to_owned(),
                scene: "S01".to_owned(),
                cast: "大和".to_owned(),
                segs: vec![Seg::Plain("次の行".to_owned())],
            },
        ]
    }

    #[test]
    fn copies_ruby_whole_word_across_lines_in_drag_direction() {
        let selection = SelectionRange::new(
            SelectionPoint {
                line: 1,
                character: 2,
            },
            SelectionPoint {
                line: 0,
                character: 2,
            },
        );
        assert_eq!(
            selection_text(&selection_lines(), selection),
            "[S01-L001] 蒼真: '東京|とうきょう'後\n[S01-L002] 大和: 次の行"
        );
    }

    #[test]
    fn empty_selection_has_no_copyable_text() {
        let point = SelectionPoint {
            line: 0,
            character: 1,
        };
        assert_eq!(
            selection_text(&selection_lines(), SelectionRange::new(point, point)),
            ""
        );
    }

    #[test]
    fn maps_drag_coordinates_to_the_character_token() {
        let hits = [HitLine {
            line: 3,
            timeline_index: None,
            rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(20.0, 10.0)),
            hits: vec![
                HitToken {
                    rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(10.0, 10.0)),
                    character: 0,
                },
                HitToken {
                    rect: Rect::from_min_size(Pos2::new(10.0, 0.0), egui::vec2(10.0, 10.0)),
                    character: 1,
                },
            ],
        }];
        assert_eq!(
            selection_point_at(&hits, Pos2::new(15.0, 5.0)),
            Some(SelectionPoint {
                line: 3,
                character: 1,
            })
        );
        assert_eq!(selection_point_at(&hits, Pos2::new(21.0, 5.0)), None);
    }

    fn selection_timeline() -> Timeline {
        Timeline {
            script_sha256: String::new(),
            duration_ms: 1_000,
            sample_rate: 48_000,
            lines: vec![crate::timeline::TimelineLine {
                id: "S01-L001".to_owned(),
                start_ms: 0,
                end_ms: 1_000,
                text: "abcdef".to_owned(),
                missing: false,
            }],
        }
    }

    fn selection_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 120.0))),
            events,
            ..Default::default()
        }
    }

    fn visual_items(line_count: usize) -> Vec<ScriptItem> {
        (0..line_count)
            .map(|index| {
                ScriptItem::Line(ScriptLine {
                    id: format!("S01-L{:03}", index + 1),
                    scene: "S01".to_owned(),
                    cast: "蒼真".to_owned(),
                    segs: vec![Seg::Plain(format!("描画テキスト{index}"))],
                })
            })
            .collect()
    }

    fn visual_timeline(items: &[ScriptItem]) -> Timeline {
        Timeline {
            script_sha256: String::new(),
            duration_ms: items.len() as u64 * 1_000,
            sample_rate: 48_000,
            lines: items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    ScriptItem::Line(line) => Some(crate::timeline::TimelineLine {
                        id: line.id.clone(),
                        start_ms: index as u64 * 1_000,
                        end_ms: index as u64 * 1_000 + 999,
                        text: crate::script::display_text(&line.segs),
                        missing: false,
                    }),
                    ScriptItem::Scene(_) => None,
                })
                .collect(),
        }
    }

    fn text_shape_count(shapes: &[egui::epaint::ClippedShape]) -> usize {
        fn count(shape: &egui::epaint::Shape) -> usize {
            match shape {
                egui::epaint::Shape::Text(_) => 1,
                egui::epaint::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
                _ => 0,
            }
        }
        shapes.iter().map(|clipped| count(&clipped.shape)).sum()
    }

    fn drawn_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
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

    fn draw_visual_frame(
        ctx: &egui::Context,
        items: &[ScriptItem],
        timeline: &Timeline,
        pos_ms: u64,
        follow: bool,
        events: Vec<egui::Event>,
        state: &mut ScriptViewState,
    ) -> eframe::egui::FullOutput {
        ctx.run(selection_input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = draw_script(ui, items, timeline, pos_ms, follow, false, state);
            });
        })
    }

    #[test]
    fn headless_script_draws_text_at_start_middle_end_and_follow_position() {
        let ctx = egui::Context::default();
        let items = visual_items(40);
        let timeline = visual_timeline(&items);
        let mut state = ScriptViewState::default();
        let start = draw_visual_frame(&ctx, &items, &timeline, 0, false, vec![], &mut state);
        let middle = draw_visual_frame(
            &ctx,
            &items,
            &timeline,
            0,
            false,
            vec![
                egui::Event::PointerMoved(Pos2::new(160.0, 60.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -320.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut state,
        );
        let end = draw_visual_frame(
            &ctx,
            &items,
            &timeline,
            0,
            false,
            vec![
                egui::Event::PointerMoved(Pos2::new(160.0, 60.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -10_000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut state,
        );
        let followed = draw_visual_frame(&ctx, &items, &timeline, 26_000, true, vec![], &mut state);
        for (name, output) in [
            ("start", start),
            ("middle", middle),
            ("end", end),
            ("follow", followed),
        ] {
            assert!(
                text_shape_count(&output.shapes) > 0,
                "{name} viewport must paint text"
            );
        }
    }

    #[test]
    fn headless_redraw_replaces_cached_text_when_same_line_id_changes() {
        let ctx = egui::Context::default();
        let mut state = ScriptViewState::default();
        let old_items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("旧本文".to_owned())],
        })];
        let new_items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("新本文".to_owned())],
        })];
        let old_timeline = visual_timeline(&old_items);
        let new_timeline = visual_timeline(&new_items);
        let old = draw_visual_frame(
            &ctx,
            &old_items,
            &old_timeline,
            0,
            false,
            vec![],
            &mut state,
        );
        let new = draw_visual_frame(
            &ctx,
            &new_items,
            &new_timeline,
            0,
            false,
            vec![],
            &mut state,
        );
        assert!(drawn_texts(&old.shapes).iter().any(|text| text == "旧"));
        assert!(drawn_texts(&new.shapes).iter().any(|text| text == "新"));
        assert!(!drawn_texts(&new.shapes).iter().any(|text| text == "旧"));
    }

    #[test]
    fn headless_width_changes_keep_layout_cache_bounded() {
        let ctx = egui::Context::default();
        let items = visual_items(3);
        let timeline = visual_timeline(&items);
        let mut state = ScriptViewState::default();
        for width in 200..800 {
            let mut cache_entries = usize::MAX;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(width as f32, 120.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                        let cache_id = ui.id().with("script-layout-cache");
                        cache_entries = ctx.data(|data| {
                            data.get_temp::<Arc<Mutex<LayoutCache>>>(cache_id)
                                .expect("draw must initialize the layout cache")
                                .lock()
                                .expect("script layout cache poisoned")
                                .entries
                                .len()
                        });
                    });
                },
            );
            assert!(text_shape_count(&output.shapes) > 0);
            assert!(
                cache_entries <= 3,
                "width={width}: cache entries={cache_entries}"
            );
        }
    }

    #[test]
    #[ignore]
    fn episode_60_script_draws_text_at_all_scroll_positions_and_follow() {
        let root = std::path::Path::new(r"D:\UDS\YouTube\AI道具箱\第60回");
        let items = crate::script::parse_script(
            &std::fs::read_to_string(root.join("第60回.txt")).expect("episode 60 script exists"),
        );
        let timeline = crate::timeline::parse_timeline(
            &std::fs::read_to_string(root.join("timeline").join("timeline.json"))
                .expect("episode 60 timeline exists"),
        )
        .expect("episode 60 timeline parses");
        let ctx = egui::Context::default();
        let mut state = ScriptViewState::default();
        for output in [
            draw_visual_frame(&ctx, &items, &timeline, 0, false, vec![], &mut state),
            draw_visual_frame(
                &ctx,
                &items,
                &timeline,
                0,
                false,
                vec![
                    egui::Event::PointerMoved(Pos2::new(160.0, 60.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -500.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut state,
            ),
            draw_visual_frame(
                &ctx,
                &items,
                &timeline,
                0,
                false,
                vec![
                    egui::Event::PointerMoved(Pos2::new(160.0, 60.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -10_000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut state,
            ),
            draw_visual_frame(
                &ctx,
                &items,
                &timeline,
                timeline.duration_ms * 2 / 3,
                true,
                vec![],
                &mut state,
            ),
        ] {
            assert!(text_shape_count(&output.shapes) > 0);
        }
    }

    #[test]
    #[ignore]
    fn episode_60_resize_frame_times_do_not_grow_with_cache_entries() {
        use std::time::Instant;

        let root = std::path::Path::new(r"D:\UDS\YouTube\AI道具箱\第60回");
        let items = crate::script::parse_script(
            &std::fs::read_to_string(root.join("第60回.txt")).expect("episode 60 script exists"),
        );
        let timeline = crate::timeline::parse_timeline(
            &std::fs::read_to_string(root.join("timeline").join("timeline.json"))
                .expect("episode 60 timeline exists"),
        )
        .expect("episode 60 timeline parses");
        let ctx = egui::Context::default();
        let mut state = ScriptViewState::default();
        let mut frame_ms = Vec::new();
        for block in 0..6 {
            let started = Instant::now();
            for step in 0..100 {
                let width = 300.0 + (block * 100 + step) as f32;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(width, 240.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let _ = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                        });
                    },
                );
                assert!(text_shape_count(&output.shapes) > 0);
            }
            frame_ms.push(started.elapsed().as_secs_f64() * 10.0);
        }
        println!("episode 60 resize average ms per frame: {frame_ms:?}");
        assert!(
            frame_ms[5] <= frame_ms[0] * 3.0 + 1.0,
            "frame time grew continuously: {frame_ms:?}"
        );
    }

    #[test]
    fn headless_drag_selection_copies_text_with_ctrl_c() {
        let ctx = egui::Context::default();
        let items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("abcdef".to_owned())],
        })];
        let timeline = selection_timeline();
        let mut state = ScriptViewState::default();

        for events in [
            vec![],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(104.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(Pos2::new(124.0, 15.0))],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(124.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = ctx.run(selection_input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                });
            });
        }

        assert!(state.selection().is_some());
        let mut copy_input = selection_input(vec![egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        copy_input.modifiers = egui::Modifiers::COMMAND;
        let output = ctx.run(copy_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
            });
        });
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .expect("Ctrl+C must emit a copy command");
        assert_eq!(
            copied,
            &selection_text(
                &[match &items[0] {
                    ScriptItem::Line(line) => line.clone(),
                    ScriptItem::Scene(_) => unreachable!(),
                }],
                state.selection().expect("drag must select text")
            )
        );
        assert!(copied.starts_with("[S01-L001] 蒼真: "));
    }

    #[test]
    fn clicking_line_margin_clears_selection_and_seeks() {
        let ctx = egui::Context::default();
        let items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("abcdef".to_owned())],
        })];
        let timeline = selection_timeline();
        let mut state = ScriptViewState {
            selection: Some(SelectionRange::new(
                SelectionPoint {
                    line: 0,
                    character: 0,
                },
                SelectionPoint {
                    line: 0,
                    character: 2,
                },
            )),
            ..Default::default()
        };
        let mut seek_to = None;
        for events in [
            vec![egui::Event::PointerButton {
                pos: Pos2::new(20.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(20.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = ctx.run(selection_input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                });
            });
        }
        assert_eq!(state.selection(), None);
        assert_eq!(seek_to, Some(0));
    }

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

    #[test]
    fn current_and_normal_text_meet_contrast_in_light_and_dark_visuals() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            assert!(contrast_ratio(visuals.text_color(), visuals.panel_fill) >= 4.5);
            assert!(
                contrast_ratio(
                    current_text_color(&visuals),
                    current_token_background(&visuals)
                ) >= 4.5
            );
        }
    }

    #[test]
    fn cache_generation_discards_entries_when_width_changes() {
        let mut cache = LayoutCache {
            width_bits: 600.0_f32.to_bits(),
            ..Default::default()
        };
        cache.entries.insert(
            ("S01-L001:old".to_owned(), true),
            Arc::new(LineLayout {
                tokens: vec![],
                rows: vec![],
                height: 0.0,
                base_size: 0.0,
                ruby_size: 0.0,
                ruby_top: 0.0,
                row_height: 0.0,
            }),
        );
        if cache.width_bits != 601.0_f32.to_bits() {
            cache.width_bits = 601.0_f32.to_bits();
            cache.entries.clear();
        }
        assert!(cache.entries.is_empty());
    }
}
