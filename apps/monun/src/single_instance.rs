//! One 모눈 at a time. A second start (a file double-clicked in Explorer, the desktop shortcut
//! again) hands its files to the 모눈 already running, which opens them as tabs and comes to the
//! front, and ends there: two runs would each reopen the file group and write `ui.json`.
//!
//! The running 모눈 listens on a localhost port. `instance.json` in the settings folder names the
//! port with a token, and a request without that token is ignored. A file left behind by a run
//! that ended badly names a port nothing answers on, so the new run takes over. Test runs (no
//! preferences, `--control`) take no part.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value as Json, json};

/// The file naming the running 모눈's port and token, in the settings folder.
const FILE: &str = "instance.json";

/// The longest request read (a few hundred paths).
const REQUEST_LIMIT: u64 = 256 * 1024;

/// What another start handed over: files to open, and whether to update (the desktop's "모눈
/// 업데이트" shortcut, `--update`).
pub struct Handed {
    pub files: Vec<String>,
    pub update: bool,
}

/// The running 모눈's side: what other starts handed over, as it arrives.
pub struct Listener {
    pub rx: mpsc::Receiver<Handed>,
    file: PathBuf,
    token: String,
    /// The UI to wake when files arrive, once it exists.
    wake: Arc<Mutex<Option<egui::Context>>>,
}

/// Hands `files` (and `update`, the shortcut's ask to update) to the 모눈 already running, if one
/// is. `true` when it took them: this run ends.
pub fn hand_over(dir: &Path, files: &[String], update: bool) -> bool {
    let Some((port, token)) = read(dir) else { return false };
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    // A port another program took since answers late or never: wait a little only.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    // The running 모눈 has a folder of its own to start from: paths go as full ones.
    let open: Vec<String> =
        files.iter().map(|f| std::path::absolute(f).ok().and_then(|p| p.to_str().map(str::to_string)).unwrap_or_else(|| f.clone())).collect();
    let request = json!({"token": token, "open": open, "update": update}).to_string();
    if writeln!(stream, "{request}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// The port and token `instance.json` names.
fn read(dir: &Path) -> Option<(u16, String)> {
    let v: Json = serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()?;
    let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
    Some((port, v.get("token")?.as_str()?.to_string()))
}

/// A token hard to guess: two hashes keyed afresh by the standard library's random state (seeded
/// from the system's randomness).
fn token() -> String {
    use std::hash::BuildHasher;
    let a = std::collections::hash_map::RandomState::new().hash_one(std::process::id());
    let b = std::collections::hash_map::RandomState::new().hash_one(a);
    format!("{a:016x}{b:016x}")
}

/// Starts listening for other starts' files and writes `instance.json`. `None` when it can't (the
/// app runs on, on its own).
pub fn listen(dir: &Path) -> Option<Listener> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = token();
    std::fs::create_dir_all(dir).ok()?;
    let file = dir.join(FILE);
    std::fs::write(&file, json!({"port": port, "token": token, "pid": std::process::id()}).to_string()).ok()?;
    let (tx, rx) = mpsc::channel();
    let wake: Arc<Mutex<Option<egui::Context>>> = Arc::default();
    let (accept_token, accept_wake) = (token.clone(), wake.clone());
    let started = std::thread::Builder::new().name("monun-instance".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &accept_token, &tx, &accept_wake);
        }
    });
    if let Err(e) = started {
        log::warn!("no single instance: {e}");
        let _ = std::fs::remove_file(&file);
        return None;
    }
    Some(Listener { rx, file, token, wake })
}

/// One request: its files are queued for the UI, which is woken, and the start that sent them is
/// told to end.
fn serve(stream: TcpStream, token: &str, tx: &mpsc::Sender<Handed>, wake: &Mutex<Option<egui::Context>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new((&stream).take(REQUEST_LIMIT)).read_line(&mut line).is_err() {
        return;
    }
    let Ok(request) = serde_json::from_str::<Json>(&line) else { return };
    if request.get("token").and_then(Json::as_str) != Some(token) {
        return;
    }
    let files: Vec<String> =
        request.get("open").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect()).unwrap_or_default();
    let update = request.get("update").and_then(Json::as_bool).unwrap_or(false);
    if tx.send(Handed { files, update }).is_err() {
        return;
    }
    if let Ok(ctx) = wake.lock()
        && let Some(ctx) = ctx.as_ref()
    {
        ctx.request_repaint();
    }
    let _ = (&stream).write_all(b"ok\n");
}

impl Listener {
    /// The UI to wake when files arrive.
    pub fn wake_with(&self, ctx: &egui::Context) {
        if let Ok(mut wake) = self.wake.lock() {
            *wake = Some(ctx.clone());
        }
    }

    /// Removes `instance.json` at exit, unless a later run has written its own.
    pub fn close(&self) {
        if read(self.file.parent().unwrap_or(Path::new("."))).is_some_and(|(_, token)| token == self.token) {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_hands_its_files_to_the_first() {
        let dir = std::env::temp_dir().join(format!("monun-instance-{}", std::process::id()));
        let first = listen(&dir).expect("the first start listens");
        assert!(hand_over(&dir, &["a.xlsx".into(), "b.md".into()], false), "the second start hands over");
        let got = first.rx.recv_timeout(Duration::from_secs(5)).expect("the files arrive");
        assert_eq!((got.files.len(), got.update), (2, false));
        assert!(got.files.iter().all(|p| Path::new(p).is_absolute()), "as full paths: {:?}", got.files);
        // The update shortcut's start asks the running one to update.
        assert!(hand_over(&dir, &[], true));
        assert!(first.rx.recv_timeout(Duration::from_secs(5)).expect("asked").update);
        first.close();
        assert!(!dir.join(FILE).exists(), "the file goes at exit");
        assert!(!hand_over(&dir, &[], false), "with no 모눈 running, nothing takes the files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_request_without_the_token_is_ignored() {
        let dir = std::env::temp_dir().join(format!("monun-instance-token-{}", std::process::id()));
        let first = listen(&dir).expect("listens");
        let (port, _) = read(&dir).expect("instance.json");
        std::fs::write(dir.join(FILE), json!({"port": port, "token": "wrong"}).to_string()).expect("rewrites");
        assert!(!hand_over(&dir, &["a.xlsx".into()], false), "refused");
        assert!(first.rx.try_recv().is_err(), "nothing queued");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
