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
fn a_word_document_opens_as_a_tab_its_interface_fills() {
    use monun_engine::docs::TextKind;
    let dir = temp_dir("word");
    // A .docx is a ZIP package, as a workbook is: it still opens as a Word document.
    let docx = dir.join("spec.docx");
    let bytes = b"PK\x03\x04 not read by the engine".to_vec();
    std::fs::write(&docx, &bytes).unwrap();
    let mut s = session();
    s.execute("file.open", json!({"path": docx.to_str().unwrap()})).unwrap();
    let d = s.active().unwrap();
    assert_eq!((d.text_kind(), d.is_word(), d.is_dirty()), (Some(TextKind::Word), true, false));
    assert_eq!((d.display_title(), d.path.as_deref()), ("spec.docx".to_string(), docx.to_str()));
    // The engine leaves it to its tab: no edit, save, export or print goes through.
    for (cmd, params) in
        [("doc.setText", json!({"text": "x"})), ("file.save", json!({})), ("file.exportPdf", json!({"path": dir.join("p.pdf").to_str().unwrap()}))]
    {
        let e = s.execute(cmd, params).unwrap_err().to_string();
        assert!(e.contains(monun_engine::docs::WORD_IN_TAB), "{cmd}: {e}");
    }
    assert_eq!(std::fs::read(&docx).unwrap(), bytes);
    // An older or other format opens as Word too, titled .docx and with no file of its own yet.
    let rtf = dir.join("letter.rtf");
    std::fs::write(&rtf, "{\\rtf1 hi}").unwrap();
    s.execute("file.open", json!({"path": rtf.to_str().unwrap()})).unwrap();
    let d = s.active().unwrap();
    assert_eq!((d.display_title(), d.path.clone(), d.is_word()), ("letter.docx".to_string(), None, true));
    assert_eq!(d.wb.source_path.as_deref(), Some(rtf.as_path()));
    // A new one.
    s.execute("file.new", json!({"document": true, "kind": "word"})).unwrap();
    assert_eq!((s.active().unwrap().display_title(), s.active().unwrap().is_word()), ("Document1.docx".to_string(), true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hangul_document_opens_as_a_tab_its_interface_fills() {
    use monun_engine::docs::TextKind;
    let dir = temp_dir("hwp");
    let mut s = session();
    // A .hwp is a compound file and a .hwpx a ZIP package, as old and new workbooks are: both
    // still open as 한글 documents, each keeping its own file.
    for (name, bytes) in [("plan.hwp", b"\xD0\xCF\x11\xE0 not read".to_vec()), ("plan.hwpx", b"PK\x03\x04 not read".to_vec())] {
        let path = dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
        let d = s.active().unwrap();
        assert_eq!((d.text_kind(), d.is_hwp(), d.is_dirty()), (Some(TextKind::Hwp), true, false), "{name}");
        assert_eq!((d.display_title(), d.path.as_deref()), (name.to_string(), path.to_str()));
        // The engine leaves it to its tab: no edit, save, export or print goes through.
        for (cmd, params) in [
            ("doc.setText", json!({"text": "x"})),
            ("file.save", json!({})),
            ("file.exportPdf", json!({"path": dir.join("p.pdf").to_str().unwrap()})),
        ] {
            let e = s.execute(cmd, params).unwrap_err().to_string();
            assert!(e.contains(monun_engine::docs::HWP_IN_TAB), "{name} {cmd}: {e}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    // A new one is a .hwpx.
    s.execute("file.new", json!({"document": true, "kind": "hwp"})).unwrap();
    assert_eq!((s.active().unwrap().display_title(), s.active().unwrap().is_hwp()), ("Document1.hwpx".to_string(), true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_picture_opens_as_a_tab_to_look_at_and_nothing_writes_it() {
    use monun_engine::docs::TextKind;
    let dir = temp_dir("picture");
    let path = dir.join("diagram.png");
    let bytes = b"\x89PNG not read by the engine".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let mut s = session();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    let d = s.active().unwrap();
    assert_eq!((d.text_kind(), d.is_image(), d.is_dirty()), (Some(TextKind::Image), true, false));
    assert_eq!((d.display_title(), d.path.as_deref()), ("diagram.png".to_string(), path.to_str()));
    for (cmd, params) in [("doc.setText", json!({"text": "x"})), ("file.save", json!({})), ("file.reload", json!({}))] {
        let e = s.execute(cmd, params).unwrap_err().to_string();
        assert!(e.contains(monun_engine::docs::IMAGE_READ_ONLY), "{cmd}: {e}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_html_file_or_a_site_link_opens_as_a_web_tab_and_nothing_writes_it() {
    use monun_engine::docs::TextKind;
    let dir = temp_dir("web");
    let page = dir.join("설계.html");
    let link = dir.join("사이트.url");
    let html = "<html><body><table><tr><td>1</td></tr></table></body></html>".as_bytes().to_vec();
    let url = b"[InternetShortcut]\r\nURL=https://example.com/\r\n".to_vec();
    std::fs::write(&page, &html).unwrap();
    std::fs::write(&link, &url).unwrap();
    let mut s = session();
    for (path, bytes) in [(&page, &html), (&link, &url)] {
        s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
        let d = s.active().unwrap();
        // The page, not its table as a sheet.
        assert_eq!((d.text_kind(), d.is_web(), d.is_dirty()), (Some(TextKind::Web), true, false), "{path:?}");
        assert_eq!(d.path.as_deref(), path.to_str());
        for (cmd, params) in [("doc.setText", json!({"text": "x"})), ("file.save", json!({})), ("file.reload", json!({}))] {
            let e = s.execute(cmd, params).unwrap_err().to_string();
            assert!(e.contains(monun_engine::docs::WEB_READ_ONLY), "{cmd}: {e}");
        }
        assert_eq!(&std::fs::read(path).unwrap(), bytes);
    }
    // A workbook is still saved as a web page.
    s.execute("file.new", json!({})).unwrap();
    let out = dir.join("표.html");
    s.execute("file.exportHtml", json!({"path": out.to_str().unwrap()})).unwrap();
    assert!(std::fs::read_to_string(&out).unwrap().contains("<html"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pdf_file_opens_as_a_tab_to_read_and_nothing_writes_it() {
    use monun_engine::docs::TextKind;
    let dir = temp_dir("pdf");
    let path = dir.join("design.pdf");
    let bytes = b"%PDF-1.7
% not drawn by the engine
%%EOF
"
    .to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let mut s = session();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    let d = s.active().unwrap();
    assert_eq!((d.text_kind(), d.is_pdf(), d.is_dirty()), (Some(TextKind::Pdf), true, false));
    let doc = d.wb.active().unwrap().document.clone().unwrap();
    assert_eq!((doc.text.as_str(), doc.fit_width), ("", true), "no text; as wide as the window");
    // How it shows is no change to it.
    s.execute("view.zoom", json!({"percent": 150})).unwrap();
    s.execute("doc.view", json!({"fitWidth": false})).unwrap();
    assert!(!s.active().unwrap().is_dirty());
    // Nothing changes, saves, exports or prints it.
    for (cmd, params) in [
        ("doc.setText", json!({"text": "x"})),
        ("file.save", json!({})),
        ("file.saveAs", json!({"path": dir.join("copy.pdf").to_str().unwrap()})),
        ("file.saveAs", json!({"path": dir.join("copy.xlsx").to_str().unwrap()})),
        ("file.exportPdf", json!({"path": dir.join("print.pdf").to_str().unwrap()})),
        ("file.print", json!({})),
    ] {
        let e = s.execute(cmd, params).unwrap_err().to_string();
        assert!(e.contains(monun_engine::docs::PDF_READ_ONLY), "{cmd}: {e}");
    }
    let e = s.execute("doc.export", json!({"path": dir.join("out.md").to_str().unwrap()})).unwrap_err().to_string();
    assert!(e.contains(monun_engine::docs::PDF_READ_ONLY), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!dir.join("copy.xlsx").exists() && !dir.join("print.pdf").exists() && !dir.join("out.md").exists());
    // Another program's change to a text file behind it still reads in, and the PDF itself
    // never reloads.
    let md = dir.join("notes.md");
    std::fs::write(
        &md, "# one
",
    )
    .unwrap();
    s.execute("file.open", json!({"path": md.to_str().unwrap()})).unwrap();
    let md_uid = s.active().unwrap().uid;
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    assert!(s.active().unwrap().is_pdf());
    std::fs::write(
        &md, "# two
",
    )
    .unwrap();
    s.execute("file.reload", json!({"document": md_uid})).unwrap();
    let reread = s.documents().iter().find(|d| d.uid == md_uid).unwrap().wb.sheet(0).unwrap().document.clone().unwrap();
    assert_eq!(
        reread.text,
        "# two
"
    );
    assert!(s.execute("file.reload", json!({})).unwrap_err().to_string().contains(monun_engine::docs::PDF_READ_ONLY));
    // A file named .pdf that isn't one is refused.
    let fake = dir.join("fake.pdf");
    std::fs::write(
        &fake, "a,b
1,2
",
    )
    .unwrap();
    let e = s.execute("file.open", json!({"path": fake.to_str().unwrap()})).unwrap_err().to_string();
    assert!(e.contains("is not a PDF file"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sql_and_text_files_open_as_text_and_save_back_as_they_came() {
    use monun_engine::docs::TextKind;
    // Windows line breaks and code page 949 (an older Korean file) come back as they were.
    let sql = "-- 사용자 목록\r\nSELECT 이름 FROM 사용자;\r\n";
    let (cp949, _, _) = encoding_rs::EUC_KR.encode(sql);
    let (wb, _) = io::open_bytes("목록.sql", &cp949).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    let doc = wb.sheet(0).unwrap().document.as_ref().unwrap();
    assert_eq!(doc.text, "-- 사용자 목록\nSELECT 이름 FROM 사용자;\n", "line breaks are \\n in the editor");
    assert_eq!(io::save_bytes(&wb, "목록.sql").unwrap(), cp949.to_vec());
    // UTF-8 with a byte-order mark keeps it; plain UTF-8 and \n stay so.
    for bytes in ["\u{feff}메모\r\n둘째 줄".as_bytes(), "메모\n둘째 줄\n".as_bytes(), b"".as_slice()] {
        let (wb, _) = io::open_bytes("메모.txt", bytes).unwrap();
        assert!(wb.sheet(0).unwrap().is_document(), "a .txt is text, not tab-separated cells");
        assert_eq!(io::save_bytes(&wb, "메모.txt").unwrap(), bytes);
    }
    // A workbook goes to .txt as its sheet's values separated by tabs, as in Excel, and never to .sql.
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "a"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "1"})).unwrap();
    let wb = s.active().unwrap().wb.clone();
    assert_eq!(io::save_bytes(&wb, "값.txt").unwrap(), b"\xEF\xBB\xBFa\t1\r\n");
    assert!(io::save_bytes(&wb, "값.sql").unwrap_err().to_string().contains("Only a text file"));
    // In a session: new files of each kind, and Save of an opened .sql that leaves it clean.
    let r = s.execute("file.new", json!({"document": true, "kind": "sql"})).unwrap();
    assert_eq!(r["title"], "Query1.sql");
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Sql));
    let r = s.execute("file.new", json!({"document": true, "kind": "text", "prefix": "텍스트"})).unwrap();
    assert_eq!(r["title"], "텍스트1.txt");
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Plain));
    assert!(s.execute("file.new", json!({"document": true, "kind": "python"})).is_err());
    let dir = temp_dir("sql");
    let path = dir.join("조회.sql");
    std::fs::write(&path, "SELECT 1\r\n").unwrap();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Sql));
    s.execute("doc.setText", json!({"text": "SELECT 1\nFROM DUAL\n"})).unwrap();
    s.execute("file.save", json!({})).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"SELECT 1\r\nFROM DUAL\r\n");
    assert!(!s.active().unwrap().is_dirty());
    // Saved under another name, it is that kind of file.
    let as_text = dir.join("조회.txt");
    s.execute("file.save", json!({"path": as_text.to_str().unwrap()})).unwrap();
    assert_eq!(s.active().unwrap().text_kind(), Some(TextKind::Plain));
    assert!(s.execute("doc.picture", json!({"base64": "AA=="})).is_err(), "pictures go beside Markdown files only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_text_file_changed_by_another_program_reads_again() {
    let dir = temp_dir("reload");
    let path = dir.join("설계.md");
    std::fs::write(&path, "# 처음\n").unwrap();
    let mut s = session();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    s.execute("doc.view", json!({"mode": "view", "fitWidth": true})).unwrap();
    assert!(!s.active().unwrap().changed_on_disk(), "as it was read");
    // Another program writes it.
    std::fs::write(&path, "# 고친 뒤\n\n새 문단.\n").unwrap();
    assert!(s.active().unwrap().changed_on_disk());
    s.execute("file.reload", json!({})).unwrap();
    let d = s.active().unwrap();
    let doc = d.wb.sheet(0).unwrap().document.as_ref().unwrap();
    assert_eq!(doc.text, "# 고친 뒤\n\n새 문단.\n");
    assert_eq!((doc.mode.map(|m| m.name()), doc.fit_width), (Some("view"), true), "it shows as it did");
    assert!(!d.is_dirty() && !d.changed_on_disk() && d.undo.is_empty());
    // Its own Save is no change of another program's.
    s.execute("doc.setText", json!({"text": "# 여기서\n"})).unwrap();
    s.execute("file.save", json!({})).unwrap();
    assert!(!s.active().unwrap().changed_on_disk());
    // A workbook isn't read again.
    s.execute("file.new", json!({})).unwrap();
    assert!(s.execute("file.reload", json!({})).unwrap_err().to_string().contains("Only a Markdown, SQL or text file"));
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

#[test]
fn a_documents_pictures_survive_xlsx() {
    // The smallest PNG: one transparent pixel.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let mut s = session();
    s.execute("doc.insert", json!({"name": "메모", "text": "![점](image:1)"})).unwrap();
    let r = s.execute("insert.picture", json!({"base64": PNG, "at": "C1", "alt": "점"})).unwrap();
    let id = r["image"].as_u64().unwrap();
    let wb = s.active().unwrap().wb.clone();
    let back = reopen(&wb);
    let doc = back.sheet(1).unwrap();
    assert!(doc.is_document());
    assert_eq!(doc.images.len(), 1, "the picture stays on the document tab");
    assert_eq!(doc.images[0].anchor.cell.a1(), "C1");
    assert!(id > 0);
}

fn pdf_of(s: &mut Session, params: Value) -> (Vec<u8>, u64) {
    let r = s.execute("file.exportPdf", params).unwrap();
    let bytes = io::base64_decode(r["base64"].as_str().unwrap()).unwrap();
    (bytes, r["pages"].as_u64().unwrap())
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn a_document_prints_as_text_pages() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "30000"})).unwrap();
    s.execute("home.numberFormat", json!({"range": "A1", "code": "#,##0"})).unwrap();
    let mut text = String::from(
        "# Plan\n\nTotal {{Sheet1!A1}} won.\n\n- [x] done\n- open\n\n| Item | Amount |\n|---|--:|\n| AAA | 10,000 |\n\n```\ncode line\n```\n\n",
    );
    for i in 0..120 {
        text.push_str(&format!("Paragraph number {i} with enough words to fill a line or two on the page.\n\n"));
    }
    s.execute("doc.insert", json!({"name": "Notes", "text": text})).unwrap();
    let (pdf, pages) = pdf_of(&mut s, json!({}));
    assert!(pages >= 3, "a long document runs over pages: {pages}");
    for needle in ["(Plan)", "(30,000)", "(Item)", "(10,000)", "(code line)", "(Paragraph number 119 "] {
        assert!(contains(&pdf, needle), "{needle} is printed");
    }
    // Printing every sheet takes the document's pages too.
    let (_, all) = pdf_of(&mut s, json!({"sheets": "all"}));
    assert!(all >= pages, "{all} pages for all sheets, {pages} for the document");
}

#[test]
fn a_sql_file_prints_its_text_line_for_line() {
    let mut s = Session::new();
    s.execute("file.new", json!({"document": true, "kind": "sql"})).unwrap();
    s.execute("doc.setText", json!({"text": "# not a heading\nSELECT *\n  FROM t -- *all*\n"})).unwrap();
    let (pdf, _) = pdf_of(&mut s, json!({}));
    for needle in ["(# not a heading)", "(SELECT *)", "(  FROM t -- *all*)"] {
        assert!(contains(&pdf, needle), "{needle} is printed as written, not as Markdown");
    }
}

#[test]
fn a_long_table_repeats_its_header_on_each_page() {
    let mut s = session();
    let mut text = String::from("| Ticker | Note |\n|---|---|\n");
    for i in 0..150 {
        text.push_str(&format!("| T{i} | row {i} |\n"));
    }
    s.execute("doc.insert", json!({"name": "Notes", "text": text})).unwrap();
    let (pdf, pages) = pdf_of(&mut s, json!({}));
    assert!(pages >= 2, "the table runs over pages: {pages}");
    let headers = pdf.windows(b"(Ticker)".len()).filter(|w| *w == b"(Ticker)").count();
    assert_eq!(headers as u64, pages, "one header row on every page");
    assert!(contains(&pdf, "(row 149)"));
}

fn html_of(s: &mut Session, params: Value) -> String {
    let mut p = json!({"format": "html"});
    if let (Some(o), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        o.extend(extra.clone());
    }
    let r = s.execute("file.saveBytes", p).unwrap();
    String::from_utf8(io::base64_decode(r["base64"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn a_document_web_page_shows_it_as_the_tab_does() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "30000"})).unwrap();
    s.execute("home.numberFormat", json!({"range": "A1", "code": "#,##0"})).unwrap();
    let text = "# Plan\n\nTotal {{Sheet1!A1}} won, **bold** and `code`.\n\n- [x] done\n- [ ] open\n\n\
                | Item | Amount |\n|---|--:|\n| AAA | 10 |\n\n> quoted\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n\
                [see](https://example.com) and [the total](#Sheet1!A1)\n";
    s.execute("doc.insert", json!({"name": "Notes", "text": text})).unwrap();
    let html = html_of(&mut s, json!({}));
    for needle in [
        "<article class=\"doc\">",
        "<h1>Plan</h1>",
        "<span class=\"field\" title=\"{{Sheet1!A1}}\">30,000</span>",
        "<strong>bold</strong>",
        "<code>code</code>",
        "<li class=\"task\"><input type=\"checkbox\" disabled checked>done</li>",
        "<table class=\"md\"><thead><tr><th>Item</th><th>Amount</th></tr></thead>",
        "<td style=\"text-align:right\">10</td>",
        "<blockquote>",
        "<a href=\"https://example.com\">see</a>",
    ] {
        assert!(html.contains(needle), "{needle} in {html}");
    }
    // A link to another sheet is plain text when that sheet isn't on the page.
    assert!(html.contains("and the total</p>"), "{html}");
    assert!(!html.contains("<nav"), "one sheet has no menu");
    // Mermaid is drawn where this build draws diagrams, else it is its code.
    assert!(html.contains(if cfg!(feature = "diagrams") { "<div class=\"diagram\"><svg" } else { "<div class=\"lang\">mermaid</div>" }));
    // Save As a web page shows the document too, not its lines as cells.
    let wb = s.active().unwrap().wb.clone();
    let saved = String::from_utf8(io::save_bytes(&wb, "notes.html").unwrap()).unwrap();
    assert!(saved.contains("<article class=\"doc\"><h1>Plan</h1>") && !saved.contains("<table class=\"flow\""), "{saved}");
    // Fit Width widens tables to the page.
    let fit = html_of(&mut s, json!({"fitWidth": true}));
    assert!(fit.contains("<article class=\"doc fit\">"));
    // A diagram keeps its own width there, centred: no wider than its drawing says.
    if cfg!(feature = "diagrams") {
        let own = |page: &str| -> Option<f32> {
            let svg = page.get(page.find("<div class=\"diagram\"")?..)?;
            let svg = svg.get(svg.find("<svg")?..)?;
            let at = svg.find(" width=\"")? + " width=\"".len();
            svg.get(at..)?.split('"').next()?.trim_end_matches("px").parse().ok()
        };
        let cap = |page: &str| -> Option<f32> {
            let tag = "<div class=\"diagram\" style=\"max-width:";
            let at = page.find(tag)? + tag.len();
            page.get(at..)?.split("px").next()?.parse().ok()
        };
        let (own, cap) = (own(&fit), cap(&fit));
        assert!(own.is_some() && cap.is_some(), "{fit}");
        assert!((cap.unwrap_or(0.0) - own.unwrap_or(0.0)).abs() <= 0.5, "{own:?} {cap:?}");
    }
}

#[test]
fn a_web_page_of_all_sheets_has_a_menu_of_their_names() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    s.execute("doc.insert", json!({"name": "Notes", "text": "# Plan\n\n[the answer](#Sheet1!A1)\n"})).unwrap();
    s.execute("home.insertSheet", json!({"name": "Hidden"})).unwrap();
    s.execute("sheet.hide", json!({"sheet": "Hidden"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": "Notes"})).unwrap();
    let html = html_of(&mut s, json!({"sheets": "all"}));
    assert!(html.contains("<nav class=\"sheets\"><a href=\"#sheet-1\">Sheet1</a><a href=\"#sheet-2\">Notes</a></nav>"), "{html}");
    assert!(!html.contains(">Hidden<"), "hidden sheets stay out");
    assert!(html.contains("<section class=\"sheet\" id=\"sheet-1\"><table class=\"flow\">"), "{html}");
    assert!(html.contains(">42</td>"));
    assert!(html.contains("<section class=\"sheet\" id=\"sheet-2\"><article class=\"doc\"><h1>Plan</h1>"), "{html}");
    // A link to a sheet on the page goes to it.
    assert!(html.contains("<a href=\"#sheet-1\">the answer</a>"), "{html}");
    // Only the active sheet: no menu.
    let one = html_of(&mut s, json!({"sheets": "active"}));
    assert!(!one.contains("<nav") && one.contains("<h1>Plan</h1>") && !one.contains(">42<"));
    assert!(s.execute("file.saveBytes", json!({"format": "html", "sheets": "some"})).is_err());
}

#[test]
fn fit_width_widens_a_document_table_on_paper() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "Notes", "text": "| A | B |\n|---|---|\n| 1 | 2 |\n"})).unwrap();
    let (plain, _) = pdf_of(&mut s, json!({}));
    let (wide, _) = pdf_of(&mut s, json!({"fitWidth": true}));
    assert_ne!(plain, wide, "the table's lines reach the page's width");
}

#[test]
fn a_document_keeps_its_mode_and_fit_width_in_its_files() {
    let mut s = session();
    s.execute("doc.insert", json!({"name": "Notes", "text": "# Plan\n"})).unwrap();
    let r = s.execute("doc.get", json!({})).unwrap();
    assert_eq!((r["mode"].clone(), r["fitWidth"].clone()), (Value::Null, json!(false)), "none chosen yet");
    let saved = s.active().unwrap().wb.clone();
    s.active_mut().unwrap().saved = saved;
    let undo = s.active().unwrap().undo.len();
    let r = s.execute("doc.view", json!({"mode": "edit", "fitWidth": true})).unwrap();
    assert_eq!(r, json!({"mode": "edit", "fitWidth": true}));
    let d = s.active().unwrap();
    assert!(d.is_dirty(), "the file is to be saved");
    assert_eq!(d.undo.len(), undo, "no undo step");
    // XLSX keeps them in the tab's mark, JSON in the document.
    let back = reopen(&d.wb);
    let doc = back.sheet(1).unwrap().document.clone().unwrap();
    assert_eq!((doc.mode, doc.fit_width, doc.text.as_str()), (Some(monun_engine::model::DocMode::Edit), true, "# Plan\n"));
    let json = io::save_bytes(&d.wb, "book.json").unwrap();
    let (back, _) = io::open_bytes("book.json", &json).unwrap();
    assert_eq!(back.sheet(1).unwrap().document.as_ref().unwrap().mode, Some(monun_engine::model::DocMode::Edit));
    // One setting at a time leaves the other.
    s.execute("doc.view", json!({"mode": "split"})).unwrap();
    assert_eq!(s.execute("doc.get", json!({})).unwrap()["fitWidth"], json!(true));
    assert!(s.execute("doc.view", json!({"mode": "wide"})).is_err());
    assert!(s.execute("doc.view", json!({"sheet": "Sheet1", "mode": "view"})).is_err(), "only a document has a mode");
}
