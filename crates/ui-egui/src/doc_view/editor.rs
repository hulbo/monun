//! The document editor: Markdown text with its marks coloured, the formatting buttons' edits, and
//! lists that continue on Enter. It is a plain `TextEdit`, so Korean input (IME) and the editor's
//! own undo work as in every other text field; the edits here are pure functions on char indices.

use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Ui};
use monun_engine::docs::TextKind;

use super::editor_id;
use crate::SheetApp;
use crate::clock::now_ms;
use crate::code_editor::{self, Lang};
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
    let (a, b) = selection(app, ctx);
    let Some(dr) = app.doc_view.draft.as_mut() else { return };
    let id = editor_id();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    replace_chars(&mut dr.text, a, b, text);
    dr.changed_at = Some(now_ms());
    let caret = a + text.chars().count();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    state.store(ctx, id);
    app.doc_view.focus_editor = true;
}

/// The editor's selection in chars, start first: the end of the text when it has none.
pub(super) fn selection(app: &SheetApp, ctx: &egui::Context) -> (usize, usize) {
    let Some(dr) = app.doc_view.draft.as_ref() else { return (0, 0) };
    let count = dr.text.chars().count();
    egui::TextEdit::load_state(ctx, editor_id()).and_then(|s| s.cursor.char_range()).map_or((count, count), |r| {
        let (x, y): (usize, usize) = (r.secondary.index.into(), r.primary.index.into());
        (x.min(y).min(count), x.max(y).min(count))
    })
}

/// Puts the editor's caret at the end of the text.
pub(super) fn caret_to_end(app: &SheetApp, ctx: &egui::Context) {
    let Some(dr) = app.doc_view.draft.as_ref() else { return };
    let id = editor_id();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(dr.text.chars().count()))));
    state.store(ctx, id);
}

/// How many documents' editor layouts are kept: the one shown and the last few before it, so going
/// round a workbook's tabs lays none out again. A megabyte of text takes most of a second to lay
/// out and some 170 MB once laid out, so no more are kept.
const KEEP_LAID: usize = 4;

/// How many of a workbook's long documents are laid out ahead (see [`super::lay_out_ahead`]).
pub(super) const AHEAD_DOCS: usize = 2;

/// What an editor layout was made for: the document, what its text is, the width it wraps at,
/// the font size and the theme.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    doc: (u64, usize),
    kind: TextKind,
    wrap: u32,
    size: u32,
    dark: bool,
}

/// An editor layout kept between frames: egui's own cache keeps only those drawn in the last
/// frame, so a document's text was laid out again every time its tab was shown.
struct Laid {
    look: Look,
    query: String,
    text: String,
    galley: Arc<egui::Galley>,
}

/// A laid-out paragraph and the width it was laid out at, with its layout at the width before when
/// it wraps differently there: going back (Edit and Side by Side, a window made wider and back)
/// takes that as it was.
#[derive(Clone)]
struct Kept {
    galley: Arc<egui::Galley>,
    wrap: f32,
    before: Option<(Arc<egui::Galley>, f32)>,
}

impl Kept {
    /// `part` laid out.
    fn new(ctx: &egui::Context, part: LayoutJob) -> Kept {
        let wrap = part.wrap.max_width;
        Kept { galley: ctx.fonts_mut(|f| f.layout_job(part)), wrap, before: None }
    }

    /// The paragraph as `part` lays it out: as kept at either width, or laid out now.
    fn at(&self, ctx: &egui::Context, part: LayoutJob) -> Kept {
        if same_at(&self.galley, self.wrap, &part) {
            return self.clone();
        }
        let before = Some((self.galley.clone(), self.wrap));
        match &self.before {
            Some((galley, wrap)) if same_at(galley, *wrap, &part) => Kept { galley: galley.clone(), wrap: *wrap, before },
            _ => Kept { before, ..Kept::new(ctx, part) },
        }
    }
}

/// Laid-out paragraphs by the hash of their layout job, its width left out (see [`same_at`]).
type Paragraphs = std::collections::HashMap<u64, Kept>;

/// A document's laid-out paragraphs, kept by their content so an edit lays out only the
/// paragraphs it changed. `whole` once every paragraph of the text was laid out, at the font size
/// and in the theme of `font` (a new zoom or theme lays them all out again); `ahead`, the paragraphs
/// being laid out ahead.
struct DocParagraphs {
    doc: (u64, usize),
    paragraphs: Paragraphs,
    whole: bool,
    font: (u32, bool),
    ahead: Option<Ahead>,
}

/// A long document's paragraphs being laid out ahead, a slice a frame: what for, the text's job,
/// and the paragraph next (its byte, and the first section that may reach it).
struct Ahead {
    look: Look,
    query: String,
    text: String,
    job: LayoutJob,
    at: usize,
    section: usize,
}

/// The editor's layouts kept between frames: whole texts and the latest documents' paragraphs.
#[derive(Default)]
pub(super) struct Layouts {
    laid: Vec<Laid>,
    docs: Vec<DocParagraphs>,
    /// The width the editor last wrapped at in Edit (0) and Side by Side (1).
    wraps: [Option<f32>; 2],
    /// A galley in egui's own cache, to tell when it starts its fonts over (see `check_fonts`).
    probe: Option<Arc<egui::Galley>>,
}

impl Layouts {
    /// Forgets every layout kept when egui has started its font atlas over (new fonts, a font
    /// found for a character, a full atlas): they point into the old atlas and would draw the
    /// wrong glyphs. egui tells only by emptying its own cache, so a probe is laid out there
    /// every frame: it comes back the same until the cache is emptied. Returns whether it was.
    pub(super) fn check_fonts(&mut self, ctx: &egui::Context) -> bool {
        // A letter every font set has.
        let job = LayoutJob::simple_singleline("x".into(), FontId::default(), Color32::WHITE);
        let probe = ctx.fonts_mut(|f| f.layout_job(job));
        let started_over = self.probe.as_ref().is_some_and(|p| !Arc::ptr_eq(p, &probe));
        if started_over {
            self.laid.clear();
            self.docs.clear();
        }
        self.probe = Some(probe);
        started_over
    }

