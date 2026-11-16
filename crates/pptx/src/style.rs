//! Fills and outlines (of shapes, backgrounds and table cells), and the theme: its colours, its
//! fonts, and the fills and outlines shapes' styles refer to by number.

use std::collections::HashMap;

use crate::color::{self, Base, Color, Rgba};
use crate::package::{self, Node, Rels, at, attr, elems, int, kid, kids, pt};

/// A fill as written; its colours are resolved for the slide it's drawn on.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    None,
    Solid(Color),
    Grad(Grad),
    Image(Image),
}

/// A gradient: its stops (where, from 0 to 1, and the colour), its angle (degrees, 0 left to
/// right) or round from the middle.
#[derive(Clone, Debug, PartialEq)]
pub struct Grad {
    pub stops: Vec<(f64, Color)>,
    pub angle: f64,
    pub radial: bool,
}

/// A picture: its part, the parts cut off it (left, top, right, bottom, as fractions) and whether
/// it's tiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub part: String,
    pub crop: [f64; 4],
    pub tile: bool,
}

/// The fill among `n`'s children (`spPr`, `bgPr`, `tcPr`, `ln`…), when it has one.
pub fn fill(n: Node, rels: &Rels) -> Option<Fill> {
    elems(n).find_map(|c| fill_of(c, rels))
}

/// A fill element.
pub fn fill_of(c: Node, rels: &Rels) -> Option<Fill> {
    match c.tag_name().name() {
        "noFill" | "grpFill" => Some(Fill::None),
        "solidFill" => Some(color::within(c).map_or(Fill::None, Fill::Solid)),
        "gradFill" => Some(gradient(c)),
        "blipFill" => Some(image(c, rels).map_or(Fill::None, Fill::Image)),
        "pattFill" => Some(kid(c, "fgClr").and_then(color::within).map_or(Fill::None, Fill::Solid)),
        _ => None,
    }
}

fn gradient(g: Node) -> Fill {
    let mut stops: Vec<(f64, Color)> = kid(g, "gsLst")
        .map(|list| elems(list).filter_map(|s| Some((int(s, "pos")? as f64 / 100_000.0, color::within(s)?))).collect())
        .unwrap_or_default();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    if stops.is_empty() {
        return Fill::None;
    }
    let angle = kid(g, "lin").and_then(|l| int(l, "ang")).map_or(0.0, |a| a as f64 / 60_000.0);
    Fill::Grad(Grad { stops, angle, radial: kid(g, "path").is_some() })
}

/// The picture of a `blipFill`.
pub fn image(b: Node, rels: &Rels) -> Option<Image> {
    let blip = kid(b, "blip")?;
    let part = package::rid(blip, "embed").and_then(|id| rels.part(id))?.to_string();
    let crop = kid(b, "srcRect").map_or([0.0; 4], |r| ["l", "t", "r", "b"].map(|side| int(r, side).map_or(0.0, |v| v as f64 / 100_000.0)));
    Some(Image { part, crop, tile: kid(b, "tile").is_some() })
}

/// An outline as written, every part of it optional: an outline is made up from the theme's (a
/// style's), then the master's, the layout's and the shape's own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineSpec {
    pub fill: Option<Fill>,
    pub width: Option<f64>,
    pub dash: Option<String>,
    pub head: Option<String>,
    pub tail: Option<String>,
}

impl LineSpec {
    /// An `ln` element.
    pub fn parse(ln: Node, rels: &Rels) -> LineSpec {
        let end = |name: &str| kid(ln, name).and_then(|e| attr(e, "type")).filter(|t| *t != "none").map(str::to_string);
        LineSpec {
            fill: fill(ln, rels),
            width: int(ln, "w").map(pt),
            dash: kid(ln, "prstDash").and_then(|d| attr(d, "val")).map(str::to_string),
            head: end("headEnd"),
            tail: end("tailEnd"),
        }
    }

    /// `o`'s parts over this one's.
    pub fn over(&mut self, o: &LineSpec) {
        if o.fill.is_some() {
            self.fill.clone_from(&o.fill);
        }
        if o.width.is_some() {
            self.width = o.width;
        }
        if o.dash.is_some() {
            self.dash.clone_from(&o.dash);
        }
        if o.head.is_some() {
            self.head.clone_from(&o.head);
        }
        if o.tail.is_some() {
            self.tail.clone_from(&o.tail);
        }
    }
}

/// A theme's typefaces: Latin and East Asian (Korean when the theme names none for East Asia).
#[derive(Clone, Debug, Default)]
pub struct Fonts {
    pub latin: String,
    pub ea: String,
}

/// A theme: its colours by name (`dk1`, `accent1`…), its heading (major) and body (minor) fonts,
/// and the fills, outlines and background fills styles refer to by number.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub colors: HashMap<String, Rgba>,
    pub major: Fonts,
    pub minor: Fonts,
    pub fills: Vec<Fill>,
    pub lines: Vec<LineSpec>,
    pub backgrounds: Vec<Fill>,
}

impl Theme {
    pub fn parse(xml: &str) -> Theme {
        let Some(doc) = package::parse(xml) else { return Theme::default() };
        let Some(el) = kid(doc.root_element(), "themeElements") else { return Theme::default() };
        let none = Rels::default();
        let colors = kid(el, "clrScheme")
            .map(|s| elems(s).filter_map(|c| Some((c.tag_name().name().to_string(), rgb(&color::within(c)?)?))).collect())
            .unwrap_or_default();
        let list = |name: &str| at(el, &["fmtScheme", name]);
        Theme {
            colors,
            major: fonts(at(el, &["fontScheme", "majorFont"])),
            minor: fonts(at(el, &["fontScheme", "minorFont"])),
            fills: list("fillStyleLst").map(|l| elems(l).filter_map(|f| fill_of(f, &none)).collect()).unwrap_or_default(),
            lines: list("lnStyleLst").map(|l| kids(l, "ln").map(|ln| LineSpec::parse(ln, &none)).collect()).unwrap_or_default(),
            backgrounds: list("bgFillStyleLst").map(|l| elems(l).filter_map(|f| fill_of(f, &none)).collect()).unwrap_or_default(),
        }
    }

    /// The fill a style's `fillRef` (or a background's `bgRef`) number names: 1 and up the fills,
    /// 1001 and up the background fills.
    pub fn fill(&self, idx: i64) -> Option<&Fill> {
        if idx >= 1001 { self.backgrounds.get(usize::try_from(idx - 1001).ok()?) } else { self.fills.get(usize::try_from(idx - 1).ok()?) }
    }

    /// The outline a style's `lnRef` number names (1 and up).
    pub fn line(&self, idx: i64) -> Option<&LineSpec> {
        self.lines.get(usize::try_from(idx - 1).ok()?)
    }
}

fn rgb(c: &Color) -> Option<Rgba> {
    match &c.base {
        Base::Rgb(rgb) => Some(color::apply(*rgb, &c.mods)),
        _ => None,
    }
}

fn fonts(f: Option<Node>) -> Fonts {
    let Some(f) = f else { return Fonts::default() };
    let face = |name: &str| kid(f, name).and_then(|n| attr(n, "typeface")).unwrap_or_default().to_string();
    let korean = kids(f, "font").find(|n| attr(*n, "script") == Some("Hang")).and_then(|n| attr(n, "typeface")).unwrap_or_default();
    let ea = face("ea");
    Fonts { latin: face("latin"), ea: if ea.is_empty() { korean.to_string() } else { ea } }
}
