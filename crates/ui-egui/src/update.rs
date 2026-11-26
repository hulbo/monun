//! Updates from the GitHub releases, as NanoGit updates itself. An installed 모눈 looks at the latest
//! release's `latest.json` a little after it starts and every six hours (Options › General turns
//! that off), and says so in the info bar when there is a newer version. Update downloads its
//! setup, checks it against the SHA-256 `latest.json` gives, and runs it once 모눈 has closed (the
//! unsaved files asked about first, as when quitting); the setup starts the new version when it is
//! done, and that one says it was updated. The desktop's "모눈 업데이트" shortcut
//! (`monun.exe --update`, which the setup makes) does all of it at once without asking.
//!
//! Only an installed copy updates itself ([`State::enabled`], which the desktop app sets): a build
//! from the source checkout updates with the checkout, and test runs never look.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::SheetApp;

/// The latest release's description, which the release script uploads with its setup.
pub const LATEST: &str = "https://github.com/hulbo/monun/releases/latest/download/latest.json";
/// Where a setup may be downloaded from: this repository's releases, nowhere else.
const SETUPS: &str = "https://github.com/hulbo/monun/releases/download/";
/// The first look, this long after the start (ms), then one every [`EVERY_MS`].
const FIRST_MS: f64 = 15_000.0;
const EVERY_MS: f64 = 6.0 * 60.0 * 60.0 * 1000.0;

/// A release as `latest.json` describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    pub version: String,
    /// Its setup.
    pub url: String,
    /// The setup's SHA-256, in hexadecimal.
    pub sha256: String,
    /// What changed, a line each.
    pub notes: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

/// Where the update stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    /// A newer version is out.
    Available(Release),
    /// Its setup on its way: bytes got, of how many.
    Downloading {
        release: Release,
        got: u64,
        total: Option<u64>,
    },
    /// Its setup downloaded and checked, ready to run.
    Ready {
        release: Release,
        setup: PathBuf,
    },
    Failed(String),
}

/// What the thread looking or downloading reports.
enum Msg {
    Found(Result<Option<Release>, String>),
    Progress(u64, Option<u64>),
    Downloaded(Result<PathBuf, String>),
}

/// The app's update: whether it may update itself, where it stands, and what was asked.
#[derive(Default)]
pub struct State {
    /// This copy updates itself: an installed one (the desktop app says so), not a build from the
    /// source checkout or a test run.
    pub enabled: bool,
    pub phase: Phase,
    inbox: Arc<Mutex<Vec<Msg>>>,
    /// When to look next (monotonic ms); `None` until the first frame.
    next: Option<f64>,
    /// A look asked for in Options: it says so when this is the latest version, or why it couldn't.
    manual: bool,
    /// Started by the shortcut (`--update`): it downloads and installs without asking.
    unattended: bool,
    /// Install once downloaded (Update Now was pressed, or the shortcut).
    then_install: bool,
    /// The setup to run once 모눈 has closed.
    install: Option<PathBuf>,
}

impl State {
    /// The setup to run now that 모눈 is closing, if an update is waiting for it.
    pub fn take_install(&mut self) -> Option<PathBuf> {
        self.install.take()
    }

    /// The quit an install waited for was cancelled (Cancel on a save question): it waits.
    pub(crate) fn quit_cancelled(&mut self) {
        self.install = None;
        self.unattended = false;
    }
}