    /// Whether the document's text needs no long layout before the editor shows it at zoom `z` in
    /// the theme: it is short, or its paragraphs were all laid out at that size in that theme
    /// (for this text or one before it).
    pub(super) fn ready(&self, doc: (u64, usize), text: &str, z: f32, dark: bool) -> bool {
        let font = (font_at(z).size.to_bits(), dark);
        !is_long(text) || self.docs.iter().any(|d| d.doc == doc && d.whole && d.font == font)
    }

    /// The width the editor wraps at in `mode`, as last seen.
    pub(super) fn wrap(&self, mode: super::Mode) -> Option<f32> {
        self.wraps.get(slot(mode)).copied().flatten()
    }

    /// The editor's galley of `text` for `look`: kept from before, or laid out with the
    /// document's paragraphs (`job` makes its layout job).
    fn layout(&mut self, ctx: &egui::Context, look: Look, query: &str, text: &str, job: impl FnOnce() -> LayoutJob) -> Arc<egui::Galley> {
        if let Some(kept) = self.laid.iter().find(|g| g.look == look && g.query == query && g.text == text) {
            return kept.galley.clone();
        }
        // The document's paragraphs as last laid out, whatever changed since.
        let before = self.docs.iter().position(|d| d.doc == look.doc).map(|i| self.docs.remove(i));
        let (paragraphs, ahead) = before.map_or_else(Default::default, |d| (d.paragraphs, d.ahead));
        let (galley, paragraphs) = layout_paragraphs(ctx, job(), paragraphs);
        if is_long(text) {
            // Only long texts' paragraphs are kept, so short documents never push one's out.
            self.keep(DocParagraphs { doc: look.doc, paragraphs, whole: true, font: (look.size, look.dark), ahead });
        }
        self.laid.insert(0, Laid { look, query: query.to_owned(), text: text.to_owned(), galley: galley.clone() });
        self.laid.truncate(KEEP_LAID);
        galley
    }

    /// Keeps a document's paragraphs first, dropping the oldest documents' beyond [`KEEP_LAID`]
    /// with their whole layouts.
    fn keep(&mut self, paragraphs: DocParagraphs) {
        self.docs.retain(|d| d.doc != paragraphs.doc);
        self.docs.insert(0, paragraphs);
        self.docs.truncate(KEEP_LAID);
        let docs = &self.docs;
        self.laid.retain(|l| !is_long(&l.text) || docs.iter().any(|d| d.doc == l.look.doc));
    }

    /// Lays out, for about `budget` ms, more of the paragraphs the editor will want to show `text`
    /// with `look` (`job` makes its layout job). Returns whether they are all laid out.
    fn ahead(&mut self, ctx: &egui::Context, look: Look, query: &str, text: &str, job: impl FnOnce() -> LayoutJob, budget: f64) -> bool {
        let deadline = crate::clock::monotonic_ms() + budget;
        if !self.docs.iter().any(|d| d.doc == look.doc) {
            self.keep(DocParagraphs { doc: look.doc, paragraphs: Paragraphs::new(), whole: false, font: (look.size, look.dark), ahead: None });
        }
        let Some(DocParagraphs { paragraphs, whole, font, ahead, .. }) = self.docs.iter_mut().find(|d| d.doc == look.doc) else { return false };
        if !ahead.as_ref().is_some_and(|a| a.look == look && a.query == query && a.text == text) {
            *ahead = Some(Ahead { look, query: query.to_owned(), text: text.to_owned(), job: rounded(job()), at: 0, section: 0 });
        }
        let Some(a) = ahead.as_mut() else { return false };
        let ppp = ctx.pixels_per_point();
        while let Some((mut part, next, section)) = paragraph_at(&a.job, a.at, a.section) {
            (a.at, a.section) = (next, section);
            let key = key_of(&mut part, ppp);
            let kept = match paragraphs.get(&key) {
                Some(k) => k.at(ctx, part),
                None => Kept::new(ctx, part),
            };
            paragraphs.insert(key, kept);
            if crate::clock::monotonic_ms() >= deadline {
                return false;
            }
        }
        (*whole, *font) = (true, (look.size, look.dark));
        *ahead = None;
        true
    }
}

/// Which of [`Layouts::wraps`] a mode's editor wraps at.
fn slot(mode: super::Mode) -> usize {
    usize::from(mode == super::Mode::Split)
}

/// Below this many bytes a text is laid out whole: it is quick, and egui's own cache serves it.
const PARAGRAPHS_FROM: usize = 64 * 1024;

/// Whether a text is laid out a paragraph at a time (see [`layout_paragraphs`]).
pub(super) fn is_long(text: &str) -> bool {
    text.len() >= PARAGRAPHS_FROM && text.contains('\n')
}

/// The job with its width rounded as egui rounds it, so a paragraph wraps as it would whole.
fn rounded(mut job: LayoutJob) -> LayoutJob {
    if job.wrap.max_width.is_finite() {
        job.wrap.max_width = job.wrap.max_width.round();
    }
    job
}

