//! Source files in their tabs (.java, .vue, .yml…): edited as text, the status bar naming their
//! language, and made new by name (New › Other File).

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_engine::docs::TextKind;
use monun_ui_egui::SheetApp;
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

/// A folder of the test's own with `name` holding `text`; its path.
fn file(test: &str, name: &str, text: &str) -> String {
    let dir = std::env::temp_dir().join(format!("monun-source-{}-{test}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join(name), text).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

/// The app with file tabs, `path` open when given.
fn harness(path: Option<&str>) -> H {
    let mut app = crate::common::app(Session::new());
    app.use_file_tabs();
    if let Some(path) = path {
        app.run("file.open", json!({"path": path})).unwrap();
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

#[test]
fn a_source_file_is_edited_in_its_tab_named_as_its_language() {
    let path = file("edit", "UserController.java", "package demo;\n\npublic class UserController {\n    String name = \"x\";\n}\n");
    let mut h = harness(Some(&path));
    assert_eq!(h.state().session.active().and_then(|d| d.text_kind()), Some(TextKind::Code));
    assert!(h.query_by_label("Java").is_some(), "the status bar names the language");
    // Typed into and saved: the file on disk has it.
    h.get_by_role(Role::MultilineTextInput).click();
    h.run_steps(2);
    h.get_by_role(Role::MultilineTextInput).type_text("// checked\n");
    h.run_steps(3);
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(2);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("// checked") && text.contains("public class UserController"), "{text}");
}

#[test]
fn other_file_makes_a_file_of_the_kind_its_name_says() {
    let mut h = harness(None);
    h.state_mut().open_dialog("newFile", json!({}));
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "File name:").type_text("photo.png");
    h.run_steps(2);
    assert!(h.query_by_label("Monun can't make a new file of that type.").is_some());
    h.get_all_by_label("OK").last().unwrap().click();
    h.run_steps(2);
    assert!(h.state().session.documents().is_empty(), "nothing made of a picture's name");

    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(2);
    h.state_mut().open_dialog("newFile", json!({}));
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "File name:").type_text("App.vue");
    h.run_steps(2);
    assert!(h.query_by_label("Type: Vue").is_some());
    h.get_all_by_label("OK").last().unwrap().click();
    h.run_steps(3);
    let d = h.state().session.active().unwrap();
    assert_eq!((d.display_title(), d.text_kind(), d.path.clone()), ("App.vue".to_string(), Some(TextKind::Code), None));
    assert!(h.query_by_label("Vue").is_some(), "the status bar names it");
}
