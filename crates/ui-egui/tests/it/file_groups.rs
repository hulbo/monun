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

/// Whether the toggle `label` (a title bar panel button) shows as on.
fn toggled(h: &egui_kittest::Harness<'static, SheetApp>, label: &str) -> Option<bool> {
    use egui_kittest::kittest::NodeT as _;
    h.query_by_role_and_label(egui::accesskit::Role::CheckBox, label).map(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
}

#[test]
fn the_memo_and_group_panels_show_and_hide_from_the_title_bar() {
    let dir = folder("panels");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    // The Markdown file is in front: its memos, in a document's words.
    assert_eq!(toggled(&h, "Comments"), Some(false));
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Comments").click();
    h.run_steps(3);
    assert_eq!(toggled(&h, "Comments"), Some(true));
    assert!(h.query_by_label("No memos in this document yet.").is_some());
    assert!(h.query_by_label("No comments on this sheet yet. Select a cell and choose New Comment.").is_none(), "no words for cells");
    // The same button hides it.
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Comments").click();
    h.run_steps(3);
    assert_eq!(toggled(&h, "Comments"), Some(false));
    assert!(h.query_by_label("No memos in this document yet.").is_none());

    // The group panel takes its place, and goes the same way.
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Comments").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    assert_eq!((toggled(&h, "Group panel"), toggled(&h, "Comments")), (Some(true), Some(false)), "one panel at a time");
    assert!(h.query_by_label("New Group…").is_some(), "the groups");
    assert_eq!(h.query_all_by_label("b.md").count(), 2, "and their files, beside the file tab");
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    assert!(h.query_by_label("New Group…").is_none());

    // A SQL file has no memos, and no memo button.
    std::fs::write(dir.join("c.sql"), "SELECT 1\n").unwrap();
    h.state_mut().run("file.open", json!({"path": path(&dir, "c.sql")})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Comments").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_of_another_group_in_the_group_panel_opens_that_group_with_it_in_front() {
    let dir = folder("panel-open");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    groups::switch(h.state_mut(), Target::New { name: "SFR002".into(), take: false });
    h.run_steps(3);
    assert!(open_names(&h).is_empty());
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    // Default group is folded: its arrow shows its files.
    assert!(h.query_by_label("a.xlsx").is_none());
    h.get_by_label("Default group").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.group, "Default group", "a click on another group goes to it");
    assert_eq!(open_names(&h), ["a.xlsx", "b.md"]);
    // Back in SFR002, a file of Default group from the panel: Default group opens with it in front.
    groups::switch(h.state_mut(), Target::Group("SFR002".into()));
    h.run_steps(3);
    let arrow = h.get_by_label("Default group").rect().left_center() + egui::vec2(10.0, 0.0);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerMoved(arrow));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: arrow,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(3);
    assert_eq!(h.state().ui.group, "SFR002", "its arrow only shows its files");
    h.get_by_label("a.xlsx").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.group, "Default group");
    assert_eq!(h.state().session.active().map(|d| d.display_title()).as_deref(), Some("a.xlsx"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_memo_panel_lists_the_groups_other_files_and_goes_to_them() {
    let dir = folder("memo-list");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    // A comment on the workbook's B2, and a memo on the Markdown file's paragraph.
    let book = h.state().session.windows()[0].id;
    h.state_mut().enter_window(book);
    h.state_mut().run("review.newComment", json!({"cell": "B2", "text": "Check this total", "author": "me"})).unwrap();
    h.state_mut().run("selection.set", json!({"cell": "A1"})).unwrap();
    h.state_mut().ui.memos.push(monun_ui_egui::memos::DocMemo {
        id: 1,
        path: path(&dir, "b.md"),
        line: 0,
        quote: "# b".into(),
        text: "Rename this heading".into(),
        created: "2026-11-05 15:00".into(),
    });
    let md = h.state().session.windows()[1].id;
    h.state_mut().enter_window(md);
    h.state_mut().run("review.showComments", json!({"on": true})).unwrap();
    h.run_steps(4);
    // The Markdown file in front: its memo first, then the workbook's comment under its name.
    assert!(h.query_by_label("Rename this heading").is_some());
    assert!(h.query_by_label("Other files of the group").is_some());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Check this total").click();
    h.run_steps(4);
    let d = h.state().session.active().unwrap();
    assert_eq!(d.display_title(), "a.xlsx", "the comment's file comes to the front");
    assert_eq!(d.selection.active.a1(), "B2", "on its cell");
    // And back: from the workbook, the Markdown file's memo.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename this heading").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().display_title(), "b.md");
    assert_eq!(h.state().memos.picked, Some(1), "with its memo picked");
    let _ = std::fs::remove_dir_all(&dir);
}

fn drag(h: &mut egui_kittest::Harness<'static, SheetApp>, from: egui::Pos2, to: egui::Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.extend([egui::Event::PointerMoved(from), button(from, true)]);
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(button(to, false));
    h.run_steps(3);
}

fn heading(h: &egui_kittest::Harness<'static, SheetApp>, label: &str) -> egui::Rect {
    h.get_by_role_and_label(egui::accesskit::Role::Heading, label).rect()
}

#[test]
fn the_side_panels_title_meets_the_document_tool_bar_and_moves_by_it() {
    let dir = folder("side");
    let mut h = harness(&[path(&dir, "b.md")]);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    // Its title strip is the document tool bar's height, side by side with it.
    let title = heading(&h, "Groups");
    let tool = h.get_by_label("Find").rect();
    assert!((title.center().y - tool.center().y).abs() < 1.5 && (title.height() - 36.0).abs() < 0.5, "{title:?} beside {tool:?}");
    assert!(title.left() > 800.0, "on the right at first");
    // New Group is the + in its title strip.
    let plus = h.get_by_role_and_label(egui::accesskit::Role::Button, "New Group…").rect();
    assert!((plus.center().y - title.center().y).abs() < 1.0, "in the title strip");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "New Group…").click();
    h.run_steps(3);
    assert!(h.query_all_by_label("Group name:").count() > 0, "it asks for the new group's name");
    press(&mut h, "Cancel");
    h.run_steps(2);
    // Dragged by its title to the left, it is a column on the left.
    drag(&mut h, title.center(), egui::pos2(60.0, title.center().y));
    assert!(h.state().ui.side_left);
    assert!(heading(&h, "Groups").left() < 400.0);
    // As a layer it floats over the document, in the corner chosen.
    h.get_by_label("Panel options").click();
    h.run_steps(2);
    h.get_by_label("Float as a layer").click();
    h.run_steps(3);
    assert!(h.state().ui.side_layer);
    let layer = heading(&h, "Groups");
    assert!(layer.left() < 400.0, "top left, as its column was: {layer:?}");
    // Dragged to the bottom right, it settles there.
    drag(&mut h, layer.center(), egui::pos2(1150.0, 700.0));
    assert_eq!(h.state().ui.side_corner, monun_ui_egui::panes::Corner::BottomRight);
    let moved = heading(&h, "Groups");
    assert!(moved.right() > 1000.0 && moved.top() > 300.0, "{moved:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layer_resizes_by_its_free_edges() {
    let dir = folder("layer-size");
    let mut h = harness(&[path(&dir, "b.md")]);
    h.state_mut().ui.side_layer = true;
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    let before = heading(&h, "Groups");
    // In the top right corner its left edge and its bottom are free: dragged out, it grows.
    let edges: Vec<egui::Rect> = h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).collect();
    let side = edges.iter().copied().find(|r| r.height() > r.width() * 3.0).unwrap();
    drag(&mut h, side.center(), side.center() - egui::vec2(120.0, 0.0));
    let wider = heading(&h, "Groups");
    assert!(wider.width() > before.width() + 100.0, "{before:?} → {wider:?}");
    assert!((wider.right() - before.right()).abs() < 1.0, "its corner stays put");
    // The bottom edge: as much taller as it is dragged, from the height it shows at.
    let shown = h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).find(|r| r.height() > r.width() * 3.0).unwrap().height() + 16.0;
    let end = h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).find(|r| r.width() > r.height() * 3.0).unwrap();
    drag(&mut h, end.center(), end.center() + egui::vec2(0.0, 150.0));
    let tall = h.state().ui.side_layer_h.unwrap();
    assert!((tall - (shown + 150.0)).abs() < 12.0, "{shown} + 150 → {tall}");
    // A double-click on an edge brings it back to its own size.
    let side = h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).find(|r| r.height() > r.width() * 3.0).unwrap();
    for _ in 0..2 {
        let at = side.center();
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run_steps(3);
    assert_eq!((h.state().ui.side_layer_w, h.state().ui.side_layer_h), (None, None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layers_auto_height_follows_what_it_holds() {
    let dir = folder("layer-auto");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    h.state_mut().ui.side_layer = true;
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    let tall = |h: &egui_kittest::Harness<'static, SheetApp>| {
        h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).find(|r| r.height() > r.width() * 3.0).unwrap().height()
    };
    let open = tall(&h);
    // Its group folded, it is shorter: as tall as what it holds.
    let arrow = heading(&h, "Groups").left_bottom() + egui::vec2(10.0, 22.0);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerMoved(arrow));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: arrow,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(3);
    let folded = tall(&h);
    assert!(folded < open - 30.0, "{open} → {folded}");
    // Auto Height off: it keeps the height it shows at, whatever it holds.
    h.get_by_label("Panel options").click();
    h.run_steps(2);
    h.get_by_label("Auto Height").click();
    h.run_steps(3);
    let kept = h.state().ui.side_layer_h.expect("a height of its own");
    assert!((kept - (folded + 20.0)).abs() < 12.0, "kept {kept} from {folded}");
    // On again: back to fitting what it holds.
    if h.query_by_label("Auto Height").is_none() {
        h.get_by_label("Panel options").click();
        h.run_steps(2);
    }
    h.get_by_label("Auto Height").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.side_layer_h, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layer_sized_to_what_it_holds_grows_as_files_open() {
    let dir = folder("layer-grow");
    for name in ["c.md", "d.md", "e.md", "f.md"] {
        std::fs::write(dir.join(name), "# x\n").unwrap();
    }
    let mut h = harness(&[path(&dir, "b.md")]);
    h.state_mut().ui.side_layer = true;
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(4);
    let tall = |h: &egui_kittest::Harness<'static, SheetApp>| {
        h.get_all_by_role(egui::accesskit::Role::Splitter).map(|n| n.rect()).find(|r| r.height() > r.width() * 3.0).unwrap().height()
    };
    let one = tall(&h);
    for name in ["c.md", "d.md", "e.md", "f.md"] {
        h.state_mut().run("file.open", json!({"path": path(&dir, name)})).unwrap();
    }
    h.run_steps(6);
    let five = tall(&h);
    assert!(five > one + 4.0 * 20.0, "a row a file more: {one} → {five}");
    // Every file shows, the last one inside the layer.
    let last = h.get_all_by_label("f.md").map(|n| n.rect()).max_by(|a, b| a.left().total_cmp(&b.left())).unwrap();
    let title = heading(&h, "Groups");
    assert!(last.bottom() < title.top() + five + 4.0, "{last:?} inside a layer {five} tall from {title:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The groups `names` as the group panel lists them, top to bottom.
fn panel_order(h: &egui_kittest::Harness<'static, SheetApp>, names: &[&str]) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = names
        .iter()
        .filter_map(|n| h.query_by_role_and_label(egui::accesskit::Role::Button, n).map(|node| (node.rect().top(), n.to_string())))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, n)| n).collect()
}