/// The paragraph of a long text's layout job that starts at byte `start`, its sections from the
/// `section`-th on: its own job, where the next paragraph starts and its first section. A
/// paragraph leaves out its `\n`, but the last keeps the one the text ends with, as egui splits.
fn paragraph_at(job: &LayoutJob, start: usize, mut section: usize) -> Option<(LayoutJob, usize, usize)> {
    use egui::text::{ByteIndex, LayoutSection};
    if start >= job.text.len() {
        return None;
    }
    let mut end = job.text.get(start..).and_then(|s| s.find('\n')).map_or(job.text.len(), |i| start + i);
    if end + 1 == job.text.len() && job.text.ends_with('\n') {
        end += 1;
    }
    let mut part = LayoutJob {
        text: job.text.get(start..end).unwrap_or_default().to_owned(),
        wrap: job.wrap.clone(),
        first_row_min_height: if start == 0 { job.first_row_min_height } else { 0.0 },
        break_on_newline: job.break_on_newline,
        direction: job.direction,
        halign: job.halign,
        halign_min_is_start: job.halign_min_is_start,
        justify: job.justify,
        round_output_to_gui: job.round_output_to_gui,
        keep_trailing_whitespace: job.keep_trailing_whitespace,
        language: job.language.clone(),
        ..LayoutJob::default()
    };
    for s in job.sections.get(section..).unwrap_or_default() {
        if s.byte_range.end <= ByteIndex(start) {
            section += 1;
        } else if ByteIndex(end) < s.byte_range.start {
            break;
        } else {
            part.sections.push(LayoutSection {
                leading_space: if ByteIndex(start) <= s.byte_range.start { s.leading_space } else { 0.0 },
                byte_range: s.byte_range.start.saturating_sub(start)..s.byte_range.end.min(ByteIndex(end)).saturating_sub(start),
                format: s.format.clone(),
            });
        }
    }
    Some((part, end + 1, section))
}

/// The key a paragraph is kept by: its job but for the width, and the pixel density.
fn key_of(part: &mut LayoutJob, ppp: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let wrap = std::mem::replace(&mut part.wrap.max_width, 0.0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&*part, ppp.to_bits()).hash(&mut h);
    part.wrap.max_width = wrap;
    h.finish()
}

/// Whether a paragraph laid out at width `laid_at` comes out as `part` would: it was laid out at
/// that width, or at another with room to spare at both. A left-to-right line that fits on one
/// row (the editor's lines, but the longest) is laid out the same at any width it fits, so
/// resizing the window or the editor lays out only the lines that wrap.
fn same_at(galley: &egui::Galley, laid_at: f32, part: &LayoutJob) -> bool {
    let wrap = part.wrap.max_width;
    if laid_at.to_bits() == wrap.to_bits() {
        return true;
    }
    // The width egui lets a row reach; 2 px spare cover the rounding of the size kept.
    let room = |w: f32| if part.round_output_to_gui { w + 0.5 } else { w };
    part.halign == egui::Align::LEFT
        && !part.justify
        && galley.rows.iter().all(|r| !r.rtl_paragraph)
        && galley.intrinsic_size().x + 2.0 <= room(wrap).min(room(laid_at))
}

/// Lays out `job` a paragraph (a line) at a time and joins them, as egui does for long texts,
/// taking every paragraph `kept` has that comes out the same (see [`same_at`]). Returns the galley
/// and its paragraphs. egui keeps its paragraphs only while the whole is laid out every frame;
/// kept here, a megabyte that changed in one line takes a few milliseconds, not a second.
fn layout_paragraphs(ctx: &egui::Context, job: LayoutJob, kept: Paragraphs) -> (Arc<egui::Galley>, Paragraphs) {
    if !is_long(&job.text) {
        return (ctx.fonts_mut(|f| f.layout_job(job)), Paragraphs::new());
    }
    let job = rounded(job);
    let ppp = ctx.pixels_per_point();
    let mut parts = Vec::new();
    let mut paragraphs = Paragraphs::new();
    let (mut start, mut section) = (0, 0);
    while let Some((mut part, next, first)) = paragraph_at(&job, start, section) {
        (start, section) = (next, first);
        let key = key_of(&mut part, ppp);
        // A line met before in this text (a blank one, a repeated one) is taken from there too.
        let kept = match paragraphs.get(&key).or_else(|| kept.get(&key)) {
            Some(k) => k.at(ctx, part),
            None => Kept::new(ctx, part),
        };
        parts.push(kept.galley.clone());
        paragraphs.insert(key, kept);
    }
    (Arc::new(egui::Galley::concat(Arc::new(job), &parts, ppp)), paragraphs)
}

/// The editor's font at zoom `z`.
fn font_at(z: f32) -> FontId {
    FontId::new(14.0 * z, FontFamily::Name(theme::MONO.into()))
}

/// The editor's layout job of `text`: its Markdown marks or SQL coloured (plain text as it is) and
/// `query`'s matches tinted. SQL and plain text lines are all as tall (see [`even_rows`]; `wide` is
/// [`Wide::of`]'s).
fn job_of(text: &str, (kind, wide): (TextKind, Wide), font: &FontId, colors: &Colors, wrap: f32, query: &str) -> LayoutJob {
    let mut job = match kind {
        TextKind::Markdown => markdown_job(text, font, colors, wrap),
        TextKind::Sql => code_editor::layout_job(text, Lang::Sql, font, colors.text, &colors.syntax, wrap),
        TextKind::Plain | TextKind::Pdf | TextKind::Word | TextKind::Hwp => {
            code_editor::layout_job(text, Lang::Plain, font, colors.text, &colors.syntax, wrap)
        }
    };
    if kind != TextKind::Markdown {
        even_rows(&mut job, code_line_height(font), wide);
    }
    tint(&mut job, &super::find_all(text, query), super::preview::FOUND);
    job
}

/// SQL and plain text lines' height, in font sizes.
const CODE_LINE_HEIGHT: f32 = 1.45;

/// The height of a SQL or plain text line in `font`.
fn code_line_height(font: &FontId) -> f32 {
    (font.size * CODE_LINE_HEIGHT).round()
}

