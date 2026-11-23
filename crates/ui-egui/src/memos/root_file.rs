//! Memos on the Markdown files under the working root folder ([`crate::projects`]), kept in the
//! root itself: `<root>\.monun\memos\memos.json`, each file's memos under its path relative to the
//! root (with `/`), so they go wherever the root goes (git) and show on any PC, as the projects do.
//! Memos on files elsewhere, or with no root, stay in `ui.json` ([`crate::UiState::memos`]). The
//! file is in a folder of its own, where a 모눈 from before memos were kept here, which takes
//! every `.json` beside the projects for one, never sees it.
//!
//! The file is read when the root is set, and again when it changes outside 모눈 (a git pull:
//! looked for every few seconds, and before each change); it is written whole, and only when its
//! memos changed. One 모눈 can't read (a git conflict left in it) is said so once and never
//! overwritten: new memos stay on this PC meanwhile and move into it once it can be read, as the
//! memos `ui.json` kept for files under the root before (an upgrade, a root just set) do.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use monun_engine::io::{DiskStamp, disk_stamp};
use serde::{Deserialize, Serialize};

use super::{DocMemo, same_path};
use crate::SheetApp;
use crate::projects;

/// The memo file's folder in the root's `.monun` folder, and its name there.
pub const FOLDER: &str = "memos";
pub const FILE: &str = "memos.json";

/// The memo file format's version: 2 once it holds a link (which a 모눈 of version 1 can't read,
/// so it leaves the file alone rather than write it back without them), else 1.
const VERSION: u32 = 2;
const VERSION_WITHOUT_LINKS: u32 = 1;

/// How often the file is looked at for a change made outside 모눈, in milliseconds.
const CHECK_MS: f64 = 2000.0;

const COULD_NOT_READ: &str = "The memos in the working root folder could not be read:";
pub(super) const COULD_NOT_SAVE: &str = "The memos in the working root folder could not be saved:";
const CONFLICT: &str = "Their file was changed outside Monun and can't be read (a git conflict?), so it is left as it is.";
const KEPT_HERE_UNTIL: &str = "New memos are kept on this PC until it can be read.";
const KEPT_HERE: &str = "The memo is kept on this PC for now.";
const MOVED: &str = "Moved {count} memo(s) into the working root folder, to go with it to other PCs.";

/// The memos of the root's files, as read in this run.
#[derive(Clone, Debug, Default)]
pub struct Store {
    pub(crate) root: String,
    /// Their files' paths as this PC opens them.
    pub(crate) memos: Vec<DocMemo>,
    /// The file as last read or written (`None`: there was none), to tell a change made outside.
    seen: Option<String>,
    stamp: Option<DiskStamp>,
    /// A version of the file 모눈 couldn't read: said so once, never overwritten.
    refused: Option<String>,
    /// When the file was last looked at ([`crate::monotonic_ms`]).
    checked: f64,
    /// Read anew by a change (a git pull it found first): the memos `ui.json` keeps for the root's
    /// files are still to move in.
    unsettled: bool,
}

/// The memo file: each file's memos by its path relative to the root.
#[derive(Serialize, Deserialize)]
struct MemoFile {
    #[serde(default)]
    monun: u32,
    /// Not optional: a file without it (a project's) is none, and never overwritten.
    memos: BTreeMap<String, Vec<StoredMemo>>,
}

/// A memo as the file keeps it, under its file's path.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredMemo {
    id: u64,
    line: usize,
    quote: String,
    text: String,
    created: String,
    /// The file a link goes to: under the root, relative to it (with `/`); elsewhere, its path.
    #[serde(skip_serializing_if = "String::is_empty")]
    link: String,
}

/// Where the memos of `root` are kept.
fn file(root: &str) -> PathBuf {
    Path::new(root).join(projects::FOLDER).join(FOLDER).join(FILE)
}

/// `path` as the memo file keeps it, when it is under `root`: relative to it, with `/`.
pub(crate) fn relative(root: &str, path: &str) -> Option<String> {
    let stored = projects::stored(root, path);
    (stored != path).then_some(stored)
}

/// A memo file's memos, their files' paths as this PC opens them.
fn parse(root: &str, text: &str) -> Result<Vec<DocMemo>, String> {
    let file: MemoFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.monun > VERSION {
        return Err("This memo file was made by a newer version of Monun.".into());
    }
    let mut out = Vec::new();
    for (stored, memos) in file.memos {
        let path = projects::resolve(root, stored.trim());
        if path.is_empty() {
            continue;
        }
        out.extend(memos.into_iter().map(|m| DocMemo {
            id: m.id,
            path: path.clone(),
            line: m.line,
            quote: m.quote,
            text: m.text,
            created: m.created,
            link: if m.link.trim().is_empty() { String::new() } else { projects::resolve(root, m.link.trim()) },
        }));
    }
    Ok(out)
}

