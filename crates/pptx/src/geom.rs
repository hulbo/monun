//! Shapes' outlines as SVG paths: PowerPoint's preset shapes (the common ones; the rest drawn as
//! rectangles) and shapes drawn point by point (`custGeom`), filled and outlined.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::color::Rgba;
use crate::package::{Node, PX, attr, elems, int, kid, kids};
use crate::scene::{Frame, Out, Paint};

/// One path of a shape (CSS pixels from its top left corner), and whether it's filled, outlined,
/// and filled even-odd (a ring).
pub struct Path {
    pub d: String,
    pub fill: bool,
    pub stroke: bool,
    pub evenodd: bool,
}

/// An outline resolved: its paint, width (points), dashes and arrowheads.
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    pub dash: Option<String>,
    pub head: Option<String>,
    pub tail: Option<String>,
}

/// The shape `geom` (a `prstGeom` or `custGeom`; a rectangle without one) at `f`, filled and
/// outlined, as an SVG covering its box; empty when it shows nothing.
pub fn svg(out: &mut Out, geom: Option<Node>, f: &Frame, fill: &Paint, stroke: Option<&Stroke>) -> String {
    if matches!(fill, Paint::None) && stroke.is_none() {
        return String::new();
    }
    let (w, h) = (f.w * PX, f.h * PX);
    let paths = match geom {
        Some(g) if g.tag_name().name() == "custGeom" => custom(g, w, h),
        Some(g) => preset(g, attr(g, "prst").unwrap_or("rect"), w, h),
        None => vec![shape(rect(w, h))],
    };
    let mut defs = String::new();
    let fill_attr = svg_paint(out, fill, &mut defs);
    let stroke_attr = stroke.map_or_else(|| "stroke=\"none\"".to_string(), |s| svg_stroke(out, s, &mut defs));
    let mut body = String::new();
    for p in &paths {
        let filled = if p.fill { fill_attr.as_str() } else { "fill=\"none\"" };
        let outlined = if p.stroke { stroke_attr.as_str() } else { "stroke=\"none\"" };
        let rule = if p.evenodd { " fill-rule=\"evenodd\"" } else { "" };
        let _ = write!(body, "<path d=\"{}\" {filled} {outlined}{rule}/>", p.d);
    }
    let flip = match (f.flip_h, f.flip_v) {
        (true, true) => " style=\"transform:scale(-1,-1)\"",
        (true, false) => " style=\"transform:scale(-1,1)\"",
        (false, true) => " style=\"transform:scale(1,-1)\"",
        (false, false) => "",
    };
    let defs = if defs.is_empty() { defs } else { format!("<defs>{defs}</defs>") };
    format!("<svg class=\"geo\"{flip}>{defs}{body}</svg>")
}

/// `fill="…"` for a paint, adding a gradient or a picture to `defs`.
fn svg_paint(out: &mut Out, p: &Paint, defs: &mut String) -> String {
    match p {
        Paint::None => "fill=\"none\"".into(),
        Paint::Solid(c) => format!("fill=\"{}\"{}", solid(*c), opacity("fill-opacity", *c)),
        Paint::Grad { stops, angle, radial } => {
            let id = format!("g{}", out.id());
            let list: String = stops
                .iter()
                .map(|(at, c)| format!("<stop offset=\"{:.3}\" stop-color=\"{}\"{}/>", at, solid(*c), opacity("stop-opacity", *c)))
                .collect();
            if *radial {
                let _ = write!(defs, "<radialGradient id=\"{id}\">{list}</radialGradient>");
            } else {
                let _ = write!(defs, "<linearGradient id=\"{id}\" gradientTransform=\"rotate({angle:.1} .5 .5)\">{list}</linearGradient>");
            }
            format!("fill=\"url(#{id})\"")
        }
        Paint::Image { src, .. } => {
            let id = format!("p{}", out.id());
            let _ = write!(
                defs,
                "<pattern id=\"{id}\" patternContentUnits=\"objectBoundingBox\" width=\"1\" height=\"1\"><image href=\"{src}\" width=\"1\" height=\"1\" preserveAspectRatio=\"none\"/></pattern>"
            );
            format!("fill=\"url(#{id})\"")
        }
    }
}

fn solid(c: Rgba) -> String {
    Rgba { a: 1.0, ..c }.css()
}

