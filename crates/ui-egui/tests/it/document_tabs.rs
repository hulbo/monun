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
