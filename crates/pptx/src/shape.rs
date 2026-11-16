//! A slide's shapes: shapes and connectors, pictures, groups and graphic frames (tables, charts,
//! SmartArt, embedded objects' pictures), each taking what it doesn't say from its placeholder on
//! the layout and on the master.

use std::fmt::Write as _;

use crate::Labels;
use crate::color;
use crate::geom::{self, Stroke};
use crate::package::{self, Node, at, attr, elems, esc, int, kid, rid};
use crate::scene::{Ctx, Frame, Out, Paint, Space, xfrm};
use crate::style::{self, LineSpec};
use crate::table::{self, TableStyles};
use crate::text::{self, Body, Levels, Run, Text};

/// The lists of levels text starts from: the presentation's, and the master's for titles, body
/// text and the rest.
pub struct Styles {
    pub default: Levels,
    pub title: Levels,
    pub body: Levels,
    pub other: Levels,
}

/// A slide being drawn: its layout and master, their contexts, the styles text starts from.
pub struct Scene<'a> {
    pub layout: Option<Node<'a, 'a>>,
    pub master: Option<Node<'a, 'a>>,
    pub layout_ctx: &'a Ctx<'a>,
    pub master_ctx: &'a Ctx<'a>,
    pub styles: &'a Styles,
    pub tables: &'a TableStyles,
    pub labels: &'a Labels,
    /// The slide's number.
    pub number: usize,
}

/// A shape and the context of the part it's in; a shape and its placeholders on the layout and
/// the master, in that order, are its sources.
type Src<'c> = (Node<'c, 'c>, &'c Ctx<'c>);

/// A placeholder: its type (`title`, `body`…; `obj` when unset) and number.
#[derive(Clone, Copy, Debug)]
pub struct Ph<'c> {
    pub kind: &'c str,
    pub idx: Option<&'c str>,
}

/// The placeholder a shape is, if any.
pub fn ph<'c>(n: Node<'c, '_>) -> Option<Ph<'c>> {
    let nv = elems(n).find(|c| c.tag_name().name().starts_with("nv"))?;
    let p = at(nv, &["nvPr", "ph"])?;
    Some(Ph { kind: attr(p, "type").unwrap_or("obj"), idx: attr(p, "idx") })
}

/// Placeholder types that stand in for one another: titles, the date, footer, slide number and
/// header each their own, everything else body text.
fn family(kind: &str) -> &str {
    match kind {
        "title" | "ctrTitle" => "title",
        "dt" | "ftr" | "sldNum" | "hdr" => kind,
        _ => "body",
    }
}

/// The placeholder in the layout or master `root` that `p` takes after: by number (on a layout),
/// else by type, else by family.
fn find<'c>(root: Node<'c, 'c>, p: &Ph, by_idx: bool) -> Option<Node<'c, 'c>> {
    let tree = at(root, &["cSld", "spTree"])?;
    let all: Vec<(Node<'c, 'c>, Ph<'c>)> = tree.descendants().filter(Node::is_element).filter_map(|d| ph(d).map(|q| (d, q))).collect();
    if let (true, Some(i)) = (by_idx, p.idx)
        && let Some((n, _)) = all.iter().find(|(_, q)| q.idx == Some(i))
    {
        return Some(*n);
    }
    all.iter().find(|(_, q)| q.kind == p.kind).or_else(|| all.iter().find(|(_, q)| family(q.kind) == family(p.kind))).map(|(n, _)| *n)
}

