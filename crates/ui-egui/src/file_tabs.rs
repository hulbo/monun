//! Visual Studio-style file tabs ([`SheetApp::file_tabs`]): one window, every open file a tab at
//! the bottom, the active workbook's sheets in the All Sheets list beside them, and a start page
//! while no file is open.
//!
//! A tab is an engine window ([`monun_engine::Session::windows`]): View › New Window gives a
//! workbook a second tab (`Book1:2`), as Visual Studio's New Window does, and closing a tab is
//! `window.close`, which asks about unsaved changes when it is the workbook's last window.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use monun_engine::docs::TextKind;
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::ribbon::ComboResult;
use crate::theme::{self, Tokens};

/// The bar's height, the sheet tab strip's.
const BAR_H: f32 = 32.0;
/// The widest a tab's name is drawn; a longer one ends in "…" (the tooltip has all of it).
const NAME_MAX_W: f32 = 200.0;
/// The widest the All Sheets button's sheet name is drawn.
const SHEET_NAME_MAX_W: f32 = 160.0;
/// The open-files list (▾) and New file (+) buttons after the tabs, and the gaps around them.
const BUTTONS_W: f32 = 24.0 + 26.0 + 10.0;
/// The All Sheets value that adds a sheet in place of picking one.
const NEW_SHEET: &str = "+";

/// One file tab.
struct FileTab {
    window: u64,
    title: String,
    path: Option<String>,
    /// What the file holds: `None` for a workbook.
    kind: Option<TextKind>,
    dirty: bool,
    active: bool,
}

/// What a click on the bar asks for, done once the tabs are drawn.
enum TabAction {
    Activate(u64),
    Close(u64),
    CloseOthers(u64),
    CloseAll,
    /// A tab dragged to another place: the window and its place among the others.
    Move(u64, usize),
}

/// What a tab did this frame: a click's action, where it is, and whether it is being dragged or
/// was just dropped.
struct TabOut {
    action: Option<TabAction>,
    rect: Rect,
    dragging: bool,
    dropped: bool,
}

/// Ctrl+Tab and Ctrl+Shift+Tab show the next and previous file (in tab order, round the end),
/// and Ctrl+F4 closes the one in front, as in Visual Studio and Excel. Taken before anything else
/// sees them, so the document editor never types a tab for Ctrl+Tab.
pub(crate) fn keys(app: &mut SheetApp, ctx: &egui::Context) {
    if app.dialog.is_some() || app.backstage.is_some() {
        return;
    }
    let (back, next, close) = ctx.input_mut(|i| {
        let back = i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab);
        let next = i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab);
        (back, next, i.consume_key(egui::Modifiers::CTRL, egui::Key::F4))
    });
    let windows: Vec<u64> = app.session.windows().iter().map(|w| w.id).collect();
    let at = app.session.active_window().and_then(|a| windows.iter().position(|w| *w == a));
    if let Some(at) = at
        && (back || next)
        && windows.len() > 1
    {
        let n = windows.len();
        let to = if next { (at + 1) % n } else { (at + n - 1) % n };
        if let Some(w) = windows.get(to) {
            app.run_or_alert("window.activate", json!({"window": w}));
        }
    }
    if close && let Some(w) = app.session.active_window() {
        app.run_or_alert("window.close", json!({"window": w}));
    }
}

/// The tabs, one per window in the order they were opened.
fn file_tabs(app: &SheetApp) -> Vec<FileTab> {
    let active = app.session.active_window();
    app.session
        .windows()
        .iter()
        .filter_map(|w| {
            let d = app.session.documents().iter().find(|d| d.uid == w.doc_uid)?;
            Some(FileTab {
                window: w.id,
                title: app.session.window_title(w.id)?,
                path: d.path.clone(),
                kind: d.text_kind(),
                dirty: d.is_dirty(),
                active: active == Some(w.id),
            })
        })
        .collect()
}

/// `text` as wide as `max` at most, its end replaced with "…" when it is wider.
pub(crate) fn fit(ui: &Ui, text: &str, font: &egui::FontId, max: f32) -> String {
    let width = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE).size().x;
    if width(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        n -= 1;
        let cut: String = chars.iter().take(n).collect::<String>() + "…";
        if width(&cut) <= max {
            return cut;
        }
    }
    "…".into()
}

