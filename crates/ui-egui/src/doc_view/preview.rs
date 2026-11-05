//! The rendered document: Markdown blocks laid out with the interface's fonts. Links, task boxes
//! and a double-click (go to that line in the editor) come back as an [`Action`].

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use monun_markdown::{Align, Block, Item, Span};

use super::diagram;
use crate::i18n::Language as Lang;
use crate::theme::{self, Tokens};

/// What a click in the document asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Tick or clear the task box at this byte of the text.
    ToggleTask(usize),
    /// Follow a link (`https://…`, `#Sheet1!A1`).
    Link(String),
    /// Put the editor's caret on this line.
    Goto(usize),
}

/// How to draw: the zoom, the theme, the language, the fields' values and the text being looked
/// for (empty for none).
pub struct Look<'a> {
    pub z: f32,
    pub t: &'a Tokens,
    pub lang: Lang,
    pub fields: &'a super::Fields,
    pub query: &'a str,
    /// The workbook's uid and the document tab's pictures, which `![…](image:N)` shows.
    pub doc: u64,
    pub pictures: &'a [monun_engine::model::Image],
    /// Fit Width: tables and diagrams as wide as the page.
    pub full: bool,
    /// `{{…}}` reads as typed, untinted: a Markdown file has no cells for a field to show.
    pub plain_fields: bool,
}

/// Found text's tint.
pub const FOUND: Color32 = Color32::from_rgba_premultiplied(110, 88, 0, 110);

type Cx<'a> = Look<'a>;

impl Look<'_> {
    fn body(&self) -> f32 {
        15.0 * self.z
    }
    fn gap(&self) -> f32 {
        10.0 * self.z
    }
}

/// What drawing the blocks produced: a click's action, and the diagram drawings to use.
struct Out<'a> {
    action: Option<Action>,
    diagrams: &'a mut diagram::Cache,
}

/// The heights of the rendered document's top-level blocks as last laid out, for one parse of the
/// text at one width and zoom. Blocks out of view keep their room without being laid out: a
/// long document (tens of thousands of lines) took most of a second to lay out whole, every time
/// its tab was shown.
#[derive(Default)]
pub struct Heights {
    key: Option<(usize, usize, u32, u32, bool)>,
    known: Vec<Option<f32>>,
}

/// Draws `blocks` into `ui`'s width: those within a screen of the view laid out, the others as
/// the room they took last (or an estimate until they have been seen). Returns what a click
/// asked for, and where each top-level block starts: its first line and its offset from the top
/// of what was drawn.
pub fn show(ui: &mut Ui, blocks: &[Block], cx: &Look, diagrams: &mut diagram::Cache, heights: &mut Heights) -> (Option<Action>, Vec<(usize, f32)>) {
    let mut out = Out { action: None, diagrams };
    if blocks.is_empty() {
        ui.label(
            egui::RichText::new(cx.lang.tr("This document is empty. Choose Edit to write it.")).font(theme::ui_font(cx.body())).color(cx.t.text_dim),
        );
    }
    let key = (blocks.as_ptr() as usize, blocks.len(), ui.available_width().to_bits(), cx.z.to_bits(), cx.full);
    if heights.key != Some(key) {
        heights.key = Some(key);
        heights.known = vec![None; blocks.len()];
    }
    let clip = ui.clip_rect();
    let reach = clip.height().max(200.0);
    let (near_top, near_bottom) = (clip.top() - reach, clip.bottom() + reach);
    let top = ui.cursor().top();
    let mut positions = Vec::with_capacity(blocks.len());
    for (i, b) in blocks.iter().enumerate() {
        let y = ui.cursor().top();
        positions.push((b.line(), y - top));
        let lines = blocks.get(i + 1).map_or(1, |next| next.line().saturating_sub(b.line()));
        let room = heights.known.get(i).copied().flatten().unwrap_or_else(|| estimate(b, lines, cx));
        if y + room < near_top || y > near_bottom {
            ui.add_space(room);
            continue;
        }
        block(ui, b, cx, &mut out, false);
        if let Some(h) = heights.known.get_mut(i) {
            *h = Some(ui.cursor().top() - y);
        }
    }
    (out.action, positions)
}

