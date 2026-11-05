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
