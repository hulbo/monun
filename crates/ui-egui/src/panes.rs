//! Task panes: Comments (the memo panel), Groups, Watch Window, Selection Pane, Format Chart. One
//! shows at a time, on the right; the memo and group panels go where they are put, a column on
//! the right or left or a layer over the files in a corner (their title dragged, or their options).
//! The memo panel is for workbooks and Markdown; while a SQL or plain text file is in front it
//! stays hidden.

use egui::{Color32, Ui, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::theme::{self, Tokens};

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    // Set again below while the group panel is tucked, before the files are drawn.
    app.group_handle = None;
    // The Comments pane is engine state (`review.showComments`, so agents can drive it); the other
    // panes are UI state. One pane shows at a time: opening another one closes Comments.
    // The group panel is the interface's, for every window (`UiState::group_panel`), with file tabs
    // only (the web build has no groups); another pane
    // shows in its place while it is open, and the group panel comes back when it closes: it is
    // where the files are listed.
    if app.grid.pane.as_deref() == Some("comments") {
        app.grid.pane = None;
        app.run_or_alert("review.showComments", json!({"on": true}));
    } else if app.grid.pane.is_some() && app.session.comments_pane {
        app.run_or_alert("review.showComments", json!({"on": false}));
    }
    let shown = app.grid.pane.clone().or_else(|| {
        if app.session.comments_pane { Some("comments".to_string()) } else { (app.file_tabs && app.ui.group_panel).then(|| "group".to_string()) }
    });
    let Some(pane) = shown else { return };
    // With no file open (the start page) only the group panel has anything to show.
    if (pane == "comments" && !memo_allowed(app)) || (app.session.active().is_none() && pane != "group") {
        return;
    }
    let t = Tokens::get(ui.ctx());
    // Tucked into its edge, the group panel is a handle there until the pointer comes to it.
    if pane == "group" && app.ui.group_tucked {
        tucked(app, ui, &t);
        return;
    }
    // The memo and group panels go where they are put: a column on either side, or a layer over
    // the files in a corner. The others are a column on the right.
    let movable = matches!(pane.as_str(), "comments" | "group");
    if movable && app.ui.side_layer {
        layer(app, ui, &pane, &t);
        return;
    }
    // Capped at half the window so a pane can never cover the sheet, whatever its content asks for.
    let max = (ui.available_width() / 2.0).max(300.0);
    let panel = if movable && app.ui.side_left { egui::Panel::left("task_pane") } else { egui::Panel::right("task_pane") };
    let panel = panel.default_size(300.0).max_size(max).resizable(true);
    let margin = egui::Margin { left: 10, right: 10, top: 0, bottom: 10 };
    let mut title = None;
    panel.frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(margin)).show(ui, |ui| {
        title = Some(header(app, ui, &pane, &t, movable));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().show(ui, |ui| content(app, ui, &pane));
    });
    if let Some((title, strip)) = title.filter(|_| movable) {
        let window = ui.ctx().content_rect();
        moved(app, ui.ctx(), &title, strip.width(), window, false);
    }
}

/// The tucked group panel's handle by the window's edge: its size, how far in from the edge it
/// stands, and how long the panel stays out once the pointer has left it (seconds).
const HANDLE: egui::Vec2 = egui::vec2(24.0, 84.0);
const HANDLE_GAP: f32 = 4.0;
const LINGER: f64 = 0.35;

/// How wide a scroll area's vertical bar is at most, along its edge.
const BAR_STRIP: f32 = 18.0;

/// `bar` (a vertical scroll bar's track, or a rect whose right-hand `strip` the bar runs down)
/// starting below the tucked group panel's handle (`handle`, [`SheetApp::group_handle`]) when the
/// handle stands over it, so the handle covers none of the bar. `None` when it is clear of it.
pub(crate) fn below_handle(handle: Option<egui::Rect>, bar: egui::Rect, strip: f32) -> Option<egui::Rect> {
    let h = handle?;
    let lane = egui::Rect::from_min_max(egui::pos2(bar.right() - strip, bar.top()), bar.max);
    if !h.intersects(lane) {
        return None;
    }
    let top = (h.bottom() + HANDLE_GAP).max(bar.top()).min(bar.bottom());
    Some(egui::Rect::from_min_max(egui::pos2(bar.left(), top), bar.max))
}

/// Where a scroll area filling `outer` shows its bars ([`egui::ScrollArea::scroll_bar_rect`]) so its
/// vertical bar starts below the tucked group panel's handle; `None` when the handle is clear of it.
pub(crate) fn scroll_bars_below(handle: Option<egui::Rect>, outer: egui::Rect) -> Option<egui::Rect> {
    below_handle(handle, outer, BAR_STRIP)
}

/// A web page's room (`area`) clear of the tucked group panel's handle: a web view draws its own
/// scroll bar, which can't start below the handle as the others do, so the page leaves the
/// handle's strip by the edge instead. The handle stays where it is in every kind of file.
pub(crate) fn beside_handle(handle: Option<egui::Rect>, area: egui::Rect) -> egui::Rect {
    let Some(h) = handle.filter(|h| h.intersects(area)) else { return area };
    let mut room = area;
    if h.center().x > area.center().x {
        room.max.x = (h.left() - HANDLE_GAP).max(area.left());
    } else {
        room.min.x = (h.right() + HANDLE_GAP).min(area.right());
    }
    room
}

