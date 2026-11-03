//! Document tabs: Markdown text in a tab beside the worksheets, kept in XLSX as a worksheet of
//! lines (column A) that Excel shows, and turned back into a document on open.

use monun_engine::model::Workbook;
use monun_engine::{Session, io};
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn text(s: &mut Session, sheet: &str) -> String {
    s.execute("doc.get", json!({"sheet": sheet})).unwrap()["text"].as_str().unwrap_or_default().to_owned()
}

fn reopen(wb: &Workbook) -> Workbook {
    io::open_bytes("book.xlsx", &io::save_bytes(wb, "book.xlsx").unwrap()).unwrap().0
}

/// Lines that a naive cell write would change: blank lines, leading spaces, a formula-looking
/// line, numbers, a Korean heading and a trailing blank line.
const AWKWARD: &str = "# 매수 메모\n\n  들여쓴 줄\n=SUM(A1:A3)\n00123\n2026-11-03\n- [ ] 가계부에 옮기기\n";

#[test]
fn a_document_tab_sits_after_the_active_sheet_and_edits_undo() {
    let mut s = session();
    let r = s.execute("doc.insert", json!({"prefix": "문서"})).unwrap();
    assert_eq!((r["index"].as_u64(), r["name"].as_str()), (Some(1), Some("문서1")));
    let d = s.active().unwrap();
    assert!(d.wb.sheet(1).unwrap().is_document());
    assert_eq!(d.wb.active_sheet(), 1);
    // A second unnamed one takes the next number.
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let r = s.execute("doc.insert", json!({"prefix": "문서", "text": "a\r\nb"})).unwrap();
    assert_eq!(r["name"], "문서2");
    assert_eq!(text(&mut s, "문서2"), "a\nb", "line breaks are kept as \\n");

    s.execute("doc.setText", json!({"sheet": "문서1", "text": "첫 줄"})).unwrap();
    assert_eq!(text(&mut s, "문서1"), "첫 줄");
    assert!(s.active().unwrap().is_dirty());
    let same = s.execute("doc.setText", json!({"sheet": "문서1", "text": "첫 줄"})).unwrap();
    assert_eq!(same["changed"], false, "the same text is no new undo step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(text(&mut s, "문서1"), "");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(text(&mut s, "문서1"), "첫 줄");
}

#[test]
fn sheet_commands_work_on_document_tabs() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "메모", "text": "내용"})).unwrap();
    s.execute("sheet.rename", json!({"sheet": "메모", "name": "매수 메모"})).unwrap();
    s.execute("sheet.move", json!({"sheet": "매수 메모", "to": 0, "copy": true})).unwrap();
    let names: Vec<String> = s.active().unwrap().wb.sheets.iter().map(|sh| sh.name.clone()).collect();
    assert_eq!(names, ["매수 메모 (2)", "Sheet1", "매수 메모"]);
    assert_eq!(text(&mut s, "매수 메모 (2)"), "내용", "a copy keeps the text");
    s.execute("home.deleteSheet", json!({"sheet": "매수 메모"})).unwrap();
    assert_eq!(s.active().unwrap().wb.sheets.len(), 2);
    let inspect = s.execute("document.inspect", json!({})).unwrap();
    let kinds: Vec<&Value> = inspect["sheets"].as_array().unwrap().iter().map(|sh| &sh["kind"]).collect();
    assert_eq!(kinds, [&json!("document"), &json!("worksheet")]);
}

#[test]
fn a_document_has_no_cells_to_edit() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "메모"})).unwrap();
    let e = s.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap_err();
    assert!(e.to_string().contains("A document has no cells"), "{e}");
    assert!(s.execute("doc.setText", json!({"sheet": "Sheet1", "text": "x"})).is_err(), "a worksheet is not a document");
}

