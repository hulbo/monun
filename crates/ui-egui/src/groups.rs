//! File groups, with file tabs ([`SheetApp::file_tabs`]): a named set of files worked on together
//! (one job, such as SFR001), the way a Visual Studio solution keeps its open documents. The group
//! button in the title bar switches between them: the files of the group in use close (unsaved
//! changes are asked about once, for all of them) and the other group's files open as they were
//! left. A new group starts empty. The group in use follows its tabs as files open and close, and
//! its files open again the next time 모눈 starts.

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets::DialogButton;

/// One group: its name and the files that were open in it, in tab order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileGroup {
    pub name: String,
    /// The files' paths, in tab order. A file never saved has none, so it is not kept.
    pub files: Vec<String>,
    /// The file whose tab was in front.
    pub active: Option<String>,
}

/// Where a switch goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A group kept already.
    Group(String),
    /// A new group: empty, or (`take`) with the files open now.
    New { name: String, take: bool },
}

/// The longest group name.
const NAME_MAX: usize = 60;

/// The widest a group's name is drawn on the button.
const NAME_MAX_W: f32 = 160.0;

/// The index of the group in use, made (as the Default group) the first time one is needed. A
/// saved name no group has any more falls back to the first group.
fn ensure(app: &mut SheetApp) -> usize {
    if let Some(i) = app.ui.groups.iter().position(|g| g.name == app.ui.group) {
        return i;
    }
    if app.ui.groups.is_empty() {
        let name = app.ui.language.tr("Default group").to_string();
        app.ui.groups.push(FileGroup { name, ..FileGroup::default() });
    }
    app.ui.group = app.ui.groups.first().map(|g| g.name.clone()).unwrap_or_default();
    0
}

/// The name of the group in use.
pub fn current(app: &mut SheetApp) -> String {
    let i = ensure(app);
    app.ui.groups.get(i).map(|g| g.name.clone()).unwrap_or_default()
}

