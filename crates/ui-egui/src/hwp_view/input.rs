//! The keyboard, the mouse and the clipboard over a 한글 document's pages.
//!
//! In Edit mode, once the pages have the keyboard (a click on them), keys become requests to the
//! document's thread — typing, Backspace and Delete, Enter, Tab between table cells, the arrows
//! with Shift and Ctrl, Ctrl+B/I/U — and an input method's composition goes through
//! [`super::ime`]. In View mode the keys scroll, and text can still be selected and copied.
//! Monun's own shortcuts (Save, Open, Close, Undo, Find) are taken first, by the Markdown tab's
//! handler.

use egui::{Event, ImeEvent, Key, Modifiers, Pos2, Rect, Sense, Ui, Vec2};

use super::proto::{Move, Point, Req, Toggle};
use super::{Doc, Go, Phase, canvas_id};
use crate::SheetApp;
use crate::page_layout::Layout;

/// One arrow key's scroll in View mode (egui points).
const LINE_STEP: f32 = 48.0;
/// The longest composition kept to draw (characters).
const MAX_PREEDIT: usize = 4096;

/// The keys and the clipboard of the tab `uid`'s document; not while a field (Find, the page
/// number) or a dialog has the keyboard.
pub(super) fn keys(app: &mut SheetApp, ctx: &egui::Context, doc: &mut Doc, uid: u64, editing: bool) {
    if app.dialog.is_some() || app.message.is_some() || ctx.text_edit_focused() || !matches!(doc.phase, Phase::Ready) {
        return;
    }
    let typing = editing && ctx.memory(|m| m.has_focus(canvas_id(uid)));
    let now = ctx.input(|i| i.time);
    let events = ctx.input(|i| i.events.clone());
    let mut used = vec![false; events.len()];
    for (k, ev) in events.iter().enumerate() {
        let done = match ev {
            Event::Key { key, pressed: true, modifiers, .. } if typing => match edit_key(*key, *modifiers) {
                Some(req) => {
                    doc.ime.changed(ctx, false);
                    doc.send(req);
                    true
                }
                None => false,
            },
            Event::Key { key, pressed: true, modifiers, .. } => {
                if let Some(go) = view_key(doc, *key, *modifiers) {
                    doc.go = Some(go);
                    used[k] = true;
                }
                continue;
            }
            Event::Text(text) if typing && !text.chars().any(char::is_control) => {
                doc.ime.changed(ctx, false);
                doc.type_text(text.clone());
                true
            }
            Event::Ime(ImeEvent::Preedit { text, .. }) if typing => {
                doc.ime.preedit = text.chars().take(MAX_PREEDIT).collect();
                true
            }
            Event::Ime(ImeEvent::Commit(text)) if typing => {
                if let Some(text) = doc.ime.commit(ctx, text) {
                    doc.type_text(text);
                }
                true
            }
            Event::Copy => {
                doc.send(Req::Copy { cut: false });
                true
            }
            Event::Cut if typing => {
                doc.ime.changed(ctx, false);
                doc.send(Req::Copy { cut: true });
                true
            }
            Event::Paste(text) if typing => {
                paste(app, ctx, doc, text.clone());
                true
            }
            _ => false,
        };
        if done {
            used[k] = true;
            doc.reveal |= typing;
            doc.blink_from = now;
        }
    }
    if used.iter().any(|u| *u) {
        ctx.input_mut(|i| {
            let mut k = 0;
            i.events.retain(|_| {
                let keep = !used.get(k).copied().unwrap_or(false);
                k += 1;
                keep
            });
        });
    }
}

/// The request a key makes in Edit mode, as 한글 and Word have them; `None` for a key left to
/// others.
fn edit_key(key: Key, m: Modifiers) -> Option<Req> {
    let word = m.command;
    let extend = m.shift;
    let go = |to: Move| Some(Req::Move { to, extend });
    match key {
        Key::ArrowLeft => go(if word { Move::WordLeft } else { Move::Left }),
        Key::ArrowRight => go(if word { Move::WordRight } else { Move::Right }),
        Key::ArrowUp => go(Move::Up),
        Key::ArrowDown => go(Move::Down),
        Key::Home => go(if word { Move::DocStart } else { Move::LineStart }),
        Key::End => go(if word { Move::DocEnd } else { Move::LineEnd }),
        Key::PageUp => go(Move::PageUp),
        Key::PageDown => go(Move::PageDown),
        Key::Backspace => Some(Req::Erase { back: true, word }),
        Key::Delete => Some(Req::Erase { back: false, word }),
        Key::Enter if !m.command => Some(Req::Enter),
        Key::Tab if !m.command => Some(Req::Tab { back: m.shift }),
        Key::Escape => Some(Req::Collapse),
        Key::A if m.command => Some(Req::SelectAll),
        Key::B if m.command && !m.shift => Some(Req::Toggle(Toggle::Bold)),
        Key::I if m.command && !m.shift => Some(Req::Toggle(Toggle::Italic)),
        Key::U if m.command && !m.shift => Some(Req::Toggle(Toggle::Underline)),
        Key::L if m.command && !m.shift => Some(Req::Align("left")),
        Key::E if m.command && !m.shift => Some(Req::Align("center")),
        Key::R if m.command && !m.shift => Some(Req::Align("right")),
        Key::J if m.command && !m.shift => Some(Req::Align("justify")),
        Key::CloseBracket if m.command => Some(Req::Grow(true)),
        Key::OpenBracket if m.command => Some(Req::Grow(false)),
        _ => None,
    }
}