/// The room a block not yet laid out is given: its source lines at the body's line height, a
/// heading or diagram about what it takes.
fn estimate(b: &Block, lines: usize, cx: &Cx) -> f32 {
    match b {
        Block::Heading { level, .. } => (if *level <= 2 { 50.0 } else { 34.0 }) * cx.z,
        Block::Code { lang, .. } if lang == "mermaid" => 260.0 * cx.z,
        _ => lines.max(1) as f32 * cx.body() * 1.45 + cx.gap(),
    }
}

fn block(ui: &mut Ui, b: &Block, cx: &Cx, out: &mut Out, tight: bool) {
    let t = cx.t;
    match b {
        Block::Heading { level, text, line } => {
            let size = match level {
                1 => 28.0,
                2 => 22.0,
                3 => 18.5,
                4 => 16.5,
                _ => 15.0,
            } * cx.z;
            ui.add_space(if *level <= 2 { 8.0 * cx.z } else { 4.0 * cx.z });
            rich(ui, text, cx, FontSpec { size, bold: true }, t.text, *line, out);
            if *level <= 2 {
                let r = ui.available_rect_before_wrap();
                ui.painter().line_segment([pos2(r.left(), r.top() + 3.0), pos2(r.right(), r.top() + 3.0)], Stroke::new(1.0, t.separator));
                ui.add_space(6.0 * cx.z);
            }
        }
        Block::Paragraph { text, line } if is_pictures(text) => pictures(ui, text, cx, *line, out),
        Block::Paragraph { text, line } => rich(ui, text, cx, FontSpec { size: cx.body(), bold: false }, t.text, *line, out),
        Block::List { start, items, .. } => {
            for (i, item) in items.iter().enumerate() {
                list_item(ui, item, start.map(|s| s + i as u64), cx, out);
            }
        }
        Block::Quote { blocks, .. } => {
            let top = ui.cursor().top();
            let left = ui.cursor().left();
            ui.horizontal_top(|ui| {
                ui.add_space(16.0 * cx.z);
                ui.vertical(|ui| {
                    for b in blocks {
                        block(ui, b, cx, out, true);
                    }
                });
            });
            let bottom = ui.min_rect().bottom();
            ui.painter().rect_filled(Rect::from_min_max(pos2(left + 2.0, top), pos2(left + 2.0 + 3.0 * cx.z, bottom)), 1.5, t.separator);
        }
        Block::Code { lang, text, line } => code(ui, lang, text, *line, cx, out),
        Block::Table { align, head, rows, line } => table(ui, align, head, rows, *line, cx, out),
        Block::Rule { .. } => {
            ui.add_space(4.0 * cx.z);
            let r = ui.available_rect_before_wrap();
            ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], Stroke::new(1.5, t.separator));
            ui.add_space(4.0 * cx.z);
        }
    }
    ui.add_space(if tight { 3.0 * cx.z } else { cx.gap() });
}

/// Whether a paragraph holds nothing but pictures (and the breaks between them).
fn is_pictures(spans: &[Span]) -> bool {
    spans.iter().any(|s| s.style.image) && spans.iter().all(|s| s.style.image || s.text.trim().is_empty())
}

/// The picture `image:N` names in the document tab's pictures.
fn picture<'a>(cx: &'a Look, link: Option<&str>) -> Option<&'a monun_engine::model::Image> {
    let id: u32 = link?.strip_prefix("image:")?.trim().parse().ok()?;
    cx.pictures.iter().find(|p| p.id == id)
}

/// A paragraph of pictures, each on its own line at its size (narrowed to fit), or its
/// description when the picture isn't there.
fn pictures(ui: &mut Ui, spans: &[Span], cx: &Look, line: usize, out: &mut Out) {
    for s in spans.iter().filter(|s| s.style.image) {
        let Some(image) = picture(cx, s.link.as_deref()) else {
            ui.label(egui::RichText::new(format!("[{}]", s.text)).font(theme::ui_font(cx.body())).color(cx.t.text_dim));
            continue;
        };
        let size = vec2(image.anchor.width.max(1.0), image.anchor.height.max(1.0)) * cx.z;
        let size = size * (ui.available_width() / size.x).min(1.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        match crate::floating_pictures::texture(ui.ctx(), cx.doc, image, rect) {
            Some(texture) => {
                ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, cx.t.separator), egui::StrokeKind::Inside);
            }
        }
        let alt = if s.text.is_empty() { image.alt.clone() } else { s.text.clone() };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &alt));
        if resp.double_clicked() {
            out.action = Some(Action::Goto(line));
        }
        ui.add_space(4.0 * cx.z);
    }
}

