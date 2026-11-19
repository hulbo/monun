//! API request tabs: a `.http` file ([`TextKind::Http`], a text file to the engine) shown as a
//! request to send, as Postman shows one: its method and address, parameters, headers and body,
//! Send (Ctrl+Enter), and the answer below (its status, time, size, body and headers; JSON laid
//! out and coloured). The request reads `{{Name}}` from the folders of the group it sits in
//! ([`crate::group_folders`]), the innermost folder's value first, then from the file's and the
//! request's own `@Name = value` lines; the folders' headers go with it (its own of a name win).
//!
//! What the tab edits goes to the file's text (the document draft), so Save, Undo, AutoRecover
//! and printing are a text file's. File Text shows the text to edit by hand.

pub mod body;
mod body_ui;
pub mod http_file;
pub mod send;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Align, FontFamily, FontId, Layout, RichText, Ui, UiBuilder, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::group_folders::{Folder, Setting};
use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::widgets::DialogButton;
use http_file::{Part, Request};
use send::{Answer, Outgoing};

/// The most of an answer's text the tab lays out (Copy takes it all).
const SHOWN_MAX: usize = 256 * 1024;

/// Headers the Common Headers menu adds.
const COMMON: [(&str, &str); 5] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json"),
    ("Authorization", "Bearer {{Token}}"),
    ("Content-Type", "application/x-www-form-urlencoded"),
    ("Accept-Language", "ko-KR"),
];

/// What the API request tabs keep, each by its document.
#[derive(Default)]
pub struct State {
    tabs: HashMap<u64, Tab>,
}

impl State {
    /// Whether the tab of `uid` has a request on its way.
    pub fn busy(&self, uid: u64) -> bool {
        self.tabs.get(&uid).and_then(|t| t.call.as_ref()).is_some_and(|c| c.slot.is_some())
    }
}

#[derive(Default)]
struct Tab {
    /// Which request of the file shows (its place among them).
    request: usize,
    section: Section,
    answer_view: AnswerView,
    /// The file's text shows as it is, to edit by hand.
    raw: bool,
    /// The request being edited, with the file's text it was read from or last written as: while
    /// the text stays so, what was typed stays as typed (a space at a value's end too).
    work: Option<Work>,
    call: Option<Call>,
    /// The bodies a switch of the Body's way left, for coming back.
    stash: body_ui::Stash,
    /// How the answer's body shows: as what (`None`: as it says it is), as it came, wrapped.
    format: Option<Format>,
    as_it_came: bool,
    wrap: bool,
}

/// What an answer's body is shown as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Json,
    Xml,
    Html,
    Text,
}

impl Format {
    const ALL: [Format; 4] = [Format::Json, Format::Xml, Format::Html, Format::Text];

    fn label(self) -> &'static str {
        match self {
            Format::Json => "JSON",
            Format::Xml => "XML",
            Format::Html => "HTML",
            Format::Text => "Text",
        }
    }

    fn lang(self) -> crate::code_editor::Lang {
        match self {
            Format::Json => crate::code_editor::Lang::Json,
            Format::Xml | Format::Html => crate::code_editor::Lang::Markup,
            Format::Text => crate::code_editor::Lang::Plain,
        }
    }

    /// What a body is, by its `Content-Type`, else by how it starts.
    fn of(content_type: &str, text: &str) -> Format {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("json") {
            return Format::Json;
        }
        if ct.contains("html") {
            return Format::Html;
        }
        if ct.contains("xml") {
            return Format::Xml;
        }
        match body::RawKind::guess(text) {
            body::RawKind::Json if http_file::pretty_json(text).is_some() => Format::Json,
            body::RawKind::Html => Format::Html,
            body::RawKind::Xml => Format::Xml,
            _ => Format::Text,
        }
    }
}

