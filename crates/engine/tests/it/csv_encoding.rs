//! A CSV or text file is written back in the encoding it was read in, so a code page 949 file
//! (Excel's plain CSV on Korean Windows) stays one. Any other workbook saves as CSV UTF-8, and an
//! export can ask for either.

use monun_engine::Session;
use monun_model::TextEncoding;
use serde_json::{Value as Json, json};

const TEXT: &str = "거래처,금액\r\n모눈상사,100\r\n";

fn cp949(text: &str) -> Vec<u8> {
    monun_xlsx::encode_text(text.as_bytes().to_vec(), TextEncoding::Cp949)
}

fn saved(s: &mut Session, params: Json) -> Vec<u8> {
    let r = s.execute("file.saveBytes", params).unwrap();
    monun_engine::io::base64_decode(r["base64"].as_str().unwrap()).unwrap()
}

#[test]
fn a_code_page_949_csv_reads_as_hangul_and_saves_back_in_it() {
    let mut s = Session::new();
    s.execute("file.open", json!({"name": "거래처.csv", "base64": monun_engine::io::base64_encode(&cp949(TEXT))})).unwrap();
    assert_eq!(s.execute("cell.get", json!({"cell": "A2"})).unwrap()["value"], "모눈상사");
    s.execute("cell.set", json!({"cell": "B2", "input": "200"})).unwrap();
    let edited = TEXT.replace("100", "200");
    assert_eq!(saved(&mut s, json!({"format": "csv"})), cp949(&edited));
    // Save As "CSV UTF-8" asks for UTF-8, with the byte order mark Excel reads it by.
    assert_eq!(saved(&mut s, json!({"format": "csv", "encoding": "utf-8"})), [b"\xEF\xBB\xBF".as_slice(), edited.as_bytes()].concat());
}

#[test]
fn a_new_workbook_saves_as_csv_utf8_unless_asked_for_code_page_949() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "가"})).unwrap();
    assert_eq!(saved(&mut s, json!({"format": "csv"})), "\u{feff}가\r\n".as_bytes());
    assert_eq!(saved(&mut s, json!({"format": "csv", "encoding": "cp949"})), [0xB0, 0xA1, b'\r', b'\n']);
    let dir = std::env::temp_dir().join(format!("monun-csv-encoding-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plain.csv");
    s.execute("file.exportCsv", json!({"path": path.to_str().unwrap(), "encoding": "cp949"})).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), [0xB0, 0xA1, b'\r', b'\n']);
    let err = s.execute("file.exportCsv", json!({"path": path.to_str().unwrap(), "encoding": "latin1"})).unwrap_err().to_string();
    assert!(err.contains("utf-8 or cp949"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}
