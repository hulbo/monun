//! Web tabs: an HTML file (.html, .htm) or a site link (.url, a Windows internet shortcut, which
//! New › Site Link writes) shown in its tab, to look at. The engine knows the page only as an
//! empty read-only tab ([`monun_engine::docs::web_workbook`]).
//!
//! The desktop app owns the web views (WebView2, the Edge engine Windows has): each frame this
//! module lists which page shows where ([`State::panes`]) and what the user asked of it
//! ([`State::orders`]), and the app reports back each page's title and address
//! ([`State::pages`]). A page is hidden while a menu, a dialog or the File view is open, as a web
//! view is a window of its own over the app and would cover them. Where there is no web view
//! (tests, the web build) the tab offers to open the page in the default browser.
//!
//! An HTML file is shown again when it changes on disk, as an AI tool writes it anew.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use egui::{Rect, RichText, Sense, Ui, vec2};
use monun_engine::docs::TextKind;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::doc_view::{self, Mode};
use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::widgets::DialogButton;

/// How often the HTML file shown is checked for a change on disk (seconds).
const WATCH_EVERY: f64 = 1.0;

/// The most of a site link file read: a .url file is a few lines.
const LINK_MAX: u64 = 64 * 1024;

/// The longest name a site link's file gets.
const NAME_MAX: usize = 80;

/// A page the window shows: its id (the tab's workbook uid), its address, and where (the window's
/// physical pixels: left, top, width, height), or `None` while it is hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub id: u64,
    pub url: String,
    pub rect: Option<[u32; 4]>,
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
}

/// A web tab: its file, the address it gives, and when it was last seen changed.
struct Tab {
    path: String,
    address: Result<String, String>,
    modified: Option<SystemTime>,
    checked: f64,
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
/// front), what a closed tab had goes, and the HTML file in front is shown again when it has
/// changed on disk.
pub(crate) fn tidy(app: &mut SheetApp, ctx: &egui::Context) {
    let open: Vec<(u64, String)> = app.session.documents().iter().filter(|d| d.is_web()).filter_map(|d| Some((d.uid, d.path.clone()?))).collect();
    let ids: HashSet<u64> = open.iter().map(|(uid, _)| *uid).collect();
    app.web.tabs.retain(|uid, _| ids.contains(uid));
    app.web.pages.retain(|uid, _| ids.contains(uid));
    app.web.orders.retain(|(uid, _)| ids.contains(uid));
    app.web.panes.clear();
    if open.is_empty() {
        return;
    }
    let front = app.session.active().filter(|d| d.is_web()).map(|d| d.uid);
    let now = ctx.input(|i| i.time);
    for (uid, path) in open {
        let tab = app.web.tabs.entry(uid).or_insert_with(|| Tab { path: String::new(), address: Err(String::new()), modified: None, checked: now });
        if tab.path != path {
            *tab = Tab { address: address_of(&path), modified: modified(&path), path, checked: now };
        } else if front == Some(uid) && now - tab.checked >= WATCH_EVERY {
            tab.checked = now;
            let seen = modified(&tab.path);
            if seen.is_some() && seen != tab.modified {
                tab.modified = seen;
                // A site link written anew may hold another address; an HTML file shows again.
                let address = address_of(&tab.path);
                if address == tab.address {
                    app.web.orders.push((uid, Order::Reload));
                }
                tab.address = address;
            }
        }
        if let Ok(url) = &tab.address {
            app.web.panes.push(Pane { id: uid, url: url.clone(), rect: None });
        }
    }
    if front.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(WATCH_EVERY));
    }
}