struct Work {
    text: String,
    index: usize,
    request: Request,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Section {
    #[default]
    Params,
    Headers,
    Body,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum AnswerView {
    #[default]
    Body,
    Headers,
}

/// What the sending thread leaves: the answer (or why there is none) and how long it took (ms).
type Slot = Arc<Mutex<Option<(Result<Answer, String>, f64)>>>;

/// A request sent: on its way (its slot), or answered.
struct Call {
    /// Which request of the file it is (its place among them): the answer shows under it only.
    index: usize,
    started: f64,
    slot: Option<Slot>,
    done: Option<Done>,
}

impl Call {
    fn failed(index: usize, why: String) -> Call {
        Call { index, started: crate::monotonic_ms(), slot: None, done: Some(Done { result: Err(why), ms: 0.0 }) }
    }
}

struct Done {
    result: Result<Shown, String>,
    ms: f64,
}

/// An answer as the tab shows it.
struct Shown {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    size: usize,
    /// The body as text, as it came; laid out as JSON and as XML where it reads so.
    raw: String,
    json: Option<String>,
    xml: Option<String>,
    /// What it says it is (else reads as).
    format: Format,
    cut: bool,
}

impl Shown {
    /// The body's text as `format` shows it: laid out, unless `as_it_came`.
    fn text(&self, format: Format, as_it_came: bool) -> &str {
        match (format, as_it_came) {
            (Format::Json, false) => self.json.as_deref().unwrap_or(&self.raw),
            (Format::Xml, false) => self.xml.as_deref().unwrap_or(&self.raw),
            _ => &self.raw,
        }
    }
}

/// Where a request's values come from: the folder it sits in and those that folder sits in.
#[derive(Default)]
struct Scope {
    /// The group and folder, and the folder's names from the group's top down.
    folder: Option<(String, u64, String)>,
    /// Name, value and whether it shows as dots; an inner folder's over an outer one's.
    vars: Vec<(String, String, bool)>,
    headers: Vec<(String, String)>,
    insecure: bool,
}

/// A request with its variables read.
struct Ready {
    out: Outgoing,
    missing: Vec<String>,
    /// The address as it goes, a hidden variable's value as dots.
    shown_url: String,
    /// Why it can't go as it is (its GraphQL variables aren't JSON, its file isn't chosen).
    error: Option<String>,
}

/// What a click in the tab asked for, done once it is drawn.
enum Act {
    Send,
    Cancel,
    Raw,
    Settings(String, u64),
    Pick(usize),
    AddRequest,
}

/// Puts `(name, value)` in `list`, over one of that name (`ci`: regardless of case).
fn upsert<T: Clone>(list: &mut Vec<(String, String, T)>, name: &str, value: &str, extra: T, ci: bool) {
    let same = |k: &str| if ci { k.eq_ignore_ascii_case(name) } else { k == name };
    match list.iter_mut().find(|(k, _, _)| same(k)) {
        Some(row) => {
            row.1 = value.to_string();
            row.2 = extra;
        }
        None => list.push((name.to_string(), value.to_string(), extra)),
    }
}

/// The folders the file of `uid` sits in (in the group in use), with what they set.
fn scope(app: &SheetApp, uid: u64) -> Scope {
    let Some(g) = app.ui.groups.iter().find(|g| g.name == app.ui.group) else { return Scope::default() };
    let path = app.session.documents().iter().find(|d| d.uid == uid).and_then(|d| d.path.clone());
    let folder = match &path {
        Some(p) => g.folder_of(p),
        None => app.folder_pending.get(&uid).copied().filter(|f| g.folder(*f).is_some()),
    };
    let Some(id) = folder else { return Scope::default() };
    let chain = g.folder_chain(id);
    let mut vars: Vec<(String, String, bool)> = Vec::new();
    let mut headers: Vec<(String, String, ())> = Vec::new();
    for f in chain.iter().rev() {
        for v in f.vars.iter().filter(|v| !v.name.trim().is_empty()) {
            upsert(&mut vars, v.name.trim(), &v.value, v.secret, false);
        }
        for h in f.headers.iter().filter(|h| !h.name.trim().is_empty()) {
            upsert(&mut headers, h.name.trim(), &h.value, (), true);
        }
    }
    Scope {
        folder: Some((g.name.clone(), id, g.folder_label(id))),
        vars,
        headers: headers.into_iter().map(|(k, v, ())| (k, v)).collect(),
        insecure: chain.iter().any(|f| f.insecure),
    }
}

/// `req` with its variables read: the folders', then the file's, then its own (the later wins). A
/// value may itself name another (`BaseUrl = https://{{Domain}}`).
fn ready(scope: &Scope, file_vars: &[(String, String)], (req, base): (&Request, Option<&Path>), now: f64) -> Ready {
    let mut values = scope.vars.clone();
    for (k, v) in file_vars.iter().chain(&req.vars) {
        upsert(&mut values, k, v, false, false);
    }
    let first = values.clone();
    let lookup_first = |n: &str| first.iter().find(|(k, _, _)| k == n).map(|(_, v, _)| v.clone());
    let values: Vec<(String, String, bool)> = values.into_iter().map(|(k, v, s)| (k, http_file::substitute(&v, &lookup_first, now).0, s)).collect();
    let lookup = |n: &str| values.iter().find(|(k, _, _)| k == n).map(|(_, v, _)| v.clone());
    let masked = |n: &str| values.iter().find(|(k, _, _)| k == n).map(|(_, v, s)| if *s { "••••".to_string() } else { v.clone() });
    let mut missing: Vec<String> = Vec::new();
    let mut read = |text: &str| {
        let (s, m) = http_file::substitute(text, &lookup, now);
        for name in m {
            if !missing.contains(&name) {
                missing.push(name);
            }
        }
        s
    };
    let url = read(req.url.trim());
    // The body as it goes, and the request's headers it goes with (a form's boundary named…).
    let (body, own, error) = match body::payload(req, &mut read, base) {
        Ok((body, own)) => (body, own, None),
        Err(e) => (body::Payload::Empty, req.headers.clone(), Some(e)),
    };
    let mut headers: Vec<(String, String, ())> = Vec::new();
    for (k, v) in scope.headers.iter().chain(&own) {
        let k = k.trim();
        if !k.is_empty() {
            let v = read(v.trim());
            upsert(&mut headers, k, &v, (), true);
        }
    }
    let method = if req.method.is_empty() { "GET".to_string() } else { req.method.clone() };
    Ready {
        out: Outgoing { method, url, headers: headers.into_iter().map(|(k, v, ())| (k, v)).collect(), body, insecure: scope.insecure },
        missing,
        shown_url: http_file::substitute(req.url.trim(), &masked, now).0,
        error,
    }
}

/// Whether the tab of `uid` shows its file's text.
pub fn raw(app: &SheetApp, uid: u64) -> bool {
    app.api.tabs.get(&uid).is_some_and(|t| t.raw)
}

/// The document tool bar's way back from the file's text to the request.
pub(crate) fn raw_button(app: &mut SheetApp, ui: &mut Ui, uid: u64) {
    let lang = app.ui.language;
    if ui.button(lang.tr("Request View")).on_hover_text(lang.tr("Show the request to send")).clicked() {
        app.api.tabs.entry(uid).or_default().raw = false;
    }
}

/// A new API request, `Request1.http`, holding a request to fill in; in the folder `folder` of the
/// group in use (once saved, by its path).
pub(crate) fn new_request(app: &mut SheetApp, folder: Option<u64>) {
    let lang = app.ui.language;
    let before: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    let text = http_file::template(lang.tr("New Request"));
    app.run_or_alert("file.new", json!({"document": true, "kind": "http", "prefix": lang.tr("Request"), "text": text}));
    if let Some(folder) = folder
        && let Some(doc) = app.session.active().map(|d| d.uid).filter(|u| !before.contains(u))
    {
        app.folder_pending.insert(doc, folder);
    }
}

/// Takes in the answer to the request of `uid`, once its thread left it.
fn poll(app: &mut SheetApp, uid: u64, ctx: &egui::Context) {
    let Some(call) = app.api.tabs.get_mut(&uid).and_then(|t| t.call.as_mut()) else { return };
    let Some(slot) = &call.slot else { return };
    let taken = slot.lock().ok().and_then(|mut s| s.take());
    match taken {
        Some((result, ms)) => {
            call.slot = None;
            call.done = Some(Done { result: result.map(shown), ms });
        }
        // The time on the way counts up.
        None => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
    }
}

/// An answer's body as text: decoded (UTF-8, or the Korean code page an older server answers
/// in), JSON laid out.
fn shown(a: Answer) -> Shown {
    let content_type = a.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone()).unwrap_or_default();
    let binary = a.body.iter().take(4096).any(|b| *b == 0);
    let raw = if binary { format!("({} bytes)", a.body.len()) } else { monun_engine::docs::decode_text_file(&a.body).0 };
    let (json, xml) = if binary { (None, None) } else { (http_file::pretty_json(&raw), http_file::pretty_xml(&raw)) };
    let format = if binary { Format::Text } else { Format::of(&content_type, &raw) };
    Shown { status: a.status, reason: a.reason, headers: a.headers, size: a.body.len(), raw, json, xml, format, cut: a.cut }
}

/// `n` bytes as `512 B`, `1.2 KB`, `3.4 MB`.
pub(super) fn size_text(n: usize) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{n} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    }
}

