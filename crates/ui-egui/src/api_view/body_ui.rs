//! The Body section of an API request tab: the ways Postman offers a body (none, form-data,
//! x-www-form-urlencoded, raw as Text, JavaScript, JSON, HTML or XML, binary, GraphQL), each with
//! its editor, and Beautify for JSON and XML. A switch keeps the body left for coming back this run.

use std::collections::HashMap;
use std::path::Path;

use egui::{Align, Layout, RichText, Ui, vec2};

use super::body::{self, BOUNDARY, BodyKind, Field, RawKind};
use super::http_file::{self, Request};
use super::mono;
use crate::code_editor::Lang;
use crate::icons::Icon;
use crate::theme::{self, Tokens};

/// The bodies a switch left (its `Content-Type` and text), by way, for coming back to them.
#[derive(Default)]
pub(super) struct Stash {
    bodies: HashMap<u8, (Option<String>, String)>,
}

/// What the Body section needs besides the request.
pub(super) struct Env<'a> {
    pub lang: crate::i18n::Language,
    pub t: &'a Tokens,
    /// The folder of the request's file: a file it sends is named relative to it.
    pub base: Option<&'a Path>,
    /// Chooses any file (the desktop's file dialog).
    pub pick: Option<&'a dyn Fn() -> Option<String>>,
    /// The variables without a value, marked in red where `{{…}}` names them.
    pub missing: &'a [String],
}

/// A way's place, the raw kinds sharing one.
fn slot(k: BodyKind) -> u8 {
    match k {
        BodyKind::None => 0,
        BodyKind::FormData => 1,
        BodyKind::UrlEncoded => 2,
        BodyKind::Raw(_) => 3,
        BodyKind::Binary => 4,
        BodyKind::GraphQl => 5,
    }
}

/// The colours a raw body of `kind` is written in.
pub(super) fn lang_of(kind: RawKind) -> Lang {
    match kind {
        RawKind::Json => Lang::Json,
        RawKind::Xml | RawKind::Html => Lang::Markup,
        RawKind::JavaScript => Lang::Script,
        RawKind::Text => Lang::Plain,
    }
}

/// Moves `req`'s body from the way `from` to `to`: its `Content-Type` (and GraphQL's mark) set, the
/// body left in `to` before coming back, a form's fields going over between form-data and
/// urlencoded.
fn switch(req: &mut Request, from: BodyKind, to: BodyKind, stash: &mut Stash) {
    let ct = body::header(req, "Content-Type").map(str::to_string);
    let carry: Vec<(String, String)> = match from {
        BodyKind::UrlEncoded => body::pairs(&req.body),
        BodyKind::FormData => {
            let b = ct.as_deref().and_then(body::boundary).unwrap_or_else(|| BOUNDARY.to_string());
            body::fields(&req.body, &b).into_iter().filter(|f| !f.file).map(|f| (f.name, f.value)).collect()
        }
        _ => Vec::new(),
    };
    stash.bodies.insert(slot(from), (ct, std::mem::take(&mut req.body)));
    body::set_header(req, "X-Request-Type", None);
    let back = stash.bodies.get(&slot(to)).cloned().filter(|(_, b)| !b.trim().is_empty());
    let (ct, text) = match to {
        BodyKind::None => (None, String::new()),
        BodyKind::FormData => match back {
            Some(b) => b,
            None => {
                let fields: Vec<Field> = carry.into_iter().map(|(name, value)| Field { name, value, ..Field::default() }).collect();
                (Some(format!("multipart/form-data; boundary={BOUNDARY}")), body::render_fields(&fields, BOUNDARY))
            }
        },
        BodyKind::UrlEncoded => back.unwrap_or_else(|| (Some("application/x-www-form-urlencoded".into()), body::render_pairs(&carry))),
        BodyKind::Raw(k) => back.unwrap_or_else(|| (Some(k.content_type().into()), String::new())),
        BodyKind::Binary => back.unwrap_or_else(|| (Some("application/octet-stream".into()), "<".into())),
        BodyKind::GraphQl => {
            body::set_header(req, "X-Request-Type", Some("GraphQL"));
            back.unwrap_or_else(|| (Some("application/json".into()), String::new()))
        }
    };
    body::set_header(req, "Content-Type", ct.as_deref());
    req.body = text;
}

