//! 한글 documents (.hwp, .hwpx) in their tab: read, and in Edit mode written, laid out as a Word
//! document's tab is — the tool bar, the find bar under it, the contents (headings) beside the
//! pages. The engine only knows the document as an empty tab ([`monun_engine::docs::hwp_workbook`]);
//! this holds it in rhwp (`rhwp`), which reads and writes .hwp and .hwpx, lays out the pages and
//! runs the edits, on a thread of the document's own ([`thread`]).
//!
//! Speed first: nothing that touches the document runs on the UI thread, which never waits for
//! it (but Save, which takes milliseconds); typed text shows at the caret at once and goes to the
//! document in one piece however fast it comes. A page is drawn from the SVG rhwp makes of it by
//! one worker ([`render`]), the pages in view first, at the screen's exact pixels; an edit draws
//! again only the pages whose SVG it changed, the old picture showing until the new one comes. A
//! print is made on several threads at once ([`pdf`]). Spell checking and macros aren't there:
//! rhwp has neither.

mod bar;
mod ime;
mod input;
mod pdf;
mod proto;
mod render;
mod thread;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use monun_engine::docs::TextKind;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::page_layout::{self as layout, GAP, Layout, POINTS_PER_PT};
use crate::theme::{self, Tokens};
use proto::{Reply, Req, State as DocState};
use render::{Job, Renderer};
use thread::{Handle, Source};

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
/// How long Save waits for the document's thread (s): longer than any save, shorter than forever.
const SAVE_WAIT: f64 = 60.0;
/// How long AutoRecover waits for a copy (s), the thread not busy.
const RECOVERY_WAIT: f64 = 5.0;

/// A print made: its PDF, or why not.
type Made = Result<Vec<u8>, String>;

/// Said when a 한글 document is asked for a PDF: it is printed instead (a PDF printer makes one).
const NO_PDF: &str = "A Hangul document isn't exported as a PDF in Monun. Print it (Ctrl+P) and choose Microsoft Print to PDF to make one.";

/// A channel and its receiving end.
type Channel<T> = Option<(mpsc::Sender<T>, mpsc::Receiver<T>)>;

/// A print being made: for which tab, and how far it is (pages drawn, of how many).
struct Making {
    uid: u64,
    done: Arc<AtomicUsize>,
    pages: Arc<AtomicUsize>,
}

/// The open 한글 documents, by workbook uid, and what they share.
#[derive(Default)]
pub struct State {
    docs: HashMap<u64, Doc>,
    /// The page drawing worker, started with the first document.
    renderer: Option<Renderer>,
    /// The installed font families, once listed, for the font box.
    fonts: Option<Arc<Vec<String>>>,
    /// Prints made off the UI thread.
    made: Channel<Made>,
    making: Option<Making>,
    /// AutoRecover copies to open tabs from, by workbook uid, until they are read.
    adopted: HashMap<u64, Vec<u8>>,
    /// The tab the worker's queue was last filled for.
    queued_for: Option<u64>,
}

enum Phase {
    Opening,
    Ready,
    Failed(String),
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

/// A page drawn, at what fingerprint and scale.
struct Tex {
    key: u64,
    scale: f32,
    size: [usize; 2],
    texture: egui::TextureHandle,
}

impl Tex {
    fn bytes(&self) -> usize {
        self.size[0].saturating_mul(self.size[1]).saturating_mul(4)
    }
}

/// One open 한글 document.
struct Doc {
    phase: Phase,
    /// Its thread; `None` once it has stopped.
    handle: Option<Handle>,
    /// How the document was, as its thread last said.
    state: DocState,
    /// Text typed and sent but not in the document yet (shown at the caret), by the request
    /// that types it.
    pending: std::collections::VecDeque<(u64, String)>,
    /// The last request that changes the document: until it is done the document has unsaved
    /// changes, whatever the thread last said.
    edited: u64,
    /// The file the document was read from, which a save that would leave things out never
    /// replaces.
    origin: Option<String>,
    /// Undo or Redo replaced the text: the input method is told next frame.
    replaced: bool,
    /// Pages' SVG, at the revision they show.
    svgs: HashMap<usize, (u64, u64, Arc<String>)>,
    /// Pages asked for, at a revision.
    asked: HashMap<usize, u64>,
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
    /// Scroll the caret into view when the document next says where it is.
    reveal: bool,
    page_typed: Option<String>,
    shown: u64,
    /// When the caret last moved, for its blinking (s).
    blink_from: f64,
    /// A drag selecting text is going on.
    dragging: bool,
    ime: ime::Ime,
    /// What Find found: the matches and which is current.
    found: Option<(usize, Option<usize>)>,
    /// The headings, for a revision of the document; and the revision last asked for.
    contents: Option<(u64, Vec<(u8, String)>)>,
    contents_asked: Option<u64>,
    opened: Option<f64>,
    /// The pages take the keyboard back next frame (after a tool bar button).
    refocus: bool,
}

impl Doc {
    fn new(uid: u64, phase: Phase, handle: Option<Handle>, spread: bool, restore: Option<usize>) -> Doc {
        Doc {
            phase,
            handle,
            state: DocState::default(),
            pending: std::collections::VecDeque::new(),
            edited: 0,
            origin: None,
            replaced: false,
            svgs: HashMap::new(),
            asked: HashMap::new(),
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
            dragging: false,
            ime: ime::Ime::new(uid),
            found: None,
            contents: None,
            contents_asked: None,
            opened: None,
            refocus: false,
        }
    }

