//! Text: the paragraph and run settings a shape's text takes from its lists of levels (the
//! presentation's, the master's, the layout's and the shape's own), a text body's own settings,
//! and the HTML of its paragraphs.

use std::fmt::Write as _;

use crate::color::{self, Color};
use crate::package::{Node, attr, elems, esc, flag, int, kid, kids, pt};
use crate::scene::Ctx;

/// A run's settings, each one set or taken from below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub latin: Option<String>,
    pub ea: Option<String>,
    pub baseline: Option<f64>,
    pub caps: Option<String>,
    pub highlight: Option<Color>,
    pub link: bool,
}

/// `$o`'s set fields over `$s`'s.
macro_rules! over {
    ($s:ident, $o:ident, $($f:ident),*) => {
        $(if $o.$f.is_some() { $s.$f.clone_from(&$o.$f); })*
    };
}

impl Run {
    /// An `rPr`, `defRPr` or `endParaRPr`.
    pub fn parse(r: Node) -> Run {
        let face = |name: &str| kid(r, name).and_then(|f| attr(f, "typeface")).filter(|t| !t.is_empty()).map(str::to_string);
        let gradient = || kid(r, "gradFill").and_then(|g| kid(g, "gsLst")).and_then(|l| elems(l).next()).and_then(color::within);
        Run {
            size: int(r, "sz").map(|v| v as f64 / 100.0),
            bold: flag(r, "b"),
            italic: flag(r, "i"),
            underline: attr(r, "u").map(|u| u != "none"),
            strike: attr(r, "strike").map(|s| s != "noStrike"),
            color: kid(r, "solidFill").and_then(color::within).or_else(gradient),
            latin: face("latin"),
            ea: face("ea"),
            baseline: int(r, "baseline").map(|v| v as f64 / 1000.0),
            caps: attr(r, "cap").map(str::to_string),
            highlight: kid(r, "highlight").and_then(color::within),
            link: kid(r, "hlinkClick").is_some(),
        }
    }

    pub fn over(&mut self, o: &Run) {
        over!(self, o, size, bold, italic, underline, strike, color, latin, ea, baseline, caps, highlight);
        self.link |= o.link;
    }
}

/// Line or paragraph spacing: a share of the line, or points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Space {
    Pct(f64),
    Pts(f64),
}

/// A paragraph's mark: none, a character, or a number (its scheme, `arabicPeriod`…, and where
/// it starts).
#[derive(Clone, Debug, PartialEq)]
pub enum Bullet {
    None,
    Char(String),
    Num(String, u32),
}

/// A paragraph's settings (of a level), each one set or taken from below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    pub align: Option<String>,
    pub mar_l: Option<f64>,
    pub indent: Option<f64>,
    pub line: Option<Space>,
    pub before: Option<Space>,
    pub after: Option<Space>,
    pub bullet: Option<Bullet>,
    /// The mark's colour: its own, or (`Some(None)`) the text's.
    pub bu_color: Option<Option<Color>>,
    pub bu_size: Option<f64>,
    /// The mark's typeface: its own, or (`Some(None)`) the text's.
    pub bu_font: Option<Option<String>>,
    pub run: Run,
}

impl Para {
    /// A `pPr` or `lvlNpPr`.
    pub fn parse(p: Node) -> Para {
        Para {
            align: attr(p, "algn").map(str::to_string),
            mar_l: int(p, "marL").map(pt),
            indent: int(p, "indent").map(pt),
            line: kid(p, "lnSpc").and_then(space),
            before: kid(p, "spcBef").and_then(space),
            after: kid(p, "spcAft").and_then(space),
            bullet: bullet(p),
            bu_color: if kid(p, "buClrTx").is_some() { Some(None) } else { kid(p, "buClr").and_then(color::within).map(Some) },
            bu_size: kid(p, "buSzPct").and_then(|s| int(s, "val")).map(|v| v as f64 / 100_000.0),
            bu_font: if kid(p, "buFontTx").is_some() {
                Some(None)
            } else {
                kid(p, "buFont").and_then(|f| attr(f, "typeface")).map(|t| Some(t.to_string()))
            },
            run: kid(p, "defRPr").map(Run::parse).unwrap_or_default(),
        }
    }

    pub fn over(&mut self, o: &Para) {
        over!(self, o, align, mar_l, indent, line, before, after, bullet, bu_color, bu_size, bu_font);
        self.run.over(&o.run);
    }
}

fn space(s: Node) -> Option<Space> {
    if let Some(p) = kid(s, "spcPct") {
        return Some(Space::Pct(int(p, "val")? as f64 / 100_000.0));
    }
    kid(s, "spcPts").and_then(|p| int(p, "val")).map(|v| Space::Pts(v as f64 / 100.0))
}