/// The Body section of `req`.
pub(super) fn show(ui: &mut Ui, req: &mut Request, stash: &mut Stash, env: &Env) {
    let (lang, t) = (env.lang, env.t);
    let now = body::kind(req);
    let mut to = None;
    ui.horizontal(|ui| {
        let raw = match now {
            BodyKind::Raw(k) => k,
            _ => RawKind::Json,
        };
        let ways = [
            (BodyKind::None, lang.tr("none")),
            (BodyKind::FormData, "form-data"),
            (BodyKind::UrlEncoded, "x-www-form-urlencoded"),
            (BodyKind::Raw(raw), "raw"),
            (BodyKind::Binary, "binary"),
            (BodyKind::GraphQl, "GraphQL"),
        ];
        for (k, label) in ways {
            if ui.radio(slot(now) == slot(k), label).clicked() && slot(now) != slot(k) {
                to = Some(k);
            }
        }
        if let BodyKind::Raw(k) = now {
            ui.add_space(4.0);
            craft_ui::ComboBox::from_id_salt("monun.api.raw_kind")
                .width(96.0)
                .selected_text(RichText::new(k.label()).color(t.accent_text).strong())
                .show_ui(ui, |ui| {
                    for each in RawKind::ALL {
                        if ui.selectable_label(each == k, each.label()).clicked() {
                            body::set_header(req, "Content-Type", Some(each.content_type()));
                        }
                    }
                })
                .response
                .accessible_name(lang.tr("Body type"));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let beautify = matches!(now, BodyKind::Raw(RawKind::Json | RawKind::Xml) | BodyKind::GraphQl);
            if beautify && ui.button(lang.tr("Beautify")).on_hover_text(lang.tr("Lay out the JSON or XML a value a line")).clicked() {
                match now {
                    BodyKind::Raw(RawKind::Json) => {
                        if let Some(p) = http_file::pretty_json(&req.body) {
                            req.body = p;
                        }
                    }
                    BodyKind::Raw(_) => {
                        if let Some(p) = http_file::pretty_xml(&req.body) {
                            req.body = p;
                        }
                    }
                    _ => {
                        let (q, v) = body::graphql(&req.body);
                        if let Some(p) = http_file::pretty_json(&v) {
                            req.body = body::render_graphql(&q, &p);
                        }
                    }
                }
            }
        });
    });
    if let Some(to) = to {
        switch(req, now, to, stash);
    }
    let kind = body::kind(req);
    if matches!(req.method.as_str(), "GET" | "HEAD") && kind != BodyKind::None {
        ui.label(RichText::new(lang.tr("A GET request sends this body too.")).font(theme::ui_font(12.0)).color(t.text_dim));
    }
    ui.add_space(4.0);
    match kind {
        BodyKind::None => {
            ui.label(RichText::new(lang.tr("This request has no body.")).color(t.text_dim));
        }
        BodyKind::FormData => form_data(ui, req, env),
        BodyKind::UrlEncoded => {
            let mut pairs = body::pairs(&req.body);
            let mut changed = super::pairs_table(ui, ("monun.api.urlencoded", 0), &mut pairs, (lang, t), env.missing);
            if ui.button(lang.tr("Add Field")).clicked() {
                pairs.push((String::new(), String::new()));
                changed = true;
            }
            if changed {
                req.body = body::render_pairs(&pairs);
            }
        }
        BodyKind::Raw(k) => {
            code_field(ui, &mut req.body, lang_of(k), (lang.tr("Body"), 6), (t, env.missing));
        }
        BodyKind::Binary => binary(ui, req, env),
        BodyKind::GraphQl => {
            let (mut q, mut v) = body::graphql(&req.body);
            let mut changed = false;
            let w = ((ui.available_width() - 12.0) / 2.0).max(160.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.label(RichText::new(lang.tr("Query")).color(t.text_dim));
                    changed |= code_field(ui, &mut q, Lang::Plain, (lang.tr("Query"), 12), (t, env.missing));
                });
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.label(RichText::new(lang.tr("GraphQL variables")).color(t.text_dim));
                    changed |= code_field(ui, &mut v, Lang::Json, (lang.tr("GraphQL variables"), 12), (t, env.missing));
                });
            });
            if changed {
                req.body = body::render_graphql(&q, &v);
            }
        }
    }
}