/// Whether version `latest` is newer than `current` (`1.2.10` after `1.2.9`).
pub fn newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() };
    let (a, b) = (parts(latest), parts(current));
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..n).map(|i| (at(&a, i), at(&b, i))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

/// This 모눈's version.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Each frame: the scheduled look, and what the thread reported.
pub(crate) fn tick(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.update.enabled {
        return;
    }
    let now = crate::monotonic_ms();
    let next = *app.update.next.get_or_insert(now + if app.update.unattended { 0.0 } else { FIRST_MS });
    if app.update.phase == Phase::Idle && now >= next && (app.ui.update_check || app.update.unattended) {
        look(app, ctx);
    }
    let msgs: Vec<Msg> = app.update.inbox.lock().map(|mut m| std::mem::take(&mut *m)).unwrap_or_default();
    for msg in msgs {
        heard(app, ctx, msg);
    }
    // The shortcut's update goes on by itself: found, downloaded, installed.
    if app.update.unattended {
        match app.update.phase.clone() {
            Phase::Available(release) => download(app, ctx, release),
            Phase::Ready { .. } if app.update.install.is_none() => install(app, ctx),
            _ => {}
        }
    }
    if matches!(app.update.phase, Phase::Checking | Phase::Downloading { .. }) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// Looks for a newer version now (Options' Check for Updates): this says so when there is none.
pub(crate) fn check_now(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.update.enabled {
        let lang = app.ui.language;
        app.message = Some(("Monun".into(), lang.tr("This Monun was built from its source: it updates with the source checkout.").to_string()));
        return;
    }
    if matches!(app.update.phase, Phase::Idle | Phase::Failed(_)) {
        app.update.manual = true;
        look(app, ctx);
    } else {
        app.open_dialog("update", json!({}));
    }
}

/// The desktop shortcut's update (`--update`): looks now, then downloads and installs without asking.
pub fn run_unattended(app: &mut SheetApp) {
    if !app.update.enabled {
        let lang = app.ui.language;
        app.message = Some(("Monun".into(), lang.tr("This Monun was built from its source: it updates with the source checkout.").to_string()));
        return;
    }
    app.update.unattended = true;
    app.update.then_install = true;
    app.update.manual = true;
    app.update.next = Some(0.0);
    if matches!(app.update.phase, Phase::Failed(_)) {
        app.update.phase = Phase::Idle;
    }
}

/// Update Now: downloads the newer version, then installs it.
pub(crate) fn start(app: &mut SheetApp, ctx: &egui::Context) {
    app.update.then_install = true;
    app.notice = None;
    match app.update.phase.clone() {
        Phase::Available(release) => download(app, ctx, release),
        Phase::Ready { .. } => install(app, ctx),
        _ => {}
    }
}

/// Later: the dialog closes; a download on its way goes on, but waits to be installed.
pub(crate) fn later(app: &mut SheetApp) {
    app.update.then_install = false;
}

fn look(app: &mut SheetApp, ctx: &egui::Context) {
    app.update.phase = Phase::Checking;
    let (inbox, ctx) = (app.update.inbox.clone(), ctx.clone());
    std::thread::spawn(move || {
        let found = net::latest();
        if let Ok(mut m) = inbox.lock() {
            m.push(Msg::Found(found));
        }
        ctx.request_repaint();
    });
}

fn download(app: &mut SheetApp, ctx: &egui::Context, release: Release) {
    app.update.phase = Phase::Downloading { release: release.clone(), got: 0, total: None };
    let (inbox, ctx) = (app.update.inbox.clone(), ctx.clone());
    std::thread::spawn(move || {
        let progress = {
            let (inbox, ctx) = (inbox.clone(), ctx.clone());
            move |got: u64, total: Option<u64>| {
                if let Ok(mut m) = inbox.lock() {
                    m.retain(|x| !matches!(x, Msg::Progress(..)));
                    m.push(Msg::Progress(got, total));
                }
                ctx.request_repaint();
            }
        };
        let got = net::download(&release, &std::env::temp_dir().join("monun-update"), &progress);
        if let Ok(mut m) = inbox.lock() {
            m.push(Msg::Downloaded(got));
        }
        ctx.request_repaint();
    });
}

/// Runs the setup once 모눈 has closed: the unsaved files are asked about first, and Cancel on any
/// of them keeps 모눈 open (the update waits).
fn install(app: &mut SheetApp, ctx: &egui::Context) {
    let Phase::Ready { setup, .. } = &app.update.phase else { return };
    app.update.install = Some(setup.clone());
    app.request_quit(ctx);
}

/// What the thread reported.
fn heard(app: &mut SheetApp, ctx: &egui::Context, msg: Msg) {
    let lang = app.ui.language;
    let now = crate::monotonic_ms();
    match msg {
        Msg::Found(found) => {
            app.update.next = Some(now + EVERY_MS);
            let manual = std::mem::take(&mut app.update.manual);
            match found {
                Ok(Some(release)) if newer(&release.version, current()) => {
                    app.update.phase = Phase::Available(release.clone());
                    if app.update.unattended {
                        // `tick` downloads it.
                    } else if manual {
                        app.open_dialog("update", json!({}));
                    } else {
                        let text = lang.tr("Monun {version} is out.").replace("{version}", &release.version);
                        app.notice = Some(crate::Notice { text, kind: crate::NoticeKind::Info, action: Some(("Update…".into(), "update".into())) });
                    }
                }
                Ok(_) => {
                    app.update.phase = Phase::Idle;
                    app.update.unattended = false;
                    if manual {
                        app.toast = Some((lang.tr("This is the latest version ({version}).").replace("{version}", current()), crate::now_ms()));
                    }
                }
                Err(e) => {
                    app.update.phase = Phase::Idle;
                    app.update.unattended = false;
                    log::warn!("update check: {e}");
                    if manual {
                        app.message = Some(("Monun".into(), format!("{}\n{}", lang.tr("Couldn't look for a newer version:"), lang.tr(&e))));
                    }
                }
            }
        }
        Msg::Progress(got, total) => {
            if let Phase::Downloading { release, .. } = &app.update.phase {
                app.update.phase = Phase::Downloading { release: release.clone(), got, total };
            }
        }
        Msg::Downloaded(Ok(setup)) => {
            let Phase::Downloading { release, .. } = app.update.phase.clone() else { return };
            app.update.phase = Phase::Ready { release: release.clone(), setup };
            if app.update.then_install {
                install(app, ctx);
            } else {
                let text = lang.tr("Monun {version} is downloaded, ready to install.").replace("{version}", &release.version);
                app.notice = Some(crate::Notice { text, kind: crate::NoticeKind::Info, action: Some(("Update…".into(), "update".into())) });
            }
        }
        Msg::Downloaded(Err(e)) => {
            log::warn!("update download: {e}");
            app.update.phase = Phase::Failed(e.clone());
            app.update.unattended = false;
            app.message = Some(("Monun".into(), format!("{}\n{}", lang.tr("Couldn't download the new version:"), lang.tr(&e))));
        }
    }
}

/// The Update dialog: the new version and what changed, Update Now (with its download's progress)
/// and Later.
pub(crate) fn dialog(app: &mut SheetApp, ui: &mut egui::Ui, open: &mut bool) {
    use egui::RichText;
    let lang = app.ui.language;
    let t = crate::theme::Tokens::get(ui.ctx());
    let release = match &app.update.phase {
        Phase::Available(r) | Phase::Downloading { release: r, .. } | Phase::Ready { release: r, .. } => r.clone(),
        Phase::Checking => {
            ui.label(lang.tr("Looking for a newer version…"));
            if crate::widgets::dialog_buttons(ui, &[crate::widgets::DialogButton::neutral("Close")]).is_some() {
                *open = false;
            }
            return;
        }
        _ => {
            *open = false;
            return;
        }
    };
    ui.add_space(4.0);
    ui.label(RichText::new(lang.tr("Monun {version} is out.").replace("{version}", &release.version)).strong());
    ui.label(RichText::new(lang.tr("This is {version}.").replace("{version}", current())).color(t.text_dim));
    if !release.notes.trim().is_empty() {
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            ui.label(RichText::new(release.notes.trim()).font(crate::theme::ui_font(12.0)));
        });
    }
    ui.add_space(8.0);
    let downloading = match &app.update.phase {
        Phase::Downloading { got, total, .. } => {
            let part = total.filter(|t| *t > 0).map(|t| *got as f32 / t as f32);
            let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_048_576.0);
            let text = match total {
                Some(t) => format!("{} / {}", mb(*got), mb(*t)),
                None => mb(*got),
            };
            ui.add(egui::ProgressBar::new(part.unwrap_or(0.0)).text(text).animate(part.is_none()));
            true
        }
        _ => false,
    };
    ui.label(
        RichText::new(lang.tr("Monun closes to install it, asking about unsaved files first, and starts again when it is done.")).color(t.text_dim),
    );
    ui.add_space(8.0);
    let row = [crate::widgets::DialogButton::confirm("Update Now"), crate::widgets::DialogButton::neutral("Later")];
    match crate::widgets::dialog_buttons(ui, &row) {
        Some(0) if !downloading => {
            start(app, ui.ctx());
            if app.update.install.is_some() {
                *open = false;
            }
        }
        Some(1) => {
            later(app);
            *open = false;
        }
        _ => {}
    }
}

