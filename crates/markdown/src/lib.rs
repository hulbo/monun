//! Markdown of Monun's document tabs.
//!
//! [`parse`] turns a document's text into [`Block`]s for the preview to lay out. Every block
//! knows the source line it starts on, so a click in the preview finds its line in the editor,
//! and every task-list item knows where its `[ ]` box is, so [`toggle_task`] can tick it in the
//! text. The syntax is CommonMark with GitHub's tables, strikethrough and task lists. A single
//! line break inside a paragraph stays a line break, as in Obsidian and most note apps, rather
//! than becoming a space. HTML in the text shows as GitHub and Obsidian show it, not as its
//! source: `<br>` breaks the line (in a table cell too), `<b>`, `<i>`, `<u>`, `<s>`, `<mark>`,
//! `<code>` and `<kbd>` style their text, `<a href>` and `<img>` are a link and a picture,
//! comments are hidden, and an HTML block (`<details>`, `<div>`, `<table>`, `<ul>`, `<h2>`, `<p>`)
//! becomes the paragraphs, headings, lists and table it holds ([`html`]).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[cfg(feature = "diagrams")]
pub mod diagram;
mod html;

pub use html::img_sources;

/// A block of a document, with the zero-based source line it starts on.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        text: Vec<Span>,
        line: usize,
    },
    Paragraph {
        text: Vec<Span>,
        line: usize,
    },
    /// A bulleted list (`start` is `None`) or a numbered one starting at `start`.
    List {
        start: Option<u64>,
        items: Vec<Item>,
        line: usize,
    },
    Quote {
        blocks: Vec<Block>,
        line: usize,
    },
    /// A fenced or indented code block; `lang` is the fence's first word (`mermaid`, `rust`).
    Code {
        lang: String,
        text: String,
        line: usize,
    },
    Table {
        align: Vec<Align>,
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
        line: usize,
    },
    Rule {
        line: usize,
    },
}

impl Block {
    pub fn line(&self) -> usize {
        match self {
            Block::Heading { line, .. }
            | Block::Paragraph { line, .. }
            | Block::List { line, .. }
            | Block::Quote { line, .. }
            | Block::Code { line, .. }
            | Block::Table { line, .. }
            | Block::Rule { line } => *line,
        }
    }
}

/// One list item: its blocks, and its checkbox when it is a task.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub task: Option<Task>,
    pub blocks: Vec<Block>,
    pub line: usize,
}

/// A task-list checkbox: whether it is ticked, and the byte offset of its `[` in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub done: bool,
    pub at: usize,
}

/// A run of text in one style. A line break is a span of `"\n"`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// Where the run links to (`https://…`, `#Sheet1!A1`), for a link or an image.
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Underlined (`<u>`, `<ins>`).
    pub underline: bool,
    /// Highlighted (`<mark>`).
    pub mark: bool,
    pub code: bool,
    /// The run is an image's alternative text (the preview shows no pictures yet).
    pub image: bool,
    /// The run is a `{{…}}` field: its text is the expression (`구매목록!D1`, `=SUM(B:B)`) whose
    /// value shows in its place.
    pub field: bool,
}

/// A table column's alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// The blocks of `text`. Front matter (a YAML block between `---` lines at the very top, as
/// documents written for static sites and by AI tools begin) is the document's data, not its
/// text: it shows in the source, not in the view.
pub fn parse(text: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut b = Builder::new(text);
    let mut front_matter = false;
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(Tag::MetadataBlock(_)) => front_matter = true,
            Event::End(TagEnd::MetadataBlock(_)) => front_matter = false,
            _ if front_matter => {}
            event => b.event(event, range.start),
        }
    }
    b.finish()
}

/// `text` with the task box at byte `at` ticked or cleared, or `None` when there is no box there.
pub fn toggle_task(text: &str, at: usize) -> Option<String> {
    let mark = text.get(at..at.checked_add(3)?)?;
    let flipped = match mark {
        "[ ]" => "[x]",
        "[x]" | "[X]" => "[ ]",
        _ => return None,
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(text.get(..at)?);
    out.push_str(flipped);
    out.push_str(text.get(at + 3..)?);
    Some(out)
}

/// The byte offset where zero-based `line` starts in `text` (the end of the text past the last line).
pub fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(i, _)| i + 1)
}

/// The zero-based line holding byte `offset` of `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes().iter().take(offset).filter(|b| **b == b'\n').count()
}

/// A container the builder is filling.
enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>, usize),
    List(Option<u64>, Vec<Item>, usize),
    Item(Item),
}

/// What the current run of spans belongs to.
#[derive(Clone, Copy)]
enum Leaf {
    Paragraph(usize),
    Heading(u8, usize),
    /// Text of a tight list item, which comes without a paragraph around it.
    Implicit(usize),
    Cell,
}

