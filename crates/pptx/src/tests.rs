//! A small presentation made here, part by part as PowerPoint writes them (theme, master, layout,
//! slide), turned into its page.

use std::io::{Cursor, Write};

use crate::{Labels, NOT_PPTX, convert};

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn rels(list: &[(&str, &str, &str)]) -> String {
    let body: String = list.iter().map(|(id, kind, target)| format!(r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#)).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn theme() -> String {
    format!(
        r#"<a:theme {NS} name="T"><a:themeElements>
<a:clrScheme name="C"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1>
<a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:hlink><a:srgbClr val="0563C1"/></a:hlink></a:clrScheme>
<a:fontScheme name="F"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:font script="Hang" typeface="맑은 고딕"/></a:majorFont>
<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:font script="Hang" typeface="맑은 고딕"/></a:minorFont></a:fontScheme>
<a:fmtScheme name="S"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"><a:tint val="50000"/></a:schemeClr></a:solidFill></a:fillStyleLst>
<a:lnStyleLst><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>
<a:effectStyleLst/><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme>
</a:themeElements></a:theme>"#
    )
}

fn master() -> String {
    format!(
        r#"<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm></p:spPr></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="9" name="Logo bar"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="6731000"/><a:ext cx="12192000" cy="127000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent2"/></a:solidFill></p:spPr></p:sp>
</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" hlink="hlink"/>
<p:txStyles><p:titleStyle><a:lvl1pPr algn="ctr"><a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/><a:ea typeface="+mj-ea"/></a:defRPr></a:lvl1pPr></p:titleStyle>
<p:bodyStyle><a:lvl1pPr marL="228600" indent="-228600"><a:buFont typeface="Arial"/><a:buChar char="•"/><a:defRPr sz="2800"/></a:lvl1pPr><a:lvl2pPr marL="685800" indent="-228600"><a:buChar char="–"/><a:defRPr sz="2400"/></a:lvl2pPr></p:bodyStyle>
<p:otherStyle/></p:txStyles></p:sldMaster>"#
    )
}

fn layout() -> String {
    format!(
        r#"<p:sldLayout {NS}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="1270000" y="635000"/><a:ext cx="6350000" cy="1270000"/></a:xfrm></p:spPr></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="1270000" y="2540000"/><a:ext cx="6350000" cy="2540000"/></a:xfrm></p:spPr></p:sp>
</p:spTree></p:cSld></p:sldLayout>"#
    )
}

fn slide() -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>분기 보고 &amp; 계획</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body 2"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr><a:normAutofit fontScale="50000"/></a:bodyPr>
<a:p><a:r><a:t>첫째</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>둘째</a:t></a:r></a:p>
<a:p><a:pPr><a:buAutoNum type="arabicPeriod"/></a:pPr><a:r><a:t>하나</a:t></a:r></a:p><a:p><a:pPr><a:buAutoNum type="arabicPeriod"/></a:pPr><a:r><a:t>둘</a:t></a:r><a:fld type="slidenum"><a:t>‹#›</a:t></a:fld></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="4" name="Box"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot="5400000"><a:off x="127000" y="254000"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom></p:spPr>
<p:style><a:lnRef idx="1"><a:schemeClr val="accent1"><a:lumMod val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></p:style>
<p:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="ko-KR" b="1"/><a:t>상자</a:t></a:r></a:p></p:txBody></p:sp>
<p:pic><p:nvPicPr><p:cNvPr id="5" name="Photo"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:srcRect l="25000"/><a:stretch/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm><a:prstGeom prst="ellipse"/></p:spPr></p:pic>
<p:pic><p:nvPicPr><p:cNvPr id="6" name="Vector"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId3"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="127000" cy="127000"/></a:xfrm></p:spPr></p:pic>
<p:grpSp><p:nvGrpSpPr><p:cNvPr id="7" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="1270000" y="1270000"/><a:ext cx="2540000" cy="2540000"/><a:chOff x="0" y="0"/><a:chExt cx="1270000" cy="1270000"/></a:xfrm></p:grpSpPr>
<p:sp><p:nvSpPr><p:cNvPr id="8" name="Inner"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="127000" y="127000"/><a:ext cx="635000" cy="635000"/></a:xfrm><a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="FF0000"><a:alpha val="50000"/></a:srgbClr></a:solidFill></p:spPr></p:sp></p:grpSp>
<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="10" name="Arrow"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="0"/></a:xfrm><a:prstGeom prst="straightConnector1"/><a:ln w="25400"><a:solidFill><a:srgbClr val="00B050"/></a:solidFill><a:prstDash val="dash"/><a:tailEnd type="triangle"/></a:ln></p:spPr></p:cxnSp>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="11" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="635000" y="5080000"/><a:ext cx="2540000" cy="508000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w="1270000"/><a:gridCol w="1270000"/></a:tblGrid>
<a:tr h="254000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>항목</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>값</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr>
<a:tr h="254000"><a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:p><a:r><a:t>합쳐진 칸</a:t></a:r></a:p></a:txBody><a:tcPr anchor="ctr"><a:solidFill><a:srgbClr val="FFFF00"/></a:solidFill></a:tcPr></a:tc><a:tc hMerge="1"><a:txBody><a:bodyPr/><a:p><a:endParaRPr/></a:p></a:txBody><a:tcPr/></a:tc></a:tr>
</a:tbl></a:graphicData></a:graphic></p:graphicFrame>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="12" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="6350000" y="5080000"/><a:ext cx="2540000" cy="1270000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId4"/></a:graphicData></a:graphic></p:graphicFrame>
</p:spTree></p:cSld></p:sld>"#
    )
}