/// Whether the group panel tucks into the window's left edge: the side its column is on, or its
/// layer's corner.
fn tuck_left(app: &SheetApp) -> bool {
    if app.ui.side_layer { matches!(app.ui.side_corner, Corner::TopLeft | Corner::BottomLeft) } else { app.ui.side_left }
}

/// The group panel tucked into the window's edge, as Visual Studio hides a panel: a handle peeks
/// out there over the files (they keep the whole width). Under the pointer the panel slides out
/// over them, stays while the pointer is on it (or on a menu it opened, or a drag goes on), and
/// goes back a moment after the pointer leaves, at a press elsewhere or at Escape. Its pin keeps
/// it shown again, as it was.
fn tucked(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    use egui::{Rect, pos2};
    let left = tuck_left(app);
    let files = ui.available_rect_before_wrap();
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    // Near the top of the edge, just below a document's or web page's tool bar (clear of its buttons).
    let top = files.top() + HEADER_H + 10.0;
    let name = app.ui.language.tr("Groups");
    let x = if left { files.left() + HANDLE_GAP } else { files.right() - HANDLE_GAP - HANDLE.x };
    let zone = Rect::from_min_size(pos2(x, top), HANDLE);
    // The files' scroll bars start below it (also while the panel is out, so they don't jump).
    app.group_handle = Some(zone);
    let Some(since) = app.group_peek else {
        if handle(&ctx, t, zone, name) {
            app.group_peek = Some(now);
        }
        return;
    };
    let panel = slid_out(app, ui, t, files, left);
    // Pinned (or tucked away again) in the panel.
    if !app.ui.group_tucked {
        app.group_peek = None;
        return;
    }
    let on = ctx.pointer_hover_pos().is_some_and(|p| {
        panel.contains(p) || zone.contains(p) || ctx.layer_id_at(p).is_some_and(|l| matches!(l.order, egui::Order::Foreground | egui::Order::Tooltip))
    });
    let busy = egui::Popup::is_any_open(&ctx) || ctx.dragged_id().is_some() || ctx.input(|i| i.pointer.any_down());
    let pressed_away = !on && ctx.input(|i| i.pointer.any_pressed());
    if pressed_away || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.group_peek = None;
    } else if on || busy {
        app.group_peek = Some(now);
    } else if now - since > LINGER {
        app.group_peek = None;
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(LINGER));
    }
}

/// The tucked group panel's handle (`shown`): a tab of the accent just inside the window's edge,
/// its corners rounded a little, raised off the page by a soft shadow and a thin light along its
/// top, with three dots down its middle to pull it out by. It fills all its room: on a web page
/// that room is cut out of the page (and the page covers its shadow). Whether the pointer is on it.
fn handle(ctx: &egui::Context, t: &Tokens, shown: egui::Rect, name: &str) -> bool {
    use egui::{Color32, Stroke, pos2, vec2};
    let mut pointed = false;
    let area =
        egui::Area::new(egui::Id::unique("monun.group_handle")).order(egui::Order::Middle).fixed_pos(shown.min).constrain(false).fade_in(false);
    area.show(ctx, |ui| {
        let (_, resp) = ui.allocate_exact_size(shown.size(), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
        pointed = resp.hovered();
        let painter = ui.painter();
        let corners = egui::CornerRadius::same(7);
        let shadow = egui::Shadow { offset: [0, 2], blur: 6, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 70 }) };
        painter.add(shadow.as_shape(shown, corners));
        // A shade lighter under the pointer.
        let fill = if pointed { t.accent.lerp_to_gamma(Color32::WHITE, 0.15) } else { t.accent };
        painter.rect_filled(shown, corners, fill);
        painter.line_segment(
            [pos2(shown.left() + 6.0, shown.top() + 1.0), pos2(shown.right() - 6.0, shown.top() + 1.0)],
            Stroke::new(1.0, Color32::from_white_alpha(70)),
        );
        for k in [-1.0, 0.0, 1.0] {
            painter.circle_filled(shown.center() + vec2(0.0, 8.0 * k), 2.0, t.on_accent);
        }
        if pointed {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    });
    pointed
}

/// The tucked group panel slid out over the files at the top of its edge, as its layer floats: a
/// card as tall as what it holds, up to the whole area. Returns where it shows.
fn slid_out(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, files: egui::Rect, left: bool) -> egui::Rect {
    use egui::{Align2, pos2};
    let top = files.top() + HEADER_H + LAYER_INSET;
    let (pivot, at) =
        if left { (Align2::LEFT_TOP, pos2(files.left() + LAYER_INSET, top)) } else { (Align2::RIGHT_TOP, pos2(files.right() - LAYER_INSET, top)) };
    let widest = (files.width() - 2.0 * LAYER_INSET).max(LAYER_MIN_W);
    let width = app.ui.side_layer_w.unwrap_or(LAYER_W).clamp(LAYER_MIN_W, widest);
    let tall = (files.bottom() - LAYER_INSET - top).max(LAYER_MIN_H);
    let shadow = ui.style().visuals.popup_shadow;
    let area = egui::Area::new(egui::Id::unique("monun.group_slid_out")).order(egui::Order::Middle).pivot(pivot).fixed_pos(at).constrain(false).show(
        ui.ctx(),
        |ui| {
            // The room it may take, or it shrinks to what it holds and never grows back.
            ui.set_max_width(width);
            ui.set_max_height(tall);
            card(t, shadow).show(ui, |ui| {
                ui.set_width(width - 20.0);
                header(app, ui, "group", t, false);
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height((tall - HEADER_H - 16.0).max(40.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| content(app, ui, "group"));
            });
        },
    );
    area.response.rect
}

/// A layer's card: the ribbon's colour, a thin border, rounded corners and a shadow.
fn card(t: &Tokens, shadow: egui::Shadow) -> egui::Frame {
    egui::Frame::new().fill(t.ribbon).stroke(egui::Stroke::new(1.0, t.ribbon_border)).corner_radius(8.0).shadow(shadow).inner_margin(egui::Margin {
        left: 10,
        right: 10,
        top: 0,
        bottom: 10,
    })
}

/// What a pane shows under its title.
fn content(app: &mut SheetApp, ui: &mut Ui, pane: &str) {
    match pane {
        // The file in front's memos (a sheet's: its cells' comments), then the group's others.
        "comments" => crate::memos::panel(app, ui),
        "group" => crate::groups::panel(app, ui),
        "watch" => watch(app, ui),
        "selection" => selection(app, ui),
        "formatChart" => format_chart(app, ui),
        _ => {}
    }
}

/// A pane's title strip's height: a document's tool bar's, so their lines meet.
const HEADER_H: f32 = 36.0;
/// A layer's width, and how far it keeps from the edges of the files' area.
const LAYER_W: f32 = 300.0;
const LAYER_INSET: f32 = 12.0;

/// A corner of the files' area, where the memo or group panel floats as a layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Corner {
    #[default]
    TopRight,
    BottomRight,
    TopLeft,
    BottomLeft,
}

