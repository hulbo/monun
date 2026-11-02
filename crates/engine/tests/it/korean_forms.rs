//! The Korean business forms of File › New: 견적서, 거래명세서 and 청구서. Each works out the
//! supply value, the 10% VAT (cut to the won) and the total, in figures and in Hangul, and prints
//! on one A4 page.

use monun_engine::Session;
use serde_json::{Value as Json, json};

fn get(s: &mut Session, cell: &str) -> Json {
    s.execute("cell.get", json!({"cell": cell})).unwrap()
}

#[test]
fn the_forms_add_up_and_print_on_a4() {
    for (name, title) in [("quote", "견적서"), ("statement", "거래명세서"), ("invoice", "청구서")] {
        let mut s = Session::new();
        s.execute("file.new", json!({"sample": name})).unwrap();
        assert_eq!(s.active().unwrap().display_title(), title);
        assert_eq!(get(&mut s, "F25")["value"], 1_200_000.0, "{name}: supply value");
        assert_eq!(get(&mut s, "G25")["value"], 120_000.0, "{name}: VAT");
        assert_eq!(get(&mut s, "F26")["value"], 1_320_000.0, "{name}: total");
        assert_eq!(get(&mut s, "C10")["value"], "일금 일백삼십이만원정", "{name}");
        assert_eq!(get(&mut s, "G10")["text"], "(₩1,320,000)", "{name}");
        assert_eq!(get(&mut s, "F4")["text"], "123-45-67890", "{name}");
        let print = s.active().unwrap().wb.sheet(0).unwrap().print.clone();
        assert_eq!((print.paper.as_str(), print.fit_to), ("A4", Some((1, 1))), "{name}");
        // A line typed in counts: 3 × 12,345 is 37,035 and its VAT is cut to 3,703.
        s.execute("range.setValues", json!({"range": "B15", "values": [["추가 작업", "식", 3, 12345]]})).unwrap();
        assert_eq!(get(&mut s, "G15")["value"], 3703.0, "{name}");
        assert_eq!(get(&mut s, "F26")["value"], 1_320_000.0 + 37_035.0 + 3703.0, "{name}");
    }
}