fn chart() -> String {
    r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>월별 매출</a:t></a:r></a:p></c:rich></c:tx></c:title></c:chart></c:chartSpace>"#.to_string()
}

fn package(hidden: bool) -> Vec<u8> {
    let presentation = format!(
        r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:defaultTextStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:defaultTextStyle></p:presentation>"#
    );
    let second = format!(
        r#"<p:sld {NS}{}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#,
        if hidden { r#" show="0""# } else { "" }
    );
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("_rels/.rels", rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]).into_bytes()),
        ("ppt/presentation.xml", presentation.into_bytes()),
        ("ppt/_rels/presentation.xml.rels", rels(&[("rId2", "slide", "slides/slide1.xml"), ("rId3", "slide", "slides/slide2.xml")]).into_bytes()),
        ("ppt/slides/slide1.xml", slide().into_bytes()),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[
                ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
                ("rId2", "image", "../media/image1.png"),
                ("rId3", "image", "../media/image2.emf"),
                ("rId4", "chart", "../charts/chart1.xml"),
            ])
            .into_bytes(),
        ),
        ("ppt/slides/slide2.xml", second.into_bytes()),
        ("ppt/slides/_rels/slide2.xml.rels", rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]).into_bytes()),
        ("ppt/slideLayouts/slideLayout1.xml", layout().into_bytes()),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]).into_bytes()),
        ("ppt/slideMasters/slideMaster1.xml", master().into_bytes()),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels(&[("rId1", "theme", "../theme/theme1.xml")]).into_bytes()),
        ("ppt/theme/theme1.xml", theme().into_bytes()),
        ("ppt/charts/chart1.xml", chart().into_bytes()),
        ("ppt/media/image1.png", b"\x89PNG not decoded here".to_vec()),
        ("ppt/media/image2.emf", b"EMF".to_vec()),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in parts {
        zip.start_file(name, options).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn page() -> crate::Deck {
    convert(&package(true), "보고서.pptx", &Labels { chart: "차트".into(), hidden: "숨김".into() }).unwrap()
}

/// The `style` of the element right before `marker` in `html`.
fn around<'h>(html: &'h str, marker: &str) -> &'h str {
    let at = html.find(marker).unwrap_or_else(|| panic!("{marker} not in the page"));
    &html[html[..at].rfind("<div class=\"sp\"").unwrap_or(0)..at]
}

