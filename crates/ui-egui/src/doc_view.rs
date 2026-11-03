//! Document tabs (`Sheet::document`): Markdown shown in place of the grid.
//!
//! View mode shows the rendered document; Edit mode puts the text on the left and the rendered
//! document beside it, with a divider to drag. What is typed stays in a draft that goes to the
//! workbook (`doc.setText`) when typing pauses, when the editor loses focus, and before anything
//! else reads the workbook (undo, a sheet switch, a save), so a burst of typing is one undo step.

mod diagram;
mod editor;
mod preview;

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Key, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};
use monun_markdown::Block;
use serde_json::json;

use crate::SheetApp;
use crate::clock::now_ms;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

pub use editor::{Format, continue_list, format};

/// How long typing must pause before the draft goes to the workbook (ms).
const IDLE_MS: f64 = 1500.0;
/// The widest the rendered document gets at 100% (px), so lines stay readable.
const PAGE_WIDTH: f32 = 860.0;

/// How a document tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The rendered document.
    View,
    /// The text on the left, the rendered document on the right.
    Edit,
}

/// Document tabs' UI state.
#[derive(Default)]
pub struct State {
    /// Text being typed into a document, not yet in its workbook.
    pub draft: Option<Draft>,
    /// The mode of each document shown so far, by (workbook, sheet).
    modes: HashMap<(u64, usize), Mode>,
    /// The editor's share of the width in Edit mode.
    split: Option<f32>,
    /// A line the editor's caret goes to next frame (a double-click in the preview).
    goto_line: Option<usize>,
    /// The editor takes the keyboard once the click that asked for it is over (a field gives
    /// up focus on a click elsewhere, and that click is elsewhere).
    focus_editor: bool,
    /// The last text parsed and its blocks.
    parsed: Option<(String, Arc<Vec<Block>>)>,
    /// Mermaid blocks drawn so far.
    diagrams: diagram::Cache,
    /// The `{{…}}` fields' values, for a workbook version (uid, revision) and a parse.
    fields: Option<((u64, u64, usize), Arc<Fields>)>,
    /// The outline (the headings) shows beside the document.
    outline: bool,
    /// Where the rendered document's top-level blocks start (first line, offset from the top of
    /// the scrolled page), as drawn last frame.
    positions: Vec<(usize, f32)>,
    /// A line the rendered document scrolls to next frame.
    scroll_to: Option<usize>,
    /// The editor's first visible line last frame; the rendered document follows it.
    editor_top: Option<usize>,
    /// Text the editor selects and shows next frame (char indices), for a found match.
    select: Option<(usize, usize)>,
    /// The find bar, when open.
    find: Option<Find>,
}

/// The find bar: what is looked for, what replaces it, and which match is current.
#[derive(Default)]
pub struct Find {
    pub query: String,
    pub replace: String,
    /// The replace field and buttons show.
    pub replacing: bool,
    current: usize,
    /// The query field takes the keyboard next frame.
    focus: bool,
}