/// A key in View mode: the pages scroll as a PDF's do; Ctrl+A selects everything and Esc drops
/// the selection.
fn view_key(doc: &mut Doc, key: Key, m: Modifiers) -> Option<Go> {
    let plain = !m.command && !m.ctrl && !m.alt;
    match key {
        Key::Home if plain || m.command => Some(Go::Top),
        Key::End if plain || m.command => Some(Go::Bottom),
        Key::PageDown if plain => Some(Go::Screens(1.0)),
        Key::Space if plain && !m.shift => Some(Go::Screens(1.0)),
        Key::PageUp if plain => Some(Go::Screens(-1.0)),
        Key::Space if plain => Some(Go::Screens(-1.0)),
        Key::ArrowDown if plain => Some(Go::By(LINE_STEP)),
        Key::ArrowUp if plain => Some(Go::By(-LINE_STEP)),
        Key::A if m.command => {
            doc.send(Req::SelectAll);
            None
        }
        Key::Escape => {
            doc.send(Req::Collapse);
            None
        }
        _ => None,
    }
}

/// Puts what Copy took on the system's clipboard, formatted (HTML) where it takes it.
pub(super) fn publish(app: &mut SheetApp, ctx: &egui::Context, text: &str, html: Option<&str>) {
    let published = match (html, app.services.copy_html.as_mut()) {
        (Some(html), Some(copy)) => copy(html, text).is_ok(),
        _ => false,
    };
    if !published {
        ctx.copy_text(text.to_owned());
    }
}

/// Pastes `text`, formatted where the system's clipboard has HTML for it.
pub(super) fn paste(app: &mut SheetApp, ctx: &egui::Context, doc: &mut Doc, text: String) {
    let html = app.services.paste_html.as_mut().and_then(|read| read());
    doc.ime.changed(ctx, false);
    doc.send(Req::Paste { text, html });
    doc.reveal = true;
}

/// What a right click on the pages asked for, done by the caller (it has the clipboard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

/// The mouse over the pages (`view`, on the screen; pages from `origin` at `scale`): a press puts
/// the caret there (Shift extends the selection) and gives the pages the keyboard, a drag selects,
/// a double click a word, a triple click a paragraph; a right click offers Copy, Cut and Paste.
pub(super) fn pointer(
    ui: &mut Ui,
    doc: &mut Doc,
    (lay, view, origin, scale): (&Layout, Rect, Vec2, f32),
    (id, editing, now, lang): (egui::Id, bool, f64, crate::i18n::Language),
) -> Option<Menu> {
    let resp = ui.interact(view, id, Sense::click_and_drag());
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    // A place on the screen as a page and a point on it (points).
    let at = |pos: Pos2, nearest: bool| -> Option<(usize, f32, f32)> {
        let c = pos - origin;
        let page = lay.page_at(c).or_else(|| nearest.then(|| lay.nearest_page(c)).flatten())?;
        let r = lay.pages.get(page)?;
        let (x, y) = ((c.x - r.left()) / scale, (c.y - r.top()) / scale);
        Some((page, x.clamp(0.0, r.width() / scale), y.clamp(0.0, r.height() / scale)))
    };
    if resp.hovered() && pointer.is_some_and(|p| lay.page_at(p - origin).is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
        ui.memory_mut(|m| m.request_focus(id));
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some((page, x, y)) = pointer.and_then(|p| at(p, false)) {
            doc.ime.changed(ui.ctx(), false);
            doc.send(Req::Point { page, x, y, how: Point::Caret { extend: shift } });
            doc.dragging = true;
            doc.blink_from = now;
        }
    }
    if doc.dragging
        && resp.dragged()
        && let Some(p) = pointer
    {
        if resp.drag_delta() != Vec2::ZERO
            && let Some((page, x, y)) = at(p, true)
        {
            doc.send(Req::Point { page, x, y, how: Point::Caret { extend: true } });
        }
        // Past the view's top or bottom edge, the view follows.
        let edge = 24.0;
        let dy = if p.y > view.bottom() - edge {
            -(p.y - (view.bottom() - edge))
        } else if p.y < view.top() + edge {
            view.top() + edge - p.y
        } else {
            0.0
        };
        if dy != 0.0 {
            ui.scroll_with_delta(egui::vec2(0.0, dy.clamp(-60.0, 60.0)));
            ui.ctx().request_repaint();
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        doc.dragging = false;
    }
    let clicks = if resp.triple_clicked() {
        Some(Point::Paragraph)
    } else if resp.double_clicked() {
        Some(Point::Word)
    } else {
        None
    };
    if let (Some(how), Some((page, x, y))) = (clicks, pointer.and_then(|p| at(p, false))) {
        doc.send(Req::Point { page, x, y, how });
        doc.dragging = false;
    }
    let mut chosen = None;
    let selected = !doc.state.selection.is_empty();
    resp.context_menu(|ui| {
        let mut item = |ui: &mut Ui, label: &str, on: bool, what: Menu| {
            if ui.add_enabled(on, egui::Button::new(label)).clicked() {
                chosen = Some(what);
                ui.close();
            }
        };
        if editing {
            item(ui, lang.tr("Cut"), selected, Menu::Cut);
        }
        item(ui, lang.tr("Copy"), selected, Menu::Copy);
        if editing {
            item(ui, lang.tr("Paste"), true, Menu::Paste);
        }
        ui.separator();
        item(ui, lang.tr("Select All"), true, Menu::SelectAll);
    });
    chosen
}
