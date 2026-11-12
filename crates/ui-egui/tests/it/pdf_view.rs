//! PDF files in their tab: opened read-only, drawn by the render threads, moved through with the
//! keys, two pages across, found in, selected and copied, and their bookmarks as the contents.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A PDF of A4 pages, each with its line of text (Helvetica 24 at 72, 720), and bookmarks
/// (title, page) to the top of their pages.
fn pdf(texts: &[&str], marks: &[(&str, usize)]) -> Vec<u8> {
    let n = texts.len();
    // Objects: 1 catalog, 2 pages, 3 outline, 4 font, then each page and its content, then marks.
    let page_id = |i: usize| 5 + 2 * i;
    let mark_id = |i: usize| 5 + 2 * n + i;
    let mut objects: Vec<String> = Vec::new();
    let outlines = if marks.is_empty() { String::new() } else { " /Outlines 3 0 R".into() };
    objects.push(format!("<< /Type /Catalog /Pages 2 0 R{outlines} >>"));
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_id(i))).collect();
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
    objects.push(if marks.is_empty() {
        "<< /Type /Outlines /Count 0 >>".into()
    } else {
        format!("<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>", mark_id(0), mark_id(marks.len() - 1), marks.len())
    });
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into());
    for (i, text) in texts.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents {} 0 R >>",
            page_id(i) + 1
        ));
        let content = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    for (i, (title, page)) in marks.iter().enumerate() {
        let prev = if i > 0 { format!(" /Prev {} 0 R", mark_id(i - 1)) } else { String::new() };
        let next = if i + 1 < marks.len() { format!(" /Next {} 0 R", mark_id(i + 1)) } else { String::new() };
        objects.push(format!("<< /Title ({title}) /Parent 3 0 R{prev}{next} /Dest [{} 0 R /XYZ 0 842 0] >>", page_id(*page)));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// Writes `bytes` to a file of the test's own and returns its path.
fn file(test: &str, name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("monun-pdf-{}-{test}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn harness(open: &str) -> Harness {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": open})).unwrap();
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
    monun_ui_egui::pdf_view::inspect(h.state())
}

/// Runs frames until `done` holds for the PDF's state (the render threads work meanwhile).
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

/// Waits until the pages in view are drawn at the zoom, and nothing is left to draw.
fn settle(h: &mut Harness) {
    wait(h, "pages drawn", |v| v["status"] == "ready" && v["queued"] == 0 && !v["rendered"].as_array().unwrap().is_empty());
}

fn has(v: &Value, key: &str, page: u64) -> bool {
    v[key].as_array().is_some_and(|a| a.contains(&json!(page)))
}

fn key(h: &mut Harness, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
}

/// Where on the screen a point of page `page` (PDF points from its top-left) is.
fn on_page(h: &Harness, page: u64, x: f32, y: f32) -> Pos2 {
    let v = inspect(h);
    let scale = v["scale"].as_f64().unwrap() as f32;
    let slot = v["onScreen"].as_array().unwrap().iter().find(|s| s[0] == page).unwrap_or_else(|| panic!("page {page} not in view: {v}"));
    pos2(slot[1].as_f64().unwrap() as f32 + x * scale, slot[2].as_f64().unwrap() as f32 + y * scale)
}

fn click(h: &mut Harness, at: Pos2, times: usize) {
    for _ in 0..times {
        h.input_mut().events.push(Event::PointerMoved(at));
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn a_pdf_opens_in_its_tab_drawn_and_nothing_writes_it() {
    let bytes = pdf(&["Hello world", "Second page", "Third page"], &[]);
    let path = file("open", "design.pdf", &bytes);
    let mut h = harness(&path);
    settle(&mut h);
    let v = inspect(&h);
    assert_eq!((v["pages"].as_u64(), v["page"].as_u64(), v["spread"].as_bool()), (Some(3), Some(1), Some(false)));
    assert!(has(&v, "rendered", 0), "{v}");
    // Fit Width: the page is as wide as the window, less the margins and the scroll bar's room.
    let width = v["onScreen"][0][3].as_f64().unwrap();
    assert!((1100.0..1175.0).contains(&width), "{v}");
    assert_eq!(h.state().session.active().unwrap().display_title(), "design.pdf");
    // Ctrl+S saves nothing over it.
    key(&mut h, Key::S, Modifiers::COMMAND);
    assert!(h.state_mut().run("file.save", json!({})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!h.state().session.active().unwrap().is_dirty());
    // Not a PDF, though named one: the tab says why.
    let broken = file("open", "broken.pdf", b"%PDF-1.7\nnothing here\n");
    h.state_mut().run("file.open", json!({"path": broken})).unwrap();
    wait(&mut h, "fails", |v| v["status"].as_str().is_some_and(|s| s.starts_with("failed")));
}

#[test]
fn the_keys_and_the_page_field_go_through_the_pages() {
    let texts: Vec<String> = (1..=8).map(|i| format!("Page {i}")).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let path = file("keys", "long.pdf", &pdf(&texts, &[]));
    let mut h = harness(&path);
    settle(&mut h);
    key(&mut h, Key::End, Modifiers::COMMAND);
    settle(&mut h);
    let v = inspect(&h);
    assert_eq!(v["page"].as_u64(), Some(8), "{v}");
    assert!(has(&v, "rendered", 7), "the last page is drawn: {v}");
    key(&mut h, Key::Home, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(1));
    key(&mut h, Key::PageDown, Modifiers::NONE);
    assert!(inspect(&h)["offset"][1].as_f64().unwrap() > 300.0);
    // A page number typed in the tool bar.
    let field = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Page").rect().center();
    click(&mut h, field, 1);
    key(&mut h, Key::A, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("5".into()));
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(5));
}

#[test]
fn two_pages_sit_side_by_side_and_the_choice_is_remembered() {
    let path = file("spread", "spread.pdf", &pdf(&["One", "Two", "Three"], &[]));
    let mut h = harness(&path);
    settle(&mut h);
    h.get_by_label("Two Pages").click();
    h.run_steps(3);
    settle(&mut h);
    let v = inspect(&h);
    assert_eq!(v["spread"].as_bool(), Some(true));
    let (a, b) = (&v["onScreen"][0], &v["onScreen"][1]);
    assert_eq!((a[0].as_u64(), b[0].as_u64()), (Some(0), Some(1)));
    assert_eq!(a[2], b[2], "the same row: {v}");
    assert!(b[1].as_f64().unwrap() > a[1].as_f64().unwrap() + a[3].as_f64().unwrap());
    // Closed and opened again, it comes back two across.
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.doc_views.iter().any(|m| m.path == path && m.spread), "{:?}", h.state().ui.doc_views);
    h.state_mut().run("file.open", json!({"path": path})).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&h)["spread"].as_bool(), Some(true));
}

