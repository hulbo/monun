//! Document tabs on paper: the Markdown laid out as text pages inside the sheet's Page Setup paper
//! and margins. Headings, wrapped paragraphs with their styles, lists and task boxes, quotes, code,
//! tables, rules, pictures (`![…](image:N)`), `{{…}}` fields with their values, and Mermaid
//! diagrams where this build draws them (else their code). Hangul goes through the PDF writer's
//! Korean font; it has no bold face, so bold Hangul prints regular.

use std::sync::Arc;

use monun_markdown::code::{Lang, Token};
use monun_markdown::{Align, Block, Item, Span};
use monun_model::{Sheet, Workbook};
use monun_pdf::{Font, Page, Rect, Rgb, text_width};

use super::SheetLayout;

/// One drawing on a page, in points from the page's top left (text at its baseline).
pub(super) enum Op {
    Text { x: f32, y: f32, size: f32, font: Font, color: Rgb, text: String },
    Fill { x: f32, y: f32, w: f32, h: f32, color: Rgb },
    Line { x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgb },
    Frame { x: f32, y: f32, w: f32, h: f32, color: Rgb },
    Image { rect: Rect, rgba: Arc<Vec<u8>>, size: [u32; 2] },
    Link { x: f32, y: f32, w: f32, h: f32, uri: String },
}

/// The most pages one document prints.
const MAX_PAGES: usize = 2000;
const BODY: f32 = 10.5;
const CODE: f32 = 9.0;
const LEADING: f32 = 1.45;
const INK: Rgb = [0x1F, 0x1F, 0x1F];
const DIM: Rgb = [0x60, 0x60, 0x60];
const LINK: Rgb = [0x06, 0x45, 0xAD];
const RULE: Rgb = [0xC0, 0xC0, 0xC0];
const SHADE: Rgb = [0xF2, 0xF2, 0xF2];
/// Highlighted text (`<mark>`).
const MARK: Rgb = [0xFF, 0xF1, 0x8C];
/// A table's header row.
const HEAD: Rgb = [0xE3, 0xE8, 0xEF];

/// The pages of document tab `sh`, as drawings; with `fit` (or the document's own Fit Width) its
/// tables are as wide as the page and its diagrams centred. With `code` (a SQL or plain text file:
/// the language it is coloured as) its text prints as it is, in a monospace font, rather than as
/// Markdown.
pub(super) fn pages(wb: &Workbook, sh: &Sheet, lay: &SheetLayout, (fit, code): (bool, Option<Lang>)) -> Vec<Vec<Op>> {
    let Some(doc) = &sh.document else { return Vec::new() };
    let [l, r, t, b, ..] = lay.margins;
    let mut flow = Flow {
        wb,
        sh,
        pages: vec![Vec::new()],
        y: t,
        left: l,
        width: (lay.page_w - l - r).max(72.0),
        top: t,
        bottom: (lay.page_h - b).max(t + 72.0),
        fit: fit || doc.fit_width,
    };
    if let Some(lang) = code {
        flow.code(&doc.text, 0.0, false, lang);
        flow.pages.truncate(MAX_PAGES);
        return flow.pages;
    }
    let blocks = monun_markdown::parse(&doc.text);
    for block in &blocks {
        if flow.pages.len() >= MAX_PAGES {
            break;
        }
        flow.block(block, 0.0, INK);
    }
    flow.pages
}

/// Draws a page's drawings.
pub(super) fn draw(pg: &mut Page, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Text { x, y, size, font, color, text } => pg.text(*x, *y, *size, *font, *color, text),
            Op::Fill { x, y, w, h, color } => pg.fill_rect(*x, *y, *w, *h, *color),
            Op::Line { x0, y0, x1, y1, width, color } => pg.stroke_line(*x0, *y0, *x1, *y1, *width, *color),
            Op::Frame { x, y, w, h, color } => pg.rect_stroke(*x, *y, *w, *h, 0.8, *color),
            Op::Image { rect, rgba, size } => pg.image_rgba(*rect, rgba, *size),
            Op::Link { x, y, w, h, uri } => pg.link_uri(*x, *y, *w, *h, uri),
        }
    }
}

/// A run of text in one look, as it is laid out.
#[derive(Clone)]
struct Piece {
    text: String,
    font: Font,
    size: f32,
    color: Rgb,
    link: Option<String>,
    /// The band behind the text: grey for code and fields, yellow for highlighted text.
    shade: Option<Rgb>,
    strike: bool,
    underline: bool,
}

