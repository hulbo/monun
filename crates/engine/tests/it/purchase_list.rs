//! File › New's template, 구매목록: what was bought (대상) and for how much (금액), with the total on
//! the header row, in the user's new-workbook defaults.

use monun_engine::Session;
use serde_json::json;

#[test]
fn the_purchase_list_totals_on_its_header_row_in_the_new_workbook_font() {
    let mut s = Session::new();
    s.prefs.default_font = "굴림체".into();
    s.prefs.default_column_width = 130.0;
    s.prefs.default_paper = "A4".into();
    s.execute("file.new", json!({"sample": "purchases"})).unwrap();
    let d = s.active().unwrap();
    assert_eq!(d.display_title(), "구매목록");
    let sheet = d.wb.active().unwrap();
    assert_eq!(sheet.name, "구매목록");
    assert_eq!(d.wb.styles.get(monun_engine::model::StyleId::DEFAULT).font.name, "굴림체");
    assert_eq!((sheet.default_col_width, sheet.print.paper.as_str()), (130.0, "A4"));
    assert!(!d.is_dirty(), "a template opens unchanged, like a blank workbook");
    // The sample's two lines: 30,000 in all, on the header row.
    s.execute("range.setValues", json!({"range": "A2", "values": [["AAA", 10000], ["BBB", 20000]]})).unwrap();
    let get = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap();
    assert_eq!(get(&mut s, "C1")["value"], "총합계:");
    assert_eq!(get(&mut s, "D1")["text"], "30,000");
    assert_eq!(get(&mut s, "B3")["text"], "20,000");
    // A line added far down still counts.
    s.execute("cell.set", json!({"cell": "B500", "input": "5000"})).unwrap();
    assert_eq!(get(&mut s, "D1")["value"], 35_000.0);
}
