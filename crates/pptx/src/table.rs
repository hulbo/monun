//! Tables: an HTML table in the frame's box, its cells filled, bordered and their text set by the
//! table's style (`tableStyles.xml`: the whole table, banded rows and columns, the first and last
//! rows and columns) and their own settings. A style the file doesn't keep is drawn as
//! PowerPoint's default one, Medium Style 2 in the first accent colour.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::color::{self, Color};
use crate::package::{self, Node, Rels, attr, elems, esc, flag, int, kid, kids, pt};
use crate::scene::{Ctx, Frame, Out};
use crate::shape::Scene;
use crate::style::{self, Fill, LineSpec};
use crate::text::{self, Body, Run, Text};

/// A fill or outline of a table style: its own, or the theme's by number in a colour.
#[derive(Clone, Debug)]
enum Ref<T> {
    Own(T),
    Theme(i64, Option<Color>),
}

/// A part of a table style (the whole table, the first row…): its fill, text colour and
/// weight, and borders by side (`left`… and `insideH`, `insideV`).
#[derive(Clone, Debug, Default)]
struct Part {
    fill: Option<Ref<Fill>>,
    color: Option<Color>,
    bold: Option<bool>,
    italic: Option<bool>,
    borders: HashMap<String, Ref<LineSpec>>,
}