/// The memo file's text for `memos`: its files in order of their paths, each one's memos in order
/// of their ids, so the same memos always make the same text.
fn render(root: &str, memos: &[DocMemo]) -> Option<String> {
    let mut files: BTreeMap<String, Vec<StoredMemo>> = BTreeMap::new();
    for m in memos {
        let link = if m.link.is_empty() { String::new() } else { projects::stored(root, &m.link) };
        let stored = StoredMemo { id: m.id, line: m.line, quote: m.quote.clone(), text: m.text.clone(), created: m.created.clone(), link };
        files.entry(projects::stored(root, &m.path)).or_default().push(stored);
    }
    for list in files.values_mut() {
        list.sort_by_key(|m| m.id);
    }
    let monun = if memos.iter().any(|m| !m.link.is_empty()) { VERSION } else { VERSION_WITHOUT_LINKS };
    let mut text = serde_json::to_string_pretty(&MemoFile { monun, memos: files }).ok()?;
    text.push('\n');
    Some(text)
}

/// The file's text; `None` when there is none.
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Says why the memos couldn't be read or kept, in a message.
pub(super) fn tell(app: &mut SheetApp, title: &str, detail: &str) {
    let lang = app.ui.language;
    app.message = Some((lang.tr("Monun").to_string(), format!("{}\n\n{detail}", lang.tr(title))));
}

/// The working root, when there is one.
fn root_of(app: &SheetApp) -> Option<&str> {
    app.ui.root.as_deref().filter(|r| !r.trim().is_empty())
}

/// The largest id `ui.json`'s memos have.
fn ui_max(app: &SheetApp) -> u64 {
    app.ui.memos.iter().map(|m| m.id).max().unwrap_or(0)
}

/// Keeps the store to the working root: read when the root is set or changed (the memos `ui.json`
/// keeps for its files moved in), dropped with it. Returns whether it was read now.
pub(crate) fn ensure(app: &mut SheetApp) -> bool {
    if app.root_memos.as_ref().map(|s| s.root.as_str()) == root_of(app) {
        return false;
    }
    app.root_memos = root_of(app).map(|root| Store { root: root.to_string(), ..Store::default() });
    if app.root_memos.is_none() {
        return false;
    }
    // A file that can't be read now is read again later; until then the root has no memos here.
    let _ = refresh(app);
    settle(app);
    true
}

/// Each frame: the store kept to the root, and its file looked at every few seconds for a change
/// made outside 모눈 (a git pull).
pub(crate) fn watch(app: &mut SheetApp) {
    if ensure(app) {
        return;
    }
    let now = crate::monotonic_ms();
    let Some(s) = app.root_memos.as_mut() else { return };
    if std::mem::take(&mut s.unsettled) {
        settle(app);
        return;
    }
    if now - s.checked < CHECK_MS {
        return;
    }
    s.checked = now;
    if disk_stamp(&file(&s.root).to_string_lossy()) == s.stamp {
        return;
    }
    if refresh(app) == Ok(true) {
        settle(app);
    }
}

/// Reads the file again now (Reload Project, after a git pull).
pub(crate) fn reload(app: &mut SheetApp) {
    if ensure(app) {
        return;
    }
    let _ = refresh(app);
    settle(app);
}

/// Reads the file: when it changed since it was last read or written (a git pull), its memos take
/// the place of those read before. Returns whether they did, or the file became readable again.
/// One that can't be read is said so once and left as it is.
fn refresh(app: &mut SheetApp) -> Result<bool, String> {
    let lang = app.ui.language;
    let Some(s) = app.root_memos.as_mut() else { return Ok(false) };
    let path = file(&s.root);
    let stamp = disk_stamp(&path.to_string_lossy());
    // Read now: the next look is a few seconds on.
    s.checked = crate::monotonic_ms();
    let disk = read(&path)?;
    s.stamp = stamp;
    if disk == s.seen {
        // As it was read last (a conflict undone): it can be written again.
        return Ok(s.refused.take().is_some());
    }
    let Some(text) = disk else {
        // Gone (deleted, or a branch without it checked out): no memos.
        s.memos.clear();
        s.seen = None;
        s.refused = None;
        return Ok(true);
    };
    match parse(&s.root, &text) {
        Ok(memos) => {
            s.memos = memos;
            s.seen = Some(text);
            s.refused = None;
            Ok(true)
        }
        Err(e) => {
            if s.refused.as_deref() != Some(text.as_str()) {
                let detail = format!("{} {}\n{}: {}", lang.tr(CONFLICT), lang.tr(KEPT_HERE_UNTIL), path.display(), lang.tr(&e));
                s.refused = Some(text);
                tell(app, COULD_NOT_READ, &detail);
            }
            Ok(false)
        }
    }
}

