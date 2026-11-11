//! Memos on Markdown files: a note on a paragraph, made with a right click on it in the rendered
//! document (View, or Side by Side's right half), listed in the memo panel and marked beside the
//! paragraph. The memo panel shows the file in front's memos (a workbook's: its sheet's comments),
//! then those of the group's other files, each file's under its name; a click on one goes there.
//! They are 모눈's own, kept by the file's path, not in the .md: a work document stays as it is.
//! A file under the working root folder keeps its memos in the root ([`root_file`]), so they go to
//! other PCs with it (git); any other with the interface's settings ([`crate::UiState::memos`]).
//! A memo remembers its paragraph's line and first words, and follows the paragraph as the text
//! around it changes; one whose paragraph is gone shows first in the panel, as lost.

pub mod root_file;

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use monun_engine::docs::TextKind;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// One memo: the file it is on, where (the paragraph's first line, 0-based, and its first words),
/// what it says and when it was written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocMemo {
    pub id: u64,
    pub path: String,
    pub line: usize,
    pub quote: String,
    pub text: String,
    /// `YYYY-MM-DD HH:MM`, local time.
    pub created: String,
}

/// The memo being written (`id` none) or changed in the panel.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub id: Option<u64>,
    pub path: String,
    pub line: usize,
    pub quote: String,
    pub text: String,
    /// The text box takes the keyboard next frame.
    focus: bool,
}

/// The memo panel's interface state.
#[derive(Default)]
pub struct State {
    pub draft: Option<Draft>,
    /// The memo picked last (its mark clicked, or its paragraph gone to), drawn outlined.
    pub picked: Option<u64>,
    /// A memo whose Delete waits for a second click.
    deleting: Option<u64>,
}

/// The most characters of a paragraph a memo keeps to find it again.
const QUOTE_MAX: usize = 60;
/// The longest memo.
const TEXT_MAX: usize = 10_000;

/// Line `line` of `text` (without its line break), when the text has it.
fn line_text(text: &str, line: usize) -> Option<&str> {
    text.split('\n').nth(line).map(|l| l.trim_end_matches('\r'))
}

/// The words a memo finds its paragraph again by: the line's text, trimmed, at most
/// [`QUOTE_MAX`] characters.
pub fn quote_of(text: &str, line: usize) -> String {
    line_text(text, line).unwrap_or_default().trim().chars().take(QUOTE_MAX).collect()
}

/// Where a memo's paragraph is in `text` now: still on `line`, or else the line starting with its
/// words nearest to it; `None` once no line does.
pub fn locate(text: &str, line: usize, quote: &str) -> Option<usize> {
    let quote = quote.trim();
    if quote.is_empty() {
        return None;
    }
    if line_text(text, line).is_some_and(|l| l.trim().starts_with(quote)) {
        return Some(line);
    }
    text.split('\n').enumerate().filter(|(_, l)| l.trim().starts_with(quote)).map(|(i, _)| i).min_by_key(|i| i.abs_diff(line))
}

/// Now, as a memo's date: `YYYY-MM-DD HH:MM` in local time.
fn stamp() -> String {
    use monun_engine::core::date;
    let Some(ms) = date::unix_now_ms() else { return String::new() };
    let local = ms + date::local_offset_ms();
    let day = (local / 86_400_000.0).floor();
    let (y, m, d) = date::civil_from_days(day as i64);
    let minutes = ((local - day * 86_400_000.0) / 60_000.0).floor() as i64;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", minutes / 60, minutes % 60)
}

/// The file the document in front is, when memos can go on it: a saved Markdown file. `Err(true)`
/// for a Markdown file not saved yet (it has no path to keep memos by), `Err(false)` for anything
/// else (a workbook's document tab, a SQL or text file).
pub(crate) fn memo_file(app: &SheetApp) -> Result<String, bool> {
    let Some(d) = app.session.active().filter(|d| d.text_kind() == Some(TextKind::Markdown)) else { return Err(false) };
    d.path.clone().ok_or(true)
}