/// The byte ranges of `query` in `text`, without overlaps; letters A–Z match either case.
pub fn find_all(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let mut at = 0;
    while at < text.len() {
        let hit = text.get(at..at + query.len()).is_some_and(|s| s.eq_ignore_ascii_case(query));
        if hit {
            out.push(at..at + query.len());
            at += query.len();
        } else {
            at += text.get(at..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        }
    }
    out
}

/// The offset of `line` in the rendered document, between the starts of the blocks around it.
fn offset_of_line(positions: &[(usize, f32)], line: usize) -> Option<f32> {
    let i = positions.partition_point(|(l, _)| *l <= line);
    let (l0, y0) = *positions.get(i.checked_sub(1)?)?;
    Some(match positions.get(i) {
        Some(&(l1, y1)) if l1 > l0 => y0 + (y1 - y0) * (line - l0) as f32 / (l1 - l0) as f32,
        _ => y0,
    })
}

/// Text typed into a document tab.
pub struct Draft {
    pub uid: u64,
    pub sheet: usize,
    pub text: String,
    /// The workbook's text the draft started from or was last written as.
    base: String,
    /// When the draft last changed (ms), while it holds typing the workbook doesn't have.
    changed_at: Option<f64>,
}

impl Draft {
    fn new(uid: u64, sheet: usize, text: &str) -> Draft {
        Draft { uid, sheet, text: text.to_owned(), base: text.to_owned(), changed_at: None }
    }
    /// Whether the draft holds text its workbook doesn't.
    pub fn is_dirty(&self) -> bool {
        self.text != self.base
    }
}

impl State {
    /// The mode `(uid, sheet)` shows in: as last chosen, else the one it first showed in, Edit for
    /// an empty document and View for one with text. It is kept, so a new document being typed or
    /// pasted into stays in Edit once its text reaches the workbook.
    pub fn mode(&mut self, uid: u64, sheet: usize, text: &str) -> Mode {
        *self.modes.entry((uid, sheet)).or_insert(if text.trim().is_empty() { Mode::Edit } else { Mode::View })
    }
    pub fn set_mode(&mut self, uid: u64, sheet: usize, mode: Mode) {
        self.modes.insert((uid, sheet), mode);
    }
    fn blocks(&mut self, text: &str) -> Arc<Vec<Block>> {
        match &self.parsed {
            Some((t, blocks)) if t == text => blocks.clone(),
            _ => {
                let blocks = Arc::new(monun_markdown::parse(text));
                self.parsed = Some((text.to_owned(), blocks.clone()));
                blocks
            }
        }
    }
}

/// What each `{{…}}` field of a document shows: its value as displayed, or its error.
pub type Fields = HashMap<String, Result<String, String>>;

/// The values of the fields in `blocks`, worked out again only when the workbook or the text
/// changed.
fn field_values(app: &mut SheetApp, blocks: &Arc<Vec<Block>>) -> Arc<Fields> {
    let Some(d) = app.session.active() else { return Arc::default() };
    let key = (d.uid, d.revision, Arc::as_ptr(blocks) as usize);
    if let Some((k, values)) = &app.doc_view.fields
        && *k == key
    {
        return values.clone();
    }
    let values: Arc<Fields> =
        Arc::new(monun_markdown::fields(blocks).into_iter().map(|e| (e.clone(), monun_engine::docs::field_value(&d.wb, &e))).collect());
    app.doc_view.fields = Some((key, values.clone()));
    values
}

/// The document editor's widget id.
pub fn editor_id() -> egui::Id {
    egui::Id::unique("monun.doc_editor")
}

/// The active workbook's uid, the active sheet and its text, when the active sheet is a document.
pub fn active_document(app: &SheetApp) -> Option<(u64, usize, String)> {
    let d = app.session.active()?;
    let i = d.wb.active_sheet();
    let doc = d.wb.sheet(i)?.document.as_ref()?;
    Some((d.uid, i, doc.text.clone()))
}

/// Whether the active sheet is a document tab.
pub fn is_active(app: &SheetApp) -> bool {
    app.session.active().and_then(|d| d.wb.active()).is_some_and(|sh| sh.is_document())
}

impl SheetApp {
    /// Writes the document draft to its workbook. A draft whose document went away is dropped; one
    /// the workbook refuses stays, and the reason comes back.
    pub(crate) fn commit_doc_draft(&mut self) -> Result<(), String> {
        let Some(dr) = self.doc_view.draft.as_mut() else { return Ok(()) };
        dr.changed_at = None;
        if !dr.is_dirty() {
            return Ok(());
        }
        let (uid, sheet, text) = (dr.uid, dr.sheet, dr.text.clone());
        let still_there = self
            .session
            .document_index(uid)
            .and_then(|i| self.session.documents().get(i))
            .and_then(|d| d.wb.sheet(sheet))
            .is_some_and(|sh| sh.is_document());
        if !still_there {
            self.doc_view.draft = None;
            return Ok(());
        }
        let r = self.session.with_target(monun_engine::Target::Doc(uid), |s| s.run("doc.setText", json!({"sheet": sheet, "text": text})));
        match r {
            Ok(Ok(_)) => {
                if let Some(dr) = self.doc_view.draft.as_mut() {
                    dr.base = text;
                }
                self.after_engine();
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Adds a document tab after the active sheet, or before sheet `before`, named in the
    /// interface's language (`문서1`).
    pub fn insert_document(&mut self, before: Option<usize>) {
        let prefix = self.ui.language.tr("Doc").to_owned();
        let params = match before {
            Some(i) => json!({"prefix": prefix, "before": i}),
            None => json!({"prefix": prefix}),
        };
        self.run_or_alert("doc.insert", params);
    }
}

impl SheetApp {
    /// Asks for a Markdown file and adds it as a document tab (before sheet `before`, else after
    /// the active one).
    pub fn import_markdown(&mut self, before: Option<usize>) {
        let Some(pick) = &self.services.pick_open else { return };
        let Some(path) = pick() else { return };
        let params = match before {
            Some(i) => json!({"path": path, "before": i}),
            None => json!({"path": path}),
        };
        self.run_or_alert("doc.import", params);
    }

    /// Asks for a picture, puts it on the document tab beside the caret's line (where Excel shows
    /// it, right of the text in column A) and writes `![name](image:N)` at the caret.
    pub fn insert_document_picture(&mut self, ctx: &egui::Context) {
        let Some(pick) = &self.services.pick_picture else { return };
        let Some(path) = pick() else { return };
        let stem = std::path::Path::new(&path).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("picture");
        let name: String = stem.chars().filter(|c| !matches!(c, '[' | ']')).collect();
        self.place_picture(ctx, ("path", path), Some(name));
    }

    /// A picture pasted with Ctrl+V (a screenshot, a copied image), as PNG.
    fn paste_document_picture(&mut self, ctx: &egui::Context, image: &egui::ColorImage) {
        match png(image) {
            Some(bytes) => self.place_picture(ctx, ("base64", monun_engine::io::base64_encode(&bytes)), None),
            None => self.alert(self.ui.language.tr("The picture on the clipboard can't be read.").to_string()),
        }
    }

    /// Puts a picture on the document tab, beside the caret's line, and its `![name](image:N)` at
    /// the caret on a line of its own. `source` is `insert.picture`'s path or base64; without a
    /// name it is called "Picture N".
    fn place_picture(&mut self, ctx: &egui::Context, source: (&str, String), name: Option<String>) {
        let Some(dr) = self.doc_view.draft.as_ref() else { return };
        let (a, b) = editor::selection(self, ctx);
        let (at, to) = (editor::byte_index(&dr.text, a), editor::byte_index(&dr.text, b));
        let (lead, trail) = picture_breaks(&dr.text, at, to);
        let line = monun_markdown::line_of(&dr.text, at) + lead.len();
        let (key, value) = source;
        match self.session.run("insert.picture", json!({key: value, "at": format!("C{}", line + 1)})) {
            Ok(r) => {
                self.after_engine();
                let id = r.get("image").and_then(serde_json::Value::as_u64).unwrap_or_default();
                let name = name.unwrap_or_else(|| format!("{} {id}", self.ui.language.tr("Picture")));
                editor::insert_at_caret(self, ctx, &format!("{lead}![{name}](image:{id}){trail}"));
            }
            Err(e) => self.alert(e),
        }
    }

    /// Asks where to save document tab `sheet` as Markdown, and writes it there.
    pub fn export_markdown(&mut self, sheet: usize) {
        if let Err(e) = self.commit_doc_draft() {
            self.alert(e);
            return;
        }
        let name = self.session.active().and_then(|d| d.wb.sheet(sheet)).map(|sh| sh.name.clone()).unwrap_or_else(|| "Doc".into());
        let Some(pick) = &self.services.pick_save else { return };
        let Some(path) = pick(&format!("{name}.md")) else { return };
        // The picker may suggest a workbook extension; the export writes Markdown.
        let path = std::path::Path::new(&path).with_extension("md").to_string_lossy().into_owned();
        match self.session.run("doc.export", json!({"sheet": sheet, "path": path})) {
            Ok(_) => self.toast = Some((format!("{} {path}", self.ui.language.tr("Saved:")), now_ms())),
            Err(e) => self.alert(e),
        }
    }
}

/// The line breaks to put before and after a picture that replaces bytes `at..to` of `text`, so it
/// stands as its own paragraph: a picture inside a line of text shows only its name.
fn picture_breaks(text: &str, at: usize, to: usize) -> (&'static str, &'static str) {
    let before = text.get(..at).unwrap_or("");
    let after = text.get(to..).unwrap_or("");
    let lead = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let trail = if after.is_empty() || after.starts_with("\n\n") {
        ""
    } else if after.starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    (lead, trail)
}

/// An image as PNG bytes.
fn png(image: &egui::ColorImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&rgba, u32::try_from(w).ok()?, u32::try_from(h).ok()?, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(bytes)
}

/// Starts, reloads or replaces the draft for the document shown. Returns whether it started one.
fn sync_draft(app: &mut SheetApp, ctx: &egui::Context, uid: u64, sheet: usize, text: &str) -> bool {
    if app.doc_view.draft.as_ref().is_some_and(|d| d.uid != uid || d.sheet != sheet)
        && let Err(e) = app.commit_doc_draft()
    {
        app.alert(e);
        app.doc_view.draft = None;
    }
    match app.doc_view.draft.as_mut() {
        None => {
            app.doc_view.draft = Some(Draft::new(uid, sheet, text));
            return true;
        }
        // The workbook's text changed under an untouched draft (undo, a script): take it, and
        // the editor's own undo history goes with the old text.
        Some(dr) if dr.base != text && !dr.is_dirty() => {
            *dr = Draft::new(uid, sheet, text);
            if let Some(mut state) = egui::TextEdit::load_state(ctx, editor_id()) {
                state.clear_undoer();
                state.store(ctx, editor_id());
            }
        }
        Some(_) => {}
    }
    false
}

/// Draws the active document tab: its tool bar, then the document in the chosen mode.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some((uid, sheet, text)) = active_document(app) else { return };
    let zoom = app.session.active().and_then(|d| d.wb.active()).map_or(100, |sh| sh.zoom);
    let z = f32::from(zoom.clamp(10, 400)) / 100.0;
    let started = sync_draft(app, ui.ctx(), uid, sheet, &text);
    let t = Tokens::get(ui.ctx());
    // The page follows the theme (the grid's stays white): the document's ink is the theme's.
    ui.painter().rect_filled(ui.max_rect(), 0.0, if t.dark { t.window } else { t.grid_bg });
    let mode = app.doc_view.mode(uid, sheet, &text);
    // An empty document opens ready to type.
    if started && mode == Mode::Edit && text.is_empty() {
        app.doc_view.focus_editor = true;
    }
    keyboard(app, ui, mode);
    let mode = toolbar(app, ui, (uid, sheet), mode, &t);
    if app.doc_view.find.is_some() {
        find_bar(app, ui, mode, &t);
    }
    let area = ui.available_rect_before_wrap();
    let body = if app.doc_view.outline {
        let side = Rect::from_min_max(area.min, pos2(area.left() + (area.width() * 0.25).clamp(140.0, 260.0), area.bottom()));
        outline(app, ui, side, mode, &t);
        Rect::from_min_max(pos2(side.right(), area.top()), area.max)
    } else {
        area
    };
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    match mode {
        Mode::View => view(app, &mut body_ui, z, &t),
        Mode::Edit => edit(app, &mut body_ui, z, &t),
    }
    ui.allocate_rect(area, Sense::hover());
    diagram::zoom_window(ui.ctx(), &mut app.doc_view.diagrams, app.ui.language);
    idle_commit(app, ui.ctx());
}

/// The headings of the document, indented by level; a click goes to one.
fn outline(app: &mut SheetApp, ui: &mut Ui, rect: Rect, mode: Mode, t: &Tokens) {
    let text = app.doc_view.draft.as_ref().map(|d| d.text.clone()).unwrap_or_default();
    let headings = headings(&app.doc_view.blocks(&text));
    ui.painter().rect_filled(rect, 0.0, t.ribbon);
    ui.painter().line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, t.ribbon_border));
    let mut side = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 8.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    side.label(egui::RichText::new(app.ui.language.tr("Contents")).font(theme::ui_bold(13.0)).color(t.text_dim));
    side.add_space(4.0);
    if headings.is_empty() {
        side.label(egui::RichText::new(app.ui.language.tr("Headings (# Title) show here.")).font(theme::ui_font(12.5)).color(t.text_dim));
    }
    let mut chosen = None;
    egui::ScrollArea::vertical().id_salt("monun.doc_outline").auto_shrink([false, false]).show(&mut side, |ui| {
        for (level, title, line) in &headings {
            ui.horizontal(|ui| {
                ui.add_space(f32::from(level.saturating_sub(1)) * 12.0);
                let font = if *level == 1 { theme::ui_bold(13.0) } else { theme::ui_font(13.0) };
                let resp = ui.add(egui::Label::new(egui::RichText::new(title).font(font).color(t.text)).sense(Sense::click()).truncate());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    chosen = Some(*line);
                }
            });
        }
    });
    if let Some(line) = chosen {
        app.doc_view.scroll_to = Some(line);
        if mode == Mode::Edit {
            app.doc_view.goto_line = Some(line);
            app.doc_view.focus_editor = true;
        }
    }
}