fn bullet(p: Node) -> Option<Bullet> {
    elems(p).find_map(|c| match c.tag_name().name() {
        "buNone" => Some(Bullet::None),
        "buChar" => Some(Bullet::Char(attr(c, "char").unwrap_or("•").to_string())),
        "buBlip" => Some(Bullet::Char("•".into())),
        "buAutoNum" => {
            let start = int(c, "startAt").map_or(1, |v| u32::try_from(v.clamp(1, 32_767)).unwrap_or(1));
            Some(Bullet::Num(attr(c, "type").unwrap_or("arabicPeriod").to_string(), start))
        }
        _ => None,
    })
}

/// The settings of the nine levels.
pub type Levels = [Para; 9];

/// A list of levels (`lstStyle`, `titleStyle`, `defaultTextStyle`…): `defPPr` under each level.
pub fn levels(list: Option<Node>) -> Levels {
    let mut out: Levels = Default::default();
    let Some(list) = list else { return out };
    if let Some(def) = kid(list, "defPPr").map(Para::parse) {
        for level in &mut out {
            level.over(&def);
        }
    }
    for (i, level) in out.iter_mut().enumerate() {
        if let Some(n) = kid(list, &format!("lvl{}pPr", i + 1)) {
            level.over(&Para::parse(n));
        }
    }
    out
}

/// `top`'s levels over `base`'s.
pub fn stack(base: &mut Levels, top: &Levels) {
    for (b, t) in base.iter_mut().zip(top) {
        b.over(t);
    }
}

/// A text body's settings (`bodyPr`), each one set or taken from below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    /// Insets: left, top, right, bottom (points).
    pub ins: [Option<f64>; 4],
    pub anchor: Option<String>,
    pub wrap: Option<bool>,
    pub vert: Option<String>,
    /// Shrunk to fit: the share of its font size and the line spacing taken off.
    pub scale: Option<f64>,
    pub reduce: Option<f64>,
}

impl Body {
    pub fn parse(b: Node) -> Body {
        let fit = kid(b, "normAutofit");
        let share = |name: &str| fit.map(|f| int(f, name).map_or(if name == "fontScale" { 1.0 } else { 0.0 }, |v| v as f64 / 100_000.0));
        Body {
            ins: ["lIns", "tIns", "rIns", "bIns"].map(|side| int(b, side).map(pt)),
            anchor: attr(b, "anchor").map(str::to_string),
            wrap: attr(b, "wrap").map(|w| w != "none"),
            vert: attr(b, "vert").map(str::to_string),
            scale: share("fontScale"),
            reduce: share("lnSpcReduction"),
        }
    }

    pub fn over(&mut self, o: &Body) {
        for (mine, theirs) in self.ins.iter_mut().zip(o.ins) {
            if theirs.is_some() {
                *mine = theirs;
            }
        }
        over!(self, o, anchor, wrap, vert, scale, reduce);
    }
}

/// What a text body is drawn with.
pub struct Text<'t, 'a> {
    pub ctx: &'t Ctx<'a>,
    pub levels: &'t Levels,
    pub body: &'t Body,
    /// The shape style's font and colour, over the levels'.
    pub base: &'t Run,
    /// The slide's number (for a slide number field).
    pub slide: usize,
    /// In a table cell: in the cell's flow rather than over the shape's box.
    pub flow: bool,
}

/// The HTML of the text body `tx`; empty when it has no text.
pub fn html(t: &Text, tx: Node) -> String {
    let has_text = tx.descendants().any(|d| {
        d.is_element()
            && match d.tag_name().name() {
                "t" => d.text().is_some_and(|s| !s.is_empty()),
                "fld" => true,
                _ => false,
            }
    });
    if !has_text {
        return String::new();
    }
    let b = t.body;
    let pad = [b.ins[1].unwrap_or(3.6), b.ins[2].unwrap_or(7.2), b.ins[3].unwrap_or(3.6), b.ins[0].unwrap_or(7.2)];
    let justify = match b.anchor.as_deref() {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    };
    let mut style = format!("padding:{:.2}pt {:.2}pt {:.2}pt {:.2}pt;justify-content:{justify}", pad[0], pad[1], pad[2], pad[3]);
    match b.vert.as_deref() {
        Some("vert" | "eaVert" | "wordArtVert" | "mongolianVert" | "wordArtVertRtl") => style.push_str(";writing-mode:vertical-rl"),
        Some("vert270") => style.push_str(";writing-mode:vertical-rl;transform:rotate(180deg)"),
        _ => {}
    }
    let mut numbers = Numbers::default();
    let paras: String = kids(tx, "p").map(|p| para(t, p, &mut numbers)).collect();
    let class = match (t.flow, b.wrap == Some(false)) {
        (true, _) => "tf",
        (false, true) => "tx nw",
        (false, false) => "tx",
    };
    format!("<div class=\"{class}\" style=\"{}\">{paras}</div>", esc(&style))
}

