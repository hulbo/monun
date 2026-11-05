//! File groups, with file tabs: the group button in the title bar switches the set of open files,
//! closing this group's files and opening the other's.

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::groups::{self, Target};
use serde_json::json;

/// A folder of its own for a test's files, with a workbook `a.xlsx` and a Markdown file `b.md`.
fn folder(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("monun-groups-{}-{test}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "a"})).unwrap();
    s.run("file.saveAs", json!({"path": dir.join("a.xlsx").to_string_lossy()})).unwrap();
    std::fs::write(dir.join("b.md"), "# b\n").unwrap();
    dir
}

fn path(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn harness(open: &[String]) -> egui_kittest::Harness<'static, SheetApp> {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    for p in open {
        app.run("file.open", json!({"path": p})).unwrap();
    }
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

/// Clicks the dialog's button `label` (the formula bar has a Cancel of its own, higher up).
fn press(h: &mut egui_kittest::Harness<'static, SheetApp>, label: &str) {
    let lowest = h.get_all_by_label(label).max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect(label).rect().center();
    h.get_all_by_label(label).find(|n| n.rect().center() == lowest).expect(label).click();
}

/// The open files' names, in tab order.
fn open_names(h: &egui_kittest::Harness<'static, SheetApp>) -> Vec<String> {
    let s = &h.state().session;
    s.windows().iter().filter_map(|w| s.documents().iter().find(|d| d.uid == w.doc_uid)).map(|d| d.display_title()).collect()
}

#[test]
fn a_new_group_starts_empty_and_switching_back_opens_its_files_again() {
    let dir = folder("switch");
    let mut h = harness(&[path(&dir, "a.xlsx")]);
    assert_eq!(groups::current(h.state_mut()), "Default group");
    assert_eq!(h.state().ui.groups[0].files, [path(&dir, "a.xlsx")], "the group in use follows its tabs");

    // Group button › New Group…, named SFR002.
    h.get_by_label("Group: Default group").click();
    h.run_steps(3);
    h.get_by_label("New Group…").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Group name:").type_text("SFR002");
    h.run_steps(2);
    press(&mut h, "OK");
    h.run_steps(4);
    assert_eq!(h.state().ui.group, "SFR002");
    assert!(open_names(&h).is_empty(), "the new group starts with no file open");
    assert!(h.query_by_label("New Excel Workbook").is_some(), "on the start page");
    assert!(h.query_by_label("Group: SFR002").is_some(), "which still has the group button");

    h.state_mut().run("file.open", json!({"path": path(&dir, "b.md")})).unwrap();
    h.run_steps(3);
    groups::switch(h.state_mut(), Target::Group("Default group".into()));
    h.run_steps(4);
    assert_eq!(open_names(&h), ["a.xlsx"], "the Default group's files are back, SFR002's closed");
    groups::switch(h.state_mut(), Target::Group("SFR002".into()));
    h.run_steps(4);
    assert_eq!(open_names(&h), ["b.md"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unsaved_files_are_asked_about_once_before_switching() {
    let dir = folder("unsaved");
    let mut h = harness(&[path(&dir, "a.xlsx")]);
    groups::switch(h.state_mut(), Target::New { name: "SFR002".into(), take: false });
    h.run_steps(3);
    h.state_mut().run("file.new", json!({})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    h.run_steps(3);

    groups::switch(h.state_mut(), Target::Group("Default group".into()));
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("switchGroup"), "one question for the unsaved files");
    assert!(h.query_by_label("•  Book1").is_some(), "naming them");
    press(&mut h, "Cancel");
    h.run_steps(3);
    assert_eq!(h.state().ui.group, "SFR002", "Cancel stays");
    assert_eq!(open_names(&h), ["Book1"]);

    groups::switch(h.state_mut(), Target::Group("Default group".into()));
    h.run_steps(3);
    press(&mut h, "Don't Save");
    h.run_steps(4);
    assert_eq!(h.state().ui.group, "Default group");
    assert_eq!(open_names(&h), ["a.xlsx"], "Don't Save drops the changes and goes");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_new_group_can_take_the_open_files_along() {
    let dir = folder("take");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    groups::switch(h.state_mut(), Target::New { name: "SFR001".into(), take: true });
    h.run_steps(3);
    assert_eq!(open_names(&h), ["a.xlsx", "b.md"], "nothing closes");
    let ui = &h.state().ui;
    assert_eq!(ui.group, "SFR001");
    let files = |name: &str| ui.groups.iter().find(|g| g.name == name).map(|g| g.files.len());
    assert_eq!((files("SFR001"), files("Default group")), (Some(2), Some(0)), "the files moved to the new group");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn groups_can_be_renamed_and_deleted() {
    let dir = folder("rename");
    let mut h = harness(&[path(&dir, "a.xlsx")]);
    h.get_by_label("Group: Default group").click();
    h.run_steps(3);
    h.get_by_label("Rename Group…").click();
    h.run_steps(3);
    // The name field starts with the group's name, selected.
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Group name:").focus();
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(1);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Group name:").type_text("SFR001");
    h.run_steps(2);
    press(&mut h, "OK");
    h.run_steps(3);
    assert_eq!(h.state().ui.group, "SFR001");
    assert_eq!(h.state().ui.groups.len(), 1, "renamed, not added");

    groups::switch(h.state_mut(), Target::New { name: "SFR002".into(), take: false });
    h.run_steps(3);
    // A name in use is refused.
    h.state_mut().open_dialog("newGroup", json!({}));
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Group name:").type_text("sfr001");
    h.run_steps(2);
    press(&mut h, "OK");
    h.run_steps(3);
    assert!(h.query_by_label("There is already a group with this name.").is_some());
    press(&mut h, "Cancel");
    h.run_steps(2);

    h.state_mut().open_dialog("deleteGroup", json!({"name": "SFR001"}));
    h.run_steps(2);
    press(&mut h, "Delete");
    h.run_steps(3);
    let names: Vec<String> = h.state().ui.groups.iter().map(|g| g.name.clone()).collect();
    assert_eq!(names, ["SFR002"]);
    assert!(dir.join("a.xlsx").exists(), "the group's files stay where they are");
    let _ = std::fs::remove_dir_all(dir);
}
