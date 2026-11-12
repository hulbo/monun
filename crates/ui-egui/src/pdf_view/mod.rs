//! PDF files in their document tab, read-only, laid out as a Markdown file in View mode is: the
//! tool bar, the find bar under it, the contents beside the pages. The engine only knows a PDF
//! as an empty document tab ([`monun_engine::docs::pdf_workbook`]); this reads the file and draws
//! it with PdfCraft's renderer (`pdfcraft-render`).
//!
//! Speed first. Opening, the file is read and its pages measured off the UI thread, so the tab
//! shows at once and its pages a moment later; the bookmarks are read after. Each PDF has a pool
//! of render threads, given every frame what to draw, most needed first: the pages in view from
//! the middle out, the next pages ahead, then the text of the pages in view (and while finding,
//! of every page). What no longer shows is cancelled mid-page. Pages are drawn at exactly the
//! screen's pixels (zoom × 96/72 × pixels per point) and placed on whole pixels, so they are
//! sharp; while the zoom changes the old pixels are stretched, and drawn anew once it rests.

mod outline;
mod render;
mod text;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

use egui::{Color32, CursorIcon, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use monun_engine::docs::TextKind;
use pdfcraft_render::{PageText, RenderConfig, RenderPool, RenderRequest, RequestKind};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::icons::Icon;
use crate::page_layout::{self as layout, GAP, Layout, POINTS_PER_PT};
use crate::theme::{self, Tokens};
use render::Raster;
use text::{Hits, Selection, Spot};

/// How long a new zoom rests before its pages are drawn anew (s); longer for a document whose
/// pages are slow to draw.
const HOLD: f64 = 0.12;
const SLOW_HOLD: f64 = 0.35;
/// A page that took this long to draw (ms) makes the document slow.
const SLOW_MS: u32 = 500;
/// Pages drawn ahead of the view in the direction it scrolls, and behind it.
const AHEAD: usize = 2;
const BEHIND: usize = 1;
/// Pages whose text layer is kept; the farthest from the view go first.
const TEXTS_KEPT: usize = 256;
/// Room left for the vertical scroll bar beside the pages (egui points).
const BAR_ROOM: f32 = 14.0;
/// One arrow key's scroll (egui points).
const LINE_STEP: f32 = 48.0;

/// The open PDFs, by workbook uid.
#[derive(Default)]
pub struct State {
    docs: HashMap<u64, Doc>,
    /// What the threads opening files send back.
    loads: Option<(mpsc::Sender<Loaded>, mpsc::Receiver<Loaded>)>,
}

/// What the thread opening a PDF sends back.
enum Loaded {
    Pages { uid: u64, bytes: Arc<Vec<u8>>, sizes: Vec<[f32; 2]> },
    Contents { uid: u64, items: Vec<outline::Item> },
    Failed { uid: u64, why: Why },
}

/// Why a PDF can't be shown.
#[derive(Clone, Debug, PartialEq)]
enum Why {
    Password,
    Damaged,
    Read(String),
    NoFile,
}

enum Phase {
    Opening,
    Ready(Box<Ready>),
    Failed(Why),
}

/// A PDF read: its pages' sizes (points, as shown) and its render threads.
struct Ready {
    sizes: Vec<[f32; 2]>,
    pool: RenderPool,
}

/// Where to scroll a PDF to next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Go {
    Top,
    Bottom,
    /// Down (or up) this far (egui points).
    By(f32),
    /// Down (or up) this many screens.
    Screens(f32),
    /// A page, this far down it (points), that far below the top of the view (egui points).
    At(usize, f32, f32),
}

/// One open PDF.
struct Doc {
    phase: Phase,
    /// Two pages across.
    spread: bool,
    rasters: HashMap<usize, Raster>,
    /// Pages that failed to draw, with the scale (its bits) they were asked at.
    failed: HashSet<(usize, u32)>,
    texts: HashMap<usize, Arc<PageText>>,
    hits: Hits,
    /// The current match of Find.
    hit: Option<Spot>,
    /// The bookmarks, once read.
    contents: Option<Vec<outline::Item>>,
    selection: Option<Selection>,
    /// A drag is selecting text.
    selecting: bool,
    /// What the render threads were last asked for.
    queue: Vec<RenderRequest>,
    /// Some page took long to draw.
    slow: bool,
    /// The scroll offset and the view (content coordinates) last frame.
    offset: Vec2,
    view: Rect,
    /// Which way the view last moved: down (1), up (-1).
    direction: i8,
    /// The page at the top of the view and how far down it, for a new zoom to keep there.
    top: Option<(usize, f32)>,
    /// The page the view shows most of (0-based), the pages in view.
    page: usize,
    shown_pages: (usize, usize),
    /// The pages in view and where they are on the screen, last frame.
    on_screen: Vec<(usize, Rect)>,
    /// Egui points per PDF point last frame, and when it last changed (s).
    scale: f32,
    scaled_at: f64,
    /// The zoom Fit Width last set, to tell one set any other way.
    fit_zoom: Option<u16>,
    /// The page to show first: where the file was being read last time.
    restore: Option<usize>,
    go: Option<Go>,
    /// The page number being typed in the tool bar.
    page_typed: Option<String>,
    /// The frame it last showed in (egui's count).
    shown: u64,
    /// When it was opened (s, egui's clock), until its first page is drawn: logged, to see how
    /// long opening takes.
    opened: Option<f64>,
}