#[test]
fn find_counts_the_matches_and_goes_to_each() {
    let path = file("find", "find.pdf", &pdf(&["alpha one", "beta", "gamma", "alpha two"], &[]));
    let mut h = harness(&path);
    settle(&mut h);
    key(&mut h, Key::F, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("ALPHA".into()));
    h.run_steps(2);
    wait(&mut h, "every page searched", |v| v["hits"] == 2 && v["texts"].as_array().is_some_and(|t| t.len() == 4));
    assert!(h.query_by_label("-/2").is_some());
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(inspect(&h)["hit"].as_u64(), Some(0));
    key(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(2);
    let v = inspect(&h);
    assert_eq!((v["hit"].as_u64(), v["page"].as_u64()), (Some(1), Some(4)), "{v}");
    assert!(h.query_by_label("2/2").is_some());
}

#[test]
fn find_reads_every_page_of_a_long_pdf_and_then_rests() {
    // More pages than the text layers kept, the matches far apart.
    let texts: Vec<String> = (1..=300).map(|i| if [1, 150, 300].contains(&i) { format!("needle {i}") } else { format!("hay {i}") }).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let path = file("long-find", "long.pdf", &pdf(&texts, &[]));
    let mut h = harness(&path);
    settle(&mut h);
    key(&mut h, Key::F, Modifiers::COMMAND);
    h.input_mut().events.push(Event::Text("needle".into()));
    h.run_steps(2);
    wait(&mut h, "every page searched, then nothing left to do", |v| v["hits"] == 3 && v["queued"] == 0);
    // The frame the last page was searched in asks for one more, to show the count.
    h.run_steps(1);
    assert!(h.query_by_label("-/3").is_some(), "no '…': every page was read");
    assert!(inspect(&h)["texts"].as_array().unwrap().len() <= 260, "the text layers kept stay bounded");
    key(&mut h, Key::Enter, Modifiers::NONE);
    key(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(150));
}

#[test]
fn a_double_click_selects_a_word_and_ctrl_c_copies_it() {
    let path = file("select", "select.pdf", &pdf(&["Hello world", "Next"], &[]));
    let mut h = harness(&path);
    settle(&mut h);
    wait(&mut h, "the page's text", |v| has(v, "texts", 0));
    // "Hello" runs from 72 pt; its letters stand above the baseline 122 pt down the page.
    let at = on_page(&h, 0, 90.0, 112.0);
    click(&mut h, at, 2);
    assert_eq!(inspect(&h)["selection"].as_str(), Some("Hello"));
    h.input_mut().events.push(Event::Copy);
    h.run_steps(1);
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "Hello"));
    assert!(copied);
    // A drag from before "Hello" to past "world" selects both words.
    let (from, to) = (on_page(&h, 0, 60.0, 112.0), on_page(&h, 0, 300.0, 112.0));
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=5 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (step as f32 / 5.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(inspect(&h)["selection"].as_str(), Some("Hello world"));
    // Esc drops it.
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(inspect(&h)["selection"].is_null());
}

#[test]
fn the_contents_list_the_bookmarks_and_go_to_them() {
    let path = file("contents", "contents.pdf", &pdf(&["Intro", "Middle", "End"], &[("Introduction", 0), ("Closing", 2)]));
    let mut h = harness(&path);
    settle(&mut h);
    wait(&mut h, "the bookmarks", |v| v["contents"] == json!(["Introduction", "Closing"]));
    if !h.state().ui.doc_outline {
        h.get_by_label("Contents").click();
        h.run_steps(2);
    }
    h.get_by_label("Closing").click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["page"].as_u64(), Some(3));
}
