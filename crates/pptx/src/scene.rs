//! What drawing needs: the context of the part a shape comes from (the theme, the slide's colour
//! map, the part's relationships), the page being written with the pictures it shows, where a
//! shape goes, and fills resolved to colours.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::color::{self, Base, Color, Rgba};
use crate::package::{Node, Package, Rels, flag, int, kid, pt};
use crate::style::{Fill, Theme};

/// The context of a part shapes are drawn from.
pub struct Ctx<'a> {
    pub pkg: &'a Package,
    pub theme: &'a Theme,
    /// The slide's colour map: `bg1` → `lt1` and so on.
    pub map: &'a HashMap<String, String>,
    pub rels: Rels,
}

impl<'a> Ctx<'a> {
    /// The same context for another part (a SmartArt drawing).
    pub fn with_rels(&self, rels: Rels) -> Ctx<'a> {
        Ctx { pkg: self.pkg, theme: self.theme, map: self.map, rels }
    }

    /// `c` on this slide, `ph` being the colour of the style using it (for `phClr`).
    pub fn rgba(&self, c: &Color, ph: Option<Rgba>) -> Rgba {
        let base = match &c.base {
            Base::Rgb(rgb) => *rgb,
            Base::Placeholder => ph.unwrap_or(Rgba::BLACK),
            Base::Scheme(name) => {
                let name = self.map.get(name).map_or(name.as_str(), String::as_str);
                self.theme.colors.get(name).copied().unwrap_or(Rgba::BLACK)
            }
        };
        color::apply(base, &c.mods)
    }

    pub fn paint(&self, out: &mut Out, f: &Fill, ph: Option<Rgba>) -> Paint {
        match f {
            Fill::None => Paint::None,
            Fill::Solid(c) => Paint::Solid(self.rgba(c, ph)),
            Fill::Grad(g) => Paint::Grad { stops: g.stops.iter().map(|(at, c)| (*at, self.rgba(c, ph))).collect(), angle: g.angle, radial: g.radial },
            Fill::Image(i) => out.media(self.pkg, &i.part).map_or(Paint::None, |src| Paint::Image { src, crop: i.crop, tile: i.tile }),
        }
    }

    /// A typeface, the theme's named (`+mj-lt` the heading font, `+mn-ea` the body's East Asian
    /// one…); empty when there's none.
    pub fn face(&self, face: &str) -> String {
        let t = self.theme;
        match face {
            "+mj-lt" => t.major.latin.clone(),
            "+mn-lt" => t.minor.latin.clone(),
            "+mj-ea" => t.major.ea.clone(),
            "+mn-ea" => t.minor.ea.clone(),
            f if f.starts_with('+') => String::new(),
            f => f.to_string(),
        }
    }
}

/// A fill resolved for the slide.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    None,
    Solid(Rgba),
    Grad { stops: Vec<(f64, Rgba)>, angle: f64, radial: bool },
    Image { src: String, crop: [f64; 4], tile: bool },
}

impl Paint {
    /// As a CSS background.
    pub fn css(&self) -> Option<String> {
        match self {
            Paint::None => None,
            Paint::Solid(c) => Some(c.css()),
            Paint::Grad { stops, angle, radial } => {
                let list = stops.iter().map(|(at, c)| format!("{} {:.1}%", c.css(), at * 100.0)).collect::<Vec<_>>().join(",");
                Some(if *radial { format!("radial-gradient(circle,{list})") } else { format!("linear-gradient({:.1}deg,{list})", angle + 90.0) })
            }
            Paint::Image { src, tile: true, .. } => Some(format!("url('{src}')")),
            Paint::Image { src, .. } => Some(format!("url('{src}') center/100% 100% no-repeat")),
        }
    }

    /// Its colour, or its first colour (a line drawn in a gradient).
    pub fn first(&self) -> Option<Rgba> {
        match self {
            Paint::Solid(c) => Some(*c),
            Paint::Grad { stops, .. } => stops.first().map(|s| s.1),
            _ => None,
        }
    }
}

/// Picture kinds a web page shows (not EMF or WMF, nor TIFF).
const SHOWN: [&str; 9] = ["png", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "svg", "ico"];

/// The page being written, and the pictures it shows.
#[derive(Default)]
pub struct Out {
    pub html: String,
    media: HashMap<String, Option<String>>,
    pub files: Vec<(String, Vec<u8>)>,
    next: usize,
}

impl Out {
    /// A number for an id in the page (a gradient, an arrowhead).
    pub fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    /// The address in the page of the picture `part` (`media/3.png`), kept once however often it
    /// shows; `None` for a picture a web page doesn't show.
    pub fn media(&mut self, pkg: &Package, part: &str) -> Option<String> {
        if let Some(known) = self.media.get(part) {
            return known.clone();
        }
        let ext = part.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let src = pkg.bytes(part).filter(|_| SHOWN.contains(&ext.as_str())).map(|bytes| {
            let name = format!("{}.{ext}", self.files.len() + 1);
            self.files.push((name.clone(), bytes.to_vec()));
            format!("media/{name}")
        });
        self.media.insert(part.to_string(), src.clone());
        src
    }
}

/// Where a shape goes (points), turned (degrees, clockwise) and flipped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Frame {
    /// Where it goes, as CSS for an absolutely placed box.
    pub fn css(&self) -> String {
        let mut s = format!("left:{:.2}pt;top:{:.2}pt;width:{:.2}pt;height:{:.2}pt", self.x, self.y, self.w.max(0.0), self.h.max(0.0));
        if self.rot.abs() > 0.01 {
            let _ = write!(s, ";transform:rotate({:.2}deg)", self.rot);
        }
        s
    }
}

/// An `xfrm` element.
pub fn xfrm(x: Node) -> Option<Frame> {
    let ext = kid(x, "ext")?;
    let off = kid(x, "off");
    let at = |name: &str| off.and_then(|o| int(o, name)).map_or(0.0, pt);
    Some(Frame {
        x: at("x"),
        y: at("y"),
        w: int(ext, "cx").map_or(0.0, pt),
        h: int(ext, "cy").map_or(0.0, pt),
        rot: int(x, "rot").map_or(0.0, |r| r as f64 / 60_000.0),
        flip_h: flag(x, "flipH") == Some(true),
        flip_v: flag(x, "flipV") == Some(true),
    })
}

/// A group's coordinates for its shapes: a shape at `x` goes at `(x - ox) * sx` in the group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub ox: f64,
    pub oy: f64,
    pub sx: f64,
    pub sy: f64,
}

impl Space {
    pub const SLIDE: Space = Space { ox: 0.0, oy: 0.0, sx: 1.0, sy: 1.0 };

    pub fn map(&self, f: Frame) -> Frame {
        Frame { x: (f.x - self.ox) * self.sx, y: (f.y - self.oy) * self.sy, w: f.w * self.sx, h: f.h * self.sy, ..f }
    }
}
