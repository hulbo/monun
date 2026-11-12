//! A 한글 document's own thread: it holds the document (rhwp's `HwpDocument`), the caret and the
//! selection, does the requests the tab sends in their order, and answers ([`super::proto`]).
//!
//! Positions are rhwp's: a section, a paragraph and a character offset (Unicode scalars) in the
//! body, or a path of (table control, cell, cell paragraph) steps from a body paragraph for a
//! place in a table. Every call into rhwp is guarded: a panic in it refuses that request and the
//! document carries on. rhwp's measures are CSS pixels; the tab's are points.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use rhwp::model::control::Control;
use rhwp::model::paragraph::Paragraph;
use rhwp::wasm_api::HwpDocument;
use serde_json::{Value as Json, json};

use super::proto::{Export, Format, Move, Point, Reply, Req, State, TableOp, Toggle};
use super::render::{PT_PER_PX, svg_key};

/// Font sizes stepped through by Grow and Shrink (points).
const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 36.0, 48.0, 72.0];
/// Undo steps kept (each a copy of the document; rhwp keeps up to 100).
const MOST_UNDO: usize = 40;
/// Lines PageUp and PageDown move.
const PAGE_LINES: usize = 20;

/// Where a document is read from.
pub(super) enum Source {
    Path(String),
    /// A file's bytes, and whether they are an AutoRecover copy (unsaved work).
    Bytes(Vec<u8>, bool),
    Blank,
}

/// The tab's end of the document's thread.
pub(super) struct Handle {
    pub uid: u64,
    tx: mpsc::Sender<(u64, Req)>,
    rx: mpsc::Receiver<Reply>,
    sent: u64,
    /// The document's own format is .hwpx (otherwise .hwp): what AutoRecover copies it as.
    pub hwpx: bool,
}

impl Handle {
    /// Starts the thread, which reads (or makes) the document and says [`Reply::Ready`] or
    /// [`Reply::Failed`].
    pub fn start(uid: u64, source: Source, hwpx: bool, ctx: egui::Context) -> Result<Handle, String> {
        let (tx, requests) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        std::thread::Builder::new().name("monun-hwp-doc".into()).spawn(move || run(source, &requests, &replies, &ctx)).map_err(|e| e.to_string())?;
        Ok(Handle { uid, tx, rx, sent: 0, hwpx })
    }

    /// Sends a request; returns its number, or 0 if the thread has stopped.
    pub fn send(&mut self, req: Req) -> u64 {
        self.sent += 1;
        if self.tx.send((self.sent, req)).is_err() {
            return 0;
        }
        self.sent
    }

    /// The number of the last request sent.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// What the thread has said, and whether it has stopped.
    pub fn poll(&self) -> (Vec<Reply>, bool) {
        let mut replies = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(r) => replies.push(r),
                Err(mpsc::TryRecvError::Empty) => return (replies, false),
                Err(mpsc::TryRecvError::Disconnected) => return (replies, true),
            }
        }
    }

    /// The document as a file, waiting up to `wait` seconds for it.
    pub fn export(&self, hwpx: bool, wait: f64) -> Result<Export, String> {
        let (answer, got) = mpsc::channel();
        self.tx.send((0, Req::Export { hwpx, answer })).map_err(|_| "the document has stopped".to_owned())?;
        got.recv_timeout(Duration::from_secs_f64(wait)).map_err(|_| "the document didn't answer".to_owned())?
    }

    /// The document's text, waiting up to `wait` seconds for it.
    pub fn text(&self, wait: f64) -> Option<String> {
        let (answer, got) = mpsc::channel();
        self.tx.send((0, Req::Text { answer })).ok()?;
        got.recv_timeout(Duration::from_secs_f64(wait)).ok()
    }
}