/// The bar at the bottom of the window: the file tabs, the open-files list, New file (+) and, for
/// a workbook, the All Sheets list and the grid's horizontal scrollbar.
pub fn bottom_bar(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("file_tabs").exact_size(BAR_H).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, t.ribbon_border));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(6.0);
            let (bar_left, bar_width) = (ui.cursor().left(), ui.available_width());
            let tabs = file_tabs(app);
            // A workbook's sheets and scrollbar follow the tabs; a Markdown file has neither.
            let workbook = app.session.active().filter(|d| !d.is_text_file()).map(|d| d.uid);
            let grid = workbook.is_some() && !crate::doc_view::is_active(app);
            let sheet_w = if workbook.is_some() { sheet_button_width(app, ui) + 14.0 } else { 0.0 };
            let fixed = BUTTONS_W + sheet_w;
            let bar_right =
                if grid { app.grid.rect.map_or(bar_left + bar_width, |r| r.right()).min(bar_left + bar_width) } else { bar_left + bar_width };
            let share = workbook.filter(|_| grid).and_then(|uid| app.grid.tab_shares.get(&uid).copied());
            let tabs_width = match share {
                Some(share) => bar_width * share - fixed,
                None if grid => (bar_width * 0.6 - fixed).min(bar_right - bar_left - fixed - crate::tabs::SPLIT_WIDTH - crate::tabs::MIN_TRACK),
                None => bar_width - fixed,
            }
            .max(60.0);
            let action = strip(app, ui, &tabs, tabs_width, &t);
            ui.add_space(4.0);
            if !tabs.is_empty() {
                open_files_list(app, ui, &tabs, &t);
            }
            new_file_menu(app, ui, &t);
            if let Some(uid) = workbook {
                ui.add_space(6.0);
                let (sep, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
                ui.painter().rect_filled(sep, 0.0, t.ribbon_border);
                ui.add_space(7.0);
                sheet_list(app, ui, &t);
                if grid {
                    ui.add_space(6.0);
                    let avail = ui.available_rect_before_wrap();
                    let right = app.grid.rect.map_or(avail.right(), |r| r.right()).min(avail.right());
                    let split_left = share.map_or(avail.left(), |share| {
                        (bar_left + bar_width * share).min(right - crate::tabs::SPLIT_WIDTH - crate::tabs::MIN_TRACK).max(avail.left())
                    });
                    let split = Rect::from_min_max(pos2(split_left, avail.top()), pos2(split_left + crate::tabs::SPLIT_WIDTH, avail.bottom()));
                    crate::tabs::tab_split_handle(app, ui, split, (bar_left, bar_width), right, uid, &t);
                    let track = Rect::from_min_max(pos2(split.right(), avail.center().y - 7.0), pos2(right, avail.center().y + 7.0));
                    crate::grid::horizontal_scrollbar(app, ui, track, &t);
                }
            }
            if let Some(action) = action {
                act(app, action);
            }
        });
    });
    crate::tabs::tab_menu(app, ui);
}

/// The file tabs in a strip at most `max_w` wide, scrolled by the wheel and to the active tab when
/// it changes. Returns what a click asked for.
fn strip(app: &mut SheetApp, ui: &mut Ui, tabs: &[FileTab], max_w: f32, t: &Tokens) -> Option<TabAction> {
    let mut action = None;
    let active = tabs.iter().find(|tab| tab.active).map(|tab| tab.window);
    let reveal_id = ui.scope_id().with("file_tabs_reveal");
    let reveal_key = egui::Id::unique((active, tabs.len(), max_w.to_bits()));
    let reveal = ui.ctx().data_mut(|data| {
        let changed = data.get_temp::<egui::Id>(reveal_id) != Some(reveal_key);
        data.insert_temp(reveal_id, reveal_key);
        changed
    });
    ui.allocate_ui_with_layout(vec2(max_w, BAR_H - 2.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let wheel = if ui.rect_contains_pointer(ui.max_rect()) {
            ui.input(|i| {
                let d = i.smooth_scroll_delta;
                if d.x.abs() > d.y.abs() { d.x } else { d.y }
            })
        } else {
            0.0
        };
        egui::ScrollArea::horizontal()
            .id_salt("file_tabs_horizontal")
            .auto_shrink([true, false])
            .min_scrolled_width(0.0)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                if wheel.abs() > 0.0 {
                    ui.scroll_with_delta(vec2(wheel, 0.0));
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let mut rects: Vec<(u64, Rect)> = Vec::with_capacity(tabs.len());
                    let mut dragged = None;
                    for tab in tabs {
                        let out = file_tab(app, ui, tab, reveal, t);
                        rects.push((tab.window, out.rect));
                        if out.action.is_some() {
                            action = out.action;
                        }
                        if out.dragging || out.dropped {
                            dragged = Some((tab.window, out.dropped));
                        }
                    }
                    if let Some((window, dropped)) = dragged
                        && let Some(moved) = drag_to(ui, &rects, window, dropped, t)
                    {
                        action = Some(moved);
                    }
                });
            });
    });
    action
}