fn list_item(ui: &mut Ui, item: &Item, number: Option<u64>, cx: &Cx, out: &mut Out) {
    let t = cx.t;
    let gutter = 26.0 * cx.z;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let row_h = cx.body() * 1.4;
        let (r, resp) = ui.allocate_exact_size(vec2(gutter, row_h), if item.task.is_some() { Sense::click() } else { Sense::hover() });
        match (item.task, number) {
            (Some(task), _) => {
                let side = 14.0 * cx.z;
                let b = Rect::from_center_size(pos2(r.left() + side / 2.0 + 2.0, r.center().y), vec2(side, side));
                if task.done {
                    ui.painter().rect_filled(b, 3.0, t.accent);
                    let pts = [
                        pos2(b.left() + side * 0.22, b.center().y),
                        pos2(b.left() + side * 0.42, b.bottom() - side * 0.25),
                        pos2(b.right() - side * 0.2, b.top() + side * 0.25),
                    ];
                    ui.painter().line(pts.to_vec(), Stroke::new(1.8 * cx.z, t.on_accent));
                } else {
                    ui.painter().rect_stroke(b, 3.0, Stroke::new(1.3, if resp.hovered() { t.accent } else { t.text_dim }), egui::StrokeKind::Inside);
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let resp = resp.on_hover_text(cx.lang.tr(if task.done { "Mark as not done" } else { "Mark as done" }));
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, task.done, "task"));
                if resp.clicked() {
                    out.action = Some(Action::ToggleTask(task.at));
                }
            }
            (None, Some(n)) => {
                ui.painter().text(
                    pos2(r.right() - 6.0 * cx.z, r.center().y),
                    Align2::RIGHT_CENTER,
                    format!("{n}."),
                    theme::ui_font(cx.body()),
                    t.text,
                );
            }
            (None, None) => {
                ui.painter().circle_filled(pos2(r.left() + 7.0 * cx.z, r.center().y), 2.6 * cx.z, t.text);
            }
        }
        ui.add_space(4.0 * cx.z);
        ui.vertical(|ui| {
            for b in &item.blocks {
                block(ui, b, cx, out, true);
            }
        });
    });
}

fn code(ui: &mut Ui, lang: &str, text: &str, line: usize, cx: &Cx, out: &mut Out) {
    if lang == "mermaid"
        && let Some(resp) = diagram::show(ui, out.diagrams, text, cx.z, cx.t, cx.lang, cx.full)
    {
        if resp.double_clicked() {
            out.action = Some(Action::Goto(line));
        }
        return;
    }
    let t = cx.t;
    let font = FontId::new(13.5 * cx.z, FontFamily::Name(theme::MONO.into()));
    let fill = if t.dark { Color32::from_white_alpha(14) } else { Color32::from_black_alpha(10) };
    let resp = egui::Frame::new()
        .fill(fill)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same((10.0 * cx.z).round() as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !lang.is_empty() {
                ui.label(egui::RichText::new(lang).font(theme::ui_font(11.0 * cx.z)).color(t.text_dim));
            }
            let mut job = LayoutJob::simple(text.trim_end_matches('\n').to_owned(), font, t.text, ui.available_width());
            job.wrap.max_width = ui.available_width();
            ui.label(job);
        })
        .response
        .interact(Sense::click());
    if resp.double_clicked() {
        out.action = Some(Action::Goto(line));
    }
}