/// The open files' paths in tab order, and the one in front.
fn open_files(app: &SheetApp) -> (Vec<String>, Option<String>) {
    let mut files: Vec<String> = Vec::new();
    for w in app.session.windows() {
        if let Some(path) = app.session.documents().iter().find(|d| d.uid == w.doc_uid).and_then(|d| d.path.clone())
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    (files, app.session.active().and_then(|d| d.path.clone()))
}

/// Keeps the group in use in step with the tabs (every frame, with file tabs). While quitting asks
/// about unsaved files it holds still: the files it closes stay in the group.
pub(crate) fn sync(app: &mut SheetApp) {
    if app.groups_frozen {
        let asking = app.dialog.as_ref().is_some_and(|d| d.name() == "saveChanges");
        if asking || app.allow_close {
            return;
        }
        // The quit was cancelled.
        app.groups_frozen = false;
    }
    let (files, active) = open_files(app);
    let i = ensure(app);
    if let Some(g) = app.ui.groups.get_mut(i)
        && (g.files != files || g.active != active)
    {
        g.files = files;
        g.active = active;
    }
}

/// Opens the files of the group in use, as they were left (the desktop app at start, before the
/// files it was asked to open).
pub fn restore(app: &mut SheetApp) {
    let i = ensure(app);
    if let Some(g) = app.ui.groups.get(i).cloned() {
        open_group(app, &g);
    }
}

/// Opens a group's files in its tab order, the one that was in front in front again. A file that
/// can't be opened (moved, deleted, password-protected) is left out and named in a message.
fn open_group(app: &mut SheetApp, g: &FileGroup) {
    let mut missing = Vec::new();
    for path in &g.files {
        if app.session.documents().iter().any(|d| d.path.as_deref() == Some(path.as_str())) {
            continue;
        }
        if app.run("file.open", json!({"path": path})).is_err() {
            missing.push(path.clone());
        }
    }
    if let Some(active) = &g.active
        && let Some(i) = app.session.documents().iter().position(|d| d.path.as_deref() == Some(active.as_str()))
    {
        app.session.set_active(i);
    }
    app.follow_active_window();
    app.after_engine();
    if !missing.is_empty() {
        let lang = app.ui.language;
        let text = format!("{}\n\n{}", lang.tr("These files of the group could not be opened:"), missing.join("\n"));
        app.message = Some((lang.tr("Monun").to_string(), text));
    }
}

/// Writes the groups now (the desktop's `ui.json`), so a change outlasts a crash.
fn persist(app: &SheetApp) {
    if let Some(save) = &app.services.save_ui {
        save(&app.ui);
    }
}

/// Goes to `target`: a cell or document being typed is entered first, and unsaved files are asked
/// about (Save, Don't Save, Cancel) before anything closes. A new group taking the open files
/// along closes nothing.
pub fn switch(app: &mut SheetApp, target: Target) {
    if let Err(e) = app.settle_drafts() {
        app.alert(e);
        return;
    }
    if let Target::New { name, take: true } = &target {
        sync(app);
        let i = ensure(app);
        let files = app.ui.groups.get_mut(i).map(std::mem::take).unwrap_or_default();
        app.ui.groups.push(FileGroup { name: name.clone(), ..files });
        if let Some(left) = app.ui.groups.get_mut(i) {
            left.name = app.ui.group.clone();
        }
        app.ui.group = name.clone();
        persist(app);
        return;
    }
    let unsaved: Vec<String> = app.session.documents().iter().filter(|d| d.is_dirty()).map(|d| d.display_title()).collect();
    if unsaved.is_empty() {
        go(app, &target);
    } else {
        let (name, new) = match &target {
            Target::Group(name) => (name.clone(), false),
            Target::New { name, .. } => (name.clone(), true),
        };
        app.open_dialog("switchGroup", json!({"target": name, "new": new, "files": unsaved}));
    }
}

/// Closes every file, then opens the target group's files (none for a new group).
fn go(app: &mut SheetApp, target: &Target) {
    sync(app);
    app.backstage = None;
    // Each close takes the workbook in front with all its windows.
    for _ in 0..app.session.documents().len() {
        if app.session.active().is_none() || app.run("file.close", json!({"force": true, "document": true})).is_err() {
            break;
        }
    }
    let group = match target {
        Target::Group(name) => app.ui.groups.iter().find(|g| &g.name == name).cloned(),
        Target::New { name, .. } => {
            let g = FileGroup { name: name.clone(), ..FileGroup::default() };
            app.ui.groups.push(g.clone());
            Some(g)
        }
    };
    if let Some(g) = group {
        app.ui.group = g.name.clone();
        open_group(app, &g);
    }
    sync(app);
    persist(app);
}

/// Saves every file with unsaved changes, asking where for one never saved. `false` when one is
/// still unsaved (its Save As was cancelled, or saving failed).
fn save_all(app: &mut SheetApp) -> bool {
    let unsaved: Vec<u64> = app.session.documents().iter().filter(|d| d.is_dirty()).map(|d| d.uid).collect();
    for uid in unsaved {
        let Some(window) = app.session.doc_window(uid) else { continue };
        app.enter_window(window);
        app.run_or_alert("file.save", json!({}));
        if app.session.documents().iter().any(|d| d.uid == uid && d.is_dirty()) {
            return false;
        }
    }
    true
}

/// A group name typed into New Group or Rename Group, trimmed; the reason it can't be used, if any.
fn check_name(app: &SheetApp, name: &str, renaming: Option<&str>) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name for the group.");
    }
    if name.chars().count() > NAME_MAX {
        return Err("A group name has at most 60 characters.");
    }
    if app.ui.groups.iter().any(|g| Some(g.name.as_str()) != renaming && g.name.eq_ignore_ascii_case(name)) {
        return Err("There is already a group with this name.");
    }
    Ok(name.to_string())
}