/// The text of the document in front as it is being typed.
fn live_text(app: &SheetApp) -> String {
    app.doc_view.draft.as_ref().map(|d| d.text.clone()).or_else(|| crate::doc_view::active_document(app).map(|(_, _, t)| t)).unwrap_or_default()
}

/// Writes the interface's settings now (the desktop's `ui.json`), so a memo outlasts a crash.
fn persist(app: &SheetApp) {
    if let Some(save) = &app.services.save_ui {
        save(&app.ui);
    }
}

/// Whether two paths are the same file's: regardless of case and of / or \ on Windows.
fn same_path(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    let norm = |c: char| if c == '/' { '\\' } else { c };
    a.chars().map(norm).flat_map(char::to_lowercase).eq(b.chars().map(norm).flat_map(char::to_lowercase))
}

/// Every memo: `ui.json`'s, then the root's file's.
fn all(app: &SheetApp) -> impl Iterator<Item = &DocMemo> {
    app.ui.memos.iter().chain(app.root_memos.iter().flat_map(|s| s.memos.iter()))
}

/// The id after every memo's.
fn next_id(app: &SheetApp) -> u64 {
    all(app).map(|m| m.id).max().map_or(1, |n| n.saturating_add(1))
}

/// Keeps a new memo: in the root's file for a file under the working root, else (or when that
/// file can't be read or written) in `ui.json`. Returns its id.
fn keep_new(app: &mut SheetApp, path: String, line: usize, quote: String, text: String) -> u64 {
    let memo = DocMemo { id: next_id(app), path, line, quote, text, created: stamp() };
    match root_file::add(app, memo) {
        Ok(id) => id,
        Err(mut memo) => {
            // The root's file may have brought ids since.
            memo.id = next_id(app);
            let id = memo.id;
            app.ui.memos.push(memo);
            persist(app);
            id
        }
    }
}

/// Deletes the memo `id` on `path`, wherever it is kept.
fn delete(app: &mut SheetApp, id: u64, path: &str) {
    let before = app.ui.memos.len();
    app.ui.memos.retain(|m| !(m.id == id && same_path(&m.path, path)));
    if app.ui.memos.len() != before {
        persist(app);
    } else if let Err(e) = root_file::delete(app, id, path) {
        root_file::tell(app, root_file::COULD_NOT_SAVE, &e);
    }
}

/// Starts a memo on line `line` of the file in front (Add Memo on a paragraph): the memo panel
/// opens with a box to write it in.
pub(crate) fn start(app: &mut SheetApp, line: usize) {
    let Ok(path) = memo_file(app) else { return };
    // A memo half written is kept, not dropped.
    if app.memos.draft.as_ref().is_some_and(|d| !d.text.trim().is_empty()) {
        save_draft(app);
        // One that couldn't be saved stays in its box (a message says why).
        if app.memos.draft.is_some() {
            show_panel(app);
            return;
        }
    }
    let quote = quote_of(&live_text(app), line);
    app.memos.draft = Some(Draft { id: None, path, line, quote, text: String::new(), focus: true });
    show_panel(app);
}

/// Opens the memo panel (in place of any other pane).
fn show_panel(app: &mut SheetApp) {
    app.grid.pane = None;
    app.ui.group_panel = false;
    if !app.session.comments_pane {
        app.run_or_alert("review.showComments", json!({"on": true}));
    }
}

/// The memos of `path` and where each paragraph is in `text` now (`None`: gone), lost ones first,
/// then in the document's order. A memo whose paragraph moved keeps its new line.
pub(crate) fn placed(app: &mut SheetApp, path: &str, text: &str) -> Vec<(DocMemo, Option<usize>)> {
    root_file::ensure(app);
    let mut out = Vec::new();
    let shared = app.root_memos.iter_mut().flat_map(|s| s.memos.iter_mut());
    for m in app.ui.memos.iter_mut().chain(shared).filter(|m| same_path(&m.path, path)) {
        let at = locate(text, m.line, &m.quote);
        if let Some(line) = at {
            m.line = line;
        }
        out.push((m.clone(), at));
    }
    out.sort_by_key(|(m, at)| (at.is_some(), at.unwrap_or(0), m.id));
    out
}

