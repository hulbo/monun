//! `formulas.koreanHolidays` writes Korean public holidays into the workbook as a 공휴일 sheet and
//! name, which WORKDAY and NETWORKDAYS take, here and in Excel (the file holds them).

use monun_engine::Session;
use serde_json::json;

fn value(s: &mut Session, cell: &str) -> serde_json::Value {
    s.execute("cell.get", json!({"cell": cell})).unwrap()["value"].clone()
}

#[test]
fn working_days_skip_korean_holidays() {
    let mut s = Session::new();
    s.new_workbook();
    let r = s.execute("formulas.koreanHolidays", json!({})).unwrap();
    assert_eq!((r["sheet"].as_str(), r["years"].clone()), (Some("공휴일"), json!([2025, 2027])));
    let wb = &s.active().unwrap().wb;
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.active_sheet(), 1, "the 공휴일 sheet is shown");
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    // 2026-09-21 to 10-09: 15 weekdays, less 추석 (24, 25) and 개천절's substitute (10-05) and 한글날.
    s.execute("cell.set", json!({"cell": "A1", "input": "=NETWORKDAYS(DATE(2026,9,21),DATE(2026,10,9),공휴일)"})).unwrap();
    assert_eq!(value(&mut s, "A1"), 11.0);
    // The working day after Friday 2027-02-05 is Wednesday the 10th: 설날 and its substitute come between.
    s.execute("cell.set", json!({"cell": "A2", "input": "=WORKDAY(DATE(2027,2,5),1,공휴일)=DATE(2027,2,10)"})).unwrap();
    assert_eq!(value(&mut s, "A2"), true);
    // 노동절 counts from 2027 on, 제헌절 from 2026, neither in 2025.
    s.execute("cell.set", json!({"cell": "A3", "input": "=COUNTIF(공휴일,DATE(2027,5,1))+COUNTIF(공휴일,DATE(2026,7,17))+COUNTIF(공휴일,DATE(2025,7,17))+COUNTIF(공휴일,DATE(2026,5,1))"})).unwrap();
    assert_eq!(value(&mut s, "A3"), 2.0);
    // Asked again, it leaves the user's sheet and name alone.
    assert!(s.execute("formulas.koreanHolidays", json!({})).is_err());
    // Saved and opened again, the name and its dates are in the file.
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut again = Session::new();
    again.execute("file.open", json!({"name": "holidays.xlsx", "base64": saved["base64"]})).unwrap();
    again.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    assert_eq!(value(&mut again, "A1"), 11.0);
}

#[test]
fn the_table_is_in_date_order_and_every_date_exists() {
    let (first, last) = monun_engine::holidays::YEARS;
    let mut previous = (0, 0, 0);
    for &(y, m, d, name) in monun_engine::holidays::KOREAN {
        assert!((first..=last).contains(&y), "{name}");
        assert!((y, m, d) > previous, "{y}-{m}-{d} {name}");
        assert!(monun_engine::core::date::serial_from_ymd(monun_engine::core::DateSystem::D1900, y.into(), m.into(), d.into()).is_some());
        previous = (y, m, d);
    }
}
