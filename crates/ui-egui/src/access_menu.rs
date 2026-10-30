//! Right-click menus with access keys, as in Excel for Windows.
//!
//! An item's English label marks its key with `&` ("&Delete…", "Cu&t"). English shows the label
//! with that letter underlined; a translated label gets the key in brackets before any ellipsis,
//! as Korean Excel shows it ("삭제(D)…"). While a menu is open it owns the keyboard: a letter runs
//! its item, or highlights the next item with that letter when several share it, and Enter runs
//! the highlighted item. Up, Down, Home and End move the highlight, Right opens a submenu and
//! Left or Escape closes it again.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Event, Galley, Id, Key, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::i18n::Language;

/// One row of a menu.
pub struct Entry<'a> {
    label: &'a str,
    id: &'a str,
    items: Vec<Entry<'a>>,
    custom: bool,
}

impl<'a> Entry<'a> {
    /// An item: its English `label` with `&` before the key, and the `id` choosing it returns.
    pub fn item(label: &'a str, id: &'a str) -> Self {
        Self { label, id, items: Vec::new(), custom: false }
    }

    pub fn separator() -> Self {
        Self::item("-", "")
    }

    /// An item that opens a submenu of `items`.
    pub fn submenu(label: &'a str, id: &'a str, items: Vec<Entry<'a>>) -> Self {
        Self { label, id, items, custom: false }
    }

    /// An item whose submenu the caller draws, like Tab Color's palette.
    pub fn custom(label: &'a str, id: &'a str) -> Self {
        Self { label, id, items: Vec::new(), custom: true }
    }

    pub fn id(&self) -> &'a str {
        self.id
    }

    fn is_separator(&self) -> bool {
        self.label == "-"
    }

    fn opens_submenu(&self) -> bool {
        self.custom || !self.items.is_empty()
    }

    fn key(&self) -> Option<char> {
        parse(self.label).1.map(|(_, key)| key)
    }
}

/// `label` without its `&` marker, and the marked letter: its byte offset in the result and the
/// letter upper-cased, which is the key. `&&` stands for an ampersand.
pub fn parse(label: &str) -> (String, Option<(usize, char)>) {
    let mut plain = String::with_capacity(label.len());
    let mut key = None;
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => plain.push('&'),
            Some(k) => {
                if key.is_none() {
                    key = Some((plain.len(), k.to_ascii_uppercase()));
                }
                plain.push(k);
            }
            None => {}
        }
    }
    (plain, key)
}

/// What a menu shows for `label` in `lang`, and the byte range of the key to underline: the
/// letter itself in English (and in a label that has no translation), or the key in brackets
/// after the translated label, before its ellipsis.
pub fn display(lang: Language, label: &str) -> (String, Option<Range<usize>>) {
    let (plain, key) = parse(label);
    let translated = lang.tr(&plain);
    let Some((at, key)) = key else { return (translated.to_owned(), None) };
    if lang == Language::En || translated == plain {
        let width = plain.get(at..).and_then(|rest| rest.chars().next()).map_or(0, char::len_utf8);
        return (plain, Some(at..at + width));
    }
    let stem = translated.trim_end_matches(['…', '.']);
    let tail = translated.get(stem.len()..).unwrap_or_default();
    let start = stem.len() + 1;
    (format!("{stem}({key}){tail}"), Some(start..start + key.len_utf8()))
}

/// What a menu shows for the item, as its accessible name.
fn shown(lang: Language, entry: &Entry<'_>) -> String {
    display(lang, entry.label).0
}

