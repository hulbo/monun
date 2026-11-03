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
    /// The mode `(uid, sheet)` shows in: as last chosen, else Edit for an empty document and View
    /// for one with text.
    pub fn mode(&self, uid: u64, sheet: usize, text: &str) -> Mode {
        self.modes.get(&(uid, sheet)).copied().unwrap_or(if text.trim().is_empty() { Mode::Edit } else { Mode::View })
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
    let values: Arc<Fields> = Arc::new(monun_markdown::fields(blocks).into_iter().map(|e| (e.clone(), field_value(&d.wb, &e))).collect());
    app.doc_view.fields = Some((key, values.clone()));
    values
}

/// What a field shows: a cell reference (`구매목록!D1`) as the cell displays it, in its number
/// format; anything else (`=SUM(구매목록!B:B)`) worked out as a formula, in General format.
pub fn field_value(wb: &monun_engine::model::Workbook, expr: &str) -> Result<String, String> {
    use monun_engine::core::{CellRef, Value};
    let e = expr.trim();
    let body = e.strip_prefix('=').unwrap_or(e);
    if !e.starts_with('=') {
        let (sheet, a1) = monun_engine::core::split_sheet(body);
        let index = match &sheet {
            Some(name) => wb.sheet_index(name),
            None => Some(wb.active_sheet()),
        };
        if let Some(sh) = index.and_then(|i| wb.sheet(i))
            && let Some(c) = CellRef::parse(&a1.replace('$', ""))
        {
            return Ok(monun_engine::display::cell_text(wb, sh, c));
        }
    }
    match monun_engine::calc::recalc::evaluate(wb, wb.active_sheet(), CellRef::default(), body) {
        Value::Error(e) => Err(e.to_string()),
        v => Ok(monun_engine::display::format(&v, "General", wb).text),
    }
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
    match mode {
        Mode::View => view(app, ui, z, &t),
        Mode::Edit => edit(app, ui, z, &t),
    }
    diagram::zoom_window(ui.ctx(), &mut app.doc_view.diagrams, app.ui.language);
    idle_commit(app, ui.ctx());
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
    if let Some(id) = run {
        app.run_or_alert(id, json!({}));
        return;
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
    }
    mode
}

/// View mode: the rendered document, centred.
fn view(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens) {
    let Some(text) = app.doc_view.draft.as_ref().map(|d| d.text.clone()) else { return };
    let blocks = app.doc_view.blocks(&text);
    let lang = app.ui.language;
    let fields = field_values(app, &blocks);
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("monun.doc_view").auto_shrink([false, false]).show(ui, |ui| {
        let width = (PAGE_WIDTH * z).min(ui.available_width() - 48.0).max(120.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.add_space(24.0 * z);
                action = preview::show(ui, &blocks, z, t, lang, &fields, &mut app.doc_view.diagrams);
                ui.add_space(48.0 * z);
            });
        });
    });
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
    editor::show(app, &mut left_ui, z, t);

    let text = app.doc_view.draft.as_ref().map(|d| d.text.clone()).unwrap_or_default();
    let blocks = app.doc_view.blocks(&text);
    let lang = app.ui.language;
    let fields = field_values(app, &blocks);
    let mut right_ui = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(20.0, 0.0))));
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("monun.doc_preview").auto_shrink([false, false]).show(&mut right_ui, |ui| {
        ui.add_space(16.0 * z);
        action = preview::show(ui, &blocks, z, t, lang, &fields, &mut app.doc_view.diagrams);
        ui.add_space(48.0 * z);
    });
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
