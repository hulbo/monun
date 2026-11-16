//! Colours: DrawingML's colour elements, the changes made to them (lighter, darker, see-through…)
//! and the theme colours they name, resolved later for the slide they're drawn on.

use crate::package::{Node, attr, elems, int};

/// A colour, its channels from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// As CSS: `#rrggbb`, or `rgba(…)` when see-through.
    pub fn css(self) -> String {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 0.999 {
            format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
        } else {
            format!("rgba({},{},{},{:.3})", c(self.r), c(self.g), c(self.b), self.a.clamp(0.0, 1.0))
        }
    }

    fn map(self, f: impl Fn(f64) -> f64) -> Rgba {
        Rgba { r: f(self.r).clamp(0.0, 1.0), g: f(self.g).clamp(0.0, 1.0), b: f(self.b).clamp(0.0, 1.0), a: self.a }
    }

    /// Hue (0 to 1), saturation and lightness.
    fn hsl(self) -> (f64, f64, f64) {
        let (max, min) = (self.r.max(self.g).max(self.b), self.r.min(self.g).min(self.b));
        let l = (max + min) / 2.0;
        let d = max - min;
        if d < 1e-9 {
            return (0.0, 0.0, l);
        }
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if (max - self.r).abs() < 1e-9 {
            (self.g - self.b) / d + if self.g < self.b { 6.0 } else { 0.0 }
        } else if (max - self.g).abs() < 1e-9 {
            (self.b - self.r) / d + 2.0
        } else {
            (self.r - self.g) / d + 4.0
        };
        (h / 6.0, s, l)
    }

    fn from_hsl(h: f64, s: f64, l: f64, a: f64) -> Rgba {
        if s <= 0.0 {
            return Rgba { r: l, g: l, b: l, a };
        }
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let channel = |t: f64| {
            let t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Rgba { r: channel(h + 1.0 / 3.0), g: channel(h), b: channel(h - 1.0 / 3.0), a }
    }
}

/// Where a colour comes from: given, a theme colour (by its name in the slide's colour map), or
/// the colour of the style that uses it (`phClr`).
#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    Rgb(Rgba),
    Scheme(String),
    Placeholder,
}

/// A colour as written: where it comes from and the changes made to it, in order (`lumMod`…, as
/// fractions).
#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    pub base: Base,
    pub mods: Vec<(String, f64)>,
}

impl Color {
    pub fn scheme(name: &str) -> Color {
        Color { base: Base::Scheme(name.to_string()), mods: Vec::new() }
    }

    pub fn with(mut self, change: &str, value: f64) -> Color {
        self.mods.push((change.to_string(), value));
        self
    }
}

const TAGS: [&str; 6] = ["srgbClr", "schemeClr", "sysClr", "prstClr", "scrgbClr", "hslClr"];

/// The colour element among `n`'s children (a fill, a gradient stop, a style reference…).
pub fn within(n: Node) -> Option<Color> {
    elems(n).find(|c| TAGS.contains(&c.tag_name().name())).and_then(parse)
}

/// A colour element.
pub fn parse(n: Node) -> Option<Color> {
    let pct = |name: &str| int(n, name).map_or(0.0, |v| v as f64 / 100_000.0);
    let base = match n.tag_name().name() {
        "srgbClr" => Base::Rgb(hex(attr(n, "val")?)?),
        "sysClr" => {
            let fallback = if attr(n, "val") == Some("window") { Rgba::WHITE } else { Rgba::BLACK };
            Base::Rgb(attr(n, "lastClr").and_then(hex).unwrap_or(fallback))
        }
        "schemeClr" => match attr(n, "val")? {
            "phClr" => Base::Placeholder,
            name => Base::Scheme(name.to_string()),
        },
        "prstClr" => Base::Rgb(preset(attr(n, "val")?)),
        "scrgbClr" => Base::Rgb(Rgba { r: gamma(pct("r")), g: gamma(pct("g")), b: gamma(pct("b")), a: 1.0 }),
        "hslClr" => Base::Rgb(Rgba::from_hsl(int(n, "hue").map_or(0.0, |v| v as f64 / 21_600_000.0), pct("sat"), pct("lum"), 1.0)),
        _ => return None,
    };
    let mods = elems(n).filter_map(|m| Some((m.tag_name().name().to_string(), attr(m, "val")?.trim().parse::<f64>().ok()? / 100_000.0))).collect();
    Some(Color { base, mods })
}

/// `rrggbb`.
pub fn hex(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let c = |shift: u32| f64::from((v >> shift) & 255) / 255.0;
    Some(Rgba { r: c(16), g: c(8), b: c(0), a: 1.0 })
}

/// Linear light to sRGB.
fn gamma(v: f64) -> f64 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The named colours (`prstClr`) slides use.
fn preset(name: &str) -> Rgba {
    let hexes: &[(&str, &str)] = &[
        ("black", "000000"),
        ("white", "FFFFFF"),
        ("red", "FF0000"),
        ("green", "008000"),
        ("lime", "00FF00"),
        ("blue", "0000FF"),
        ("yellow", "FFFF00"),
        ("cyan", "00FFFF"),
        ("magenta", "FF00FF"),
        ("gray", "808080"),
        ("grey", "808080"),
        ("dkGray", "A9A9A9"),
        ("ltGray", "D3D3D3"),
        ("silver", "C0C0C0"),
        ("orange", "FFA500"),
        ("purple", "800080"),
        ("navy", "000080"),
        ("maroon", "800000"),
        ("olive", "808000"),
        ("teal", "008080"),
    ];
    hexes.iter().find(|(n, _)| *n == name).and_then(|(_, h)| hex(h)).unwrap_or(Rgba::BLACK)
}

/// `c` with the changes `mods` made to it, in order.
pub fn apply(c: Rgba, mods: &[(String, f64)]) -> Rgba {
    mods.iter().fold(c, |c, (change, v)| changed(c, change, *v))
}

fn changed(c: Rgba, change: &str, v: f64) -> Rgba {
    match change {
        "alpha" => Rgba { a: v.clamp(0.0, 1.0), ..c },
        "alphaMod" => Rgba { a: (c.a * v).clamp(0.0, 1.0), ..c },
        "alphaOff" => Rgba { a: (c.a + v).clamp(0.0, 1.0), ..c },
        "tint" => c.map(|x| x * v + (1.0 - v)),
        "shade" => c.map(|x| x * v),
        "inv" => c.map(|x| 1.0 - x),
        "gray" => {
            let y = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
            Rgba { r: y, g: y, b: y, a: c.a }
        }
        "lumMod" | "lumOff" | "satMod" | "satOff" => {
            let (h, s, l) = c.hsl();
            let (s, l) = match change {
                "lumMod" => (s, l * v),
                "lumOff" => (s, l + v),
                "satMod" => (s * v, l),
                _ => (s + v, l),
            };
            Rgba::from_hsl(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0), c.a)
        }
        _ => c,
    }
}
