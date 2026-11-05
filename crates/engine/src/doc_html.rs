//! Document tabs as web pages: the Markdown laid out as the document tab shows it. Headings,
//! paragraphs with their styles, lists and task boxes, quotes, code, tables with their lines and
//! shaded header row, pictures (`![…](image:N)`), `{{…}}` fields with their values, and Mermaid
//! diagrams as SVG where this build draws them (else their code).

use std::collections::HashMap;
use std::fmt::Write as _;

use monun_markdown::{Align, Block, Item, Span};
use monun_model::{Sheet, Workbook};

use crate::io::{base64_encode, esc};

/// The look of the document page, as the document tab draws it in the light theme.
pub(crate) const CSS: &str = concat!(
    ".doc{max-width:860px;margin:24px auto 48px;padding:0 24px;color:#1f1f1f;font-family:\"Malgun Gothic\",\"Apple SD Gothic Neo\",",
    "\"Noto Sans KR\",sans-serif;font-size:15px;line-height:1.45;word-break:keep-all;overflow-wrap:anywhere}",
    ".doc h1,.doc h2,.doc h3,.doc h4,.doc h5,.doc h6{font-weight:bold;line-height:1.3;margin:14px 0 8px}",
    ".doc h1{font-size:28px}.doc h2{font-size:22px}.doc h3{font-size:18.5px}.doc h4{font-size:16.5px}.doc h5,.doc h6{font-size:15px}",
    ".doc h1,.doc h2{padding-bottom:6px;border-bottom:1px solid #e0e0e0}",
    ".doc p{margin:0 0 10px}.doc ul,.doc ol{margin:0 0 10px;padding-left:26px}.doc li>p{margin:0 0 3px}",
    ".doc li.task{list-style:none;margin-left:-22px}.doc li.task>input{margin:0 8px 0 0;vertical-align:-2px}",
    ".doc blockquote{margin:0 0 10px 2px;padding-left:13px;border-left:3px solid #e0e0e0}",
    ".doc code{font-family:Consolas,\"D2Coding\",monospace;font-size:.9em;background:rgba(0,0,0,.055);border-radius:3px;padding:0 3px}",
    ".doc .code{background:rgba(0,0,0,.04);border-radius:6px;padding:10px;margin:0 0 10px}",
    ".doc .code .lang{font-size:11px;color:#606060;margin-bottom:4px}",
    ".doc pre{margin:0;white-space:pre-wrap;font-family:Consolas,\"D2Coding\",monospace;font-size:13.5px}",
    ".doc pre code{background:none;padding:0;font-size:inherit}",
    ".doc hr{border:0;border-top:1.5px solid #e0e0e0;margin:4px 0 14px}",
    ".doc a{color:#107c41}.doc del{color:inherit}",
    ".doc .field{background:#dff1e6;border-radius:2px}.doc .field.err{color:#c42b1c}",
    ".doc .dim{color:#606060}",
    ".doc .pics{margin:0 0 10px}.doc .pics img{display:block;max-width:100%;height:auto;margin-bottom:4px}",
    ".doc .tbl{overflow-x:auto;margin:0 0 10px}",
    ".doc table.md{border-collapse:separate;border-spacing:0;border:1px solid #c9ced4;border-radius:4px}",
    ".doc table.md th,.doc table.md td{padding:6px 10px;border-right:1px solid #c9ced4;border-bottom:1px solid #c9ced4;vertical-align:middle}",
    ".doc table.md th{background:#e3e8ef;font-weight:bold;text-align:center;border-bottom:1.5px solid #c9ced4}",
    ".doc table.md tr>:last-child{border-right:0}.doc table.md tbody tr:last-child>td{border-bottom:0}",
    ".doc table.md th:first-child{border-top-left-radius:4px}.doc table.md th:last-child{border-top-right-radius:4px}",
    ".doc.fit table.md{width:100%}",
    ".doc .diagram{margin:0 0 10px}.doc .diagram svg{display:block;max-width:100%;height:auto}",
    ".doc.fit .diagram{margin:0 auto 10px}.doc.fit .diagram svg{width:100%}",
);

/// What a document page needs besides its text: the workbook (fields and pictures), which other
/// sheets are on the page (their anchors, by lowercased name, for links between sheets), and
/// whether tables and diagrams fill the page's width (the document tab's Fit Width).
#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    pub wb: &'a Workbook,
    pub sheet: &'a Sheet,
    pub anchors: &'a HashMap<String, String>,
    pub fit: bool,
}

/// The document of `cx.sheet` as an `<article>`, its tables and diagrams as wide as the page with
/// its own Fit Width or the page's.
pub(crate) fn article(cx: &Context) -> String {
    let text = cx.sheet.document.as_ref().map_or("", |d| d.text.as_str());
    let cx = &Context { fit: cx.fit || cx.sheet.document.as_ref().is_some_and(|d| d.fit_width), ..*cx };
    let mut out = String::with_capacity(text.len() * 2 + 64);
    out.push_str(if cx.fit { "<article class=\"doc fit\">" } else { "<article class=\"doc\">" });
    blocks(&mut out, &monun_markdown::parse(text), cx);
    out.push_str("</article>");
    out
}

