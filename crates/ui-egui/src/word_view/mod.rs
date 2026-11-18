//! Word documents in their tab: read, and in Edit mode written, laid out as a Markdown file is —
//! the tool bar, the find bar under it, the contents (headings) beside the pages. The engine only
//! knows the document as an empty tab ([`monun_engine::docs::word_workbook`]); this holds it in a
//! WordCraft session (`wordcraft-engine`), which reads and writes .docx, lays out the pages, runs
//! the editing commands and keeps the undo history, and exports the PDF (`wordcraft-pdf`).
//!
//! Speed first: the file is read and parsed off the UI thread (the system's fonts are listed
//! meanwhile); pages are drawn by one worker ([`render`]), the pages in view first, at the
//! screen's exact pixels; an edit lays out again only what it touched (WordCraft's paragraph
//! cache) and draws again only the pages whose content changed, the old picture showing until
//! the new one comes. Spell checking and AutoCorrect are left out entirely: the session never
//! checks, and typing goes in as typed.

mod bar;
mod ime;
mod input;
mod render;

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use monun_engine::docs::TextKind;
use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, Pos};
use wordcraft_engine::layout::DocLayout;

use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::page_layout::{self as layout, GAP, Layout, POINTS_PER_PT};
use crate::theme::{self, Tokens};
use render::{Job, PageKeys, Renderer};

/// How long a new zoom rests before its pages are drawn anew (s).
const HOLD: f64 = 0.12;
/// Pages drawn ahead of the view in the direction it scrolls, and behind it.
const AHEAD: usize = 2;
const BEHIND: usize = 1;
/// Room left for the vertical scroll bar beside the pages (egui points).
const BAR_ROOM: f32 = 14.0;
/// Pages drawn taken from the worker in one frame.
const PER_FRAME: usize = 6;
/// How long the caret shows, then hides (s).
const BLINK: f64 = 0.53;

/// A heading of a document: where it is, how deep, what it says.
type Heading = (wordcraft_engine::doc::Path, u8, String);
/// A channel and its receiving end.
type Channel<T> = Option<(mpsc::Sender<T>, mpsc::Receiver<T>)>;
/// Something worked out for a document's revision and its selection.
type ForSelection<T> = Option<(u64, wordcraft_engine::Selection, T)>;

/// The open Word documents, by workbook uid, and what they share.
#[derive(Default)]
pub struct State {
    docs: HashMap<u64, Doc>,
    /// What the threads opening files send back.
    loads: Channel<Loaded>,
    /// The page drawing worker, started with the first document.
    renderer: Option<Renderer>,
    /// The installed font families, once listed, for the font box.
    fonts: Option<Arc<Vec<String>>>,
    /// PDF exports finishing off the UI thread: the file made, or why not.
    exports: Channel<Result<String, String>>,
    /// AutoRecover copies to open tabs from, by workbook uid, until they are read.
    adopted: HashMap<u64, Vec<u8>>,
    /// The tab the worker's queue was last filled for.
    queued_for: Option<u64>,
}

/// What a thread opening a document sends back.
enum Loaded {
    Ready { uid: u64, word: Box<Word> },
    Failed { uid: u64, why: String },
}

enum Phase {
    Opening,
    Ready(Box<Word>),
    Failed(String),
}

/// A document read: its session, its layout (the one drawn, and the document it was made from).
pub(crate) struct Word {
    session: Session,
    layout: Arc<DocLayout>,
    snapshot: Arc<Document>,
    keys: PageKeys,
}

impl Word {
    fn new(doc: Document, password: Option<String>, recovered: bool) -> Word {
        let mut session = Session::new(doc);
        session.system_region = region();
        // No spelling or grammar checks, and typing goes in as typed.
        session.prefs.check_spelling = false;
        session.prefs.mark_grammar = false;
        session.prefs.autocorrect.enabled = false;
        session.view.proofing = false;
        session.password = password.map(|p| wordcraft_engine::Password::new(&p));
        // A recovered copy is unsaved work.
        session.dirty = recovered;
        let layout = session.layout();
        let snapshot = Arc::new(session.doc.clone());
        Word { session, layout, snapshot, keys: PageKeys::default() }
    }

    /// The layout of the document as it is now: laid out again after an edit (only what it
    /// touched), with the document it shows.
    fn refresh(&mut self) {
        let layout = self.session.layout();
        if !Arc::ptr_eq(&layout, &self.layout) {
            self.layout = layout;
            self.snapshot = Arc::new(self.session.doc.clone());
        }
    }

    /// Runs a WordCraft command; what it asks of an interface WordCraft's own has (dialogs, panes)
    /// is let go.
    fn run(&mut self, id: &str, params: &Json) -> Result<Json, String> {
        let r = self.session.run(id, params).map_err(|e| e.to_string());
        self.session.ui_requests.clear();
        r
    }

    /// The document as .docx bytes.
    fn docx(&self) -> Result<Vec<u8>, String> {
        let password = self.session.password.as_ref().map(wordcraft_engine::Password::as_str);
        wordcraft_engine::io::save_bytes_with("document.docx", &self.session.doc, password, &wordcraft_engine::io::ExportOptions::default())
    }
}

/// The region documents are made and read for: Korean, so a new one is A4 in millimetres.
fn region() -> wordcraft_engine::Region {
    wordcraft_engine::Region::from_tag("ko-KR")
}

