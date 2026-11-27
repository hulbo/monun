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
    serve("application/json", answer)
}

/// A server on this PC answering one request with `answer` of `content_type`.
fn serve(content_type: &'static str, answer: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
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
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
    });
    (port, rx)
}

/// Puts the file `path` of the group in use in a new folder `API` holding `vars` and `headers`.
fn in_folder(app: &mut SheetApp, path: &str, vars: &[(&str, &str, bool)], headers: &[(&str, &str)]) -> u64 {
    let group = groups::current(app);
    let g = app.ui.groups.iter_mut().find(|g| g.name == group).unwrap();
    let id = g.add_folder("API", None);
    let f = g.folders.iter_mut().find(|f| f.id == id).unwrap();
    f.vars = vars.iter().map(|(n, v, s)| Setting { name: n.to_string(), value: v.to_string(), secret: *s, ..Setting::default() }).collect();
    f.headers = headers.iter().map(|(n, v)| Setting { name: n.to_string(), value: v.to_string(), ..Setting::default() }).collect();
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
    assert!(h.query_by_label("{\n    \"ok\": true,\n    \"n\": 1\n}").is_some());
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
    assert_eq!(f.vars, [Setting { name: "Domain".into(), value: "dev.example.com".into(), ..Setting::default() }]);
    assert_eq!(f.headers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["X-Key"]);
    assert!(h.query_by_label("→ https://dev.example.com/").is_some(), "the request reads it");
    let _ = std::fs::remove_file(&path);
}

