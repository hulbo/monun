//! The document editor: Markdown text with its marks coloured, the formatting buttons' edits, and
//! lists that continue on Enter. It is a plain `TextEdit`, so Korean input (IME) and the editor's
//! own undo work as in every other text field; the edits here are pure functions on char indices.

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Ui};

use super::editor_id;
use crate::SheetApp;
use crate::clock::now_ms;
use crate::theme::{self, Tokens};

/// An edit of the formatting buttons and shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Heading(u8),
    Bold,
    Italic,
    Strike,
    Code,
    Bullet,
    Number,
    Task,
    Quote,
    Table,
    Link,
    /// A `{{…}}` field showing a cell's value.
    Field,
    /// A block inserted below the current line (a diagram's template).
    Snippet(&'static str),
}

/// The tool bar's formatting buttons: the edit, the label, the tooltip.
pub const BUTTONS: &[(Format, &str, &str)] = &[
    (Format::Heading(1), "H1", "Heading 1"),
    (Format::Heading(2), "H2", "Heading 2"),
    (Format::Heading(3), "H3", "Heading 3"),
    (Format::Bold, "B", "Bold (Ctrl+B)"),
    (Format::Italic, "I", "Italic (Ctrl+I)"),
    (Format::Strike, "S", "Strikethrough"),
    (Format::Bullet, "• List", "Bulleted list"),
    (Format::Number, "1. List", "Numbered list"),
    (Format::Task, "Task", "Task list (click a box in the document to tick it)"),
    (Format::Quote, "Quote", "Quote"),
    (Format::Code, "Code", "Code"),
    (Format::Table, "Table", "Insert a table"),
    (Format::Link, "Link", "Link to a web page, or to a cell as #Sheet1!A1"),
    (Format::Field, "Cell value", "Show a cell's value: {{Sheet1!A1}}, or a formula's: {{=SUM(Sheet1!B:B)}}"),
];

/// Where a new group of [`BUTTONS`] starts (a separator goes before it).
pub const GROUP_STARTS: &[usize] = &[3, 6, 10];

/// Diagram templates of the tool bar's Diagram menu (Mermaid).
pub const DIAGRAMS: &[(&str, &str)] = &[
    ("Flowchart", "```mermaid\nflowchart TD\n    A[시작] --> B{확인}\n    B -->|예| C[처리]\n    B -->|아니오| D[다시 보기]\n    D --> B\n```"),
    (
        "Sequence diagram",
        "```mermaid\nsequenceDiagram\n    participant 나\n    participant 증권사\n    나->>증권사: 매수 주문\n    증권사-->>나: 체결 알림\n```",
    ),
    (
        "Gantt chart",
        "```mermaid\ngantt\n    title 일정\n    dateFormat YYYY-MM-DD\n    section 준비\n    자료 정리 :a1, 2026-11-03, 3d\n    검토 :after a1, 2d\n```",
    ),
    ("State diagram", "```mermaid\nstateDiagram-v2\n    [*] --> 주문\n    주문 --> 체결\n    체결 --> 정산\n    정산 --> [*]\n```"),
    ("Class diagram", "```mermaid\nclassDiagram\n    class Account {\n        +balance\n        +deposit()\n    }\n    Account <|-- Savings\n```"),
    ("Entity relationship", "```mermaid\nerDiagram\n    CUSTOMER ||--o{ ORDER : places\n    ORDER ||--|{ LINE : contains\n```"),
    ("Mind map", "```mermaid\nmindmap\n  root((가계부))\n    수입\n      월급\n    지출\n      식비\n      주식\n```"),
    ("Pie chart", "```mermaid\npie title 지출\n    \"식비\" : 40\n    \"주거\" : 35\n    \"주식\" : 25\n```"),
    ("Timeline", "```mermaid\ntimeline\n    title 진행\n    2025 : 시작\n    2026 : 확장\n```"),
];

const TABLE: &str = "| 항목 | 금액 |\n| --- | ---: |\n|  |  |\n|  |  |";

/// Applies a formatting edit to the draft at the editor's selection, and gives the editor focus.
pub(super) fn apply(app: &mut SheetApp, ctx: &egui::Context, f: Format) {
    let Some(dr) = app.doc_view.draft.as_mut() else { return };
    let id = editor_id();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let count = dr.text.chars().count();
    let sel = state.cursor.char_range().map_or((count, count), |r| (r.secondary.index.into(), r.primary.index.into()));
    let (a, b) = format(&mut dr.text, sel, f);
    dr.changed_at = Some(now_ms());
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    state.store(ctx, id);
    app.doc_view.focus_editor = true;
}