/// A document made ready to show — its session made and first laid out — or `None` if WordCraft
/// stopped on it.
fn ready(doc: Document, password: Option<String>, recovered: bool) -> Option<Word> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Word::new(doc, password, recovered))).ok()
}

/// Where to scroll to next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Go {
    Top,
    Bottom,
    By(f32),
    Screens(f32),
    /// A page, this far down it (points), that far below the top of the view (egui points).
    At(usize, f32, f32),
}

/// A page drawn, at what fingerprint and scale; with the layout it came from (see [`render::Done`]).
struct Tex {
    key: u64,
    scale: f32,
    size: [usize; 2],
    texture: egui::TextureHandle,
    _layout: Arc<DocLayout>,
}

impl Tex {
    fn bytes(&self) -> usize {
        self.size[0].saturating_mul(self.size[1]).saturating_mul(4)
    }
}

/// One open Word document.
struct Doc {
    phase: Phase,
    spread: bool,
    textures: HashMap<usize, Tex>,
    /// What the worker was last asked for.
    queue: Vec<(u64, usize, u64, u32)>,
    offset: Vec2,
    view: Rect,
    direction: i8,
    top: Option<(usize, f32)>,
    page: usize,
    shown_pages: (usize, usize),
    on_screen: Vec<(usize, Rect)>,
    scale: f32,
    scaled_at: f64,
    fit_zoom: Option<u16>,
    restore: Option<usize>,
    go: Option<Go>,
    /// Scroll the caret into view next frame (after typing or a key).
    reveal: bool,
    page_typed: Option<String>,
    shown: u64,
    /// When the caret last moved, for its blinking (s).
    blink_from: f64,
    /// Where a drag selecting text started.
    drag: Option<Pos>,
    ime: ime::Ime,
    /// What Find found: the matches and which is current.
    found: Option<(usize, Option<usize>)>,
    /// The headings, for a revision of the document.
    contents: Option<(u64, Vec<Heading>)>,
    /// The formatting at the selection, for a revision and a selection.
    format: ForSelection<Json>,
    /// The selection's rectangles, for a layout and a selection.
    selected: ForSelection<Vec<(usize, [f32; 4])>>,
    opened: Option<f64>,
    /// Commands the tool bar asked for that need the document back in its place (Export as PDF).
    after: Vec<(&'static str, Json)>,
    /// The pages take the keyboard back next frame (after a tool bar button: the click that
    /// pressed it leaves the keyboard with nothing).
    refocus: bool,
}

impl Doc {
    fn new(phase: Phase, spread: bool, restore: Option<usize>) -> Doc {
        Doc {
            phase,
            spread,
            textures: HashMap::new(),
            queue: Vec::new(),
            offset: Vec2::ZERO,
            view: Rect::NOTHING,
            direction: 1,
            top: None,
            page: 0,
            shown_pages: (0, 0),
            on_screen: Vec::new(),
            scale: 0.0,
            scaled_at: 0.0,
            fit_zoom: None,
            restore,
            go: None,
            reveal: false,
            page_typed: None,
            shown: 0,
            blink_from: 0.0,
            drag: None,
            ime: ime::Ime::default(),
            found: None,
            contents: None,
            format: None,
            selected: None,
            opened: None,
            after: Vec::new(),
            refocus: false,
        }
    }

    fn pages(&self) -> usize {
        match &self.phase {
            Phase::Ready(w) => w.layout.pages.len(),
            _ => 0,
        }
    }

    fn word(&mut self) -> Option<&mut Word> {
        match &mut self.phase {
            Phase::Ready(w) => Some(w),
            _ => None,
        }
    }
}

/// The id of the pages' area, which takes the keyboard to type.
fn canvas_id(uid: u64) -> egui::Id {
    egui::Id::unique(("monun.word.canvas", uid))
}

/// Draws the active Word document's tab.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    receive_loads(app);
    let Some(d) = app.session.active() else { return };
    let key = (d.uid, d.wb.active_sheet());
    let source = d.path.clone().or_else(|| d.wb.source_path.as_ref().map(|p| p.to_string_lossy().into_owned()));
    let sheet = d.wb.active();
    let zoom = sheet.map_or(100, |sh| sh.zoom.clamp(10, 400));
    let docmode = sheet.and_then(|sh| sh.document.as_ref());
    let fit = docmode.is_some_and(|doc| doc.fit_width);
    let editing = docmode.and_then(|doc| doc.mode) == Some(Mode::Edit);
    // Save, Open, New, Close, Undo, Redo and Find as in a Markdown file's tab: first, while the
    // document is where Save and Undo look for it.
    doc_view::keyboard(app, ui, Mode::View, TextKind::Word);
    let mut doc = match app.word.docs.remove(&key.0) {
        Some(doc) => doc,
        None => open(app, ui.ctx(), key, source),
    };
    doc.shown = ui.ctx().cumulative_frame_nr();
    draw(app, ui, &mut doc, key, (zoom, fit, editing));
    let after = std::mem::take(&mut doc.after);
    app.word.docs.insert(key.0, doc);
    keep_to_budget(app, key.0);
    for (id, params) in after {
        app.run_or_alert(id, params);
    }
}

/// Gives tab `uid` a document's bytes (an AutoRecover copy) to open from, rather than a file;
/// read when the tab first shows, as unsaved work.
pub(crate) fn adopt(app: &mut SheetApp, uid: u64, bytes: Vec<u8>) {
    app.word.adopted.insert(uid, bytes);
}