/// The group button in the title bar, in place of Share: the group in use, and a menu to switch to
/// another group, make one, rename this one or delete another.
pub fn button(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let name = current(app);
    let font = theme::ui_font(12.5);
    let shown = crate::file_tabs::fit(ui, &name, &font, NAME_MAX_W);
    let tw = ui.painter().layout_no_wrap(shown.clone(), font.clone(), t.text).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(24.0 + tw + 18.0, 23.0), Sense::click());
    let label = lang.tr("Group: {name}").replace("{name}", &name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(r, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let icon = Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(14.0, 14.0));
    icons::paint(ui.painter(), icon, Icon::Folder, t.text_dim);
    ui.painter().text(pos2(icon.right() + 5.0, r.center().y), Align2::LEFT_CENTER, &shown, font, t.text);
    let arrow = Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(9.0, 9.0));
    icons::paint(ui.painter(), arrow, Icon::Chevron, t.text_dim);
    let resp = resp.on_hover_text(lang.tr("Group: the set of files you work on together. Switching closes these files and opens the other group's."));
    egui::Popup::menu(&resp).show(|ui| menu(app, ui));
}

fn menu(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    ui.set_min_width(220.0);
    let current = current(app);
    let groups = app.ui.groups.clone();
    let mut chosen = None;
    for g in &groups {
        let on = g.name == current;
        let count = lang.tr("{n} files").replace("{n}", &g.files.len().to_string());
        let text = egui::RichText::new(format!("{}    {count}", g.name));
        if ui.add(egui::Button::selectable(on, text)).clicked() && !on {
            chosen = Some(g.name.clone());
        }
    }
    ui.separator();
    if ui.button(lang.tr("New Group…")).clicked() {
        app.open_dialog("newGroup", json!({}));
    }
    if ui.button(lang.tr("Rename Group…")).clicked() {
        app.open_dialog("renameGroup", json!({"name": current}));
    }
    let others: Vec<String> = groups.iter().map(|g| g.name.clone()).filter(|n| *n != current).collect();
    if !others.is_empty() {
        ui.menu_button(lang.tr("Delete Group"), |ui| {
            for name in others {
                if ui.button(&name).clicked() {
                    app.open_dialog("deleteGroup", json!({"name": name}));
                }
            }
        });
    }
    if let Some(name) = chosen {
        ui.close();
        switch(app, Target::Group(name));
    }
}

