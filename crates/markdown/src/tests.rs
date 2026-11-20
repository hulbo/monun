use super::*;

fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn headings_paragraphs_and_inline_styles() {
    let blocks = parse("# 매수 메모\n\n**굵게** 와 *기울임*, ~~취소~~, `code`\n둘째 줄");
    let [Block::Heading { level: 1, text, line: 0 }, Block::Paragraph { text: para, line: 2 }] = blocks.as_slice() else {
        panic!("{blocks:?}");
    };
    assert_eq!(plain(text), "매수 메모");
    assert!(para[0].style.bold && para[0].text == "굵게");
    assert!(para.iter().any(|s| s.style.italic && s.text == "기울임"));
    assert!(para.iter().any(|s| s.style.strike && s.text == "취소"));
    assert!(para.iter().any(|s| s.style.code && s.text == "code"));
    // A single line break stays a break.
    assert!(para.iter().any(|s| s.text == "\n"));
    assert!(plain(para).ends_with("\n둘째 줄"));
}

#[test]
fn task_lists_know_where_their_box_is() {
    let text = "할 일\n\n- [ ] 가계부에 옮기기\n- [x] 매수\n  - 하위 항목\n";
    let blocks = parse(text);
    let Some(Block::List { start: None, items, line: 2 }) = blocks.get(1) else { panic!("{blocks:?}") };
    let first = items[0].task.unwrap();
    assert!(!first.done);
    assert_eq!(&text[first.at..first.at + 3], "[ ]");
    assert!(items[1].task.unwrap().done);
    // The nested list belongs to the second item.
    assert!(matches!(items[1].blocks.last(), Some(Block::List { .. })));
    let ticked = toggle_task(text, first.at).unwrap();
    assert!(ticked.contains("- [x] 가계부에 옮기기"));
    assert_eq!(toggle_task(&ticked, first.at).unwrap(), text);
    assert_eq!(toggle_task(text, 0), None);
    assert_eq!(toggle_task(text, text.len()), None);
}

#[test]
fn numbered_lists_quotes_rules_and_code() {
    let blocks = parse("3. 셋\n4. 넷\n\n> 인용\n\n---\n\n```mermaid\nflowchart TD\n  A --> B\n```\n");
    assert!(matches!(&blocks[0], Block::List { start: Some(3), items, .. } if items.len() == 2));
    assert!(matches!(&blocks[1], Block::Quote { blocks, line: 3 } if matches!(&blocks[0], Block::Paragraph { .. })));
    assert!(matches!(&blocks[2], Block::Rule { line: 5 }));
    let Block::Code { lang, text, line: 7 } = &blocks[3] else { panic!("{blocks:?}") };
    assert_eq!((lang.as_str(), text.as_str()), ("mermaid", "flowchart TD\n  A --> B\n"));
}

#[test]
fn tables_and_links() {
    let blocks = parse("| 대상 | 금액 |\n|:--|--:|\n| AAA | 10,000 |\n| [합계](#구매목록!D1) | 30,000 |\n");
    let Block::Table { align, head, rows, line: 0 } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(align, &[Align::Left, Align::Right]);
    assert_eq!(head.iter().map(|c| plain(c)).collect::<Vec<_>>(), ["대상", "금액"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1][0][0].link.as_deref(), Some("#구매목록!D1"));
}

#[test]
fn line_helpers() {
    let text = "a\nbb\n\nc";
    assert_eq!(line_start(text, 0), 0);
    assert_eq!(line_start(text, 1), 2);
    assert_eq!(line_start(text, 3), 6);
    assert_eq!(line_start(text, 9), text.len());
    assert_eq!(line_of(text, 3), 1);
    assert_eq!(line_of(text, 99), 3);
}

#[test]
fn odd_input_never_panics() {
    for text in ["", "\n\n", "- ", "> > >", "|", "| a |\n|---", "```", "[x](", "* [ ]", "\u{0}\u{feff}", "#######"] {
        let _ = parse(text);
    }
}

