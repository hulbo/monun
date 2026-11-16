//! Monun's egui frontend: an Excel-style window (title bar with Quick Access Toolbar,
//! ribbon, formula bar, grid, sheet tabs, status bar) over `monun-engine`.
//!
//! The UI is thin: it reads engine state and acts through [`SheetApp::run`] (engine commands by
//! id). Everything it can do is also reachable over the control channel ([`control`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod access_menu;
pub mod backstage;
mod border_preview;
mod cell_pictures;
pub mod chartview;
pub mod code_editor;
pub mod control;
mod craft_fonts_policy;
pub mod dialogs;
pub mod dnd;
pub mod doc_view;
#[cfg(not(target_arch = "wasm32"))]
pub mod doc_windows;
pub mod editor;
pub mod file_tabs;
mod floating_pictures;
pub mod fonts;
pub mod formula_bar;
mod formula_locale;
pub mod grid;
pub mod groups;
#[cfg(feature = "hwp")]
pub mod hwp_view;
#[cfg(not(feature = "hwp"))]
#[path = "hwp_view_off.rs"]
pub mod hwp_view;
pub mod i18n;
pub mod icons;
pub mod image_view;
pub mod keymap;
pub mod keytips;
pub mod macro_editor;
pub mod macro_ui;
pub mod memos;
pub mod outline;
#[cfg(any(feature = "pdf", feature = "word"))]
pub(crate) mod page_layout;
pub mod panes;
pub mod paper_view;
#[cfg(feature = "pdf")]
pub mod pdf_view;
#[cfg(not(feature = "pdf"))]
#[path = "pdf_view_off.rs"]
pub mod pdf_view;
pub mod pivot_pane;
pub mod printing;
pub mod projects;
pub mod recovery;
pub mod ribbon;
pub mod selection_glide;
pub mod slides_view;
pub mod tabs;
pub mod text_box;
pub mod theme;
pub mod web_view;
pub mod widgets;
mod window_ui;
#[cfg(feature = "word")]
pub mod word_view;
#[cfg(not(feature = "word"))]
#[path = "word_view_off.rs"]
pub mod word_view;

pub use window_ui::WindowUi;

use std::collections::HashMap;

