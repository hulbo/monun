//! Mermaid blocks in the rendered document. Each is drawn on a worker thread
//! (`monun_markdown::diagram`) at the pixel size it shows at, so its lines and labels stay sharp,
//! and kept as a texture by its source, scale and theme; a click opens it in a window of its own
//! ([`zoom_window`]). Without the `diagrams` feature (the web build) a Mermaid block shows as code.

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
    zoomed: Option<Zoomed>,
}

/// The diagram shown in a window of its own.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
struct Zoomed {
    source: u64,
    dark: bool,
    text: String,
    /// Points per SVG unit as zoomed; `None` while it fits the window.
    zoom: Option<f32>,
    /// The window's first size and place (points), worked out when it opens.
    size: egui::Vec2,
    place: Option<egui::Pos2>,
    /// A scroll position to take next frame (a zoom keeps the point under the pointer; a drag).
    scroll: Option<egui::Vec2>,
    /// Where the diagram showed last frame and how far it was scrolled.
    last: Option<(egui::Rect, egui::Vec2)>,
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
            let resp = resp.on_hover_text(lang.tr("Click to open in its own window"));
            if resp.clicked() {
                cache.open(&ctx, hash, dark, source);
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

    /// Opens the diagram's window (or shows another diagram in it): sized for the diagram at its
    /// own size, within most of the screen, over the middle of the main window.
    fn open(&mut self, ctx: &egui::Context, source: u64, dark: bool, text: &str) {
        let own = self.sizes.get(&(source, dark)).copied().unwrap_or(egui::vec2(640.0, 400.0));
        let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
        let screen = monitor.unwrap_or_else(|| ctx.content_rect().size());
        let least = egui::vec2(560.0, 380.0);
        // The diagram, its margins and the tool bar.
        let size = (own + egui::vec2(48.0, 90.0)).max(least).min((screen * 0.85).max(least));
        let place = outer.map(|r| (r.center() - size / 2.0).max(egui::pos2(0.0, 0.0)));
        self.zoomed = Some(Zoomed { source, dark, text: text.to_owned(), zoom: None, size, place, scroll: None, last: None });
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
            self.entries.retain(|_, e| matches!(e.state, State::Pending) || frame.saturating_sub(e.used) < 2);
        }
    }
}

/// The diagram's own window, after a click on it: a window of the system's (moved, resized and
/// maximized as any other), the whole diagram fitted in it at first. Ctrl+wheel, the − and +
/// buttons and Ctrl+−, Ctrl++ zoom; a drag moves it about; a double-click or Ctrl+0 fits it again.
/// Esc or the window's close button closes it.
#[cfg(not(feature = "diagrams"))]
pub fn zoom_window(_ctx: &egui::Context, _cache: &mut Cache, _lang: Language, _icon: Option<std::sync::Arc<egui::IconData>>) {}

/// See the other `zoom_window`.
#[cfg(feature = "diagrams")]
pub fn zoom_window(ctx: &egui::Context, cache: &mut Cache, lang: Language, icon: Option<std::sync::Arc<egui::IconData>>) {
    let Some(z) = cache.zoomed.as_ref() else { return };
    let mut builder = egui::ViewportBuilder::default()
        .with_title(lang.tr("Diagram"))
        .with_inner_size(z.size)
        .with_min_inner_size([360.0, 260.0])
        .with_resizable(true);
    if let Some(place) = z.place {
        builder = builder.with_position(place);
    }
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }
    let id = egui::ViewportId::from_hash_of("monun.diagram_window");
    let close = ctx.show_viewport_immediate(id, builder, |ui, _class| window_ui(ui, cache, lang));
    if close {
        cache.zoomed = None;
    }
}

/// The diagram window's zoom: how small and large it goes (points per SVG unit), and the most a
/// small diagram is enlarged to fit the window.
#[cfg(feature = "diagrams")]
const ZOOM_MIN: f32 = 0.1;
#[cfg(feature = "diagrams")]
const ZOOM_MAX: f32 = 6.0;
#[cfg(feature = "diagrams")]
const FIT_MAX: f32 = 2.0;
/// One step of the zoom buttons and keys.
#[cfg(feature = "diagrams")]
const ZOOM_STEP: f32 = 1.25;

