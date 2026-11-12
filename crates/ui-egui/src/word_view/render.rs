//! Pages drawn off the UI thread, and the fingerprints that tell which pages an edit changed.
//!
//! One worker draws (WordCraft's rasteriser already spreads a page over several cores), given
//! every frame the pages wanted, most needed first; a page no longer wanted is never started. It
//! makes each page a texture itself, so the UI thread only shows it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};

use egui::{ColorImage, TextureHandle, TextureOptions};
use wordcraft_engine::doc::Document;
use wordcraft_engine::layout::{DocLayout, Page, Placed};
use wordcraft_engine::render::{RenderOptions, render_page};

/// The longest side a page is drawn with, below the GPU's own limit.
const MAX_SIDE: f32 = 8192.0;
/// The most pixels one page is drawn with; a page zoomed past it is drawn at this size and stretched.
const MAX_PIXELS: f32 = 40.0e6;
/// Texture memory the open Word documents keep between them (bytes).
pub(super) const BUDGET: usize = 384 << 20;

/// A page to draw: of which tab, which page of which layout of which document, its fingerprint
/// and the scale (device pixels per point).
pub(super) struct Job {
    pub uid: u64,
    pub page: usize,
    pub key: u64,
    pub scale: f32,
    pub doc: Arc<Document>,
    pub layout: Arc<DocLayout>,
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
    /// The layout it was drawn from, kept with the texture: while it lives, the paragraph layouts
    /// its fingerprint points at can't be freed and their addresses taken by others.
    pub layout: Arc<DocLayout>,
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
        std::thread::Builder::new().name("monun-word-render".into()).spawn(move || work(&worker, &tx, &ctx)).ok()?;
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

/// The worker's loop: the first job waiting, drawn and made a texture, until the renderer goes.
fn work(shared: &Shared, tx: &mpsc::Sender<Done>, ctx: &egui::Context) {
    let options = RenderOptions::default();
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
        let Some(page) = job.layout.pages.get(job.page) else { continue };
        let started = crate::clock::monotonic_ms();
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_page(&job.doc, page, job.scale, &options)));
        let Ok(drawn) = drawn else {
            log::warn!("Word page {} could not be drawn", job.page + 1);
            continue;
        };
        let size = [drawn.width as usize, drawn.height as usize];
        if size[0] == 0 || size[1] == 0 || drawn.pixels.len() != size[0] * size[1] * 4 {
            continue;
        }
        let image = ColorImage::from_rgba_premultiplied(size, &drawn.pixels);
        let texture = ctx.load_texture(format!("monun.word.{}.{}", job.uid, job.page), image, TextureOptions::LINEAR);
        let done = Done {
            uid: job.uid,
            page: job.page,
            key: job.key,
            scale: job.scale,
            size,
            texture,
            layout: job.layout,
            ms: crate::clock::monotonic_ms() - started,
        };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
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

/// Fingerprint of what a laid-out page shows (WordCraft's own, `page_content_key`): a layout never
/// changes once made, and an edit lays out again only what it touched, so the paragraph layouts
/// of an untouched page are the same ones and its fingerprint the same.
pub(super) fn page_key(page: &Page) -> u64 {
    let mut h = DefaultHasher::new();
    (page.w.to_bits(), page.h.to_bits()).hash(&mut h);
    for list in [&page.items, &page.header, &page.footer] {
        list.len().hash(&mut h);
        for it in list.iter() {
            match it {
                Placed::Lines { para, l0, l1, x, y, turn, .. } => {
                    (Arc::as_ptr(para) as usize, l0, l1, x.to_bits(), y.to_bits(), turn).hash(&mut h);
                }
                Placed::Fill { rect, color } => format!("{rect:?}{color:?}").hash(&mut h),
                Placed::Rule { x0, y0, x1, y1, border } => format!("{x0}{y0}{x1}{y1}{border:?}").hash(&mut h),
                Placed::Image { rect, media, crop, spin, fx, .. } => format!("{rect:?}{media}{crop:?}{spin:?}{fx:?}").hash(&mut h),
                Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin, extra, .. } => {
                    format!("{rect:?}{kind:?}{fill:?}{stroke:?}{stroke_width}{effects:?}{spin:?}{extra:?}").hash(&mut h);
                    freeform.as_ref().map(|f| Arc::as_ptr(f) as usize).hash(&mut h);
                }
                Placed::Graphic { rect, graphic, spin, .. } => {
                    (Arc::as_ptr(graphic) as usize, rect.x.to_bits(), rect.y.to_bits(), rect.w.to_bits(), rect.h.to_bits()).hash(&mut h);
                    format!("{spin:?}").hash(&mut h);
                }
                Placed::Clip { rect, spin, items } => format!("{rect:?}{spin:?}{items:?}").hash(&mut h),
                Placed::Cell { .. } | Placed::Object { .. } => {}
            }
        }
    }
    h.finish()
}

/// The fingerprints of one layout's pages, taken when a page is first needed: hashing a page of
/// thousands of table borders every frame would cost.
#[derive(Default)]
pub(super) struct PageKeys {
    layout: Option<Arc<DocLayout>>,
    keys: Vec<Option<u64>>,
}

impl PageKeys {
    pub fn get(&mut self, layout: &Arc<DocLayout>, page: usize) -> u64 {
        if !self.layout.as_ref().is_some_and(|l| Arc::ptr_eq(l, layout)) {
            self.layout = Some(layout.clone());
            self.keys = vec![None; layout.pages.len()];
        }
        if let Some(Some(k)) = self.keys.get(page) {
            return *k;
        }
        let k = layout.pages.get(page).map_or(0, page_key);
        if let Some(slot) = self.keys.get_mut(page) {
            *slot = Some(k);
        }
        k
    }
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
}
