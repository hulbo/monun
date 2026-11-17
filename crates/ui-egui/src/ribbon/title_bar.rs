use super::controls::{act, control_keytip};
use super::{Color32, Icon, Sense, SheetApp, Stroke, StrokeKind, Tokens, Ui, icon_button, json, pos2, small_button, theme, theme_choices, vec2};
use crate::widgets::toggle_icon_button;

/// macOS draws the window into the title bar (`with_fullsize_content_view`) and AppKit paints the
/// traffic lights itself, centred in its standard 28 pt title-bar band. Moving them would take
/// AppKit calls (`unsafe`, forbidden here), so on macOS the title bar is that band: the lights sit
/// vertically centred in it, with the controls beside them (#170).
const MAC: bool = cfg!(target_os = "macos");

pub const MAC_TITLE_BAR: f32 = 28.0;

/// The traffic lights as AppKit lays them out: button diameter, centre-to-centre pitch and the
/// first button's left edge (8–10 pt depending on the macOS version; the larger value keeps the
/// gap after them from shrinking).
const MAC_LIGHT: f32 = 12.0;

const MAC_LIGHT_PITCH: f32 = 20.0;

const MAC_LIGHTS_LEFT: f32 = 10.0;

/// Where title-bar content starts on macOS: the end of the three lights plus the gap the lights
/// keep from the top of the window.
pub const MAC_CONTENT_LEFT: f32 = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT + (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;

/// Leave room for the macOS traffic lights when the content extends into the title bar.
pub const TITLE_LEFT_PAD: f32 = if MAC { MAC_CONTENT_LEFT } else { 8.0 };

pub(super) const TITLE_BAR_H: f32 = if MAC { MAC_TITLE_BAR } else { 38.0 };

/// The tallest title-bar controls, smaller on macOS to keep a margin in the shorter bar.
pub(super) const TITLE_CONTROL_H: f32 = if MAC { 22.0 } else { 26.0 };

/// Draws the title bar: drag/maximize, the ribbon tabs, then AutoSave toggle and
/// Home/Save/Undo/Redo/More, with search, share, comments and theme controls at the right. The
/// window's own title already names the workbook, so the bar repeats no title and the tabs need
/// no row of their own.
pub fn title_bar(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ch = TITLE_CONTROL_H;
    egui::Panel::top("title_bar").exact_size(TITLE_BAR_H).frame(egui::Frame::NONE.fill(t.window)).show(ui, |ui| {
        let rect = ui.max_rect();
        // Dragging the empty title area moves the window.
        let bg = ui.interact(rect, ui.scope_id().with("drag_title"), Sense::click_and_drag());
        if bg.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        // The start page of file tabs (no file open) has no ribbon, file or undo to offer.
        let open = app.session.active().is_some();
        ui.horizontal_centered(|ui| {
            ui.add_space(TITLE_LEFT_PAD);
            if !open {
                ui.label(egui::RichText::new(app.ui.language.tr("Monun")).font(theme::ui_bold(13.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    theme_switch(app, ui, &t);
                    // A new group starts here, with no file open: the way back to another one.
                    if app.file_tabs {
                        ui.add_space(6.0);
                        crate::groups::button(app, ui, &t);
                        crate::projects::button(app, ui, &t);
                        group_panel_button(app, ui, ch);
                    }
                });
                return;
            }
            // The right side first (search, the file in front, its group and project, the panels'
            // buttons): it keeps its room, and the menus and quick commands take what is left, cut
            // off there rather than run under it in a narrow window.
            let row = ui.available_rect_before_wrap();
            let mut right = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::right_to_left(egui::Align::Center)));
            right.spacing_mut().item_spacing.x = 2.0;
            right_side(app, &mut right, &t, ch);
            let end = (right.min_rect().left() - 12.0).max(row.left());
            let room = egui::Rect::from_min_max(row.min, pos2(end, row.max.y));
            let mut left = ui.new_child(egui::UiBuilder::new().max_rect(room).layout(egui::Layout::left_to_right(egui::Align::Center)));
            left.set_clip_rect(room.intersect(ui.clip_rect()));
            left_side(app, &mut left, &t, ch, room.width());
        });
    });
}

/// A guess at the quick commands' width (AutoSave's switch to More) before they were first drawn.
const QUICK_GUESS: f32 = 190.0;