/// Says the update just installed, once: `from` is the version it replaced.
pub fn updated_from(app: &mut SheetApp, from: &str) {
    if from.trim().is_empty() || from.trim() == current() {
        return;
    }
    let lang = app.ui.language;
    let text = lang.tr("Updated Monun from {old} to {new}.").replace("{old}", from.trim()).replace("{new}", current());
    app.toast = Some((text, crate::now_ms()));
}

#[cfg(feature = "api")]
mod net {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use sha2::Digest;

    use super::{Release, SETUPS};

    /// The most a setup may weigh.
    const SETUP_MAX: u64 = 300 * 1024 * 1024;

    fn agent(timeout: Duration) -> ureq::Agent {
        use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
        let tls = TlsConfig::builder().provider(TlsProvider::Rustls).root_certs(RootCerts::PlatformVerifier).build();
        ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .user_agent(concat!("Monun/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }

    /// The latest release; `None` when it says nothing (a release from before `latest.json`).
    pub fn latest() -> Result<Option<Release>, String> {
        let mut answer = agent(Duration::from_secs(30)).get(super::LATEST).call().map_err(|e| e.to_string())?;
        match answer.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            s => return Err(format!("HTTP {s}")),
        }
        let text = answer.body_mut().with_config().limit(256 * 1024).read_to_string().map_err(|e| e.to_string())?;
        serde_json::from_str::<Release>(&text).map(Some).map_err(|e| format!("latest.json: {e}"))
    }

    /// Downloads the release's setup into `dir`, checking it against its SHA-256 as it comes;
    /// `progress` hears the bytes got. Its path.
    pub fn download(release: &Release, dir: &Path, progress: &dyn Fn(u64, Option<u64>)) -> Result<PathBuf, String> {
        let name = release.url.rsplit('/').next().unwrap_or_default();
        let safe = !name.is_empty() && name.ends_with(".exe") && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        if !release.url.starts_with(SETUPS) || !safe {
            return Err(format!("The new version's address isn't one of Monun's releases: {}", release.url));
        }
        let want = release.sha256.trim().to_ascii_lowercase();
        if want.len() != 64 || !want.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("The new version comes without a valid SHA-256.".into());
        }
        // An earlier download (another version's, or one cut short) goes first.
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(name);
        let mut answer = agent(Duration::from_secs(30 * 60)).get(&release.url).call().map_err(|e| e.to_string())?;
        if answer.status().as_u16() != 200 {
            return Err(format!("HTTP {}", answer.status().as_u16()));
        }
        let total = answer.body().content_length();
        let mut reader = answer.body_mut().with_config().limit(SETUP_MAX).reader();
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut hash = sha2::Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut got: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
            hash.update(chunk);
            file.write_all(chunk).map_err(|e| e.to_string())?;
            got = got.saturating_add(n as u64);
            progress(got, total);
        }
        file.flush().map_err(|e| e.to_string())?;
        drop(file);
        let have: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if have != want {
            let _ = std::fs::remove_file(&path);
            return Err("The download doesn't match its SHA-256, so it wasn't run.".into());
        }
        Ok(path)
    }
}