impl Part {
    fn parse(n: Node) -> Part {
        let none = Rels::default();
        let tx = kid(n, "tcTxStyle");
        let tc = kid(n, "tcStyle");
        let on = |name: &str| tx.and_then(|t| attr(t, name)).map(|v| v == "on");
        let fill = tc.and_then(|t| {
            if let Some(f) = kid(t, "fill") {
                style::fill(f, &none).map(Ref::Own)
            } else {
                kid(t, "fillRef").map(|r| Ref::Theme(int(r, "idx").unwrap_or(0), color::within(r)))
            }
        });
        let borders = tc
            .and_then(|t| kid(t, "tcBdr"))
            .map(|b| {
                elems(b)
                    .filter_map(|side| {
                        let line = if let Some(ln) = kid(side, "ln") {
                            Ref::Own(LineSpec::parse(ln, &none))
                        } else {
                            let r = kid(side, "lnRef")?;
                            Ref::Theme(int(r, "idx").unwrap_or(0), color::within(r))
                        };
                        Some((side.tag_name().name().to_string(), line))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Part {
            fill,
            color: tx.and_then(|t| color::within(t).or_else(|| kid(t, "fontRef").and_then(color::within))),
            bold: on("b"),
            italic: on("i"),
            borders,
        }
    }
}

/// A table style: its parts by name.
#[derive(Clone, Debug, Default)]
struct TableStyle {
    parts: HashMap<String, Part>,
}

/// The table styles a presentation keeps, and its default one.
#[derive(Debug, Default)]
pub struct TableStyles {
    by_id: HashMap<String, TableStyle>,
    default: Option<String>,
}

impl TableStyles {
    pub fn parse(xml: Option<&str>) -> TableStyles {
        let Some(doc) = xml.and_then(package::parse) else { return TableStyles::default() };
        let root = doc.root_element();
        let by_id = kids(root, "tblStyle")
            .filter_map(|s| {
                let parts = elems(s).map(|p| (p.tag_name().name().to_string(), Part::parse(p))).collect();
                Some((attr(s, "styleId")?.to_string(), TableStyle { parts }))
            })
            .collect();
        TableStyles { by_id, default: attr(root, "def").map(str::to_string) }
    }

    fn get(&self, id: Option<&str>) -> TableStyle {
        id.or(self.default.as_deref()).and_then(|id| self.by_id.get(id)).cloned().unwrap_or_else(medium_two)
    }
}

/// PowerPoint's default table style, Medium Style 2 in the first accent colour: light rows in two
/// tints, the first and last rows and columns in the accent, white rules between cells.
fn medium_two() -> TableStyle {
    let accent = |tint: Option<f64>| {
        let c = Color::scheme("accent1");
        Some(Ref::Own(Fill::Solid(tint.map_or(c.clone(), |t| c.with("tint", t)))))
    };
    let rule = |w: f64| Ref::Own(LineSpec { fill: Some(Fill::Solid(Color::scheme("lt1"))), width: Some(w), ..LineSpec::default() });
    let mut parts = HashMap::new();
    let all_sides = ["left", "right", "top", "bottom", "insideH", "insideV"].map(|s| (s.to_string(), rule(1.0))).into_iter().collect();
    parts.insert("wholeTbl".to_string(), Part { fill: accent(Some(0.2)), color: Some(Color::scheme("dk1")), borders: all_sides, ..Part::default() });
    parts.insert("band1H".to_string(), Part { fill: accent(Some(0.4)), ..Part::default() });
    parts.insert("band1V".to_string(), Part { fill: accent(Some(0.4)), ..Part::default() });
    let strong = |edge: Option<&str>| Part {
        fill: accent(None),
        color: Some(Color::scheme("lt1")),
        bold: Some(true),
        borders: edge.map(|e| (e.to_string(), rule(3.0))).into_iter().collect(),
        ..Part::default()
    };
    parts.insert("firstRow".to_string(), strong(Some("bottom")));
    parts.insert("lastRow".to_string(), strong(Some("top")));
    parts.insert("firstCol".to_string(), strong(None));
    parts.insert("lastCol".to_string(), strong(None));
    TableStyle { parts }
}

/// Which of the table's parts apply (`tblPr`'s switches).
struct Switches {
    first_row: bool,
    last_row: bool,
    first_col: bool,
    last_col: bool,
    band_row: bool,
    band_col: bool,
}

/// A cell's place in the table: its row and column, and how many there are.
#[derive(Clone, Copy)]
struct Place {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}

impl Place {
    /// The style parts a cell takes, lowest first, with whether each is a band of rows (its own
    /// top and bottom borders are the cell's) or of columns.
    fn parts(&self, s: &Switches) -> Vec<(&'static str, bool)> {
        let mut list = vec![("wholeTbl", false)];
        let body_col = self.col.saturating_sub(usize::from(s.first_col));
        let body_row = self.row.saturating_sub(usize::from(s.first_row));
        let in_body_rows = !(s.first_row && self.row == 0 || s.last_row && self.row + 1 == self.rows);
        let in_body_cols = !(s.first_col && self.col == 0 || s.last_col && self.col + 1 == self.cols);
        if s.band_col && in_body_cols {
            list.push((if body_col % 2 == 0 { "band1V" } else { "band2V" }, false));
        }
        if s.band_row && in_body_rows {
            list.push((if body_row % 2 == 0 { "band1H" } else { "band2H" }, true));
        }
        if s.first_col && self.col == 0 {
            list.push(("firstCol", false));
        }
        if s.last_col && self.col + 1 == self.cols {
            list.push(("lastCol", false));
        }
        if s.last_row && self.row + 1 == self.rows {
            list.push(("lastRow", true));
        }
        if s.first_row && self.row == 0 {
            list.push(("firstRow", true));
        }
        list
    }

    /// The border names of a cell's top, right, bottom and left in a part of the table (a band of
    /// rows: its own top and bottom; of columns: its own left and right).
    fn sides(&self, rows_band: bool, cols_band: bool) -> [&'static str; 4] {
        let top = if rows_band || self.row == 0 { "top" } else { "insideH" };
        let bottom = if rows_band || self.row + 1 == self.rows { "bottom" } else { "insideH" };
        let left = if cols_band || self.col == 0 { "left" } else { "insideV" };
        let right = if cols_band || self.col + 1 == self.cols { "right" } else { "insideV" };
        [top, right, bottom, left]
    }
}

/// The table `tbl` in `frame`.
pub fn html(scene: &Scene, out: &mut Out, tbl: Node, own: &Ctx, frame: &Frame) {
    let pr = kid(tbl, "tblPr");
    let on = |name: &str| pr.and_then(|p| flag(p, name)) == Some(true);
    let switches = Switches {
        first_row: on("firstRow"),
        last_row: on("lastRow"),
        first_col: on("firstCol"),
        last_col: on("lastCol"),
        band_row: on("bandRow"),
        band_col: on("bandCol"),
    };
    let style = scene.tables.get(pr.and_then(|p| kid(p, "tableStyleId")).and_then(|i| i.text()).map(str::trim));
    let cols: Vec<f64> = kid(tbl, "tblGrid").map(|g| kids(g, "gridCol").map(|c| int(c, "w").map_or(0.0, pt)).collect()).unwrap_or_default();
    let rows: Vec<Node> = kids(tbl, "tr").collect();
    let width: f64 = cols.iter().sum();
    let _ = write!(out.html, "<table class=\"t\" style=\"left:{:.2}pt;top:{:.2}pt;width:{width:.2}pt\"><colgroup>", frame.x, frame.y);
    for w in &cols {
        let _ = write!(out.html, "<col style=\"width:{w:.2}pt\">");
    }
    out.html.push_str("</colgroup>");
    for (r, tr) in rows.iter().enumerate() {
        let _ = write!(out.html, "<tr style=\"height:{:.2}pt\">", int(*tr, "h").map_or(0.0, pt));
        for (c, tc) in kids(*tr, "tc").enumerate() {
            if flag(tc, "hMerge") == Some(true) || flag(tc, "vMerge") == Some(true) {
                continue;
            }
            let place = Place { row: r, col: c, rows: rows.len(), cols: cols.len() };
            cell(scene, out, tc, own, &style, (&switches, place));
        }
        out.html.push_str("</tr>");
    }
    out.html.push_str("</table>");
}

fn cell(scene: &Scene, out: &mut Out, tc: Node, own: &Ctx, style: &TableStyle, (switches, place): (&Switches, Place)) {
    let pr = kid(tc, "tcPr");
    let mut fill: Option<(Fill, Option<Color>)> = None;
    let mut run = Run::default();
    let mut edges: [Option<(LineSpec, Option<Color>)>; 4] = Default::default();
    for (name, rows_band) in place.parts(switches) {
        let Some(part) = style.parts.get(name) else { continue };
        if let Some(f) = &part.fill {
            fill = resolve_fill(own, f);
        }
        run.over(&Run { color: part.color.clone(), bold: part.bold, italic: part.italic, ..Run::default() });
        let cols_band = matches!(name, "firstCol" | "lastCol" | "band1V" | "band2V");
        for (edge, side) in edges.iter_mut().zip(place.sides(rows_band, cols_band)) {
            if let Some(line) = part.borders.get(side) {
                *edge = resolve_line(own, line);
            }
        }
    }
    if let Some(pr) = pr {
        if let Some(f) = style::fill(pr, &own.rels) {
            fill = Some((f, None));
        }
        for (edge, name) in edges.iter_mut().zip(["lnT", "lnR", "lnB", "lnL"]) {
            if let Some(ln) = kid(pr, name) {
                *edge = Some((LineSpec::parse(ln, &own.rels), None));
            }
        }
    }
    let mut css = String::new();
    if let Some((f, ph)) = &fill
        && let Some(bg) = own.paint(out, f, ph.as_ref().map(|c| own.rgba(c, None))).css()
    {
        let _ = write!(css, "background:{bg};");
    }
    for ((line, ph), side) in edges.iter().zip(["top", "right", "bottom", "left"]).filter_map(|(e, s)| Some((e.as_ref()?, s))) {
        let paint = line.fill.as_ref().map(|f| own.paint(out, f, ph.as_ref().map(|c| own.rgba(c, None))));
        if let Some(c) = paint.and_then(|p| p.first()) {
            let _ = write!(css, "border-{side}:{:.2}pt solid {};", line.width.unwrap_or(1.0), c.css());
        }
    }
    let anchor = pr.and_then(|p| attr(p, "anchor"));
    let _ = write!(
        css,
        "vertical-align:{}",
        match anchor {
            Some("ctr") => "middle",
            Some("b") => "bottom",
            _ => "top",
        }
    );
    let margin = |name: &str, default: f64| Some(pr.and_then(|p| int(p, name)).map_or(default, pt));
    let body = Body { ins: [margin("marL", 7.2), margin("marT", 3.6), margin("marR", 7.2), margin("marB", 3.6)], ..Body::default() };
    let mut levels = scene.styles.default.clone();
    text::stack(&mut levels, &scene.styles.other);
    let content = kid(tc, "txBody")
        .map(|tx| text::html(&Text { ctx: own, levels: &levels, body: &body, base: &run, slide: scene.number, flow: true }, tx))
        .unwrap_or_default();
    let span = |name: &str, html: &str| int(tc, name).filter(|v| *v > 1).map(|v| format!(" {html}=\"{v}\"")).unwrap_or_default();
    let _ = write!(out.html, "<td{}{} style=\"{}\">{content}</td>", span("gridSpan", "colspan"), span("rowSpan", "rowspan"), esc(&css));
}

fn resolve_fill(own: &Ctx, f: &Ref<Fill>) -> Option<(Fill, Option<Color>)> {
    match f {
        Ref::Own(fill) => Some((fill.clone(), None)),
        Ref::Theme(idx, color) => own.theme.fill(*idx).map(|fill| (fill.clone(), color.clone())),
    }
}

fn resolve_line(own: &Ctx, l: &Ref<LineSpec>) -> Option<(LineSpec, Option<Color>)> {
    match l {
        Ref::Own(line) => Some((line.clone(), None)),
        Ref::Theme(idx, color) => own.theme.line(*idx).map(|line| (line.clone(), color.clone())),
    }
}