impl Corner {
    const ALL: [(Corner, &'static str); 4] =
        [(Corner::TopRight, "Top right"), (Corner::BottomRight, "Bottom right"), (Corner::TopLeft, "Top left"), (Corner::BottomLeft, "Bottom left")];

    /// The corner of `area` nearest `p`.
    fn nearest(area: egui::Rect, p: egui::Pos2) -> Corner {
        match (p.x < area.center().x, p.y < area.center().y) {
            (false, true) => Corner::TopRight,
            (false, false) => Corner::BottomRight,
            (true, true) => Corner::TopLeft,
            (true, false) => Corner::BottomLeft,
        }
    }
}

/// The memo or group panel floating over the files' area (`ui`'s room) in its corner: as large as
/// it was made (its free edges, those away from its corner, drag to resize it; a double-click on
/// one goes back), or at first a column's width and as tall as it needs up to most of the area.
/// Its top corners keep clear of a document's tool bar.
fn layer(app: &mut SheetApp, ui: &mut Ui, pane: &str, t: &Tokens) {
    use egui::{Align2, pos2};
    let area = ui.available_rect_before_wrap();
    let top = area.top() + HEADER_H + LAYER_INSET;
    let widest = (area.width() - 2.0 * LAYER_INSET).max(LAYER_MIN_W);
    let tallest = (area.bottom() - LAYER_INSET - top).max(LAYER_MIN_H);
    let width = app.ui.side_layer_w.unwrap_or(LAYER_W).clamp(LAYER_MIN_W, widest);
    // Auto Height (no height chosen): as tall as what it holds, up to the whole area.
    let chosen = app.ui.side_layer_h.map(|h| h.clamp(LAYER_MIN_H, tallest));
    let tall = chosen.unwrap_or(tallest);
    let corner = app.ui.side_corner;
    let (pivot, at) = match corner {
        Corner::TopRight => (Align2::RIGHT_TOP, pos2(area.right() - LAYER_INSET, top)),
        Corner::BottomRight => (Align2::RIGHT_BOTTOM, pos2(area.right() - LAYER_INSET, area.bottom() - LAYER_INSET)),
        Corner::TopLeft => (Align2::LEFT_TOP, pos2(area.left() + LAYER_INSET, top)),
        Corner::BottomLeft => (Align2::LEFT_BOTTOM, pos2(area.left() + LAYER_INSET, area.bottom() - LAYER_INSET)),
    };
    // While its title is dragged it follows the pointer.
    let at = at + app.side_drag.unwrap_or_default();
    let mut title = None;
    let mut resized = None;
    let mut shown = 0.0;
    let shadow = ui.style().visuals.popup_shadow;
    egui::Area::new(egui::Id::unique("monun.side_layer")).order(egui::Order::Middle).pivot(pivot).fixed_pos(at).constrain_to(area).show(
        ui.ctx(),
        |ui| {
            // An area gives what it holds the room it took last frame: without the room it may
            // take, a layer sized to what it holds shrinks but never grows back (a group opened,
            // files added).
            ui.set_max_width(width);
            ui.set_max_height(tall);
            let frame = card(t, shadow).show(ui, |ui| {
                ui.set_width(width - 20.0);
                title = Some(header(app, ui, pane, t, true));
                ui.add_space(6.0);
                let room = (tall - HEADER_H - 16.0).max(40.0);
                let mut scroll = egui::ScrollArea::vertical().max_height(room);
                // A height chosen is kept, however little it holds.
                scroll =
                    if chosen.is_some() { scroll.min_scrolled_height(room).auto_shrink([false, false]) } else { scroll.auto_shrink([false, true]) };
                scroll.show(ui, |ui| content(app, ui, pane));
            });
            shown = frame.response.rect.height();
            app.side_layer_shown_h = shown;
            resized = edges(ui, frame.response.rect, corner, t);
        },
    );
    match resized {
        // From its height as chosen, or the one it shows at while it is sized to what it holds.
        Some(Resized::By(dw, dh)) => {
            app.ui.side_layer_w = Some((width + dw).clamp(LAYER_MIN_W, widest));
            if dh != 0.0 {
                app.ui.side_layer_h = Some((chosen.unwrap_or(shown) + dh).clamp(LAYER_MIN_H, tallest));
            }
        }
        Some(Resized::Back) => {
            app.ui.side_layer_w = None;
            app.ui.side_layer_h = None;
        }
        None => {}
    }
    if let Some((title, strip)) = title {
        moved(app, ui.ctx(), &title, strip.width(), area, true);
    }
}

/// The smallest a layer is made.
const LAYER_MIN_W: f32 = 220.0;
const LAYER_MIN_H: f32 = 80.0;
/// How wide a layer's edge is to grab.
const EDGE: f32 = 10.0;

/// What a layer's edges were dragged to: its size changed by this much (width, height), or back to
/// its own (a double-click).
enum Resized {
    By(f32, f32),
    Back,
}

/// A layer's free edges and the corner between them (those away from its `corner`, which stays
/// put), to drag: the cursor tells which way each one goes.
fn edges(ui: &mut Ui, rect: egui::Rect, corner: Corner, t: &Tokens) -> Option<Resized> {
    use egui::{CursorIcon, Rect, Sense, pos2};
    let left = matches!(corner, Corner::TopRight | Corner::BottomRight);
    let below = matches!(corner, Corner::TopRight | Corner::TopLeft);
    // The free side edge and the free top or bottom edge, as strips across them (half in, half
    // out), and the corner between them.
    let x = if left { rect.left() } else { rect.right() } - EDGE / 2.0;
    let y = if below { rect.bottom() } else { rect.top() } - EDGE / 2.0;
    let side = Rect::from_min_size(pos2(x, rect.top() + 10.0), egui::vec2(EDGE, rect.height() - 20.0));
    let end = Rect::from_min_size(pos2(rect.left() + 10.0, y), egui::vec2(rect.width() - 20.0, EDGE));
    let tip = Rect::from_min_size(pos2(x, y), egui::vec2(EDGE, EDGE)).expand(3.0);
    let diagonal = if left == below { CursorIcon::ResizeNeSw } else { CursorIcon::ResizeNwSe };
    let mut out = None;
    for (zone, grows, cursor, id) in [
        (side, (true, false), CursorIcon::ResizeHorizontal, "side"),
        (end, (false, true), CursorIcon::ResizeVertical, "end"),
        (tip, (true, true), diagonal, "tip"),
    ] {
        let resp = ui.interact(zone, ui.scope_id().with(("monun.layer_edge", id)), Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, true, "Resize the panel"));
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
            ui.painter().rect_filled(zone, 2.0, t.accent.gamma_multiply(0.35));
        }
        if resp.double_clicked() {
            out = Some(Resized::Back);
        } else if resp.dragged() {
            let d = resp.drag_delta();
            // An edge on the left grows as it goes left; one on top, as it goes up.
            let dw = if grows.0 { if left { -d.x } else { d.x } } else { 0.0 };
            let dh = if grows.1 { if below { d.y } else { -d.y } } else { 0.0 };
            if dw != 0.0 || dh != 0.0 {
                out = Some(Resized::By(dw, dh));
            }
        }
    }
    out
}