/// How egui lays out Hangul in code: it draws it in a fallback face on the code's baseline, moving
/// the section that holds it down by `above` and making its row `grow` taller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wide {
    above: f32,
    grow: f32,
}

impl Wide {
    /// Measured in `font` at the code's line height; nothing for Markdown, which isn't evened.
    fn of(ctx: &egui::Context, kind: TextKind, font: &FontId) -> Wide {
        // Without a face that has Hangul there is nothing to even out (and nothing to lay it in).
        if kind == TextKind::Markdown || !ctx.fonts_mut(|f| f.has_glyph(font, '가')) {
            return Wide::default();
        }
        let format =
            TextFormat { line_height: Some(code_line_height(font)), valign: egui::Align::TOP, ..TextFormat::simple(font.clone(), Color32::WHITE) };
        let lay = |s: &str| ctx.fonts_mut(|f| f.layout_job(LayoutJob::single_section(s.to_owned(), format.clone())));
        let (latin, hangul) = (lay("a"), lay("가"));
        let y = |g: &egui::Galley| g.rows.first().and_then(|r| r.glyphs.first()).map_or(0.0, |glyph| glyph.pos.y);
        Wide { above: (y(&hangul) - y(&latin)).max(0.0), grow: (hangul.size().y - latin.size().y).max(0.0) }
    }
}

/// Whether a character is drawn in the wide (CJK) fallback face: Hangul, kana, CJK ideographs and
/// full-width forms.
fn is_wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF)
}

/// Gives every line of a code layout the same `height` and the same baseline, as code editors do,
/// though egui moves Hangul down and grows its row (see [`Wide`]). The sections are cut into runs
/// of wide characters and of the rest, within a line. The rest is `height` tall. A run of Hangul
/// beside other characters is laid out `2 × above` shorter at the top of its row, so that once
/// moved down and grown it shares their baseline and row; a line of Hangul alone, `grow` shorter.
fn even_rows(job: &mut LayoutJob, height: f32, wide: Wide) {
    use egui::text::{ByteIndex, LayoutSection};
    let sections = std::mem::take(&mut job.sections);
    // Where each line ends (past its line break), and whether it has anything but wide characters.
    let mut lines = Vec::new();
    let mut end = 0;
    for line in job.text.split_inclusive('\n') {
        end += line.len();
        lines.push((end, line.trim_end_matches(['\n', '\r']).chars().any(|c| !is_wide(c))));
    }
    let beside = (height - 2.0 * wide.above).max(1.0);
    let alone = (height - wide.grow).max(1.0);
    let mut out = Vec::with_capacity(sections.len() + lines.len());
    let mut at_line = 0;
    for s in sections {
        let (start, end) = (s.byte_range.start.0, s.byte_range.end.0);
        // The section's runs: (start, end, wide).
        let mut runs: Vec<(usize, usize, bool)> = Vec::new();
        let mut run: Option<(usize, bool)> = None;
        for (i, c) in job.text.get(start..end).unwrap_or_default().char_indices() {
            let (at, w) = (start + i, is_wide(c));
            let line_starts = i > 0 && job.text.as_bytes().get(at - 1) == Some(&b'\n');
            match run {
                Some((from, was)) if was != w || line_starts => {
                    runs.push((from, at, was));
                    run = Some((at, w));
                }
                Some(_) => {}
                None => run = Some((at, w)),
            }
        }
        let (from, was) = run.unwrap_or((start, false));
        runs.push((from, end, was));
        for (a, b, w) in runs {
            while lines.get(at_line).is_some_and(|(line_end, _)| *line_end <= a) {
                at_line += 1;
            }
            let others = lines.get(at_line).is_none_or(|(_, others)| *others);
            let mut format = s.format.clone();
            format.line_height = Some(match (w, others) {
                (false, _) => height,
                (true, true) => beside,
                (true, false) => alone,
            });
            if w && others {
                format.valign = egui::Align::TOP;
            }
            let leading_space = if a == start { s.leading_space } else { 0.0 };
            out.push(LayoutSection { byte_range: ByteIndex(a)..ByteIndex(b), leading_space, format });
        }
    }
    job.sections = out;
}

/// Lays out ahead, for about `budget` ms, the editor's paragraphs of document `doc`'s `text` (of
/// `kind`) at zoom `z`, wrapping at `wrap`. Returns whether they are all laid out.
pub(super) fn lay_out_ahead(
    layouts: &mut Layouts,
    ctx: &egui::Context,
    doc: (u64, usize),
    text: &str,
    (z, wrap, query, kind): (f32, f32, &str, TextKind),
    t: &Tokens,
    budget: f64,
) -> bool {
    let font = font_at(z);
    let colors = Colors::of(t);
    let look = Look { doc, kind, wrap: wrap.to_bits(), size: font.size.to_bits(), dark: t.dark };
    let code = (kind, Wide::of(ctx, kind, &font));
    layouts.ahead(ctx, look, query, text, || job_of(text, code, &font, &colors, wrap, query), budget)
}

/// The room left of the editor's text: its margin, and for SQL and plain text the line numbers'
/// column (as wide as the most digits the text's lines need, at least three).
fn text_margin(ui: &Ui, kind: TextKind, text: &str, font: &FontId) -> f32 {
    if kind == TextKind::Markdown {
        return MARGIN;
    }
    let lines = text.bytes().filter(|&b| b == b'\n').count() + 1;
    let digits = lines.to_string().len().max(3) as f32;
    let digit = ui.painter().layout_no_wrap("0".to_owned(), font.clone(), Color32::WHITE).size().x;
    NUMBERS_LEFT + digits * digit + NUMBERS_GAP
}

