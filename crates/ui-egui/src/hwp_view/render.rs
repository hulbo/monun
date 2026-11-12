//! Pages drawn off the UI thread from the SVG rhwp makes of them (resvg), and the system's fonts
//! they are drawn with.
//!
//! One worker draws, given every frame the pages wanted, most needed first; a page no longer
//! wanted is never started. It makes each page a texture itself, so the UI thread only shows it.
//! A page's fingerprint is its SVG's: an edit that leaves a page's SVG as it was leaves its
//! picture too.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, mpsc};

use egui::{ColorImage, TextureHandle, TextureOptions};
use resvg::{tiny_skia, usvg};

/// The longest side a page is drawn with, below the GPU's own limit.
const MAX_SIDE: f32 = 8192.0;
/// The most pixels one page is drawn with; a page zoomed past it is drawn at this size and stretched.
const MAX_PIXELS: f32 = 40.0e6;
/// Texture memory the open 한글 documents keep between them (bytes).
pub(super) const BUDGET: usize = 384 << 20;
/// rhwp measures a page in CSS pixels (96 an inch); the tab in points (72 an inch).
pub(super) const PT_PER_PX: f32 = 0.75;

/// A page to draw: of which tab, which page, its fingerprint, the scale (device pixels per point)
/// and its SVG.
pub(super) struct Job {
    pub uid: u64,
    pub page: usize,
    pub key: u64,
    pub scale: f32,
    pub svg: Arc<String>,
}

impl Job {
    /// What tells two jobs apart.
    pub fn id(&self) -> (u64, usize, u64, u32) {
        (self.uid, self.page, self.key, self.scale.to_bits())
    }
}

/// A page drawn.
pub(super) struct Done {
    pub uid: u64,
    pub page: usize,
    pub key: u64,
    pub scale: f32,
    pub size: [usize; 2],
    pub texture: TextureHandle,
    pub ms: f64,
}

struct Shared {
    queue: Mutex<Vec<Job>>,
    wake: Condvar,
    stop: AtomicBool,
}

/// The worker and what it has drawn.
pub(super) struct Renderer {
    shared: Arc<Shared>,
    done: mpsc::Receiver<Done>,
}

impl Renderer {
    /// Starts the worker; `None` if no thread could be made.
    pub fn start(ctx: egui::Context) -> Option<Renderer> {
        let shared = Arc::new(Shared { queue: Mutex::new(Vec::new()), wake: Condvar::new(), stop: AtomicBool::new(false) });
        let (tx, done) = mpsc::channel();
        let worker = shared.clone();
        std::thread::Builder::new().name("monun-hwp-render".into()).spawn(move || work(&worker, &tx, &ctx)).ok()?;
        Some(Renderer { shared, done })
    }

    /// What to draw next, most needed first; replaces what was waiting.
    pub fn set_queue(&self, jobs: Vec<Job>) {
        *self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner) = jobs;
        self.shared.wake.notify_one();
    }

    pub fn try_recv(&self) -> Option<Done> {
        self.done.try_recv().ok()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
}

/// The installed fonts (the system's and the user's own), and the font files pages use read into
/// memory.
///
/// fontdb opens and maps a font's file again each time a glyph run asks for it: a page of a few
/// thousand runs then spends about 0.3 s on it (most of drawing it, and of printing it), and
/// pages drawn at once wait for each other in the system. The files of the families pages name
/// (rhwp names a few, with long lists of stand-ins: the first one installed is the one used) are
/// read once instead, and every page after uses them from memory, more than twenty times faster.
struct FontSet {
    db: Arc<usvg::fontdb::Database>,
    /// The files read into memory.
    held: std::collections::HashSet<std::path::PathBuf>,
}

fn font_set() -> &'static Mutex<FontSet> {
    static FONTS: OnceLock<Mutex<FontSet>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        // What a family nobody has falls back to, as Korean Windows has them.
        db.set_serif_family("Batang");
        db.set_sans_serif_family("Malgun Gothic");
        db.set_monospace_family("GulimChe");
        Mutex::new(FontSet { db: Arc::new(db), held: std::collections::HashSet::new() })
    })
}

/// The installed fonts, as they are now.
pub(super) fn fonts() -> Arc<usvg::fontdb::Database> {
    font_set().lock().unwrap_or_else(PoisonError::into_inner).db.clone()
}

/// The installed fonts, the files of the families `svg` uses read into memory first.
pub(super) fn fonts_for(svg: &str) -> Arc<usvg::fontdb::Database> {
    let mut guard = font_set().lock().unwrap_or_else(PoisonError::into_inner);
    let FontSet { db, held } = &mut *guard;
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for list in families(svg) {
        let Some(name) = first_installed(db, &list) else { continue };
        for face in db.faces().filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name))) {
            if let usvg::fontdb::Source::File(path) = &face.source
                && !held.contains(path)
            {
                files.push(path.clone());
            }
        }
    }
    files.sort();
    files.dedup();
    if !files.is_empty() {
        let db = Arc::make_mut(db);
        for path in files {
            hold(db, &path);
            held.insert(path);
        }
    }
    db.clone()
}

/// Reads the font file at `path` into memory, in place of its faces read from the file.
fn hold(db: &mut usvg::fontdb::Database, path: &std::path::Path) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let faces: Vec<usvg::fontdb::ID> = db.faces().filter(|f| matches!(&f.source, usvg::fontdb::Source::File(p) if p == path)).map(|f| f.id).collect();
    for id in faces {
        db.remove_face(id);
    }
    db.load_font_source(usvg::fontdb::Source::SharedFile(path.to_path_buf(), Arc::new(bytes)));
}

