//! Web tabs: an HTML file (.html, .htm) or a site link (.url, a Windows internet shortcut, which
//! New › Site Link writes) shown in its tab, to look at. The engine knows the page only as an
//! empty read-only tab ([`monun_engine::docs::web_workbook`]).
//!
//! The desktop app owns the web views (WebView2, the Edge engine Windows has): each frame this
//! module lists which page shows where ([`State::panes`]) and what the user asked of it
//! ([`State::orders`]), and the app reports back each page's title and address
//! ([`State::pages`]). A web view is a window of its own over the app, so what the app floats over
//! the page (the memo or group panel as a layer, a menu, a dialog, a tooltip) is cut out of it
//! ([`Pane::holes`]) and shows through; the page hides only behind the File view. Where there is
//! no web view (tests, the web build) the tab offers to open the page in the default browser.
//!
//! An HTML file is shown again when it changes on disk, as an AI tool writes it anew.
//!
//! A PowerPoint presentation is shown the same way: its slides are written as a page first
//! ([`crate::slides_view`]), again when the file changes.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use egui::{Rect, RichText, Sense, Ui, vec2};
use monun_engine::docs::TextKind;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::doc_view::{self, Mode};
use crate::icons::Icon;
use crate::slides_view;
use crate::theme::{self, Tokens};
use crate::widgets::DialogButton;

/// How often the HTML file shown is checked for a change on disk (seconds).
const WATCH_EVERY: f64 = 1.0;

/// The most of a site link file read: a .url file is a few lines.
const LINK_MAX: u64 = 64 * 1024;

/// The longest name a site link's file gets.
const NAME_MAX: usize = 80;

/// A presentation's page being written: the tab says so.
const OPENING: &str = "Opening…";

/// Whether a document is shown as a page: an HTML file, a site link or a presentation.
fn paged(d: &monun_engine::DocState) -> bool {
    d.is_web() || d.is_slides()
}

/// A page the window shows: its id (the tab's workbook uid), its address, and where (the window's
/// physical pixels: left, top, width, height), or `None` while it is hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub id: u64,
    pub url: String,
    pub rect: Option<[u32; 4]>,
    /// What the app floats over the page, cut out of it so that it shows: left, top, width and
    /// height in physical pixels from the page's top left corner, in order.
    pub holes: Vec<[i32; 4]>,
}

/// What the user asked of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Back,
    Forward,
    Reload,
}

/// What a page did, as the app reports it: its title and address now, whether it is loading, and
/// why it couldn't be shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub error: Option<String>,
}

/// The web tabs and the pages the window shows.
#[derive(Default)]
pub struct State {
    /// Whether this window shows web pages: set by the desktop app (`Err` says why not, such as no
    /// WebView2), left `None` where there is no web view.
    pub host: Option<Result<(), String>>,
    /// The pages to show, rebuilt each frame: every open web tab's, hidden but the one in front.
    pub panes: Vec<Pane>,
    /// What the user asked of the pages since the app last took them.
    pub orders: Vec<(u64, Order)>,
    /// What the app last reported of each page.
    pub pages: HashMap<u64, Page>,
    tabs: HashMap<u64, Tab>,
    /// The page in front this frame and where it goes (points), for [`settle`] to cut what floats
    /// over it out of it.
    front: Option<(u64, Rect)>,
}

/// A web tab: its file, the address it gives, and when it was last seen changed; a presentation's
/// page being written, and its number of slides.
struct Tab {
    path: String,
    address: Result<String, String>,
    modified: Option<SystemTime>,
    checked: f64,
    presentation: bool,
    job: Option<slides_view::Job>,
    slides: Option<usize>,
}

impl Tab {
    fn open(path: String, presentation: bool, uid: u64, words: &(String, String), now: f64) -> Tab {
        let (address, job) =
            if presentation { (Err(OPENING.to_string()), Some(slides_view::start(&path, uid, words.clone()))) } else { (address_of(&path), None) };
        Tab { modified: modified(&path), path, address, checked: now, presentation, job, slides: None }
    }

    /// Its file changed on disk: shown again (a presentation's page written anew). Whether the page
    /// is reloaded as it is (its address the same).
    fn watch(&mut self, uid: u64, words: &(String, String)) -> bool {
        let seen = modified(&self.path);
        if seen.is_none() || seen == self.modified {
            return false;
        }
        self.modified = seen;
        if self.presentation {
            self.job = Some(slides_view::start(&self.path, uid, words.clone()));
            return false;
        }
        // A site link written anew may hold another address; an HTML file shows again.
        let address = address_of(&self.path);
        let same = address == self.address;
        self.address = address;
        same
    }

