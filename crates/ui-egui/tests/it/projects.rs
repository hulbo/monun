//! Projects above file groups: kept in the working root folder's `.monun` folder, their files
//! relative to the root, so the root opens the same anywhere (another PC, after a git clone).

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use monun_ui_egui::groups;
use serde_json::{Value as Json, json};

/// A root folder of its own for a test: `design-docs` with `a/x.md`, `a/y.md` and a workbook `a/b.xlsx`.
fn root(test: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("monun-projects-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("design-docs");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a").join("x.md"), "# x\n").unwrap();
    std::fs::write(root.join("a").join("y.md"), "# y\n").unwrap();
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "b"})).unwrap();
    s.run("file.saveAs", json!({"path": root.join("a").join("b.xlsx").to_string_lossy()})).unwrap();
    (base, root)
}

fn path(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
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

fn run(h: &mut egui_kittest::Harness<'static, SheetApp>, id: &str, params: Json) -> Json {
    let r = h.state_mut().run(id, params).unwrap();
    h.run_steps(4);
    r
}

fn open_paths(h: &egui_kittest::Harness<'static, SheetApp>) -> Vec<String> {
    h.state().session.documents().iter().filter_map(|d| d.path.clone()).collect()
}

fn project_file(root: &std::path::Path, name: &str) -> String {
    std::fs::read_to_string(root.join(".monun").join(format!("{name}.json"))).unwrap_or_default()
}

/// Copies a folder and all it holds.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() { copy_dir(&src, &dst) } else { std::fs::copy(&src, &dst).map(|_| ()).unwrap() }
    }
}