/// The font family lists of an SVG's text, each once: `font-family="'A','B',serif"`.
fn families(svg: &str) -> Vec<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut lists = Vec::new();
    for part in svg.split("font-family=\"").skip(1) {
        let Some(value) = part.split('"').next() else { continue };
        if !seen.insert(value) {
            continue;
        }
        let value = value.replace("&apos;", "'").replace("&quot;", "\"").replace("&amp;", "&");
        lists.push(value.split(',').map(|f| f.trim().trim_matches(['\'', '"']).to_owned()).filter(|f| !f.is_empty()).collect());
    }
    lists
}

/// The first family of `list` that is installed, as fontdb names it (`serif` and the other
/// generic names as the families they stand for).
fn first_installed(db: &usvg::fontdb::Database, list: &[String]) -> Option<String> {
    use usvg::fontdb::Family;
    list.iter().find_map(|name| {
        let family = match name.to_ascii_lowercase().as_str() {
            "serif" => db.family_name(&Family::Serif).to_owned(),
            "sans-serif" => db.family_name(&Family::SansSerif).to_owned(),
            "monospace" => db.family_name(&Family::Monospace).to_owned(),
            _ => name.clone(),
        };
        db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(&family))).then_some(family)
    })
}

/// Starts listing the fonts off the UI thread (once).
pub(super) fn warm() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("monun-hwp-fonts".into()).spawn(|| {
            fonts();
        });
    });
}

/// The worker's loop: the first job waiting, drawn and made a texture, until the renderer goes.
fn work(shared: &Shared, tx: &mpsc::Sender<Done>, ctx: &egui::Context) {
    let mut options = usvg::Options::default();
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if !queue.is_empty() {
                    break queue.remove(0);
                }
                queue = shared.wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let started = crate::clock::monotonic_ms();
        options.fontdb = fonts_for(&job.svg);
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| draw(&job.svg, job.scale, &options)));
        let Ok(Some((size, pixels))) = drawn else {
            log::warn!("한글 page {} could not be drawn", job.page + 1);
            continue;
        };
        let image = ColorImage::from_rgba_premultiplied(size, &pixels);
        let texture = ctx.load_texture(format!("monun.hwp.{}.{}", job.uid, job.page), image, TextureOptions::LINEAR);
        let done = Done { uid: job.uid, page: job.page, key: job.key, scale: job.scale, size, texture, ms: crate::clock::monotonic_ms() - started };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// A page's SVG drawn at `scale` device pixels per point, on white: its size and premultiplied
/// RGBA pixels. `None` if the SVG can't be read or the page has no area.
pub(super) fn draw(svg: &str, scale: f32, options: &usvg::Options) -> Option<([usize; 2], Vec<u8>)> {
    let tree = usvg::Tree::from_str(svg, options).ok()?;
    let s = scale * PT_PER_PX;
    let (w, h) = ((tree.size().width() * s).ceil(), (tree.size().height() * s).ceil());
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(w as u32, h as u32)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    Some((size, pixmap.take()))
}

/// The scale (device pixels per point) to draw a page of `w` × `h` points at for `scale`: the same,
/// or less for a page larger than a texture can be (`max_side`) or than [`MAX_PIXELS`].
pub(super) fn raster_scale(w: f32, h: f32, scale: f32, max_side: f32) -> f32 {
    let (w, h) = (w.max(1.0) * scale, h.max(1.0) * scale);
    let side = max_side.min(MAX_SIDE);
    let fit = (side / w).min(side / h).min((MAX_PIXELS / (w * h)).sqrt());
    if fit < 1.0 { scale * fit } else { scale }
}

/// Whether two scales are the same for drawing.
pub(super) fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= b.abs() * 1e-4
}

/// A page's fingerprint: its SVG's.
pub(super) fn svg_key(svg: &str) -> u64 {
    let mut h = DefaultHasher::new();
    svg.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_drawn_no_larger_than_a_texture_can_be() {
        assert_eq!(raster_scale(595.0, 842.0, 2.0, 16384.0), 2.0);
        let s = raster_scale(595.0, 842.0, 20.0, 4096.0);
        assert!((842.0 * s - 4096.0).abs() < 0.5);
        assert!(same(1.0, 1.00001) && !same(1.0, 1.01));
    }

    #[test]
    fn an_svg_page_is_drawn_white_at_its_scale() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="48"><rect x="0" y="0" width="48" height="48" fill="black"/></svg>"#;
        let options = usvg::Options::default();
        // 96 × 48 px is 72 × 36 pt; at 2 device pixels a point, 144 × 72.
        let (size, pixels) = draw(svg, 2.0, &options).unwrap();
        assert_eq!(size, [144, 72]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 255], "the black half");
        assert_eq!(&pixels[pixels.len() - 4..], &[255, 255, 255, 255], "white paper");
        assert_eq!(svg_key(svg), svg_key(svg));
        assert!(draw("not svg", 1.0, &options).is_none());
    }

    #[test]
    fn the_families_a_page_names_are_read_from_its_svg() {
        let svg = "<svg><text font-family=\"&apos;HCR Batang&apos;,&apos;Batang&apos;,serif\">a</text><text font-family=\"&apos;HCR Batang&apos;,&apos;Batang&apos;,serif\">b</text></svg>";
        assert_eq!(families(svg), vec![vec!["HCR Batang".to_owned(), "Batang".to_owned(), "serif".to_owned()]]);
        let mut db = usvg::fontdb::Database::new();
        db.set_serif_family("Nothing Installed");
        assert_eq!(first_installed(&db, &["Nowhere".to_owned()]), None);
    }
}