/// What the panel asks for, done once it is drawn.
enum Act {
    Go(u64, usize),
    Edit(DocMemo),
    Delete(u64, String),
    Save,
    Cancel,
}

/// The memo panel: the file in front's memos (a workbook's: its sheet's comments), then the group's
/// other files'.
pub(crate) fn panel(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if let Some(name) = app.session.active().map(|d| d.display_title()) {
        file_header(ui, &t, &name, None, true);
    }
    if crate::doc_view::is_active(app) {
        this_document(app, ui);
    } else {
        crate::panes::comments(app, ui);
    }
    group(app, ui, &t);
}

/// A file's name over its memos: its icon, its name (the file in front's in the accent colour) and
/// how many memos it has.
fn file_header(ui: &mut Ui, t: &Tokens, name: &str, count: Option<usize>, front: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let icon = Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(14.0, 14.0));
    icons::paint(ui.painter(), icon, crate::file_tabs::path_icon(name), if front { t.accent } else { t.text_dim });
    let small = theme::ui_font(11.5);
    let count = count.map(|n| n.to_string()).unwrap_or_default();
    let count_w = ui.painter().layout_no_wrap(count.clone(), small.clone(), t.text_dim).size().x;
    ui.painter().text(pos2(r.right() - 2.0, r.center().y), Align2::RIGHT_CENTER, &count, small, t.text_dim);
    let font = theme::ui_bold(12.5);
    let shown = crate::file_tabs::fit(ui, name, &font, (r.right() - icon.right() - count_w - 14.0).max(20.0));
    ui.painter().text(pos2(icon.right() + 6.0, r.center().y), Align2::LEFT_CENTER, shown, font, if front { t.text } else { t.text_dim });
    ui.add_space(2.0);
}

/// A memo of another file of the group, as the panel lists it.
#[derive(Clone, Debug, PartialEq)]
enum Other {
    /// A Markdown file's memo; `line` none when its paragraph is gone.
    Memo { id: u64, line: Option<usize>, quote: String, text: String, created: String },
    /// A workbook's cell comment.
    Comment { sheet: usize, cell: String, place: String, author: String, text: String },
}

/// The most memos a file of the group lists.
const OTHERS_MAX: usize = 100;

/// The group's other files with memos (the tabs other than the one in front, in tab order): each
/// one's document uid, name and memos. SQL and text files have none.
fn others(app: &mut SheetApp) -> Vec<(u64, String, Vec<Other>)> {
    let active = app.session.active().map(|d| d.uid);
    let mut uids: Vec<u64> = Vec::new();
    for w in app.session.windows() {
        if Some(w.doc_uid) != active && !uids.contains(&w.doc_uid) {
            uids.push(w.doc_uid);
        }
    }
    let mut out = Vec::new();
    for uid in uids {
        let Some(d) = app.session.documents().iter().find(|d| d.uid == uid) else { continue };
        let name = d.display_title();
        match d.text_kind() {
            Some(TextKind::Markdown) => {
                let Some(path) = d.path.clone() else { continue };
                let text = d.wb.sheet(0).and_then(|sh| sh.document.as_ref()).map(|doc| doc.text.clone()).unwrap_or_default();
                let items: Vec<Other> = placed(app, &path, &text)
                    .into_iter()
                    .map(|(m, line)| Other::Memo { id: m.id, line, quote: m.quote, text: m.text, created: m.created })
                    .collect();
                out.push((uid, name, items));
            }
            None => {
                let mut items = Vec::new();
                for (i, sh) in d.wb.sheets.iter().enumerate() {
                    for (cell, c) in sh.comments.iter() {
                        let place = format!("{}!{}", sh.name, cell.a1());
                        items.push(Other::Comment { sheet: i, cell: cell.a1(), place, author: c.author.clone(), text: c.text.clone() });
                    }
                }
                out.push((uid, name, items));
            }
            Some(_) => {}
        }
    }
    out.retain(|(_, _, items)| !items.is_empty());
    for (_, _, items) in &mut out {
        items.truncate(OTHERS_MAX);
    }
    out
}