/// The letter or digit `key` types, upper-cased.
fn key_char(key: Key) -> Option<char> {
    let mut name = key.name().chars();
    match (name.next(), name.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// The highlight and open submenu of a menu, kept between frames.
#[derive(Clone, Copy, Default)]
struct State {
    /// The last frame the menu was drawn in: one that was closed starts afresh.
    frame: u64,
    highlight: Option<usize>,
    /// The item whose submenu is open.
    open: Option<usize>,
    /// The keyboard has moved into the open submenu.
    inside: bool,
    sub_highlight: Option<usize>,
    /// Where the open submenu was drawn.
    sub_rect: Option<Rect>,
    /// Which opening of the menu this state belongs to.
    opening: Option<u64>,
}

/// What a frame of the menu did.
pub struct Outcome<'a> {
    /// The id of the item chosen, or of the custom submenu that finished.
    pub chosen: Option<&'a str>,
    /// Where the open submenu is: a press there is not outside the menu.
    pub submenu: Option<Rect>,
}

/// Draws `entries` into the menu's `ui`, at least `min_width` wide, and handles its keys and
/// pointer. `opening` tells this opening of the menu from the last (where and when it opened), so
/// a menu closed and opened again in one frame starts afresh. `custom` draws the submenu of a
/// [`Entry::custom`] item, given its id, and returns whether it finished (a colour was picked),
/// which closes the menu.
pub fn show<'a>(
    ui: &mut Ui,
    id: Id,
    opening: impl Hash,
    entries: &[Entry<'a>],
    min_width: f32,
    mut custom: impl FnMut(&mut Ui, &str) -> bool,
) -> Outcome<'a> {
    let frame = ui.ctx().cumulative_frame_nr();
    let mut hasher = DefaultHasher::new();
    opening.hash(&mut hasher);
    let opening = hasher.finish();
    let mut st: State = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if st.frame + 1 < frame || st.opening != Some(opening) {
        st = State { opening: Some(opening), ..State::default() };
    }
    st.frame = frame;
    let lang = crate::i18n::current(ui.ctx());
    let mut chosen = keys(ui, entries, &mut st);

    let moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    // A pointer on its way to the open submenu may cross other items without closing it.
    let heading_in = st.open.is_some() && st.sub_rect.is_some_and(|r| ui.input(|i| i.pointer.is_moving_towards_rect(&r)));
    let rows = item_rows(ui, lang, entries, min_width);
    let mut hovered = false;
    for (i, row) in rows.iter().enumerate() {
        let (Some(resp), Some(entry)) = (row, entries.get(i)) else { continue };
        if resp.hovered() && moved && !(heading_in && st.open != Some(i)) {
            hovered = true;
            st.highlight = Some(i);
            st.inside = false;
            st.sub_highlight = None;
            st.open = entry.opens_submenu().then_some(i);
        }
        if resp.clicked() {
            st.highlight = Some(i);
            if entry.opens_submenu() {
                st.open = Some(i);
            } else {
                chosen = chosen.or(Some(entry.id));
            }
        }
    }
    if moved && !hovered && st.open.is_none() {
        // Off the items, with no submenu to keep its item lit, nothing is highlighted.
        st.highlight = None;
    }
    paint(ui, lang, entries, &rows, st.highlight);

    let mut submenu = None;
    let open = st.open.and_then(|i| Some((i, entries.get(i)?, rows.get(i)?.as_ref()?.rect)));
    if let Some((i, entry, anchor)) = open {
        // Beside the menu, on its left when the right side of the window has no room.
        let margin = egui::Frame::popup(ui.style()).total_margin();
        let top = anchor.top() - margin.top;
        let width = st.sub_rect.map_or(0.0, |r| r.width());
        let (at, pivot) = if anchor.right() + margin.right + 2.0 + width > ui.ctx().content_rect().right() {
            (pos2(anchor.left() - margin.left - 2.0, top), Align2::RIGHT_TOP)
        } else {
            (pos2(anchor.right() + margin.right + 2.0, top), Align2::LEFT_TOP)
        };
        let area = egui::Area::new(id.with(("submenu", i))).order(egui::Order::Foreground).pivot(pivot).fixed_pos(at).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if entry.custom {
                    if custom(ui, entry.id) {
                        chosen = chosen.or(Some(entry.id));
                    }
                    return;
                }
                let rows = item_rows(ui, lang, &entry.items, 120.0);
                for (j, row) in rows.iter().enumerate() {
                    let (Some(resp), Some(item)) = (row, entry.items.get(j)) else { continue };
                    if resp.hovered() && moved {
                        st.inside = true;
                        st.sub_highlight = Some(j);
                    }
                    if resp.clicked() {
                        chosen = chosen.or(Some(item.id));
                    }
                }
                paint(ui, lang, &entry.items, &rows, st.sub_highlight.filter(|_| st.inside));
            });
        });
        ui.ctx().set_sublayer(ui.layer_id(), area.response.layer_id);
        // The pointer in a palette takes the keyboard there too, for its arrows.
        if entry.custom && moved && area.response.contains_pointer() {
            st.inside = true;
        }
        submenu = Some(area.response.rect);
        if heading_in {
            ui.ctx().request_repaint();
        }
    }
    st.sub_rect = submenu;
    ui.ctx().data_mut(|d| d.insert_temp(id, st));
    Outcome { chosen, submenu }
}

