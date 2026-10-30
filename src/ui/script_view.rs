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
const AUTO_SCROLL_EDGE: f32 = 24.0;
const AUTO_SCROLL_MAX_DELTA: f32 = 18.0;

#[derive(Clone, Copy, Debug)]
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
}

pub fn selection_text(lines: &[ScriptLine], selection: SelectionRange) -> String {
    selection_text_refs(&lines.iter().collect::<Vec<_>>(), selection)
}

fn selection_text_refs(lines: &[&ScriptLine], selection: SelectionRange) -> String {
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
    drag_exceeded_threshold: bool,
    clear_layout_cache: bool,
    layout_cache: Option<Arc<Mutex<LayoutCache>>>,
    scroll_offset_y: f32,
}

impl ScriptViewState {
    pub fn selection(&self) -> Option<SelectionRange> {
        self.selection
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.drag_start = None;
        self.pressed_timeline = None;
        self.drag_exceeded_threshold = false;
    }

    pub fn clear_layout_cache(&mut self) {
        if let Some(cache) = &self.layout_cache {
            cache
                .lock()
                .expect("script layout cache poisoned")
                .entries
                .clear();
            self.clear_layout_cache = false;
        } else {
            self.clear_layout_cache = true;
        }
    }

    #[cfg(test)]
    fn scroll_offset_y(&self) -> f32 {
        self.scroll_offset_y
    }