#[derive(Default)]
struct TableBuild {
    align: Vec<Align>,
    head: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
    line: usize,
}

struct Builder<'a> {
    src: &'a str,
    /// Byte offsets where lines start.
    starts: Vec<usize>,
    stack: Vec<Frame>,
    spans: Vec<Span>,
    leaf: Option<Leaf>,
    bold: u32,
    italic: u32,
    strike: u32,
    underline: u32,
    mark: u32,
    /// `<code>`, `<kbd>` and the like open around the text.
    code_tags: u32,
    image: u32,
    links: Vec<String>,
    code: Option<(String, String, usize)>,
    table: Option<TableBuild>,
    /// The HTML tags open that style the text, innermost last.
    open_tags: Vec<(String, html::Effect)>,
    /// The `<ul>` and `<ol>` open inside a paragraph or cell: the next item's number in a
    /// numbered one.
    leaf_lists: Vec<Option<u64>>,
    /// An HTML block being read: its text and the line it starts on.
    html: Option<(String, usize)>,
    /// While an HTML block is laid out: how many containers were open before it (it closes
    /// only its own).
    html_depth: Option<usize>,
    html_table: Option<html::HtmlTable>,
    /// The leaf's last run is a line break of the source, which a `<br>` right after it doesn't
    /// add to.
    soft: bool,
}

impl<'a> Builder<'a> {
    fn new(src: &'a str) -> Self {
        let starts = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
        Builder {
            src,
            starts,
            stack: vec![Frame::Root(Vec::new())],
            spans: Vec::new(),
            leaf: None,
            bold: 0,
            italic: 0,
            strike: 0,
            underline: 0,
            mark: 0,
            code_tags: 0,
            image: 0,
            links: Vec::new(),
            code: None,
            table: None,
            open_tags: Vec::new(),
            leaf_lists: Vec::new(),
            html: None,
            html_depth: None,
            html_table: None,
            soft: false,
        }
    }

    fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|s| *s <= offset).saturating_sub(1)
    }

    fn event(&mut self, event: Event, at: usize) {
        let line = self.line(at);
        match event {
            Event::Start(tag) => self.start(tag, line),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some((_, code, _)) = self.code.as_mut() {
                    code.push_str(&t);
                } else if let Some((html, _)) = self.html.as_mut() {
                    // An indented HTML block's indent comes as text.
                    html.push_str(&t);
                } else {
                    self.text(&t, false, line);
                }
            }
            Event::Code(t) => self.text(&t, true, line),
            Event::InlineMath(t) | Event::DisplayMath(t) | Event::FootnoteReference(t) => self.text(&t, false, line),
            Event::InlineHtml(t) => self.inline_html(&t, line),
            Event::Html(t) => {
                if let Some((_, code, _)) = self.code.as_mut() {
                    code.push_str(&t);
                } else if let Some((html, _)) = self.html.as_mut() {
                    html.push_str(&t);
                } else {
                    self.inline_html(&t, line);
                }
            }
            // A `<br>` ending the line has broken it already (GitHub shows one break, not two).
            Event::SoftBreak if self.spans.last().is_some_and(|s| s.text == "\n") => {}
            Event::SoftBreak => {
                self.text("\n", false, line);
                self.soft = true;
            }
            Event::HardBreak => self.text("\n", false, line),
            Event::Rule => {
                self.flush_implicit();
                self.push(Block::Rule { line });
            }
            Event::TaskListMarker(done) => {
                // The marker's range starts at its `[`.
                let at = self.src.get(at..).and_then(|rest| rest.find('[')).map_or(at, |i| at + i);
                if let Some(Frame::Item(item)) = self.stack.last_mut() {
                    item.task = Some(Task { done, at });
                }
            }
        }
    }

    fn start(&mut self, tag: Tag, line: usize) {
        match tag {
            Tag::Paragraph => {
                self.flush_implicit();
                self.leaf = Some(Leaf::Paragraph(line));
            }
            Tag::Heading { level, .. } => {
                self.flush_implicit();
                self.leaf = Some(Leaf::Heading(heading_level(level), line));
            }
            Tag::BlockQuote(_) => {
                self.flush_implicit();
                self.stack.push(Frame::Quote(Vec::new(), line));
            }
            Tag::CodeBlock(kind) => {
                self.flush_implicit();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or_default().to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new(), line));
            }
            Tag::HtmlBlock => {
                self.flush_implicit();
                self.html = Some((String::new(), line));
            }
            Tag::List(start) => {
                self.flush_implicit();
                self.stack.push(Frame::List(start, Vec::new(), line));
            }
            Tag::Item => {
                self.flush_implicit();
                self.stack.push(Frame::Item(Item { task: None, blocks: Vec::new(), line }));
            }
            Tag::Table(align) => {
                self.flush_implicit();
                let align = align
                    .into_iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                self.table = Some(TableBuild { align, line, ..TableBuild::default() });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.row.clear();
                }
            }
            Tag::TableCell => {
                self.spans.clear();
                self.leaf = Some(Leaf::Cell);
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(dest_url.to_string()),
            Tag::Image { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.image += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if let Some(Leaf::Paragraph(line)) = self.leaf.take() {
                    let text = self.take_spans();
                    self.push(Block::Paragraph { text, line });
                }
            }
            TagEnd::Heading(_) => {
                if let Some(Leaf::Heading(level, line)) = self.leaf.take() {
                    let text = self.take_spans();
                    self.push(Block::Heading { level, text, line });
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush_implicit();
                if let Some(Frame::Quote(blocks, line)) = self.pop() {
                    self.push(Block::Quote { blocks, line });
                }
            }
            TagEnd::CodeBlock => {
                if let Some((lang, text, line)) = self.code.take() {
                    self.push(Block::Code { lang, text, line });
                }
            }
            TagEnd::HtmlBlock => {
                if let Some((html, line)) = self.html.take() {
                    self.html_block(&html, line);
                }
            }
            TagEnd::List(_) => {
                self.flush_implicit();
                if let Some(Frame::List(start, items, line)) = self.pop() {
                    self.push(Block::List { start, items, line });
                }
            }
            TagEnd::Item => {
                self.flush_implicit();
                if let Some(Frame::Item(item)) = self.pop()
                    && let Some(Frame::List(_, items, _)) = self.stack.last_mut()
                {
                    items.push(item);
                }
            }
            TagEnd::TableCell => {
                self.leaf = None;
                let cell = self.take_spans();
                if let Some(t) = self.table.as_mut() {
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.head = std::mem::take(&mut t.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.push(Block::Table { align: t.align, head: t.head, rows: t.rows, line: t.line });
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => {
                self.links.pop();
                self.image = self.image.saturating_sub(1);
            }
            _ => {}
        }
    }

    /// The look of text added now.
    fn style(&self, code: bool) -> Style {
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            underline: self.underline > 0,
            mark: self.mark > 0,
            code: code || self.code_tags > 0,
            image: self.image > 0,
            field: false,
        }
    }

    /// Adds a run of text to the current leaf, opening an implicit paragraph for the bare text of
    /// a tight list item (or of an HTML block).
    fn text(&mut self, text: &str, code: bool, line: usize) {
        let brk = text == "\n";
        // A line starts without the spaces after the `<br>` that broke it.
        let text = if !code && !brk && self.spans.last().is_some_and(|s| s.text == "\n") { text.trim_start_matches(' ') } else { text };
        if text.is_empty() {
            return;
        }
        if self.leaf.is_none() {
            self.leaf = Some(Leaf::Implicit(line));
        }
        let style = if brk { Style { code: false, ..self.style(false) } } else { self.style(code) };
        let link = self.links.last().cloned();
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == link && text != "\n" && last.text != "\n" => last.text.push_str(text),
            _ => self.spans.push(Span { text: text.to_owned(), style, link }),
        }
        self.soft = false;
    }

    fn flush_implicit(&mut self) {
        if let Some(Leaf::Implicit(line)) = self.leaf {
            self.leaf = None;
            let text = self.take_spans();
            if !text.is_empty() {
                self.push(Block::Paragraph { text, line });
            }
        } else if self.leaf.is_none() && self.html_depth.is_none() {
            // Tags opened in a list item without text (`- <b>`) end with it.
            self.close_tags();
        }
    }

    /// The finished leaf's runs: a break or space at its end dropped (a `<br>` closing a
    /// paragraph adds no line, as in a browser), its fields split out. In Markdown the HTML
    /// tags left open in a leaf close with it.
    fn take_spans(&mut self) -> Vec<Span> {
        if self.html_depth.is_none() {
            self.close_tags();
        }
        self.leaf_lists.clear();
        let mut spans = std::mem::take(&mut self.spans);
        while spans.last().is_some_and(|s| !s.style.code && !s.style.image && s.text.trim_matches([' ', '\n']).is_empty()) {
            spans.pop();
        }
        if let Some(last) = spans.last_mut().filter(|s| !s.style.code && !s.style.image) {
            let end = last.text.trim_end_matches([' ', '\n']).len();
            last.text.truncate(end);
        }
        split_fields(spans)
    }

    fn push(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(Frame::Root(blocks) | Frame::Quote(blocks, _)) => blocks.push(block),
            Some(Frame::Item(item)) => item.blocks.push(block),
            // Blocks never come straight inside a list; keep one that does as its own item.
            Some(Frame::List(_, items, _)) => {
                let line = block.line();
                items.push(Item { task: None, blocks: vec![block], line });
            }
            None => self.stack.push(Frame::Root(vec![block])),
        }
    }

    /// Pops a container, never the root.
    fn pop(&mut self) -> Option<Frame> {
        if self.stack.len() > 1 { self.stack.pop() } else { None }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush_implicit();
        // Close whatever the parser left open, innermost first.
        while self.stack.len() > 1 {
            match self.stack.pop() {
                Some(Frame::Quote(blocks, line)) => self.push(Block::Quote { blocks, line }),
                Some(Frame::List(start, items, line)) => self.push(Block::List { start, items, line }),
                Some(Frame::Item(item)) => {
                    if let Some(Frame::List(_, items, _)) = self.stack.last_mut() {
                        items.push(item);
                    }
                }
                Some(Frame::Root(blocks)) => {
                    for b in blocks {
                        self.push(b);
                    }
                }
                None => break,
            }
        }
        match self.stack.pop() {
            Some(Frame::Root(blocks)) => blocks,
            _ => Vec::new(),
        }
    }
}