/// At the end of a frame: the page of a web tab closed during it goes, and one no longer in front
/// hides, so the window never shows a page over another tab.
pub(crate) fn settle(app: &mut SheetApp) {
    if app.web.panes.is_empty() {
        return;
    }
    let open: HashSet<u64> = app.session.documents().iter().filter(|d| d.is_web()).map(|d| d.uid).collect();
    let front = app.session.active().filter(|d| d.is_web()).map(|d| d.uid);
    app.web.panes.retain(|p| open.contains(&p.id));
    for pane in &mut app.web.panes {
        if Some(pane.id) != front {
            pane.rect = None;
        }
    }
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether something of the app's is drawn over the tab, which a web view (a window of its own
/// over the app) would cover: a menu or a list, a dialog, a message, the File view, or the memo
/// or group panel while its title is dragged about.
fn covered(app: &SheetApp, ctx: &egui::Context) -> bool {
    app.dialog.is_some()
        || app.message.is_some()
        || app.backstage.is_some()
        || app.grid.tab_menu.is_some()
        || app.side_drag.is_some()
        || egui::Popup::is_any_open(ctx)
}

/// The room between a page and the memo or group panel floating beside it (points).
const LAYER_GAP: f32 = 8.0;

/// The smallest a page is shown beside a floating panel (points); with less room it goes above or
/// below it, or hides.
const ROOM_MIN: f32 = 160.0;

/// The part of `area` a page shows in clear of `cover` (the memo or group panel floating over the
/// files in a corner): beside it, on whichever side has more room, as a column of its own would
/// leave; above or below it when there's too little room beside it; `None` when there's too
/// little room anywhere.
pub fn clear_of(area: Rect, cover: Rect) -> Option<Rect> {
    let cover = cover.expand(LAYER_GAP);
    if !cover.intersects(area) {
        return Some(area);
    }
    let left = Rect::from_min_max(area.min, egui::pos2(cover.left().min(area.right()), area.bottom()));
    let right = Rect::from_min_max(egui::pos2(cover.right().max(area.left()), area.top()), area.max);
    let above = Rect::from_min_max(area.min, egui::pos2(area.right(), cover.top().min(area.bottom())));
    let below = Rect::from_min_max(egui::pos2(area.left(), cover.bottom().max(area.top())), area.max);
    let roomy = |r: &Rect| r.width() >= ROOM_MIN && r.height() >= ROOM_MIN;
    let larger = |a: Rect, b: Rect| if a.area() >= b.area() { a } else { b };
    [larger(left, right), larger(above, below)].into_iter().find(roomy)
}

/// Draws the active web tab: its tool bar (Back, Forward, Show Again, the address, Open in
/// Browser) and the page under it.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some(d) = app.session.active().filter(|d| d.is_web()) else { return };
    let uid = d.uid;
    // Open, New, Close, Find… as in any document's tab.
    doc_view::keyboard(app, ui, Mode::View, TextKind::Web);
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    ui.painter().rect_filled(ui.max_rect(), 0.0, if t.dark { t.window } else { t.grid_bg });
    let address = app.web.tabs.get(&uid).map(|tab| tab.address.clone()).unwrap_or_else(|| Err(String::new()));
    let page = app.web.pages.get(&uid).cloned().unwrap_or_default();
    let shown = if page.url.is_empty() { address.clone().unwrap_or_default() } else { page.url.clone() };
    let showing = matches!(app.web.host, Some(Ok(()))) && address.is_ok();
    doc_view::tool_strip(ui, &t, |ui| {
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
        doc_view::separator(ui, &t);
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
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    // A line in the middle of the tab instead of the page, read out as any label is.
    let note = |ui: &mut Ui, text: &str| {
        let at = Rect::from_center_size(area.center(), vec2((area.width() - 40.0).max(80.0), 40.0));
        ui.put(at, egui::Label::new(RichText::new(text).font(theme::ui_font(14.0)).color(t.text_dim)).halign(egui::Align::Center));
    };
    match (&app.web.host, &address) {
        (_, Err(why)) if !why.is_empty() => note(ui, &format!("{} {}", lang.tr("The page couldn't be shown:"), lang.tr(why))),
        (_, Err(_)) => {}
        (Some(Ok(())), Ok(_)) => {
            if let Some(why) = &page.error {
                note(ui, &format!("{} {why}", lang.tr("The page couldn't be shown:")));
            } else if !covered(app, ui.ctx()) {
                // Beside the memo or group panel when it floats over the files.
                let room = match app.side_layer_rect {
                    Some(layer) => clear_of(area, layer),
                    None => Some(area),
                };
                let ppp = ui.ctx().pixels_per_point();
                let px = |v: f32| (v * ppp).round().max(0.0) as u32;
                let rect = room.map(|r| [px(r.left()), px(r.top()), px(r.width()), px(r.height())]);
                if let Some(pane) = app.web.panes.iter_mut().find(|p| p.id == uid) {
                    pane.rect = rect;
                }
            }
        }
        (Some(Err(why)), Ok(_)) => note(ui, &format!("{} {why}", lang.tr("Web pages can't be shown on this PC:"))),
        // The tool bar's Open in Browser shows it there.
        (None, Ok(_)) => note(ui, lang.tr("Web pages show here in the desktop app.")),
    }
}

fn open_in_browser(app: &SheetApp, url: &str) {
    if let Some(open) = &app.services.open_url {
        open(url);
    }
}

/// The status bar's words for the active web tab: its page's title.
pub fn status(app: &SheetApp) -> Option<String> {
    let d = app.session.active().filter(|d| d.is_web())?;
    Some(app.web.pages.get(&d.uid).map(|p| p.title.clone()).unwrap_or_default())
}

/// The active web tab as `ui.inspect` reports it (`null` for any other tab).
pub fn inspect(app: &SheetApp) -> Json {
    let Some(d) = app.session.active().filter(|d| d.is_web()) else { return Json::Null };
    let address = app.web.tabs.get(&d.uid).map(|t| t.address.clone());
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
    fn a_page_goes_beside_a_floating_panel() {
        let area = Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(1200.0, 800.0));
        // Top right, most of the height: the page to its left.
        let layer = Rect::from_min_max(egui::pos2(888.0, 92.0), egui::pos2(1188.0, 700.0));
        assert_eq!(clear_of(area, layer), Some(Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(880.0, 800.0))));
        // Top left: to its right.
        let layer = Rect::from_min_max(egui::pos2(12.0, 92.0), egui::pos2(312.0, 400.0));
        assert_eq!(clear_of(area, layer).map(|r| r.left()), Some(320.0));
        // Elsewhere (a column, the start page): the whole area.
        let layer = Rect::from_min_max(egui::pos2(1210.0, 0.0), egui::pos2(1500.0, 800.0));
        assert_eq!(clear_of(area, layer), Some(area));
        // Too narrow beside it: above or below.
        let narrow = Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(400.0, 800.0));
        let layer = Rect::from_min_max(egui::pos2(60.0, 500.0), egui::pos2(360.0, 790.0));
        assert_eq!(clear_of(narrow, layer), Some(Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(400.0, 492.0))));
        // No room anywhere: hidden.
        let layer = Rect::from_min_max(egui::pos2(20.0, 100.0), egui::pos2(380.0, 790.0));
        assert_eq!(clear_of(narrow, layer), None);
    }

    #[test]
    fn a_link_is_named_after_its_site() {
        assert_eq!(site_name("https://www.example.com/a/b"), "example.com");
        assert_eq!(site_name("http://intra.go.kr:8080?x"), "intra.go.kr:8080");
        assert_eq!(file_name("a/b:c*?\"<>|.."), "a b c");
        assert_eq!(file_name("intra.go.kr:8080"), "intra.go.kr 8080");
    }
}
