//! A PDF's bookmarks (its outline) as the contents beside its pages: every bookmark, indented by
//! depth, and where on which page it goes.

use pdfcraft_render::{DestView, DocInfo, OutlineItem, PageInfo};

/// The most bookmarks listed.
const MOST: usize = 5000;
/// The deepest a bookmark is indented.
const DEEPEST: u8 = 8;

/// One bookmark.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Item {
    pub depth: u8,
    pub title: String,
    pub bold: bool,
    /// The page it goes to (0-based), if any.
    pub page: Option<usize>,
    /// How far down that page it goes (view-space points), when it says.
    pub top: Option<f32>,
}

/// The bookmarks of the document `info` describes, depth first, as they read.
pub(super) fn items(info: &DocInfo) -> Vec<Item> {
    fn walk(out: &mut Vec<Item>, items: &[OutlineItem], depth: u8, pages: &[PageInfo]) {
        for item in items {
            if out.len() >= MOST {
                return;
            }
            let page = item.page.filter(|p| *p < pages.len());
            // Line breaks and tabs in a title read as spaces.
            let title = item.title.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push(Item { depth, title, bold: item.bold, page, top: top_of(&item.view, page.and_then(|p| pages.get(p))) });
            walk(out, &item.children, depth.saturating_add(1).min(DEEPEST), pages);
        }
    }
    let mut out = Vec::new();
    walk(&mut out, &info.outline, 0, &info.pages);
    out
}

/// How far down its page (view-space points) a destination puts the top of the window.
fn top_of(view: &DestView, page: Option<&PageInfo>) -> Option<f32> {
    let page = page?;
    match *view {
        DestView::Xyz { left, top: Some(top), .. } => Some(page.user_to_view(left.unwrap_or(0.0), top)[1]),
        DestView::FitH { top: Some(top) } => Some(page.user_to_view(0.0, top)[1]),
        DestView::FitR { rect } => Some(page.user_to_view(rect[0], rect[3])[1]),
        _ => None,
    }
    .map(|y| y.clamp(0.0, page.height))
}
