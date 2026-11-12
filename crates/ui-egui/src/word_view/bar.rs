//! A Word tab's tool bar (as a Markdown file's: Edit and View, Contents, Fit Width, Find; then One
//! and Two Pages, the page, PDF; in Edit mode the formatting), its find bar and its contents.

use egui::{Color32, Key, Rect, RichText, Ui, vec2};
use serde_json::{Value as Json, json};
use wordcraft_engine::doc::{Document, StoryRef};

use super::{Doc, Go, Phase, Word};
use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::icons::Icon;
use crate::page_layout::GAP;
use crate::theme::{self, Tokens};

/// The most headings listed.
const MOST_HEADINGS: usize = 5000;
/// The font sizes offered.
const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 36.0, 48.0, 72.0];
/// The paragraph styles offered: the style's id and its name to show.
const STYLES: [(&str, &str); 5] =
    [("Normal", "Style|Normal"), ("Title", "Style|Title"), ("Heading1", "Heading 1"), ("Heading2", "Heading 2"), ("Heading3", "Heading 3")];
/// Font colours: their hex and their name.
const COLORS: [(&str, &str); 10] = [
    ("000000", "Black"),
    ("7F7F7F", "Gray"),
    ("C00000", "Dark Red"),
    ("FF0000", "Red"),
    ("FFC000", "Orange"),
    ("00B050", "Green"),
    ("00B0F0", "Light Blue"),
    ("0070C0", "Blue"),
    ("002060", "Dark Blue"),
    ("7030A0", "Purple"),
];
/// Highlight colours: Word's name, its colour, the name to show.
const HIGHLIGHTS: [(&str, [u8; 3], &str); 5] = [
    ("yellow", [255, 255, 0], "Yellow"),
    ("green", [0, 255, 0], "Bright Green"),
    ("cyan", [0, 255, 255], "Turquoise"),
    ("magenta", [255, 0, 255], "Pink"),
    ("lightGray", [192, 192, 192], "Gray"),
];

/// A text button of the tool bar, shown pressed when `on`.
fn button(ui: &mut Ui, on: bool, text: RichText, tip: &str) -> bool {
    ui.add(egui::Button::selectable(on, text).min_size(vec2(26.0, 26.0))).on_hover_text(tip).clicked()
}

/// The tool bar.
pub(super) fn toolbar(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, (_, sheet): (u64, usize), (fit, editing): (bool, bool), t: &Tokens) {
    let lang = app.ui.language;
    let label = |s: &str| RichText::new(lang.tr(s)).font(theme::ui_font(13.0));
    for (m, icon, name, tip) in [(Mode::Edit, Icon::Pen, "Edit", "Edit the document"), (Mode::View, Icon::Document, "View", "Read the document")] {
        let on = (m == Mode::Edit) == editing;
        if doc_view::mode_button(ui, t, on, icon, (lang.tr(name), lang.tr(tip))) && !on {
            app.run_or_alert("doc.view", json!({"sheet": sheet, "mode": m.name()}));
            doc.refocus = m == Mode::Edit;
        }
    }
    doc_view::separator(ui, t);
    let outline = app.ui.doc_outline;
    if button(ui, outline, label("Contents"), lang.tr("Show the document's headings beside its pages")) {
        app.ui.doc_outline = !outline;
    }
    if button(ui, fit, label("Fit Width"), lang.tr("Make the pages as wide as the window")) {
        app.run_or_alert("doc.view", json!({"sheet": sheet, "fitWidth": !fit}));
    }
    doc_view::find_button(app, ui);
    doc_view::separator(ui, t);
    for (two, name, tip) in [(false, "One Page", "One page across"), (true, "Two Pages", "Two pages side by side")] {
        if button(ui, doc.spread == two, label(name), lang.tr(tip)) && doc.spread != two {
            doc.spread = two;
            doc.go = Some(Go::At(doc.top.map_or(doc.page, |(p, _)| p), 0.0, GAP));
        }
    }
    doc_view::separator(ui, t);
    page_field(app, ui, doc, sheet, t);
    doc_view::separator(ui, t);
    if button(ui, false, label("PDF"), lang.tr("Export the document as a PDF")) {
        doc.after.push(("file.exportPdf", json!({})));
    }
    if editing && let Phase::Ready(word) = &mut doc.phase {
        let state = format_state(&mut doc.format, word);
        doc_view::separator(ui, t);
        doc.refocus |= formatting(app, ui, word, &state, t);
    }
}