/// Sends the request of `uid`: once its variables all have values and its address is a web one.
fn send_now(app: &mut SheetApp, (uid, index): (u64, usize), ready: &Ready, ctx: &egui::Context) {
    let lang = app.ui.language;
    let tab = app.api.tabs.entry(uid).or_default();
    if let Some(e) = &ready.error {
        tab.call = Some(Call::failed(index, e.clone()));
        return;
    }
    if !ready.missing.is_empty() {
        let why = lang.tr("These variables have no value: {names}").replace("{names}", &ready.missing.join(", "));
        tab.call = Some(Call::failed(index, why));
        return;
    }
    let url = ready.out.url.trim().to_ascii_lowercase();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        tab.call = Some(Call::failed(index, lang.tr("The address must start with http:// or https://.").to_string()));
        return;
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    let (left, waker, out) = (slot.clone(), ctx.clone(), ready.out.clone());
    let spawned = std::thread::Builder::new().name("monun-api".into()).spawn(move || {
        let start = crate::monotonic_ms();
        let result = send::send(&out);
        let ms = crate::monotonic_ms() - start;
        if let Ok(mut s) = left.lock() {
            *s = Some((result, ms));
        }
        waker.request_repaint();
    });
    tab.call = Some(match spawned {
        Ok(_) => Call { index, started: crate::monotonic_ms(), slot: Some(slot), done: None },
        Err(e) => Call::failed(index, e.to_string()),
    });
}

/// A request's name in the request list: its own, else its method and address.
fn title(r: &Request) -> String {
    if r.name.trim().is_empty() { format!("{} {}", r.method, r.url) } else { r.name.clone() }
}

pub(super) fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(theme::MONO.into()))
}

