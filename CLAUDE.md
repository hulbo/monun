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
- Files in the window (`SheetApp::file_tabs`, the desktop's): no tabs at the bottom. The group panel
  (`groups::panel`, shown from the start) lists the group in use's open windows
  (`groups::open_files_listed`, unsaved files too), closes and orders them, and its + makes files and
  groups; the title bar's `groups::file_button` names the file in front and lists the others. A
  workbook shows Excel's sheet tabs (`tabs::sheet_tabs`, through `file_tabs::bottom_bar`); other
  files have no bottom bar. Its title strip tucks it into the window's edge (`UiState::group_tucked`,
  a strip from `panes::tucked`; clicked, the panel shows over the files, `SheetApp::group_peek`).
  Another pane shows in the group panel's place while open. Tests:
  `common::app` turns the group panel off; a test of it turns it on.
- Translations: `crates/ui-egui/src/i18n/ko.rs` (`cargo xtask i18n-report` lists gaps); wrap new
  labels in `lang.tr(...)`.
- App icon: `assets/app-icon/monun.svg` is the source; the PNG, `.ico` and `.icns` files are
  rendered from it (see `assets/app-icon/README.md`).
- PDF tabs: `crates/ui-egui/src/pdf_view/` (feature `pdf`, desktop only), drawn by `pdfcraft-render`
  from a pinned PdfCraft commit; its patched hayro and vello crates are patched in from the same
  commit in the root `Cargo.toml`. Bump all of them together. The engine knows a PDF only as an
  empty read-only document tab (`TextKind::Pdf`, `docs::pdf_workbook`).
- Word tabs: `crates/ui-egui/src/word_view/` (feature `word`, desktop only). The document lives in a
  WordCraft `Session` (`wordcraft-engine`, `-render`, `-pdf` from a pinned WordCraft commit; its
  Krilla patch is copied into the root `Cargo.toml`). The engine knows only an empty tab
  (`TextKind::Word`, `docs::word_workbook`) and refuses edits, saves, exports and prints of it.
  Save, Save As, Export as PDF, Print and Undo/Redo are handled by `word_view::command`, hooked into
  `SheetApp::run_command`. `word_view::tidy` mirrors the session's dirty flag onto the tab.
- Picture tabs: `crates/ui-egui/src/image_view.rs`. The engine knows a picture only as an empty
  read-only tab (`TextKind::Image`, `docs::image_workbook`); the tab decodes the file off the UI
  thread with the `image` crate and shows it fitted, at 100% or at the tab's zoom.
- Web tabs: `crates/ui-egui/src/web_view.rs` and `apps/monun/src/web.rs` (Windows). An HTML file or
  a site link (`.url`, which New › Site Link writes) is an empty read-only tab to the engine
  (`TextKind::Web`, `docs::web_workbook`). Each frame the tab lists which page shows where
  (`web.panes`, physical pixels; hidden behind the Backstage) and what was asked of it
  (`web.orders`). A web view is a child window over egui's, so every egui layer drawn over the page
  this frame (Middle, Foreground and Tooltip: the floating memo or group panel, popups, dialogs,
  tooltips) is listed as a hole (`Pane::holes`, `web_view::settle`). After `ui`, the desktop app's
  `web::Web` makes, moves, shows and hides a WebView2 view per tab with `wry` (`=0.57.2`, no
  default features) as a child of the eframe window, cuts the holes out of its `WRY_WEBVIEW`
  container with a window region (`winsafe =0.0.29`, safe Win32; the region is the window's once
  set, so it is `leak`ed), hands the keyboard back on a click in egui (`focus_parent`), and reports
  titles and addresses back (`web.pages`). Control screenshots don't show the page (it isn't drawn by egui): `ui.inspect`'s
  `web` says whether it loaded.
- Presentation tabs: `crates/pptx` (`monun-pptx`, layer 0, `zip` and `roxmltree` only) turns a .pptx
  into one HTML page of its slides (HTML and inline SVG; placeholders inherit from the layout and
  master by `idx`, then type; text levels stack `defaultTextStyle` < master `txStyles` < master,
  layout and shape `lstStyle`; colours resolve through the slide's colour map). The engine knows an
  empty read-only tab (`TextKind::Slides`, `docs::slides_workbook`). `crates/ui-egui/src/slides_view.rs`
  (feature `slides`, desktop) writes the page on a thread into `%TEMP%\monun-slides\<pid>-<uid>` and
  `web_view` shows it as a web tab's page (rewritten when the file changes; the folder goes when
  the tab closes and in `slides_view::clean_up` at exit). Charts are a box with their title.
- 한글 tabs: `crates/ui-egui/src/hwp_view/` (feature `hwp`, desktop only). The document lives on a thread of
  its own (`thread.rs`) in rhwp's `HwpDocument` (crates.io `rhwp = "=0.9.0"`, no default features but
  `wasm-api`, whose facade holds some caret queries the core keeps crate-private; its error path panics
  outside a browser, so every call is guarded). The tab talks to it by `proto::Req`/`Reply` and never
  waits but for Save. Pages are rhwp's SVG drawn by resvg (`render.rs`), the font files a page names
  read into memory once (`render::fonts_for`: fontdb otherwise maps a file per glyph run, 0.3 s a
  page). Printing draws the pages at 300 dpi on several threads into a PDF of pictures (`pdf.rs`,
  lopdf); rhwp's own PDF writer is too slow, so there is no PDF export. The engine knows only an
  empty tab (`TextKind::Hwp`,
  `docs::hwp_workbook`) and refuses edits, saves, exports and prints of it.

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
- Disk: build output grows fast. One worktree's `target` reached 80 GB (47 GB of it
  `debug\incremental`) and filled C:, which crashed rustc. Once a change is committed, delete
  that worktree's `target\debug\incremental`. Delete the `target` of worktrees no longer in use,
  after checking that no cargo, rustc or monun process runs from them. Say what was freed.

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
- Releases (for PCs that don't build; README "릴리스 만들기"): `pwsh scripts/release.ps1 -Version x.y.z`
  bumps `[workspace.package] version`, builds, compiles `packaging/monun.iss` (Inno Setup 6.7+) into
  `dist\Monun-x.y.z-Setup.exe`, commits "모눈 x.y.z", pushes it to main with tag `vx.y.z` and creates
  the GitHub release with `gh`. Publishing is outward-facing: only when the user asks. It needs a
  clean tree at `origin/main`; `-BuildOnly` makes the setup alone. When the user's 모눈 runs from
  `target\release` it builds in `target\installer`. The setup installs per user (no admin) into
  `%LOCALAPPDATA%\Programs\Monun` under a fixed `AppId`, never touches `%APPDATA%\Monun`, and its
  uninstaller removes the file-type entries when they point at the installed exe. The antivirus now and then
  stops ISCC writing the setup's icon (`EndUpdateResource failed (110)`); the script retries.
  `.cargo/config.toml` links the C runtime statically, so the exe needs no VC++ Redistributable.

## Skill and subagents (`.claude/`)
- `verify` skill: `.claude/skills/verify/verify.ps1` runs fmt, clippy, `cargo xtask layers` and
  `assets`, the touched crates' tests (the known failures pass), a release build, then the app with
  `-Setup` commands and a screenshot in `target\verify\app.png`. It runs in the `verifier`
  subagent. Use it before every commit.
- `verifier` runs the checks and reports, `reviewer` reviews the diff read-only, and `ko-translator`
  fills `ko.rs`; only `ko-translator` edits files. Cargo runs one at a time (one target directory),
  and two agents never edit the same file.