/// A tab being dragged along the others: a line marks where it would go, and the drop moves it
/// there.
fn drag_to(ui: &Ui, rects: &[(u64, Rect)], window: u64, dropped: bool, t: &Tokens) -> Option<TabAction> {
    let x = ui.input(|i| i.pointer.interact_pos())?.x;
    let from = rects.iter().position(|(w, _)| *w == window)?;
    let others: Vec<Rect> = rects.iter().filter(|(w, _)| *w != window).map(|(_, r)| *r).collect();
    let to = others.iter().filter(|r| r.center().x < x).count();
    if to == from {
        return None;
    }
    if dropped {
        return Some(TabAction::Move(window, to));
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let at = match others.get(to) {
        Some(r) => r.left(),
        None => others.last().map_or(x, |r| r.right()),
    };
    let y = others.first().map_or(egui::Rangef::new(0.0, 0.0), |r| r.y_range());
    ui.painter().vline(at, y.shrink(3.0), Stroke::new(2.0, t.accent));
    None
}

/// One file tab: its file's icon, name and a close button (a dot while it has unsaved changes and
/// the pointer is elsewhere). A click shows the file, the middle button or ✕ closes it, and the
/// right button offers Close, Close Others, Close All and Copy Full Path.
fn file_tab(app: &SheetApp, ui: &mut Ui, tab: &FileTab, reveal: bool, t: &Tokens) -> TabOut {
    let lang = app.ui.language;
    let font = theme::ui_font(13.0);
    let name = fit(ui, &tab.title, &font, NAME_MAX_W);
    let tw = ui.painter().layout_no_wrap(name.clone(), font.clone(), t.text).size().x;
    let w = 10.0 + 16.0 + 6.0 + tw + 6.0 + 18.0 + 6.0;
    // Dragged along the strip, a tab takes another place.
    let (r, resp) = ui.allocate_exact_size(vec2(w, BAR_H - 2.0), Sense::click_and_drag());
    let (rect, dragging, dropped) = (r, resp.dragged(), resp.drag_stopped());
    if tab.active && reveal {
        resp.scroll_to_me(None);
    }
    let label = if tab.dirty { format!("{} ({})", tab.title, lang.tr("unsaved changes")) } else { tab.title.clone() };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, tab.active, &label));
    let hovered = ui.rect_contains_pointer(r);
    let r = r.shrink2(vec2(1.0, 1.0));
    if tab.active {
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.left(), r.top() - 2.0), r.max), 4.0, t.tab_active);
        ui.painter().rect_stroke(
            Rect::from_min_max(pos2(r.left(), r.top() - 2.0), r.max),
            4.0,
            Stroke::new(1.0, t.ribbon_border),
            StrokeKind::Inside,
        );
        let band = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 4.0), pos2(r.right() - 6.0, r.bottom() - 1.5));
        ui.painter().rect_filled(band, 1.0, t.accent);
    } else if hovered {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let mid = r.center().y - 1.0;
    let icon = Rect::from_center_size(pos2(r.left() + 9.0 + 8.0, mid), vec2(15.0, 15.0));
    let ink = if tab.active { t.text } else { t.text_dim };
    icons::paint(ui.painter(), icon, file_icon(tab.kind), if tab.active { t.accent } else { t.text_dim });
    ui.painter().text(pos2(icon.right() + 6.0, mid), Align2::LEFT_CENTER, &name, font, ink);
    // The close button, or the unsaved dot in its place.
    let x = Rect::from_center_size(pos2(r.right() - 6.0 - 9.0, mid), vec2(18.0, 18.0));
    let close = ui.interact(x, ui.scope_id().with(("file_tab_close", tab.window)), Sense::click());
    let close_label = lang.tr("Close");
    close.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, close_label));
    let mut action = None;
    if hovered || (tab.active && !tab.dirty) {
        if close.hovered() {
            ui.painter().rect_filled(x, 3.0, t.hover);
        }
        icons::paint(ui.painter(), x.shrink(4.5), Icon::Close, if close.hovered() { t.text } else { t.text_dim });
        if close.on_hover_text(close_label).clicked() {
            action = Some(TabAction::Close(tab.window));
        }
    } else if tab.dirty {
        ui.painter().circle_filled(x.center(), 3.5, ink);
    }
    let tip = tab.path.clone().unwrap_or_else(|| tab.title.clone());
    let resp = resp.on_hover_text(tip);
    if resp.middle_clicked() {
        action = Some(TabAction::Close(tab.window));
    } else if resp.clicked() && !tab.active && action.is_none() {
        action = Some(TabAction::Activate(tab.window));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(180.0);
        if ui.button(lang.tr("Close")).clicked() {
            action = Some(TabAction::Close(tab.window));
        }
        if ui.button(lang.tr("Close Other Tabs")).clicked() {
            action = Some(TabAction::CloseOthers(tab.window));
        }
        if ui.button(lang.tr("Close All Tabs")).clicked() {
            action = Some(TabAction::CloseAll);
        }
        if let Some(path) = &tab.path {
            ui.separator();
            if ui.button(lang.tr("Copy Full Path")).clicked() {
                ui.ctx().copy_text(path.clone());
            }
        }
    });
    TabOut { action, rect, dragging, dropped }
}

