//! Mermaid diagrams (```` ```mermaid ```` blocks) drawn to pixels: mermaid-rs-renderer lays the
//! diagram out as SVG, measuring its labels with the system's fonts, and resvg paints it. Both are
//! compiled in, so nothing is downloaded. A panic inside them comes back as an error.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};

use mermaid_rs_renderer::{DiagramKind, RenderOptions, Theme};

mod layered;
mod sequence;
use resvg::{tiny_skia, usvg};

/// The largest side of a drawn diagram, in pixels.
const MAX_SIDE: f32 = 4096.0;

/// A drawn diagram: `width` × `height` pixels of premultiplied RGBA, and the diagram's own size
/// in SVG units (CSS pixels), whatever the scale it was drawn at.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub size: (f32, f32),
}

/// How to draw a diagram.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The dark theme's colours.
    pub dark: bool,
    /// Pixels per SVG unit (the zoom times the screen's pixels per point).
    pub scale: f32,
}

/// The font labels are measured and drawn in: one that has Hangul.
pub fn font_family() -> &'static str {
    if cfg!(target_os = "windows") {
        "Malgun Gothic"
    } else if cfg!(target_os = "macos") {
        "Apple SD Gothic Neo"
    } else {
        "Noto Sans CJK KR"
    }
}

/// The system's fonts, read once (the first diagram waits for it).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.set_sans_serif_family(font_family());
            Arc::new(db)
        })
        .clone()
}

/// The SVG of a Mermaid diagram, or why it can't be drawn (its syntax error).
pub fn svg(source: &str, dark: bool) -> Result<String, String> {
    let mut options = RenderOptions::default();
    if dark {
        options.theme = Theme::dark();
    } else {
        // The default theme's slate-400 boxes and slate-500 lines read faint beside the document's
        // black text: a shade darker, as on paper.
        options.theme.primary_border_color = "#475569".to_owned();
        options.theme.line_color = "#334155".to_owned();
        options.theme.cluster_border = "#94A3B8".to_owned();
    }
    options.theme.font_family = font_family().to_owned();
    match std::panic::catch_unwind(AssertUnwindSafe(|| draw(source, &options))) {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("The diagram could not be drawn.".to_owned()),
    }
}

/// The renderer's own steps (parse, lay out, write SVG), a flowchart or an ER diagram laid out as
/// Mermaid lays it out ([`layered`]): `|"text"|` without its quotes, and each label on its arrow
/// over a plain background.
fn draw(source: &str, options: &RenderOptions) -> Result<String, mermaid_rs_renderer::ParseError> {
    let (layout, config, kind) = laid_out(source, options)?;
    let svg = cylinders(&mermaid_rs_renderer::render_svg(&layout, &options.theme, &config));
    let svg = if matches!(kind, DiagramKind::Flowchart | DiagramKind::Er) { without_label_borders(&svg) } else { svg };
    Ok(styled_text(&svg))
}

/// The diagram parsed and laid out, with the layout settings it was laid out with, and its kind.
fn laid_out(
    source: &str,
    options: &RenderOptions,
) -> Result<(mermaid_rs_renderer::Layout, mermaid_rs_renderer::LayoutConfig, DiagramKind), mermaid_rs_renderer::ParseError> {
    // A flowchart's chains an arrow a line, as the renderer reads them whole that way.
    let split = if is_flowchart(source) { split_chains(source) } else { None };
    let parsed = match split.as_deref().map(mermaid_rs_renderer::parse_mermaid_strict) {
        Some(Ok(parsed)) => parsed,
        _ => mermaid_rs_renderer::parse_mermaid_strict(source)?,
    };
    let mut graph = parsed.graph;
    let flowchart = graph.kind == DiagramKind::Flowchart;
    let er = graph.kind == DiagramKind::Er;
    // A label's inline HTML (`<b>`, `<i>`…) as marks made into styled text once drawn, then `&lt;`,
    // `#quot;` and the like as the characters they stand for, as Mermaid shows them (the renderer
    // escaped them again: `List&lt;Dto&gt;` showed so). In that order, `&lt;b&gt;` shows as written.
    for node in graph.nodes.values_mut() {
        node.label = entities(&inline_html(&node.label));
    }
    for sub in &mut graph.subgraphs {
        sub.label = entities(&inline_html(&sub.label));
    }
    for edge in &mut graph.edges {
        for label in [&mut edge.label, &mut edge.start_label, &mut edge.end_label].into_iter().flatten() {
            *label = entities(&inline_html(label));
            if flowchart || er {
                *label = unquote(label);
            }
        }
    }
    let mut config = options.layout.clone();
    if flowchart {
        // A flowchart's boxes hug their text as Mermaid's do (its padding is 15; the renderer's 30
        // made wide drawings, small once fitted to the page).
        config.node_padding_x = FLOWCHART_PADDING_X;
    }
    if er {
        // A table's columns a line each: the renderer wrapped a long one and then read each piece
        // as a column of its own (`NUMBER` alone, then `PK SN`).
        config.max_label_width_chars = config.max_label_width_chars.max(ER_LINE_CHARS);
        for node in graph.nodes.values_mut() {
            node.label = er_comments(&node.label);
        }
    }
    let mut layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &config);
    if er {
        fit_tables(&mut layout, options.theme.font_size, options.theme.font_size * config.label_line_height);
    }
    if flowchart || er {
        layered::relayout(&graph, &mut layout);
    }
    if graph.kind == DiagramKind::Sequence {
        // Rows and frames opened up where they met.
        sequence::tidy(source, &graph, &mut layout, options.theme.font_size);
    }
    if graph.kind == DiagramKind::Kanban {
        kanban(&mut layout);
    }
    readable_text(&mut layout);
    Ok((layout, config, graph.kind))
}

/// The room around a kanban board's cards: within a column, before its title, between the title
/// and the first card, between cards, below the last; and between columns.
const KANBAN_PAD_X: f32 = 10.0;
const KANBAN_TITLE_TOP: f32 = 12.0;
const KANBAN_TITLE_GAP: f32 = 12.0;
const KANBAN_CARD_GAP: f32 = 8.0;
const KANBAN_PAD_BOTTOM: f32 = 10.0;
const KANBAN_COLUMN_GAP: f32 = 12.0;