/// The revision of tab `uid`'s document, for AutoRecover to tell whether its copy is behind: an
/// AutoRecover copy not read yet is revision 0. `None` for any other tab.
pub(crate) fn recovery_rev(state: &State, uid: u64) -> Option<u64> {
    if let Some(Phase::Ready(w)) = state.docs.get(&uid).map(|d| &d.phase) {
        return Some(w.session.rev());
    }
    state.adopted.contains_key(&uid).then_some(0)
}

/// Tab `uid`'s document as .docx bytes, for AutoRecover: the copy it was recovered from while that
/// isn't read yet. `None` for any other tab.
pub(crate) fn recovery_bytes(state: &State, uid: u64) -> Option<Vec<u8>> {
    if let Some(Phase::Ready(w)) = state.docs.get(&uid).map(|d| &d.phase) {
        return w.docx().ok();
    }
    state.adopted.get(&uid).cloned()
}

/// Every frame, after the window is drawn: forgets the documents closed; stops drawing for those
/// not shown; mirrors each document's unsaved changes onto its tab (the title, the dot, closing
/// asks), and says how the PDF exports ended.
pub(crate) fn tidy(app: &mut SheetApp, ctx: &egui::Context) {
    // The system's fonts are listed off the UI thread from the first frame, so the first Word
    // document opens as fast as the next (listing them took most of a second, once).
    static FONTS: std::sync::Once = std::sync::Once::new();
    FONTS.call_once(|| wordcraft_fonts::FontDb::global().scan_in_background());
    if let Some((_, rx)) = &app.word.exports {
        let ended: Vec<Result<String, String>> = rx.try_iter().collect();
        for r in ended {
            let lang = app.ui.language;
            match r {
                Ok(path) => app.toast = Some((format!("{} {path}", lang.tr("Exported as PDF:")), crate::clock::now_ms())),
                Err(e) => app.message = Some(("Monun".into(), format!("{} {e}", lang.tr("The PDF couldn't be made:")))),
            }
        }
    }
    if app.word.docs.is_empty() {
        return;
    }
    let open: std::collections::HashSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.word.docs.retain(|uid, _| open.contains(uid));
    app.word.adopted.retain(|uid, _| open.contains(uid));
    let frame = ctx.cumulative_frame_nr();
    let shown_any = app.word.docs.values().any(|d| d.shown == frame);
    if !shown_any
        && let Some(r) = &app.word.renderer
        && app.word.docs.values().any(|d| !d.queue.is_empty())
    {
        r.set_queue(Vec::new());
        app.word.docs.values_mut().for_each(|d| d.queue.clear());
    }
    let dirty: Vec<(u64, bool)> =
        app.word.docs.iter().filter_map(|(uid, d)| if let Phase::Ready(w) = &d.phase { Some((*uid, w.session.dirty)) } else { None }).collect();
    for (uid, dirty) in dirty {
        let Some(d) = app.session.document_mut(uid) else { continue };
        if d.is_dirty() != dirty {
            d.saved = if dirty { Arc::new((*d.wb).clone()) } else { d.wb.clone() };
        }
    }
}

/// Where a document is read from.
enum Source {
    Path(String),
    Bytes(String, Vec<u8>),
    Blank,
}

/// Starts reading the document of tab `key`, off the UI thread (a new document is made at once).
fn open(app: &mut SheetApp, ctx: &egui::Context, (uid, sheet): (u64, usize), path: Option<String>) -> Doc {
    let spread = path.as_ref().and_then(|p| app.ui.doc_views.iter().find(|v| &v.path == p)).is_some_and(|v| v.spread);
    let restore = app.doc_view.restore.remove(&(uid, sheet));
    let tx = app.word.loads.get_or_insert_with(mpsc::channel).0.clone();
    // A recovered copy stays at hand until it is read, for AutoRecover to keep writing.
    let source = match (app.word.adopted.get(&uid).cloned(), path) {
        (Some(bytes), _) => Source::Bytes("recovered.docx".into(), bytes),
        (None, Some(p)) => Source::Path(p),
        (None, None) => Source::Blank,
    };
    let phase = read(uid, source, tx, ctx.clone());
    let mut doc = Doc::new(phase, spread, restore);
    doc.opened = Some(ctx.input(|i| i.time));
    doc
}

/// Reads a document on a thread of its own and sends it back; a new one is made here and now. The
/// system's fonts are listed meanwhile, once.
fn read(uid: u64, source: Source, tx: mpsc::Sender<Loaded>, ctx: egui::Context) -> Phase {
    wordcraft_fonts::FontDb::global().scan_in_background();
    if let Source::Blank = source {
        let doc = wordcraft_engine::blank_document(&region());
        return match ready(doc, None, false) {
            Some(word) => Phase::Ready(Box::new(word)),
            None => Phase::Failed("damaged".into()),
        };
    }
    let recovered = matches!(source, Source::Bytes(..));
    let started = std::thread::Builder::new().name("monun-word-open".into()).spawn(move || {
        let (name, bytes) = match source {
            Source::Path(p) => match std::fs::read(&p) {
                Ok(b) => (p, b),
                Err(e) => {
                    let _ = tx.send(Loaded::Failed { uid, why: e.to_string() });
                    return ctx.request_repaint();
                }
            },
            Source::Bytes(name, bytes) => (name, bytes),
            Source::Blank => return,
        };
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wordcraft_engine::io::open_bytes_in(&name, &bytes, None, &region())));
        // Its session is made and first laid out here too, off the UI thread.
        let loaded = match read {
            Ok(Ok((doc, _))) => match ready(doc, None, recovered) {
                Some(word) => Loaded::Ready { uid, word: Box::new(word) },
                None => Loaded::Failed { uid, why: "damaged".into() },
            },
            Ok(Err(e)) if wordcraft_engine::io::needs_password(&e) => Loaded::Failed { uid, why: "password".into() },
            Ok(Err(e)) => Loaded::Failed { uid, why: e },
            Err(_) => Loaded::Failed { uid, why: "damaged".into() },
        };
        let _ = tx.send(loaded);
        ctx.request_repaint();
    });
    match started {
        Ok(_) => Phase::Opening,
        Err(e) => Phase::Failed(e.to_string()),
    }
}

