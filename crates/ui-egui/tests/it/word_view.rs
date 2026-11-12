//! Word documents in their tab: opened and drawn, read in View mode and written in Edit mode
//! (typing, an input method's commit, formatting, Undo), saved as .docx, exported as a PDF, asked
//! about on closing, found in, and their headings as the contents.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A .docx made with WordCraft: a heading, paragraphs and page breaks, so it has three pages.
fn docx() -> Vec<u8> {
    let doc = wordcraft_engine::blank_document(&wordcraft_engine::Region::default());
    let mut s = wordcraft_engine::Session::new(doc);
    let mut run = |id: &str, p: Value| {
        s.run(id, &p).unwrap();
    };
    run("text.insert", json!({"text": "Overview", "raw": true}));
    run("para.heading1", json!({}));
    run("text.newParagraph", json!({}));
    run("para.normal", json!({}));
    run("text.insert", json!({"text": "Hello world from page one.", "raw": true}));
    run("text.pageBreak", json!({}));
    run("text.newParagraph", json!({}));
    run("text.insert", json!({"text": "Details", "raw": true}));
    run("para.heading1", json!({}));
    run("text.newParagraph", json!({}));
    run("para.normal", json!({}));
    run("text.insert", json!({"text": "Hello again on page two.", "raw": true}));
    run("text.pageBreak", json!({}));
    run("text.newParagraph", json!({}));
    run("text.insert", json!({"text": "The end.", "raw": true}));
    wordcraft_engine::io::save_bytes("test.docx", &s.doc).unwrap()
}

/// The body text of a .docx file.
fn text_of(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap();
    let doc = wordcraft_engine::io::open_bytes("x.docx", &bytes).unwrap();
    doc.plain_text(wordcraft_engine::doc::StoryRef::Body)
}

fn file(test: &str, name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("monun-word-{}-{test}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn harness(open: Option<&str>) -> Harness {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    if let Some(path) = open {
        app.run("file.open", json!({"path": path})).unwrap();
    }
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    h
}

fn inspect(h: &Harness) -> Value {
    monun_ui_egui::word_view::inspect(h.state())
}