/// The group's other files' memos, under their names; a click on one goes there.
fn group(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let files = others(app);
    if files.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.separator();
    ui.label(egui::RichText::new(lang.tr("Other files of the group")).font(theme::ui_font(12.0)).color(t.text_dim));
    ui.add_space(4.0);
    let mut chosen = None;
    for (uid, name, items) in &files {
        file_header(ui, t, name, Some(items.len()), false);
        for item in items {
            let (head, body, lost) = match item {
                Other::Memo { line, quote, text, created, .. } => (format!("“{}”  {created}", plain_quote(quote)), text.clone(), line.is_none()),
                Other::Comment { place, author, text, .. } => (format!("{place}  {author}"), text.clone(), false),
            };
            let resp = egui::Frame::new()
                .fill(t.window)
                .stroke(Stroke::new(1.0, t.ribbon_border))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut job = egui::text::LayoutJob::default();
                    if lost {
                        let lost = format!("{}  ", lang.tr("Paragraph not found:"));
                        job.append(&lost, 0.0, egui::TextFormat::simple(theme::ui_font(11.0), t.danger));
                    }
                    job.append(&head, 0.0, egui::TextFormat::simple(theme::ui_font(11.0), t.text_dim));
                    job.wrap.max_width = ui.available_width();
                    job.wrap.max_rows = 1;
                    ui.add(egui::Label::new(job).selectable(false));
                    let mut text = egui::text::LayoutJob::simple(body.clone(), theme::ui_font(12.5), t.text, ui.available_width());
                    text.wrap.max_rows = 3;
                    ui.add(egui::Label::new(text).selectable(false));
                })
                .response
                .interact(Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &body));
            if resp.on_hover_text(lang.tr("Go there")).clicked() {
                chosen = Some((*uid, item.clone()));
            }
            ui.add_space(4.0);
        }
        ui.add_space(4.0);
    }
    if let Some((uid, item)) = chosen {
        go_to(app, uid, &item);
    }
}

/// Goes to another file's memo: its tab comes to the front, at the memo's paragraph (picked) or
/// on the comment's cell.
fn go_to(app: &mut SheetApp, uid: u64, item: &Other) {
    let Some(window) = app.session.doc_window(uid) else { return };
    app.run_or_alert("window.activate", json!({"window": window}));
    app.follow_active_window();
    if app.session.active().map(|d| d.uid) != Some(uid) {
        return;
    }
    match item {
        Other::Memo { id, line, .. } => {
            app.memos.picked = Some(*id);
            // The document shows its paragraph once it has been laid out.
            if let Some(line) = line {
                crate::doc_view::show_line_later(app, (uid, 0), *line);
            }
        }
        Other::Comment { sheet, cell, .. } => {
            app.run_or_alert("sheet.activate", json!({"sheet": sheet}));
            app.run_or_alert("selection.set", json!({"cell": cell}));
            app.grid.ensure_visible = true;
        }
    }
}

/// The memos of the Markdown file (or a workbook's document tab) in front.
fn this_document(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let path = match memo_file(app) {
        Ok(path) => path,
        Err(unsaved) => {
            let note = if unsaved { "Save the file to put memos on its paragraphs." } else { "No memos in this document yet." };
            ui.label(egui::RichText::new(lang.tr(note)).italics());
            return;
        }
    };
    let text = live_text(app);
    let memos = placed(app, &path, &text);
    ui.label(
        egui::RichText::new(lang.tr("Right-click a paragraph in View or Side by Side to put a memo on it."))
            .font(theme::ui_font(12.0))
            .color(t.text_dim),
    );
    ui.add_space(6.0);
    let mut act = None;
    if app.memos.draft.as_ref().is_some_and(|d| d.id.is_none() && d.path == path) {
        draft_card(app, ui, &t, &mut act);
    }
    if memos.is_empty() && app.memos.draft.is_none() {
        ui.label(egui::RichText::new(lang.tr("No memos in this document yet.")).italics());
    }
    for (m, at) in &memos {
        if app.memos.draft.as_ref().is_some_and(|d| d.id == Some(m.id)) {
            draft_card(app, ui, &t, &mut act);
            continue;
        }
        memo_card(app, ui, &t, m, *at, &mut act);
    }
    match act {
        Some(Act::Go(id, line)) => {
            app.memos.picked = Some(id);
            crate::doc_view::go_to_line(app, line);
        }
        Some(Act::Edit(m)) => {
            app.memos.draft = Some(Draft { id: Some(m.id), path: m.path, line: m.line, quote: m.quote, text: m.text, focus: true });
        }
        Some(Act::Delete(id, path)) => {
            app.memos.deleting = None;
            delete(app, id, &path);
        }
        Some(Act::Save) => save_draft(app),
        Some(Act::Cancel) => app.memos.draft = None,
        None => {}
    }
}

