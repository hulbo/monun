//! Korean business forms built in code (original layouts): 견적서 (quote), 거래명세서 (transaction
//! statement) and 청구서 (invoice). One A4 page each, with the supplier's details, an item table
//! that works out the supply value and 10% VAT, and the total written out in Hangul.

use serde_json::{Value as Json, json};

use super::run;
use crate::Session;

/// What differs between the three forms.
struct Form {
    title: &'static str,
    /// Under the title (`(공급받는자 보관용)`), or nothing.
    subtitle: &'static str,
    /// The date label and the lines of the left block under it: (label, value).
    date_label: &'static str,
    left: [(&'static str, &'static str); 3],
    sentence: &'static str,
    total_label: &'static str,
    /// The first column of the item table: `번호` counts the rows, `월/일` takes dates.
    first_column: &'static str,
    /// Rows under the table: (label, value), each across the page.
    footer: [(&'static str, &'static str); 3],
}

const QUOTE: Form = Form {
    title: "견   적   서",
    subtitle: "",
    date_label: "견 적 일",
    left: [("수    신", "거래처명 귀하"), ("참    조", "담당자명"), ("유효기간", "견적일로부터 30일")],
    sentence: "아래와 같이 견적합니다.",
    total_label: "합계금액\n(공급가액+세액)",
    first_column: "번호",
    footer: [
        ("특기사항", "1. 위 금액은 부가가치세를 포함한 금액입니다."),
        ("", "2. 납기: 계약 후 협의"),
        ("", "3. 결제 조건: 납품 후 30일 이내 현금 결제"),
    ],
};

const STATEMENT: Form = Form {
    title: "거  래  명  세  서",
    subtitle: "(공급받는자 보관용)",
    date_label: "거 래 일",
    left: [("공급받는자", "거래처명 귀하"), ("등록번호", "000-00-00000"), ("주    소", "거래처 주소")],
    sentence: "아래와 같이 거래합니다.",
    total_label: "합계금액\n(공급가액+세액)",
    first_column: "월/일",
    footer: [("비    고", ""), ("인 수 자", "(인)"), ("공 급 자", "(인)")],
};

const INVOICE: Form = Form {
    title: "청   구   서",
    subtitle: "",
    date_label: "청 구 일",
    left: [("수    신", "거래처명 귀하"), ("참    조", "담당자명"), ("건    명", "용역 대금 청구")],
    sentence: "아래와 같이 청구합니다.",
    total_label: "청구금액\n(공급가액+세액)",
    first_column: "번호",
    footer: [("입금계좌", "은행명 000-000000-00-000 (예금주: 상호)"), ("입금기한", ""), ("비    고", "입금 후 연락 부탁드립니다.")],
};

/// The form `name` names (`quote`, `statement`, `invoice`).
pub(super) fn build(s: &mut Session, name: &str) -> bool {
    let form = match name {
        "quote" => &QUOTE,
        "statement" => &STATEMENT,
        "invoice" => &INVOICE,
        _ => return false,
    };
    let mut steps = Steps { s, ok: true };
    steps.layout(form);
    steps.ok
}

/// The item table: header row, the item rows and the total rows.
const HEAD: u32 = 12;
const FIRST: u32 = 13;
const LAST: u32 = 24;
const SUM: u32 = 25;
const TOTAL: u32 = 26;
const NOTES: u32 = 28;

const LABEL_FILL: &str = "#EAF1EA";
const HEAD_FILL: &str = "#D7E6D7";

struct Steps<'a> {
    s: &'a mut Session,
    ok: bool,
}

