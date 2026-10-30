//! Form dialogs fit their content: OK and Cancel sit right under the last field, not halfway down
//! a window stretched to the screen's height.

use egui_kittest::kittest::Queryable as _;
use monun_engine::Session;
use monun_ui_egui::SheetApp;
use serde_json::json;

#[test]
/// OK sits between 0 and 40 points below the last dialog option.
fn ok_and_cancel_sit_under_the_last_field() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 1000.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        crate::common::app(s),
    );
    h.state_mut().open_dialog("cfQuick", json!({"type": "cellIs", "operator": "greaterThan", "title": "Greater Than…"}));
    h.run_steps(4);
    let last = h.get_by_label("Red Border").rect();
    let ok = h.get_by_label_contains("OK").rect();
    let gap = ok.top() - last.bottom();
    assert!((0.0..40.0).contains(&gap), "OK is {gap} points under the last option");
}

#[test]
/// The closing row sits centred under the title, OK before Cancel, each button at least 84 × 26.
fn the_closing_row_is_centred_with_ok_first() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 1000.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        crate::common::app(s),
    );
    h.state_mut().open_dialog("cfQuick", json!({"type": "cellIs", "operator": "greaterThan", "title": "Greater Than…"}));
    h.run_steps(4);
    let title = h.get_by_label("Greater Than…").rect();
    let ok = crate::common::dialog_button(&h, "OK").rect();
    let cancel = crate::common::dialog_button(&h, "Cancel").rect();
    assert!(ok.right() <= cancel.left(), "OK comes first, as in Excel for Windows");
    for b in [ok, cancel] {
        assert!(b.width() >= 84.0 && b.height() >= 26.0, "a dialog button is at least 84 × 26: {b:?}");
    }
    let middle = (ok.left() + cancel.right()) / 2.0;
    assert!((middle - title.center().x).abs() < 4.0, "the row's middle {middle} is under the title's {}", title.center().x);
}
