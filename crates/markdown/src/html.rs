//! The HTML a Markdown document may hold: a `<br>` in a table cell, `<b>`…`</b>`, a
//! `<!-- … -->` note, a `<details>` or `<table>` block. Markdown allows raw HTML, and GitHub,
//! Obsidian and Notion show it rather than its source. [`tokens`] reads it into tags and text
//! (comments and declarations dropped, anything that isn't a well-formed tag left as text), and
//! the builder turns the tags into styles, breaks and blocks: inside a paragraph or table cell
//! ([`Builder::inline_html`]) block tags only break the line, while an HTML block
//! ([`Builder::html_block`]) becomes the paragraphs, headings, lists, quotes and table it holds.

use crate::{Align, Block, Builder, Frame, Item, Leaf, Span, Style, TableBuild};

/// What an HTML tag does to the text inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Bold,
    Italic,
    Strike,
    Underline,
    Mark,
    Code,
    Link,
}

fn effect(tag: &str) -> Option<Effect> {
    Some(match tag {
        "b" | "strong" => Effect::Bold,
        "i" | "em" | "cite" | "dfn" | "var" => Effect::Italic,
        "s" | "del" | "strike" => Effect::Strike,
        "u" | "ins" => Effect::Underline,
        "mark" => Effect::Mark,
        "code" | "kbd" | "samp" | "tt" => Effect::Code,
        "a" => Effect::Link,
        _ => return None,
    })
}

/// Tags that start a block of their own: a line break inside a paragraph or cell.
fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "html"
            | "legend"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "search"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

/// Tags whose content isn't shown.
fn is_hidden(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "template" | "head" | "title")
}

fn is_table_part(tag: &str) -> bool {
    matches!(tag, "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th")
}

/// `h1`…`h6`'s level.
fn heading_tag(tag: &str) -> Option<u8> {
    tag.strip_prefix('h').and_then(|n| n.parse().ok()).filter(|n| (1..=6).contains(n))
}

/// A table cell's alignment, from its `align` or its `style="text-align: …"`.
fn cell_align(attrs: &[(String, String)]) -> Align {
    let style = attr(attrs, "style").map(str::to_ascii_lowercase);
    let from_style = style
        .as_deref()
        .and_then(|s| s.split(';').find_map(|d| d.split_once(':').filter(|(k, _)| k.trim() == "text-align").map(|(_, v)| v.trim().to_owned())));
    match attr(attrs, "align").map(str::to_ascii_lowercase).or(from_style).as_deref() {
        Some("left") => Align::Left,
        Some("center") => Align::Center,
        Some("right") => Align::Right,
        _ => Align::None,
    }
}

/// An `<ol>`'s first number.
fn list_start(attrs: &[(String, String)]) -> u64 {
    attr(attrs, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1)
}

/// The `<table>` an HTML block is filling (its rows and cells go in [`TableBuild`]).
#[derive(Default)]
pub(crate) struct HtmlTable {
    /// Inside `<thead>`.
    in_head: bool,
    row_open: bool,
    cell_open: bool,
    /// The open row has `<th>` cells, `<td>` ones.
    row_th: bool,
    row_td: bool,
    /// Tables inside this one, whose rows and cells are lines of its cell.
    nested: u32,
}

/// The pictures `<img src="…">` of a document's HTML, each once, in order.
pub fn img_sources(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if !text.contains("<img") && !text.contains("<IMG") {
        return out;
    }
    for (_, token) in tokens(text) {
        if let Token::Open { name, attrs } = token
            && name == "img"
            && let Some(src) = attr(&attrs, "src").map(str::trim).filter(|s| !s.is_empty())
            && !out.iter().any(|s| s == src)
        {
            out.push(src.to_owned());
        }
    }
    out
}