impl Steps<'_> {
    fn run(&mut self, id: &str, p: Json) {
        self.ok &= run(self.s, id, p);
    }
    fn set(&mut self, cell: &str, input: &str) {
        self.run("cell.set", json!({"cell": cell, "input": input}));
    }
    fn merge(&mut self, range: &str) {
        self.run("home.mergeCells", json!({"range": range}));
    }
    fn label(&mut self, range: &str) {
        self.run("home.fillColor", json!({"range": range, "color": LABEL_FILL}));
        self.run("home.alignCenter", json!({"range": range}));
    }

    fn layout(&mut self, f: &Form) {
        // Columns: 번호, 품목, 규격, 수량, 단가, 공급가액, 세액, 비고 (pixels).
        for (cols, width) in [("A:A", 64), ("B:B", 190), ("C:C", 84), ("D:D", 60), ("E:E", 96), ("F:F", 108), ("G:G", 92), ("H:H", 96)] {
            self.run("home.columnWidth", json!({"cols": cols, "width": width}));
        }
        self.run("view.gridlines", json!({"on": false}));
        self.run("home.alignMiddle", json!({"range": "A1:H40"}));

        // Title.
        self.set("A2", f.title);
        self.merge("A2:H2");
        self.run("home.fontSize", json!({"range": "A2", "size": 22}));
        self.run("home.bold", json!({"range": "A2", "on": true}));
        self.run("home.alignCenter", json!({"range": "A2"}));
        self.run("home.rowHeight", json!({"rows": "2:2", "points": 36}));
        self.run("home.borders", json!({"range": "A2:H2", "preset": "doubleBottom"}));
        if !f.subtitle.is_empty() {
            self.set("A3", f.subtitle);
            self.merge("A3:H3");
            self.run("home.alignCenter", json!({"range": "A3"}));
        }

        // Left block: the date, then the customer.
        self.set("A4", f.date_label);
        self.set("B4", "=TODAY()");
        self.run("home.numberFormat", json!({"range": "B4", "code": "yyyy\"년\" m\"월\" d\"일\""}));
        self.merge("B4:D4");
        self.run("home.alignLeft", json!({"range": "B4"}));
        for (i, (label, value)) in f.left.iter().enumerate() {
            let r = 5 + i;
            self.set(&format!("A{r}"), label);
            self.set(&format!("B{r}"), value);
            self.merge(&format!("B{r}:D{r}"));
        }
        self.label("A4:A7");
        self.run("home.borders", json!({"range": "A4:D7", "preset": "all"}));
        self.set("A8", f.sentence);
        self.merge("A8:D8");

        // Right block: the supplier.
        self.set("E4", "등록번호");
        self.set("F4", "1234567890");
        self.merge("F4:H4");
        self.run("home.numberFormat", json!({"range": "F4", "code": "000-00-00000"}));
        self.run("home.alignCenter", json!({"range": "F4"}));
        self.set("E5", "상    호");
        self.set("F5", "상호명");
        self.set("G5", "대 표 자");
        self.set("H5", "성명 (인)");
        self.set("E6", "주    소");
        self.set("F6", "사업장 주소");
        self.merge("F6:H6");
        self.set("E7", "업    태");
        self.set("F7", "서비스");
        self.set("G7", "종    목");
        self.set("H7", "소프트웨어 개발");
        self.set("E8", "전화번호");
        self.set("F8", "010-0000-0000");
        self.merge("F8:H8");
        self.label("E4:E8");
        self.label("G5");
        self.label("G7");
        self.run("home.borders", json!({"range": "E4:H8", "preset": "all"}));
        self.run("home.fontSize", json!({"range": "A4:H8", "size": 9}));

        // The total in words and in figures.
        self.set("A10", f.total_label);
        self.merge("A10:B10");
        self.run("home.wrapText", json!({"range": "A10", "on": true}));
        self.label("A10");
        self.set("C10", &format!("=\"일금 \"&NUMBERSTRING(F{TOTAL},1)&\"원정\""));
        self.merge("C10:F10");
        self.run("home.alignCenter", json!({"range": "C10"}));
        self.set("G10", &format!("=F{TOTAL}"));
        self.merge("G10:H10");
        self.run("home.numberFormat", json!({"range": "G10", "code": "\"(₩\"#,##0\")\""}));
        self.run("home.alignCenter", json!({"range": "G10"}));
        self.run("home.bold", json!({"range": "A10:H10", "on": true}));
        self.run("home.fontSize", json!({"range": "C10:H10", "size": 12}));
        self.run("home.rowHeight", json!({"rows": "10:10", "points": 32}));
        self.run("home.borders", json!({"range": "A10:H10", "preset": "all"}));
        self.run("home.borders", json!({"range": "A10:H10", "preset": "thickOutside"}));

        self.table(f);
        self.footer(f);
        self.run(
            "pageLayout.pageSetup",
            json!({
                "paper": "A4",
                "orientation": "portrait",
                "margins": {"left": 0.5, "right": 0.5, "top": 0.6, "bottom": 0.6, "header": 0.3, "footer": 0.3},
                "fitWidth": 1,
                "fitHeight": 1,
                "centerH": true,
                "printArea": format!("A1:H{}", NOTES + 3),
            }),
        );
        self.run("selection.set", json!({"range": format!("B{FIRST}")}));
    }

    fn table(&mut self, f: &Form) {
        let head = [f.first_column, "품    목", "규  격", "수량", "단  가", "공급가액", "세  액", "비  고"];
        self.run("range.setValues", json!({"range": format!("A{HEAD}"), "values": [head]}));
        let header = format!("A{HEAD}:H{HEAD}");
        self.run("home.fillColor", json!({"range": header, "color": HEAD_FILL}));
        self.run("home.bold", json!({"range": header, "on": true}));
        self.run("home.alignCenter", json!({"range": header}));
        self.run("home.rowHeight", json!({"rows": format!("{HEAD}:{TOTAL}"), "points": 21}));
        for r in FIRST..=LAST {
            if f.first_column == "번호" {
                self.set(&format!("A{r}"), &format!("=IF(B{r}=\"\",\"\",ROW()-{HEAD})"));
            }
            self.set(&format!("F{r}"), &format!("=IF(OR(D{r}=\"\",E{r}=\"\"),\"\",D{r}*E{r})"));
            // VAT is cut to the won, as tax invoices round it.
            self.set(&format!("G{r}"), &format!("=IF(F{r}=\"\",\"\",ROUNDDOWN(F{r}*0.1,0))"));
        }
        // Example lines, to be typed over.
        let first = if f.first_column == "번호" { Json::Null } else { json!("11/1") };
        self.run(
            "range.setValues",
            json!({"range": format!("B{FIRST}"), "values": [["홈페이지 유지보수", "월", 1, 1_000_000], ["서버 호스팅", "월", 1, 200_000]]}),
        );
        if !first.is_null() {
            self.run("range.setValues", json!({"range": format!("A{FIRST}"), "values": [[first], ["11/1"]]}));
            self.run("home.numberFormat", json!({"range": format!("A{FIRST}:A{LAST}"), "code": "m/d"}));
        }
        self.run("home.alignCenter", json!({"range": format!("A{FIRST}:A{LAST}")}));
        self.run("home.alignCenter", json!({"range": format!("C{FIRST}:D{LAST}")}));
        self.run("home.numberFormat", json!({"range": format!("D{FIRST}:G{TOTAL}"), "code": "#,##0"}));

        // 합계 and 총액.
        self.set(&format!("A{SUM}"), "합    계");
        self.merge(&format!("A{SUM}:E{SUM}"));
        self.label(&format!("A{SUM}"));
        self.set(&format!("F{SUM}"), &format!("=SUM(F{FIRST}:F{LAST})"));
        self.set(&format!("G{SUM}"), &format!("=SUM(G{FIRST}:G{LAST})"));
        self.set(&format!("A{TOTAL}"), "총    액 (공급가액 + 세액)");
        self.merge(&format!("A{TOTAL}:E{TOTAL}"));
        self.label(&format!("A{TOTAL}"));
        self.set(&format!("F{TOTAL}"), &format!("=F{SUM}+G{SUM}"));
        self.merge(&format!("F{TOTAL}:G{TOTAL}"));
        self.run("home.alignRight", json!({"range": format!("F{TOTAL}")}));
        self.run("home.bold", json!({"range": format!("A{SUM}:H{TOTAL}"), "on": true}));
        let table = format!("A{HEAD}:H{TOTAL}");
        self.run("home.borders", json!({"range": table, "preset": "all"}));
        self.run("home.borders", json!({"range": table, "preset": "thickOutside"}));
    }

    fn footer(&mut self, f: &Form) {
        for (i, (label, value)) in f.footer.iter().enumerate() {
            let r = NOTES + i as u32;
            if !label.is_empty() {
                self.set(&format!("A{r}"), label);
                self.label(&format!("A{r}"));
            }
            if !value.is_empty() {
                self.set(&format!("B{r}"), value);
            }
            self.merge(&format!("B{r}:H{r}"));
            // A signature line: the seal mark at its end.
            if *value == "(인)" {
                self.run("home.alignRight", json!({"range": format!("B{r}")}));
            }
        }
        // The invoice is due two weeks after it is issued.
        if f.footer.iter().any(|(label, _)| *label == "입금기한") {
            self.set(&format!("B{}", NOTES + 1), "=B4+14");
            self.run("home.numberFormat", json!({"range": format!("B{}", NOTES + 1), "code": "yyyy\"년\" m\"월\" d\"일\""}));
            self.run("home.alignLeft", json!({"range": format!("B{}", NOTES + 1)}));
        }
        self.run("home.borders", json!({"range": format!("A{NOTES}:H{}", NOTES + 2), "preset": "all"}));
        self.run("home.rowHeight", json!({"rows": format!("{NOTES}:{}", NOTES + 2), "points": 21}));
    }
}