use monun_engine::core::{CellRef, RangeRef};
use monun_engine::{Session, UiRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

pub use control::{ControlRequest, ControlResponse};

mod app;
mod clock;
mod services;
mod state;

pub use clock::{monotonic_ms, now_ms};
pub use services::{Chrome, Inbox, Services};
pub use state::{NoticeKind, OpenWindow, SaveAnswer, SheetView};
pub mod desktop_theme;

/// Persisted UI preferences.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    /// The manual preference, retained when following the system appearance.
    pub dark: bool,
    pub system_theme: bool,
    /// The interface's colour (File › Options › General).
    #[serde(deserialize_with = "theme::lenient_accent")]
    pub accent: theme::Accent,
    pub ribbon_tab: String,
    pub ribbon_collapsed: bool,
    pub formula_bar_expanded: bool,
    pub status_bar: bool,
    /// Motion in the UI: the selection outline glides to a new selection (View ▸ Animations).
    pub animations: bool,
    /// Enable AutoComplete for cell values (File ▸ Options ▸ Editing): typed text is completed
    /// from the one matching entry in the same column.
    pub autocomplete: bool,
    /// AutoSave (the title-bar switch): save saved-to-file .xlsx workbooks shortly after every
    /// change. A preference like the rest of this struct — it stays on across restarts (#1901).
    pub autosave: bool,
    pub recent: Vec<String>,
    /// One-shot keytip request for a ribbon control: set when a keytip action resolves to
    /// [`keytips::Action::Control`], consumed (and cleared) by the control's render site.
    #[serde(skip)]
    pub keytip_invoke: Option<&'static str>,
    /// Interface language. English by default (so tests and headless renders never depend on the
    /// host's locale); the desktop app starts from [`i18n::Language::system`] when `ui.json` has
    /// no usable language (see [`i18n::saved_language`]). An unknown saved value reads as English
    /// rather than failing the whole `UiState`.
    #[serde(deserialize_with = "i18n::lenient")]
    pub language: i18n::Language,
    /// Interface size in percent on top of the desktop's scale (100 = Auto), applied as egui's
    /// zoom factor so the ribbon, menus, dialogs and grid grow together. Kept within
    /// [`INTERFACE_SCALE_MIN`]..=[`INTERFACE_SCALE_MAX`]; a malformed saved value reads as 100.
    #[serde(deserialize_with = "lenient_interface_scale")]
    pub interface_scale: u16,
    /// File groups ([`groups`]), with file tabs: each one's files, and the one in use by name. They
    /// are the groups of the project in use: This PC's own (as before projects), or a copy of a
    /// project of the root's, read again from its file at start ([`projects`]).
    pub groups: Vec<groups::FileGroup>,
    pub group: String,
    /// The working root folder (File › Options › General): the folder of documents, kept in git,
    /// whose `.monun` folder holds the projects.
    pub root: Option<String>,
    /// The project of the root in use; `None` for This PC.
    pub project: Option<String>,
    /// This PC's groups, set aside while a project of the root is in use.
    pub this_pc: Option<projects::GroupSet>,
    /// This PC's place in each project of the root it has left: the group in use, and each group's
    /// file in front and last use.
    pub project_views: Vec<projects::ProjectView>,
    /// The group panel ([`groups::panel`]) shows, whichever file is in front: it lists the files
    /// (there are no file tabs). Another pane shows in its place while that one is open.
    pub group_panel: bool,
    /// The group panel is tucked into the window's edge: a strip there shows it over the files while
    /// clicked ([`panes`]); its pin keeps it shown again.
    pub group_tucked: bool,
    /// The order the group panel lists the groups and their files in (its Sort button).
    #[serde(deserialize_with = "groups::lenient")]
    pub group_order: groups::GroupOrder,
    #[serde(deserialize_with = "groups::lenient")]
    pub file_order: groups::FileOrder,
    /// The memo and group panels float over the files as a layer, in `side_corner`, rather than
    /// take a column of their own (on the left with `side_left`, else on the right).
    pub side_layer: bool,
    pub side_corner: panes::Corner,
    pub side_left: bool,
    /// The layer's size as made by dragging its edges; `None` until then (a column's width, as
    /// tall as it needs).
    pub side_layer_w: Option<f32>,
    pub side_layer_h: Option<f32>,
    /// The memos on Markdown files' paragraphs ([`memos`]), by the files' paths: those of files
    /// outside the working root. The root's files keep theirs in the root ([`memos::root_file`]),
    /// but for memos kept here while the root's memo file couldn't be read or written, and those
    /// of a run that keeps nothing.
    pub memos: Vec<memos::DocMemo>,
    /// How Markdown files were last shown, most recent first (their .md keeps the text only).
    pub doc_views: Vec<DocViewMemory>,
    /// Contents (the headings) shows beside Markdown documents.
    pub doc_outline: bool,
}

/// How a Markdown file was last shown: Edit, Side by Side or View, Fit Width, the zoom and where it
/// was being read. Its .md keeps the text only, so 모눈 remembers the rest by the file's path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocViewMemory {
    pub path: String,
    /// `edit`, `split` or `view`; empty when never chosen.
    pub mode: String,
    pub fit_width: bool,
    pub zoom: u16,
    /// The first line in view (0-based), to read on from there; a PDF's first page in view.
    pub line: usize,
    /// A PDF shown two pages across.
    pub spread: bool,
}

/// The most Markdown files whose view is remembered.
pub const DOC_VIEWS_MAX: usize = 200;

pub const INTERFACE_SCALE_MIN: u16 = 75;
pub const INTERFACE_SCALE_MAX: u16 = 200;

/// A requested interface size, rounded and clamped to the supported range; `None` when it isn't
/// a finite number.
pub fn clamp_interface_scale(percent: f64) -> Option<u16> {
    percent.is_finite().then(|| percent.round().clamp(INTERFACE_SCALE_MIN as f64, INTERFACE_SCALE_MAX as f64) as u16)
}

fn lenient_interface_scale<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let v = Json::deserialize(d)?;
    Ok(v.as_f64().and_then(clamp_interface_scale).unwrap_or(100))
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            dark: false,
            system_theme: false,
            accent: theme::Accent::Green,
            ribbon_tab: "Home".into(),
            ribbon_collapsed: false,
            formula_bar_expanded: false,
            status_bar: true,
            animations: true,
            autocomplete: true,
            autosave: false,
            recent: vec![],
            keytip_invoke: None,
            language: i18n::Language::En,
            interface_scale: 100,
            groups: vec![],
            group: String::new(),
            root: None,
            project: None,
            this_pc: None,
            project_views: vec![],
            doc_views: vec![],
            group_panel: true,
            group_tucked: false,
            group_order: groups::GroupOrder::Custom,
            file_order: groups::FileOrder::Tabs,
            side_layer: false,
            side_corner: panes::Corner::TopRight,
            side_left: false,
            side_layer_w: None,
            side_layer_h: None,
            memos: vec![],
            doc_outline: false,
        }
    }
}

