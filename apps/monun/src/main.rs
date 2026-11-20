//! Monun desktop app.
//!
//! Usage: `monun [--control <port> [--control-token <token>]] [--sample <name>] [files…]`
//!
//! `--control <port>` (or `MONUN_CONTROL_PORT`) starts a localhost JSON-lines control
//! server: `{"id":1,"method":"ui.inspect","params":{}}` → `{"id":1,"ok":true,"result":…}`.
//! The first line of each connection must carry the token (`docs/control-protocol.md`).
//! See `monun_ui_egui::control` and `docs/control-protocol.md` for the methods.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "macos")]
mod apple_events;
mod control_server;
#[cfg(any(target_os = "windows", test))]
mod file_types;
mod logging;
#[cfg(target_os = "macos")]
mod native_menu;
mod print;
mod single_instance;
#[cfg(target_os = "windows")]
mod web;

use std::cell::RefCell;
use std::rc::Rc;

use monun_engine::Session;
use monun_ui_egui::{Services, SheetApp};
use serde_json::json;

/// The app in one window, every open file a tab at its bottom ([`SheetApp::file_tabs`]), the
/// files later starts hand over ([`single_instance`]), and (macOS) the menu bar and Finder events.
struct App {
    app: SheetApp,
    instance: Option<single_instance::Listener>,
    #[cfg(target_os = "macos")]
    menu: Option<native_menu::NativeMenu>,
    #[cfg(target_os = "macos")]
    events: fmv_macos_events::Inbox,
    /// Web tabs' pages, each a WebView2 view in the window.
    #[cfg(target_os = "windows")]
    web: web::Web,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // The whole of the last frame, painting included (the app's own log covers its parts).
        if let Some(cpu) = frame.info().cpu_usage
            && f64::from(cpu) * 1000.0 > monun_ui_egui::SLOW_FRAME_MS
        {
            log::warn!("slow frame: {:.0} ms in all, painting included", f64::from(cpu) * 1000.0);
        }
        #[cfg(target_os = "macos")]
        {
            if self.menu.is_none() && std::env::var_os("MONUN_NO_NATIVE_MENU").is_none() {
                self.menu = Some(native_menu::NativeMenu::install(ctx));
            }
            if let Some(m) = &mut self.menu {
                m.poll(&mut self.app, ctx, egui::ViewportId::ROOT);
            }
            apple_events::poll(&self.events, &mut self.app, ctx);
        }
        // Files another start handed over open as tabs here, and this window comes forward.
        let handed: Vec<Vec<String>> = self.instance.as_ref().map(|l| l.rx.try_iter().collect()).unwrap_or_default();
        for files in handed {
            for f in files {
                self.app.open_path(&f);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.logic(ctx);
        if ctx.input(|input| input.viewport().close_requested()) && self.app.printing.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }
    /// Forwards raw input to the shared app, then opens every non-empty dropped file path as a workbook.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
        // Files dropped on the window open as workbooks.
        for f in std::mem::take(&mut raw.dropped_files) {
            if let Some(p) = f.path().to_str().map(str::to_string).filter(|p| !p.is_empty()) {
                self.app.open_path(&p);
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui);
        // The web tabs' pages where the frame just drawn puts them.
        #[cfg(target_os = "windows")]
        self.web.sync(&*frame, &mut self.app.web, ui.ctx());
        #[cfg(not(target_os = "windows"))]
        let _ = frame;
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        print::finish_pending_jobs();
        #[cfg(target_os = "windows")]
        self.web.close();
        // The pages presentations were shown as.
        monun_ui_egui::slides_view::clean_up();
        if let Some(l) = &self.instance {
            l.close();
        }
        if persists() {
            // Where each Markdown file was being read, to open it there next time.
            self.app.remember_open_doc_views();
            save_prefs(&self.app);
        }
        // A clean exit: the save questions are answered, so the recovery copies go.
        self.app.finish_recovery();
        // macOS (#117): end the process here, before eframe tears the window down. AccessKit's
        // macOS adapter subclasses the window's content view and restores its class on teardown;
        // AppKit's Touch Bar support observes that view (KVO), so in the next display cycle
        // `-[_NSTouchBarFinderObservation invalidate]` throws "not registered" and the quit ends
        // in a crash report. eframe calls `on_exit` on every way of quitting (Quit/⌘Q, the Dock,
        // the close button, `app.quit`), and nothing after it is needed: the preferences are
        // saved and the log file is unbuffered. eframe does the same when it doesn't
        // run-and-return. `on_exit` only runs once the app exists, never on a failed start.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

fn config_dir() -> Option<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support/Monun"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Monun"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .map(|c| c.join("monun"))
    }
}

/// Runs without preferences (`MONUN_NO_PREFS`, agents' test runs) neither read nor write
/// them, and keep no log file.
fn prefs_enabled() -> bool {
    std::env::var_os("MONUN_NO_PREFS").is_none()
}

/// A run a script drives (`--control`): it reads the preferences but never writes them, so a
/// test run leaves the user's file groups, recent files and settings as they were.
static CONTROLLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// This run writes the preferences (`ui.json`, `prefs.json`): it has them and no script drives it.
fn persists() -> bool {
    prefs_enabled() && !CONTROLLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Where the web tabs keep sign-ins and cookies: `%LOCALAPPDATA%\Monun\WebView2`, or for a test run
/// (which keeps nothing of the user's) one folder in the temporary folder, emptied as each test run
/// starts and ends (WebView2's helpers may still hold it at the end, so one folder, never one a run).
#[cfg(target_os = "windows")]
fn web_data() -> std::path::PathBuf {
    match std::env::var_os("LOCALAPPDATA").filter(|_| persists()) {
        Some(local) => std::path::PathBuf::from(local).join("Monun").join("WebView2"),
        None => {
            let dir = std::env::temp_dir().join("monun-webview-test");
            let _ = std::fs::remove_dir_all(&dir);
            dir
        }
    }
}

/// Writes `ui.json` (the interface preferences, file groups included) now.
fn save_ui(ui: &monun_ui_egui::UiState) {
    let Some(dir) = config_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(b) = serde_json::to_vec_pretty(ui) {
        let _ = std::fs::write(dir.join("ui.json"), b);
    }
}

/// Where the log files live: `logs` in the settings folder, next to `ui.json` (see `logging`).
fn log_dir() -> Option<std::path::PathBuf> {
    Some(config_dir()?.join("logs"))
}

fn load_prefs(app: &mut SheetApp) {
    if !prefs_enabled() {
        return;
    }
    // First run (or a ui.json without a usable language): follow the desktop's language.
    app.ui.language = monun_ui_egui::i18n::Language::system();
    // ...and its light or dark appearance (#239), until a theme is chosen and saved in ui.json.
    app.ui.system_theme = true;
    let Some(dir) = config_dir() else { return };
    if let Ok(b) = std::fs::read(dir.join("ui.json"))
        && let Ok(ui) = serde_json::from_slice::<monun_ui_egui::UiState>(&b)
    {
        let saved = serde_json::from_slice(&b).ok().and_then(|v| monun_ui_egui::i18n::saved_language(&v));
        let language = saved.unwrap_or(app.ui.language);
        app.ui = ui.at_startup();
        app.ui.language = language;
    }
    if let Ok(b) = std::fs::read(dir.join("prefs.json"))
        && let Ok(p) = serde_json::from_slice(&b)
    {
        app.session.prefs = p;
    }
    // A prefs.json saved before Monun set its own default still holds the model's, and one saved
    // by 1.1.2 or earlier holds the default of then; nothing in the app changes it, so it takes
    // today's. Any other `defaultFont` there is kept.
    if [monun_engine::model::DEFAULT_FONT, EARLIER_WORKBOOK_FONT].contains(&app.session.prefs.default_font.as_str()) {
        app.session.prefs.default_font = NEW_WORKBOOK_FONT.into();
    }
    if (app.session.prefs.default_column_width - monun_engine::model::DEFAULT_COL_WIDTH).abs() < 0.01 {
        app.session.prefs.default_column_width = NEW_WORKBOOK_COLUMN_WIDTH;
    }
    if app.session.prefs.default_paper == monun_engine::model::PrintSettings::default().paper {
        app.session.prefs.default_paper = NEW_WORKBOOK_PAPER.into();
    }
}

/// The font new workbooks start in (Excel's File › Options › "Use this as the default font"):
/// D2Coding, the font Naver gives away under the SIL Open Font License. Monun ships no font: on a PC
/// without it, cells in it draw in the cell font's fallbacks. `defaultFont` in prefs.json picks
/// another.
const NEW_WORKBOOK_FONT: &str = "D2Coding";

/// The font new workbooks started in until 1.1.2, which a prefs.json written then still holds.
const EARLIER_WORKBOOK_FONT: &str = "굴림체";

/// The standard column width of new workbooks, in pixels; `defaultColumnWidth` in prefs.json
/// picks another.
const NEW_WORKBOOK_COLUMN_WIDTH: f32 = 130.0;

/// The paper new workbooks and sheets print on, as on a Korean printer; `defaultPaper` in
/// prefs.json picks another.
const NEW_WORKBOOK_PAPER: &str = "A4";

fn save_prefs(app: &SheetApp) {
    if !prefs_enabled() {
        return;
    }
    let Some(dir) = config_dir() else { return };
    save_ui(&app.ui);
    if let Ok(b) = serde_json::to_vec_pretty(&app.session.prefs) {
        let _ = std::fs::write(dir.join("prefs.json"), b);
    }
}

/// Read the system clipboard through the shared handle, creating it on first use.
fn read_clipboard<T>(
    handle: &RefCell<Option<arboard::Clipboard>>,
    read: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Option<T> {
    let mut clipboard = handle.try_borrow_mut().ok()?;
    if clipboard.is_none() {
        *clipboard = arboard::Clipboard::new().ok();
    }
    read(clipboard.as_mut()?).ok()
}

fn services() -> Services {
    // Keep the owner alive: X11/Wayland serve clipboard data from this handle.
    let clipboard: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    let (html_clipboard, text_clipboard) = (clipboard.clone(), clipboard.clone());
    Services {
        copy_html: Some(Box::new(move |html, text| {
            let mut clipboard = clipboard.borrow_mut();
            if clipboard.is_none() {
                *clipboard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
            }
            clipboard.as_mut().ok_or("clipboard unavailable")?.set_html(html, Some(text)).map_err(|e| e.to_string())
        })),
        paste_html: Some(Box::new(move || read_clipboard(&html_clipboard, |clipboard| clipboard.get().html()))),
        paste_text: Some(Box::new(move || read_clipboard(&text_clipboard, |clipboard| clipboard.get_text()))),
        pick_open: Some(Box::new(|| {
            // Source files (.java, .vue, .yml, .xml…) open as text to read and edit.
            let sources: Vec<&str> = monun_engine::markdown::code::source_extensions().collect();
            let mut every = vec![
                "xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt", "json", "md", "sql", "pdf", "docx", "doc", "rtf", "odt", "hwp", "hwpx",
                "png", "jpg", "jpeg", "gif", "bmp", "webp", "html", "htm", "url", "pptx", "ppsx", "potx", "pptm", "http", "rest",
            ];
            every.extend(sources.iter().filter(|x| !every.contains(x)).collect::<Vec<_>>());
            rfd::FileDialog::new()
                .add_filter("Spreadsheets, documents and source files", &every)
                .add_filter("Excel Workbook", &["xlsx", "xlsm"])
                .add_filter("Excel 97-2003 Workbook (import)", &["xls"])
                .add_filter("Excel Binary Workbook (data import)", &["xlsb"])
                .add_filter("OpenDocument Spreadsheet (data import)", &["ods"])
                .add_filter("CSV", &["csv"])
                .add_filter("Markdown document", &["md", "markdown"])
                .add_filter("SQL file", &["sql"])
                .add_filter("Text document", &["txt"])
                .add_filter("API request", &["http", "rest"])
                .add_filter("Word document", &["docx", "dotx", "doc", "rtf", "odt"])
                .add_filter("Hangul document", &["hwp", "hwpx"])
                .add_filter("Pictures (view only)", &["png", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "ico", "tif", "tiff"])
                .add_filter("Web pages and site links (view only)", &["html", "htm", "url"])
                .add_filter("PowerPoint presentations (view only)", &["pptx", "ppsx", "potx", "pptm"])
                .add_filter("PDF document (read only)", &["pdf"])
                .add_filter("Source files (Java, JavaScript, Vue, YAML, XML…)", &sources)
                .add_filter("All files", &["*"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_picture: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_folder: Some(Box::new(|| rfd::FileDialog::new().pick_folder().and_then(|p| p.to_str().map(str::to_string)))),
        pick_any: Some(Box::new(|| rfd::FileDialog::new().pick_file().and_then(|p| p.to_str().map(str::to_string)))),
        pick_save: Some(Box::new(|suggested: &str| {
            let path = std::path::Path::new(suggested);
            let stem = path.file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Book1").to_string();
            // The suggested name's type (an export asks for .pdf, .csv or .html) comes first.
            const TYPES: [(&str, &str); 12] = [
                ("Excel Workbook (.xlsx)", "xlsx"),
                ("Word Document (.docx)", "docx"),
                ("Hangul Document (.hwpx)", "hwpx"),
                ("Hangul 97-2022 Document (.hwp)", "hwp"),
                ("CSV (.csv)", "csv"),
                ("Web Page (.html)", "html"),
                ("PDF (.pdf)", "pdf"),
                ("Markdown (.md)", "md"),
                ("SQL (.sql)", "sql"),
                ("Text (.txt)", "txt"),
                ("Site Link (.url)", "url"),
                ("API Request (.http)", "http"),
            ];
            let named = path.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
            // A source file (Main.java, Dockerfile) keeps its name and type, plain text after it.
            if let Some(source) = monun_engine::markdown::code::source_of(suggested) {
                let file = path.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(suggested).to_string();
                let x = named.unwrap_or_else(|| "*".to_string());
                let label = if x == "*" { source.label.to_string() } else { format!("{} (.{x})", source.label) };
                return rfd::FileDialog::new()
                    .set_file_name(file)
                    .add_filter(label, &[x])
                    .add_filter("Text (.txt)", &["txt"])
                    .add_filter("All files", &["*"])
                    .save_file()
                    .and_then(|p| p.to_str().map(str::to_string));
            }
            let ext = named.filter(|e| TYPES.iter().any(|(_, x)| x == e));
            let ext = ext.unwrap_or_else(|| "xlsx".to_string());
            let mut dialog = rfd::FileDialog::new().set_file_name(format!("{stem}.{ext}"));
            for (label, x) in TYPES.iter().filter(|(_, x)| *x == ext).chain(TYPES.iter().filter(|(_, x)| *x != ext)) {
                dialog = dialog.add_filter(*label, &[*x]);
            }
            dialog.save_file().and_then(|p| p.to_str().map(str::to_string))
        })),
        open_url: Some(Box::new(|url: &str| open_with_system(url))),
        printers: Some(print::discover()),
        print_native: Some(print::submit()),
        ..Default::default()
    }
}

/// Opens a URL or file with the system's default application.
fn open_with_system(target: &str) {
    // Last line of defence behind `link.open` and the click prompt: the system only ever gets
    // http, https and mailto addresses.
    let monun_engine::links::LinkKind::External(target) = monun_engine::links::classify(target) else {
        log::warn!("refused to open {target:?}: only http, https and mailto links are opened");
        return;
    };
    let target = target.as_str();
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(cmd).arg(target).spawn() {
        log::warn!("{cmd} {target}: {e}");
    }
}

fn icon() -> Option<egui::IconData> {
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../../../assets/app-icon/monun-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/kr.hulbo.monun.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

fn main() -> eframe::Result<()> {
    // First, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    // Commands catch their panics (`Session::execute`), but the default hook only prints to
    // standard error; this one sends every panic, caught or not, to the log file too, with a
    // backtrace when RUST_BACKTRACE is set.
    std::panic::set_hook(Box::new(|info| {
        let trace = std::backtrace::Backtrace::capture();
        let report = if trace.status() == std::backtrace::BacktraceStatus::Captured {
            format!("internal error: {info}\n{trace}")
        } else {
            format!("internal error: {info}")
        };
        // Standard error and the log file; standard error alone when RUST_LOG turned errors off.
        if log::log_enabled!(log::Level::Error) {
            log::error!("{report}");
        } else {
            // `eprintln!` panics on a broken stderr pipe, and a panic inside the panic hook aborts.
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("monun: {report}\n"));
        }
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("Monun {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut control_port: Option<u16> = std::env::var("MONUN_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let mut control_token: Option<String> = std::env::var("MONUN_CONTROL_TOKEN").ok();
    let mut sample: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--control" => {
                i += 1;
                control_port = args.get(i).and_then(|p| p.parse().ok());
            }
            "--control-token" => {
                i += 1;
                control_token = args.get(i).cloned();
            }
            "--sample" => {
                i += 1;
                sample = Some(args.get(i).cloned().unwrap_or_else(|| "sales".into()));
            }
            a if a.starts_with("-psn_") => {}
            a => files.push(a.to_string()),
        }
        i += 1;
    }
    // A 모눈 already running takes the files and comes forward; this start ends. Test runs (no
    // preferences, `--control`) keep to themselves.
    let instance_dir = config_dir().filter(|_| prefs_enabled() && control_port.is_none());
    if let Some(dir) = &instance_dir
        && single_instance::hand_over(dir, &files)
    {
        return Ok(());
    }
    // The log file lives in the settings folder, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences and runs a script drives (agents' test runs, `--control`)
    // log to standard error only, so they neither rotate away the user's own logs nor hold the
    // file while the user's 모눈 starts, which left that run with no log at all.
    if let Some(logger) = logger {
        match log_dir().filter(|_| prefs_enabled() && control_port.is_none()) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => log::info!("Monun {}, log file {}", env!("CARGO_PKG_VERSION"), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
        }
    }
    // Explorer's Open with lists 모눈 for spreadsheet files (`file_types`). Debug builds and runs an
    // agent or a test drives (no preferences, or the control channel) may be a build from another
    // checkout, so they leave the registration to the user's own app.
    #[cfg(target_os = "windows")]
    if !cfg!(debug_assertions) && prefs_enabled() && control_port.is_none() {
        file_types::register_running_app();
    }
    let mut session = Session::new();
    session.prefs.default_font = NEW_WORKBOOK_FONT.into();
    session.prefs.default_column_width = NEW_WORKBOOK_COLUMN_WIDTH;
    session.prefs.default_paper = NEW_WORKBOOK_PAPER.into();
    if let Some(s) = &sample
        && let Err(e) = session.execute("file.new", json!({"sample": s}))
    {
        log::warn!("sample {s}: {e}");
    }
    // Like Visual Studio, 모눈 starts on its start page rather than a blank workbook: every file
    // opened or made is a tab at the bottom of its one window.
    let mut app = SheetApp::new(session, services());
    app.use_file_tabs();
    // A diagram's own window carries the app's icon too.
    app.window_icon = icon().map(std::sync::Arc::new);
    app.watch_desktop_theme();
    app.after_engine(); // Show warnings from the sample.
    let control_port = control_port;
    CONTROLLED.store(control_port.is_some(), std::sync::atomic::Ordering::Relaxed);
    load_prefs(&mut app);
    // File groups are written as soon as they change, so a crash keeps them. A run that keeps
    // nothing (an agent's or a test's) starts on This PC with no working root: it never writes to
    // the user's projects.
    if persists() {
        app.services.save_ui = Some(Box::new(save_ui));
    } else {
        monun_ui_egui::projects::detach(&mut app);
    }
    // AutoRecover, and the unsaved work of a run that crashed. Not for agents' runs (`--control`,
    // no prefs), which must never pick up or leave the user's recovery copies.
    if prefs_enabled()
        && control_port.is_none()
        && let Some(dir) = config_dir()
    {
        app.start_recovery(&dir.join("recovery"));
    }
    // The files of the group in use open as they were left (not in a test run: its files are the
    // user's own).
    if persists() {
        monun_ui_egui::groups::restore(&mut app);
    }
    // Files named on the command line (a double-click on Windows and Linux) open as File › Open
    // opens them: a password-protected one asks for its password, and errors are shown, not
    // only logged. Each is a tab of its own.
    for f in &files {
        app.open_path(f);
    }
    // Later starts hand their files to this one.
    let instance = instance_dir.as_deref().and_then(single_instance::listen);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Monun")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([640.0, 420.0])
        .with_app_id("kr.hulbo.monun")
        .with_drag_and_drop(true);
    if cfg!(target_os = "macos") {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    // OpenGL through egui's glow renderer: on Windows it needs about 110 MB less memory than wgpu on
    // DirectX 12 and starts faster, and the exe carries no wgpu or shader compiler. Nothing in the
    // app paints with renderer-specific callbacks.
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut options = options;
    // winit has no file drag-and-drop on Wayland (only on X11), so dropping files from the file
    // manager showed a "no" cursor. Run through XWayland when it's there; MONUN_WAYLAND=1
    // keeps the native Wayland backend.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()) && std::env::var_os("MONUN_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // Registered before the event loop starts, so it catches the Finder event that launched us as
    // well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    eframe::run_native(
        "Monun",
        options,
        Box::new(move |cc| {
            let mut app = app;
            // Catalog the installed fonts in the background, so the font lists offer them and
            // cells set in them render with them once it is done.
            monun_ui_egui::fonts::start_scan();
            SheetApp::setup_context_for_language(&cc.egui_ctx, app.ui.dark, app.ui.language);
            if let Some(port) = control_port {
                match control_server::start(port, control_token, cc.egui_ctx.clone()) {
                    Ok(rx) => app.control_rx = Some(rx),
                    Err(e) => log::error!("control channel on port {port}: {e}"),
                }
            }
            if let Some(l) = &instance {
                l.wake_with(&cc.egui_ctx);
            }
            Ok(Box::new(App {
                app,
                instance,
                #[cfg(target_os = "macos")]
                menu: None,
                #[cfg(target_os = "macos")]
                events: apple_events.connect(&cc.egui_ctx),
                #[cfg(target_os = "windows")]
                web: web::Web::new(web_data(), !persists()),
            }))
        }),
    )
}