/// Lays out a row per entry: a response for each item, `None` for a separator.
fn item_rows(ui: &mut Ui, lang: Language, entries: &[Entry<'_>], min_width: f32) -> Vec<Option<Response>> {
    let pad = ui.spacing().button_padding;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let arrow = if entries.iter().any(Entry::opens_submenu) { 18.0 } else { 0.0 };
    let widest = entries
        .iter()
        .filter(|e| !e.is_separator())
        .map(|e| ui.painter().layout_no_wrap(shown(lang, e), font.clone(), Color32::PLACEHOLDER).size().x)
        .fold(0.0, f32::max);
    let size = vec2((widest + 2.0 * pad.x + arrow).max(min_width), (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * pad.y).max(20.0));
    entries
        .iter()
        .map(|entry| {
            if entry.is_separator() {
                ui.separator();
                return None;
            }
            let (_, resp) = ui.allocate_exact_size(size, Sense::click());
            let name = shown(lang, entry);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            Some(resp)
        })
        .collect()
}

/// Paints the rows: the highlighted one with the hover background, each label with its key
/// underlined, and an arrow on items with a submenu.
fn paint(ui: &Ui, lang: Language, entries: &[Entry<'_>], rows: &[Option<Response>], highlight: Option<usize>) {
    let visuals = &ui.style().visuals.widgets;
    let pad = ui.spacing().button_padding;
    for (i, (row, entry)) in rows.iter().zip(entries).enumerate() {
        let Some(resp) = row else { continue };
        let lit = highlight == Some(i);
        let look = if lit { &visuals.hovered } else { &visuals.inactive };
        if lit {
            ui.painter().rect(resp.rect, look.corner_radius, look.weak_bg_fill, look.bg_stroke, StrokeKind::Inside);
        }
        let color = look.text_color();
        let galley = label_galley(ui, lang, entry.label, color);
        let at = pos2(resp.rect.left() + pad.x, resp.rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, color);
        if entry.opens_submenu() {
            let c = pos2(resp.rect.right() - pad.x - 4.0, resp.rect.center().y);
            let points = vec![pos2(c.x - 3.0, c.y - 4.0), pos2(c.x + 2.0, c.y), pos2(c.x - 3.0, c.y + 4.0)];
            ui.painter().add(Shape::convex_polygon(points, color, Stroke::NONE));
        }
    }
}

/// The label in `color` with its key underlined.
fn label_galley(ui: &Ui, lang: Language, label: &str, color: Color32) -> Arc<Galley> {
    let (text, key) = display(lang, label);
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let plain = TextFormat { font_id, color, ..Default::default() };
    let mut job = LayoutJob::default();
    match key.filter(|k| text.get(k.clone()).is_some()) {
        Some(k) => {
            let underlined = TextFormat { underline: Stroke::new(1.0, color), ..plain.clone() };
            job.append(text.get(..k.start).unwrap_or_default(), 0.0, plain.clone());
            job.append(text.get(k.clone()).unwrap_or_default(), 0.0, underlined);
            job.append(text.get(k.end..).unwrap_or_default(), 0.0, plain);
        }
        None => job.append(&text, 0.0, plain),
    }
    ui.painter().layout_job(job)
}

/// Handles this frame's key presses and takes them out of the input, so that nothing behind the
/// menu (the grid would start typing) sees them. An Escape that closes only a submenu is taken
/// too; the menu's own Escape stays for the caller, which closes the menu on it. In a palette
/// the caller draws (Tab Color) only Escape is the menu's: the palette's arrows and Enter are
/// left for it.
fn keys<'a>(ui: &Ui, entries: &[Entry<'a>], st: &mut State) -> Option<&'a str> {
    let events = ui.input(|i| i.events.clone());
    let in_palette = st.inside && st.open.and_then(|i| entries.get(i)).is_some_and(|e| e.custom);
    let mut chosen = None;
    let mut escape_taken = false;
    for event in &events {
        let Event::Key { key, physical_key, pressed: true, repeat, modifiers, .. } = event else { continue };
        if modifiers.ctrl || modifiers.command || modifiers.alt || modifiers.mac_cmd || chosen.is_some() {
            continue;
        }
        if in_palette {
            if *key == Key::Escape && st.open.is_some() {
                st.open = None;
                st.inside = false;
                escape_taken = true;
            }
            continue;
        }
        let parent = st.open.and_then(|i| entries.get(i));
        let inside = st.inside && parent.is_some();
        // The submenu the keyboard is in, or the menu itself.
        let list: &[Entry<'a>] = match parent {
            Some(p) if inside => &p.items,
            _ => entries,
        };
        let highlight = if inside { st.sub_highlight } else { st.highlight };
        let mut moved_to = None;
        let mut activate = None;
        match key {
            Key::Escape | Key::ArrowLeft if inside => {
                st.open = None;
                st.inside = false;
                st.sub_highlight = None;
                escape_taken |= *key == Key::Escape;
            }
            Key::ArrowDown => moved_to = step(list, highlight, true),
            Key::ArrowUp => moved_to = step(list, highlight, false),
            Key::Home => moved_to = step(list, None, true),
            Key::End => moved_to = step(list, None, false),
            Key::ArrowRight if !inside => activate = highlight.filter(|&i| entries.get(i).is_some_and(Entry::opens_submenu)),
            Key::Enter | Key::Space => activate = highlight,
            _ if !repeat => {
                let Some(letter) = key_char(*key).or_else(|| physical_key.and_then(key_char)) else { continue };
                let matches: Vec<usize> = list.iter().enumerate().filter(|(_, e)| e.key() == Some(letter)).map(|(i, _)| i).collect();
                match matches.as_slice() {
                    [] => {}
                    [only] => activate = Some(*only),
                    // Several items share the letter: each press moves to the next, Enter runs it.
                    many => moved_to = many.iter().copied().find(|&i| highlight.is_some_and(|h| i > h)).or(many.first().copied()),
                }
            }
            _ => {}
        }
        if let Some(i) = moved_to {
            if inside {
                st.sub_highlight = Some(i);
            } else {
                st.highlight = Some(i);
                st.open = None;
            }
        }
        let Some(i) = activate else { continue };
        match (inside, list.get(i)) {
            (true, Some(item)) => chosen = Some(item.id),
            (false, Some(item)) if item.opens_submenu() => {
                st.highlight = Some(i);
                st.open = Some(i);
                st.inside = true;
                st.sub_highlight = step(&item.items, None, true);
            }
            (false, Some(item)) => {
                st.highlight = Some(i);
                chosen = Some(item.id);
            }
            _ => {}
        }
    }
    ui.ctx().input_mut(|i| {
        i.events.retain(|e| match e {
            Event::Key { key: Key::Escape, .. } => !escape_taken,
            _ if in_palette => true,
            Event::Key { modifiers, .. } => modifiers.ctrl || modifiers.command || modifiers.alt || modifiers.mac_cmd,
            Event::Text(_) => false,
            _ => true,
        })
    });
    chosen
}

