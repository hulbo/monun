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
fn side_by_side_shows_the_text_beside_the_document() {
    let mut h = with_document("# Plan\n");
    h.get_by_label("Side by Side").click();
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
    h.get_by_label("Side by Side").click();
    h.run_steps(3);
    press(&mut h, Key::End, Modifiers::COMMAND);
    h.get_by_label("Picture").click();
    h.run_steps(4);
    let text = h.state().doc_view.draft.as_ref().unwrap().text.clone();
    let d = h.state().session.active().unwrap();
    let image = d.wb.sheet(1).unwrap().images.first().expect("the picture is on the document tab").clone();
    assert_eq!(text, format!("# Plan\n\n![chart](image:{})", image.id), "a paragraph of its own");
    assert_eq!(image.anchor.cell.a1(), "C3", "beside the picture's line, right of the text");
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

fn paste_picture(h: &mut Harness) {
    let image = egui::ColorImage::filled([40, 20], egui::Color32::from_rgb(30, 120, 200));
    h.input_mut().events.push(Event::PasteImage(std::sync::Arc::new(image)));
    h.run_steps(4);
}

#[test]
fn a_pasted_picture_goes_in_at_the_caret() {
    let mut h = with_document("# Plan\n\nfirst line\nsecond line\n");
    h.get_by_label("Side by Side").click();
    h.run_steps(4);
    // The caret starts at the end; put it after "first line".
    for _ in 0..2 {
        press(&mut h, Key::ArrowUp, Modifiers::NONE);
    }
    press(&mut h, Key::End, Modifiers::NONE);
    paste_picture(&mut h);
    let d = h.state().session.active().unwrap();
    let image = d.wb.sheet(1).unwrap().images.first().expect("the picture is on the document tab").clone();
    assert_eq!(image.anchor.width.round(), 40.0);
    assert!(image.data.starts_with(&[0x89, b'P', b'N', b'G']), "kept as PNG");
    let text = h.state().doc_view.draft.as_ref().unwrap().text.clone();
    assert_eq!(text, format!("# Plan\n\nfirst line\n\n![Picture {0}](image:{0})\n\nsecond line\n", image.id));
    assert_eq!(image.anchor.cell.a1(), "C5", "beside the picture's line");
    // The rendered document shows it.
    assert!(h.query_by_label(&format!("Picture {}", image.id)).is_some());
}

#[test]
fn a_picture_pasted_in_view_mode_goes_at_the_end() {
    let mut h = with_document("# Plan\n\ntext");
    paste_picture(&mut h);
    let d = h.state().session.active().unwrap();
    let id = d.wb.sheet(1).unwrap().images.first().expect("the picture is on the document tab").id;
    assert_eq!(h.state().doc_view.draft.as_ref().unwrap().text, format!("# Plan\n\ntext\n\n![Picture {id}](image:{id})"));
}

#[test]
fn a_new_document_stays_in_edit_mode_once_its_text_is_in() {
    let mut h = blank_harness(4);
    h.get_by_label("New document (Markdown)").click();
    h.run_steps(4);
    h.input_mut().events.push(Event::Paste("# Notes\n\n- one".into()));
    h.run_steps(3);
    // The draft reaches the workbook after a pause in typing; the document then has text, and
    // still shows the editor it was opened with.
    std::thread::sleep(std::time::Duration::from_millis(1600));
    h.run_steps(3);
    assert_eq!(doc_text(&h, 1), "# Notes\n\n- one");
    assert!(h.query_by_label("Document text").is_some(), "still in Edit mode");
}

#[test]
fn fit_width_widens_tables_to_the_page() {
    let mut h = with_document("| A | B |\n|---|---|\n| 1 | 2 |\n");
    let before = h.get_by_label("B").rect();
    h.get_by_label("Fit Width").click();
    h.run_steps(3);
    let d = h.state().session.active().unwrap();
    assert!(d.wb.sheet(1).unwrap().document.as_ref().unwrap().fit_width, "the document keeps it");
    let after = h.get_by_label("B").rect();
    assert!(after.center().x > before.center().x + 200.0, "the last column reaches the page's edge: {before:?} → {after:?}");
    // Off again, the table is as wide as its cells.
    h.get_by_label("Fit Width").click();
    h.run_steps(3);
    assert!((h.get_by_label("B").rect().center().x - before.center().x).abs() < 1.0);
}

#[test]
fn each_document_keeps_its_own_mode_and_fit_width_in_its_file() {
    use monun_engine::model::DocMode;
    let mut s: Session = blank_session();
    s.run("doc.insert", json!({"name": "One", "text": "# One\n"})).unwrap();
    s.run("doc.insert", json!({"name": "Two", "text": "# Two\n"})).unwrap();
    let mut h = harness(s, 4);
    let undo = h.state().session.active().unwrap().undo.len();
    let sheet = |h: &Harness| h.state().session.active().unwrap().wb.active().unwrap().name.clone();
    assert_eq!(sheet(&h), "Two");
    // Two: side by side, with Fit Width.
    h.get_by_label("Side by Side").click();
    h.run_steps(3);
    h.get_by_label("Fit Width").click();
    h.run_steps(3);
    // One: its text alone.
    press(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert_eq!(sheet(&h), "One");
    assert!(h.query_by_label("One").is_some(), "a document with text opens to read");
    h.get_by_label("Edit").click();
    h.run_steps(3);
    assert!(h.query_by_label("Document text").is_some() && h.query_by_label("One").is_none(), "only the text shows");
    // Each tab comes back as it was left.
    press(&mut h, Key::PageDown, Modifiers::COMMAND);
    assert!(h.query_by_label("Document text").is_some() && h.query_by_label("Two").is_some(), "Two is side by side again");
    press(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert!(h.query_by_label("One").is_none());
    // The choices are the workbook's: it has changed, with nothing to undo, and its file keeps them.
    let d = h.state().session.active().unwrap();
    assert!(d.is_dirty());
    assert_eq!(d.undo.len(), undo, "switching modes leaves Undo for the text");
    let bytes = monun_engine::io::save_bytes(&d.wb, "book.xlsx").unwrap();
    let (wb, _) = monun_engine::io::open_bytes("book.xlsx", &bytes).unwrap();
    let doc = |n: &str| wb.sheets.iter().find(|s| s.name == n).and_then(|s| s.document.clone()).unwrap();
    assert_eq!((doc("One").mode, doc("One").fit_width), (Some(DocMode::Edit), false));
    assert_eq!((doc("Two").mode, doc("Two").fit_width), (Some(DocMode::Split), true));
}

/// A Ctrl+wheel turn at `at`, `dy` notches (+ in), and the frames its glide takes.
fn ctrl_wheel(h: &mut Harness, at: egui::Pos2, dy: f32) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.input_mut().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        source: egui::MouseWheelSource::Wheel,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::CTRL | Modifiers::COMMAND,
    });
    h.run_steps(40);
}