/// The numbers of numbered paragraphs, by level.
#[derive(Default)]
struct Numbers {
    counts: [u32; 9],
    schemes: [Option<String>; 9],
}

impl Numbers {
    /// The number of a paragraph at `level` with `bullet`; it ends deeper levels' lists.
    fn next(&mut self, level: usize, bullet: Option<&Bullet>) -> Option<u32> {
        for deeper in level + 1..9 {
            if let (Some(c), Some(s)) = (self.counts.get_mut(deeper), self.schemes.get_mut(deeper)) {
                *c = 0;
                *s = None;
            }
        }
        let (count, scheme) = (self.counts.get_mut(level)?, self.schemes.get_mut(level)?);
        match bullet {
            Some(Bullet::Num(kind, start)) => {
                *count = if scheme.as_deref() == Some(kind.as_str()) && *count > 0 { *count + 1 } else { *start };
                *scheme = Some(kind.clone());
                Some(*count)
            }
            _ => {
                *count = 0;
                *scheme = None;
                None
            }
        }
    }
}

fn para(t: &Text, p: Node, numbers: &mut Numbers) -> String {
    let ppr = kid(p, "pPr");
    let level = ppr.and_then(|n| int(n, "lvl")).map_or(0, |v| usize::try_from(v.clamp(0, 8)).unwrap_or(0));
    let mut props = t.levels.get(level).cloned().unwrap_or_default();
    if let Some(n) = ppr {
        props.over(&Para::parse(n));
    }
    let mut base = props.run.clone();
    base.over(t.base);
    let scale = t.body.scale.unwrap_or(1.0);
    let runs: Vec<(Run, String)> = runs(t, p, &base);
    let end = kid(p, "endParaRPr").map(|e| {
        let mut r = base.clone();
        r.over(&Run::parse(e));
        r
    });
    let first = runs.first().map(|r| &r.0).or(end.as_ref()).unwrap_or(&base);
    let size = first.size.unwrap_or(18.0) * scale;
    let number = numbers.next(level, props.bullet.as_ref());
    let mark = if runs.is_empty() { String::new() } else { mark(t, &props, first, size, number) };
    let body: String = if runs.is_empty() { "<br>".into() } else { runs.iter().map(|(r, html)| span(t, r, scale, html)).collect() };
    format!("<p style=\"{}\">{mark}{body}</p>", esc(&para_css(t, &props, size)))
}

/// A paragraph's runs, line breaks and fields: their settings and their HTML.
fn runs(t: &Text, p: Node, base: &Run) -> Vec<(Run, String)> {
    elems(p)
        .filter_map(|c| {
            let name = c.tag_name().name();
            let mut run = base.clone();
            if let Some(r) = kid(c, "rPr") {
                run.over(&Run::parse(r));
            }
            let text = kid(c, "t").and_then(|t| t.text()).unwrap_or_default();
            match name {
                "r" => Some((run, esc(text))),
                "br" => Some((run, "<br>".into())),
                "fld" if attr(c, "type").is_some_and(|k| k.starts_with("slidenum")) => Some((run, t.slide.to_string())),
                "fld" => Some((run, esc(text))),
                _ => None,
            }
        })
        .collect()
}

fn para_css(t: &Text, p: &Para, size: f64) -> String {
    let align = match p.align.as_deref() {
        Some("ctr") => "center",
        Some("r") => "right",
        Some("just" | "dist" | "justLow" | "thaiDist") => "justify",
        _ => "left",
    };
    let reduce = t.body.reduce.unwrap_or(0.0);
    let line = match p.line {
        Some(Space::Pts(v)) => format!("{v:.2}pt"),
        Some(Space::Pct(v)) => format!("{:.3}", ((v - reduce).max(0.1)) * 1.2),
        None => format!("{:.3}", (1.0 - reduce).max(0.1) * 1.2),
    };
    let gap = |s: Option<Space>| match s {
        Some(Space::Pts(v)) => format!("{v:.2}pt"),
        Some(Space::Pct(v)) => format!("{:.2}em", v * 1.2),
        None => "0".into(),
    };
    format!(
        "text-align:{align};margin:{} 0 {} {:.2}pt;text-indent:{:.2}pt;line-height:{line};font-size:{size:.2}pt",
        gap(p.before),
        gap(p.after),
        p.mar_l.unwrap_or(0.0),
        p.indent.unwrap_or(0.0)
    )
}