/// Draws the API request tab of `(uid, sheet)` (its document draft holds the file's text).
pub fn show(app: &mut SheetApp, outer: &mut Ui, (uid, sheet): (u64, usize)) {
    let lang = app.ui.language;
    let t = Tokens::get(outer.ctx());
    let ctx = outer.ctx().clone();
    let Some(text) = app.doc_view.draft.as_ref().filter(|d| d.uid == uid && d.sheet == sheet).map(|d| d.text.clone()) else { return };
    poll(app, uid, &ctx);
    let area = outer.available_rect_before_wrap();
    outer.painter().rect_filled(area, 0.0, t.window);
    let mut acts: Vec<Act> = Vec::new();
    // Ctrl+Enter sends, before a text field takes the key.
    if app.dialog.is_none() && outer.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
        acts.push(Act::Send);
    }
    let parts = http_file::parts(&text);
    let requests: Vec<&Part> = parts.iter().filter(|p| p.is_request).collect();
    let file_vars: Vec<(String, String)> = parts.iter().filter(|p| !p.is_request).flat_map(|p| p.request.vars.clone()).collect();
    let mut inner = outer.new_child(UiBuilder::new().max_rect(area.shrink2(vec2(14.0, 10.0))).layout(Layout::top_down(Align::Min)));
    let ui = &mut inner;
    let Some(last) = requests.len().checked_sub(1) else {
        ui.label(RichText::new(lang.tr("This file holds no request yet.")).color(t.text_dim));
        ui.horizontal(|ui| {
            if ui.button(lang.tr("Add Request")).clicked() {
                acts.push(Act::AddRequest);
            }
            if ui.button(lang.tr("File Text")).clicked() {
                acts.push(Act::Raw);
            }
        });
        finish(app, uid, (&text, None), acts);
        outer.allocate_rect(area, egui::Sense::hover());
        return;
    };
    let tab = app.api.tabs.entry(uid).or_default();
    let index = tab.request.min(last);
    if tab.work.as_ref().is_none_or(|w| w.text != text || w.index != index) {
        tab.work = Some(Work { text: text.clone(), index, request: requests.get(index).map(|p| p.request.clone()).unwrap_or_default() });
    }
    let mut req = tab.work.as_ref().map(|w| w.request.clone()).unwrap_or_default();
    let (mut section, mut answer_view) = (tab.section, tab.answer_view);
    let mut look = Look { format: tab.format, as_it_came: tab.as_it_came, wrap: tab.wrap };
    let mut stash = std::mem::take(&mut tab.stash);
    let before = req.clone();
    let scope = scope(app, uid);
    // A file the request sends is named relative to the request's own file.
    let base: Option<PathBuf> = app
        .session
        .documents()
        .iter()
        .find(|d| d.uid == uid)
        .and_then(|d| d.path.as_deref())
        .and_then(|p| Path::new(p).parent())
        .map(Path::to_path_buf);
    let prepared = ready(&scope, &file_vars, (&req, base.as_deref()), crate::now_ms());
    let busy = app.api.busy(uid);

    request_bar(ui, (uid, lang, &t), (&requests, index), &mut req, (busy, &mut acts));
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(format!("→ {}", prepared.shown_url)).font(mono(12.0)).color(t.text_dim)).truncate());
    if !prepared.missing.is_empty() {
        let note = lang.tr("These variables have no value: {names}").replace("{names}", &prepared.missing.join(", "));
        ui.label(RichText::new(note).color(t.danger));
    }
    scope_line(ui, &scope, lang, &t, &mut acts);
    ui.add_space(6.0);
    let counts = (query_pairs(&req.url).len(), req.headers.len() + scope.headers.len());
    section_tabs(ui, &mut section, counts, lang, &mut acts);
    ui.separator();
    let height = (ui.available_height() * 0.45).clamp(110.0, 420.0);
    let width = ui.available_width();
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
        ui.set_min_size(vec2(width, height));
        let env = body_ui::Env { lang, t: &t, base: base.as_deref(), pick: app.services.pick_any.as_deref() };
        egui::ScrollArea::vertical().id_salt(("monun.api.request", uid)).auto_shrink([false, false]).show(ui, |ui| match section {
            Section::Params => params(ui, &mut req, lang, &t),
            Section::Headers => headers(ui, &mut req, &scope, lang, &t),
            Section::Body => body_ui::show(ui, &mut req, &mut stash, &env),
        });
    });
    ui.separator();
    // The answer of the request shown (another's waits until it shows again).
    let call = app.api.tabs.get(&uid).and_then(|tab| tab.call.as_ref()).filter(|c| c.index == index);
    answer(ui, (uid, lang, &t), call, (&mut answer_view, &mut look), &mut acts);

    if let Some(tab) = app.api.tabs.get_mut(&uid) {
        tab.request = index;
        tab.section = section;
        tab.answer_view = answer_view;
        (tab.format, tab.as_it_came, tab.wrap) = (look.format, look.as_it_came, look.wrap);
        tab.stash = stash;
    }
    let edited = (req != before).then(|| {
        let range = requests.get(index).map(|p| p.range.clone()).unwrap_or(text.len()..text.len());
        (http_file::replace(&text, range, &req), index, req.clone())
    });
    if acts.iter().any(|a| matches!(a, Act::Send)) {
        // What goes is what shows, typed this frame or not.
        let now = ready(&scope, &file_vars, (&req, base.as_deref()), crate::now_ms());
        send_now(app, (uid, index), &now, &ctx);
        acts.retain(|a| !matches!(a, Act::Send));
    }
    finish(app, uid, (&text, edited), acts);
    outer.allocate_rect(area, egui::Sense::hover());
}

/// Writes what was edited to the draft, then does what was asked.
fn finish(app: &mut SheetApp, uid: u64, (text, edited): (&str, Option<(String, usize, Request)>), acts: Vec<Act>) {
    if let Some((new_text, index, request)) = edited {
        if let Some(dr) = app.doc_view.draft.as_mut() {
            dr.set(new_text.clone());
        }
        app.api.tabs.entry(uid).or_default().work = Some(Work { text: new_text, index, request });
    }
    for act in acts {
        match act {
            Act::Send => {}
            Act::Cancel => {
                if let Some(call) = app.api.tabs.get_mut(&uid).and_then(|t| t.call.as_mut()) {
                    // The thread runs on; what it brings is dropped.
                    call.slot = None;
                    call.done = Some(Done { result: Err(app.ui.language.tr("Cancelled.").to_string()), ms: 0.0 });
                }
            }
            Act::Raw => app.api.tabs.entry(uid).or_default().raw = true,
            Act::Settings(group, id) => app.open_dialog("folderSettings", json!({"group": group, "id": id})),
            Act::Pick(i) => {
                let tab = app.api.tabs.entry(uid).or_default();
                tab.request = i;
                tab.work = None;
            }
            Act::AddRequest => {
                let lang = app.ui.language;
                let current = app.doc_view.draft.as_ref().map_or_else(|| text.to_string(), |d| d.text.clone());
                let mut new_text = current.trim_end().to_string();
                if !new_text.is_empty() {
                    new_text.push_str("\n\n");
                }
                new_text.push_str(&http_file::template(lang.tr("New Request")));
                let count = http_file::requests(&new_text).len();
                if let Some(dr) = app.doc_view.draft.as_mut() {
                    dr.set(new_text);
                }
                let tab = app.api.tabs.entry(uid).or_default();
                tab.request = count.saturating_sub(1);
                tab.work = None;
            }
        }
    }
}

