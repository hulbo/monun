//! Grouped shapes (`xdr:grpSp`) are read as separate shapes where the group draws them, nested
//! groups included, so they show and are saved (ungrouped) instead of being dropped.

use monun_model::{ShapeKind, Sheet};

use super::minimal;
use crate::{read_xlsx, write_xlsx};

/// A group over B2:F7 whose own coordinates start at (1000000, 500000) and span 4000000 × 2000000:
/// a rectangle in its top-left quarter and, in its bottom-right quarter, a nested group holding a
/// connector across its whole box.
const GROUP: &str = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
<xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id="2" name="Group 1"/><xdr:cNvGrpSpPr/></xdr:nvGrpSpPr>
<xdr:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4000000" cy="2000000"/><a:chOff x="1000000" y="500000"/><a:chExt cx="4000000" cy="2000000"/></a:xfrm></xdr:grpSpPr>
<xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="3" name="Rectangle 2"/><xdr:cNvSpPr/></xdr:nvSpPr>
<xdr:spPr><a:xfrm><a:off x="1000000" y="500000"/><a:ext cx="2000000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr>
<xdr:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>메뉴</a:t></a:r></a:p></xdr:txBody></xdr:sp>
<xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id="4" name="Group 3"/><xdr:cNvGrpSpPr/></xdr:nvGrpSpPr>
<xdr:grpSpPr><a:xfrm><a:off x="3000000" y="1500000"/><a:ext cx="2000000" cy="1000000"/><a:chOff x="0" y="0"/><a:chExt cx="4000" cy="2000"/></a:xfrm></xdr:grpSpPr>
<xdr:cxnSp macro=""><xdr:nvCxnSpPr><xdr:cNvPr id="5" name="Connector 4"/><xdr:cNvCxnSpPr/></xdr:nvCxnSpPr>
<xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4000" cy="2000"/></a:xfrm><a:prstGeom prst="line"><a:avLst/></a:prstGeom></xdr:spPr></xdr:cxnSp>
</xdr:grpSp></xdr:grpSp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#;

fn read(drawing: &str) -> (monun_model::Workbook, Vec<String>) {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#;
    let bytes = minimal(
        r#"<sheetData/><drawing r:id="rId1"/>"#,
        &[("xl/worksheets/_rels/sheet1.xml.rels", rels), ("xl/drawings/drawing1.xml", drawing)],
        "",
        "",
    );
    let (wb, report) = read_xlsx(&bytes).unwrap();
    (wb, report.warnings)
}

/// A shape's box on the sheet: left, top, width, height in pixels.
fn rect(sheet: &Sheet, i: usize) -> [f64; 4] {
    let a = sheet.shapes[i].anchor;
    [sheet.col_left(a.cell.col) + f64::from(a.dx), sheet.row_top(a.cell.row) + f64::from(a.dy), f64::from(a.width), f64::from(a.height)]
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.5)
}

#[test]
fn grouped_shapes_are_read_where_the_group_draws_them() {
    let (wb, warnings) = read(GROUP);
    let sheet = wb.sheet(0).unwrap();
    assert_eq!(sheet.shapes.len(), 2, "{warnings:?}");
    let (x, y) = (sheet.col_left(1), sheet.row_top(1));
    let (w, h) = (sheet.col_left(5) - x, sheet.row_top(6) - y);
    assert_eq!(sheet.shapes[0].kind, ShapeKind::Rectangle);
    assert_eq!(sheet.shapes[0].text, "메뉴");
    assert!(close(rect(sheet, 0), [x, y, w / 2.0, h / 2.0]), "{:?}", rect(sheet, 0));
    assert_eq!(sheet.shapes[1].kind, ShapeKind::Line);
    assert!(close(rect(sheet, 1), [x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0]), "{:?}", rect(sheet, 1));
    assert!(warnings.iter().any(|w| w.contains("ungrouped")), "{warnings:?}");

    // Saved, they stay: as two shapes of their own.
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let sheet = back.sheet(0).unwrap();
    assert_eq!(sheet.shapes.len(), 2);
    assert_eq!(sheet.shapes[0].text, "메뉴");
}
