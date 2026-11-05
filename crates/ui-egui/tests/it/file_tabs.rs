//! Visual Studio-style file tabs (`SheetApp::file_tabs`, the desktop app's): a start page while no
//! file is open, every file a tab at the bottom, and a workbook's sheets in the All Sheets list.

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::json;

fn harness(session: Session) -> egui_kittest::Harness<'static, SheetApp> {
    let mut app = crate::common::app(session);
    app.use_file_tabs();
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The labels of the file tabs, left to right.
fn tab_labels(h: &egui_kittest::Harness<'static, SheetApp>) -> Vec<String> {
    let mut tabs: Vec<_> =
        h.query_all_by_role(egui::accesskit::Role::Tab).filter_map(|n| n.accesskit_node().label().map(|l| (n.rect().min.x, l.to_string()))).collect();
    tabs.sort_by(|a, b| a.0.total_cmp(&b.0));
    tabs.into_iter().map(|(_, l)| l).collect()
}

#[test]
fn it_starts_on_the_start_page_rather_than_a_blank_workbook() {
    let h = harness(Session::new());
    assert!(h.state().session.documents().is_empty(), "no Book1 is made");
    assert!(h.query_by_label("New Excel Workbook").is_some(), "the start page offers a new workbook");
    assert!(h.query_by_label("New Markdown Document").is_some(), "and a new Markdown file");
    assert!(h.query_by_label("AutoSave").is_none(), "with no file there is nothing to save");
    assert!(h.query_by_label("Paste").is_none(), "nor a ribbon");
}

#[test]
fn new_files_open_as_tabs_and_a_click_switches_between_them() {
    let mut h = harness(Session::new());
    h.get_by_label("New Markdown Document").click();
    h.run_steps(4);
    let d = h.state().session.active().expect("a Markdown file");
    assert!(d.is_markdown(), "one document tab and nothing else");
    assert_eq!(d.display_title(), "Doc1.md");
    assert_eq!(tab_labels(&h), ["Doc1.md"]);
    assert!(h.query_by_label("All sheets").is_none(), "a Markdown file has no sheets to list");

    // + beside the tabs: a new workbook, after the Markdown file.
    h.get_by_label("New file").click();
    h.run_steps(3);
    h.get_by_label("New Excel Workbook").click();
    h.run_steps(4);
    assert_eq!(tab_labels(&h), ["Doc1.md", "Book1"]);
    assert!(!h.state().session.active().unwrap().is_markdown(), "the new workbook is in front");
    assert!(h.query_by_label("All sheets").is_some(), "its sheets are listed beside the tabs");
    assert!(h.query_by_label("New document (Markdown)").is_none(), "a workbook adds sheets only");

    h.get_by_label("Doc1.md").click();
    h.run_steps(4);
    assert!(h.state().session.active().unwrap().is_markdown(), "a click on a tab shows its file");
    h.get_by_label("Book1").click();
    h.run_steps(4);
    assert!(!h.state().session.active().unwrap().is_markdown());
}

#[test]
fn the_all_sheets_list_adds_and_switches_sheets() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s);
    let sheets = |h: &egui_kittest::Harness<'static, SheetApp>| h.state().session.active().map_or(0, |d| d.wb.sheets.len());
    assert_eq!(sheets(&h), 1);
    h.get_by_label("All sheets").click();
    h.run_steps(4);
    h.get_by_label("+ New sheet").click();
    h.run_steps(4);
    assert_eq!(sheets(&h), 2, "New sheet at the end of the list adds one");
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet(), 1, "and shows it");
    h.get_by_label("All sheets").click();
    h.run_steps(4);
    h.get_by_label("Sheet1").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet(), 0, "a sheet's row shows that sheet");
}

#[test]
fn closing_the_last_tab_goes_back_to_the_start_page() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s);
    h.get_by_label("Close").click();
    h.run_steps(4);
    assert!(h.state().session.documents().is_empty(), "the untouched workbook closes at once");
    assert!(h.query_by_label("New Excel Workbook").is_some(), "and the start page is back");
}