/// Takes the documents the opening threads have read.
fn receive_loads(app: &mut SheetApp) {
    let Some((_, rx)) = &app.word.loads else { return };
    let loaded: Vec<Loaded> = rx.try_iter().collect();
    for l in loaded {
        match l {
            Loaded::Ready { uid, word } => {
                app.word.adopted.remove(&uid);
                if let Some(d) = app.word.docs.get_mut(&uid) {
                    d.phase = Phase::Ready(word);
                }
            }
            Loaded::Failed { uid, why } => {
                if let Some(d) = app.word.docs.get_mut(&uid) {
                    d.phase = Phase::Failed(why);
                }
            }
        }
    }
}

/// The tab: the tool bar, the find bar, the contents and the pages.
fn draw(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, key: (u64, usize), (zoom, fit, editing): (u16, bool, bool)) {
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    ui.painter().rect_filled(ui.max_rect(), 0.0, if t.dark { t.window } else { t.ribbon });
    input::keys(app, ui.ctx(), doc, key.0, editing);
    doc_view::tool_strip(ui, &t, |ui| bar::toolbar(app, ui, doc, key, (fit, editing), &t));
    if app.doc_view.find.is_some() {
        bar::find_bar(app, ui, doc, editing, &t);
    }
    let area = ui.available_rect_before_wrap();
    let body = if app.ui.doc_outline {
        let side = Rect::from_min_max(area.min, pos2(area.left() + doc_view::contents_width(area), area.bottom()));
        bar::contents(app, ui, doc, side, &t);
        Rect::from_min_max(pos2(side.right(), area.top()), area.max)
    } else {
        area
    };
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    match std::mem::replace(&mut doc.phase, Phase::Opening) {
        Phase::Ready(mut word) => {
            pages(app, &mut body_ui, doc, &mut word, key, (zoom, fit, editing));
            doc.phase = Phase::Ready(word);
        }
        Phase::Opening => note(&mut body_ui, lang.tr("Opening…"), &t),
        Phase::Failed(why) => {
            let text = match why.as_str() {
                "password" => lang.tr("This document is protected by a password. Monun can't open it.").to_owned(),
                "damaged" => lang.tr("This file isn't a document Monun can read: it may be damaged.").to_owned(),
                other => format!("{} {other}", lang.tr("The file couldn't be read:")),
            };
            note(&mut body_ui, &text, &t);
            doc.phase = Phase::Failed(why);
        }
    }
    ui.allocate_rect(area, Sense::hover());
    let (zoom_delta, now) = ui.input(|i| (i.zoom_delta(), i.time));
    if zoom_delta != 1.0 && ui.rect_contains_pointer(area) {
        crate::grid::turn_wheel_zoom(app, ui.ctx(), key, zoom, zoom_delta, now);
    }
    crate::grid::update_zoom_glide(app, ui, key, zoom, now);
    if let Some((page, _)) = doc.top {
        app.doc_view.tops.insert(key, page);
    }
}

/// A line of text in the middle of the tab, instead of pages.
fn note(ui: &mut Ui, text: &str, t: &Tokens) {
    let rect = ui.max_rect();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::ui_font(14.0), t.text_dim);
}