/// A pane's title strip: the title, a memo or group panel's options (where it goes) and Close; the
/// group panel's New Group and Sort as well. Returns the title beside the buttons, by which a memo
/// or group panel is dragged, and the whole strip.
fn header(app: &mut SheetApp, ui: &mut Ui, pane: &str, t: &Tokens, movable: bool) -> (egui::Response, egui::Rect) {
    use egui::{Align2, Rect, Sense, Stroke, pos2};
    let lang = app.ui.language;
    let title = match pane {
        "comments" => "Comments",
        "group" => "Groups",
        "watch" => "Watch Window",
        "selection" => "Selection",
        "formatChart" => "Format Chart Area",
        _ => "",
    };
    let title = lang.tr(title).to_string();
    let (strip, heading) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    heading.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, &title));
    // The line under it runs the pane's whole width, out through its margins, on the very row a
    // document tool bar's line is drawn on.
    ui.painter()
        .line_segment([pos2(strip.left() - 10.0, strip.bottom()), pos2(strip.right() + 10.0, strip.bottom())], Stroke::new(1.0, t.ribbon_border));
    ui.painter().text(pos2(strip.left(), strip.center().y), Align2::LEFT_CENTER, &title, theme::ui_bold(14.0), t.text);
    let buttons = Rect::from_min_max(pos2(strip.right() - 120.0, strip.top()), strip.max);
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let size = vec2(22.0, 22.0);
    // The group panel tucks into its edge rather than closing (the title bar's button hides it);
    // slid out from there, its pin keeps it shown again.
    let close = match pane {
        "group" if app.ui.group_tucked => crate::widgets::icon_button(&mut row, crate::icons::Icon::Pin, t.text_dim, "Keep the Panel Shown", size),
        "group" => {
            let icon = if tuck_left(app) { crate::icons::Icon::TuckLeft } else { crate::icons::Icon::TuckRight };
            crate::widgets::icon_button(&mut row, icon, t.text_dim, "Hide to the Side", size)
        }
        _ => crate::widgets::icon_button(&mut row, crate::icons::Icon::Close, t.text_dim, "Close", size),
    };
    let mut pointing = close.hovered();
    if close.clicked() {
        match pane {
            "comments" => app.run_or_alert("review.showComments", json!({"on": false})),
            "group" => {
                app.ui.group_tucked = !app.ui.group_tucked;
                app.group_peek = None;
            }
            _ => app.grid.pane = None,
        }
    }
    if movable {
        let more = crate::widgets::icon_button(&mut row, crate::icons::Icon::More, t.text_dim, "Panel options", size);
        more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr("Panel options")));
        pointing |= more.hovered();
        egui::Popup::menu(&more).show(|ui| options(app, ui));
    }
    // The group panel's Sort and New Group…, beside them.
    if pane == "group" {
        let sort = crate::widgets::icon_button(&mut row, crate::icons::Icon::SortAsc, t.text_dim, "Sort Groups and Files", size);
        sort.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr("Sort Groups and Files")));
        pointing |= sort.hovered();
        egui::Popup::menu(&sort).show(|ui| crate::groups::sort_menu(app, ui));
        let new = crate::widgets::icon_button(&mut row, crate::icons::Icon::Plus, t.text_dim, "New File or Group", size);
        new.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, lang.tr("New File or Group")));
        pointing |= new.hovered();
        egui::Popup::menu(&new).show(|ui| crate::groups::new_menu(app, ui));
    }
    // The buttons are for clicking: the hand shows over them, and the title drags only beside them
    // (a press on one never takes the panel along).
    if pointing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let grip = Rect::from_min_max(strip.min, pos2((row.min_rect().left() - 4.0).max(strip.left()), strip.bottom()));
    let sense = if movable { Sense::click_and_drag() } else { Sense::hover() };
    (ui.interact(grip, ui.scope_id().with("monun.pane_title"), sense), strip)
}