/// The request's line: which request of the file (when it holds more than one), its method, its
/// address, and Send (Cancel while it is on its way).
fn request_bar(
    ui: &mut Ui,
    (uid, lang, t): (u64, crate::i18n::Language, &Tokens),
    (requests, index): (&[&Part], usize),
    req: &mut Request,
    (busy, acts): (bool, &mut Vec<Act>),
) {
    ui.horizontal(|ui| {
        if requests.len() > 1 {
            craft_ui::ComboBox::from_id_salt(("monun.api.pick", uid))
                .width(150.0)
                .selected_text(title(req))
                .show_ui(ui, |ui| {
                    for (i, p) in requests.iter().enumerate() {
                        if ui.selectable_label(i == index, title(&p.request)).clicked() {
                            acts.push(Act::Pick(i));
                        }
                    }
                    ui.separator();
                    if ui.button(lang.tr("Add Request")).clicked() {
                        acts.push(Act::AddRequest);
                    }
                })
                .response
                .accessible_name(lang.tr("Request"));
        }
        craft_ui::ComboBox::from_id_salt(("monun.api.method", uid))
            .width(92.0)
            .selected_text(RichText::new(&req.method).font(mono(13.0)).strong())
            .show_ui(ui, |ui| {
                for m in http_file::METHODS {
                    if ui.selectable_label(req.method == m, RichText::new(m).font(mono(13.0))).clicked() {
                        req.method = m.to_string();
                    }
                }
            })
            .response
            .accessible_name(lang.tr("Method"));
        let button_w = 84.0;
        let url_w = (ui.available_width() - button_w - 8.0).max(120.0);
        let field = egui::TextEdit::singleline(&mut req.url).font(mono(13.5)).desired_width(url_w).hint_text("https://{{Domain}}/path");
        ui.add(field).accessible_name(lang.tr("Address"));
        let size = vec2(button_w, 26.0);
        if busy {
            if ui.add_sized(size, egui::Button::new(lang.tr("Cancel"))).clicked() {
                acts.push(Act::Cancel);
            }
        } else {
            let send = egui::Button::new(RichText::new(lang.tr("Send")).color(t.on_accent).strong()).fill(t.accent);
            if ui.add_sized(size, send).on_hover_text(lang.tr("Send the request (Ctrl+Enter)")).clicked() {
                acts.push(Act::Send);
            }
        }
    });
}

/// Which folder the request takes its variables and headers from, and its settings' button.
fn scope_line(ui: &mut Ui, scope: &Scope, lang: crate::i18n::Language, t: &Tokens, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        let small = theme::ui_font(12.0);
        match &scope.folder {
            Some((group, id, label)) => {
                crate::icons::paint(
                    ui.painter(),
                    egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 1.0), vec2(14.0, 14.0)),
                    Icon::OpenFolder,
                    t.text_dim,
                );
                ui.add_space(18.0);
                let counts = lang
                    .tr("{name} · {vars} variables · {headers} headers")
                    .replace("{name}", label)
                    .replace("{vars}", &scope.vars.len().to_string())
                    .replace("{headers}", &scope.headers.len().to_string());
                ui.label(RichText::new(counts).font(small).color(t.text_dim));
                if ui.link(lang.tr("Folder Settings…")).clicked() {
                    acts.push(Act::Settings(group.clone(), *id));
                }
            }
            None => {
                let note = lang.tr("Put this file in a folder of its group (in the group panel) to use the folder's variables and headers.");
                ui.label(RichText::new(note).font(small).color(t.text_dim));
            }
        }
        if scope.insecure {
            ui.label(RichText::new(lang.tr("Certificates unchecked")).font(theme::ui_font(12.0)).color(t.warning));
        }
    });
}

/// Parameters, Headers and Body, and File Text at the right.
fn section_tabs(ui: &mut Ui, section: &mut Section, (params, headers): (usize, usize), lang: crate::i18n::Language, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        let count = |label: &str, n: usize| if n > 0 { format!("{} {n}", lang.tr(label)) } else { lang.tr(label).to_string() };
        for (s, label) in [
            (Section::Params, count("Parameters", params)),
            (Section::Headers, count("Headers", headers)),
            (Section::Body, lang.tr("Body").to_string()),
        ] {
            if ui.selectable_label(*section == s, label).clicked() {
                *section = s;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(lang.tr("File Text")).on_hover_text(lang.tr("Show the file's text, to edit it by hand")).clicked() {
                acts.push(Act::Raw);
            }
        });
    });
}

/// The address's parameters (`?a=1&b=2`), as written (not decoded).
fn query_pairs(url: &str) -> Vec<(String, String)> {
    let Some((_, query)) = url.split_once('?') else { return Vec::new() };
    let query = query.split_once('#').map_or(query, |(q, _)| q);
    if query.is_empty() {
        return Vec::new();
    }
    query.split('&').map(|p| p.split_once('=').map_or((p.to_string(), String::new()), |(k, v)| (k.to_string(), v.to_string()))).collect()
}

