//! Folders in a file group ([`crate::groups`]): in the group panel a group's files sort into
//! folders, and folders into folders. A folder also keeps settings for the API requests in it and
//! in the folders within it: variables (`{{Name}}` in a request) and headers every request sends,
//! the innermost folder's over those of the folders it sits in.
//!
//! A group keeps its folders and which folder each of its files sits in (by path); a file not
//! placed sits at the group's top. A file of the group in use never saved goes in a folder by its
//! document until it is saved ([`SheetApp::folder_pending`]), and a file saved under a new name
//! keeps its folder ([`follow_documents`]).

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::groups::{FileGroup, natural_cmp};
use crate::theme::Tokens;
use crate::widgets::DialogButton;

/// The longest folder name.
pub const NAME_MAX: usize = 60;

/// How deep folders go at most: a guard, as moves never make a loop.
pub const DEPTH_MAX: usize = 32;

/// A folder of a group.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Folder {
    /// Unique in its group.
    pub id: u64,
    pub name: String,
    /// The folder it sits in; `None` at the group's top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    /// Folded in the group panel: what it holds is not listed.
    #[serde(skip_serializing_if = "is_false")]
    pub folded: bool,
    /// Variables of the API requests in it: `{{Name}}` reads as the value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vars: Vec<Setting>,
    /// Headers every API request in it sends (a request's own header of that name wins).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Setting>,
    /// Its API requests (and those of the folders within it) leave certificates unchecked: a
    /// development server's own.
    #[serde(skip_serializing_if = "is_false")]
    pub insecure: bool,
}

