//! Projects, above file groups ([`groups`]): each one a set of groups kept in the working root
//! folder (File › Options › General), a folder of design documents the user keeps in git. A project
//! is the file `<root>\.monun\<name>.json` with its groups and their files, a file under the root
//! by its path relative to the root (with `/`), so the root can be cloned anywhere, on any PC, and
//! the groups still open. This PC is the groups kept on this PC alone, as they were before
//! projects. Which group is in use, which file was in front and when a group was last used stay on
//! this PC ([`UiState::project_views`]): switching tabs never changes a file git sees. The memos on
//! the root's Markdown files are kept in the `.monun` folder too, in a folder of their own
//! ([`crate::memos::root_file`]).
//!
//! [`UiState::project_views`]: crate::UiState::project_views

use std::path::{Component, Path, PathBuf};

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::groups::{self, FileGroup, Target};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets::DialogButton;

/// The folder in the root that holds the project files.
pub const FOLDER: &str = ".monun";

/// The project file format's version.
const VERSION: u32 = 1;

/// The longest project name.
const NAME_MAX: usize = 60;

/// The widest a project's name is drawn on its button.
const NAME_MAX_W: f32 = 140.0;

/// Groups and the one in use: This PC's, set aside while a project of the root is in use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GroupSet {
    pub groups: Vec<FileGroup>,
    pub group: String,
}

/// This PC's place in a project of the root: the group in use, and each group's file in front and
/// last use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectView {
    pub project: String,
    pub group: String,
    pub groups: Vec<GroupView>,
}

/// A group's file in front and last use, on this PC.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GroupView {
    pub name: String,
    pub active: Option<String>,
    pub used: Option<u64>,
    /// Its folders folded here (by id): folding never changes a file git sees.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folded: Vec<u64>,
}

/// A folder as a project file keeps it: folded or not is each PC's own.
fn shared(f: &crate::group_folders::Folder) -> crate::group_folders::Folder {
    crate::group_folders::Folder { folded: false, ..f.clone() }
}

/// Whether two groups are the same as a project file keeps them: their files, their folders and
/// which file sits in which (not which file is in front, when they were used or what is folded).
fn same_kept(a: &FileGroup, b: &FileGroup) -> bool {
    a.name == b.name
        && a.files == b.files
        && a.placed == b.placed
        && a.folders.len() == b.folders.len()
        && a.folders.iter().zip(&b.folders).all(|(x, y)| shared(x) == shared(y))
}

/// The project of the root in use, as read in this run: its root, its name, and its file as last
/// read or written, to tell a change made outside 모눈 (a git pull).
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub root: String,
    pub name: String,
    pub seen: Option<String>,
    /// A version of its file 모눈 couldn't read (a git conflict left in it, a newer version's),
    /// told about once and never overwritten.
    pub refused: Option<String>,
    /// Why its file couldn't be read at all last time (held by another program, not text): told
    /// about once, and the file left as it is.
    pub unreadable: Option<String>,
}

/// A project read for a switch to it.
pub(crate) struct Opened {
    root: String,
    text: String,
    groups: Vec<FileGroup>,
}

/// A project file.
#[derive(Serialize, Deserialize)]
struct ProjectFile {
    #[serde(default)]
    monun: u32,
    #[serde(default)]
    groups: Vec<StoredGroup>,
}

/// A group as a project file keeps it: its name and its files, in tab order.
#[derive(Serialize, Deserialize)]
struct StoredGroup {
    name: String,
    #[serde(default)]
    files: Vec<String>,
    /// Its folders ([`crate::group_folders`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<crate::group_folders::Folder>,
    /// The folder each file sits in, the file as `files` keeps it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    placed: std::collections::BTreeMap<String, u64>,
}

/// Whether two parts of a path are the same: regardless of case on Windows.
fn same_part(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    if cfg!(windows) { a.to_lowercase() == b.to_lowercase() } else { a == b }
}

/// `path` as a project file keeps it: under `root`, relative to it with `/` between its parts;
/// elsewhere, as it is.
pub fn stored(root: &str, path: &str) -> String {
    let root: Vec<Component> = Path::new(root).components().filter(|c| !matches!(c, Component::CurDir)).collect();
    if root.is_empty() {
        return path.to_string();
    }
    let mut parts = Path::new(path).components().filter(|c| !matches!(c, Component::CurDir));
    for r in &root {
        match parts.next() {
            Some(p) if same_part(r.as_os_str(), p.as_os_str()) => {}
            _ => return path.to_string(),
        }
    }
    let mut rest = Vec::new();
    for p in parts {
        match p {
            Component::Normal(s) => rest.push(s.to_string_lossy().into_owned()),
            _ => return path.to_string(),
        }
    }
    if rest.is_empty() { path.to_string() } else { rest.join("/") }
}