/// Puts `text` at the editor's caret, in place of the selection, and gives the editor focus.
pub(super) fn insert_at_caret(app: &mut SheetApp, ctx: &egui::Context, text: &str) {
    let Some(dr) = app.doc_view.draft.as_mut() else { return };
    let id = editor_id();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let count = dr.text.chars().count();
    let (a, b) = state.cursor.char_range().map_or((count, count), |r| {
        let (x, y): (usize, usize) = (r.secondary.index.into(), r.primary.index.into());
        (x.min(y).min(count), x.max(y).min(count))
    });
    replace_chars(&mut dr.text, a, b, text);
    dr.changed_at = Some(now_ms());
    let caret = a + text.chars().count();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    state.store(ctx, id);
    app.doc_view.focus_editor = true;
}

/// The line of the editor's caret.
pub(super) fn caret_line(app: &SheetApp, ctx: &egui::Context) -> usize {
    let Some(dr) = app.doc_view.draft.as_ref() else { return 0 };
    let caret = egui::TextEdit::load_state(ctx, editor_id()).and_then(|s| s.cursor.char_range()).map_or(0, |r| r.primary.index.into());
    monun_markdown::line_of(&dr.text, byte_index(&dr.text, caret))
}

/// The editor, filling `ui`. Returns the first line in view.
pub(super) fn show(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens) -> Option<usize> {
    let goto = app.doc_view.goto_line.take();
    let select = app.doc_view.select.take();
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
    let ctx = ui.ctx().clone();
    if app.doc_view.focus_editor && !ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
        app.doc_view.focus_editor = false;
        ctx.memory_mut(|m| m.request_focus(editor_id()));
    }
    let dr = app.doc_view.draft.as_mut()?;
    let font = FontId::new(14.0 * z, FontFamily::Name(theme::MONO.into()));
    // A line to go to (the outline, a double-click) or a found match to select.
    let goto = goto.map(|line| char_index(&dr.text, monun_markdown::line_start(&dr.text, line))).map(|at| (at, at)).or(select);
    if let Some((a, b)) = goto {
        let mut state = egui::TextEdit::load_state(&ctx, editor_id()).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(&ctx, editor_id());
    }
    let colors = Colors::of(t);
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = markdown_job(buf.as_str(), &font, &colors, wrap);
        tint(&mut job, &super::find_all(buf.as_str(), &query), super::preview::FOUND);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let rows = ((ui.available_height() / (font.size * 1.35)).floor() as usize).max(4);
    let scroll = egui::ScrollArea::vertical().id_salt("monun.doc_editor_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let out = egui::TextEdit::multiline(&mut dr.text)
            .id(editor_id())
            .font(font.clone())
            .frame(egui::Frame::NONE.inner_margin(14))
            .margin(egui::Margin::same(14))
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        if let Some((at, _)) = goto {
            let r = out.galley.pos_from_cursor(CCursor::new(at)).translate(out.galley_pos.to_vec2());
            ui.scroll_to_rect(r.expand(40.0), Some(egui::Align::Center));
        }
        out
    });
    let viewport_top = scroll.inner_rect.top();
    let out = scroll.inner;
    let top_char: usize = out.galley.cursor_from_pos(egui::vec2(0.0, (viewport_top - out.galley_pos.y).max(0.0))).index.into();
    let top = monun_markdown::line_of(&dr.text, byte_index(&dr.text, top_char));
    out.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "Document text"));
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none());
    if out.response.changed() {
        dr.changed_at = Some(now_ms());
        if enter
            && let Some(range) = out.cursor_range
            && let Some(caret) = continue_list(&mut dr.text, range.primary.index.into())
        {
            let mut state = out.state.clone();
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
            state.store(&ctx, editor_id());
        }
    }
    if out.response.lost_focus()
        && let Err(e) = app.commit_doc_draft()
    {
        app.alert(e);
    }
    Some(top)
}