impl Doc {
    fn new(phase: Phase, spread: bool, restore: Option<usize>) -> Doc {
        Doc {
            phase,
            spread,
            rasters: HashMap::new(),
            failed: HashSet::new(),
            texts: HashMap::new(),
            hits: Hits::default(),
            hit: None,
            contents: None,
            selection: None,
            selecting: false,
            queue: Vec::new(),
            slow: false,
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
            page_typed: None,
            shown: 0,
            opened: None,
        }
    }

    fn pages(&self) -> usize {
        match &self.phase {
            Phase::Ready(r) => r.sizes.len(),
            _ => 0,
        }
    }
}

/// Draws the active PDF's tab.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    receive_loads(app);
    let Some(d) = app.session.active() else { return };
    let key = (d.uid, d.wb.active_sheet());
    let path = d.path.clone();
    let (zoom, fit) = d.wb.active().map_or((100, true), |sh| (sh.zoom.clamp(10, 400), sh.document.as_ref().is_some_and(|doc| doc.fit_width)));
    let mut doc = match app.pdf.docs.remove(&key.0) {
        Some(doc) => doc,
        None => open(app, ui.ctx(), key, path),
    };
    doc.shown = ui.ctx().cumulative_frame_nr();
    draw(app, ui, &mut doc, key, (zoom, fit));
    app.pdf.docs.insert(key.0, doc);
    keep_to_budget(app, key.0);
}

/// Forgets the PDFs closed, and stops drawing for those not shown: every frame, after the window
/// is drawn.
pub(crate) fn tidy(app: &mut SheetApp, ctx: &egui::Context) {
    if app.pdf.docs.is_empty() {
        return;
    }
    let open: HashSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.pdf.docs.retain(|uid, _| open.contains(uid));
    let frame = ctx.cumulative_frame_nr();
    for doc in app.pdf.docs.values_mut() {
        if doc.shown != frame && !doc.queue.is_empty() {
            doc.queue.clear();
            if let Phase::Ready(r) = &doc.phase {
                r.pool.set_queue(Vec::new());
            }
        }
    }
}

/// Starts reading the PDF of tab `key` (workbook uid, sheet), off the UI thread.
fn open(app: &mut SheetApp, ctx: &egui::Context, (uid, sheet): (u64, usize), path: Option<String>) -> Doc {
    let spread = path.as_ref().and_then(|p| app.ui.doc_views.iter().find(|v| &v.path == p)).is_some_and(|v| v.spread);
    let restore = app.doc_view.restore.remove(&(uid, sheet));
    let phase = match path {
        Some(path) => {
            let tx = app.pdf.loads.get_or_insert_with(mpsc::channel).0.clone();
            read(uid, path, tx, ctx.clone())
        }
        None => Phase::Failed(Why::NoFile),
    };
    let mut doc = Doc::new(phase, spread, restore);
    doc.opened = Some(ctx.input(|i| i.time));
    doc
}

/// Reads the file at `path` on a thread of its own: its pages' sizes, then its bookmarks.
fn read(uid: u64, path: String, tx: mpsc::Sender<Loaded>, ctx: egui::Context) -> Phase {
    let started = std::thread::Builder::new().name("monun-pdf-open".into()).spawn(move || {
        let send = |m: Loaded| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => Arc::new(bytes),
            Err(e) => return send(Loaded::Failed { uid, why: Why::Read(e.to_string()) }),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page_sizes(&bytes))) {
            Ok(Ok(sizes)) => send(Loaded::Pages { uid, bytes: bytes.clone(), sizes }),
            Ok(Err(why)) => return send(Loaded::Failed { uid, why }),
            Err(_) => return send(Loaded::Failed { uid, why: Why::Damaged }),
        }
        let info = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfcraft_render::inspect(bytes, None)));
        let items = match info {
            Ok(Ok(info)) => outline::items(&info),
            _ => Vec::new(),
        };
        send(Loaded::Contents { uid, items });
    });
    match started {
        Ok(_) => Phase::Opening,
        Err(e) => Phase::Failed(Why::Read(e.to_string())),
    }
}

/// The size of each page of the PDF `bytes` (points, rotation applied).
fn page_sizes(bytes: &Arc<Vec<u8>>) -> Result<Vec<[f32; 2]>, Why> {
    use hayro::hayro_syntax::{LoadPdfError, Pdf};
    let pdf = Pdf::new(bytes.clone()).map_err(|e| match e {
        LoadPdfError::Decryption(_) => Why::Password,
        _ => Why::Damaged,
    })?;
    Ok(pdf.pages().iter().map(|p| p.render_dimensions()).map(|(w, h)| [w, h]).collect())
}