/// A path a project file keeps, as this PC opens it: one relative to the root, under `root`.
pub fn resolve(root: &str, stored: &str) -> String {
    let p = Path::new(stored);
    let anchored = p.is_absolute() || p.has_root() || matches!(p.components().next(), Some(Component::Prefix(_)));
    if stored.is_empty() || anchored {
        return stored.to_string();
    }
    let mut out = PathBuf::from(root);
    for part in stored.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".") {
        out.push(part);
    }
    out.to_string_lossy().into_owned()
}

/// Where the project `name` of `root` is kept.
fn project_path(root: &str, name: &str) -> PathBuf {
    Path::new(root).join(FOLDER).join(format!("{name}.json"))
}

/// The projects of `root`, by name.
pub fn list(root: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(Path::new(root).join(FOLDER)) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .filter_map(|p| p.file_stem().and_then(std::ffi::OsStr::to_str).map(str::to_string))
        .collect();
    names.sort_by(|a, b| groups::natural_cmp(a, b));
    names
}

/// A project file's groups, their files as this PC opens them. A group without a name, or with a
/// name an earlier one has, is left out.
fn parse(root: &str, text: &str) -> Result<Vec<FileGroup>, String> {
    let file: ProjectFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.monun > VERSION {
        return Err("This project file was made by a newer version of Monun.".into());
    }
    let mut out: Vec<FileGroup> = Vec::new();
    for g in file.groups {
        let name = g.name.trim().to_string();
        if name.is_empty() || out.iter().any(|o| o.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let mut files: Vec<String> = Vec::new();
        for f in g.files {
            let f = resolve(root, &f);
            if !f.is_empty() && !files.contains(&f) {
                files.push(f);
            }
        }
        let placed = g.placed.iter().map(|(f, id)| (resolve(root, f), *id)).filter(|(f, _)| files.contains(f)).collect();
        let mut group = FileGroup { name, files, folders: g.folders, placed, ..FileGroup::default() };
        group.tidy_folders();
        out.push(group);
    }
    Ok(out)
}

/// Reads the project `name` of `root`: its file's text and its groups.
fn read(root: &str, name: &str, lang: crate::i18n::Language) -> Result<(String, Vec<FileGroup>), String> {
    let path = project_path(root, name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let groups = parse(root, &text).map_err(|e| format!("{}: {}", path.display(), lang.tr(&e)))?;
    Ok((text, groups))
}

/// A project file's text for `groups`: every file under `root` relative to it.
fn render(root: &str, groups: &[FileGroup]) -> String {
    let file = ProjectFile {
        monun: VERSION,
        groups: groups
            .iter()
            .map(|g| StoredGroup {
                name: g.name.clone(),
                files: g.files.iter().map(|f| stored(root, f)).collect(),
                folders: g.folders.iter().map(shared).collect(),
                placed: g.placed.iter().map(|(f, id)| (stored(root, f), *id)).collect(),
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
    text.push('\n');
    text
}

/// Writes a project file whole: beside it first, then in its place, so it is never left half
/// written.
fn write(root: &str, name: &str, text: &str) -> Result<(), String> {
    write_whole(&project_path(root, name), text)
}

/// Writes a file of the `.monun` folder whole (a project's, the memos'), and the folders it is in:
/// beside it first, then in its place, so it is never left half written. The beside one goes when
/// the move fails.
pub(crate) fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", tmp.display())
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// The project `name` of the root, when there is one: a name an agent gives is checked against
/// the root's projects, so it can never reach a file outside the `.monun` folder.
fn existing(app: &SheetApp, name: &str) -> Result<String, String> {
    let lang = app.ui.language;
    let Some(root) = app.ui.root.clone() else {
        return Err(lang.tr("Choose a working root folder first: File › Options › General.").to_string());
    };
    if list(&root).iter().any(|p| p == name) { Ok(root) } else { Err(lang.tr("There is no project “{name}”.").replace("{name}", name)) }
}

/// A run that keeps nothing (an agent's or a test's: `--control`, or no preferences) starts on
/// This PC with no working root, so it can never write to the user's projects or to the memos of
/// the root's files ([`crate::memos::root_file`]); an agent sets a root of its own
/// (`project.root`).
pub fn detach(app: &mut SheetApp) {
    if app.ui.project.is_some() {
        let set = app.ui.this_pc.take().unwrap_or_default();
        app.ui.groups = set.groups;
        app.ui.group = set.group;
        app.ui.project = None;
    }
    app.ui.root = None;
    app.project_loaded = None;
    app.root_memos = None;
}

/// A project name typed into New Project or Rename Project, trimmed; the reason it can't be used,
/// if any. It names a file, so Windows' rules for file names apply.
pub(crate) fn check_name(app: &SheetApp, name: &str, renaming: Option<&str>) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name for the project.");
    }
    if name.chars().count() > NAME_MAX {
        return Err("A project name has at most 60 characters.");
    }
    if name.chars().any(|c| "\\/:*?\"<>|".contains(c) || c.is_control()) || name.starts_with('.') || name.ends_with('.') {
        return Err("A project name can't have \\ / : * ? \" < > | in it, or start or end with a dot.");
    }
    let stem = name.split('.').next().unwrap_or(name).trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.chars().last().is_some_and(|c| c.is_ascii_digit()));
    if reserved {
        return Err("Windows keeps this name for itself.");
    }
    let taken = app.ui.root.as_deref().map(list).unwrap_or_default();
    if taken.iter().any(|p| Some(p.as_str()) != renaming && p.eq_ignore_ascii_case(name)) {
        return Err("There is already a project with this name.");
    }
    Ok(name.to_string())
}

/// A name for a copy of the group `name` among `taken`: its own, or with " (2)", " (3)"…
fn unique(taken: &[FileGroup], name: &str) -> String {
    let free = |n: &str| !taken.iter().any(|g| g.name.eq_ignore_ascii_case(n));
    if free(name) {
        return name.to_string();
    }
    (2..10_000).map(|n| format!("{name} ({n})")).find(|n| free(n)).unwrap_or_else(|| format!("{name} ({})", taken.len() + 2))
}

/// This PC's place in the groups in use.
fn view_of(project: &str, app: &SheetApp) -> ProjectView {
    ProjectView {
        project: project.to_string(),
        group: app.ui.group.clone(),
        groups: app
            .ui
            .groups
            .iter()
            .map(|g| GroupView {
                name: g.name.clone(),
                active: g.active.clone(),
                used: g.used,
                folded: g.folders.iter().filter(|f| f.folded).map(|f| f.id).collect(),
            })
            .collect(),
    }
}

/// Gives `groups` this PC's place in them; returns the group to use: the one last used here, or
/// else the first.
fn apply_view(groups: &mut [FileGroup], view: Option<&ProjectView>) -> String {
    if let Some(v) = view {
        for g in groups.iter_mut() {
            if let Some(gv) = v.groups.iter().find(|x| x.name == g.name) {
                g.active = gv.active.clone();
                g.used = gv.used;
                for f in &mut g.folders {
                    f.folded = gv.folded.contains(&f.id);
                }
            }
        }
    }
    view.map(|v| v.group.clone())
        .filter(|n| groups.iter().any(|g| &g.name == n))
        .or_else(|| groups.first().map(|g| g.name.clone()))
        .unwrap_or_default()
}

/// The display name of a project: its own, or This PC.
pub fn display(app: &SheetApp, project: Option<&str>) -> String {
    project.map_or_else(|| app.ui.language.tr("This PC").to_string(), str::to_string)
}

/// Reads the project `name` of the root, for a switch to it.
pub(crate) fn open(app: &SheetApp, name: &str) -> Result<Opened, String> {
    let lang = app.ui.language;
    let Some(root) = app.ui.root.clone() else {
        return Err(lang.tr("Choose a working root folder first: File › Options › General.").to_string());
    };
    let (text, groups) = read(&root, name, lang)?;
    Ok(Opened { root, text, groups })
}

/// Puts the project `target` (`None`: This PC, `opened` its read file otherwise) in place of the one
/// in use: its groups become the groups in use. The one left is written and its place kept, or
/// This PC's groups are set aside.
pub(crate) fn swap(app: &mut SheetApp, target: Option<String>, opened: Option<Opened>) {
    match app.ui.project.clone() {
        Some(project) => {
            save_current(app);
            let view = view_of(&project, app);
            app.ui.project_views.retain(|v| v.project != project);
            app.ui.project_views.push(view);
        }
        None => {
            let set = GroupSet { groups: std::mem::take(&mut app.ui.groups), group: std::mem::take(&mut app.ui.group) };
            app.ui.this_pc = Some(set);
        }
    }
    match (target, opened) {
        (Some(name), Some(o)) => {
            let view = app.ui.project_views.iter().find(|v| v.project == name).cloned();
            let mut groups = o.groups;
            app.ui.group = apply_view(&mut groups, view.as_ref());
            app.ui.groups = groups;
            app.ui.project = Some(name.clone());
            app.project_loaded = Some(Loaded { root: o.root, name, seen: Some(o.text), ..Loaded::default() });
        }
        _ => {
            let set = app.ui.this_pc.take().unwrap_or_default();
            app.ui.groups = set.groups;
            app.ui.group = set.group;
            app.ui.project = None;
            app.project_loaded = None;
        }
    }
}

/// At start: the project in use is read again from its file (git may have brought changes), with
/// this PC's place in it as `ui.json` kept it. One that can't be read (no root, the file gone)
/// gives way to This PC's groups, and a message says why.
pub(crate) fn at_start(app: &mut SheetApp) {
    let Some(name) = app.ui.project.clone() else { return };
    match open(app, &name) {
        Ok(o) => {
            let view = view_of(&name, app);
            let mut groups = o.groups;
            app.ui.group = apply_view(&mut groups, Some(&view));
            app.ui.groups = groups;
            app.project_loaded = Some(Loaded { root: o.root, name, seen: Some(o.text), ..Loaded::default() });
        }
        Err(e) => {
            // This PC's place in it is kept for when it can be read again.
            let view = view_of(&name, app);
            app.ui.project_views.retain(|v| v.project != name);
            app.ui.project_views.push(view);
            let set = app.ui.this_pc.take().unwrap_or_default();
            app.ui.groups = set.groups;
            app.ui.group = set.group;
            app.ui.project = None;
            app.project_loaded = None;
            let lang = app.ui.language;
            let text =
                format!("{}\n\n{e}", lang.tr("The project “{name}” could not be opened, so This PC's groups are in use:").replace("{name}", &name));
            app.message = Some((lang.tr("Monun").to_string(), text));
        }
    }
}

/// Writes the project in use to its file, when it is a project of the root read in this run and
/// its groups or their files changed (which group and file are in front, and when each was used,
/// stay on this PC). When the file was changed outside 모눈 since it was read (a git pull), that
/// version is taken, but for the group in use's files: its tabs here.
pub(crate) fn save_current(app: &mut SheetApp) {
    let Some(loaded) = app.project_loaded.clone() else { return };
    if app.ui.project.as_deref() != Some(loaded.name.as_str()) {
        return;
    }
    let path = project_path(&loaded.root, &loaded.name);
    let disk = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        // There, but it can't be read just now (held by a git checkout, not text): left as it is,
        // said so once; a later change tries again.
        Err(e) => {
            let error = format!("{}: {e}", path.display());
            log::warn!("project file not read: {error}");
            if loaded.unreadable.as_deref() != Some(error.as_str()) {
                let lang = app.ui.language;
                let message = format!(
                    "{}\n\n{}\n{error}",
                    lang.tr("The project “{name}” could not be saved:").replace("{name}", &loaded.name),
                    lang.tr("Its file can't be read just now, so it is left as it is.")
                );
                app.message = Some((lang.tr("Monun").to_string(), message));
                if let Some(l) = app.project_loaded.as_mut() {
                    l.unreadable = Some(error);
                }
            }
            return;
        }
    };
    if let Some(l) = app.project_loaded.as_mut() {
        l.unreadable = None;
    }
    if let Some(text) = disk.as_deref()
        && disk != loaded.seen
    {
        match parse(&loaded.root, text) {
            Ok(theirs) => app.ui.groups = merged(app, theirs),
            // A file 모눈 can't read (a git conflict left in it, a newer version's) is left as it
            // is, and said so once.
            Err(e) => {
                if loaded.refused.as_deref() != Some(text) {
                    let lang = app.ui.language;
                    let message = format!(
                        "{}\n\n{}\n{}: {}",
                        lang.tr("The project “{name}” could not be saved:").replace("{name}", &loaded.name),
                        lang.tr("Its file was changed outside Monun and can't be read (a git conflict?), so it is left as it is."),
                        path.display(),
                        lang.tr(&e)
                    );
                    app.message = Some((lang.tr("Monun").to_string(), message));
                    if let Some(l) = app.project_loaded.as_mut() {
                        l.refused = Some(text.to_string());
                    }
                }
                return;
            }
        }
    }
    // The same groups, files and folders (and which file sits in which), however the file is laid
    // out, are left as they are: no change for git to see.
    let same = disk
        .as_deref()
        .and_then(|text| parse(&loaded.root, text).ok())
        .is_some_and(|theirs| theirs.len() == app.ui.groups.len() && theirs.iter().zip(&app.ui.groups).all(|(a, b)| same_kept(a, b)));
    if same {
        set_seen(app, disk);
        return;
    }
    let text = render(&loaded.root, &app.ui.groups);
    match write(&loaded.root, &loaded.name, &text) {
        Ok(()) => set_seen(app, Some(text)),
        Err(e) => {
            let lang = app.ui.language;
            let message = format!("{}\n\n{e}", lang.tr("The project “{name}” could not be saved:").replace("{name}", &loaded.name));
            app.message = Some((lang.tr("Monun").to_string(), message));
            // Not again for every tab opened or closed: the next change tries again.
            set_seen(app, disk);
        }
    }
}

fn set_seen(app: &mut SheetApp, seen: Option<String>) {
    if let Some(l) = app.project_loaded.as_mut() {
        l.seen = seen;
    }
}

/// `theirs` (the project file as changed outside 모눈) with this PC's place in it, and the group in
/// use's files as its tabs here are.
fn merged(app: &SheetApp, mut theirs: Vec<FileGroup>) -> Vec<FileGroup> {
    for g in theirs.iter_mut() {
        if let Some(o) = app.ui.groups.iter().find(|o| o.name == g.name) {
            g.active = o.active.clone();
            g.used = o.used;
            for f in &mut g.folders {
                f.folded = o.folder(f.id).is_some_and(|x| x.folded);
            }
        }
    }
    if let Some(ours) = app.ui.groups.iter().find(|g| g.name == app.ui.group).cloned() {
        match theirs.iter_mut().find(|g| g.name == ours.name) {
            Some(g) => {
                // Its tabs here, each in the folder it sits in here (a folder theirs lacks kept).
                for id in ours.placed.values() {
                    if g.folder(*id).is_none()
                        && let Some(f) = ours.folder(*id)
                    {
                        g.folders.push(f.clone());
                    }
                }
                g.files = ours.files;
                g.placed = ours.placed;
                g.tidy_folders();
            }
            None => theirs.push(ours),
        }
    }
    theirs
}

/// Copies the group `group` (of the project in use) into the project `target` (`None`: This PC),
/// under its name or one free there. Returns the name it got.
pub(crate) fn send_group(app: &mut SheetApp, group: &str, target: Option<&str>) -> Result<String, String> {
    let lang = app.ui.language;
    let Some(g) = app.ui.groups.iter().find(|g| g.name == group).cloned() else {
        return Err(lang.tr("There is no group “{name}”.").replace("{name}", group));
    };
    if app.ui.project.as_deref() == target {
        return Err(lang.tr("The group is in that project already.").to_string());
    }
    let copy = |taken: &[FileGroup]| FileGroup {
        name: unique(taken, &g.name),
        files: g.files.clone(),
        folders: g.folders.clone(),
        placed: g.placed.clone(),
        ..FileGroup::default()
    };
    match target {
        None => {
            let set = app.ui.this_pc.get_or_insert_with(GroupSet::default);
            let c = copy(&set.groups);
            let name = c.name.clone();
            set.groups.push(c);
            if let Some(save) = &app.services.save_ui {
                save(&app.ui);
            }
            Ok(name)
        }
        Some(project) => {
            let root = existing(app, project)?;
            let (_, mut groups) = read(&root, project, lang)?;
            let c = copy(&groups);
            let name = c.name.clone();
            groups.push(c);
            write(&root, project, &render(&root, &groups))?;
            Ok(name)
        }
    }
}

/// The projects a group can be sent to: This PC and the root's, but the one in use.
pub(crate) fn targets(app: &SheetApp) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    if app.ui.project.is_some() {
        out.push((display(app, None), None));
    }
    for p in app.ui.root.as_deref().map(list).unwrap_or_default() {
        if app.ui.project.as_deref() != Some(p.as_str()) {
            out.push((p.clone(), Some(p)));
        }
    }
    out
}