/// Every heading of `blocks` (in lists and quotes too): level, text, line.
fn headings(blocks: &[Block]) -> Vec<(u8, String, usize)> {
    fn walk(out: &mut Vec<(u8, String, usize)>, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Heading { level, text, line } => out.push((*level, text.iter().map(|s| s.text.as_str()).collect(), *line)),
                Block::List { items, .. } => items.iter().for_each(|i| walk(out, &i.blocks)),
                Block::Quote { blocks, .. } => walk(out, blocks),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&mut out, blocks);
    out
}

/// The find bar under the tool bar: what to look for, the count, previous and next, and with
/// Replace the replacement and its buttons.
fn find_bar(app: &mut SheetApp, ui: &mut Ui, mode: Mode, t: &Tokens) {
    let lang = app.ui.language;
    let text = app.doc_view.draft.as_ref().map(|d| d.text.clone()).unwrap_or_default();
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    ui.painter().rect_filled(bar, 0.0, t.ribbon);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.ribbon_border));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    let Some(find) = app.doc_view.find.as_mut() else { return };
    let query_id = egui::Id::unique("monun.doc_find");
    ui.label(egui::RichText::new(lang.tr("Find:")).font(theme::ui_font(13.0)));
    let q = ui.add(egui::TextEdit::singleline(&mut find.query).id(query_id).desired_width(200.0).hint_text(lang.tr("Text to find")));
    if std::mem::take(&mut find.focus) {
        q.request_focus();
    }
    let hits = find_all(&text, &find.query);
    if q.changed() {
        find.current = 0;
    }
    let enter = q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let back = ui.input(|i| i.modifiers.shift);
    let count = if find.query.is_empty() {
        String::new()
    } else if hits.is_empty() {
        lang.tr("No matches").to_owned()
    } else {
        format!("{}/{}", find.current.min(hits.len() - 1) + 1, hits.len())
    };
    ui.label(egui::RichText::new(count).font(theme::ui_font(12.5)).color(t.text_dim));
    let mut step: i64 = 0;
    let icon = |ui: &mut Ui, icon: Icon, tip: &str| {
        let resp = crate::widgets::icon_button(ui, icon, t.text_dim, lang.tr(tip), vec2(24.0, 24.0));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr(tip)));
        resp.clicked()
    };
    if icon(ui, Icon::ChevronUp, "Previous (Shift+Enter)") {
        step = -1;
    }
    if icon(ui, Icon::Chevron, "Next (Enter)") {
        step = 1;
    }
    if enter {
        step = if back { -1 } else { 1 };
        find.focus = true;
    }
    let mut replace_one = false;
    let mut replace_all = false;
    ui.add_space(8.0);
    if find.replacing {
        ui.label(egui::RichText::new(lang.tr("Replace with:")).font(theme::ui_font(13.0)));
        ui.add(egui::TextEdit::singleline(&mut find.replace).desired_width(160.0));
        replace_one = ui.button(lang.tr("Replace")).clicked();
        replace_all = ui.button(lang.tr("Replace All")).clicked();
    } else if ui.add(egui::Button::new(lang.tr("Replace…")).frame_when_inactive(false)).clicked() {
        find.replacing = true;
    }
    let close = icon(ui, Icon::Close, "Close (Esc)") || (q.has_focus() || q.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape));
    if close {
        app.doc_view.find = None;
        return;
    }
    if hits.is_empty() {
        return;
    }
    let n = hits.len();
    if step != 0 {
        let cur = find.current.min(n - 1) as i64;
        find.current = (cur + step).rem_euclid(n as i64) as usize;
        reveal(app, &text, &hits, mode);
    }
    if replace_one || replace_all {
        replace(app, &text, &hits, replace_all);
    }
}

