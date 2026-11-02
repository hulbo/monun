//! New workbooks and inserted sheets print on the default paper (`defaultPaper`, A4 in the
//! desktop app as on a Korean printer); a file keeps the paper it was saved with.

use monun_engine::Session;
use serde_json::json;

fn paper(s: &Session, sheet: usize) -> String {
    s.active().unwrap().wb.sheet(sheet).unwrap().print.paper.clone()
}

#[test]
fn new_workbooks_and_sheets_take_the_default_paper() {
    let mut s = Session::new();
    s.new_workbook();
    assert_eq!(paper(&s, 0), "Letter", "the model's own default");
    s.prefs.default_paper = "A4".into();
    s.new_workbook();
    assert_eq!(paper(&s, 0), "A4");
    s.execute("home.insertSheet", json!({})).unwrap();
    let inserted = s.active().unwrap().wb.active_sheet();
    assert_eq!(paper(&s, inserted), "A4");
    // A paper Monun doesn't know is ignored.
    s.prefs.default_paper = "Napkin".into();
    s.new_workbook();
    assert_eq!(paper(&s, 0), "Letter");
}