/// Without the `api` feature (the web app) nothing is looked for.
#[cfg(not(feature = "api"))]
mod net {
    use std::path::{Path, PathBuf};

    use super::Release;

    pub fn latest() -> Result<Option<Release>, String> {
        Ok(None)
    }

    pub fn download(_release: &Release, _dir: &Path, _progress: &dyn Fn(u64, Option<u64>)) -> Result<PathBuf, String> {
        Err("Updates need the desktop app.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_newer_by_its_numbers() {
        assert!(newer("1.1.10", "1.1.9"));
        assert!(newer("v1.2.0", "1.1.9"));
        assert!(newer("2.0", "1.9.9"));
        assert!(!newer("1.1.9", "1.1.9"));
        assert!(!newer("1.1.8", "1.1.9"));
        assert!(!newer("1.1", "1.1.0"));
        assert!(!newer("", "1.0.0"));
    }

    #[test]
    fn latest_json_reads_as_a_release() {
        let text = r#"{"version":"1.2.0","url":"https://github.com/hulbo/monun/releases/download/v1.2.0/Monun-1.2.0-Setup.exe","sha256":"ab","notes":"- a","date":"2026-11-26","extra":1}"#;
        let r: Release = serde_json::from_str(text).unwrap_or_default();
        assert_eq!((r.version.as_str(), r.date.as_str()), ("1.2.0", "2026-11-26"));
        assert!(r.url.starts_with(SETUPS));
    }

    #[cfg(feature = "api")]
    #[test]
    fn a_setup_from_elsewhere_or_without_a_checksum_isnt_downloaded() {
        let dir = std::env::temp_dir().join(format!("monun-update-test-{}", std::process::id()));
        let elsewhere = Release { url: "https://example.com/Monun-9.9.9-Setup.exe".into(), sha256: "0".repeat(64), ..Release::default() };
        assert!(net::download(&elsewhere, &dir, &|_, _| {}).is_err());
        let odd = Release { url: format!("{SETUPS}v9.9.9/..\\evil.exe"), sha256: "0".repeat(64), ..Release::default() };
        assert!(net::download(&odd, &dir, &|_, _| {}).is_err());
        let unchecked = Release { url: format!("{SETUPS}v9.9.9/Monun-9.9.9-Setup.exe"), sha256: "nope".into(), ..Release::default() };
        assert!(net::download(&unchecked, &dir, &|_, _| {}).is_err());
        assert!(!dir.exists(), "nothing was written");
    }

    /// Downloads a real release's setup and checks it (run by hand: `cargo test -p monun-ui-egui
    /// --lib update -- --ignored`). It is never run.
    #[cfg(feature = "api")]
    #[test]
    #[ignore = "downloads from GitHub"]
    fn a_real_releases_setup_downloads_and_checks() {
        let dir = std::env::temp_dir().join(format!("monun-update-real-{}", std::process::id()));
        let release = Release {
            version: "1.1.8".into(),
            url: format!("{SETUPS}v1.1.8/Monun-1.1.8-Setup.exe"),
            sha256: "3cf653c3a96555fd10a0a61b674873cd931a2501457ec1c5a987c596cd23eadd".into(),
            ..Release::default()
        };
        let seen = std::sync::atomic::AtomicU64::new(0);
        let path = net::download(&release, &dir, &|got, _| seen.store(got, std::sync::atomic::Ordering::Relaxed)).unwrap_or_else(|e| panic!("{e}"));
        assert!(path.is_file() && seen.load(std::sync::atomic::Ordering::Relaxed) > 1_000_000);
        let wrong = Release { sha256: "0".repeat(64), ..release };
        assert!(net::download(&wrong, &dir, &|_, _| {}).is_err(), "a checksum that doesn't match");
        assert!(!dir.join("Monun-1.1.8-Setup.exe").exists(), "and the file goes");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