/// After the file was read: every memo an id of its own, and the memos `ui.json` keeps for files
/// under the root moved in.
fn settle(app: &mut SheetApp) {
    if let Some(s) = app.root_memos.as_mut() {
        s.unsettled = false;
    }
    unique_ids(app);
    migrate(app);
}

/// Gives every memo an id of its own across the root's file and `ui.json` (memos made on two PCs
/// at once can come with the same one): the root's keep theirs, the first of two alike its own.
/// The memo being changed keeps to it.
fn unique_ids(app: &mut SheetApp) {
    let Some(s) = app.root_memos.as_mut() else { return };
    let mut next = s.memos.iter().chain(app.ui.memos.iter()).map(|m| m.id).max().unwrap_or(0);
    // Each id taken, and the file of the memo that has it.
    let mut taken: HashMap<u64, String> = HashMap::new();
    for m in s.memos.iter_mut() {
        if m.id == 0 || taken.contains_key(&m.id) {
            next = next.saturating_add(1);
            follow(&mut app.memos.draft, m.id, next, &m.path, taken.get(&m.id));
            m.id = next;
        }
        taken.insert(m.id, m.path.clone());
    }
    let mut renumbered = false;
    for m in app.ui.memos.iter_mut() {
        if m.id == 0 || taken.contains_key(&m.id) {
            next = next.saturating_add(1);
            follow(&mut app.memos.draft, m.id, next, &m.path, taken.get(&m.id));
            m.id = next;
            renumbered = true;
        }
        taken.insert(m.id, m.path.clone());
    }
    if renumbered {
        super::persist(app);
    }
}

/// The memo being changed follows its memo from id `old` to `new`, when it is that one: on its file,
/// which isn't the file of the memo keeping `old` (`holder`).
fn follow(draft: &mut Option<super::Draft>, old: u64, new: u64, path: &str, holder: Option<&String>) {
    if let Some(d) = draft.as_mut()
        && d.id == Some(old)
        && same_path(&d.path, path)
        && !holder.is_some_and(|h| same_path(h, path))
    {
        d.id = Some(new);
    }
}

/// Changes the root's memos by `f` and writes them, the file as it is now first (a git pull since
/// it was read is kept). `f` is given the largest id `ui.json`'s memos have then. The same memos
/// however the file is laid out leave it as it is. `Err`, with nothing changed: the file can't be
/// read (left as it is) or written.
fn commit<R>(app: &mut SheetApp, f: impl FnOnce(&mut Vec<DocMemo>, u64) -> R) -> Result<R, String> {
    let lang = app.ui.language;
    if refresh(app)? {
        unique_ids(app);
        // The memos ui.json keeps for the root's files move in next frame.
        if let Some(s) = app.root_memos.as_mut() {
            s.unsettled = true;
        }
    }
    let ui_max = ui_max(app);
    let Some(s) = app.root_memos.as_mut() else {
        return Err(lang.tr("Choose a working root folder first: File › Options › General.").to_string());
    };
    let path = file(&s.root);
    if s.refused.is_some() {
        return Err(format!("{}\n{}", lang.tr(CONFLICT), path.display()));
    }
    if !Path::new(&s.root).is_dir() {
        return Err(format!("{}: {}", s.root, lang.tr("That folder doesn't exist.")));
    }
    let before = s.memos.clone();
    let r = f(&mut s.memos, ui_max);
    let text = render(&s.root, &s.memos);
    let theirs = s.seen.as_deref().and_then(|t| parse(&s.root, t).ok()).and_then(|m| render(&s.root, &m));
    if (text.is_some() && theirs == text) || (s.seen.is_none() && s.memos.is_empty()) {
        return Ok(r);
    }
    let written = match text {
        Some(t) => projects::write_whole(&path, &t).map(|()| t),
        None => Err(path.display().to_string()),
    };
    match written {
        Ok(t) => {
            s.seen = Some(t);
            s.stamp = disk_stamp(&path.to_string_lossy());
            Ok(r)
        }
        Err(e) => {
            s.memos = before;
            Err(e)
        }
    }
}