/// Takes what the opening threads have sent.
fn receive_loads(app: &mut SheetApp) {
    let Some((_, rx)) = &app.pdf.loads else { return };
    let loaded: Vec<Loaded> = rx.try_iter().collect();
    for l in loaded {
        match l {
            Loaded::Pages { uid, bytes, sizes } => {
                if let Some(doc) = app.pdf.docs.get_mut(&uid) {
                    let threads = std::thread::available_parallelism().map_or(2, std::num::NonZero::get).clamp(2, 8) - 1;
                    // Installed Korean, Chinese and Japanese faces stand in for fonts the file doesn't
                    // embed (on screen only, which is all this is).
                    let pool = RenderPool::new(bytes, threads, RenderConfig::default().on_screen());
                    doc.phase = Phase::Ready(Box::new(Ready { sizes, pool }));
                }
            }
            Loaded::Contents { uid, items } => {
                if let Some(doc) = app.pdf.docs.get_mut(&uid) {
                    doc.contents = Some(items);
                }
            }
            Loaded::Failed { uid, why } => {
                if let Some(doc) = app.pdf.docs.get_mut(&uid) {
                    doc.phase = Phase::Failed(why);
                }
            }
        }
    }
}

/// The tab: the tool bar, the find bar, the contents and the pages.
fn draw(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, key: (u64, usize), (zoom, fit): (u16, bool)) {
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    ui.painter().rect_filled(ui.max_rect(), 0.0, backdrop(&t));
    // Save, Open, New, Close and Find as in a Markdown file's tab.
    doc_view::keyboard(app, ui, Mode::View, TextKind::Pdf);
    keys(app, ui.ctx(), doc);
    doc_view::tool_strip(ui, &t, |ui| toolbar(app, ui, doc, key, fit, &t));
    if app.doc_view.find.is_some() {
        find_bar(app, ui, doc, &t);
    }
    let area = ui.available_rect_before_wrap();
    let body = if app.ui.doc_outline {
        let side = Rect::from_min_max(area.min, pos2(area.left() + doc_view::contents_width(area), area.bottom()));
        contents(app, ui, doc, side, &t);
        Rect::from_min_max(pos2(side.right(), area.top()), area.max)
    } else {
        area
    };
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    match std::mem::replace(&mut doc.phase, Phase::Opening) {
        Phase::Ready(ready) => {
            pages(app, &mut body_ui, doc, &ready, key, (zoom, fit));
            doc.phase = Phase::Ready(ready);
        }
        Phase::Opening => note(&mut body_ui, lang.tr("Opening…"), &t),
        Phase::Failed(why) => {
            note(&mut body_ui, &why_text(&why, lang), &t);
            doc.phase = Phase::Failed(why);
        }
    }
    ui.allocate_rect(area, Sense::hover());
    // Ctrl+wheel zooms, in the steps and with the glide a Markdown file's has.
    let (zoom_delta, now) = ui.input(|i| (i.zoom_delta(), i.time));
    if zoom_delta != 1.0 && ui.rect_contains_pointer(area) {
        crate::grid::turn_wheel_zoom(app, ui.ctx(), key, zoom, zoom_delta, now);
    }
    crate::grid::update_zoom_glide(app, ui, key, zoom, now);
    // Where it is being read, for the next time it opens.
    if let Some((page, _)) = doc.top {
        app.doc_view.tops.insert(key, page);
    }
}

/// The colour around the pages.
fn backdrop(t: &Tokens) -> Color32 {
    if t.dark { t.window } else { t.ribbon }
}

/// A line of text in the middle of the tab, instead of pages.
fn note(ui: &mut Ui, text: &str, t: &Tokens) {
    let rect = ui.max_rect();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::ui_font(14.0), t.text_dim);
}

/// Why a PDF can't be shown, in words.
fn why_text(why: &Why, lang: crate::i18n::Language) -> String {
    match why {
        Why::Password => lang.tr("This PDF is protected by a password. Monun can't open it.").to_owned(),
        Why::Damaged => lang.tr("This file isn't a PDF Monun can read: it may be damaged.").to_owned(),
        Why::Read(e) => format!("{} {e}", lang.tr("The file couldn't be read:")),
        Why::NoFile => lang.tr("This PDF has no file to read.").to_owned(),
    }
}

/// The keys of the pages: Ctrl+Home and Ctrl+End (and Home and End), Page Up and Page Down (and
/// Space), the arrows; Ctrl+A selects the page's text, Ctrl+C copies the selection, Esc drops it.
/// Not while a field (the page number, Find) has the keyboard, or a dialog shows.
fn keys(app: &SheetApp, ctx: &egui::Context, doc: &mut Doc) {
    if ctx.text_edit_focused() || app.dialog.is_some() || app.message.is_some() {
        return;
    }
    let (cmd, none, shift) = (Modifiers::COMMAND, Modifiers::NONE, Modifiers::SHIFT);
    let (go, all, copy, escape) = ctx.input_mut(|i| {
        let go = if i.consume_key(cmd, Key::Home) || i.consume_key(none, Key::Home) {
            Some(Go::Top)
        } else if i.consume_key(cmd, Key::End) || i.consume_key(none, Key::End) {
            Some(Go::Bottom)
        } else if i.consume_key(none, Key::PageDown) || i.consume_key(none, Key::Space) {
            Some(Go::Screens(1.0))
        } else if i.consume_key(none, Key::PageUp) || i.consume_key(shift, Key::Space) {
            Some(Go::Screens(-1.0))
        } else if i.consume_key(none, Key::ArrowDown) {
            Some(Go::By(LINE_STEP))
        } else if i.consume_key(none, Key::ArrowUp) {
            Some(Go::By(-LINE_STEP))
        } else {
            None
        };
        let all = i.consume_key(cmd, Key::A);
        let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
        let escape = i.consume_key(none, Key::Escape);
        (go, all, copy, escape)
    });
    if go.is_some() {
        doc.go = go;
    }
    if all && let Some(text) = doc.texts.get(&doc.page) {
        doc.selection = Some(Selection::page(doc.page, text.glyphs.len()));
    }
    if copy {
        copy_selection(ctx, doc);
    }
    if escape {
        doc.selection = None;
    }
}