/// The left of the title bar, `width` wide: File and the ribbon's tabs, then AutoSave and the quick
/// commands (Home, Save, Undo, Redo, More). The quick commands keep their room; the tabs that
/// don't fit wait in an overflow list (»), and in a very narrow bar AutoSave's label goes first.
fn left_side(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, ch: f32, width: f32) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let quick_id = egui::Id::unique("monun.title_quick_w");
    let quick = ui.ctx().data(|d| d.get_temp::<f32>(quick_id)).unwrap_or(QUICK_GUESS);
    let label = egui::RichText::new(app.ui.language.tr("AutoSave")).font(theme::ui_font(12.5)).color(t.text_dim);
    let label_w = ui.painter().layout_no_wrap(app.ui.language.tr("AutoSave").to_string(), theme::ui_font(12.5), t.text_dim).size().x + 2.0;
    let show_label = width - 16.0 - quick - label_w >= super::TABS_MIN;
    let tabs_room = width - 16.0 - quick - if show_label { label_w } else { 0.0 };
    super::tab_strip_within(app, ui, tabs_room);
    ui.add_space(16.0);
    // AutoSave toggle (saves after each change when the workbook has a file).
    if show_label {
        ui.label(label);
    }
    let quick_from = ui.cursor().left();
    let on = app.ui.autosave;
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
    ui.painter().rect_filled(r, 8.0, if on { t.accent } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(r, 8.0, Stroke::new(1.0, if on { t.accent } else { t.text_dim }), StrokeKind::Inside);
    let knob = if on { pos2(r.right() - 8.0, r.center().y) } else { pos2(r.left() + 8.0, r.center().y) };
    ui.painter().circle_filled(knob, 5.0, if on { t.on_accent } else { t.text_dim });
    if resp.on_hover_text(app.ui.language.tr("AutoSave: save after every change (workbooks saved to a file)")).clicked() {
        app.toggle_autosave();
    }
    ui.add_space(6.0);
    let __k = icon_button(ui, Icon::Home, t.text_dim, "Home", vec2(26.0, ch));
    control_keytip(app, ui, &__k, "start");
    if __k.clicked() {
        app.open_dialog("start", json!({}));
    }
    if icon_button(ui, Icon::Save, t.text_dim, "Save", vec2(26.0, ch)).clicked()
        // File operations must include the cell the user is still editing (as the File > Save menu item already does).
        && app.commit_edit(0, 0, false, false)
    {
        app.run_or_alert("file.save", json!({}));
    }
    // The commands' own availability: there must be a step, and no undo group may be open on
    // the document (an action that spans several calls is still running).
    let can_undo = monun_engine::cmd::can_undo(&app.session).is_ok();
    let can_redo = monun_engine::cmd::can_redo(&app.session).is_ok();
    // Named and marked enabled or not for assistive technology (and the UI tests).
    let undo = icon_button(ui, Icon::Undo, if can_undo { t.text_dim } else { t.text_disabled }, "Undo", vec2(26.0, ch));
    undo.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_undo, "Undo"));
    if undo.clicked() && can_undo {
        app.run_or_alert("edit.undo", json!({}));
    }
    let undo_list = icon_button(ui, Icon::Chevron, t.text_dim, "Undo list", vec2(14.0, ch));
    egui::Popup::menu(&undo_list).show(|ui| {
        let labels: Vec<String> = if can_undo {
            app.session.active().map(|d| d.undo.iter().rev().take(20).map(|e| e.label.clone()).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };
        if labels.is_empty() {
            ui.label(app.ui.language.tr("Can't Undo"));
        }
        for (i, l) in labels.iter().enumerate() {
            if ui.button(format!("{} {l}", app.ui.language.tr("Undo"))).clicked() {
                app.run_or_alert("edit.undo", json!({"steps": i + 1}));
            }
        }
    });
    let redo = icon_button(ui, Icon::Redo, if can_redo { t.text_dim } else { t.text_disabled }, "Redo", vec2(26.0, ch));
    redo.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_redo, "Redo"));
    if redo.clicked() && can_redo {
        app.run_or_alert("edit.redo", json!({}));
    }
    let more = icon_button(ui, Icon::More, t.text_dim, "More commands", vec2(26.0, ch));
    egui::Popup::menu(&more).show(|ui| {
        for (label, id) in [
            ("New Workbook", "file.new"),
            ("Open…", "file.open"),
            ("Save As…", "file.saveAs"),
            ("Sort A to Z", "data.sortAscending"),
            ("Calculate Now", "formulas.calculateNow"),
        ] {
            if ui.button(app.ui.language.tr(label)).clicked() {
                app.run_or_alert(id, json!({}));
            }
        }
        if ui.button(app.ui.language.tr("Print…")).clicked() {
            app.print_now(json!({}));
        }
    });
    let quick = ui.cursor().left() - quick_from;
    ui.ctx().data_mut(|d| d.insert_temp(quick_id, quick));
}

