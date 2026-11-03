//! Document tabs: the new-document button beside +, View and Edit, typing that reaches the
//! workbook, task boxes, links to cells, and the shortcuts a document keeps.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use monun_engine::Session;
use serde_json::json;

use crate::common::Harness;
use crate::support::{blank_harness, blank_session, harness};

fn press(h: &mut Harness, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(3);
}

fn doc_text(h: &Harness, sheet: usize) -> String {
    h.state().session.active().unwrap().wb.sheet(sheet).unwrap().document.as_ref().unwrap().text.clone()
}

/// A workbook with a document tab after Sheet1, active.
fn with_document(text: &str) -> Harness {
    let mut s: Session = blank_session();
    s.run("doc.insert", json!({"name": "Notes", "text": text})).unwrap();
    harness(s, 4)
}

#[test]
fn the_document_button_adds_a_document_ready_to_type() {
    let mut h = blank_harness(4);
    h.get_by_label("New document (Markdown)").click();
    h.run_steps(4);
    let sheet = h.state().session.active().unwrap().wb.active().unwrap().clone();
    assert!(sheet.is_document());
    assert_eq!(sheet.name, "Doc1");
    // An empty document opens in Edit mode with the editor focused.
    h.input_mut().events.push(Event::Text("# Notes".into()));
    h.run_steps(3);
    assert_eq!(h.state().doc_view.draft.as_ref().unwrap().text, "# Notes");
    // Going to another sheet writes what was typed into the workbook first.
    press(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet(), 0);
    assert_eq!(doc_text(&h, 1), "# Notes");
    assert!(h.state().session.active().unwrap().is_dirty());
}

#[test]
fn view_mode_ticks_task_boxes_and_undo_takes_it_back() {
    let mut h = with_document("# Plan\n\n- [ ] buy AAA\n");
    // A document with text opens in View mode, rendered.
    assert!(h.query_by_label("Plan").is_some());
    h.get_by_label("task").click();
    h.run_steps(3);
    assert_eq!(doc_text(&h, 1), "# Plan\n\n- [x] buy AAA\n");
    press(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(doc_text(&h, 1), "# Plan\n\n- [ ] buy AAA\n");
}

#[test]
fn a_link_to_a_cell_goes_there() {
    let mut h = with_document("[go to the total](#Sheet1!B2)\n");
    h.get_by_label("go to the total").click();
    h.run_steps(4);
    let d = h.state().session.active().unwrap();
    assert_eq!(d.wb.active_sheet(), 0);
    assert_eq!(d.selection.active.a1(), "B2");
}

#[test]
fn edit_mode_shows_the_text_beside_the_document() {
    let mut h = with_document("# Plan\n");
    h.get_by_label("Edit").click();
    h.run_steps(4);
    // The editor took focus: typing goes in at the caret, and the preview follows.
    press(&mut h, Key::End, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("\n- one".into()));
    h.run_steps(3);
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().doc_view.draft.as_ref().unwrap().text, "# Plan\n\n- one\n- ", "Enter continues the list");
    assert!(h.query_by_label("one").is_some(), "the preview shows the list beside the editor");
    // Back in View mode the draft is in the workbook.
    h.get_by_label("View").click();
    h.run_steps(3);
    assert_eq!(doc_text(&h, 1), "# Plan\n\n- one\n- ");
}