/// Puts the selected text on the clipboard.
fn copy_selection(ctx: &egui::Context, doc: &Doc) {
    let text = doc.selection.map(|s| s.text(&doc.texts)).unwrap_or_default();
    if !text.is_empty() {
        ctx.copy_text(text);
    }
}

/// The tool bar: Contents, Fit Width and Find as a Markdown file has them, One Page and Two Pages,
/// and the page shown, which takes a page number to go to.
fn toolbar(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, (_, sheet): (u64, usize), fit: bool, t: &Tokens) {
    let lang = app.ui.language;
    let button = |ui: &mut Ui, on: bool, label: &str, tip: &str| {
        ui.add(egui::Button::selectable(on, egui::RichText::new(lang.tr(label)).font(theme::ui_font(13.0))).min_size(vec2(28.0, 26.0)))
            .on_hover_text(lang.tr(tip))
            .clicked()
    };
    let outline = app.ui.doc_outline;
    if button(ui, outline, "Contents", "Show the PDF's bookmarks beside its pages") {
        app.ui.doc_outline = !outline;
    }
    if button(ui, fit, "Fit Width", "Make the pages as wide as the window") {
        app.run_or_alert("doc.view", json!({"sheet": sheet, "fitWidth": !fit}));
    }
    doc_view::find_button(app, ui);
    doc_view::separator(ui, t);
    for (two, label, tip) in [(false, "One Page", "One page across"), (true, "Two Pages", "Two pages side by side")] {
        if button(ui, doc.spread == two, label, tip) && doc.spread != two {
            doc.spread = two;
            // The page read stays at the top.
            doc.go = Some(Go::At(doc.top.map_or(doc.page, |(p, _)| p), 0.0, GAP));
        }
    }
    doc_view::separator(ui, t);
    let pages = doc.pages();
    ui.label(egui::RichText::new(lang.tr("Page")).font(theme::ui_font(13.0)));
    ui.add_space(4.0);
    let mut typed = doc.page_typed.clone().unwrap_or_else(|| if pages == 0 { String::new() } else { (doc.page + 1).to_string() });
    let field = ui.add(
        egui::TextEdit::singleline(&mut typed)
            .id(egui::Id::unique("monun.pdf_page").with(sheet))
            .desired_width(44.0)
            .align(egui::Align2::CENTER_CENTER)
            .font(theme::ui_font(13.0)),
    );
    let field = field.on_hover_text(lang.tr("Type a page number and press Enter to go to it"));
    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(lang.tr("Page")));
    if field.has_focus() {
        doc.page_typed = Some(typed.clone());
    }
    if field.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Enter))
            && let Ok(n) = typed.trim().parse::<usize>()
            && pages > 0
        {
            doc.go = Some(Go::At(n.clamp(1, pages) - 1, 0.0, GAP));
        }
        doc.page_typed = None;
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("/ {pages}")).font(theme::ui_font(13.0)).color(t.text_dim));
}

/// The find bar: as a Markdown file's, without Replace. The count grows while pages are being
/// read ("…").
fn find_bar(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, t: &Tokens) {
    let lang = app.ui.language;
    let mut child = doc_view::find_strip(ui, t);
    let ui = &mut child;
    let Some(find) = app.doc_view.find.as_mut() else { return };
    ui.label(egui::RichText::new(lang.tr("Find:")).font(theme::ui_font(13.0)));
    let q = ui.add(
        egui::TextEdit::singleline(&mut find.query).id(egui::Id::unique("monun.doc_find")).desired_width(200.0).hint_text(lang.tr("Text to find")),
    );
    if std::mem::take(&mut find.focus) {
        q.request_focus();
    }
    let query = find.query.clone();
    doc.hits.update(&query, &doc.texts);
    if q.changed() {
        doc.hit = None;
    }
    let enter = q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let back = ui.input(|i| i.modifiers.shift);
    let total = doc.hits.total();
    let searching = matches!(doc.phase, Phase::Opening) || doc.hits.pages_searched() < doc.pages();
    let count = if query.trim().is_empty() {
        String::new()
    } else if total == 0 {
        lang.tr(if searching { "Searching…" } else { "No matches" }).to_owned()
    } else {
        let at = doc.hit.and_then(|h| doc.hits.index_of(h)).map_or("-".to_owned(), |i| (i + 1).to_string());
        format!("{at}/{total}{}", if searching { "…" } else { "" })
    };
    ui.label(egui::RichText::new(count).font(theme::ui_font(12.5)).color(t.text_dim));
    let icon = |ui: &mut Ui, icon: Icon, tip: &str| {
        let resp = crate::widgets::icon_button(ui, icon, t.text_dim, lang.tr(tip), vec2(24.0, 24.0));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr(tip)));
        resp.clicked()
    };
    let mut step = None;
    if icon(ui, Icon::ChevronUp, "Previous (Shift+Enter)") {
        step = Some(false);
    }
    if icon(ui, Icon::Chevron, "Next (Enter)") {
        step = Some(true);
    }
    if enter {
        step = Some(!back);
        find.focus = true;
    }
    ui.add_space(8.0);
    let close = icon(ui, Icon::Close, "Close (Esc)") || (q.has_focus() || q.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape));
    if close {
        app.doc_view.find = None;
        doc.hit = None;
        return;
    }
    if let Some(forward) = step
        && let Some(hit) = doc.hits.step(doc.hit, forward, doc.page)
    {
        doc.hit = Some(hit);
        // The match a third of the way down the view.
        let top = doc.texts.get(&hit.page).and_then(|text| {
            let range = doc.hits.on_page(hit.page).iter().find(|r| r.start == hit.at)?.clone();
            text.line_rects(range).first().map(|r| r[1])
        });
        doc.go = Some(Go::At(hit.page, top.unwrap_or(0.0), doc.view.height() / 3.0));
    }
}

