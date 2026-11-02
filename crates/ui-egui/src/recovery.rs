//! AutoRecover: every ten minutes a copy of each workbook with unsaved changes goes to this run's
//! folder under `<settings>/recovery`, as Excel's AutoRecover does. A clean exit removes the
//! folder; a run that ended any other way (a crash, a power cut, the process killed) leaves it,
//! and the next start reopens its workbooks as unsaved copies.
//!
//! Each run keeps its folder's `lock` file locked for as long as it lives, so a second Monun
//! running at the same time never takes the first one's copies.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::{Notice, NoticeKind, SheetApp};

/// How often copies are written: Excel's default AutoRecover interval.
pub const INTERVAL_MS: f64 = 10.0 * 60_000.0;

/// This run's recovery folder.
pub struct Recovery {
    dir: PathBuf,
    /// Open and locked until the run ends.
    _lock: File,
    /// The revision each workbook's copy holds, by workbook uid.
    written: HashMap<u64, u64>,
    /// When copies were last written ([`crate::monotonic_ms`]).
    last: f64,
}

/// A workbook a run that ended abnormally left behind.
pub struct LeftOver {
    pub title: String,
    /// `xlsx`, or `xlsm` for a workbook with a VBA project.
    pub ext: &'static str,
    pub bytes: Vec<u8>,
}

impl Recovery {
    /// Starts this run's folder in `root`; `None` when it can't be made or locked.
    pub fn start(root: &Path) -> Option<Recovery> {
        let stamp = monun_engine::core::date::unix_now_ms().unwrap_or(0.0) as u64;
        let dir = root.join(format!("{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).ok()?;
        let lock = File::create(dir.join("lock")).ok()?;
        lock.try_lock().ok()?;
        Some(Recovery { dir, _lock: lock, written: HashMap::new(), last: crate::monotonic_ms() })
    }

    /// Removes the copies of workbooks that were saved or closed since they were written: a
    /// saved workbook is never offered back as a recovered one.
    pub fn tidy(&mut self, session: &monun_engine::Session) {
        let stale: Vec<u64> = self.written.keys().copied().filter(|uid| !session.documents().iter().any(|d| d.uid == *uid && d.is_dirty())).collect();
        for uid in stale {
            self.written.remove(&uid);
            self.remove(uid);
        }
    }

    /// Writes a copy of every workbook whose unsaved changes its copy doesn't hold yet.
    pub fn write(&mut self, session: &monun_engine::Session) {
        self.tidy(session);
        for d in session.documents().iter().filter(|d| d.is_dirty()) {
            if self.written.get(&d.uid) == Some(&d.revision) {
                continue;
            }
            let ext = if d.wb.vba_project.is_some() { "xlsm" } else { "xlsx" };
            let Ok(bytes) = monun_engine::io::save_bytes(&d.wb, &format!("recovery.{ext}")) else { continue };
            let base = self.dir.join(d.uid.to_string());
            // Written aside and then renamed, so a copy is never left half written.
            let tmp = base.with_extension("tmp");
            if fs::write(&tmp, &bytes).is_err() || fs::rename(&tmp, base.with_extension(ext)).is_err() {
                continue;
            }
            let _ = fs::write(base.with_extension("json"), json!({"title": d.display_title()}).to_string());
            self.written.insert(d.uid, d.revision);
        }
    }

    fn remove(&self, uid: u64) {
        for ext in ["xlsx", "xlsm", "json", "tmp"] {
            let _ = fs::remove_file(self.dir.join(format!("{uid}.{ext}")));
        }
    }

    /// Ends the run cleanly: its copies are no longer needed.
    pub fn finish(self) {
        let Recovery { dir, _lock, .. } = self;
        // Windows deletes no file that is open.
        drop(_lock);
        let _ = fs::remove_dir_all(dir);
    }

    /// The workbooks that runs which ended without finishing left in `root`, and their folders,
    /// for the caller to remove once it has reopened them. A folder whose lock is held belongs to
    /// a run that is still going.
    pub fn left_over(root: &Path) -> (Vec<LeftOver>, Vec<PathBuf>) {
        let (mut books, mut dirs) = (vec![], vec![]);
        let Ok(entries) = fs::read_dir(root) else { return (books, dirs) };
        for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if File::open(dir.join("lock")).is_ok_and(|lock| lock.try_lock().is_err()) {
                continue;
            }
            for copy in fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
                let ext = match copy.extension().and_then(|e| e.to_str()) {
                    Some("xlsx") => "xlsx",
                    Some("xlsm") => "xlsm",
                    _ => continue,
                };
                let Ok(bytes) = fs::read(&copy) else { continue };
                let meta: Option<Json> = fs::read(copy.with_extension("json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
                let title = meta.as_ref().and_then(|m| m["title"].as_str()).unwrap_or("Book").to_string();
                books.push(LeftOver { title, ext, bytes });
            }
            dirs.push(dir);
        }
        (books, dirs)
    }
}

impl SheetApp {
    /// Starts AutoRecover with its folders in `root` and reopens, as unsaved copies, the
    /// workbooks that a run which ended abnormally left there.
    pub fn start_recovery(&mut self, root: &Path) {
        let (books, dirs) = Recovery::left_over(root);
        self.recovery = Recovery::start(root);
        let lang = self.ui.language;
        let mut reopened = 0;
        for book in &books {
            let stem = Path::new(&book.title).file_stem().and_then(|s| s.to_str()).unwrap_or("Book").to_string();
            let name = format!("{stem}.{}", book.ext);
            let base64 = monun_engine::io::base64_encode(&book.bytes);
            if self.session.run("file.open", json!({"name": name, "base64": base64})).is_err() {
                continue;
            }
            if let Some(d) = self.session.active_mut() {
                d.title = lang.tr("{title} (Recovered)").replace("{title}", &stem);
                // Unsaved, so closing it asks first and AutoRecover keeps a copy.
                d.saved = std::sync::Arc::new((*d.wb).clone());
            }
            reopened += 1;
        }
        self.after_engine();
        // The old folders go once this run holds copies of what they had.
        if let Some(r) = self.recovery.as_mut() {
            r.write(&self.session);
            for dir in dirs {
                let _ = fs::remove_dir_all(dir);
            }
        }
        if reopened > 0 {
            let text = lang
                .tr("Monun closed unexpectedly last time and recovered {count} unsaved workbook(s): save the ones you want to keep.")
                .replace("{count}", &reopened.to_string());
            self.notice = Some(Notice { text, kind: NoticeKind::Warning, action: None });
        }
    }

    /// Writes AutoRecover copies when they are due, and drops those of workbooks saved since.
    pub(crate) fn auto_recover(&mut self, ctx: &egui::Context) {
        let Some(r) = self.recovery.as_mut() else { return };
        r.tidy(&self.session);
        let now = crate::monotonic_ms();
        let wait = INTERVAL_MS - (now - r.last);
        if wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(wait as u64));
            return;
        }
        r.last = now;
        r.write(&self.session);
    }