/// Gives the byte ranges `ranges` of a laid-out text the background `color`, splitting sections
/// where they start and end.
pub fn tint(job: &mut LayoutJob, ranges: &[std::ops::Range<usize>], color: Color32) {
    if ranges.is_empty() {
        return;
    }
    use egui::text::{ByteIndex, LayoutSection};
    let mut out = Vec::with_capacity(job.sections.len() + ranges.len() * 2);
    for s in job.sections.drain(..) {
        let (start, end) = (s.byte_range.start.0, s.byte_range.end.0);
        let lead = |at: usize| if at == start { s.leading_space } else { 0.0 };
        let mut at = start;
        for r in ranges.iter().filter(|r| r.start < end && r.end > start) {
            let (a, b) = (r.start.max(start), r.end.min(end));
            if a > at {
                out.push(LayoutSection { byte_range: ByteIndex(at)..ByteIndex(a), leading_space: lead(at), format: s.format.clone() });
            }
            let format = TextFormat { background: color, ..s.format.clone() };
            out.push(LayoutSection { byte_range: ByteIndex(a)..ByteIndex(b), leading_space: lead(a), format });
            at = b;
        }
        if at < end {
            out.push(LayoutSection { byte_range: ByteIndex(at)..ByteIndex(end), leading_space: lead(at), format: s.format.clone() });
        }
    }
    job.sections = out;
}

/// The colours of the editor's Markdown marks.
pub struct Colors {
    text: Color32,
    heading: Color32,
    mark: Color32,
    code: Color32,
    quote: Color32,
    link: Color32,
    field: Color32,
}

impl Colors {
    fn of(t: &Tokens) -> Colors {
        Colors {
            text: t.text,
            heading: t.syntax.keyword,
            mark: t.syntax.number,
            code: t.syntax.string,
            quote: t.syntax.comment,
            link: t.syntax.builtin,
            field: t.accent,
        }
    }
}

/// A layout job of `text` with its Markdown marks coloured: headings, list marks, quotes, code,
/// links and `{{cell}}` fields. Covers every byte of `text`, as the editor requires.
pub fn markdown_job(text: &str, font: &FontId, c: &Colors, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let put = |job: &mut LayoutJob, s: &str, color: Color32| {
        if !s.is_empty() {
            job.append(s, 0.0, TextFormat::simple(font.clone(), color));
        }
    };
    let mut fence = false;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let lead = body.trim_start();
        if lead.starts_with("```") || lead.starts_with("~~~") {
            put(&mut job, line, c.code);
            fence = !fence;
        } else if fence {
            put(&mut job, line, c.code);
        } else if lead.starts_with('#') && lead.trim_start_matches('#').starts_with([' ', '\t']) {
            put(&mut job, line, c.heading);
        } else if lead.starts_with('>') {
            put(&mut job, line, c.quote);
        } else {
            let mark = list_mark(body).map_or(0, |m| m.len);
            let (head, rest) = line.split_at_checked(mark).unwrap_or(("", line));
            put(&mut job, head, c.mark);
            inline(&mut job, rest, font, c);
        }
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(font.clone(), c.text));
    }
    job.wrap.max_width = wrap;
    job
}

/// Colours a line's inline marks: `code`, [links](…), {{fields}}, and the ** and ~~ marks.
fn inline(job: &mut LayoutJob, s: &str, font: &FontId, c: &Colors) {
    let fmt = |color| TextFormat::simple(font.clone(), color);
    let mut plain_from = 0;
    let mut i = 0;
    while let Some(rest) = s.get(i..) {
        let Some(ch) = rest.chars().next() else { break };
        let span = if rest.starts_with('`') {
            rest.get(1..).and_then(|r| r.find('`')).map(|e| (e + 2, c.code))
        } else if rest.starts_with("{{") {
            rest.find("}}").map(|e| (e + 2, c.field))
        } else if rest.starts_with('[') {
            rest.find("](").and_then(|m| rest.get(m..).and_then(|r| r.find(')')).map(|e| (m + e + 1, c.link)))
        } else if rest.starts_with("**") || rest.starts_with("~~") {
            Some((2, c.mark))
        } else {
            None
        };
        match span {
            Some((len, color)) => {
                if let Some(plain) = s.get(plain_from..i) {
                    job.append(plain, 0.0, fmt(c.text));
                }
                if let Some(marked) = s.get(i..i + len) {
                    job.append(marked, 0.0, fmt(color));
                }
                i += len;
                plain_from = i;
            }
            None => i += ch.len_utf8(),
        }
    }
    if let Some(plain) = s.get(plain_from..)
        && !plain.is_empty()
    {
        job.append(plain, 0.0, fmt(c.text));
    }
}