/// Runs `f`, a call into rhwp, catching a panic.
fn guard<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// A JSON answer of rhwp's, read.
fn parse<E>(r: Result<String, E>) -> Option<Json> {
    r.ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// A call into rhwp's web facade, whose error path can't be taken outside a browser (it
/// panics): `None` on either.
fn web<T, E>(f: impl FnOnce() -> Result<T, E>) -> Option<T> {
    guard(f)?.ok()
}

fn num(v: &Json, key: &str) -> Option<usize> {
    v.get(key).and_then(Json::as_u64).and_then(|n| usize::try_from(n).ok())
}

fn float(v: &Json, key: &str) -> Option<f32> {
    v.get(key).and_then(Json::as_f64).map(|f| f as f32)
}

/// The thread: reads the document, then does what it is asked until the tab goes.
fn run(source: Source, requests: &mpsc::Receiver<(u64, Req)>, replies: &mpsc::Sender<Reply>, ctx: &egui::Context) {
    let opened = guard(|| open(source)).unwrap_or_else(|| Err("damaged".into()));
    let mut ed = match opened {
        Ok((doc, recovered)) => Editor::new(doc, recovered),
        Err(why) => {
            let _ = replies.send(Reply::Failed(why));
            ctx.request_repaint();
            return;
        }
    };
    let ready = guard(|| ed.state(0)).unwrap_or_default();
    let _ = replies.send(Reply::Ready(ready));
    ctx.request_repaint();
    while let Ok(first) = requests.recv() {
        // Everything waiting is done at once: typing that came faster than the document took it
        // goes in as one piece, and pages are drawn once, as the document is at the end.
        let mut batch = vec![first];
        batch.extend(requests.try_iter());
        let (mut done, mut pages, mut changed) = (0, Vec::new(), false);
        let mut typed: Option<(u64, String)> = None;
        let flush = |ed: &mut Editor, typed: &mut Option<(u64, String)>, changed: &mut bool| {
            if let Some((_, text)) = typed.take() {
                if guard(|| ed.type_text(&text)).is_none() {
                    let _ = replies.send(Reply::Refused("typing".into()));
                }
                *changed = true;
            }
        };
        for (seq, req) in batch {
            done = done.max(seq);
            match req {
                Req::Type(text) => match &mut typed {
                    Some((_, so_far)) => so_far.push_str(&text),
                    None => typed = Some((seq, text)),
                },
                Req::Pages(list) => {
                    flush(&mut ed, &mut typed, &mut changed);
                    pages.extend(list);
                }
                other => {
                    flush(&mut ed, &mut typed, &mut changed);
                    let sync = matches!(other, Req::Export { .. } | Req::PrintPages { .. } | Req::Text { .. } | Req::Outline);
                    if guard(|| ed.handle(other, replies)).is_none() {
                        let _ = replies.send(Reply::Refused("an edit".into()));
                    }
                    changed |= !sync;
                }
            }
        }
        flush(&mut ed, &mut typed, &mut changed);
        if changed || done > ed.told {
            ed.told = done;
            if let Some(state) = guard(|| ed.state(done)) {
                let _ = replies.send(Reply::State(state));
            }
        }
        pages.sort_unstable();
        pages.dedup();
        for page in pages {
            let Some(Ok(svg)) = guard(|| ed.doc.render_page_svg_native(page as u32)) else { continue };
            let _ = replies.send(Reply::Page { rev: ed.rev, page, key: svg_key(&svg), svg: Arc::new(svg) });
        }
        ctx.request_repaint();
    }
}

/// Reads the document (or makes a new one); whether it is unsaved work too.
fn open(source: Source) -> Result<(HwpDocument, bool), String> {
    let (bytes, recovered) = match source {
        Source::Blank => {
            let mut core = rhwp::DocumentCore::new_empty();
            core.create_blank_document_native().map_err(|e| e.to_string())?;
            return Ok((HwpDocument::from(core), false));
        }
        Source::Path(p) => (std::fs::read(&p).map_err(|e| e.to_string())?, false),
        Source::Bytes(b, recovered) => (b, recovered),
    };
    let service = rhwp::service::DocumentService::new().with_title_scan(false).with_max_bytes(None);
    match service.open_bytes(&bytes, &rhwp::service::OpenOptions::default()) {
        Ok(opened) => Ok((HwpDocument::from(opened.into_core()), recovered)),
        Err(e) if e.needs_password() => Err("password".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// A place in the document: a body paragraph's, or (with `cell`) a table cell paragraph's.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pos {
    sec: usize,
    /// The body paragraph; in a table, the one holding the (outermost) table.
    para: usize,
    off: usize,
    /// The steps into tables, (control, cell, cell paragraph), the last one's paragraph the one
    /// the place is in.
    cell: Vec<(usize, usize, usize)>,
}

impl Pos {
    fn body(sec: usize, para: usize, off: usize) -> Pos {
        Pos { sec, para, off, cell: Vec::new() }
    }

    /// The paragraph the place is in, within its container (the section, or the cell).
    fn para_in(&self) -> usize {
        self.cell.last().map_or(self.para, |s| s.2)
    }

    /// The same place in another paragraph of the same container.
    fn at(&self, para: usize, off: usize) -> Pos {
        let mut p = self.clone();
        match p.cell.last_mut() {
            Some(step) => step.2 = para,
            None => p.para = para,
        }
        p.off = off;
        p
    }

    /// Whether two places are in the same container (the same section, or the same cell).
    fn same_container(&self, other: &Pos) -> bool {
        self.sec == other.sec
            && (self.cell.is_empty() && other.cell.is_empty()
                || self.para == other.para
                    && self.cell.len() == other.cell.len()
                    && self
                        .cell
                        .iter()
                        .zip(&other.cell)
                        .enumerate()
                        .all(|(i, (a, b))| a.0 == b.0 && a.1 == b.1 && (i + 1 == self.cell.len() || a.2 == b.2)))
    }

    /// The path as rhwp's web facade takes it.
    fn path_json(&self) -> String {
        json!(self.cell.iter().map(|(c, k, p)| json!({"controlIndex": c, "cellIndex": k, "cellParaIndex": p})).collect::<Vec<_>>()).to_string()
    }

    /// A place from rhwp's answer to a hit test, a vertical move or a search.
    fn from_answer(v: &Json) -> Option<Pos> {
        if v.get("isTextBox").and_then(Json::as_bool) == Some(true) {
            return None;
        }
        let sec = num(v, "sectionIndex")?;
        let off = num(v, "charOffset")?;
        if let Some(path) = v.get("cellPath").and_then(Json::as_array).filter(|p| !p.is_empty()) {
            let cell =
                path.iter().map(|s| Some((num(s, "controlIndex")?, num(s, "cellIndex")?, num(s, "cellParaIndex")?))).collect::<Option<Vec<_>>>()?;
            return Some(Pos { sec, para: num(v, "parentParaIndex")?, off, cell });
        }
        if let (Some(para), Some(control), Some(cell), Some(cpi)) =
            (num(v, "parentParaIndex"), num(v, "controlIndex"), num(v, "cellIndex"), num(v, "cellParaIndex"))
        {
            return Some(Pos { sec, para, off, cell: vec![(control, cell, cpi)] });
        }
        Some(Pos::body(sec, num(v, "paragraphIndex")?, off))
    }
}

/// An undo step: the document before (a snapshot of rhwp's) and the caret then.
struct Step {
    snapshot: u32,
    caret: Pos,
    anchor: Pos,
}

/// What the last edit was, for grouping typing into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Typing,
    Other,
}

/// The document and everything about it the thread keeps.
struct Editor {
    doc: HwpDocument,
    rev: u64,
    saved_rev: u64,
    recovered: bool,
    caret: Pos,
    anchor: Pos,
    /// Where Up and Down aim (px), kept while they repeat.
    preferred_x: Option<f64>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The last edit and where it left the caret.
    last: Option<(Kind, Pos)>,
    /// The page sizes at a revision.
    sizes: (u64, Arc<Vec<[f32; 2]>>),
    /// The outline's places, as last asked.
    outline: Vec<Pos>,
    /// The text last copied, to paste with its formatting while the clipboard still holds it.
    copied: Option<String>,
    /// The last request the tab has been told is done.
    told: u64,
}

impl Editor {
    fn new(doc: HwpDocument, recovered: bool) -> Editor {
        Editor {
            doc,
            rev: 1,
            saved_rev: 1,
            recovered,
            caret: Pos::default(),
            anchor: Pos::default(),
            preferred_x: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            sizes: (0, Arc::new(Vec::new())),
            outline: Vec::new(),
            copied: None,
            told: 0,
        }
    }

    // ------------------------------------------------------------------ reading the document

    /// The paragraph a place is in.
    fn paragraph(&self, p: &Pos) -> Option<&Paragraph> {
        let mut para = self.doc.document().sections.get(p.sec)?.paragraphs.get(p.para)?;
        for (control, cell, cpi) in &p.cell {
            let Control::Table(t) = para.controls.get(*control)? else { return None };
            para = t.cells.get(*cell)?.paragraphs.get(*cpi)?;
        }
        Some(para)
    }

    /// The paragraphs of a place's container (its section, or its cell).
    fn paragraph_count(&self, p: &Pos) -> usize {
        let section = self.doc.document().sections.get(p.sec);
        let Some((last, outer)) = p.cell.split_last() else { return section.map_or(0, |s| s.paragraphs.len()) };
        let mut para = section.and_then(|s| s.paragraphs.get(p.para));
        for (control, cell, cpi) in outer {
            para = para.and_then(|pa| match pa.controls.get(*control) {
                Some(Control::Table(t)) => t.cells.get(*cell).and_then(|c| c.paragraphs.get(*cpi)),
                _ => None,
            });
        }
        match para.and_then(|pa| pa.controls.get(last.0)) {
            Some(Control::Table(t)) => t.cells.get(last.1).map_or(0, |c| c.paragraphs.len()),
            _ => 0,
        }
    }

    fn text(&self, p: &Pos) -> Vec<char> {
        self.paragraph(p).map(|pa| pa.text.chars().collect()).unwrap_or_default()
    }

    fn len(&self, p: &Pos) -> usize {
        self.paragraph(p).map_or(0, |pa| pa.text.chars().count())
    }

    /// The place, made to exist: in a paragraph there is, at an offset it has.
    fn clamp(&self, p: &Pos) -> Pos {
        if self.paragraph(p).is_some() {
            let len = self.len(p);
            let mut q = p.clone();
            q.off = q.off.min(len);
            return q;
        }
        // Outside a table that went, or past the end: the body paragraph, or the last one.
        let sections = &self.doc.document().sections;
        let sec = p.sec.min(sections.len().saturating_sub(1));
        let count = sections.get(sec).map_or(0, |s| s.paragraphs.len());
        let para = p.para.min(count.saturating_sub(1));
        let q = Pos::body(sec, para, 0);
        let len = self.len(&q);
        Pos { off: p.off.min(len), ..q }
    }

    /// The caret's rectangle (px): its page, x, y and height.
    fn caret_rect(&self, p: &Pos) -> Option<(usize, [f32; 3])> {
        let v = if p.cell.is_empty() {
            parse(self.doc.get_cursor_rect_native(p.sec, p.para, p.off))?
        } else {
            let s = web(|| self.doc.get_cursor_rect_by_path(p.sec as u32, p.para as u32, &p.path_json(), p.off as u32))?;
            serde_json::from_str(&s).ok()?
        };
        Some((num(&v, "pageIndex")?, [float(&v, "x")?, float(&v, "y")?, float(&v, "height")?]))
    }

    /// The selection's start and end, in document order (same container).
    fn ordered(&self) -> (Pos, Pos) {
        let (a, b) = (&self.anchor, &self.caret);
        if (a.para_in(), a.off) <= (b.para_in(), b.off) { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) }
    }

    fn selecting(&self) -> bool {
        self.anchor != self.caret && self.anchor.same_container(&self.caret)
    }

    /// The selection's rectangles (px), by page.
    fn selection_rects(&self) -> Vec<(usize, [f32; 4])> {
        if !self.selecting() {
            return Vec::new();
        }
        let (a, b) = self.ordered();
        let s = if a.cell.is_empty() {
            web(|| self.doc.get_selection_rects(a.sec as u32, a.para as u32, a.off as u32, b.para as u32, b.off as u32))
        } else {
            web(|| {
                self.doc.get_selection_rects_in_cell_by_path(
                    a.sec as u32,
                    a.para as u32,
                    &a.path_json(),
                    a.para_in() as u32,
                    a.off as u32,
                    b.para_in() as u32,
                    b.off as u32,
                )
            })
        };
        let Some(Json::Array(rects)) = s.and_then(|s| serde_json::from_str(&s).ok()) else { return Vec::new() };
        rects.iter().filter_map(|r| Some((num(r, "pageIndex")?, [float(r, "x")?, float(r, "y")?, float(r, "width")?, float(r, "height")?]))).collect()
    }

    /// The formatting at the caret (of the character before it).
    fn format(&mut self) -> Format {
        let p = if self.selecting() { self.ordered().0 } else { self.caret.clone() };
        let at = if p.off > 0 && !self.selecting() { p.off - 1 } else { p.off };
        let (chars, para) = if p.cell.is_empty() {
            (parse(self.doc.get_char_properties_at_native(p.sec, p.para, at)), parse(self.doc.get_para_properties_at_native(p.sec, p.para)))
        } else {
            (
                parse(self.doc.get_cell_char_properties_at_by_path(p.sec, p.para, &p.cell, at)),
                parse(self.doc.get_cell_para_properties_at_by_path(p.sec, p.para, &p.cell)),
            )
        };
        let flag =
            |k: &str| chars.as_ref().and_then(|c| c.get(k)).is_some_and(|v| v.as_bool().unwrap_or_else(|| v.as_str().is_some_and(|s| s != "None")));
        Format {
            bold: flag("bold"),
            italic: flag("italic"),
            underline: flag("underline"),
            strike: flag("strikethrough"),
            size: chars.as_ref().and_then(|c| c.get("fontSize")).and_then(Json::as_f64).map(|s| (s / 100.0) as f32),
            font: chars.as_ref().and_then(|c| c.get("fontFamily")).and_then(Json::as_str).map(str::to_owned),
            align: para.as_ref().and_then(|p| p.get("alignment")).and_then(Json::as_str).map(str::to_lowercase),
        }
    }

    /// The pages' sizes (points), worked out again only after a change.
    fn sizes(&mut self) -> Arc<Vec<[f32; 2]>> {
        if self.sizes.0 != self.rev {
            let n = self.doc.page_count();
            let sizes: Vec<[f32; 2]> = (0..n)
                .map(|p| {
                    parse(self.doc.get_page_info_native(p))
                        .and_then(|v| Some([float(&v, "width")? * PT_PER_PX, float(&v, "height")? * PT_PER_PX]))
                        .unwrap_or([595.3, 841.9])
                })
                .collect();
            self.sizes = (self.rev, Arc::new(sizes));
        }
        self.sizes.1.clone()
    }

    /// How the document is, for the tab.
    fn state(&mut self, done: u64) -> State {
        let pt = |r: [f32; 3]| [r[0] * PT_PER_PX, r[1] * PT_PER_PX, r[2] * PT_PER_PX];
        State {
            rev: self.rev,
            done,
            pages: self.sizes(),
            caret: self.caret_rect(&self.caret).map(|(p, r)| (p, pt(r))),
            selection: self.selection_rects().into_iter().map(|(p, r)| (p, r.map(|v| v * PT_PER_PX))).collect(),
            dirty: self.recovered || self.rev != self.saved_rev,
            format: self.format(),
            in_table: !self.caret.cell.is_empty(),
        }
    }

    // ------------------------------------------------------------------ requests

    fn handle(&mut self, req: Req, replies: &mpsc::Sender<Reply>) {
        match req {
            Req::Type(text) => self.type_text(&text),
            Req::Move { to, extend } => self.move_caret(to, extend),
            Req::Erase { back, word } => self.erase(back, word),
            Req::Enter => self.enter(),
            Req::Tab { back } => self.tab(back),
            Req::Point { page, x, y, how } => self.point(page, x / PT_PER_PX, y / PT_PER_PX, how),
            Req::SelectAll => {
                let last = self.doc.document().sections.get(self.caret.sec).map_or(0, |s| s.paragraphs.len()).saturating_sub(1);
                let (first, end) = if self.caret.cell.is_empty() {
                    (Pos::body(self.caret.sec, 0, 0), Pos::body(self.caret.sec, last, 0))
                } else {
                    (self.caret.at(0, 0), self.caret.at(self.paragraph_count(&self.caret).saturating_sub(1), 0))
                };
                self.anchor = first;
                self.caret = Pos { off: self.len(&end), ..end };
            }
            Req::Collapse => self.anchor = self.caret.clone(),
            Req::Toggle(t) => self.toggle(t),
            Req::Font(name) => {
                let id = self.doc.find_or_create_font_id_native(&name);
                if id >= 0 {
                    self.char_format(&json!({"fontId": id}));
                }
            }
            Req::Size(pt) => self.char_format(&json!({"fontSize": (pt * 100.0).round() as i64})),
            Req::Grow(up) => {
                let now = self.format().size.unwrap_or(10.0);
                let next = if up { SIZES.iter().find(|s| **s > now + 0.01) } else { SIZES.iter().rev().find(|s| **s < now - 0.01) };
                if let Some(s) = next {
                    self.char_format(&json!({"fontSize": (s * 100.0).round() as i64}));
                }
            }
            Req::Color(rgb) => self.char_format(&json!({"textColor": format!("#{rgb:06x}")})),
            Req::Align(a) => self.align(a),
            Req::Table(op) => self.table(op, replies),
            Req::Undo => self.undo(),
            Req::Redo => self.redo(),
            Req::Find { text, forward, again } => {
                let found = self.find(&text, forward, again);
                let _ = replies.send(found);
            }
            Req::Replace { text, with } => {
                if self.selected_text().to_lowercase() == text.to_lowercase() {
                    self.type_text(&with);
                }
                let found = self.find(&text, true, true);
                let _ = replies.send(found);
            }
            Req::ReplaceAll { text, with } => {
                self.begin(Kind::Other);
                let n = parse(self.doc.replace_all_native(&text, &with, false)).and_then(|v| num(&v, "count")).unwrap_or(0);
                self.changed();
                self.caret = self.clamp(&self.caret);
                self.anchor = self.caret.clone();
                let _ = replies.send(Reply::Replaced(n));
            }
            Req::Copy { cut } => {
                if let Some((text, html)) = self.copy() {
                    self.copied = Some(text.clone());
                    let _ = replies.send(Reply::Copied { text, html });
                    if cut {
                        self.begin(Kind::Other);
                        self.delete_selection();
                        self.changed();
                    }
                }
            }
            Req::Paste { text, html } => self.paste(&text, html.as_deref()),
            Req::Goto(n) => {
                if let Some(p) = self.outline.get(n).cloned() {
                    self.caret = self.clamp(&p);
                    self.anchor = self.caret.clone();
                }
            }
            Req::Pages(_) => {}
            Req::Outline => {
                let items = self.outline();
                let _ = replies.send(Reply::Outline(self.rev, items));
            }
            Req::Export { hwpx, answer } => {
                let _ = answer.send(guard(|| self.export(hwpx)).unwrap_or_else(|| Err("the document couldn't be written".into())));
            }
            Req::PrintPages { answer } => {
                let n = self.doc.page_count();
                let pages: Option<Vec<String>> =
                    guard(|| (0..n).map(|p| self.doc.render_page_svg_layer_with_profile_native(p, rhwp::paint::RenderProfile::Print).ok()).collect())
                        .flatten();
                let _ = answer.send(pages.ok_or_else(|| "a page couldn't be drawn".to_owned()));
            }
            Req::Saved => {
                self.saved_rev = self.rev;
                self.recovered = false;
            }
            Req::Text { answer } => {
                let mut lines = Vec::new();
                for section in &self.doc.document().sections {
                    texts(&section.paragraphs, &mut lines);
                }
                let _ = answer.send(lines.join("\n"));
            }
        }
    }

    /// Before an edit: an undo step, but typing on from where the last typing left goes into
    /// its step.
    fn begin(&mut self, kind: Kind) {
        if kind == Kind::Typing && self.last.as_ref().is_some_and(|(k, at)| *k == Kind::Typing && *at == self.caret && self.anchor == self.caret) {
            return;
        }
        let snapshot = self.doc.save_snapshot_native();
        self.undo.push(Step { snapshot, caret: self.caret.clone(), anchor: self.anchor.clone() });
        if self.undo.len() > MOST_UNDO {
            let old = self.undo.remove(0);
            self.doc.discard_snapshot_native(old.snapshot);
        }
        for step in self.redo.drain(..) {
            self.doc.discard_snapshot_native(step.snapshot);
        }
        self.last = Some((kind, self.caret.clone()));
    }

    /// After an edit: a new revision.
    fn changed(&mut self) {
        self.rev += 1;
        self.preferred_x = None;
        if let Some((k, _)) = self.last.take() {
            self.last = Some((k, self.caret.clone()));
        }
    }

    fn undo(&mut self) {
        let Some(step) = self.undo.pop() else { return };
        let now = self.doc.save_snapshot_native();
        self.redo.push(Step { snapshot: now, caret: self.caret.clone(), anchor: self.anchor.clone() });
        if self.doc.restore_snapshot_native(step.snapshot).is_ok() {
            self.doc.discard_snapshot_native(step.snapshot);
            self.caret = self.clamp(&step.caret);
            self.anchor = self.clamp(&step.anchor);
        }
        self.last = None;
        self.changed();
    }

    fn redo(&mut self) {
        let Some(step) = self.redo.pop() else { return };
        let now = self.doc.save_snapshot_native();
        self.undo.push(Step { snapshot: now, caret: self.caret.clone(), anchor: self.anchor.clone() });
        if self.doc.restore_snapshot_native(step.snapshot).is_ok() {
            self.doc.discard_snapshot_native(step.snapshot);
            self.caret = self.clamp(&step.caret);
            self.anchor = self.clamp(&step.anchor);
        }
        self.last = None;
        self.changed();
    }

    /// The new caret offset an edit answered with.
    fn moved_to(answer: Result<String, rhwp::HwpError>) -> Option<usize> {
        parse(answer).and_then(|v| num(&v, "charOffset"))
    }

    /// Types `text` (over the selection) at the caret; line breaks in it make new paragraphs.
    fn type_text(&mut self, text: &str) {
        self.begin(if text.contains('\n') { Kind::Other } else { Kind::Typing });
        self.insert_lines(text);
    }

    /// Types `text` (over the selection) at the caret, in the undo step already begun.
    fn insert_lines(&mut self, text: &str) {
        if self.selecting() {
            self.delete_selection();
        }
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.split();
            }
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let p = self.caret.clone();
            let answer = if p.cell.is_empty() {
                self.doc.insert_text_native(p.sec, p.para, p.off, line)
            } else {
                self.doc.insert_text_in_cell_by_path(p.sec, p.para, &p.cell, p.off, line)
            };
            self.caret.off = Self::moved_to(answer).unwrap_or(p.off + line.chars().count());
        }
        self.anchor = self.caret.clone();
        self.changed();
    }

    /// Splits the caret's paragraph there (Enter).
    fn split(&mut self) {
        let p = self.caret.clone();
        if p.cell.is_empty() {
            if let Some(v) = parse(self.doc.split_paragraph_native(p.sec, p.para, p.off, None)) {
                self.caret = Pos::body(p.sec, num(&v, "paraIdx").unwrap_or(p.para + 1), 0);
            }
        } else if let Some(v) = parse(self.doc.split_paragraph_in_cell_by_path(p.sec, p.para, &p.cell, p.off, None)) {
            self.caret = p.at(num(&v, "cellParaIndex").unwrap_or(p.para_in() + 1), 0);
        }
    }

    fn enter(&mut self) {
        self.begin(Kind::Other);
        if self.selecting() {
            self.delete_selection();
        }
        self.split();
        self.anchor = self.caret.clone();
        self.changed();
    }

    /// Deletes the selection; the caret where it began.
    fn delete_selection(&mut self) {
        let (a, b) = self.ordered();
        let answer = if a.cell.is_empty() {
            self.doc.delete_range_native(a.sec, a.para, a.off, b.para, b.off, None)
        } else {
            self.doc.delete_range_in_cell_by_path(a.sec, a.para, &a.cell, a.para_in(), a.off, b.para_in(), b.off)
        };
        self.caret = a;
        if let Some(v) = parse(answer)
            && let Some(off) = num(&v, "charOffset")
        {
            self.caret.off = off;
        }
        self.caret = self.clamp(&self.caret);
        self.anchor = self.caret.clone();
    }

    /// Backspace or Delete: the selection, or a character (a word), or the paragraph break.
    fn erase(&mut self, back: bool, word: bool) {
        if self.selecting() {
            self.begin(Kind::Other);
            self.delete_selection();
            self.changed();
            return;
        }
        let p = self.caret.clone();
        let len = self.len(&p);
        // Nothing before the first character (after the last): no edit, no undo step.
        if back && p.off == 0 && p.para_in() == 0 || !back && p.off >= len && p.para_in() + 1 >= self.paragraph_count(&p) {
            return;
        }
        self.begin(Kind::Other);
        let count = if word {
            let to = if back { self.word_left(&p) } else { self.word_right(&p) };
            to.off.abs_diff(p.off).max(1)
        } else {
            1
        };
        if back && p.off > 0 {
            let from = p.off.saturating_sub(count);
            let answer = if p.cell.is_empty() {
                self.doc.delete_text_native(p.sec, p.para, from, p.off - from)
            } else {
                self.doc.delete_text_in_cell_by_path(p.sec, p.para, &p.cell, from, p.off - from)
            };
            self.caret.off = Self::moved_to(answer).unwrap_or(from);
        } else if !back && p.off < len {
            let n = count.min(len - p.off);
            let answer = if p.cell.is_empty() {
                self.doc.delete_text_native(p.sec, p.para, p.off, n)
            } else {
                self.doc.delete_text_in_cell_by_path(p.sec, p.para, &p.cell, p.off, n)
            };
            self.caret.off = Self::moved_to(answer).unwrap_or(p.off);
        } else {
            // At a paragraph's start (Backspace) or end (Delete): joins it with the one before
            // (after).
            let join = if back { Some(p.para_in()).filter(|i| *i > 0) } else { Some(p.para_in() + 1).filter(|i| *i < self.paragraph_count(&p)) };
            let Some(second) = join else { return };
            let answer = if p.cell.is_empty() {
                self.doc.merge_paragraph_native(p.sec, second)
            } else {
                let mut q = p.clone();
                if let Some(step) = q.cell.last_mut() {
                    step.2 = second;
                }
                self.doc.merge_paragraph_in_cell_by_path(q.sec, q.para, &q.cell)
            };
            if let Some(v) = parse(answer) {
                let into = second - 1;
                let para = num(&v, "paraIdx").or_else(|| num(&v, "cellParaIndex")).unwrap_or(into);
                self.caret = p.at(para, num(&v, "charOffset").unwrap_or(0));
            }
        }
        self.caret = self.clamp(&self.caret);
        self.anchor = self.caret.clone();
        self.changed();
    }

    /// Tab: in a table, the next (previous) cell; elsewhere a tab character.
    fn tab(&mut self, back: bool) {
        let Some(&(control, cell, _)) = self.caret.cell.last() else {
            self.type_text("\t");
            return;
        };
        let mut host = self.caret.clone();
        host.cell.pop();
        let cells = match self.paragraph(&host).and_then(|pa| pa.controls.get(control)) {
            Some(Control::Table(t)) => t.cells.len(),
            _ => 0,
        };
        let next = if back { cell.checked_sub(1) } else { Some(cell + 1).filter(|c| *c < cells) };
        if let Some(next) = next {
            let mut p = self.caret.clone();
            if let Some(step) = p.cell.last_mut() {
                *step = (control, next, 0);
            }
            p.off = 0;
            // The whole cell's text, as Tab does in a word processor.
            let len = self.len(&p);
            self.anchor = p.clone();
            self.caret = Pos { off: len, ..p };
        }
    }

    fn word_left(&self, p: &Pos) -> Pos {
        let text = self.text(p);
        let mut i = p.off.min(text.len());
        while i > 0 && text.get(i - 1).is_some_and(|c| c.is_whitespace()) {
            i -= 1;
        }
        while i > 0 && text.get(i - 1).is_some_and(|c| !c.is_whitespace()) {
            i -= 1;
        }
        Pos { off: i, ..p.clone() }
    }

    fn word_right(&self, p: &Pos) -> Pos {
        let text = self.text(p);
        let mut i = p.off.min(text.len());
        while i < text.len() && text.get(i).is_some_and(|c| !c.is_whitespace()) {
            i += 1;
        }
        while i < text.len() && text.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        Pos { off: i, ..p.clone() }
    }

    /// One line up (`delta` −1) or down (1), keeping the column Up and Down aim at.
    fn vertical(&mut self, p: &Pos, delta: i32) -> Option<Pos> {
        let x = self.preferred_x.unwrap_or(-1.0);
        // A table one deep goes through the flat call, which leaves the table at its first and
        // last lines; a deeper one through the path.
        let s = match p.cell.as_slice() {
            [] => web(|| self.doc.move_vertical(p.sec as u32, p.para as u32, p.off as u32, delta, x, u32::MAX, 0, 0, 0))?,
            [(control, cell, cpi)] => web(|| {
                self.doc.move_vertical(p.sec as u32, p.para as u32, p.off as u32, delta, x, p.para as u32, *control as u32, *cell as u32, *cpi as u32)
            })?,
            _ => web(|| self.doc.move_vertical_by_path(p.sec as u32, p.para as u32, &p.path_json(), p.off as u32, delta, x))?,
        };
        let v: Json = serde_json::from_str(&s).ok()?;
        if let Some(x) = v.get("preferredX").and_then(Json::as_f64) {
            self.preferred_x = Some(x);
        }
        Pos::from_answer(&v).map(|q| self.clamp(&q))
    }

    /// The caret's line: its first and last offsets.
    fn line(&self, p: &Pos) -> Option<(usize, usize)> {
        let s = match p.cell.as_slice() {
            [] => web(|| self.doc.get_line_info(p.sec as u32, p.para as u32, p.off as u32))?,
            [(control, cell, cpi)] => {
                web(|| self.doc.get_line_info_in_cell(p.sec as u32, p.para as u32, *control as u32, *cell as u32, *cpi as u32, p.off as u32))?
            }
            _ => return None,
        };
        let v: Json = serde_json::from_str(&s).ok()?;
        Some((num(&v, "charStart")?, num(&v, "charEnd")?))
    }

    fn move_caret(&mut self, to: Move, extend: bool) {
        let p = self.caret.clone();
        let keep_x = matches!(to, Move::Up | Move::Down | Move::PageUp | Move::PageDown);
        if !keep_x {
            self.preferred_x = None;
        }
        // An arrow with a selection and no Shift goes to its start (end).
        if !extend && self.selecting() && matches!(to, Move::Left | Move::Right) {
            let (a, b) = self.ordered();
            self.caret = if to == Move::Left { a } else { b };
            self.anchor = self.caret.clone();
            return;
        }
        let len = self.len(&p);
        let count = self.paragraph_count(&p);
        let next = match to {
            Move::Left if p.off > 0 => Some(Pos { off: p.off - 1, ..p.clone() }),
            Move::Left => p.para_in().checked_sub(1).map(|i| {
                let q = p.at(i, 0);
                Pos { off: self.len(&q), ..q }
            }),
            Move::Right if p.off < len => Some(Pos { off: p.off + 1, ..p.clone() }),
            Move::Right => Some(p.para_in() + 1).filter(|i| *i < count).map(|i| p.at(i, 0)),
            Move::WordLeft if p.off == 0 => p.para_in().checked_sub(1).map(|i| {
                let q = p.at(i, 0);
                Pos { off: self.len(&q), ..q }
            }),
            Move::WordLeft => Some(self.word_left(&p)),
            Move::WordRight if p.off >= len => Some(p.para_in() + 1).filter(|i| *i < count).map(|i| p.at(i, 0)),
            Move::WordRight => Some(self.word_right(&p)),
            Move::Up => self.vertical(&p, -1),
            Move::Down => self.vertical(&p, 1),
            Move::PageUp | Move::PageDown => {
                let delta = if to == Move::PageUp { -1 } else { 1 };
                let mut at = p.clone();
                for _ in 0..PAGE_LINES {
                    match self.vertical(&at, delta) {
                        Some(q) if q != at => at = q,
                        _ => break,
                    }
                }
                Some(at)
            }
            Move::LineStart => Some(Pos { off: self.line(&p).map_or(0, |(s, _)| s), ..p.clone() }),
            Move::LineEnd => {
                let end = self.line(&p).map_or(len, |(_, e)| e);
                // A wrapped line ends before the next one's first character.
                let end = if end < len && self.text(&p).get(end.saturating_sub(1)).is_some_and(|c| c.is_whitespace()) { end - 1 } else { end };
                Some(Pos { off: end.min(len), ..p.clone() })
            }
            Move::DocStart => Some(Pos::body(0, 0, 0)),
            Move::DocEnd => {
                let sections = &self.doc.document().sections;
                let sec = sections.len().saturating_sub(1);
                let para = sections.get(sec).map_or(0, |s| s.paragraphs.len()).saturating_sub(1);
                let q = Pos::body(sec, para, 0);
                Some(Pos { off: self.len(&q), ..q })
            }
        };
        if let Some(q) = next {
            self.caret = q;
        }
        if !extend || !self.anchor.same_container(&self.caret) {
            self.anchor = self.caret.clone();
        }
    }

    /// A press on a page at `x`, `y` (px).
    fn point(&mut self, page: usize, x: f32, y: f32, how: Point) {
        self.preferred_x = None;
        let Some(at) = parse(self.doc.hit_test_native(page as u32, f64::from(x), f64::from(y))).as_ref().and_then(Pos::from_answer) else { return };
        let at = self.clamp(&at);
        match how {
            Point::Caret { extend } => {
                self.caret = at;
                if !extend || !self.anchor.same_container(&self.caret) {
                    self.anchor = self.caret.clone();
                }
            }
            Point::Word => {
                let text = self.text(&at);
                let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
                let (mut s, mut e) = (at.off.min(text.len()), at.off.min(text.len()));
                while s > 0 && word(text.get(s - 1)) {
                    s -= 1;
                }
                while e < text.len() && word(text.get(e)) {
                    e += 1;
                }
                self.anchor = Pos { off: s, ..at.clone() };
                self.caret = Pos { off: e, ..at };
            }
            Point::Paragraph => {
                let len = self.len(&at);
                self.anchor = Pos { off: 0, ..at.clone() };
                self.caret = Pos { off: len, ..at };
            }
        }
    }

    /// Applies character formatting to the selection, paragraph by paragraph; with none, to the
    /// word at the caret.
    fn char_format(&mut self, props: &Json) {
        let (a, b) = if self.selecting() {
            self.ordered()
        } else {
            let p = self.caret.clone();
            let text = self.text(&p);
            let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
            let (mut s, mut e) = (p.off.min(text.len()), p.off.min(text.len()));
            while s > 0 && word(text.get(s - 1)) {
                s -= 1;
            }
            while e < text.len() && word(text.get(e)) {
                e += 1;
            }
            if s == e {
                return;
            }
            (Pos { off: s, ..p.clone() }, Pos { off: e, ..p })
        };
        self.begin(Kind::Other);
        let props = props.to_string();
        for para in a.para_in()..=b.para_in() {
            let start = if para == a.para_in() { a.off } else { 0 };
            let end = if para == b.para_in() { b.off } else { self.len(&a.at(para, 0)) };
            if start >= end {
                continue;
            }
            let q = a.at(para, 0);
            let _ = if q.cell.is_empty() {
                self.doc.apply_char_format_native(q.sec, q.para, start, end, &props)
            } else {
                self.doc.apply_char_format_in_cell_by_path(q.sec, q.para, &q.cell, start, end, &props)
            };
        }
        self.changed();
    }

    fn toggle(&mut self, t: Toggle) {
        let f = self.format();
        let (key, on) = match t {
            Toggle::Bold => ("bold", f.bold),
            Toggle::Italic => ("italic", f.italic),
            Toggle::Underline => ("underline", f.underline),
            Toggle::Strike => ("strikethrough", f.strike),
        };
        let mut props = serde_json::Map::new();
        props.insert(key.into(), json!(!on));
        if t == Toggle::Underline && !on {
            props.insert("underlineType".into(), json!("Bottom"));
        }
        self.char_format(&Json::Object(props));
    }

    fn align(&mut self, alignment: &str) {
        self.begin(Kind::Other);
        let (a, b) = self.ordered();
        let props = json!({"alignment": alignment}).to_string();
        for para in a.para_in()..=b.para_in() {
            let q = a.at(para, 0);
            let _ = if q.cell.is_empty() {
                self.doc.apply_para_format_native(q.sec, q.para, &props)
            } else {
                self.doc.apply_para_format_in_cell_by_path(q.sec, q.para, &q.cell, &props)
            };
        }
        self.changed();
    }

    fn table(&mut self, op: TableOp, replies: &mpsc::Sender<Reply>) {
        if let TableOp::Insert { rows, cols } = op {
            if !self.caret.cell.is_empty() {
                let _ = replies.send(Reply::Refused("a table in a table".into()));
                return;
            }
            self.begin(Kind::Other);
            let p = self.caret.clone();
            if let Some(v) = parse(self.doc.create_table_native(p.sec, p.para, p.off, rows.max(1), cols.max(1)))
                && let (Some(para), Some(control)) = (num(&v, "paraIdx"), num(&v, "controlIdx"))
            {
                self.caret = Pos { sec: p.sec, para, off: 0, cell: vec![(control, 0, 0)] };
                self.anchor = self.caret.clone();
            }
            self.changed();
            return;
        }
        // Rows and columns of the outermost table the caret is in.
        let Some(&(control, cell, _)) = self.caret.cell.first() else {
            let _ = replies.send(Reply::Refused("not in a table".into()));
            return;
        };
        let (sec, para) = (self.caret.sec, self.caret.para);
        let host = Pos::body(sec, para, 0);
        let Some((row, col)) = (match self.paragraph(&host).and_then(|pa| pa.controls.get(control)) {
            Some(Control::Table(t)) => t.cells.get(cell).map(|c| (c.row, c.col)),
            _ => None,
        }) else {
            return;
        };
        self.begin(Kind::Other);
        let _ = match op {
            TableOp::RowAbove | TableOp::RowBelow => self.doc.insert_table_row_native(sec, para, control, row, op == TableOp::RowBelow),
            TableOp::ColumnLeft | TableOp::ColumnRight => self.doc.insert_table_column_native(sec, para, control, col, op == TableOp::ColumnRight),
            TableOp::DeleteRow => self.doc.delete_table_row_native(sec, para, control, row),
            TableOp::DeleteColumn => self.doc.delete_table_column_native(sec, para, control, col),
            TableOp::Insert { .. } => return,
        };
        // A cell deleted takes the caret to the table's paragraph.
        self.caret = self.clamp(&self.caret);
        if self.paragraph(&self.caret).is_none() || matches!(op, TableOp::DeleteRow | TableOp::DeleteColumn) {
            self.caret = host;
        }
        self.anchor = self.caret.clone();
        self.changed();
    }

    /// The selection's text.
    fn selected_text(&self) -> String {
        if !self.selecting() {
            return String::new();
        }
        let (a, b) = self.ordered();
        let mut out = String::new();
        for para in a.para_in()..=b.para_in() {
            let text = self.text(&a.at(para, 0));
            let start = if para == a.para_in() { a.off } else { 0 };
            let end = if para == b.para_in() { b.off } else { text.len() };
            if para > a.para_in() {
                out.push('\n');
            }
            out.extend(text.get(start.min(end)..end.min(text.len())).unwrap_or_default());
        }
        out
    }

    /// Copies the selection to rhwp's own clipboard: its text and HTML.
    fn copy(&mut self) -> Option<(String, Option<String>)> {
        if !self.selecting() {
            return None;
        }
        let (a, b) = self.ordered();
        let (copied, html) = if a.cell.is_empty() {
            (
                parse(self.doc.copy_selection_native(a.sec, a.para, a.off, b.para, b.off)),
                self.doc.export_selection_html_native(a.sec, a.para, a.off, b.para, b.off).ok(),
            )
        } else {
            (
                parse(self.doc.copy_selection_in_cell_by_path_native(a.sec, a.para, &a.cell, a.para_in(), a.off, b.para_in(), b.off)),
                self.doc.export_selection_in_cell_html_by_path_native(a.sec, a.para, &a.cell, a.para_in(), a.off, b.para_in(), b.off).ok(),
            )
        };
        let text = copied.and_then(|v| v.get("text").and_then(Json::as_str).map(str::to_owned)).unwrap_or_else(|| self.selected_text());
        Some((text, html))
    }

    /// Pastes: what was copied here with its formatting (while the clipboard holds it), HTML
    /// from elsewhere with its, otherwise the text.
    fn paste(&mut self, text: &str, html: Option<&str>) {
        self.begin(Kind::Other);
        if self.selecting() {
            self.delete_selection();
        }
        let p = self.caret.clone();
        let ours = self.copied.as_deref().is_some_and(|c| c.replace("\r\n", "\n") == text.replace("\r\n", "\n"));
        let answer = match (ours, html, p.cell.is_empty()) {
            (true, _, true) => parse(self.doc.paste_internal_native(p.sec, p.para, p.off)),
            (false, Some(html), true) => parse(self.doc.paste_html_native(p.sec, p.para, p.off, html)),
            (false, Some(html), false) => parse(self.doc.paste_html_in_cell_by_path_native(p.sec, p.para, &p.cell, p.off, html)),
            _ => None,
        }
        .filter(|v| v.get("ok").and_then(Json::as_bool) != Some(false));
        match answer {
            Some(v) => {
                let para = num(&v, "paraIdx").or_else(|| num(&v, "cellParaIndex")).unwrap_or(p.para_in());
                self.caret = self.clamp(&p.at(para, num(&v, "charOffset").unwrap_or(p.off)));
                self.anchor = self.caret.clone();
                self.changed();
            }
            None => {
                // As typed, in the undo step begun here.
                self.insert_lines(&text.replace("\r\n", "\n"));
            }
        }
    }

    /// Selects the next (previous) `text` from the selection, wrapping: from its end (start), or
    /// with `again` false from its start (the match at the caret is found again as it is typed);
    /// how many there are and which one it is.
    fn find(&mut self, text: &str, forward: bool, again: bool) -> Reply {
        if text.is_empty() {
            return Reply::Found { count: 0, index: None };
        }
        let (start, end) = self.ordered();
        let from = if again && forward { end } else { start };
        let Some(v) = parse(self.doc.search_text_native(text, from.sec, from.para, from.off, forward, false, true)) else {
            return Reply::Found { count: 0, index: None };
        };
        if v.get("found").and_then(Json::as_bool) != Some(true) {
            return Reply::Found { count: 0, index: None };
        }
        let (Some(sec), Some(para), Some(off), Some(len)) = (num(&v, "sec"), num(&v, "para"), num(&v, "charOffset"), num(&v, "length")) else {
            return Reply::Found { count: 0, index: None };
        };
        let at = match v.get("cellContext") {
            Some(c) => match (num(c, "parentPara"), num(c, "ctrlIdx"), num(c, "cellIdx"), num(c, "cellPara")) {
                (Some(pp), Some(ctrl), Some(cell), Some(cp)) => Pos { sec, para: pp, off, cell: vec![(ctrl, cell, cp)] },
                _ => Pos::body(sec, para, off),
            },
            None => Pos::body(sec, para, off),
        };
        self.anchor = at.clone();
        self.caret = Pos { off: off + len, ..at };
        let all = parse(self.doc.search_all_text_native(text, false, true));
        let list = all.as_ref().and_then(Json::as_array).cloned().unwrap_or_default();
        let index = list.iter().position(|m| {
            let host = m.get("cellContext").and_then(|c| num(c, "parentPara")).or_else(|| num(m, "para"));
            num(m, "sec") == Some(sec) && host == Some(self.anchor.para) && num(m, "charOffset") == Some(off)
        });
        // How many Next goes through, as rhwp counts them.
        let count = num(&v, "totalMatchCount").unwrap_or(list.len());
        Reply::Found { count: count.max(usize::from(index.is_some())), index }
    }

    /// The outline: each heading's level and title; its places kept for [`Req::Goto`].
    fn outline(&mut self) -> Vec<(u8, String)> {
        let items =
            parse(self.doc.get_outline_navigation_native()).and_then(|v| v.get("outline").and_then(Json::as_array).cloned()).unwrap_or_default();
        self.outline.clear();
        let mut out = Vec::new();
        for item in items {
            let (Some(sec), Some(para)) = (num(&item, "section"), num(&item, "paragraph")) else { continue };
            let at = if item.get("cellPath").is_some() || item.get("parentParaIndex").is_some() {
                let mut v = item.clone();
                v["sectionIndex"] = json!(sec);
                v["charOffset"] = json!(0);
                Pos::from_answer(&v).unwrap_or_else(|| Pos::body(sec, para, 0))
            } else {
                Pos::body(sec, para, 0)
            };
            let level = num(&item, "level").unwrap_or(1).saturating_sub(1).min(8) as u8;
            let number = item.get("number").and_then(Json::as_str).unwrap_or_default();
            let title = item.get("title").and_then(Json::as_str).unwrap_or_default();
            let title = if number.is_empty() { title.to_owned() } else { format!("{number} {title}") };
            self.outline.push(at);
            out.push((level, title.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        out
    }

    /// The document as a .hwpx (or .hwp) file, with what couldn't be kept.
    fn export(&self, hwpx: bool) -> Result<Export, String> {
        let saved = if hwpx { self.doc.export_hwpx_native_with_report() } else { self.doc.export_hwp_with_adapter_snapshot_with_report() };
        let (bytes, report) = saved.map_err(|e| e.to_string())?.into_parts();
        Ok(Export { bytes, lost: report.losses().iter().map(ToString::to_string).collect() })
    }
}

/// The text of `paragraphs`, a line each, with their tables' cells' after each.
fn texts(paragraphs: &[Paragraph], lines: &mut Vec<String>) {
    for p in paragraphs {
        lines.push(p.text.clone());
        for control in &p.controls {
            if let Control::Table(t) = control {
                for cell in &t.cells {
                    texts(&cell.paragraphs, lines);
                }
            }
        }
    }
}