/// Sets the working root folder (`None`: none). A project of the old root in use gives way to This
/// PC first (unsaved files asked about); until then it keeps to the root it was read from.
fn set_root(app: &mut SheetApp, root: Option<String>) -> Result<(), String> {
    let lang = app.ui.language;
    if let Some(r) = &root
        && !Path::new(r).is_dir()
    {
        return Err(lang.tr("That folder doesn't exist.").to_string());
    }
    if root == app.ui.root {
        return Ok(());
    }
    app.ui.root = root;
    if let Some(save) = &app.services.save_ui {
        save(&app.ui);
    }
    if app.ui.project.is_some() {
        groups::switch(app, Target::Project(None));
    }
    Ok(())
}

/// The project commands (the title bar's project menu, the dialogs, agents).
pub(crate) fn command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let text = |k: &str| p.get(k).and_then(Json::as_str).map(str::trim).map(str::to_string);
    let lang = app.ui.language;
    let r = match id {
        "project.root" => {
            let root = text("path").filter(|s| !s.is_empty());
            set_root(app, root).map(|()| json!({"root": app.ui.root}))
        }
        "project.list" => Ok(json!({
            "root": app.ui.root,
            "projects": app.ui.root.as_deref().map(list).unwrap_or_default(),
            "current": app.ui.project,
        })),
        "project.new" => {
            let Some(root) = app.ui.root.clone() else {
                return Some(Err(lang.tr("Choose a working root folder first: File › Options › General.").to_string()));
            };
            match check_name(app, &text("name").unwrap_or_default(), None) {
                Err(e) => Err(e.to_string()),
                Ok(name) => write(&root, &name, &render(&root, &[])).map(|()| {
                    groups::switch(app, Target::Project(Some(name.clone())));
                    json!({"project": name})
                }),
            }
        }
        "project.switch" => {
            let name = text("name").filter(|s| !s.is_empty());
            if name == app.ui.project {
                Ok(json!({"project": name}))
            } else if let Some(n) = &name
                && !app.ui.root.as_deref().map(list).unwrap_or_default().contains(n)
            {
                Err(lang.tr("There is no project “{name}”.").replace("{name}", n))
            } else {
                groups::switch(app, Target::Project(name.clone()));
                Ok(json!({"project": app.ui.project}))
            }
        }
        "project.rename" => rename(app, &text("name").unwrap_or_default(), &text("to").unwrap_or_default()),
        "project.delete" => delete(app, &text("name").unwrap_or_default()),
        "project.reload" => {
            // The file as it is now (a git pull), but for the group in use's files: its tabs.
            if let Some(l) = app.project_loaded.as_mut() {
                l.seen = None;
                // Asked for: a file that still can't be read is said so again.
                l.unreadable = None;
            }
            save_current(app);
            // And the memos of the root's files, as their file is now.
            crate::memos::root_file::reload(app);
            Ok(json!({"project": app.ui.project}))
        }
        "group.sendToProject" => {
            let group = text("group").unwrap_or_default();
            let target = text("project").filter(|s| !s.is_empty());
            send_group(app, &group, target.as_deref()).map(|name| json!({"group": name, "project": target}))
        }
        _ => return None,
    };
    Some(r)
}