/// Does what a tab click asked for. Closing several tabs stops at the first one whose file asks
/// whether to save it (the question is answered first, as in Visual Studio).
fn act(app: &mut SheetApp, action: TabAction) {
    match action {
        TabAction::Activate(w) => app.run_or_alert("window.activate", json!({"window": w})),
        TabAction::Close(w) => app.run_or_alert("window.close", json!({"window": w})),
        TabAction::CloseOthers(keep) => close_each(app, Some(keep)),
        TabAction::CloseAll => close_each(app, None),
        TabAction::Move(w, to) => app.session.move_window(w, to),
    }
}

/// Closes every tab but `keep`, one after another, until one asks to be saved.
fn close_each(app: &mut SheetApp, keep: Option<u64>) {
    let windows: Vec<u64> = app.session.windows().iter().map(|w| w.id).filter(|w| Some(*w) != keep).collect();
    for w in windows {
        if app.session.window(w).is_none() {
            continue;
        }
        match app.run("window.close", json!({"window": w})) {
            Ok(r) if r.get("closed").and_then(serde_json::Value::as_bool) == Some(false) => return,
            Ok(_) => {}
            Err(e) => {
                app.alert(e);
                return;
            }
        }
    }
    if let Some(keep) = keep.filter(|k| app.session.window(*k).is_some()) {
        app.run_or_alert("window.activate", json!({"window": keep}));
    }
}

/// The list of every open file (▾), filtered by typing, for finding one among many tabs.
fn open_files_list(app: &mut SheetApp, ui: &mut Ui, tabs: &[FileTab], t: &Tokens) {
    let lang = app.ui.language;
    let button = crate::widgets::icon_button(ui, Icon::Chevron, t.text_dim, lang.tr("Open files"), vec2(24.0, 24.0));
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr("Open files")));
    let values: Vec<String> = tabs.iter().map(|tab| tab.window.to_string()).collect();
    let rows: Vec<(&str, String)> =
        values.iter().zip(tabs).map(|(v, tab)| (v.as_str(), if tab.dirty { format!("{} ●", tab.title) } else { tab.title.clone() })).collect();
    let current = tabs.iter().find(|tab| tab.active).map(|tab| tab.window.to_string()).unwrap_or_default();
    if let ComboResult::Picked(v) = crate::ribbon::editable_combo(ui, &button, "open_files", &current, &rows, false)
        && let Ok(w) = v.parse::<u64>()
        && Some(w) != app.session.active_window()
    {
        app.run_or_alert("window.activate", json!({"window": w}));
    }
}