#[test]
fn fields_become_their_own_runs() {
    let blocks = parse("총합계: **{{구매목록!D1}}** 원, 합 {{ =SUM(B:B) }}\n\n`{{코드}}` {{}} {{열림\n\n| a |\n|---|\n| {{Sheet1!A1}} |\n");
    let Block::Paragraph { text, .. } = &blocks[0] else { panic!("{blocks:?}") };
    let found: Vec<(&str, bool)> = text.iter().filter(|s| s.style.field).map(|s| (s.text.as_str(), s.style.bold)).collect();
    assert_eq!(found, [("구매목록!D1", true), ("=SUM(B:B)", false)]);
    assert_eq!(plain(text), "총합계: 구매목록!D1 원, 합 =SUM(B:B)");
    // Code, an empty pair and an unclosed pair stay text.
    let Block::Paragraph { text, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert!(text.iter().all(|s| !s.style.field), "{text:?}");
    assert_eq!(plain(text), "{{코드}} {{}} {{열림");
    assert_eq!(fields(&blocks), ["구매목록!D1", "=SUM(B:B)", "Sheet1!A1"]);
}

#[test]
fn table_columns_fit_the_room() {
    // Everything fits: each column keeps its width.
    assert_eq!(fit_columns(&[50.0, 80.0], 200.0, 10.0), vec![50.0, 80.0]);
    // Too wide: the narrow columns keep theirs and the wide ones share the rest by width.
    let w = fit_columns(&[60.0, 200.0, 400.0, 400.0], 700.0, 10.0);
    assert_eq!(w[0], 60.0);
    assert_eq!(w[1], 200.0);
    assert!((w[2] - 220.0).abs() < 0.01 && (w[3] - 220.0).abs() < 0.01, "{w:?}");
    assert!((w.iter().sum::<f32>() - 700.0).abs() < 0.01);
    // No column goes below the floor, even if the table then overflows.
    let w = fit_columns(&[100.0; 10], 50.0, 20.0);
    assert!(w.iter().all(|w| *w == 20.0), "{w:?}");
    assert!(fit_columns(&[], 100.0, 10.0).is_empty());
    assert_eq!(fit_columns(&[f32::NAN, 10.0], 100.0, 5.0), vec![0.0, 10.0]);
}

#[test]
fn filled_columns_reach_the_room() {
    // Narrower than the room: each column grows by its share, to the room's width.
    let w = fill_columns(&[50.0, 150.0], 400.0, 10.0);
    assert!((w[0] - 100.0).abs() < 0.01 && (w[1] - 300.0).abs() < 0.01, "{w:?}");
    // Wider: the same as fitting.
    assert_eq!(fill_columns(&[60.0, 200.0, 400.0, 400.0], 700.0, 10.0), fit_columns(&[60.0, 200.0, 400.0, 400.0], 700.0, 10.0));
    assert!(fill_columns(&[], 100.0, 10.0).is_empty());
    assert_eq!(fill_columns(&[0.0, 0.0], 100.0, 10.0), vec![0.0, 0.0]);
}

#[test]
fn front_matter_is_left_out_of_the_view() {
    let text = "---\ntitle: 재이관 설계서\nversion: 2\n---\n\n# 개요\n\n본문";
    let blocks = parse(text);
    let [Block::Heading { level: 1, text: title, line: 5 }, Block::Paragraph { text: body, line: 7 }] = blocks.as_slice() else {
        panic!("{blocks:?}");
    };
    assert_eq!(plain(title), "개요");
    assert_eq!(plain(body), "본문");
    // Only at the very top: further down, `---` is a rule (or a heading's underline) as before.
    let blocks = parse("# 개요\n\n---\n\n본문\n");
    assert!(blocks.iter().any(|b| matches!(b, Block::Rule { line: 2 })), "{blocks:?}");
    let blocks = parse("본문\n---\n");
    assert!(matches!(blocks.as_slice(), [Block::Heading { level: 2, .. }]), "{blocks:?}");
}

/// The runs of a table cell joined, line breaks and all.
fn cell(blocks: &[Block], row: usize, col: usize) -> String {
    let Some(Block::Table { rows, .. }) = blocks.iter().find(|b| matches!(b, Block::Table { .. })) else { panic!("{blocks:?}") };
    plain(&rows[row][col])
}

#[test]
fn br_breaks_the_line_in_a_table_cell_and_a_paragraph() {
    let blocks =
        parse("| 단계 | 하는 일 |\n|---|---|\n| 2 이관 | insert<br>**희리패 행**은 그대로<br/>· 공고 18<BR />· 신청 16 |\n\n첫 줄<br>둘째 줄 <br>\n");
    assert_eq!(cell(&blocks, 0, 1), "insert\n희리패 행은 그대로\n· 공고 18\n· 신청 16");
    let Some(Block::Table { rows, .. }) = blocks.first() else { panic!("{blocks:?}") };
    assert!(rows[0][1].iter().any(|s| s.style.bold && s.text == "희리패 행"));
    // A break is a run of its own; the paragraph's last one adds no line.
    let Some(Block::Paragraph { text, .. }) = blocks.get(1) else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "첫 줄\n둘째 줄");
    assert!(text.iter().any(|s| s.text == "\n"));
    // A `<br>` at a line's end or start breaks the line once with the source's break, as on
    // GitHub; two make an empty line.
    for (source, shown) in
        [("가<br>\n나", "가\n나"), ("가\n<br>나", "가\n나"), ("가\n<br>\n나", "가\n나"), ("가<br><br>\n나", "가\n\n나"), ("가\n나", "가\n나")]
    {
        let blocks = parse(source);
        let [Block::Paragraph { text, .. }] = blocks.as_slice() else { panic!("{source:?}: {blocks:?}") };
        assert_eq!(plain(text), shown, "{source:?}");
    }
}

