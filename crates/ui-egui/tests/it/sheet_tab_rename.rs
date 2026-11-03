//! Renaming a sheet tab: the name field must let go of the keyboard when the user is done (#145).

use egui::{Event, Key, Modifiers, PointerButton};
use monun_engine::Session;
use monun_engine::core::CellRef;
use monun_ui_egui::{SheetApp, grid::Geo};

fn harness() -> egui_kittest::Harness<'static, SheetApp> {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        crate::common::app(s),
    );
    h.run_steps(4);
    h
}

fn start_rename(h: &mut egui_kittest::Harness<'static, SheetApp>, new_name: &str) {
    h.state_mut().grid.renaming_tab = Some(0);
    h.state_mut().grid.rename_text = new_name.to_string();
    h.run_steps(3);
}

fn sheet_name(h: &egui_kittest::Harness<'static, SheetApp>) -> String {
    h.state().session.active().unwrap().wb.sheets[0].name.clone()
}

#[test]
fn enter_saves_the_sheet_name_and_closes_the_field() {
    let mut h = harness();
    start_rename(&mut h, "Budget");
    h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().grid.renaming_tab, None, "Enter must close the rename field");
    assert_eq!(sheet_name(&h), "Budget");
}

#[test]
fn clicking_a_cell_saves_the_sheet_name_and_closes_the_field() {
    let mut h = harness();
    start_rename(&mut h, "Ledger");
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    let pos = geo.cell_rect(sheet, CellRef::parse("B2").unwrap()).center();
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(3);
    assert_eq!(h.state().grid.renaming_tab, None, "clicking elsewhere must close the rename field");
    assert_eq!(sheet_name(&h), "Ledger");
}

#[test]
fn the_name_field_fits_in_its_tab_and_grows_with_the_name() {
    use egui_kittest::kittest::Queryable as _;
    let mut h = harness();
    start_rename(&mut h, "Q");
    let field = h.get_by_label("Sheet Name").rect();
    // The tabs show in a 30 px band in the middle of the strip, level with its arrows; the field keeps
    // clear of the band's edges, so its frame shows all round.
    let middle = h.get_by_label("Previous sheet").rect().center().y;
    assert!(field.height() >= 19.0, "the whole field shows: {field:?}");
    assert!(field.top() > middle - 15.0 && field.bottom() <= middle + 14.0, "inside the band around {middle}: {field:?}");
    h.state_mut().grid.rename_text = "A much longer sheet name".into();
    h.run_steps(3);
    let longer = h.get_by_label("Sheet Name").rect();
    assert!(longer.width() > field.width() + 100.0, "{field:?} → {longer:?}");
}