/// A kanban board laid out as Mermaid draws one: each column's cards as wide as the column, below
/// its title with room between (the renderer started the first card where the title is drawn), a
/// little apart from each other, and every column as tall as the tallest.
fn kanban(layout: &mut mermaid_rs_renderer::Layout) {
    if !layout.edges.is_empty() || layout.subgraphs.is_empty() {
        return;
    }
    let origin = 6.0;
    let mut x = origin;
    let mut bottom = origin;
    // Left to right as the renderer put them (the board's order; its list of columns is in another).
    let mut order: Vec<usize> = (0..layout.subgraphs.len()).collect();
    order.sort_by(|a, b| {
        let at = |i: &usize| layout.subgraphs.get(*i).map_or(0.0, |s| s.x);
        at(a).total_cmp(&at(b))
    });
    for i in order {
        let Some(sub) = layout.subgraphs.get_mut(i) else { continue };
        let ids: Vec<String> = sub.nodes.iter().filter(|id| layout.nodes.contains_key(*id)).cloned().collect();
        if ids.is_empty() {
            continue;
        }
        let widest = ids.iter().filter_map(|id| layout.nodes.get(id)).map(|n| n.width).fold(sub.label_block.width, f32::max);
        let title = if sub.label.trim().is_empty() { 0.0 } else { sub.label_block.height + KANBAN_TITLE_GAP };
        let mut y = origin + KANBAN_TITLE_TOP + title;
        for (i, id) in ids.iter().enumerate() {
            if let Some(n) = layout.nodes.get_mut(id) {
                if i > 0 {
                    y += KANBAN_CARD_GAP;
                }
                (n.x, n.y, n.width) = (x + KANBAN_PAD_X, y, widest);
                y += n.height;
            }
        }
        (sub.x, sub.y, sub.width) = (x, origin, widest + 2.0 * KANBAN_PAD_X);
        sub.height = y + KANBAN_PAD_BOTTOM - origin;
        bottom = bottom.max(y + KANBAN_PAD_BOTTOM);
        x += sub.width + KANBAN_COLUMN_GAP;
    }
    for sub in layout.subgraphs.iter_mut().filter(|s| s.width > 0.0) {
        sub.height = bottom - origin;
    }
    layout.width = x - KANBAN_COLUMN_GAP + origin;
    layout.height = bottom + origin;
}

/// Each table (an ER diagram's entity with columns) as tall as its title and rows as the renderer
/// draws them: it measured the label's lines, its divider among them, and left room below the
/// last row.
fn fit_tables(layout: &mut mermaid_rs_renderer::Layout, font_size: f32, line_height: f32) {
    for node in layout.nodes.values_mut() {
        let lines = &node.label.lines;
        let Some(divider) = lines.iter().position(|l| l.trim() == "---") else { continue };
        let rows = lines.iter().skip(divider + 1).filter(|l| !l.trim().is_empty()).count();
        if rows == 0 {
            continue;
        }
        // The renderer's title row, then a line a column, and a little below.
        let title = (line_height + font_size * 0.6).max(line_height + 6.0);
        let fit = title + rows as f32 * line_height + font_size * 0.5;
        if fit < node.height {
            node.height = fit;
        }
    }
}

/// The longest line of a table (an ER diagram's entity) kept on one line.
const ER_LINE_CHARS: usize = 80;

/// An entity's label (`NAME`, `---`, then a column a line) with each column's comment (`"…"`,
/// which Mermaid shows in a column of its own) in brackets after the column's name, as the
/// renderer shows the rest of the line as the name.
fn er_comments(label: &str) -> String {
    let mut body = false;
    let lines: Vec<String> = label
        .lines()
        .map(|line| {
            if line.trim() == "---" {
                body = true;
            }
            match (body, line.find('"'), line.rfind('"')) {
                (true, Some(open), Some(close)) if close > open => {
                    let comment = line.get(open + 1..close).unwrap_or("").trim();
                    let before = line.get(..open).unwrap_or("").trim_end();
                    let after = line.get(close + 1..).unwrap_or("").trim();
                    let mut out = before.to_owned();
                    if !comment.is_empty() {
                        out.push_str(&format!(" ({comment})"));
                    }
                    if !after.is_empty() {
                        out.push(' ');
                        out.push_str(after);
                    }
                    out
                }
                _ => line.to_owned(),
            }
        })
        .collect();
    lines.join("\n")
}

/// Marks standing for a label's inline HTML formatting while the renderer lays it out (it would
/// draw `<b>` as written): each turns a style on or off, and [`styled_text`] makes them into styled
/// text in the SVG. Private-use characters, so no label has them of its own.
const MARKS: [(char, Style, bool); 8] = [
    ('\u{E000}', Style::Bold, true),
    ('\u{E001}', Style::Bold, false),
    ('\u{E002}', Style::Italic, true),
    ('\u{E003}', Style::Italic, false),
    ('\u{E004}', Style::Underline, true),
    ('\u{E005}', Style::Underline, false),
    ('\u{E006}', Style::Strike, true),
    ('\u{E007}', Style::Strike, false),
];

/// A style a label's inline HTML gives its text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Bold,
    Italic,
    Underline,
    Strike,
}

/// The mark that turns `style` on (`on`) or off.
fn mark(style: Style, on: bool) -> char {
    MARKS.iter().find(|(_, s, o)| *s == style && *o == on).map_or('\u{E000}', |(c, _, _)| *c)
}

/// The inline HTML tags Mermaid's labels allow that only style their text: their text is kept, the
/// tags taken out.
const PLAIN_TAGS: &[&str] = &["span", "font", "small", "big", "sup", "sub", "code", "kbd", "mark", "a", "tt", "var", "cite", "q", "abbr"];