/// Chooses `label` in the group panel's Sort menu: the `nth` of that name (Order of groups' By
/// name is the first, Order of files' the second).
fn sort_by(h: &mut egui_kittest::Harness<'static, SheetApp>, label: &str, nth: usize) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Sort Groups and Files").click();
    h.run_steps(2);
    let mut found: Vec<egui::Rect> = h.get_all_by_label(label).map(|n| n.rect()).collect();
    found.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = found[nth].center();
    h.get_all_by_label(label).find(|n| n.rect().center() == at).unwrap().click();
    h.run_steps(3);
}

#[test]
fn the_group_panel_sorts_groups_and_files_and_keeps_your_own_order() {
    let dir = folder("sort");
    let mut h = harness(&[path(&dir, "a.xlsx"), path(&dir, "b.md")]);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    // The files as their tabs are, then by type: the Markdown file first.
    let top = |h: &egui_kittest::Harness<'static, SheetApp>, name: &str| {
        h.get_all_by_label(name).map(|n| n.rect()).max_by(|a, b| a.left().total_cmp(&b.left())).unwrap().top()
    };
    assert!(top(&h, "a.xlsx") < top(&h, "b.md"));
    sort_by(&mut h, "By type", 0);
    assert_eq!(h.state().ui.file_order, groups::FileOrder::Kind);
    assert!(top(&h, "b.md") < top(&h, "a.xlsx"), "by type");

    // The groups as made, by name (a number by its value), and the one in use first.
    groups::switch(h.state_mut(), Target::New { name: "REQ-10".into(), take: false });
    groups::switch(h.state_mut(), Target::New { name: "REQ-9".into(), take: false });
    h.run_steps(3);
    let all = ["Default group", "REQ-10", "REQ-9"];
    assert_eq!(panel_order(&h, &all), all);
    sort_by(&mut h, "By name", 0);
    assert_eq!(panel_order(&h, &all), ["Default group", "REQ-9", "REQ-10"]);
    sort_by(&mut h, "Most recently used first", 0);
    assert_eq!(panel_order(&h, &all), ["REQ-9", "REQ-10", "Default group"]);
    // The title bar's group menu lists them the same way.
    assert_eq!(groups::ordered(h.state()).iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["REQ-9", "REQ-10", "Default group"]);

    // Your own order: a group dragged goes where it is let go.
    sort_by(&mut h, "Your own order", 0);
    assert_eq!(panel_order(&h, &all), all);
    let from = h.get_by_role_and_label(egui::accesskit::Role::Button, "REQ-9").rect().center();
    let to = h.get_by_role_and_label(egui::accesskit::Role::Button, "Default group").rect().center_top() + egui::vec2(0.0, 4.0);
    drag(&mut h, from, to);
    assert_eq!(panel_order(&h, &all), ["REQ-9", "Default group", "REQ-10"]);
    assert_eq!(h.state().ui.group, "REQ-9", "dragging it doesn't go to it");
    // Or moved by its menu.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "REQ-10").click_secondary();
    h.run_steps(2);
    h.get_by_label("Move Up").click();
    h.run_steps(3);
    assert_eq!(panel_order(&h, &all), ["REQ-9", "REQ-10", "Default group"]);
    assert_eq!(h.state().ui.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["REQ-9", "REQ-10", "Default group"], "and kept so");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_side_panels_buttons_point_and_only_its_title_grabs() {
    let dir = folder("cursor");
    let mut h = harness(&[path(&dir, "b.md")]);
    h.state_mut().ui.side_layer = true;
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Group panel").click();
    h.run_steps(3);
    let cursor = |h: &mut egui_kittest::Harness<'static, SheetApp>, at: egui::Pos2| {
        h.hover_at(at);
        h.run_steps(2);
        h.output().platform_output.cursor_icon
    };
    for button in ["New Group…", "Sort Groups and Files", "Panel options", "Close"] {
        let at = h.get_all_by_label(button).map(|n| n.rect()).max_by(|a, b| a.left().total_cmp(&b.left())).unwrap().center();
        assert_eq!(cursor(&mut h, at), egui::CursorIcon::PointingHand, "{button}");
    }
    let title = heading(&h, "Groups");
    assert_eq!(cursor(&mut h, title.left_center() + egui::vec2(20.0, 0.0)), egui::CursorIcon::Grab);
    // A press on a button and a drag never take the panel along.
    let plus = h.get_by_role_and_label(egui::accesskit::Role::Button, "New Group…").rect().center();
    drag(&mut h, plus, plus + egui::vec2(-400.0, 300.0));
    assert_eq!(h.state().ui.side_corner, monun_ui_egui::panes::Corner::TopRight);
    let _ = std::fs::remove_dir_all(&dir);
}