/// A document tab alone as a web page.
pub(crate) fn page(cx: &Context) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><style>body{{margin:0;background:#fff}}{CSS}</style></head><body>",
        esc(&cx.sheet.name)
    );
    out.push_str(&article(cx));
    out.push_str("</body></html>");
    out
}

fn blocks(out: &mut String, blocks: &[Block], cx: &Context) {
    for b in blocks {
        block(out, b, cx);
    }
}

fn block(out: &mut String, b: &Block, cx: &Context) {
    match b {
        Block::Heading { level, text, .. } => {
            let n = (*level).clamp(1, 6);
            let _ = write!(out, "<h{n}>");
            spans(out, text, cx);
            let _ = write!(out, "</h{n}>");
        }
        Block::Paragraph { text, .. } if text.iter().any(|s| s.style.image) && text.iter().all(|s| s.style.image || s.text.trim().is_empty()) => {
            out.push_str("<div class=\"pics\">");
            for s in text.iter().filter(|s| s.style.image) {
                picture(out, s, cx);
            }
            out.push_str("</div>");
        }
        Block::Paragraph { text, .. } => {
            out.push_str("<p>");
            spans(out, text, cx);
            out.push_str("</p>");
        }
        Block::List { start, items, .. } => {
            match start {
                Some(n) => {
                    let _ = write!(out, "<ol start=\"{n}\">");
                }
                None => out.push_str("<ul>"),
            }
            for item in items {
                list_item(out, item, cx);
            }
            out.push_str(if start.is_some() { "</ol>" } else { "</ul>" });
        }
        Block::Quote { blocks: inner, .. } => {
            out.push_str("<blockquote>");
            blocks(out, inner, cx);
            out.push_str("</blockquote>");
        }
        Block::Code { lang, text, .. } => {
            if lang != "mermaid" || !diagram(out, text, cx.fit) {
                code(out, lang, text);
            }
        }
        Block::Table { align, head, rows, .. } => table(out, align, head, rows, cx),
        Block::Rule { .. } => out.push_str("<hr>"),
    }
}

fn list_item(out: &mut String, item: &Item, cx: &Context) {
    match item.task {
        Some(task) => {
            let _ = write!(out, "<li class=\"task\"><input type=\"checkbox\" disabled{}>", if task.done { " checked" } else { "" });
        }
        None => out.push_str("<li>"),
    }
    // The item's first paragraph sits beside its bullet or box, as on screen.
    match item.blocks.split_first() {
        Some((Block::Paragraph { text, .. }, rest)) if !text.iter().any(|s| s.style.image) => {
            spans(out, text, cx);
            blocks(out, rest, cx);
        }
        _ => blocks(out, &item.blocks, cx),
    }
    out.push_str("</li>");
}

fn code(out: &mut String, lang: &str, text: &str) {
    out.push_str("<div class=\"code\">");
    if !lang.is_empty() {
        let _ = write!(out, "<div class=\"lang\">{}</div>", esc(lang));
    }
    let _ = write!(out, "<pre><code>{}</code></pre></div>", esc(text.trim_end_matches('\n')));
}

/// A Mermaid diagram as SVG; false when this build draws none (or it has an error), so its code
/// shows instead.
#[cfg(feature = "diagrams")]
fn diagram(out: &mut String, source: &str, fit: bool) -> bool {
    let Ok(svg) = monun_markdown::diagram::svg(source, false) else { return false };
    // With Fit Width a diagram grows to the page, at most twice its own size, as on screen.
    match svg_width(&svg).filter(|_| fit) {
        Some(w) => {
            let _ = write!(out, "<div class=\"diagram\" style=\"max-width:{}px\">", (w * 2.0).round());
        }
        None => out.push_str("<div class=\"diagram\">"),
    }
    out.push_str(&svg);
    out.push_str("</div>");
    true
}

#[cfg(not(feature = "diagrams"))]
fn diagram(_out: &mut String, _source: &str, _fit: bool) -> bool {
    false
}

/// The `width` an SVG's root element gives itself, in CSS pixels.
#[cfg_attr(not(feature = "diagrams"), allow(dead_code))]
fn svg_width(svg: &str) -> Option<f32> {
    let root = svg.get(svg.find("<svg")?..)?;
    let root = root.get(..root.find('>')?)?;
    let at = root.find(" width=\"")? + " width=\"".len();
    let rest = root.get(at..)?;
    rest.get(..rest.find('"')?)?.trim_end_matches("px").parse().ok().filter(|w: &f32| w.is_finite() && *w > 0.0)
}

