//! Web tabs: an HTML file or a site link (.url) shown in its tab by the desktop app's web view.
//! These tests stand in for the desktop app: they read the pages the tab lists for the window
//! (`web.panes`) and what the user asked of them (`web.orders`), and say a web view is there
//! (`web.host`) or not.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::web_view::Order;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(app: SheetApp) -> Harness {
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            // Korean names and titles: this machine's test font set has no Hangul.
            ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("monun-web-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn pane(h: &Harness) -> Option<monun_ui_egui::web_view::Pane> {
    let uid = h.state().session.active()?.uid;
    h.state().web.panes.iter().find(|p| p.id == uid).cloned()
}

#[test]
fn an_html_file_shows_in_its_tab_where_the_page_goes() {
    let dir = temp_dir("page");
    let path = dir.join("설계 문서.html");
    std::fs::write(&path, "<html><head><title>설계</title></head><body><h1>설계</h1></body></html>").unwrap();
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    let mut h = harness(app);
    assert!(h.state().session.active().unwrap().is_web(), "a web tab, not its table as a sheet");
    // No web view here (a test, the web build): the tab says so and offers the browser.
    let p = pane(&h).expect("the page is listed for the window");
    assert_eq!(p.url, monun_ui_egui::web_view::file_url(path.to_str().unwrap()));
    assert!(p.url.starts_with("file:///") && p.url.ends_with("/%EC%84%A4%EA%B3%84%20%EB%AC%B8%EC%84%9C.html"), "{}", p.url);
    assert_eq!(p.rect, None, "nothing to show it in");
    assert!(h.query_by_label("Web pages show here in the desktop app.").is_some());
    assert!(h.query_by_label("Open in Browser").is_some());
    // With a web view (the desktop app): the page goes under the tool bar, in physical pixels.
    h.state_mut().web.host = Some(Ok(()));
    h.run_steps(2);
    let [x, y, w, hgt] = pane(&h).and_then(|p| p.rect).expect("shown");
    assert!(x < 10 && y > 60 && w > 1100 && hgt > 500, "{:?}", (x, y, w, hgt));
    assert_eq!(pane(&h).map(|p| p.holes), Some(vec![]), "nothing floats over it");
    // A menu open over it (the title bar's list of the files) is cut out of it (a web view would
    // cover the menu); the page stays.
    h.get_by_label("Open files").click();
    h.run_steps(2);
    let p = pane(&h).expect("listed");
    assert!(p.rect.is_some(), "still shown under the menu");
    let [_, top, _, height] = *p.holes.first().expect("the menu is cut out");
    assert!(height > 10 && top + height <= hgt as i32, "{:?}", p.holes);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(pane(&h).map(|p| p.holes), Some(vec![]), "closed, nothing is cut out");
    // Back, Forward and Show Again are asked of the page.
    for (label, order) in [("Back", Order::Back), ("Forward", Order::Forward), ("Show Again", Order::Reload)] {
        h.get_by_label(label).click();
        h.run_steps(1);
        assert_eq!(h.state().web.orders.last().map(|o| o.1), Some(order), "{label}");
    }
    // The page's title on the status bar, once the web view reports it.
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().web.pages.insert(uid, monun_ui_egui::web_view::Page { title: "설계 페이지".into(), ..Default::default() });
    h.run_steps(1);
    assert_eq!(monun_ui_egui::web_view::status(h.state()).as_deref(), Some("설계 페이지"));
    // Nothing writes it.
    assert!(h.state_mut().run("file.save", json!({})).is_err());
    // Closed, its page goes from the window.
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().web.panes.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_html_file_written_anew_is_shown_again() {
    let dir = temp_dir("watch");
    let path = dir.join("report.html");
    std::fs::write(&path, "<p>one</p>").unwrap();
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    let mut h = harness(app);
    h.state_mut().web.orders.clear();
    // A later time stamp, as an AI tool writes the file again.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&path, "<p>two</p>").unwrap();
    for _ in 0..120 {
        h.run_steps(1);
        if !h.state().web.orders.is_empty() {
            break;
        }
    }
    assert_eq!(h.state().web.orders.last().map(|o| o.1), Some(Order::Reload), "shown again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_site_link_is_made_from_its_address_and_opens() {
    let dir = temp_dir("link");
    let path = dir.join("사업 포털.url");
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    let target = path.to_str().unwrap().to_string();
    app.services.pick_save = Some(Box::new(move |suggested: &str| {
        assert_eq!(suggested, "사업 포털.url", "named as typed");
        Some(target.clone())
    }));
    let mut h = harness(app);
    // New Site Link… (on the start page, as in New file (+) and File › New): the address, a name, OK.
    h.get_by_label("New Site Link…").click();
    h.run_steps(3);
    // An address that isn't a web page's is refused.
    h.get_by_role_and_label(Role::TextInput, "Address:").type_text("javascript:alert(1)");
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.query_by_label("Type the site's address, such as https://example.com.").is_some());
    assert!(!path.exists());
    h.get_by_role_and_label(Role::TextInput, "Address:").focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Address:").type_text("portal.example.go.kr/main");
    h.get_by_role_and_label(Role::TextInput, "Name:").focus();
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Name:").type_text("사업 포털");
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let text = std::fs::read_to_string(&path).expect("the link file is written");
    assert_eq!(text, "[InternetShortcut]\r\nURL=https://portal.example.go.kr/main\r\n");
    let d = h.state().session.active().unwrap();
    assert!(d.is_web() && d.path.as_deref() == path.to_str(), "and opened in a tab");
    assert_eq!(pane(&h).map(|p| p.url).as_deref(), Some("https://portal.example.go.kr/main"));
    // Written anew with another address, the tab goes there.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&path, "[InternetShortcut]\r\nURL=https://example.com/other\r\n").unwrap();
    for _ in 0..120 {
        h.run_steps(1);
        if pane(&h).is_some_and(|p| p.url.ends_with("/other")) {
            break;
        }
    }
    assert_eq!(pane(&h).map(|p| p.url).as_deref(), Some("https://example.com/other"));
    // A link made by command, as the control channel does.
    let other = dir.join("b.url");
    let r = h.state_mut().run("web.newLink", json!({"url": "localhost:8080", "path": other.to_str().unwrap()})).unwrap();
    assert_eq!(r["url"], "http://localhost:8080");
    assert!(std::fs::read_to_string(&other).unwrap().contains("URL=http://localhost:8080"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_group_panel_floats_over_the_page() {
    let dir = temp_dir("panel");
    let path = dir.join("page.html");
    std::fs::write(&path, "<p>page</p>").unwrap();
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    // The group panel floating over the files in the top right corner, as a layer.
    app.ui.group_panel = true;
    app.ui.side_layer = true;
    app.ui.side_corner = monun_ui_egui::panes::Corner::TopRight;
    app.web.host = Some(Ok(()));
    let mut h = harness(app);
    h.run_steps(2);
    assert!(h.query_by_label("Groups").is_some(), "the group panel shows");
    // The page takes the whole width, the panel cut out of it in its corner.
    let p = pane(&h).expect("listed");
    let [x, _, w, _] = p.rect.expect("shown");
    assert!(x < 10 && w > 1150, "{:?}", (x, w));
    assert_eq!(p.holes.len(), 1, "{:?}", p.holes);
    let [left, top, width, _] = p.holes[0];
    assert!((870..=900).contains(&left) && top < 30 && (280..=320).contains(&width), "{:?}", p.holes);
    // In the top left corner: cut out there.
    h.state_mut().ui.side_corner = monun_ui_egui::panes::Corner::TopLeft;
    h.run_steps(3);
    let p = pane(&h).expect("listed");
    assert!(p.holes.len() == 1 && p.holes[0][0] < 30, "{:?}", p.holes);
    // As a column of its own: the page beside it, nothing cut out.
    h.state_mut().ui.side_layer = false;
    h.run_steps(3);
    let p = pane(&h).expect("listed");
    let [x, _, w, _] = p.rect.expect("shown");
    assert!(x + w <= 1200 - 250 && p.holes.is_empty(), "{:?}", (x, w, &p.holes));
    // Tucked into the edge: the page takes the width again but for the handle's strip by the right
    // edge (the handle would cover the page's own scroll bar), and nothing is cut out of it.
    h.state_mut().ui.group_tucked = true;
    h.run_steps(3);
    let p = pane(&h).expect("listed");
    let [x, _, w, _] = p.rect.expect("shown");
    let handle = h.get_by_role_and_label(egui::accesskit::Role::Button, "Groups").rect();
    assert!(x < 10 && w > 1150 && p.holes.is_empty(), "{:?}", (x, w, &p.holes));
    assert!((x + w) as f32 <= handle.left() && (x + w) as f32 >= handle.left() - 8.0, "beside the handle: {:?}", (x, w, handle));
    h.state_mut().ui.group_tucked = false;
    // Closed: the whole width, nothing cut out.
    h.state_mut().ui.group_panel = false;
    h.run_steps(3);
    let p = pane(&h).expect("listed");
    assert!(p.rect.is_some_and(|r| r[2] > 1150) && p.holes.is_empty(), "{:?}", p);
    let _ = std::fs::remove_dir_all(&dir);
}
