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

struct Cx<'a> {
    z: f32,
    t: &'a Tokens,
    lang: Lang,
}

impl Cx<'_> {
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

/// Draws `blocks` into `ui`'s width.
pub fn show(ui: &mut Ui, blocks: &[Block], z: f32, t: &Tokens, lang: Lang, diagrams: &mut diagram::Cache) -> Option<Action> {
    let cx = Cx { z, t, lang };
    let mut out = Out { action: None, diagrams };
    if blocks.is_empty() {
        ui.label(egui::RichText::new(lang.tr("This document is empty. Choose Edit to write it.")).font(theme::ui_font(cx.body())).color(t.text_dim));
    }
    for b in blocks {
        block(ui, b, &cx, &mut out, false);
    }
    out.action
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
        && let Some(resp) = diagram::show(ui, out.diagrams, text, cx.z, cx.t, cx.lang)
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

fn table(ui: &mut Ui, align: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], line: usize, cx: &Cx, out: &mut Out) {
    let t = cx.t;
    egui::ScrollArea::horizontal().id_salt(("monun.doc_table", line)).auto_shrink([false, true]).show(ui, |ui| {
        let resp = egui::Grid::new(("monun.doc_table_grid", line))
            .striped(true)
            .spacing(vec2(18.0 * cx.z, 6.0 * cx.z))
            .show(ui, |ui| {
                for (i, cell) in head.iter().enumerate() {
                    cell_ui(ui, cell, align.get(i).copied().unwrap_or_default(), cx, true, line, out);
                }
                ui.end_row();
                for row in rows {
                    for (i, cell) in row.iter().enumerate() {
                        cell_ui(ui, cell, align.get(i).copied().unwrap_or_default(), cx, false, line, out);
                    }
                    ui.end_row();
                }
            })
            .response;
        let r = resp.rect;
        let y = r.top() + cx.body() * 1.45 + 3.0 * cx.z;
        ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, t.separator));
    });
}

fn cell_ui(ui: &mut Ui, spans: &[Span], align: Align, cx: &Cx, head: bool, line: usize, out: &mut Out) {
    let layout = match align {
        Align::Right => egui::Layout::right_to_left(egui::Align::Center),
        Align::Center => egui::Layout::top_down(egui::Align::Center),
        _ => egui::Layout::left_to_right(egui::Align::Center),
    };
    ui.with_layout(layout, |ui| {
        rich_line(ui, spans, cx, FontSpec { size: cx.body(), bold: head }, cx.t.text, line, out, f32::INFINITY);
    });
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

/// Lays out styled spans, draws them, and reports a click on a link (or a double-click).
#[allow(clippy::too_many_arguments)]
fn rich_line(ui: &mut Ui, spans: &[Span], cx: &Cx, font: FontSpec, color: Color32, line: usize, out: &mut Out, width: f32) {
    let t = cx.t;
    let mut job = LayoutJob::default();
    let mut links: Vec<(usize, usize, &str)> = Vec::new();
    for s in spans {
        let family = if font.bold || s.style.bold { theme::UI_BOLD } else { theme::UI };
        let mut f = TextFormat::simple(FontId::new(font.size, FontFamily::Name(family.into())), color);
        if s.style.code {
            f.font_id = FontId::new(font.size * 0.9, FontFamily::Name(theme::MONO.into()));
            f.background = if t.dark { Color32::from_white_alpha(20) } else { Color32::from_black_alpha(14) };
        }
        f.italics = s.style.italic;
        if s.style.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        let text = if s.style.image { format!("[{}]", s.text) } else { s.text.clone() };
        if s.style.image {
            f.color = t.text_dim;
        }
        if let Some(link) = &s.link {
            f.color = t.accent;
            f.underline = Stroke::new(1.0, t.accent);
            let start = job.text.chars().count();
            links.push((start, start + text.chars().count(), link.as_str()));
        }
        job.append(&text, 0.0, f);
    }
    if job.sections.is_empty() {
        job.append(" ", 0.0, TextFormat::simple(FontId::new(font.size, FontFamily::Name(theme::UI.into())), color));
    }
    job.wrap.max_width = width;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let plain = galley.text().to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(if links.is_empty() { egui::Role::Label } else { egui::Role::Link }, true, &plain));
    ui.painter().galley(rect.min, galley.clone(), color);
    let link = resp.hover_pos().and_then(|p| {
        let at: usize = galley.cursor_from_pos(p - rect.min).index.into();
        links.iter().find(|(a, b, _)| (*a..*b).contains(&at)).map(|(_, _, l)| *l)
    });
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