    fn pages(&self) -> usize {
        match self.phase {
            Phase::Ready => self.state.pages.len(),
            _ => 0,
        }
    }

    /// Sends a request to the document's thread; returns its number (0 if the thread is gone).
    fn send(&mut self, req: Req) -> u64 {
        let edits = req.edits();
        let seq = self.handle.as_mut().map_or(0, |h| h.send(req));
        if edits && seq > 0 {
            self.edited = seq;
        }
        seq
    }

    /// Types `text`: sent at once, shown at the caret until the document has it.
    fn type_text(&mut self, text: String) {
        let seq = self.send(Req::Type(text.clone()));
        if seq > 0 {
            self.pending.push_back((seq, text));
        }
    }

    /// Whether the document has changes not saved: as its thread last said, or sent and not done.
    fn dirty(&self) -> bool {
        self.state.dirty || self.edited > self.state.done
    }
}

/// The id of the pages' area, which takes the keyboard to type.
fn canvas_id(uid: u64) -> egui::Id {
    egui::Id::unique(("monun.hwp.canvas", uid))
}

/// Draws the active 한글 document's tab.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some(d) = app.session.active() else { return };
    let key = (d.uid, d.wb.active_sheet());
    let source = d.path.clone();
    let sheet = d.wb.active();
    let zoom = sheet.map_or(100, |sh| sh.zoom.clamp(10, 400));
    let docmode = sheet.and_then(|sh| sh.document.as_ref());
    let fit = docmode.is_some_and(|doc| doc.fit_width);
    let editing = docmode.and_then(|doc| doc.mode) == Some(Mode::Edit);
    // Save, Open, New, Close, Undo, Redo and Find as in a Markdown file's tab: first, while the
    // document is where Save and Undo look for it.
    doc_view::keyboard(app, ui, Mode::View, TextKind::Hwp);
    let mut doc = match app.hwp.docs.remove(&key.0) {
        Some(doc) => doc,
        None => open(app, ui.ctx(), key, source),
    };
    doc.shown = ui.ctx().cumulative_frame_nr();
    receive(app, ui.ctx(), &mut doc);
    draw(app, ui, &mut doc, key, (zoom, fit, editing));
    app.hwp.docs.insert(key.0, doc);
    keep_to_budget(app, key.0);
}

/// Gives tab `uid` a document's bytes (an AutoRecover copy) to open from, rather than a file;
/// read when the tab first shows, as unsaved work.
pub(crate) fn adopt(app: &mut SheetApp, uid: u64, bytes: Vec<u8>) {
    app.hwp.adopted.insert(uid, bytes);
}

/// The revision of tab `uid`'s document, for AutoRecover to tell whether its copy is behind: an
/// AutoRecover copy not read yet is revision 0. `None` for any other tab.
pub(crate) fn recovery_rev(state: &State, uid: u64) -> Option<u64> {
    if let Some(d) = state.docs.get(&uid).filter(|d| matches!(d.phase, Phase::Ready)) {
        return Some(d.state.rev);
    }
    state.adopted.contains_key(&uid).then_some(0)
}

/// Tab `uid`'s document as a file in its own format (.hwpx or .hwp), for AutoRecover: the copy it
/// was recovered from while that isn't read yet. `None` for any other tab, and while its thread is
/// busy (a long edit, a print): AutoRecover tries again next time rather than hold the window.
pub(crate) fn recovery_bytes(state: &State, uid: u64) -> Option<Vec<u8>> {
    if let Some(bytes) = state.adopted.get(&uid) {
        return Some(bytes.clone());
    }
    let d = state.docs.get(&uid).filter(|d| matches!(d.phase, Phase::Ready))?;
    let handle = d.handle.as_ref()?;
    if handle.sent() > d.state.done || state.making.is_some() {
        return None;
    }
    handle.export(handle.hwpx, RECOVERY_WAIT).ok().map(|e| e.bytes)
}