impl UiState {
    /// The state saved by an earlier run, as the app starts with it. The ribbon tab is written to
    /// the preferences but not restored: Excel opens on Home every launch, so a contextual tab
    /// (Table Design) cannot come back with nothing selected to justify it.
    pub fn at_startup(mut self) -> Self {
        self.ribbon_tab = "Home".into();
        self
    }

    pub fn theme_preference(&self) -> egui::ThemePreference {
        if self.system_theme {
            egui::ThemePreference::System
        } else if self.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        }
    }

    /// The zoom factor for the interface size preference.
    pub fn zoom_factor(&self) -> f32 {
        clamp_interface_scale(self.interface_scale as f64).unwrap_or(100) as f32 / 100.0
    }

    pub fn theme_mode(&self) -> &'static str {
        if self.system_theme {
            "system"
        } else if self.dark {
            "dark"
        } else {
            "light"
        }
    }
}

/// A dismissible bar under the ribbon, in the spirit of Excel's message bar. Used to tell the
/// user what an opened file carried that Monun handled differently (e.g. preserved macros).
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub kind: NoticeKind,
    /// Optional button: `(label, dialog name)` — opens that dialog when clicked.
    pub action: Option<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Perf {
    pub frame_ms: f64,
    pub grid_ms: f64,
    pub fps: f64,
    last: f64,
    /// This frame's logic pass (the control channel, AutoSave, AutoRecover), and the saves in it.
    pub logic_ms: f64,
    pub save_ms: f64,
    /// The slowest command this frame: its id and time.
    pub command: Option<(String, f64)>,
    /// The latest frames' times (logic and interface together), newest last.
    pub recent: std::collections::VecDeque<f64>,
}

/// A frame slower than this is logged, with where its time went.
pub const SLOW_FRAME_MS: f64 = 250.0;

impl Perf {
    /// Notes how long `command` took this frame, keeping the slowest.
    pub fn note_command(&mut self, command: &str, ms: f64) {
        if self.command.as_ref().is_none_or(|(_, slowest)| ms > *slowest) {
            self.command = Some((command.to_owned(), ms));
        }
    }

    /// The slowest of the latest frames (ms).
    pub fn slowest(&self) -> f64 {
        self.recent.iter().copied().fold(0.0, f64::max)
    }

    /// Ends a frame: keeps its time, logs it when it was slow, and clears what it measured.
    fn finish_frame(&mut self) {
        let total = self.logic_ms + self.frame_ms;
        if self.recent.len() >= 240 {
            self.recent.pop_front();
        }
        self.recent.push_back(total);
        if total > SLOW_FRAME_MS {
            let command = self.command.as_ref().map_or_else(|| "none".to_owned(), |(id, ms)| format!("{id} {ms:.0} ms"));
            log::warn!(
                "slow frame: {total:.0} ms (logic {:.0} ms, of it saves {:.0} ms; interface {:.0} ms, of it sheet or document {:.0} ms; slowest command {command})",
                self.logic_ms,
                self.save_ms,
                self.frame_ms,
                self.grid_ms
            );
        }
        self.command = None;
        self.save_ms = 0.0;
    }
}