#[test]
fn ctrl_and_the_wheel_zoom_a_document_a_step_a_notch() {
    let text: String = (0..200).map(|i| format!("Line {i}\n\n")).collect();
    let mut h = with_document(&text);
    let zoom = |h: &Harness| h.state().session.active().unwrap().wb.active().unwrap().zoom;
    let at = h.get_by_label("Line 1").rect().center();
    assert_eq!(zoom(&h), 100);
    // The grid's steps: 100 → 120, and back out past 100 to 85.
    ctrl_wheel(&mut h, at, 1.0);
    assert_eq!(zoom(&h), 120);
    ctrl_wheel(&mut h, at, -1.0);
    ctrl_wheel(&mut h, at, -1.0);
    assert_eq!(zoom(&h), 85);
    // In Edit mode too, and the text stays there to type into.
    h.get_by_label("Edit").click();
    h.run_steps(3);
    ctrl_wheel(&mut h, at, 1.0);
    assert_eq!(zoom(&h), 100);
    assert!(h.query_by_label("Document text").is_some());
    // The zoom is the document tab's own: Sheet1 keeps 100%, and its grid zooms the same way.
    assert_eq!(h.state().session.active().unwrap().wb.sheet(0).unwrap().zoom, 100);
    press(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet(), 0, "on Sheet1");
    let grid = h.state().grid.rect.unwrap().center();
    ctrl_wheel(&mut h, grid, 1.0);
    assert_eq!(h.state().session.active().unwrap().wb.sheet(0).unwrap().zoom, 120);
    assert_eq!(h.state().session.active().unwrap().wb.sheet(1).unwrap().zoom, 100);
}