/// Every frame, after the window is drawn: forgets the documents closed; stops drawing for those
/// not shown; takes what the documents' threads said; mirrors each document's unsaved changes
/// onto its tab (the title, the dot, closing asks); and sends finished prints on.
pub(crate) fn tidy(app: &mut SheetApp, ctx: &egui::Context) {
    // The system's fonts are listed off the UI thread from the first frame, so the first page
    // drawn doesn't wait for them.
    render::warm();
    finish_making(app);
    // A print being made: its progress shows, and its end is taken, without waiting for input.
    if app.hwp.making.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if app.hwp.docs.is_empty() {
        return;
    }
    let open: std::collections::HashSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.hwp.docs.retain(|uid, _| open.contains(uid));
    app.hwp.adopted.retain(|uid, _| open.contains(uid));
    let frame = ctx.cumulative_frame_nr();
    let shown_any = app.hwp.docs.values().any(|d| d.shown == frame);
    if !shown_any
        && let Some(r) = &app.hwp.renderer
        && app.hwp.docs.values().any(|d| !d.queue.is_empty())
    {
        r.set_queue(Vec::new());
        app.hwp.docs.values_mut().for_each(|d| d.queue.clear());
    }
    // The tabs not shown hear from their threads too (an edit finishing, a page).
    let uids: Vec<u64> = app.hwp.docs.iter().filter(|(_, d)| d.shown != frame).map(|(uid, _)| *uid).collect();
    for uid in uids {
        if let Some(mut doc) = app.hwp.docs.remove(&uid) {
            receive(app, ctx, &mut doc);
            app.hwp.docs.insert(uid, doc);
        }
    }
    let dirty: Vec<(u64, bool)> = app.hwp.docs.iter().filter(|(_, d)| matches!(d.phase, Phase::Ready)).map(|(uid, d)| (*uid, d.dirty())).collect();
    for (uid, dirty) in dirty {
        let Some(d) = app.session.document_mut(uid) else { continue };
        if d.is_dirty() != dirty {
            d.saved = if dirty { Arc::new((*d.wb).clone()) } else { d.wb.clone() };
        }
    }
}

/// Starts reading the document of tab `key` on its own thread (a new document is made there too).
fn open(app: &mut SheetApp, ctx: &egui::Context, (uid, sheet): (u64, usize), path: Option<String>) -> Doc {
    render::warm();
    let spread = path.as_ref().and_then(|p| app.ui.doc_views.iter().find(|v| &v.path == p)).is_some_and(|v| v.spread);
    let restore = app.doc_view.restore.remove(&(uid, sheet));
    let title = app.session.documents().iter().find(|d| d.uid == uid).map(monun_engine::DocState::display_title).unwrap_or_default();
    // A recovered copy stays at hand until it is read, for AutoRecover to keep writing.
    let recovered = app.hwp.adopted.get(&uid).cloned();
    let source = match (recovered, path.clone()) {
        (Some(bytes), _) => Source::Bytes(bytes, true),
        (None, Some(p)) => Source::Path(p),
        (None, None) => Source::Blank,
    };
    let hwpx = !title.to_ascii_lowercase().ends_with(".hwp");
    let (phase, handle) = match Handle::start(uid, source, hwpx, ctx.clone()) {
        Ok(h) => (Phase::Opening, Some(h)),
        Err(e) => (Phase::Failed(e), None),
    };
    let mut doc = Doc::new(uid, phase, handle, spread, restore);
    doc.origin = path;
    doc.opened = Some(ctx.input(|i| i.time));
    doc
}

