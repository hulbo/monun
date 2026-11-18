//! Back and forward between the files in front, as a browser goes between pages: the mouse's Back
//! and Forward buttons bring the file that was in front before (or after) to the front again, so a
//! document read a moment ago is a click away without the group panel. A web page goes back
//! through its own pages first and passes the button on once it has none left that way
//! ([`crate::web_view::State::file_steps`]).
//!
//! The history is of the open windows: one that has closed (its file closed, the group changed) is
//! passed over.

use egui::PointerButton;
use serde_json::json;

use crate::SheetApp;

/// The most files kept each way.
const KEPT: usize = 50;

/// The windows that were in front, nearest last.
#[derive(Default)]
pub struct History {
    back: Vec<u64>,
    forward: Vec<u64>,
    /// The window in front when last looked.
    front: Option<u64>,
}

/// Notes the window in front (each frame): a new one puts the one before it on the way back, and
/// clears the way forward, as a link followed in a browser does.
pub(crate) fn track(app: &mut SheetApp) {
    let now = app.session.active_window();
    let h = &mut app.history;
    if now == h.front {
        return;
    }
    if let Some(before) = h.front {
        keep(&mut h.back, before);
        h.forward.clear();
    }
    h.front = now;
}

/// The mouse's Back and Forward buttons anywhere in the window, and those a web page passed on.
pub(crate) fn input(app: &mut SheetApp, ctx: &egui::Context) {
    let (back, forward) = ctx.input(|i| (i.pointer.button_pressed(PointerButton::Extra1), i.pointer.button_pressed(PointerButton::Extra2)));
    let mut steps: Vec<bool> = std::mem::take(&mut app.web.file_steps);
    if back {
        steps.push(true);
    }
    if forward {
        steps.push(false);
    }
    for back in steps {
        step(app, back);
    }
}

/// Brings the file before (`back`) or after the one in front to the front; whether there was one.
/// Not while a dialog or the File view is open.
pub fn step(app: &mut SheetApp, back: bool) -> bool {
    if app.dialog.is_some() || app.backstage.is_some() {
        return false;
    }
    track(app);
    let open: Vec<u64> = app.session.windows().iter().map(|w| w.id).collect();
    let now = app.session.active_window();
    loop {
        let h = &mut app.history;
        let from = if back { &mut h.back } else { &mut h.forward };
        let Some(w) = from.pop() else { return false };
        if !open.contains(&w) || Some(w) == now {
            continue;
        }
        match app.run("window.activate", json!({"window": w})) {
            Ok(_) => {
                let h = &mut app.history;
                if let Some(now) = now {
                    keep(if back { &mut h.forward } else { &mut h.back }, now);
                }
                h.front = app.session.active_window();
                app.follow_active_window();
                return true;
            }
            Err(e) => {
                app.alert(e);
                return false;
            }
        }
    }
}

/// Puts `w` at the near end of a way, once (a window in front twice in a row is one step).
fn keep(way: &mut Vec<u64>, w: u64) {
    if way.last() != Some(&w) {
        way.push(w);
    }
    if way.len() > KEPT {
        way.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_way_keeps_a_window_once_in_a_row_and_its_newest_ones() {
        let mut way = Vec::new();
        keep(&mut way, 1);
        keep(&mut way, 1);
        keep(&mut way, 2);
        assert_eq!(way, [1, 2]);
        for w in 0..100 {
            keep(&mut way, w);
        }
        assert_eq!(way.len(), KEPT);
        assert_eq!(way.last(), Some(&99));
    }
}