/// The pages: laid out at the zoom (or as wide as the window), scrolled, drawn, with the
/// selection, the caret and the composition over them; then the worker told what to draw next.
fn pages(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, word: &mut Word, key: (u64, usize), (zoom, fit, editing): (u16, bool, bool)) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let lang = app.ui.language;
    let (ppp, now) = (ctx.pixels_per_point(), ui.input(|i| i.time));
    let max_side = ctx.input(|i| i.max_texture_side) as f32;
    if app.word.renderer.is_none() {
        app.word.renderer = Renderer::start(ctx.clone());
    }
    receive_pages(app, doc, key.0, now);
    word.refresh();
    let sizes: Vec<[f32; 2]> = word.layout.pages.iter().map(|p| [p.w, p.h]).collect();
    let width = (ui.available_width() - BAR_ROOM).max(40.0);
    let zoom = fit_width(app, doc, &sizes, key, (zoom, fit), width);
    let scale = f32::from(zoom) / 100.0 * POINTS_PER_PT;
    let lay = layout::lay_out(&sizes, scale, doc.spread, width);
    let jump = jump(doc, word, &lay, scale, now);
    let mut area = egui::ScrollArea::both().id_salt(("monun.word_view", key)).auto_shrink([false, false]);
    if let Some(bars) = crate::panes::scroll_bars_below(app.group_handle, ui.available_rect_before_wrap()) {
        area = area.scroll_bar_rect(bars);
    }
    if let Some(j) = jump {
        area = area.horizontal_scroll_offset(j.x.max(0.0)).vertical_scroll_offset(j.y.max(0.0));
    }
    let scale_dev = scale * ppp;
    let id = canvas_id(key.0);
    // Not in the frame of the click itself, which leaves the keyboard with what was clicked.
    if doc.refocus && !ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
        doc.refocus = false;
        if editing {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
    let shown = area.show_viewport(ui, |ui, view| {
        ui.set_min_size(lay.size);
        let origin = ui.max_rect().min.to_vec2();
        let visible: Vec<usize> = lay.visible(view.top(), view.bottom()).collect();
        doc.on_screen.clear();
        for &p in &visible {
            let Some(slot) = lay.pages.get(p).map(|r| r.translate(origin)) else { continue };
            doc.on_screen.push((p, slot));
            let want = sizes.get(p).map_or(scale_dev, |s| render::raster_scale(s[0], s[1], scale_dev, max_side));
            paint_page(ui, doc, p, slot, (want, ppp), &t);
        }
        let caret = overlays(ui, doc, word, (scale, now), editing && ctx.memory(|m| m.has_focus(id)));
        let menu = input::pointer(ui, doc, word, (&lay, view.translate(origin), origin, scale), (id, editing, now, lang));
        doc.top = lay.top_at(view.top());
        doc.page = lay.most_shown(view.top(), view.bottom()).unwrap_or(0);
        doc.shown_pages = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
        (visible, view, caret, view.translate(origin), menu)
    });
    let (visible, view, caret, screen_view, menu) = shown.inner;
    match menu {
        Some(input::Menu::Copy) => {
            input::copy(app, &ctx, word, false);
        }
        Some(input::Menu::Cut) => {
            input::copy(app, &ctx, word, true);
        }
        Some(input::Menu::Paste) => {
            if let Some(text) = app.services.paste_text.as_mut().and_then(|read| read()) {
                input::paste(app, word, &text);
            }
        }
        Some(input::Menu::SelectAll) => {
            let _ = word.run("select.all", &json!({}));
        }
        None => {}
    }
    let offset = shown.state.clamped_offset();
    if offset.y != doc.offset.y {
        doc.direction = if offset.y > doc.offset.y { 1 } else { -1 };
    }
    doc.offset = offset;
    doc.view = view;
    // The platform's input method, at the caret, while the document takes typing.
    let focused = editing && ctx.memory(|m| m.has_focus(id));
    doc.ime.observe(&mut word.session, &ctx, caret.filter(|_| focused).map(|c| (screen_view, c)));
    if focused {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false })
        });
        let since = (now - doc.blink_from).rem_euclid(2.0 * BLINK);
        let next = if since < BLINK { BLINK - since } else { 2.0 * BLINK - since };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.02)));
    }
    let settled = now - doc.scaled_at >= HOLD;
    if !settled {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOLD - (now - doc.scaled_at)));
    }
    let jobs = plan(doc, word, &lay, &visible, (key.0, scale_dev, max_side), settled);
    let ids: Vec<(u64, usize, u64, u32)> = jobs.iter().map(Job::id).collect();
    if ids != doc.queue || app.word.queued_for != Some(key.0) {
        app.word.queued_for = Some(key.0);
        if let Some(r) = &app.word.renderer {
            r.set_queue(jobs);
        }
        doc.queue = ids;
    }
}

/// Where the view goes this frame: a new zoom keeps the page at the top; a key or a page number
/// goes where it says; typing keeps the caret in view.
fn jump(doc: &mut Doc, word: &mut Word, lay: &Layout, scale: f32, now: f64) -> Option<Vec2> {
    let mut jump = None;
    if doc.scale > 0.0 && !render::same(doc.scale, scale) {
        doc.scaled_at = now;
        if let Some((page, down)) = doc.top
            && let Some(r) = lay.pages.get(page)
        {
            jump = Some(vec2(doc.offset.x * scale / doc.scale, r.top() + down * r.height()));
        }
    }
    doc.scale = scale;
    if let Some(page) = doc.restore.take() {
        doc.go = Some(Go::At(page, 0.0, GAP));
    }
    if std::mem::take(&mut doc.reveal)
        && let Some(c) = word.layout.caret_on(&word.session.sel.focus, word.session.page_hint)
        && let Some(r) = lay.pages.get(c.page)
    {
        let (top, bottom) = (r.top() + c.top * scale, r.top() + (c.top + c.height) * scale);
        if top < doc.offset.y || bottom > doc.offset.y + doc.view.height() {
            doc.go = Some(Go::At(c.page, c.top, doc.view.height() / 3.0));
        }
    }
    if let Some(go) = doc.go.take() {
        let y = match go {
            Go::Top => 0.0,
            Go::Bottom => lay.size.y,
            Go::By(dy) => doc.offset.y + dy,
            Go::Screens(n) => doc.offset.y + n * doc.view.height() * 0.9,
            Go::At(page, down, room) => lay.pages.get(page).map_or(doc.offset.y, |r| r.top() + down * scale - room),
        };
        jump = Some(vec2(jump.map_or(doc.offset.x, |j: Vec2| j.x), y));
    }
    jump
}