#[test]
fn a_deck_is_its_slides_in_order_at_their_size() {
    let deck = page();
    assert_eq!(deck.slides, 2);
    assert_eq!(deck.size, (960.0, 540.0));
    assert!(deck.html.contains(".slide{width:960.00pt;height:540.00pt}"));
    let (one, two) = (deck.html.find("id=\"s1\"").unwrap(), deck.html.find("id=\"s2\"").unwrap());
    assert!(one < two);
    assert!(deck.html.contains("<title>보고서.pptx</title>"));
    // The hidden slide is marked, and its master's background (the theme's, in bg1) is white.
    assert!(
        deck.html.contains("<section class=\"s h\" id=\"s2\"><div class=\"n\">2 · 숨김</div><div class=\"slide\" style=\"background:#ffffff\">"),
        "{}",
        deck.html
    );
}

#[test]
fn placeholders_take_their_place_and_text_from_the_layout_and_master() {
    let html = page().html;
    // The title: where the layout puts it, as large and in the heading font the master gives it.
    let title = around(&html, "분기 보고 &amp; 계획");
    assert!(title.contains("left:100.00pt;top:50.00pt;width:500.00pt;height:100.00pt"), "{title}");
    assert!(title.contains("text-align:center"), "{title}");
    assert!(title.contains("font-size:44.00pt;font-family:&#39;Calibri Light&#39;,&#39;맑은 고딕&#39;"), "{title}");
    assert!(title.contains("color:#000000"), "{title}");
    // The body, shrunk to fit (half size): the master's bullets and indents, level by level.
    let body = around(&html, "첫째");
    assert!(body.contains("left:100.00pt;top:200.00pt"), "{body}");
    assert!(body.contains(">•</span>") && body.contains("font-size:14.00pt"), "{body}");
    assert!(html.contains(">–</span>") && html.contains("margin:0 0 0 54.00pt;text-indent:-18.00pt"), "{html}");
    // Numbered paragraphs count on; a slide number field shows the slide's.
    assert!(html.contains(">1.</span>") && html.contains(">2.</span>"), "{html}");
    assert!(
        html.contains(
            "<span style=\"font-size:14.00pt;font-family:&#39;Calibri&#39;,&#39;맑은 고딕&#39;,&#39;Malgun Gothic&#39;,sans-serif\">1</span></p>"
        ),
        "{html}"
    );
    // The master's own shape is drawn behind the slide's; its title placeholder isn't.
    assert!(html.contains("fill=\"#ed7d31\""), "{html}");
}

#[test]
fn shapes_take_their_style_and_turn() {
    let html = page().html;
    let box_ = around(&html, "상자");
    // Turned a quarter, filled in the accent and outlined darker, its text white and centred.
    assert!(box_.contains("transform:rotate(90.00deg)"), "{box_}");
    // (Accent 1 half as light is PowerPoint's "darker 50%", #203864.)
    assert!(box_.contains("fill=\"#4472c4\"") && box_.contains("stroke=\"#203864\" stroke-width=\"1.33\""), "{box_}");
    assert!(box_.contains("justify-content:center") && box_.contains("color:#ffffff") && box_.contains("font-weight:700"), "{box_}");
    assert!(box_.contains("<path d=\"M11.11 0H"), "a rounded rectangle, a sixth of its height round: {box_}");
    // A connector with a dashed line and an arrowhead.
    assert!(
        html.contains("stroke=\"#00b050\" stroke-width=\"2.67\" stroke-linejoin=\"round\" stroke-dasharray=\"10.67 8.00\" marker-end=\"url(#m"),
        "{html}"
    );
}