/// A code editor of `text` coloured as `code` reads, its `{{…}}` marked as variables, its rows even
/// (Hangul too); `true` when edited.
fn code_field(ui: &mut Ui, text: &mut String, code: Lang, (name, rows): (&str, usize), (t, missing): (&Tokens, &[String])) -> bool {
    let font = mono(13.0);
    let (plain, syntax) = (t.text, t.syntax);
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = crate::code_editor::layout_job(buf.as_str(), code, &font, plain, &syntax, wrap);
        super::mark_vars(&mut job, missing, t);
        crate::doc_view::even_code_rows(ui.ctx(), &mut job, &font);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    ui.add(egui::TextEdit::multiline(text).code_editor().desired_width(f32::INFINITY).desired_rows(rows).layouter(&mut layouter))
        .accessible_name(name)
        .changed()
}

/// A file path field and Choose File…: the path as the request writes it (relative to its file's
/// folder when inside it), and whether the file is there.
fn file_field(ui: &mut Ui, path: &mut String, width: f32, env: &Env) -> bool {
    let (lang, t) = (env.lang, env.t);
    let mut changed = false;
    ui.horizontal(|ui| {
        let field = egui::TextEdit::singleline(path).font(mono(13.0)).desired_width(width).hint_text(lang.tr("Choose a file"));
        changed |= ui.add(field).accessible_name(lang.tr("File")).changed();
        if let Some(pick) = env.pick
            && ui.button(lang.tr("Choose File…")).clicked()
            && let Some(p) = pick()
        {
            *path = body::written(env.base, &p);
            changed = true;
        }
        if !path.trim().is_empty() {
            let full = body::resolve(env.base, path.trim());
            match std::fs::metadata(&full) {
                Ok(m) if m.is_file() => {
                    ui.label(RichText::new(super::size_text(m.len() as usize)).color(t.text_dim));
                }
                _ => {
                    ui.label(RichText::new(lang.tr("Not found")).color(t.danger)).on_hover_text(full.display().to_string());
                }
            }
        }
    });
    changed
}

fn binary(ui: &mut Ui, req: &mut Request, env: &Env) {
    let mut path = body::file_ref(&req.body).unwrap_or_default().to_string();
    let width = (ui.available_width() - 240.0).max(200.0);
    if file_field(ui, &mut path, width, env) {
        req.body = format!("< {}", path.trim());
    }
    ui.label(RichText::new(env.lang.tr("The file goes as it is, the body of the request.")).font(theme::ui_font(12.0)).color(env.t.text_dim));
}

/// form-data: a field a row, its name, Text or File, and its value (a file's path); ✕ takes it out.
fn form_data(ui: &mut Ui, req: &mut Request, env: &Env) {
    let (lang, t) = (env.lang, env.t);
    let boundary = body::header(req, "Content-Type").and_then(body::boundary).unwrap_or_else(|| BOUNDARY.to_string());
    let mut fields = body::fields(&req.body, &boundary);
    let mut changed = false;
    let mut remove = None;
    let name_w = 180.0;
    let value_w = (ui.available_width() - name_w - 140.0).max(160.0);
    for (i, f) in fields.iter_mut().enumerate() {
        ui.push_id(("monun.api.form", i), |ui| {
            ui.horizontal(|ui| {
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut f.name).font(mono(13.0)).desired_width(name_w).hint_text(lang.tr("Name")))
                    .accessible_name(lang.tr("Name"))
                    .changed();
                let kind = if f.file { lang.tr("File") } else { lang.tr("Text") };
                craft_ui::ComboBox::from_id_salt("kind")
                    .width(64.0)
                    .selected_text(kind)
                    .show_ui(ui, |ui| {
                        for (file, label) in [(false, lang.tr("Text")), (true, lang.tr("File"))] {
                            if ui.selectable_label(f.file == file, label).clicked() && f.file != file {
                                f.file = file;
                                f.value.clear();
                                f.content_type.clear();
                                changed = true;
                            }
                        }
                    })
                    .response
                    .accessible_name(lang.tr("Field type"));
                if f.file {
                    changed |= file_field(ui, &mut f.value, (value_w - 200.0).max(140.0), env);
                } else {
                    let mut marked = super::var_layouter(mono(13.0), (env.missing, t));
                    let value = egui::TextEdit::singleline(&mut f.value).font(mono(13.0)).desired_width(value_w).hint_text(lang.tr("Value"));
                    changed |= ui.add(value.layouter(&mut marked)).accessible_name(lang.tr("Value")).changed();
                }
                if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, lang.tr("Remove"), vec2(22.0, 22.0)).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove.filter(|i| *i < fields.len()) {
        fields.remove(i);
        changed = true;
    }
    if ui.button(lang.tr("Add Field")).clicked() {
        fields.push(Field::default());
        changed = true;
    }
    if changed {
        req.body = body::render_fields(&fields, &boundary);
    }
}