/// The memo and group panels' options: a column on the right or left, or a layer in a corner.
fn options(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    ui.set_min_width(200.0);
    let mut layer = app.ui.side_layer;
    if ui.checkbox(&mut layer, lang.tr("Float as a layer")).changed() {
        app.ui.side_layer = layer;
        // It stays on its side: a layer in the corner on the column's side (its top or bottom as
        // last chosen), a column on the layer's side.
        let bottom = matches!(app.ui.side_corner, Corner::BottomRight | Corner::BottomLeft);
        if layer {
            app.ui.side_corner = match (app.ui.side_left, bottom) {
                (false, false) => Corner::TopRight,
                (false, true) => Corner::BottomRight,
                (true, false) => Corner::TopLeft,
                (true, true) => Corner::BottomLeft,
            };
        } else {
            app.ui.side_left = matches!(app.ui.side_corner, Corner::TopLeft | Corner::BottomLeft);
        }
    }
    ui.separator();
    if app.ui.side_layer {
        for (corner, label) in Corner::ALL {
            if ui.radio(app.ui.side_corner == corner, lang.tr(label)).clicked() {
                app.ui.side_corner = corner;
                ui.close();
            }
        }
        ui.separator();
        // As tall as what it holds, growing and shrinking with it; off, it keeps the height it
        // shows at now (its bottom or top edge drags it, which turns this off too).
        let mut auto = app.ui.side_layer_h.is_none();
        if ui.checkbox(&mut auto, lang.tr("Auto Height")).on_hover_text(lang.tr("As tall as what the panel holds")).changed() {
            app.ui.side_layer_h = if auto { None } else { Some(app.side_layer_shown_h.max(LAYER_MIN_H)) };
        }
    } else {
        for (left, label) in [(false, "Right side"), (true, "Left side")] {
            if ui.radio(app.ui.side_left == left, lang.tr(label)).clicked() {
                app.ui.side_left = left;
                ui.close();
            }
        }
    }
    ui.separator();
    ui.label(egui::RichText::new(lang.tr("Drag the title to move it.")).small().weak());
}

/// A memo or group panel's title dragged: a layer follows the pointer and settles in the corner of
/// `area` (the files') it is let go nearest; a column (`strip_w` wide inside its margins), shown
/// where it would go, moves to the side of `area` (the window) it is let go on.
fn moved(app: &mut SheetApp, ctx: &egui::Context, title: &egui::Response, strip_w: f32, area: egui::Rect, layer: bool) {
    let t = Tokens::get(ctx);
    if title.dragged() {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        *app.side_drag.get_or_insert(egui::Vec2::ZERO) += title.drag_delta();
        if !layer && let Some(p) = ctx.pointer_interact_pos() {
            // Where the column would go.
            let width = strip_w + 20.0;
            let side = if p.x < area.center().x {
                egui::Rect::from_min_size(area.min, vec2(width, area.height()))
            } else {
                egui::Rect::from_min_max(egui::pos2(area.right() - width, area.top()), area.max)
            };
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("monun.side_drop")));
            painter.rect_filled(side, 0.0, t.accent.gamma_multiply(0.15));
            painter.rect_stroke(side, 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
    } else if title.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::Grab);
    }
    if title.drag_stopped() {
        let p = ctx.input(|i| i.pointer.latest_pos());
        if let Some(p) = p {
            if layer {
                app.ui.side_corner = Corner::nearest(area, p);
            } else {
                app.ui.side_left = p.x < area.center().x;
            }
        }
        app.side_drag = None;
    }
}

/// Whether the memo panel and its button are offered for the file in front: a workbook or a
/// Markdown file, not a SQL or plain text file (nor the start page).
pub(crate) fn memo_allowed(app: &SheetApp) -> bool {
    app.session.active().is_some_and(|d| matches!(d.text_kind(), None | Some(monun_engine::docs::TextKind::Markdown)))
}

/// Whether the memo panel shows (its button is on).
pub(crate) fn memos_shown(app: &SheetApp) -> bool {
    app.session.comments_pane && app.grid.pane.is_none()
}