/// The next item from `from` (or from either end), skipping separators and wrapping around.
fn step(list: &[Entry<'_>], from: Option<usize>, down: bool) -> Option<usize> {
    let n = list.len();
    let mut at = from;
    for _ in 0..n {
        let next = match (at, down) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        if list.get(next).is_some_and(|e| !e.is_separator()) {
            return Some(next);
        }
        at = Some(next);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Entry, display, parse, step};
    use crate::i18n::Language;

    #[test]
    fn the_marker_names_the_key_and_leaves_the_label() {
        assert_eq!(parse("Cu&t"), ("Cut".to_string(), Some((2, 'T'))));
        assert_eq!(parse("&Delete…"), ("Delete…".to_string(), Some((0, 'D'))));
        assert_eq!(parse("Save && Close"), ("Save & Close".to_string(), None));
        assert_eq!(parse("Plain"), ("Plain".to_string(), None));
    }

    #[test]
    fn english_underlines_the_letter_and_korean_adds_it_in_brackets() {
        assert_eq!(display(Language::En, "Cu&t"), ("Cut".to_string(), Some(2..3)));
        assert_eq!(display(Language::Ko, "Cu&t"), ("잘라내기(T)".to_string(), Some("잘라내기(".len().."잘라내기(T".len())));
        assert_eq!(display(Language::Ko, "&Delete…").0, "삭제(D)…");
        assert_eq!(display(Language::Ko, "&Format Cells…").0, "셀 서식(F)…");
        // A label with no translation keeps its English look.
        assert_eq!(display(Language::Ko, "Zz &Untranslated"), ("Zz Untranslated".to_string(), Some(3..4)));
    }

    #[test]
    fn arrows_skip_separators_and_wrap() {
        let list = [Entry::item("&A", "a"), Entry::separator(), Entry::item("&B", "b")];
        assert_eq!(step(&list, None, true), Some(0));
        assert_eq!(step(&list, Some(0), true), Some(2));
        assert_eq!(step(&list, Some(2), true), Some(0));
        assert_eq!(step(&list, None, false), Some(2));
        assert_eq!(step(&[], None, true), None);
    }
}