/// `label` with its inline HTML as Mermaid shows it: `<b>`/`<strong>`, `<i>`/`<em>`, `<u>`/`<ins>`
/// and `<s>`/`<del>`/`<strike>` as [`MARKS`]; a line break however written (`<br/>`, `<br />`,
/// `<BR>`) as the renderer's `<br>`; the other formatting tags ([`PLAIN_TAGS`], with or without
/// attributes) taken out, their text kept. Anything else in angle brackets (`List<Dto>`, `<id>`,
/// `a < b > c`) stays as written.
fn inline_html(label: &str) -> String {
    if !label.contains('<') {
        return label.to_owned();
    }
    let mut out = String::with_capacity(label.len());
    let mut rest = label;
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let after = rest.get(open + 1..).unwrap_or("");
        // The tag's inside, up to its `>` (none before another `<`).
        let inside = after.find(['>', '<']).filter(|&i| after.get(i..).is_some_and(|t| t.starts_with('>'))).and_then(|i| after.get(..i));
        let Some(inside) = inside else {
            out.push('<');
            rest = after;
            continue;
        };
        let closing = inside.starts_with('/');
        let body = inside.trim_start_matches('/');
        let name_len = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
        let name = body.get(..name_len).unwrap_or("").to_ascii_lowercase();
        let tail = body.get(name_len..).unwrap_or("");
        // `<b>`, `<b class="x">`, `<br/>`: a name, then nothing, white space or `/`.
        let tag_like = !name.is_empty() && (tail.is_empty() || tail.starts_with(char::is_whitespace) || tail.trim() == "/");
        let style = match name.as_str() {
            "b" | "strong" => Some(Style::Bold),
            "i" | "em" => Some(Style::Italic),
            "u" | "ins" => Some(Style::Underline),
            "s" | "del" | "strike" => Some(Style::Strike),
            _ => None,
        };
        match (tag_like, style) {
            (true, Some(style)) => out.push(mark(style, !closing)),
            (true, None) if name == "br" && !closing => out.push_str("<br>"),
            (true, None) if PLAIN_TAGS.contains(&name.as_str()) => {}
            _ => {
                out.push('<');
                out.push_str(inside);
                out.push('>');
            }
        }
        rest = after.get(inside.len() + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// How far italic text leans: the tangent of 12°, as browsers slant a font without an italic face
/// (the Hangul faces have none, and resvg makes no oblique of its own).
const SLANT: f32 = 0.21;

/// A `<tspan>` opening tag giving its text the styles `on`: bold, a line under or through it, and
/// for italic text in the line laid out (`hidden`) no ink, as the slanted copy draws it.
fn style_span(on: &[Style], hidden: bool) -> String {
    let mut tag = String::from("<tspan");
    if on.contains(&Style::Bold) {
        tag.push_str(" font-weight=\"bold\"");
    }
    if hidden {
        tag.push_str(" fill-opacity=\"0\"");
    }
    let lines: Vec<&str> =
        [(Style::Underline, "underline"), (Style::Strike, "line-through")].iter().filter(|(s, _)| on.contains(s)).map(|(_, d)| *d).collect();
    if !lines.is_empty() && !hidden {
        tag.push_str(&format!(" text-decoration=\"{}\"", lines.join(" ")));
    }
    tag.push('>');
    tag
}

/// A line of a `<text>`: its `<tspan …>` (none for text written straight in the `<text>`), and its
/// pieces of text with the styles each has.
struct Line<'a> {
    tag: Option<&'a str>,
    pieces: Vec<(Vec<Style>, String)>,
}

/// The pieces of `content` (a line's text, [`MARKS`] in it) with the styles on for each, `on`
/// carried from the line before and left as this line ends.
fn pieces(content: &str, on: &mut Vec<Style>) -> Vec<(Vec<Style>, String)> {
    let mut out: Vec<(Vec<Style>, String)> = vec![(on.clone(), String::new())];
    for c in content.chars() {
        match MARKS.iter().find(|(m, _, _)| *m == c) {
            Some((_, style, turn_on)) => {
                on.retain(|s| s != style);
                if *turn_on {
                    on.push(*style);
                }
                out.push((on.clone(), String::new()));
            }
            None => {
                if let Some((_, text)) = out.last_mut() {
                    text.push(c);
                }
            }
        }
    }
    out.retain(|(_, text)| !text.is_empty());
    out
}

/// The lines of a `<text>`'s inside: its `<tspan>`s, or the text itself. `None` when it holds
/// anything else (a `<tspan>` within a `<tspan>`).
fn lines_of(inside: &str) -> Option<Vec<(Option<&str>, &str)>> {
    if !inside.trim_start().starts_with("<tspan") {
        return (!inside.contains('<')).then(|| vec![(None, inside)]);
    }
    let mut out = Vec::new();
    let mut rest = inside.trim_start();
    while !rest.trim().is_empty() {
        let open_end = rest.strip_prefix("<tspan").and_then(|_| rest.find('>'))? + 1;
        let tag = rest.get(..open_end)?;
        let after = rest.get(open_end..)?;
        let close = after.find("</tspan>")?;
        let content = after.get(..close)?;
        if content.contains('<') {
            return None;
        }
        out.push((Some(tag), content));
        rest = after.get(close + "</tspan>".len()..)?.trim_start();
    }
    Some(out)
}

/// A length of an SVG attribute (`12.5`, `12.5px`, `1.15em` of `font_size`).
fn length(value: &str, font_size: f32) -> Option<f32> {
    let value = value.trim();
    if let Some(em) = value.strip_suffix("em") {
        return em.trim().parse::<f32>().ok().map(|v| v * font_size);
    }
    value.trim_end_matches("px").trim().parse::<f32>().ok()
}

/// `tag` (an opening tag) with its attribute `name` set to `value`, added when it has none.
fn with_attr(tag: &str, name: &str, value: &str) -> String {
    let key = format!(" {name}=\"");
    if let Some(at) = tag.find(&key) {
        let from = at + key.len();
        if let Some(len) = tag.get(from..).and_then(|t| t.find('"')) {
            return format!("{}{value}{}", tag.get(..from).unwrap_or(""), tag.get(from + len..).unwrap_or(""));
        }
    }
    let close = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len().saturating_sub(1) };
    format!("{}{key}{value}\"{}", tag.get(..close).unwrap_or(tag), tag.get(close..).unwrap_or(""))
}

/// One `<text>` element (its whole markup) with its [`MARKS`] made into styles. Italic text is drawn
/// by a copy of its line slanted about the line's baseline, the rest of that copy without ink, so
/// each glyph lands where the line laid it out; in the line itself it has no ink.
fn styled_element(element: &str) -> String {
    if !element.contains(|c: char| MARKS.iter().any(|(m, _, _)| *m == c)) {
        return element.to_owned();
    }
    let open_end = element.find('>').map_or(element.len(), |i| i + 1);
    let open = element.get(..open_end).unwrap_or("");
    let inside = element.get(open_end..element.len().saturating_sub("</text>".len())).unwrap_or("");
    let Some(raw) = lines_of(inside) else {
        // Not a layout this knows: the marks dropped, the text kept.
        return element.chars().filter(|c| !MARKS.iter().any(|(m, _, _)| m == c)).collect();
    };
    let mut on = Vec::new();
    let lines: Vec<Line> = raw.into_iter().map(|(tag, content)| Line { tag, pieces: pieces(content, &mut on) }).collect();
    let mut out = String::from(open);
    for line in &lines {
        out.push_str(line.tag.unwrap_or(""));
        for (styles, text) in &line.pieces {
            if styles.is_empty() {
                out.push_str(text);
            } else {
                out.push_str(&style_span(styles, styles.contains(&Style::Italic)));
                out.push_str(text);
                out.push_str("</tspan>");
            }
        }
        if line.tag.is_some() {
            out.push_str("</tspan>");
        }
    }
    out.push_str("</text>");
    // The slanted copies, a line each.
    let font_size = attr(open, "font-size").and_then(|v| length(v, 16.0)).unwrap_or(16.0);
    let mut y = attr(open, "y").and_then(|v| length(v, font_size)).unwrap_or(0.0);
    let text_x = attr(open, "x").unwrap_or("0");
    for line in &lines {
        if let Some(dy) = line.tag.and_then(|t| attr(t, "dy")).and_then(|v| length(v, font_size)) {
            y += dy;
        }
        if !line.pieces.iter().any(|(styles, _)| styles.contains(&Style::Italic)) {
            continue;
        }
        let x = line.tag.and_then(|t| attr(t, "x")).unwrap_or(text_x);
        let copy = with_attr(&with_attr(open, "x", x), "y", &format!("{y:.2}"));
        let copy = with_attr(&copy, "transform", &format!("matrix(1 0 {:.2} 1 {:.2} 0)", -SLANT, SLANT * y));
        out.push_str(&copy);
        for (styles, text) in &line.pieces {
            let italic = styles.contains(&Style::Italic);
            out.push_str(&style_span(styles, !italic));
            out.push_str(text);
            out.push_str("</tspan>");
        }
        out.push_str("</text>");
    }
    out
}