/// The page shown, and a field taking a page number to go to.
fn page_field(app: &SheetApp, ui: &mut Ui, doc: &mut Doc, sheet: usize, t: &Tokens) {
    let lang = app.ui.language;
    let pages = doc.pages();
    ui.label(RichText::new(lang.tr("Page")).font(theme::ui_font(13.0)));
    ui.add_space(4.0);
    let mut typed = doc.page_typed.clone().unwrap_or_else(|| if pages == 0 { String::new() } else { (doc.page + 1).to_string() });
    let field = ui.add(
        egui::TextEdit::singleline(&mut typed)
            .id(egui::Id::unique("monun.word_page").with(sheet))
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
    ui.label(RichText::new(format!("/ {pages}")).font(theme::ui_font(13.0)).color(t.text_dim));
}

/// The formatting at the selection (`format.state`), asked again only after an edit or a move.
fn format_state(cache: &mut super::ForSelection<Json>, word: &mut Word) -> Json {
    let (rev, sel) = (word.session.rev(), word.session.sel.clone());
    if let Some((r, s, state)) = cache.as_ref()
        && *r == rev
        && *s == sel
    {
        return state.clone();
    }
    let state = word.run("format.state", &json!({})).unwrap_or(Json::Null);
    *cache = Some((rev, sel, state.clone()));
    state
}

/// The formatting tools of Edit mode: style, font, size, bold…, colours, alignment, lists,
/// indents and tables. Returns whether one was used (the pages take the keyboard back).
fn formatting(app: &mut SheetApp, ui: &mut Ui, word: &mut Word, state: &Json, t: &Tokens) -> bool {
    let lang = app.ui.language;
    let mut ran: Option<(&'static str, Json)> = None;
    // Style.
    let style = state.get("style").and_then(Json::as_str).unwrap_or("Normal");
    let shown = STYLES
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(style))
        .map_or_else(|| state.get("styleName").and_then(Json::as_str).unwrap_or(style).to_owned(), |(_, name)| lang.tr(name).to_owned());
    craft_ui::ComboBox::from_id_salt("monun.word_style").width(92.0).selected_text(shown).show_ui(ui, |ui| {
        for (id, name) in STYLES {
            if ui.selectable_label(id.eq_ignore_ascii_case(style), lang.tr(name)).clicked() {
                ran = Some(("para.style", json!({"style": id})));
            }
        }
    });
    // Font.
    let font = state.get("font").and_then(Json::as_str).unwrap_or_default().to_owned();
    let fonts = app.word.fonts.clone();
    let mut listed = None;
    craft_ui::ComboBox::from_id_salt("monun.word_font").width(120.0).selected_text(font.clone()).show_ui(ui, |ui| {
        let list = fonts.clone().unwrap_or_else(|| {
            let mut v = wordcraft_fonts::FontDb::global().families();
            v.sort_by_key(|f| f.to_lowercase());
            v.dedup();
            let v = std::sync::Arc::new(v);
            listed = Some(v.clone());
            v
        });
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for name in list.iter() {
                if ui.selectable_label(*name == font, name).clicked() {
                    ran = Some(("format.font", json!({"name": name})));
                }
            }
        });
    });
    if listed.is_some() {
        app.word.fonts = listed;
    }
    // Size.
    let size = state.get("size").and_then(Json::as_f64);
    let size_text = size.map_or_else(String::new, |s| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s:.1}") });
    craft_ui::ComboBox::from_id_salt("monun.word_size").width(48.0).selected_text(size_text).show_ui(ui, |ui| {
        for s in SIZES {
            if ui.selectable_label(size.is_some_and(|z| (z - f64::from(s)).abs() < 0.01), format!("{s}")).clicked() {
                ran = Some(("format.size", json!({"size": s})));
            }
        }
    });
    if button(ui, false, RichText::new("A+").font(theme::ui_font(13.0)), lang.tr("Increase Font Size")) {
        ran = Some(("format.growFont", json!({})));
    }
    if button(ui, false, RichText::new("A-").font(theme::ui_font(13.0)), lang.tr("Decrease Font Size")) {
        ran = Some(("format.shrinkFont", json!({})));
    }
    doc_view::separator(ui, t);
    let on = |k: &str| state.get(k).and_then(Json::as_bool).unwrap_or(false);
    let toggles: [(&str, RichText, &str, &'static str); 4] = [
        ("bold", RichText::new("B").font(theme::ui_bold(13.0)), "Bold (Ctrl+B)", "format.bold"),
        ("italic", RichText::new("I").font(theme::ui_font(13.0)).italics(), "Italic (Ctrl+I)", "format.italic"),
        ("underline", RichText::new("U").font(theme::ui_font(13.0)).underline(), "Underline (Ctrl+U)", "format.underline"),
        ("strike", RichText::new("S").font(theme::ui_font(13.0)).strikethrough(), "Strikethrough", "format.strikethrough"),
    ];
    for (key, text, tip, id) in toggles {
        if button(ui, on(key), text, lang.tr(tip)) {
            ran = Some((id, json!({})));
        }
    }
    colours(ui, lang, &mut ran);
    doc_view::separator(ui, t);
    for (value, icon_text, tip) in
        [("left", "≡←", "Align Left"), ("center", "≡", "Center"), ("right", "→≡", "Align Right"), ("justify", "☰", "Justify")]
    {
        if button(ui, false, RichText::new(icon_text).font(theme::ui_font(13.0)), lang.tr(tip)) {
            ran = Some(("para.align", json!({"value": value})));
        }
    }
    doc_view::separator(ui, t);
    for (id, text, tip) in [
        ("para.bullets", "• ≡", "Bullets"),
        ("para.numbering", "1. ≡", "Numbering"),
        ("para.outdent", "⇤", "Decrease Indent"),
        ("para.indent", "⇥", "Increase Indent"),
    ] {
        if button(ui, false, RichText::new(text).font(theme::ui_font(13.0)), lang.tr(tip)) {
            ran = Some((id, json!({})));
        }
    }
    tables(ui, lang, &mut ran);
    let Some((id, params)) = ran else { return false };
    let _ = word.run(id, &params);
    true
}

