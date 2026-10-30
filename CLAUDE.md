# 모눈 (Monun) — instructions for agents

Monun is a personal, Rust-native spreadsheet for one Korean freelancer (Windows desktop). It reads
and writes XLSX and aims to behave like Excel for Windows. The repository is private
(github.com/hulbo/monun); nothing is distributed.

## Working here
- Talk to the user in Korean.
- When a change is built, tested and checked in the app, commit and push to `origin main` without
  asking. Ask first only for destructive git operations.
- Defaults the user chose: new workbooks in 굴림체 11, 130 px default columns, Excel-for-Windows
  cell size at 100%, Korean interface.

## Layout
Cargo workspace, layered (`cargo xtask layers`): `core`/`numfmt` → `formula`/`functions` →
`model`/`calc`/`xlsx` → `chart` → `engine` → `ui-egui`/`mcp` → `apps/monun` (desktop),
`apps/monun-cli`, `apps/monun-web`. Nothing below `ui-egui` depends on egui.
- Worksheet functions: `crates/functions/src/*.rs` (`f!(...)` rows plus tests).
- Commands (every user-visible action): `crates/engine/src/cmd/*`; UI-only ones in `crates/ui-egui`.
- UI: `crates/ui-egui` (ribbon, grid painting in `grid/quadrant.rs` and `grid/cell_paint.rs`,
  fonts in `fonts.rs` and `theme/font_setup.rs`).
- Translations: `crates/ui-egui/src/i18n/ko.rs` (`cargo xtask i18n-report` lists gaps); wrap new
  labels in `lang.tr(...)`.
- App icon: `assets/app-icon/monun.svg` is the source; the PNG, `.ico` and `.icns` files are
  rendered from it (see `assets/app-icon/README.md`).

## Rules
- Never crash: no `unwrap()`, `expect()`, `panic!`, indexing on input-derived data or `unsafe` in
  non-test code; errors are `Result`s. Production crate roots deny the panicking lints.
- No vendor (Microsoft and others) icons, artwork, fonts, templates or sample files. Font files
  are never committed; the app uses the system's fonts. Every asset file has a row in
  `ATTRIBUTION.md` (`cargo xtask assets`).
- Before committing: `cargo fmt --all`, `cargo clippy --workspace --lib --bins -- -D warnings`, and
  the tests of the crates touched (`cargo test -p monun-ui-egui` and so on).
- Known failures unrelated to a change: three `monun-ui-egui` integration tests
  (`file_menu::save_a_copy_with_a_bare_name_asks_where_to_save_it`,
  `interaction::column_autocomplete_folds_case_beyond_ascii`,
  `interaction::language_switch_renders_the_ribbon_and_persists`) need an embedded font set this
  machine lacks, and `monun-engine`'s unit tests use a unix-only API and don't build on Windows.

## Running and looking at the app
- `cargo build --release -p monun -p monun-cli`; the desktop shortcut runs
  `target/release/monun.exe`. Close the app before building (Windows locks the exe).
- Drive it: start with `--control 7979` (monun-cli reads the token file the app writes), then
  `monun-cli send 7979 <method> '<json>'` (`ui.screenshot`, `ui.click`, `ui.text`,
  `engine.execute`; see `crates/ui-egui/src/control.rs`).
- After moving the checkout, run `cargo clean -p xtask`: xtask compiles its own path in.
- Headless: `monun-cli eval '=SUM(1,2,3)'`,
  `monun-cli run --cmd 'cell.set={"cell":"A1","input":"1"}' --out book.xlsx`.
- Settings and logs: `%APPDATA%\Monun` (`prefs.json`, `ui.json`, `logs/monun.log`).

## Skill and subagents (`.claude/`)
- `verify` skill: `.claude/skills/verify/verify.ps1` runs fmt, clippy, `cargo xtask layers` and
  `assets`, the touched crates' tests (the known failures pass), a release build, then the app with
  `-Setup` commands and a screenshot in `target\verify\app.png`. It runs in the `verifier`
  subagent. Use it before every commit.
- `verifier` runs the checks and reports, `reviewer` reviews the diff read-only, and `ko-translator`
  fills `ko.rs`; only `ko-translator` edits files. Cargo runs one at a time (one target directory),
  and two agents never edit the same file.