pub struct SheetApp {
    pub session: Session,
    pub ui: UiState,
    /// The window chrome to draw (see [`Chrome`]).
    pub chrome: Chrome,
    /// Scroll position per (window id, sheet index).
    pub views: HashMap<(u64, usize), SheetView>,
    pub editor: Option<editor::EditState>,
    pub text_box_editor: Option<text_box::EditState>,
    /// Document tabs: the draft being typed, each document's mode.
    pub doc_view: doc_view::State,
    /// PDF tabs: each open PDF's pages, text and render threads.
    pub pdf: pdf_view::State,
    /// Word tabs: each open document's session, pages and input method.
    pub word: word_view::State,
    /// 한글 tabs: each open document's thread, pages and input method.
    pub hwp: hwp_view::State,
    /// Picture tabs: each open picture's texture.
    pub images: image_view::State,
    /// Web tabs (an HTML file, a site link): the pages the desktop app's web views show.
    pub web: web_view::State,
    /// The memo panel's memo being written and the one picked.
    pub memos: memos::State,
    pub dialog: Option<dialogs::Dialog>,
    /// The File tab's Backstage view, when open (UI-only; replaces the ribbon, grid and tabs).
    pub backstage: Option<backstage::Backstage>,
    pub message: Option<(String, String)>,
    pub toast: Option<(String, f64)>,
    /// Dismissible info bar under the ribbon (open-time notices).
    pub notice: Option<Notice>,
    /// AutoRecover's folder for this run: copies of unsaved work, reopened after a crash. `None`
    /// where the host keeps no settings (tests, the web, agents' runs).
    pub recovery: Option<recovery::Recovery>,
    pub services: Services,
    pub printing: printing::State,
    print_feedback_seen: Option<printing::Feedback>,
    /// Synthetic input from agents, one batch per step (drags need several steps).
    pub synthetic: std::collections::VecDeque<Vec<egui::Event>>,
    /// The front synthetic batch was injected into `RawInput` but no `ui` pass has applied it
    /// yet. eframe retains that unconsumed input across hidden logic-only ticks; the flag makes
    /// the hook offer it once and asks for a hidden paint until a UI pass applies it.
    pub(crate) synthetic_pending: bool,
    /// `cumulative_pass_nr` when the front batch was last injected. A completed egui pass that
    /// did not apply it (the font gate's early return) consumed the events with `begin_pass`
    /// without reaching input handling, so the hook must offer the same step again.
    pub(crate) synthetic_injected_at_pass: Option<u64>,
    pub control_rx: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    pub grid: grid::GridState,
    pub perf: Perf,
    pub fonts_ready: bool,
    fonts_set: bool,
    /// Test harness set: rebuilds use the deterministic pinned font set instead of the host scan.
    prepared_fonts: bool,
    /// Workbook fonts bound into the current font set; a change rebinds (installed fonts only).
    fonts_bound: Vec<String>,
    /// The open workbooks' (uid, revision), folded, that `fonts_bound` was computed for, so the
    /// style tables are walked only when a workbook changes.
    fonts_seen: (u64, u64),
    /// Set while a font-set change is pending activation: skip this frame's paint, like the
    /// startup `fonts_ready` gate (egui applies new definitions at the next pass).
    fonts_rebind: bool,
    /// Han face order the installed fonts were built with; a language switch rebuilds the fonts
    /// only when it changes this (see `theme::HanOrder`).
    fonts_han: Option<theme::HanOrder>,
    /// Whether the installed font set carries the Han faces (see `theme::HanOrder::eager`).
    fonts_with_han: bool,
    effective_dark: bool,
    /// The desktop's light/dark choice, watched while the app runs (Linux has no winit answer).
    desktop_theme: desktop_theme::DesktopTheme,
    /// The last answer [`desktop_theme`] gave, so a frame that asks costs nothing.
    desktop_dark: Option<bool>,
    pub name_box: Option<String>,
    /// Transient ribbon keyboard navigation; never saved with UI preferences.
    pub keytips: keytips::KeyTips,
    pub(crate) shots: control::Shots,
    /// Monotonic epoch of [`control::CaptureGeometry`]: any resize/DPI/zoom/surface change bumps
    /// it, including a change back to an earlier value, so a capture taken in another layout is
    /// never saved against the current geometry.
    pub(crate) capture_epoch: u64,
    /// The geometry `capture_epoch` describes; `None` before the first frame.
    pub(crate) capture_geometry: Option<control::CaptureGeometry>,
    /// The recorded geometry came from a native `RawInput` frame (see `raw_input_hook`). When
    /// false (a bare test/embedder that never runs the hook) capture code falls back to egui's
    /// current input instead of reusing a stale recorded value.
    pub(crate) capture_geometry_from_raw: bool,
    /// Chart selected on the sheet (id).
    pub selected_chart: Option<u32>,
    /// The system clipboard's text when Monun last copied without publishing to it (a
    /// control-channel or MCP `edit.copy`/`edit.cut`). While the system clipboard still holds this
    /// text, Monun's copy is the newer one and Paste uses it.
    clipboard_text_at_copy: Option<String>,
    /// The system clipboard's text right after Monun's last cut (the published text, or the
    /// text noted for an unpublished cut); `None` after a copy, or once the clipboard is seen
    /// holding other text. Once the cut has been pasted, Paste leaves this text alone.
    cut_text: Option<String>,
    /// A run of Tabs: (doc uid, sheet, column it started in, cell the last Tab landed on).
    /// Enter on that cell goes to the next row in the starting column, as in Excel (#911).
    tab_run: Option<(u64, usize, u32, CellRef)>,
    /// Automate › Macros: relative recording, recent macros, a macro's question.
    pub macros: macro_ui::MacroUi,
    pub started: f64,
    /// Every unsaved workbook has been answered for quitting (or `app.quit` asked): the next
    /// window close goes through.
    pub allow_close: bool,
    /// View › Switch Windows opens its list on the next frame the View tab draws (asked for by a
    /// key tip, the command search or a menu, possibly while another tab was showing).
    pub(crate) open_switch_windows: bool,
    /// The window whose interface state the per-window fields above hold (see [`WindowUi`]).
    ui_window: Option<u64>,
    /// The other windows' interface state, by window id.
    parked: HashMap<u64, WindowUi>,
    /// Each window is a native window of its own ([`doc_windows`] on the desktop): a window's
    /// close button closes that window, and quitting is asked for separately. Without it (web,
    /// tests, embedding hosts) one window shows the active workbook window, and its close button
    /// quits.
    pub native_windows: bool,
    /// The app's icon, for windows it opens of its own (a diagram's).
    pub window_icon: Option<std::sync::Arc<egui::IconData>>,
    /// When the open text files were last checked for another program's changes (monotonic ms),
    /// and the changes already told of, by document, that wait for an answer.
    pub(crate) files_checked: f64,
    pub(crate) changes_told: std::collections::HashMap<u64, Option<monun_engine::io::DiskStamp>>,
    /// How far the memo or group panel's title has been dragged, while it is.
    pub(crate) side_drag: Option<egui::Vec2>,
    /// The layer's height as it showed last frame.
    pub(crate) side_layer_shown_h: f32,
    /// Visual Studio's one window of files: every open file (a workbook, a Markdown file…) is listed
    /// in the group panel, a workbook shows its sheet tabs as in Excel, and with no file open a start
    /// page shows instead of a blank workbook. The desktop app turns it on; without it (web, tests,
    /// embedding hosts) one workbook shows, its sheet tabs as in Excel.
    pub file_tabs: bool,
    /// The tucked group panel shows over the files, its strip clicked ([`UiState::group_tucked`]).
    pub(crate) group_peek: bool,
    /// Quitting is asking about unsaved files: the group in use keeps the files it closes
    /// ([`groups::sync`]).
    pub(crate) groups_frozen: bool,
    /// The project of the root in use, as read in this run ([`projects`]); `None` for This PC, or
    /// before it is read (a run that doesn't restore the user's files never writes to it).
    pub(crate) project_loaded: Option<projects::Loaded>,
    /// The memos on the working root's files, kept in the root ([`memos::root_file`]): read for the
    /// root in use, `None` with no root.
    pub(crate) root_memos: Option<memos::root_file::Store>,
    /// Files of the group in use that couldn't be opened on this PC, while a project of the root
    /// is in use: they stay in its list for the PCs that have them ([`groups::sync`]).
    pub(crate) unopened: Vec<String>,
    /// Where the file in front shows (a sheet's grid, a document, the start page), as drawn last:
    /// toasts show over it.
    pub(crate) files_rect: Option<egui::Rect>,
    /// View › Arrange All's layout and windows, waiting for the native windows to move.
    pub arrange_windows: Option<(String, Vec<u64>)>,
}