/// Renames the project `name` (its file) to `to`.
fn rename(app: &mut SheetApp, name: &str, to: &str) -> Result<Json, String> {
    let root = existing(app, name)?;
    let to = check_name(app, to, Some(name)).map_err(str::to_string)?;
    if to != name {
        let (from, dest) = (project_path(&root, name), project_path(&root, &to));
        std::fs::rename(&from, &dest).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    for v in app.ui.project_views.iter_mut().filter(|v| v.project == name) {
        v.project = to.clone();
    }
    if app.ui.project.as_deref() == Some(name) {
        app.ui.project = Some(to.clone());
        if let Some(l) = app.project_loaded.as_mut() {
            l.name = to.clone();
        }
    }
    if let Some(save) = &app.services.save_ui {
        save(&app.ui);
    }
    Ok(json!({"project": to}))
}

/// Deletes the project `name`: its file only, never the documents. Not the project in use.
fn delete(app: &mut SheetApp, name: &str) -> Result<Json, String> {
    let lang = app.ui.language;
    if app.ui.project.as_deref() == Some(name) {
        return Err(lang.tr("The project in use can't be deleted.").to_string());
    }
    let root = existing(app, name)?;
    let path = project_path(&root, name);
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    app.ui.project_views.retain(|v| v.project != name);
    if let Some(save) = &app.services.save_ui {
        save(&app.ui);
    }
    Ok(json!({"deleted": name}))
}

/// Asks for the working root folder and sets it.
pub(crate) fn choose_root(app: &mut SheetApp) {
    let picked = app.services.pick_folder.as_ref().and_then(|pick| pick());
    if let Some(path) = picked {
        app.run_or_alert("project.root", json!({"path": path}));
    }
}

/// The project button in the title bar, left of the group button: the project in use, and a menu
/// to open another, make one, rename, reload or delete one, or choose the working root folder.
/// The groups show as ever, the project's own.
pub fn button(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.ui.language;
    let name = display(app, app.ui.project.as_deref());
    let font = theme::ui_font(12.5);
    let shown = crate::file_tabs::fit(ui, &name, &font, NAME_MAX_W);
    let tw = ui.painter().layout_no_wrap(shown.clone(), font.clone(), t.text).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(24.0 + tw + 18.0, 23.0), Sense::click());
    let label = lang.tr("Project: {name}").replace("{name}", &name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(r, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let icon = Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(14.0, 14.0));
    icons::paint(ui.painter(), icon, Icon::Project, t.text_dim);
    ui.painter().text(pos2(icon.right() + 5.0, r.center().y), Align2::LEFT_CENTER, &shown, font, t.text);
    let arrow = Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(9.0, 9.0));
    icons::paint(ui.painter(), arrow, Icon::Chevron, t.text_dim);
    let resp = resp
        .on_hover_text(lang.tr("Project: groups kept in the working root folder, to share with git. This PC is the groups kept on this PC alone."));
    egui::Popup::menu(&resp).show(|ui| menu(app, ui));
}