fn table(out: &mut String, align: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], cx: &Context) {
    let cols = rows.iter().map(Vec::len).chain(std::iter::once(head.len())).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    out.push_str("<div class=\"tbl\"><table class=\"md\"><thead><tr>");
    for c in 0..cols {
        out.push_str("<th>");
        if let Some(cell) = head.get(c) {
            spans(out, cell, cx);
        }
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for c in 0..cols {
            match align.get(c).copied().unwrap_or_default() {
                Align::Right => out.push_str("<td style=\"text-align:right\">"),
                Align::Center => out.push_str("<td style=\"text-align:center\">"),
                _ => out.push_str("<td>"),
            }
            if let Some(cell) = row.get(c) {
                spans(out, cell, cx);
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
}

/// A picture of the document tab at its size (narrowed to the page), or its name when it isn't
/// there.
fn picture(out: &mut String, s: &Span, cx: &Context) {
    match s.link.as_deref().and_then(|l| crate::docs::picture(cx.sheet, l)) {
        Some(image) => {
            let alt = if s.text.is_empty() { image.alt.as_str() } else { s.text.as_str() };
            let _ = write!(
                out,
                "<img src=\"data:{};base64,{}\" alt=\"{}\" style=\"width:{}px\">",
                esc(&image.mime),
                base64_encode(&image.data),
                esc(alt),
                image.anchor.width.max(1.0).round()
            );
        }
        None => {
            let _ = write!(out, "<p class=\"dim\">[{}]</p>", esc(&s.text));
        }
    }
}

fn spans(out: &mut String, spans: &[Span], cx: &Context) {
    for s in spans {
        if s.text == "\n" && !s.style.code {
            out.push_str("<br>");
            continue;
        }
        let href = s.link.as_deref().and_then(|l| href(l, cx.anchors));
        if let Some(h) = &href {
            let _ = write!(out, "<a href=\"{}\">", esc(h));
        }
        let tags: Vec<&str> =
            [(s.style.bold, "strong"), (s.style.italic, "em"), (s.style.strike, "del")].iter().filter(|(on, _)| *on).map(|(_, t)| *t).collect();
        for t in &tags {
            let _ = write!(out, "<{t}>");
        }
        if s.style.field {
            match crate::docs::field_value(cx.wb, &s.text) {
                Ok(v) => {
                    let _ = write!(out, "<span class=\"field\" title=\"{{{{{}}}}}\">{}</span>", esc(&s.text), esc(&v));
                }
                Err(e) => {
                    let _ = write!(out, "<span class=\"field err\" title=\"{{{{{}}}}}\">{}</span>", esc(&s.text), esc(&e));
                }
            }
        } else if s.style.image {
            let _ = write!(out, "<span class=\"dim\">[{}]</span>", esc(&s.text));
        } else if s.style.code {
            let _ = write!(out, "<code>{}</code>", esc(&s.text));
        } else {
            out.push_str(&esc(&s.text).replace('\n', "<br>"));
        }
        for t in tags.iter().rev() {
            let _ = write!(out, "</{t}>");
        }
        if href.is_some() {
            out.push_str("</a>");
        }
    }
}

/// Where a link goes on the page: web and mail links as they are, a link to a sheet (`#Sheet1!B2`)
/// to that sheet's part of the page when it is there. Anything else (a script, a file) is not a
/// link on the page.
fn href(link: &str, anchors: &HashMap<String, String>) -> Option<String> {
    let lower = link.trim().to_ascii_lowercase();
    if ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
        return Some(link.trim().to_owned());
    }
    let target = link.trim().strip_prefix('#')?;
    let sheet = target.rsplit_once('!').map_or(target, |(s, _)| s);
    let sheet = sheet.trim().trim_matches('\'').replace("''", "'");
    anchors.get(&sheet.to_lowercase()).map(|a| format!("#{a}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_go_to_the_web_or_to_sheets_on_the_page() {
        let anchors: HashMap<String, String> = [("sheet1".to_owned(), "sheet-1".to_owned()), ("my notes".to_owned(), "sheet-2".to_owned())].into();
        assert_eq!(href("https://example.com/a?b=1", &anchors).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(href("mailto:me@example.com", &anchors).as_deref(), Some("mailto:me@example.com"));
        assert_eq!(href("#Sheet1!B2", &anchors).as_deref(), Some("#sheet-1"));
        assert_eq!(href("#'My Notes'!A1", &anchors).as_deref(), Some("#sheet-2"));
        assert_eq!(href("#Other!A1", &anchors), None);
        assert_eq!(href("javascript:alert(1)", &anchors), None);
        assert_eq!(href("notes.xlsx", &anchors), None);
    }

    #[test]
    fn an_svg_says_how_wide_it_is() {
        assert_eq!(svg_width("<svg xmlns=\"x\" width=\"689.5\" height=\"75\"><rect width=\"9\"/></svg>"), Some(689.5));
        assert_eq!(svg_width("<svg width=\"100%\">"), None);
        assert_eq!(svg_width("<svg viewBox=\"0 0 1 1\"><rect width=\"9\"/></svg>"), None);
        assert_eq!(svg_width("no svg"), None);
    }
}