/// Strips the engine's error prefixes for message boxes.
pub fn clean_error(e: &str) -> String {
    if let Some(rest) = e.split_once("`: ").map(|(_, r)| r) {
        return rest.to_string();
    }
    e.to_string()
}

pub fn cell(r: u32, c: u32) -> CellRef {
    CellRef::new(r, c)
}

#[cfg(test)]
mod perf_tests {
    use super::Perf;

    #[test]
    fn frames_keep_the_slowest_command_and_the_latest_times() {
        let mut perf = Perf::default();
        perf.note_command("sheet.activate", 3.0);
        perf.note_command("doc.setText", 40.0);
        perf.note_command("cell.set", 1.0);
        assert_eq!(perf.command.as_ref().map(|(id, _)| id.as_str()), Some("doc.setText"));
        (perf.logic_ms, perf.frame_ms) = (5.0, 300.0);
        perf.finish_frame();
        assert!(perf.command.is_none(), "a frame's command is its own");
        assert_eq!(perf.slowest(), 305.0);
        for _ in 0..300 {
            (perf.logic_ms, perf.frame_ms) = (0.0, 2.0);
            perf.finish_frame();
        }
        assert_eq!(perf.recent.len(), 240, "only the latest frames are kept");
        assert_eq!(perf.slowest(), 2.0);
    }
}
