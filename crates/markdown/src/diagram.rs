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
    let mut graph = mermaid_rs_renderer::parse_mermaid_strict(source)?.graph;
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
    let svg = cylinders(&mermaid_rs_renderer::render_svg(&layout, &options.theme, &config));
    Ok(if flowchart { without_label_borders(&svg) } else { svg })
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