/// The font colour and highlight menus.
fn colours(ui: &mut Ui, lang: crate::i18n::Language, ran: &mut Option<(&'static str, Json)>) {
    let colour = ui.add(
        egui::Button::new(RichText::new("A").font(theme::ui_bold(13.0)).underline().color(Color32::from_rgb(192, 0, 0))).min_size(vec2(26.0, 26.0)),
    );
    let colour = colour.on_hover_text(lang.tr("Font Color"));
    egui::Popup::menu(&colour).show(|ui| {
        if ui.button(lang.tr("Automatic")).clicked() {
            *ran = Some(("format.color", json!({"color": "auto"})));
        }
        for (hex, name) in COLORS {
            let rgb = u32::from_str_radix(hex, 16).unwrap_or(0);
            let c = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
            if ui.button(RichText::new(format!("■ {}", lang.tr(name))).color(c)).clicked() {
                *ran = Some(("format.color", json!({"color": hex})));
            }
        }
    });
    let mark = ui.add(
        egui::Button::new(RichText::new("ab").font(theme::ui_font(13.0)).background_color(Color32::YELLOW).color(Color32::BLACK))
            .min_size(vec2(26.0, 26.0)),
    );
    let mark = mark.on_hover_text(lang.tr("Text Highlight Color"));
    egui::Popup::menu(&mark).show(|ui| {
        for (word, [r, g, b], name) in HIGHLIGHTS {
            if ui.button(RichText::new(format!("■ {}", lang.tr(name))).color(Color32::from_rgb(r, g, b))).clicked() {
                *ran = Some(("format.highlight", json!({"color": word})));
            }
        }
        if ui.button(lang.tr("No Color")).clicked() {
            *ran = Some(("format.highlight", json!({"color": "none"})));
        }
    });
}

/// The table menu: a grid to insert a table, then rows, columns and the table itself.
fn tables(ui: &mut Ui, lang: crate::i18n::Language, ran: &mut Option<(&'static str, Json)>) {
    let menu =
        ui.add(egui::Button::new(RichText::new(lang.tr("Table ▾")).font(theme::ui_font(13.0))).frame_when_inactive(false).min_size(vec2(26.0, 26.0)));
    egui::Popup::menu(&menu).show(|ui| {
        ui.label(RichText::new(lang.tr("Insert Table")).font(theme::ui_bold(12.5)));
        let id = ui.scope_id().with("monun.word_table_grid");
        let hovered: (usize, usize) = ui.data(|d| d.get_temp(id)).unwrap_or((0, 0));
        let mut now_hovered = (0, 0);
        egui::Grid::new(id).spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for r in 1..=8 {
                for c in 1..=10 {
                    let on = r <= hovered.0 && c <= hovered.1;
                    let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::click());
                    let fill = if on { Color32::from_rgb(255, 214, 120) } else { Color32::from_gray(245) };
                    ui.painter().rect(rect, 1.0, fill, egui::Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Inside);
                    if resp.hovered() {
                        now_hovered = (r, c);
                    }
                    if resp.clicked() {
                        *ran = Some(("insert.table", json!({"rows": r, "cols": c})));
                        ui.close();
                    }
                }
                ui.end_row();
            }
        });
        ui.data_mut(|d| d.insert_temp(id, now_hovered));
        if now_hovered != (0, 0) {
            ui.label(format!("{} × {}", now_hovered.1, now_hovered.0));
        }
        ui.separator();
        for (cmd, name) in [
            ("table.insertRowAbove", "Insert Row Above"),
            ("table.insertRowBelow", "Insert Row Below"),
            ("table.insertColumnLeft", "Insert Column Left"),
            ("table.insertColumnRight", "Insert Column Right"),
            ("table.deleteRow", "Delete Row"),
            ("table.deleteColumn", "Delete Column"),
            ("table.deleteTable", "Delete Table"),
        ] {
            if ui.button(lang.tr(name)).clicked() {
                *ran = Some((cmd, json!({})));
            }
        }
    });
}

