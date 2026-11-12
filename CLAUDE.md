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
- PDF tabs: `crates/ui-egui/src/pdf_view/` (feature `pdf`, desktop only), drawn by `pdfcraft-render`
  from a pinned PdfCraft commit; its patched hayro and vello crates are patched in from the same
  commit in the root `Cargo.toml`. Bump all of them together. The engine knows a PDF only as an
  empty read-only document tab (`TextKind::Pdf`, `docs::pdf_workbook`).

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
- The antivirus on this machine blocks serde's build script as the dev profile builds it
  ("access denied" running `build-script-build`). Release builds are unaffected. In a worktree,
  add `debug-assertions = false` for `[profile.dev.package.serde]` and `serde_core` to
  `.cargo/config.toml` locally, with `git update-index --skip-worktree` so it isn't committed.

## Running and looking at the app
- `cargo build --release -p monun -p monun-cli`; the desktop shortcut runs
  `target/release/monun.exe`. Close the app before building (Windows locks the exe).
- Drive it: start with `--control 7979` (monun-cli reads the token file the app writes), then
  `monun-cli send 7979 <method> '<json>'` (`ui.screenshot`, `ui.click`, `ui.text`,
  `engine.execute`; see `crates/ui-egui/src/control.rs`). It starts on the start page with no file
  open (file tabs, `crates/ui-egui/src/file_tabs.rs`): run `file.new` first. `MONUN_NO_PREFS=1` keeps
  a test run off the user's `ui.json` and `prefs.json` entirely; a `--control` run reads them but never
  writes them, and doesn't reopen the user's file group (`crates/ui-egui/src/groups.rs`).
- A normal start hands its files to a 모눈 already running and ends (`apps/monun/src/single_instance.rs`,
  `%APPDATA%\Monun\instance.json`); `--control` and `MONUN_NO_PREFS` runs never do, so a test run
  never reaches the user's own 모눈.
- After moving the checkout, run `cargo clean -p xtask`: xtask compiles its own path in.
- Headless: `monun-cli eval '=SUM(1,2,3)'`,
  `monun-cli run --cmd 'cell.set={"cell":"A1","input":"1"}' --out book.xlsx`.
- Settings and logs: `%APPDATA%\Monun` (`prefs.json`, `ui.json`, `logs/monun.log`; a frame over
  250 ms is logged there as `slow frame`, with where its time went). `--control` runs keep no log
  file, so they never touch the user's: their log is standard error (`verify` keeps it in
  `target\verify\app.log`).
- A normal start of the release build registers the running exe for .xlsx and the other types it
  opens under `HKCU` (Open with, Default apps; `apps/monun/src/file_types.rs`). Debug builds and
  `--control` runs don't, so a worktree's build never takes over the user's file types.
- AutoRecover (`crates/ui-egui/src/recovery.rs`) keeps copies of unsaved work in
  `%APPDATA%\Monun\recovery\<pid>-<stamp>` and reopens a crashed run's copies at start. `--control`
  and `MONUN_NO_PREFS` runs neither write nor pick them up.
- `update.cmd` (the user's "모눈 업데이트" desktop shortcut) runs `scripts/update.ps1` in
  `C:\Project\monun`: fast-forwards `main` from origin and rebuilds the release exe.

## Skill and subagents (`.claude/`)
- `verify` skill: `.claude/skills/verify/verify.ps1` runs fmt, clippy, `cargo xtask layers` and
  `assets`, the touched crates' tests (the known failures pass), a release build, then the app with
  `-Setup` commands and a screenshot in `target\verify\app.png`. It runs in the `verifier`
  subagent. Use it before every commit.
- `verifier` runs the checks and reports, `reviewer` reviews the diff read-only, and `ko-translator`
  fills `ko.rs`; only `ko-translator` edits files. Cargo runs one at a time (one target directory),
  and two agents never edit the same file.
