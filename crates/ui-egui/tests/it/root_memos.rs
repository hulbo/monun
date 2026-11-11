//! Memos on the working root folder's Markdown files: kept in the root's `.monun\memos\memos.json`,
//! relative to it, so they go with the root (git) to other PCs as the projects do. Files elsewhere,
//! or no root, keep theirs in `ui.json` as before.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::memos::DocMemo;
use monun_ui_egui::memos::root_file::in_root;
use serde_json::{Value as Json, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

const PLAN: &str = "# Plan\n\nFirst paragraph.\n\nSecond paragraph.\n";

/// A root folder of its own for a test, `design-docs` with `a/x.md` and `a/y.md`, and a Markdown
/// file outside it.
fn root(test: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("monun-root-memos-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("design-docs");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a").join("x.md"), PLAN).unwrap();
    std::fs::write(root.join("a").join("y.md"), "# y\n\nWhy paragraph.\n").unwrap();
    std::fs::write(base.join("outside.md"), "# Outside\n\nOutside paragraph.\n").unwrap();
    (base, root)
}

fn path(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

fn harness() -> Harness {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
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

/// The interface's settings as last written (the desktop's `ui.json`): a run that keeps them.
fn keeps_settings(h: &mut Harness) -> Rc<RefCell<Option<monun_ui_egui::UiState>>> {
    let saved = Rc::new(RefCell::new(None));
    let into = saved.clone();
    h.state_mut().services.save_ui = Some(Box::new(move |ui| *into.borrow_mut() = Some(ui.clone())));
    saved
}

fn run(h: &mut Harness, id: &str, params: Json) -> Json {
    let r = h.state_mut().run(id, params).unwrap();
    h.run_steps(4);
    r
}

/// A memo on the paragraph `paragraph` of the Markdown file in front, as the user puts one: a
/// right click on it in View, Add Memo, the text, Save.
fn add_memo(h: &mut Harness, paragraph: &str, text: &str) {
    start_memo(h, paragraph, text);
    h.get_by_label("Save").click();
    h.run_steps(3);
}

/// A memo written, not saved yet.
fn start_memo(h: &mut Harness, paragraph: &str, text: &str) {
    h.get_by_label(paragraph).click_secondary();
    h.run_steps(3);
    h.get_by_label("Add Memo").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::MultilineTextInput, "Memo").type_text(text);
    h.run_steps(2);
}

/// Where the root keeps its memos.
fn memo_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".monun").join("memos").join("memos.json")
}

fn memo_file(root: &std::path::Path) -> String {
    std::fs::read_to_string(memo_path(root)).unwrap_or_default()
}

/// The memo file's memos as (file, text), and their ids.
fn memo_file_memos(root: &std::path::Path) -> (Vec<(String, String)>, Vec<u64>) {
    let file: Json = serde_json::from_str(&memo_file(root)).unwrap();
    let mut memos = Vec::new();
    let mut ids = Vec::new();
    for (file, list) in file["memos"].as_object().unwrap() {
        for m in list.as_array().unwrap() {
            memos.push((file.clone(), m["text"].as_str().unwrap().to_string()));
            ids.push(m["id"].as_u64().unwrap());
        }
    }
    memos.sort();
    (memos, ids)
}

/// Copies a folder and all it holds.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() { copy_dir(&src, &dst) } else { std::fs::copy(&src, &dst).map(|_| ()).unwrap() }
    }
}

fn memo(id: u64, path: &str, quote: &str, text: &str) -> DocMemo {
    DocMemo { id, path: path.into(), line: 2, quote: quote.into(), text: text.into(), created: "2026-11-01 09:00".into() }
}

