//! File menu: new, open, save, close, export, properties, options.

use serde_json::{Value as Json, json};

use super::{CommandSpec, Result, Session, always, bad, bool_param, cmd, edit, has_doc, ok, str_param, target_sheet, u32_param, writable};
use crate::DocState;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "file.new", "New Workbook", ["File"], Some("Cmd+N"), "{sample?: \"purchases\" (구매목록, the File › New template)|\"budget\"|\"sales\"|\"grades\"} | {document: true, kind?: \"markdown\"|\"sql\"|\"text\"|\"word\"|\"hwp\", prefix?} (a new text file: Markdown `Doc1.md`, SQL `Query1.sql`, plain text `Text1.txt`, a Word document `Document1.docx` or a 한글 document `Document1.hwpx`, or `{prefix}1.md` and so on)", always, new_workbook),
        cmd!(noundo "file.open", "Open…", ["File"], Some("Cmd+O"), "{path} | {name, base64}, plus {password} for a password-protected workbook and {csvAsText?: bool} to keep every csv/tsv/txt field as text, with no number or date conversion (xlsx, xlsm, xls import, xlsb and ods data import, csv, tsv, txt, json)", always, open),
        cmd!(noundo "file.save", "Save", ["File"], Some("Cmd+S"), "{path?} (xlsx by default; .csv/.tsv/.json/.html/.pdf by extension)", writable, save),
        cmd!(noundo "file.saveAs", "Save As…", ["File"], Some("Cmd+Shift+S"), "{path} (.xlsx, .csv, .tsv, .json, .html, or .pdf: a PDF of the active sheet)", writable, save_as),
        cmd!(query "file.saveBytes", "Encode Workbook", [], None, "{format?: xlsx|csv|tsv|json|html, encoding?: utf-8|cp949, sheets?: active|all, fitWidth?: bool} → {base64} (encoding: csv and tsv only, see file.exportCsv; sheets and fitWidth: html only, see file.exportHtml)", writable, save_bytes),
        cmd!(noundo "file.close", "Close", ["File"], Some("Cmd+W"), "{force?: bool, document?: bool} (closes the active window, or with `document` the workbook and all its windows; the workbook closes with its last window)", has_doc, close),
        cmd!(noundo "file.exportCsv", "Export as CSV", ["File", "Export"], None, "{path, sheet?, encoding?: utf-8|cp949} (utf-8 is Excel's CSV UTF-8, with a byte order mark; cp949 its plain CSV on Korean Windows; without it a workbook read from CSV or text keeps that file's encoding, any other is CSV UTF-8)", writable, |s, p| export(s, p, "csv")),
        cmd!(noundo "file.exportHtml", "Save as Web Page", ["File", "Export"], None, "{path, sheet?, sheets?: active|all, fitWidth?: bool} (all: every visible sheet on one page, behind a menu of their names; fitWidth: document tabs' tables and diagrams as wide as the page)", writable, |s, p| export(s, p, "html")),
        cmd!("file.properties", "Properties", ["File"], None, "{title?, subject?, author?, company?, keywords?, description?}", writable, properties),
        cmd!(
            query "file.options",
            "Options",
            ["File"],
            None,
            "{} → {calculation: {mode, iterative, maxIterations, maxChange}} for the active workbook (the interface opens File › Options; change settings with their own commands, e.g. formulas.calculationOptions, view.theme, app.language.set)",
            always,
            options
        ),
        cmd!(noundo "window.activate", "Switch Window", ["Window"], None, "{window} | {doc} | {index} (a window id, a workbook id, or the workbook's position in the list)", has_doc, activate_window),
        cmd!(noundo "window.close", "Close Window", ["Window"], None, "{window?, doc?, force?: bool} → {closed, document}: closes a window (the active one by default); its workbook closes with its last window, asking first when it has unsaved changes unless `force`", has_doc, close_window),
        cmd!(
            noundo "file.reload",
            "Reload from File",
            [],
            None,
            "{document?: uid} → {document, path}: reads a Markdown, SQL or text file again from its file, as another program left it; how it shows stays, its undo history goes",
            has_doc,
            reload
        ),
        cmd!(query "window.list", "Windows", ["Window"], None, "{} → [{window, doc, index, title, dirty, active, hidden}]", always, |s, _| Ok(s.window_list())),
    ]
}