/// A list or quote mark at the start of a line: its length in bytes (indentation included) and
/// the mark the next line continues with.
#[derive(Debug, PartialEq, Eq)]
struct ListMark {
    len: usize,
    next: String,
}

fn list_mark(line: &str) -> Option<ListMark> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (pad, rest) = line.split_at_checked(indent)?;
    let (mark, next) = if rest.starts_with("> ") {
        (2, format!("{pad}> "))
    } else if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        let bullet = rest.get(..2).unwrap_or("- ");
        (2, format!("{pad}{bullet}"))
    } else {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let after = rest.get(digits..)?;
        if digits == 0 || digits > 9 || !(after.starts_with(". ") || after.starts_with(") ")) {
            return None;
        }
        let n: u64 = rest.get(..digits)?.parse().ok()?;
        let sep = after.get(..1).unwrap_or(".");
        (digits + 2, format!("{pad}{}{sep} ", n + 1))
    };
    let mut len = indent + mark;
    let mut next = next;
    // A task item continues as an open task.
    let tail = line.get(len..).unwrap_or("");
    if ["[ ] ", "[x] ", "[X] "].iter().any(|b| tail.starts_with(b)) {
        len += 4;
        next.push_str("[ ] ");
    }
    Some(ListMark { len, next })
}

/// After Enter at char `caret` (just past the new line break): continues the list or quote of the
/// line above, or ends it when that line held only its mark (the mark goes). Returns the caret.
pub fn continue_list(text: &mut String, caret: usize) -> Option<usize> {
    let b = byte_index(text, caret);
    let before = text.get(..b)?;
    let above = before.strip_suffix('\n')?;
    let start = above.rfind('\n').map_or(0, |i| i + 1);
    let line = above.get(start..)?;
    let mark = list_mark(line)?;
    if line.get(mark.len..).is_some_and(|rest| rest.trim().is_empty()) {
        // An empty item: Enter ends the list, leaving the line blank.
        text.replace_range(start..b, "");
        return Some(char_index(text, start));
    }
    text.insert_str(b, &mark.next);
    Some(caret + mark.next.chars().count())
}

/// Applies `f` to the text with the selection `sel` (anchor, caret as char indices) and returns the
/// new selection.
pub fn format(text: &mut String, sel: (usize, usize), f: Format) -> (usize, usize) {
    let count = text.chars().count();
    let (a, b) = (sel.0.min(sel.1).min(count), sel.0.max(sel.1).min(count));
    match f {
        Format::Bold => wrap(text, a, b, "**"),
        Format::Italic => wrap(text, a, b, "*"),
        Format::Strike => wrap(text, a, b, "~~"),
        Format::Code => wrap(text, a, b, "`"),
        Format::Field => wrap_pair(text, a, b, "{{", "}}"),
        Format::Link => {
            let label: String = text.chars().skip(a).take(b - a).collect();
            let label = if label.is_empty() { "링크".to_owned() } else { label };
            let url = "https://";
            let insert = format!("[{label}]({url})");
            replace_chars(text, a, b, &insert);
            let url_at = a + label.chars().count() + 3;
            (url_at, url_at + url.chars().count())
        }
        Format::Heading(n) => lines(text, a, b, |line, _| heading(line, n)),
        Format::Bullet => lines(text, a, b, |line, _| toggle_mark(line, "- ")),
        Format::Task => lines(text, a, b, |line, _| toggle_mark(line, "- [ ] ")),
        Format::Quote => lines(text, a, b, |line, _| toggle_mark(line, "> ")),
        Format::Number => {
            let numbered =
                line_range(text, a, b).1.iter().all(|l| list_mark(l).is_some_and(|m| m.next.trim_start().starts_with(|c: char| c.is_ascii_digit())));
            lines(text, a, b, |line, i| {
                let bare = strip_mark(line);
                if numbered { bare.to_owned() } else { format!("{}. {bare}", i + 1) }
            })
        }
        Format::Table => snippet(text, b, TABLE),
        Format::Snippet(s) => snippet(text, b, s),
    }
}