impl Piece {
    fn width(&self) -> f32 {
        text_width(self.font, self.size, &self.text)
    }
}

/// The layout going down the pages.
struct Flow<'a> {
    wb: &'a Workbook,
    sh: &'a Sheet,
    pages: Vec<Vec<Op>>,
    y: f32,
    left: f32,
    width: f32,
    top: f32,
    bottom: f32,
    /// Fit Width: tables and diagrams as wide as the page.
    fit: bool,
}

impl Flow<'_> {
    fn push(&mut self, op: Op) {
        if let Some(page) = self.pages.last_mut() {
            page.push(op);
        }
    }

    /// Starts a new page unless `h` more points fit on this one (or it is still empty).
    fn need(&mut self, h: f32) {
        if self.y + h > self.bottom && self.y > self.top + 0.5 && self.pages.len() < MAX_PAGES {
            self.pages.push(Vec::new());
            self.y = self.top;
        }
    }

    fn gap(&mut self, h: f32) {
        if self.y > self.top + 0.5 {
            self.y += h;
        }
    }

    fn block(&mut self, b: &Block, indent: f32, color: Rgb) {
        match b {
            Block::Heading { level, text, .. } => {
                let size = match level {
                    1 => 20.0,
                    2 => 16.0,
                    3 => 13.5,
                    4 => 12.0,
                    _ => 11.0,
                };
                self.gap(if *level <= 2 { 10.0 } else { 6.0 });
                // A heading keeps the first lines of what follows it on its page.
                self.need(size * LEADING + BODY * LEADING * 2.0);
                self.paragraph(text, indent, size, true, color);
                if *level <= 2 {
                    let y = self.y + 1.0;
                    self.push(Op::Line { x0: self.left + indent, y0: y, x1: self.left + self.width, y1: y, width: 0.6, color: RULE });
                    self.y += 4.0;
                }
                self.y += 2.0;
            }
            Block::Paragraph { text, .. } if text.iter().any(|s| s.style.image) && text.iter().all(|s| s.style.image || s.text.trim().is_empty()) => {
                for s in text.iter().filter(|s| s.style.image) {
                    self.picture(s, indent);
                }
                self.y += 4.0;
            }
            Block::Paragraph { text, .. } => {
                self.paragraph(text, indent, BODY, false, color);
                self.y += 4.0;
            }
            Block::List { start, items, .. } => {
                for (i, item) in items.iter().enumerate() {
                    self.item(item, start.map(|s| s.saturating_add(i as u64)), indent, color);
                }
                self.y += 2.0;
            }
            Block::Quote { blocks, .. } => {
                let (page, top) = (self.pages.len(), self.y);
                for b in blocks {
                    self.block(b, indent + 14.0, DIM);
                }
                self.bar(page, top, self.left + indent + 3.0);
                self.y += 2.0;
            }
            Block::Code { lang, text, .. } => {
                if lang == "mermaid" && self.diagram(text, indent) {
                    self.y += 6.0;
                } else {
                    self.code(text, indent, true, monun_markdown::code::lang_of(lang));
                    self.y += 6.0;
                }
            }
            Block::Table { align, head, rows, .. } => {
                self.table(align, head, rows, indent);
                self.y += 6.0;
            }
            Block::Rule { .. } => {
                self.need(10.0);
                let y = self.y + 5.0;
                self.push(Op::Line { x0: self.left + indent, y0: y, x1: self.left + self.width, y1: y, width: 0.8, color: RULE });
                self.y += 10.0;
            }
        }
    }

    /// The pieces of a run of spans: fonts by style, fields replaced by their values.
    fn pieces(&self, spans: &[Span], size: f32, bold: bool, color: Rgb) -> Vec<Piece> {
        spans
            .iter()
            .map(|s| {
                let text = if s.style.field {
                    crate::docs::field_value(self.wb, &s.text).unwrap_or_else(|e| e)
                } else if s.style.image {
                    format!("[{}]", s.text)
                } else {
                    s.text.clone()
                };
                let font = if s.style.code { Font::Courier } else { Font::styled(monun_pdf::Family::Sans, bold || s.style.bold, s.style.italic) };
                let web = s.link.as_deref().filter(|l| ["http://", "https://", "mailto:"].iter().any(|p| l.to_ascii_lowercase().starts_with(p)));
                Piece {
                    text,
                    font,
                    size: if s.style.code { size * 0.92 } else { size },
                    color: if s.link.is_some() {
                        LINK
                    } else if s.style.image {
                        DIM
                    } else {
                        color
                    },
                    link: web.map(str::to_owned),
                    shade: if s.style.mark {
                        Some(MARK)
                    } else if s.style.code || s.style.field {
                        Some(SHADE)
                    } else {
                        None
                    },
                    strike: s.style.strike,
                    underline: s.style.underline,
                }
            })
            .collect()
    }

    fn paragraph(&mut self, spans: &[Span], indent: f32, size: f32, bold: bool, color: Rgb) {
        let pieces = self.pieces(spans, size, bold, color);
        self.lines(pieces, indent);
    }

    /// Lays out pieces as wrapped lines from `indent`; a `"\n"` piece breaks the line.
    fn lines(&mut self, pieces: Vec<Piece>, indent: f32) {
        let max = (self.width - indent).max(24.0);
        let mut line: Vec<Piece> = Vec::new();
        let mut used = 0.0;
        for piece in pieces {
            if piece.text == "\n" {
                self.emit(std::mem::take(&mut line), indent, piece.size);
                used = 0.0;
                continue;
            }
            for word in words(&piece) {
                let w = word.width();
                let bare = text_width(word.font, word.size, word.text.trim_end());
                if used + bare > max && !line.is_empty() {
                    self.emit(std::mem::take(&mut line), indent, word.size);
                    used = 0.0;
                }
                let word = if line.is_empty() { Piece { text: word.text.trim_start().to_owned(), ..word } } else { word };
                if word.width() > max {
                    // A word wider than the line: cut it where it fills a line.
                    for chunk in cut(&word, max) {
                        if !line.is_empty() {
                            self.emit(std::mem::take(&mut line), indent, chunk.size);
                        }
                        used = chunk.width();
                        line.push(chunk);
                    }
                    continue;
                }
                used += if line.is_empty() { word.width() } else { w };
                line.push(word);
            }
        }
        if !line.is_empty() {
            self.emit(line, indent, BODY);
        }
    }

    /// Draws one line of pieces and moves down past it.
    fn emit(&mut self, line: Vec<Piece>, indent: f32, empty: f32) {
        let size = line.iter().map(|p| p.size).fold(0.0, f32::max);
        let size = if size > 0.0 { size } else { empty };
        let h = size * LEADING;
        self.need(h);
        let base = self.y + size * 1.05;
        let mut x = self.left + indent;
        for p in runs(line) {
            x += self.piece(p, x, base);
        }
        self.y += h;
    }

    /// Draws a piece with its band, lines and link at `x` on baseline `base`; its width.
    fn piece(&mut self, p: Piece, x: f32, base: f32) -> f32 {
        let w = p.width();
        if let Some(color) = p.shade {
            self.push(Op::Fill { x: x - 1.0, y: base - p.size * 0.85, w: w + 2.0, h: p.size * 1.15, color });
        }
        if p.strike {
            let y = base - p.size * 0.3;
            self.push(Op::Line { x0: x, y0: y, x1: x + w, y1: y, width: 0.6, color: p.color });
        }
        if p.underline || p.link.is_some() {
            self.push(Op::Line { x0: x, y0: base + 1.2, x1: x + w, y1: base + 1.2, width: 0.5, color: p.color });
        }
        if let Some(uri) = &p.link {
            self.push(Op::Link { x, y: base - p.size, w, h: p.size * 1.25, uri: uri.clone() });
        }
        self.push(Op::Text { x, y: base, size: p.size, font: p.font, color: p.color, text: p.text });
        w
    }

    fn item(&mut self, item: &Item, number: Option<u64>, indent: f32, color: Rgb) {
        let inner = indent + 18.0;
        self.need(BODY * LEADING);
        let base = self.y + BODY * 1.05;
        let x = self.left + indent;
        match (item.task, number) {
            (Some(task), _) => {
                let (s, top) = (7.5, base - 7.0);
                self.push(Op::Frame { x: x + 2.0, y: top, w: s, h: s, color: DIM });
                if task.done {
                    self.push(Op::Line { x0: x + 3.5, y0: top + 3.8, x1: x + 5.5, y1: top + 6.0, width: 1.1, color: INK });
                    self.push(Op::Line { x0: x + 5.5, y0: top + 6.0, x1: x + 8.8, y1: top + 1.4, width: 1.1, color: INK });
                }
            }
            (None, Some(n)) => {
                let label = format!("{n}.");
                let w = text_width(Font::Helvetica, BODY, &label);
                self.push(Op::Text { x: x + 14.0 - w, y: base, size: BODY, font: Font::Helvetica, color, text: label });
            }
            (None, None) => self.push(Op::Text { x: x + 4.0, y: base, size: BODY, font: Font::Helvetica, color, text: "•".into() }),
        }
        for b in &item.blocks {
            self.block(b, inner, color);
        }
    }

    /// A quote's bar from `(page, top)` down to here, on every page between.
    fn bar(&mut self, page: usize, top: f32, x: f32) {
        let last = self.pages.len();
        for p in page..=last {
            let from = if p == page { top } else { self.top };
            let to = if p == last { self.y } else { self.bottom };
            if to > from
                && let Some(ops) = self.pages.get_mut(p.saturating_sub(1))
            {
                ops.push(Op::Fill { x, y: from, w: 2.5, h: to - from, color: RULE });
            }
        }
    }

    /// Code, line for line in a monospace font, long lines cut, coloured as `lang` reads (as on
    /// screen, in the light theme's colours); `shade` puts it on a grey band (a code block in a
    /// document, not a SQL or text file's own text).
    fn code(&mut self, text: &str, indent: f32, shade: bool, lang: Lang) {
        let max = (self.width - indent - 12.0).max(24.0);
        let lead = CODE * 1.35;
        let text = text.trim_end_matches('\n');
        let spans = monun_markdown::code::highlight(text, lang);
        for row in code_rows(text, &spans, max) {
            self.need(lead);
            if shade {
                self.push(Op::Fill { x: self.left + indent, y: self.y, w: self.width - indent, h: lead, color: SHADE });
            }
            let mut x = self.left + indent + 6.0;
            for (run, color) in row {
                let w = text_width(Font::Courier, CODE, &run);
                self.push(Op::Text { x, y: self.y + CODE * 1.0, size: CODE, font: Font::Courier, color, text: run });
                x += w;
            }
            self.y += lead;
        }
    }

    /// A Mermaid diagram as a picture; false when this build draws none (or it has an error).
    #[cfg(feature = "diagrams")]
    fn diagram(&mut self, source: &str, indent: f32) -> bool {
        let draw = |scale| monun_markdown::diagram::render(source, monun_markdown::diagram::Options { dark: false, scale });
        let Ok(image) = draw(2.0) else { return false };
        // An SVG unit is about 0.75 points. With Fit Width the diagram is centred, never grown to
        // the page: a narrow flowchart as wide as the page would print huge.
        let natural = (image.size.0 * 0.75, image.size.1 * 0.75);
        let rgba = unpremultiply(image.rgba);
        self.place(Arc::new(rgba), [image.width, image.height], natural, indent, self.fit);
        true
    }

    #[cfg(not(feature = "diagrams"))]
    fn diagram(&mut self, _source: &str, _indent: f32) -> bool {
        false
    }

    /// A picture of the document tab, at its size narrowed to fit.
    fn picture(&mut self, span: &Span, indent: f32) {
        let Some(image) = span.link.as_deref().and_then(|l| crate::docs::picture(self.sh, l)) else {
            let piece = Piece {
                text: format!("[{}]", span.text),
                font: Font::Helvetica,
                size: BODY,
                color: DIM,
                link: None,
                shade: None,
                strike: false,
                underline: false,
            };
            self.lines(vec![piece], indent);
            return;
        };
        let natural = (image.anchor.width.max(1.0) * 0.75, image.anchor.height.max(1.0) * 0.75);
        let Some((rgba, w, h)) = super::objects::decode_image(&image.data, 1600, 1600) else { return };
        self.place(Arc::new(rgba), [w, h], natural, indent, false);
    }

    /// Places an image of `natural` size (points), narrowed to the line and the page, at the left
    /// or `centre`d.
    fn place(&mut self, rgba: Arc<Vec<u8>>, size: [u32; 2], natural: (f32, f32), indent: f32, centre: bool) {
        let (mut w, mut h) = natural;
        let room = self.width - indent;
        if w > room {
            h *= room / w;
            w = room;
        }
        let tall = self.bottom - self.top;
        if h > tall {
            w *= tall / h;
            h = tall;
        }
        self.need(h + 2.0);
        let x = self.left + indent + if centre { ((room - w) / 2.0).max(0.0) } else { 0.0 };
        self.push(Op::Image { rect: Rect::new(x, self.y, w, h), rgba, size });
        self.y += h + 2.0;
    }

    fn table(&mut self, align: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], indent: f32) {
        let cols = rows.iter().map(Vec::len).chain(std::iter::once(head.len())).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let pad = 4.0;
        let cells = |row: &[Vec<Span>], bold: bool| -> Vec<Vec<Piece>> {
            (0..cols).map(|c| row.get(c).map(|spans| self.pieces(spans, BODY, bold, INK)).unwrap_or_default()).collect()
        };
        let head_cells = cells(head, true);
        let body: Vec<Vec<Vec<Piece>>> = rows.iter().map(|r| cells(r, false)).collect();
        // Columns as wide as their widest cell, narrowed to the line (the narrow ones keep theirs).
        let natural: Vec<f32> = (0..cols)
            .map(|c| {
                std::iter::once(&head_cells)
                    .chain(body.iter())
                    .filter_map(|row| row.get(c))
                    // Measured as `wrap` measures it, word by word and line by line (a `<br>` breaks
                    // one), with a point to spare for rounding.
                    .map(|cell| {
                        cell.split(|p| p.text == "\n").map(|line| line.iter().flat_map(words).map(|w| w.width()).sum::<f32>()).fold(0.0, f32::max)
                            + pad * 2.0
                            + 1.0
                    })
                    .fold(24.0, f32::max)
            })
            .collect();
        let widths = if self.fit { monun_markdown::fill_columns } else { monun_markdown::fit_columns }(&natural, self.width - indent, 24.0);
        let x0 = self.left + indent;
        let total: f32 = widths.iter().sum();
        let line_h = BODY * LEADING;
        // A row's cells wrapped to their columns, and its height.
        let lay_out = |row: &[Vec<Piece>]| -> (Vec<Vec<Vec<Piece>>>, f32) {
            let laid: Vec<Vec<Vec<Piece>>> = row.iter().zip(&widths).map(|(cell, w)| wrap(cell.clone(), w - pad * 2.0)).collect();
            let h = laid.iter().map(Vec::len).max().unwrap_or(1).max(1) as f32 * line_h + pad * 1.5;
            (laid, h)
        };
        // A row with its lines: the header shaded and centred, the rest aligned by their column.
        let draw_row = |flow: &mut Flow, (laid, h): (Vec<Vec<Vec<Piece>>>, f32), header: bool| {
            flow.need(h);
            if header {
                flow.push(Op::Fill { x: x0, y: flow.y, w: total, h, color: HEAD });
            }
            let mut x = x0;
            for ((lines, w), c) in laid.into_iter().zip(&widths).zip(0..) {
                let mut y = flow.y + (h - lines.len() as f32 * line_h) / 2.0;
                let column = if header { Align::Center } else { align.get(c).copied().unwrap_or_default() };
                for line in lines {
                    // Aligned without the space after its last word.
                    let mut line = runs(line);
                    if let Some(last) = line.last_mut() {
                        let end = last.text.trim_end().len();
                        last.text.truncate(end);
                    }
                    let lw: f32 = line.iter().map(Piece::width).sum();
                    let mut lx = match column {
                        Align::Right => x + w - pad - lw,
                        Align::Center => x + (w - lw) / 2.0,
                        _ => x + pad,
                    };
                    for p in line {
                        lx += flow.piece(p, lx, y + BODY * 1.05);
                    }
                    y += line_h;
                }
                x += w;
            }
            let (top, bottom) = (flow.y, flow.y + h);
            flow.push(Op::Line { x0, y0: top, x1: x0 + total, y1: top, width: 0.5, color: RULE });
            flow.push(Op::Line { x0, y0: bottom, x1: x0 + total, y1: bottom, width: if header { 1.0 } else { 0.5 }, color: RULE });
            let mut x = x0;
            for w in std::iter::once(&0.0).chain(widths.iter()) {
                x += w;
                flow.push(Op::Line { x0: x, y0: top, x1: x, y1: bottom, width: 0.5, color: RULE });
            }
            flow.y = bottom;
        };
        draw_row(self, lay_out(&head_cells), true);
        for row in &body {
            let row = lay_out(row);
            let page = self.pages.len();
            self.need(row.1);
            if self.pages.len() > page {
                // The table goes on over a page: its header row comes again first.
                draw_row(self, lay_out(&head_cells), true);
            }
            draw_row(self, row, false);
        }
    }
}