/// A paragraph's mark: a character or a number, hanging in its indent.
fn mark(t: &Text, p: &Para, first: &Run, size: f64, number: Option<u32>) -> String {
    let text = match (&p.bullet, number) {
        (Some(Bullet::Num(kind, _)), Some(n)) => label(kind, n),
        (Some(Bullet::Char(c)), _) => c.clone(),
        _ => return String::new(),
    };
    let color = match &p.bu_color {
        Some(Some(c)) => Some(c),
        _ => first.color.as_ref(),
    };
    let mut css = format!("font-size:{:.2}pt", size * p.bu_size.unwrap_or(1.0));
    let font = match &p.bu_font {
        Some(Some(face)) => families(t, Some(face.as_str()), None),
        _ => families(t, first.latin.as_deref(), first.ea.as_deref()),
    };
    let _ = write!(css, ";font-family:{font}");
    if let Some(c) = color {
        let _ = write!(css, ";color:{}", t.ctx.rgba(c, None).css());
    }
    let indent = p.indent.unwrap_or(0.0);
    if indent < 0.0 {
        let _ = write!(css, ";display:inline-block;width:{:.2}pt;text-indent:0", -indent);
    } else {
        css.push_str(";margin-right:.4em");
    }
    format!("<span class=\"bu\" style=\"{}\">{}</span>", esc(&css), esc(&text))
}

/// A number in a numbering scheme: `arabicPeriod` 3 is `3.`, `alphaLcParenR` 2 is `b)`, `romanUcPeriod`
/// 4 is `IV.`, `circleNumDbPlain` 5 is `⑤`.
pub fn label(kind: &str, n: u32) -> String {
    let body = if kind.starts_with("alphaLc") {
        alpha(n).to_lowercase()
    } else if kind.starts_with("alphaUc") {
        alpha(n)
    } else if kind.starts_with("romanLc") {
        roman(n).to_lowercase()
    } else if kind.starts_with("romanUc") {
        roman(n)
    } else if kind.starts_with("circleNum") && (1..=20).contains(&n) {
        char::from_u32(0x2460 + n - 1).map_or_else(|| n.to_string(), String::from)
    } else {
        n.to_string()
    };
    if kind.ends_with("ParenBoth") {
        format!("({body})")
    } else if kind.ends_with("ParenR") {
        format!("{body})")
    } else if kind.ends_with("Period") {
        format!("{body}.")
    } else {
        body
    }
}

fn alpha(n: u32) -> String {
    // A … Z, then AA … ZZ, as PowerPoint counts.
    let letter = char::from_u32(u32::from(b'A') + (n.max(1) - 1) % 26).unwrap_or('A');
    letter.to_string().repeat(usize::try_from((n.max(1) - 1) / 26 + 1).unwrap_or(1))
}

fn roman(mut n: u32) -> String {
    const NUMERALS: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

/// A run as a `span`.
fn span(t: &Text, r: &Run, scale: f64, html: &str) -> String {
    let mut css = format!("font-size:{:.2}pt;font-family:{}", r.size.unwrap_or(18.0) * scale, families(t, r.latin.as_deref(), r.ea.as_deref()));
    if r.bold == Some(true) {
        css.push_str(";font-weight:700");
    }
    if r.italic == Some(true) {
        css.push_str(";font-style:italic");
    }
    let lines: Vec<&str> = [(r.underline == Some(true) || r.link, "underline"), (r.strike == Some(true), "line-through")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, l)| *l)
        .collect();
    if !lines.is_empty() {
        let _ = write!(css, ";text-decoration:{}", lines.join(" "));
    }
    let color = r.color.clone().or_else(|| r.link.then(|| Color::scheme("hlink")));
    if let Some(c) = color {
        let _ = write!(css, ";color:{}", t.ctx.rgba(&c, None).css());
    }
    if let Some(h) = &r.highlight {
        let _ = write!(css, ";background:{}", t.ctx.rgba(h, None).css());
    }
    match r.baseline {
        Some(b) if b > 0.0 => css.push_str(";vertical-align:super"),
        Some(b) if b < 0.0 => css.push_str(";vertical-align:sub"),
        _ => {}
    }
    match r.caps.as_deref() {
        Some("all") => css.push_str(";text-transform:uppercase"),
        Some("small") => css.push_str(";font-variant:small-caps"),
        _ => {}
    }
    format!("<span style=\"{}\">{html}</span>", esc(&css))
}

/// A run's typefaces as a CSS list: the Latin one, then the East Asian one (Korean letters fall
/// to it when the Latin face has none), the theme's body fonts when unset.
fn families(t: &Text, latin: Option<&str>, ea: Option<&str>) -> String {
    let latin = t.ctx.face(latin.unwrap_or("+mn-lt"));
    let ea = t.ctx.face(ea.unwrap_or("+mn-ea"));
    let mut list: Vec<String> = Vec::new();
    for face in [latin, ea, "Malgun Gothic".to_string()] {
        if !face.is_empty() && !list.contains(&face) {
            list.push(face);
        }
    }
    let quoted: Vec<String> = list.iter().map(|f| format!("'{}'", f.replace(['\'', '\\'], ""))).collect();
    format!("{},sans-serif", quoted.join(","))
}