/// With Fit Width, the zoom that makes the pages as wide as `width`, set as the tab's zoom; a zoom
/// set any other way since ends Fit Width. Returns the zoom to draw at.
fn fit_width(app: &mut SheetApp, doc: &mut Doc, sizes: &[[f32; 2]], (_, sheet): (u64, usize), (zoom, fit): (u16, bool), width: f32) -> u16 {
    if !fit {
        doc.fit_zoom = None;
        return zoom;
    }
    if doc.fit_zoom.is_some_and(|z| z != zoom) {
        doc.fit_zoom = None;
        let _ = app.session.run("doc.view", json!({"sheet": sheet, "fitWidth": false}));
        return zoom;
    }
    let Some(s) = layout::fit_scale(sizes, doc.spread, width) else { return zoom };
    let fitted = (s / POINTS_PER_PT * 100.0).floor().clamp(10.0, 400.0) as u16;
    if fitted != zoom {
        let _ = app.session.run("view.zoom", json!({"percent": fitted}));
    }
    doc.fit_zoom = Some(fitted);
    fitted
}

/// Takes the worker's pages, a few a frame, into their documents.
fn receive_pages(app: &mut SheetApp, shown: &mut Doc, shown_uid: u64, now: f64) {
    let Some(r) = &app.word.renderer else { return };
    for _ in 0..PER_FRAME {
        let Some(done) = r.try_recv() else { return };
        let tex = Tex { key: done.key, scale: done.scale, size: done.size, texture: done.texture, _layout: done.layout };
        if let Some(opened) = shown.opened.take_if(|_| done.uid == shown_uid) {
            log::info!("Word document: its first page shows {:.0} ms after opening ({:.0} ms to draw)", (now - opened) * 1000.0, done.ms);
        }
        let target = if done.uid == shown_uid { Some(&mut *shown) } else { app.word.docs.get_mut(&done.uid) };
        if let Some(d) = target {
            d.textures.insert(done.page, tex);
        }
    }
}