#[test]
fn pictures_are_cropped_and_only_those_a_page_shows_kept() {
    let deck = page();
    // The PNG, a quarter cut off its left, in an ellipse; the EMF (no browser draws it) left out.
    assert_eq!(deck.media.len(), 1, "{:?}", deck.media.iter().map(|m| &m.0).collect::<Vec<_>>());
    assert_eq!(deck.media[0].0, "1.png");
    assert!(
        deck.html
            .contains("border-radius:50%\"><img alt=\"\" src=\"media/1.png\" style=\"left:-33.33pt;top:0.00pt;width:133.33pt;height:100.00pt\">"),
        "{}",
        deck.html
    );
    assert!(!deck.html.contains("emf"));
}

#[test]
fn a_group_scales_its_shapes_into_its_box() {
    let html = page().html;
    // The group's own coordinates are half its size: its shape at (10, 10) 50 wide goes at
    // (20, 20) 100 wide in it, see-through red.
    assert!(html.contains("<div class=\"sp\" style=\"left:100.00pt;top:100.00pt;width:200.00pt;height:200.00pt\"><div class=\"sp\" style=\"left:20.00pt;top:20.00pt;width:100.00pt;height:100.00pt\">"), "{html}");
    assert!(html.contains("fill=\"#ff0000\" fill-opacity=\"0.500\""), "{html}");
}

#[test]
fn tables_take_their_style_and_cells_their_own() {
    let html = page().html;
    assert!(html.contains("<table class=\"t\" style=\"left:50.00pt;top:400.00pt;width:200.00pt\"><colgroup><col style=\"width:100.00pt\"><col style=\"width:100.00pt\"></colgroup>"), "{html}");
    // The default style: the first row in the accent, its text white and bold, under a thick rule.
    let header = &html[html.find("<tr").unwrap()..html.find("항목").unwrap()];
    assert!(header.contains("background:#4472c4") && header.contains("border-bottom:3.00pt solid #ffffff"), "{header}");
    assert!(
        html[html.find("항목").unwrap() - 200..].contains("color:#ffffff;font-weight") || html.contains("font-weight:700;color:#ffffff"),
        "{html}"
    );
    // The merged cell spans both columns in its own fill, centred; the cell it covers is gone.
    assert!(html.contains("<td colspan=\"2\" style=\"background:#ffff00;"), "{html}");
    assert!(html.contains("vertical-align:middle"), "{html}");
    assert_eq!(html.matches("<td").count(), 3);
}

#[test]
fn a_chart_shows_as_a_box_with_its_title() {
    let html = page().html;
    assert!(
        html.contains("<div class=\"chart\" style=\"left:500.00pt;top:400.00pt;width:200.00pt;height:100.00pt\">차트: 월별 매출</div>"),
        "{html}"
    );
}

#[test]
fn not_a_pptx_is_said_so() {
    assert_eq!(convert(b"\xD0\xCF\x11\xE0 an old .ppt", "x.ppt", &Labels::default()).unwrap_err(), NOT_PPTX);
}

#[test]
fn numbers_count_in_their_scheme() {
    use crate::text::label;
    assert_eq!(label("arabicPeriod", 3), "3.");
    assert_eq!(label("alphaLcParenR", 2), "b)");
    assert_eq!(label("alphaUcPeriod", 28), "BB.");
    assert_eq!(label("romanUcPeriod", 14), "XIV.");
    assert_eq!(label("arabicParenBoth", 7), "(7)");
    assert_eq!(label("circleNumDbPlain", 5), "⑤");
}

#[test]
fn colours_change_as_written() {
    use crate::color::{Rgba, apply};
    let accent = Rgba { r: 0x44 as f64 / 255.0, g: 0x72 as f64 / 255.0, b: 0xC4 as f64 / 255.0, a: 1.0 };
    assert_eq!(apply(accent, &[("lumMod".into(), 0.75)]).css(), "#2f5597");
    assert_eq!(apply(accent, &[("lumMod".into(), 0.2), ("lumOff".into(), 0.8)]).css(), "#dae3f3");
    assert_eq!(apply(Rgba::WHITE, &[("lumMod".into(), 0.85)]).css(), "#d9d9d9");
    assert_eq!(apply(accent, &[("alpha".into(), 0.4)]).css(), "rgba(68,114,196,0.400)");
}