/// The contents beside the pages: the PDF's bookmarks, as a Markdown file's headings are.
fn contents(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, rect: Rect, t: &Tokens) {
    let lang = app.ui.language;
    let chosen = {
        let (items, empty) = match &doc.contents {
            Some(items) => (items.as_slice(), lang.tr("This PDF has no bookmarks.")),
            None => (&[][..], lang.tr("Reading the bookmarks…")),
        };
        let entries: Vec<doc_view::Entry<'_>> =
            items.iter().map(|i| doc_view::Entry { indent: i.depth, title: &i.title, bold: i.depth == 0 || i.bold }).collect();
        doc_view::contents_panel(ui, rect, t, lang.tr("Contents"), empty, &entries)
    };
    let target = chosen.and_then(|i| doc.contents.as_ref()?.get(i)).and_then(|item| Some((item.page?, item.top)));
    if let Some((page, top)) = target {
        doc.go = Some(match top {
            Some(y) => Go::At(page, y, 8.0),
            None => Go::At(page, 0.0, GAP),
        });
    }
}

/// The pages: laid out at the zoom (or as wide as the window, with Fit Width), scrolled, drawn,
/// selected; then the render threads told what to draw next.
fn pages(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, ready: &Ready, key: (u64, usize), (zoom, fit): (u16, bool)) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let lang = app.ui.language;
    let (ppp, now) = (ctx.pixels_per_point(), ui.input(|i| i.time));
    let max_side = ctx.input(|i| i.max_texture_side) as f32;
    if receive_pages(&ctx, key.0, doc, ready) {
        ctx.request_repaint();
    }
    // Text layers just read are searched at once, before any is let go (`keep_texts`).
    let query = app.doc_view.find.as_ref().map(|f| f.query.clone()).filter(|q| !q.trim().is_empty());
    if let Some(query) = &query {
        let searched = doc.hits.pages_searched();
        doc.hits.update(query, &doc.texts);
        // The find bar, drawn above, shows the new count next frame.
        if doc.hits.pages_searched() != searched {
            ctx.request_repaint();
        }
    }
    let finding = query.is_some();
    let width = (ui.available_width() - BAR_ROOM).max(40.0);
    let zoom = fit_width(app, doc, ready, key, (zoom, fit), width);
    let scale = f32::from(zoom) / 100.0 * POINTS_PER_PT;
    let lay = layout::lay_out(&ready.sizes, scale, doc.spread, width);
    let mut jump = None;
    // A new zoom keeps the page at the top where it was.
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
    let mut area = egui::ScrollArea::both().id_salt(("monun.pdf_view", key)).auto_shrink([false, false]);
    if let Some(j) = jump {
        area = area.horizontal_scroll_offset(j.x.max(0.0)).vertical_scroll_offset(j.y.max(0.0));
    }
    let scale_dev = scale * ppp;
    let shown = area.show_viewport(ui, |ui, view| {
        ui.set_min_size(lay.size);
        let origin = ui.max_rect().min.to_vec2();
        let visible: Vec<usize> = lay.visible(view.top(), view.bottom()).collect();
        doc.on_screen.clear();
        for &p in &visible {
            let Some(slot) = lay.pages.get(p).map(|r| r.translate(origin)) else { continue };
            doc.on_screen.push((p, slot));
            let want = ready.sizes.get(p).map_or(scale_dev, |s| render::raster_scale(*s, scale_dev, max_side));
            paint_page(ui, doc, p, slot, (want, scale, ppp), (finding, &t));
        }
        select(ui, doc, &lay, (view.translate(origin), origin), scale, lang);
        doc.top = lay.top_at(view.top());
        doc.page = lay.most_shown(view.top(), view.bottom()).unwrap_or(0);
        doc.shown_pages = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
        (visible, view)
    });
    let (visible, view) = shown.inner;
    let offset = shown.state.clamped_offset();
    if offset.y != doc.offset.y {
        doc.direction = if offset.y > doc.offset.y { 1 } else { -1 };
    }
    doc.offset = offset;
    doc.view = view;
    let hold = if doc.slow { SLOW_HOLD } else { HOLD };
    let settled = now - doc.scaled_at >= hold;
    if !settled {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(hold - (now - doc.scaled_at)));
    }
    let queue = plan(doc, ready, &lay, &visible, (scale_dev, max_side), settled, finding);
    if queue != doc.queue {
        ready.pool.set_queue(queue.clone());
        doc.queue = queue;
    }
    if !doc.queue.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    keep_texts(doc, finding);
}