/// A line's words, those in the same look joined into one run.
fn runs(line: Vec<Piece>) -> Vec<Piece> {
    let mut runs: Vec<Piece> = Vec::with_capacity(line.len());
    for p in line {
        match runs.last_mut() {
            Some(last)
                if last.font == p.font
                    && last.size == p.size
                    && last.color == p.color
                    && last.link == p.link
                    && last.shade == p.shade
                    && last.strike == p.strike
                    && last.underline == p.underline =>
            {
                last.text.push_str(&p.text);
            }
            _ => runs.push(p),
        }
    }
    runs
}

/// A piece cut into words, each with the spaces after it.
fn words(piece: &Piece) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut start = 0;
    let text = piece.text.as_str();
    let mut in_space = false;
    for (i, c) in text.char_indices() {
        if c == ' ' {
            in_space = true;
        } else if in_space {
            if let Some(word) = text.get(start..i) {
                out.push(Piece { text: word.to_owned(), ..piece.clone() });
            }
            start = i;
            in_space = false;
        }
    }
    if let Some(word) = text.get(start..).filter(|w| !w.is_empty()) {
        out.push(Piece { text: word.to_owned(), ..piece.clone() });
    }
    out
}

/// Code's printed rows, each its coloured runs: its lines (tabs as four spaces), a line wider than
/// `max` cut where it fills a row. `spans` are the code's coloured stretches (bytes, in order).
fn code_rows(text: &str, spans: &[(std::ops::Range<usize>, Token)], max: f32) -> Vec<Vec<(String, Rgb)>> {
    let mut rows = Vec::new();
    let mut next = 0;
    let mut at = 0;
    for line in text.split('\n') {
        let mut row: Vec<(String, Rgb)> = Vec::new();
        let mut used = 0.0;
        for (i, c) in line.char_indices() {
            let byte = at + i;
            while spans.get(next).is_some_and(|(r, _)| r.end <= byte) {
                next += 1;
            }
            let color = spans.get(next).filter(|(r, _)| r.start <= byte).map_or(INK, |(_, t)| syntax(*t));
            let s = if c == '\t' { "    ".to_string() } else { c.to_string() };
            let w = text_width(Font::Courier, CODE, &s);
            if used + w > max && used > 0.0 {
                rows.push(std::mem::take(&mut row));
                used = 0.0;
            }
            used += w;
            match row.last_mut() {
                Some((run, last)) if *last == color => run.push_str(&s),
                _ => row.push((s, color)),
            }
        }
        rows.push(row);
        at += line.len() + 1;
    }
    rows
}

