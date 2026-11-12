//! Document tabs in spreadsheet files.
//!
//! A document tab ([`Sheet::document`]) is written to XLSX, CSV and HTML as a worksheet holding
//! its Markdown text one line per row of column A, so Excel and other spreadsheet apps show the
//! text and keep it through their own saves. A hidden name scoped to that sheet, [`MARK`], tells
//! Monun the worksheet is a document, and opening the file turns it back into one. A line longer
//! than a cell holds continues in the next columns of its row.

use std::collections::BTreeMap;
use std::sync::Arc;

use monun_core::{CellRef, MAX_ROWS, Value};
use monun_model::{CellStore, DefinedName, Document, Sheet, TextEncoding, TextFile, Workbook};

/// The hidden sheet-scope name that marks a document tab. `_gc_` names are Monun's own (see
/// `cmd::stash`).
pub const MARK: &str = "_gc_doc";
/// The first word of the mark's text: the format of the stored document. The second word is its
/// line count, so blank lines at the end survive.
const TAG: &str = "md1";
/// The most characters (UTF-16 code units) Excel keeps in a cell.
const CELL_LIMIT: usize = 32_767;
/// Column A's width in a saved document (pixels at 96 dpi), about a hundred characters.
const TEXT_WIDTH: f32 = 720.0;

/// Refused when Markdown is asked of a worksheet.
pub const NOT_A_DOCUMENT: &str = "Only a document tab can be saved as Markdown (.md). Switch to the document first.";

/// Refused when a command would put cells on a document tab.
pub const NO_CELLS: &str = "A document has no cells. Switch to a worksheet to edit cells.";

/// Refused, with workbooks and Markdown files apart ([`crate::Session::separate_files`]), when a
/// document tab would go into a workbook.
pub const SHEETS_ONLY: &str = "A workbook holds sheets only. Make a Markdown file with + › New Markdown Document.";

/// Refused, with workbooks and Markdown files apart, when a sheet or a second document would go
/// into a Markdown or text file.
pub const ONE_DOCUMENT: &str = "A Markdown or text file holds its text only. Make an Excel workbook with + › New Excel Workbook for sheets.";

/// Refused when a workbook would be saved as SQL.
pub const NOT_TEXT: &str = "Only a text file can be saved as .sql. Save a workbook as .xlsx, or as .txt for its sheet's values.";

/// A text file rather than a workbook: one document tab and nothing else (a `.md`, `.sql` or
/// `.txt` file opened, or `file.new` with `document`), saved as its text. What the text is
/// ([`TextKind`]) goes by the file's name.
pub fn is_text_file(wb: &Workbook) -> bool {
    wb.sheets.len() == 1 && wb.sheets.first().is_some_and(|sh| sh.is_document())
}

/// What a text file holds, by its name's extension: Markdown (rendered beside its text), SQL
/// (coloured) or plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextKind {
    Markdown,
    Sql,
    Plain,
    /// A PDF file, shown read-only by the interface: its document tab holds no text.
    Pdf,
}

impl TextKind {
    /// The kind of the file called `name`: `.md` and `.markdown` are Markdown, `.sql` SQL, the
    /// rest plain text.
    pub fn of_name(name: &str) -> TextKind {
        let ext = std::path::Path::new(name).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("md" | "markdown") => TextKind::Markdown,
            Some("sql") => TextKind::Sql,
            Some("pdf") => TextKind::Pdf,
            _ => TextKind::Plain,
        }
    }

    /// The extension a new file of the kind gets.
    pub fn extension(self) -> &'static str {
        match self {
            TextKind::Markdown => "md",
            TextKind::Sql => "sql",
            TextKind::Plain => "txt",
            TextKind::Pdf => "pdf",
        }
    }

    /// The kind `file.new` names: `markdown`, `sql` or `text`.
    pub fn parse(s: &str) -> Option<TextKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Some(TextKind::Markdown),
            "sql" => Some(TextKind::Sql),
            "text" | "txt" | "plain" => Some(TextKind::Plain),
            _ => None,
        }
    }
}

