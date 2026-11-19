//! The interface's symbol characters must not render as empty boxes (#197, #54): the UI font often
//! lacks arrows, check marks and geometric shapes, so egui's bundled symbol faces (Hack and the
//! monochrome emoji fonts) and code-drawn chrome icons back it up.
//!
//! The checks ask the font set whether it has a real glyph for each character (`has_glyph`), which
//! is the property the user sees. The replacement-box probe is a separate test that documents why
//! "differs from the tofu glyph" alone is not evidence that a face has the character.
//!
//! Bundled/system faces provide `↘ ◀ ↑ 📋`; the three symbols no bundled face has (`✕ ✓ ⌃`) are
//! chrome-only gaps covered by code-drawn icons and their ASCII equivalents.

use egui::{FontFamily, FontId};
use monun_ui_egui::icons::{self, Icon};
use monun_ui_egui::theme;

/// Symbols the bundled faces provide on a host with no symbol, CJK or Latin-Extended face:
/// Hack supplies `↘ ◀ ↑`; Noto Emoji and emoji-icon-font supply `📋`.
const BUNDLED_REQUIRED_UI_SYMBOLS: &str = "↘◀↑📋";

/// The rest of what the interface draws as text that egui's bundled faces cover: Ubuntu-Light
/// supplies punctuation and maths, Hack the arrows/triangles and keyboard arrows, and the
/// monochrome emoji faces ⌘ and ★.
const BUNDLED_UI_SYMBOLS: &str = "▲▼▸▾←→↓↳↗⇧⌘★•›≠≤≥∞";

/// A context with the deterministic bundled font set installed, plus the families
/// the interface uses. System fonts are deliberately excluded so this check is the same on CI.
fn themed() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(theme::pinned_font_definitions());
    let mut out = ctx.run_ui(egui::RawInput::default(), |_ui| {});
    out.textures_delta.clear(); // nothing consumes the atlas in a headless test
    ctx
}

/// Returns the characters in `symbols` that lack a glyph in the given font family.
fn missing(ctx: &egui::Context, symbols: &str, family: FontFamily) -> String {
    let font = FontId::new(14.0, family);
    symbols.chars().filter(|&c| !ctx.fonts_mut(|f| f.has_glyph(&font, c))).collect()
}

/// Returns the proportional and cell font families checked by the symbol-glyph tests.
fn families() -> [FontFamily; 2] {
    [FontFamily::Proportional, FontFamily::Name(theme::CELL.into())]
}

#[test]
/// Every required bundled interface symbol resolves in both checked font families.
fn bundled_required_ui_symbols_resolve_to_real_glyphs() {
    let ctx = themed();
    for family in families() {
        let missing = missing(&ctx, BUNDLED_REQUIRED_UI_SYMBOLS, family.clone());
        assert!(
            missing.is_empty(),
            "these required interface symbols have no real glyph in {family:?}: {missing} — \
             egui must be built with `monochrome_emoji_fonts` and the Hack fallback must stay in \
             theme::font_definitions"
        );
    }
}

#[test]
/// The remaining bundled interface symbols resolve in both checked font families.
fn bundled_symbols_cover_the_rest_of_the_interface() {
    let ctx = themed();
    for family in families() {
        let missing = missing(&ctx, BUNDLED_UI_SYMBOLS, family.clone());
        assert!(missing.is_empty(), "egui's bundled faces have no real glyph in {family:?} for: {missing}");
    }
}

#[test]
/// U+10FFFD is the only tofu probe and does not resolve in either family.
fn unknown_codepoint_is_the_tofu_probe_not_a_symbol() {
    // A private-use code point no font covers. This is the only place that relies on the
    // replacement box: real-glyph checks above use `has_glyph`, because comparing a glyph's
    // texture with the replacement glyph would pass for two distinct missing characters.
    let ctx = themed();
    for family in families() {
        let font = FontId::new(14.0, family.clone());
        assert!(!ctx.fonts_mut(|f| f.has_glyph(&font, '\u{10FFFD}')), "U+10FFFD must not resolve in {family:?}");
    }
}

#[test]
/// `✕`/`✓` map to code icons and `⌃` falls back to `^`; the semantic `×` and `^` resolve.
fn chrome_symbols_without_a_bundled_glyph_have_code_drawn_or_semantic_fallbacks() {
    // No bundled face has ✕, ✓ or ⌃. The buttons that used to paint ✕/✓ as text now draw the
    // original code icons Icon::Close/Icon::Check, and the one Control-key tooltip falls back to
    // "^" when ⌃ is absent. Nothing here suppresses a glyph check or the accessibility check.
    assert_eq!(icons::fallback_for_symbol('✕'), Some(Icon::Close), "the close buttons must be code-painted");
    assert_eq!(icons::fallback_for_symbol('✓'), Some(Icon::Check), "the Symbol gallery check must be code-painted");
    assert_eq!(theme::symbol_fallback('⌃'), Some('^'), "the Control-key glyph needs a readable ASCII fallback");
    let ctx = themed();
    for family in families() {
        let missing = missing(&ctx, "×^", family.clone());
        assert!(missing.is_empty(), "the semantic fallbacks × and ^ must resolve in {family:?}: missing {missing}");
    }
}

#[test]
/// Prints the `✓`/`⌃` gaps of the bundled faces as a diagnostic; the strict bundled checks still run separately.
fn no_font_mode_documents_the_optional_symbol_gaps() {
    let ctx = themed();
    let missing: String = families().iter().map(|family| missing(&ctx, "✓⌃", family.clone())).collect();
    // A diagnostic, not an acceptance skip: the bundled assertions above still run strictly.
    eprintln!("{missing:?} have no bundled face; chrome uses code-drawn icons or ASCII fallbacks instead");
}
