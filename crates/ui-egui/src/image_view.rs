//! Pictures (.png, .jpg, .gif, .bmp, .webp, .ico, .tif) in their tab, to look at: as large as fits
//! the window, or at their actual size, or at the tab's zoom (Ctrl+wheel, the status bar). The
//! engine only knows the picture as an empty read-only tab ([`monun_engine::docs::image_workbook`]).
//!
//! The file is read and decoded off the UI thread and made a texture there (a picture larger
//! than a texture can be is scaled down to fit one first); the tab shows it with mipmaps, so a
//! large photo fitted to the window stays smooth. An animated GIF shows its first frame.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use monun_engine::docs::TextKind;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::theme::{self, Tokens};

/// Room around a picture fitted to the window (egui points).
const MARGIN: f32 = 16.0;

/// A picture read: its texture, its size (pixels), its format and its file's size (bytes).
struct Picture {
    texture: TextureHandle,
    size: [usize; 2],
    format: String,
    bytes: u64,
}

enum Phase {
    Opening,
    Ready(Picture),
    Failed(String),
}

struct Pic {
    phase: Phase,
    /// The zoom Fit set, to tell a zoom set any other way (which ends Fit).
    fit_zoom: Option<u16>,
}

/// What a thread reading a picture sends back.
type Loaded = (u64, Result<Picture, String>);

/// The open pictures, by workbook uid.
#[derive(Default)]
pub struct State {
    pics: HashMap<u64, Pic>,
    loads: Option<(mpsc::Sender<Loaded>, mpsc::Receiver<Loaded>)>,
}

/// Draws the active picture's tab.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    receive(app);
    let Some(d) = app.session.active() else { return };
    let key = (d.uid, d.wb.active_sheet());
    let path = d.path.clone();
    let (zoom, fit) = d.wb.active().map_or((100, true), |sh| (sh.zoom.clamp(10, 400), sh.document.as_ref().is_some_and(|doc| doc.fit_width)));
    // Open, New, Close, Find… as in any document's tab.
    doc_view::keyboard(app, ui, Mode::View, TextKind::Image);
    if !app.images.pics.contains_key(&key.0) {
        let pic = open(app, ui.ctx(), key.0, path);
        app.images.pics.insert(key.0, pic);
    }
    draw(app, ui, key, (zoom, fit));
}

/// Starts reading the picture of tab `uid` on a thread of its own.
fn open(app: &mut SheetApp, ctx: &egui::Context, uid: u64, path: Option<String>) -> Pic {
    let Some(path) = path else { return Pic { phase: Phase::Failed("no file".into()), fit_zoom: None } };
    let tx = app.images.loads.get_or_insert_with(mpsc::channel).0.clone();
    let max_side = ctx.input(|i| i.max_texture_side).max(256);
    let ctx = ctx.clone();
    let started = std::thread::Builder::new().name("monun-image-open".into()).spawn(move || {
        let read =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(&ctx, &path, max_side, uid))).unwrap_or_else(|_| Err("damaged".into()));
        let _ = tx.send((uid, read));
        ctx.request_repaint();
    });
    let phase = match started {
        Ok(_) => Phase::Opening,
        Err(e) => Phase::Failed(e.to_string()),
    };
    Pic { phase, fit_zoom: None }
}

/// Reads and decodes the picture at `path`, scaled down to `max_side` if larger, as a texture.
fn read(ctx: &egui::Context, path: &str, max_side: usize, uid: u64) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let format = image::guess_format(&bytes).map_err(|_| "damaged".to_owned())?;
    let picture = image::load_from_memory_with_format(&bytes, format).map_err(|_| "damaged".to_owned())?;
    let size = [picture.width() as usize, picture.height() as usize];
    let side = u32::try_from(max_side).unwrap_or(u32::MAX);
    let picture =
        if picture.width() > side || picture.height() > side { picture.resize(side, side, image::imageops::FilterType::Triangle) } else { picture };
    let rgba = picture.to_rgba8();
    let shown = [rgba.width() as usize, rgba.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(shown, rgba.as_raw());
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let texture = ctx.load_texture(format!("monun.image.{uid}"), image, options);
    let name = format!("{format:?}").to_uppercase();
    Ok(Picture { texture, size, format: name, bytes: bytes.len() as u64 })
}

/// Takes the pictures the reading threads have read.
fn receive(app: &mut SheetApp) {
    let Some((_, rx)) = &app.images.loads else { return };
    let loaded: Vec<Loaded> = rx.try_iter().collect();
    for (uid, read) in loaded {
        if let Some(pic) = app.images.pics.get_mut(&uid) {
            pic.phase = match read {
                Ok(p) => Phase::Ready(p),
                Err(why) => Phase::Failed(why),
            };
        }
    }
}

/// Forgets the pictures closed: every frame, after the window is drawn.
pub(crate) fn tidy(app: &mut SheetApp) {
    if app.images.pics.is_empty() {
        return;
    }
    receive(app);
    let open: HashSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.images.pics.retain(|uid, _| open.contains(uid));
}

