//! Mermaid diagrams (```` ```mermaid ```` blocks) drawn to pixels: mermaid-rs-renderer lays the
//! diagram out as SVG, measuring its labels with the system's fonts, and resvg paints it. Both are
//! compiled in, so nothing is downloaded. A panic inside them comes back as an error.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};

use mermaid_rs_renderer::{RenderOptions, Theme};
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
    match std::panic::catch_unwind(AssertUnwindSafe(|| mermaid_rs_renderer::render_strict(source, options))) {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("The diagram could not be drawn.".to_owned()),
    }
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
    fn bad_syntax_is_an_error() {
        // The renderer reads loosely (an unclosed label still draws); what isn't a diagram fails.
        for source in ["", "not a diagram at all"] {
            assert!(render(source, Options { dark: false, scale: 1.0 }).is_err(), "{source:?}");
        }
    }
}