fn opacity(name: &str, c: Rgba) -> String {
    if c.a < 0.999 { format!(" {name}=\"{:.3}\"", c.a.clamp(0.0, 1.0)) } else { String::new() }
}

/// `stroke="…"` and the rest for an outline, adding its arrowheads to `defs`.
fn svg_stroke(out: &mut Out, s: &Stroke, defs: &mut String) -> String {
    let Some(c) = s.paint.first() else { return "stroke=\"none\"".into() };
    let width = (s.width * PX).max(0.75);
    let mut a = format!("stroke=\"{}\"{} stroke-width=\"{width:.2}\" stroke-linejoin=\"round\"", solid(c), opacity("stroke-opacity", c));
    if let Some(dash) = s.dash.as_deref().and_then(dashes) {
        let list: Vec<String> = dash.iter().map(|d| format!("{:.2}", d * width)).collect();
        let _ = write!(a, " stroke-dasharray=\"{}\"", list.join(" "));
    }
    for (end, kind) in [("marker-start", &s.head), ("marker-end", &s.tail)] {
        if let Some(kind) = kind {
            let id = format!("m{}", out.id());
            let _ = write!(defs, "{}", marker(&id, kind, c));
            let _ = write!(a, " {end}=\"url(#{id})\"");
        }
    }
    a
}

/// A preset dash's dashes and gaps, in line widths.
fn dashes(name: &str) -> Option<&'static [f64]> {
    Some(match name {
        "dash" => &[4.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "dot" | "sysDot" => &[1.0, 1.0],
        "dashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDash" => &[8.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        "sysDashDot" => &[3.0, 1.0, 1.0, 1.0],
        "sysDashDotDot" => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => return None,
    })
}

/// An arrowhead (`triangle`, `stealth`, `arrow`, `diamond`, `oval`) pointing along the line.
fn marker(id: &str, kind: &str, c: Rgba) -> String {
    let color = solid(c);
    let shape = match kind {
        "oval" => format!("<circle cx=\"5\" cy=\"5\" r=\"4\" fill=\"{color}\"/>"),
        "diamond" => format!("<path d=\"M5 0L10 5L5 10L0 5Z\" fill=\"{color}\"/>"),
        "arrow" => format!("<path d=\"M0 0L10 5L0 10\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/>"),
        "stealth" => format!("<path d=\"M0 0L10 5L0 10L3 5Z\" fill=\"{color}\"/>"),
        _ => format!("<path d=\"M0 0L10 5L0 10Z\" fill=\"{color}\"/>"),
    };
    format!(
        "<marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"3.5\" markerHeight=\"3.5\" orient=\"auto-start-reverse\">{shape}</marker>"
    )
}

fn shape(d: String) -> Path {
    Path { d, fill: true, stroke: true, evenodd: false }
}

fn line(d: String) -> Path {
    Path { d, fill: false, stroke: true, evenodd: false }
}

fn ring(d: String) -> Path {
    Path { d, fill: true, stroke: true, evenodd: true }
}

/// A preset's adjustment `name` (as a fraction), or `default`.
fn adj(g: Node, name: &str, default: f64) -> f64 {
    kid(g, "avLst")
        .and_then(|list| kids(list, "gd").find(|gd| attr(*gd, "name") == Some(name)))
        .and_then(|gd| attr(gd, "fmla"))
        .and_then(|f| f.strip_prefix("val "))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map_or(default, |v| v / 100_000.0)
}

fn n(v: f64) -> String {
    format!("{v:.2}")
}

fn poly(points: &[(f64, f64)]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, n(*x), n(*y));
    }
    d.push('Z');
    d
}

fn rect(w: f64, h: f64) -> String {
    poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])
}

fn round_rect(w: f64, h: f64, r: f64) -> String {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let (rs, wr, hr, w, h) = (n(r), n(w - r), n(h - r), n(w), n(h));
    format!("M{rs} 0H{wr}A{rs} {rs} 0 0 1 {w} {rs}V{hr}A{rs} {rs} 0 0 1 {wr} {h}H{rs}A{rs} {rs} 0 0 1 0 {hr}V{rs}A{rs} {rs} 0 0 1 {rs} 0Z")
}