    /// A presentation's page written: shown (again). Whether it is reloaded as it is.
    fn receive(&mut self) -> bool {
        let Some(done) = self.job.as_ref().and_then(slides_view::Job::poll) else { return false };
        self.job = None;
        match done {
            Ok((url, slides)) => {
                let same = self.address.as_ref().ok() == Some(&url);
                self.address = Ok(url);
                self.slides = Some(slides);
                same
            }
            Err(why) => {
                self.address = Err(why);
                false
            }
        }
    }
}

/// The address a file shows: an HTML file's own (`file:///…`), or the one a site link holds.
pub fn address_of(path: &str) -> Result<String, String> {
    let link = std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("url"));
    if !link {
        return Ok(file_url(path));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, LINK_MAX), &mut text).map_err(|e| e.to_string())?;
    link_address(&text).ok_or_else(|| "This site link holds no web address.".to_string())
}

/// The address in a site link's text (`URL=…` under `[InternetShortcut]`), when it is a web page's
/// (http, https) or a file's.
pub fn link_address(text: &str) -> Option<String> {
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if section == "[internetshortcut]" && key.trim().eq_ignore_ascii_case("url") {
            return normalize(value);
        }
    }
    None
}

/// A web address as typed (`example.com`, `https://…`) made whole, or `None` for one that isn't a
/// web page's or a file's (`javascript:`, `mailto:`…).
pub fn normalize(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() || typed.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = typed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file:///") {
        return Some(typed.to_string());
    }
    // No scheme: a site's name (`example.com/path`), as a browser's address bar takes it; a
    // development server on this PC or the network (`localhost:8080`, `10.0.0.5`) over http.
    let host = typed.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    let local = host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::Ipv4Addr>().is_ok();
    if local {
        Some(format!("http://{typed}"))
    } else if host.contains('.') && !host.contains(':') && !host.contains('@') {
        Some(format!("https://{typed}"))
    } else {
        None
    }
}

/// A file's address (`C:\설계\a b.html` → `file:///C:/%EC%84%A4%EA%B3%84/a%20b.html`; a share's
/// `\\server\share\x.html` → `file://server/share/x.html`).
pub fn file_url(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let (prefix, rest) = match slashed.strip_prefix("//") {
        Some(share) => ("file://", share.to_string()),
        None => ("file:///", slashed.trim_start_matches('/').to_string()),
    };
    let mut out = String::from(prefix);
    for b in rest.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// At the start of each frame: the open web tabs' pages (all hidden; [`show`] shows the one in
/// front), what a closed tab had goes, and the file in front is shown again when it has changed
/// on disk.
pub(crate) fn tidy(app: &mut SheetApp, ctx: &egui::Context) {
    let open: Vec<(u64, String, bool)> =
        app.session.documents().iter().filter(|d| paged(d)).filter_map(|d| Some((d.uid, d.path.clone()?, d.is_slides()))).collect();
    let ids: HashSet<u64> = open.iter().map(|(uid, ..)| *uid).collect();
    app.web.tabs.retain(|uid, tab| {
        let keep = ids.contains(uid);
        if !keep && tab.presentation {
            slides_view::forget(*uid);
        }
        keep
    });
    app.web.pages.retain(|uid, _| ids.contains(uid));
    app.web.orders.retain(|(uid, _)| ids.contains(uid));
    app.web.panes.clear();
    app.web.front = None;
    if open.is_empty() {
        return;
    }
    let front = app.session.active().filter(|d| paged(d)).map(|d| d.uid);
    let now = ctx.input(|i| i.time);
    let lang = app.ui.language;
    let words = (lang.tr("Chart").to_string(), lang.tr("Hidden").to_string());
    let mut writing = false;
    for (uid, path, presentation) in open {
        if app.web.tabs.get(&uid).is_none_or(|t| t.path != path) {
            app.web.tabs.insert(uid, Tab::open(path, presentation, uid, &words, now));
        }
        let Some(tab) = app.web.tabs.get_mut(&uid) else { continue };
        let mut reload = false;
        if front == Some(uid) && now - tab.checked >= WATCH_EVERY {
            tab.checked = now;
            reload = tab.watch(uid, &words);
        }
        reload |= tab.receive();
        if reload {
            app.web.orders.push((uid, Order::Reload));
        }
        writing |= tab.job.is_some();
        if let Ok(url) = &tab.address {
            app.web.panes.push(Pane { id: uid, url: url.clone(), rect: None, holes: Vec::new() });
        }
    }
    if writing {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    } else if front.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(WATCH_EVERY));
    }
}

