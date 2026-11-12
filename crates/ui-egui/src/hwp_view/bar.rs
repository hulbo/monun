//! A 한글 tab's tool bar (as a Word tab's: Edit and View, Contents, Fit Width, Find; One and Two
//! Pages and the page; in Edit mode the formatting), its find bar and its contents.

use egui::{Color32, Key, Rect, RichText, Ui, vec2};

use super::proto::{Req, TableOp, Toggle};
use super::{Doc, Go, Phase};
use crate::SheetApp;
use crate::doc_view::{self, Mode};
use crate::icons::Icon;
use crate::page_layout::GAP;
use crate::theme::{self, Tokens};

/// The font sizes offered.
const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 36.0, 48.0, 72.0];
/// Text colours: their RGB and their name.
const COLORS: [(u32, &str); 10] = [
    (0x000000, "Black"),
    (0x7F7F7F, "Gray"),
    (0xC00000, "Dark Red"),
    (0xFF0000, "Red"),
    (0xFFC000, "Orange"),
    (0x00B050, "Green"),
    (0x00B0F0, "Light Blue"),
    (0x0070C0, "Blue"),
    (0x002060, "Dark Blue"),
    (0x7030A0, "Purple"),
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
            app.run_or_alert("doc.view", serde_json::json!({"sheet": sheet, "mode": m.name()}));
            doc.refocus = m == Mode::Edit;
        }
    }
    doc_view::separator(ui, t);
    let outline = app.ui.doc_outline;
    if button(ui, outline, label("Contents"), lang.tr("Show the document's headings beside its pages")) {
        app.ui.doc_outline = !outline;
    }
    if button(ui, fit, label("Fit Width"), lang.tr("Make the pages as wide as the window")) {
        app.run_or_alert("doc.view", serde_json::json!({"sheet": sheet, "fitWidth": !fit}));
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
    if editing && matches!(doc.phase, Phase::Ready) {
        doc_view::separator(ui, t);
        if let Some(req) = formatting(app, ui, doc, t) {
            doc.send(req);
            doc.refocus = true;
        }
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
            .id(egui::Id::unique("monun.hwp_page").with(sheet))
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

/// The installed font families, listed once.
fn font_list(app: &mut SheetApp) -> std::sync::Arc<Vec<String>> {
    app.hwp
        .fonts
        .get_or_insert_with(|| {
            let db = super::render::fonts();
            let mut v: Vec<String> =
                db.faces().flat_map(|f| f.families.iter().map(|(name, _)| name.clone())).filter(|n| !n.starts_with('@')).collect();
            v.sort_by_key(|f| f.to_lowercase());
            v.dedup();
            std::sync::Arc::new(v)
        })
        .clone()
}

/// The formatting tools of Edit mode: font, size, bold…, colour, alignment and tables. Returns
/// the request of the one used.
fn formatting(app: &mut SheetApp, ui: &mut Ui, doc: &Doc, t: &Tokens) -> Option<Req> {
    let lang = app.ui.language;
    let state = &doc.state.format;
    let mut ran: Option<Req> = None;
    // Font.
    let font = state.font.clone().unwrap_or_default();
    let fonts = font_list(app);
    craft_ui::ComboBox::from_id_salt("monun.hwp_font").width(120.0).selected_text(font.clone()).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for name in fonts.iter() {
                if ui.selectable_label(*name == font, name).clicked() {
                    ran = Some(Req::Font(name.clone()));
                }
            }
        });
    });
    // Size.
    let size = state.size;
    let size_text = size.map_or_else(String::new, |s| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s:.1}") });
    craft_ui::ComboBox::from_id_salt("monun.hwp_size").width(48.0).selected_text(size_text).show_ui(ui, |ui| {
        for s in SIZES {
            if ui.selectable_label(size.is_some_and(|z| (z - s).abs() < 0.01), format!("{s}")).clicked() {
                ran = Some(Req::Size(s));
            }
        }
    });
    if button(ui, false, RichText::new("A+").font(theme::ui_font(13.0)), lang.tr("Increase Font Size")) {
        ran = Some(Req::Grow(true));
    }
    if button(ui, false, RichText::new("A-").font(theme::ui_font(13.0)), lang.tr("Decrease Font Size")) {
        ran = Some(Req::Grow(false));
    }
    doc_view::separator(ui, t);
    let toggles = [
        (state.bold, RichText::new("B").font(theme::ui_bold(13.0)), "Bold (Ctrl+B)", Toggle::Bold),
        (state.italic, RichText::new("I").font(theme::ui_font(13.0)).italics(), "Italic (Ctrl+I)", Toggle::Italic),
        (state.underline, RichText::new("U").font(theme::ui_font(13.0)).underline(), "Underline (Ctrl+U)", Toggle::Underline),
        (state.strike, RichText::new("S").font(theme::ui_font(13.0)).strikethrough(), "Strikethrough", Toggle::Strike),
    ];
    for (on, text, tip, toggle) in toggles {
        if button(ui, on, text, lang.tr(tip)) {
            ran = Some(Req::Toggle(toggle));
        }
    }
    let colour = ui.add(
        egui::Button::new(RichText::new("A").font(theme::ui_bold(13.0)).underline().color(Color32::from_rgb(192, 0, 0))).min_size(vec2(26.0, 26.0)),
    );
    let colour = colour.on_hover_text(lang.tr("Font Color"));
    egui::Popup::menu(&colour).show(|ui| {
        for (rgb, name) in COLORS {
            let c = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
            if ui.button(RichText::new(format!("■ {}", lang.tr(name))).color(c)).clicked() {
                ran = Some(Req::Color(rgb));
            }
        }
    });
    doc_view::separator(ui, t);
    let align = state.align.as_deref().unwrap_or_default();
    for (value, icon_text, tip) in
        [("left", "≡←", "Align Left"), ("center", "≡", "Center"), ("right", "→≡", "Align Right"), ("justify", "☰", "Justify")]
    {
        if button(ui, align == value, RichText::new(icon_text).font(theme::ui_font(13.0)), lang.tr(tip)) {
            ran = Some(Req::Align(value));
        }
    }
    doc_view::separator(ui, t);
    tables(ui, lang, doc.state.in_table, &mut ran);
    ran
}

