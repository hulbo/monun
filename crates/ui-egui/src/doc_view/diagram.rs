//! Mermaid blocks in the rendered document. Each is drawn on a worker thread
//! (`monun_markdown::diagram`) and kept as a texture by its source, scale and theme; a click shows
//! it enlarged. Without the `diagrams` feature (the web build) a Mermaid block shows as code.

use std::collections::HashMap;

use egui::Ui;

use crate::i18n::Language;
use crate::theme::Tokens;

/// A drawing: which source, at what scale (percent of a pixel per SVG unit), in which theme.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Key {
    source: u64,
    scale: u32,
    dark: bool,
}

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
pub fn show(_ui: &mut Ui, _cache: &mut Cache, _source: &str, _z: f32, _t: &Tokens, _lang: Language) -> Option<egui::Response> {
    None
}

/// Draws the diagram of `source` (a Mermaid block's text): the picture once it is drawn, a note
/// while it is being drawn, or the syntax error.
#[cfg(feature = "diagrams")]
pub fn show(ui: &mut Ui, cache: &mut Cache, source: &str, z: f32, t: &Tokens, lang: Language) -> Option<egui::Response> {
    use std::hash::{Hash, Hasher};
    let ctx = ui.ctx().clone();
    cache.receive(&ctx);
    let ppp = ctx.pixels_per_point();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let key = Key { source: h.finish(), scale: (z * ppp * 100.0).round().clamp(10.0, 1600.0) as u32, dark: t.dark };
    if !cache.entries.contains_key(&key) {
        cache.start(&ctx, key, source.to_owned());
    }
    let frame = ctx.cumulative_frame_nr();
    let entry = cache.entries.get_mut(&key)?;
    entry.used = frame;
    let resp = match &entry.state {
        State::Ready(texture) => {
            let size = texture.size_vec2() / ppp;
            let fit = (ui.available_width() / size.x).min(1.0);
            let resp = ui.add(egui::Image::new((texture.id(), size * fit)).alt_text(lang.tr("Diagram")).sense(egui::Sense::click()));
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
            }
            let resp = resp.on_hover_text(lang.tr("Click to enlarge"));
            if resp.clicked() {
                cache.zoomed = Some(key);
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
    /// Starts drawing `source` on a worker thread.
    fn start(&mut self, ctx: &egui::Context, key: Key, source: String) {
        self.forget_unused(ctx.cumulative_frame_nr());
        let (tx, _) = self.channel.get_or_insert_with(std::sync::mpsc::channel);
        let tx = tx.clone();
        let repaint = ctx.clone();
        let options = monun_markdown::diagram::Options { dark: key.dark, scale: key.scale as f32 / 100.0 };
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
    let Some(State::Ready(texture)) = cache.entries.get(&key).map(|e| &e.state) else {
        cache.zoomed = None;
        return;
    };
    let size = texture.size_vec2() / ctx.pixels_per_point();
    let mut open = true;
    let screen = ctx.content_rect();
    egui::Window::new(lang.tr("Diagram"))
        .id(egui::Id::unique("monun.diagram_zoom"))
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_size(size.min(screen.size() * 0.85))
        .show(ctx, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Image::new((texture.id(), size)));
            });
        });
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cache.zoomed = None;
    }
}