/// At the end of a frame: the page of a web tab closed during it goes, and one no longer in front
/// hides, so the window never shows a page over another tab.
pub(crate) fn settle(app: &mut SheetApp, ctx: &egui::Context) {
    if app.web.panes.is_empty() {
        return;
    }
    let open: HashSet<u64> = app.session.documents().iter().filter(|d| paged(d)).map(|d| d.uid).collect();
    let front = app.session.active().filter(|d| paged(d)).map(|d| d.uid);
    app.web.panes.retain(|p| open.contains(&p.id));
    for pane in &mut app.web.panes {
        if Some(pane.id) != front {
            pane.rect = None;
        }
    }
    // What floats over the page in front, drawn this frame (every layer above the files: the memo
    // or group panel, menus, lists, dialogs, tooltips, notices), is cut out of it.
    let Some((uid, area)) = app.web.front.filter(|(uid, _)| Some(*uid) == front) else { return };
    // (The context is read once at a time: it is locked while it is read.)
    let layers: Vec<(egui::LayerId, Rect)> = ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|l| matches!(l.order, egui::Order::Middle | egui::Order::Foreground | egui::Order::Tooltip))
            .filter_map(|l| Some((l, m.area_rect(l.id)?)))
            .collect()
    });
    // Those still drawn this frame: a menu closed in it is no longer over the page.
    let over: Vec<Rect> =
        ctx.graphics(|g| layers.iter().filter(|(l, _)| g.get(*l).is_some_and(|shapes| !shapes.is_empty())).map(|(_, r)| *r).collect());
    let cut = holes(area, &over, ctx.pixels_per_point());
    if let Some(pane) = app.web.panes.iter_mut().find(|p| p.id == uid && p.rect.is_some()) {
        pane.holes = cut;
    }
}