/// A text file's text, line breaks as `\n`, and how it was written: a byte-order mark says UTF-8
/// (or UTF-16, written back as UTF-8); without one, valid UTF-8 is UTF-8, and other bytes are read
/// as code page 949 when they read as Korean (as a CSV is).
pub fn decode_text_file(bytes: &[u8]) -> (String, TextFile) {
    let (text, encoding) = match std::str::from_utf8(bytes) {
        Ok(s) if !s.starts_with('\u{feff}') => (s.to_owned(), TextEncoding::Utf8),
        _ => monun_xlsx::decode_text_with_encoding(bytes),
    };
    // The line break most lines end in.
    let breaks = text.matches('\n').count();
    let crlf = breaks > 0 && text.matches("\r\n").count() * 2 > breaks;
    (normalize(&text), TextFile { encoding, crlf })
}

/// A text file's bytes: its text with the line breaks and in the encoding it was read with
/// ([`Document::file`]); a new file's in UTF-8 with `\n`.
pub fn text_file_bytes(doc: &Document) -> Vec<u8> {
    let file = doc.file.unwrap_or(TextFile { encoding: TextEncoding::Utf8, crlf: false });
    let text = if file.crlf { doc.text.replace('\n', "\r\n") } else { doc.text.clone() };
    monun_xlsx::encode_text(text.into_bytes(), file.encoding)
}

/// Refused for every change to a PDF file and for saving, exporting or printing it.
pub const PDF_READ_ONLY: &str = "A PDF file can only be read in Monun.";

/// Whether `bytes` are a PDF file: `%PDF-` within its first kilobyte, where readers look for it.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.get(..bytes.len().min(1024)).is_some_and(|head| head.windows(5).any(|w| w == b"%PDF-"))
}

/// A workbook of one document tab standing for a PDF file, named after it: no text, shown read
/// (View) and as wide as the window (Fit Width) until its tab is set otherwise. The interface draws
/// the file's pages in it.
pub fn pdf_workbook(name: &str) -> Workbook {
    let mut wb = Workbook::new();
    let tab = name_from_path(name).unwrap_or_else(|| "PDF1".into());
    let mut sheet = Sheet::new_document(tab, String::new());
    if let Some(doc) = sheet.document.as_mut() {
        doc.mode = Some(monun_model::DocMode::View);
        doc.fit_width = true;
    }
    wb.sheets = vec![Arc::new(sheet)];
    wb.set_active_sheet(0);
    wb
}

/// A workbook of one document tab holding a text file (.sql, .txt), named after it, that saves
/// back in the encoding and with the line breaks it came in.
pub fn text_workbook(name: &str, bytes: &[u8]) -> Workbook {
    let (text, file) = decode_text_file(bytes);
    let mut wb = Workbook::new();
    let tab = name_from_path(name).unwrap_or_else(|| "Text1".into());
    let mut sheet = Sheet::new_document(tab, text);
    if let Some(doc) = sheet.document.as_mut() {
        doc.file = Some(file);
    }
    wb.sheets = vec![Arc::new(sheet)];
    wb.set_active_sheet(0);
    wb
}

/// The text of a Markdown file: UTF-8 (a byte-order mark dropped), anything else read as well as
/// it goes.
pub fn markdown_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    normalize(&String::from_utf8_lossy(bytes))
}

/// A tab name from a file's name: its stem without the characters a sheet name can't have, at
/// most 31 characters; `None` when nothing usable is left.
pub fn name_from_path(path: &str) -> Option<String> {
    let stem = std::path::Path::new(path).file_stem().and_then(std::ffi::OsStr::to_str)?;
    let clean: String = stem.chars().filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']')).take(31).collect();
    let clean = clean.trim_matches('\'').trim().to_string();
    (!clean.is_empty() && !clean.eq_ignore_ascii_case("History")).then_some(clean)
}