impl Builder<'_> {
    /// Closes the HTML tags still open that style the text.
    pub(crate) fn close_tags(&mut self) {
        while let Some((_, effect)) = self.open_tags.pop() {
            self.undo(effect);
        }
    }

    fn count(&mut self, effect: Effect) -> Option<&mut u32> {
        match effect {
            Effect::Bold => Some(&mut self.bold),
            Effect::Italic => Some(&mut self.italic),
            Effect::Strike => Some(&mut self.strike),
            Effect::Underline => Some(&mut self.underline),
            Effect::Mark => Some(&mut self.mark),
            Effect::Code => Some(&mut self.code_tags),
            Effect::Link => None,
        }
    }

    fn undo(&mut self, effect: Effect) {
        match self.count(effect) {
            Some(n) => *n = n.saturating_sub(1),
            None => {
                self.links.pop();
            }
        }
    }

    /// Closes the innermost open tag `name` that styles the text.
    fn close_style(&mut self, name: &str) {
        if let Some(at) = self.open_tags.iter().rposition(|(n, _)| n == name) {
            let (_, effect) = self.open_tags.remove(at);
            self.undo(effect);
        }
    }

    /// Raw HTML in a paragraph, heading or table cell (`<br>`, `<b>`, `<!-- -->`): each tag
    /// styles the text after it or breaks the line; what isn't a tag shows as text.
    pub(crate) fn inline_html(&mut self, raw: &str, line: usize) {
        for (_, token) in tokens(raw) {
            match token {
                Token::Text(t) => self.text(&t, false, line),
                Token::Open { name, attrs } => self.open_tag(&name, &attrs, line),
                Token::Close { name } => self.close_tag(&name, line),
            }
        }
    }

    /// Whether a block tag goes to the HTML block's layout ([`Self::open_block`]) rather than
    /// breaking the line: in an HTML block outside a table cell, and a table's own tags.
    fn lays_out(&self, tag: &str) -> bool {
        self.html_depth.is_some() && (!matches!(self.leaf, Some(Leaf::Cell)) || is_table_part(tag))
    }

    fn open_tag(&mut self, name: &str, attrs: &[(String, String)], line: usize) {
        if let Some(effect) = effect(name) {
            if effect == Effect::Link {
                let Some(href) = attr(attrs, "href").map(str::trim).filter(|h| !h.is_empty()) else { return };
                self.links.push(href.to_owned());
            }
            if let Some(n) = self.count(effect) {
                *n += 1;
            }
            self.open_tags.push((name.to_owned(), effect));
            return;
        }
        match name {
            "br" => self.line_break(line),
            "img" => self.picture(attrs, line),
            "input" if attr(attrs, "type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox")) => {
                let mark = if attr(attrs, "checked").is_some() { "☑" } else { "☐" };
                self.text(mark, false, line);
            }
            _ if self.lays_out(name) => self.open_block(name, attrs, line),
            "ul" => {
                self.break_line(line);
                self.leaf_lists.push(None);
            }
            "ol" => {
                self.break_line(line);
                self.leaf_lists.push(Some(list_start(attrs)));
            }
            "li" => {
                self.break_line(line);
                let mark = match self.leaf_lists.last_mut() {
                    Some(Some(n)) => {
                        let mark = format!("{n}. ");
                        *n = n.saturating_add(1);
                        mark
                    }
                    _ => "• ".to_owned(),
                };
                self.text(&mark, false, line);
            }
            _ if is_block(name) => self.break_line(line),
            _ => {}
        }
    }

    fn close_tag(&mut self, name: &str, line: usize) {
        if effect(name).is_some() {
            self.close_style(name);
            return;
        }
        match name {
            // `</br>` breaks the line in a browser too.
            "br" => self.line_break(line),
            _ if self.lays_out(name) => self.close_block(name),
            "ul" | "ol" => {
                self.leaf_lists.pop();
                self.break_line(line);
            }
            _ if is_block(name) => self.break_line(line),
            _ => {}
        }
    }

    /// A `<br>`: a new line in the paragraph or cell (none outside one). One starting a line of
    /// the source breaks it once with the line break before it, as on GitHub.
    fn line_break(&mut self, line: usize) {
        if self.soft && self.spans.last().is_some_and(|s| s.text == "\n") {
            self.soft = false;
        } else if self.leaf.is_some() {
            self.text("\n", false, line);
        }
    }

    /// Starts a new line in the leaf, unless it is empty or already at a line's start.
    fn break_line(&mut self, line: usize) {
        if self.spans.last().is_some_and(|s| s.text != "\n") {
            self.text("\n", false, line);
        }
    }

    /// An `<img>`: a picture in the leaf, described by its `alt` (or its file's name).
    fn picture(&mut self, attrs: &[(String, String)], line: usize) {
        let Some(src) = attr(attrs, "src").map(str::trim).filter(|s| !s.is_empty()) else { return };
        let alt = attr(attrs, "alt").map(str::trim).filter(|a| !a.is_empty()).unwrap_or_else(|| src.rsplit(['/', '\\']).next().unwrap_or(src));
        if self.leaf.is_none() {
            self.leaf = Some(Leaf::Implicit(line));
        }
        let style = Style { image: true, ..self.style(false) };
        self.spans.push(Span { text: alt.to_owned(), style, link: Some(src.to_owned()) });
    }

    /// An HTML block, laid out as a browser shows it: its text in paragraphs with white space run
    /// together, its headings, rules, lists, quotes, tables and `<pre>` as those blocks. What it
    /// leaves open closes at its end.
    pub(crate) fn html_block(&mut self, html: &str, first: usize) {
        let depth = self.stack.len();
        self.html_depth = Some(depth);
        let mut line = first;
        let mut counted = 0;
        let mut hidden = 0u32;
        let mut pre: Option<(String, usize)> = None;
        for (at, token) in tokens(html) {
            line += html.get(counted..at).map_or(0, |s| s.matches('\n').count());
            counted = counted.max(at);
            if hidden > 0 {
                match &token {
                    Token::Open { name, .. } if is_hidden(name) => hidden += 1,
                    Token::Close { name } if is_hidden(name) => hidden -= 1,
                    _ => {}
                }
                continue;
            }
            if let Some((text, _)) = pre.as_mut() {
                match token {
                    Token::Text(t) => text.push_str(&t),
                    Token::Open { name, .. } if name == "br" => text.push('\n'),
                    Token::Close { name } if name == "pre" => self.end_pre(pre.take()),
                    _ => {}
                }
                continue;
            }
            // Between a table's cells only its tags count.
            let gap = self.html_table.as_ref().is_some_and(|t| !t.cell_open);
            match token {
                Token::Text(t) if !gap => {
                    // Its words start on the line after the breaks it begins with.
                    let lead = t.get(..t.len() - t.trim_start().len()).map_or(0, |s| s.matches('\n').count());
                    self.html_text(&t, line + lead);
                }
                Token::Text(_) => {}
                Token::Open { name, .. } if is_hidden(&name) => hidden += 1,
                Token::Open { name, .. } if name == "pre" && self.html_table.is_none() => {
                    self.end_leaf();
                    pre = Some((String::new(), line));
                }
                Token::Open { name, .. } if gap && matches!(name.as_str(), "br" | "img" | "input") => {}
                Token::Open { name, attrs } => self.open_tag(&name, &attrs, line),
                Token::Close { name } => self.close_tag(&name, line),
            }
        }
        self.end_pre(pre);
        self.end_leaf();
        self.end_html_table();
        while self.stack.len() > depth {
            let before = self.stack.len();
            self.close_frame();
            if self.stack.len() >= before {
                break;
            }
        }
        self.close_tags();
        self.html_depth = None;
    }

    /// Text of an HTML block: runs of white space as one space, none at a line's start.
    fn html_text(&mut self, text: &str, line: usize) {
        let mut out = String::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_ascii_whitespace() {
                space = true;
            } else {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
        }
        if space {
            out.push(' ');
        }
        let fresh = self.spans.last().is_none_or(|s| s.text == "\n" || s.text.ends_with(' '));
        let out = if fresh { out.trim_start_matches(' ') } else { out.as_str() };
        if !out.is_empty() {
            self.text(out, false, line);
        }
    }

    fn end_pre(&mut self, pre: Option<(String, usize)>) {
        let Some((text, line)) = pre else { return };
        // A browser drops the break right after `<pre>`.
        let text = text.strip_prefix("\r\n").or_else(|| text.strip_prefix('\n')).unwrap_or(&text);
        let mut text = text.trim_end().to_owned();
        if text.is_empty() {
            return;
        }
        text.push('\n');
        self.push(Block::Code { lang: String::new(), text, line });
    }

    /// A block tag of an HTML block opening (outside a table cell, or a table's own).
    fn open_block(&mut self, name: &str, attrs: &[(String, String)], line: usize) {
        if let Some(level) = heading_tag(name) {
            self.end_leaf();
            self.leaf = Some(Leaf::Heading(level, line));
            return;
        }
        let nested = self.html_table.as_ref().is_some_and(|t| t.nested > 0);
        match name {
            "hr" => {
                self.end_leaf();
                self.push(Block::Rule { line });
            }
            "ul" | "ol" | "menu" => {
                self.end_leaf();
                let start = (name == "ol").then(|| list_start(attrs));
                self.stack.push(Frame::List(start, Vec::new(), line));
            }
            "li" => {
                self.end_leaf();
                // A new item ends the one before it.
                let innermost = self.stack.iter().rev().find(|f| matches!(f, Frame::List(..) | Frame::Item(_)));
                if matches!(innermost, Some(Frame::Item(_))) {
                    self.close_through(|f| matches!(f, Frame::Item(_)));
                }
                if !matches!(self.stack.last(), Some(Frame::List(..))) {
                    self.stack.push(Frame::List(None, Vec::new(), line));
                }
                self.stack.push(Frame::Item(Item { task: None, blocks: Vec::new(), line }));
            }
            "blockquote" => {
                self.end_leaf();
                self.stack.push(Frame::Quote(Vec::new(), line));
            }
            "summary" => {
                self.end_leaf();
                self.bold += 1;
                self.open_tags.push(("summary".to_owned(), Effect::Bold));
            }
            "table" => {
                if let Some(t) = self.html_table.as_mut() {
                    t.nested += 1;
                    self.break_line(line);
                    return;
                }
                self.end_leaf();
                self.table = Some(TableBuild { line, ..TableBuild::default() });
                self.html_table = Some(HtmlTable::default());
            }
            // A table inside a cell: its rows are lines of the cell.
            "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" if nested => {
                if name == "tr" {
                    self.break_line(line);
                }
            }
            "thead" | "tbody" | "tfoot" if self.html_table.is_some() => {
                self.end_row();
                if let Some(t) = self.html_table.as_mut() {
                    t.in_head = name == "thead";
                }
            }
            "tr" if self.html_table.is_some() => self.start_row(),
            "td" | "th" if self.html_table.is_some() => {
                self.end_cell();
                if !self.html_table.as_ref().is_some_and(|t| t.row_open) {
                    self.start_row();
                }
                if let Some(t) = self.table.as_mut()
                    && t.head.is_empty()
                    && t.rows.is_empty()
                {
                    t.align.push(cell_align(attrs));
                }
                if let Some(t) = self.html_table.as_mut() {
                    t.cell_open = true;
                    t.row_th |= name == "th";
                    t.row_td |= name == "td";
                }
                self.spans.clear();
                self.leaf = Some(Leaf::Cell);
            }
            _ if is_block(name) => self.end_leaf(),
            _ => {}
        }
    }

    /// A block tag of an HTML block closing (outside a table cell, or a table's own).
    fn close_block(&mut self, name: &str) {
        if heading_tag(name).is_some() {
            self.end_leaf();
            return;
        }
        let nested = self.html_table.as_ref().is_some_and(|t| t.nested > 0);
        match name {
            "ul" | "ol" | "menu" => {
                self.end_leaf();
                self.close_through(|f| matches!(f, Frame::List(..)));
            }
            "li" => {
                self.end_leaf();
                self.close_through(|f| matches!(f, Frame::Item(_)));
            }
            "blockquote" => {
                self.end_leaf();
                self.close_through(|f| matches!(f, Frame::Quote(..)));
            }
            "summary" => {
                self.end_leaf();
                self.close_style("summary");
            }
            "table" if nested => {
                if let Some(t) = self.html_table.as_mut() {
                    t.nested -= 1;
                }
            }
            "table" => self.end_html_table(),
            "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" if nested => {}
            "tr" | "thead" | "tbody" | "tfoot" => {
                self.end_row();
                if let Some(t) = self.html_table.as_mut() {
                    t.in_head = false;
                }
            }
            "td" | "th" => self.end_cell(),
            _ if is_block(name) => self.end_leaf(),
            _ => {}
        }
    }

    /// Ends the paragraph, heading or cell an HTML block has open.
    fn end_leaf(&mut self) {
        match self.leaf {
            Some(Leaf::Implicit(_)) => self.flush_implicit(),
            Some(Leaf::Heading(level, line)) => {
                self.leaf = None;
                let text = self.take_spans();
                if !text.is_empty() {
                    self.push(Block::Heading { level, text, line });
                }
            }
            Some(Leaf::Paragraph(line)) => {
                self.leaf = None;
                let text = self.take_spans();
                if !text.is_empty() {
                    self.push(Block::Paragraph { text, line });
                }
            }
            Some(Leaf::Cell) => self.end_cell(),
            None => {}
        }
    }

    fn start_row(&mut self) {
        self.end_row();
        if let Some(t) = self.html_table.as_mut() {
            t.row_open = true;
            t.row_th = false;
            t.row_td = false;
        }
        if let Some(t) = self.table.as_mut() {
            t.row.clear();
        }
    }

    fn end_cell(&mut self) {
        let Some(t) = self.html_table.as_mut().filter(|t| t.cell_open) else { return };
        t.cell_open = false;
        self.leaf = None;
        // A cell closes the tags open in it.
        self.close_tags();
        let cell = self.take_spans();
        if let Some(t) = self.table.as_mut() {
            t.row.push(cell);
        }
    }

    fn end_row(&mut self) {
        self.end_cell();
        let Some(t) = self.html_table.as_mut().filter(|t| t.row_open) else { return };
        t.row_open = false;
        let head = t.in_head || (t.row_th && !t.row_td);
        if let Some(t) = self.table.as_mut() {
            let row = std::mem::take(&mut t.row);
            if head && t.head.is_empty() && t.rows.is_empty() {
                t.head = row;
            } else {
                t.rows.push(row);
            }
        }
    }

    /// Ends an HTML block's table; one without a header row takes its first row as the header.
    fn end_html_table(&mut self) {
        self.end_row();
        if self.html_table.take().is_none() {
            return;
        }
        let Some(mut t) = self.table.take() else { return };
        if t.head.is_empty() && !t.rows.is_empty() {
            t.head = t.rows.remove(0);
        }
        // A table with no text at all (`<table><tr><td>` closed by a blank line, its content
        // Markdown after it) is left out.
        let empty = std::iter::once(&t.head).chain(&t.rows).flatten().all(Vec::is_empty);
        if !empty {
            self.push(Block::Table { align: t.align, head: t.head, rows: t.rows, line: t.line });
        }
    }

    /// Closes the containers the HTML block opened, down to and including the innermost one `is`
    /// picks; nothing when that one was open before the block.
    fn close_through(&mut self, is: impl Fn(&Frame) -> bool) {
        let Some(at) = self.stack.iter().rposition(is) else { return };
        if self.html_depth.is_none_or(|depth| at < depth) {
            return;
        }
        while self.stack.len() > at {
            let before = self.stack.len();
            self.close_frame();
            if self.stack.len() >= before {
                break;
            }
        }
    }

    /// Closes the innermost container, putting what it holds in the one around it (an empty list
    /// or quote is left out, as is a list of empty items: `<ul><li>` closed by a blank line).
    fn close_frame(&mut self) {
        match self.pop() {
            Some(Frame::Quote(blocks, line)) => {
                if !blocks.is_empty() {
                    self.push(Block::Quote { blocks, line });
                }
            }
            Some(Frame::List(start, items, line)) => {
                if items.iter().any(|i| !i.blocks.is_empty()) {
                    self.push(Block::List { start, items, line });
                }
            }
            Some(Frame::Item(item)) => match self.stack.last_mut() {
                Some(Frame::List(_, items, _)) => items.push(item),
                _ => {
                    for b in item.blocks {
                        self.push(b);
                    }
                }
            },
            Some(Frame::Root(blocks)) => {
                for b in blocks {
                    self.push(b);
                }
            }
            None => {}
        }
    }
}