/// Code's colours on paper: the light theme's.
fn syntax(token: Token) -> Rgb {
    match token {
        Token::Keyword => [0x0B, 0x57, 0xB3],
        Token::String => [0xA3, 0x36, 0x1B],
        Token::Number => [0x15, 0x7A, 0x5A],
        Token::Comment => [0x6B, 0x72, 0x80],
        Token::Builtin => [0x7B, 0x3F, 0xB5],
    }
}

/// A piece cut into chunks no wider than `max`.
fn cut(piece: &Piece, max: f32) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut chunk = String::new();
    for c in piece.text.chars() {
        chunk.push(c);
        if text_width(piece.font, piece.size, &chunk) > max && chunk.chars().count() > 1 {
            chunk.pop();
            out.push(Piece { text: std::mem::take(&mut chunk), ..piece.clone() });
            chunk.push(c);
        }
    }
    if !chunk.is_empty() {
        out.push(Piece { text: chunk, ..piece.clone() });
    }
    out
}

/// A cell's pieces wrapped to `max`: its lines.
fn wrap(pieces: Vec<Piece>, max: f32) -> Vec<Vec<Piece>> {
    let mut lines: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut used = 0.0;
    for piece in &pieces {
        // A line break (a `<br>` in a table cell) starts a line.
        if piece.text == "\n" {
            lines.push(Vec::new());
            used = 0.0;
            continue;
        }
        for word in words(piece) {
            let w = word.width();
            if used + w > max && lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(Vec::new());
                used = 0.0;
            }
            let chunks = if w > max { cut(&word, max) } else { vec![word] };
            for chunk in chunks {
                let cw = chunk.width();
                if used + cw > max && lines.last().is_some_and(|l| !l.is_empty()) {
                    lines.push(Vec::new());
                    used = 0.0;
                }
                used += cw;
                if let Some(line) = lines.last_mut() {
                    line.push(chunk);
                }
            }
        }
    }
    lines
}

/// Straight alpha from premultiplied pixels (the PDF writer takes straight alpha).
#[cfg(feature = "diagrams")]
fn unpremultiply(mut rgba: Vec<u8>) -> Vec<u8> {
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0.iter_mut() {
        if *a > 0 && *a < 255 {
            let a = u16::from(*a);
            for c in [r, g, b] {
                *c = ((u16::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    rgba
}