/// A workbook of one document tab holding a Markdown file, named after it. The pictures its text
/// shows from files beside it (`![…](pictures/a.png)`) come along, to show and save with it.
pub fn markdown_workbook(name: &str, bytes: &[u8]) -> Workbook {
    let mut wb = Workbook::new();
    let tab = name_from_path(name).unwrap_or_else(|| "Doc1".into());
    let mut sheet = Sheet::new_document(tab, markdown_text(bytes));
    // Only a file read from a folder has pictures beside it.
    let folder = std::path::Path::new(name).parent().filter(|f| !f.as_os_str().is_empty());
    if let Some(folder) = folder {
        let text = sheet.document.as_ref().map(|d| d.text.clone()).unwrap_or_default();
        for link in image_links(&text) {
            let Some(path) = local_path(&link) else { continue };
            let file = folder.join(&path);
            let small = std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() <= PICTURE_LIMIT);
            if let Some(data) = std::fs::read(&file).ok().filter(|_| small) {
                add_picture(&mut sheet, &link, data);
            }
        }
    }
    wb.sheets = vec![Arc::new(sheet)];
    wb.set_active_sheet(0);
    wb
}

/// The largest picture file read beside a Markdown file.
const PICTURE_LIMIT: u64 = 32 * 1024 * 1024;

/// Refused for a picture that is neither PNG nor JPEG.
pub const NOT_A_PICTURE: &str = "Only PNG and JPEG pictures can be put in a document.";

/// The links of the text's pictures (`![…](link)`), each once, in order.
pub fn image_links(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        let Some(after) = rest.get(start + 2..) else { break };
        let Some(close) = after.find("](") else { break };
        let Some(target) = after.get(close + 2..) else { break };
        let Some(end) = target.find(')') else { break };
        let link = target.get(..end).unwrap_or_default().trim();
        let link = link.strip_prefix('<').and_then(|l| l.strip_suffix('>')).unwrap_or(link);
        if !link.is_empty() && !link.contains('\n') && !links.iter().any(|l| l == link) {
            links.push(link.to_string());
        }
        rest = target.get(end..).unwrap_or_default();
    }
    links
}

/// The file a picture link names, `%20` and the like decoded; `None` for a web address, a
/// `data:` picture or a document tab's own `image:N`.
pub fn local_path(link: &str) -> Option<String> {
    let link = link.split_once(' ').map_or(link, |(path, _title)| path);
    if link.contains("://") || link.starts_with("image:") || link.starts_with("data:") || link.starts_with("mailto:") {
        return None;
    }
    let bytes = link.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| bytes.get(at).and_then(|c| (*c as char).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().filter(|p| !p.is_empty())
}

/// Adds a picture to a document tab as the file `link` names; `None` when the bytes are no PNG
/// or JPEG. Returns the picture's id.
pub fn add_picture(sheet: &mut Sheet, link: &str, data: Vec<u8>) -> Option<u32> {
    let (w, h) = crate::io::image_size(&data)?;
    let mime = if data.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
    let id = sheet.images.iter().map(|p| p.id).max().map_or(1, |m| m.saturating_add(1));
    let anchor = monun_model::Anchor { width: w as f32, height: h as f32, ..monun_model::Anchor::default() };
    sheet.document.as_mut()?.files.push((link.to_string(), id));
    sheet.images.push(monun_model::Image { id, anchor, data: Arc::new(data), mime: mime.into(), alt: link.to_string() });
    Some(id)
}

/// The picture a document's `![…](link)` shows: `image:N` names one of the tab's pictures, and in a
/// Markdown file any other link one of the files beside it.
pub fn picture<'a>(sheet: &'a Sheet, link: &str) -> Option<&'a monun_model::Image> {
    let link = link.trim();
    let id: u32 = match link.strip_prefix("image:") {
        Some(n) => n.trim().parse().ok()?,
        None => sheet.document.as_ref()?.files.iter().find(|(l, _)| l == link)?.1,
    };
    sheet.images.iter().find(|p| p.id == id)
}