/// The parts of a page at `area` (points) that `over` cover, as [`Pane::holes`]: in physical pixels
/// (`ppp` to a point) from the page's top left corner, whole pixels outwards, in order.
pub fn holes(area: Rect, over: &[Rect], ppp: f32) -> Vec<[i32; 4]> {
    let mut out: Vec<[i32; 4]> = over
        .iter()
        .map(|r| r.intersect(area))
        .filter(|r| r.width() > 0.0 && r.height() > 0.0)
        .map(|r| {
            let at = |v: f32, from: f32, up: bool| {
                let v = (v - from) * ppp;
                (if up { v.ceil() } else { v.floor() }) as i32
            };
            let (left, top) = (at(r.left(), area.left(), false), at(r.top(), area.top(), false));
            let (right, bottom) = (at(r.right(), area.left(), true), at(r.bottom(), area.top(), true));
            [left, top, right - left, bottom - top]
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether the File view is open over the files: the page hides behind it. (What else floats over
/// the page is cut out of it, [`settle`].)
fn covered(app: &SheetApp) -> bool {
    app.backstage.is_some()
}

/// Draws the active web tab: its tool bar (a web page's Back, Forward, Show Again, address and
/// Open in Browser; a presentation's number of slides and Open in Default App) and the page under
/// it.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some(d) = app.session.active().filter(|d| paged(d)) else { return };
    let (uid, presentation) = (d.uid, d.is_slides());
    // Open, New, Close, Find… as in any document's tab.
    doc_view::keyboard(app, ui, Mode::View, if presentation { TextKind::Slides } else { TextKind::Web });
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    ui.painter().rect_filled(ui.max_rect(), 0.0, if t.dark { t.window } else { t.grid_bg });
    let address = app.web.tabs.get(&uid).map(|tab| tab.address.clone()).unwrap_or_else(|| Err(String::new()));
    let page = app.web.pages.get(&uid).cloned().unwrap_or_default();
    if presentation {
        slides_strip(app, ui, &t, uid);
    } else {
        web_strip(app, ui, &t, uid, (&address, &page));
    }
    // Beside the tucked group panel's handle, which would cover the page's scroll bar.
    let area = crate::panes::beside_handle(app.group_handle, ui.available_rect_before_wrap());
    ui.allocate_rect(area, Sense::hover());
    // A line in the middle of the tab instead of the page, read out as any label is.
    let note = |ui: &mut Ui, text: &str| {
        let at = Rect::from_center_size(area.center(), vec2((area.width() - 40.0).max(80.0), 40.0));
        ui.put(at, egui::Label::new(RichText::new(text).font(theme::ui_font(14.0)).color(t.text_dim)).halign(egui::Align::Center));
    };
    let failed = if presentation { "The presentation couldn't be shown:" } else { "The page couldn't be shown:" };
    match (&app.web.host, &address) {
        (_, Err(why)) if why == OPENING => note(ui, lang.tr(OPENING)),
        (_, Err(why)) if !why.is_empty() => note(ui, &format!("{} {}", lang.tr(failed), lang.tr(why))),
        (_, Err(_)) => {}
        (Some(Ok(())), Ok(_)) => {
            if let Some(why) = &page.error {
                note(ui, &format!("{} {why}", lang.tr(failed)));
            } else if !covered(app) {
                let ppp = ui.ctx().pixels_per_point();
                let px = |v: f32| (v * ppp).round().max(0.0) as u32;
                let rect = [px(area.left()), px(area.top()), px(area.width()), px(area.height())];
                if let Some(pane) = app.web.panes.iter_mut().find(|p| p.id == uid) {
                    pane.rect = Some(rect);
                    app.web.front = Some((uid, area));
                }
            }
        }
        (Some(Err(why)), Ok(_)) => note(ui, &format!("{} {why}", lang.tr("Web pages can't be shown on this PC:"))),
        // The tool bar's Open in Browser (Open in Default App) shows it there.
        (None, Ok(_)) if presentation => note(ui, lang.tr("Presentations show here in the desktop app.")),
        (None, Ok(_)) => note(ui, lang.tr("Web pages show here in the desktop app.")),
    }
}

/// A presentation's tool bar: how many slides it has, and Open in Default App (PowerPoint, where
/// there is one).
fn slides_strip(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, uid: u64) {
    let lang = app.ui.language;
    let count = app.web.tabs.get(&uid).and_then(|tab| tab.slides);
    let path = app.session.active().and_then(|d| d.path.clone());
    doc_view::tool_strip(ui, t, |ui| {
        if let Some(n) = count {
            ui.label(RichText::new(lang.tr("{n} slides").replace("{n}", &n.to_string())).font(theme::ui_font(13.0)).color(t.text_dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(path) = &path
                && ui.button(lang.tr("Open in Default App")).on_hover_text(lang.tr("Open the file in the app Windows opens it with")).clicked()
            {
                open_in_browser(app, path);
            }
        });
    });
}

/// A web page's tool bar: Back, Forward, Show Again, its address and Open in Browser.
fn web_strip(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, uid: u64, (address, page): (&Result<String, String>, &Page)) {
    let lang = app.ui.language;
    let shown = if page.url.is_empty() { address.clone().unwrap_or_default() } else { page.url.clone() };
    let showing = matches!(app.web.host, Some(Ok(()))) && address.is_ok();
    doc_view::tool_strip(ui, t, |ui| {
        let size = vec2(28.0, 28.0);
        for (icon, tip, order) in
            [(Icon::Left, "Back", Order::Back), (Icon::Right, "Forward", Order::Forward), (Icon::Reload, "Show Again", Order::Reload)]
        {
            let b = ui.add_enabled_ui(showing, |ui| crate::widgets::icon_button(ui, icon, t.text_dim, lang.tr(tip), size)).inner;
            b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, showing, lang.tr(tip)));
            if b.clicked() {
                app.web.orders.push((uid, order));
            }
        }
        doc_view::separator(ui, t);
        ui.add_space(6.0);
        let room = (ui.available_width() - 150.0).max(80.0);
        let text = if page.loading { format!("{shown}  ·  {}", lang.tr("Loading…")) } else { shown.clone() };
        // The address from the left, as an address bar shows it, cut short when long.
        ui.allocate_ui_with_layout(vec2(room, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(room);
            ui.add(egui::Label::new(RichText::new(text).font(theme::ui_font(12.5)).color(t.text_dim)).truncate().selectable(true));
        });
        if !shown.is_empty() && ui.button(lang.tr("Open in Browser")).on_hover_text(lang.tr("Open the page in the default web browser")).clicked() {
            open_in_browser(app, &shown);
        }
    });
}

fn open_in_browser(app: &SheetApp, url: &str) {
    if let Some(open) = &app.services.open_url {
        open(url);
    }
}

/// The status bar's words for the active web tab: its page's title, or a presentation's number of
/// slides.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| paged(d))?;
    if d.is_slides() {
        let count = app.web.tabs.get(&d.uid).and_then(|t| t.slides);
        return Some(count.map(|n| app.ui.language.tr("{n} slides").replace("{n}", &n.to_string())).unwrap_or_default());
    }
    Some(app.web.pages.get(&d.uid).map(|p| p.title.clone()).unwrap_or_default())
}

/// The active web tab as `ui.inspect` reports it (`null` for any other tab).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| paged(d)) else { return Json::Null };
    let address = app.web.tabs.get(&d.uid).map(|t| t.address.clone());
    let slides = app.web.tabs.get(&d.uid).and_then(|t| t.slides);
    let page = app.web.pages.get(&d.uid).cloned().unwrap_or_default();
    let pane = app.web.panes.iter().find(|p| p.id == d.uid);
    json!({
        "address": address.as_ref().and_then(|a| a.as_ref().ok()),
        "error": address.as_ref().and_then(|a| a.as_ref().err()).or(page.error.as_ref()),
        "host": match &app.web.host { None => json!(null), Some(Ok(())) => json!("ready"), Some(Err(e)) => json!(e) },
        "shown": pane.is_some_and(|p| p.rect.is_some()),
        "title": page.title,
        "url": page.url,
        "loading": page.loading,
        "rect": pane.and_then(|p| p.rect),
        "holes": pane.map(|p| p.holes.clone()).unwrap_or_default(),
        "presentation": d.is_slides(),
        "slides": slides,
    })
}

