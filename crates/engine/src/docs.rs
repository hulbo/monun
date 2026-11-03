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
use monun_model::{CellStore, DefinedName, Document, Sheet, Workbook};

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

/// A workbook of one document tab holding a Markdown file, named after it.
pub fn markdown_workbook(name: &str, bytes: &[u8]) -> Workbook {
    let mut wb = Workbook::new();
    let tab = name_from_path(name).unwrap_or_else(|| "Doc1".into());
    wb.sheets = vec![Arc::new(Sheet::new_document(tab, markdown_text(bytes)))];
    wb.set_active_sheet(0);
    wb
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
            formula: format!("\"{TAG} {}\"", lines.len()),
            comment: String::new(),
            hidden: true,
        });
    }
    Some(out)
}

/// Turns the marked worksheets of a file just read back into document tabs, and takes the marks
/// out of the names. A document that arrived with cells (a hand-made JSON file) loses them.
pub fn load(wb: &mut Workbook) {
    let marks: Vec<(usize, usize)> = wb.names.iter().filter(|n| is_mark(n)).filter_map(|n| Some((n.scope?, line_count(&n.formula)))).collect();
    if !marks.is_empty() {
        wb.names.retain(|n| !is_mark(n));
    }
    for (i, count) in marks {
        let Some(sh) = wb.sheet(i) else { continue };
        if sh.is_document() {
            continue;
        }
        let text = text_of(wb, sh, count);
        if let Some(sh) = wb.sheet_mut(i) {
            clear_grid(sh);
            sh.document = Some(Document { text });
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