/// Shows the current match: selected in the editor (Edit mode), scrolled to in the document.
fn reveal(app: &mut SheetApp, text: &str, hits: &[std::ops::Range<usize>], mode: Mode) {
    let Some(find) = &app.doc_view.find else { return };
    let Some(hit) = hits.get(find.current.min(hits.len().saturating_sub(1))) else { return };
    app.doc_view.scroll_to = Some(monun_markdown::line_of(text, hit.start));
    if mode == Mode::Edit {
        let chars = |b: usize| text.get(..b).map_or(0, |s| s.chars().count());
        app.doc_view.select = Some((chars(hit.start), chars(hit.end)));
    }
}

/// Replaces the current match, or every match, in the draft; Replace All is one undo step.
fn replace(app: &mut SheetApp, text: &str, hits: &[std::ops::Range<usize>], all: bool) {
    let Some(find) = &app.doc_view.find else { return };
    let with = find.replace.clone();
    let current = find.current.min(hits.len().saturating_sub(1));
    let mut out = text.to_owned();
    let chosen: Vec<&std::ops::Range<usize>> = if all { hits.iter().collect() } else { hits.get(current).into_iter().collect() };
    for hit in chosen.into_iter().rev() {
        if out.is_char_boundary(hit.start) && out.is_char_boundary(hit.end) && hit.end <= out.len() {
            out.replace_range(hit.clone(), &with);
        }
    }
    let Some(dr) = app.doc_view.draft.as_mut() else { return };
    dr.text = out;
    if let Err(e) = app.commit_doc_draft() {
        app.alert(e);
    }
}