/// With Fit Width, the zoom that makes the pages as wide as `width`, set as the tab's zoom (so the
/// status bar shows it and it is remembered); a zoom set any other way since ends Fit Width.
/// Returns the zoom to draw at.
fn fit_width(app: &mut SheetApp, doc: &mut Doc, ready: &Ready, (_, sheet): (u64, usize), (zoom, fit): (u16, bool), width: f32) -> u16 {
    if !fit {
        doc.fit_zoom = None;
        return zoom;
    }
    if doc.fit_zoom.is_some_and(|z| z != zoom) {
        doc.fit_zoom = None;
        let _ = app.session.run("doc.view", json!({"sheet": sheet, "fitWidth": false}));
        return zoom;
    }
    let Some(s) = layout::fit_scale(&ready.sizes, doc.spread, width) else { return zoom };
    let fitted = (s / POINTS_PER_PT * 100.0).floor().clamp(10.0, 400.0) as u16;
    if fitted != zoom {
        let _ = app.session.run("view.zoom", json!({"percent": fitted}));
    }
    doc.fit_zoom = Some(fitted);
    fitted
}

/// Takes the render threads' answers, a few a frame: pages become textures, text layers are kept.
/// Returns whether more may be waiting.
fn receive_pages(ctx: &egui::Context, uid: u64, doc: &mut Doc, ready: &Ready) -> bool {
    let mut bytes = 0;
    for _ in 0..render::PER_FRAME {
        if bytes >= render::BYTES_PER_FRAME {
            break;
        }
        let Some(answer) = ready.pool.try_recv() else { return false };
        let page = answer.request.page;
        if answer.request.kind == RequestKind::Text {
            doc.texts.insert(page, answer.text.clone().unwrap_or_default());
            continue;
        }
        doc.slow |= answer.millis >= SLOW_MS;
        if answer.millis >= SLOW_MS {
            log::info!("PDF page {} took {} ms to draw", page + 1, answer.millis);
        }
        if let Some(opened) = doc.opened.take_if(|_| answer.error.is_none()) {
            let ms = (ctx.input(|i| i.time) - opened) * 1000.0;
            log::info!("PDF of {} pages: its first page shows {ms:.0} ms after opening ({} ms to draw)", ready.sizes.len(), answer.millis);
        }
        if answer.error.is_some() {
            doc.failed.insert((page, answer.request.scale.to_bits()));
        } else if let Some(raster) = render::texture(ctx, format!("monun.pdf.{uid}.{page}"), answer) {
            bytes += raster.bytes();
            doc.rasters.insert(page, raster);
        }
    }
    true
}