fn new_workbook(s: &mut Session, p: &Json) -> Result<Json> {
    if bool_param(p, "document").unwrap_or(false) {
        let kind = match str_param(p, "kind") {
            Some(k) => {
                crate::docs::TextKind::parse(k).ok_or_else(|| bad("file.new", format!("`kind` must be markdown, sql, text or word, not `{k}`")))?
            }
            None => crate::docs::TextKind::Markdown,
        };
        let i = s.new_text_file(kind, str_param(p, "prefix").unwrap_or(""));
        return Ok(json!({"index": i, "title": s.doc()?.display_title()}));
    }
    let i = match str_param(p, "sample") {
        Some(name) => {
            let wb = crate::sample::build_with(name, &s.prefs).ok_or_else(|| bad("file.new", format!("unknown sample `{name}`")))?;
            let title = crate::sample::title(name);
            s.add_document(DocState::new(wb, None, title, crate::Trust::Local))
        }
        None => s.new_workbook(),
    };
    Ok(json!({"index": i, "title": s.doc()?.display_title()}))
}

fn open(s: &mut Session, p: &Json) -> Result<Json> {
    let (name, bytes) = if let Some(b) = str_param(p, "base64") {
        (str_param(p, "name").unwrap_or("Book.xlsx").to_string(), crate::io::base64_decode(b).ok_or_else(|| bad("file.open", "invalid base64"))?)
    } else if let Some(path) = str_param(p, "path") {
        // Already open? Switch to it.
        if let Some(i) = s.documents().iter().position(|d| d.path.as_deref() == Some(path)) {
            s.set_active(i);
            return Ok(json!({"index": i, "alreadyOpen": true}));
        }
        // A PDF, a Word or a 한글 document is only recognised here: the interface reads the file
        // itself.
        let bytes = match crate::io::FileKind::from_path(path) {
            Some(crate::io::FileKind::Pdf | crate::io::FileKind::Word | crate::io::FileKind::Hwp) => crate::io::read_head(path, 1024)?,
            _ => crate::io::read_file(path)?,
        };
        (path.to_string(), bytes)
    } else {
        s.ui_requests.push(crate::UiRequest::Dialog("open".into(), json!({})));
        return ok();
    };
    // A Word document is read by the interface, password and all; .doc, .dotx, .rtf and .odt are
    // saved as .docx, so like an import they have no save target of their own.
    let word = crate::io::FileKind::from_path(&name) == Some(crate::io::FileKind::Word);
    let word_import = word && !std::path::Path::new(&name).extension().is_some_and(|e| e.eq_ignore_ascii_case("docx"));
    // A 한글 document is read by the interface too, as it is.
    let hwp = crate::io::FileKind::from_path(&name) == Some(crate::io::FileKind::Hwp);
    let encrypted = !word && !hwp && monun_xlsx::sniff(&bytes) == monun_xlsx::Format::Encrypted;
    let (mut wb, warnings) = crate::io::open_bytes_with(&name, &bytes, str_param(p, "password"), bool_param(p, "csvAsText").unwrap_or(false))?;
    wb.source_dir = str_param(p, "path").and_then(crate::io::source_dir);
    // What `CELL("filename")` reports (#1489). A workbook opened from bytes knows its name but
    // never a folder, so it reports `[name]Sheet` with none; an import reports the file it came
    // from, as Excel does, even though Save must still ask where to put the .xlsx.
    wb.source_path = match str_param(p, "path") {
        Some(path) => crate::io::source_path(path),
        None => Some(std::path::PathBuf::from(&name)),
    };
    let imported = !word
        && !hwp
        && (matches!(crate::io::FileKind::from_path(&name), Some(crate::io::FileKind::Ods | crate::io::FileKind::Xlsb | crate::io::FileKind::Xls))
            || matches!(monun_xlsx::sniff(&bytes), monun_xlsx::Format::Ods | monun_xlsx::Format::Xlsb | monun_xlsx::Format::Xls));
    let title = if word_import {
        format!("{}.docx", std::path::Path::new(&name).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Document"))
    } else if imported {
        format!("{} (imported).xlsx", std::path::Path::new(&name).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Book"))
    } else {
        std::path::Path::new(&name).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or("Book").to_string()
    };
    // An import has no save target: ordinary Save must ask for a new destination. Neither does a
    // workbook decrypted with its password, so Save never replaces the encrypted original with
    // an unprotected copy.
    let path = str_param(p, "path").filter(|_| !imported && !encrypted && !word_import).map(str::to_string);
    // Replace an untouched blank Book1 like Excel does.
    if s.documents().len() == 1
        && s.active().is_some_and(|d| d.path.is_none() && !d.is_dirty() && d.undo.is_empty() && d.wb.sheets.iter().all(|sh| sh.cells.is_empty()))
    {
        s.close_document(0);
    }
    // Someone else may have made the file: its active content asks before it runs.
    let disk = path.as_deref().and_then(crate::io::disk_stamp);
    let mut doc = DocState::new(wb, path, title, crate::Trust::FromFile);
    doc.disk = disk;
    if crate::io::FileKind::from_path(&name).is_none() && monun_xlsx::sniff(&bytes) == monun_xlsx::Format::Xlsx {
        doc.content_kind = Some(crate::io::FileKind::Xlsx);
    }
    let i = s.add_document(doc);
    // The decrypted-workbook notice is for the info bar, not a message box.
    let shown: Vec<&str> = warnings.iter().map(String::as_str).filter(|w| *w != crate::io::DECRYPTED_NOTICE).collect();
    if !shown.is_empty() {
        let mut message = shown.iter().take(5).copied().collect::<Vec<_>>().join("\n\n");
        if shown.len() > 5 {
            message.push_str(&format!("\n\n{} additional import warnings.", shown.len() - 5));
        }
        s.ui_requests.push(crate::UiRequest::Message(message));
    }
    let d = s.doc()?;
    Ok(
        json!({"index": i, "title": d.display_title(), "sheets": d.wb.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), "warnings": warnings}),
    )
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let path = match str_param(p, "path").map(str::to_string).or_else(|| s.active().and_then(|d| d.path.clone())) {
        Some(p) => p,
        None => {
            s.ui_requests.push(crate::UiRequest::Dialog("saveAs".into(), json!({})));
            return ok();
        }
    };
    // A PDF is a print of the sheet (Page Setup applies), not a copy of the workbook: the
    // document keeps its file and its unsaved changes.
    if std::path::Path::new(&path).extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return super::print::export_pdf(s, &json!({"path": path}));
    }
    let d = s.doc_mut()?;
    // The document's own file keeps the format its content had when opened; any other path
    // goes by its extension. Relative paths to linked workbooks are rewritten for the new folder.
    let kind = crate::io::FileKind::from_path(&path).or(d.content_kind.filter(|_| d.path.as_deref() == Some(path.as_str())));
    let rebased = crate::io::source_dir(&path).and_then(|f| monun_xlsx::rebase_links(&d.wb, &f));
    let wb = rebased.as_ref().unwrap_or(&d.wb);
    let bytes = match kind {
        Some(kind) => crate::io::save_bytes_as(wb, &path, kind)?,
        None => crate::io::save_bytes(wb, &path)?,
    };
    crate::io::write_file(&path, &bytes)?;
    // A Markdown file's pictures are files beside it.
    if kind == Some(crate::io::FileKind::Markdown)
        && let Some(sh) = d.wb.sheet(d.wb.active_sheet())
    {
        crate::docs::write_pictures(sh, &path).map_err(crate::EngineError::Other)?;
    }
    // Only a full-fidelity format clears the dirty flag and becomes the document's file: Markdown
    // and text are for a workbook that is a single document tab (a .md, .sql or .txt file opened).
    let text = crate::docs::is_text_file(&d.wb) && matches!(kind, Some(crate::io::FileKind::Markdown | crate::io::FileKind::Text));
    if matches!(kind, Some(crate::io::FileKind::Xlsx | crate::io::FileKind::Json)) || text {
        // The new file is the workbook's own now, so `CELL("filename")` reports it (#1489). Set
        // before the `saved` snapshot, or the workbook would read as dirty the moment it was
        // saved; and only when it changes, so an ordinary Save does not clone the workbook.
        let file = crate::io::source_path(&path);
        if d.wb.source_path != file {
            std::sync::Arc::make_mut(&mut d.wb).source_path = file;
        }
        d.saved = d.wb.clone();
        d.path = Some(path.clone());
        d.disk = crate::io::disk_stamp(&path);
    }
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

/// Refused when a workbook would be read again from its file.
pub const RELOAD_TEXT_ONLY: &str = "Only a Markdown, SQL or text file is read again from its file.";

/// Reads a Markdown, SQL or text file again from its file, as another program left it (see
/// `file.reload`'s params).
fn reload(s: &mut Session, p: &Json) -> Result<Json> {
    let uid = match p.get("document").and_then(Json::as_u64) {
        Some(uid) => uid,
        None => s.doc()?.uid,
    };
    let d = s.documents().iter().find(|d| d.uid == uid).ok_or_else(|| bad("document", format!("no open document {uid}")))?;
    if d.is_pdf() {
        return Err(crate::EngineError::Other(crate::docs::PDF_READ_ONLY.into()));
    }
    if d.is_word() {
        return Err(crate::EngineError::Other(crate::docs::WORD_IN_TAB.into()));
    }
    if d.is_hwp() {
        return Err(crate::EngineError::Other(crate::docs::HWP_IN_TAB.into()));
    }
    if !d.is_text_file() {
        return Err(crate::EngineError::Other(RELOAD_TEXT_ONLY.into()));
    }
    let path = d.path.clone().ok_or_else(|| bad("file.reload", "the file has not been saved yet"))?;
    let bytes = crate::io::read_file(&path)?;
    let (mut wb, _) = crate::io::open_bytes(&path, &bytes)?;
    // How it shows stays: its mode, Fit Width and zoom.
    if let (Some(old), Some(new)) = (d.wb.sheet(0), wb.sheet_mut(0)) {
        new.zoom = old.zoom;
        if let (Some(was), Some(now)) = (old.document.as_ref(), new.document.as_mut()) {
            now.mode = was.mode;
            now.fit_width = was.fit_width;
        }
    }
    let disk = crate::io::disk_stamp(&path);
    let d = s.document_mut(uid).ok_or(crate::EngineError::NoDocument)?;
    d.wb = std::sync::Arc::new(wb);
    d.saved = d.wb.clone();
    d.undo.clear();
    d.redo.clear();
    d.revision += 1;
    d.disk = disk;
    Ok(json!({"document": uid, "path": path}))
}

fn save_as(s: &mut Session, p: &Json) -> Result<Json> {
    if str_param(p, "path").is_none() {
        s.ui_requests.push(crate::UiRequest::Dialog("saveAs".into(), json!({})));
        return ok();
    }
    save(s, p)
}

fn save_bytes(s: &mut Session, p: &Json) -> Result<Json> {
    let fmt = str_param(p, "format").unwrap_or("xlsx");
    let encoding = csv_encoding(p, "file.saveBytes")?;
    let d = s.doc()?;
    let stem = std::path::Path::new(&d.display_title()).file_stem().and_then(|x| x.to_str()).unwrap_or("Book").to_owned();
    let name = format!("{stem}.{fmt}");
    if fmt == "html" {
        let bytes = web_page(&d.wb, p, &stem, "file.saveBytes")?;
        return Ok(json!({"name": name, "base64": crate::io::base64_encode(&bytes), "bytes": bytes.len()}));
    }
    // The file name goes into a PivotChart's link to its PivotTable.
    let bytes = match encoding {
        Some(encoding) => {
            let mut wb = (*d.wb).clone();
            wb.csv_encoding = Some(encoding);
            crate::io::save_bytes(&wb, &name)?
        }
        None => crate::io::save_bytes(&d.wb, &name)?,
    };
    Ok(json!({"name": name, "base64": crate::io::base64_encode(&bytes), "bytes": bytes.len()}))
}

/// The `encoding` of a CSV or text export: `utf-8` is Excel's "CSV UTF-8" (with a byte order mark),
/// `cp949` its plain "CSV" on Korean Windows. `None` keeps the encoding of the file the workbook
/// was read from.
fn csv_encoding(p: &Json, id: &str) -> Result<Option<monun_model::TextEncoding>> {
    match str_param(p, "encoding") {
        None => Ok(None),
        Some(e) if e.eq_ignore_ascii_case("utf-8") || e.eq_ignore_ascii_case("utf8") => Ok(Some(monun_model::TextEncoding::Utf8Bom)),
        Some(e) if e.eq_ignore_ascii_case("cp949") => Ok(Some(monun_model::TextEncoding::Cp949)),
        Some(e) => Err(bad(id, format!("unknown encoding `{e}`: use utf-8 or cp949"))),
    }
}

/// File › Close closes the active window; the workbook closes with its last window.
fn close(s: &mut Session, p: &Json) -> Result<Json> {
    if !bool_param(p, "document").unwrap_or(false)
        && let Some(id) = s.active_window()
        && let Some(doc) = s.window(id).map(|w| w.doc_uid)
        && s.windows().iter().filter(|w| w.doc_uid == doc).count() > 1
    {
        s.close_window(id);
        return Ok(json!({"closed": true, "document": false}));
    }
    let force = bool_param(p, "force").unwrap_or(false);
    let d = s.doc()?;
    if d.is_dirty() && !force {
        s.ui_requests.push(crate::UiRequest::Dialog("saveChanges".into(), json!({"title": d.display_title()})));
        return Ok(json!({"closed": false, "dirty": true}));
    }
    let i = s.active_index();
    s.close_document(i);
    Ok(json!({"closed": true}))
}

fn export(s: &mut Session, p: &Json, ext: &str) -> Result<Json> {
    let path = str_param(p, "path").ok_or_else(|| bad("file.export", "missing `path`"))?.to_string();
    let encoding = if ext == "csv" { csv_encoding(p, "file.exportCsv")? } else { None };
    let d = s.doc()?;
    let mut wb = (*d.wb).clone();
    if encoding.is_some() {
        wb.csv_encoding = encoding;
    }
    if p.get("sheet").is_some() {
        wb.set_active_sheet(target_sheet(s, p)?);
    }
    let target = if path.ends_with(&format!(".{ext}")) { path.clone() } else { format!("{path}.{ext}") };
    let bytes = if ext == "html" {
        let title = std::path::Path::new(&d.display_title()).file_stem().and_then(|x| x.to_str()).unwrap_or("Book").to_owned();
        web_page(&wb, p, &title, "file.exportHtml")?
    } else {
        crate::io::save_bytes(&wb, &target)?
    };
    crate::io::write_file(&target, &bytes)?;
    Ok(json!({"path": target, "bytes": bytes.len()}))
}

/// A workbook as a web page: its active sheet, or with `sheets: all` every visible sheet behind a
/// menu of their names.
fn web_page(wb: &monun_model::Workbook, p: &Json, title: &str, id: &str) -> Result<Vec<u8>> {
    let sheets: Vec<usize> = match str_param(p, "sheets") {
        None | Some("active") => vec![wb.active_sheet()],
        Some("all") | Some("workbook") => {
            (0..wb.sheets.len()).filter(|i| wb.sheet(*i).is_some_and(|s| s.visibility == monun_model::Visibility::Visible)).collect()
        }
        Some(other) => return Err(bad(id, format!("`sheets` must be active or all, not `{other}`"))),
    };
    let fit = bool_param(p, "fitWidth").unwrap_or(false);
    Ok(crate::io::workbook_html(wb, &sheets, title, fit)?.into_bytes())
}

fn properties(s: &mut Session, p: &Json) -> Result<Json> {
    edit(s, |cx| {
        let pr = &mut cx.wb.props;
        for (k, slot) in [
            ("title", &mut pr.title),
            ("subject", &mut pr.subject),
            ("author", &mut pr.author),
            ("company", &mut pr.company),
            ("keywords", &mut pr.keywords),
            ("description", &mut pr.description),
        ] {
            if let Some(v) = str_param(p, k) {
                *slot = v.to_string();
            }
        }
        Ok(serde_json::to_value(&cx.wb.props).unwrap_or_default())
    })
}

/// File › Options: reports the workbook-level settings and asks the interface to show its Options
/// page. Nothing changes here; every setting has its own command.
fn options(s: &mut Session, _: &Json) -> Result<Json> {
    s.ui_requests.push(crate::UiRequest::Dialog("options".into(), json!({})));
    let calculation = s.active().map(|d| {
        let c = &d.wb.calc;
        let mode = match c.mode {
            monun_model::CalcMode::Automatic => "automatic",
            monun_model::CalcMode::AutomaticExceptTables => "automaticExceptTables",
            monun_model::CalcMode::Manual => "manual",
        };
        json!({"mode": mode, "iterative": c.iterative, "maxIterations": c.max_iterations, "maxChange": c.max_change})
    });
    Ok(json!({"calculation": calculation}))
}

fn activate_window(s: &mut Session, p: &Json) -> Result<Json> {
    let id = match u32_param(p, "index").filter(|_| p.get("window").is_none_or(Json::is_null) && p.get("doc").is_none_or(Json::is_null)) {
        Some(i) => {
            let uid = s.documents().get(i as usize).map(|d| d.uid).ok_or_else(|| bad("window.activate", "no such window"))?;
            s.doc_window(uid).ok_or_else(|| bad("window.activate", "no such window"))?
        }
        None if p.get("window").is_none_or(Json::is_null) && p.get("doc").is_none_or(Json::is_null) => {
            return Err(bad("window.activate", "missing `window`"));
        }
        None => super::extra::window_param(s, "window.activate", p)?,
    };
    s.unhide_window(id);
    ok()
}

fn close_window(s: &mut Session, p: &Json) -> Result<Json> {
    let id = super::extra::window_param(s, "window.close", p)?;
    let doc = s.window(id).map(|w| w.doc_uid).unwrap_or_default();
    if s.windows().iter().filter(|w| w.doc_uid == doc).count() > 1 {
        s.close_window(id);
        return Ok(json!({"closed": true, "document": false}));
    }
    let force = bool_param(p, "force").unwrap_or(false);
    let d = s.documents().iter().find(|d| d.uid == doc).ok_or(crate::EngineError::NoDocument)?;
    if d.is_dirty() && !force {
        let title = d.display_title();
        s.activate_window(id);
        s.ui_requests.push(crate::UiRequest::Dialog("saveChanges".into(), json!({"title": title})));
        return Ok(json!({"closed": false, "dirty": true}));
    }
    s.close_window(id);
    Ok(json!({"closed": true, "document": true}))
}