/// Whether the group panel shows (its button is on).
pub(crate) fn groups_shown(app: &SheetApp) -> bool {
    app.file_tabs && app.ui.group_panel && !app.session.comments_pane && app.grid.pane.is_none()
}

/// Shows or hides the memo panel (the title bar's memo button), closing any other pane (the group
/// panel shows again when it closes).
pub(crate) fn toggle_memos(app: &mut SheetApp) {
    let on = !memos_shown(app);
    if on {
        app.grid.pane = None;
    }
    app.run_or_alert("review.showComments", json!({"on": on}));
}

/// Shows or hides the group panel (the title bar's group panel button), closing any other pane.
/// Shown again, it is no longer tucked into its edge.
pub(crate) fn toggle_groups(app: &mut SheetApp) {
    let on = !groups_shown(app);
    app.ui.group_tucked = false;
    app.group_peek = None;
    if on {
        app.grid.pane = None;
        if app.session.comments_pane {
            app.run_or_alert("review.showComments", json!({"on": false}));
        }
    }
    app.ui.group_panel = on;
}

/// Lists comments and replies, links each to its cell, and runs delete/resolve/reopen/reply commands.
pub(crate) fn comments(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let Some(d) = app.session.active() else { return };
    let Some(sh) = d.wb.active() else { return };
    let list: Vec<(String, monun_engine::model::Comment)> = sh.comments.iter().map(|(c, m)| (c.a1(), m.clone())).collect();
    if list.is_empty() {
        ui.label(egui::RichText::new(lang.tr("No comments on this sheet yet. Select a cell and choose New Comment.")).italics());
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    for (cell, c) in list {
        egui::Frame::NONE
            .fill(if c.resolved { t.window } else { t.menu_bg })
            .corner_radius(6.0)
            .inner_margin(8.0)
            .stroke(egui::Stroke::new(1.0, t.separator))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    if ui.link(egui::RichText::new(&cell).strong()).clicked() {
                        let _ = app.run("selection.set", json!({"cell": cell}));
                        app.grid.ensure_visible = true;
                    }
                    ui.label(egui::RichText::new(&c.author).color(t.text_dim));
                    if c.resolved {
                        ui.label(egui::RichText::new(lang.tr("Resolved")).small().color(t.accent));
                    }
                });
                ui.label(&c.text);
                for (a, txt) in &c.replies {
                    ui.label(egui::RichText::new(format!("↳ {a}: {txt}")).color(t.text));
                }
                let key = egui::Id::unique(("reply", &cell));
                let mut draft: String = ui.ctx().data_mut(|m| m.get_temp(key)).unwrap_or_default();
                // Buttons first, right to left, so the reply field gets exactly the width they leave:
                // a field sized by a guess wider than the row pushes the resizable pane wider every frame.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(lang.tr("Delete")).clicked() {
                            app.run_or_alert("review.deleteComment", json!({"cell": cell}));
                        }
                        if ui.small_button(lang.tr(if c.resolved { "Reopen" } else { "Resolve" })).clicked() {
                            app.run_or_alert("review.resolveComment", json!({"cell": cell, "resolved": !c.resolved}));
                        }
                        let send = ui.small_button(lang.tr("Send")).clicked();
                        let r = ui.add(egui::TextEdit::singleline(&mut draft).hint_text(lang.tr("Reply…")).desired_width(ui.available_width()));
                        if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || send) && !draft.is_empty() {
                            app.run_or_alert("review.replyComment", json!({"cell": cell, "text": draft}));
                            draft.clear();
                        }
                    });
                });
                ui.ctx().data_mut(|m| m.insert_temp(key, draft));
            });
        ui.add_space(6.0);
    }
}

/// Adds the active cell, lists watch values and formulas, and removes watched entries.
fn watch(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    if ui.button(lang.tr("Add Watch (active cell)")).clicked()
        && let Some(d) = app.session.active()
        && let Some(sh) = d.wb.active()
    {
        let r = format!("{}!{}", monun_engine::formula::quote_sheet(&sh.name), d.selection.active.a1());
        let _ = app.session.run("formulas.watchWindow", json!({"add": r}));
    }
    let list = app.session.run("formulas.watchWindow", json!({})).unwrap_or(Json::Null);
    egui::Grid::new("watch_grid").striped(true).num_columns(4).show(ui, |ui| {
        ui.strong(lang.tr("Cell"));
        ui.strong(lang.tr("Value"));
        ui.strong(lang.tr("Formula"));
        ui.label("");
        ui.end_row();
        for w in list.as_array().cloned().unwrap_or_default() {
            let cell = w["cell"].as_str().unwrap_or("").to_string();
            ui.label(&cell);
            ui.label(w["value"].as_str().unwrap_or(""));
            ui.label(w["formula"].as_str().unwrap_or(""));
            if crate::widgets::icon_button(ui, crate::icons::Icon::Close, Tokens::get(ui.ctx()).text_dim, "Delete", vec2(18.0, 18.0)).clicked() {
                let _ = app.session.run("formulas.watchWindow", json!({"remove": cell}));
            }
            ui.end_row();
        }
    });
}

