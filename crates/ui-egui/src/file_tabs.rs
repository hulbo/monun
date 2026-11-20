//! The window's files, one window for all of them (Visual Studio's way, [`SheetApp::file_tabs`]):
//! the group panel lists the files of the group in use (no tabs at the bottom), a workbook shows its
//! sheet tabs below the grid as Excel does, and a start page shows while no file is open.
//!
//! A file shows in an engine window ([`monun_engine::Session::windows`]): View › New Window gives a
//! workbook a second one (`Book1:2`), as Visual Studio's New Window does, and closing one is
//! `window.close`, which asks about unsaved changes when it is the workbook's last window.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use monun_engine::docs::TextKind;
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

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

/// The bar at the bottom of the window: a workbook's sheet tabs, with the grid's horizontal
/// scrollbar, as Excel has them; a file of any other kind has none (the group panel lists the files).
pub fn bottom_bar(app: &mut SheetApp, ui: &mut Ui) {
    if app.session.active().is_some_and(|d| !d.is_text_file()) {
        crate::tabs::sheet_tabs(app, ui);
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
            | monun_engine::io::FileKind::Image
            | monun_engine::io::FileKind::Html
            | monun_engine::io::FileKind::Web
            | monun_engine::io::FileKind::Slides,
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
        Some(TextKind::Web) => Icon::WebFile,
        Some(TextKind::Slides) => Icon::SlidesFile,
        Some(TextKind::Http) => Icon::ApiFile,
        Some(TextKind::Code) => Icon::CodeFile,
    }
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

/// New › Other File: asks for the new file's name (in `folder` of the group in use, when given).
/// Its extension says what it is (`Main.java` is coloured as Java), so one entry makes every kind
/// of source file.
pub(crate) fn new_other(app: &mut SheetApp, folder: Option<u64>) {
    app.open_dialog("newFile", json!({"folder": folder}));
}

/// What a file called `name` will be, as its status bar says it (`Java`, `YAML`, `Plain Text`);
/// `None` for a name Monun doesn't make files of (a picture, a PDF…).
fn kind_name(name: &str, lang: crate::i18n::Language) -> Option<String> {
    if let Some(s) = monun_markdown::code::source_of(name) {
        return Some(s.label.to_string());
    }
    let label = match TextKind::of_name(name) {
        TextKind::Markdown => "Markdown Document",
        TextKind::Sql => "SQL",
        TextKind::Http => "API Request",
        TextKind::Word => "Word Document",
        TextKind::Hwp => "Hangul Document",
        TextKind::Plain | TextKind::Code => "Plain Text",
        TextKind::Pdf | TextKind::Image | TextKind::Web | TextKind::Slides => return None,
    };
    Some(lang.tr(label).to_string())
}

/// The New File dialog: the file's name with its extension, and what that makes it. OK makes the
/// file (unsaved: its first save asks where).
pub(crate) fn new_file_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut crate::dialogs::Dialog, open: &mut bool) {
    use crate::widgets::DialogButton;
    use egui::RichText;
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let text = |d: &crate::dialogs::Dialog, k: &str| d.values.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    ui.add_space(4.0);
    ui.label(lang.tr("File name:"));
    let mut name = text(d, "name");
    let field =
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).hint_text("Main.java")).accessible_name(lang.tr("File name:"));
    if d.values.get("focused").and_then(serde_json::Value::as_bool) != Some(true) {
        field.request_focus();
        d.values.insert("focused".into(), json!(true));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if field.changed() {
        d.values.remove("error");
    }
    d.values.insert("name".into(), json!(name));
    let what = (!name.trim().is_empty()).then(|| kind_name(name.trim(), lang));
    match &what {
        Some(Some(kind)) => {
            ui.label(RichText::new(format!("{} {kind}", lang.tr("Type:"))).color(t.text));
        }
        Some(None) => {
            ui.label(RichText::new(lang.tr("Monun can't make a new file of that type.")).color(t.danger));
        }
        None => {}
    }
    ui.label(
        RichText::new(lang.tr("The extension says what the file is and how it is coloured: Main.java, App.vue, application.yml, mapper.xml…"))
            .color(t.text_dim),
    );
    let error = text(d, "error");
    if !error.is_empty() {
        ui.label(RichText::new(lang.tr(&error)).color(t.danger));
    }
    ui.add_space(8.0);
    let row = [DialogButton::confirm("OK"), DialogButton::neutral("Cancel")];
    let submit = match crate::widgets::dialog_buttons(ui, &row) {
        Some(0) => true,
        Some(_) => {
            *open = false;
            false
        }
        None => enter,
    };
    if !submit || !matches!(what, Some(Some(_))) {
        return;
    }
    let before: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    match app.run("file.new", json!({"document": true, "name": name.trim()})) {
        Ok(_) => {
            *open = false;
            // Made from a folder's menu: it goes in that folder.
            if let Some(folder) = d.values.get("folder").and_then(serde_json::Value::as_u64)
                && let Some(doc) = app.session.active().map(|d| d.uid).filter(|u| !before.contains(u))
            {
                app.folder_pending.insert(doc, folder);
            }
        }
        Err(e) => {
            d.values.insert("error".into(), json!(e));
        }
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
                    egui::RichText::new(lang.tr("Open a file or make a new one. The group panel lists the files open."))
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
                    if start_button(ui, Icon::WebFile, lang.tr("New Site Link…"), &t).clicked() {
                        crate::web_view::new_link(app);
                    }
                    if start_button(ui, Icon::CodeFile, lang.tr("New Other File…"), &t).clicked() {
                        new_other(app, None);
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
