//! Mermaid diagrams (```` ```mermaid ```` blocks) drawn to pixels: mermaid-rs-renderer lays the
//! diagram out as SVG, measuring its labels with the system's fonts, and resvg paints it. Both are
//! compiled in, so nothing is downloaded. A panic inside them comes back as an error.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};

use mermaid_rs_renderer::{DiagramKind, RenderOptions, Theme};

mod layered;
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

/// The renderer's own steps (parse, lay out, write SVG), a flowchart laid out as Mermaid lays it
/// out ([`layered`]): `|"text"|` without its quotes, and each label on its arrow over a plain
/// background.
fn draw(source: &str, options: &RenderOptions) -> Result<String, mermaid_rs_renderer::ParseError> {
    // A flowchart's chains an arrow a line, as the renderer reads them whole that way.
    let split = if is_flowchart(source) { split_chains(source) } else { None };
    let parsed = match split.as_deref().map(mermaid_rs_renderer::parse_mermaid_strict) {
        Some(Ok(parsed)) => parsed,
        _ => mermaid_rs_renderer::parse_mermaid_strict(source)?,
    };
    let mut graph = parsed.graph;
    let flowchart = graph.kind == DiagramKind::Flowchart;
    if flowchart {
        for edge in &mut graph.edges {
            for label in [&mut edge.label, &mut edge.start_label, &mut edge.end_label].into_iter().flatten() {
                *label = unquote(label);
            }
        }
    }
    // A flowchart's boxes hug their text as Mermaid's do (its padding is 15; the renderer's 30 made
    // wide drawings, small once fitted to the page).
    let mut config = options.layout.clone();
    if flowchart {
        config.node_padding_x = FLOWCHART_PADDING_X;
    }
    let mut layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &config);
    if flowchart {
        layered::relayout(&graph, &mut layout);
    }
    readable_text(&mut layout);
    let svg = cylinders(&mermaid_rs_renderer::render_svg(&layout, &options.theme, &config));
    Ok(if flowchart { without_label_borders(&svg) } else { svg })
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
    fn bad_syntax_is_an_error() {
        // The renderer reads loosely (an unclosed label still draws); what isn't a diagram fails.
        for source in ["", "not a diagram at all"] {
            assert!(render(source, Options { dark: false, scale: 1.0 }).is_err(), "{source:?}");
        }
    }
}