fn wait(h: &mut Harness, what: &str, done: impl Fn(&Value) -> bool) {
    for _ in 0..600 {
        h.run_steps(1);
        if done(&inspect(h)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{what}: {}", inspect(h));
}

fn settle(h: &mut Harness) {
    wait(h, "pages drawn", |v| v["status"] == "ready" && v["queued"] == 0 && !v["rendered"].as_array().unwrap().is_empty());
}

fn key(h: &mut Harness, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
}

fn click(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Where a point of page `page` (points from its top-left) is on the screen.
fn on_page(h: &Harness, page: u64, x: f32, y: f32) -> Pos2 {
    let v = inspect(h);
    let scale = v["scale"].as_f64().unwrap() as f32;
    let slot = v["onScreen"].as_array().unwrap().iter().find(|s| s[0] == page).unwrap_or_else(|| panic!("page {page}: {v}"));
    pos2(slot[1].as_f64().unwrap() as f32 + x * scale, slot[2].as_f64().unwrap() as f32 + y * scale)
}

/// Switches the tab to Edit mode and gives the first page the keyboard, the caret at its end.
fn edit(h: &mut Harness) {
    h.get_by_label("Edit").click();
    h.run_steps(2);
    let at = on_page(h, 0, 300.0, 90.0);
    click(h, at);
}

fn text(h: &Harness) -> String {
    inspect(h)["text"].as_str().unwrap_or_default().to_owned()
}

#[test]
fn a_docx_opens_in_its_tab_and_reads_in_view_mode() {
    let path = file("open", "spec.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    let v = inspect(&h);
    assert_eq!(v["pages"].as_u64(), Some(3), "{v}");
    assert_eq!(h.state().session.active().unwrap().display_title(), "spec.docx");
    assert!(!h.state().session.active().unwrap().is_dirty());
    // View mode: typing goes nowhere.
    let at = on_page(&h, 0, 300.0, 90.0);
    click(&mut h, at);
    h.input_mut().events.push(Event::Text("xyz".into()));
    h.run_steps(2);
    assert!(!text(&h).contains("xyz"));
    // Ctrl+End goes to the last page.
    key(&mut h, Key::End, Modifiers::COMMAND);
    settle(&mut h);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(3));
}

#[test]
fn edit_mode_types_saves_and_undoes() {
    let path = file("edit", "notes.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    h.input_mut().events.push(Event::Text(" Typed here.".into()));
    h.run_steps(2);
    assert!(text(&h).contains("Typed here."), "{}", text(&h));
    assert!(h.state().session.active().unwrap().is_dirty(), "the tab shows the unsaved change");
    // An input method's commit goes in too.
    h.input_mut().events.push(Event::Ime(egui::ImeEvent::Commit(" ime".into())));
    h.run_steps(2);
    assert!(text(&h).contains("Typed here. ime"), "{}", text(&h));
    // Ctrl+S writes the .docx and the tab is clean.
    key(&mut h, Key::S, Modifiers::COMMAND);
    h.run_steps(2);
    assert!(text_of(&path).contains("Typed here. ime"), "{}", text_of(&path));
    assert!(!h.state().session.active().unwrap().is_dirty());
    // Undo takes typing back.
    key(&mut h, Key::Z, Modifiers::COMMAND);
    h.run_steps(2);
    assert!(!text(&h).contains(" ime"), "{}", text(&h));
}

#[test]
fn bold_from_the_tool_bar_and_from_the_keyboard() {
    let path = file("bold", "bold.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    key(&mut h, Key::A, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["selection"], json!(true));
    h.get_by_label("B").click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["format"]["bold"], json!(true), "{}", inspect(&h));
    key(&mut h, Key::B, Modifiers::COMMAND);
    h.run_steps(2);
    assert_eq!(inspect(&h)["format"]["bold"], json!(false), "{}", inspect(&h));
}

#[test]
fn closing_an_edited_document_asks_to_save_it() {
    let path = file("close", "close.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    h.input_mut().events.push(Event::Text("!".into()));
    h.run_steps(2);
    let r = h.state_mut().run("file.close", json!({})).unwrap();
    assert_eq!(r["closed"], json!(false), "{r}");
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("saveChanges"));
}

#[test]
fn find_counts_and_the_contents_go_to_a_heading() {
    let path = file("find", "find.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    key(&mut h, Key::F, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("Hello".into()));
    h.run_steps(3);
    assert_eq!(inspect(&h)["found"]["count"].as_u64(), Some(2), "{}", inspect(&h));
    if !h.state().ui.doc_outline {
        h.get_by_label("Contents").click();
        h.run_steps(3);
    }
    assert_eq!(inspect(&h)["headings"], json!(["Overview", "Details"]));
    h.get_by_label("Details").click();
    h.run_steps(3);
    settle(&mut h);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(2));
}

#[test]
fn a_pdf_is_exported_and_a_new_document_is_saved_where_asked() {
    let path = file("pdf", "export.docx", &docx());
    let mut h = harness(Some(&path));
    settle(&mut h);
    let pdf = path.replace(".docx", ".pdf");
    h.state_mut().run("file.exportPdf", json!({"path": pdf})).unwrap();
    for _ in 0..300 {
        h.run_steps(1);
        if std::fs::read(&pdf).is_ok_and(|b| b.starts_with(b"%PDF")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    // A new document, typed and saved as.
    h.state_mut().run("file.new", json!({"document": true, "kind": "word"})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().display_title(), "Document1.docx");
    edit(&mut h);
    h.input_mut().events.push(Event::Text("Fresh".into()));
    h.run_steps(2);
    let saved = path.replace("export.docx", "fresh.docx");
    h.state_mut().run("file.save", json!({"path": saved})).unwrap();
    assert!(text_of(&saved).contains("Fresh"));
    assert_eq!(h.state().session.active().unwrap().display_title(), "fresh.docx");
}