/// Steps the app until `label` shows, giving the worker thread that draws diagrams time.
#[cfg(feature = "diagrams")]
fn wait_for(h: &mut Harness, label: &str) -> bool {
    for _ in 0..100 {
        h.run_steps(1);
        if h.query_by_label(label).is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    false
}

#[cfg(feature = "diagrams")]
#[test]
fn mermaid_blocks_show_as_diagrams_and_errors_as_messages() {
    let mut h = with_document("```mermaid\nflowchart TD\n  A[start] --> B[end]\n```\n");
    assert!(wait_for(&mut h, "Diagram"), "the flowchart is drawn");
    // A click shows it enlarged.
    h.get_by_label("Diagram").click();
    h.run_steps(3);
    assert!(h.query_all_by_label("Diagram").count() >= 2, "the enlarged diagram's window opened");

    let mut h = with_document("```mermaid\nnot a diagram\n```\n");
    assert!(wait_for(&mut h, "This diagram has an error:"));
}

#[test]
fn fields_show_cell_values_and_follow_them() {
    let mut s = blank_session();
    s.run("cell.set", json!({"cell": "A1", "input": "30000"})).unwrap();
    s.run("home.numberFormat", json!({"range": "A1", "code": "#,##0"})).unwrap();
    s.run("doc.insert", json!({"name": "Notes", "text": "Total {{Sheet1!A1}} and {{=Sheet1!A1*2}}, {{Nowhere!A1}}\n"})).unwrap();
    let mut h = harness(s, 4);
    // A reference shows as its cell does; a formula in General; a bad reference as its error.
    assert!(h.query_by_label("Total 30,000 and 60000, #REF!").is_some());
    h.state_mut().run("cell.set", json!({"cell": "Sheet1!A1", "input": "10"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Total 10 and 20, #REF!").is_some(), "the fields follow the sheet");
}

#[test]
fn find_counts_matches_and_replace_all_is_one_step() {
    let mut h = with_document("# Plan\n\nbuy AAA, then AAA again\n");
    press(&mut h, Key::F, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("aaa".into()));
    h.run_steps(3);
    assert!(h.query_by_label("1/2").is_some(), "two matches, the first current");
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.query_by_label("2/2").is_some(), "Enter goes to the next");
    // Ctrl+H adds the replacement.
    press(&mut h, Key::H, Modifiers::COMMAND);
    h.get_by_label("Replace All").click();
    h.run_steps(3);
    // An empty replacement removes them.
    assert_eq!(doc_text(&h, 1), "# Plan\n\nbuy , then  again\n");
    h.get_by_label("Close (Esc)").click();
    h.run_steps(3);
    press(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(doc_text(&h, 1), "# Plan\n\nbuy AAA, then AAA again\n", "one undo takes back every replacement");
}

#[test]
fn the_contents_panel_goes_to_a_heading() {
    let mut h = with_document("# First\n\ntext\n\n## Second\n\nmore\n");
    h.get_by_label("Edit").click();
    h.run_steps(3);
    h.get_by_label("Contents").click();
    h.run_steps(3);
    // The panel lists the headings; the first "Second" is the panel's, drawn before the document.
    h.get_all_by_label("Second").next().expect("the panel lists the heading").click();
    h.run_steps(4);
    h.input_mut().events.push(Event::Text("Z".into()));
    h.run_steps(3);
    assert_eq!(h.state().doc_view.draft.as_ref().unwrap().text, "# First\n\ntext\n\nZ## Second\n\nmore\n", "the caret went to the heading's line");
}

#[test]
fn pictures_go_into_the_document_and_show_in_it() {
    // A 40 × 20 PNG to pick.
    let dir = std::env::temp_dir().join(format!("monun-doc-picture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chart.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([30, 120, 200, 255])).save(&path).unwrap();
    let picked = path.to_str().unwrap().to_owned();
    let services = monun_ui_egui::Services { pick_picture: Some(Box::new(move || Some(picked.clone()))), ..Default::default() };
    let mut s = blank_session();
    s.run("doc.insert", json!({"name": "Notes", "text": "# Plan\n"})).unwrap();
    let mut app = monun_ui_egui::SheetApp::new(s, services);
    app = crate::common::prepare(app);
    let mut h = crate::support::app_harness(app, 4);
    h.get_by_label("Edit").click();
    h.run_steps(3);
    press(&mut h, Key::End, Modifiers::COMMAND);
    h.get_by_label("Picture").click();
    h.run_steps(4);
    let text = h.state().doc_view.draft.as_ref().unwrap().text.clone();
    let d = h.state().session.active().unwrap();
    let image = d.wb.sheet(1).unwrap().images.first().expect("the picture is on the document tab").clone();
    assert_eq!(text, format!("# Plan\n![chart](image:{})", image.id));
    assert_eq!(image.anchor.cell.a1(), "C2", "beside the caret's line, right of the text");
    // The rendered document shows the picture.
    assert!(h.query_by_label("chart").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_tab_has_no_ribbon() {
    let mut h = with_document("# Plan\n");
    assert!(h.query_by_label("File").is_some(), "File stays");
    assert!(h.query_by_label("Ribbon").is_none(), "no button to show the ribbon");
    assert!(h.query_by_label("Paste").is_none(), "no ribbon commands");
    // Ctrl+F1 there leaves the sheets' ribbon as it was.
    press(&mut h, Key::F1, Modifiers::COMMAND);
    assert!(!h.state().ui.ribbon_collapsed);
    // Back on a sheet the ribbon is there again.
    press(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert!(h.query_by_label("Ribbon").is_some());
    assert!(h.query_by_label("Paste").is_some());
}

#[test]
fn tables_centre_their_header_and_wrap_to_the_width() {
    let long = vec!["word"; 80].join(" ");
    let mut h = with_document(&format!("| No | Detail |\n|---|---|\n| X1 | {long} |\n| X2 | short detail text |\n"));
    h.run_steps(2);
    let head = h.get_by_label("Detail").rect();
    let wrapped = h.get_by_label(&long).rect();
    let short = h.get_by_label("short detail text").rect();
    assert!(wrapped.right() < 1200.0, "the long cell fits the window: {wrapped:?}");
    assert!(wrapped.height() > 2.5 * short.height(), "its text wraps: {wrapped:?}");
    // The header is in the middle of its column; the body starts at its left.
    assert!((head.center().x - wrapped.center().x).abs() < 30.0, "{head:?} over {wrapped:?}");
    assert!(head.left() > short.left() + 100.0, "{head:?} vs {short:?}");
    assert!((short.left() - wrapped.left()).abs() < 1.0);
}
