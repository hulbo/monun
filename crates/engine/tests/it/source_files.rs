//! Source files (.java, .vue, .yml, .xml…): opened as text to read and edit, saved back as they
//! came, coloured as their extension says, and made new by name (New › Other File).

use monun_engine::docs::TextKind;
use monun_engine::io::{self, FileKind};
use monun_engine::{Session, docs};
use serde_json::json;

fn doc_text(wb: &monun_engine::model::Workbook) -> String {
    wb.sheets.first().and_then(|sh| sh.document.as_ref()).map(|d| d.text.clone()).unwrap_or_default()
}

#[test]
fn source_files_open_as_text_by_extension_or_name() {
    for name in ["Main.java", "App.vue", "application.yml", "UserMapper.xml", "messages.properties", "list.jsp", "Dockerfile", ".gitignore"] {
        assert_eq!(FileKind::from_path(name), Some(FileKind::Text), "{name}");
        assert_eq!(TextKind::of_name(name), TextKind::Code, "{name}");
    }
    assert_eq!(TextKind::of_name("readme.txt"), TextKind::Plain);
    assert_eq!(TextKind::of_name("query.sql"), TextKind::Sql);
    assert_eq!(FileKind::from_path("firmware.bin"), None, "an unknown file still isn't opened");
}

#[test]
fn a_source_file_saves_back_in_its_encoding_and_line_breaks() {
    // An old project's source in EUC-KR (CP949) with CRLF: "// 한글" then a class.
    let mut bytes = b"// ".to_vec();
    bytes.extend([0xC7, 0xD1, 0xB1, 0xDB]);
    bytes.extend(b"\r\npublic class Main {}\r\n");
    let (wb, _) = io::open_bytes("Main.java", &bytes).unwrap();
    assert!(docs::is_text_file(&wb));
    assert_eq!(doc_text(&wb), "// 한글\npublic class Main {}\n");
    assert_eq!(io::save_bytes(&wb, "Main.java").unwrap(), bytes);
}

#[test]
fn json_of_another_program_is_source_and_a_monun_workbook_stays_one() {
    let package = b"{\r\n  \"name\": \"front\",\r\n  \"version\": \"1.0.0\"\r\n}\r\n";
    let (wb, _) = io::open_bytes("package.json", package).unwrap();
    assert!(docs::is_text_file(&wb));
    assert!(doc_text(&wb).contains("\"front\""));
    assert_eq!(io::save_bytes(&wb, "package.json").unwrap(), package, "saved as its text, not as a workbook");
    // Not even JSON: still opens, to be fixed.
    let (wb, _) = io::open_bytes("broken.json", b"{ \"a\": ").unwrap();
    assert_eq!(doc_text(&wb), "{ \"a\": ");

    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let saved = io::save_bytes(&s.active().unwrap().wb, "book.json").unwrap();
    let (wb, _) = io::open_bytes("book.json", &saved).unwrap();
    assert!(!docs::is_text_file(&wb), "a Monun workbook saved as JSON opens as one");
}

#[test]
fn a_new_file_is_made_by_its_name() {
    let mut s = Session::new();
    let r = s.execute("file.new", json!({"document": true, "name": "Main.java", "text": "class Main {}\n"})).unwrap();
    assert_eq!(r["title"], "Main.java");
    let d = s.active().unwrap();
    assert_eq!((d.text_kind(), d.path.clone()), (Some(TextKind::Code), None));
    assert_eq!(doc_text(&d.wb), "class Main {}\n");
    // The same name again gets a number; a name of its own kind is that kind.
    assert_eq!(s.execute("file.new", json!({"document": true, "name": "Main.java"})).unwrap()["title"], "Main (2).java");
    assert_eq!(s.execute("file.new", json!({"document": true, "name": "notes.md"})).unwrap()["title"], "notes.md");
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Markdown));
    // Not a file's name, or a kind Monun doesn't write.
    for name in ["", "a/b.java", "x:y.js", "photo.png", "slides.pptx"] {
        assert!(s.execute("file.new", json!({"document": true, "name": name})).is_err(), "{name:?}");
    }
}

#[test]
fn a_source_file_opened_edited_and_saved_on_disk() {
    let dir = std::env::temp_dir().join(format!("monun-source-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("application.yml");
    std::fs::write(&path, "server:\n  port: 8080\n").unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Code));
    let sheet = s.active().unwrap().wb.sheets[0].name.clone();
    s.execute("doc.setText", json!({"sheet": sheet, "text": "server:\n  port: 9090\n"})).unwrap();
    s.execute("file.save", json!({})).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "server:\n  port: 9090\n");
    let _ = std::fs::remove_dir_all(&dir);
}