/// `url` with its parameters `pairs`.
fn with_pairs(url: &str, pairs: &[(String, String)]) -> String {
    let (head, frag) = url.split_once('#').map_or((url, None), |(h, f)| (h, Some(f)));
    let base = head.split_once('?').map_or(head, |(b, _)| b);
    let mut out = base.to_string();
    if !pairs.is_empty() {
        out.push('?');
        out.push_str(&pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    }
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// A table of names and values to edit, a row's ✕ taking it out; `true` when one changed. Rows are
/// laid out side by side (a grid would keep a field as narrow as it first was).
pub(super) fn pairs_table(ui: &mut Ui, salt: (&str, usize), rows: &mut Vec<(String, String)>, lang: crate::i18n::Language, t: &Tokens) -> bool {
    let mut changed = false;
    let mut remove = None;
    let name_w = 200.0;
    let value_w = (ui.available_width() - name_w - 50.0).max(120.0);
    if !rows.is_empty() {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(vec2(name_w, 16.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(name_w);
                ui.label(RichText::new(lang.tr("Name")).color(t.text_dim));
            });
            ui.label(RichText::new(lang.tr("Value")).color(t.text_dim));
        });
    }
    for (i, (k, v)) in rows.iter_mut().enumerate() {
        ui.push_id((salt, i), |ui| {
            ui.horizontal(|ui| {
                changed |= ui.add(egui::TextEdit::singleline(k).font(mono(13.0)).desired_width(name_w)).accessible_name(lang.tr("Name")).changed();
                changed |= ui.add(egui::TextEdit::singleline(v).font(mono(13.0)).desired_width(value_w)).accessible_name(lang.tr("Value")).changed();
                if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, lang.tr("Remove"), vec2(22.0, 22.0)).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove.filter(|i| *i < rows.len()) {
        rows.remove(i);
        changed = true;
    }
    changed
}

fn params(ui: &mut Ui, req: &mut Request, lang: crate::i18n::Language, t: &Tokens) {
    let mut pairs = query_pairs(&req.url);
    let mut changed = pairs_table(ui, ("monun.api.params", 0), &mut pairs, lang, t);
    if ui.button(lang.tr("Add Parameter")).clicked() {
        pairs.push((String::new(), String::new()));
        changed = true;
    }
    if changed {
        req.url = with_pairs(&req.url, &pairs);
    }
}

fn headers(ui: &mut Ui, req: &mut Request, scope: &Scope, lang: crate::i18n::Language, t: &Tokens) {
    if !scope.headers.is_empty() {
        let own: Vec<String> = req.headers.iter().map(|(k, _)| k.trim().to_ascii_lowercase()).collect();
        egui::Grid::new("monun.api.folder_headers").num_columns(3).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
            for (k, v) in &scope.headers {
                let over = own.contains(&k.to_ascii_lowercase());
                let mut name = RichText::new(k).font(mono(13.0)).color(t.text_dim);
                let mut value = RichText::new(v).font(mono(13.0)).color(t.text_dim);
                if over {
                    name = name.strikethrough();
                    value = value.strikethrough();
                }
                ui.label(name);
                ui.label(value);
                let tag = if over { "the request's own goes instead" } else { "from the folder" };
                ui.label(RichText::new(lang.tr(tag)).font(theme::ui_font(11.5)).color(t.text_dim));
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    }
    pairs_table(ui, ("monun.api.headers", 0), &mut req.headers, lang, t);
    ui.horizontal(|ui| {
        if ui.button(lang.tr("Add Header")).clicked() {
            req.headers.push((String::new(), String::new()));
        }
        ui.menu_button(lang.tr("Common Headers"), |ui| {
            for (k, v) in COMMON {
                if ui.button(format!("{k}: {v}")).clicked() {
                    match req.headers.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
                        Some(row) => row.1 = v.to_string(),
                        None => req.headers.push((k.to_string(), v.to_string())),
                    }
                    ui.close();
                }
            }
        });
    });
}

/// How the answer's body shows (Body's tool bar): as what, as it came, wrapped.
struct Look {
    format: Option<Format>,
    as_it_came: bool,
    wrap: bool,
}

/// The answer: its status, time and size, Body or Headers, and for the body what it shows as (JSON,
/// XML, HTML, Text; laid out unless As It Came), Wrap and Copy; the time counting up while it is on
/// its way. The body shows coloured as what it is, its lines numbered (unless wrapped).
fn answer(
    ui: &mut Ui,
    (uid, lang, t): (u64, crate::i18n::Language, &Tokens),
    call: Option<&Call>,
    (view, look): (&mut AnswerView, &mut Look),
    acts: &mut Vec<Act>,
) {
    let Some(call) = call else {
        ui.label(RichText::new(lang.tr("Send (Ctrl+Enter) and the answer shows here.")).color(t.text_dim));
        return;
    };
    let Some(done) = &call.done else {
        ui.horizontal(|ui| {
            ui.spinner();
            let secs = (crate::monotonic_ms() - call.started) / 1000.0;
            ui.label(lang.tr("Sending… {secs} s").replace("{secs}", &format!("{secs:.1}")));
            if ui.button(lang.tr("Cancel")).clicked() {
                acts.push(Act::Cancel);
            }
        });
        return;
    };
    let shown = match &done.result {
        Ok(s) => s,
        Err(why) => {
            let (head, detail) = why.split_once('\n').unwrap_or((why.as_str(), ""));
            ui.label(RichText::new(lang.tr(head)).color(t.danger));
            if !detail.is_empty() {
                ui.label(RichText::new(detail).font(mono(12.0)).color(t.text_dim));
            }
            return;
        }
    };
    let format = look.format.unwrap_or(shown.format);
    let text = shown.text(format, look.as_it_came);
    ui.horizontal(|ui| {
        let colour = match shown.status {
            200..=299 => t.error_check_green,
            300..=399 => t.accent,
            400..=499 => t.warning,
            _ => t.danger,
        };
        ui.label(RichText::new(format!("{} {}", shown.status, shown.reason)).font(theme::ui_bold(13.0)).color(colour));
        ui.label(RichText::new(format!("· {} ms · {}", done.ms.round(), size_text(shown.size))).color(t.text_dim));
        if shown.cut {
            ui.label(RichText::new(lang.tr("(only its start: the answer is too long)")).color(t.warning));
        }
        ui.add_space(12.0);
        let header_label = format!("{} {}", lang.tr("Headers"), shown.headers.len());
        for (v, label) in [(AnswerView::Body, lang.tr("Body").to_string()), (AnswerView::Headers, header_label)] {
            if ui.selectable_label(*view == v, label).clicked() {
                *view = v;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if crate::widgets::icon_button(ui, Icon::Copy, t.text_dim, lang.tr("Copy the answer"), vec2(24.0, 22.0)).clicked() {
                ui.ctx().copy_text(text.to_string());
            }
            if *view != AnswerView::Body {
                return;
            }
            ui.checkbox(&mut look.wrap, lang.tr("Wrap"));
            ui.checkbox(&mut look.as_it_came, lang.tr("As It Came")).on_hover_text(lang.tr("The body as the server sent it, not laid out"));
            craft_ui::ComboBox::from_id_salt(("monun.api.answer_format", uid))
                .width(72.0)
                .selected_text(RichText::new(format.label()).color(t.accent_text).strong())
                .show_ui(ui, |ui| {
                    for f in Format::ALL {
                        if ui.selectable_label(f == format, f.label()).clicked() {
                            look.format = Some(f);
                        }
                    }
                })
                .response
                .accessible_name(lang.tr("Show the body as"));
        });
    });
    ui.add_space(4.0);
    match view {
        AnswerView::Body => code_view(ui, (uid, t), text, format.lang(), look.wrap, lang),
        AnswerView::Headers => {
            egui::ScrollArea::both().id_salt(("monun.api.answer_headers", uid)).auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new(("monun.api.answer_headers", uid)).num_columns(2).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
                    for (k, v) in &shown.headers {
                        ui.add(egui::Label::new(RichText::new(k).font(mono(13.0)).color(t.text_dim)).selectable(true));
                        ui.add(egui::Label::new(RichText::new(v).font(mono(13.0))).selectable(true));
                        ui.end_row();
                    }
                });
            });
        }
    }
}