#[test]
fn memos_on_the_roots_files_are_kept_in_it_and_show_on_another_pc() {
    let (base, root) = root("other-pc");
    let mut h = harness();
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    add_memo(&mut h, "Second paragraph.", "Check the totals");
    assert!(h.state().ui.memos.is_empty(), "not in ui.json");
    let text = memo_file(&root);
    assert!(text.contains("\"a/x.md\"") && text.contains("Check the totals"), "relative to the root, with /: {text}");
    let escaped = serde_json::to_string(&path(&root)).unwrap();
    assert!(!text.contains(escaped.trim_matches('"')), "no path of this PC in it: {text}");
    assert!(h.query_by_label("Check the totals").is_some(), "the panel lists it");
    assert!(h.query_by_role_and_label(Role::Button, "Memo").is_some(), "and the paragraph is marked");
    // It is no project, here or for a 모눈 from before (its folder is none's).
    let projects = run(&mut h, "project.list", json!({}));
    assert_eq!(projects["projects"], json!([]));
    let monun_files: Vec<_> = std::fs::read_dir(root.join(".monun")).unwrap().flatten().filter(|e| e.path().is_file()).collect();
    assert!(monun_files.is_empty(), "no file beside the projects");

    // A file outside the root keeps its memos in ui.json, as before.
    run(&mut h, "file.open", json!({"path": path(&base.join("outside.md"))}));
    add_memo(&mut h, "Outside paragraph.", "Not shared");
    assert_eq!(h.state().ui.memos.len(), 1);
    assert!(!memo_file(&root).contains("Not shared"));

    // Another PC: the root cloned elsewhere shows the memo on its copy of the file.
    let elsewhere = base.join("other-pc").join("docs");
    copy_dir(&root, &elsewhere);
    let mut other = harness();
    other.state_mut().ui.root = Some(path(&elsewhere));
    run(&mut other, "file.open", json!({"path": path(&elsewhere.join("a").join("x.md"))}));
    run(&mut other, "review.showComments", json!({"on": true}));
    assert!(other.query_by_label("Check the totals").is_some(), "the memo shows there");
    assert!(other.query_by_role_and_label(Role::Button, "Memo").is_some(), "on its paragraph");
    assert!(other.query_by_label("Not shared").is_none());
    // One added there goes into that root's file, beside the first.
    add_memo(&mut other, "First paragraph.", "Agreed");
    let (memos, ids) = memo_file_memos(&elsewhere);
    assert_eq!(memos, [("a/x.md".to_string(), "Agreed".to_string()), ("a/x.md".to_string(), "Check the totals".to_string())]);
    assert_ne!(ids[0], ids[1]);
    assert!(other.state().message.is_none(), "{:?}", other.state().message);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memos_kept_in_ui_json_move_into_the_root_once_without_losing_any() {
    let (base, root) = root("upgrade");
    let (x, y, outside) = (path(&root.join("a").join("x.md")), path(&root.join("a").join("y.md")), path(&base.join("outside.md")));
    // The root's file already has a memo another PC made, with an id a memo here has too.
    std::fs::create_dir_all(memo_path(&root).parent().unwrap()).unwrap();
    let theirs = "{\"monun\": 1, \"memos\": {\"a/y.md\": [{\"id\": 1, \"line\": 2, \"quote\": \"Why paragraph.\", \"text\": \"Theirs\", \"created\": \"2026-11-02 10:00\"}]}}\n";
    std::fs::write(memo_path(&root), theirs).unwrap();
    let mut h = harness();
    let saved = keeps_settings(&mut h);
    let before = vec![
        memo(1, &x, "Second paragraph.", "Old note"),
        memo(2, &y, "Why paragraph.", "Another"),
        memo(3, &outside, "Outside paragraph.", "Stays here"),
    ];
    h.state_mut().ui.memos = before.clone();
    // The upgraded app's first frames, with the root set.
    h.state_mut().ui.root = Some(path(&root));
    h.run_steps(3);
    assert_eq!(h.state().ui.memos, [before[2].clone()], "only the memo of a file outside the root stays in ui.json");
    let written = saved.borrow().clone().expect("ui.json written");
    assert_eq!(written.memos, [before[2].clone()]);
    let (memos, ids) = memo_file_memos(&root);
    assert_eq!(
        memos,
        [("a/x.md".to_string(), "Old note".to_string()), ("a/y.md".to_string(), "Another".to_string()), ("a/y.md".to_string(), "Theirs".to_string())]
    );
    let mut all: Vec<u64> = ids.clone();
    all.extend(h.state().ui.memos.iter().map(|m| m.id));
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 4, "every memo an id of its own: {ids:?}");
    let file: Json = serde_json::from_str(&memo_file(&root)).unwrap();
    let theirs = file["memos"]["a/y.md"].as_array().unwrap().iter().find(|m| m["text"] == "Theirs").unwrap();
    assert_eq!(theirs["id"], 1, "the root's keep theirs");
    let (told, _) = h.state().toast.clone().expect("the move is told");
    assert!(told.contains("Moved 2 memo(s)"), "{told}");

    // A move cut short (ui.json not written after the root's file): nothing is added twice.
    h.state_mut().ui.memos.push(before[0].clone());
    run(&mut h, "project.reload", json!({}));
    assert_eq!(memo_file_memos(&root).0.len(), 3);
    assert_eq!(h.state().ui.memos, [before[2].clone()]);

    // The moved memos show on their files.
    run(&mut h, "file.open", json!({"path": x}));
    run(&mut h, "review.showComments", json!({"on": true}));
    assert!(h.query_by_label("Old note").is_some());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_change_made_outside_is_taken_and_the_same_memos_are_left_as_they_are() {
    let (base, root) = root("pull");
    let mut h = harness();
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    add_memo(&mut h, "Second paragraph.", "Mine");
    // A git pull: a memo made on another PC, the file laid out otherwise.
    let mut file: Json = serde_json::from_str(&memo_file(&root)).unwrap();
    let theirs = json!({"id": 40, "line": 2, "quote": "First paragraph.", "text": "Theirs", "created": "2026-11-02 10:00"});
    file["memos"]["a/x.md"].as_array_mut().unwrap().push(theirs);
    let pulled = serde_json::to_string(&file).unwrap();
    std::fs::write(memo_path(&root), &pulled).unwrap();
    run(&mut h, "project.reload", json!({}));
    assert!(h.query_by_label("Theirs").is_some(), "read again");
    assert_eq!(in_root(h.state()).len(), 2);
    // Saved as it was: the same memos, so the file stays as git has it. (A memo card's Edit, in the
    // panel on the right, not the document's.)
    let edit = h.get_all_by_label("Edit").map(|n| n.rect().left()).fold(f32::MIN, f32::max);
    h.get_all_by_label("Edit").find(|n| n.rect().left() == edit).unwrap().click();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert!(h.state().memos.draft.is_none());
    assert_eq!(memo_file(&root), pulled, "not written again");

    // Another pull, not read yet: a memo added here keeps it.
    file["memos"]["a/y.md"] = json!([{"id": 41, "line": 2, "quote": "Why paragraph.", "text": "Later", "created": "2026-11-03 10:00"}]);
    std::fs::write(memo_path(&root), serde_json::to_string(&file).unwrap()).unwrap();
    add_memo(&mut h, "First paragraph.", "Here too");
    let (memos, ids) = memo_file_memos(&root);
    let texts: Vec<&str> = memos.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, ["Here too", "Mine", "Theirs", "Later"]);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4, "{ids:?}");
    assert!(ids.iter().all(|&id| id <= 41 || id == 42), "the new one after the pulled ones: {ids:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_memo_file_that_cant_be_read_is_never_overwritten() {
    let (base, root) = root("conflict");
    let mut h = harness();
    keeps_settings(&mut h);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    add_memo(&mut h, "Second paragraph.", "Before");
    let good = memo_file(&root);
    // A git conflict left in it just as a memo is saved (read a moment before, so it is the save
    // that finds it, not a look every few seconds).
    start_memo(&mut h, "First paragraph.", "During");
    h.state_mut().run("project.reload", json!({})).unwrap();
    let conflict = "<<<<<<< HEAD\n{\"monun\": 1}\n=======\n{\"monun\": 1}\n>>>>>>> theirs\n";
    std::fs::write(memo_path(&root), conflict).unwrap();
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(memo_file(&root), conflict, "left as it is");
    let (_, text) = h.state().message.clone().expect("said so");
    assert!(text.contains("can't be read") && text.contains("kept on this PC"), "{text}");
    assert_eq!(h.state().ui.memos.len(), 1, "the memo is kept on this PC meanwhile");
    // Said once: another memo doesn't say it again, and still doesn't write.
    h.state_mut().message = None;
    h.run_steps(2);
    add_memo(&mut h, "Second paragraph.", "Also during");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(memo_file(&root), conflict);
    assert_eq!(h.state().ui.memos.len(), 2);

    // The conflict resolved: the memos kept here move into it.
    std::fs::write(memo_path(&root), &good).unwrap();
    run(&mut h, "project.reload", json!({}));
    let texts: Vec<String> = memo_file_memos(&root).0.into_iter().map(|(_, t)| t).collect();
    assert_eq!(texts, ["Also during", "Before", "During"]);
    assert!(h.state().ui.memos.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_run_that_keeps_nothing_never_writes_memos_to_the_users_root() {
    let (base, root) = root("detach");
    let x = path(&root.join("a").join("x.md"));
    std::fs::create_dir_all(memo_path(&root).parent().unwrap()).unwrap();
    let shared = "{\"monun\": 1, \"memos\": {\"a/x.md\": [{\"id\": 1, \"line\": 2, \"quote\": \"First paragraph.\", \"text\": \"Shared\"}]}}\n";
    std::fs::write(memo_path(&root), shared).unwrap();
    // The user's settings, read but never written (`--control`): the root, and a memo of a file
    // under it from before.
    let mut h = harness();
    h.state_mut().ui.root = Some(path(&root));
    h.state_mut().ui.memos = vec![memo(5, &x, "Second paragraph.", "Old local")];
    monun_ui_egui::projects::detach(h.state_mut());
    h.run_steps(2);
    assert_eq!(h.state().ui.root, None);
    run(&mut h, "file.open", json!({"path": x.clone()}));
    add_memo(&mut h, "First paragraph.", "Agent note");
    assert_eq!(h.state().ui.memos.len(), 2, "kept in this run alone");
    assert_eq!(memo_file(&root), shared, "the user's root is untouched");

    // An agent's own root: the memos of its files the user's ui.json has aren't moved by a run
    // that can't take them out of it.
    run(&mut h, "project.root", json!({"path": path(&root)}));
    assert_eq!(memo_file(&root), shared);
    assert!(h.state().ui.memos.iter().any(|m| m.text == "Old local"));
    assert_eq!(in_root(h.state()).len(), 1, "the root's own are read");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_git_pull_shows_in_a_few_seconds_without_a_reload() {
    let (base, root) = root("watch");
    let mut h = harness();
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    run(&mut h, "review.showComments", json!({"on": true}));
    assert!(h.query_by_label("Pulled").is_none());
    std::fs::create_dir_all(memo_path(&root).parent().unwrap()).unwrap();
    let pulled = r#"{"monun": 1, "memos": {"a/x.md": [{"id": 3, "line": 2, "quote": "First paragraph.", "text": "Pulled"}]}}"#;
    std::fs::write(memo_path(&root), pulled).unwrap();
    // The file is looked at every two seconds.
    std::thread::sleep(std::time::Duration::from_millis(2200));
    h.run_steps(3);
    assert!(h.query_by_label("Pulled").is_some(), "read again");
    assert!(h.query_by_role_and_label(Role::Button, "Memo").is_some(), "and its paragraph marked");
    assert_eq!(memo_file(&root), pulled, "the file left as git has it");
    let _ = std::fs::remove_dir_all(&base);
}