fn ellipse_in(x: f64, y: f64, w: f64, h: f64) -> String {
    let (rx, ry) = (w / 2.0, h / 2.0);
    format!("M{} {}A{} {} 0 1 1 {} {}A{} {} 0 1 1 {} {}Z", n(x), n(y + ry), n(rx), n(ry), n(x + w), n(y + ry), n(rx), n(ry), n(x), n(y + ry))
}

/// Which way an arrow points.
#[derive(Clone, Copy)]
enum Dir {
    Right,
    Left,
    Up,
    Down,
}

/// A block arrow: its shaft `thick` (a fraction of its width across), its head `head` long.
fn arrow(w: f64, h: f64, thick: f64, head: f64, dir: Dir) -> String {
    // Drawn pointing right in a box `len` long and `across` wide, then turned.
    let (len, across) = match dir {
        Dir::Right | Dir::Left => (w, h),
        Dir::Up | Dir::Down => (h, w),
    };
    let head = head.min(len);
    let (a1, a2) = (across / 2.0 - across * thick / 2.0, across / 2.0 + across * thick / 2.0);
    let x = len - head;
    let pts = [(0.0, a1), (x, a1), (x, 0.0), (len, across / 2.0), (x, across), (x, a2), (0.0, a2)];
    let turned: Vec<(f64, f64)> = pts
        .iter()
        .map(|&(p, q)| match dir {
            Dir::Right => (p, q),
            Dir::Left => (len - p, q),
            Dir::Down => (q, p),
            Dir::Up => (q, len - p),
        })
        .collect();
    poly(&turned)
}

fn double_arrow(w: f64, h: f64, thick: f64, head: f64) -> String {
    let head = head.min(w / 2.0);
    let (a1, a2) = (h / 2.0 - h * thick / 2.0, h / 2.0 + h * thick / 2.0);
    poly(&[
        (0.0, h / 2.0),
        (head, 0.0),
        (head, a1),
        (w - head, a1),
        (w - head, 0.0),
        (w, h / 2.0),
        (w - head, h),
        (w - head, a2),
        (head, a2),
        (head, h),
    ])
}

fn star(w: f64, h: f64, points: usize, inner: f64) -> String {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let pts: Vec<(f64, f64)> = (0..points * 2)
        .map(|i| {
            let a = -PI / 2.0 + PI * i as f64 / points as f64;
            let r = if i % 2 == 0 { 1.0 } else { inner };
            (cx + cx * r * a.cos(), cy + cy * r * a.sin())
        })
        .collect();
    poly(&pts)
}

/// The arc of an ellipse filling the box, from `start` round by `sweep` (degrees, clockwise).
fn arc(w: f64, h: f64, start: f64, sweep: f64) -> String {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let at = |deg: f64| (rx + rx * deg.to_radians().cos(), ry + ry * deg.to_radians().sin());
    let (a, b) = (at(start), at(start + sweep));
    let large = u8::from(sweep.abs() > 180.0);
    format!("M{} {}A{} {} 0 {large} 1 {} {}", n(a.0), n(a.1), n(rx), n(ry), n(b.0), n(b.1))
}

fn preset(g: Node, name: &str, w: f64, h: f64) -> Vec<Path> {
    let ss = w.min(h);
    let a = |name: &str, default: f64| adj(g, name, default);
    match name {
        "rect" | "flowChartProcess" | "wedgeRectCallout" | "flowChartPredefinedProcess" | "flowChartInternalStorage" | "textBox" => {
            vec![shape(rect(w, h))]
        }
        "roundRect" | "flowChartAlternateProcess" | "wedgeRoundRectCallout" => vec![shape(round_rect(w, h, ss * a("adj", 0.166_67)))],
        "flowChartTerminator" => vec![shape(round_rect(w, h, ss / 2.0))],
        "ellipse" | "flowChartConnector" | "wedgeEllipseCallout" | "cloud" | "cloudCallout" | "smileyFace" | "sun" | "moon" | "heart" => {
            vec![shape(ellipse_in(0.0, 0.0, w, h))]
        }
        "donut" => {
            let t = ss * a("adj", 0.25);
            vec![ring(format!("{}{}", ellipse_in(0.0, 0.0, w, h), ellipse_in(t, t, w - 2.0 * t, h - 2.0 * t)))]
        }
        "frame" => {
            let t = ss * a("adj1", 0.125);
            vec![ring(format!("{}{}", rect(w, h), poly(&[(t, t), (w - t, t), (w - t, h - t), (t, h - t)])))]
        }
        _ => polygons(g, name, w, h).unwrap_or_else(|| lines(g, name, w, h)),
    }
}