/// The shapes among `tree`'s children, alternate content opened (its fallback, which uses only
/// what every version reads).
fn shapes<'c>(tree: Node<'c, 'c>) -> Vec<Node<'c, 'c>> {
    let mut out = Vec::new();
    for c in elems(tree) {
        if c.tag_name().name() == "AlternateContent" {
            if let Some(pick) = kid(c, "Fallback").or_else(|| kid(c, "Choice")) {
                out.extend(elems(pick));
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn frame_of(src: &[Src], path: &[&str]) -> Option<Frame> {
    src.iter().find_map(|(s, _)| at(*s, path).and_then(xfrm))
}

/// The run settings a shape's style gives its text: its font (heading or body) and colour.
fn font_ref(style: Option<Node>) -> Run {
    let Some(f) = style.and_then(|s| kid(s, "fontRef")) else { return Run::default() };
    let face = match attr(f, "idx") {
        Some("major") => Some("+mj"),
        Some("minor") => Some("+mn"),
        _ => None,
    };
    Run { color: color::within(f), latin: face.map(|f| format!("{f}-lt")), ea: face.map(|f| format!("{f}-ea")), ..Run::default() }
}

impl<'a> Scene<'a> {
    /// The shapes of `tree` (an `spTree` or a group) in `space`; a layout's or master's
    /// placeholders (`templates`) are left out, as they only shape the slide's.
    pub fn tree<'c>(&self, out: &mut Out, tree: Node<'c, 'c>, own: &'c Ctx<'c>, space: Space, templates: bool)
    where
        'a: 'c,
    {
        for n in shapes(tree) {
            if templates && ph(n).is_some() {
                continue;
            }
            match n.tag_name().name() {
                "sp" | "cxnSp" => self.sp(out, n, own, space),
                "pic" => self.pic(out, n, own, space, None),
                "grpSp" => self.group(out, n, own, space, templates),
                "graphicFrame" => self.graphic(out, n, own, space),
                _ => {}
            }
        }
    }

    /// A shape and its placeholders on the layout and the master.
    fn sources<'c>(&self, n: Node<'c, 'c>, own: &'c Ctx<'c>) -> Vec<Src<'c>>
    where
        'a: 'c,
    {
        let mut src: Vec<Src<'c>> = vec![(n, own)];
        if let Some(p) = ph(n) {
            if let Some(l) = self.layout.and_then(|l| find(l, &p, true)) {
                src.push((l, self.layout_ctx));
            }
            if let Some(m) = self.master.and_then(|m| find(m, &p, false)) {
                src.push((m, self.master_ctx));
            }
        }
        src
    }

    fn sp<'c>(&self, out: &mut Out, n: Node<'c, 'c>, own: &'c Ctx<'c>, space: Space)
    where
        'a: 'c,
    {
        let src = self.sources(n, own);
        let Some(frame) = frame_of(&src, &["spPr", "xfrm"]).map(|f| space.map(f)) else { return };
        let geometry = src.iter().find_map(|(s, _)| kid(*s, "spPr").and_then(|sp| kid(sp, "prstGeom").or_else(|| kid(sp, "custGeom"))));
        let style = src.iter().find_map(|(s, c)| kid(*s, "style").map(|st| (st, *c)));
        let fill = self.fill(out, &src, style);
        let stroke = self.stroke(out, &src, style);
        let svg = geom::svg(out, geometry, &frame, &fill, stroke.as_ref());
        let text = kid(n, "txBody").map(|tx| self.text(n, tx, &src, style, (&frame, space))).unwrap_or_default();
        if !svg.is_empty() || !text.is_empty() {
            let _ = write!(out.html, "<div class=\"sp\" style=\"{}\">{svg}{text}</div>", frame.css());
        }
    }

    /// A shape's fill: its own or its placeholders', else its style's.
    fn fill(&self, out: &mut Out, src: &[Src], style: Option<Src>) -> Paint {
        for (s, c) in src {
            if let Some(f) = kid(*s, "spPr").and_then(|sp| style::fill(sp, &c.rels)) {
                return c.paint(out, &f, None);
            }
        }
        let Some(r) = style.and_then(|(st, _)| kid(st, "fillRef")) else { return Paint::None };
        let Some((_, c)) = style else { return Paint::None };
        let ph = color::within(r).map(|col| c.rgba(&col, None));
        c.theme.fill(int(r, "idx").unwrap_or(0)).map_or(Paint::None, |f| c.paint(out, f, ph))
    }

    /// A shape's outline: its style's, then the master's, the layout's and its own over it.
    fn stroke(&self, out: &mut Out, src: &[Src], style: Option<Src>) -> Option<Stroke> {
        let (_, own) = src.first()?;
        let mut spec = LineSpec::default();
        let mut ph = None;
        if let Some((st, c)) = style
            && let Some(r) = kid(st, "lnRef")
        {
            ph = color::within(r).map(|col| c.rgba(&col, None));
            if let Some(l) = c.theme.line(int(r, "idx").unwrap_or(0)) {
                spec = l.clone();
            }
        }
        for (s, c) in src.iter().rev() {
            if let Some(ln) = at(*s, &["spPr", "ln"]) {
                spec.over(&LineSpec::parse(ln, &c.rels));
            }
        }
        let paint = own.paint(out, &spec.fill?, ph);
        if matches!(paint, Paint::None) {
            return None;
        }
        Some(Stroke { paint, width: spec.width.unwrap_or(0.75), dash: spec.dash, head: spec.head, tail: spec.tail })
    }

    /// A shape's text, in its box (or the box SmartArt gives it, `txXfrm`).
    fn text(&self, n: Node, tx: Node, src: &[Src], style: Option<Src>, (frame, space): (&Frame, Space)) -> String {
        let Some((_, own)) = src.first() else { return String::new() };
        let p = ph(n);
        let mut levels = self.styles.default.clone();
        let start = match p.map(|p| family(p.kind)) {
            Some("title") => &self.styles.title,
            Some("body") => &self.styles.body,
            Some(_) => &self.styles.other,
            None => &self.styles.default,
        };
        text::stack(&mut levels, start);
        let mut body = Body::default();
        for (s, _) in src.iter().rev() {
            if let Some(list) = at(*s, &["txBody", "lstStyle"]) {
                text::stack(&mut levels, &text::levels(Some(list)));
            }
            if let Some(b) = at(*s, &["txBody", "bodyPr"]) {
                body.over(&Body::parse(b));
            }
        }
        let base = font_ref(style.map(|s| s.0));
        let t = Text { ctx: own, levels: &levels, body: &body, base: &base, slide: self.number, flow: false };
        let html = text::html(&t, tx);
        match kid(n, "txXfrm").and_then(xfrm).map(|f| space.map(f)) {
            Some(box_) if !html.is_empty() => {
                let inner = Frame { x: box_.x - frame.x, y: box_.y - frame.y, rot: 0.0, ..box_ };
                format!("<div class=\"sp\" style=\"{}\">{html}</div>", inner.css())
            }
            _ => html,
        }
    }

    /// A picture, cut to its crop, in its box (or `frame`, an embedded object's).
    fn pic<'c>(&self, out: &mut Out, n: Node<'c, 'c>, own: &'c Ctx<'c>, space: Space, frame: Option<Frame>)
    where
        'a: 'c,
    {
        let src = self.sources(n, own);
        let Some(frame) = frame.or_else(|| frame_of(&src, &["spPr", "xfrm"]).map(|f| space.map(f))) else { return };
        let Some(image) = kid(n, "blipFill").and_then(|b| style::image(b, &own.rels)) else { return };
        let Some(url) = out.media(own.pkg, &image.part) else { return };
        let [l, t, r, b] = image.crop;
        let (w, h) = (frame.w / (1.0 - l - r).max(0.01), frame.h / (1.0 - t - b).max(0.01));
        let geometry = kid(n, "spPr").and_then(|sp| kid(sp, "prstGeom"));
        let mut css = format!("{};overflow:hidden", frame.css());
        match geometry.and_then(|g| attr(g, "prst")) {
            Some("ellipse") => css.push_str(";border-radius:50%"),
            Some("roundRect") => {
                let _ = write!(css, ";border-radius:{:.2}pt", frame.w.min(frame.h) * 0.166_67);
            }
            _ => {}
        }
        if let Some(alpha) = at(n, &["blipFill", "blip", "alphaModFix"]).and_then(|a| int(a, "amt")) {
            let _ = write!(css, ";opacity:{:.3}", alpha as f64 / 100_000.0);
        }
        let flip = match (frame.flip_h, frame.flip_v) {
            (false, false) => "",
            (true, false) => ";transform:scale(-1,1)",
            (false, true) => ";transform:scale(1,-1)",
            (true, true) => ";transform:scale(-1,-1)",
        };
        let style = src.iter().find_map(|(s, c)| kid(*s, "style").map(|st| (st, *c)));
        let outline = self
            .stroke(out, &src, style)
            .map(|s| geom::svg(out, geometry, &Frame { flip_h: false, flip_v: false, ..frame }, &Paint::None, Some(&s)))
            .unwrap_or_default();
        let _ = write!(
            out.html,
            "<div class=\"sp\" style=\"{css}\"><img alt=\"\" src=\"{}\" style=\"left:{:.2}pt;top:{:.2}pt;width:{w:.2}pt;height:{h:.2}pt{flip}\">{outline}</div>",
            esc(&url),
            // (`+ 0.0`: no `-0.00` when nothing is cut off.)
            -l * w + 0.0,
            -t * h + 0.0
        );
    }

    /// A group: its shapes in its own coordinates, scaled to its box.
    fn group<'c>(&self, out: &mut Out, n: Node<'c, 'c>, own: &'c Ctx<'c>, space: Space, templates: bool)
    where
        'a: 'c,
    {
        let x = at(n, &["grpSpPr", "xfrm"]);
        let Some((x, raw)) = x.and_then(|x| Some((x, xfrm(x)?))) else {
            self.tree(out, n, own, space, templates);
            return;
        };
        let frame = space.map(raw);
        let child = |name: &str, key: &str| kid(x, name).and_then(|c| int(c, key)).map(package::pt);
        let (ox, oy) = (child("chOff", "x").unwrap_or(raw.x), child("chOff", "y").unwrap_or(raw.y));
        let (cw, ch) = (child("chExt", "cx").unwrap_or(raw.w), child("chExt", "cy").unwrap_or(raw.h));
        let scale = |size: f64, of: f64| if of > 0.0 { size / of } else { 1.0 };
        let inner = Space { ox, oy, sx: scale(frame.w, cw), sy: scale(frame.h, ch) };
        let _ = write!(out.html, "<div class=\"sp\" style=\"{}\">", frame.css());
        self.tree(out, n, own, inner, templates);
        out.html.push_str("</div>");
    }

    /// A graphic frame: a table, a chart, SmartArt, or an embedded object's picture.
    fn graphic<'c>(&self, out: &mut Out, n: Node<'c, 'c>, own: &'c Ctx<'c>, space: Space)
    where
        'a: 'c,
    {
        let src = self.sources(n, own);
        let Some(frame) = frame_of(&src, &["xfrm"]).map(|f| space.map(f)) else { return };
        let Some(data) = at(n, &["graphic", "graphicData"]) else { return };
        let uri = attr(data, "uri").unwrap_or_default();
        if let Some(tbl) = kid(data, "tbl") {
            table::html(self, out, tbl, own, &frame);
        } else if uri.ends_with("/chart") || uri.ends_with("/chartex") {
            self.chart(out, data, own, &frame);
        } else if uri.ends_with("/diagram") {
            self.diagram(out, data, own, &frame);
        } else if let Some(p) = data.descendants().find(|d| d.is_element() && d.tag_name().name() == "pic") {
            self.pic(out, p, own, space, Some(frame));
        }
    }

    /// A chart: a box with its title.
    fn chart(&self, out: &mut Out, data: Node, own: &Ctx, frame: &Frame) {
        let title = data
            .descendants()
            .find(|d| d.is_element() && d.tag_name().name() == "chart")
            .and_then(|c| rid(c, "id"))
            .and_then(|id| own.rels.part(id))
            .and_then(|p| own.pkg.text(p))
            .map(|xml| chart_title(&xml))
            .unwrap_or_default();
        let label = if title.is_empty() { self.labels.chart.clone() } else { format!("{}: {title}", self.labels.chart) };
        let _ = write!(out.html, "<div class=\"chart\" style=\"{}\">{}</div>", frame.css(), esc(&label));
    }

    /// SmartArt: the shapes of the drawing PowerPoint keeps of it, in its box.
    fn diagram(&self, out: &mut Out, data: Node, own: &Ctx, frame: &Frame) {
        let Some(part) = drawing(data, own) else { return };
        let Some(xml) = own.pkg.text(&part) else { return };
        let Some(doc) = package::parse(&xml) else { return };
        let Some(tree) = kid(doc.root_element(), "spTree") else { return };
        let ctx = own.with_rels(own.pkg.rels(&part));
        let _ = write!(out.html, "<div class=\"sp\" style=\"{}\">", Frame { rot: 0.0, ..*frame }.css());
        self.tree(out, tree, &ctx, Space::SLIDE, false);
        out.html.push_str("</div>");
    }
}

/// The drawing part of a SmartArt frame: named in its data part, or the slide's only one.
fn drawing(data: Node, own: &Ctx) -> Option<String> {
    let dm = kid(data, "relIds").and_then(|ids| rid(ids, "dm")).and_then(|id| own.rels.part(id));
    let named = dm.and_then(|p| own.pkg.text(p)).and_then(|xml| {
        let doc = package::parse(&xml)?;
        let id = doc.descendants().find(|d| d.is_element() && d.tag_name().name() == "dataModelExt").and_then(|d| attr(d, "relId"))?.to_string();
        own.rels.part(&id).map(str::to_string)
    });
    named.or_else(|| own.rels.first("diagramDrawing").map(str::to_string))
}

/// A chart part's title, its runs joined.
fn chart_title(xml: &str) -> String {
    let Some(doc) = package::parse(xml) else { return String::new() };
    let Some(title) = doc.descendants().find(|d| d.is_element() && d.tag_name().name() == "title") else { return String::new() };
    title.descendants().filter(|d| d.is_element() && d.tag_name().name() == "t").filter_map(|t| t.text()).collect::<Vec<_>>().join("")
}