/// The find bar: as a Markdown file's; in Edit mode with Replace.
pub(super) fn find_bar(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, editing: bool, t: &Tokens) {
    let lang = app.ui.language;
    let mut child = doc_view::find_strip(ui, t);
    let ui = &mut child;
    let Some(find) = app.doc_view.find.as_mut() else { return };
    ui.label(RichText::new(lang.tr("Find:")).font(theme::ui_font(13.0)));
    let q = ui.add(
        egui::TextEdit::singleline(&mut find.query).id(egui::Id::unique("monun.doc_find")).desired_width(200.0).hint_text(lang.tr("Text to find")),
    );
    if std::mem::take(&mut find.focus) {
        q.request_focus();
    }
    let query = find.query.clone();
    let Phase::Ready(word) = &mut doc.phase else { return };
    if q.changed() {
        doc.found = if query.is_empty() {
            None
        } else {
            let r = word.run("edit.find", &json!({"text": query}));
            doc.reveal = true;
            Some((r.ok().and_then(|v| v.get("count").and_then(Json::as_u64)).unwrap_or(0) as usize, None))
        };
    }
    let enter = q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let back = ui.input(|i| i.modifiers.shift);
    let count = match doc.found {
        _ if query.is_empty() => String::new(),
        None | Some((0, _)) => lang.tr("No matches").to_owned(),
        Some((n, Some(i))) => format!("{}/{n}", i + 1),
        Some((n, None)) => format!("-/{n}"),
    };
    ui.label(RichText::new(count).font(theme::ui_font(12.5)).color(t.text_dim));
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
    let mut replace = None;
    if editing && find.replacing {
        ui.label(RichText::new(lang.tr("Replace with:")).font(theme::ui_font(13.0)));
        ui.add(egui::TextEdit::singleline(&mut find.replace).desired_width(160.0));
        if ui.button(lang.tr("Replace")).clicked() {
            replace = Some(false);
        }
        if ui.button(lang.tr("Replace All")).clicked() {
            replace = Some(true);
        }
    } else if editing && ui.add(egui::Button::new(lang.tr("Replace…")).frame_when_inactive(false)).clicked() {
        find.replacing = true;
    }
    let with = find.replace.clone();
    let close = icon(ui, Icon::Close, "Close (Esc)") || (q.has_focus() || q.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape));
    if close {
        app.doc_view.find = None;
        doc.found = None;
        return;
    }
    if query.is_empty() {
        return;
    }
    if let Some(forward) = step {
        let r = word.run(if forward { "edit.findNext" } else { "edit.findPrevious" }, &json!({"text": query}));
        if let Ok(v) = r {
            let n = v.get("count").and_then(Json::as_u64).unwrap_or(0) as usize;
            let i = v.get("index").and_then(Json::as_u64).map(|i| i as usize);
            doc.found = Some((n, i));
            doc.reveal = true;
        }
    }
    match replace {
        Some(false) => {
            let _ = word.run("edit.replace", &json!({"text": query, "with": with}));
            let r = word.run("edit.find", &json!({"text": query}));
            doc.found = Some((r.ok().and_then(|v| v.get("count").and_then(Json::as_u64)).unwrap_or(0) as usize, None));
            doc.reveal = true;
        }
        Some(true) => {
            let n = word
                .run("edit.replaceAll", &json!({"text": query, "with": with}))
                .ok()
                .and_then(|v| v.get("replaced").and_then(Json::as_u64))
                .unwrap_or(0);
            app.toast = Some((lang.tr("Replaced {count}.").replace("{count}", &n.to_string()), crate::clock::now_ms()));
            doc.found = Some((0, None));
        }
        None => {}
    }
}