/// Lists the sheet's objects and dispatches select, bring-forward, send-backward and delete.
fn selection(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let list = app.session.run("arrange.selectionPane", json!({})).unwrap_or(Json::Null);
    let items = list.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        ui.label(egui::RichText::new(lang.tr("There are no shapes, pictures or charts on this sheet.")).italics());
    }
    for it in items.iter().rev() {
        let kind = it["kind"].as_str().unwrap_or("").to_string();
        let id = it["id"].as_u64().unwrap_or(0) as u32;
        ui.horizontal(|ui| {
            let sel = app.selected_chart == Some(id);
            if ui.selectable_label(sel, it["name"].as_str().unwrap_or("")).clicked() {
                app.selected_chart = Some(id);
            }
            if ui.small_button("▲").on_hover_text(lang.tr("Bring Forward")).clicked() {
                app.run_or_alert("arrange.bringForward", json!({"kind": kind, "id": id}));
            }
            if ui.small_button("▼").on_hover_text(lang.tr("Send Backward")).clicked() {
                app.run_or_alert("arrange.sendBackward", json!({"kind": kind, "id": id}));
            }
            if crate::widgets::icon_button(ui, crate::icons::Icon::Close, Tokens::get(ui.ctx()).text_dim, "Delete", vec2(18.0, 18.0)).clicked() {
                app.run_or_alert("object.delete", json!({"kind": kind, "id": id}));
            }
        });
    }
}