/// The tab: the tool bar and the picture.
fn draw(app: &mut SheetApp, ui: &mut Ui, key: (u64, usize), (zoom, fit): (u16, bool)) {
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    ui.painter().rect_filled(ui.max_rect(), 0.0, if t.dark { t.window } else { t.ribbon });
    doc_view::tool_strip(ui, &t, |ui| {
        let label = |s: &str| RichText::new(lang.tr(s)).font(theme::ui_font(13.0));
        if ui.add(egui::Button::selectable(fit, label("Fit to Window"))).on_hover_text(lang.tr("Show the whole picture in the window")).clicked() {
            app.run_or_alert("doc.view", json!({"sheet": key.1, "fitWidth": !fit}));
        }
        let actual = !fit && zoom == 100;
        if ui.add(egui::Button::selectable(actual, label("Actual Size"))).on_hover_text(lang.tr("Show the picture at its own size (100%)")).clicked()
        {
            if fit {
                app.run_or_alert("doc.view", json!({"sheet": key.1, "fitWidth": false}));
            }
            app.run_or_alert("view.zoom", json!({"percent": 100}));
        }
        doc_view::separator(ui, &t);
        if let Some(Phase::Ready(p)) = app.images.pics.get(&key.0).map(|pic| &pic.phase) {
            ui.label(RichText::new(format!("{} × {} · {}", p.size[0], p.size[1], p.format)).font(theme::ui_font(13.0)).color(t.text_dim));
        }
    });
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    let Some(pic) = app.images.pics.get_mut(&key.0) else { return };
    let picture = match &pic.phase {
        Phase::Ready(p) => p,
        Phase::Opening => return note(ui, area, lang.tr("Opening…"), &t),
        Phase::Failed(why) => {
            let text = match why.as_str() {
                "damaged" => lang.tr("This picture couldn't be read: it may be damaged, or of a kind Monun doesn't show.").to_owned(),
                other => format!("{} {other}", lang.tr("The file couldn't be read:")),
            };
            return note(ui, area, &text, &t);
        }
    };
    let (w, h) = (picture.size[0] as f32, picture.size[1] as f32);
    // Fit: the zoom that shows it whole (never larger than its own size), set as the tab's zoom;
    // a zoom set any other way since ends Fit.
    let mut zoom = zoom;
    if fit && pic.fit_zoom.is_some_and(|z| z != zoom) {
        pic.fit_zoom = None;
        let _ = app.session.run("doc.view", json!({"sheet": key.1, "fitWidth": false}));
    } else if fit {
        let room = (area.width() - 2.0 * MARGIN).max(16.0).min((area.height() - 2.0 * MARGIN).max(16.0) * w / h.max(1.0));
        let fitted = ((room / w.max(1.0)) * 100.0).floor().clamp(10.0, 100.0) as u16;
        if fitted != zoom {
            let _ = app.session.run("view.zoom", json!({"percent": fitted}));
        }
        pic.fit_zoom = Some(fitted);
        zoom = fitted;
    } else {
        pic.fit_zoom = None;
    }
    let scale = f32::from(zoom) / 100.0;
    let shown = vec2(w * scale, h * scale);
    let texture = picture.texture.id();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
    egui::ScrollArea::both().id_salt(("monun.image_view", key)).auto_shrink([false, false]).show(&mut child, |ui| {
        let room = vec2(area.width().max(shown.x + 2.0 * MARGIN), area.height().max(shown.y + 2.0 * MARGIN));
        let (rect, _) = ui.allocate_exact_size(room, Sense::hover());
        let at = Rect::from_center_size(rect.center(), shown);
        let p = ui.painter();
        checks(p, at);
        p.image(texture, at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.rect_stroke(at, 0.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Outside);
    });
    let (zoom_delta, now) = ui.input(|i| (i.zoom_delta(), i.time));
    if zoom_delta != 1.0 && ui.rect_contains_pointer(area) {
        crate::grid::turn_wheel_zoom(app, ui.ctx(), key, zoom, zoom_delta, now);
    }
    crate::grid::update_zoom_glide(app, ui, key, zoom, now);
}

/// A checkerboard under a picture, which shows where it is transparent.
fn checks(p: &egui::Painter, at: Rect) {
    let clip = p.clip_rect().intersect(at);
    if !clip.is_positive() {
        return;
    }
    p.rect_filled(clip, 0.0, Color32::from_gray(255));
    let cell = 8.0;
    let (x0, y0) = (((clip.left() - at.left()) / cell).floor(), ((clip.top() - at.top()) / cell).floor());
    let (cols, rows) = (((clip.right() - at.left()) / cell).ceil() - x0, ((clip.bottom() - at.top()) / cell).ceil() - y0);
    for r in 0..rows.max(0.0) as i64 {
        for c in 0..cols.max(0.0) as i64 {
            let (gx, gy) = (x0 as i64 + c, y0 as i64 + r);
            if (gx + gy) % 2 == 0 {
                continue;
            }
            let square = Rect::from_min_size(pos2(at.left() + gx as f32 * cell, at.top() + gy as f32 * cell), vec2(cell, cell)).intersect(clip);
            p.rect_filled(square, 0.0, Color32::from_gray(232));
        }
    }
}

/// A line of text in the middle of the tab, instead of the picture.
fn note(ui: &mut Ui, area: Rect, text: &str, t: &Tokens) {
    ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, text, theme::ui_font(14.0), t.text_dim);
}

/// The status bar's words for the active picture: its size, kind and file size.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| d.is_image())?;
    let Some(Phase::Ready(p)) = app.images.pics.get(&d.uid).map(|pic| &pic.phase) else { return Some(String::new()) };
    let kb = (p.bytes as f64 / 1024.0).ceil() as u64;
    let file = if kb >= 1024 { format!("{:.1} MB", kb as f64 / 1024.0) } else { format!("{kb} KB") };
    Some(format!("{} × {} · {} · {file}", p.size[0], p.size[1], p.format))
}

/// The active picture as `ui.inspect` reports it (`null` for any other tab).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| d.is_image()) else { return Json::Null };
    match app.images.pics.get(&d.uid).map(|pic| &pic.phase) {
        Some(Phase::Ready(p)) => json!({"status": "ready", "width": p.size[0], "height": p.size[1], "format": p.format, "bytes": p.bytes}),
        Some(Phase::Failed(why)) => json!({"status": format!("failed: {why}")}),
        _ => json!({"status": "opening"}),
    }
}