/// Wraps the selection in `mark`, or unwraps it when it already is; with nothing selected, puts
/// the caret between a pair of marks.
fn wrap(text: &mut String, a: usize, b: usize, mark: &str) -> (usize, usize) {
    wrap_pair(text, a, b, mark, mark)
}

/// [`wrap`] with different opening and closing marks.
fn wrap_pair(text: &mut String, a: usize, b: usize, open: &str, close: &str) -> (usize, usize) {
    let (o, c) = (open.chars().count(), close.chars().count());
    let chars: Vec<char> = text.chars().collect();
    let around = |from: usize, to: usize| chars.get(from..to).map(|s| s.iter().collect::<String>());
    if a >= o && around(a - o, a).as_deref() == Some(open) && around(b, b + c).as_deref() == Some(close) {
        replace_chars(text, b, b + c, "");
        replace_chars(text, a - o, a, "");
        return (a - o, b - o);
    }
    replace_chars(text, b, b, close);
    replace_chars(text, a, a, open);
    (a + o, b + o)
}

/// The lines the selection touches: the char index where the first starts, and the lines.
fn line_range(text: &str, a: usize, b: usize) -> (usize, Vec<String>) {
    let ba = byte_index(text, a);
    let bb = byte_index(text, b);
    let start = text.get(..ba).and_then(|s| s.rfind('\n')).map_or(0, |i| i + 1);
    let end = text.get(bb..).and_then(|s| s.find('\n')).map_or(text.len(), |i| bb + i);
    let lines = text.get(start..end).unwrap_or("").split('\n').map(str::to_owned).collect();
    (char_index(text, start), lines)
}

/// Rewrites each line the selection touches with `f(line, index)`; selects the rewritten lines.
fn lines(text: &mut String, a: usize, b: usize, f: impl Fn(&str, usize) -> String) -> (usize, usize) {
    let (start, old) = line_range(text, a, b);
    let new: Vec<String> = old.iter().enumerate().map(|(i, l)| f(l, i)).collect();
    let old_len = old.iter().map(|l| l.chars().count()).sum::<usize>() + old.len().saturating_sub(1);
    let joined = new.join("\n");
    replace_chars(text, start, start + old_len, &joined);
    (start, start + joined.chars().count())
}

fn heading(line: &str, n: u8) -> String {
    let hashes = line.len() - line.trim_start_matches('#').len();
    let bare = if hashes > 0 && line.get(hashes..).is_some_and(|r| r.is_empty() || r.starts_with(' ')) {
        line.get(hashes..).unwrap_or("").trim_start()
    } else {
        line
    };
    if hashes == usize::from(n) { bare.to_owned() } else { format!("{} {bare}", "#".repeat(usize::from(n))) }
}

/// The line without its list or quote mark (indentation kept).
fn strip_mark(line: &str) -> &str {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    match list_mark(line) {
        Some(m) => line.get(m.len..).unwrap_or(line),
        None => line.get(indent..).unwrap_or(line),
    }
}

/// Puts `mark` on the line, replacing another list mark, or takes it off when it is already there.
fn toggle_mark(line: &str, mark: &str) -> String {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let pad = line.get(..indent).unwrap_or("");
    let rest = line.get(indent..).unwrap_or("");
    if let Some(bare) = rest.strip_prefix(mark) {
        return format!("{pad}{bare}");
    }
    format!("{pad}{mark}{}", strip_mark(line).trim_start())
}

/// Inserts a block after the line holding char `at`, set off by blank lines; the caret follows it.
fn snippet(text: &mut String, at: usize, block: &str) -> (usize, usize) {
    let b = byte_index(text, at);
    let end = text.get(b..).and_then(|s| s.find('\n')).map_or(text.len(), |i| b + i);
    let line_start = text.get(..end).and_then(|s| s.rfind('\n')).map_or(0, |i| i + 1);
    let blank = text.get(line_start..end).is_some_and(|l| l.trim().is_empty());
    let insert = if blank && end == line_start { format!("{block}\n") } else { format!("\n\n{block}\n") };
    text.insert_str(end, &insert);
    let caret = char_index(text, end) + insert.chars().count();
    (caret, caret)
}

fn replace_chars(text: &mut String, a: usize, b: usize, with: &str) {
    let (ba, bb) = (byte_index(text, a), byte_index(text, b));
    if ba <= bb && text.is_char_boundary(ba) && text.is_char_boundary(bb) {
        text.replace_range(ba..bb, with);
    }
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b)
}

