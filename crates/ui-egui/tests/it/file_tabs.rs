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
    assert!(d.is_text_file(), "one document tab and nothing else");
    assert_eq!(d.display_title(), "Doc1.md");
    assert_eq!(tab_labels(&h), ["Doc1.md"]);
    assert!(h.query_by_label("All sheets").is_none(), "a Markdown file has no sheets to list");

    // + beside the tabs: a new workbook, after the Markdown file.
    h.get_by_label("New file").click();
    h.run_steps(3);
    h.get_by_label("New Excel Workbook").click();
    h.run_steps(4);
    assert_eq!(tab_labels(&h), ["Doc1.md", "Book1"]);
    assert!(!h.state().session.active().unwrap().is_text_file(), "the new workbook is in front");
    assert!(h.query_by_label("All sheets").is_some(), "its sheets are listed beside the tabs");
    assert!(h.query_by_label("New document (Markdown)").is_none(), "a workbook adds sheets only");

    h.get_by_label("Doc1.md").click();
    h.run_steps(4);
    assert!(h.state().session.active().unwrap().is_text_file(), "a click on a tab shows its file");
    h.get_by_label("Book1").click();
    h.run_steps(4);
    assert!(!h.state().session.active().unwrap().is_text_file());
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
    assert!(h.state().session.active().unwrap().is_text_file());
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
        assert!(refused.contains("Markdown or text file holds its text only"), "{id}: {refused}");
    }
    let d = h.state().session.active().unwrap();
    assert!(d.is_text_file());
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
fn sql_and_text_files_show_their_text_as_code_editors_do() {
    use monun_engine::docs::TextKind;
    let mut h = harness(Session::new());
    assert!(h.query_by_label("New SQL File").is_some() && h.query_by_label("New Text File").is_some(), "the start page offers both");
    h.get_by_label("New SQL File").click();
    h.run_steps(4);
    let d = h.state().session.active().expect("a SQL file");
    assert_eq!((d.display_title(), d.text_kind()), ("Query1.sql".to_owned(), Some(TextKind::Sql)));
    assert_eq!(tab_labels(&h), ["Query1.sql"]);
    assert!(h.query_by_label("SQL").is_some() && h.query_by_label("LF").is_some(), "the tool bar says what it is and how it saves");
    assert!(h.query_by_label("Side by Side").is_none() && h.query_by_label("Contents").is_none(), "its text only, no document");
    // An empty file opens ready to type: Enter keeps the indentation, Tab indents.
    h.input_mut().events.push(egui::Event::Text("SELECT *\n  FROM t".into()));
    h.run_steps(3);
    key(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    key(&mut h, egui::Key::Tab, egui::Modifiers::NONE);
    h.input_mut().events.push(egui::Event::Text("WHERE".into()));
    h.run_steps(3);
    assert_eq!(h.state().doc_view.draft.as_ref().unwrap().text, "SELECT *\n  FROM t\n      WHERE");

    // + beside the tabs: a plain text file.
    h.get_by_label("New file").click();
    h.run_steps(3);
    h.get_by_label("New Text File").click();
    h.run_steps(4);
    assert_eq!(tab_labels(&h), ["Query1.sql (unsaved changes)", "Text1.txt"]);
    assert_eq!(h.state().session.active().unwrap().text_kind(), Some(TextKind::Plain));
    assert!(h.query_by_label("Plain Text").is_some());

    // A file with Windows line breaks says so, and saves them.
    let path = std::env::temp_dir().join(format!("monun-file-tabs-{}.sql", std::process::id()));
    std::fs::write(&path, "select 1\r\n").unwrap();
    h.state_mut().run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("CRLF").is_some());
    h.state_mut().run("doc.setText", json!({"text": "select 2\n"})).unwrap();
    h.state_mut().run("file.save", json!({})).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"select 2\r\n");
    let _ = std::fs::remove_file(&path);
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