/// A file's icon: a sheet for a workbook, a page for Markdown and plain text, a code window for SQL.
/// The icon of the file at `path`: a workbook's, or a text file's of the kind its name says.
pub(crate) fn path_icon(path: &str) -> Icon {
    let kind = match monun_engine::io::FileKind::from_path(path) {
        Some(
            monun_engine::io::FileKind::Markdown
            | monun_engine::io::FileKind::Text
            | monun_engine::io::FileKind::Pdf
            | monun_engine::io::FileKind::Word
            | monun_engine::io::FileKind::Hwp
            | monun_engine::io::FileKind::Image,
        ) => Some(TextKind::of_name(path)),
        _ => None,
    };
    file_icon(kind)
}

pub(crate) fn file_icon(kind: Option<TextKind>) -> Icon {
    match kind {
        None => Icon::Sheet,
        Some(TextKind::Sql) => Icon::SqlFile,
        Some(TextKind::Markdown) => Icon::MarkdownFile,
        Some(TextKind::Plain) => Icon::TextFile,
        Some(TextKind::Pdf) => Icon::Pdf,
        Some(TextKind::Word) => Icon::Word,
        Some(TextKind::Hwp) => Icon::Hwp,
        Some(TextKind::Image) => Icon::ImageFile,
    }
}

/// New file (+): a new workbook, Markdown, SQL or text file, or Open….
fn new_file_menu(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let button = crate::widgets::icon_button(ui, Icon::Plus, t.text_dim, lang.tr("New file"), vec2(26.0, 26.0));
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr("New file")));
    egui::Popup::menu(&button).show(|ui| {
        ui.set_min_width(200.0);
        if ui.button(lang.tr("New Excel Workbook")).clicked() {
            new_workbook(app);
        }
        if ui.button(lang.tr("New Markdown Document")).clicked() {
            new_markdown(app);
        }
        if ui.button(lang.tr("New SQL File")).clicked() {
            new_text(app, TextKind::Sql);
        }
        if ui.button(lang.tr("New Text File")).clicked() {
            new_text(app, TextKind::Plain);
        }
        if cfg!(feature = "word") && ui.button(lang.tr("New Word Document")).clicked() {
            new_text(app, TextKind::Word);
        }
        if cfg!(feature = "hwp") && ui.button(lang.tr("New Hangul Document")).clicked() {
            new_text(app, TextKind::Hwp);
        }
        ui.separator();
        if ui.button(lang.tr("Open…")).clicked() {
            app.run_or_alert("file.open", json!({}));
        }
    });
}

fn new_workbook(app: &mut SheetApp) {
    app.run_or_alert("file.new", json!({}));
}

pub(crate) fn new_markdown(app: &mut SheetApp) {
    let prefix = app.ui.language.tr("Doc").to_string();
    app.run_or_alert("file.new", json!({"document": true, "prefix": prefix}));
}

/// A new SQL, plain text or Word file, `Query1.sql`, `Text1.txt` or `Document1.docx` in the
/// interface's language.
pub(crate) fn new_text(app: &mut SheetApp, kind: TextKind) {
    let (prefix, name) = match kind {
        TextKind::Sql => ("Query", "sql"),
        TextKind::Word => ("Document", "word"),
        TextKind::Hwp => ("Document", "hwp"),
        _ => ("Text", "text"),
    };
    let prefix = app.ui.language.tr(prefix).to_string();
    app.run_or_alert("file.new", json!({"document": true, "kind": name, "prefix": prefix}));
}

/// The All Sheets button's width: its icon, the active sheet's name and the arrow.
fn sheet_button_width(app: &SheetApp, ui: &Ui) -> f32 {
    let name = app.session.active().and_then(|d| d.wb.active()).map(|s| s.name.clone()).unwrap_or_default();
    let font = theme::ui_font(13.0);
    let name = fit(ui, &name, &font, SHEET_NAME_MAX_W);
    22.0 + ui.painter().layout_no_wrap(name, font, egui::Color32::WHITE).size().x + 22.0
}