fn char_index(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or_else(|| text.chars().count(), |s| s.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str, sel: (usize, usize), f: Format) -> (String, (usize, usize)) {
        let mut t = text.to_owned();
        let s = format(&mut t, sel, f);
        (t, s)
    }

    #[test]
    fn wrapping_toggles() {
        assert_eq!(fmt("가나다", (0, 2), Format::Bold), ("**가나**다".to_owned(), (2, 4)));
        assert_eq!(fmt("**가나**다", (2, 4), Format::Bold), ("가나다".to_owned(), (0, 2)));
        assert_eq!(fmt("ab", (1, 1), Format::Italic), ("a**b".to_owned(), (2, 2)));
    }

    #[test]
    fn line_marks_toggle_and_replace() {
        assert_eq!(fmt("제목", (1, 1), Format::Heading(2)).0, "## 제목");
        assert_eq!(fmt("## 제목", (1, 1), Format::Heading(2)).0, "제목");
        assert_eq!(fmt("## 제목", (1, 1), Format::Heading(1)).0, "# 제목");
        assert_eq!(fmt("a\nb", (0, 3), Format::Bullet).0, "- a\n- b");
        assert_eq!(fmt("- a\n- b", (0, 6), Format::Bullet).0, "a\nb");
        assert_eq!(fmt("- a", (0, 0), Format::Task).0, "- [ ] a");
        assert_eq!(fmt("a\nb", (0, 3), Format::Number).0, "1. a\n2. b");
        assert_eq!(fmt("1. a\n2. b", (0, 8), Format::Number).0, "a\nb");
    }

    #[test]
    fn snippets_go_below_the_line() {
        let (t, (c, _)) = fmt("메모", (1, 1), Format::Table);
        assert!(t.starts_with("메모\n\n| 항목"));
        assert_eq!(c, t.chars().count());
        assert_eq!(fmt("", (0, 0), Format::Snippet("X")).0, "X\n");
    }

    #[test]
    fn fields_wrap_the_selection() {
        assert_eq!(fmt("A1", (0, 2), Format::Field), ("{{A1}}".to_owned(), (2, 4)));
        assert_eq!(fmt("{{A1}}", (2, 4), Format::Field), ("A1".to_owned(), (0, 2)));
        assert_eq!(fmt("", (0, 0), Format::Field), ("{{}}".to_owned(), (2, 2)));
    }

    #[test]
    fn links_select_their_address() {
        let (t, sel) = fmt("홈", (0, 1), Format::Link);
        assert_eq!(t, "[홈](https://)");
        assert_eq!(t.chars().skip(sel.0).take(sel.1 - sel.0).collect::<String>(), "https://");
    }

    #[test]
    fn enter_continues_and_ends_lists() {
        let mut t = "- 하나\n".to_owned();
        assert_eq!(continue_list(&mut t, 5), Some(7));
        assert_eq!(t, "- 하나\n- ");
        let mut t = "3. 셋\n".to_owned();
        continue_list(&mut t, 5);
        assert_eq!(t, "3. 셋\n4. ");
        let mut t = "  - [x] 끝\n".to_owned();
        continue_list(&mut t, 10);
        assert_eq!(t, "  - [x] 끝\n  - [ ] ");
        // Enter on an empty item ends the list.
        let mut t = "- 하나\n- \n".to_owned();
        assert_eq!(continue_list(&mut t, 8), Some(5));
        assert_eq!(t, "- 하나\n");
        let mut t = "plain\n".to_owned();
        assert_eq!(continue_list(&mut t, 6), None);
    }

    #[test]
    fn the_layout_covers_every_byte() {
        let c = Colors {
            text: Color32::BLACK,
            heading: Color32::RED,
            mark: Color32::BLUE,
            code: Color32::GREEN,
            quote: Color32::GRAY,
            link: Color32::YELLOW,
            field: Color32::WHITE,
        };
        let font = FontId::monospace(12.0);
        for text in ["", "# 제목\n- [ ] 할 일 `x` [a](#b) {{구매목록!D1}} **굵게**\n```\ncode\n```\n> 인용", "[unclosed `tick {{"] {
            let job = markdown_job(text, &font, &c, 100.0);
            assert_eq!(job.text, text);
        }
    }
}