/// Clicks Send and waits for the answer `status` (a line like `200 OK`).
fn send_and_wait(h: &mut H, status: &str) {
    h.get_by_label("Send").click();
    for _ in 0..200 {
        h.run_steps(1);
        if h.query_by_label(status).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no answer {status}");
}

#[test]
fn a_body_is_chosen_the_ways_postman_offers_and_goes_so() {
    let (port, got) = serve("application/xml", "<?xml version=\"1.0\"?><Envelope><Body><name>Kim</name><n>1</n></Body></Envelope>");
    let path = file("form", "form.http", &format!("POST http://127.0.0.1:{port}/form\n"));
    let mut h = harness(&path, |_| {});
    h.get_by_label("Body").click();
    h.run_steps(2);
    assert!(h.query_by_label("This request has no body.").is_some(), "none, to start with");
    // x-www-form-urlencoded: its type set, a field added and typed.
    h.get_by_label("x-www-form-urlencoded").click();
    h.run_steps(2);
    assert!(draft(&h).contains("Content-Type: application/x-www-form-urlencoded\n"), "{}", draft(&h));
    h.get_by_label("Add Field").click();
    h.run_steps(2);
    type_into(&mut h, "Name", 0, "name");
    type_into(&mut h, "Value", 0, "Kim Lee&1");
    assert!(
        draft(&h).ends_with("Content-Type: application/x-www-form-urlencoded\n\nname=Kim Lee%261\n"),
        "an & that would split it is written %26: {}",
        draft(&h)
    );
    send_and_wait(&mut h, "200 OK");
    let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(request.to_ascii_lowercase().contains("\r\ncontent-type: application/x-www-form-urlencoded\r\n"), "{request}");
    assert!(request.ends_with("\r\n\r\nname=Kim+Lee%261"), "{request}");
    // The XML answer laid out, a tag a line.
    assert!(
        h.query_by_label("<?xml version=\"1.0\"?>\n<Envelope>\n    <Body>\n        <name>Kim</name>\n        <n>1</n>\n    </Body>\n</Envelope>")
            .is_some()
    );
    // As It Came: as the server sent it.
    h.get_by_label("As It Came").click();
    h.run_steps(2);
    assert!(h.query_by_label("<?xml version=\"1.0\"?><Envelope><Body><name>Kim</name><n>1</n></Body></Envelope>").is_some());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_form_sends_its_file_and_a_switch_back_finds_the_body_left() {
    let (port, got) = serve("text/plain", "ok");
    let dir = std::path::Path::new(&file("multipart", "a.txt", "file text")).parent().unwrap().to_path_buf();
    let text = format!(
        "POST http://127.0.0.1:{port}/up\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"title\"\n\nMonun\n--B\nContent-Disposition: form-data; name=\"doc\"; filename=\"a.txt\"\n\n< ./a.txt\n--B--\n"
    );
    std::fs::write(dir.join("up.http"), &text).unwrap();
    let path = dir.join("up.http").to_string_lossy().into_owned();
    let mut h = harness(&path, |_| {});
    h.get_by_label("Body").click();
    h.run_steps(2);
    assert!(h.query_by_label("9 B").is_some(), "the file is there, its size shown");
    send_and_wait(&mut h, "200 OK");
    let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(request.to_ascii_lowercase().contains("\r\ncontent-type: multipart/form-data; boundary=b\r\n"), "{request}");
    assert!(
        request.ends_with("\r\n\r\n--B\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nMonun\r\n--B\r\nContent-Disposition: form-data; name=\"doc\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nfile text\r\n--B--\r\n"),
        "{request}"
    );
    // raw, then back to form-data: the form as it was.
    h.get_by_label("raw").click();
    h.run_steps(2);
    assert!(!draft(&h).contains("--B"), "{}", draft(&h));
    h.get_by_label("form-data").click();
    h.run_steps(2);
    // (Written by the tab, the request starts with its `###` line.)
    assert_eq!(draft(&h).trim_start_matches("###\n"), text);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_environment_chosen_at_the_front_of_the_line_reads_its_values() {
    let (port, got) = serve_once(r#"{"env":"dev"}"#);
    let path = file(
        "env",
        "orders.http",
        "GET {{url}}/orders
",
    );
    let mut h = harness(&path, |app| {
        let id = in_folder(app, &path, &[("url", "http://127.0.0.1:1", false)], &[]);
        let group = groups::current(app);
        let f = app.ui.groups.iter_mut().find(|g| g.name == group).unwrap().folders.iter_mut().find(|f| f.id == id).unwrap();
        f.envs = vec!["Dev".into(), "Prod".into()];
        f.vars[0].by_env = [("Dev".to_string(), format!("http://127.0.0.1:{port}"))].into();
    });
    // No environment yet: the value for all.
    assert!(h.query_by_label("Environment").is_some(), "the environment at the front of the line");
    assert!(h.query_by_label("→ http://127.0.0.1:1/orders").is_some());
    // Dev chosen at the front of the line: {{url}} reads its value there, and the group keeps it.
    h.get_by_label("Environment").click();
    h.run_steps(2);
    h.get_by_label("Dev").click();
    h.run_steps(3);
    assert!(h.query_by_label(&format!("→ http://127.0.0.1:{port}/orders")).is_some());
    let group = groups::current(h.state_mut());
    assert_eq!(h.state().ui.groups.iter().find(|g| g.name == group).unwrap().env.as_deref(), Some("Dev"));
    send_and_wait(&mut h, "200 OK");
    let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(request.starts_with("GET /orders "), "{request}");
    // Prod has no value of its own: the value for all.
    h.get_by_label("Environment").click();
    h.run_steps(2);
    h.get_by_label("Prod").click();
    h.run_steps(3);
    assert!(h.query_by_label("→ http://127.0.0.1:1/orders").is_some());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn folder_settings_name_environments_and_keep_each_variables_values() {
    let path = file(
        "env-settings",
        "a.http",
        "GET {{url}}/
",
    );
    let mut id = 0;
    let mut h = harness(&path, |app| id = in_folder(app, &path, &[("url", "https://example.test", false)], &[]));
    let group = groups::current(h.state_mut());
    h.state_mut().open_dialog("folderSettings", json!({"group": group, "id": id}));
    h.run_steps(3);
    // Add Environment suggests Dev, then Local.
    h.get_by_label("Add Environment").click();
    h.run_steps(2);
    h.get_by_label("Add Environment").click();
    h.run_steps(2);
    type_into(&mut h, "Value · Dev", 0, "https://dev.example.test");
    // Renamed, the environment keeps its values.
    type_into(&mut h, "Environment name", 0, "2");
    h.get_by_label("OK").click();
    h.run_steps(3);
    let g = h.state().ui.groups.iter().find(|g| g.name == group).unwrap();
    let f = g.folder(id).unwrap();
    assert_eq!(f.envs, ["Dev2", "Local"]);
    assert_eq!(f.vars[0].by_env.get("Dev2").map(String::as_str), Some("https://dev.example.test"));
    assert!(!f.vars[0].by_env.contains_key("Local"), "an empty value isn't kept");
    // Kept with the folder (in a project's file too): its environments and values by name.
    let json = serde_json::to_string(f).unwrap();
    assert!(json.contains("\"envs\":[\"Dev2\",\"Local\"]") && json.contains("\"byEnv\":{\"Dev2\":"), "{json}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn variables_of_one_name_are_kept_and_the_one_in_use_is_read() {
    let path = file(
        "var-use",
        "use.http",
        "GET {{URL}}/x
",
    );
    let mut id = 0;
    let mut h = harness(&path, |app| id = in_folder(app, &path, &[("URL", "http://a.test", false)], &[]));
    let group = groups::current(h.state_mut());
    let settings = |h: &mut H| {
        h.state_mut().open_dialog("folderSettings", json!({"group": group, "id": id}));
        h.run_steps(3);
    };
    let used =
        |h: &H| h.state().ui.groups.iter().find(|g| g.name == group).unwrap().folder(id).unwrap().vars.iter().map(|v| !v.off).collect::<Vec<_>>();
    // A second URL added: the first stays in use, the new one is out of use.
    settings(&mut h);
    h.get_by_label("Add Variable").click();
    h.run_steps(2);
    let uses = |h: &H| {
        use egui_kittest::kittest::NodeT as _;
        let mut boxes: Vec<(f32, bool)> = h
            .get_all_by_role_and_label(egui::accesskit::Role::CheckBox, "Use")
            .map(|n| (n.rect().top(), n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)))
            .collect();
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
        boxes.into_iter().map(|b| b.1).collect::<Vec<_>>()
    };
    assert_eq!(uses(&h), [true, true], "a new variable is in use");
    type_into(&mut h, "Name", 1, "URL");
    assert_eq!(uses(&h), [true, true], "not while its name is typed");
    type_into(&mut h, "Value", 1, "http://b.test");
    assert_eq!(uses(&h), [true, false], "once typed, the same name is out of use");
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(used(&h), [true, false]);
    assert!(h.query_by_label("→ http://a.test/x").is_some(), "the request reads the one in use");
    // The second's Use ticked: it takes the name from the first.
    settings(&mut h);
    let mut boxes: Vec<egui::Rect> = h.get_all_by_role_and_label(egui::accesskit::Role::CheckBox, "Use").map(|n| n.rect()).collect();
    boxes.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let second = boxes[1].center();
    h.get_all_by_role_and_label(egui::accesskit::Role::CheckBox, "Use").find(|n| n.rect().center() == second).unwrap().click();
    h.run_steps(2);
    assert_eq!(uses(&h), [false, true]);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(used(&h), [false, true]);
    assert!(h.query_by_label("→ http://b.test/x").is_some());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_request_with_a_body_opens_on_it() {
    let path = file("body-first", "post.http", "POST https://x.test/orders\nContent-Type: application/json\n\n{\"id\": 1}\n");
    let mut h = harness(&path, |_| {});
    assert!(h.query_by_label("x-www-form-urlencoded").is_some(), "the Body shows");
    assert!(h.query_by_label("Add Parameter").is_none());
    // The answer follows right under it, without an empty band.
    let splitter = |h: &H| h.get_by_role_and_label(egui::accesskit::Role::Splitter, "Resize the request and its answer").rect();
    let body = h.get_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Body").rect();
    let line = splitter(&h);
    assert!(line.top() >= body.bottom() - 1.0 && line.top() - body.bottom() < 16.0, "{body:?} then {line:?}");
    // Dragged down, the request's part is taller; a double-click fits it again.
    let from = line.center();
    let to = from + egui::vec2(0.0, 120.0);
    for (pos, pressed) in [(from, true), (from + egui::vec2(0.0, 60.0), true), (to, true), (to, false)] {
        h.input_mut().events.push(egui::Event::PointerMoved(pos));
        if pressed && pos == from || !pressed {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run_steps(2);
    let dragged = splitter(&h);
    assert!((dragged.top() - (line.top() + 120.0)).abs() < 6.0, "as far as it was dragged: {line:?} → {dragged:?}");
    for _ in 0..2 {
        for pressed in [true, false] {
            let pos = dragged.center();
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run_steps(2);
    assert!((splitter(&h).top() - line.top()).abs() < 1.0, "fitted again");
    let _ = std::fs::remove_file(&path);
    // One without a body opens on its Parameters.
    let path = file("body-first", "get.http", "GET https://x.test/orders\n");
    let h = harness(&path, |_| {});
    assert!(h.query_by_label("Add Parameter").is_some());
    let _ = std::fs::remove_file(&path);
}