/// The table menu: a grid to insert a table, then rows and columns of the table at the caret.
fn tables(ui: &mut Ui, lang: crate::i18n::Language, in_table: bool, ran: &mut Option<Req>) {
    let menu =
        ui.add(egui::Button::new(RichText::new(lang.tr("Table ▾")).font(theme::ui_font(13.0))).frame_when_inactive(false).min_size(vec2(26.0, 26.0)));
    egui::Popup::menu(&menu).show(|ui| {
        if !in_table {
            ui.label(RichText::new(lang.tr("Insert Table")).font(theme::ui_bold(12.5)));
            let id = ui.scope_id().with("monun.hwp_table_grid");
            let hovered: (u16, u16) = ui.data(|d| d.get_temp(id)).unwrap_or((0, 0));
            let mut now_hovered = (0, 0);
            egui::Grid::new(id).spacing(vec2(2.0, 2.0)).show(ui, |ui| {
                for r in 1..=8u16 {
                    for c in 1..=10u16 {
                        let on = r <= hovered.0 && c <= hovered.1;
                        let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{c} × {r}")));
                        let fill = if on { Color32::from_rgb(255, 214, 120) } else { Color32::from_gray(245) };
                        ui.painter().rect(rect, 1.0, fill, egui::Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Inside);
                        if resp.hovered() {
                            now_hovered = (r, c);
                        }
                        if resp.clicked() {
                            *ran = Some(Req::Table(TableOp::Insert { rows: r, cols: c }));
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
            return;
        }
        for (op, name) in [
            (TableOp::RowAbove, "Insert Row Above"),
            (TableOp::RowBelow, "Insert Row Below"),
            (TableOp::ColumnLeft, "Insert Column Left"),
            (TableOp::ColumnRight, "Insert Column Right"),
            (TableOp::DeleteRow, "Delete Row"),
            (TableOp::DeleteColumn, "Delete Column"),
        ] {
            if ui.button(lang.tr(name)).clicked() {
                *ran = Some(Req::Table(op));
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
    if q.changed() {
        doc.found = None;
        if !query.is_empty() {
            doc.send(Req::Find { text: query.clone(), forward: true, again: false });
        }
    }
    let enter = q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let back = ui.input(|i| i.modifiers.shift);
    let count = match doc.found {
        _ if query.is_empty() => String::new(),
        None => String::new(),
        Some((0, _)) => lang.tr("No matches").to_owned(),
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
        doc.send(Req::Find { text: query.clone(), forward, again: true });
    }
    match replace {
        Some(false) => {
            doc.send(Req::Replace { text: query, with });
        }
        Some(true) => {
            doc.send(Req::ReplaceAll { text: query, with });
        }
        None => {}
    }
}

/// The contents beside the pages: the document's outline (개요) headings.
pub(super) fn contents(app: &mut SheetApp, ui: &mut Ui, doc: &mut Doc, rect: Rect, t: &Tokens) {
    let lang = app.ui.language;
    if !matches!(doc.phase, Phase::Ready) {
        let _ = doc_view::contents_panel(ui, rect, t, lang.tr("Contents"), lang.tr("Opening…"), &[]);
        return;
    }
    // Asked again after an edit, once typing pauses.
    let (rev, now) = (doc.state.rev, ui.input(|i| i.time));
    let typing = now - doc.blink_from < 0.4;
    let current = doc.contents.as_ref().is_some_and(|(r, _)| *r == rev);
    if !current && doc.contents_asked != Some(rev) && (doc.contents.is_none() || !typing) {
        doc.contents_asked = Some(rev);
        doc.send(Req::Outline);
    } else if !current && typing {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    let items = doc.contents.as_ref().map(|(_, items)| items.clone()).unwrap_or_default();
    let entries: Vec<doc_view::Entry<'_>> = items.iter().map(|(level, title)| doc_view::Entry { indent: *level, title, bold: *level == 0 }).collect();
    let chosen = doc_view::contents_panel(ui, rect, t, lang.tr("Contents"), lang.tr("Outline headings show here."), &entries);
    if let Some(i) = chosen {
        doc.send(Req::Goto(i));
        doc.reveal = true;
    }
}