/// The preset shapes made of straight sides.
fn polygons(g: Node, name: &str, w: f64, h: f64) -> Option<Vec<Path>> {
    let ss = w.min(h);
    let a = |name: &str, default: f64| adj(g, name, default);
    let d = match name {
        "triangle" | "flowChartExtract" => poly(&[(w * a("adj", 0.5), 0.0), (w, h), (0.0, h)]),
        "rtTriangle" => poly(&[(0.0, 0.0), (0.0, h), (w, h)]),
        "diamond" | "flowChartDecision" => poly(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)]),
        "parallelogram" | "flowChartInputOutput" => {
            let x = (ss * a("adj", 0.25)).min(w);
            poly(&[(x, 0.0), (w, 0.0), (w - x, h), (0.0, h)])
        }
        "trapezoid" => {
            let x = (ss * a("adj", 0.25)).min(w / 2.0);
            poly(&[(0.0, h), (x, 0.0), (w - x, 0.0), (w, h)])
        }
        "pentagon" => poly(&[(w / 2.0, 0.0), (w, h * 0.382), (w * 0.809, h), (w * 0.191, h), (0.0, h * 0.382)]),
        "hexagon" => {
            let x = (ss * a("adj", 0.25)).min(w / 2.0);
            poly(&[(x, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (x, h), (0.0, h / 2.0)])
        }
        "octagon" => {
            let x = (ss * a("adj", 0.292_89)).min(ss / 2.0);
            poly(&[(x, 0.0), (w - x, 0.0), (w, x), (w, h - x), (w - x, h), (x, h), (0.0, h - x), (0.0, x)])
        }
        "homePlate" => {
            let x = w - (ss * a("adj", 0.5)).min(w);
            poly(&[(0.0, 0.0), (x, 0.0), (w, h / 2.0), (x, h), (0.0, h)])
        }
        "chevron" => {
            let x = (ss * a("adj", 0.5)).min(w);
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h), (x, h / 2.0)])
        }
        "rightArrow" | "notchedRightArrow" | "stripedRightArrow" => arrow(w, h, a("adj1", 0.5), ss * a("adj2", 0.5), Dir::Right),
        "leftArrow" => arrow(w, h, a("adj1", 0.5), ss * a("adj2", 0.5), Dir::Left),
        "upArrow" => arrow(w, h, a("adj1", 0.5), ss * a("adj2", 0.5), Dir::Up),
        "downArrow" => arrow(w, h, a("adj1", 0.5), ss * a("adj2", 0.5), Dir::Down),
        "leftRightArrow" => double_arrow(w, h, a("adj1", 0.5), ss * a("adj2", 0.5)),
        "plus" | "mathPlus" => {
            let x = (ss * a("adj", 0.25)).min(ss / 2.0);
            poly(&[
                (x, 0.0),
                (w - x, 0.0),
                (w - x, x),
                (w, x),
                (w, h - x),
                (w - x, h - x),
                (w - x, h),
                (x, h),
                (x, h - x),
                (0.0, h - x),
                (0.0, x),
                (x, x),
            ])
        }
        "star4" => star(w, h, 4, 0.25),
        "star5" => star(w, h, 5, 0.382),
        "star6" => star(w, h, 6, 0.577),
        "star8" => star(w, h, 8, 0.7),
        "snip1Rect" => {
            let x = (ss * a("adj", 0.166_67)).min(ss);
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, x), (w, h), (0.0, h)])
        }
        "snip2SameRect" => {
            let x = (ss * a("adj1", 0.166_67)).min(ss / 2.0);
            poly(&[(x, 0.0), (w - x, 0.0), (w, x), (w, h), (0.0, h), (0.0, x)])
        }
        "flowChartManualInput" => poly(&[(0.0, h * 0.2), (w, 0.0), (w, h), (0.0, h)]),
        "flowChartManualOperation" => poly(&[(0.0, 0.0), (w, 0.0), (w * 0.8, h), (w * 0.2, h)]),
        "flowChartPunchedCard" => poly(&[(w * 0.2, 0.0), (w, 0.0), (w, h), (0.0, h), (0.0, h * 0.2)]),
        "flowChartDocument" => {
            format!("M0 0H{}V{}C{} {} {} {} 0 {}Z", n(w), n(h * 0.83), n(w * 0.75), n(h * 0.66), n(w * 0.3), n(h * 1.1), n(h * 0.9))
        }
        "round1Rect" | "round2SameRect" => {
            let r = (ss * a(if name == "round1Rect" { "adj" } else { "adj1" }, 0.166_67)).min(ss / 2.0);
            let left = if name == "round2SameRect" { format!("M0 {}A{} {} 0 0 1 {} 0", n(r), n(r), n(r), n(r)) } else { "M0 0".to_string() };
            format!("{left}H{}A{} {} 0 0 1 {} {}V{}H0Z", n(w - r), n(r), n(r), n(w), n(r), n(h))
        }
        "can" | "flowChartMagneticDisk" => {
            let ry = (ss * a("adj", 0.25)).min(h / 2.0) / 2.0;
            format!(
                "M0 {}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 0 {}Z M0 {}A{} {} 0 0 0 {} {}",
                n(ry),
                n(w / 2.0),
                n(ry),
                n(w),
                n(ry),
                n(h - ry),
                n(w / 2.0),
                n(ry),
                n(h - ry),
                n(ry),
                n(w / 2.0),
                n(ry),
                n(w),
                n(ry)
            )
        }
        "cube" => {
            let x = (ss * a("adj", 0.25)).min(ss);
            format!(
                "M0 {}L{} 0H{}V{}L{} {}H0Z M0 {}H{}L{} 0M{} {}V{}",
                n(x),
                n(x),
                n(w),
                n(h - x),
                n(w - x),
                n(h),
                n(x),
                n(w - x),
                n(w),
                n(w - x),
                n(x),
                n(h)
            )
        }
        _ => return None,
    };
    Some(vec![shape(d)])
}