/// A folder's variable or header: a name and a value; a secret one shows as dots.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Setting {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "is_false")]
    pub secret: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl FileGroup {
    /// The folder `id`.
    pub fn folder(&self, id: u64) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    fn folder_mut(&mut self, id: u64) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    /// The folders right inside `parent` (`None`: at the group's top), by name.
    pub fn subfolders(&self, parent: Option<u64>) -> Vec<&Folder> {
        let mut list: Vec<&Folder> = self.folders.iter().filter(|f| f.parent == parent && Some(f.id) != parent).collect();
        list.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        list
    }

    /// The folder `id` and the folders it sits in, innermost first.
    pub fn folder_chain(&self, id: u64) -> Vec<&Folder> {
        let mut chain: Vec<&Folder> = Vec::new();
        let mut at = Some(id);
        while let Some(id) = at {
            let Some(f) = self.folder(id) else { break };
            if chain.len() >= DEPTH_MAX || chain.iter().any(|c| c.id == id) {
                break;
            }
            chain.push(f);
            at = f.parent;
        }
        chain
    }

    /// The folder's names from the group's top down: `Payments / Live`.
    pub fn folder_label(&self, id: u64) -> String {
        let names: Vec<&str> = self.folder_chain(id).iter().rev().map(|f| f.name.as_str()).collect();
        names.join(" / ")
    }

    /// Every folder with its depth, each followed by the folders within it (as the panel lists them).
    pub fn folder_tree(&self) -> Vec<(&Folder, usize)> {
        fn walk<'a>(g: &'a FileGroup, parent: Option<u64>, depth: usize, out: &mut Vec<(&'a Folder, usize)>) {
            if depth >= DEPTH_MAX {
                return;
            }
            for f in g.subfolders(parent) {
                out.push((f, depth));
                walk(g, Some(f.id), depth + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(self, None, 0, &mut out);
        out
    }

    /// Whether the folder `id` is `outer` or sits somewhere within it.
    pub fn is_within(&self, id: u64, outer: u64) -> bool {
        self.folder_chain(id).iter().any(|f| f.id == outer)
    }

    /// Makes a folder `name` in `parent` (`None`: at the group's top); returns its id.
    pub fn add_folder(&mut self, name: &str, parent: Option<u64>) -> u64 {
        let id = self.folders.iter().map(|f| f.id).max().unwrap_or(0).saturating_add(1);
        let parent = parent.filter(|p| self.folder(*p).is_some());
        self.folders.push(Folder { id, name: name.to_string(), parent, ..Folder::default() });
        id
    }

    pub fn rename_folder(&mut self, id: u64, name: &str) {
        if let Some(f) = self.folder_mut(id) {
            f.name = name.to_string();
        }
    }

    /// Deletes the folder `id`: what it holds (files and folders) moves up to where it sat. The
    /// files themselves stay where they are.
    pub fn delete_folder(&mut self, id: u64) {
        let Some(up) = self.folder(id).map(|f| f.parent) else { return };
        for f in &mut self.folders {
            if f.parent == Some(id) {
                f.parent = up;
            }
        }
        match up {
            Some(up) => {
                for v in self.placed.values_mut() {
                    if *v == id {
                        *v = up;
                    }
                }
            }
            None => self.placed.retain(|_, v| *v != id),
        }
        self.folders.retain(|f| f.id != id);
    }

    /// Moves the folder `id` into `to` (`None`: to the group's top). Never into itself or a folder
    /// within it: then nothing moves and `false` comes back.
    pub fn move_folder(&mut self, id: u64, to: Option<u64>) -> bool {
        if to.is_some_and(|t| self.folder(t).is_none() || self.is_within(t, id)) {
            return false;
        }
        match self.folder_mut(id) {
            Some(f) => {
                f.parent = to;
                true
            }
            None => false,
        }
    }

    /// Puts the file `path` in the folder `folder` (`None`, or a folder gone: at the group's top).
    pub fn place(&mut self, path: &str, folder: Option<u64>) {
        match folder.filter(|f| self.folder(*f).is_some()) {
            Some(f) => {
                self.placed.insert(path.to_string(), f);
            }
            None => {
                self.placed.remove(path);
            }
        }
    }

    /// The folder the file `path` sits in; `None` at the group's top.
    pub fn folder_of(&self, path: &str) -> Option<u64> {
        self.placed.get(path).copied().filter(|f| self.folder(*f).is_some())
    }

    /// Drops what no longer holds: the place of a file that left the group, a folder in a folder
    /// gone (it goes to the top).
    pub fn tidy_folders(&mut self) {
        let ids: Vec<u64> = self.folders.iter().map(|f| f.id).collect();
        let files = &self.files;
        self.placed.retain(|p, f| ids.contains(f) && files.contains(p));
        for f in &mut self.folders {
            if f.parent.is_some_and(|p| p == f.id || !ids.contains(&p)) {
                f.parent = None;
            }
        }
        // A loop (never made here, but a file edited by hand may hold one) is broken at its top.
        let looped: Vec<u64> = self.folders.iter().map(|f| f.id).filter(|id| self.folder_chain(*id).len() >= DEPTH_MAX).collect();
        for f in &mut self.folders {
            if looped.contains(&f.id) {
                f.parent = None;
            }
        }
    }

    /// A folder name typed into New Folder or Rename Folder, trimmed; the reason it can't be used
    /// in `parent`, if any. Folders side by side have names of their own.
    pub fn check_folder_name(&self, name: &str, parent: Option<u64>, renaming: Option<u64>) -> Result<String, &'static str> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Type a name for the folder.");
        }
        if name.chars().count() > NAME_MAX {
            return Err("A folder name has at most 60 characters.");
        }
        let lower = name.to_lowercase();
        if self.folders.iter().any(|f| f.parent == parent && Some(f.id) != renaming && f.name.to_lowercase() == lower) {
            return Err("There is already a folder with this name here.");
        }
        Ok(name.to_string())
    }
}

/// Keeps the folders of the group `i` (the one in use) in step with its documents: a file saved
/// under a new name keeps its folder, and one put in a folder before it was ever saved goes there
/// once it is. Returns whether a file's folder changed.
pub(crate) fn follow_documents(app: &mut SheetApp, i: usize) -> bool {
    let docs: Vec<(u64, Option<String>)> = app.session.documents().iter().map(|d| (d.uid, d.path.clone())).collect();
    let mut changed = false;
    if let Some(g) = app.ui.groups.get_mut(i) {
        for (uid, path) in &docs {
            let Some(path) = path else { continue };
            if let Some(Some(was)) = app.doc_paths.get(uid)
                && was != path
                && let Some(f) = g.placed.remove(was)
            {
                g.placed.insert(path.clone(), f);
                changed = true;
            }
            if let Some(f) = app.folder_pending.remove(uid) {
                g.place(path, Some(f));
                changed = true;
            }
        }
    }
    if !app.folder_pending.is_empty() {
        app.folder_pending.retain(|uid, _| docs.iter().any(|(u, _)| u == uid));
    }
    if app.doc_paths.len() != docs.len() || docs.iter().any(|(u, p)| app.doc_paths.get(u) != Some(p)) {
        app.doc_paths = docs.into_iter().collect();
    }
    changed
}

/// Where a file or a folder goes from its Move to Folder menu.
pub(crate) enum MoveTo {
    /// Into this folder; `None`: to the group's top.
    Folder(Option<u64>),
    /// Into a new folder, made at the group's top.
    NewFolder,
}

/// The Move to Folder menu of a file or a folder: the group's top, every folder (but, for a folder
/// moved, itself and the folders within it), and for a file New Folder…. `now` is where it is.
pub(crate) fn move_menu(ui: &mut egui::Ui, g: &FileGroup, lang: crate::i18n::Language, now: Option<u64>, moving: Option<u64>) -> Option<MoveTo> {
    let mut chosen = None;
    ui.menu_button(lang.tr("Move to Folder"), |ui| {
        ui.set_min_width(180.0);
        if ui.add_enabled(now.is_some(), egui::Button::new(lang.tr("Top of the Group"))).clicked() {
            chosen = Some(MoveTo::Folder(None));
        }
        for (f, depth) in g.folder_tree() {
            if moving.is_some_and(|m| g.is_within(f.id, m)) {
                continue;
            }
            let label = format!("{}{}", "    ".repeat(depth), f.name);
            if ui.add_enabled(now != Some(f.id), egui::Button::new(label)).clicked() {
                chosen = Some(MoveTo::Folder(Some(f.id)));
            }
        }
        if moving.is_none() {
            ui.separator();
            if ui.button(lang.tr("New Folder…")).clicked() {
                chosen = Some(MoveTo::NewFolder);
            }
        }
    });
    chosen
}

/// The folder dialogs: New Folder (in a group or a folder, and for a file to go into), Rename
/// Folder and Delete Folder.
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let text = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    let group = text(d, "group");
    let id = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_u64);
    let Some(g) = app.ui.groups.iter().find(|g| g.name == group) else {
        *open = false;
        return;
    };
    match d.name() {
        "newFolder" | "renameFolder" => {
            let renaming = if d.name() == "renameFolder" { id(d, "id") } else { None };
            let parent = match renaming {
                Some(r) => g.folder(r).and_then(|f| f.parent),
                None => id(d, "parent"),
            };
            ui.add_space(4.0);
            let place = match parent {
                Some(p) => g.folder_label(p),
                None => lang.tr("the group “{name}”").replace("{name}", &group),
            };
            ui.label(egui::RichText::new(lang.tr("In {place}").replace("{place}", &place)).color(t.text_dim));
            ui.add_space(4.0);
            ui.label(lang.tr("Folder name:"));
            let mut name = text(d, "name");
            let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).accessible_name(lang.tr("Folder name:"));
            if d.values.get("focused").and_then(Json::as_bool) != Some(true) {
                field.request_focus();
                d.values.insert("focused".into(), json!(true));
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            d.values.insert("name".into(), json!(name));
            let error = text(d, "error");
            if !error.is_empty() {
                ui.label(egui::RichText::new(lang.tr(&error)).color(t.danger));
            }
            ui.add_space(8.0);
            let row = [DialogButton::confirm("OK"), DialogButton::neutral("Cancel")];
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => submit(app, d, open, (parent, renaming)),
                Some(_) => *open = false,
                None if enter => submit(app, d, open, (parent, renaming)),
                None => {}
            }
        }
        "deleteFolder" => {
            let Some(f) = id(d, "id").and_then(|i| g.folder(i)) else {
                *open = false;
                return;
            };
            let id = f.id;
            ui.add_space(4.0);
            ui.label(lang.tr("Delete the folder “{name}”?").replace("{name}", &f.name));
            ui.label(egui::RichText::new(lang.tr("What it holds moves up a level. The files themselves stay where they are.")).color(t.text_dim));
            if !f.vars.is_empty() || !f.headers.is_empty() {
                ui.label(egui::RichText::new(lang.tr("Its variables and headers go with it.")).color(t.text_dim));
            }
            ui.add_space(8.0);
            let row = [DialogButton::discard("Delete"), DialogButton::neutral("Cancel")];
            match crate::widgets::dialog_buttons(ui, &row) {
                Some(0) => {
                    *open = false;
                    if let Some(g) = app.ui.groups.iter_mut().find(|g| g.name == group) {
                        g.delete_folder(id);
                    }
                    app.folder_pending.retain(|_, f| *f != id);
                    crate::groups::persist(app);
                }
                Some(_) => *open = false,
                None => {}
            }
        }
        _ => *open = false,
    }
}

