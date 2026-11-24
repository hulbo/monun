//! Find File (Ctrl+Shift+R), as Eclipse's Open Resource: a name typed finds the files of every
//! group (the group in use's open files, those never saved too, and the files the others keep), and
//! the one chosen comes to the front at once, its group opened first when it is another's.
//!
//! A name is found anywhere in a file's name, regardless of case; words apart must all be in it. With
//! `*` (any text) or `?` (any one character) the name must start as the pattern does, as Eclipse
//! reads it. Names that start with what was typed come first, then the group in use's files.

use egui::{Align2, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::groups::{self, Target};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// The most files the list shows.
const SHOWN_MAX: usize = 200;

/// A file found: the group it is in, its window (an open file of the group in use), its path
/// (`None` for one never saved), its name and its folder in the group.
#[derive(Clone, Debug, PartialEq)]
struct Hit {
    group: String,
    window: Option<u64>,
    path: Option<String>,
    name: String,
    folder: Option<String>,
    in_use: bool,
}

/// A path's file name.
fn name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(path)
}

/// Every file of every group: the group in use's open files first, in the order the panel lists
/// them, then each other group's files.
fn every_file(app: &mut SheetApp) -> Vec<Hit> {
    let current = groups::current(app);
    let mut out = Vec::new();
    let folder = |g: &groups::FileGroup, path: Option<&str>| path.and_then(|p| g.folder_of(p)).map(|id| g.folder_label(id));
    if let Some(g) = app.ui.groups.iter().find(|g| g.name == current) {
        for f in groups::open_files_listed(app) {
            out.push(Hit {
                group: current.clone(),
                window: Some(f.window),
                folder: folder(g, f.path.as_deref()),
                path: f.path,
                name: f.title,
                in_use: true,
            });
        }
    }
    for g in app.ui.groups.iter().filter(|g| g.name != current) {
        for p in &g.files {
            out.push(Hit {
                group: g.name.clone(),
                window: None,
                path: Some(p.clone()),
                name: name_of(p).to_string(),
                folder: folder(g, Some(p)),
                in_use: false,
            });
        }
    }
    out
}

/// Whether `name` starts as `pattern` does, `*` standing for any text and `?` for any one character
/// (the rest of the name may be anything); both lower-cased.
fn glob_start(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => true,
        Some(('*', rest)) => (0..=name.len()).any(|i| name.get(i..).is_some_and(|n| glob_start(rest, n))),
        Some((p, rest)) => name.split_first().is_some_and(|(c, n)| (*p == '?' || p == c) && glob_start(rest, n)),
    }
}

/// How well `name` answers `typed`: `None` when it doesn't; `Some(0)` when it starts with it,
/// `Some(1)` when it only holds it.
fn score(typed: &str, name: &str) -> Option<u8> {
    let typed = typed.trim().to_lowercase();
    let name = name.to_lowercase();
    if typed.is_empty() {
        return Some(1);
    }
    if typed.contains(['*', '?']) {
        let p: Vec<char> = typed.chars().filter(|c| !c.is_whitespace()).collect();
        let n: Vec<char> = name.chars().collect();
        return glob_start(&p, &n).then_some(0);
    }
    let words: Vec<&str> = typed.split_whitespace().collect();
    if !words.iter().all(|w| name.contains(w)) {
        return None;
    }
    Some(if words.first().is_some_and(|w| name.starts_with(w)) { 0 } else { 1 })
}

/// The files `typed` finds, the best first: those starting with it, then the group in use's, then
/// by name.
fn found(all: Vec<Hit>, typed: &str) -> Vec<Hit> {
    let mut hits: Vec<(u8, Hit)> = all.into_iter().filter_map(|h| score(typed, &h.name).map(|s| (s, h))).collect();
    hits.sort_by(|(a, x), (b, y)| a.cmp(b).then(y.in_use.cmp(&x.in_use)).then_with(|| groups::natural_cmp(&x.name, &y.name)));
    hits.into_iter().map(|(_, h)| h).collect()
}

/// Brings the file found to the front: in the group in use, its window (or it opens); in another
/// group, that group opens with it in front.
fn go(app: &mut SheetApp, hit: &Hit) {
    let current = groups::current(app);
    if hit.group == current {
        match (hit.window, &hit.path) {
            (Some(w), _) if app.session.window(w).is_some() => app.run_or_alert("window.activate", json!({"window": w})),
            (_, Some(path)) => app.open_path(path),
            _ => {}
        }
        return;
    }
    if let Some(g) = app.ui.groups.iter_mut().find(|g| g.name == hit.group) {
        g.active = hit.path.clone();
    }
    groups::switch(app, Target::Group(hit.group.clone()));
}