/// Draws page `p` into `slot`: white paper, the page's pixels (on whole pixels when drawn at
/// `want`, stretched while a new zoom waits), the matches of Find (while `finding`) and the
/// selection.
fn paint_page(ui: &Ui, doc: &Doc, p: usize, slot: Rect, (want, scale, ppp): (f32, f32, f32), (finding, t): (bool, &Tokens)) {
    let painter = ui.painter();
    painter.rect_filled(slot.translate(vec2(0.0, 1.5)), 0.0, Color32::from_black_alpha(if t.dark { 90 } else { 30 }));
    painter.rect_filled(slot, 0.0, Color32::WHITE);
    if let Some(r) = doc.rasters.get(&p) {
        let rect = if render::same(r.scale, want) {
            let min = pos2((slot.min.x * ppp).round() / ppp, (slot.min.y * ppp).round() / ppp);
            Rect::from_min_size(min, vec2(r.size[0] as f32 / ppp, r.size[1] as f32 / ppp))
        } else {
            slot
        };
        painter.image(r.texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    painter.rect_stroke(slot, 0.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Outside);
    let Some(text) = doc.texts.get(&p) else { return };
    let on_screen = |r: [f32; 4]| Rect::from_min_max(slot.min + vec2(r[0], r[1]) * scale, slot.min + vec2(r[2], r[3]) * scale);
    for hit in doc.hits.on_page(p).iter().filter(|_| finding) {
        let current = doc.hit == Some(Spot { page: p, at: hit.start });
        let ink = if current { Color32::from_rgba_unmultiplied(255, 140, 0, 130) } else { Color32::from_rgba_unmultiplied(255, 214, 0, 100) };
        for r in text.line_rects(hit.clone()) {
            painter.rect_filled(on_screen(r), 1.0, ink);
        }
    }
    if let Some(range) = doc.selection.and_then(|s| s.on_page(p, text.glyphs.len())) {
        for r in text.line_rects(range) {
            painter.rect_filled(on_screen(r), 0.0, Color32::from_rgba_unmultiplied(0, 120, 215, 80));
        }
    }
}

/// Selecting text with the pointer: a drag from one glyph boundary to another (over pages too,
/// the view scrolling at its edges), a double click a word, a triple click a line; a click
/// elsewhere drops it. Over text the pointer is an I-beam; a right click offers Copy.
fn select(ui: &mut Ui, doc: &mut Doc, lay: &Layout, (view, origin): (Rect, Vec2), scale: f32, lang: crate::i18n::Language) {
    let resp = ui.interact(view, ui.scope_id().with("monun.pdf_pages"), Sense::click_and_drag());
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    // The page under `pos` (or nearest it) and the point on it (view-space points).
    let on_page = |pos: Pos2, nearest: bool| {
        let c = pos - origin;
        let page = lay.page_at(c).or_else(|| nearest.then(|| lay.nearest_page(c)).flatten())?;
        let r = lay.pages.get(page)?;
        Some((page, (c.x - r.left()) / scale, (c.y - r.top()) / scale))
    };
    let caret = |doc: &Doc, pos: Pos2, nearest: bool| {
        let (page, x, y) = on_page(pos, nearest)?;
        text::caret_at(doc.texts.get(&page)?, x, y).map(|at| Spot { page, at })
    };
    let glyph = |doc: &Doc, pos: Pos2| {
        let (page, x, y) = on_page(pos, false)?;
        Some((page, doc.texts.get(&page)?.nearest(x, y)?))
    };
    if let Some(pos) = resp.hover_pos()
        && let Some((page, x, y)) = on_page(pos, false)
        && doc.texts.get(&page).is_some_and(|text| text.glyph_at(x, y).is_some())
    {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
        let start = pointer.and_then(|pos| caret(doc, pos, false));
        doc.selection = start.map(|s| Selection { anchor: s, head: s });
        doc.selecting = start.is_some();
    }
    if doc.selecting
        && let Some(pos) = pointer
    {
        if let Some(head) = caret(doc, pos, true)
            && let Some(sel) = doc.selection.as_mut()
        {
            sel.head = head;
        }
        // Past the view's top or bottom edge, the view follows.
        let edge = 24.0;
        let dy = if pos.y > view.bottom() - edge {
            -(pos.y - (view.bottom() - edge))
        } else if pos.y < view.top() + edge {
            view.top() + edge - pos.y
        } else {
            0.0
        };
        if dy != 0.0 {
            ui.scroll_with_delta(vec2(0.0, dy.clamp(-60.0, 60.0)));
            ui.ctx().request_repaint();
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        doc.selecting = false;
    }
    if (resp.double_clicked() || resp.triple_clicked())
        && let Some((page, g)) = pointer.and_then(|pos| glyph(doc, pos))
        && let Some(text) = doc.texts.get(&page)
    {
        let span = if resp.triple_clicked() { text.line_at(g) } else { text.word_at(g) };
        doc.selection = span.map(|span| Selection::glyphs(page, span));
    }
    if doc.selection.is_some_and(|s| s.is_empty()) && !doc.selecting {
        doc.selection = None;
    }
    let ctx = ui.ctx().clone();
    resp.context_menu(|ui| {
        if ui.add_enabled(doc.selection.is_some(), egui::Button::new(lang.tr("Copy"))).clicked() {
            copy_selection(&ctx, doc);
            ui.close();
        }
        if ui.button(lang.tr("Select All")).clicked() {
            if let Some(text) = doc.texts.get(&doc.page) {
                doc.selection = Some(Selection::page(doc.page, text.glyphs.len()));
            }
            ui.close();
        }
    });
}

/// What the render threads should draw, most needed first: the pages in view from the middle out
/// (a page drawn at an older zoom waits until the zoom rests), the pages ahead and behind, the
/// text of the pages in view, and while finding, the text of every page not searched yet, from
/// the page shown on.
fn plan(
    doc: &Doc,
    ready: &Ready,
    lay: &Layout,
    visible: &[usize],
    (scale_dev, max_side): (f32, f32),
    settled: bool,
    finding: bool,
) -> Vec<RenderRequest> {
    let n = ready.sizes.len();
    let centre = doc.view.center().y;
    let middle = |p: &usize| lay.pages.get(*p).map_or(f32::MAX, |r| (r.center().y - centre).abs());
    let mut order = visible.to_vec();
    order.sort_by(|a, b| middle(a).total_cmp(&middle(b)));
    let need = |p: usize| -> Option<RenderRequest> {
        let want = render::raster_scale(*ready.sizes.get(p)?, scale_dev, max_side);
        if doc.failed.contains(&(p, want.to_bits())) {
            return None;
        }
        match doc.rasters.get(&p) {
            Some(r) if render::same(r.scale, want) => None,
            Some(_) if !settled => None,
            _ => Some(render::pixels(p, want)),
        }
    };
    let mut queue: Vec<RenderRequest> = order.iter().filter_map(|&p| need(p)).collect();
    // The pages ahead and behind only while the pages in view leave them room in the budget, so
    // a large zoom never draws a page only to let it go again.
    let bytes = |p: usize| ready.sizes.get(p).map_or(0, |s| render::raster_bytes(*s, render::raster_scale(*s, scale_dev, max_side)));
    let mut room = render::BUDGET.saturating_sub(visible.iter().map(|&p| bytes(p)).sum());
    let mut fits = |p: &usize| {
        let b = bytes(*p);
        let fits = b <= room;
        room = room.saturating_sub(b);
        fits
    };
    let (first, last) = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
    let after = (last + 1..n).take(if doc.direction >= 0 { AHEAD } else { BEHIND });
    let before = (0..first).rev().take(if doc.direction >= 0 { BEHIND } else { AHEAD });
    let (near, far): (Vec<usize>, Vec<usize>) =
        if doc.direction >= 0 { (after.collect(), before.collect()) } else { (before.collect(), after.collect()) };
    queue.extend(near.into_iter().chain(far).take_while(|p| fits(p)).filter_map(need));
    queue.extend(order.iter().filter(|p| !doc.texts.contains_key(p)).map(|&p| render::text(p)));
    if finding && n > 0 {
        let wanted: HashSet<usize> = queue.iter().filter(|r| r.kind == RequestKind::Text).map(|r| r.page).collect();
        let rest = (0..n).map(|k| (doc.page + k) % n).filter(|p| !doc.hits.searched(*p) && !doc.texts.contains_key(p) && !wanted.contains(p));
        queue.extend(rest.map(render::text));
    }
    queue
}

/// Keeps the text layers of at most [`TEXTS_KEPT`] pages: those farthest from the view go (their
/// matches stay). While finding, a page not searched yet stays until it is.
fn keep_texts(doc: &mut Doc, finding: bool) {
    if doc.texts.len() <= TEXTS_KEPT {
        return;
    }
    let (first, last) = doc.shown_pages;
    let selected = doc.selection.map(|s| (s.anchor.page.min(s.head.page), s.anchor.page.max(s.head.page)));
    let mut pages: Vec<usize> = doc.texts.keys().copied().collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(doc.page)));
    let extra = doc.texts.len() - TEXTS_KEPT;
    let kept = |p: &usize| (first..=last).contains(p) || selected.is_some_and(|(a, b)| (a..=b).contains(p)) || (finding && !doc.hits.searched(*p));
    let gone: Vec<usize> = pages.into_iter().filter(|p| !kept(p)).take(extra).collect();
    for p in gone {
        doc.texts.remove(&p);
    }
}

/// Keeps the PDFs' textures within [`render::BUDGET`]: the other tabs' go first, the longest
/// unseen first, then the active tab's pages farthest from its view.
fn keep_to_budget(app: &mut SheetApp, active: u64) {
    let size = |d: &Doc| d.rasters.values().map(Raster::bytes).sum::<usize>();
    let mut total: usize = app.pdf.docs.values().map(size).sum();
    if total <= render::BUDGET {
        return;
    }
    let mut others: Vec<(u64, u64)> = app.pdf.docs.iter().filter(|(uid, _)| **uid != active).map(|(uid, d)| (d.shown, *uid)).collect();
    others.sort_unstable();
    for (_, uid) in others {
        if total <= render::BUDGET {
            return;
        }
        if let Some(d) = app.pdf.docs.get_mut(&uid) {
            total = total.saturating_sub(size(d));
            d.rasters.clear();
        }
    }
    let Some(d) = app.pdf.docs.get_mut(&active) else { return };
    let (first, last) = d.shown_pages;
    let mut pages: Vec<usize> = d.rasters.keys().copied().filter(|p| !(first..=last).contains(p)).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(d.page)));
    for p in pages {
        if total <= render::BUDGET {
            return;
        }
        if let Some(r) = d.rasters.remove(&p) {
            total = total.saturating_sub(r.bytes());
        }
    }
}