/// Writes the draft once typing has paused, and wakes up for it before then.
fn idle_commit(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(at) = app.doc_view.draft.as_ref().and_then(|d| d.changed_at) else { return };
    let left = IDLE_MS - (now_ms() - at);
    if left <= 0.0 {
        if let Err(e) = app.commit_doc_draft() {
            app.alert(e);
        }
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(left.ceil() as u64));
    }
}

/// The workbook shortcuts a document tab keeps (the grid, which has the rest, isn't drawn).
/// Undo and redo are the editor's own while it has focus.
fn keyboard(app: &mut SheetApp, ui: &mut Ui, mode: Mode) {
    let ctx = ui.ctx().clone();
    if app.dialog.is_some() || app.message.is_some() || app.grid.tab_menu.is_some() || app.grid.renaming_tab.is_some() {
        return;
    }
    let editing = mode == Mode::Edit && ctx.memory(|m| m.focused()) == Some(editor_id());
    let cmd = Modifiers::COMMAND;
    let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut run: Option<&str> = None;
    ctx.input_mut(|i| {
        if i.consume_key(shift_cmd, Key::S) {
            run = Some("file.saveAs");
        } else if i.consume_key(cmd, Key::S) {
            run = Some("file.save");
        } else if i.consume_key(cmd, Key::O) {
            run = Some("file.open");
        } else if i.consume_key(cmd, Key::N) {
            run = Some("file.new");
        } else if i.consume_key(cmd, Key::W) {
            run = Some("file.close");
        } else if i.consume_key(cmd, Key::PageDown) {
            run = Some("sheet.next");
        } else if i.consume_key(cmd, Key::PageUp) {
            run = Some("sheet.previous");
        } else if !editing && (i.consume_key(shift_cmd, Key::Z) || i.consume_key(cmd, Key::Y)) {
            run = Some("edit.redo");
        } else if !editing && i.consume_key(cmd, Key::Z) {
            run = Some("edit.undo");
        }
    });
    let find = ctx.input_mut(|i| {
        if i.consume_key(cmd, Key::F) {
            Some(false)
        } else if i.consume_key(cmd, Key::H) {
            Some(true)
        } else {
            None
        }
    });
    if let Some(replacing) = find {
        let f = app.doc_view.find.get_or_insert_with(Find::default);
        f.replacing |= replacing;
        f.focus = true;
    }
    if let Some(id) = run {
        app.run_or_alert(id, json!({}));
        return;
    }
    // Ctrl+V with a picture and no text on the clipboard (a screenshot, a copied image) puts the
    // picture in the document: at the caret in Edit mode, at the end in View mode. Not while
    // another field (Find) has the focus.
    let mut pasted = None;
    let elsewhere = ctx.text_edit_focused() && ctx.memory(|m| m.focused()) != Some(editor_id());
    ctx.input_mut(|i| {
        if elsewhere {
            return;
        }
        i.events.retain(|e| match e {
            egui::Event::PasteImage(image) if pasted.is_none() => {
                pasted = Some(image.clone());
                false
            }
            _ => true,
        })
    });
    if let Some(image) = pasted {
        if mode == Mode::View {
            editor::caret_to_end(app, &ctx);
        }
        app.paste_document_picture(&ctx, &image);
    }
    if editing {
        let f = ctx.input_mut(|i| {
            if i.consume_key(cmd, Key::B) {
                Some(Format::Bold)
            } else if i.consume_key(cmd, Key::I) {
                Some(Format::Italic)
            } else {
                None
            }
        });
        if let Some(f) = f {
            editor::apply(app, &ctx, f);
        }
    }
}