#[test]
fn inline_tags_style_their_text() {
    let blocks = parse(
        "<b>굵게</b> <i>기울임</i> <u>밑줄</u> <s>취소</s> <mark>강조</mark> <kbd>Ctrl</kbd>+<code>C</code> <sup>2</sup> <span style=\"color:red\">빨강</span> <a href=\"https://x.kr\">링크</a> <!-- 메모 -->끝",
    );
    let Block::Paragraph { text, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "굵게 기울임 밑줄 취소 강조 Ctrl+C 2 빨강 링크 끝");
    let find = |t: &str| text.iter().find(|s| s.text == t).unwrap_or_else(|| panic!("{t}: {text:?}"));
    assert!(find("굵게").style.bold);
    assert!(find("기울임").style.italic);
    assert!(find("밑줄").style.underline);
    assert!(find("취소").style.strike);
    assert!(find("강조").style.mark);
    assert!(find("Ctrl").style.code && find("C").style.code);
    assert_eq!(find("링크").link.as_deref(), Some("https://x.kr"));
    assert!(text.iter().any(|s| s.text.contains("빨강") && s.style == Style::default()));
    assert!(find(" 끝").link.is_none());
}

#[test]
fn a_tag_left_open_ends_with_its_cell() {
    let blocks = parse("| a | b |\n|---|---|\n| <b>굵게 | 보통 |\n\n<i>기울임\n\n보통\n");
    let Some(Block::Table { rows, .. }) = blocks.first() else { panic!("{blocks:?}") };
    assert!(rows[0][0][0].style.bold);
    assert!(!rows[0][1][0].style.bold);
    let Some(Block::Paragraph { text, .. }) = blocks.last() else { panic!("{blocks:?}") };
    assert!(!text[0].style.italic, "{blocks:?}");
}

#[test]
fn lists_in_a_cell_become_lines() {
    let blocks = parse("| 항목 |\n|---|\n| <ul><li>하나</li><li>둘</li></ul> |\n| <ol start=\"3\"><li>셋</li><li>넷</li></ol> |\n");
    assert_eq!(cell(&blocks, 0, 0), "• 하나\n• 둘");
    assert_eq!(cell(&blocks, 1, 0), "3. 셋\n4. 넷");
}

#[test]
fn comments_are_hidden() {
    let blocks = parse("<!-- 머리말 메모\n여러 줄 -->\n\n본문 <!-- 안쪽 --> 계속\n\n<!-- 끝 -->\n");
    let [Block::Paragraph { text, line: 3 }] = blocks.as_slice() else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "본문  계속");
}

