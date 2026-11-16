//! Presentation tabs: a PowerPoint file's slides written as a page on a thread of their own and
//! shown as a web tab's page, written again when the file changes, gone when the tab closes.

use std::io::{Cursor, Write};

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::web_view::Order;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

fn rels(list: &[(&str, &str)]) -> String {
    let body: String = list
        .iter()
        .map(|(id, target)| {
            let kind = if target.contains("slides/") { "slide" } else { "officeDocument" };
            format!(
                r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
            )
        })
        .collect();
    format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#)
}

/// A presentation of `texts.len()` slides, each a text box with its text (no layout, master or
/// theme: what's missing is left out).
fn presentation(texts: &[&str]) -> Vec<u8> {
    let ids: String = (0..texts.len()).map(|i| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + i, i + 1)).collect();
    let targets: Vec<(String, String)> = (0..texts.len()).map(|i| (format!("rId{}", i + 1), format!("slides/slide{}.xml", i + 1))).collect();
    let pairs: Vec<(&str, &str)> = targets.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut parts: Vec<(String, String)> = vec![
        ("_rels/.rels".into(), rels(&[("rId1", "ppt/presentation.xml")])),
        (
            "ppt/presentation.xml".into(),
            format!(r#"<p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="5143500"/></p:presentation>"#),
        ),
        ("ppt/_rels/presentation.xml.rels".into(), rels(&pairs)),
    ];
    for (i, text) in texts.iter().enumerate() {
        parts.push((
            format!("ppt/slides/slide{}.xml", i + 1),
            format!(
                r#"<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="914400"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            ),
        ));
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, text) in parts {
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn harness(path: &std::path::Path) -> Harness {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    app.web.host = Some(Ok(()));
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            // Korean names: this machine's test font set has no Hangul.
            ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    h
}

fn inspect(h: &Harness) -> Value {
    monun_ui_egui::web_view::inspect(h.state())
}

/// Frames until `done` says so (the page is written on a thread).
fn until(h: &mut Harness, done: impl Fn(&Value) -> bool) {
    for _ in 0..400 {
        h.run_steps(1);
        if done(&inspect(h)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    panic!("{}", inspect(h));
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("monun-slides-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_presentation_shows_its_slides_as_a_page_written_again_when_it_changes() {
    let dir = temp_dir("show");
    let path = dir.join("발표 자료.pptx");
    std::fs::write(&path, presentation(&["첫 장", "둘째 장"])).unwrap();
    let mut h = harness(&path);
    assert!(h.state().session.active().unwrap().is_slides());
    until(&mut h, |v| v["slides"] == 2);
    let v = inspect(&h);
    assert_eq!(v["presentation"], true);
    // Its page, written into this run's folder, shown in the tab.
    let url = v["address"].as_str().unwrap().to_string();
    assert!(url.starts_with("file:///") && url.ends_with("/index.html"), "{url}");
    assert_eq!(v["shown"], true, "{v}");
    let uid = h.state().session.active().unwrap().uid;
    let folder = std::env::temp_dir().join("monun-slides").join(format!("{}-{uid}", std::process::id()));
    let page = std::fs::read_to_string(folder.join("index.html")).unwrap();
    assert!(page.contains("첫 장") && page.contains("둘째 장") && page.contains("<title>발표 자료.pptx</title>"));
    // Its tool bar and status bar: how many slides; and the app Windows opens it with.
    assert_eq!(h.query_all_by_label("2 slides").count(), 2);
    assert!(h.query_by_label("Open in Default App").is_some());
    assert_eq!(monun_ui_egui::web_view::status(h.state()).as_deref(), Some("2 slides"));
    // Saved again (in PowerPoint): written again and shown again.
    h.state_mut().web.orders.clear();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&path, presentation(&["첫 장", "둘째 장", "셋째 장"])).unwrap();
    until(&mut h, |v| v["slides"] == 3);
    assert_eq!(h.state().web.orders.last().map(|o| o.1), Some(Order::Reload));
    assert!(std::fs::read_to_string(folder.join("index.html")).unwrap().contains("셋째 장"));
    // Nothing writes it; closed, its page goes.
    assert!(h.state_mut().run("file.save", json!({})).is_err());
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    assert!(!folder.exists(), "{folder:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_is_no_pptx_says_so() {
    let dir = temp_dir("bad");
    let path = dir.join("old.pptx");
    std::fs::write(&path, b"\xD0\xCF\x11\xE0 a .ppt saved as .pptx").unwrap();
    let mut h = harness(&path);
    until(&mut h, |v| v["error"].as_str().is_some_and(|e| e.contains(".pptx")));
    assert_eq!(inspect(&h)["shown"], false);
    let _ = std::fs::remove_dir_all(&dir);
}