fn menu(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    ui.set_min_width(240.0);
    let projects = app.ui.root.as_deref().map(list).unwrap_or_default();
    let current = app.ui.project.clone();
    let mut chosen: Option<Option<String>> = None;
    let this_pc = ui.add(egui::Button::selectable(current.is_none(), display(app, None)));
    if this_pc.on_hover_text(lang.tr("The groups kept on this PC alone")).clicked() && current.is_some() {
        chosen = Some(None);
    }
    for p in &projects {
        let on = current.as_deref() == Some(p.as_str());
        if ui.add(egui::Button::selectable(on, p)).clicked() && !on {
            chosen = Some(Some(p.clone()));
        }
    }
    if app.ui.root.is_none() {
        ui.label(egui::RichText::new(lang.tr("Choose a working root folder to keep projects in it.")).small().weak());
    }
    ui.separator();
    if ui.button(lang.tr("New Project…")).clicked() {
        if app.ui.root.is_none() {
            choose_root(app);
        }
        if app.ui.root.is_some() {
            app.open_dialog("newProject", json!({}));
        }
    }
    if let Some(cur) = &current {
        if ui.button(lang.tr("Rename Project…")).clicked() {
            app.open_dialog("renameProject", json!({"name": cur}));
        }
        if ui.button(lang.tr("Reload Project")).on_hover_text(lang.tr("Read the project file again, after a git pull")).clicked() {
            app.run_or_alert("project.reload", json!({}));
        }
    }
    let others: Vec<&String> = projects.iter().filter(|p| current.as_deref() != Some(p.as_str())).collect();
    if !others.is_empty() {
        ui.menu_button(lang.tr("Delete Project"), |ui| {
            for name in others {
                if ui.button(name).clicked() {
                    app.open_dialog("deleteProject", json!({"name": name}));
                }
            }
        });
    }
    ui.separator();
    if app.services.pick_folder.is_some() && ui.button(lang.tr("Choose Working Root Folder…")).clicked() {
        choose_root(app);
    }
    if let Some(root) = &app.ui.root {
        ui.label(egui::RichText::new(root).small().weak());
    }
    if let Some(choice) = chosen {
        ui.close();
        groups::switch(app, Target::Project(choice));
    }
}