/// Draws page `p` into `slot`: white paper and the page's picture (on whole pixels when drawn at
/// `want`, stretched while a new one is drawn).
fn paint_page(ui: &Ui, doc: &Doc, p: usize, slot: Rect, (want, ppp): (f32, f32), t: &Tokens) {
    let painter = ui.painter();
    painter.rect_filled(slot.translate(vec2(0.0, 1.5)), 0.0, Color32::from_black_alpha(if t.dark { 90 } else { 30 }));
    painter.rect_filled(slot, 0.0, Color32::WHITE);
    if let Some(tex) = doc.textures.get(&p) {
        let exact = render::same(tex.scale, want);
        let rect = if exact {
            let min = pos2((slot.min.x * ppp).round() / ppp, (slot.min.y * ppp).round() / ppp);
            Rect::from_min_size(min, vec2(tex.size[0] as f32 / ppp, tex.size[1] as f32 / ppp))
        } else {
            slot
        };
        painter.image(tex.texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    painter.rect_stroke(slot, 0.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Outside);
}

/// The selection over the pages, the caret (blinking, while typing goes to the document) and the
/// composition in progress at it. Returns the caret's rectangle on the screen.
fn overlays(ui: &Ui, doc: &mut Doc, word: &mut Word, (scale, now): (f32, f64), typing: bool) -> Option<Rect> {
    let painter = ui.painter();
    let pages = doc.on_screen.clone();
    let on_screen = |page: usize, x: f32, y: f32| -> Option<egui::Pos2> {
        pages.iter().find(|(p, _)| *p == page).map(|(_, r)| pos2(r.left() + x * scale, r.top() + y * scale))
    };
    let sel = word.session.sel.clone();
    if !sel.is_collapsed() {
        let rev = word.session.rev();
        if !doc.selected.as_ref().is_some_and(|(r, s, _)| *r == rev && *s == sel) {
            let (a, b) = sel.ordered();
            let rects = word.layout.selection_rects(&word.session.doc, &a, &b, word.session.page_hint);
            doc.selected = Some((rev, sel.clone(), rects.into_iter().map(|(p, r)| (p, [r.x, r.y, r.w, r.h])).collect()));
        }
        if let Some((_, _, rects)) = &doc.selected {
            for (page, [x, y, w, h]) in rects {
                if let (Some(a), Some(b)) = (on_screen(*page, *x, *y), on_screen(*page, x + w, y + h)) {
                    painter.rect_filled(Rect::from_min_max(a, b), 0.0, Color32::from_rgba_unmultiplied(0, 120, 215, 80));
                }
            }
        }
    }
    let caret = word.layout.caret_on(&sel.focus, word.session.page_hint)?;
    let top = on_screen(caret.page, caret.x, caret.top)?;
    let rect = Rect::from_min_size(top, vec2(1.5, (caret.height * scale).max(4.0)));
    if typing && !doc.ime.preedit.is_empty() {
        let font = egui::FontId::proportional((caret.height * scale * 0.72).clamp(8.0, 96.0));
        let galley = painter.layout_no_wrap(doc.ime.preedit.clone(), font, Color32::BLACK);
        let shown = Rect::from_min_size(rect.min, vec2(galley.size().x, rect.height()));
        painter.rect_filled(shown, 0.0, Color32::WHITE);
        let y = shown.center().y - galley.size().y / 2.0;
        painter.galley(pos2(shown.left(), y), galley, Color32::BLACK);
        painter.line_segment([shown.left_bottom(), shown.right_bottom()], Stroke::new(1.0, Color32::BLACK));
    } else if typing && (now - doc.blink_from).rem_euclid(2.0 * BLINK) < BLINK {
        painter.rect_filled(rect, 0.0, Color32::BLACK);
    }
    Some(rect)
}

/// What the worker should draw, most needed first: the pages in view from the middle out (one
/// drawn at an older zoom waits until the zoom rests; one whose content an edit changed doesn't),
/// then the pages ahead and behind while the budget has room for them.
fn plan(doc: &Doc, word: &mut Word, lay: &Layout, visible: &[usize], (uid, scale_dev, max_side): (u64, f32, f32), settled: bool) -> Vec<Job> {
    let n = word.layout.pages.len();
    let centre = doc.view.center().y;
    let middle = |p: &usize| lay.pages.get(*p).map_or(f32::MAX, |r| (r.center().y - centre).abs());
    let mut order = visible.to_vec();
    order.sort_by(|a, b| middle(a).total_cmp(&middle(b)));
    let (first, last) = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
    let after = (last + 1..n).take(if doc.direction >= 0 { AHEAD } else { BEHIND });
    let before = (0..first).rev().take(if doc.direction >= 0 { BEHIND } else { AHEAD });
    let (near, far): (Vec<usize>, Vec<usize>) =
        if doc.direction >= 0 { (after.collect(), before.collect()) } else { (before.collect(), after.collect()) };
    let want = |p: usize| word.layout.pages.get(p).map(|pg| render::raster_scale(pg.w, pg.h, scale_dev, max_side));
    let bytes = |p: usize| {
        word.layout.pages.get(p).map_or(0, |pg| {
            let s = render::raster_scale(pg.w, pg.h, scale_dev, max_side);
            ((pg.w * s).ceil() as usize).saturating_mul((pg.h * s).ceil() as usize).saturating_mul(4)
        })
    };
    let mut room = render::BUDGET.saturating_sub(visible.iter().map(|&p| bytes(p)).sum());
    let ahead: Vec<usize> = near
        .into_iter()
        .chain(far)
        .take_while(|p| {
            let b = bytes(*p);
            let fits = b <= room;
            room = room.saturating_sub(b);
            fits
        })
        .collect();
    let mut jobs = Vec::new();
    for p in order.into_iter().chain(ahead) {
        let Some(want) = want(p) else { continue };
        let key = word.keys.get(&word.layout, p);
        let needed = match doc.textures.get(&p) {
            Some(t) if t.key == key && render::same(t.scale, want) => false,
            Some(t) if t.key == key && !settled => false,
            _ => true,
        };
        if needed {
            jobs.push(Job { uid, page: p, key, scale: want, doc: word.snapshot.clone(), layout: word.layout.clone() });
        }
    }
    jobs
}

/// Keeps the documents' pictures within [`render::BUDGET`]: the other tabs' go first, then the
/// active tab's pages farthest from its view.
fn keep_to_budget(app: &mut SheetApp, active: u64) {
    let size = |d: &Doc| d.textures.values().map(Tex::bytes).sum::<usize>();
    let mut total: usize = app.word.docs.values().map(size).sum();
    if total <= render::BUDGET {
        return;
    }
    let mut others: Vec<(u64, u64)> = app.word.docs.iter().filter(|(uid, _)| **uid != active).map(|(uid, d)| (d.shown, *uid)).collect();
    others.sort_unstable();
    for (_, uid) in others {
        if total <= render::BUDGET {
            return;
        }
        if let Some(d) = app.word.docs.get_mut(&uid) {
            total = total.saturating_sub(size(d));
            d.textures.clear();
        }
    }
    let Some(d) = app.word.docs.get_mut(&active) else { return };
    let (first, last) = d.shown_pages;
    let mut pages: Vec<usize> = d.textures.keys().copied().filter(|p| !(first..=last).contains(p)).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(d.page)));
    for p in pages {
        if total <= render::BUDGET {
            return;
        }
        if let Some(t) = d.textures.remove(&p) {
            total = total.saturating_sub(t.bytes());
        }
    }
}

/// Save, Save As, Export as PDF, Print and Undo/Redo of a Word tab, done here rather than by the
/// engine (which knows the tab only as empty). `None` for any other tab or command.
pub(crate) fn command(app: &mut SheetApp, id: &str, params: &Json) -> Option<Result<Json, String>> {
    if !matches!(id, "file.save" | "file.saveAs" | "file.saveBytes" | "file.exportPdf" | "file.print" | "edit.undo" | "edit.redo") {
        return None;
    }
    let d = app.session.active().filter(|d| d.is_word())?;
    let uid = d.uid;
    let own_path = d.path.clone();
    let path = params.get("path").and_then(Json::as_str).filter(|p| !p.is_empty()).map(str::to_owned);
    let lang = app.ui.language;
    let not_read = || Err(lang.tr("The document hasn't been read yet.").to_owned());
    let Some(word) = app.word.docs.get_mut(&uid).and_then(Doc::word) else { return Some(not_read()) };
    Some(match id {
        "edit.undo" | "edit.redo" => word.run(id, &json!({})),
        "file.saveBytes" => word.docx().map(|b| json!({"base64": monun_engine::io::base64_encode(&b)})),
        "file.print" => pdf_bytes(&word.session.doc).map(|b| json!({"base64": monun_engine::io::base64_encode(&b)})),
        "file.exportPdf" => match path {
            Some(path) => {
                export_pdf(app, uid, path.clone());
                Ok(json!({"path": path}))
            }
            None => Err(lang.tr("Choose where to save the PDF.").to_owned()),
        },
        // Saving a document not changed leaves its file as it is: WordCraft would write it anew.
        "file.save" if path.is_none() && own_path.is_some() && !word.session.dirty => Ok(json!({"path": own_path, "unchanged": true})),
        _ => match path.or(if id == "file.save" { own_path } else { None }) {
            None => {
                app.open_dialog("saveAs", json!({}));
                Ok(json!({}))
            }
            Some(path) if path.to_ascii_lowercase().ends_with(".pdf") => {
                export_pdf(app, uid, path.clone());
                Ok(json!({"path": path}))
            }
            Some(path) => save(app, uid, &path),
        },
    })
}