/// OK in New Folder or Rename Folder: the name is checked, then the folder made (and a file put
/// in it, when it was made for one) or renamed.
fn submit(app: &mut SheetApp, d: &mut Dialog, open: &mut bool, (parent, renaming): (Option<u64>, Option<u64>)) {
    let typed = d.values.get("name").and_then(Json::as_str).unwrap_or_default().to_string();
    let group = d.values.get("group").and_then(Json::as_str).unwrap_or_default().to_string();
    let path = d.values.get("path").and_then(Json::as_str).map(str::to_string);
    let doc = d.values.get("doc").and_then(Json::as_u64);
    let Some(g) = app.ui.groups.iter_mut().find(|g| g.name == group) else {
        *open = false;
        return;
    };
    let name = match g.check_folder_name(&typed, parent, renaming) {
        Ok(name) => name,
        Err(e) => {
            d.values.insert("error".into(), json!(e));
            return;
        }
    };
    *open = false;
    match renaming {
        Some(r) => g.rename_folder(r, &name),
        None => {
            let made = g.add_folder(&name, parent);
            match (path, doc) {
                (Some(path), _) => g.place(&path, Some(made)),
                (None, Some(doc)) => {
                    app.folder_pending.insert(doc, made);
                }
                (None, None) => {}
            }
        }
    }
    crate::groups::persist(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> FileGroup {
        FileGroup { name: "G".into(), files: vec!["/a.md".into(), "/b.http".into(), "/c.sql".into()], ..FileGroup::default() }
    }

    #[test]
    fn deleting_a_folder_moves_what_it_holds_up_a_level() {
        let mut g = group();
        let api = g.add_folder("API", None);
        let live = g.add_folder("Live", Some(api));
        let deep = g.add_folder("Deep", Some(live));
        g.place("/a.md", Some(api));
        g.place("/b.http", Some(live));
        g.delete_folder(live);
        assert_eq!(g.folder_of("/b.http"), Some(api), "the file moves to the folder it sat in");
        assert_eq!(g.folder(deep).and_then(|f| f.parent), Some(api), "and so does the folder");
        g.delete_folder(api);
        assert_eq!((g.folder_of("/a.md"), g.folder_of("/b.http")), (None, None), "at the top, a file is placed nowhere");
        assert_eq!(g.folder(deep).and_then(|f| f.parent), None);
        assert_eq!(g.folders.len(), 1);
    }

    #[test]
    fn a_folder_never_moves_into_itself() {
        let mut g = group();
        let a = g.add_folder("A", None);
        let b = g.add_folder("B", Some(a));
        assert!(!g.move_folder(a, Some(b)), "into a folder within it");
        assert!(!g.move_folder(a, Some(a)), "into itself");
        assert!(g.move_folder(b, None));
        assert!(g.move_folder(a, Some(b)));
        assert_eq!(g.folder_label(a), "B / A");
        assert_eq!(g.folder_tree().iter().map(|(f, d)| (f.name.as_str(), *d)).collect::<Vec<_>>(), [("B", 0), ("A", 1)]);
    }

    #[test]
    fn a_file_that_left_the_group_loses_its_place() {
        let mut g = group();
        let a = g.add_folder("A", None);
        g.place("/a.md", Some(a));
        g.place("/c.sql", Some(a));
        g.files.retain(|f| f != "/c.sql");
        g.tidy_folders();
        assert_eq!(g.placed.len(), 1);
        g.place("/a.md", Some(99));
        assert_eq!(g.folder_of("/a.md"), None, "a folder gone puts it at the top");
    }

    #[test]
    fn names_side_by_side_differ() {
        let mut g = group();
        let a = g.add_folder("API", None);
        assert_eq!(g.check_folder_name("  api ", None, None), Err("There is already a folder with this name here."));
        assert_eq!(g.check_folder_name("API", Some(a), None), Ok("API".into()), "within another folder it may");
        assert_eq!(g.check_folder_name("API", None, Some(a)), Ok("API".into()), "renamed as it is");
        assert!(g.check_folder_name(" ", None, None).is_err());
        assert!(g.check_folder_name(&"x".repeat(61), None, None).is_err());
    }

    #[test]
    fn a_group_without_folders_is_written_as_before() {
        let g = group();
        let v = serde_json::to_value(&g).unwrap();
        assert!(v.get("folders").is_none() && v.get("placed").is_none(), "{v}");
        let mut g = group();
        let a = g.add_folder("API", None);
        g.place("/b.http", Some(a));
        let back: FileGroup = serde_json::from_value(serde_json::to_value(&g).unwrap()).unwrap();
        assert_eq!(back, g);
    }
}
