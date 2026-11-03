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