#[test]
fn html_blocks_become_blocks() {
    let text = "<div align=\"center\">\n  <h2>제목</h2>\n  <b>굵게</b> 그리고\n  여러   칸<br>둘째 줄\n</div>\n\n<details>\n<summary>펼치기</summary>\n\n본문 **굵게**\n\n</details>\n\n<ul>\n  <li>하나</li>\n  <li>둘<ol><li>안쪽</li></ol></li>\n</ul>\n\n<hr>\n\n<blockquote>인용</blockquote>\n\n<pre>\n  코드\n    줄\n</pre>\n";
    let blocks = parse(text);
    let Block::Heading { level: 2, text: title, line: 1 } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(plain(title), "제목");
    let Block::Paragraph { text: para, line: 2 } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(plain(para), "굵게 그리고 여러 칸\n둘째 줄");
    assert!(para[0].style.bold);
    let Block::Paragraph { text: summary, line: 7 } = &blocks[2] else { panic!("{blocks:?}") };
    assert_eq!(plain(summary), "펼치기");
    assert!(summary.iter().all(|s| s.style.bold));
    let Block::Paragraph { text: body, .. } = &blocks[3] else { panic!("{blocks:?}") };
    assert_eq!(plain(body), "본문 굵게");
    let Block::List { start: None, items, line: 13 } = &blocks[4] else { panic!("{blocks:?}") };
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[1].blocks[..], [Block::Paragraph { .. }, Block::List { start: Some(1), .. }]), "{items:?}");
    assert!(matches!(blocks[5], Block::Rule { line: 18 }));
    assert!(matches!(&blocks[6], Block::Quote { blocks, .. } if matches!(&blocks[..], [Block::Paragraph { .. }])));
    let Block::Code { lang, text: code, .. } = &blocks[7] else { panic!("{blocks:?}") };
    assert_eq!((lang.as_str(), code.as_str()), ("", "  코드\n    줄\n"));
    assert_eq!(blocks.len(), 8, "{blocks:?}");
}