/// Keeps the memo being written: a new one, or the text of the one changed.
fn save_draft(app: &mut SheetApp) {
    let Some(d) = app.memos.draft.take() else { return };
    let text = d.text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let Some(id) = d.id else {
        app.memos.picked = Some(keep_new(app, d.path, d.line, d.quote, text));
        return;
    };
    if let Some(m) = app.ui.memos.iter_mut().find(|m| m.id == id && same_path(&m.path, &d.path)) {
        m.text = text;
        persist(app);
        return;
    }
    match root_file::edit(app, id, &d.path, &text) {
        Ok(true) => {}
        // Gone meanwhile (deleted on another PC): what was written is kept, as a new memo.
        Ok(false) => app.memos.picked = Some(keep_new(app, d.path, d.line, d.quote, text)),
        Err(e) => {
            root_file::tell(app, root_file::COULD_NOT_SAVE, &e);
            // The text stays in its box.
            app.memos.draft = Some(d);
        }
    }
}

/// A card's frame: picked ones outlined in the accent colour.
fn card(ui: &mut Ui, t: &Tokens, picked: bool, add: impl FnOnce(&mut Ui)) {
    let stroke = if picked { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.ribbon_border) };
    egui::Frame::new().fill(t.window).stroke(stroke).corner_radius(6.0).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(6.0);
}

/// A paragraph's words as the panel quotes them: without the Markdown marks its line starts with (a
/// heading's #, a list's - or 1., a task's [ ], a quote's >), a table row's cells between dots.
pub fn plain_quote(quote: &str) -> String {
    let mut s = quote.trim();
    loop {
        let before = s;
        s = s.trim_start_matches(['#', '>']).trim_start();
        for mark in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "- ", "* ", "+ "] {
            if let Some(rest) = s.strip_prefix(mark) {
                s = rest;
            }
        }
        let digits = s.chars().take_while(char::is_ascii_digit).count();
        if digits > 0
            && let Some(rest) = s.get(digits..).and_then(|r| r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")))
        {
            s = rest;
        }
        s = s.trim_start();
        if s == before {
            break;
        }
    }
    if s.starts_with('|') {
        s.trim_matches('|').split('|').map(str::trim).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" · ")
    } else {
        s.to_string()
    }
}

/// The paragraph a memo is on, quoted, or that it was lost.
fn quote_line(ui: &mut Ui, t: &Tokens, lang: crate::i18n::Language, quote: &str, lost: bool) -> egui::Response {
    let quote = plain_quote(quote);
    let shown = if quote.is_empty() { "…".to_string() } else { format!("“{quote}”") };
    let mut job = egui::text::LayoutJob::default();
    if lost {
        let lost = format!("{}  ", lang.tr("Paragraph not found:"));
        job.append(&lost, 0.0, egui::TextFormat::simple(theme::ui_font(11.5), t.danger));
    }
    job.append(&shown, 0.0, egui::TextFormat::simple(theme::ui_font(11.5), t.text_dim));
    job.wrap.max_width = ui.available_width();
    job.wrap.max_rows = 2;
    ui.add(egui::Label::new(job).sense(Sense::click()))
}

