//! The right-click menus take Excel's access keys: a letter runs its item, a letter several items
//! share moves between them and Enter runs the highlighted one, and the key shows on the label,
//! underlined in English and in brackets in Korean (삭제(D)…).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Shape, accesskit::Role, pos2};
use egui_kittest::kittest::Queryable as _;
use monun_engine::core::CellRef;
use monun_ui_egui::{SheetApp, grid};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut h = crate::support::blank_harness(4);
    h.state_mut().run("cell.set", json!({"cell": "C5", "input": "x"})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "C6", "input": "y"})).unwrap();
    h.run_steps(2);
    h
}

fn geo(h: &Harness) -> grid::Geo {
    let app = h.state();
    let sh = app.session.active().unwrap().wb.active().unwrap();
    grid::Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll)
}

fn cell(h: &Harness, a1: &str) -> Pos2 {
    let app = h.state();
    let sh = app.session.active().unwrap().wb.active().unwrap();
    geo(h).cell_rect(sh, CellRef::parse(a1).unwrap()).center()
}

fn right_click(h: &mut Harness, pos: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

/// A key press as Windows delivers it: the key, then the text a letter types.
fn press(h: &mut Harness, key: Key) {
    h.input_mut().events.push(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    let name = key.name();
    if name.len() == 1 {
        h.input_mut().events.push(Event::Text(name.to_lowercase()));
    }
    h.input_mut().events.push(Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn dialog(h: &Harness) -> Option<String> {
    h.state().dialog.as_ref().map(|d| d.name.clone())
}

fn text(h: &Harness, a1: &str) -> String {
    let d = h.state().session.active().unwrap();
    d.wb.active().unwrap().value(CellRef::parse(a1).unwrap()).display()
}

/// Where a sheet tab's name is painted.
fn tab(h: &Harness, name: &str) -> Pos2 {
    h.output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text == name => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("tab {name} is not painted"))
}

#[test]
fn a_letter_runs_its_item_and_types_nothing_into_the_cell() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    assert!(h.state().grid.context_menu.is_some());
    press(&mut h, Key::D);
    assert_eq!(dialog(&h).as_deref(), Some("deleteCells"), "D is Delete…");
    assert!(h.state().grid.context_menu.is_none(), "the menu closes");
    assert!(h.state().editor.is_none(), "the letter doesn't start an edit");
}

#[test]
fn a_shared_letter_moves_between_its_items_and_enter_runs_the_highlighted_one() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    // Clear Contents and New Note are both N, as in Excel.
    press(&mut h, Key::N);
    assert!(h.state().grid.context_menu.is_some(), "the first N only highlights Clear Contents");
    assert_eq!(text(&h, "C5"), "x");
    press(&mut h, Key::N);
    press(&mut h, Key::Enter);
    assert_eq!(dialog(&h).as_deref(), Some("note"), "Enter runs New Note, the second N");
    assert_eq!(text(&h, "C5"), "x", "Clear Contents didn't run");
    assert!(h.state().editor.is_none());
}

#[test]
fn arrows_move_the_highlight_and_enter_runs_it() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::ArrowDown);
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "C5", "the arrows stay in the menu");
    press(&mut h, Key::Enter);
    assert!(h.state().grid.context_menu.is_none());
    assert!(h.state().session.clipboard.is_some(), "the second item, Copy, ran");
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "C5", "Enter doesn't move the cell pointer");
}

#[test]
fn filter_e_then_v_filters_by_the_cells_value() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    press(&mut h, Key::E);
    assert!(h.query_by_label("Filter by Selected Cell's Value").is_some(), "E opens the Filter submenu");
    press(&mut h, Key::V);
    assert!(h.state().grid.context_menu.is_none());
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    assert!(sh.autofilter.is_some(), "the range is filtered");
    assert!(sh.is_row_hidden(5) && !sh.is_row_hidden(4), "only C5's value is shown");
}

#[test]
fn escape_closes_the_submenu_before_the_menu() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    press(&mut h, Key::O);
    assert!(h.query_by_label("Sort Z to A").is_some(), "O opens the Sort submenu");
    press(&mut h, Key::Escape);
    assert!(h.query_by_label("Sort Z to A").is_none(), "Escape closes the submenu");
    assert!(h.state().grid.context_menu.is_some(), "and leaves the menu open");
    press(&mut h, Key::Escape);
    assert!(h.state().grid.context_menu.is_none());
}

#[test]
fn row_and_column_header_menus_take_their_letters() {
    let mut h = harness();
    let g = geo(&h);
    let row = pos2((g.rect.left() + g.cells.left()) / 2.0, cell(&h, "A3").y);
    right_click(&mut h, row);
    assert_eq!(h.state().grid.header_menu.map(|(_, rows)| rows), Some(true));
    press(&mut h, Key::R);
    assert_eq!(dialog(&h).as_deref(), Some("rowHeight"), "R is Row Height…");
    h.state_mut().dialog = None;
    h.run_steps(2);

    let column = pos2(cell(&h, "B1").x, (g.rect.top() + g.cells.top()) / 2.0);
    right_click(&mut h, column);
    assert_eq!(h.state().grid.header_menu.map(|(_, rows)| rows), Some(false));
    press(&mut h, Key::W);
    assert_eq!(dialog(&h).as_deref(), Some("columnWidth"), "W is Column Width…");
}