/// The SVG with each label's [`MARKS`] made into styled text ([`styled_element`]): bold
/// (`font-weight`), a line under or through it (`text-decoration`), italic slanted, across the
/// label's lines. A mark anywhere else is dropped.
fn styled_text(svg: &str) -> String {
    let is_mark = |c: char| MARKS.iter().any(|(m, _, _)| *m == c);
    if !svg.contains(is_mark) {
        return svg.to_owned();
    }
    let mut out = String::with_capacity(svg.len() + 256);
    let mut rest = svg;
    // The next `<text …>` (not `<textPath`).
    let next_text = |s: &str| s.match_indices("<text").map(|(i, _)| i).find(|&i| s.get(i + 5..).is_some_and(|t| t.starts_with([' ', '>'])));
    while let Some(start) = next_text(rest) {
        out.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start..).unwrap_or("");
        let Some(end) = tail.find("</text>").map(|i| i + "</text>".len()) else {
            out.push_str(tail);
            rest = "";
            break;
        };
        out.push_str(&styled_element(tail.get(..end).unwrap_or("")));
        rest = tail.get(end..).unwrap_or("");
    }
    out.push_str(rest);
    out.retain(|c| !is_mark(c));
    out
}

/// `text` with its character references decoded: HTML's (`&lt;`, `&amp;`, `&#39;`, `&#x2192;`) and
/// Mermaid's own (`#lt;`, `#quot;`, `#35;`).
fn entities(text: &str) -> String {
    if !text.contains(['&', '#']) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(['&', '#']) {
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at..).unwrap_or("");
        // A reference is `&name;` or `#name;`, short, without spaces.
        let decoded = tail.get(1..).and_then(|t| t.find(';').filter(|&end| end > 0 && end <= 10)).and_then(|end| {
            let name = tail.get(1..=end)?;
            let c = match name {
                "lt" => '<',
                "gt" => '>',
                "amp" => '&',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                // `&#39;` and `&#x2192;`; Mermaid's `#35;`.
                _ => {
                    let number = if tail.starts_with('&') { name.strip_prefix('#')? } else { name };
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse::<u32>().ok()?,
                    };
                    char::from_u32(code).filter(|c| !c.is_control())?
                }
            };
            Some((c, end + 2))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = tail.get(len..).unwrap_or("");
            }
            None => {
                out.push_str(tail.get(..1).unwrap_or(""));
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text on a box or group given a fill (`style X fill:#…`) but no text colour: dark on a light
/// fill, white on a dark one, whichever reads better. A style written for the light theme (pale
/// fills) otherwise had the dark theme's pale text on it, hard to read.
fn readable_text(layout: &mut mermaid_rs_renderer::Layout) {
    let styles = layout.nodes.values_mut().map(|n| &mut n.style).chain(layout.subgraphs.iter_mut().map(|s| &mut s.style));
    for style in styles {
        if style.text_color.is_none()
            && let Some(fill) = style.fill.as_deref().and_then(luminance)
        {
            // Dark text (luminance about 0.02) reads better above about 0.22, white below.
            let text = if fill > 0.22 { "#1E293B" } else { "#F8FAFC" };
            style.text_color = Some(text.to_owned());
        }
    }
}

/// The relative luminance (0 black to 1 white) of a `#rgb` or `#rrggbb` colour.
fn luminance(color: &str) -> Option<f32> {
    let hex = color.trim().strip_prefix('#')?;
    let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(f32::from);
    let (r, g, b) = match hex.len() {
        3 => {
            let one = |i: usize| hex.get(i..=i).and_then(channel).map(|v| v * 17.0);
            (one(0)?, one(1)?, one(2)?)
        }
        6 | 8 => (channel(hex.get(0..2)?)?, channel(hex.get(2..4)?)?, channel(hex.get(4..6)?)?),
        _ => return None,
    };
    let linear = |v: f32| {
        let s = v / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    Some(0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b))
}

/// The SVG with each cylinder (`[(…)]`, a database) drawn as Mermaid draws one. The renderer
/// writes a cylinder as a top ellipse, a rectangle and a bottom ellipse, each with its full
/// outline: a line across the top, a straight bottom and the bottom's back arc all showed. Here it
/// is one outline (the top's back arc, the sides, the bottom's front arc) with the top's front arc
/// over it, and its label (centred on the whole shape) moved down below the top where they met.
fn cylinders(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find("<ellipse ") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        match cylinder_at(from) {
            Some((drawing, used)) => {
                out.push_str(&drawing);
                rest = from.get(used..).unwrap_or_default();
            }
            None => {
                let end = from.find("/>").map_or(from.len(), |e| e + 2);
                out.push_str(from.get(..end).unwrap_or(from));
                rest = from.get(end..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// The tag opening `s` (after any white space) when it starts with `open`, and the bytes up to its
/// end.
fn tag_at<'a>(s: &'a str, open: &str) -> Option<(&'a str, usize)> {
    let lead = s.len() - s.trim_start().len();
    let t = s.get(lead..)?;
    if !t.starts_with(open) {
        return None;
    }
    let end = t.find("/>")? + 2;
    Some((t.get(..end)?, lead + end))
}

/// An attribute's value in a tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let rest = tag.get(at..)?;
    rest.get(..rest.find('"')?)
}

/// The cylinder starting `s`, when its three tags are one: what to write in their place, and the
/// bytes they took.
fn cylinder_at(s: &str) -> Option<(String, usize)> {
    let (top, a) = tag_at(s, "<ellipse ")?;
    let (body, b) = tag_at(s.get(a..)?, "<rect ")?;
    let (bottom, c) = tag_at(s.get(a + b..)?, "<ellipse ")?;
    let num = |t: &str, k: &str| attr(t, k).and_then(|v| v.parse::<f32>().ok());
    let (cx, cy, rx, ry) = (num(top, "cx")?, num(top, "cy")?, num(top, "rx")?, num(top, "ry")?);
    let (x, y, w, h) = (num(body, "x")?, num(body, "y")?, num(body, "width")?, num(body, "height")?);
    let near = |p: f32, q: f32| (p - q).abs() < 0.05;
    let one = near(x, cx - rx) && near(w, 2.0 * rx) && near(y, cy) && near(num(bottom, "cx")?, cx) && near(num(bottom, "cy")?, cy + h);
    if !one || attr(bottom, "fill") != Some("none") {
        return None;
    }
    // The top's fill, and its stroke as written after it.
    let fill = attr(top, "fill")?;
    let after_fill = top.find(" fill=\"")?;
    let strokes = top.get(after_fill..)?.splitn(3, '"').nth(2)?.trim_end_matches("/>").trim();
    let (left, right, bottom_y) = (cx - rx, cx + rx, cy + h);
    let mut drawing = format!(
        "<path d=\"M {left:.2} {cy:.2} A {rx:.2} {ry:.2} 0 0 1 {right:.2} {cy:.2} L {right:.2} {bottom_y:.2} A {rx:.2} {ry:.2} 0 0 1 {left:.2} {bottom_y:.2} Z\" fill=\"{fill}\" {strokes}/>\
         <path d=\"M {left:.2} {cy:.2} A {rx:.2} {ry:.2} 0 0 0 {right:.2} {cy:.2}\" fill=\"none\" {strokes}/>"
    );
    let mut used = a + b + c;
    if let Some((label, taken)) = s.get(used..).and_then(|rest| cylinder_label(rest, (cx, cy, ry, h))) {
        drawing.push_str(&label);
        used += taken;
    }
    Some((drawing, used))
}

/// A cylinder's label (the `<text>` that follows it, centred on it) moved down to clear the top's
/// front arc, as far as the bottom's leaves room: its opening tag rewritten, and the bytes it took.
/// `None` when it needs no move.
fn cylinder_label(rest: &str, (cx, cy, ry, h): (f32, f32, f32, f32)) -> Option<(String, usize)> {
    let lead = rest.len() - rest.trim_start().len();
    let t = rest.get(lead..)?;
    if !t.starts_with("<text ") {
        return None;
    }
    let open = t.get(..=t.find('>')?)?;
    let num = |k: &str| attr(open, k).and_then(|v| v.parse::<f32>().ok());
    if (num("x")? - cx).abs() > 0.05 {
        return None;
    }
    let (y, size) = (num("y")?, num("font-size").unwrap_or(14.0));
    let lines = t.find("</text>").and_then(|e| t.get(..e)).map_or(1, |inner| inner.matches("<tspan").count().max(1));
    // The first line's top and the last line's bottom, at the renderer's 1.5 line spacing.
    let first_top = y - 0.85 * size;
    let last_bottom = y + (lines as f32 - 1.0) * size * 1.5 + 0.3 * size;
    let shift = (cy + ry + 3.0 - first_top).min(cy + h + ry - 3.0 - last_bottom);
    if shift <= 0.0 {
        return None;
    }
    let old = format!(" y=\"{}\"", attr(open, "y")?);
    Some((open.replacen(&old, &format!(" y=\"{:.2}\"", y + shift), 1), lead + open.len()))
}

/// The room either side of a flowchart box's text.
const FLOWCHART_PADDING_X: f32 = 20.0;

/// A label written `"text"` (Mermaid's quoted label, which may hold any character) as its text.
fn unquote(label: &str) -> String {
    let t = label.trim();
    match t.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        Some(inner) => inner.to_owned(),
        None => label.to_owned(),
    }
}

/// Whether `source` is a flowchart (`flowchart …` or `graph …` its first statement, after any front
/// matter and comments).
fn is_flowchart(source: &str) -> bool {
    let mut lines = source.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%")).peekable();
    if lines.peek() == Some(&"---") {
        lines.next();
        for line in lines.by_ref() {
            if line == "---" {
                break;
            }
        }
    }
    lines.next().is_some_and(|l| l.starts_with("flowchart") || l.starts_with("graph"))
}

/// A flowchart with each chain of arrows (`A --> B --> C`) written an arrow a line, when it has
/// any: the renderer drops what follows a labelled arrow in a chain (of `C -->|Y| D -->|N| E` it
/// kept only `C → D`).
fn split_chains(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len() + 64);
    let mut changed = false;
    for line in source.lines() {
        match chain(line) {
            Some(parts) => {
                changed = true;
                let indent = line.get(..line.len() - line.trim_start().len()).unwrap_or("");
                let mut steps = parts.chunks(2).peekable();
                while let Some(step) = steps.next() {
                    if let ([from, arrow], Some([to, ..])) = (step, steps.peek().copied()) {
                        out.push_str(&format!("{indent}{from} {arrow} {to}\n"));
                    }
                }
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    changed.then_some(out)
}

/// A line's nodes and arrows (each with its `|label|`), alternately, when it chains more than one
/// arrow; `None` for any other line, or one written in a form this leaves as it is (`A -- text --> B`,
/// statements after `;`, an arrow's id `e1@-->`).
fn chain(line: &str) -> Option<Vec<&str>> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let byte = |i: usize| chars.get(i).map_or(line.len(), |c| c.0);
    let arrow_char = |c: char| matches!(c, '-' | '=' | '~' | '.' | '<' | '>');
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start, mut i) = (0i32, false, 0usize, 0usize);
    let mut prev: Option<char> = None;
    while let Some(&(at, c)) = chars.get(i) {
        if quoted {
            quoted = c != '"';
        } else if c == '"' {
            quoted = true;
        } else if matches!(c, '[' | '(' | '{') || (c == '>' && depth == 0 && prev.is_some_and(|p| p.is_alphanumeric() || p == '_')) {
            // `A>flag]` opens a shape too.
            depth += 1;
        } else if matches!(c, ']' | ')' | '}') {
            depth -= 1;
        } else if depth == 0 && matches!(c, ';' | '@') {
            return None;
        } else if depth == 0 && arrow_char(c) && c != '>' {
            let mut end = i;
            while chars.get(end).is_some_and(|&(_, c)| arrow_char(c)) {
                end += 1;
            }
            let token = line.get(at..byte(end)).unwrap_or("");
            if token.chars().filter(|c| matches!(c, '-' | '=' | '~')).count() >= 2 {
                // An arrow opening text written inside it.
                if matches!(token, "--" | "==" | "-.") {
                    return None;
                }
                let mut next = end;
                while chars.get(next).is_some_and(|c| c.1 == ' ') {
                    next += 1;
                }
                if chars.get(next).is_some_and(|c| c.1 == '|') {
                    let close = (next + 1..chars.len()).find(|&k| chars.get(k).is_some_and(|c| c.1 == '|'))?;
                    end = close + 1;
                }
                parts.push(line.get(start..at)?.trim());
                parts.push(line.get(at..byte(end))?.trim());
                start = byte(end);
                i = end;
                prev = None;
                continue;
            }
            i = end;
            prev = chars.get(end.saturating_sub(1)).map(|c| c.1);
            continue;
        }
        prev = Some(c);
        i += 1;
    }
    parts.push(line.get(start..)?.trim());
    (parts.len() >= 5 && parts.iter().step_by(2).all(|p| !p.is_empty())).then_some(parts)
}

/// The SVG with the arrow labels' backgrounds kept and their outlines taken away: Mermaid's labels
/// sit on their arrows without a frame.
fn without_label_borders(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find("<rect data-edge-id=") {
        let (before, tag_on) = rest.split_at(at);
        out.push_str(before);
        let end = tag_on.find("/>").map_or(tag_on.len(), |e| e + 2);
        let (tag, after) = tag_on.split_at(end);
        match tag.find("stroke-opacity=\"") {
            Some(o) => {
                let value = o + "stroke-opacity=\"".len();
                let close = tag.get(value..).and_then(|v| v.find('"')).map_or(value, |c| value + c);
                out.push_str(tag.get(..value).unwrap_or(tag));
                out.push('0');
                out.push_str(tag.get(close..).unwrap_or_default());
            }
            None => out.push_str(tag),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Draws a Mermaid diagram.
pub fn render(source: &str, o: Options) -> Result<Image, String> {
    let svg = svg(source, o.dark)?;
    let drawn = std::panic::catch_unwind(AssertUnwindSafe(|| paint(&svg, o.scale)));
    drawn.unwrap_or_else(|_| Err("The diagram could not be drawn.".to_owned()))
}

fn paint(svg: &str, scale: f32) -> Result<Image, String> {
    let options = usvg::Options { fontdb: fonts(), font_family: font_family().to_owned(), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err("The diagram is empty.".to_owned());
    }
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let scale = scale.min(MAX_SIDE / w).min(MAX_SIDE / h);
    let (pw, ph) = ((w * scale).ceil().max(1.0) as u32, (h * scale).ceil().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| "The diagram is too large.".to_owned())?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(Image { width: pw, height: ph, rgba: pixmap.take(), size: (w, h) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(source: &str) -> Image {
        render(source, Options { dark: false, scale: 1.0 }).unwrap_or_else(|e| panic!("{source}: {e}"))
    }

    #[test]
    fn the_diagram_families_offered_draw() {
        for source in [
            "flowchart TD\n    A[시작] --> B{확인}\n    B -->|예| C[처리]\n    B -->|아니오| D[다시 보기]\n    D --> B",
            "sequenceDiagram\n    participant 나\n    participant 증권사\n    나->>증권사: 매수 주문\n    증권사-->>나: 체결 알림",
            "gantt\n    title 일정\n    dateFormat YYYY-MM-DD\n    section 준비\n    자료 정리 :a1, 2026-11-03, 3d\n    검토 :after a1, 2d",
            "stateDiagram-v2\n    [*] --> 주문\n    주문 --> 체결\n    체결 --> 정산\n    정산 --> [*]",
            "classDiagram\n    class Account {\n        +balance\n        +deposit()\n    }\n    Account <|-- Savings",
            "erDiagram\n    CUSTOMER ||--o{ ORDER : places\n    ORDER ||--|{ LINE : contains",
            "mindmap\n  root((가계부))\n    수입\n      월급\n    지출\n      식비\n      주식",
            "pie title 지출\n    \"식비\" : 40\n    \"주거\" : 35\n    \"주식\" : 25",
            "timeline\n    title 진행\n    2025 : 시작\n    2026 : 확장",
        ] {
            let image = draws(source);
            assert!(image.width > 20 && image.height > 20, "{source}");
            assert_eq!(image.rgba.len(), image.width as usize * image.height as usize * 4);
            assert!(image.rgba.chunks(4).any(|px| px[3] > 0), "{source}: something is drawn");
        }
    }

    #[test]
    fn a_kanban_columns_cards_sit_below_its_title_as_wide_as_the_column() {
        let source = "kanban\n  done[✅ 완료 3]\n    a1[요구사항 정리]\n    a2[화면 설계 · 12칸]\n    a3[개발 서버 배포와 데이터 점검 11-17]\n  doing[🔄 진행 중 1]\n    b1[주문 화면]\n  todo[⬜ 남음 2]\n    c1[운영 배포]\n    c2[인수인계]";
        let (layout, _, kind) = laid_out(source, &RenderOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kind, DiagramKind::Kanban);
        let heights: Vec<f32> = layout.subgraphs.iter().map(|s| s.height).collect();
        assert!(heights.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5), "every column as tall: {heights:?}");
        let mut columns: Vec<&mermaid_rs_renderer::SubgraphLayout> = layout.subgraphs.iter().collect();
        columns.sort_by(|a, b| a.x.total_cmp(&b.x));
        let titles: Vec<&str> = columns.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(titles, ["✅ 완료 3", "🔄 진행 중 1", "⬜ 남음 2"], "in the board's order");
        for pair in columns.windows(2) {
            assert!(pair[1].x >= pair[0].x + pair[0].width + 8.0, "columns apart");
        }
        for sub in &layout.subgraphs {
            let title_bottom = sub.y + 12.0 + sub.label_block.height;
            let cards: Vec<&mermaid_rs_renderer::NodeLayout> = sub.nodes.iter().filter_map(|id| layout.nodes.get(id)).collect();
            let first = cards.first().map(|n| n.y).unwrap_or_default();
            assert!(first >= title_bottom + 8.0, "{}: the first card at {first}, the title ends at {title_bottom}", sub.label);
            for n in &cards {
                assert!((n.width - (sub.width - 20.0)).abs() < 0.5, "{}: a card as wide as its column", sub.label);
                assert!(n.x >= sub.x + 8.0 && n.y + n.height <= sub.y + sub.height - 8.0, "{}: inside it with room", sub.label);
            }
            for pair in cards.windows(2) {
                assert!(pair[1].y >= pair[0].y + pair[0].height + 6.0, "{}: cards apart", sub.label);
            }
        }
        assert!(layout.width >= layout.subgraphs.last().map_or(0.0, |s| s.x + s.width));
        assert!(draws(source).width > 20);
    }

    #[test]
    fn text_on_a_filled_box_or_group_reads_on_its_fill() {
        let source = "flowchart LR\n    subgraph G[묶음]\n        A[밝은 칸] --> B[어두운 칸]\n    end\n    B --> C[글자색 지정]\n    B --> D[기본]\n    style A fill:#bbdefb\n    style B fill:#1e3a8a\n    style C fill:#bbdefb,color:#ff0000\n    style G fill:#e3f2fd";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source).unwrap_or_else(|e| panic!("{e}")).graph;
        let options = RenderOptions::default();
        let mut layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &options.layout);
        readable_text(&mut layout);
        let text = |id: &str| layout.nodes.get(id).and_then(|n| n.style.text_color.clone());
        assert_eq!(text("A").as_deref(), Some("#1E293B"), "dark on a light fill");
        assert_eq!(text("B").as_deref(), Some("#F8FAFC"), "white on a dark fill");
        assert_eq!(text("C").as_deref(), Some("#ff0000"), "a colour given stays");
        assert_eq!(text("D"), None, "no fill, the theme's text");
        assert_eq!(layout.subgraphs.first().and_then(|s| s.style.text_color.as_deref()), Some("#1E293B"), "a group's title too");
        assert_eq!(luminance("#fff"), Some(1.0));
        assert_eq!(luminance("#000000"), Some(0.0));
        assert_eq!(luminance("red"), None);
        assert_eq!(luminance("#12"), None);
    }

    #[test]
    fn a_chain_of_arrows_keeps_every_arrow() {
        let source = "flowchart TB\n    A --> B --> C\n    C -->|Y| D[\"둘 --> 셋\"] -->|N| D1\n    G -->|Y| H --> I & J\n    K -- 글 --> L\n    M -. 점선 .-> N\n    style A fill:#fff";
        let split = split_chains(source);
        assert_eq!(
            split.as_deref(),
            Some(
                "flowchart TB\n    A --> B\n    B --> C\n    C -->|Y| D[\"둘 --> 셋\"]\n    D[\"둘 --> 셋\"] -->|N| D1\n    G -->|Y| H\n    H --> I & J\n    K -- 글 --> L\n    M -. 점선 .-> N\n    style A fill:#fff\n"
            )
        );
        let graph = mermaid_rs_renderer::parse_mermaid_strict(split.as_deref().unwrap_or_default()).unwrap_or_else(|e| panic!("{e}")).graph;
        let arrows: Vec<(&str, &str)> = graph.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
        for arrow in [("A", "B"), ("B", "C"), ("C", "D"), ("D", "D1"), ("G", "H"), ("H", "I"), ("H", "J"), ("K", "L"), ("M", "N")] {
            assert!(arrows.contains(&arrow), "{arrow:?} in {arrows:?}");
        }
        assert_eq!(split_chains("flowchart LR\n    A --> B\n    B -->|예| C"), None, "nothing to split");
        assert_eq!(chain("    A -- 글 --> B --> C"), None, "text inside an arrow is left as it is");
        assert!(is_flowchart("---\ntitle: 흐름\n---\n%% 메모\nflowchart LR\n    A --> B"));
        assert!(is_flowchart("graph TD\n    A --> B"));
        assert!(!is_flowchart("sequenceDiagram\n    A->>B: 안녕"));
        // Drawn, a chain keeps its arrows.
        assert!(svg(source, false).is_ok_and(|svg| svg.contains("D1")));
    }

    #[test]
    fn character_references_show_as_their_characters() {
        assert_eq!(entities("List&lt;Dto&gt; #lt;b#gt; &amp; &quot;q&quot; &#39;x&#39; &#x2192; #35;"), "List<Dto> <b> & \"q\" 'x' → #");
        // Not references: left as they are.
        assert_eq!(entities("C# code; A & B; &unknown; #1; &#;"), "C# code; A & B; &unknown; #1; &#;");
        assert_eq!(entities("한글 & 영어"), "한글 & 영어");
        let source = "flowchart LR\n    A[\"List&lt;Dto&gt;\"] -->|\"x &lt; y\"| B";
        assert!(svg(source, false).is_ok_and(|svg| svg.contains("List&lt;Dto&gt;") && !svg.contains("&amp;lt;")));
    }

    #[test]
    fn a_table_shows_a_column_a_row_with_its_comment() {
        assert_eq!(
            er_comments("T\n---\nVARCHAR UNQ_NO \"고유 번호\"\nNUMBER SN PK \"일련\"\nCHAR DEL_YN"),
            "T\n---\nVARCHAR UNQ_NO (고유 번호)\nNUMBER SN PK (일련)\nCHAR DEL_YN"
        );
        let source = "erDiagram\n    RCPT ||--o{ LINK : \"APLY_RCPT_NO\"\n    LINK ||--o{ DAT : \"SN (FK)\"\n    LINK {\n        NUMBER  ORDER_LINK_SN PK\n        VARCHAR APLY_RCPT_NO FK\n        VARCHAR SEND_CK_VL \"Y인 경우 외부 송신, 아니면 보내지 않음\"\n        CHAR    DEL_YN\n    }\n    DAT {\n        NUMBER LINK_DATA_SN PK\n        VARCHAR LINK_SE_CD \"A/B/C/D\"\n    }";
        let (layout, _, _) = laid_out(source, &RenderOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        // Each column one line (title, divider, columns): none wrapped into a row of its own.
        let lines = |id: &str| layout.nodes.get(id).map(|n| n.label.lines.clone()).unwrap_or_default();
        assert_eq!(lines("LINK").len(), 2 + 4, "{:?}", lines("LINK"));
        assert!(lines("LINK").iter().any(|l| l.contains("SEND_CK_VL (Y인 경우 외부 송신, 아니면 보내지 않음)")));
        assert_eq!(lines("DAT").len(), 2 + 2, "{:?}", lines("DAT"));
        // A relationship's label without its quotes.
        assert!(layout.edges.iter().all(|e| e.label.as_ref().is_some_and(|l| l.lines.iter().all(|t| !t.contains('"')))));
        // Laid out in layers: each table after the one it hangs from, none overlapping.
        let top = |id: &str| layout.nodes.get(id).map(|n| (n.y, n.y + n.height)).unwrap_or_default();
        assert!(top("RCPT").1 < top("LINK").0 && top("LINK").1 < top("DAT").0);
        // As tall as its rows: no room left below the last.
        let options = RenderOptions::default();
        let line = options.theme.font_size * options.layout.label_line_height;
        assert!(top("LINK").1 - top("LINK").0 < 6.0 * line, "{:?}", top("LINK"));
    }

    #[test]
    fn scale_multiplies_the_pixels_and_stays_bounded() {
        let one = draws("flowchart LR\n  A --> B");
        let two = render("flowchart LR\n  A --> B", Options { dark: false, scale: 2.0 }).unwrap_or_else(|e| panic!("{e}"));
        assert!(two.width >= one.width * 2 - 2 && two.width <= one.width * 2 + 2);
        // The diagram's own size is the same at any scale.
        assert!(one.size.0 > 0.0 && one.size == two.size);
        let huge = render("flowchart LR\n  A --> B", Options { dark: false, scale: 1e9 }).unwrap_or_else(|e| panic!("{e}"));
        assert!(huge.width <= MAX_SIDE as u32 && huge.height <= MAX_SIDE as u32);
    }

    #[test]
    fn the_dark_theme_draws_too() {
        let dark = render("flowchart LR\n  가 --> 나", Options { dark: true, scale: 1.0 }).unwrap_or_else(|e| panic!("{e}"));
        assert!(dark.rgba.chunks(4).any(|px| px[3] > 0));
    }

    #[test]
    fn a_quoted_arrow_label_shows_without_its_quotes_or_a_frame() {
        let svg = svg("flowchart LR\n    A[가] -->|\"보내기 · 받기\"| B[나]", false).unwrap_or_else(|e| panic!("{e}"));
        assert!(svg.contains("보내기 · 받기"), "the label is drawn");
        assert!(!svg.contains("&quot;보내기") && !svg.contains("\"보내기"), "without the quotes Mermaid writes it in");
        assert!(svg.contains("data-label-kind=\"center\""), "on a background");
        assert!(!svg.contains("stroke-opacity=\"0.45\""), "with no frame");
    }

    #[test]
    fn a_cylinder_is_one_outline_with_its_top_in_front() {
        let drawn = svg("flowchart LR\n    A[(원장)] --> B[끝]", false).unwrap_or_else(|e| panic!("{e}"));
        assert!(!drawn.contains("<ellipse"), "no ellipse of its own, nor its full bottom");
        assert_eq!(drawn.matches(" A ").count(), 3, "the top's two arcs and the bottom's front one");
        // A label of three lines starts below the top.
        let drawn = svg("flowchart LR\n    A[(\"하나<br>둘<br>셋\")] --> B[끝]", false).unwrap_or_else(|e| panic!("{e}"));
        let num = |tag: &str, k: &str| attr(tag, k).and_then(|v| v.parse::<f32>().ok()).unwrap_or_default();
        let arc = drawn.find(" A ").unwrap_or_default();
        let path = drawn.get(..arc).and_then(|head| head.rfind("<path")).and_then(|i| drawn.get(i..)).unwrap_or_default();
        let text = drawn.find("<text ").and_then(|i| drawn.get(i..)).unwrap_or_default();
        let words: Vec<f32> = path.split_whitespace().map(|v| v.parse().unwrap_or(f32::NAN)).collect();
        let (cy, ry) = (words.get(3).copied().unwrap_or_default(), words.get(6).copied().unwrap_or_default());
        assert!(num(text, "y") - 0.85 * num(text, "font-size") >= cy + ry, "{cy} {ry}: {text:.80}");
        // Anything else stays as written.
        let other = "<svg><ellipse cx=\"1\" cy=\"1\" rx=\"1\" ry=\"1\"/><rect x=\"0\" y=\"0\" width=\"5\" height=\"5\"/></svg>";
        assert_eq!(cylinders(other), other);
    }

    #[test]
    fn a_labels_inline_html_styles_its_text_and_other_angle_brackets_stay() {
        // As marks while laid out: styles, line breaks however written, the plain tags taken out.
        assert_eq!(inline_html("<b>굵게</b><br/>보통"), "\u{E000}굵게\u{E001}<br>보통");
        assert_eq!(inline_html("<STRONG>a</STRONG><br />b<BR>c"), "\u{E000}a\u{E001}<br>b<br>c");
        assert_eq!(
            inline_html("<i>x</i> <em>y</em> <u>u</u> <s>s</s> <del>d</del>"),
            "\u{E002}x\u{E003} \u{E002}y\u{E003} \u{E004}u\u{E005} \u{E006}s\u{E007} \u{E006}d\u{E007}"
        );
        assert_eq!(inline_html("<span style=\"color:red\">빨강</span> <sup>2</sup> <code>c</code>"), "빨강 2 c");
        // What isn't a formatting tag stays as written; an open tag with no end still styles.
        for text in ["List<OrderDto>", "<id>, <ssn>", "a < b > c", "x <= y", "<b", "<>"] {
            assert_eq!(inline_html(text), text);
        }
        assert_eq!(inline_html("a<b>"), "a\u{E000}");

        // Drawn: the bold part a bold tspan; the rest as it was.
        let drawn = svg("flowchart LR\n    A[\"<b>대상</b><br>운영 화면이 읽는 표\"] --> B[\"<i>기울임</i> List&lt;Dto&gt; <id>\"]", false)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(drawn.contains("<tspan font-weight=\"bold\">대상</tspan>"), "{drawn:.3000}");
        // Italic: no ink where the line lays it out, and drawn by a slanted copy of the line.
        assert!(drawn.contains("<tspan fill-opacity=\"0\">기울임</tspan>"), "{drawn:.3000}");
        assert!(drawn.contains("transform=\"matrix(1 0 -0.21 1 "), "a slanted copy");
        assert!(drawn.contains("<tspan>기울임</tspan>"), "inked in the copy");
        assert!(!drawn.contains("&lt;b&gt;") && !drawn.contains("&lt;/b&gt;") && !drawn.contains("&lt;i&gt;"), "no tag drawn as text");
        assert!(drawn.contains("List&lt;Dto&gt;") && drawn.contains("&lt;id&gt;"), "angle brackets that aren't tags stay");
        assert!(!drawn.contains(|c: char| ('\u{E000}'..='\u{E007}').contains(&c)), "no mark left");
        // A bold span wrapped over two lines is bold on both.
        let wrapped =
            styled_text("<text x=\"0\"><tspan x=\"0\" dy=\"0\">\u{E000}one</tspan><tspan x=\"0\" dy=\"1\">two\u{E001} three</tspan></text>");
        assert_eq!(
            wrapped,
            "<text x=\"0\"><tspan x=\"0\" dy=\"0\"><tspan font-weight=\"bold\">one</tspan></tspan><tspan x=\"0\" dy=\"1\"><tspan font-weight=\"bold\">two</tspan> three</tspan></text>"
        );
        // An italic word on the second line: its copy sits on that line's baseline (y 10 + dy 17).
        let slanted = styled_text(
            "<text x=\"5\" y=\"10\" font-size=\"14\"><tspan x=\"5\" dy=\"0\">a</tspan><tspan x=\"5\" dy=\"17\">b \u{E002}c\u{E003}</tspan></text>",
        );
        assert_eq!(
            slanted,
            "<text x=\"5\" y=\"10\" font-size=\"14\"><tspan x=\"5\" dy=\"0\">a</tspan><tspan x=\"5\" dy=\"17\">b <tspan fill-opacity=\"0\">c</tspan></tspan></text>\
             <text x=\"5\" y=\"27.00\" font-size=\"14\" transform=\"matrix(1 0 -0.21 1 5.67 0)\"><tspan fill-opacity=\"0\">b </tspan><tspan>c</tspan></text>"
        );
        // A text this doesn't know the layout of keeps its text, without the marks.
        assert_eq!(styled_text("<text><tspan><tspan>\u{E000}x</tspan></tspan></text>"), "<text><tspan><tspan>x</tspan></tspan></text>");
        assert!(render("flowchart LR\n  A[\"<b>가</b>\"] --> B", Options { dark: false, scale: 1.0 }).is_ok());
    }

    #[test]
    fn bad_syntax_is_an_error() {
        // The renderer reads loosely (an unclosed label still draws); what isn't a diagram fails.
        for source in ["", "not a diagram at all"] {
            assert!(render(source, Options { dark: false, scale: 1.0 }).is_err(), "{source:?}");
        }
    }
}