    /// Ends AutoRecover on a clean exit, removing this run's copies.
    pub fn finish_recovery(&mut self) {
        if let Some(r) = self.recovery.take() {
            r.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("monun-recovery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn edited_session() -> monun_engine::Session {
        let mut s = monun_engine::Session::new();
        s.new_workbook();
        s.execute("cell.set", json!({"cell": "A1", "input": "견적서"})).unwrap();
        s
    }

    #[test]
    fn a_clean_exit_leaves_nothing_and_a_live_run_is_left_alone() {
        let root = root("clean");
        let mut r = Recovery::start(&root).unwrap();
        r.write(&edited_session());
        // Another start sees a folder whose lock is held: a run still going.
        assert!(Recovery::left_over(&root).0.is_empty());
        let dir = r.dir.clone();
        assert_eq!(fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "xlsx")).count(), 1);
        r.finish();
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copies_follow_saves_and_closes() {
        let root = root("tidy");
        let mut r = Recovery::start(&root).unwrap();
        let mut s = edited_session();
        r.write(&s);
        let copies = |r: &Recovery| fs::read_dir(&r.dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "xlsx")).count();
        assert_eq!(copies(&r), 1);
        s.execute("file.close", json!({"force": true})).unwrap();
        r.tidy(&s);
        assert_eq!(copies(&r), 0, "a closed workbook's copy goes");
        r.finish();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_that_ended_abnormally_is_reopened_unsaved() {
        let root = root("crash");
        let mut r = Recovery::start(&root).unwrap();
        r.write(&edited_session());
        // The process "dies": its lock goes with it, its folder stays.
        drop(r);
        let mut app = SheetApp::new(monun_engine::Session::new(), crate::Services::default());
        app.start_recovery(&root);
        let d = app.session.active().unwrap();
        assert!(d.is_dirty(), "a recovered workbook is unsaved");
        assert!(d.title.ends_with("(Recovered)"), "{}", d.title);
        assert_eq!(d.wb.active().unwrap().value(monun_engine::core::CellRef::new(0, 0)), monun_engine::core::Value::text("견적서"));
        assert!(app.notice.as_ref().is_some_and(|n| n.text.contains("recovered 1 unsaved")), "{:?}", app.notice.as_ref().map(|n| &n.text));
        // Its copy now lives in this run's folder; the old one is gone.
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(Recovery::left_over(&root).0.is_empty(), "this run holds it");
        app.finish_recovery();
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