/// Whether a new memo on `path` goes into the root's file: a file under the root, whose memo file
/// can be read.
fn takes(app: &SheetApp, path: &str) -> bool {
    app.root_memos.as_ref().is_some_and(|s| s.refused.is_none() && relative(&s.root, path).is_some() && Path::new(&s.root).is_dir())
}

/// The id after every memo's, here and in `ui.json`.
fn next_id(memos: &[DocMemo], ui_max: u64) -> u64 {
    memos.iter().map(|m| m.id).max().unwrap_or(0).max(ui_max).saturating_add(1)
}

/// Keeps a new memo in the root's file, when it is on a file under the root, with an id after every
/// memo's (the file's as it is now, a git pull's too); returns its id. It is given back (for
/// `ui.json`) otherwise: no root, a file elsewhere, a memo file 모눈 can't read or write.
pub(crate) fn add(app: &mut SheetApp, memo: DocMemo) -> Result<u64, Box<DocMemo>> {
    ensure(app);
    if !takes(app, &memo.path) {
        return Err(Box::new(memo));
    }
    let mut kept = memo.clone();
    match commit(app, |memos, ui_max| {
        kept.id = next_id(memos, ui_max);
        let id = kept.id;
        memos.push(kept);
        id
    }) {
        Ok(id) => Ok(id),
        Err(e) => {
            // A file found unreadable just now was said so already.
            if app.root_memos.as_ref().is_some_and(|s| s.refused.is_none()) {
                let lang = app.ui.language;
                tell(app, COULD_NOT_SAVE, &format!("{e}\n\n{}", lang.tr(KEPT_HERE)));
            }
            Err(Box::new(memo))
        }
    }
}

/// Changes the text of the root's memo `id` on `path`. `Ok(false)`: it is gone (deleted on another
/// PC since).
pub(crate) fn edit(app: &mut SheetApp, id: u64, path: &str, text: &str) -> Result<bool, String> {
    ensure(app);
    if app.root_memos.is_none() {
        return Ok(false);
    }
    commit(app, |memos, _| match memos.iter_mut().find(|m| m.id == id && same_path(&m.path, path)) {
        Some(m) => {
            m.text = text.to_string();
            true
        }
        None => false,
    })
}

/// Whether the root's file has the memo `id` on `path`.
fn holds(app: &SheetApp, id: u64, path: &str) -> bool {
    app.root_memos.as_ref().is_some_and(|s| s.memos.iter().any(|m| m.id == id && same_path(&m.path, path)))
}

/// Deletes the root's memo `id` on `path`.
pub(crate) fn delete(app: &mut SheetApp, id: u64, path: &str) -> Result<(), String> {
    ensure(app);
    if !holds(app, id, path) {
        return Ok(());
    }
    commit(app, |memos, _| memos.retain(|m| !(m.id == id && same_path(&m.path, path))))
}

/// Moves the memos `ui.json` keeps for files under the root into the root's file (an upgrade, a
/// root just set, memos kept on this PC while the file couldn't be read), and says how many. One
/// the file had already (a move cut short) isn't added twice; one whose id the file has takes a new
/// one. Not in a run that keeps nothing: it couldn't take them out of `ui.json`.
fn migrate(app: &mut SheetApp) {
    if app.services.save_ui.is_none() {
        return;
    }
    let Some(s) = app.root_memos.as_ref() else { return };
    if s.refused.is_some() || !Path::new(&s.root).is_dir() {
        return;
    }
    let root = s.root.clone();
    let moving: Vec<DocMemo> = app.ui.memos.iter().filter(|m| relative(&root, &m.path).is_some()).cloned().collect();
    if moving.is_empty() {
        return;
    }
    let moved = commit(app, |memos, ui_max| {
        // Compared with the file's own only: two memos alike in ui.json both move.
        let had = memos.len();
        let mut renumbered = Vec::new();
        for mut m in moving {
            let there =
                memos.iter().take(had).any(|o| same_path(&o.path, &m.path) && o.quote == m.quote && o.text == m.text && o.created == m.created);
            if there {
                continue;
            }
            if m.id == 0 || memos.iter().any(|o| o.id == m.id) {
                let id = next_id(memos, ui_max);
                renumbered.push((m.id, id, m.path.clone()));
                m.id = id;
            }
            memos.push(m);
        }
        (memos.len().saturating_sub(had), renumbered)
    });
    match moved {
        Ok((count, renumbered)) => {
            for (old, new, path) in renumbered {
                if let Some(d) = app.memos.draft.as_mut()
                    && d.id == Some(old)
                    && same_path(&d.path, &path)
                {
                    d.id = Some(new);
                }
            }
            // As ui.json's are now (the file read again may have given some a new id).
            app.ui.memos.retain(|m| relative(&root, &m.path).is_none());
            super::persist(app);
            if count > 0 {
                let lang = app.ui.language;
                app.toast = Some((lang.tr(MOVED).replace("{count}", &count.to_string()), crate::now_ms()));
            }
        }
        Err(e) => {
            // They stay in ui.json, shown as ever, and move on a later try.
            if app.root_memos.as_ref().is_some_and(|s| s.refused.is_none()) {
                tell(app, COULD_NOT_SAVE, &e);
            }
        }
    }
}