#[test]
fn the_sheet_tab_menu_takes_its_letters() {
    let mut h = harness();
    h.state_mut().run("home.insertSheet", json!({})).unwrap();
    h.run_steps(4);
    let at = tab(&h, "Sheet2");
    right_click(&mut h, at);
    assert!(h.state().grid.tab_menu.is_some());
    press(&mut h, Key::R);
    assert_eq!(h.state().grid.renaming_tab, Some(1), "R is Rename");
    assert!(h.state().grid.tab_menu.is_none());
    press(&mut h, Key::Escape);
    h.state_mut().grid.renaming_tab = None;
    h.run_steps(2);

    let at = tab(&h, "Sheet2");
    right_click(&mut h, at);
    press(&mut h, Key::D);
    let names: Vec<String> = h.state().session.active().unwrap().wb.sheets.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["Sheet1"], "D is Delete");
    assert!(h.state().editor.is_none(), "the letter doesn't reach the cells");
}

#[test]
fn the_tab_color_letter_opens_its_palette() {
    let mut h = harness();
    let at = tab(&h, "Sheet1");
    right_click(&mut h, at);
    assert!(h.query_by_label("No Color").is_none());
    press(&mut h, Key::T);
    assert!(h.query_by_label("No Color").is_some(), "T opens Tab Color");
    assert!(h.state().grid.tab_menu.is_some());
}

#[test]
fn english_underlines_the_key() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    let first = egui::text::ByteIndex(0)..egui::text::ByteIndex(1);
    let underlined = h.output().shapes.iter().any(|s| match &s.shape {
        Shape::Text(t) if t.galley.job.text == "Delete…" => {
            t.galley.job.sections.iter().any(|sec| sec.byte_range == first && sec.format.underline.width > 0.0)
        }
        _ => false,
    });
    assert!(underlined, "the D of Delete… is underlined");
}

#[test]
fn the_tab_color_palette_takes_arrows_and_enter() {
    let mut h = harness();
    let at = tab(&h, "Sheet1");
    right_click(&mut h, at);
    press(&mut h, Key::T);
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::Enter);
    assert!(h.state().grid.tab_menu.is_none(), "Enter picks the colour and closes the menu");
    assert!(h.state().session.active().unwrap().wb.sheets[0].tab_color.is_some(), "the tab has the picked colour");
}

#[test]
fn escape_on_the_tab_menu_leaves_the_copy_marquee() {
    let mut h = harness();
    h.state_mut().run("selection.set", json!({"cell": "C5"})).unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().copy_to_clipboard(&ctx, "edit.copy");
    h.run_steps(2);
    assert!(h.state().session.clipboard.is_some());
    let at = tab(&h, "Sheet1");
    right_click(&mut h, at);
    press(&mut h, Key::Escape);
    assert!(h.state().grid.tab_menu.is_none(), "Escape closes the menu");
    assert!(h.state().session.clipboard.is_some(), "and only the menu");
}

#[test]
fn a_menu_opened_again_starts_afresh() {
    let mut h = harness();
    let at = cell(&h, "C5");
    right_click(&mut h, at);
    press(&mut h, Key::O);
    assert!(h.query_by_label("Sort Z to A").is_some());
    // A quick right-click elsewhere closes and opens the menu in one frame.
    let other = cell(&h, "E8");
    h.input_mut().events.push(Event::PointerMoved(other));
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: other, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
    assert!(h.state().grid.context_menu.is_some());
    assert!(h.query_by_label("Sort Z to A").is_none(), "the new menu has no submenu open");
    press(&mut h, Key::D);
    assert_eq!(dialog(&h).as_deref(), Some("deleteCells"), "its letters act on the menu itself");
}

#[test]
fn a_right_click_enters_the_open_edit_first() {
    let mut h = harness();
    h.state_mut().run("selection.set", json!({"cell": "C7"})).unwrap();
    h.state_mut().begin_edit(Some("abc".into()), false);
    h.run_steps(2);
    let at = cell(&h, "C8");
    right_click(&mut h, at);
    assert!(h.state().editor.is_none(), "the edit was entered");
    assert_eq!(text(&h, "C7"), "abc");
    assert!(h.state().grid.context_menu.is_some());
}

#[test]
fn a_submenu_with_no_room_on_the_right_opens_on_the_left() {
    let mut h = harness();
    let right = h.state().grid.rect.unwrap().right() - 30.0;
    let at = pos2(right, cell(&h, "C5").y);
    right_click(&mut h, at);
    press(&mut h, Key::E);
    h.run_steps(2);
    let menu = h.get_by_role_and_label(Role::Button, "Cut").rect();
    let sub = h.get_by_role_and_label(Role::Button, "Filter by Selected Cell's Value").rect();
    assert!(sub.right() <= menu.left(), "the submenu {sub:?} sits left of the menu {menu:?}");
}
