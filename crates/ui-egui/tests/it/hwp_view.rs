//! 한글 documents in their tab: opened and drawn, read in View mode and written in Edit mode
//! (typing, an input method's commit, formatting, a table and its cells, Undo), saved as .hwp and
//! .hwpx, made a PDF, asked about on closing, found in, and their outline as the contents.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A document made with rhwp: an outline heading over a paragraph, twice. As .hwpx, or as .hwp.
fn document(hwpx: bool) -> Vec<u8> {
    let mut core = rhwp::DocumentCore::new_empty();
    core.create_blank_document_native().unwrap();
    let mut para = 0;
    for (i, (text, style)) in [("Overview", 2), ("Hello world from page one.", 0), ("Details", 2), ("Hello again.", 0)].into_iter().enumerate() {
        if i > 0 {
            let len = core.get_paragraph_length_native(0, para).unwrap();
            core.split_paragraph_native(0, para, len, None).unwrap();
            para += 1;
        }
        core.apply_style_native(0, para, style).unwrap();
        core.insert_text_native(0, para, 0, text).unwrap();
    }
    if hwpx { core.export_hwpx_native().unwrap() } else { core.export_hwp_with_adapter_snapshot_with_report().unwrap().into_parts().0 }
}

/// The text of a .hwp or .hwpx file, its tables' cells included.
fn text_of(path: &str) -> String {
    fn walk(paragraphs: &[rhwp::model::paragraph::Paragraph], out: &mut String) {
        for p in paragraphs {
            out.push_str(&p.text);
            out.push('\n');
            for c in &p.controls {
                if let rhwp::model::control::Control::Table(t) = c {
                    for cell in &t.cells {
                        walk(&cell.paragraphs, out);
                    }
                }
            }
        }
    }
    let doc = rhwp::parse_document(&std::fs::read(path).unwrap()).unwrap();
    let mut out = String::new();
    for s in &doc.sections {
        walk(&s.paragraphs, &mut out);
    }
    out
}

fn file(test: &str, name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("monun-hwp-{}-{test}", std::process::id()));
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
    // The tool bar shows the document's font (함초롬바탕 in a new 한글 document): the test's fonts
    // have no Hangul, so it shows as boxes rather than stopping the test.
    h.ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    h.run_steps(2);
    h
}

fn inspect(h: &Harness) -> Value {
    monun_ui_egui::hwp_view::inspect(h.state())
}

