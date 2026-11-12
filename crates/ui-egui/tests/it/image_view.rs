//! Pictures in their tab: read off the UI thread, shown whole in the window, or at their actual
//! size; their size and kind on the status bar; nothing writes them.

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

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
    monun_ui_egui::image_view::inspect(h.state())
}

fn wait_ready(h: &mut Harness) {
    for _ in 0..300 {
        h.run_steps(1);
        if inspect(h)["status"] == "ready" {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{}", inspect(h));
}

fn zoom(h: &Harness) -> u16 {
    h.state().session.active().and_then(|d| d.wb.active()).map_or(0, |sh| sh.zoom)
}

#[test]
fn a_picture_is_shown_whole_or_at_its_size() {
    let dir = std::env::temp_dir().join(format!("monun-image-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("wide.png");
    // Wider than the window: fitted, it shows smaller than its own size.
    let mut pixels = image::RgbaImage::new(2400, 300);
    for (x, _, p) in pixels.enumerate_pixels_mut() {
        *p = image::Rgba([(x % 256) as u8, 120, 200, 255]);
    }
    pixels.save(&path).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut h = harness(&path);
    wait_ready(&mut h);
    let v = inspect(&h);
    assert_eq!((v["width"].as_u64(), v["height"].as_u64(), v["format"].as_str()), (Some(2400), Some(300), Some("PNG")), "{v}");
    h.run_steps(3);
    assert!(zoom(&h) < 100, "fitted to the window: {}", zoom(&h));
    let status = monun_ui_egui::image_view::status(h.state()).unwrap();
    assert!(status.starts_with("2400 × 300 · PNG ·"), "{status}");
    h.get_by_label("Actual Size").click();
    h.run_steps(3);
    assert_eq!(zoom(&h), 100);
    // Nothing writes it.
    assert!(h.state_mut().run("file.save", json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_isnt_a_picture_says_so() {
    let dir = std::env::temp_dir().join(format!("monun-image-bad-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("broken.jpg");
    std::fs::write(&path, b"not a picture").unwrap();
    let mut h = harness(&path.to_string_lossy());
    for _ in 0..100 {
        h.run_steps(1);
        if inspect(&h)["status"].as_str().is_some_and(|s| s.starts_with("failed")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(inspect(&h)["status"], json!("failed: damaged"));
    let _ = std::fs::remove_dir_all(&dir);
}