/// Whether the active PDF shows two pages across, for remembering it: `None` when the active tab
/// isn't a PDF shown yet.
pub fn spread(app: &SheetApp, uid: u64) -> Option<bool> {
    app.pdf.docs.get(&uid).map(|d| d.spread)
}

/// The status bar's words for the active PDF: the page shown and how many there are (nothing
/// while it opens or if it can't be shown). `None` for any other tab.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| d.is_pdf())?;
    let pages = app.pdf.docs.get(&d.uid).map_or(0, Doc::pages);
    let page = app.pdf.docs.get(&d.uid).map_or(0, |doc| doc.page);
    Some(if pages > 0 { format!("{} {} / {pages}", app.ui.language.tr("Page"), page + 1) } else { String::new() })
}

/// The active PDF as `ui.inspect` reports it (`null` for any other tab).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| d.is_pdf()) else { return Json::Null };
    let Some(doc) = app.pdf.docs.get(&d.uid) else { return json!({"status": "opening"}) };
    let status = match &doc.phase {
        Phase::Opening => "opening".to_owned(),
        Phase::Ready(_) => "ready".to_owned(),
        Phase::Failed(why) => format!("failed: {why:?}"),
    };
    let sorted = |keys: Vec<usize>| {
        let mut keys = keys;
        keys.sort_unstable();
        keys
    };
    json!({
        "status": status,
        "pages": doc.pages(),
        "page": doc.page + 1,
        "spread": doc.spread,
        "scale": doc.scale,
        "offset": [doc.offset.x, doc.offset.y],
        "rendered": sorted(doc.rasters.keys().copied().collect()),
        "texts": sorted(doc.texts.keys().copied().collect()),
        "selection": doc.selection.map(|s| s.text(&doc.texts)),
        "hits": doc.hits.total(),
        "hit": doc.hit.and_then(|h| doc.hits.index_of(h)),
        "contents": doc.contents.as_ref().map(|items| items.iter().map(|i| i.title.clone()).collect::<Vec<_>>()),
        "queued": doc.queue.len(),
        "onScreen": doc.on_screen.iter().map(|(p, r)| json!([p, r.left(), r.top(), r.width(), r.height()])).collect::<Vec<_>>(),
    })
}