/// Runs frames until `done` holds for the document's state (its threads work meanwhile).
fn wait(h: &mut Harness, what: &str, done: impl Fn(&Value) -> bool) {
    for _ in 0..600 {
        h.run_steps(1);
        let v = inspect(h);
        if done(&v) && v["busy"] == json!(false) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{what}: {}", inspect(h));
}

/// Waits until the document's thread has done what it was asked.
fn idle(h: &mut Harness) {
    wait(h, "the document idle", |_| true);
}

fn settle(h: &mut Harness) {
    wait(h, "pages drawn", |v| v["status"] == "ready" && v["queued"] == 0 && !v["rendered"].as_array().unwrap().is_empty());
}

fn key(h: &mut Harness, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
    idle(h);
}

fn click(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    idle(h);
}

fn typed(h: &mut Harness, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    h.run_steps(2);
    idle(h);
}

/// Where a point of page `page` (points from its top-left) is on the screen.
fn on_page(h: &Harness, page: u64, x: f32, y: f32) -> Pos2 {
    let v = inspect(h);
    let scale = v["scale"].as_f64().unwrap() as f32;
    let slot = v["onScreen"].as_array().unwrap().iter().find(|s| s[0] == page).unwrap_or_else(|| panic!("page {page}: {v}"));
    pos2(slot[1].as_f64().unwrap() as f32 + x * scale, slot[2].as_f64().unwrap() as f32 + y * scale)
}

/// Switches the tab to Edit mode and gives the first page the keyboard, the caret at the end of
/// the document.
fn edit(h: &mut Harness) {
    h.get_by_label("Edit").click();
    h.run_steps(2);
    let at = on_page(h, 0, 300.0, 120.0);
    click(h, at);
    key(h, Key::End, Modifiers::COMMAND);
}

fn text(h: &Harness) -> String {
    inspect(h)["text"].as_str().unwrap_or_default().to_owned()
}

#[test]
fn a_hwp_opens_in_its_tab_and_reads_in_view_mode() {
    let path = file("open", "plan.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    let v = inspect(&h);
    assert_eq!(v["pages"].as_u64(), Some(1), "{v}");
    assert!(text(&h).contains("Hello world from page one."), "{}", text(&h));
    assert_eq!(h.state().session.active().unwrap().display_title(), "plan.hwp");
    assert!(!h.state().session.active().unwrap().is_dirty());
    // View mode: typing goes nowhere.
    let at = on_page(&h, 0, 300.0, 120.0);
    click(&mut h, at);
    typed(&mut h, "xyz");
    assert!(!text(&h).contains("xyz"));
    assert!(std::fs::read(&path).unwrap() == document(false) || text_of(&path).contains("Hello again."), "the file is as it was");
}

#[test]
fn edit_mode_types_saves_and_undoes() {
    let path = file("edit", "notes.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    typed(&mut h, " Typed here.");
    assert!(text(&h).contains("Hello again. Typed here."), "{}", text(&h));
    assert!(h.state().session.active().unwrap().is_dirty(), "the tab shows the unsaved change");
    // An input method's commit goes in too.
    h.input_mut().events.push(Event::Ime(egui::ImeEvent::Commit(" ime".into())));
    h.run_steps(2);
    idle(&mut h);
    assert!(text(&h).contains("Typed here. ime"), "{}", text(&h));
    // Ctrl+S writes the .hwp and the tab is clean.
    key(&mut h, Key::S, Modifiers::COMMAND);
    assert!(text_of(&path).contains("Typed here. ime"), "{}", text_of(&path));
    assert!(!h.state().session.active().unwrap().is_dirty());
    // Undo takes typing back.
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert!(!text(&h).contains(" ime"), "{}", text(&h));
    assert!(h.state().session.active().unwrap().is_dirty());
}

#[test]
fn a_hwpx_saves_as_hwpx_and_enter_makes_a_paragraph() {
    let path = file("hwpx", "report.hwpx", &document(true));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    key(&mut h, Key::Enter, Modifiers::NONE);
    typed(&mut h, "Next line");
    key(&mut h, Key::Backspace, Modifiers::NONE);
    key(&mut h, Key::S, Modifiers::COMMAND);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"PK"), "saved as .hwpx");
    let saved = text_of(&path);
    assert!(saved.contains("Hello again.\nNext lin\n"), "{saved}");
}

#[test]
fn bold_from_the_tool_bar_and_from_the_keyboard() {
    let path = file("bold", "bold.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    key(&mut h, Key::A, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["selection"], json!(true));
    h.get_by_label("B").click();
    h.run_steps(3);
    idle(&mut h);
    assert_eq!(inspect(&h)["format"]["bold"], json!(true), "{}", inspect(&h));
    key(&mut h, Key::B, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["format"]["bold"], json!(false), "{}", inspect(&h));
}

#[test]
fn a_table_is_inserted_and_its_cells_take_typing() {
    let path = file("table", "table.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    key(&mut h, Key::Enter, Modifiers::NONE);
    // The table menu's grid: two columns, two rows.
    h.get_by_label("Table ▾").click();
    h.run_steps(2);
    h.get_by_label("2 × 2").click_accesskit();
    h.run_steps(2);
    idle(&mut h);
    assert_eq!(inspect(&h)["inTable"], json!(true), "{}", inspect(&h));
    typed(&mut h, "A1");
    key(&mut h, Key::Tab, Modifiers::NONE);
    typed(&mut h, "B1");
    let t = text(&h);
    assert!(t.contains("A1") && t.contains("B1"), "{t}");
    key(&mut h, Key::S, Modifiers::COMMAND);
    let saved = text_of(&path);
    assert!(saved.contains("A1\n") && saved.contains("B1\n"), "{saved}");
}

#[test]
fn closing_an_edited_document_asks_to_save_it() {
    let path = file("close", "close.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    typed(&mut h, "!");
    let r = h.state_mut().run("file.close", json!({})).unwrap();
    assert_eq!(r["closed"], json!(false), "{r}");
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("saveChanges"));
}

#[test]
fn find_counts_and_the_contents_list_the_outline() {
    let path = file("find", "find.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    key(&mut h, Key::F, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("Hello".into()));
    h.run_steps(3);
    wait(&mut h, "found", |v| v["found"]["count"].as_u64().is_some());
    assert_eq!(inspect(&h)["found"]["count"].as_u64(), Some(2), "{}", inspect(&h));
    assert_eq!(inspect(&h)["selection"], json!(true), "the match is selected");
    if !h.state().ui.doc_outline {
        h.get_by_label("Contents").click();
        h.run_steps(3);
    }
    wait(&mut h, "the outline", |v| v["headings"].is_array());
    let headings = inspect(&h)["headings"].clone();
    assert!(headings.as_array().unwrap().iter().any(|t| t.as_str().is_some_and(|t| t.contains("Details"))), "{headings}");
}

#[test]
fn printing_sends_every_page_and_a_new_document_is_a_hwpx() {
    let path = file("print", "print.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    // A PDF isn't exported: printing makes one.
    let e = h.state_mut().run("file.exportPdf", json!({"path": path.replace(".hwp", ".pdf")})).unwrap_err();
    assert!(e.contains("Print"), "{e}");
    let printed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let into = printed.clone();
    h.state_mut().services.print = Some(Box::new(move |pdf: &[u8]| *into.lock().unwrap() = pdf.to_vec()));
    let r = h.state_mut().run("ui.printSubmit", json!({})).unwrap();
    assert_eq!(r["pending"], json!(true), "{r}");
    for _ in 0..600 {
        h.run_steps(1);
        if !printed.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pdf = printed.lock().unwrap().clone();
    assert!(pdf.starts_with(b"%PDF"));
    assert_eq!(lopdf::Document::load_mem(&pdf).unwrap().get_pages().len(), 1);
    // A new document, typed and saved as.
    h.state_mut().run("file.new", json!({"document": true, "kind": "hwp"})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().display_title(), "Document1.hwpx");
    edit(&mut h);
    typed(&mut h, "Fresh");
    let saved = path.replace("print.hwp", "fresh.hwpx");
    h.state_mut().run("file.save", json!({"path": saved})).unwrap();
    assert!(std::fs::read(&saved).unwrap().starts_with(b"PK"));
    assert!(text_of(&saved).contains("Fresh"), "{}", text_of(&saved));
    assert_eq!(h.state().session.active().unwrap().display_title(), "fresh.hwpx");
}

#[test]
fn cut_takes_the_selection_out_and_is_saved() {
    let path = file("cut", "cut.hwp", &document(false));
    let mut h = harness(Some(&path));
    settle(&mut h);
    edit(&mut h);
    // The last paragraph's text, selected back to its start.
    key(&mut h, Key::Home, Modifiers::SHIFT);
    assert_eq!(inspect(&h)["selection"], json!(true));
    let rev = inspect(&h)["rev"].clone();
    h.input_mut().events.push(Event::Cut);
    h.run_steps(2);
    idle(&mut h);
    assert!(!text(&h).contains("Hello again."), "{}", text(&h));
    assert_ne!(inspect(&h)["rev"], rev, "the pages are drawn again");
    assert!(h.state().session.active().unwrap().is_dirty(), "a cut is an unsaved change");
    key(&mut h, Key::S, Modifiers::COMMAND);
    assert!(!text_of(&path).contains("Hello again."), "{}", text_of(&path));
}
