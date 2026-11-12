//! The text of a PDF's pages (PdfCraft's text layer, one `PageText` a page): the selection, which
//! can run over several pages, and what Find matches.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

use pdfcraft_render::PageText;

/// A place in the text: a page, and a glyph boundary on it (0 before its first glyph, its glyph
/// count after its last).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Spot {
    pub page: usize,
    pub at: usize,
}

/// Selected text: from where the drag started to where it is, either way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub anchor: Spot,
    pub head: Spot,
}

impl Selection {
    /// Glyphs `first..=last` of page `page` (a word or a line).
    pub fn glyphs(page: usize, (first, last): (usize, usize)) -> Selection {
        Selection { anchor: Spot { page, at: first }, head: Spot { page, at: last + 1 } }
    }

    /// Every glyph of page `page`, which has `len`.
    pub fn page(page: usize, len: usize) -> Selection {
        Selection { anchor: Spot { page, at: 0 }, head: Spot { page, at: len } }
    }

    fn ends(&self) -> (Spot, Spot) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    /// The glyphs selected on page `page`, which has `len` glyphs; `None` for none.
    pub fn on_page(&self, page: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ends();
        if page < start.page || page > end.page {
            return None;
        }
        let from = if page == start.page { start.at.min(len) } else { 0 };
        let to = if page == end.page { end.at.min(len) } else { len };
        (from < to).then_some(from..to)
    }

    /// The selected text, the pages' parts one after another on lines of their own. A page whose
    /// text isn't at hand (`texts`) is left out.
    pub fn text(&self, texts: &HashMap<usize, Arc<PageText>>) -> String {
        let (start, end) = self.ends();
        (start.page..=end.page)
            .filter_map(|p| {
                let text = texts.get(&p)?;
                Some(text.text_of(self.on_page(p, text.glyphs.len())?))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether anything is selected.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The glyph boundary nearest to (`x`, `y`) on a page (view-space points): before or after the
/// nearest glyph, by which half of it the point is on along its writing direction.
pub(super) fn caret_at(text: &PageText, x: f32, y: f32) -> Option<usize> {
    let i = text.nearest(x, y)?;
    let g = text.glyphs.get(i)?;
    let (cx, cy) = ((g.rect[0] + g.rect[2]) / 2.0, (g.rect[1] + g.rect[3]) / 2.0);
    let [dx, dy] = g.direction;
    Some(if (x - cx) * dx + (y - cy) * dy > 0.0 { i + 1 } else { i })
}

/// What Find matches, by page: the glyph ranges of each match, for the pages searched so far.
#[derive(Default)]
pub(super) struct Hits {
    query: String,
    pages: BTreeMap<usize, Vec<Range<usize>>>,
}

impl Hits {
    /// Searches the pages of `texts` not searched for `query` yet; a new query starts over.
    pub fn update(&mut self, query: &str, texts: &HashMap<usize, Arc<PageText>>) {
        if self.query != query {
            self.query = query.to_owned();
            self.pages.clear();
        }
        if query.trim().is_empty() {
            return;
        }
        for (page, text) in texts {
            self.pages.entry(*page).or_insert_with(|| text.find(query));
        }
    }

    /// Whether page `page` has been searched for the query.
    pub fn searched(&self, page: usize) -> bool {
        self.pages.contains_key(&page)
    }

    /// How many pages have been searched.
    pub fn pages_searched(&self) -> usize {
        self.pages.len()
    }

    /// The matches found so far.
    pub fn total(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    /// Page `page`'s matches.
    pub fn on_page(&self, page: usize) -> &[Range<usize>] {
        self.pages.get(&page).map_or(&[], Vec::as_slice)
    }

    /// Every match, in page order, each as where it starts.
    fn spots(&self) -> impl Iterator<Item = Spot> + '_ {
        self.pages.iter().flat_map(|(page, hits)| hits.iter().map(|h| Spot { page: *page, at: h.start }))
    }

    /// Where match `spot` is in the order of those found (0-based).
    pub fn index_of(&self, spot: Spot) -> Option<usize> {
        self.spots().position(|s| s == spot)
    }

    /// The match after `current` (`forward`) or before it, round from the end; without one, the
    /// first on or after page `page` (or before it, going back).
    pub fn step(&self, current: Option<Spot>, forward: bool, page: usize) -> Option<Spot> {
        let all: Vec<Spot> = self.spots().collect();
        let n = all.len();
        if n == 0 {
            return None;
        }
        let i = match current.and_then(|c| all.iter().position(|s| *s == c)) {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None if forward => all.iter().position(|s| s.page >= page).unwrap_or(0),
            None => all.iter().rposition(|s| s.page <= page).unwrap_or(n - 1),
        };
        all.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(page: usize, at: usize) -> Spot {
        Spot { page, at }
    }

    #[test]
    fn a_selection_runs_over_pages_either_way_round() {
        let s = Selection { anchor: spot(3, 2), head: spot(1, 5) };
        assert_eq!(s.on_page(0, 10), None);
        assert_eq!(s.on_page(1, 10), Some(5..10));
        assert_eq!(s.on_page(2, 7), Some(0..7));
        assert_eq!(s.on_page(3, 10), Some(0..2));
        assert_eq!(s.on_page(4, 10), None);
        assert_eq!(Selection { anchor: spot(1, 4), head: spot(1, 4) }.on_page(1, 10), None);
        assert!(Selection { anchor: spot(1, 4), head: spot(1, 4) }.is_empty());
        assert_eq!(Selection::glyphs(2, (3, 5)).on_page(2, 10), Some(3..6));
        assert_eq!(Selection::page(2, 8).on_page(2, 8), Some(0..8));
        assert_eq!(Selection::page(2, 0).on_page(2, 0), None);
    }

    #[test]
    fn find_steps_through_the_matches_round_and_round() {
        let mut hits = Hits::default();
        hits.query = "a".into();
        hits.pages.insert(0, vec![1..2, 5..6]);
        hits.pages.insert(2, vec![]);
        hits.pages.insert(4, vec![0..1]);
        assert_eq!(hits.total(), 3);
        assert_eq!(hits.step(None, true, 3), Some(spot(4, 0)));
        assert_eq!(hits.step(None, false, 3), Some(spot(0, 5)));
        assert_eq!(hits.step(Some(spot(4, 0)), true, 0), Some(spot(0, 1)));
        assert_eq!(hits.step(Some(spot(0, 1)), false, 0), Some(spot(4, 0)));
        assert_eq!(hits.index_of(spot(0, 5)), Some(1));
        assert!(hits.searched(2) && !hits.searched(3));
        // A new query starts over.
        hits.update("b", &HashMap::new());
        assert_eq!((hits.total(), hits.pages_searched()), (0, 0));
        assert_eq!(hits.step(None, true, 0), None);
    }
}