/// What a click in the group panel asks for, done once the panel is drawn.
enum PanelAction {
    /// Go to this group.
    Switch(String),
    /// Show or hide a group's files.
    Fold(String),
    /// Show this file: its tab in the group in use, or its group's files with it in front.
    Open { group: String, path: String },
    /// Open a group dialog (New, Rename, Delete).
    Dialog(&'static str, Json),
}

/// The group panel (the right-hand pane beside the files, with file tabs): every group with its
/// files, the group in use marked and its files shown. A click on a group goes to it, and on a file
/// shows it, going to its group first when that isn't the group in use. New Group is the + in the
/// panel's title strip; a group's right-click menu renames or deletes it. The title bar's group button keeps its menu.
pub(crate) fn panel(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let current = current(app);
    let groups = app.ui.groups.clone();
    let open_tab = app.session.active().and_then(|d| d.path.clone());
    let dirty: Vec<String> = app.session.documents().iter().filter(|d| d.is_dirty()).filter_map(|d| d.path.clone()).collect();
    let mut action = None;
    let width = ui.available_width();
    for g in &groups {
        let on = g.name == current;
        let fold_id = egui::Id::unique(("monun.group_panel.open", &g.name));
        let open = ui.data(|d| d.get_temp::<bool>(fold_id)).unwrap_or(on);
        // The group's row: the arrow folds it, the rest goes to it.
        let (r, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &g.name));
        if on {
            ui.painter().rect_filled(r, 4.0, t.pressed);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        let arrow = Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(10.0, 10.0));
        icons::paint(ui.painter(), arrow, if open { Icon::Chevron } else { Icon::Right }, t.text_dim);
        let folder = Rect::from_center_size(pos2(arrow.right() + 12.0, r.center().y), vec2(14.0, 14.0));
        icons::paint(ui.painter(), folder, Icon::Folder, if on { t.accent } else { t.text_dim });
        let count = lang.tr("{n} files").replace("{n}", &g.files.len().to_string());
        let small = theme::ui_font(11.5);
        let count_w = ui.painter().layout_no_wrap(count.clone(), small.clone(), t.text_dim).size().x;
        ui.painter().text(pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, &count, small, t.text_dim);
        let font = if on { theme::ui_bold(13.0) } else { theme::ui_font(13.0) };
        let name = crate::file_tabs::fit(ui, &g.name, &font, (r.right() - folder.right() - count_w - 20.0).max(20.0));
        ui.painter().text(pos2(folder.right() + 6.0, r.center().y), Align2::LEFT_CENTER, name, font, t.text);
        let on_arrow = resp.interact_pointer_pos().is_some_and(|p| p.x < folder.left());
        if resp.clicked() {
            action = Some(if on || on_arrow { PanelAction::Fold(g.name.clone()) } else { PanelAction::Switch(g.name.clone()) });
        }
        let resp = resp.on_hover_text(if on { lang.tr("The group in use") } else { lang.tr("Go to this group: its files open, these close") });
        resp.context_menu(|ui| {
            if !on && ui.button(lang.tr("Go to This Group")).clicked() {
                action = Some(PanelAction::Switch(g.name.clone()));
            }
            if ui.button(lang.tr("Rename Group…")).clicked() {
                action = Some(PanelAction::Dialog("renameGroup", json!({"name": g.name})));
            }
            if !on && ui.button(lang.tr("Delete Group…")).clicked() {
                action = Some(PanelAction::Dialog("deleteGroup", json!({"name": g.name})));
            }
        });
        if !open {
            continue;
        }
        if g.files.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(egui::RichText::new(lang.tr("No files")).font(theme::ui_font(12.0)).color(t.text_dim).italics());
            });
        }
        for path in &g.files {
            let name = std::path::Path::new(path).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path).to_string();
            let front = on && open_tab.as_deref() == Some(path.as_str());
            let (r, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, front, &name));
            if front || resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover);
            }
            let icon = Rect::from_center_size(pos2(r.left() + 46.0, r.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), icon, crate::file_tabs::path_icon(path), if front { t.accent } else { t.text_dim });
            let font = theme::ui_font(12.5);
            let unsaved = on && dirty.contains(path);
            let shown = crate::file_tabs::fit(ui, &name, &font, (r.right() - icon.right() - 24.0).max(20.0));
            let ink = if on { t.text } else { t.text_dim };
            ui.painter().text(pos2(icon.right() + 6.0, r.center().y), Align2::LEFT_CENTER, shown, font, ink);
            if unsaved {
                ui.painter().circle_filled(pos2(r.right() - 10.0, r.center().y), 3.0, t.text_dim);
            }
            if resp.on_hover_text(path).clicked() {
                action = Some(PanelAction::Open { group: g.name.clone(), path: path.clone() });
            }
        }
        ui.add_space(4.0);
    }
    match action {
        Some(PanelAction::Switch(name)) => switch(app, Target::Group(name)),
        Some(PanelAction::Fold(name)) => {
            let fold_id = egui::Id::unique(("monun.group_panel.open", &name));
            let open = ui.data(|d| d.get_temp::<bool>(fold_id)).unwrap_or(name == current);
            ui.data_mut(|d| d.insert_temp(fold_id, !open));
        }
        Some(PanelAction::Open { group, path }) if group == current => {
            app.open_path(&path);
            app.follow_active_window();
        }
        Some(PanelAction::Open { group, path }) => {
            // Its group's files open with it in front.
            if let Some(g) = app.ui.groups.iter_mut().find(|g| g.name == group) {
                g.active = Some(path);
            }
            switch(app, Target::Group(group));
        }
        Some(PanelAction::Dialog(name, params)) => app.open_dialog(name, params),
        None => {}
    }
}