/// The project dialogs: New Project, Rename Project, Delete Project.
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let text = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    match d.name() {
        "newProject" | "renameProject" => {
            let renaming = (d.name() == "renameProject").then(|| text(d, "was"));
            ui.add_space(4.0);
            ui.label(lang.tr("Project name:"));
            let mut name = text(d, "name");
            let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).accessible_name(lang.tr("Project name:"));
            if d.values.get("focused").and_then(Json::as_bool) != Some(true) {
                field.request_focus();
                d.values.insert("focused".into(), json!(true));
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            d.values.insert("name".into(), json!(name));
            if renaming.is_none() {
                ui.add_space(4.0);
                let hint =
                    lang.tr("Kept in the working root folder's .monun folder. The open files close; the new project starts with an empty group.");
                ui.label(egui::RichText::new(hint).color(t.text_dim));
            }
            let error = text(d, "error");
            if !error.is_empty() {
                ui.label(egui::RichText::new(lang.tr(&error)).color(t.danger));
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
                let r = match &renaming {
                    Some(was) => app.run("project.rename", json!({"name": was, "to": name})),
                    None => app.run("project.new", json!({"name": name})),
                };
                match r {
                    Ok(_) => *open = false,
                    Err(e) => {
                        d.values.insert("error".into(), json!(e));
                    }
                }
            }
        }
        "deleteProject" => {
            let name = text(d, "name");
            ui.add_space(4.0);
            ui.label(
                lang.tr("Delete the project “{name}”? Only its file in the .monun folder goes; its documents stay where they are.")
                    .replace("{name}", &name),
            );
            ui.add_space(8.0);
            let row = [DialogButton::discard("Delete"), DialogButton::neutral("Cancel")];
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => {
                    *open = false;
                    app.run_or_alert("project.delete", json!({"name": name}));
                }
                Some(_) => *open = false,
                None => {}
            }
        }
        _ => *open = false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn paths_under_the_root_are_kept_relative_with_slashes() {
        let root = r"C:\설계문서";
        assert_eq!(stored(root, r"C:\설계문서\a프로젝트\REQ-037\설계.md"), "a프로젝트/REQ-037/설계.md");
        assert_eq!(stored(root, r"c:\설계문서\A.md"), "A.md", "regardless of case");
        assert_eq!(stored(r"C:\설계문서\", r"C:\설계문서\b\c.xlsx"), "b/c.xlsx", "a root with a trailing separator");
        assert_eq!(stored(root, r"D:\밖\x.xlsx"), r"D:\밖\x.xlsx", "elsewhere, as it is");
        assert_eq!(stored(root, r"C:\설계문서2\x.md"), r"C:\설계문서2\x.md", "a folder named alike isn't under it");
        assert_eq!(stored(root, r"C:\설계문서\..\x.md"), r"C:\설계문서\..\x.md");
        assert_eq!(resolve(r"E:\work\설계문서", "a프로젝트/REQ-037/설계.md"), r"E:\work\설계문서\a프로젝트\REQ-037\설계.md");
        assert_eq!(resolve(root, r"D:\밖\x.xlsx"), r"D:\밖\x.xlsx");
    }

    #[cfg(not(windows))]
    #[test]
    fn paths_under_the_root_are_kept_relative_with_slashes() {
        assert_eq!(stored("/home/u/docs", "/home/u/docs/a/b.md"), "a/b.md");
        assert_eq!(stored("/home/u/docs", "/home/u/other/b.md"), "/home/u/other/b.md");
        assert_eq!(resolve("/srv/docs", "a/b.md"), "/srv/docs/a/b.md");
    }

    #[test]
    fn a_project_file_round_trips_and_a_newer_one_is_refused() {
        let root = std::env::temp_dir().join(format!("monun-project-unit-{}", std::process::id()));
        let root = root.to_string_lossy().into_owned();
        let inside = Path::new(&root).join("a").join("x.md").to_string_lossy().into_owned();
        let groups = vec![
            FileGroup {
                name: "REQ-1".into(),
                files: vec![inside.clone(), "/elsewhere/y.xlsx".into()],
                active: Some(inside.clone()),
                used: Some(5),
                ..FileGroup::default()
            },
            FileGroup { name: "req-1".into(), files: vec![], ..FileGroup::default() },
        ];
        let text = render(&root, &groups);
        assert!(text.contains("\"a/x.md\""), "{text}");
        assert!(!text.contains("active") && !text.contains("used"), "this PC's place stays out of it");
        let back = parse(&root, &text).unwrap();
        assert_eq!(back.len(), 1, "a name an earlier group has is left out");
        assert_eq!(back[0].files, [inside, "/elsewhere/y.xlsx".to_string()]);
        assert!(parse(&root, r#"{"monun": 2, "groups": []}"#).is_err());
        assert!(parse(&root, "not json").is_err());
    }

    #[test]
    fn a_groups_folders_go_in_its_project_file_with_its_files_relative() {
        let root = std::env::temp_dir().join(format!("monun-project-folders-{}", std::process::id()));
        let root = root.to_string_lossy().into_owned();
        let inside = Path::new(&root).join("api").join("pay.http").to_string_lossy().into_owned();
        let mut g = FileGroup { name: "REQ-1".into(), files: vec![inside.clone()], ..FileGroup::default() };
        let api = g.add_folder("API", None);
        g.place(&inside, Some(api));
        let text = render(&root, std::slice::from_ref(&g));
        assert!(text.contains("\"api/pay.http\": 1"), "{text}");
        let back = parse(&root, &text).unwrap();
        assert_eq!((back[0].folders.clone(), back[0].folder_of(&inside)), (g.folders.clone(), Some(api)));
        // A file the group no longer lists has no place in it.
        let stale = text.replace("\"files\": [\n        \"api/pay.http\"\n      ]", "\"files\": []");
        assert!(parse(&root, &stale).unwrap()[0].placed.is_empty(), "{stale}");
    }

    #[test]
    fn a_copy_takes_a_free_name() {
        let taken = vec![FileGroup { name: "A".into(), ..FileGroup::default() }, FileGroup { name: "a (2)".into(), ..FileGroup::default() }];
        assert_eq!(unique(&taken, "B"), "B");
        assert_eq!(unique(&taken, "A"), "A (3)");
    }
}
