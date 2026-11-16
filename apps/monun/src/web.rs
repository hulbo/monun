//! Web tabs' pages (Windows): each a WebView2 view (the Edge engine Windows has, through Tauri's
//! `wry`) placed in the window where the tab's page goes ([`monun_ui_egui::web_view`]). A view
//! is made the first time its tab is shown, follows the window as it is moved and resized, hides
//! while its tab isn't in front, and closes with its tab. A link that would open a window of its
//! own opens in the same tab.
//!
//! A view is a window of its own over the app's: what the app floats over the page (the memo or
//! group panel as a layer, a menu, a dialog, a tooltip) is cut out of it with a window region, and
//! shows through, with the page around it. A click on the app's own parts takes the keyboard back
//! from the page.
//!
//! Sign-ins and cookies are kept in `%LOCALAPPDATA%\Monun\WebView2`; a test run keeps its own in
//! a folder of its own, removed at its end.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use monun_ui_egui::web_view::{Order, Page, State};
use winsafe::prelude::*;
use winsafe::{self as w, co};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::raw_window_handle::HasWindowHandle;
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

/// The window class of the window `wry` hosts a view in.
const CONTAINER: &str = "WRY_WEBVIEW";

/// What a page's handlers saw since the last frame: its title, whether it is loading, and the
/// addresses it asked to open in a window of their own.
#[derive(Default)]
struct Seen {
    title: String,
    loading: bool,
    opened: Vec<String>,
}

/// A page shown: its view, the address it was given, where it is (or hidden), and what it did.
struct View {
    webview: WebView,
    url: String,
    rect: Option<[u32; 4]>,
    seen: Rc<RefCell<Seen>>,
    /// The window the view is in, and the holes cut out of it.
    container: Option<w::HWND>,
    holes: Vec<[i32; 4]>,
}

/// The web views of the window.
pub struct Web {
    context: WebContext,
    views: HashMap<u64, View>,
    /// Pages that couldn't be shown: the address tried and why, so it isn't tried every frame.
    failed: HashMap<u64, (String, String)>,
    /// A test run's data folder, removed at its end.
    temporary: Option<PathBuf>,
}

impl Web {
    /// The web views, keeping their data in `data` (removed at the end when `temporary`).
    pub fn new(data: PathBuf, temporary: bool) -> Web {
        Web { context: WebContext::new(Some(data.clone())), views: HashMap::new(), failed: HashMap::new(), temporary: temporary.then_some(data) }
    }

    /// After the app has drawn a frame: the pages it lists are made, placed, shown or hidden, those
    /// of closed tabs close, the user's Back, Forward and Show Again are done, and what each page
    /// did is reported back.
    pub fn sync(&mut self, window: &impl HasWindowHandle, state: &mut State, ctx: &egui::Context) {
        state.host = Some(Ok(()));
        // A click on the app's own parts (the page's clicks never reach it) takes the keyboard back
        // from the page, so the app's keys and fields work again.
        if !self.views.is_empty() && ctx.input(|i| i.pointer.any_pressed()) {
            for view in self.views.values() {
                let _ = view.webview.focus_parent();
            }
        }
        let ids: Vec<u64> = state.panes.iter().map(|p| p.id).collect();
        self.views.retain(|id, _| ids.contains(id));
        self.failed.retain(|id, _| ids.contains(id));
        for pane in &state.panes {
            if let Some(view) = self.views.get_mut(&pane.id) {
                if view.url != pane.url {
                    let _ = view.webview.load_url(&pane.url);
                    view.url.clone_from(&pane.url);
                }
                let moved = view.rect != pane.rect;
                if moved {
                    if let Some(r) = pane.rect {
                        let _ = view.webview.set_bounds(bounds(r));
                    }
                    if view.rect.is_some() != pane.rect.is_some() {
                        let _ = view.webview.set_visible(pane.rect.is_some());
                    }
                    view.rect = pane.rect;
                }
                // Cut again when the holes change, or the page's size (the region is its size).
                if let Some([_, _, wide, high]) = pane.rect
                    && (moved || view.holes != pane.holes)
                {
                    view.holes.clone_from(&pane.holes);
                    if let Some(container) = &view.container
                        && let Err(e) = cut(container, wide, high, &pane.holes)
                    {
                        log::debug!("web page holes: {e}");
                    }
                }
                continue;
            }
            // Made once it is first shown; one that failed isn't tried again for that address.
            if pane.rect.is_none() || self.failed.get(&pane.id).is_some_and(|(url, _)| *url == pane.url) {
                continue;
            }
            let seen = Rc::new(RefCell::new(Seen::default()));
            match build(&mut self.context, window, &pane.url, pane.rect, &seen, ctx) {
                Ok(webview) => {
                    self.failed.remove(&pane.id);
                    // Its window: the one of the class no other view is in.
                    let container = containers().into_iter().find(|c| !self.views.values().any(|v| v.container.as_ref() == Some(c)));
                    if let (Some(c), Some([_, _, wide, high])) = (&container, pane.rect)
                        && !pane.holes.is_empty()
                        && let Err(e) = cut(c, wide, high, &pane.holes)
                    {
                        log::debug!("web page holes: {e}");
                    }
                    let holes = pane.holes.clone();
                    self.views.insert(pane.id, View { webview, url: pane.url.clone(), rect: pane.rect, seen, container, holes });
                }
                Err(why) => {
                    log::warn!("web page {}: {why}", pane.url);
                    self.failed.insert(pane.id, (pane.url.clone(), why));
                }
            }
        }
        for (id, order) in state.orders.drain(..) {
            let Some(view) = self.views.get(&id) else { continue };
            let _ = match order {
                Order::Back => view.webview.evaluate_script("history.back()"),
                Order::Forward => view.webview.evaluate_script("history.forward()"),
                Order::Reload => view.webview.reload(),
            };
        }
        state.pages.clear();
        for (id, view) in &self.views {
            let (title, loading, opened) = match view.seen.try_borrow_mut() {
                Ok(mut seen) => (seen.title.clone(), seen.loading, std::mem::take(&mut seen.opened)),
                Err(_) => (String::new(), false, Vec::new()),
            };
            // A link to a window of its own opens here, in the tab.
            if let Some(url) = opened.last() {
                let _ = view.webview.load_url(url);
            }
            // Before its first page has loaded a view is at about:blank: its address is the one given.
            let url = view.webview.url().ok().filter(|u| !u.is_empty() && u != "about:blank").unwrap_or_else(|| view.url.clone());
            state.pages.insert(*id, Page { title, url, loading, error: None });
        }
        for (id, (url, why)) in &self.failed {
            state.pages.insert(*id, Page { url: url.clone(), error: Some(why.clone()), ..Page::default() });
        }
    }

