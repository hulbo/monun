//! Presentation tabs: a PowerPoint file's slides turned into a web page ([`monun_pptx`]) and shown
//! as a web tab's page ([`crate::web_view`]), to look at. The page is written on a thread of its
//! own into a folder of this run in the temporary folder, written again when the file changes,
//! and removed when its tab closes and when Monun ends.

use std::sync::mpsc;

/// What writing a presentation's page gives: the page's address and how many slides it has.
pub type Done = Result<(String, usize), String>;

/// When the thread writing a page ends without a word (it failed outright).
pub const STOPPED: &str = "Monun stopped reading this presentation. Close it and open it again.";

/// Where presentations can't be shown (the web build).
pub const NOT_HERE: &str = "This version of Monun can't show presentations.";

/// A page being written.
pub struct Job {
    rx: mpsc::Receiver<Done>,
}

impl Job {
    /// The page, once it's written.
    pub fn poll(&self) -> Option<Done> {
        match self.rx.try_recv() {
            Ok(done) => Some(done),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(STOPPED.into())),
        }
    }
}

/// Starts writing the page of the presentation at `path`, shown in tab `uid`; `words` are the
/// page's own (a chart's label, a hidden slide's) in the interface's language.
pub fn start(path: &str, uid: u64, words: (String, String)) -> Job {
    let (tx, rx) = mpsc::channel();
    #[cfg(all(feature = "slides", not(target_arch = "wasm32")))]
    {
        let path = path.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(write(&path, uid, words));
        });
    }
    #[cfg(not(all(feature = "slides", not(target_arch = "wasm32"))))]
    {
        let _ = (path, uid, words);
        let _ = tx.send(Err(NOT_HERE.into()));
    }
    Job { rx }
}

#[cfg(all(feature = "slides", not(target_arch = "wasm32")))]
fn write(path: &str, uid: u64, (chart, hidden): (String, String)) -> Done {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let title = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("Presentation");
    let deck = monun_pptx::convert(&bytes, title, &monun_pptx::Labels { chart, hidden })?;
    let page = monun_pptx::write(&deck, &folder(uid))?;
    Ok((crate::web_view::file_url(&page.to_string_lossy()), deck.slides))
}

/// The folder of this run's presentation pages.
#[cfg(not(target_arch = "wasm32"))]
fn base() -> std::path::PathBuf {
    std::env::temp_dir().join("monun-slides")
}

/// Where this run writes the page of tab `uid`.
#[cfg(not(target_arch = "wasm32"))]
fn folder(uid: u64) -> std::path::PathBuf {
    base().join(format!("{}-{uid}", std::process::id()))
}

/// A closed tab's page goes.
pub fn forget(uid: u64) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::fs::remove_dir_all(folder(uid));
    #[cfg(target_arch = "wasm32")]
    let _ = uid;
}

/// At the end of a run: its pages go, and those runs that ended without tidying left more than a
/// day ago.
pub fn clean_up() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mine = format!("{}-", std::process::id());
        let Ok(entries) = std::fs::read_dir(base()) else { return };
        let day = std::time::Duration::from_secs(24 * 3600);
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
            if name.starts_with(&mine) || stale {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}
