//! Mermaid blocks in the rendered document. Each is drawn on a worker thread
//! (`monun_markdown::diagram`) at the pixel size it shows at, so its lines and labels stay sharp,
//! and kept as a texture by its source, scale and theme; a click shows it enlarged. Without the
//! `diagrams` feature (the web build) a Mermaid block shows as code.

use std::collections::HashMap;

use egui::Ui;

use crate::i18n::Language;
use crate::theme::Tokens;

/// A drawing: which source, at what scale (thousandths of a pixel per SVG unit), in which theme.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Key {
    source: u64,
    scale: u32,
    dark: bool,
}

/// A scale as a [`Key`] holds it, rounded down.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
fn thousandths(scale: f32) -> u32 {
    (scale * 1000.0).floor().clamp(100.0, 16000.0) as u32
}

/// How much larger than its own size a diagram may be drawn to fill the page's width.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
const MAX_GROW: f32 = 2.0;

/// The enlarged view's size, against the diagram's own.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
const ZOOM: f32 = 1.5;

#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
enum State {
    Pending,
    Ready(egui::TextureHandle),
    Failed(String),
}

#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
struct Entry {
    state: State,
    /// The frame it was last shown on.
    used: u64,
}

/// The drawings so far, and the one shown enlarged.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<Key, Entry>,
    /// Each diagram's own size in SVG units, by source and theme, once it has been drawn.
    #[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
    sizes: HashMap<(u64, bool), egui::Vec2>,
    #[cfg(feature = "diagrams")]
    channel: Option<(std::sync::mpsc::Sender<Done>, std::sync::mpsc::Receiver<Done>)>,
    zoomed: Option<Key>,
}

#[cfg(feature = "diagrams")]
type Done = (Key, Result<monun_markdown::diagram::Image, String>);

/// The most drawings kept; past it, those not shown lately go.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
const KEEP: usize = 48;

/// Draws the diagram of `source` (a Mermaid block's text), or `None` when this build draws none.
#[cfg(not(feature = "diagrams"))]
pub fn show(_ui: &mut Ui, _cache: &mut Cache, _source: &str, _z: f32, _t: &Tokens, _lang: Language, _fill: bool) -> Option<egui::Response> {
    None
}

/// Draws the diagram of `source` (a Mermaid block's text): the picture once it is drawn, a note
/// while it is being drawn, or the syntax error. It shows at its own size narrowed to the width,
/// or with `fill` as wide as the width (up to [`MAX_GROW`] times its size, centred).
#[cfg(feature = "diagrams")]
pub fn show(ui: &mut Ui, cache: &mut Cache, source: &str, z: f32, t: &Tokens, lang: Language, fill: bool) -> Option<egui::Response> {
    use std::hash::{Hash, Hasher};
    let ctx = ui.ctx().clone();
    cache.receive(&ctx);
    let ppp = ctx.pixels_per_point();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let (hash, dark) = (h.finish(), t.dark);
    let room = ui.available_width().max(1.0);
    // The width to show it at, once its own size is known, and the scale that draws it at that
    // width in whole pixels.
    let own = cache.sizes.get(&(hash, dark)).copied().filter(|s| s.x > 0.0);
    let width = own.map(|s| if fill { room.min(s.x * z * MAX_GROW) } else { (s.x * z).min(room) });
    let scale = match (own, width) {
        (Some(s), Some(w)) if (w - s.x * z).abs() >= 0.5 => (w * ppp).floor() / s.x,
        _ => z * ppp,
    };
    let key = Key { source: hash, scale: thousandths(scale), dark };
    // One drawing at a time per diagram: while the window is being resized the last one shows,
    // stretched, until the drawing at the new width is in.
    if !cache.entries.contains_key(&key) && !cache.drawing(hash, dark) {
        cache.start(&ctx, key, source.to_owned());
    }
    let frame = ctx.cumulative_frame_nr();
    let ready = |k: &Key, e: &Entry| k.source == hash && k.dark == dark && matches!(e.state, State::Ready(_));
    let shown = match cache.entries.get(&key) {
        Some(e) if !matches!(e.state, State::Pending) => Some(key),
        _ => cache.entries.iter().filter(|(k, e)| ready(k, e)).max_by_key(|(k, _)| k.scale).map(|(k, _)| *k),
    };
    for k in [Some(key), shown].into_iter().flatten() {
        if let Some(e) = cache.entries.get_mut(&k) {
            e.used = frame;
        }
    }
    let state = match shown.and_then(|k| cache.entries.get(&k)).map(|e| &e.state) {
        Some(State::Ready(texture)) => State::Ready(texture.clone()),
        Some(State::Failed(e)) => State::Failed(e.clone()),
        _ => State::Pending,
    };
    let resp = match state {
        State::Ready(texture) => {
            // At its own pixels when it was drawn for this width, else stretched to the width.
            let mut size = texture.size_vec2() / ppp;
            if shown != Some(key)
                && let Some(w) = width
            {
                size *= w / size.x.max(1.0);
            }
            size *= (room / size.x).min(1.0);
            let resp = ui
                .horizontal(|ui| {
                    if fill {
                        ui.add_space(((room - size.x) / 2.0).max(0.0));
                    }
                    ui.add(egui::Image::new((texture.id(), size)).alt_text(lang.tr("Diagram")).sense(egui::Sense::click()))
                })
                .inner;
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
            }
            let resp = resp.on_hover_text(lang.tr("Click to enlarge"));
            if resp.clicked() {
                cache.zoom(&ctx, hash, dark, z, source);
            }
            resp
        }
        State::Failed(e) => egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, t.danger))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(lang.tr("This diagram has an error:")).color(t.danger));
                ui.label(egui::RichText::new(e).font(egui::FontId::new(12.5 * z, egui::FontFamily::Name(crate::theme::MONO.into()))).color(t.text));
            })
            .response
            .interact(egui::Sense::click()),
        State::Pending => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new(lang.tr("Drawing the diagram…")).color(t.text_dim));
            })
            .response
        }
    };
    Some(resp)
}