#[test]
fn closing_a_tab_with_unsaved_changes_asks_first() {
    let mut s = Session::new();
    s.new_markdown("Doc");
    s.run("doc.setText", json!({"text": "# Notes"})).unwrap();
    let mut h = harness(s);
    assert!(tab_labels(&h)[0].contains("unsaved changes"), "the tab says it has unsaved changes");
    // The unsaved dot gives way to the close button under the pointer.
    h.get_by_role(egui::accesskit::Role::Tab).hover();
    h.run_steps(2);
    h.get_by_label("Close").click();
    h.run_steps(4);
    assert_eq!(h.state().session.documents().len(), 1, "the file stays open until the question is answered");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("saveChanges"));
}

#[test]
fn close_other_tabs_keeps_the_one_clicked() {
    let mut s = Session::new();
    s.new_workbook();
    s.new_markdown("Doc");
    s.new_workbook();
    let mut h = harness(s);
    assert_eq!(tab_labels(&h), ["Book1", "Doc1.md", "Book2"]);
    h.get_by_label("Doc1.md").click_secondary();
    h.run_steps(3);
    h.get_by_label("Close Other Tabs").click();
    h.run_steps(4);
    assert_eq!(tab_labels(&h), ["Doc1.md"]);
    assert!(h.state().session.active().unwrap().is_markdown());
}

#[test]
fn a_workbook_takes_sheets_only() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s);
    let refused = h.state_mut().run("doc.insert", json!({})).expect_err("no document tab in a workbook");
    assert!(refused.contains("sheets only"), "{refused}");
    assert!(h.state_mut().run("home.insertSheet", json!({})).is_ok(), "a sheet goes in as in Excel");
    h.state_mut().ui.ribbon_tab = "Insert".into();
    h.run_steps(3);
    assert!(h.query_by_label("Text Box").is_some(), "the Insert tab shows");
    assert!(h.query_by_label("Document").is_none(), "without its Document button");
}