/// The Find File dialog: a name to type, the files it finds (Up and Down pick one, Enter or a
/// double-click goes to it).
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    ui.label(lang.tr("Name of the file (* any text, ? any character):"));
    let mut typed = d.values.get("find").and_then(Json::as_str).unwrap_or_default().to_string();
    let mut picked = d.values.get("picked").and_then(Json::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    // Up and Down move through the list while the name is typed.
    let (up, down) =
        ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)));
    let field = ui.add(egui::TextEdit::singleline(&mut typed).desired_width(f32::INFINITY)).accessible_name(lang.tr("File name"));
    if d.values.get("focused").and_then(Json::as_bool) != Some(true) {
        field.request_focus();
        d.values.insert("focused".into(), json!(true));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if field.changed() {
        picked = 0;
    }
    let hits = found(every_file(app), &typed);
    let last = hits.len().min(SHOWN_MAX).saturating_sub(1);
    if down {
        picked = (picked + 1).min(last);
    }
    if up {
        picked = picked.saturating_sub(1);
    }
    picked = picked.min(last);
    ui.add_space(6.0);
    let mut chosen = None;
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.ribbon_border)).corner_radius(4.0).inner_margin(4).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(360.0).min_scrolled_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
            if hits.is_empty() {
                ui.label(RichText::new(lang.tr("No file of the groups has that name.")).italics().color(t.text_dim));
            }
            for (i, h) in hits.iter().take(SHOWN_MAX).enumerate() {
                let resp = row(ui, &t, h, i == picked);
                if (up || down) && i == picked {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    picked = i;
                }
                if resp.double_clicked() {
                    chosen = Some(i);
                }
            }
        });
    });
    let count = if hits.len() > SHOWN_MAX {
        lang.tr("{n} files (the first {max} shown)").replace("{n}", &hits.len().to_string()).replace("{max}", &SHOWN_MAX.to_string())
    } else {
        lang.tr("{n} files").replace("{n}", &hits.len().to_string())
    };
    ui.label(RichText::new(count).font(theme::ui_font(11.5)).color(t.text_dim));
    d.values.insert("find".into(), json!(typed));
    d.values.insert("picked".into(), json!(picked));
    ui.add_space(6.0);
    let row = [crate::widgets::DialogButton::confirm("Open"), crate::widgets::DialogButton::neutral("Cancel")];
    match crate::widgets::dialog_buttons(ui, &row) {
        Some(0) => chosen = Some(picked),
        Some(_) => *open = false,
        None if enter => chosen = Some(picked),
        None => {}
    }
    if let Some(hit) = chosen.and_then(|i| hits.get(i)) {
        *open = false;
        go(app, hit);
    }
}

/// A file's row: its icon and name, and on the right its group (and folder in it); the row picked
/// is tinted.
fn row(ui: &mut Ui, t: &Tokens, h: &Hit, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, on, &h.name));
    if on {
        ui.painter().rect_filled(r, 4.0, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let icon = Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(14.0, 14.0));
    let kind = h.path.as_deref().map_or(Icon::Document, crate::file_tabs::path_icon);
    icons::paint(ui.painter(), icon, kind, if h.in_use { t.accent } else { t.text_dim });
    let small = theme::ui_font(11.5);
    let place = match &h.folder {
        Some(f) => format!("{} › {f}", h.group),
        None => h.group.clone(),
    };
    let place_w = ui.painter().layout_no_wrap(place.clone(), small.clone(), t.text_dim).size().x.min(r.width() * 0.45);
    let place_shown = crate::file_tabs::fit(ui, &place, &small, place_w);
    ui.painter().text(pos2(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, place_shown, small, t.text_dim);
    let font = theme::ui_font(13.0);
    let name = crate::file_tabs::fit(ui, &h.name, &font, (r.right() - place_w - 24.0 - icon.right() - 6.0).max(40.0));
    ui.painter().text(pos2(icon.right() + 8.0, r.center().y), Align2::LEFT_CENTER, name, font, t.text);
    resp.on_hover_text(h.path.clone().unwrap_or_else(|| h.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(group: &str, name: &str, in_use: bool) -> Hit {
        Hit { group: group.into(), window: None, path: Some(format!("/w/{name}")), name: name.into(), folder: None, in_use }
    }

    #[test]
    fn a_name_is_found_anywhere_and_those_starting_with_it_come_first() {
        let all =
            vec![hit("B", "22_order_search.http", false), hit("A", "order.sql", true), hit("B", "Order_list.md", false), hit("A", "notes.md", true)];
        let names = |typed: &str| found(all.clone(), typed).into_iter().map(|h| h.name).collect::<Vec<_>>();
        assert_eq!(
            names("order"),
            ["order.sql", "Order_list.md", "22_order_search.http"],
            "regardless of case; the group in use's first among equals"
        );
        assert_eq!(names("order http"), ["22_order_search.http"], "every word in it");
        assert_eq!(names("o*.md"), ["Order_list.md"], "a pattern: the name starts as it does");
        assert_eq!(names("*search"), ["22_order_search.http"]);
        assert_eq!(names("no?es"), ["notes.md"]);
        assert_eq!(names("").len(), 4, "nothing typed: every file");
        assert!(names("missing").is_empty());
    }

    #[test]
    fn hangul_names_are_found_too() {
        let all = vec![hit("주문", "10_주문조회.http", false), hit("주문", "11_주문등록.http", false)];
        let names: Vec<String> = found(all, "등록").into_iter().map(|h| h.name).collect();
        assert_eq!(names, ["11_주문등록.http"]);
    }
}