/// The preset lines and connectors (outlined, never filled); any other preset is a rectangle.
fn lines(g: Node, name: &str, w: f64, h: f64) -> Vec<Path> {
    let a = |name: &str, default: f64| adj(g, name, default);
    let d = match name {
        "line" | "straightConnector1" => format!("M0 0L{} {}", n(w), n(h)),
        "bentConnector2" => format!("M0 0H{}V{}", n(w), n(h)),
        "bentConnector3" | "bentConnector4" | "bentConnector5" => {
            let x = w * a("adj1", 0.5);
            format!("M0 0H{}V{}H{}", n(x), n(h), n(w))
        }
        "curvedConnector2" => format!("M0 0Q{} 0 {} {}", n(w), n(w), n(h)),
        "curvedConnector3" | "curvedConnector4" | "curvedConnector5" => format!("M0 0C{} 0 {} {} {} {}", n(w / 2.0), n(w / 2.0), n(h), n(w), n(h)),
        "arc" => {
            // Its adjustments are angles in 60000ths of a degree (270° and 0° unless set), which
            // `adj` reads as hundred-thousandths.
            let degrees = |name: &str, default: f64| a(name, default * 60_000.0 / 100_000.0) * 100_000.0 / 60_000.0;
            let (start, end) = (degrees("adj1", 270.0), degrees("adj2", 0.0));
            arc(w, h, start, (end - start).rem_euclid(360.0))
        }
        "leftBracket" => format!("M{} 0Q0 0 0 {}V{}Q0 {} {} {}", n(w), n(w), n(h - w), n(h), n(w), n(h)),
        "rightBracket" => format!("M0 0Q{} 0 {} {}V{}Q{} {} 0 {}", n(w), n(w), n(w), n(h - w), n(w), n(h), n(h)),
        "leftBrace" => format!(
            "M{} 0Q{} 0 {} {}V{}Q{} {} 0 {}Q{} {} {} {}V{}Q{} {} {} {}",
            n(w),
            n(w / 2.0),
            n(w / 2.0),
            n(w),
            n(h / 2.0 - w / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(h / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(w / 2.0),
            n(h / 2.0 + w / 2.0),
            n(h - w),
            n(w / 2.0),
            n(h),
            n(w),
            n(h)
        ),
        "rightBrace" => format!(
            "M0 0Q{} 0 {} {}V{}Q{} {} {} {}Q{} {} {} {}V{}Q{} {} 0 {}",
            n(w / 2.0),
            n(w / 2.0),
            n(w),
            n(h / 2.0 - w / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(w),
            n(h / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(w / 2.0),
            n(h / 2.0 + w / 2.0),
            n(h - w),
            n(w / 2.0),
            n(h),
            n(h)
        ),
        _ => return vec![shape(rect(w, h))],
    };
    vec![line(d)]
}

/// A shape drawn point by point: each path scaled from its own coordinates to the box.
fn custom(g: Node, w: f64, h: f64) -> Vec<Path> {
    let Some(list) = kid(g, "pathLst") else { return Vec::new() };
    kids(list, "path").map(|p| drawn(p, w, h)).collect()
}

fn drawn(p: Node, w: f64, h: f64) -> Path {
    // Without its own size a path is in the shape's EMUs.
    let scale = |own: Option<i64>, size: f64| own.filter(|v| *v > 0).map_or(PX / 12_700.0, |v| size / v as f64);
    let (sx, sy) = (scale(int(p, "w"), w), scale(int(p, "h"), h));
    let point = |q: Node| (int(q, "x").unwrap_or(0) as f64 * sx, int(q, "y").unwrap_or(0) as f64 * sy);
    let mut d = String::new();
    let mut at = (0.0, 0.0);
    for c in elems(p) {
        let pts: Vec<(f64, f64)> = kids(c, "pt").map(point).collect();
        match (c.tag_name().name(), pts.as_slice()) {
            ("moveTo", [q]) => {
                at = *q;
                let _ = write!(d, "M{} {}", n(q.0), n(q.1));
            }
            ("lnTo", [q]) => {
                at = *q;
                let _ = write!(d, "L{} {}", n(q.0), n(q.1));
            }
            ("cubicBezTo", [a, b, q]) => {
                at = *q;
                let _ = write!(d, "C{} {} {} {} {} {}", n(a.0), n(a.1), n(b.0), n(b.1), n(q.0), n(q.1));
            }
            ("quadBezTo", [a, q]) => {
                at = *q;
                let _ = write!(d, "Q{} {} {} {}", n(a.0), n(a.1), n(q.0), n(q.1));
            }
            ("arcTo", _) => at = arc_to(&mut d, c, at, (sx, sy)),
            ("close", _) => d.push('Z'),
            _ => {}
        }
    }
    let stroke = !matches!(attr(p, "stroke"), Some("0" | "false"));
    Path { d, fill: attr(p, "fill") != Some("none"), stroke, evenodd: false }
}

/// An `arcTo` from `at`: an ellipse's arc (radii in the path's coordinates, angles in 60000ths of
/// a degree, clockwise). Returns where it ends.
fn arc_to(d: &mut String, c: Node, at: (f64, f64), (sx, sy): (f64, f64)) -> (f64, f64) {
    let (rx, ry) = (int(c, "wR").unwrap_or(0) as f64 * sx, int(c, "hR").unwrap_or(0) as f64 * sy);
    let start = (int(c, "stAng").unwrap_or(0) as f64 / 60_000.0).to_radians();
    let sweep = (int(c, "swAng").unwrap_or(0) as f64 / 60_000.0).to_radians();
    let (cx, cy) = (at.0 - rx * start.cos(), at.1 - ry * start.sin());
    let point = |angle: f64| (cx + rx * angle.cos(), cy + ry * angle.sin());
    let sweep_flag = u8::from(sweep > 0.0);
    // A whole turn is two halves (an arc ending where it starts draws nothing).
    let steps: Vec<f64> = if sweep.abs() >= 2.0 * PI - 1e-6 { vec![start + sweep / 2.0, start + sweep] } else { vec![start + sweep] };
    let large = u8::from(steps.len() == 1 && sweep.abs() > PI);
    let mut end = at;
    for angle in &steps {
        end = point(*angle);
        let _ = write!(d, "A{} {} 0 {large} {sweep_flag} {} {}", n(rx), n(ry), n(end.0), n(end.1));
    }
    end
}