/// A table with its lines drawn: the header row shaded, bold and centred, the other rows aligned
/// as their columns say. Columns are as wide as their cells, narrowed to fit (their text wraps),
/// or with Fit Width widened to the page.
fn table(ui: &mut Ui, align: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], line: usize, cx: &Cx, out: &mut Out) {
    let t = cx.t;
    let cols = rows.iter().map(Vec::len).chain(std::iter::once(head.len())).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let pad = vec2(10.0, 6.0) * cx.z;
    let plain = FontSpec { size: cx.body(), bold: false };
    let all: Vec<(&[Vec<Span>], bool)> = std::iter::once((head, true)).chain(rows.iter().map(|r| (r.as_slice(), false))).collect();
    let font = |header: bool| FontSpec { bold: header, ..plain };
    let mut natural = vec![0.0f32; cols];
    for (row, header) in &all {
        for (cell, w) in row.iter().zip(natural.iter_mut()) {
            // A pixel to spare, so a cell that fits on one line isn't wrapped by rounding.
            *w = w.max(measure(ui, cell, cx, font(*header), f32::INFINITY).x.ceil() + 1.0 + pad.x * 2.0);
        }
    }
    let room = ui.available_width();
    let widths = if cx.full { monun_markdown::fill_columns } else { monun_markdown::fit_columns }(&natural, room, 48.0 * cx.z);
    let line_h = cx.body() * 1.4;
    let heights: Vec<f32> = all
        .iter()
        .map(|(row, header)| {
            let tallest = row.iter().zip(&widths).map(|(cell, w)| measure(ui, cell, cx, font(*header), w - pad.x * 2.0).y).fold(line_h, f32::max);
            tallest + pad.y * 2.0
        })
        .collect();
    let size = vec2(widths.iter().sum(), heights.iter().sum());
    let head_fill = if t.dark { Color32::from_rgb(0x34, 0x38, 0x3D) } else { Color32::from_rgb(0xE3, 0xE8, 0xEF) };
    let rule = if t.dark { Color32::from_rgb(0x55, 0x58, 0x5C) } else { Color32::from_rgb(0xC9, 0xCE, 0xD4) };
    let draw = |ui: &mut Ui, out: &mut Out| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let radius = 4.0 * cx.z;
        let head_h = heights.first().copied().unwrap_or(0.0);
        let r8 = radius.round().clamp(0.0, 255.0) as u8;
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(rect.width(), head_h)),
            egui::CornerRadius { nw: r8, ne: r8, sw: 0, se: 0 },
            head_fill,
        );
        let mut y = rect.top();
        for (r, ((row, header), h)) in all.iter().zip(&heights).enumerate() {
            let mut x = rect.left();
            for (c, w) in widths.iter().enumerate() {
                if let Some(cell) = row.get(c) {
                    let inner = Rect::from_min_size(pos2(x, y), vec2(*w, *h)).shrink2(pad);
                    let shown = measure(ui, cell, cx, font(*header), inner.width());
                    let column = if *header { Align::Center } else { align.get(c).copied().unwrap_or_default() };
                    let left = match column {
                        Align::Right => inner.right() - shown.x,
                        Align::Center => inner.center().x - shown.x / 2.0,
                        _ => inner.left(),
                    };
                    let at = Rect::from_min_size(pos2(left, inner.center().y - shown.y / 2.0), shown);
                    let mut cell_ui = ui.new_child(egui::UiBuilder::new().id_salt(("monun.doc_cell", line, r, c)).max_rect(at));
                    rich_line(&mut cell_ui, cell, cx, font(*header), t.text, line, out, inner.width());
                }
                x += w;
            }
            y += h;
            // The line under the header is the stronger one.
            if r + 1 < all.len() {
                let stroke = if r == 0 { Stroke::new(1.5, rule) } else { Stroke::new(1.0, rule) };
                ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
            }
        }
        let mut x = rect.left();
        for w in widths.iter().take(widths.len().saturating_sub(1)) {
            x += w;
            ui.painter().line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, rule));
        }
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, rule), egui::StrokeKind::Inside);
    };
    if size.x <= room + 0.5 {
        draw(ui, out);
    } else {
        // Too many columns to fit even narrowed: the table scrolls sideways.
        egui::ScrollArea::horizontal().id_salt(("monun.doc_table", line)).auto_shrink([false, true]).show(ui, |ui| draw(ui, out));
    }
}

#[derive(Clone, Copy)]
struct FontSpec {
    size: f32,
    bold: bool,
}

/// A paragraph or heading, wrapped to the width.
fn rich(ui: &mut Ui, spans: &[Span], cx: &Cx, font: FontSpec, color: Color32, line: usize, out: &mut Out) {
    let width = ui.available_width();
    rich_line(ui, spans, cx, font, color, line, out, width);
}

/// Styled spans as text to lay out `width` wide.
struct Styled<'a> {
    job: LayoutJob,
    /// Links' char ranges and targets.
    links: Vec<(usize, usize, &'a str)>,
    /// `{{…}}` fields: their char range and the expression, shown on hover.
    tips: Vec<(usize, usize, String)>,
}