/// Takes what the document's thread has said: how it is, pages' SVG, what Copy took, Find's count…
fn receive(app: &mut SheetApp, ctx: &egui::Context, doc: &mut Doc) {
    let Some(handle) = &doc.handle else { return };
    let (replies, gone) = handle.poll();
    let uid = handle.uid;
    for reply in replies {
        match reply {
            Reply::Ready(state) => {
                app.hwp.adopted.remove(&uid);
                doc.phase = Phase::Ready;
                take_state(doc, state);
            }
            Reply::Failed(why) => doc.phase = Phase::Failed(why),
            Reply::State(state) => take_state(doc, state),
            Reply::Page { rev, page, key, svg } => {
                if doc.asked.get(&page) == Some(&rev) {
                    doc.asked.remove(&page);
                }
                if rev == doc.state.rev {
                    doc.svgs.insert(page, (rev, key, svg));
                }
            }
            Reply::Copied { text, html } => input::publish(app, ctx, &text, html.as_deref()),
            Reply::Found { count, index } => {
                doc.found = Some((count, index));
                doc.reveal = true;
            }
            Reply::Replaced(n) => {
                let lang = app.ui.language;
                app.toast = Some((lang.tr("Replaced {count}.").replace("{count}", &n.to_string()), crate::clock::now_ms()));
                doc.found = Some((0, None));
            }
            Reply::Outline(rev, items) => doc.contents = Some((rev, items)),
            Reply::Refused(why) => log::info!("한글 document: an edit was refused: {why}"),
        }
    }
    if gone && !matches!(doc.phase, Phase::Failed(_)) {
        doc.phase = Phase::Failed("stopped".into());
        doc.handle = None;
    }
    // Typed text the document now has no longer shows over it.
    let done = doc.state.done;
    doc.pending.retain(|(seq, _)| *seq > done);
}