    /// Closes every page (the app is ending), and a test run's data folder goes.
    pub fn close(&mut self) {
        self.views.clear();
        if let Some(dir) = &self.temporary {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// The windows views are in (`wry`'s, of class [`CONTAINER`]) in this process's windows.
fn containers() -> Vec<w::HWND> {
    let me = w::GetCurrentProcessId();
    let mut found = Vec::new();
    let _ = w::EnumWindows(|top: w::HWND| {
        if top.GetWindowThreadProcessId().1 == me {
            top.EnumChildWindows(|child: w::HWND| {
                if child.GetClassName().is_ok_and(|class| class == CONTAINER) {
                    found.push(child);
                }
                true
            });
        }
        true
    });
    found
}

/// Cuts `holes` (physical pixels from its top left corner) out of a view's window `wide` × `high`,
/// or with none, shows all of it.
fn cut(container: &w::HWND, wide: u32, high: u32, holes: &[[i32; 4]]) -> w::SysResult<()> {
    if holes.is_empty() {
        return container.SetWindowRgn(&w::HRGN::NULL, true);
    }
    let size = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    let mut shown = w::HRGN::CreateRectRgn(w::RECT { left: 0, top: 0, right: size(wide), bottom: size(high) })?;
    for &[x, y, hole_w, hole_h] in holes {
        let hole = w::HRGN::CreateRectRgn(w::RECT { left: x, top: y, right: x.saturating_add(hole_w), bottom: y.saturating_add(hole_h) })?;
        shown.CombineRgn(&shown, &hole, co::RGN::DIFF)?;
    }
    container.SetWindowRgn(&shown, true)?;
    // The window owns the region now: it must not be deleted here.
    let _ = shown.leak();
    Ok(())
}

/// Where a page goes, in the window's physical pixels.
fn bounds([x, y, w, h]: [u32; 4]) -> Rect {
    Rect {
        position: PhysicalPosition::new(i32::try_from(x).unwrap_or(0), i32::try_from(y).unwrap_or(0)).into(),
        size: PhysicalSize::new(w.max(1), h.max(1)).into(),
    }
}

/// A view of `url` in the window at `rect`, telling `seen` its title, its loading and the windows
/// it asks for, and the app to draw again when they change.
fn build(
    context: &mut WebContext,
    window: &impl HasWindowHandle,
    url: &str,
    rect: Option<[u32; 4]>,
    seen: &Rc<RefCell<Seen>>,
    ctx: &egui::Context,
) -> Result<WebView, String> {
    let (title, load, open) = (Rc::clone(seen), Rc::clone(seen), Rc::clone(seen));
    let (c1, c2, c3) = (ctx.clone(), ctx.clone(), ctx.clone());
    WebViewBuilder::new_with_web_context(context)
        .with_url(url)
        .with_bounds(bounds(rect.unwrap_or([0, 0, 1, 1])))
        .with_visible(rect.is_some())
        .with_focused(false)
        .with_document_title_changed_handler(move |t| {
            if let Ok(mut s) = title.try_borrow_mut() {
                s.title = t;
            }
            c1.request_repaint();
        })
        .with_on_page_load_handler(move |event, _| {
            if let Ok(mut s) = load.try_borrow_mut() {
                s.loading = matches!(event, PageLoadEvent::Started);
            }
            c2.request_repaint();
        })
        .with_new_window_req_handler(move |url, _| {
            if let Ok(mut s) = open.try_borrow_mut() {
                s.opened.push(url);
            }
            c3.request_repaint();
            NewWindowResponse::Deny
        })
        .build_as_child(window)
        .map_err(|e| e.to_string())
}
