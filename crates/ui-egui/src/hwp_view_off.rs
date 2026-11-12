//! 한글 documents in a build without the `hwp` feature (the web build): their tab says it can't
//! show them, and nothing else does anything for them.

use egui::Ui;
use serde_json::Value as Json;

use crate::SheetApp;
use crate::theme::{self, Tokens};

/// Nothing to keep without rhwp.
#[derive(Default)]
pub struct State;

/// Says the document can't be shown here.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let rect = ui.max_rect();
    let text = app.ui.language.tr("This version of Monun can't show Hangul documents.");
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::ui_font(14.0), t.text_dim);
}

pub(crate) fn tidy(_: &mut SheetApp, _: &egui::Context) {}

pub(crate) fn command(_: &mut SheetApp, _: &str, _: &Json) -> Option<Result<Json, String>> {
    None
}

pub(crate) fn print(_: &mut SheetApp) -> Option<Result<Json, String>> {
    None
}

pub(crate) fn adopt(_: &mut SheetApp, _: u64, _: Vec<u8>) {}

pub(crate) fn recovery_rev(_: &State, _: u64) -> Option<u64> {
    None
}

pub(crate) fn recovery_bytes(_: &State, _: u64) -> Option<Vec<u8>> {
    None
}

pub fn spread(_: &SheetApp, _: u64) -> Option<bool> {
    None
}

pub fn status(_: &SheetApp) -> Option<String> {
    None
}

pub fn inspect(_: &SheetApp) -> Json {
    Json::Null
}
