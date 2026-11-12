//! Where the pages of a PDF or a Word document go in its tab: one or two across (Two Pages: 1–2,
//! 3–4, …), each row centred, with room around them and between them. In egui points from the
//! top-left of the scrolled content, at a scale of egui points per point (1/72 inch) of the page.

use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Room around the pages (egui points).
pub(crate) const MARGIN: f32 = 16.0;
/// Room between rows and between the two pages of a row (egui points).
pub(crate) const GAP: f32 = 12.0;
/// egui points per PDF point at 100%: a PDF point is 1/72 inch, an egui point 1/96.
pub(crate) const POINTS_PER_PT: f32 = 96.0 / 72.0;

/// The pages laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    /// Each page's rectangle.
    pub pages: Vec<Rect>,
    /// The rows from top to bottom.
    rows: Vec<Row>,
    /// The size of everything, margins included.
    pub size: Vec2,
}

/// A row of pages: its top and bottom, its first and last page.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    top: f32,
    bottom: f32,
    first: usize,
    last: usize,
}

/// The rows of `n` pages, as (first, last): one page each, or two with `spread`.
fn rows_of(n: usize, spread: bool) -> impl Iterator<Item = (usize, usize)> {
    let step = if spread { 2 } else { 1 };
    (0..n).step_by(step).map(move |first| (first, (first + step - 1).min(n.saturating_sub(1))))
}

/// Lays out pages of `sizes` (PDF points, as shown: rotation applied) at `scale` egui points per
/// PDF point, one or two across, centred in at least `min_width`.
pub(crate) fn lay_out(sizes: &[[f32; 2]], scale: f32, spread: bool, min_width: f32) -> Layout {
    let size_of = |i: usize| sizes.get(i).map_or(Vec2::ZERO, |s| vec2(s[0].max(1.0) * scale, s[1].max(1.0) * scale));
    let row_width = |(a, b): (usize, usize)| (a..=b).map(|i| size_of(i).x).sum::<f32>() + GAP * (b - a) as f32;
    let widest = rows_of(sizes.len(), spread).map(row_width).fold(0.0, f32::max);
    let width = (widest + 2.0 * MARGIN).max(min_width);
    let mut pages = vec![Rect::NOTHING; sizes.len()];
    let mut rows = Vec::new();
    let mut y = MARGIN;
    for (a, b) in rows_of(sizes.len(), spread) {
        let height = (a..=b).map(|i| size_of(i).y).fold(0.0, f32::max);
        let mut x = ((width - row_width((a, b))) / 2.0).max(MARGIN);
        for i in a..=b {
            let s = size_of(i);
            if let Some(slot) = pages.get_mut(i) {
                *slot = Rect::from_min_size(pos2(x, y + (height - s.y) / 2.0), s);
            }
            x += s.x + GAP;
        }
        rows.push(Row { top: y, bottom: y + height, first: a, last: b });
        y += height + GAP;
    }
    let height = if rows.is_empty() { 2.0 * MARGIN } else { y - GAP + MARGIN };
    Layout { pages, rows, size: vec2(width, height) }
}

/// The scale (egui points per PDF point) that makes the widest row of pages as wide as `width`
/// less the margins: Fit Width. `None` without pages.
pub(crate) fn fit_scale(sizes: &[[f32; 2]], spread: bool, width: f32) -> Option<f32> {
    rows_of(sizes.len(), spread)
        .filter_map(|(a, b)| {
            let points: f32 = (a..=b).filter_map(|i| sizes.get(i)).map(|s| s[0].max(1.0)).sum();
            let room = width - 2.0 * MARGIN - GAP * (b - a) as f32;
            (points > 0.0).then_some(room / points)
        })
        .reduce(f32::min)
        .map(|s| s.max(0.01))
}

impl Layout {
    /// The index of the first row whose bottom is at or below `y`.
    fn row_from(&self, y: f32) -> usize {
        self.rows.partition_point(|r| r.bottom < y)
    }

