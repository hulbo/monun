//! API request tabs: a `.http` file shown as a request to send, reading the variables and headers
//! of the group folders it sits in, and its answer shown below.

use std::io::{Read, Write};
use std::time::Duration;

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::group_folders::Setting;
use monun_ui_egui::groups;
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

/// A folder of the test's own with `name` holding `text`; its path.
fn file(test: &str, name: &str, text: &str) -> String {
    let dir = std::env::temp_dir().join(format!("monun-api-{}-{test}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join(name), text).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

/// The app with file tabs and `path` open; `setup` runs on it first (folders, say).
fn harness(path: &str, setup: impl FnOnce(&mut SheetApp)) -> H {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    app.run("file.open", json!({"path": path})).unwrap();
    setup(&mut app);
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

/// A server on this PC answering one request with `answer` (JSON); the request comes back whole.
fn serve_once(answer: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut s, _)) = listener.accept() else { return };
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut got = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = s.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            got.extend_from_slice(&chunk[..n]);
            if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                if got.len() >= end + 4 + len {
                    break;
                }
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&got).into_owned());
        let _ =
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
    });
    (port, rx)
}

/// Puts the file `path` of the group in use in a new folder `API` holding `vars` and `headers`.
fn in_folder(app: &mut SheetApp, path: &str, vars: &[(&str, &str, bool)], headers: &[(&str, &str)]) -> u64 {
    let group = groups::current(app);
    let g = app.ui.groups.iter_mut().find(|g| g.name == group).unwrap();
    let id = g.add_folder("API", None);
    let f = g.folders.iter_mut().find(|f| f.id == id).unwrap();
    f.vars = vars.iter().map(|(n, v, s)| Setting { name: n.to_string(), value: v.to_string(), secret: *s }).collect();
    f.headers = headers.iter().map(|(n, v)| Setting { name: n.to_string(), value: v.to_string(), secret: false }).collect();
    g.place(path, Some(id));
    id
}

/// Types `text` into the `nth` text field labelled `label` (from the top), focused first.
fn type_into(h: &mut H, label: &str, nth: usize, text: &str) {
    let nth_field = |h: &H| {
        let mut fields: Vec<egui::Rect> = h.get_all_by_role_and_label(egui::accesskit::Role::TextInput, label).map(|n| n.rect()).collect();
        fields.sort_by(|a, b| a.top().total_cmp(&b.top()));
        fields[nth].center()
    };
    let at = nth_field(h);
    h.get_all_by_role_and_label(egui::accesskit::Role::TextInput, label).find(|n| n.rect().center() == at).unwrap().focus();
    h.run_steps(1);
    let at = nth_field(h);
    h.get_all_by_role_and_label(egui::accesskit::Role::TextInput, label).find(|n| n.rect().center() == at).unwrap().type_text(text);
    h.run_steps(2);
}

fn draft(h: &H) -> String {
    h.state().doc_view.draft.as_ref().map(|d| d.text.clone()).unwrap_or_default()
}

#[test]
fn a_request_goes_with_its_folders_values_and_its_answer_shows() {
    let (port, got) = serve_once(r#"{"ok":true,"n":1}"#);
    let text = "@CustNo = C1\n\n### Approve\nPOST http://{{Host}}/pay/{{CustNo}}\nContent-Type: application/json\n\n{\"amount\": 1000}\n";
    let path = file("send", "pay.http", text);
    let host = format!("127.0.0.1:{port}");
    let mut h = harness(&path, |app| {
        in_folder(app, &path, &[("Host", &host, false), ("Token", "abc", true)], &[("Authorization", "Bearer {{Token}}")]);
    });
    // The address as it goes, and where its values come from.
    assert!(h.query_by_label(&format!("→ http://{host}/pay/C1")).is_some());
    assert!(h.query_by_label("API · 2 variables · 1 headers").is_some());
    h.get_by_label("Send").click();
    for _ in 0..200 {
        h.run_steps(1);
        if h.query_by_label("200 OK").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.query_by_label("200 OK").is_some(), "answered");
    let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(request.starts_with("POST /pay/C1 HTTP/1.1\r\n"), "{request}");
    let lower = request.to_ascii_lowercase();
    assert!(lower.contains("\r\nauthorization: bearer abc\r\n") && lower.contains("\r\ncontent-type: application/json\r\n"), "{request}");
    assert!(request.ends_with("\r\n\r\n{\"amount\": 1000}"), "{request}");
    // The JSON laid out, in its own order.
    assert!(h.query_by_label("{\n  \"ok\": true,\n  \"n\": 1\n}").is_some());
    assert_eq!(draft(&h), text, "sending writes nothing");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_variable_without_a_value_keeps_the_request_from_going() {
    let path = file("missing", "a.http", "GET https://{{Domain}}/v1/{{Nope}}\n");
    let mut h = harness(&path, |app| {
        in_folder(app, &path, &[("Domain", "x.test", false)], &[]);
    });
    assert!(h.query_by_label("These variables have no value: Nope").is_some());
    h.get_by_label("Send").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("These variables have no value: Nope").count(), 2, "said again where the answer shows");
    assert!(h.query_by_label("Cancel").is_none(), "nothing on its way");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn what_the_tab_edits_goes_to_the_file_and_the_rest_stays() {
    let text = "@Domain = x.test\n\n### First\nGET https://{{Domain}}/a\n\n### Second\nDELETE https://{{Domain}}/b\n";
    let path = file("edit", "two.http", text);
    let mut h = harness(&path, |_| {});
    // A parameter added and typed: on the address of the request shown, the rest as it was.
    h.get_by_label("Add Parameter").click();
    h.run_steps(2);
    type_into(&mut h, "Name", 0, "q");
    type_into(&mut h, "Value", 0, "1 2");
    let now = draft(&h);
    assert!(now.contains("### First\nGET https://{{Domain}}/a?q=1 2\n"), "{now}");
    assert!(now.starts_with("@Domain = x.test\n\n") && now.ends_with("### Second\nDELETE https://{{Domain}}/b\n"), "{now}");
    // The file's text to edit by hand, and back.
    h.get_by_label("File Text").click();
    h.run_steps(3);
    assert!(h.query_by_label("Request View").is_some());
    h.get_by_label("Request View").click();
    h.run_steps(3);
    assert!(h.query_by_label("Send").is_some());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn folder_settings_keep_a_folders_variables_and_headers() {
    let path = file("settings", "s.http", "GET https://{{Domain}}/\n");
    let mut id = 0;
    let mut h = harness(&path, |app| {
        id = in_folder(app, &path, &[], &[]);
    });
    let group = h.state().ui.group.clone();
    h.state_mut().open_dialog("folderSettings", json!({"group": group, "id": id}));
    h.run_steps(3);
    h.get_by_label("Add Variable").click();
    h.run_steps(2);
    type_into(&mut h, "Name", 0, "Domain");
    type_into(&mut h, "Value", 0, "dev.example.com");
    h.get_by_label("Add Header").click();
    h.run_steps(2);
    type_into(&mut h, "Name", 1, "X-Key");
    h.get_by_label("OK").click();
    h.run_steps(3);
    let g = h.state().ui.groups.iter().find(|g| g.name == group).unwrap();
    let f = g.folder(id).unwrap();
    assert_eq!(f.vars, [Setting { name: "Domain".into(), value: "dev.example.com".into(), secret: false }]);
    assert_eq!(f.headers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["X-Key"]);
    assert!(h.query_by_label("→ https://dev.example.com/").is_some(), "the request reads it");
    let _ = std::fs::remove_file(&path);
}