#[cfg(feature = "diagrams")]
impl Cache {
    /// Whether a drawing of this diagram is under way.
    fn drawing(&self, source: u64, dark: bool) -> bool {
        self.entries.iter().any(|(k, e)| k.source == source && k.dark == dark && matches!(e.state, State::Pending))
    }

    /// Shows the diagram enlarged, drawn for that size.
    fn zoom(&mut self, ctx: &egui::Context, source: u64, dark: bool, z: f32, text: &str) {
        let key = Key { source, scale: thousandths(z * ctx.pixels_per_point() * ZOOM), dark };
        if !self.entries.contains_key(&key) {
            self.start(ctx, key, text.to_owned());
        }
        self.zoomed = Some(key);
    }

    /// Starts drawing `source` on a worker thread.
    fn start(&mut self, ctx: &egui::Context, key: Key, source: String) {
        self.forget_unused(ctx.cumulative_frame_nr());
        let (tx, _) = self.channel.get_or_insert_with(std::sync::mpsc::channel);
        let tx = tx.clone();
        let repaint = ctx.clone();
        let options = monun_markdown::diagram::Options { dark: key.dark, scale: key.scale as f32 / 1000.0 };
        let spawned = std::thread::Builder::new().name("monun-diagram".into()).spawn(move || {
            let drawn = monun_markdown::diagram::render(&source, options);
            let _ = tx.send((key, drawn));
            repaint.request_repaint();
        });
        let state = match spawned {
            Ok(_) => State::Pending,
            Err(e) => State::Failed(e.to_string()),
        };
        self.entries.insert(key, Entry { state, used: ctx.cumulative_frame_nr() });
    }

    /// Takes the drawings the workers finished.
    fn receive(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.channel else { return };
        let done: Vec<Done> = rx.try_iter().collect();
        for (key, drawn) in done {
            let state = match drawn {
                Ok(image) => {
                    self.sizes.insert((key.source, key.dark), egui::vec2(image.size.0, image.size.1));
                    let pixels = egui::ColorImage::from_rgba_premultiplied([image.width as usize, image.height as usize], &image.rgba);
                    State::Ready(ctx.load_texture(format!("monun.diagram.{key:?}"), pixels, egui::TextureOptions::LINEAR))
                }
                Err(e) => State::Failed(e),
            };
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.state = state;
            }
        }
    }

    fn forget_unused(&mut self, frame: u64) {
        if self.entries.len() >= KEEP {
            let zoomed = self.zoomed;
            self.entries.retain(|k, e| matches!(e.state, State::Pending) || Some(*k) == zoomed || frame.saturating_sub(e.used) < 2);
        }
    }
}

/// The enlarged diagram, in a window of its own, after a click on it.
pub fn zoom_window(ctx: &egui::Context, cache: &mut Cache, lang: Language) {
    let Some(key) = cache.zoomed else { return };
    #[cfg(feature = "diagrams")]
    cache.receive(ctx);
    // Being drawn at the enlarged size: a spinner until it is in.
    let texture = match cache.entries.get(&key).map(|e| &e.state) {
        Some(State::Ready(texture)) => Some(texture.clone()),
        Some(State::Pending) => None,
        _ => {
            cache.zoomed = None;
            return;
        }
    };
    let size = texture.as_ref().map_or(egui::vec2(320.0, 200.0), |t| t.size_vec2() / ctx.pixels_per_point());
    let mut open = true;
    let screen = ctx.content_rect();
    egui::Window::new(lang.tr("Diagram"))
        .id(egui::Id::unique("monun.diagram_zoom"))
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_size(size.min(screen.size() * 0.85))
        .show(ctx, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| match &texture {
                Some(texture) => {
                    ui.add(egui::Image::new((texture.id(), size)));
                }
                None => {
                    ui.spinner();
                }
            });
        });
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cache.zoomed = None;
    }
}