#[test]
fn html_tables_become_tables() {
    let text = "<table>\n<tr><th>이름</th><th align=\"right\">금액</th></tr>\n<tr><td>가<br>나</td><td style=\"text-align: right\">1,000</td></tr>\n<tr><td><b>합계</b></td><td>&nbsp;</td></tr>\n</table>\n\n<table><tr><td>머리 없음</td><td>둘</td></tr><tr><td>셋</td></tr></table>\n";
    let blocks = parse(text);
    let Block::Table { align, head, rows, line: 0 } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(align, &[Align::None, Align::Right]);
    assert_eq!(head.iter().map(|c| plain(c)).collect::<Vec<_>>(), ["이름", "금액"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(plain(&rows[0][0]), "가\n나");
    assert!(rows[1][0][0].style.bold);
    assert_eq!(plain(&rows[1][1]), "\u{a0}");
    // Without `<th>`, the first row is the header.
    let Block::Table { head, rows, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(head.iter().map(|c| plain(c)).collect::<Vec<_>>(), ["머리 없음", "둘"]);
    assert_eq!(rows.len(), 1);
}

#[test]
fn html_pictures_and_checkboxes() {
    let blocks = parse(
        "<p align=\"center\"><img src=\"pictures/로고.png\" width=\"200\"></p>\n\n<img alt=\"그림\" src='a.png'>\n\n<input type=\"checkbox\" checked> 끝냄 <input type=checkbox> 남음\n",
    );
    let Block::Paragraph { text, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert!(text.len() == 1 && text[0].style.image && text[0].text == "로고.png" && text[0].link.as_deref() == Some("pictures/로고.png"), "{text:?}");
    let Block::Paragraph { text, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!((text[0].text.as_str(), text[0].link.as_deref()), ("그림", Some("a.png")));
    let Block::Paragraph { text, .. } = &blocks[2] else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "☑ 끝냄 ☐ 남음");
    assert_eq!(img_sources("![x](b.png) <img src=\"a.png\"> <IMG SRC=a.png> <img src=\"c.jpg\">"), ["a.png", "c.jpg"]);
}

#[test]
fn what_looks_like_html_but_isnt_stays_text() {
    let blocks = parse("a < b, 3<4, <기본값>, 화살표 <-- 와 -->\n\n`<br>` 는 코드\n\n```html\n<b>코드 블록</b>\n```\n");
    let Block::Paragraph { text, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "a < b, 3<4, <기본값>, 화살표 <-- 와 -->");
    let Block::Paragraph { text, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert!(text[0].style.code && text[0].text == "<br>");
    assert!(matches!(&blocks[2], Block::Code { text, .. } if text == "<b>코드 블록</b>\n"));
}

#[test]
fn odd_html_never_panics() {
    for text in [
        "<",
        "</",
        "<table>",
        "<table><td>",
        "</table></tr></td>",
        "<ul><li>",
        "</li></ul></ol>",
        "<li>a</li>",
        "<blockquote><ul><li>a</blockquote>b</li>",
        "<table><tr><td><table><tr><td>안</td></tr></table></td></tr></table>",
        "<pre>",
        "<script>x",
        "<!--",
        "<a href=\"x\"><a href=\"y\">a</a>",
        "> <div>\n> <b>인용 안",
        "- <ul>\n  <li>목록 안",
        "| <table> |\n|---|\n| </td> |",
        "<h7>x</h7><h0>",
        "<img>",
        "<b></b></b></i>",
        "<details><summary></summary></details>",
    ] {
        let _ = parse(text);
    }
}

#[test]
fn html_blocks_cut_by_a_blank_line_leave_no_empty_blocks() {
    // GitHub's way of putting Markdown in a table cell or list item: the HTML around it ends at
    // the blank line, and the Markdown shows as itself.
    for (text, at) in [("<table>\n<tr>\n<td>\n\n**본문**\n\n</td>\n</tr>\n</table>\n", 4), ("<ul>\n<li>\n\n**본문**\n\n</li>\n</ul>\n", 3)] {
        let blocks = parse(text);
        let [Block::Paragraph { text: body, line }] = blocks.as_slice() else { panic!("{text:?}: {blocks:?}") };
        assert_eq!((plain(body).as_str(), *line), ("본문", at));
    }
}

#[test]
fn html_block_paragraphs_know_their_line() {
    let blocks = parse("<div>\n본문\n</div>\n\n<details>\n<summary>요약</summary>\n\n내용\n\n</details>\n");
    assert!(matches!(&blocks[0], Block::Paragraph { line: 1, .. }), "{blocks:?}");
    assert!(matches!(&blocks[1], Block::Paragraph { line: 5, .. }), "{blocks:?}");
}

#[test]
fn an_indented_html_block_starts_without_spaces() {
    let blocks = parse("  <span>\n  글\n  </span>\n");
    let [Block::Paragraph { text, .. }] = blocks.as_slice() else { panic!("{blocks:?}") };
    assert_eq!(plain(text), "글");
}

#[test]
fn tags_in_an_item_without_text_end_with_it() {
    let blocks = parse("- <b><i>\n- 다음\n");
    let Some(Block::List { items, .. }) = blocks.first() else { panic!("{blocks:?}") };
    let Some(Block::Paragraph { text, .. }) = items.get(1).and_then(|i| i.blocks.first()) else { panic!("{items:?}") };
    assert_eq!(text[0].style, Style::default());
}

#[test]
// A test reads the clock natively, never on wasm.
#[allow(clippy::disallowed_methods)]
fn many_ampersands_read_quickly() {
    let text = format!("<img src=\"a.png\">\n\n{}", "R&D ".repeat(50_000));
    let started = std::time::Instant::now();
    assert_eq!(img_sources(&text), ["a.png"]);
    let _ = parse(&text);
    assert!(started.elapsed().as_secs_f32() < 2.0, "{:?}", started.elapsed());
}

#[test]
fn source_files_are_known_by_extension_or_name() {
    use crate::code::{Lang, source_of};
    let of = |name: &str| source_of(name).map(|s| (s.label, s.lang));
    assert_eq!(of("src/main/java/demo/Main.java"), Some(("Java", Lang::Java)));
    assert_eq!(of(r"C:\work\front\App.vue"), Some(("Vue", Lang::Markup)));
    assert_eq!(of("application-local.YML"), Some(("YAML", Lang::Config)));
    assert_eq!(of("mapper/UserMapper.xml"), Some(("XML", Lang::Markup)));
    assert_eq!(of("messages.properties"), Some(("Properties", Lang::Config)));
    assert_eq!(of("index.tsx"), Some(("TypeScript JSX", Lang::Script)));
    assert_eq!(of("build.gradle"), Some(("Gradle", Lang::Java)));
    assert_eq!(of("Dockerfile"), Some(("Dockerfile", Lang::Shell)));
    assert_eq!(of(".gitignore"), Some(("Ignore", Lang::Shell)));
    assert_eq!(of(".env.production"), Some(("Environment", Lang::Config)));
    assert_eq!(of("server.log"), Some(("Log", Lang::Plain)));
    // Files of their own kinds, and files that aren't source.
    for name in ["query.sql", "notes.md", "a.http", "readme.txt", "page.html", "book.xlsx", "photo.png", "noext", ".java"] {
        assert_eq!(of(name), None, "{name}");
    }
    assert!(crate::code::source_extensions().any(|x| x == "jsp"));
}