#[test]
fn a_project_keeps_its_groups_in_the_root_relative_to_it_and_opens_on_another_pc() {
    let (base, root) = root("relative");
    let x = path(&root.join("a").join("x.md"));
    let mut h = harness(std::slice::from_ref(&x));
    assert_eq!(h.state().ui.project, None, "This PC to begin with");
    run(&mut h, "project.root", json!({"path": path(&root)}));
    assert_eq!(run(&mut h, "project.new", json!({"name": "a-project"}))["project"], "a-project");
    assert_eq!(h.state().ui.project.as_deref(), Some("a-project"));
    assert!(open_paths(&h).is_empty(), "This PC's files closed");
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("y.md"))}));
    let text = project_file(&root, "a-project");
    assert!(text.contains("\"a/y.md\""), "relative to the root, with /: {text}");
    let escaped = serde_json::to_string(&path(&root)).unwrap();
    assert!(!text.contains(escaped.trim_matches('"')), "no path of this PC in it: {text}");

    // This PC's groups are as they were.
    run(&mut h, "project.switch", json!({"name": null}));
    assert_eq!(h.state().ui.project, None);
    assert_eq!(open_paths(&h), [x.clone()]);

    // Another PC: the root cloned elsewhere opens the project's files from there.
    let elsewhere = base.join("other-pc").join("docs");
    copy_dir(&root, &elsewhere);
    let mut other = harness(&[]);
    other.state_mut().ui.root = Some(path(&elsewhere));
    other.state_mut().ui.project = Some("a-project".into());
    groups::restore(other.state_mut());
    other.run_steps(4);
    assert_eq!(open_paths(&other), [path(&elsewhere.join("a").join("y.md"))]);
    assert!(other.state().message.is_none(), "{:?}", other.state().message);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_project_button_opens_and_makes_projects() {
    let (base, root) = root("button");
    let mut h = harness(&[path(&root.join("a").join("x.md"))]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    // Left of the group button.
    let project = h.get_by_role_and_label(egui::accesskit::Role::Button, "Project: This PC").rect();
    let group = h.get_by_role_and_label(egui::accesskit::Role::Button, "Group: Default group").rect();
    assert!(project.right() <= group.left() + 1.0, "{project:?} left of {group:?}");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Project: This PC").click();
    h.run_steps(2);
    h.get_by_label("New Project…").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Project name:").type_text("ALPHA");
    h.run_steps(2);
    let ok = h.get_all_by_label("OK").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().rect().center();
    h.get_all_by_label("OK").find(|n| n.rect().center() == ok).unwrap().click();
    h.run_steps(4);
    assert_eq!(h.state().ui.project.as_deref(), Some("ALPHA"));
    assert!(root.join(".monun").join("ALPHA.json").is_file());
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Project: ALPHA").is_some());
    // And back to This PC from its menu.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Project: ALPHA").click();
    h.run_steps(2);
    h.get_by_label("This PC").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.project, None);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_group_is_sent_to_another_project_as_a_copy() {
    let (base, root) = root("send");
    let x = path(&root.join("a").join("x.md"));
    let mut h = harness(std::slice::from_ref(&x));
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "P"}));
    run(&mut h, "project.switch", json!({"name": null}));
    // This PC's Default group, with x.md, into P: relative there, still here.
    // P has a Default group of its own (its first, empty): the copies take free names.
    let r = run(&mut h, "group.sendToProject", json!({"group": "Default group", "project": "P"}));
    assert_eq!(r["group"], "Default group (2)", "a name free there");
    let r = run(&mut h, "group.sendToProject", json!({"group": "Default group", "project": "P"}));
    assert_eq!(r["group"], "Default group (3)");
    let text = project_file(&root, "P");
    assert!(text.contains("\"a/x.md\"") && text.contains("Default group (3)"), "{text}");
    assert_eq!(h.state().ui.groups.len(), 1, "the group stays here");
    // From P back to This PC, as a copy too.
    run(&mut h, "project.switch", json!({"name": "P"}));
    assert_eq!(open_paths(&h), Vec::<String>::new(), "P's group in use was its empty first one");
    let r = run(&mut h, "group.sendToProject", json!({"group": "Default group (2)", "project": null}));
    assert_eq!(r["group"], "Default group (2)");
    let this_pc = h.state().ui.this_pc.clone().unwrap();
    let copy = this_pc.groups.iter().find(|g| g.name == "Default group (2)").unwrap();
    assert_eq!(copy.files, [x], "the absolute path, on this PC");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn going_to_a_project_asks_about_unsaved_files_first() {
    let (base, root) = root("unsaved");
    let mut h = harness(&[path(&root.join("a").join("b.xlsx"))]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    std::fs::create_dir_all(root.join(".monun")).unwrap();
    std::fs::write(root.join(".monun").join("P.json"), "{\"monun\": 1, \"groups\": [{\"name\": \"G\", \"files\": [\"a/y.md\"]}]}\n").unwrap();
    run(&mut h, "cell.set", json!({"cell": "A2", "input": "changed"}));
    run(&mut h, "project.switch", json!({"name": "P"}));
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("switchGroup"));
    assert!(h.query_by_label("Save changes to these files before going to the project “P”?").is_some());
    let cancel = h.get_all_by_label("Cancel").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().rect().center();
    h.get_all_by_label("Cancel").find(|n| n.rect().center() == cancel).unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.project, None, "Cancel stays");
    let dont = h.state_mut();
    dont.run("project.switch", json!({"name": "P"})).unwrap();
    h.run_steps(3);
    let button = h.get_all_by_label("Don't Save").next().unwrap().rect().center();
    h.get_all_by_label("Don't Save").find(|n| n.rect().center() == button).unwrap().click();
    h.run_steps(4);
    assert_eq!(h.state().ui.project.as_deref(), Some("P"));
    assert_eq!(h.state().ui.group, "G");
    assert_eq!(open_paths(&h), [path(&root.join("a").join("y.md"))], "its group's files, from the root");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_change_made_outside_is_taken_but_for_the_group_in_use() {
    let (base, root) = root("merge");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "P"}));
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    // A git pull: another group, made on another PC.
    let pulled =
        "{\"monun\": 1, \"groups\": [{\"name\": \"Default group\", \"files\": [\"a/x.md\"]}, {\"name\": \"Theirs\", \"files\": [\"a/b.xlsx\"]}]}\n";
    std::fs::write(root.join(".monun").join("P.json"), pulled).unwrap();
    // A tab opened here writes the project: theirs kept, the group in use as its tabs are.
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("y.md"))}));
    let text = project_file(&root, "P");
    assert!(text.contains("Theirs") && text.contains("\"a/b.xlsx\""), "{text}");
    assert!(text.contains("\"a/y.md\""), "{text}");
    assert!(h.state().ui.groups.iter().any(|g| g.name == "Theirs"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_project_that_cant_be_read_at_start_gives_way_to_this_pc() {
    let (base, root) = root("missing");
    let x = path(&root.join("a").join("x.md"));
    let mut h = harness(&[]);
    let state = &mut h.state_mut().ui;
    state.root = Some(path(&base.join("no-such-folder")));
    state.project = Some("Gone".into());
    state.this_pc = Some(monun_ui_egui::projects::GroupSet {
        groups: vec![groups::FileGroup { name: "Mine".into(), files: vec![x.clone()], ..Default::default() }],
        group: "Mine".into(),
    });
    // Checked without drawing: the message quotes Windows' own words, in the system's language,
    // which the test fonts can't draw.
    groups::restore(h.state_mut());
    assert_eq!(h.state().ui.project, None);
    assert_eq!(h.state().ui.group, "Mine");
    assert_eq!(open_paths(&h), [x]);
    let (_, text) = h.state().message.clone().expect("a message says why");
    assert!(text.contains("could not be opened"), "{text}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn groups_saved_before_projects_are_this_pcs() {
    let ui: monun_ui_egui::UiState =
        serde_json::from_str(r#"{"groups": [{"name": "REQ-037", "files": ["C:\\work\\a.md"]}], "group": "REQ-037"}"#).unwrap();
    assert_eq!((ui.project.as_deref(), ui.root.as_deref(), ui.group.as_str()), (None, None, "REQ-037"));
    assert_eq!(ui.groups[0].files, ["C:\\work\\a.md"]);
}

#[test]
fn a_run_that_keeps_nothing_starts_on_this_pc_without_the_users_root() {
    let (base, root) = root("detach");
    std::fs::create_dir_all(root.join(".monun")).unwrap();
    std::fs::write(root.join(".monun").join("P.json"), "{\"monun\": 1, \"groups\": []}\n").unwrap();
    let mut h = harness(&[]);
    let ui = &mut h.state_mut().ui;
    ui.root = Some(path(&root));
    ui.project = Some("P".into());
    ui.this_pc = Some(monun_ui_egui::projects::GroupSet {
        groups: vec![groups::FileGroup { name: "Mine".into(), ..Default::default() }],
        group: "Mine".into(),
    });
    monun_ui_egui::projects::detach(h.state_mut());
    assert_eq!((h.state().ui.project.clone(), h.state().ui.root.clone(), h.state().ui.group.clone()), (None, None, "Mine".to_string()));
    // Nothing it does reaches the user's root.
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    assert!(h.state_mut().run("project.switch", json!({"name": "P"})).is_err());
    assert_eq!(project_file(&root, "P"), "{\"monun\": 1, \"groups\": []}\n");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_project_file_that_cant_be_read_is_never_overwritten() {
    let (base, root) = root("conflict");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "P"}));
    // A git conflict left in it.
    let conflict = "<<<<<<< HEAD\n{\"monun\": 1}\n=======\n{\"monun\": 1}\n>>>>>>> theirs\n";
    std::fs::write(root.join(".monun").join("P.json"), conflict).unwrap();
    h.state_mut().run("file.open", json!({"path": path(&root.join("a").join("x.md"))})).unwrap();
    h.run_steps(3);
    assert_eq!(project_file(&root, "P"), conflict, "left as it is");
    let (_, text) = h.state().message.clone().expect("said so");
    assert!(text.contains("can't be read"), "{text}");
    // Said once: another change doesn't ask again, and still doesn't write.
    h.state_mut().message = None;
    h.state_mut().run("file.open", json!({"path": path(&root.join("a").join("y.md"))})).unwrap();
    h.run_steps(3);
    assert!(h.state().message.is_none());
    assert_eq!(project_file(&root, "P"), conflict);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_projects_file_this_pc_cant_open_is_taken_out_of_its_group() {
    let (base, root) = root("unopened");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    let file = "{\"monun\": 1, \"groups\": [{\"name\": \"G\", \"files\": [\"a/x.md\", \"a/not-there.md\", \"a/y.md\"]}]}\n";
    std::fs::create_dir_all(root.join(".monun")).unwrap();
    std::fs::write(root.join(".monun").join("P.json"), file).unwrap();
    h.state_mut().run("project.switch", json!({"name": "P"})).unwrap();
    h.run_steps(3);
    assert_eq!(open_paths(&h).len(), 2);
    let (_, text) = h.state().message.clone().expect("said so");
    assert!(text.contains("not-there.md") && text.contains("taken out of it"), "{text}");
    // The group follows its tabs: the project's list no longer has it, nor the panel.
    let text = project_file(&root, "P");
    assert!(!text.contains("not-there.md") && text.contains("\"a/x.md\"") && text.contains("\"a/y.md\""), "{text}");
    assert!(h.state().ui.groups.iter().all(|g| g.files.iter().all(|f| !f.contains("not-there"))));
    assert!(monun_ui_egui::groups::open_files_listed(h.state()).iter().all(|f| !f.title.contains("not-there")));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn projects_are_renamed_and_deleted_by_name_only() {
    let (base, root) = root("rename");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "Q"}));
    run(&mut h, "project.new", json!({"name": "P"}));
    assert_eq!(run(&mut h, "project.rename", json!({"name": "Q", "to": "R"}))["project"], "R");
    assert!(root.join(".monun").join("R.json").is_file() && !root.join(".monun").join("Q.json").exists());
    // The one in use, renamed, stays in use.
    run(&mut h, "project.rename", json!({"name": "P", "to": "P2"}));
    assert_eq!(h.state().ui.project.as_deref(), Some("P2"));
    assert!(h.state_mut().run("project.delete", json!({"name": "P2"})).is_err(), "not the one in use");
    // A name that isn't a project of the root never reaches a file.
    std::fs::write(base.join("outside.json"), "{}").unwrap();
    assert!(h.state_mut().run("project.delete", json!({"name": "..\\..\\outside"})).is_err());
    assert!(h.state_mut().run("group.sendToProject", json!({"group": "Default group", "project": "../../outside"})).is_err());
    assert_eq!(std::fs::read_to_string(base.join("outside.json")).unwrap(), "{}");
    run(&mut h, "project.delete", json!({"name": "R"}));
    assert!(!root.join(".monun").join("R.json").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn coming_back_to_a_project_goes_to_the_group_used_last_in_it() {
    let (base, root) = root("views");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    let file = "{\"monun\": 1, \"groups\": [{\"name\": \"G1\", \"files\": [\"a/x.md\"]}, {\"name\": \"G2\", \"files\": [\"a/y.md\"]}]}\n";
    std::fs::create_dir_all(root.join(".monun")).unwrap();
    std::fs::write(root.join(".monun").join("P.json"), file).unwrap();
    run(&mut h, "project.switch", json!({"name": "P"}));
    assert_eq!(h.state().ui.group, "G1", "the first, the first time");
    groups::switch(h.state_mut(), groups::Target::Group("G2".into()));
    h.run_steps(3);
    run(&mut h, "project.switch", json!({"name": null}));
    run(&mut h, "project.switch", json!({"name": "P"}));
    assert_eq!(h.state().ui.group, "G2");
    assert_eq!(open_paths(&h), [path(&root.join("a").join("y.md"))]);
    assert_eq!(project_file(&root, "P"), file, "which group is in use stays on this PC");
    // Another root: back to This PC first.
    let other = base.join("other-root");
    std::fs::create_dir_all(&other).unwrap();
    run(&mut h, "project.root", json!({"path": path(&other)}));
    assert_eq!(h.state().ui.project, None);
    assert_eq!(h.state().ui.root.as_deref(), Some(path(&other).as_str()));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_project_file_that_cant_be_read_just_now_is_left_as_it_is() {
    let (base, root) = root("unreadable");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "P"}));
    // Not text (a checkout gone wrong): it can't even be read.
    let broken: &[u8] = b"{\"monun\": 1, \"groups\": [{\"name\": \"G\xff\", \"files\": []}]}\n";
    let file = root.join(".monun").join("P.json");
    std::fs::write(&file, broken).unwrap();
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("x.md"))}));
    assert_eq!(std::fs::read(&file).unwrap(), broken, "left as it is");
    let (_, text) = h.state().message.clone().expect("said so");
    assert!(text.contains("can't be read just now"), "{text}");
    // Said once; each change tries again, and the one after it can be read writes.
    h.state_mut().message = None;
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("y.md"))}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(std::fs::read(&file).unwrap(), broken);
    std::fs::write(&file, "{\"monun\": 1, \"groups\": [{\"name\": \"Default group\", \"files\": []}]}\n").unwrap();
    run(&mut h, "file.open", json!({"path": path(&root.join("a").join("b.xlsx"))}));
    let text = project_file(&root, "P");
    assert!(text.contains("\"a/b.xlsx\"") && text.contains("\"a/y.md\""), "{text}");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_file_put_in_a_folder_stays_there_after_a_restart() {
    let (base, root) = root("folder-restart");
    let mut h = harness(&[]);
    run(&mut h, "project.root", json!({"path": path(&root)}));
    run(&mut h, "project.new", json!({"name": "p"}));
    let y = path(&root.join("a").join("y.md"));
    run(&mut h, "file.open", json!({"path": y}));
    let group = h.state().ui.group.clone();
    let before = project_file(&root, "p");
    // A folder made for the file: the file moves into it, and no file opens or closes.
    h.state_mut().open_dialog("newFolder", json!({"group": group, "path": y}));
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Folder name:").type_text("API");
    h.run_steps(2);
    h.get_all_by_label("OK").last().unwrap().click();
    h.run_steps(3);
    let text = project_file(&root, "p");
    assert_ne!(text, before, "written though its files are the same");
    assert!(text.contains("\"a/y.md\": 1"), "{text}");
    // Folded here: this PC's, so the project file stays as it is.
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "API").click();
    h.run_steps(3);
    assert_eq!(project_file(&root, "p"), text, "folding changes nothing git sees");

    // Started again with this PC's settings: the file in its folder, the folder folded.
    let ui = h.state().ui.clone();
    let mut again = harness(&[]);
    again.state_mut().ui = ui;
    groups::restore(again.state_mut());
    again.run_steps(4);
    assert_eq!(open_paths(&again), [y.clone()]);
    let g = again.state().ui.groups.iter().find(|g| g.name == group).unwrap();
    let folder = g.folders.first().unwrap();
    assert_eq!((g.folder_of(&y), folder.folded), (Some(folder.id), true));
    let _ = std::fs::remove_dir_all(&base);
}