/// The group dialogs: New Group, Rename Group, Delete Group, and the unsaved files a switch asks
/// about.
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let text = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    match d.name() {
        "newGroup" | "renameGroup" => {
            let renaming = (d.name() == "renameGroup").then(|| text(d, "was"));
            ui.add_space(4.0);
            ui.label(lang.tr("Group name:"));
            let mut name = text(d, "name");
            let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).accessible_name(lang.tr("Group name:"));
            if d.values.get("focused").and_then(Json::as_bool) != Some(true) {
                field.request_focus();
                d.values.insert("focused".into(), json!(true));
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            d.values.insert("name".into(), json!(name));
            if renaming.is_none() {
                let mut take = d.values.get("take").and_then(Json::as_bool).unwrap_or(false);
                ui.add_space(4.0);
                if ui.checkbox(&mut take, lang.tr("Keep the open files, in the new group")).changed() {
                    d.values.insert("take".into(), json!(take));
                }
                let hint = if take {
                    "The open files stay open and move to the new group."
                } else {
                    "The open files close and stay in this group; the new group starts empty."
                };
                ui.label(egui::RichText::new(lang.tr(hint)).color(t.text_dim));
            }
            let error = text(d, "error");
            if !error.is_empty() {
                ui.label(egui::RichText::new(lang.tr(&error)).color(t.danger));
            }
            ui.add_space(8.0);
            let row = [DialogButton::confirm("OK"), DialogButton::neutral("Cancel")];
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => submit_name(app, d, open, renaming.as_deref()),
                Some(_) => *open = false,
                None if enter => submit_name(app, d, open, renaming.as_deref()),
                None => {}
            }
        }
        "deleteGroup" => {
            let name = text(d, "name");
            ui.add_space(4.0);
            ui.label(lang.tr("Delete the group “{name}”? Its files stay where they are.").replace("{name}", &name));
            ui.add_space(8.0);
            let row = [DialogButton::discard("Delete"), DialogButton::neutral("Cancel")];
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => {
                    if name != app.ui.group {
                        app.ui.groups.retain(|g| g.name != name);
                        persist(app);
                    }
                    *open = false;
                }
                Some(_) => *open = false,
                None => {}
            }
        }
        "switchGroup" => {
            let name = text(d, "target");
            let new = d.values.get("new").and_then(Json::as_bool) == Some(true);
            let files: Vec<String> = d
                .values
                .get("files")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            ui.add_space(4.0);
            ui.label(lang.tr("Save changes to these files before going to the group “{name}”?").replace("{name}", &name));
            ui.add_space(4.0);
            for f in &files {
                ui.label(egui::RichText::new(format!("•  {f}")).color(t.text_dim));
            }
            ui.add_space(8.0);
            let row = [DialogButton::confirm("Save"), DialogButton::discard("Don't Save"), DialogButton::neutral("Cancel")];
            let target = if new { Target::New { name, take: false } } else { Target::Group(name) };
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => {
                    *open = false;
                    if save_all(app) {
                        go(app, &target);
                    }
                }
                Some(1) => {
                    *open = false;
                    go(app, &target);
                }
                Some(_) => *open = false,
                None => {}
            }
        }
        _ => *open = false,
    }
}

/// OK in New Group or Rename Group: the name is checked, then the group made (and gone to) or
/// renamed.
fn submit_name(app: &mut SheetApp, d: &mut Dialog, open: &mut bool, renaming: Option<&str>) {
    let typed = d.values.get("name").and_then(Json::as_str).unwrap_or_default().to_string();
    let name = match check_name(app, &typed, renaming) {
        Ok(name) => name,
        Err(e) => {
            d.values.insert("error".into(), json!(e));
            return;
        }
    };
    *open = false;
    match renaming {
        Some(was) => {
            if let Some(g) = app.ui.groups.iter_mut().find(|g| g.name == was) {
                g.name = name.clone();
            }
            if app.ui.group == was {
                app.ui.group = name;
            }
            persist(app);
        }
        None => {
            let take = d.values.get("take").and_then(Json::as_bool).unwrap_or(false);
            switch(app, Target::New { name, take });
        }
    }
}