/// Writes a Markdown file's pictures beside `path` (the .md saved): each one its text still shows
/// and that isn't there yet, so a Save As elsewhere takes its pictures along.
pub fn write_pictures(sheet: &Sheet, path: &str) -> Result<(), String> {
    let Some(doc) = sheet.document.as_ref() else { return Ok(()) };
    let Some(folder) = std::path::Path::new(path).parent() else { return Ok(()) };
    let shown = image_links(&doc.text);
    for (link, id) in &doc.files {
        let (Some(file), true) = (local_path(link), shown.contains(link)) else { continue };
        let Some(image) = sheet.images.iter().find(|p| p.id == *id) else { continue };
        let target = folder.join(file);
        if target.exists() {
            continue;
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&target, image.data.as_slice()).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

/// What a document's `{{…}}` field shows: a cell reference (`구매목록!D1`) as the cell displays
/// it, in its number format; anything else (`=SUM(구매목록!B:B)`) worked out as a formula and shown
/// in General format. An error comes back as its code (`#REF!`).
///
/// A Markdown file has no cells to show: its fields read as they are written, `{{…}}` and all.
pub fn field_value(wb: &Workbook, expr: &str) -> Result<String, String> {
    if is_text_file(wb) {
        return Ok(format!("{{{{{expr}}}}}"));
    }
    let e = expr.trim();
    let body = e.strip_prefix('=').unwrap_or(e);
    if !e.starts_with('=') {
        let (sheet, a1) = monun_core::split_sheet(body);
        let index = match &sheet {
            Some(name) => wb.sheet_index(name),
            None => Some(wb.active_sheet()),
        };
        if let Some(sh) = index.and_then(|i| wb.sheet(i))
            && let Some(c) = CellRef::parse(&a1.replace('$', ""))
        {
            return Ok(crate::display::cell_text(wb, sh, c));
        }
    }
    match monun_calc::recalc::evaluate(wb, wb.active_sheet(), CellRef::default(), body) {
        Value::Error(e) => Err(e.to_string()),
        v => Ok(crate::display::format(&v, "General", wb).text),
    }
}

/// Whether a document tab's grid is as it must be: empty.
pub fn grid_is_empty(sh: &Sheet) -> bool {
    sh.cells.is_empty()
}

/// Line breaks as `\n`, the only kind a document holds.
pub fn normalize(text: &str) -> String {
    if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.to_owned() }
}

/// A copy of `wb` with every document tab written out as a worksheet of lines plus its mark, or
/// `None` when the workbook has no documents.
pub fn with_cells(wb: &Workbook) -> Option<Workbook> {
    if !wb.sheets.iter().any(|sh| sh.is_document()) {
        return None;
    }
    let mut out = wb.clone();
    out.names.retain(|n| !is_mark(n));
    for i in 0..out.sheets.len() {
        let Some(sh) = out.sheet_mut(i) else { continue };
        let Some(doc) = sh.document.take() else { continue };
        let max = usize::try_from(MAX_ROWS).unwrap_or(usize::MAX);
        let lines: Vec<&str> = doc.text.split('\n').take(max).collect();
        for (row, line) in lines.iter().enumerate() {
            let Ok(row) = u32::try_from(row) else { break };
            for (col, chunk) in chunks(line).enumerate() {
                let Ok(col) = u32::try_from(col) else { break };
                sh.set_value(CellRef::new(row, col), Value::Text(Arc::from(chunk)));
            }
        }
        sh.cols.entry(0).or_default().size = Some(TEXT_WIDTH);
        out.names.push(DefinedName {
            name: MARK.into(),
            scope: Some(i),
            formula: format!("\"{}\"", mark_text(lines.len(), &doc)),
            comment: String::new(),
            hidden: true,
        });
    }
    Some(out)
}

/// Turns the marked worksheets of a file just read back into document tabs, and takes the marks
/// out of the names. A document that arrived with cells (a hand-made JSON file) loses them.
pub fn load(wb: &mut Workbook) {
    let marks: Vec<(usize, usize, Document)> =
        wb.names.iter().filter(|n| is_mark(n)).filter_map(|n| Some((n.scope?, line_count(&n.formula), view_of(&n.formula)))).collect();
    if !marks.is_empty() {
        wb.names.retain(|n| !is_mark(n));
    }
    for (i, count, view) in marks {
        let Some(sh) = wb.sheet(i) else { continue };
        if sh.is_document() {
            continue;
        }
        let text = text_of(wb, sh, count);
        if let Some(sh) = wb.sheet_mut(i) {
            clear_grid(sh);
            sh.document = Some(Document { text, ..view });
        }
    }
    for i in 0..wb.sheets.len() {
        if wb.sheet(i).is_some_and(|sh| sh.is_document() && !grid_is_empty(sh))
            && let Some(sh) = wb.sheet_mut(i)
        {
            clear_grid(sh);
        }
    }
}

fn is_mark(n: &DefinedName) -> bool {
    n.scope.is_some() && n.name.eq_ignore_ascii_case(MARK)
}

/// A mark's text: the format, the line count, then how the tab shows when that was chosen (its
/// mode, and `fit` for Fit Width), as in `md1 12 split fit`.
fn mark_text(lines: usize, doc: &Document) -> String {
    let mut text = format!("{TAG} {lines}");
    if let Some(mode) = doc.mode {
        text.push(' ');
        text.push_str(mode.name());
    }
    if doc.fit_width {
        text.push_str(" fit");
    }
    text
}

/// How a mark's text says the tab shows (an older mark says nothing: the defaults).
fn view_of(formula: &str) -> Document {
    let text = formula.trim().trim_matches('"');
    let words = || text.split_whitespace().skip(2);
    Document {
        text: String::new(),
        mode: words().find_map(monun_model::DocMode::parse),
        fit_width: words().any(|w| w.eq_ignore_ascii_case("fit")),
        ..Document::default()
    }
}

/// The line count a mark's text (`"md1 12"`) records; 0 when it records none.
fn line_count(formula: &str) -> usize {
    let text = formula.trim().trim_matches('"');
    let max = usize::try_from(MAX_ROWS).unwrap_or(usize::MAX);
    text.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(max)
}

/// A marked worksheet's lines: each row's cells as displayed, left to right, and at least
/// `count` lines.
fn text_of(wb: &Workbook, sh: &Sheet, count: usize) -> String {
    let mut rows: BTreeMap<u32, Vec<(u32, CellRef)>> = BTreeMap::new();
    for (c, _) in sh.cells.iter() {
        rows.entry(c.row).or_default().push((c.col, c));
    }
    let last = rows.keys().next_back().map_or(0, |r| usize::try_from(*r).map_or(0, |r| r + 1));
    let mut lines = vec![String::new(); count.max(last)];
    for (row, mut cells) in rows {
        cells.sort_unstable_by_key(|(col, _)| *col);
        let Some(line) = usize::try_from(row).ok().and_then(|r| lines.get_mut(r)) else { continue };
        for (_, c) in cells {
            line.push_str(&crate::display::cell_text(wb, sh, c));
        }
    }
    lines.join("\n")
}

fn clear_grid(sh: &mut Sheet) {
    sh.cells = CellStore::new();
    sh.cell_pictures.clear();
    sh.cols.clear();
    sh.rows.clear();
    sh.merges.clear();
}

/// `line` cut into pieces a cell holds.
fn chunks(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = line;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut units = 0;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            units += c.len_utf16();
            if units > CELL_LIMIT {
                end = i;
                break;
            }
        }
        let (head, tail) = rest.split_at_checked(end)?;
        rest = tail;
        Some(head)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut_at_the_cell_limit() {
        let line = "가".repeat(CELL_LIMIT + 5);
        let parts: Vec<&str> = chunks(&line).collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].chars().count(), CELL_LIMIT);
        assert_eq!(parts[1].chars().count(), 5);
    }

    #[test]
    fn the_mark_records_the_line_count() {
        assert_eq!(line_count("\"md1 12\""), 12);
        assert_eq!(line_count("\"md1\""), 0);
        assert_eq!(line_count("junk"), 0);
    }
}