fn memo_card(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, m: &DocMemo, at: Option<usize>, act: &mut Option<Act>) {
    let lang = app.ui.language;
    let picked = app.memos.picked == Some(m.id);
    let deleting = app.memos.deleting == Some(m.id);
    let mut confirm = None;
    card(ui, t, picked, |ui| {
        let quote = quote_line(ui, t, lang, &m.quote, at.is_none());
        if let Some(line) = at {
            if quote.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if quote.on_hover_text(lang.tr("Go to the paragraph")).clicked() {
                *act = Some(Act::Go(m.id, line));
            }
        }
        ui.add_space(2.0);
        ui.add(egui::Label::new(egui::RichText::new(&m.text).font(theme::ui_font(13.0)).color(t.text)).wrap());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&m.created).font(theme::ui_font(11.0)).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if deleting {
                    if ui.small_button(lang.tr("Cancel")).clicked() {
                        confirm = Some(false);
                    }
                    if ui.small_button(egui::RichText::new(lang.tr("Delete")).color(t.danger)).clicked() {
                        *act = Some(Act::Delete(m.id, m.path.clone()));
                    }
                } else {
                    if ui.small_button(lang.tr("Delete")).clicked() {
                        confirm = Some(true);
                    }
                    if ui.small_button(lang.tr("Edit")).clicked() {
                        *act = Some(Act::Edit(m.clone()));
                    }
                }
            });
        });
    });
    match confirm {
        Some(true) => app.memos.deleting = Some(m.id),
        Some(false) => app.memos.deleting = None,
        None => {}
    }
}

/// The box a memo is written or changed in: Save (Ctrl+Enter) and Cancel (Esc).
fn draft_card(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, act: &mut Option<Act>) {
    let lang = app.ui.language;
    let Some(d) = app.memos.draft.as_mut() else { return };
    card(ui, t, true, |ui| {
        quote_line(ui, t, lang, &d.quote, false);
        ui.add_space(2.0);
        let id = egui::Id::unique("monun.memo_draft");
        // Taken before the box sees them (it would break the line, or let go of the keyboard).
        let (save_keys, cancel_keys) = if ui.memory(|m| m.has_focus(id)) {
            ui.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
        } else {
            (false, false)
        };
        let field = ui.add(
            egui::TextEdit::multiline(&mut d.text)
                .id(id)
                .char_limit(TEXT_MAX)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text(lang.tr("Write a memo"))
                .font(theme::ui_font(13.0)),
        );
        let field = field.accessible_name(lang.tr("Memo"));
        if std::mem::take(&mut d.focus) {
            field.request_focus();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can_save = !d.text.trim().is_empty();
            if ui.add_enabled(can_save, egui::Button::new(lang.tr("Save"))).on_hover_text("Ctrl+Enter").clicked() || (save_keys && can_save) {
                *act = Some(Act::Save);
            }
            if ui.button(lang.tr("Cancel")).clicked() || cancel_keys {
                *act = Some(Act::Cancel);
            }
        });
    });
}

/// The marks of the memos on the rendered document's paragraphs in view: a bar beside each and a
/// memo icon, in a column left of the page (whose left edge is `ui`'s), drawn from `lines` (each
/// drawn element's source line and place). The memos to mark are `marks` (memo id, line, text).
/// Returns a memo whose mark was clicked.
pub(crate) fn draw_marks(ui: &mut Ui, lines: &[(usize, Rect)], marks: &[(u64, usize, String)], picked: Option<u64>) -> Option<u64> {
    let left = ui.max_rect().left();
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let mut done: Vec<usize> = Vec::new();
    for (id, line, _) in marks {
        if done.contains(line) {
            continue;
        }
        done.push(*line);
        let Some(rect) = lines.iter().find(|(l, _)| l == line).map(|(_, r)| *r) else { continue };
        let here: Vec<&(u64, usize, String)> = marks.iter().filter(|(_, l, _)| l == line).collect();
        let on = here.iter().any(|(i, _, _)| Some(*i) == picked);
        let ink = if on { t.accent } else { MARK };
        let bar = Rect::from_min_max(pos2(left - 7.0, rect.top()), pos2(left - 4.0, rect.bottom()));
        ui.painter().rect_filled(bar, 1.0, ink);
        let icon = Rect::from_center_size(pos2(left - 20.0, rect.top() + 9.0), vec2(16.0, 16.0));
        let resp = ui.interact(icon.expand(2.0), ui.scope_id().with(("monun.memo_mark", line)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(icon.expand(2.0), 4.0, t.hover);
        }
        icons::paint(ui.painter(), icon, Icon::Comment, ink);
        if here.len() > 1 {
            ui.painter().text(icon.right_bottom(), Align2::CENTER_CENTER, here.len().to_string(), theme::ui_bold(9.0), ink);
        }
        let tip: Vec<&str> = here.iter().map(|(_, _, text)| text.as_str()).collect();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Memo"));
        if resp.on_hover_text(tip.join("\n\n")).clicked() {
            clicked = Some(*id);
        }
    }
    clicked
}

/// The memo marks' colour: an amber that reads on light and dark pages.
const MARK: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x8E, 0x04);

