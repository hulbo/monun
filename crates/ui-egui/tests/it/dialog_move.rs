//! Dialogs can be dragged by their title bar, like Excel's, and stay where they are dropped.

use egui::{Event, Modifiers, PointerButton, vec2};
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = crate::common::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        crate::common::app(session),
    );
    h.run_steps(6);
    h
}

/// Returns the remembered egui area rect for a named dialog.
fn dialog_rect(h: &Harness, name: &str) -> egui::Rect {
    h.ctx.memory(|m| m.area_rect(egui::Id::unique("dialog").with(name))).unwrap()
}

fn button(h: &mut Harness, pos: egui::Pos2, pressed: bool) {
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn drag_title_bar(h: &mut Harness, name: &str, by: egui::Vec2) {
    let start = dialog_rect(h, name).center_top() + vec2(0.0, 10.0);
    h.input_mut().events.push(Event::PointerMoved(start));
    h.run_steps(2);
    button(h, start, true);
    for step in 1..=4 {
        h.input_mut().events.push(Event::PointerMoved(start + by * (step as f32 / 4.0)));
        h.run_steps(1);
    }
    button(h, start + by, false);
}

#[test]
fn dragging_the_find_and_replace_title_bar_moves_the_dialog_and_it_stays_there() {
    let mut h = harness();
    h.state_mut().open_dialog("find", json!({}));
    h.run_steps(6);
    let before = dialog_rect(&h, "find");
    drag_title_bar(&mut h, "find", vec2(120.0, 80.0));
    h.run_steps(10);
    let after = dialog_rect(&h, "find");
    assert_eq!(after.min - before.min, vec2(120.0, 80.0), "dialog went from {before:?} to {after:?}");
    assert!(h.state().dialog.is_some(), "the drag does not close the dialog");
}

#[test]
fn go_to_format_cells_and_sort_can_be_dragged_too() {
    for name in ["goTo", "formatCells", "sort"] {
        let mut h = harness();
        h.state_mut().open_dialog(name, json!({}));
        h.run_steps(6);
        let before = dialog_rect(&h, name);
        drag_title_bar(&mut h, name, vec2(-100.0, 60.0));
        h.run_steps(10);
        let after = dialog_rect(&h, name);
        assert_eq!(after.min - before.min, vec2(-100.0, 60.0), "{name} went from {before:?} to {after:?}");
    }
}

#[test]
fn a_dialog_opens_in_the_middle_of_the_window_again_after_it_was_dragged() {
    let mut h = harness();
    h.state_mut().open_dialog("goTo", json!({}));
    h.run_steps(6);
    let middle = h.ctx.content_rect().center() + vec2(0.0, -40.0);
    let first = dialog_rect(&h, "goTo");
    assert!((first.center() - middle).length() < 2.0, "{first:?} about {middle:?}");
    drag_title_bar(&mut h, "goTo", vec2(-300.0, 250.0));
    h.run_steps(4);
    // Closed and opened again: in the middle, not where it was left.
    h.state_mut().dialog = None;
    h.run_steps(3);
    h.state_mut().open_dialog("goTo", json!({}));
    h.run_steps(6);
    let again = dialog_rect(&h, "goTo");
    assert!((again.center() - middle).length() < 2.0, "{again:?} about {middle:?}");
}