#[test]
fn a_markdown_file_keeps_its_one_document() {
    let mut s = Session::new();
    s.new_markdown("Doc");
    s.run("doc.setText", json!({"text": "Total {{=1+2}}"})).unwrap();
    let mut h = harness(s);
    for (id, params) in [("home.insertSheet", json!({})), ("doc.insert", json!({})), ("sheet.move", json!({"sheet": 0, "to": 1, "copy": true}))] {
        let refused = h.state_mut().run(id, params).expect_err(id);
        assert!(refused.contains("Markdown file holds its document only"), "{id}: {refused}");
    }
    let d = h.state().session.active().unwrap();
    assert!(d.is_markdown());
    assert_eq!(monun_engine::docs::field_value(&d.wb, "=1+2").as_deref(), Ok("{{=1+2}}"), "a field reads as written: there are no cells");
    // How the file shows is not part of its .md text, so it stays as saved.
    let saved = h.state_mut().run("file.saveAs", json!({"path": std::env::temp_dir().join("monun-file-tabs-test.md").to_string_lossy()}));
    assert!(saved.is_ok(), "{saved:?}");
    assert!(!h.state().session.active().unwrap().is_dirty());
    h.state_mut().run("doc.view", json!({"mode": "view"})).unwrap();
    assert!(!h.state().session.active().unwrap().is_dirty(), "View mode leaves the file as saved");
    h.state_mut().run("doc.view", json!({"mode": "edit"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Bold (Ctrl+B)").is_some() || h.query_by_label("B").is_some(), "the formatting buttons show");
    assert!(h.query_by_label("Cell value").is_none(), "but not Cell value: there are no cells");
    let _ = std::fs::remove_file(std::env::temp_dir().join("monun-file-tabs-test.md"));
}

fn key(h: &mut egui_kittest::Harness<'static, SheetApp>, key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.run_steps(3);
}

fn active_title(h: &egui_kittest::Harness<'static, SheetApp>) -> String {
    h.state().session.active().map(|d| d.display_title()).unwrap_or_default()
}

#[test]
fn ctrl_tab_goes_round_the_tabs_and_ctrl_f4_closes_one() {
    let mut s = Session::new();
    s.new_workbook();
    s.new_markdown("Doc");
    s.new_workbook();
    let mut h = harness(s);
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    assert_eq!(active_title(&h), "Book2");
    key(&mut h, egui::Key::Tab, ctrl);
    assert_eq!(active_title(&h), "Book1", "Ctrl+Tab goes on, round the end");
    key(&mut h, egui::Key::Tab, ctrl);
    assert_eq!(active_title(&h), "Doc1.md");
    key(&mut h, egui::Key::Tab, ctrl | egui::Modifiers::SHIFT);
    assert_eq!(active_title(&h), "Book1", "Ctrl+Shift+Tab goes back");
    key(&mut h, egui::Key::F4, ctrl);
    assert_eq!(tab_labels(&h), ["Doc1.md", "Book2"], "Ctrl+F4 closes the tab in front");
}

#[test]
fn a_tab_dragged_along_the_others_moves_there() {
    let mut s = Session::new();
    s.new_workbook();
    s.new_markdown("Doc");
    s.new_workbook();
    let mut h = harness(s);
    assert_eq!(tab_labels(&h), ["Book1", "Doc1.md", "Book2"]);
    let from = h.get_by_label("Book1").rect().center();
    let to = h.get_by_label("Book2").rect().right_center() + egui::vec2(-4.0, 0.0);
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    for i in 1..=8 {
        h.input_mut().events.push(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert_eq!(tab_labels(&h), ["Doc1.md", "Book2", "Book1"], "Book1 goes where it was dropped");
}

#[test]
fn a_markdown_file_opens_again_as_it_was_shown() {
    let path = std::env::temp_dir().join(format!("monun-doc-view-{}.md", std::process::id()));
    std::fs::write(&path, "# Notes\n").unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    h.state_mut().run("doc.view", json!({"mode": "edit", "fitWidth": true})).unwrap();
    h.state_mut().run("view.zoom", json!({"percent": 130})).unwrap();
    h.run_steps(3);
    assert!(!h.state().session.active().unwrap().is_dirty(), "how it shows leaves the file as saved");
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    h.run_steps(3);
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    let doc = sh.document.as_ref().unwrap();
    assert_eq!(doc.mode.map(|m| m.name()), Some("edit"), "the mode it had");
    assert!(doc.fit_width, "Fit Width as it was");
    assert_eq!(sh.zoom, 130, "and the zoom");
    assert!(!d.is_dirty());
    let _ = std::fs::remove_file(&path);
}

/// A 1×1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[test]
fn a_markdown_files_pictures_are_files_beside_it() {
    let dir = std::env::temp_dir().join(format!("monun-md-pictures-{}", std::process::id()));
    let _ = std::fs::create_dir_all(dir.join("copy"));
    let md = dir.join("notes.md");
    std::fs::write(&md, "# Notes\n").unwrap();
    let md = md.to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": md})).unwrap();
    let r = h.state_mut().run("doc.picture", json!({"base64": PNG, "name": "screen shot"})).unwrap();
    let link = r["link"].as_str().unwrap().to_string();
    assert_eq!(link, "notes.assets/screen-shot.png", "beside the file, in a folder named after it");
    h.state_mut().run("doc.setText", json!({"text": format!("# Notes\n\n![screen shot]({link})\n")})).unwrap();
    h.state_mut().run("doc.view", json!({"mode": "view"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Image, "screen shot").is_some(), "the picture shows");

    h.state_mut().run("file.save", json!({})).unwrap();
    assert!(dir.join("notes.assets").join("screen-shot.png").exists(), "saving writes the picture beside the .md");
    assert!(std::fs::read_to_string(&md).unwrap().contains("](notes.assets/screen-shot.png)"), "and the .md links to it");

    let copy = dir.join("copy").join("other.md").to_string_lossy().into_owned();
    h.state_mut().run("file.saveAs", json!({"path": copy})).unwrap();
    assert!(dir.join("copy").join("notes.assets").join("screen-shot.png").exists(), "a Save As elsewhere takes its pictures along");

    h.state_mut().run("file.close", json!({"force": true, "document": true})).unwrap();
    h.state_mut().run("file.open", json!({"path": md})).unwrap();
    h.run_steps(4);
    let sh = h.state().session.active().unwrap().wb.active().unwrap().clone();
    assert_eq!(sh.images.len(), 1, "opening reads the pictures beside it");
    assert_eq!(sh.document.as_ref().unwrap().files[0].0, link);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Image, "screen shot").is_some(), "and shows them");
    let _ = std::fs::remove_dir_all(&dir);
}