/// The memos to mark on the document in front (memo id, line, text), and the one picked: none
/// for a document without memos.
pub(crate) fn marks_of(app: &mut SheetApp) -> Vec<(u64, usize, String)> {
    let Ok(path) = memo_file(app) else { return Vec::new() };
    root_file::ensure(app);
    if !all(app).any(|m| same_path(&m.path, &path)) {
        return Vec::new();
    }
    let text = live_text(app);
    placed(app, &path, &text).into_iter().filter_map(|(m, at)| Some((m.id, at?, m.text))).collect()
}

/// A mark clicked: the memo panel opens with the memo picked.
pub(crate) fn pick(app: &mut SheetApp, id: u64) {
    app.memos.picked = Some(id);
    show_panel(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memo_follows_its_paragraph_and_is_lost_with_it() {
        let text = "# 제목\n\n첫 문단입니다.\n\n둘째 문단.\n";
        assert_eq!(quote_of(text, 2), "첫 문단입니다.");
        assert_eq!(locate(text, 2, "첫 문단입니다."), Some(2));
        // Lines added above: it moves down with the paragraph.
        let moved = "# 제목\n\n새 줄\n\n첫 문단입니다. 그리고 더.\n\n둘째 문단.\n";
        assert_eq!(locate(moved, 2, "첫 문단입니다."), Some(4));
        // Two candidates: the nearest wins.
        let twice = "첫 문단입니다.\n\n\n\n\n\n\n\n첫 문단입니다.\n";
        assert_eq!(locate(twice, 7, "첫 문단입니다."), Some(8));
        assert_eq!(locate("# 제목\n\n다른 글\n", 2, "첫 문단입니다."), None, "gone");
        assert_eq!(locate(text, 0, ""), None, "an empty line marks nothing");
        let long = "가".repeat(100);
        assert_eq!(quote_of(&long, 0).chars().count(), QUOTE_MAX);
        assert_eq!(locate(&format!("{long}\n"), 0, &quote_of(&long, 0)), Some(0));
    }

    #[test]
    fn the_panel_quotes_a_paragraph_without_its_marks() {
        assert_eq!(plain_quote("- 레거시와 새 시스템"), "레거시와 새 시스템");
        assert_eq!(plain_quote("## 1차 원칙"), "1차 원칙");
        assert_eq!(plain_quote("  3. 셋째"), "셋째");
        assert_eq!(plain_quote("- [ ] 할 일"), "할 일");
        assert_eq!(plain_quote("> - 인용 안 목록"), "인용 안 목록");
        assert_eq!(plain_quote("| 1 | 교육 수료증 | O |"), "1 · 교육 수료증 · O");
        assert_eq!(plain_quote("2026년 계획"), "2026년 계획", "a number that is no list mark stays");
    }

    #[test]
    fn a_memo_is_dated_in_local_time() {
        let s = stamp();
        assert_eq!(s.len(), "2026-11-05 14:06".len(), "{s}");
        assert_eq!((s.get(4..5), s.get(7..8), s.get(13..14)), (Some("-"), Some("-"), Some(":")));
    }

    #[test]
    fn a_files_memos_are_found_by_its_path_as_windows_compares_them() {
        assert!(same_path("/r/a/x.md", "/r/a/x.md"));
        assert!(!same_path("/r/a/x.md", "/r/a/y.md"));
        assert_eq!(same_path(r"C:\Docs\a\X.md", "c:/docs/A/x.md"), cfg!(windows), "regardless of case and slashes on Windows");
    }
}