/// The strip above the document: View and Edit, and in Edit mode the formatting buttons.
/// Returns the mode to draw this frame.
fn toolbar(app: &mut SheetApp, ui: &mut Ui, (uid, sheet): (u64, usize), mut mode: Mode, t: &Tokens) -> Mode {
    let lang = app.ui.language;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    ui.painter().rect_filled(bar, 0.0, t.ribbon);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.ribbon_border));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 4.0;
    for (m, icon, label) in [(Mode::View, Icon::Document, "View"), (Mode::Edit, Icon::Pen, "Edit")] {
        let on = mode == m;
        let text = egui::RichText::new(lang.tr(label)).font(theme::ui_font(13.0)).color(if on { t.accent } else { t.text });
        let (r, resp) = ui.allocate_exact_size(vec2(72.0, 28.0), Sense::click());
        if on {
            ui.painter().rect_filled(r, 4.0, t.pressed);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0)),
            icon,
            if on { t.accent } else { t.text_dim },
        );
        let galley = ui.painter().layout_no_wrap(text.text().to_owned(), theme::ui_font(13.0), if on { t.accent } else { t.text });
        ui.painter().galley(pos2(r.left() + 28.0, r.center().y - galley.size().y / 2.0), galley, t.text);
        let resp = resp.on_hover_text(lang.tr(if m == Mode::View { "Read the document" } else { "Edit the text beside the document" }));
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, lang.tr(label)));
        if resp.clicked() && !on {
            if let Err(e) = app.commit_doc_draft() {
                app.alert(e);
            }
            app.doc_view.set_mode(uid, sheet, m);
            mode = m;
            if m == Mode::Edit {
                app.doc_view.focus_editor = true;
            }
        }
    }
    let on = app.doc_view.outline;
    let outline =
        ui.add(egui::Button::selectable(on, egui::RichText::new(lang.tr("Contents")).font(theme::ui_font(13.0))).min_size(vec2(28.0, 26.0)));
    if outline.on_hover_text(lang.tr("Show the headings beside the document")).clicked() {
        app.doc_view.outline = !on;
    }
    let fit = app.ui.doc_fit_width;
    let fit_width =
        ui.add(egui::Button::selectable(fit, egui::RichText::new(lang.tr("Fit Width")).font(theme::ui_font(13.0))).min_size(vec2(28.0, 26.0)));
    if fit_width.on_hover_text(lang.tr("Show tables and diagrams as wide as the page")).clicked() {
        app.ui.doc_fit_width = !fit;
    }
    let find = ui.add(
        egui::Button::new(egui::RichText::new(lang.tr("Find")).font(theme::ui_font(13.0))).frame_when_inactive(false).min_size(vec2(28.0, 26.0)),
    );
    if find.on_hover_text(lang.tr("Find and replace in the document (Ctrl+F, Ctrl+H)")).clicked() {
        app.doc_view.find.get_or_insert_with(Find::default).focus = true;
    }
    if mode == Mode::Edit {
        ui.add_space(8.0);
        let (sep, _) = ui.allocate_exact_size(vec2(1.0, 20.0), Sense::hover());
        ui.painter().rect_filled(sep, 0.0, t.separator);
        ui.add_space(8.0);
        for (i, (f, label, tip)) in editor::BUTTONS.iter().enumerate() {
            // Groups: headings, character styles, lists and quotes, blocks.
            if editor::GROUP_STARTS.contains(&i) {
                ui.add_space(6.0);
                let (sep, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
                ui.painter().rect_filled(sep, 0.0, t.separator);
                ui.add_space(6.0);
            }
            let rich = match f {
                Format::Bold => egui::RichText::new(*label).font(theme::ui_bold(13.0)),
                Format::Italic => egui::RichText::new(*label).font(theme::ui_font(13.0)).italics(),
                Format::Strike => egui::RichText::new(*label).font(theme::ui_font(13.0)).strikethrough(),
                _ => egui::RichText::new(lang.tr(label)).font(theme::ui_font(13.0)),
            };
            if ui.add(egui::Button::new(rich).frame_when_inactive(false).min_size(vec2(28.0, 26.0))).on_hover_text(lang.tr(tip)).clicked() {
                editor::apply(app, &ui.ctx().clone(), *f);
            }
        }
        let diagrams = ui.add(
            egui::Button::new(egui::RichText::new(lang.tr("Diagram ▾")).font(theme::ui_font(13.0)))
                .frame_when_inactive(false)
                .min_size(vec2(28.0, 26.0)),
        );
        let diagrams = diagrams.on_hover_text(lang.tr("Insert a Mermaid diagram: flowchart, sequence, Gantt…"));
        egui::Popup::menu(&diagrams).show(|ui| {
            for (label, snippet) in editor::DIAGRAMS {
                if ui.button(lang.tr(label)).clicked() {
                    editor::apply(app, &ui.ctx().clone(), Format::Snippet(snippet));
                }
            }
        });
        if app.services.pick_picture.is_some() {
            let picture = ui.add(
                egui::Button::new(egui::RichText::new(lang.tr("Picture")).font(theme::ui_font(13.0)))
                    .frame_when_inactive(false)
                    .min_size(vec2(28.0, 26.0)),
            );
            if picture.on_hover_text(lang.tr("Insert a picture into the document")).clicked() {
                app.insert_document_picture(&ui.ctx().clone());
            }
        }
    }
    mode
}

