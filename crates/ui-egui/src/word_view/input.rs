//! The keyboard, the mouse and the clipboard over a Word document's pages.
//!
//! In Edit mode, once the pages have the keyboard (a click on them), keys go to WordCraft's own
//! key map — Word's shortcuts (Ctrl+B, Enter, Tab in lists and tables, Shift+arrows…) — typed text
//! is inserted as typed, and an input method's composition goes through [`super::ime`]. In View
//! mode the keys scroll as in a PDF, and text can still be selected and copied. Monun's own
//! shortcuts (Save, Open, Close, Undo, Find) are taken first, by the Markdown tab's handler.

use egui::{Event, ImeEvent, Key, Modifiers, Pos2, Rect, Sense, Ui, Vec2};
use serde_json::json;
use wordcraft_engine::doc::{Pos, StoryRef};

use super::{Doc, Go, Phase, Word, canvas_id};
use crate::SheetApp;
use crate::page_layout::Layout;

/// One arrow key's scroll in View mode (egui points).
const LINE_STEP: f32 = 48.0;
/// The longest composition kept to draw.
const MAX_PREEDIT_BYTES: usize = 4096;

/// The keys and the clipboard of the tab `uid`'s document; not while a field (Find, the page
/// number) or a dialog has the keyboard.
pub(super) fn keys(app: &mut SheetApp, ctx: &egui::Context, doc: &mut Doc, uid: u64, editing: bool) {
    if app.dialog.is_some() || app.message.is_some() || ctx.text_edit_focused() {
        return;
    }
    let typing = editing && ctx.memory(|m| m.has_focus(canvas_id(uid)));
    let now = ctx.input(|i| i.time);
    let events = ctx.input(|i| i.events.clone());
    let mut used = vec![false; events.len()];
    for (k, ev) in events.iter().enumerate() {
        let Phase::Ready(word) = &mut doc.phase else { return };
        let done = match ev {
            Event::Key { key, pressed: true, modifiers, .. } if typing => dispatch(word, *key, *modifiers),
            Event::Key { key, pressed: true, modifiers, .. } => {
                let go = view_key(word, *key, *modifiers);
                if let Some(go) = go {
                    doc.go = Some(go);
                }
                if go.is_some() {
                    used[k] = true;
                }
                continue;
            }
            Event::Text(text) if typing && !text.chars().any(char::is_control) => {
                word.run("text.insert", &json!({"text": text, "raw": true})).is_ok()
            }
            Event::Ime(ImeEvent::Preedit { text, .. }) if typing => {
                doc.ime.preedit = text.chars().take(MAX_PREEDIT_BYTES).collect();
                true
            }
            Event::Ime(ImeEvent::Commit(text)) if typing => {
                doc.ime.commit(&mut word.session, ctx, text);
                true
            }
            Event::Copy => copy(app, ctx, word, false),
            Event::Cut if typing => copy(app, ctx, word, true),
            Event::Paste(text) if typing => paste(app, word, text),
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

/// The key map's name for a key (WordCraft's own names).
fn key_name(k: Key) -> Option<&'static str> {
    Some(match k {
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Escape => "Escape",
        Key::Space => "Space",
        Key::Equals | Key::Plus => "=",
        Key::Minus => "-",
        Key::Period => ".",
        Key::Comma => ",",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::A => "A",
        Key::B => "B",
        Key::D => "D",
        Key::E => "E",
        Key::I => "I",
        Key::J => "J",
        Key::L => "L",
        Key::M => "M",
        Key::R => "R",
        Key::T => "T",
        Key::U => "U",
        _ => return None,
    })
}

/// A key with its modifiers as the key map writes shortcuts (`Mod+Shift+K`).
fn combo(m: Modifiers, key: &str, with_shift: bool) -> String {
    let mut s = String::new();
    if m.command {
        s.push_str("Mod+");
    }
    if m.ctrl && !m.command {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift && with_shift {
        s.push_str("Shift+");
    }
    s.push_str(key);
    s
}

/// The commands a key may run here: typing, moving and selecting, formatting, paragraphs and
/// tables. Others (dialogs, panes, mail merge…) have no place in this tab.
fn allowed(id: &str) -> bool {
    ["text.", "caret.", "select.", "format.", "para.", "table."].iter().any(|p| id.starts_with(p))
        && !matches!(id, "format.painter" | "format.fontDialog")
}

/// Runs the command a key is given in the key map; Shift with a key that moves the caret extends
/// the selection. Returns whether the key was used.
fn dispatch(word: &mut Word, key: Key, m: Modifiers) -> bool {
    let Some(name) = key_name(key) else { return false };
    let registry = word.session.registry.clone();
    if let Some(spec) = word.session.keymap.resolve(&registry, &combo(m, name, true)) {
        if !allowed(spec.id) {
            return false;
        }
        let _ = word.run(spec.id, &json!({}));
        return true;
    }
    if m.shift
        && let Some(spec) = word.session.keymap.resolve(&registry, &combo(m, name, false))
        && (spec.id.starts_with("caret.") || spec.id == "text.backspace")
    {
        let params = if spec.id.starts_with("caret.") { json!({"extend": true}) } else { json!({}) };
        let _ = word.run(spec.id, &params);
        return true;
    }
    false
}

/// A key in View mode: the pages scroll as a PDF's do; Ctrl+A selects everything and Esc drops
/// the selection.
fn view_key(word: &mut Word, key: Key, m: Modifiers) -> Option<Go> {
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
            let _ = word.run("select.all", &json!({}));
            None
        }
        Key::Escape => {
            let focus = word.session.sel.focus.clone();
            let _ = word.run("caret.set", &json!({"pos": focus}));
            None
        }
        _ => None,
    }
}

/// Copies (or cuts) the selection, formatted (HTML) where the system's clipboard takes it.
pub(super) fn copy(app: &mut SheetApp, ctx: &egui::Context, word: &mut Word, cut: bool) -> bool {
    if word.session.sel.is_collapsed() {
        return false;
    }
    let Ok(got) = word.run(if cut { "edit.cut" } else { "edit.copy" }, &json!({})) else { return false };
    let text = got.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_owned();
    let html = got.get("html").and_then(|h| h.as_str());
    let published = match (html, app.services.copy_html.as_mut()) {
        (Some(html), Some(copy)) => copy(html, &text).is_ok(),
        _ => false,
    };
    if !published {
        ctx.copy_text(text);
    }
    true
}

/// Pastes `text`, formatted where the system's clipboard has HTML for it.
pub(super) fn paste(app: &mut SheetApp, word: &mut Word, text: &str) -> bool {
    let html = app.services.paste_html.as_mut().and_then(|read| read());
    let params = match html {
        Some(html) => json!({"text": text, "html": html}),
        None => json!({"text": text}),
    };
    word.run("edit.paste", &params).is_ok()
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
    word: &mut Word,
    (lay, view, origin, scale): (&Layout, Rect, Vec2, f32),
    (id, editing, now, lang): (egui::Id, bool, f64, crate::i18n::Language),
) -> Option<Menu> {
    let resp = ui.interact(view, id, Sense::click_and_drag());
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let hit = |word: &Word, pos: Pos2, nearest: bool| -> Option<Pos> {
        let c = pos - origin;
        let page = lay.page_at(c).or_else(|| nearest.then(|| lay.nearest_page(c)).flatten())?;
        let r = lay.pages.get(page)?;
        word.layout.hit(page, (c.x - r.left()) / scale, (c.y - r.top()) / scale, StoryRef::Body)
    };
    if resp.hovered() && pointer.is_some_and(|p| lay.page_at(p - origin).is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
        ui.memory_mut(|m| m.request_focus(id));
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(at) = pointer.and_then(|p| hit(word, p, false)) {
            let _ = word.run("caret.set", &json!({"pos": at, "extend": shift}));
            doc.drag = Some(at);
            doc.blink_from = now;
        }
    }
    if doc.drag.is_some()
        && resp.dragged()
        && let Some(p) = pointer
    {
        if let Some(at) = hit(word, p, true) {
            let _ = word.run("caret.set", &json!({"pos": at, "extend": true}));
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
        doc.drag = None;
    }
    if resp.triple_clicked() {
        let _ = word.run("select.paragraph", &json!({"cell": true}));
        doc.drag = None;
    } else if resp.double_clicked() {
        let _ = word.run("select.word", &json!({}));
        doc.drag = None;
    }
    let mut chosen = None;
    resp.context_menu(|ui| {
        let selected = !word.session.sel.is_collapsed();
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