/// The memos of the root's files as read in this run (their paths as this PC opens them).
pub fn in_root(app: &SheetApp) -> Vec<DocMemo> {
    app.root_memos.as_ref().map(|s| s.memos.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(id: u64, path: &str, text: &str) -> DocMemo {
        DocMemo { id, path: path.into(), line: 2, quote: "First".into(), text: text.into(), created: "2026-11-11 10:00".into(), ..DocMemo::default() }
    }

    #[test]
    fn the_memo_file_keeps_each_files_memos_relative_to_the_root_in_one_order() {
        let root = std::env::temp_dir().join("monun-memo-unit").to_string_lossy().into_owned();
        let x = projects::resolve(&root, "a/x.md");
        let y = projects::resolve(&root, "b/y.md");
        let memos = vec![memo(3, &y, "c"), memo(2, &x, "b"), memo(1, &y, "a")];
        let text = render(&root, &memos).unwrap();
        assert!(text.contains("\"a/x.md\"") && text.contains("\"b/y.md\""), "{text}");
        assert!(!text.contains("monun-memo-unit"), "no path of this PC in it: {text}");
        let mut back = parse(&root, &text).unwrap();
        back.sort_by_key(|m| m.id);
        let mut want = memos.clone();
        want.sort_by_key(|m| m.id);
        assert_eq!(back, want);
        // The same memos in another order make the same text.
        let reordered = vec![memos[1].clone(), memos[2].clone(), memos[0].clone()];
        assert_eq!(render(&root, &reordered).unwrap(), text);
        // Laid out otherwise, it reads the same.
        let compact = serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&text).unwrap()).unwrap();
        assert_eq!(render(&root, &parse(&root, &compact).unwrap()), Some(text));
    }

    #[test]
    fn a_link_is_kept_relative_to_the_root_and_marks_the_file_for_this_version() {
        let root = std::env::temp_dir().join("monun-memo-links").to_string_lossy().into_owned();
        let md = projects::resolve(&root, "docs/guide.md");
        let plain = render(&root, &[memo(1, &md, "a memo")]).unwrap();
        assert!(plain.contains("\"monun\": 1") && !plain.contains("link"), "a file without links stays readable by version 1: {plain}");
        let inside = DocMemo { link: projects::resolve(&root, "api/orders.http"), ..memo(2, &md, "") };
        let outside = DocMemo { link: "/elsewhere/sheet.xlsx".into(), ..memo(3, &md, "") };
        let text = render(&root, &[memo(1, &md, "a memo"), inside.clone(), outside.clone()]).unwrap();
        assert!(text.contains("\"monun\": 2") && text.contains("\"link\": \"api/orders.http\""), "{text}");
        let back = parse(&root, &text).unwrap();
        assert_eq!(back.iter().find(|m| m.id == 2).map(|m| m.link.as_str()), Some(inside.link.as_str()));
        assert_eq!(back.iter().find(|m| m.id == 3).map(|m| m.link.as_str()), Some(outside.link.as_str()));
    }

    #[test]
    fn a_file_that_isnt_a_memo_file_is_refused() {
        let root = "/r";
        assert!(parse(root, r#"{"monun": 3, "memos": {}}"#).is_err(), "a newer version's");
        assert!(parse(root, r#"{"monun": 1, "groups": []}"#).is_err(), "a project's");
        assert!(parse(root, "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> theirs\n").is_err(), "a git conflict");
        assert_eq!(parse(root, r#"{"monun": 1, "memos": {}}"#), Ok(vec![]));
    }

    #[test]
    fn the_memo_file_is_where_a_projects_list_never_looks() {
        let path = file("/r");
        assert_eq!(path.parent().and_then(Path::file_name).and_then(std::ffi::OsStr::to_str), Some(FOLDER));
        assert_eq!(path.parent().and_then(Path::parent), Some(Path::new("/r").join(projects::FOLDER).as_path()));
    }
}