#[test]
fn a_memo_goes_on_a_paragraph_of_a_markdown_file_and_follows_it() {
    use egui::accesskit::Role;
    let path = std::env::temp_dir().join(format!("monun-memos-{}.md", std::process::id()));
    std::fs::write(&path, "# Plan\n\nFirst paragraph.\n\nSecond paragraph.\n").unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    h.run_steps(4);
    // View mode: a right click on the paragraph, Add Memo.
    h.get_by_label("Second paragraph.").click_secondary();
    h.run_steps(3);
    h.get_by_label("Add Memo").click();
    h.run_steps(3);
    assert!(h.state().session.comments_pane, "the memo panel opens");
    h.get_by_role_and_label(Role::MultilineTextInput, "Memo").type_text("Check the totals");
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    let memos = h.state().ui.memos.clone();
    assert_eq!(memos.len(), 1);
    assert_eq!(
        (memos[0].path.as_str(), memos[0].line, memos[0].quote.as_str(), memos[0].text.as_str()),
        (path.as_str(), 4, "Second paragraph.", "Check the totals")
    );
    assert!(h.query_by_label("Check the totals").is_some(), "the panel lists it");
    assert!(h.query_by_role_and_label(Role::Button, "Memo").is_some(), "and the paragraph is marked");
    // Ctrl+Enter keeps one too.
    h.get_by_label("First paragraph.").click_secondary();
    h.run_steps(3);
    h.get_by_label("Add Memo").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::MultilineTextInput, "Memo").type_text("Source?");
    h.run_steps(2);
    key(&mut h, egui::Key::Enter, egui::Modifiers::COMMAND);
    assert_eq!(h.state().ui.memos.len(), 2);
    assert!(h.state().memos.draft.is_none());
    h.state_mut().ui.memos.retain(|m| m.text != "Source?");
    h.run_steps(2);
    // Lines written above: the memo stays on its paragraph, and the .md is untouched by it.
    h.state_mut().run("doc.setText", json!({"text": "# Plan\n\nNew line.\n\nFirst paragraph.\n\nSecond paragraph.\n"})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.memos[0].line, 6);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("Check the totals"));
    // Delete asks once more.
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.memos.len(), 1);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().ui.memos.is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unsaved_markdown_file_waits_for_a_save_before_memos() {
    let mut h = harness(Session::new());
    h.state_mut().run("file.new", json!({"document": true})).unwrap();
    h.state_mut().run("doc.setText", json!({"text": "Some text.\n"})).unwrap();
    h.state_mut().run("doc.view", json!({"mode": "view"})).unwrap();
    h.run_steps(4);
    h.get_by_label("Some text.").click_secondary();
    h.run_steps(3);
    let item = h.get_by_label("Add Memo");
    use egui_kittest::kittest::NodeT as _;
    assert!(item.accesskit_node().is_disabled(), "greyed until the file has a path");
}

#[test]
fn a_markdown_file_opens_again_where_it_was_being_read_with_contents_as_left() {
    let path = std::env::temp_dir().join(format!("monun-doc-line-{}.md", std::process::id()));
    let text: String = (0..300).map(|i| format!("Paragraph {i} of the plan.\n\n")).collect();
    std::fs::write(&path, text).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    h.run_steps(4);
    h.get_by_label("Contents").click();
    h.run_steps(2);
    assert!(h.state().ui.doc_outline, "Contents is the interface's, kept with its settings");
    // Scroll well down, then close the file.
    let over = h.get_by_label("Paragraph 3 of the plan.").rect().center();
    for _ in 0..12 {
        h.input_mut().events.push(egui::Event::PointerMoved(over));
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Wheel,
            delta: egui::vec2(0.0, -400.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run_steps(2);
    }
    h.run_steps(4);
    let uid = h.state().session.active().unwrap().uid;
    let read = monun_ui_egui::doc_view::top_line(h.state(), (uid, 0)).unwrap();
    assert!(read > 20, "scrolled down to line {read}");
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    let kept = h.state().ui.doc_views.iter().find(|v| v.path == path).map(|v| v.line);
    assert_eq!(kept, Some(read), "closing keeps the line it was read at");
    // Opened again: there, not at the top.
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    h.run_steps(8);
    let uid = h.state().session.active().unwrap().uid;
    let shown = monun_ui_egui::doc_view::top_line(h.state(), (uid, 0)).unwrap();
    assert!(shown.abs_diff(read) <= 4, "opens at line {shown}, was read at {read}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_markdown_file_changed_by_another_program_shows_as_it_is_now() {
    let path = std::env::temp_dir().join(format!("monun-reload-{}.md", std::process::id()));
    std::fs::write(&path, "# Before\n\nOld paragraph.\n").unwrap();
    let path_s = path.to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": path_s})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Old paragraph.").is_some());
    // Another program writes it: with nothing changed here, it is read again.
    std::fs::write(&path, "# After\n\nNew paragraph from elsewhere.\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run_steps(4);
    assert!(h.query_by_label("New paragraph from elsewhere.").is_some(), "the change shows");
    assert!(!h.state().session.active().unwrap().is_dirty());
    // With a change made here, it asks first.
    h.state_mut().run("doc.setText", json!({"text": "# Mine\n"})).unwrap();
    std::fs::write(&path, "# Theirs\n\nTheir paragraph.\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run_steps(4);
    assert!(h.query_by_label("Their paragraph.").is_none(), "nothing is lost without asking");
    h.get_by_label("Reload").click();
    h.run_steps(4);
    assert!(h.query_by_label("Their paragraph.").is_some());
    assert!(!h.state().session.active().unwrap().is_dirty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_markdown_file_changed_elsewhere_is_shown_as_it_is_now_and_said_so() {
    let path = std::env::temp_dir().join(format!("monun-changed-{}.md", std::process::id()));
    std::fs::write(&path, "# Plan\n\nFirst paragraph.\n").unwrap();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let mut h = harness(Session::new());
    h.state_mut().run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    h.run_steps(4);
    std::fs::write(&path, "# Plan\n\nChanged by another program.\n").unwrap();
    // Open files are looked at about every second.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run_steps(3);
    assert!(h.query_by_label("Changed by another program.").is_some(), "shown as it is now");
    let note = format!("“{name}” was changed elsewhere and is shown as it is now.");
    assert!(h.query_by_label(&note).is_some(), "and said so, over the document");
    let _ = std::fs::remove_file(&path);
}
