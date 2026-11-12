//! Page rasters: what to ask PdfCraft's render pool for, in what order, and the answers made into
//! textures.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use pdfcraft_render::{RenderRequest, RenderedPage, RequestKind};

/// The most pixels one page is drawn with (~160 MB of RGBA); a page zoomed past it is drawn at
/// this size and stretched.
const MAX_PIXELS: f32 = 40.0e6;
/// The longest side a page is drawn with, below the GPU's own limit.
const MAX_SIDE: f32 = 8192.0;
/// Texture memory the open PDFs keep between them (bytes).
pub(super) const BUDGET: usize = 384 << 20;
/// Answers taken in one frame, and the most pixels made into textures in one (bytes), so a burst
/// of them never stalls a frame.
pub(super) const PER_FRAME: usize = 8;
pub(super) const BYTES_PER_FRAME: usize = 48 << 20;

/// A page drawn: at what scale (device pixels per PDF point), its size in pixels, its texture.
pub(super) struct Raster {
    pub scale: f32,
    pub size: [usize; 2],
    pub texture: TextureHandle,
}

impl Raster {
    pub fn bytes(&self) -> usize {
        self.size[0].saturating_mul(self.size[1]).saturating_mul(4)
    }
}

/// The scale (device pixels per PDF point) to draw a page of `size` points at for `scale`: the
/// same, or less for a page that would be larger than a texture can be (`max_side`) or than
/// [`MAX_PIXELS`].
pub(super) fn raster_scale(size: [f32; 2], scale: f32, max_side: f32) -> f32 {
    let (w, h) = (size[0].max(1.0) * scale, size[1].max(1.0) * scale);
    let side = max_side.min(MAX_SIDE);
    let fit = (side / w).min(side / h).min((MAX_PIXELS / (w * h)).sqrt());
    if fit < 1.0 { scale * fit } else { scale }
}

/// About how many bytes page `size` (points) takes drawn at `scale` (device pixels per point).
pub(super) fn raster_bytes(size: [f32; 2], scale: f32) -> usize {
    let side = |points: f32| (points.max(1.0) * scale).ceil() as usize;
    side(size[0]).saturating_mul(side(size[1])).saturating_mul(4)
}

/// Whether two scales are the same for drawing.
pub(super) fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= b.abs() * 1e-4
}

/// A request for page `page`'s pixels at `scale`.
pub(super) fn pixels(page: usize, scale: f32) -> RenderRequest {
    RenderRequest { page, kind: RequestKind::Pixels, scale, ..RenderRequest::default() }
}

/// A request for page `page`'s text layer.
pub(super) fn text(page: usize) -> RenderRequest {
    RenderRequest { page, kind: RequestKind::Text, scale: 1.0, ..RenderRequest::default() }
}

/// An answer's pixels as a texture named `name`; `None` for one without them.
pub(super) fn texture(ctx: &egui::Context, name: String, page: RenderedPage) -> Option<Raster> {
    let size = [usize::try_from(page.width).ok()?, usize::try_from(page.height).ok()?];
    let words = page.rgba.into_words();
    if words.is_empty() || words.len() != size[0].saturating_mul(size[1]) {
        return None;
    }
    // Premultiplied RGBA bytes in memory order, as `Color32` holds them.
    let pixels = words
        .into_iter()
        .map(|w| {
            let [r, g, b, a] = w.to_ne_bytes();
            Color32::from_rgba_premultiplied(r, g, b, a)
        })
        .collect();
    let texture = ctx.load_texture(name, ColorImage::new(size, pixels), TextureOptions::LINEAR);
    Some(Raster { scale: page.request.scale, size, texture })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_drawn_no_larger_than_a_texture_can_be() {
        assert_eq!(raster_scale([595.0, 842.0], 2.0, 16384.0), 2.0);
        let s = raster_scale([595.0, 842.0], 20.0, 4096.0);
        assert!((842.0 * s - 4096.0).abs() < 0.5, "the longer side fits the GPU's limit");
        let s = raster_scale([2000.0, 2000.0], 10.0, 16384.0);
        assert!(2000.0 * s * 2000.0 * s <= MAX_PIXELS * 1.0001);
        assert!(same(1.0, 1.00001) && !same(1.0, 1.01));
    }
}