/// Code to read and select: coloured as `code` reads, its rows even (Hangul too), its lines
/// numbered in a margin (unless it wraps), scrolling both ways. The start of a long text shows.
fn code_view(ui: &mut Ui, (uid, t): (u64, &Tokens), text: &str, code: crate::code_editor::Lang, wrap: bool, lang: crate::i18n::Language) {
    let mut end = text.len().min(SHOWN_MAX);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let shown = text.get(..end).unwrap_or_default();
    let font = mono(13.0);
    let lines = shown.lines().count().max(1);
    let digits = lines.to_string().len() as f32;
    let gutter = if wrap { 0.0 } else { digits * 8.0 + 18.0 };
    let width = if wrap { (ui.available_width() - 16.0).max(100.0) } else { f32::INFINITY };
    let mut job = crate::code_editor::layout_job(shown, code, &font, t.text, &t.syntax, width);
    let row = crate::doc_view::even_code_rows(ui.ctx(), &mut job, &font);
    let area = if wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
    area.id_salt(("monun.api.answer", uid)).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if !wrap {
                let (margin, _) = ui.allocate_exact_size(vec2(gutter, lines as f32 * row), egui::Sense::hover());
                let clip = ui.clip_rect();
                let first = ((clip.top() - margin.top()) / row).floor().max(0.0) as usize;
                let last = (((clip.bottom() - margin.top()) / row).ceil().max(0.0) as usize).min(lines);
                let small = mono(12.0);
                for i in first..last {
                    let y = margin.top() + i as f32 * row + row / 2.0;
                    ui.painter().text(
                        egui::pos2(margin.right() - 10.0, y),
                        egui::Align2::RIGHT_CENTER,
                        (i + 1).to_string(),
                        small.clone(),
                        t.text_dim,
                    );
                }
            }
            let label = egui::Label::new(job).selectable(true);
            ui.add(if wrap { label.wrap() } else { label.extend() });
        });
        if end < text.len() {
            ui.label(RichText::new(lang.tr("Only its start shows here; Copy takes it all.")).color(t.text_dim));
        }
    });
}