/// One piece of HTML.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Token {
    /// `<name attr="value">` (or `<name/>`): the name and attribute names in lower case, the
    /// values with their entities decoded.
    Open { name: String, attrs: Vec<(String, String)> },
    /// `</name>`.
    Close { name: String },
    /// Text between tags, its entities decoded (white space as it is).
    Text(String),
}

/// The tags and text of `html`, each with the byte offset it starts at.
pub(crate) fn tokens(html: &str) -> Vec<(usize, Token)> {
    let mut out = Vec::new();
    let mut text_from = 0;
    let mut i = 0;
    while let Some(rel) = html.get(i..).and_then(|rest| rest.find('<')) {
        let lt = i + rel;
        let Some((len, tag)) = html.get(lt..).and_then(tag) else {
            i = lt + 1;
            continue;
        };
        if let Some(text) = html.get(text_from..lt).filter(|t| !t.is_empty()) {
            out.push((text_from, Token::Text(decode(text))));
        }
        if let Some(tag) = tag {
            out.push((lt, tag));
        }
        i = lt + len;
        text_from = i;
    }
    if let Some(text) = html.get(text_from..).filter(|t| !t.is_empty()) {
        out.push((text_from, Token::Text(decode(text))));
    }
    out
}

/// The value of attribute `name` (in lower case) of a tag.
pub(crate) fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// The tag `s` starts with: its length in bytes and the tag, which is `None` for a comment, a
/// declaration (`<!DOCTYPE …>`) or a processing instruction. `None` when `s` doesn't start with
/// a tag.
fn tag(s: &str) -> Option<(usize, Option<Token>)> {
    if let Some(body) = s.strip_prefix("<!--") {
        // An unclosed comment runs to the end, as an HTML block of one does.
        let len = body.find("-->").map_or(s.len(), |end| 4 + end + 3);
        return Some((len, None));
    }
    if s.starts_with("<!") || s.starts_with("<?") {
        return Some((s.find('>')? + 1, None));
    }
    let (close, body) = match s.strip_prefix("</") {
        Some(body) => (true, body),
        None => (false, s.get(1..)?),
    };
    if !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_len = body.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(body.len());
    let name = body.get(..name_len)?.to_ascii_lowercase();
    let rest = body.get(name_len..)?;
    // The tag ends at the first `>` outside a quoted value.
    let mut quote = None;
    let mut end = None;
    for (j, c) in rest.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                end = Some(j);
                break;
            }
            (None, '<') => return None,
            _ => {}
        }
    }
    let end = end?;
    let inside = rest.get(..end)?;
    // A name runs straight into white space, `/` or `>`; `<a+b>` is text.
    if !inside.is_empty() && !inside.starts_with(|c: char| c.is_ascii_whitespace() || c == '/') {
        return None;
    }
    let len = s.len() - body.len() + name_len + end + 1;
    let token = if close { Token::Close { name } } else { Token::Open { name, attrs: attrs(inside) } };
    Some((len, Some(token)))
}