/// View mode: the rendered document, centred.
fn view(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens) {
    let Some(text) = app.doc_view.draft.as_ref().map(|d| d.text.clone()) else { return };
    let blocks = app.doc_view.blocks(&text);
    let lang = app.ui.language;
    let fields = field_values(app, &blocks);
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
    let wb = app.session.active().map(|d| (d.uid, d.wb.clone()));
    let pictures = wb.as_ref().and_then(|(_, wb)| wb.active()).map_or(&[][..], |sh| sh.images.as_slice());
    let doc = wb.as_ref().map_or(0, |(uid, _)| *uid);
    let look = preview::Look { z, t, lang, fields: &fields, query: &query, doc, pictures, full: app.ui.doc_fit_width };
    let lead = 24.0 * z;
    let mut scroll = egui::ScrollArea::vertical().id_salt("monun.doc_view").auto_shrink([false, false]);
    if let Some(y) = app.doc_view.scroll_to.take().and_then(|line| offset_of_line(&app.doc_view.positions, line)) {
        scroll = scroll.vertical_scroll_offset((y + lead - 8.0 * z).max(0.0));
    }
    let (mut action, mut positions) = (None, Vec::new());
    scroll.show(ui, |ui| {
        let width = (PAGE_WIDTH * z).min(ui.available_width() - 48.0).max(120.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.add_space(lead);
                (action, positions) = preview::show(ui, &blocks, &look, &mut app.doc_view.diagrams);
                ui.add_space(48.0 * z);
            });
        });
    });
    app.doc_view.positions = positions;
    act(app, ui.ctx(), action, Mode::View);
}

/// Edit mode: the editor on the left, the divider, the rendered document on the right.
fn edit(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens) {
    let area = ui.available_rect_before_wrap();
    let split = app.doc_view.split.unwrap_or(0.5).clamp(0.2, 0.8);
    let mid = area.left() + area.width() * split;
    let left = Rect::from_min_max(area.min, pos2(mid - 3.0, area.bottom()));
    let divider = Rect::from_min_max(pos2(mid - 3.0, area.top()), pos2(mid + 3.0, area.bottom()));
    let right = Rect::from_min_max(pos2(mid + 3.0, area.top()), area.max);
    ui.painter().rect_filled(left, 0.0, t.input_bg);
    let handle = ui.interact(divider, ui.scope_id().with("monun.doc_split"), Sense::drag());
    if handle.hovered() || handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if handle.dragged()
        && let Some(p) = ui.ctx().pointer_interact_pos()
        && area.width() > 0.0
    {
        app.doc_view.split = Some(((p.x - area.left()) / area.width()).clamp(0.2, 0.8));
    }
    ui.painter()
        .line_segment([pos2(mid, area.top()), pos2(mid, area.bottom())], Stroke::new(1.0, if handle.hovered() { t.accent } else { t.ribbon_border }));

    let mut left_ui = ui.new_child(egui::UiBuilder::new().max_rect(left));
    let top = editor::show(app, &mut left_ui, z, t);
    // The rendered document follows the editor: scrolling the text brings its blocks into view.
    if top.is_some() && top != app.doc_view.editor_top {
        app.doc_view.scroll_to = app.doc_view.scroll_to.or(top);
    }
    app.doc_view.editor_top = top;

    let text = app.doc_view.draft.as_ref().map(|d| d.text.clone()).unwrap_or_default();
    let blocks = app.doc_view.blocks(&text);
    let lang = app.ui.language;
    let fields = field_values(app, &blocks);
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
    let wb = app.session.active().map(|d| (d.uid, d.wb.clone()));
    let pictures = wb.as_ref().and_then(|(_, wb)| wb.active()).map_or(&[][..], |sh| sh.images.as_slice());
    let doc = wb.as_ref().map_or(0, |(uid, _)| *uid);
    let look = preview::Look { z, t, lang, fields: &fields, query: &query, doc, pictures, full: app.ui.doc_fit_width };
    let lead = 16.0 * z;
    let mut right_ui = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(20.0, 0.0))));
    let mut scroll = egui::ScrollArea::vertical().id_salt("monun.doc_preview").auto_shrink([false, false]);
    if let Some(y) = app.doc_view.scroll_to.take().and_then(|line| offset_of_line(&app.doc_view.positions, line)) {
        scroll = scroll.vertical_scroll_offset(if y > 0.0 { y + lead - 8.0 * z } else { 0.0 });
    }
    let (mut action, mut positions) = (None, Vec::new());
    scroll.show(&mut right_ui, |ui| {
        ui.add_space(lead);
        (action, positions) = preview::show(ui, &blocks, &look, &mut app.doc_view.diagrams);
        ui.add_space(48.0 * z);
    });
    app.doc_view.positions = positions;
    ui.allocate_rect(area, Sense::hover());
    act(app, ui.ctx(), action, Mode::Edit);
}