/// New › Site Link: asks for the site's name and address.
pub fn new_link(app: &mut SheetApp) {
    app.open_dialog("siteLink", json!({}));
}

/// The Site Link dialog: a name and an address; OK asks where to keep the link and opens it.
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let text = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    ui.add_space(4.0);
    ui.label(lang.tr("Address:"));
    let mut url = text(d, "url");
    let field = ui.add(egui::TextEdit::singleline(&mut url).desired_width(f32::INFINITY).hint_text("https://")).accessible_name(lang.tr("Address:"));
    if d.values.get("focused").and_then(Json::as_bool) != Some(true) {
        field.request_focus();
        d.values.insert("focused".into(), json!(true));
    }
    let mut enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    d.values.insert("url".into(), json!(url));
    ui.add_space(4.0);
    ui.label(lang.tr("Name:"));
    let mut name = text(d, "name");
    let named =
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).hint_text(site_name(&url))).accessible_name(lang.tr("Name:"));
    enter |= named.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    d.values.insert("name".into(), json!(name));
    ui.label(RichText::new(lang.tr("The link is kept as a file (.url), so it goes in groups and projects as files do.")).color(t.text_dim));
    let error = text(d, "error");
    if !error.is_empty() {
        ui.label(RichText::new(lang.tr(&error)).color(t.danger));
    }
    ui.add_space(8.0);
    let row = [DialogButton::confirm("OK"), DialogButton::neutral("Cancel")];
    let submit = match crate::widgets::dialog_buttons(ui, &row) {
        Some(0) => true,
        Some(_) => {
            *open = false;
            false
        }
        None => enter,
    };
    if submit {
        match app.run("web.newLink", json!({"url": url, "name": name})) {
            Ok(_) => *open = false,
            Err(e) => {
                d.values.insert("error".into(), json!(e));
            }
        }
    }
}

/// A site's name from its address, for a link left unnamed: `https://www.example.com/a` →
/// `example.com`.
fn site_name(url: &str) -> String {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// A name fit for a file: no `\ / : * ? " < > |`, no dots or spaces at its ends, not too long.
fn file_name(name: &str) -> String {
    let kept: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).take(NAME_MAX).collect();
    kept.trim().trim_matches('.').trim().to_string()
}