/// The `{{…}}` fields of `blocks`, each expression once, in order.
pub fn fields(blocks: &[Block]) -> Vec<String> {
    fn spans<'a>(out: &mut Vec<String>, spans: impl IntoIterator<Item = &'a Span>) {
        for s in spans {
            if s.style.field && !out.contains(&s.text) {
                out.push(s.text.clone());
            }
        }
    }
    fn walk(out: &mut Vec<String>, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Heading { text, .. } | Block::Paragraph { text, .. } => spans(out, text),
                Block::List { items, .. } => items.iter().for_each(|i| walk(out, &i.blocks)),
                Block::Quote { blocks, .. } => walk(out, blocks),
                Block::Table { head, rows, .. } => {
                    head.iter().chain(rows.iter().flatten()).for_each(|cell| spans(out, cell));
                }
                Block::Code { .. } | Block::Rule { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&mut out, blocks);
    out
}

/// A table's column widths in `room`: each column as wide as its widest cell (`natural`) when
/// they all fit. Otherwise the columns narrower than an even share keep their width and the
/// others share what is left by their width, none below `floor` (so the table can still be wider
/// than `room` when there are many columns).
pub fn fit_columns(natural: &[f32], room: f32, floor: f32) -> Vec<f32> {
    let natural: Vec<f32> = natural.iter().map(|w| if w.is_finite() { w.max(0.0) } else { 0.0 }).collect();
    if natural.iter().sum::<f32>() <= room {
        return natural;
    }
    let mut kept = vec![false; natural.len()];
    let mut left = room;
    loop {
        let open = kept.iter().filter(|k| !**k).count();
        if open == 0 {
            break;
        }
        let share = left / open as f32;
        let mut changed = false;
        for (w, k) in natural.iter().zip(kept.iter_mut()) {
            if !*k && *w <= share {
                *k = true;
                left -= w;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let wide: f32 = natural.iter().zip(&kept).filter(|(_, k)| !**k).map(|(w, _)| w).sum();
    natural.iter().zip(&kept).map(|(w, k)| if *k || wide <= 0.0 { *w } else { (w * left.max(0.0) / wide).max(floor) }).collect()
}

/// [`fit_columns`] widened to fill `room` when the columns are narrower, each by its share of the
/// width, so the table is as wide as the page.
pub fn fill_columns(natural: &[f32], room: f32, floor: f32) -> Vec<f32> {
    let widths = fit_columns(natural, room, floor);
    let total: f32 = widths.iter().sum();
    if total <= 0.0 || total >= room || !room.is_finite() {
        return widths;
    }
    widths.iter().map(|w| w * room / total).collect()
}

/// Splits the `{{…}}` fields out of a leaf's runs. Code keeps its braces, and so does an empty
/// or unclosed pair.
fn split_fields(spans: Vec<Span>) -> Vec<Span> {
    if !spans.iter().any(|s| !s.style.code && s.text.contains("{{")) {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + 2);
    for s in spans {
        if s.style.code || !s.text.contains("{{") {
            out.push(s);
            continue;
        }
        let piece = |text: &str, field: bool| Span { text: text.to_owned(), style: Style { field, ..s.style }, link: s.link.clone() };
        let mut rest = s.text.as_str();
        while let Some(open) = rest.find("{{") {
            let Some(after) = rest.get(open + 2..) else { break };
            let Some(close) = after.find("}}") else { break };
            let expr = after.get(..close).unwrap_or("").trim();
            if expr.is_empty() || expr.contains('\n') {
                out.push(piece(rest.get(..open + 2 + close + 2).unwrap_or(rest), false));
            } else {
                if let Some(before) = rest.get(..open).filter(|b| !b.is_empty()) {
                    out.push(piece(before, false));
                }
                out.push(piece(expr, true));
            }
            rest = after.get(close + 2..).unwrap_or("");
        }
        if !rest.is_empty() {
            out.push(piece(rest, false));
        }
    }
    out
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests;