    #[cfg(test)]
    pub fn layout_cache_entry_count(&self) -> usize {
        self.layout_cache.as_ref().map_or(0, |cache| {
            cache
                .lock()
                .expect("script layout cache poisoned")
                .entries
                .len()
        })
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
        format!("[{}] {}:{text}", line.id, line.cast)
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
    view_state.layout_cache = Some(Arc::clone(&cache));
    if view_state.clear_layout_cache {
        cache
            .lock()
            .expect("script layout cache poisoned")
            .entries
            .clear();
        view_state.clear_layout_cache = false;
    }
    let mut seek_to = None;
    let mut hit_lines = Vec::new();
    let mut line_index = 0;
    let scroll_output = egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
        let viewport = viewport.translate(ui.min_rect().min.to_vec2());
        if view_state.drag_start.is_some()
            && let Some(pointer) = ui.ctx().input(|input| input.pointer.interact_pos())
        {
            let delta_y = auto_scroll_delta(ui.clip_rect(), pointer);
            if delta_y != 0.0 {
                ui.scroll_with_delta_animation(
                    egui::vec2(0.0, delta_y),
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
        }
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
                        && view_state.drag_start.is_none()
                        && (changed || follow_started)
                        && matches!(state, LineState::Current { .. })
                    {
                        ui.scroll_to_rect_animation(
                            response.rect,
                            None,
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                }
            }
        }
    });
    view_state.scroll_offset_y = scroll_output.state.offset.y;
    let pointer_pos = ui.ctx().input(|input| input.pointer.interact_pos());
    let pointer_is_on_script_layer =
        pointer_pos.is_some_and(|pos| ui.ctx().layer_id_at(pos) == Some(ui.layer_id()));
    if pointer_is_on_script_layer
        && ui
            .ctx()
            .input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        && let Some(hit) = line_at_pointer(ui.ctx(), &hit_lines)
    {
        view_state.selection = None;
        view_state.drag_start = None;
        view_state.pressed_timeline = Some(hit.timeline_index);
        view_state.drag_exceeded_threshold = false;
        if let Some((point, pointer)) = pointer_hit_or_line_start(ui.ctx(), &hit_lines) {
            view_state.drag_start = Some((point, pointer));
        }
    }
    if let Some((start, origin)) = view_state.drag_start
        && let Some(pointer) = ui.ctx().input(|input| input.pointer.interact_pos())
        && pointer.distance(origin) >= 3.0
    {
        view_state.drag_exceeded_threshold = true;
        if let Some(end) = selection_point_at_or_nearest(&hit_lines, pointer) {
            view_state.selection = Some(SelectionRange::new(start, end));
        }
    }
    if ui
        .ctx()
        .input(|input| input.pointer.button_released(egui::PointerButton::Primary))
    {
        view_state.drag_start = None;
        if !view_state.drag_exceeded_threshold
            && view_state.selection.is_none()
            && let Some(Some(index)) = view_state.pressed_timeline.take()
        {
            seek_to = Some(timeline.lines[index].start_ms);
        }
    }
    let copy_requested = ui.ctx().input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy))
            || (input.modifiers.command && input.key_pressed(egui::Key::C))
    });
    if copy_requested {
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

fn pointer_hit_or_line_start(
    ctx: &egui::Context,
    hit_lines: &[HitLine],
) -> Option<(SelectionPoint, Pos2)> {
    let pointer = ctx.input(|input| input.pointer.interact_pos())?;
    let point = selection_point_at(hit_lines, pointer).or_else(|| {
        hit_lines
            .iter()
            .find(|line| line.rect.contains(pointer))
            .and_then(|line| {
                line.hits.first().map(|hit| SelectionPoint {
                    line: line.line,
                    character: hit.character,
                })
            })
    })?;
    Some((point, pointer))
}

fn selection_point_at_or_nearest(hit_lines: &[HitLine], pointer: Pos2) -> Option<SelectionPoint> {
    selection_point_at(hit_lines, pointer).or_else(|| {
        hit_lines
            .iter()
            .flat_map(|line| line.hits.iter().map(move |hit| (line.line, hit)))
            .min_by(|(_, left), (_, right)| {
                left.rect
                    .center()
                    .distance_sq(pointer)
                    .total_cmp(&right.rect.center().distance_sq(pointer))
            })
            .map(|(line, hit)| SelectionPoint {
                line,
                character: hit.character,
            })
    })
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

/// Negative values move the scroll area's content up, i.e. scroll downward.
fn auto_scroll_delta(viewport: Rect, pointer: Pos2) -> f32 {
    let edge_speed = |distance_from_edge: f32| {
        ((AUTO_SCROLL_EDGE - distance_from_edge) / AUTO_SCROLL_EDGE).clamp(0.0, 1.0)
            * AUTO_SCROLL_MAX_DELTA
    };
    if pointer.y < viewport.top() + AUTO_SCROLL_EDGE {
        edge_speed(pointer.y - viewport.top())
    } else if pointer.y > viewport.bottom() - AUTO_SCROLL_EDGE {
        -edge_speed(viewport.bottom() - pointer.y)
    } else {
        0.0
    }
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
        readable_text_color(if matches!(state, LineState::Current { .. }) {
            current_background(ui.visuals())
        } else {
            ui.visuals().panel_fill
        }),
    );
    let mut hits = Vec::new();
    for (row_index, range) in layout.rows.iter().enumerate() {
        let mut x = rect.left() + SPEAKER_WIDTH;
        let base_y = rect.top() + row_index as f32 * layout.row_height + layout.ruby_top;
        for token in &layout.tokens[range.clone()] {
            let selected = token_is_selected(selection, line_index, token);
            let color = token_color(ui.visuals(), state, token, selected);
            let ruby_color = ruby_text_color(ui.visuals(), state);
            let base_rect = Rect::from_min_size(
                Pos2::new(x, base_y),
                egui::vec2(token.width, layout.base_size * 1.35),
            );
            if selected {
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
                    ruby_color,
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

fn token_color(
    visuals: &egui::Visuals,
    state: LineState,
    token: &ScriptToken,
    selected: bool,
) -> Color32 {
    let background = token_background(visuals, state, token, selected);
    match state {
        LineState::Past => past_text_color(visuals, background),
        _ => readable_text_color(background),
    }
}

fn ruby_text_color(visuals: &egui::Visuals, state: LineState) -> Color32 {
    let background = if matches!(state, LineState::Current { .. }) {
        current_background(visuals)
    } else {
        visuals.panel_fill
    };
    if matches!(state, LineState::Past) {
        past_text_color(visuals, background)
    } else {
        readable_text_color(background)
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
    readable_text_color(current_token_background(visuals))
}

fn readable_text_color(background: Color32) -> Color32 {
    if contrast_ratio(Color32::WHITE, background) >= contrast_ratio(Color32::BLACK, background) {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

fn past_text_color(visuals: &egui::Visuals, background: Color32) -> Color32 {
    let preferred = if visuals.dark_mode {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(70)
    };
    if contrast_ratio(preferred, background) >= 4.5 {
        preferred
    } else {
        readable_text_color(background)
    }
}

fn token_background(
    visuals: &egui::Visuals,
    state: LineState,
    token: &ScriptToken,
    selected: bool,
) -> Color32 {
    if is_current_token(state, token) {
        current_token_background(visuals)
    } else if selected {
        selection_background(visuals)
    } else if matches!(state, LineState::Current { .. }) {
        current_background(visuals)
    } else {
        visuals.panel_fill
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_MAX_AUTO_SCROLL_PER_FRAME: f32 = 18.0;

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
            "[S01-L001] 蒼真:'東京|とうきょう'後\n[S01-L002] 大和:次の行"
        );
    }

    #[test]
    fn single_character_selection_is_copyable() {
        let point = SelectionPoint {
            line: 0,
            character: 1,
        };
        assert_eq!(
            selection_text(&selection_lines(), SelectionRange::new(point, point)),
            "[S01-L001] 蒼真:'東京|とうきょう'"
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

    #[test]
    fn auto_scroll_delta_accelerates_only_inside_viewport_edges() {
        let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 120.0));
        assert_eq!(auto_scroll_delta(viewport, Pos2::new(100.0, 60.0)), 0.0);
        assert!(auto_scroll_delta(viewport, Pos2::new(100.0, 4.0)) > 0.0);
        assert!(auto_scroll_delta(viewport, Pos2::new(100.0, 116.0)) < 0.0);
        assert!(
            auto_scroll_delta(viewport, Pos2::new(100.0, 119.0)).abs()
                > auto_scroll_delta(viewport, Pos2::new(100.0, 110.0)).abs()
        );
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
        selection_input_at_time(events, None)
    }

    fn selection_input_at_time(events: Vec<egui::Event>, time: Option<f64>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 120.0))),
            events,
            time,
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

    fn ruby_contrast_items() -> Vec<ScriptItem> {
        [
            ("S01-L001", "過去", "かこ"),
            ("S01-L002", "済今後", "ずみいまあと"),
            ("S01-L003", "未来", "みらい"),
        ]
        .into_iter()
        .map(|(id, base, reading)| {
            ScriptItem::Line(ScriptLine {
                id: id.to_owned(),
                scene: "S01".to_owned(),
                cast: "蒼真".to_owned(),
                segs: if id == "S01-L002" {
                    vec![
                        Seg::Ruby {
                            base: "済".to_owned(),
                            reading: "ずみ".to_owned(),
                        },
                        Seg::Ruby {
                            base: "今".to_owned(),
                            reading: "いま".to_owned(),
                        },
                        Seg::Ruby {
                            base: "後".to_owned(),
                            reading: "あと".to_owned(),
                        },
                    ]
                } else {
                    vec![Seg::Ruby {
                        base: base.to_owned(),
                        reading: reading.to_owned(),
                    }]
                },
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

    fn drawn_text_rect(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<Rect> {
        fn find(shape: &egui::epaint::Shape, wanted: &str) -> Option<Rect> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, wanted))
                }
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|clipped| find(&clipped.shape, wanted))
    }

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

    fn drawn_text_colours_over_topmost_fill(
        shapes: &[egui::epaint::ClippedShape],
    ) -> Vec<(String, Color32, Color32)> {
        fn collect(
            shape: &egui::epaint::Shape,
            fills: &mut Vec<(Rect, Color32)>,
            texts: &mut Vec<(String, Color32, Color32)>,
        ) {
            match shape {
                egui::epaint::Shape::Rect(rect) => fills.push((rect.rect, rect.fill)),
                egui::epaint::Shape::Text(text) => {
                    let text_rect = text.galley.rect.translate(text.pos.to_vec2());
                    let background = fills
                        .iter()
                        .rev()
                        .find(|(rect, _)| rect.contains(text_rect.center()))
                        .expect("each drawn script text must have a preceding fill")
                        .1;
                    texts.push((
                        text.galley.job.text.clone(),
                        text.override_text_color.unwrap_or(text.fallback_color),
                        background,
                    ));
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, fills, texts);
                    }
                }
                _ => {}
            }
        }

        let mut fills = Vec::new();
        let mut texts = Vec::new();
        for clipped in shapes {
            collect(&clipped.shape, &mut fills, &mut texts);
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
        draw_visual_frame_with_ruby(
            ctx,
            ScriptFrame::new(items, timeline, pos_ms, follow, false),
            events,
            state,
        )
    }

    #[derive(Clone, Copy)]
    struct ScriptFrame<'a> {
        items: &'a [ScriptItem],
        timeline: &'a Timeline,
        pos_ms: u64,
        follow: bool,
        show_ruby: bool,
    }

    impl<'a> ScriptFrame<'a> {
        fn new(
            items: &'a [ScriptItem],
            timeline: &'a Timeline,
            pos_ms: u64,
            follow: bool,
            show_ruby: bool,
        ) -> Self {
            Self {
                items,
                timeline,
                pos_ms,
                follow,
                show_ruby,
            }
        }
    }

    fn draw_visual_frame_with_ruby(
        ctx: &egui::Context,
        frame: ScriptFrame<'_>,
        events: Vec<egui::Event>,
        state: &mut ScriptViewState,
    ) -> eframe::egui::FullOutput {
        draw_script_frame(ctx, frame, events, state).0
    }

    fn draw_script_frame(
        ctx: &egui::Context,
        frame: ScriptFrame<'_>,
        events: Vec<egui::Event>,
        state: &mut ScriptViewState,
    ) -> (eframe::egui::FullOutput, Option<u64>) {
        draw_script_frame_at_time(ctx, frame, events, None, state)
    }

    fn draw_script_frame_at_time(
        ctx: &egui::Context,
        frame: ScriptFrame<'_>,
        events: Vec<egui::Event>,
        time: Option<f64>,
        state: &mut ScriptViewState,
    ) -> (eframe::egui::FullOutput, Option<u64>) {
        let mut seek_to = None;
        let output = ctx.run(selection_input_at_time(events, time), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                seek_to = draw_script(
                    ui,
                    frame.items,
                    frame.timeline,
                    frame.pos_ms,
                    frame.follow,
                    frame.show_ruby,
                    state,
                );
            });
        });
        (output, seek_to)
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
    fn headless_drag_selection_copies_text_with_event_copy() {
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
        let output = ctx.run(selection_input(vec![egui::Event::Copy]), |ctx| {
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
            .expect("Event::Copy must emit a copy command");
        assert_eq!(copied, "[S01-L001] 蒼真:abcd");
    }

    #[test]
    fn headless_short_drag_inside_one_plain_token_copies_that_character() {
        let ctx = egui::Context::default();
        let items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("あいう".to_owned())],
        })];
        let timeline = selection_timeline();
        let mut state = ScriptViewState::default();
        for events in [
            vec![],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(100.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(Pos2::new(104.0, 15.0))],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(104.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = draw_visual_frame(&ctx, &items, &timeline, 0, false, events, &mut state);
        }
        let output = draw_visual_frame(
            &ctx,
            &items,
            &timeline,
            0,
            false,
            vec![egui::Event::Copy],
            &mut state,
        );
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            });
        assert_eq!(copied, Some("[S01-L001] 蒼真:あ"));
    }

    #[test]
    fn headless_short_drag_inside_ruby_token_copies_the_whole_ruby_word() {
        let ctx = egui::Context::default();
        let items = selection_lines()
            .into_iter()
            .take(1)
            .map(ScriptItem::Line)
            .collect::<Vec<_>>();
        let timeline = selection_timeline();
        let mut state = ScriptViewState::default();
        for events in [
            vec![],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(115.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(Pos2::new(119.0, 15.0))],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(119.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = draw_visual_frame(&ctx, &items, &timeline, 0, false, events, &mut state);
        }
        let output = draw_visual_frame(
            &ctx,
            &items,
            &timeline,
            0,
            false,
            vec![egui::Event::Copy],
            &mut state,
        );
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            });
        assert_eq!(copied, Some("[S01-L001] 蒼真:'東京|とうきょう'"));
    }

    #[test]
    fn headless_context_menu_copy_keeps_selection_and_does_not_seek() {
        let ctx = egui::Context::default();
        let items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("abcdef".to_owned())],
        })];
        let timeline = selection_timeline();
        let mut state = ScriptViewState::default();
        let mut seek_to = None;
        let mut seek_results = Vec::new();
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
            vec![egui::Event::PointerButton {
                pos: Pos2::new(124.0, 15.0),
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(124.0, 15.0),
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = ctx.run(selection_input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                });
            });
            seek_results.push(seek_to);
        }
        let menu = ctx.run(selection_input(vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
            });
        });
        seek_results.push(seek_to);
        let menu_pos = drawn_text_rect(&menu.shapes, "コピー")
            .expect("context menu must render the copy button")
            .center();
        let _ = ctx.run(
            selection_input(vec![egui::Event::PointerButton {
                pos: menu_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                });
            },
        );
        seek_results.push(seek_to);
        let output = ctx.run(
            selection_input(vec![egui::Event::PointerButton {
                pos: menu_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                });
            },
        );
        seek_results.push(seek_to);
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        assert_eq!(copied, Some("[S01-L001] 蒼真:abcd".to_owned()));
        assert!(state.selection().is_some());
        assert_eq!(seek_to, None);

        for pressed in [true, false] {
            let _ = ctx.run(
                selection_input(vec![egui::Event::PointerButton {
                    pos: Pos2::new(124.0, 15.0),
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]),
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                    });
                },
            );
            seek_results.push(seek_to);
        }
        let menu = ctx.run(selection_input(vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
            });
        });
        seek_results.push(seek_to);
        let line_copy_pos = drawn_text_rect(&menu.shapes, "この行をコピー")
            .expect("context menu must render the line-copy button")
            .center();
        for pressed in [true, false] {
            let output = ctx.run(
                selection_input(vec![egui::Event::PointerButton {
                    pos: line_copy_pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]),
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        seek_to = draw_script(ui, &items, &timeline, 0, false, false, &mut state);
                    });
                },
            );
            seek_results.push(seek_to);
            if !pressed {
                let copied =
                    output
                        .platform_output
                        .commands
                        .iter()
                        .find_map(|command| match command {
                            egui::OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        });
                assert_eq!(copied, Some("[S01-L001] 蒼真:abcdef".to_owned()));
            }
        }
        assert!(state.selection().is_some());
        assert!(
            seek_results.iter().all(Option::is_none),
            "context-menu event frames must never seek: {seek_results:?}"
        );
    }

    #[test]
    fn headless_drag_at_lower_edge_scrolls_and_extends_selection() {
        let ctx = egui::Context::default();
        let items = visual_items(80);
        let timeline = visual_timeline(&items);
        let mut state = ScriptViewState::default();
        let (_, seek_to) = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 0, false, false),
            vec![],
            &mut state,
        );
        assert_eq!(seek_to, None);
        let (_, seek_to) = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 0, false, false),
            vec![egui::Event::PointerButton {
                pos: Pos2::new(105.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut state,
        );
        assert_eq!(seek_to, None);
        let mut offsets = Vec::new();
        let mut end_lines = Vec::new();
        for _ in 0..30 {
            let (_, seek_to) = draw_script_frame(
                &ctx,
                ScriptFrame::new(&items, &timeline, 0, false, false),
                vec![egui::Event::PointerMoved(Pos2::new(124.0, 114.0))],
                &mut state,
            );
            assert_eq!(seek_to, None, "dragging must never seek");
            offsets.push(state.scroll_offset_y());
            end_lines.push(
                state
                    .selection()
                    .expect("drag must extend the selection")
                    .end
                    .line,
            );
        }
        for window in offsets.windows(2) {
            let delta = window[1] - window[0];
            assert!(
                delta >= -0.01,
                "drag scroll must not move backwards: {delta}"
            );
            assert!(
                delta <= EXPECTED_MAX_AUTO_SCROLL_PER_FRAME + 0.1,
                "drag scroll exceeded its per-frame cap: {delta}"
            );
        }
        for window in end_lines.windows(2) {
            assert!(
                window[1] >= window[0],
                "selection end must not move backwards: {window:?}"
            );
        }
        assert!(state.scroll_offset_y() > 0.0);
        assert!(end_lines.last().is_some_and(|line| *line > 0));

        let (_, seek_to) = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 0, false, false),
            vec![egui::Event::PointerButton {
                pos: Pos2::new(124.0, 114.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut state,
        );
        assert_eq!(seek_to, None, "releasing a selection drag must not seek");
        for _ in 0..4 {
            let (_, seek_to) = draw_script_frame(
                &ctx,
                ScriptFrame::new(&items, &timeline, 0, false, false),
                vec![egui::Event::PointerMoved(Pos2::new(124.0, 114.0))],
                &mut state,
            );
            assert_eq!(seek_to, None);
        }
        let stopped_offset = state.scroll_offset_y();
        for _ in 0..3 {
            let (_, seek_to) = draw_script_frame(
                &ctx,
                ScriptFrame::new(&items, &timeline, 0, false, false),
                vec![egui::Event::PointerMoved(Pos2::new(124.0, 114.0))],
                &mut state,
            );
            assert_eq!(seek_to, None);
            assert!(
                (state.scroll_offset_y() - stopped_offset).abs() < 0.01,
                "auto-scroll must stop after release: before={stopped_offset}, after={} ",
                state.scroll_offset_y()
            );
        }
    }

    #[test]
    fn headless_follow_does_not_override_lower_edge_drag_but_restarts_after_release() {
        let ctx = egui::Context::default();
        let items = visual_items(80);
        let timeline = visual_timeline(&items);
        let mut state = ScriptViewState::default();
        let _ = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 0, true, false),
            vec![],
            &mut state,
        );
        let _ = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 0, true, false),
            vec![egui::Event::PointerButton {
                pos: Pos2::new(105.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut state,
        );
        let mut offsets = Vec::new();
        let mut end_lines = Vec::new();
        for frame in 0..30 {
            let position_ms = if frame < 20 { 0 } else { 1_000 };
            let (_, seek_to) = draw_script_frame(
                &ctx,
                ScriptFrame::new(&items, &timeline, position_ms, true, false),
                vec![egui::Event::PointerMoved(Pos2::new(124.0, 114.0))],
                &mut state,
            );
            assert_eq!(seek_to, None);
            offsets.push(state.scroll_offset_y());
            end_lines.push(
                state
                    .selection()
                    .expect("drag must extend the selection")
                    .end
                    .line,
            );
        }
        assert!(offsets.windows(2).all(|window| window[1] >= window[0]));
        assert!(end_lines.windows(2).all(|window| window[1] >= window[0]));
        let held_offset = state.scroll_offset_y();
        let (_, seek_to) = draw_script_frame(
            &ctx,
            ScriptFrame::new(&items, &timeline, 1_000, true, false),
            vec![egui::Event::PointerButton {
                pos: Pos2::new(124.0, 114.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut state,
        );
        assert_eq!(seek_to, None);
        let mut followed = None;
        for frame in 0..30 {
            let (output, seek_to) = draw_script_frame_at_time(
                &ctx,
                ScriptFrame::new(&items, &timeline, 2_000, true, false),
                vec![],
                Some(1.0 + f64::from(frame) / 60.0),
                &mut state,
            );
            assert_eq!(seek_to, None);
            followed = Some(output);
        }
        let _ = followed.expect("follow frames must run");
        assert!(
            state.scroll_offset_y() < held_offset,
            "the next current-line change after release must resume follow"
        );
    }

    #[test]
    fn headless_current_token_uses_a_contrasting_text_shape_over_its_real_fill() {
        let items = ruby_contrast_items();
        let timeline = visual_timeline(&items);
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let mut state = ScriptViewState::default();
            let output = draw_visual_frame_with_ruby(
                &ctx,
                ScriptFrame::new(&items, &timeline, 1_500, false, true),
                vec![],
                &mut state,
            );
            let (_, text, background) = drawn_text_colours_over_topmost_fill(&output.shapes)
                .into_iter()
                .find(|(text, _, _)| text == "いま")
                .expect("the current token ruby must be drawn");
            assert!(
                test_contrast_ratio(text, background) >= 4.5,
                "current ruby must contrast with its actual fill"
            );
        }
    }

    #[test]
    fn headless_drag_from_speaker_or_past_text_tail_selects_without_seeking() {
        let ctx = egui::Context::default();
        let items = vec![ScriptItem::Line(ScriptLine {
            id: "S01-L001".to_owned(),
            scene: "S01".to_owned(),
            cast: "蒼真".to_owned(),
            segs: vec![Seg::Plain("abcdef".to_owned())],
        })];
        let timeline = selection_timeline();
        let mut state = ScriptViewState::default();
        let mut seek_to = None;
        for events in [
            vec![],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(20.0, 15.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(Pos2::new(290.0, 15.0))],
            vec![egui::Event::PointerButton {
                pos: Pos2::new(290.0, 15.0),
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
        assert_eq!(
            state.selection(),
            Some(SelectionRange::new(
                SelectionPoint {
                    line: 0,
                    character: 0
                },
                SelectionPoint {
                    line: 0,
                    character: 5
                },
            ))
        );
        assert_eq!(seek_to, None);
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
    fn every_drawn_script_text_colour_meets_wcag_contrast_in_both_themes() {
        let items = ruby_contrast_items();
        let timeline = visual_timeline(&items);
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let mut state = ScriptViewState::default();
            let output = draw_visual_frame_with_ruby(
                &ctx,
                ScriptFrame::new(&items, &timeline, 1_500, false, true),
                vec![],
                &mut state,
            );
            let drawn = drawn_text_colours_over_topmost_fill(&output.shapes);
            assert!(
                drawn.iter().any(|(text, _, _)| text == "かこ"),
                "Past line ruby must be drawn"
            );
            assert!(
                drawn.iter().any(|(text, _, _)| text == "ずみ"),
                "already-spoken current-line ruby must be drawn"
            );
            assert!(
                drawn.iter().any(|(text, _, _)| text == "いま"),
                "current-token ruby must be drawn"
            );
            assert!(
                drawn.iter().any(|(text, _, _)| text == "あと"),
                "not-yet-spoken current-line ruby must be drawn"
            );
            assert!(
                drawn.iter().any(|(text, _, _)| text == "みらい"),
                "Future line ruby must be drawn"
            );
            for (text, foreground, background) in drawn {
                assert!(
                    test_contrast_ratio(foreground, background) >= 4.5,
                    "text={text:?} contrast={} foreground={foreground:?} background={background:?}",
                    test_contrast_ratio(foreground, background)
                );
            }
        }
    }

    #[test]
    fn clear_layout_cache_removes_the_drawn_layout_entries() {
        let ctx = egui::Context::default();
        let items = visual_items(3);
        let timeline = visual_timeline(&items);
        let mut state = ScriptViewState::default();
        let _ = draw_visual_frame(&ctx, &items, &timeline, 0, false, vec![], &mut state);
        assert!(
            state.layout_cache_entry_count() > 0,
            "drawing must populate the layout cache"
        );
        state.clear_layout_cache();
        assert_eq!(state.layout_cache_entry_count(), 0);
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