/// The right of the title bar, right to left: search, then (with file tabs) the file in front, its
/// group, its project and the group panel's button (else Share), the memo panel's button, the
/// theme switch and the ribbon's collapse button.
fn right_side(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, ch: f32) {
    ui.add_space(8.0);
    let __k = icon_button(ui, Icon::Search, t.text_dim, "Search commands", vec2(28.0, ch));
    control_keytip(app, ui, &__k, "commandSearch");
    if __k.clicked() {
        app.open_dialog("commandSearch", json!({}));
    }
    // With file tabs the group button takes Share's place; File › Export and Save a
    // Copy keep what Share offered.
    if app.file_tabs {
        // Right to left: the project, its group, then the file in front.
        crate::groups::file_button(app, ui, t);
        crate::groups::button(app, ui, t);
        crate::projects::button(app, ui, t);
        group_panel_button(app, ui, ch);
    } else {
        share(app, ui);
    }
    // Memos are for workbooks and Markdown files: a SQL or text file has no button.
    if crate::panes::memo_allowed(app) {
        let shown = crate::panes::memos_shown(app);
        let __k = toggle_icon_button(ui, Icon::Comment, shown, "Comments", vec2(28.0, ch));
        control_keytip(app, ui, &__k, "comments");
        if __k.clicked() {
            crate::panes::toggle_memos(app);
        }
    }
    theme_switch(app, ui, t);
    let lang = app.ui.language;
    // Show or hide the ribbon's commands to give the sheet more room (Excel's Ctrl+F1);
    // the tabs stay, and clicking one shows its commands again. A document tab has no
    // ribbon to show.
    if crate::doc_view::is_active(app) {
        return;
    }
    let collapsed = app.ui.ribbon_collapsed;
    let (icon, tip) = if collapsed { (Icon::Chevron, "Show the Ribbon (Ctrl+F1)") } else { (Icon::ChevronUp, "Collapse the Ribbon (Ctrl+F1)") };
    let ribbon = icon_button(ui, icon, t.text_dim, tip, vec2(28.0, ch));
    ribbon.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, !collapsed, lang.tr("Ribbon")));
    if ribbon.clicked() {
        act(app, "view.collapseRibbon", json!({}));
    }
}

/// Share (without file tabs): export or save a copy.
fn share(app: &mut SheetApp, ui: &mut Ui) {
    let share = small_button(ui, Icon::Share, "Share", "Share: export or save a copy", true);
    egui::Popup::menu(&share).show(|ui| {
        // A PDF or web page of a workbook with several sheets: all of them or this one.
        let several = crate::backstage::several_sheets(app);
        let pdf: &[(&str, bool)] =
            if several { &[("Export All Sheets as PDF…", true), ("Export This Sheet as PDF…", false)] } else { &[("Export as PDF…", false)] };
        for (label, all) in pdf {
            if ui.button(app.ui.language.tr(label)).clicked() {
                crate::backstage::export_as(app, "pdf", *all);
            }
        }
        for (label, fmt) in [("Save a Copy as Excel Workbook (.xlsx)…", "xlsx"), ("Export as CSV…", "csv")] {
            if ui.button(app.ui.language.tr(label)).clicked() {
                app.open_dialog("saveCopy", json!({"format": fmt}));
            }
        }
        let page: &[(&str, bool)] = if several {
            &[("Export All Sheets as a Web Page…", true), ("Export This Sheet as a Web Page…", false)]
        } else {
            &[("Export as Web Page (.html)…", false)]
        };
        for (label, all) in page {
            if ui.button(app.ui.language.tr(label)).clicked() {
                crate::backstage::export_as(app, "html", *all);
            }
        }
    });
}

/// Quick light/dark switch (#194); right-click offers System as well. The same choice lives in
/// File › Display theme and the View tab.
fn theme_switch(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let theme = icon_button(ui, Icon::Appearance, t.text_dim, "Dark Mode", vec2(28.0, 26.0));
    let dark = ui.ctx().theme() == egui::Theme::Dark;
    theme.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, dark, lang.tr("Dark Mode")));
    if theme.clicked() {
        act(app, "view.darkMode", json!({}));
    }
    theme.context_menu(|ui| theme_choices(app, ui));
}

#[cfg(test)]
mod title_bar_tests {
    use super::*;

    #[test]
    fn mac_title_bar_centres_and_clears_the_traffic_lights() {
        // The lights are centred in the bar, and content keeps the same gap after them.
        let margin = (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;
        let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
        assert_eq!(MAC_CONTENT_LEFT - lights_end, margin);
    }
}

/// The group panel's button beside the group button: shows and hides the panel as the memo
/// button does the memo panel.
fn group_panel_button(app: &mut SheetApp, ui: &mut Ui, height: f32) {
    let shown = crate::panes::groups_shown(app);
    if toggle_icon_button(ui, Icon::SheetList, shown, "Group panel", vec2(28.0, height)).clicked() {
        crate::panes::toggle_groups(app);
    }
}