/// Carries out what a click in the rendered document asked for.
fn act(app: &mut SheetApp, ctx: &egui::Context, action: Option<preview::Action>, mode: Mode) {
    match action {
        Some(preview::Action::ToggleTask(at)) => {
            let Some(dr) = app.doc_view.draft.as_mut() else { return };
            if let Some(text) = monun_markdown::toggle_task(&dr.text, at) {
                dr.text = text;
                if let Err(e) = app.commit_doc_draft() {
                    app.alert(e);
                }
            }
        }
        Some(preview::Action::Link(target)) => follow_link(app, ctx, &target),
        Some(preview::Action::Goto(line)) if mode == Mode::Edit => {
            app.doc_view.goto_line = Some(line);
            app.doc_view.focus_editor = true;
        }
        _ => {}
    }
}

/// Opens a link of the document: a web or mail address in the browser; `#Sheet!A1`, `#Sheet` or
/// `#Doc` in this workbook (the sheet, then the cell).
pub fn follow_link(app: &mut SheetApp, ctx: &egui::Context, target: &str) {
    let lower = target.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
        match &app.services.open_url {
            Some(open) => open(target),
            None => ctx.open_url(egui::OpenUrl::new_tab(target)),
        }
        return;
    }
    let Some(place) = target.strip_prefix('#') else { return };
    let (sheet, cell) = match place.rsplit_once('!') {
        Some((s, c)) => (s.trim_matches('\''), Some(c)),
        None => (place, None),
    };
    let Some(i) = app.session.active().and_then(|d| d.wb.sheet_index(sheet)) else {
        app.alert(format!("{}: {sheet}", app.ui.language.tr("No sheet or document by that name")));
        return;
    };
    if let Err(e) = app.commit_doc_draft() {
        app.alert(e);
        return;
    }
    app.run_or_alert("sheet.activate", json!({"sheet": i}));
    if let Some(cell) = cell {
        app.run_or_alert("selection.set", json!({"range": cell}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_matches_without_overlap_and_ascii_case() {
        assert_eq!(find_all("Total total TOTAL", "total"), [0..5, 6..11, 12..17]);
        assert_eq!(find_all("가계부 가계", "가계"), [0..6, 10..16]);
        assert_eq!(find_all("aaaa", "aa"), [0..2, 2..4]);
        assert!(find_all("abc", "").is_empty());
        assert!(find_all("", "x").is_empty());
    }

    #[test]
    fn lines_map_between_block_starts() {
        let positions = [(0, 0.0), (4, 100.0), (8, 300.0)];
        assert_eq!(offset_of_line(&positions, 0), Some(0.0));
        assert_eq!(offset_of_line(&positions, 2), Some(50.0));
        assert_eq!(offset_of_line(&positions, 6), Some(200.0));
        assert_eq!(offset_of_line(&positions, 20), Some(300.0));
        assert_eq!(offset_of_line(&[], 3), None);
        assert_eq!(offset_of_line(&[(5, 40.0)], 1), None);
    }

    #[test]
    fn a_picture_gets_a_paragraph_of_its_own() {
        // Mid-line: blank lines both sides; at a line's start or end only what is missing.
        assert_eq!(picture_breaks("one two", 3, 3), ("\n\n", "\n\n"));
        assert_eq!(picture_breaks("one\ntwo", 4, 4), ("\n", "\n\n"));
        assert_eq!(picture_breaks("one\ntwo", 3, 3), ("\n\n", "\n"));
        assert_eq!(picture_breaks("one\n\ntwo", 5, 5), ("", "\n\n"));
        assert_eq!(picture_breaks("# Plan\n", 7, 7), ("\n", ""));
        assert_eq!(picture_breaks("", 0, 0), ("", ""));
        // A selection is replaced: what is around it counts.
        assert_eq!(picture_breaks("a\n\nxyz\n\nb", 3, 6), ("", ""));
        assert_eq!(picture_breaks("abc", 9, 9), ("", ""));
    }

    #[test]
    fn pasted_pictures_become_png() {
        let image = egui::ColorImage::new([3, 2], vec![egui::Color32::from_rgb(30, 120, 200); 6]);
        let bytes = png(&image).expect("encodes");
        let back = image::load_from_memory(&bytes).expect("a PNG").to_rgba8();
        assert_eq!(back.dimensions(), (3, 2));
        assert_eq!(back.get_pixel(1, 1).0, [30, 120, 200, 255]);
    }
}