/// Takes a new state of the document: pages from an older revision are asked for again.
fn take_state(doc: &mut Doc, state: DocState) {
    if state.rev != doc.state.rev {
        doc.asked.clear();
    }
    if state.caret != doc.state.caret {
        doc.reveal |= doc.state.done != state.done;
    }
    doc.state = state;
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
    match &doc.phase {
        Phase::Ready => pages(app, &mut body_ui, doc, key, (zoom, fit, editing)),
        Phase::Opening => note(&mut body_ui, lang.tr("Opening…"), &t),
        Phase::Failed(why) => {
            let text = match why.as_str() {
                "password" => lang.tr("This document is protected by a password. Monun can't open it.").to_owned(),
                "damaged" => lang.tr("This file isn't a document Monun can read: it may be damaged.").to_owned(),
                "stopped" => lang.tr("Monun stopped reading this document. Close it and open it again.").to_owned(),
                other => format!("{} {other}", lang.tr("The file couldn't be read:")),
            };
            note(&mut body_ui, &text, &t);
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
fn pages(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, key: (u64, usize), (zoom, fit, editing): (u16, bool, bool)) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let lang = app.ui.language;
    let (ppp, now) = (ctx.pixels_per_point(), ui.input(|i| i.time));
    let max_side = ctx.input(|i| i.max_texture_side) as f32;
    if app.hwp.renderer.is_none() {
        app.hwp.renderer = Renderer::start(ctx.clone());
    }
    receive_pages(app, doc, key.0, now);
    let sizes = doc.state.pages.clone();
    let width = (ui.available_width() - BAR_ROOM).max(40.0);
    let zoom = fit_width(app, doc, &sizes, key, (zoom, fit), width);
    let scale = f32::from(zoom) / 100.0 * POINTS_PER_PT;
    let lay = layout::lay_out(&sizes, scale, doc.spread, width);
    let jump = jump(doc, &lay, scale, now);
    let mut area = egui::ScrollArea::both().id_salt(("monun.hwp_view", key)).auto_shrink([false, false]);
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
        let caret = overlays(ui, doc, (scale, now), editing && ctx.memory(|m| m.has_focus(id)));
        let menu = input::pointer(ui, doc, (&lay, view.translate(origin), origin, scale), (id, editing, now, lang));
        doc.top = lay.top_at(view.top());
        doc.page = lay.most_shown(view.top(), view.bottom()).unwrap_or(0);
        doc.shown_pages = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
        (visible, view, caret, view.translate(origin), menu)
    });
    let (visible, view, caret, screen_view, menu) = shown.inner;
    match menu {
        Some(input::Menu::Copy) => {
            doc.send(Req::Copy { cut: false });
        }
        Some(input::Menu::Cut) => {
            doc.send(Req::Copy { cut: true });
            doc.ime.changed(&ctx, false);
        }
        Some(input::Menu::Paste) => {
            if let Some(text) = app.services.paste_text.as_mut().and_then(|read| read()) {
                input::paste(app, &ctx, doc, text);
            }
        }
        Some(input::Menu::SelectAll) => {
            doc.send(Req::SelectAll);
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
    if std::mem::take(&mut doc.replaced) {
        doc.ime.changed(&ctx, true);
    }
    doc.ime.observe(&ctx, caret.filter(|_| focused).map(|c| (screen_view, c)));
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
    let jobs = plan(doc, &lay, &visible, (key.0, scale_dev, max_side), settled);
    let ids: Vec<(u64, usize, u64, u32)> = jobs.iter().map(Job::id).collect();
    if ids != doc.queue || app.hwp.queued_for != Some(key.0) {
        app.hwp.queued_for = Some(key.0);
        if let Some(r) = &app.hwp.renderer {
            r.set_queue(jobs);
        }
        doc.queue = ids;
    }
}

/// Where the view goes this frame: a new zoom keeps the page at the top; a key or a page number
/// goes where it says; an edit keeps the caret in view.
fn jump(doc: &mut Doc, lay: &Layout, scale: f32, now: f64) -> Option<Vec2> {
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
        && let Some((page, [_, y, h])) = doc.state.caret
        && let Some(r) = lay.pages.get(page)
    {
        let (top, bottom) = (r.top() + y * scale, r.top() + (y + h) * scale);
        if top < doc.offset.y || bottom > doc.offset.y + doc.view.height() {
            doc.go = Some(Go::At(page, y, doc.view.height() / 3.0));
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
    let Some(r) = &app.hwp.renderer else { return };
    for _ in 0..PER_FRAME {
        let Some(done) = r.try_recv() else { return };
        let tex = Tex { key: done.key, scale: done.scale, size: done.size, texture: done.texture };
        if let Some(opened) = shown.opened.take_if(|_| done.uid == shown_uid) {
            log::info!("한글 document: its first page shows {:.0} ms after opening ({:.0} ms to draw)", (now - opened) * 1000.0, done.ms);
        }
        let target = if done.uid == shown_uid { Some(&mut *shown) } else { app.hwp.docs.get_mut(&done.uid) };
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

/// The selection over the pages, the caret (blinking, while typing goes to the document), and at
/// it the text typed but not in the document yet and the composition in progress. Returns the
/// caret's rectangle on the screen.
fn overlays(ui: &Ui, doc: &mut Doc, (scale, now): (f32, f64), typing: bool) -> Option<Rect> {
    let painter = ui.painter();
    let on_screen = |page: usize, x: f32, y: f32| -> Option<egui::Pos2> {
        doc.on_screen.iter().find(|(p, _)| *p == page).map(|(_, r)| pos2(r.left() + x * scale, r.top() + y * scale))
    };
    for (page, [x, y, w, h]) in &doc.state.selection {
        if let (Some(a), Some(b)) = (on_screen(*page, *x, *y), on_screen(*page, x + w, y + h)) {
            painter.rect_filled(Rect::from_min_max(a, b), 0.0, Color32::from_rgba_unmultiplied(0, 120, 215, 80));
        }
    }
    let (page, [x, y, h]) = doc.state.caret?;
    let top = on_screen(page, x, y)?;
    let mut rect = Rect::from_min_size(top, vec2(1.5, (h * scale).max(4.0)));
    let font = egui::FontId::proportional((h * scale * 0.72).clamp(8.0, 96.0));
    // Typed, not in the document yet: shown where it goes.
    if !doc.pending.is_empty() {
        let pending: String = doc.pending.iter().map(|(_, t)| t.as_str()).collect();
        let galley = painter.layout_no_wrap(pending, font.clone(), Color32::BLACK);
        let shown = Rect::from_min_size(rect.min, vec2(galley.size().x, rect.height()));
        painter.rect_filled(shown, 0.0, Color32::WHITE);
        painter.galley(pos2(shown.left(), shown.center().y - galley.size().y / 2.0), galley, Color32::BLACK);
        rect = rect.translate(vec2(shown.width(), 0.0));
    }
    if typing && !doc.ime.preedit.is_empty() {
        let galley = painter.layout_no_wrap(doc.ime.preedit.clone(), font, Color32::BLACK);
        let shown = Rect::from_min_size(rect.min, vec2(galley.size().x, rect.height()));
        painter.rect_filled(shown, 0.0, Color32::WHITE);
        painter.galley(pos2(shown.left(), shown.center().y - galley.size().y / 2.0), galley, Color32::BLACK);
        painter.line_segment([shown.left_bottom(), shown.right_bottom()], Stroke::new(1.0, Color32::BLACK));
    } else if typing && (now - doc.blink_from).rem_euclid(2.0 * BLINK) < BLINK {
        painter.rect_filled(rect, 0.0, Color32::BLACK);
    }
    Some(rect)
}

/// What the worker should draw, most needed first: the pages in view from the middle out (one
/// drawn at an older zoom waits until the zoom rests), then the pages ahead and behind while the
/// budget has room for them. Pages whose SVG at this revision isn't here yet are asked for.
fn plan(doc: &mut Doc, lay: &Layout, visible: &[usize], (uid, scale_dev, max_side): (u64, f32, f32), settled: bool) -> Vec<Job> {
    let sizes = doc.state.pages.clone();
    let n = sizes.len();
    let centre = doc.view.center().y;
    let middle = |p: &usize| lay.pages.get(*p).map_or(f32::MAX, |r| (r.center().y - centre).abs());
    let mut order = visible.to_vec();
    order.sort_by(|a, b| middle(a).total_cmp(&middle(b)));
    let (first, last) = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
    let after = (last + 1..n).take(if doc.direction >= 0 { AHEAD } else { BEHIND });
    let before = (0..first).rev().take(if doc.direction >= 0 { BEHIND } else { AHEAD });
    let (near, far): (Vec<usize>, Vec<usize>) =
        if doc.direction >= 0 { (after.collect(), before.collect()) } else { (before.collect(), after.collect()) };
    let want = |p: usize| sizes.get(p).map(|s| render::raster_scale(s[0], s[1], scale_dev, max_side));
    let bytes = |p: usize| {
        sizes.get(p).map_or(0, |s| {
            let k = render::raster_scale(s[0], s[1], scale_dev, max_side);
            ((s[0] * k).ceil() as usize).saturating_mul((s[1] * k).ceil() as usize).saturating_mul(4)
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
    let wanted: Vec<usize> = order.into_iter().chain(ahead).collect();
    let rev = doc.state.rev;
    let mut jobs = Vec::new();
    let mut ask = Vec::new();
    for &p in &wanted {
        let Some(want) = want(p) else { continue };
        let Some((_, key, svg)) = doc.svgs.get(&p).filter(|(r, _, _)| *r == rev) else {
            if doc.asked.get(&p) != Some(&rev) {
                ask.push(p);
            }
            continue;
        };
        let needed = match doc.textures.get(&p) {
            Some(t) if t.key == *key && render::same(t.scale, want) => false,
            Some(t) if t.key == *key && !settled => false,
            _ => true,
        };
        if needed {
            jobs.push(Job { uid, page: p, key: *key, scale: want, svg: svg.clone() });
        }
    }
    if !ask.is_empty() {
        for &p in &ask {
            doc.asked.insert(p, rev);
        }
        doc.send(Req::Pages(ask));
    }
    // Only the pages wanted keep their SVG (a page of pictures can take megabytes).
    doc.svgs.retain(|p, _| wanted.contains(p));
    jobs
}

/// Keeps the documents' pictures within [`render::BUDGET`]: the other tabs' go first, then the
/// active tab's pages farthest from its view.
fn keep_to_budget(app: &mut SheetApp, active: u64) {
    let size = |d: &Doc| d.textures.values().map(Tex::bytes).sum::<usize>();
    let mut total: usize = app.hwp.docs.values().map(size).sum();
    if total <= render::BUDGET {
        return;
    }
    let mut others: Vec<(u64, u64)> = app.hwp.docs.iter().filter(|(uid, _)| **uid != active).map(|(uid, d)| (d.shown, *uid)).collect();
    others.sort_unstable();
    for (_, uid) in others {
        if total <= render::BUDGET {
            return;
        }
        if let Some(d) = app.hwp.docs.get_mut(&uid) {
            total = total.saturating_sub(size(d));
            d.textures.clear();
        }
    }
    let Some(d) = app.hwp.docs.get_mut(&active) else { return };
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

/// Save, Save As, Export as PDF and Undo/Redo of a 한글 tab, done here rather than by the engine
/// (which knows the tab only as empty). `None` for any other tab or command. (Printing goes
/// through [`print`].)
pub(crate) fn command(app: &mut SheetApp, id: &str, params: &Json) -> Option<Result<Json, String>> {
    if !matches!(id, "file.save" | "file.saveAs" | "file.saveBytes" | "file.exportPdf" | "file.print" | "edit.undo" | "edit.redo") {
        return None;
    }
    let d = app.session.active().filter(|d| d.is_hwp())?;
    let uid = d.uid;
    let own_path = d.path.clone();
    let title = d.display_title();
    let path = params.get("path").and_then(Json::as_str).filter(|p| !p.is_empty()).map(str::to_owned);
    let lang = app.ui.language;
    let Some(doc) = app.hwp.docs.get_mut(&uid).filter(|d| matches!(d.phase, Phase::Ready)) else {
        return Some(Err(lang.tr("The document hasn't been read yet.").to_owned()));
    };
    Some(match id {
        "edit.undo" | "edit.redo" => {
            doc.send(if id == "edit.undo" { Req::Undo } else { Req::Redo });
            doc.reveal = true;
            doc.replaced = true;
            Ok(json!({}))
        }
        "file.saveBytes" => {
            let hwpx = !title.to_ascii_lowercase().ends_with(".hwp");
            match doc.handle.as_ref().map(|h| h.export(hwpx, SAVE_WAIT)) {
                Some(Ok(e)) => Ok(json!({"base64": monun_engine::io::base64_encode(&e.bytes)})),
                Some(Err(e)) => Err(e),
                None => Err(lang.tr("The document hasn't been read yet.").to_owned()),
            }
        }
        "file.print" => print(app).unwrap_or_else(|| Err(lang.tr("The document hasn't been read yet.").to_owned())),
        "file.exportPdf" => Err(lang.tr(NO_PDF).to_owned()),
        // Saving a document not changed leaves its file as it is.
        "file.save" if path.is_none() && own_path.is_some() && !doc.dirty() => Ok(json!({"path": own_path, "unchanged": true})),
        _ => match path.or(if id == "file.save" { own_path } else { None }) {
            None => {
                app.open_dialog("saveAs", json!({}));
                Ok(json!({}))
            }
            Some(path) if path.to_ascii_lowercase().ends_with(".pdf") => Err(lang.tr(NO_PDF).to_owned()),
            Some(path) => save(app, uid, &path),
        },
    })
}

/// Writes the document to `path` (.hwpx or .hwp, by its name) and makes it the tab's file. What
/// rhwp couldn't keep stops a save over the file the document came from (Save As can still keep
/// a copy).
fn save(app: &mut SheetApp, uid: u64, path: &str) -> Result<Json, String> {
    let lang = app.ui.language;
    let lower = path.to_ascii_lowercase();
    let hwpx = if lower.ends_with(".hwpx") {
        true
    } else if lower.ends_with(".hwp") {
        false
    } else {
        return Err(lang.tr("A Hangul document is saved as .hwpx or .hwp.").to_owned());
    };
    let doc = app.hwp.docs.get_mut(&uid).ok_or_else(|| "no document".to_owned())?;
    let handle = doc.handle.as_ref().ok_or_else(|| lang.tr("The document hasn't been read yet.").to_owned())?;
    let export = handle.export(hwpx, SAVE_WAIT)?;
    if !export.lost.is_empty() && doc.origin.as_deref().is_some_and(|o| same_file(o, path)) {
        return Err(format!(
            "{}\n\n{}",
            lang.tr("Saving would leave out parts of this document Monun can't keep, so its file was left as it was. Save a copy under another name (File › Save As) to keep your changes:"),
            export.lost.iter().take(8).map(|l| format!("• {l}")).collect::<Vec<_>>().join("\n")
        ));
    }
    monun_engine::io::write_file(path, &export.bytes).map_err(|e| e.to_string())?;
    // Everything sent before the save is in the file.
    doc.send(Req::Saved);
    doc.state.dirty = false;
    doc.edited = 0;
    if let Some(d) = app.session.document_mut(uid) {
        d.path = Some(path.to_owned());
        d.title = std::path::Path::new(path).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path).to_owned();
        d.disk = monun_engine::io::disk_stamp(path);
        d.saved = d.wb.clone();
    }
    if let Some(h) = doc.handle.as_mut() {
        h.hwpx = hwpx;
    }
    Ok(json!({"path": path, "bytes": export.bytes.len(), "lost": export.lost}))
}

/// Whether two paths name the same file (Windows: letters' case aside, either slash).
fn same_file(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// Starts printing the active 한글 document: its print PDF is made in the background and sent to
/// the printer when ready ([`SheetApp::submit_pdf`]). `None` for any other tab.
pub(crate) fn print(app: &mut SheetApp) -> Option<Result<Json, String>> {
    let uid = app.session.active().filter(|d| d.is_hwp())?.uid;
    Some(make_print(app, uid).map(|()| json!({"printing": true, "pending": true})))
}

/// Makes tab `uid`'s document a print PDF off the UI thread ([`pdf`]).
fn make_print(app: &mut SheetApp, uid: u64) -> Result<(), String> {
    let lang = app.ui.language;
    if app.hwp.making.is_some() {
        return Err(lang.tr("A print is already being prepared. Wait for it to finish.").to_owned());
    }
    let doc = app
        .hwp
        .docs
        .get_mut(&uid)
        .filter(|d| matches!(d.phase, Phase::Ready))
        .ok_or_else(|| lang.tr("The document hasn't been read yet.").to_owned())?;
    let (answer, svgs) = mpsc::channel();
    if doc.send(Req::PrintPages { answer }) == 0 {
        return Err(lang.tr("The document hasn't been read yet.").to_owned());
    }
    let tx = app.hwp.made.get_or_insert_with(mpsc::channel).0.clone();
    let (done, pages) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(doc.state.pages.len())));
    let making = Making { uid, done: done.clone(), pages: pages.clone() };
    std::thread::Builder::new()
        .name("monun-hwp-pdf".into())
        .spawn(move || {
            let started = crate::clock::monotonic_ms();
            // Whatever happens, the end is sent: the next print waits for it.
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match svgs.recv() {
                Ok(Ok(svgs)) => {
                    pages.store(svgs.len(), Ordering::Relaxed);
                    pdf::make(&svgs, &done)
                }
                Ok(Err(e)) => Err(e),
                Err(_) => Err("the document stopped".into()),
            }))
            .unwrap_or_else(|_| Err("the print stopped".into()));
            log::info!(
                "한글 document: a print of {} pages made in {:.1} s",
                pages.load(Ordering::Relaxed),
                (crate::clock::monotonic_ms() - started) / 1000.0
            );
            let _ = tx.send(made);
        })
        .map_err(|e| e.to_string())?;
    app.hwp.making = Some(making);
    Ok(())
}

/// Sends finished prints to the printer, or says why one couldn't be made.
fn finish_making(app: &mut SheetApp) {
    let Some((_, rx)) = &app.hwp.made else { return };
    let finished: Vec<Made> = rx.try_iter().collect();
    if finished.is_empty() {
        return;
    }
    app.hwp.making = None;
    for made in finished {
        if let Err(e) = made.and_then(|bytes| app.submit_pdf(bytes)) {
            app.alert(e);
        }
    }
}

/// Whether the active 한글 document shows two pages across, for remembering it.
pub fn spread(app: &SheetApp, uid: u64) -> Option<bool> {
    app.hwp.docs.get(&uid).map(|d| d.spread)
}

/// The status bar's words for the active 한글 document: the page shown and how many there are;
/// and how far a print being made is.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| d.is_hwp())?;
    let lang = app.ui.language;
    let doc = app.hwp.docs.get(&d.uid);
    let pages = doc.map_or(0, Doc::pages);
    let page = doc.map_or(0, |doc| doc.page);
    let mut words = if pages > 0 { format!("{} {} / {pages}", lang.tr("Page"), page + 1) } else { String::new() };
    if let Some(m) = app.hwp.making.as_ref().filter(|m| m.uid == d.uid) {
        let made = format!("{} {}/{}", lang.tr("Preparing to print"), m.done.load(Ordering::Relaxed), m.pages.load(Ordering::Relaxed));
        words = if words.is_empty() { made } else { format!("{words} · {made}") };
    }
    Some(words)
}

/// The active 한글 document as `ui.inspect` reports it (`null` for any other tab). With `text`,
/// its text too (asked of its thread).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| d.is_hwp()) else { return Json::Null };
    let Some(doc) = app.hwp.docs.get(&d.uid) else { return json!({"status": "opening"}) };
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
        "rev": doc.state.rev,
        "dirty": doc.state.dirty,
        "busy": doc.handle.as_ref().is_some_and(|h| h.sent() > doc.state.done),
        "selection": !doc.state.selection.is_empty(),
        "caret": doc.state.caret.map(|(p, [x, y, h])| json!([p, x, y, h])),
        "inTable": doc.state.in_table,
        "format": {
            "bold": doc.state.format.bold,
            "italic": doc.state.format.italic,
            "underline": doc.state.format.underline,
            "strike": doc.state.format.strike,
            "size": doc.state.format.size,
            "font": doc.state.format.font,
            "align": doc.state.format.align,
        },
        "headings": doc.contents.as_ref().map(|(_, h)| h.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>()),
        "printing": app.hwp.making.as_ref().map(|m| json!([m.done.load(Ordering::Relaxed), m.pages.load(Ordering::Relaxed)])),
    });
    out["status"] = json!(match &doc.phase {
        Phase::Opening => "opening".to_owned(),
        Phase::Ready => "ready".to_owned(),
        Phase::Failed(why) => format!("failed: {why}"),
    });
    if matches!(doc.phase, Phase::Ready)
        && let Some(h) = &doc.handle
    {
        out["text"] = json!(h.text(5.0));
    }
    out
}