/// The editor's margin around its text.
const MARGIN: f32 = 14.0;
/// Room left of the line numbers.
const NUMBERS_LEFT: f32 = 10.0;
/// Room between the line numbers and the text.
const NUMBERS_GAP: f32 = 18.0;

/// Paints the line numbers of the rows in view, right-aligned left of the text, the caret's line
/// brighter.
fn line_numbers(ui: &Ui, out: &egui::text_edit::TextEditOutput, font: &FontId, t: &Tokens, focused: bool) {
    let clip = ui.clip_rect();
    let caret_y = out.cursor_range.filter(|_| focused).map(|r| out.galley.pos_from_cursor(r.primary).center().y);
    // The first row of each line in view: its number and its middle.
    let (mut shown, mut caret_line) = (Vec::new(), None);
    let (mut line, mut starts) = (0usize, true);
    for row in &out.galley.rows {
        let rect = row.rect();
        if starts {
            line += 1;
        }
        if caret_y.is_some_and(|y| y >= rect.top() && y < rect.bottom()) {
            caret_line = Some(line);
        }
        let (top, bottom) = (out.galley_pos.y + rect.top(), out.galley_pos.y + rect.bottom());
        if top > clip.bottom() {
            break;
        }
        if starts && bottom >= clip.top() {
            shown.push((line, (top + bottom) / 2.0));
        }
        starts = row.ends_with_newline;
    }
    let right = out.galley_pos.x - NUMBERS_GAP + 4.0;
    for (n, y) in shown {
        let ink = if caret_line == Some(n) { t.text } else { t.text_dim };
        ui.painter().text(egui::pos2(right, y), egui::Align2::RIGHT_CENTER, n.to_string(), font.clone(), ink);
    }
}

/// After Enter at char `caret` (just past the new line break) in SQL or plain text: the new line
/// starts as indented as the one above. Returns the caret.
pub fn keep_indent(text: &mut String, caret: usize) -> Option<usize> {
    let b = byte_index(text, caret);
    let above = text.get(..b)?.strip_suffix('\n')?;
    let start = above.rfind('\n').map_or(0, |i| i + 1);
    let indent: String = above.get(start..)?.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if indent.is_empty() {
        return None;
    }
    text.insert_str(b, &indent);
    Some(caret + indent.chars().count())
}

/// Tab or Shift+Tab in SQL or plain text, while the editor has the keyboard: indents or dedents
/// the lines selected, or puts four spaces at the caret, as code editors do. Returns whether the
/// text changed.
fn indent_keys(ctx: &egui::Context, text: &mut String) -> bool {
    let tab = ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
            Some(true)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
            Some(false)
        } else {
            None
        }
    });
    let Some(dedent) = tab else { return false };
    let mut state = egui::TextEdit::load_state(ctx, editor_id()).unwrap_or_default();
    let count = text.chars().count();
    let sel = state.cursor.char_range().map_or((count, count), |r| (r.secondary.index.into(), r.primary.index.into()));
    let e = code_editor::indent(text, sel, dedent);
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(e.anchor), CCursor::new(e.caret))));
    state.store(ctx, editor_id());
    true
}

/// How long a frame lays out a shown document's paragraphs while its first screen stands in
/// (ms): the window stays responsive, at some 25 frames a second.
const FIRST_SHOW_MS: f64 = 30.0;

/// The editor's place while a long document's text is first laid out, a slice a frame: its first
/// lines, drawn as the editor will draw them, so the tab shows at once and not a second later.
/// Typing waits for the editor.
fn first_screen(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens, mode: super::Mode) {
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
    let kind = super::text_kind(app);
    let (draft, layouts) = (&app.doc_view.draft, &mut app.doc_view.layouts);
    let Some(dr) = draft.as_ref() else { return };
    let font = font_at(z);
    let colors = Colors::of(t);
    let rows = ((ui.available_height() / (font.size * 1.35)).floor() as usize).max(4);
    // From the line a zoom keeps at the top, which the editor goes to once it shows; two screens
    // of lines, so the scroll bar shows as it will.
    let doc = (dr.uid, dr.sheet);
    let line = app.doc_view.anchor.filter(|(d, _)| *d == doc).map_or(0, |(_, line)| line);
    app.doc_view.top = Some((doc, line));
    let start = monun_markdown::line_start(&dr.text, line);
    let end = dr.text.get(start..).and_then(|s| s.match_indices('\n').nth(rows * 2)).map_or(dr.text.len(), |(i, _)| start + i);
    let mut head = dr.text.get(start..end).unwrap_or_default().to_owned();
    let left = text_margin(ui, kind, &dr.text, &font);
    let code = (kind, Wide::of(ui.ctx(), kind, &font));
    let mut wrap_at = None;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        wrap_at = Some(wrap);
        ui.fonts_mut(|f| f.layout_job(job_of(buf.as_str(), code, &font, &colors, wrap, &query)))
    };
    egui::ScrollArea::vertical().id_salt("monun.doc_editor_first_screen").auto_shrink([false, false]).show(ui, |ui| {
        egui::TextEdit::multiline(&mut head)
            .id(editor_id().with("first_screen"))
            .font(font.clone())
            .frame(egui::Frame::NONE.inner_margin(margin(left)))
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .interactive(false)
            .layouter(&mut layouter)
            .show(ui)
    });
    let Some(wrap) = wrap_at else { return };
    if let Some(w) = layouts.wraps.get_mut(slot(mode)) {
        *w = Some(wrap);
    }
    let ctx = ui.ctx().clone();
    lay_out_ahead(layouts, &ctx, (dr.uid, dr.sheet), &dr.text, (z, wrap, &query, kind), t, FIRST_SHOW_MS);
    ctx.request_repaint();
}