/// Writes the document to `path` as .docx and makes it the tab's file.
fn save(app: &mut SheetApp, uid: u64, path: &str) -> Result<Json, String> {
    if !path.to_ascii_lowercase().ends_with(".docx") {
        return Err(app.ui.language.tr("A Word document is saved as .docx.").to_owned());
    }
    let word = app.word.docs.get_mut(&uid).and_then(Doc::word).ok_or_else(|| "no document".to_owned())?;
    // The file holds when it was saved, as WordCraft's own Save writes it.
    word.session.stamp_save();
    let bytes = word.docx()?;
    monun_engine::io::write_file(path, &bytes).map_err(|e| e.to_string())?;
    word.session.dirty = false;
    word.session.path = Some(std::path::PathBuf::from(path));
    if let Some(d) = app.session.document_mut(uid) {
        d.path = Some(path.to_owned());
        d.title = std::path::Path::new(path).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path).to_owned();
        d.disk = monun_engine::io::disk_stamp(path);
        d.saved = d.wb.clone();
    }
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

/// The document as a PDF (fonts embedded, text selectable), laid out for print.
fn pdf_bytes(doc: &Document) -> Result<Vec<u8>, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wordcraft_pdf::export(doc, &wordcraft_pdf::PdfOptions::default()))) {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err("the exporter stopped".into()),
    }
}

/// Exports the document of tab `uid` as a PDF to `path` off the UI thread; a toast says when it is
/// done.
fn export_pdf(app: &mut SheetApp, uid: u64, path: String) {
    let Some(word) = app.word.docs.get_mut(&uid).and_then(Doc::word) else { return };
    // Laid out for print and written on a thread of its own.
    let doc = word.session.doc.clone();
    let tx = app.word.exports.get_or_insert_with(mpsc::channel).0.clone();
    let started = std::thread::Builder::new().name("monun-word-pdf".into()).spawn(move || {
        let r = pdf_bytes(&doc).and_then(|bytes| monun_engine::io::write_file(&path, &bytes).map(|()| path).map_err(|e| e.to_string()));
        let _ = tx.send(r);
    });
    if let Err(e) = started {
        app.message = Some(("Monun".into(), e.to_string()));
    }
}

/// Whether the active Word document shows two pages across, for remembering it.
pub fn spread(app: &SheetApp, uid: u64) -> Option<bool> {
    app.word.docs.get(&uid).map(|d| d.spread)
}

/// The status bar's words for the active Word document: the page shown and how many there are.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| d.is_word())?;
    let doc = app.word.docs.get(&d.uid);
    let pages = doc.map_or(0, Doc::pages);
    let page = doc.map_or(0, |doc| doc.page);
    Some(if pages > 0 { format!("{} {} / {pages}", app.ui.language.tr("Page"), page + 1) } else { String::new() })
}

/// The active Word document as `ui.inspect` reports it (`null` for any other tab).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| d.is_word()) else { return Json::Null };
    let Some(doc) = app.word.docs.get(&d.uid) else { return json!({"status": "opening"}) };
    let mut rendered: Vec<usize> = doc.textures.keys().copied().collect();
    rendered.sort_unstable();
    let mut out = json!({
        "pages": doc.pages(),
        "page": doc.page + 1,
        "spread": doc.spread,
        "scale": doc.scale,
        "offset": [doc.offset.x, doc.offset.y],
        "rendered": rendered,
        "queued": doc.queue.len(),
        "found": doc.found.map(|(n, i)| json!({"count": n, "index": i})),
        "onScreen": doc.on_screen.iter().map(|(p, r)| json!([p, r.left(), r.top(), r.width(), r.height()])).collect::<Vec<_>>(),
    });
    out["status"] = json!(match &doc.phase {
        Phase::Opening => "opening".to_owned(),
        Phase::Ready(_) => "ready".to_owned(),
        Phase::Failed(why) => format!("failed: {why}"),
    });
    if let Phase::Ready(w) = &doc.phase {
        out["dirty"] = json!(w.session.dirty);
        out["text"] = json!(w.session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body));
        out["selection"] = json!(!w.session.sel.is_collapsed());
        out["format"] = doc.format.as_ref().map_or(Json::Null, |(_, _, f)| f.clone());
        out["headings"] = json!(doc.contents.as_ref().map(|(_, h)| h.iter().map(|(_, _, t)| t.clone()).collect::<Vec<_>>()));
    }
    out
}