/// The All Sheets button (≡ Sheet1 ▾) in place of the sheet tabs: a click lists the sheets (type
/// to filter) with New sheet at the end, a right click opens the sheet tab menu for the active
/// sheet (Rename, Delete, Move or Copy…), and Rename turns the button into the name field.
fn sheet_list(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let Some(d) = app.session.active() else { return };
    let active = d.wb.active_sheet();
    let name = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let mut items = crate::tabs::navigator_items(&d.wb.sheets);
    if let Some(i) = app.grid.renaming_tab {
        rename_field(app, ui, i, &name);
        return;
    }
    let font = theme::ui_font(13.0);
    let shown = fit(ui, &name, &font, SHEET_NAME_MAX_W);
    let w = sheet_button_width(app, ui);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    let all = lang.tr("All sheets");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, all));
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(r, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let icon = Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(15.0, 15.0));
    icons::paint(ui.painter(), icon, Icon::SheetList, t.text_dim);
    ui.painter().text(pos2(icon.right() + 5.0, r.center().y - 1.0), Align2::LEFT_CENTER, &shown, font, t.accent);
    let arrow = Rect::from_center_size(pos2(r.right() - 10.0, r.center().y), vec2(10.0, 10.0));
    icons::paint(ui.painter(), arrow, Icon::Chevron, t.text_dim);
    let resp = resp.on_hover_text(lang.tr("All sheets"));
    if resp.secondary_clicked()
        && let Some(p) = ui.input(|inp| inp.pointer.interact_pos())
        && (app.editor.is_none() || app.commit_edit(0, 0, false, false))
    {
        app.grid.tab_menu = Some((active, p, ui.ctx().cumulative_frame_nr()));
    }
    items.push((NEW_SHEET.into(), format!("+ {}", lang.tr("New sheet"))));
    let rows: Vec<(&str, String)> = items.iter().map(|(i, name)| (i.as_str(), name.clone())).collect();
    match crate::ribbon::editable_combo(ui, &resp, "file_tabs_sheets", &active.to_string(), &rows, false) {
        ComboResult::Picked(v) if v == NEW_SHEET => app.run_or_alert("home.insertSheet", json!({})),
        ComboResult::Picked(v) => {
            if let Ok(i) = v.parse::<usize>()
                && i != active
            {
                crate::tabs::activate_sheet(app, i);
            }
        }
        ComboResult::None => {}
    }
}

/// The sheet's name being typed in place of the All Sheets button (the tab menu's Rename).
fn rename_field(app: &mut SheetApp, ui: &mut Ui, i: usize, name: &str) {
    let font = theme::ui_font(13.0);
    let w = (ui.painter().layout_no_wrap(app.grid.rename_text.clone(), font.clone(), egui::Color32::WHITE).size().x + 24.0).max(120.0);
    let mut text = app.grid.rename_text.clone();
    let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let field = Rect::from_center_size(r.center(), vec2(w, crate::tabs::RENAME_H));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    child.spacing_mut().interact_size.y = crate::tabs::RENAME_H;
    let ed = child
        .add(egui::TextEdit::singleline(&mut text).font(font).margin(egui::Margin::symmetric(4, 2)).desired_width(field.width() - 8.0))
        .accessible_name(app.ui.language.tr("Sheet Name"));
    app.grid.rename_text = text.clone();
    if crate::tabs::rename_needs_focus(ed.has_focus(), ed.lost_focus()) {
        ed.request_focus();
    }
    if ed.lost_focus() {
        if !ui.input(|inp| inp.key_pressed(egui::Key::Escape)) && text != name {
            app.run_or_alert("sheet.rename", json!({"sheet": i, "name": text}));
        }
        app.grid.renaming_tab = None;
    }
}