/// The editor's margins: `left` of the text, the usual elsewhere.
fn margin(left: f32) -> egui::Margin {
    let side = |v: f32| v.round().clamp(0.0, 127.0) as i8;
    egui::Margin { left: side(left), right: side(MARGIN), top: side(MARGIN), bottom: side(MARGIN) }
}

/// The editor, filling `ui`, in `mode` (Edit or Side by Side). Returns the first line in view.
pub(super) fn show(app: &mut SheetApp, ui: &mut Ui, z: f32, t: &Tokens, mode: super::Mode) -> Option<usize> {
    if let Some(dr) = app.doc_view.draft.as_ref()
        && !app.doc_view.layouts.ready((dr.uid, dr.sheet), &dr.text, z, t.dark)
    {
        // Before anything else, so a line to go to and the focus wait for the editor.
        first_screen(app, ui, z, t, mode);
        return None;
    }
    let goto = app.doc_view.goto_line.take();
    let select = app.doc_view.select.take();
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
    let kind = super::text_kind(app);
    let ctx = ui.ctx().clone();
    if app.doc_view.focus_editor && !ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
        app.doc_view.focus_editor = false;
        ctx.memory_mut(|m| m.request_focus(editor_id()));
    }
    let dr = app.doc_view.draft.as_mut()?;
    let font = font_at(z);
    let focused = ctx.memory(|m| m.has_focus(editor_id()));
    if kind != TextKind::Markdown && focused && indent_keys(&ctx, &mut dr.text) {
        dr.changed_at = Some(now_ms());
    }
    // A line to go to (the outline, a double-click) or a found match to select.
    let goto = goto.map(|line| char_index(&dr.text, monun_markdown::line_start(&dr.text, line))).map(|at| (at, at)).or(select);
    if let Some((a, b)) = goto {
        let mut state = egui::TextEdit::load_state(&ctx, editor_id()).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(&ctx, editor_id());
    }
    let colors = Colors::of(t);
    let doc = (dr.uid, dr.sheet);
    // The line to keep at the top after a zoom (a line to go to wins).
    let anchor = app.doc_view.anchor.take_if(|(d, _)| *d == doc).map(|(_, line)| line).filter(|_| goto.is_none());
    let anchor_at = anchor.map(|line| char_index(&dr.text, monun_markdown::line_start(&dr.text, line)));
    let code = (kind, Wide::of(&ctx, kind, &font));
    let layouts = &mut app.doc_view.layouts;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let text = buf.as_str();
        if let Some(w) = layouts.wraps.get_mut(slot(mode)) {
            *w = Some(wrap);
        }
        let look = Look { doc, kind, wrap: wrap.to_bits(), size: font.size.to_bits(), dark: t.dark };
        layouts.layout(ui.ctx(), look, &query, text, || job_of(text, code, &font, &colors, wrap, &query))
    };
    let left = text_margin(ui, kind, &dr.text, &font);
    let rows = ((ui.available_height() / (font.size * 1.35)).floor() as usize).max(4);
    // Each document keeps its own place in its text.
    let scroll = egui::ScrollArea::vertical().id_salt(("monun.doc_editor_scroll", doc)).auto_shrink([false, false]).show(ui, |ui| {
        let out = egui::TextEdit::multiline(&mut dr.text)
            .id(editor_id())
            .font(font.clone())
            // The frame's margin is the text's (egui leaves `margin` aside once a frame is set).
            .frame(egui::Frame::NONE.inner_margin(margin(left)))
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        if kind != TextKind::Markdown {
            line_numbers(ui, &out, &font, t, focused);
        }
        if let Some((at, _)) = goto {
            let r = out.galley.pos_from_cursor(CCursor::new(at)).translate(out.galley_pos.to_vec2());
            ui.scroll_to_rect(r.expand(40.0), Some(egui::Align::Center));
        }
        if let Some(at) = anchor_at {
            let r = out.galley.pos_from_cursor(CCursor::new(at)).translate(out.galley_pos.to_vec2());
            ui.scroll_to_rect_animation(r, Some(egui::Align::Min), egui::style::ScrollAnimation::none());
        }
        out
    });
    let viewport_top = scroll.inner_rect.top();
    let out = scroll.inner;
    let top_char: usize = out.galley.cursor_from_pos(egui::vec2(0.0, (viewport_top - out.galley_pos.y).max(0.0))).index.into();
    // Scrolled to the line kept, from the next frame on.
    let top = anchor.unwrap_or_else(|| monun_markdown::line_of(&dr.text, byte_index(&dr.text, top_char)));
    app.doc_view.top = Some((doc, top));
    out.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "Document text"));
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none());
    if out.response.changed() {
        dr.changed_at = Some(now_ms());
        let carried =
            |text: &mut String, caret: usize| if kind == TextKind::Markdown { continue_list(text, caret) } else { keep_indent(text, caret) };
        if enter
            && let Some(range) = out.cursor_range
            && let Some(caret) = carried(&mut dr.text, range.primary.index.into())
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

/// The colours of the editor's Markdown marks, and of SQL.
pub struct Colors {
    syntax: theme::Syntax,
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
            syntax: t.syntax,
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

pub(super) fn byte_index(text: &str, chars: usize) -> usize {
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
    fn code_lines_are_all_as_tall_with_hangul_or_without() {
        // A comment with Hangul, a line of Hangul alone, an empty line and a plain one.
        let text = "SELECT 1 /* 주석\n끝맺음\n\nend */";
        let font = FontId::monospace(14.0);
        let mut job = code_editor::layout_job(text, Lang::Sql, &font, Color32::BLACK, &Tokens::light().syntax, 100.0);
        even_rows(&mut job, 20.0, Wide { above: 3.0, grow: 4.0 });
        assert_eq!(job.text, text);
        let mut at = 0;
        let mut seen = Vec::new();
        for s in &job.sections {
            let (a, b) = (s.byte_range.start.0, s.byte_range.end.0);
            assert_eq!(a, at, "the sections cover the text in order");
            at = b;
            let piece = &text[a..b];
            assert!(!piece.trim_end_matches('\n').contains('\n'), "{piece:?}: one line each");
            let hangul = piece.chars().any(is_wide);
            assert!(!hangul || piece.chars().all(is_wide), "{piece:?}: Hangul in runs of its own");
            seen.push((piece.trim_end_matches('\n'), s.format.line_height, s.format.valign));
        }
        assert_eq!(at, text.len());
        let top = egui::Align::TOP;
        let bottom = egui::Align::BOTTOM;
        assert!(seen.contains(&("SELECT", Some(20.0), bottom)));
        assert!(seen.contains(&("주석", Some(14.0), top)), "beside other characters: 2 × above shorter, at the top: {seen:?}");
        assert!(seen.contains(&("끝맺음", Some(16.0), bottom)), "alone: grow shorter: {seen:?}");
        assert!(seen.iter().filter(|(p, ..)| !p.chars().any(is_wide)).all(|(_, h, _)| *h == Some(20.0)));
    }

    #[test]
    fn enter_in_code_keeps_the_indentation() {
        let mut t = "SELECT a\n    FROM 표\n".to_owned();
        assert_eq!(keep_indent(&mut t, 20), Some(24));
        assert_eq!(t, "SELECT a\n    FROM 표\n    ");
        let mut t = "\tx\n".to_owned();
        assert_eq!(keep_indent(&mut t, 3), Some(4));
        assert_eq!(t, "\tx\n\t");
        let mut t = "x\n".to_owned();
        assert_eq!(keep_indent(&mut t, 2), None);
        assert_eq!(keep_indent(&mut "x".to_owned(), 1), None, "not after a line break");
    }

    #[test]
    fn the_layout_covers_every_byte() {
        let c = Colors {
            syntax: Tokens::light().syntax,
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
            for kind in [TextKind::Markdown, TextKind::Sql, TextKind::Plain] {
                assert_eq!(job_of(text, (kind, Wide { above: 3.0, grow: 4.0 }), &font, &c, 100.0, "x").text, text, "{kind:?}");
            }
        }
    }
}

#[cfg(test)]
mod paragraph_tests {
    use super::*;

    /// About `lines` lines of Markdown: headings, list items with code, blank lines and prose.
    fn long_text(lines: usize) -> String {
        let mut s = String::new();
        for i in 0..lines {
            match i % 8 {
                0 => s.push_str(&format!("## 계정 {i} 정보\n")),
                1 | 2 => s.push_str(&format!("- 아이디: user{i} · 메모 `code{i}` 확인 필요\n")),
                3 => s.push('\n'),
                _ => s.push_str(&format!("설명 문장 {i} 은 이렇게 길게 이어진다, 새 시스템 접속 정보와 담당자 연락처\n")),
            }
        }
        s
    }

    /// Runs `f` in a frame of a context with the interface's fonts.
    fn in_frame(f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::pinned_font_definitions());
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let mut f = Some(f);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
        out.textures_delta.clear();
    }

    fn rows(g: &egui::Galley) -> Vec<(String, (i32, i32), bool)> {
        g.rows.iter().map(|r| (r.text(), ((r.pos.x * 10.0) as i32, (r.pos.y * 10.0) as i32), r.ends_with_newline)).collect()
    }

    #[test]
    fn a_long_text_lays_out_a_paragraph_at_a_time_as_it_would_whole() {
        in_frame(|ui| {
            let t = Tokens::light();
            let colors = Colors::of(&t);
            let font = FontId::new(14.0, FontFamily::Name(theme::MONO.into()));
            let text = long_text(2000);
            assert!(text.len() > PARAGRAPHS_FROM);
            let job = || {
                let mut job = markdown_job(&text, &font, &colors, 400.0);
                tint(&mut job, &crate::doc_view::find_all(&text, "user1"), crate::doc_view::preview::FOUND);
                job
            };
            let whole = ui.fonts_mut(|f| f.layout_job(job()));
            let (parts, kept) = layout_paragraphs(ui.ctx(), job(), Paragraphs::new());
            assert_eq!(parts.text(), text);
            assert_eq!(rows(&parts), rows(&whole));
            assert_eq!(parts.size(), whole.size());
            // An edit lays out again only what changed, and comes out the same as a fresh layout.
            let mut edited = text.clone();
            edited.insert_str(text.len() / 2, "고침 ");
            let job = markdown_job(&edited, &font, &colors, 400.0);
            let fresh = ui.fonts_mut(|f| f.layout_job(job.clone()));
            let reused_before = kept.len();
            let (again, _) = layout_paragraphs(ui.ctx(), job, kept);
            assert_eq!(again.text(), edited);
            assert_eq!(rows(&again), rows(&fresh));
            assert!(reused_before > 1000);
            // A short text is laid out whole, as before.
            let (short, none) = layout_paragraphs(ui.ctx(), markdown_job("# 제목\n본문\n", &font, &colors, 400.0), Paragraphs::new());
            assert!(none.is_empty() && short.text() == "# 제목\n본문\n");
        });
    }

    /// The paragraphs of `kept` the galley `g` shares (by their rows).
    /// The paragraphs of `kept` the galley `g` shares (by their rows), at either width kept.
    fn shared(g: &egui::Galley, kept: &Paragraphs) -> usize {
        let used = |p: &egui::Galley| p.rows.first().is_some_and(|r| g.rows.iter().any(|gr| Arc::ptr_eq(&gr.row, &r.row)));
        kept.values().filter(|k| used(&k.galley) || k.before.as_ref().is_some_and(|(b, _)| used(b))).count()
    }

    #[test]
    fn paragraphs_laid_out_at_one_width_serve_another_where_they_fit() {
        in_frame(|ui| {
            let t = Tokens::light();
            let colors = Colors::of(&t);
            let font = FontId::new(14.0, FontFamily::Name(theme::MONO.into()));
            let text = long_text(2000);
            let (_, kept) = layout_paragraphs(ui.ctx(), markdown_job(&text, &font, &colors, 700.0), Paragraphs::new());
            // Wider, narrower (the prose wraps), back to about the first (a fraction of a pixel
            // off) and without a limit; with whether every line is taken as it was.
            let mut kept = kept;
            for (wrap, all) in [(900.0, true), (320.0, false), (700.4, true), (f32::INFINITY, true)] {
                let job = markdown_job(&text, &font, &colors, wrap);
                let fresh = ui.fonts_mut(|f| f.layout_job(job.clone()));
                let before = kept.clone();
                let (again, k) = layout_paragraphs(ui.ctx(), job, kept);
                assert_eq!(rows(&again), rows(&fresh), "at {wrap}");
                assert_eq!(again.size(), fresh.size(), "at {wrap}");
                // The headings and list items fit at every width; the prose wraps at 320, and
                // back at 700 is taken as laid out there.
                let taken = shared(&again, &before);
                assert!(if all { taken == before.len() } else { taken > 700 && taken < before.len() }, "at {wrap}: {taken} of {}", before.len());
                kept = k;
            }
        });
    }

    #[test]
    fn a_long_text_laid_out_ahead_in_slices_shows_without_laying_out_again() {
        in_frame(|ui| {
            let t = Tokens::light();
            let colors = Colors::of(&t);
            let font = FontId::new(14.0, FontFamily::Name(theme::MONO.into()));
            let text = long_text(2000);
            let doc = (7, 1);
            let job = || job_of(&text, (TextKind::Markdown, Wide::default()), &font, &colors, 500.0, "");
            let look = Look { doc, kind: TextKind::Markdown, wrap: 500f32.to_bits(), size: font.size.to_bits(), dark: false };
            let mut layouts = Layouts::default();
            assert!(!layouts.ready(doc, &text, 1.0, false));
            let mut slices = 1;
            // No time at all: a paragraph a call.
            while !layouts.ahead(ui.ctx(), look, "", &text, job, 0.0) {
                slices += 1;
            }
            assert!(slices > 1000);
            assert!(layouts.ready(doc, &text, 1.0, false) && layouts.docs.first().is_some_and(|d| d.ahead.is_none()));
            // Another zoom or theme lays it out again.
            assert!(!layouts.ready(doc, &text, 1.2, false) && !layouts.ready(doc, &text, 1.0, true));
            let laid = layouts.docs.first().map(|d| d.paragraphs.len()).unwrap_or_default();
            let galley = layouts.layout(ui.ctx(), look, "", &text, job);
            let whole = ui.fonts_mut(|f| f.layout_job(job()));
            assert_eq!(rows(&galley), rows(&whole));
            let kept = layouts.docs.first().map(|d| &d.paragraphs).map_or(0, |p| shared(&galley, p));
            assert_eq!(kept, laid, "every paragraph laid out ahead is used as it is");
            // A short document is ready at once and keeps no paragraphs.
            assert!(layouts.ready((7, 2), "# 짧은 문서\n", 1.0, false));
            layouts.layout(ui.ctx(), Look { doc: (7, 2), ..look }, "", "# 짧은 문서\n", || {
                job_of("# 짧은 문서\n", (TextKind::Markdown, Wide::default()), &font, &colors, 500.0, "")
            });
            assert_eq!(layouts.docs.len(), 1);
        });
    }

    #[test]
    fn layouts_are_forgotten_when_egui_starts_its_fonts_over() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::pinned_font_definitions());
        let frame = |layouts: &mut Layouts| {
            let mut started_over = false;
            ctx.run_ui(egui::RawInput::default(), |ui| started_over = layouts.check_fonts(ui.ctx())).textures_delta.clear();
            started_over
        };
        let text = long_text(4000);
        let mut layouts = Layouts::default();
        assert!(!frame(&mut layouts));
        layouts.docs.push(DocParagraphs {
            doc: (1, 1),
            paragraphs: Paragraphs::new(),
            whole: true,
            font: (font_at(1.0).size.to_bits(), false),
            ahead: None,
        });
        assert!(!frame(&mut layouts) && !frame(&mut layouts));
        assert!(layouts.ready((1, 1), &text, 1.0, false));
        ctx.set_fonts(egui::FontDefinitions::default());
        assert!(frame(&mut layouts));
        assert!(!layouts.ready((1, 1), &text, 1.0, false));
        assert!(!frame(&mut layouts));
    }

    #[test]
    #[ignore]
    fn timing_of_an_edit_in_a_megabyte() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions_for_language(crate::i18n::Language::Ko));
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let text = long_text(24_000);
        let t = Tokens::light();
        let colors = Colors::of(&t);
        let font = FontId::new(14.0, FontFamily::Name(theme::MONO.into()));
        let mut kept = Paragraphs::new();
        for (step, at) in [(0, None), (1, Some(text.len() / 2)), (2, Some(text.len() / 3)), (3, Some(text.len() / 4))] {
            let mut edited = text.clone();
            if let Some(at) = at.and_then(|at| (at..text.len()).find(|i| text.is_char_boundary(*i))) {
                edited.insert(at, 'x');
            }
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let clock = std::time::Instant::now();
                let job = markdown_job(&edited, &font, &colors, 800.0);
                let (_, k) = layout_paragraphs(ui.ctx(), job, std::mem::take(&mut kept));
                kept = k;
                eprintln!("TIMING step {step}: {:?}", clock.elapsed());
            });
            out.textures_delta.clear();
        }
    }
}