    /// The pages of the rows between `top` and `bottom`, top to bottom.
    pub fn visible(&self, top: f32, bottom: f32) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().skip(self.row_from(top)).take_while(move |r| r.top <= bottom).flat_map(|r| r.first..=r.last)
    }

    /// The page at the top of a view that starts at `y` (the first of its row), and how far down
    /// that page `y` is (0 at its top edge, 1 at its bottom): where to stay when the scale changes.
    pub fn top_at(&self, y: f32) -> Option<(usize, f32)> {
        let row = self.rows.get(self.row_from(y)).or(self.rows.last())?;
        let rect = self.pages.get(row.first)?;
        Some((row.first, ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0)))
    }

    /// The page most of the view between `top` and `bottom` shows (the first of its row): the
    /// page the tool bar names.
    pub fn most_shown(&self, top: f32, bottom: f32) -> Option<usize> {
        let start = self.row_from(top);
        let mut best: Option<(usize, f32)> = None;
        for r in self.rows.iter().skip(start).take_while(|r| r.top <= bottom) {
            let shown = r.bottom.min(bottom) - r.top.max(top);
            if best.is_none_or(|(_, b)| shown > b + 0.5) {
                best = Some((r.first, shown));
            }
        }
        best.map(|(p, _)| p).or_else(|| self.rows.last().map(|r| r.first))
    }

    /// The page under `p`, if it is on one.
    pub fn page_at(&self, p: Pos2) -> Option<usize> {
        let row = self.rows.get(self.row_from(p.y))?;
        (row.first..=row.last).find(|&i| self.pages.get(i).is_some_and(|r| r.contains(p)))
    }

    /// The page nearest to `p`: its row's (or the nearest row's) page nearest across. For a
    /// selection dragged past the pages' edges.
    pub fn nearest_page(&self, p: Pos2) -> Option<usize> {
        let row = self.rows.get(self.row_from(p.y)).or(self.rows.last())?;
        let row = if p.y < row.top {
            self.rows.get(self.row_from(p.y).saturating_sub(1)).filter(|r| p.y - r.bottom < row.top - p.y).unwrap_or(row)
        } else {
            row
        };
        (row.first..=row.last).min_by(|&a, &b| {
            let d = |i: usize| self.pages.get(i).map_or(f32::MAX, |r| (r.center().x - p.x).abs());
            d(a).total_cmp(&d(b))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: [f32; 2] = [595.0, 842.0];

    #[test]
    fn pages_stack_centred_with_room_between_them() {
        let l = lay_out(&[A4, A4, [842.0, 595.0]], 1.0, false, 1000.0);
        assert_eq!(l.size.x, 1000.0);
        let [a, b, c] = [l.pages[0], l.pages[1], l.pages[2]];
        assert_eq!(a.top(), MARGIN);
        assert_eq!(b.top(), a.bottom() + GAP);
        assert_eq!(c.top(), b.bottom() + GAP);
        assert_eq!(a.center().x, 500.0);
        assert_eq!(c.center().x, 500.0);
        assert_eq!(l.size.y, c.bottom() + MARGIN);
    }

    #[test]
    fn two_pages_go_side_by_side_and_an_odd_last_one_alone() {
        let l = lay_out(&[A4, A4, A4], 0.5, true, 0.0);
        let [a, b, c] = [l.pages[0], l.pages[1], l.pages[2]];
        assert_eq!(a.top(), b.top());
        assert_eq!(b.left(), a.right() + GAP);
        assert_eq!(c.top(), a.bottom() + GAP);
        assert_eq!(l.size.x, 2.0 * 595.0 * 0.5 + GAP + 2.0 * MARGIN);
        assert_eq!(c.center().x, l.size.x / 2.0);
    }

    #[test]
    fn fit_width_fills_the_width_with_the_widest_row() {
        let sizes = [A4, [842.0, 595.0]];
        let s = fit_scale(&sizes, false, 1000.0).unwrap();
        let l = lay_out(&sizes, s, false, 1000.0);
        assert!((l.pages[1].width() - (1000.0 - 2.0 * MARGIN)).abs() < 0.01);
        let s = fit_scale(&[A4, A4], true, 1000.0).unwrap();
        let l = lay_out(&[A4, A4], s, true, 1000.0);
        assert!((l.pages[1].right() - (1000.0 - MARGIN)).abs() < 0.01);
        assert_eq!(fit_scale(&[], false, 1000.0), None);
    }

    #[test]
    fn the_view_finds_its_pages() {
        let l = lay_out(&[A4; 10], 1.0, false, 800.0);
        let second = l.pages[1];
        assert_eq!(l.visible(second.top() + 5.0, second.bottom() + 20.0).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(l.top_at(second.center().y), Some((1, 0.5)));
        assert_eq!(l.most_shown(second.top() - 100.0, second.bottom() - 100.0), Some(1));
        assert_eq!(l.page_at(second.center()), Some(1));
        assert_eq!(l.page_at(pos2(1.0, second.center().y)), None, "beside the page");
        assert_eq!(l.nearest_page(pos2(1.0, second.bottom() + 2.0)), Some(1), "in the gap, nearer the page above");
        assert_eq!(l.nearest_page(pos2(1.0, 1.0e6)), Some(9));
        let spread = lay_out(&[A4; 4], 1.0, true, 0.0);
        assert_eq!(spread.page_at(spread.pages[1].center()), Some(1));
        assert_eq!(spread.top_at(spread.pages[3].top()).map(|(p, _)| p), Some(2));
    }
}
