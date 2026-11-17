//! Files wrapped by document security (DRM), such as those of the Fasoo agent public offices
//! install: the agent decrypts them only for the programs it is made for. Monun isn't one of them,
//! and never gets round it. Opening one says how to release it (the agent's own Explorer menu,
//! Secure Document › Remove Encryption), with a button that shows it in its folder, and watches
//! it: once it is released into a plain file, it opens.

use std::time::SystemTime;

use crate::SheetApp;

/// How often a file waiting to be released is looked at, and for how long (milliseconds).
const LOOK_MS: f64 = 1000.0;
const WATCH_MS: f64 = 60.0 * 60.0 * 1000.0;

/// The protected files watched until they are released.
#[derive(Default)]
pub struct Drm {
    waiting: Vec<Wait>,
}

impl Drm {
    /// Whether `path` is watched until it is released.
    pub fn is_waiting(&self, path: &str) -> bool {
        self.waiting.iter().any(|w| w.path == path)
    }
}

/// A protected file watched until it is released.
struct Wait {
    path: String,
    /// Its size and modification time when last looked at (`None` while it can't be seen).
    seen: Option<(u64, Option<SystemTime>)>,
    /// It changed, and is read once it has settled (the same at the next look).
    changed: bool,
    /// When it was last looked at, and until when it is watched ([`crate::monotonic_ms`]).
    looked: f64,
    until: f64,
}

/// `path` was refused for its document security: it is watched until it is released. With `ask`,
/// the message says how to release it, with a button that shows it in its folder.
pub(crate) fn refused(app: &mut SheetApp, path: &str, ask: bool) {
    let now = crate::monotonic_ms();
    app.drm.waiting.retain(|w| w.path != path);
    app.drm.waiting.push(Wait { path: path.to_string(), seen: look(path), changed: false, looked: now, until: now + WATCH_MS });
    if ask {
        let lang = app.ui.language;
        let file = std::path::Path::new(path).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path);
        let text = lang
            .tr("'{file}' is encrypted by document security (DRM), which Monun can't read. In File Explorer, right-click it and choose Secure Document › Remove Encryption: Monun opens it as soon as it is released.")
            .replace("{file}", file);
        app.message = Some(("Secure Document".into(), text.clone()));
        app.message_file = Some((text, path.to_string()));
    }
}

/// A file's size and modification time, if it can be seen.
fn look(path: &str) -> Option<(u64, Option<SystemTime>)> {
    std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

/// Whether the file at `path` is still protected; `None` while it can't be read (being written).
fn protected(path: &str) -> Option<bool> {
    use std::io::Read as _;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(4096).read_to_end(&mut head).ok()?;
    Some(monun_engine::io::is_drm_protected(&head))
}

/// Looks at the files waiting to be released about every second, and opens those released, as
/// they are left (`logic`, every frame). After an hour a file is no longer watched.
pub(crate) fn watch(app: &mut SheetApp, ctx: &egui::Context) {
    if app.drm.waiting.is_empty() {
        return;
    }
    let now = crate::monotonic_ms();
    let mut released = Vec::new();
    app.drm.waiting.retain_mut(|w| {
        if now > w.until {
            return false;
        }
        if now - w.looked < LOOK_MS {
            return true;
        }
        w.looked = now;
        // Read once it has changed and then stayed the same a second: released, not still being
        // written.
        let seen = look(&w.path);
        if seen != w.seen {
            w.seen = seen;
            w.changed = true;
            return true;
        }
        if !w.changed || seen.is_none() {
            return true;
        }
        match protected(&w.path) {
            Some(true) => {
                w.changed = false;
                true
            }
            Some(false) => {
                released.push(w.path.clone());
                false
            }
            None => true,
        }
    });
    for path in released {
        // Its message goes, if it is still up: the file it was about opens.
        if let Some((text, _)) = app.message_file.take_if(|(_, p)| *p == path)
            && app.message.as_ref().is_some_and(|(_, m)| *m == text)
        {
            app.message = None;
        }
        app.open_path(&path);
    }
    if !app.drm.waiting.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(LOOK_MS as u64));
    }
}