/// The headings of `doc` (paragraphs with an outline level): where, how deep, what they say.
fn headings(doc: &Document) -> Vec<super::Heading> {
    doc.para_paths(StoryRef::Body)
        .into_iter()
        .filter_map(|path| {
            let p = doc.para(StoryRef::Body, &path)?;
            let level = doc.styles.resolve_para(&p.props).outline_level?;
            let text = p.plain_text().split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then_some((path, level, text))
        })
        .take(MOST_HEADINGS)
        .collect()
}

/// The contents beside the pages: the document's headings, as a Markdown file's are.
pub(super) fn contents(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, rect: Rect, t: &Tokens) {
    let lang = app.ui.language;
    let Phase::Ready(word) = &mut doc.phase else {
        let _ = doc_view::contents_panel(ui, rect, t, lang.tr("Contents"), lang.tr("Opening…"), &[]);
        return;
    };
    // Read again after an edit, once typing pauses: every paragraph is looked at.
    let (rev, now) = (word.session.rev(), ui.input(|i| i.time));
    let typing = now - doc.blink_from < 0.4;
    if doc.contents.as_ref().is_none_or(|(r, _)| *r != rev && !typing) {
        doc.contents = Some((rev, headings(&word.session.doc)));
    } else if doc.contents.as_ref().is_some_and(|(r, _)| *r != rev) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    let Some((_, items)) = &doc.contents else { return };
    let entries: Vec<doc_view::Entry<'_>> =
        items.iter().map(|(_, level, title)| doc_view::Entry { indent: level.min(&8).to_owned(), title, bold: *level == 0 }).collect();
    let chosen = doc_view::contents_panel(ui, rect, t, lang.tr("Contents"), lang.tr("Headings (Heading 1, 2, …) show here."), &entries);
    let Some(path) = chosen.and_then(|i| items.get(i)).map(|(path, _, _)| path.clone()) else { return };
    let _ = word.run("caret.set", &json!({"pos": {"story": "body", "path": path, "off": 0}}));
    if let Some(c) = word.layout.caret_on(&word.session.sel.focus, word.session.page_hint) {
        doc.go = Some(Go::At(c.page, c.top, GAP));
    }
}