/// What shows while no file is open: New workbook, Markdown document, SQL or text file, Open…
/// and the recent files. Ctrl+N and Ctrl+O work here too (no sheet takes the keys).
pub fn start_page(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let (new, open) =
        ui.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, egui::Key::N), i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)));
    if new {
        new_workbook(app);
        return;
    }
    if open {
        app.run_or_alert("file.open", json!({}));
        return;
    }
    let recent: Vec<String> = app.ui.recent.iter().take(12).cloned().collect();
    let mut chosen: Option<String> = None;
    egui::ScrollArea::vertical().id_salt("start_page").show(ui, |ui| {
        let width = (ui.available_width() - 48.0).clamp(240.0, 640.0);
        ui.vertical_centered(|ui| {
            ui.add_space(56.0);
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(width);
                ui.label(egui::RichText::new(lang.tr("Monun")).font(theme::ui_bold(26.0)).color(t.text));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(lang.tr("Open a file or make a new one. Each file opens as a tab at the bottom."))
                        .font(theme::ui_font(13.0))
                        .color(t.text_dim),
                );
                ui.add_space(20.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    if start_button(ui, Icon::Sheet, lang.tr("New Excel Workbook"), &t).clicked() {
                        new_workbook(app);
                    }
                    if start_button(ui, Icon::MarkdownFile, lang.tr("New Markdown Document"), &t).clicked() {
                        new_markdown(app);
                    }
                    if start_button(ui, Icon::SqlFile, lang.tr("New SQL File"), &t).clicked() {
                        new_text(app, TextKind::Sql);
                    }
                    if start_button(ui, Icon::TextFile, lang.tr("New Text File"), &t).clicked() {
                        new_text(app, TextKind::Plain);
                    }
                    if cfg!(feature = "word") && start_button(ui, Icon::Word, lang.tr("New Word Document"), &t).clicked() {
                        new_text(app, TextKind::Word);
                    }
                    if cfg!(feature = "hwp") && start_button(ui, Icon::Hwp, lang.tr("New Hangul Document"), &t).clicked() {
                        new_text(app, TextKind::Hwp);
                    }
                    if start_button(ui, Icon::Folder, lang.tr("Open…"), &t).clicked() {
                        app.run_or_alert("file.open", json!({}));
                    }
                });
                ui.add_space(28.0);
                ui.label(egui::RichText::new(lang.tr("Recent")).font(theme::ui_bold(14.0)).color(t.text));
                ui.add_space(6.0);
                if recent.is_empty() {
                    ui.label(egui::RichText::new(lang.tr("Files you open show here.")).font(theme::ui_font(13.0)).color(t.text_dim));
                }
                for path in &recent {
                    if recent_row(ui, path, width, &t).clicked() {
                        chosen = Some(path.clone());
                    }
                }
            });
        });
    });
    if let Some(path) = chosen {
        app.open_path(&path);
    }
}

/// A large button of the start page: an icon and a label.
fn start_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens) -> egui::Response {
    let font = theme::ui_font(13.5);
    let tw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.text).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2((tw + 58.0).max(150.0), 48.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        t.tab_active
    };
    ui.painter().rect_filled(r, 6.0, fill);
    ui.painter().rect_stroke(r, 6.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Inside);
    let i = Rect::from_center_size(pos2(r.left() + 24.0, r.center().y), vec2(20.0, 20.0));
    icons::paint(ui.painter(), i, icon, t.accent);
    ui.painter().text(pos2(i.right() + 12.0, r.center().y), Align2::LEFT_CENTER, label, font, t.text);
    resp
}

/// A recent file on the start page: its icon, name and folder; the full path in the tooltip.
fn recent_row(ui: &mut Ui, path: &str, width: f32, t: &Tokens) -> egui::Response {
    let p = std::path::Path::new(path);
    let name = p.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path).to_string();
    let folder = p.parent().and_then(std::path::Path::to_str).unwrap_or_default().to_string();
    let (r, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let i = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), i, path_icon(path), t.text_dim);
    let font = theme::ui_font(13.0);
    let name_shown = fit(ui, &name, &font, width - 48.0);
    ui.painter().text(pos2(i.right() + 12.0, r.top() + 13.0), Align2::LEFT_CENTER, name_shown, font, t.text);
    let small = theme::ui_font(11.5);
    let folder_shown = fit(ui, &folder, &small, width - 48.0);
    ui.painter().text(pos2(i.right() + 12.0, r.bottom() - 11.0), Align2::LEFT_CENTER, folder_shown, small, t.text_dim);
    resp.on_hover_text(path)
}
