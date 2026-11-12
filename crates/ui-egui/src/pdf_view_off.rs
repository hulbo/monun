//! PDF files in a build without the `pdf` feature (the web build): their tab says it can't show
//! them.

use egui::Ui;
use serde_json::Value as Json;

use crate::SheetApp;
use crate::theme::{self, Tokens};

/// Nothing to keep without a renderer.
#[derive(Default)]
pub struct State;

/// Says the PDF can't be shown here.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let rect = ui.max_rect();
    let text = app.ui.language.tr("This version of Monun can't show PDF files.");
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::ui_font(14.0), t.text_dim);
}

pub(crate) fn tidy(_: &mut SheetApp, _: &egui::Context) {}

pub fn spread(_: &SheetApp, _: u64) -> Option<bool> {
    None
}

pub fn status(_: &SheetApp) -> Option<String> {
    None
}

pub fn inspect(_: &SheetApp) -> Json {
    Json::Null
}