fn styled<'a>(spans: &'a [Span], cx: &Cx, font: FontSpec, color: Color32, width: f32) -> Styled<'a> {
    let t = cx.t;
    let mut job = LayoutJob::default();
    let mut links: Vec<(usize, usize, &str)> = Vec::new();
    let mut tips: Vec<(usize, usize, String)> = Vec::new();
    for s in spans {
        let family = if font.bold || s.style.bold { theme::UI_BOLD } else { theme::UI };
        let mut f = TextFormat::simple(FontId::new(font.size, FontFamily::Name(family.into())), color);
        if s.style.code {
            f.font_id = FontId::new(font.size * 0.9, FontFamily::Name(theme::DOC_CODE.into()));
            f.background = if t.dark { Color32::from_white_alpha(20) } else { Color32::from_black_alpha(14) };
        }
        f.italics = s.style.italic;
        if s.style.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        let text = if s.style.field && cx.plain_fields {
            format!("{{{{{}}}}}", s.text)
        } else if s.style.field {
            // A field shows its value, on a tint that tells it apart from typed text.
            f.background = t.accent_soft;
            let start = job.text.chars().count();
            let shown = match cx.fields.get(&s.text) {
                Some(Ok(value)) => value.clone(),
                Some(Err(e)) => {
                    f.color = t.danger;
                    e.clone()
                }
                None => "…".to_owned(),
            };
            tips.push((start, start + shown.chars().count(), format!("{{{{{}}}}}", s.text)));
            shown
        } else if s.style.image {
            f.color = t.text_dim;
            format!("[{}]", s.text)
        } else {
            s.text.clone()
        };
        if let Some(link) = &s.link {
            f.color = t.accent;
            f.underline = Stroke::new(1.0, t.accent);
            let start = job.text.chars().count();
            links.push((start, start + text.chars().count(), link.as_str()));
        }
        // Text being looked for is tinted (not inside a field's value).
        if cx.query.is_empty() || s.style.field {
            job.append(&text, 0.0, f);
        } else {
            let mut from = 0;
            for found in super::find_all(&text, cx.query) {
                if let (Some(before), Some(hit)) = (text.get(from..found.start), text.get(found.clone())) {
                    job.append(before, 0.0, f.clone());
                    job.append(hit, 0.0, TextFormat { background: FOUND, ..f.clone() });
                    from = found.end;
                }
            }
            job.append(text.get(from..).unwrap_or(""), 0.0, f);
        }
    }
    if job.sections.is_empty() {
        job.append(" ", 0.0, TextFormat::simple(FontId::new(font.size, FontFamily::Name(theme::UI.into())), color));
    }
    job.wrap.max_width = width;
    Styled { job, links, tips }
}

/// The size styled spans take `width` wide.
fn measure(ui: &Ui, spans: &[Span], cx: &Cx, font: FontSpec, width: f32) -> egui::Vec2 {
    let job = styled(spans, cx, font, cx.t.text, width).job;
    ui.fonts_mut(|f| f.layout_job(job)).size()
}

/// Lays out styled spans, draws them, and reports a click on a link (or a double-click).
#[allow(clippy::too_many_arguments)]
fn rich_line(ui: &mut Ui, spans: &[Span], cx: &Cx, font: FontSpec, color: Color32, line: usize, out: &mut Out, width: f32) {
    let Styled { job, links, tips } = styled(spans, cx, font, color, width);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let plain = galley.text().to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(if links.is_empty() { egui::Role::Label } else { egui::Role::Link }, true, &plain));
    ui.painter().galley(rect.min, galley.clone(), color);
    let at: Option<usize> = resp.hover_pos().map(|p| galley.cursor_from_pos(p - rect.min).index.into());
    let link = at.and_then(|at| links.iter().find(|(a, b, _)| (*a..*b).contains(&at)).map(|(_, _, l)| *l));
    if let Some(tip) = at.and_then(|at| tips.iter().find(|(a, b, _)| (*a..*b).contains(&at))).filter(|_| link.is_none()) {
        resp.clone().on_hover_text(&tip.2);
    }
    if let Some(l) = link {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            out.action = Some(Action::Link(l.to_owned()));
        }
        resp.clone().on_hover_text(l);
    }
    if resp.double_clicked() {
        out.action = Some(Action::Goto(line));
    }
}