/// What the window's tool bar and keys ask of the zoom.
#[cfg(feature = "diagrams")]
enum ZoomTo {
    Fit,
    By(f32),
    Exactly(f32),
}

/// The diagram window's content. Returns whether it is to close.
#[cfg(feature = "diagrams")]
fn window_ui(ui: &mut Ui, cache: &mut Cache, lang: Language) -> bool {
    use egui::{Key as K, Modifiers, Sense, pos2, vec2};
    let ctx = ui.ctx().clone();
    cache.receive(&ctx);
    let t = Tokens::get(&ctx);
    if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(K::Escape)) {
        return true;
    }
    let Some(z) = cache.zoomed.as_ref() else { return true };
    let (source, dark) = (z.source, z.dark);
    let own = cache.sizes.get(&(source, dark)).copied().filter(|s| s.x > 0.0 && s.y > 0.0).unwrap_or(vec2(640.0, 400.0));
    let page = ui.max_rect();
    ui.painter().rect_filled(page, 0.0, if t.dark { t.window } else { t.grid_bg });

    // The tool bar: − zoom +, Fit to Window, 100%, and how to move about.
    let bar = egui::Rect::from_min_size(page.min, vec2(page.width(), 36.0));
    ui.painter().rect_filled(bar, 0.0, t.ribbon);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], egui::Stroke::new(1.0, t.ribbon_border));
    let canvas = egui::Rect::from_min_max(pos2(page.left(), bar.bottom()), page.max);
    let fit = ((canvas.width() - 32.0) / own.x).min((canvas.height() - 32.0) / own.y).clamp(ZOOM_MIN, FIT_MAX);
    let zoom = z.zoom.unwrap_or(fit);
    let fitting = z.zoom.is_none();
    let mut to = None;
    let mut bar_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if bar_ui.button("−").on_hover_text(format!("{} (Ctrl+−)", lang.tr("Zoom Out"))).clicked() {
        to = Some(ZoomTo::By(1.0 / ZOOM_STEP));
    }
    bar_ui.add(egui::Label::new(egui::RichText::new(format!("{:.0}%", zoom * 100.0)).font(crate::theme::ui_font(12.5))).selectable(false));
    if bar_ui.button("+").on_hover_text(format!("{} (Ctrl++)", lang.tr("Zoom In"))).clicked() {
        to = Some(ZoomTo::By(ZOOM_STEP));
    }
    bar_ui.add_space(6.0);
    if bar_ui.add(egui::Button::selectable(fitting, lang.tr("Fit to Window"))).on_hover_text("Ctrl+0").clicked() {
        to = Some(ZoomTo::Fit);
    }
    if bar_ui.button("100%").clicked() {
        to = Some(ZoomTo::Exactly(1.0));
    }
    bar_ui.add_space(10.0);
    let hint =
        egui::RichText::new(lang.tr("Ctrl+wheel to zoom, drag to move, double-click to fit")).font(crate::theme::ui_font(11.5)).color(t.text_dim);
    bar_ui.add(egui::Label::new(hint).truncate());
    let keys = ctx.input_mut(|i| {
        if i.consume_key(Modifiers::COMMAND, K::Plus) || i.consume_key(Modifiers::COMMAND, K::Equals) {
            Some(ZoomTo::By(ZOOM_STEP))
        } else if i.consume_key(Modifiers::COMMAND, K::Minus) {
            Some(ZoomTo::By(1.0 / ZOOM_STEP))
        } else if i.consume_key(Modifiers::COMMAND, K::Num0) {
            Some(ZoomTo::Fit)
        } else {
            None
        }
    });
    to = to.or(keys);
    // Ctrl+wheel (and a pinch) over the diagram zooms about the pointer.
    let (wheel, pointer) = ctx.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
    let over = pointer.filter(|p| canvas.contains(*p));
    if (wheel - 1.0).abs() > f32::EPSILON && over.is_some() {
        to = Some(ZoomTo::By(wheel));
    }
    if let Some(to) = to
        && let Some(z) = cache.zoomed.as_mut()
    {
        let new = match to {
            ZoomTo::Fit => None,
            ZoomTo::By(f) => Some((zoom * f).clamp(ZOOM_MIN, ZOOM_MAX)),
            ZoomTo::Exactly(v) => Some(v),
        };
        // The point under the pointer (or the middle) stays where it is.
        if let Some((inner, offset)) = z.last {
            let after = new.unwrap_or(fit);
            let anchor = over.unwrap_or(inner.center());
            let at = anchor - inner.min;
            z.scroll = Some(((at + offset) * (after / zoom) - at).max(vec2(0.0, 0.0)));
        }
        z.zoom = new;
        ctx.request_repaint();
    }
    let zoom = cache.zoomed.as_ref().and_then(|z| z.zoom).unwrap_or(fit);

    // The drawing at this zoom, drawn for it once it is in (the nearest one stretched meanwhile).
    let ppp = ctx.pixels_per_point();
    let key = Key { source, scale: thousandths(zoom * ppp), dark };
    if !cache.entries.contains_key(&key) && !cache.drawing(source, dark) {
        let text = cache.zoomed.as_ref().map(|z| z.text.clone()).unwrap_or_default();
        cache.start(&ctx, key, text);
    }
    let frame = ctx.cumulative_frame_nr();
    let ready = |k: &Key, e: &Entry| k.source == source && k.dark == dark && matches!(e.state, State::Ready(_));
    let shown = match cache.entries.get(&key) {
        Some(e) if matches!(e.state, State::Ready(_)) => Some(key),
        _ => cache.entries.iter().filter(|(k, e)| ready(k, e)).min_by_key(|(k, _)| k.scale.abs_diff(key.scale)).map(|(k, _)| *k),
    };
    for k in [Some(key), shown].into_iter().flatten() {
        if let Some(e) = cache.entries.get_mut(&k) {
            e.used = frame;
        }
    }
    let texture = shown.and_then(|k| cache.entries.get(&k)).and_then(|e| match &e.state {
        State::Ready(texture) => Some(texture.clone()),
        _ => None,
    });
    let scroll = cache.zoomed.as_mut().and_then(|z| z.scroll.take());
    let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(canvas));
    let wheel_and_bars = egui::scroll_area::ScrollSource { scroll_bar: true, drag: egui::scroll_area::DragScroll::Never, mouse_wheel: true };
    let mut area = egui::ScrollArea::both().id_salt("monun.diagram_window").auto_shrink([false, false]).scroll_source(wheel_and_bars);
    if let Some(offset) = scroll {
        area = area.scroll_offset(offset);
    }
    let mut fit_again = false;
    let mut pan = None;
    let out = area.show(&mut canvas_ui, |ui| {
        let size = own * zoom;
        let room = ui.available_size();
        let pad = ((room - size) / 2.0).max(vec2(16.0, 16.0));
        let (rect, resp) = ui.allocate_exact_size(size + pad * 2.0, Sense::click_and_drag());
        let image = egui::Rect::from_min_size(rect.min + pad, size);
        match &texture {
            Some(texture) => {
                ui.painter().image(texture.id(), image, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => {
                ui.put(egui::Rect::from_center_size(image.center(), vec2(24.0, 24.0)), egui::Spinner::new());
            }
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, lang.tr("Diagram")));
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            pan = Some(resp.drag_delta());
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        fit_again = resp.double_clicked();
    });
    if let Some(z) = cache.zoomed.as_mut() {
        z.last = Some((out.inner_rect, out.state.clamped_offset()));
        if let Some(delta) = pan {
            z.scroll = Some((out.state.clamped_offset() - delta).max(vec2(0.0, 0.0)));
            ctx.request_repaint();
        }
        if fit_again {
            z.zoom = None;
            z.scroll = Some(vec2(0.0, 0.0));
        }
    }
    false
}