#[test]
fn documents_round_trip_through_xlsx_as_lines_in_column_a() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "10"})).unwrap();
    s.execute("doc.insert", json!({"name": "메모", "text": AWKWARD})).unwrap();
    let long = format!("{}끝", "가".repeat(40_000));
    s.execute("doc.insert", json!({"name": "긴 줄", "text": long})).unwrap();
    let wb = s.active().unwrap().wb.clone();

    // In the file, the document is a worksheet of text cells plus the hidden mark.
    let bytes = io::save_bytes(&wb, "book.xlsx").unwrap();
    let (raw, _) = monun_xlsx::read_xlsx(&bytes).unwrap();
    let sheet = raw.sheet(1).unwrap();
    assert!(!sheet.is_document());
    let cell = |r: u32| sheet.value(monun_engine::core::CellRef::new(r, 0));
    assert_eq!(cell(0), monun_engine::core::Value::Text("# 매수 메모".into()));
    assert_eq!(cell(1), monun_engine::core::Value::Empty);
    assert_eq!(cell(3), monun_engine::core::Value::Text("=SUM(A1:A3)".into()), "a formula-looking line stays text");
    assert_eq!(cell(4), monun_engine::core::Value::Text("00123".into()));
    let mark = raw.names.iter().find(|n| n.name == "_gc_doc" && n.scope == Some(1)).unwrap();
    assert!(mark.hidden);
    assert!(raw.sheet(2).unwrap().cell(monun_engine::core::CellRef::new(0, 1)).is_some(), "a line longer than a cell continues in column B");

    // Opened again, the documents are back as they were, and the marks are gone.
    let back = reopen(&wb);
    assert_eq!(back.sheet(1).unwrap().document.as_ref().unwrap().text, AWKWARD);
    assert_eq!(back.sheet(2).unwrap().document.as_ref().unwrap().text, long);
    assert!(back.sheet(1).unwrap().cells.is_empty());
    assert!(!back.names.iter().any(|n| n.name == "_gc_doc"));
    assert!(!back.sheet(0).unwrap().is_document());
    // Saving again writes the same file content.
    assert_eq!(reopen(&back).sheet(1).unwrap().document.as_ref().unwrap().text, AWKWARD);
}

#[test]
fn a_document_edited_in_another_app_reads_its_column_a() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "메모", "text": "하나\n둘"})).unwrap();
    let wb = s.active().unwrap().wb.clone();
    // Excel added a line and typed a number into it.
    let bytes = io::save_bytes(&wb, "book.xlsx").unwrap();
    let (mut raw, _) = monun_xlsx::read_xlsx(&bytes).unwrap();
    if let Some(sh) = raw.sheet_mut(1) {
        sh.set_value(monun_engine::core::CellRef::new(2, 0), monun_engine::core::Value::Number(3.0));
    }
    let edited = monun_xlsx::write_xlsx(&raw).unwrap();
    let back = io::open_bytes("book.xlsx", &edited).unwrap().0;
    assert_eq!(back.sheet(1).unwrap().document.as_ref().unwrap().text, "하나\n둘\n3");
}

#[test]
fn documents_round_trip_through_json() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "메모", "text": AWKWARD})).unwrap();
    let wb = s.active().unwrap().wb.clone();
    let bytes = io::save_bytes(&wb, "book.json").unwrap();
    let back = io::open_bytes("book.json", &bytes).unwrap().0;
    assert_eq!(back.sheet(1).unwrap().document.as_ref().unwrap().text, AWKWARD);
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("monun-doc-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_markdown_file_opens_as_a_document_and_saves_back() {
    let bytes = "\u{feff}# 메모\r\n\r\n- [ ] 할 일\r\n".as_bytes();
    let (wb, _) = io::open_bytes("매수 메모.md", bytes).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    let sheet = wb.sheet(0).unwrap();
    assert_eq!(sheet.name, "매수 메모");
    assert_eq!(sheet.document.as_ref().unwrap().text, "# 메모\n\n- [ ] 할 일\n", "the mark goes and line breaks are \n");
    assert_eq!(io::save_bytes(&wb, "매수 메모.md").unwrap(), "# 메모\n\n- [ ] 할 일\n".as_bytes());
    // A worksheet isn't Markdown.
    let mut s = session();
    let e = io::save_bytes(&s.active().unwrap().wb, "x.md").unwrap_err();
    assert!(e.to_string().contains("Only a document tab"), "{e}");
    // In a session: Save of a .md opened alone writes it back and leaves it clean.
    let dir = temp_dir("save");
    let path = dir.join("메모.md");
    std::fs::write(&path, "# 처음\n").unwrap();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    s.execute("doc.setText", json!({"text": "# 고침\n"})).unwrap();
    s.execute("file.save", json!({})).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 고침\n");
    assert!(!s.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn documents_import_and_export_markdown_files() {
    let dir = temp_dir("io");
    let src = dir.join("계획.md");
    std::fs::write(&src, "# 계획\n\n{{Sheet1!A1}}\n").unwrap();
    let mut s = session();
    let path = src.to_str().unwrap();
    let r = s.execute("doc.import", json!({"path": path})).unwrap();
    assert_eq!(r["name"], "계획");
    let r = s.execute("doc.import", json!({"path": path})).unwrap();
    assert_eq!(r["name"], "계획 (2)", "a name in use gets a number");
    assert_eq!(text(&mut s, "계획 (2)"), "# 계획\n\n{{Sheet1!A1}}\n");
    let out = dir.join("out.md");
    s.execute("doc.export", json!({"sheet": "계획", "path": out.to_str().unwrap()})).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "# 계획\n\n{{Sheet1!A1}}\n");
    assert!(s.execute("doc.export", json!({"sheet": "Sheet1", "path": out.to_str().unwrap()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