/// The attributes of a tag's inside (`href="…" width=300 hidden`).
fn attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
    while !rest.is_empty() {
        let before = rest.len();
        let n = rest.find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/').unwrap_or(rest.len());
        let name = rest.get(..n).unwrap_or("").to_ascii_lowercase();
        rest = rest.get(n..).unwrap_or("").trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (raw, left) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let inner = after.get(1..).unwrap_or("");
                    match inner.find(q) {
                        Some(e) => (inner.get(..e).unwrap_or(""), inner.get(e + 1..).unwrap_or("")),
                        None => (inner, ""),
                    }
                }
                _ => {
                    let e = after.find(|c: char| c.is_ascii_whitespace()).unwrap_or(after.len());
                    (after.get(..e).unwrap_or(""), after.get(e..).unwrap_or(""))
                }
            };
            value = decode(raw);
            rest = left;
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.len() >= before {
            break;
        }
    }
    out
}

/// `s` with its character references (`&nbsp;`, `&amp;`, `&#8594;`, `&#x2192;`) decoded; one it
/// doesn't know stays as it is.
pub(crate) fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or(""));
        let after = rest.get(amp + 1..).unwrap_or("");
        // A reference's name is short: its `;` is looked for only that far.
        let semi = after.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i);
        let found = semi.and_then(|end| Some((end, entity(after.get(..end)?)?)));
        match found {
            Some((end, c)) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character a reference names (without its `&` and `;`).
fn entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "nbsp" => '\u{a0}',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "middot" => '·',
        "bull" => '•',
        "hellip" => '…',
        "ndash" => '–',
        "mdash" => '—',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "lArr" => '⇐',
        "rArr" => '⇒',
        "hArr" => '⇔',
        "times" => '×',
        "divide" => '÷',
        "plusmn" => '±',
        "deg" => '°',
        "le" => '≤',
        "ge" => '≥',
        "ne" => '≠',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "sect" => '§',
        "para" => '¶',
        "check" => '✓',
        "cross" => '✗',
        "star" => '☆',
        "starf" => '★',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str, attrs: &[(&str, &str)]) -> Token {
        Token::Open { name: name.to_owned(), attrs: attrs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect() }
    }

    #[test]
    fn tags_text_and_comments() {
        let found: Vec<Token> = tokens("a<br>b<BR/><br />  <!-- 메모 -->c</B><!DOCTYPE html>").into_iter().map(|(_, t)| t).collect();
        assert_eq!(
            found,
            [
                Token::Text("a".into()),
                open("br", &[]),
                Token::Text("b".into()),
                open("br", &[]),
                open("br", &[]),
                Token::Text("  ".into()),
                Token::Text("c".into()),
                Token::Close { name: "b".into() },
            ]
        );
    }

    #[test]
    fn attributes_quoted_bare_and_bare_names() {
        let found = tokens(r#"<a href="https://x.kr/?a=1&amp;b=2" title='말 "인용"' target=_blank hidden>"#);
        assert_eq!(found, [(0, open("a", &[("href", "https://x.kr/?a=1&b=2"), ("title", "말 \"인용\""), ("target", "_blank"), ("hidden", "")]))]);
        // A `>` inside a quoted value doesn't end the tag.
        assert_eq!(tokens(r#"<img alt="a>b" src=p.png>"#), [(0, open("img", &[("alt", "a>b"), ("src", "p.png")]))]);
    }

    #[test]
    fn what_is_not_a_tag_stays_text() {
        for s in ["a < b", "<3", "Vec<T", "<a+b>", "<>", "</>", "<기본값>", "<"] {
            let text: String = tokens(s)
                .into_iter()
                .map(|(_, t)| match t {
                    Token::Text(t) => t,
                    other => panic!("{s}: {other:?}"),
                })
                .collect();
            assert_eq!(text, s);
        }
    }

    #[test]
    fn offsets_and_unclosed_comments() {
        let found = tokens("가<b>나</b><!-- 끝 없음");
        assert_eq!(found.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [0, 3, 6, 9]);
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn character_references() {
        assert_eq!(decode("a&nbsp;b &amp; &lt;x&gt; &#8594; &#x2192; &rarr; &bogus; & &#0; &#xZZ;"), "a\u{a0}b & <x> → → → &bogus; & &#0; &#xZZ;");
        assert_eq!(decode("평문"), "평문");
    }
}
