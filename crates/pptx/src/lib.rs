//! Monun's presentation tabs: a PowerPoint file (.pptx, .ppsx, .potx, .pptm) turned into a web page
//! of its slides, to look at. Monun shows the page in a web tab; nothing is edited or saved.
//!
//! Each slide is drawn in HTML and SVG as PowerPoint lays it out: the master's and the layout's
//! shapes behind the slide's own, placeholders taking where they go and how their text looks
//! from the layout and the master, the theme's colours and fonts, backgrounds, text with bullets
//! and numbering, pictures (cropped), preset and drawn shapes, lines with arrowheads, groups,
//! tables in their table styles, and SmartArt (from the drawing PowerPoint keeps of it). A chart
//! shows as a box with its title. The page fits the slides to the window's width.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod color;
mod geom;
mod package;
mod scene;
mod shape;
mod slide;
mod style;
mod table;
mod text;

use std::path::{Path, PathBuf};

/// Words the page shows, in the interface's language.
#[derive(Clone, Debug)]
pub struct Labels {
    /// Over a chart, before its title.
    pub chart: String,
    /// Beside the number of a slide hidden in the slide show.
    pub hidden: String,
}

impl Default for Labels {
    fn default() -> Self {
        Labels { chart: "Chart".into(), hidden: "Hidden".into() }
    }
}

/// A presentation as a web page: the page, the pictures it shows (by their names in `media/`),
/// how many slides it has and their size (points).
#[derive(Debug)]
pub struct Deck {
    pub html: String,
    pub media: Vec<(String, Vec<u8>)>,
    pub slides: usize,
    pub size: (f64, f64),
}

/// The error for a file that isn't a PowerPoint file Monun shows: an old .ppt, or one with a
/// password (both are not zip packages).
pub const NOT_PPTX: &str = "This isn't a PowerPoint file Monun can show. Monun shows .pptx files: an old .ppt file, or one with a password, has to be saved as .pptx in PowerPoint first.";

/// The web page of the presentation `bytes` (a .pptx file), titled `title`.
pub fn convert(bytes: &[u8], title: &str, labels: &Labels) -> Result<Deck, String> {
    slide::deck(bytes, title, labels)
}

/// Writes `deck` into `dir`, emptied first: `index.html` and its pictures in `media/`. Returns
/// the page's path.
pub fn write(deck: &Deck, dir: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);
    let media = dir.join("media");
    std::fs::create_dir_all(&media).map_err(|e| e.to_string())?;
    for (name, bytes) in &deck.media {
        std::fs::write(media.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let page = dir.join("index.html");
    std::fs::write(&page, &deck.html).map_err(|e| e.to_string())?;
    Ok(page)
}

#[cfg(test)]
mod tests;