/// Draws the chart format pane: title, type, legend, elements/axis titles, series colors, palettes/layouts and size.
fn format_chart(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let Some(id) = app.selected_chart else {
        ui.label(egui::RichText::new(lang.tr("Select a chart to format it.")).italics());
        return;
    };
    let Some(chart) = app.session.active().and_then(|d| d.wb.active().and_then(|s| s.charts.iter().find(|c| c.id == id).cloned())) else {
        ui.label(lang.tr("The chart no longer exists."));
        return;
    };
    let theme = app.session.active().map(|d| d.wb.theme.clone()).unwrap_or_default();
    ui.collapsing(lang.tr("Chart Title"), |ui| {
        let field = egui::Id::unique(("monun.chart_title", id));
        if let Some(title) = crate::widgets::committed_text(ui, field, chart.title.as_deref().unwrap_or(""), None) {
            app.run_or_alert("chart.set", json!({"chart": id, "title": title}));
        }
    });
    ui.collapsing(lang.tr("Chart Type"), |ui| {
        ui.horizontal_wrapped(|ui| {
            for (l, k, s) in [
                ("Chart Type|Column", "column", ""),
                ("Stacked Column", "column", "stacked"),
                ("100% Stacked Column", "column", "stacked100"),
                ("Bar", "bar", ""),
                ("Stacked Bar", "bar", "stacked"),
                ("100% Stacked Bar", "bar", "stacked100"),
                ("Line", "line", "markers"),
                ("Stacked Line", "line", "stacked"),
                ("Area", "area", ""),
                ("Stacked Area", "area", "stacked"),
                ("100% Stacked Area", "area", "stacked100"),
                ("Pie", "pie", ""),
                ("Doughnut", "doughnut", ""),
                ("Scatter", "scatter", ""),
                ("Radar", "radar", ""),
            ] {
                let cur = chart.kind == monun_engine::cmd::insert::chart_kind(k, s);
                if ui.selectable_label(cur, lang.tr(l)).clicked() {
                    app.run_or_alert("chart.set", json!({"chart": id, "type": k, "subtype": s}));
                }
            }
        });
    });
    ui.collapsing(lang.tr("Legend"), |ui| {
        ui.horizontal(|ui| {
            for (l, v) in [("None", "none"), ("Bottom", "bottom"), ("Top", "top"), ("Left", "left"), ("Right", "right")] {
                let cur = format!("{:?}", chart.legend).eq_ignore_ascii_case(v);
                if ui.selectable_label(cur, lang.tr(l)).clicked() {
                    app.run_or_alert("chart.set", json!({"chart": id, "legend": v}));
                }
            }
        });
    });
    ui.collapsing(lang.tr("Elements"), |ui| {
        let mut dl = chart.data_labels;
        if ui.checkbox(&mut dl, lang.tr("Data Labels")).changed() {
            app.run_or_alert("chart.set", json!({"chart": id, "dataLabels": dl}));
        }
        let mut gl = chart.gridlines;
        if ui.checkbox(&mut gl, lang.tr("Gridlines")).changed() {
            app.run_or_alert("chart.set", json!({"chart": id, "gridlines": gl}));
        }
        ui.horizontal(|ui| {
            ui.label(lang.tr("Horizontal axis title:"));
            let field = egui::Id::unique(("monun.chart_x_title", id));
            if let Some(xt) = crate::widgets::committed_text(ui, field, chart.x_title.as_deref().unwrap_or(""), None) {
                app.run_or_alert("chart.set", json!({"chart": id, "xTitle": if xt.is_empty() { Json::Null } else { json!(xt) }}));
            }
        });
        ui.horizontal(|ui| {
            ui.label(lang.tr("Vertical axis title:"));
            let field = egui::Id::unique(("monun.chart_y_title", id));
            if let Some(yt) = crate::widgets::committed_text(ui, field, chart.y_title.as_deref().unwrap_or(""), None) {
                app.run_or_alert("chart.set", json!({"chart": id, "yTitle": if yt.is_empty() { Json::Null } else { json!(yt) }}));
            }
        });
    });
    ui.collapsing(lang.tr("Chart|Series"), |ui| {
        let data = app.session.active().map(|d| monun_chart::resolve(&d.wb, d.wb.active_sheet(), &chart));
        for (i, s) in chart.series.iter().enumerate() {
            let name = data.as_ref().and_then(|d| d.series.get(i)).map(|x| x.name.clone()).unwrap_or_else(|| format!("Series {}", i + 1));
            let col =
                data.as_ref().and_then(|d| d.series.get(i)).map(|x| Color32::from_rgb(x.color[0], x.color[1], x.color[2])).unwrap_or(Color32::GRAY);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 2.0, col);
                ui.label(&name);
                ui.label(egui::RichText::new(&s.values).small().color(crate::theme::Tokens::get(ui.ctx()).text_dim));
                ui.menu_button(lang.tr("Fill…"), |ui| {
                    if let Some(c) = crate::widgets::color_palette_fixed(ui, &theme.colors, lang.tr("Automatic")) {
                        app.run_or_alert(
                            "chart.formatSelection",
                            json!({"chart": id, "series": i, "color": if c == "none" { Json::Null } else { json!(c) }}),
                        );
                        ui.close();
                    }
                });
            });
        }
    });
    ui.collapsing(lang.tr("Colors & Layout"), |ui| {
        ui.horizontal_wrapped(|ui| {
            for p in ["colorful", "monochrome", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6"] {
                if ui.button(p).clicked() {
                    app.run_or_alert("chart.changeColors", json!({"chart": id, "palette": p}));
                }
            }
        });
        ui.horizontal(|ui| {
            for n in 1..=6 {
                if ui.button(lang.tr("Layout {n}").replace("{n}", &n.to_string())).clicked() {
                    app.run_or_alert("chart.quickLayout", json!({"chart": id, "layout": n}));
                }
            }
        });
    });
    ui.collapsing(lang.tr("Size"), |ui| {
        // Keep the value being typed or dragged between frames; reloading it from the chart every frame threw the edit away.
        let draft = egui::Id::unique(("monun.chart_size_draft", id));
        let (mut w, mut h) = ui.data_mut(|d| d.get_temp::<(f32, f32)>(draft)).unwrap_or((chart.anchor.width, chart.anchor.height));
        let a = ui.add(egui::DragValue::new(&mut w).prefix(format!("{} ", lang.tr("Width"))).range(40.0..=4000.0));
        let b = ui.add(egui::DragValue::new(&mut h).prefix(format!("{} ", lang.tr("Height"))).range(40.0..=4000.0));
        if a.drag_stopped() || a.lost_focus() || b.drag_stopped() || b.lost_focus() {
            ui.data_mut(|d| d.remove::<(f32, f32)>(draft));
            if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.run_or_alert("chart.set", json!({"chart": id, "width": w, "height": h}));
            }
        } else if a.dragged() || a.has_focus() || b.dragged() || b.has_focus() {
            ui.data_mut(|d| d.insert_temp(draft, (w, h)));
        } else {
            ui.data_mut(|d| d.remove::<(f32, f32)>(draft));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{HANDLE, HANDLE_GAP, below_handle, scroll_bars_below};
    use egui::{Rect, pos2};

    /// A scroll area of 1000 × 700 at (100, 50), and the handle by its right edge, under its top.
    fn area_and_handle() -> (Rect, Rect) {
        let area = Rect::from_min_max(pos2(100.0, 50.0), pos2(1100.0, 750.0));
        let handle = Rect::from_min_size(pos2(area.right() - HANDLE_GAP - HANDLE.x, area.top() + 10.0), HANDLE);
        (area, handle)
    }

    #[test]
    fn a_scroll_bar_under_the_handle_starts_below_it() {
        let (area, handle) = area_and_handle();
        let bars = scroll_bars_below(Some(handle), area).unwrap_or(area);
        assert_eq!(bars.top(), handle.bottom() + HANDLE_GAP);
        assert_eq!((bars.left(), bars.right(), bars.bottom()), (area.left(), area.right(), area.bottom()));
        // A sheet's own track, as wide as itself.
        let track = Rect::from_min_max(pos2(area.right() - 17.0, area.top()), area.max);
        assert_eq!(below_handle(Some(handle), track, track.width()).map(|r| r.top()), Some(handle.bottom() + HANDLE_GAP));
    }

    #[test]
    fn a_scroll_bar_clear_of_the_handle_stays_as_it_is() {
        let (area, handle) = area_and_handle();
        assert_eq!(scroll_bars_below(None, area), None, "no handle");
        // The handle by the left edge: the bar runs down the right one.
        let left = handle.translate(egui::vec2(area.left() + HANDLE_GAP - handle.left(), 0.0));
        assert_eq!(scroll_bars_below(Some(left), area), None);
        // A scroll area that ends before the handle (a side-by-side editor's left half).
        let half = Rect::from_min_max(area.min, pos2(area.center().x, area.bottom()));
        assert_eq!(scroll_bars_below(Some(handle), half), None);
        // A handle below the area.
        let below = handle.translate(egui::vec2(0.0, area.height() + 20.0));
        assert_eq!(scroll_bars_below(Some(below), area), None);
    }

    #[test]
    fn a_web_page_leaves_the_handle_its_strip() {
        use super::beside_handle;
        let (area, handle) = area_and_handle();
        let room = beside_handle(Some(handle), area);
        assert_eq!((room.left(), room.right()), (area.left(), handle.left() - HANDLE_GAP));
        assert_eq!((room.top(), room.bottom()), (area.top(), area.bottom()));
        assert_eq!(beside_handle(None, area), area);
        // By the left edge, the page starts beside it.
        let left = handle.translate(egui::vec2(area.left() + HANDLE_GAP - handle.left(), 0.0));
        assert_eq!(beside_handle(Some(left), area).left(), left.right() + HANDLE_GAP);
    }
}