/// The UI commands of web tabs: `web.newLink {url, name?, path?}` writes a site link (asking where,
/// without `path`) and opens it.
pub(crate) fn command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    match id {
        "web.newLink" => Some(new_link_file(app, p)),
        _ => None,
    }
}

fn new_link_file(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let typed = p.get("url").and_then(Json::as_str).unwrap_or_default();
    let url = normalize(typed).ok_or("Type the site's address, such as https://example.com.")?;
    let name = file_name(p.get("name").and_then(Json::as_str).unwrap_or_default());
    let name = if name.is_empty() { file_name(&site_name(&url)) } else { name };
    let name = if name.is_empty() { "Site".to_string() } else { name };
    let path = match p.get("path").and_then(Json::as_str) {
        Some(path) => path.to_string(),
        None => {
            let Some(pick) = &app.services.pick_save else { return Err("No folder to keep the link in.".into()) };
            match pick(&format!("{name}.url")) {
                Some(path) => path,
                None => return Ok(json!({"cancelled": true})),
            }
        }
    };
    let text = format!("[InternetShortcut]\r\nURL={url}\r\n");
    std::fs::write(&path, text).map_err(|e| format!("{} {e}", app.ui.language.tr("The link couldn't be saved:")))?;
    app.open_path(&path);
    Ok(json!({"path": path, "url": url}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_link_gives_its_address() {
        assert_eq!(link_address("[InternetShortcut]\r\nURL=https://example.com/a?b=1\r\n").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(
            link_address("[{000214A0-0000-0000-C000-000000000046}]\nProp3=19,11\n[InternetShortcut]\nIDList=\nurl = http://x.go.kr\n").as_deref(),
            Some("http://x.go.kr")
        );
        assert_eq!(link_address("[Other]\nURL=https://not.this\n"), None);
        assert_eq!(link_address("[InternetShortcut]\nURL=javascript:alert(1)\n"), None);
    }

    #[test]
    fn an_address_is_made_whole() {
        assert_eq!(normalize("example.com/x").as_deref(), Some("https://example.com/x"));
        assert_eq!(normalize(" HTTPS://Example.com ").as_deref(), Some("HTTPS://Example.com"));
        assert_eq!(normalize("localhost:8080/app").as_deref(), Some("http://localhost:8080/app"), "a development server over http");
        assert_eq!(normalize("10.0.0.5:8080").as_deref(), Some("http://10.0.0.5:8080"));
        assert_eq!(normalize("intra.local:8080/app").as_deref(), Some("https://intra.local:8080/app"));
        for bad in ["", "mailto:a@b.c", "javascript:x", "a b.com", "word"] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_file_gives_its_file_address() {
        assert_eq!(file_url(r"C:\설계\a b.html"), "file:///C:/%EC%84%A4%EA%B3%84/a%20b.html");
        assert_eq!(file_url(r"\\nas\share\x.html"), "file://nas/share/x.html");
        assert_eq!(file_url("/home/u/x#1.html"), "file:///home/u/x%231.html");
    }

    #[test]
    fn what_floats_over_a_page_is_cut_out_of_it() {
        let area = Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(1200.0, 800.0));
        // The group panel in the top right corner, a menu reaching over the tool bar, and a layer
        // beside the page (a column): only what is over the page, from its corner, at 1.5x.
        let over = [
            Rect::from_min_max(egui::pos2(888.0, 92.0), egui::pos2(1188.0, 700.0)),
            Rect::from_min_max(egui::pos2(100.5, 40.0), egui::pos2(300.2, 200.0)),
            Rect::from_min_max(egui::pos2(1210.0, 0.0), egui::pos2(1500.0, 800.0)),
        ];
        assert_eq!(holes(area, &over, 1.5), vec![[150, 0, 301, 180], [1332, 18, 450, 912]]);
        // The same twice is one hole; nothing over it, none.
        assert_eq!(holes(area, &[over[0], over[0]], 1.0).len(), 1);
        assert!(holes(area, &[], 1.0).is_empty());
    }

    #[test]
    fn a_link_is_named_after_its_site() {
        assert_eq!(site_name("https://www.example.com/a/b"), "example.com");
        assert_eq!(site_name("http://intra.go.kr:8080?x"), "intra.go.kr:8080");
        assert_eq!(file_name("a/b:c*?\"<>|.."), "a b c");
        assert_eq!(file_name("intra.go.kr:8080"), "intra.go.kr 8080");
    }
}
