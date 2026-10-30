use super::Dialog;

/// Help ▸ About Monun: the name, version and licence.
pub(super) fn about(ui: &mut egui::Ui, _d: &mut Dialog) {
    let lang = crate::i18n::current(ui.ctx());
    ui.heading(lang.tr("Monun"));
    ui.label(lang.tr("Version {version}").replace("{version}", env!("CARGO_PKG_VERSION")));
    ui.add_space(4.0);
    ui.label(lang.tr("Licensed under MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE, NOTICE)."));
}