/// Folder Settings: a folder's variables (each may show as dots) and headers, and whether its
/// requests check certificates. The changes go to the folder on OK.
pub(crate) fn settings_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let group = d.values.get("group").and_then(Json::as_str).unwrap_or_default().to_string();
    let Some(id) = d.values.get("id").and_then(Json::as_u64) else {
        *open = false;
        return;
    };
    let Some(folder) = app.ui.groups.iter().find(|g| g.name == group).and_then(|g| g.folder(id)).cloned() else {
        *open = false;
        return;
    };
    let mut work: Folder = d.values.get("work").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_else(|| folder.clone());
    let note = lang.tr("For the API requests in the folder “{name}” and in the folders within it.").replace("{name}", &folder.name);
    ui.label(RichText::new(note).color(t.text_dim));
    ui.add_space(8.0);
    ui.label(RichText::new(lang.tr("Variables")).strong());
    ui.label(RichText::new(lang.tr("Write {{Name}} in a request's address, headers or body.")).font(theme::ui_font(12.0)).color(t.text_dim));
    settings_table(ui, ("monun.api.folder_vars", id), &mut work.vars, true, (lang, &t));
    ui.add_space(8.0);
    ui.label(RichText::new(lang.tr("Headers")).strong());
    let note = lang.tr("Every request in the folder sends these; a request's own header of the same name goes instead.");
    ui.label(RichText::new(note).font(theme::ui_font(12.0)).color(t.text_dim));
    settings_table(ui, ("monun.api.folder_headers", id), &mut work.headers, false, (lang, &t));
    ui.add_space(8.0);
    ui.checkbox(&mut work.insecure, lang.tr("Don't check certificates (a development server's own)"));
    if let Ok(v) = serde_json::to_value(&work) {
        d.values.insert("work".into(), v);
    }
    ui.add_space(10.0);
    let row = [DialogButton::confirm("OK"), DialogButton::neutral("Cancel")];
    match crate::widgets::dialog_buttons(ui, &row) {
        Some(0) => {
            *open = false;
            let clean = |rows: Vec<Setting>| -> Vec<Setting> {
                rows.into_iter().filter(|r| !r.name.trim().is_empty()).map(|r| Setting { name: r.name.trim().to_string(), ..r }).collect()
            };
            if let Some(f) = app.ui.groups.iter_mut().find(|g| g.name == group).and_then(|g| g.folders.iter_mut().find(|f| f.id == id)) {
                f.vars = clean(work.vars);
                f.headers = clean(work.headers);
                f.insecure = work.insecure;
            }
            crate::groups::persist(app);
        }
        Some(_) => *open = false,
        None => {}
    }
}

/// A folder's variables or headers to edit: name, value (dots for a hidden variable), Hidden, ✕.
fn settings_table(ui: &mut Ui, salt: (&str, u64), rows: &mut Vec<Setting>, secrets: bool, (lang, t): (crate::i18n::Language, &Tokens)) {
    let mut remove = None;
    let name_w = 170.0;
    let value_w = if secrets { 290.0 } else { 360.0 };
    for (i, r) in rows.iter_mut().enumerate() {
        ui.push_id((salt, i), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut r.name).font(mono(13.0)).desired_width(name_w).hint_text(lang.tr("Name")))
                    .accessible_name(lang.tr("Name"));
                ui.add(
                    egui::TextEdit::singleline(&mut r.value).font(mono(13.0)).desired_width(value_w).password(r.secret).hint_text(lang.tr("Value")),
                )
                .accessible_name(lang.tr("Value"));
                if secrets {
                    ui.checkbox(&mut r.secret, lang.tr("Hidden")).on_hover_text(lang.tr("Show the value as dots (a password or a token)"));
                }
                if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, lang.tr("Remove"), vec2(22.0, 22.0)).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove.filter(|i| *i < rows.len()) {
        rows.remove(i);
    }
    let add = if secrets { "Add Variable" } else { "Add Header" };
    if ui.button(lang.tr(add)).clicked() {
        rows.push(Setting::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reads_the_innermost_folders_values_and_its_own_headers_win() {
        let scope = Scope {
            folder: None,
            vars: vec![
                ("Domain".into(), "dev.example.com".into(), false),
                ("Token".into(), "abc".into(), true),
                ("Base".into(), "https://{{Domain}}".into(), false),
            ],
            headers: vec![("Authorization".into(), "Bearer {{Token}}".into()), ("Accept".into(), "*/*".into())],
            insecure: false,
        };
        let req = Request {
            method: "POST".into(),
            url: "{{Base}}/pay/{{CustNo}}?a=1".into(),
            headers: vec![("accept".into(), "application/json".into())],
            body: "{\"t\":\"{{Token}}\"}".into(),
            vars: vec![("CustNo".into(), "C1024".into())],
            ..Request::default()
        };
        let r = ready(&scope, &[], (&req, None), 0.0);
        assert!(r.missing.is_empty(), "{:?}", r.missing);
        assert_eq!(r.out.url, "https://dev.example.com/pay/C1024?a=1");
        // The folder's first, the request's own of a name instead, and the type its JSON body reads as.
        let headers = [("Authorization", "Bearer abc"), ("Accept", "application/json"), ("Content-Type", "application/json")];
        assert_eq!(r.out.headers, headers.map(|(k, v)| (k.to_string(), v.to_string())));
        assert_eq!(r.out.body, body::Payload::Text("{\"t\":\"abc\"}".into()));
        assert_eq!(r.shown_url, "https://dev.example.com/pay/C1024?a=1");
        let hidden = ready(&scope, &[], (&Request { url: "https://x/{{Token}}/{{Nope}}".into(), ..Request::default() }, None), 0.0);
        assert_eq!((hidden.shown_url.as_str(), hidden.missing.as_slice()), ("https://x/••••/{{Nope}}", ["Nope".to_string()].as_slice()));
    }

    #[test]
    fn parameters_edit_the_address() {
        assert_eq!(query_pairs("https://x/a?b=1&c=&d#top"), [("b".into(), "1".into()), ("c".into(), String::new()), ("d".into(), String::new())]);
        assert_eq!(with_pairs("https://x/a?b=1#top", &[("b".into(), "2".into()), ("e".into(), "{{E}}".into())]), "https://x/a?b=2&e={{E}}#top");
        assert_eq!(with_pairs("https://x/a?b=1", &[]), "https://x/a");
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(1536), "1.5 KB");
    }
}
