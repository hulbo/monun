//! A sequence diagram's rows and frames given the room they need. The renderer stacks the rows,
//! but:
//! - it counted a note written just before a frame (`Note …` then `opt …`) into the frame, whose
//!   top then rose to its parent's, the two titles over each other;
//! - it left a frame's title over a tall label of the frame's first message;
//! - it let an arrow to its own lane run into the next row (its number over the return arrow).
//!
//! Here the drawing is opened up wherever two things meet (all below moved down), and each frame's
//! top and bottom are set round what the source puts in it.

use mermaid_rs_renderer::Layout;
use mermaid_rs_renderer::ir::{Graph, SequenceFrame};
use mermaid_rs_renderer::layout::{DiagramData, SequenceData};

/// Between two things that would otherwise meet.
const GAP: f32 = 6.0;
/// Between a frame's title and what is first in it.
const TITLE_GAP: f32 = 4.0;
/// Between the last of the drawing and the participants' boxes at its foot.
const FOOT_GAP: f32 = 16.0;
/// Round a label's text, as the renderer draws its background.
const LABEL_PAD: f32 = 3.0;

/// A box on the drawing: left, top, right, bottom.
type Rect = [f32; 4];

fn meet_across(a: &Rect, b: &Rect) -> bool {
    a[0] < b[2] && b[0] < a[2]
}

/// Where each note sits among the frames, from the source: inside a frame it is written in at
/// the frame's very start (the renderer can't tell it from one just before), or after a frame
/// that closed with no message since.
#[derive(Clone, Default)]
struct NotePlace {
    /// Frames (in the renderer's order, as they close) the note opens.
    opens: Vec<usize>,
    /// Frames the note comes just after.
    follows: Vec<usize>,
}

/// The notes' places, when the source reads as the renderer read it (as many messages, notes and
/// frames, at the same indexes); else `None`, and notes at a frame's start count as before it.
fn note_places(source: &str, graph: &Graph) -> Option<Vec<NotePlace>> {
    const FRAMES: [&str; 7] = ["alt", "opt", "loop", "par", "rect", "critical", "break"];
    const OTHER: [&str; 17] = [
        "sequencediagram",
        "participant",
        "actor",
        "activate",
        "deactivate",
        "autonumber",
        "title",
        "acctitle",
        "accdescr",
        "box",
        "else",
        "and",
        "option",
        "create",
        "destroy",
        "link",
        "links",
    ];
    let starts_with_word = |lower: &str, word: &str| lower == word || lower.strip_prefix(word).is_some_and(|rest| rest.starts_with(' '));
    let mut messages = 0usize;
    // Open frames: their order of opening and the message count they opened at.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut opened = 0usize;
    // Frames closed, by order of opening, in the order they close, with the count they closed at.
    let mut closed: Vec<(usize, usize)> = Vec::new();
    let mut notes: Vec<(usize, Vec<usize>, Vec<usize>)> = Vec::new();
    for raw in source.lines() {
        let line = raw.trim();
        let lower = line.to_ascii_lowercase();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if FRAMES.iter().any(|w| starts_with_word(&lower, w)) {
            open.push((opened, messages));
            opened += 1;
        } else if lower == "end" {
            if let Some((ordinal, _)) = open.pop() {
                closed.push((ordinal, messages));
            }
        } else if lower.starts_with("note ") {
            let opens = open.iter().filter(|(_, at)| *at == messages).map(|(o, _)| *o).collect();
            let follows = closed.iter().filter(|(_, at)| *at == messages).map(|(o, _)| *o).collect();
            notes.push((messages, opens, follows));
        } else if OTHER.iter().any(|w| starts_with_word(&lower, w)) {
        } else if line.split(':').next().is_some_and(|head| ["->", "-x", "-)"].iter().any(|a| head.contains(a))) {
            messages += 1;
        }
    }
    // The renderer's frames are listed as they close: an ordinal of opening for each.
    let order: Vec<usize> = closed.iter().map(|(o, _)| *o).collect();
    let same = messages == graph.edges.len()
        && order.len() == graph.sequence_frames.len()
        && notes.len() == graph.sequence_notes.len()
        && notes.iter().zip(&graph.sequence_notes).all(|(n, g)| n.0 == g.index);
    if !same {
        return None;
    }
    let index_of = |ordinal: usize| order.iter().position(|&o| o == ordinal);
    Some(
        notes
            .into_iter()
            .map(|(_, opens, follows)| NotePlace {
                opens: opens.into_iter().filter_map(index_of).collect(),
                follows: follows.into_iter().filter_map(index_of).collect(),
            })
            .collect(),
    )
}

/// Gives the rows and frames of a laid-out sequence diagram the room they need.
pub(super) fn tidy(source: &str, graph: &Graph, layout: &mut Layout, font_size: f32) {
    if !matches!(layout.diagram, DiagramData::Sequence(_)) {
        return;
    }
    let places = note_places(source, graph).unwrap_or_else(|| vec![NotePlace::default(); graph.sequence_notes.len()]);
    let badge = (font_size * 0.45).max(6.0);
    open_rows(layout, badge, font_size);
    // The renderer's frames, as it lists them (by first message, the outer first), each with the
    // source's frame it draws.
    let mut frames: Vec<usize> = (0..graph.sequence_frames.len()).collect();
    frames.sort_by(|&a, &b| {
        let (fa, fb) = (graph.sequence_frames.get(a), graph.sequence_frames.get(b));
        let key = |f: Option<&SequenceFrame>| f.map_or((0, std::cmp::Reverse(0)), |f| (f.start_idx, std::cmp::Reverse(f.end_idx)));
        key(fa).cmp(&key(fb))
    });
    frames.retain(|&f| graph.sequence_frames.get(f).is_some_and(|f| f.start_idx < f.end_idx && f.start_idx < graph.edges.len()));
    let drawn = match &layout.diagram {
        DiagramData::Sequence(seq) => seq.frames.len(),
        _ => 0,
    };
    if drawn != frames.len() {
        return;
    }
    // The innermost first, so a frame holding another sets its top above the other's title.
    let mut by_size: Vec<usize> = (0..frames.len()).collect();
    let span = |d: usize| frames.get(d).and_then(|&f| graph.sequence_frames.get(f)).map_or(0, |f| f.end_idx - f.start_idx);
    by_size.sort_by_key(|&d| (span(d), std::cmp::Reverse(d)));
    for d in by_size {
        if let Some(frame) = frames.get(d).and_then(|&f| graph.sequence_frames.get(f)) {
            let source_index = frames.get(d).copied().unwrap_or(0);
            fit_frame(layout, graph, &places, &frames, d, frame, source_index, badge, font_size);
        }
    }
    clear_feet(layout, badge, font_size);
}

/// The participants' boxes at the foot below everything (an arrow to its own lane last of all
/// ran into them).
fn clear_feet(layout: &mut Layout, badge: f32, font_size: f32) {
    let DiagramData::Sequence(seq) = &layout.diagram else { return };
    let feet = seq.footboxes.iter().map(|f| f.y).fold(f32::MAX, f32::min);
    if feet == f32::MAX {
        return;
    }
    let mut lowest: Vec<Rect> = rows(layout).into_iter().flat_map(|row| row_boxes(layout, row, badge, font_size)).collect();
    if let DiagramData::Sequence(seq) = &layout.diagram {
        lowest.extend(seq.frames.iter().map(|f| [f.x, f.y, f.x + f.width, f.y + f.height]));
    }
    let dy = bottom(&lowest) + FOOT_GAP - feet;
    if lowest.is_empty() || dy <= 0.0 {
        return;
    }
    // Only the feet (and the lifelines down to them) move: what reaches below them stays.
    if let DiagramData::Sequence(seq) = &mut layout.diagram {
        for f in &mut seq.footboxes {
            f.y += dy;
        }
        for l in &mut seq.lifelines {
            if l.y2 >= feet - 0.5 {
                l.y2 += dy;
            }
        }
    }
    layout.height += dy;
}

/// The boxes of message `i`: its line (with its loop, for an arrow to its own lane), its label and
/// its number.
fn message_boxes(layout: &Layout, i: usize, badge: f32, font_size: f32) -> Vec<Rect> {
    let mut boxes = Vec::new();
    let Some(edge) = layout.edges.get(i) else { return boxes };
    let xs = edge.points.iter().map(|p| p.0);
    let ys = edge.points.iter().map(|p| p.1);
    let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
    let (y0, y1) = (ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max));
    if x0 <= x1 {
        boxes.push([x0, y0 - 1.0, x1, y1 + 1.0]);
    }
    if let Some(label) = &edge.label {
        let (x, y) = edge.label_anchor.unwrap_or_else(|| {
            let gap = (font_size * 0.25).clamp(3.0, 5.0);
            ((x0 + x1) / 2.0, y0 - gap - label.height / 2.0)
        });
        let (w, h) = (label.width / 2.0 + LABEL_PAD, label.height / 2.0 + LABEL_PAD);
        boxes.push([x - w, y - h, x + w, y + h]);
    }
    // Numbered (`autonumber`), each message has one, a little above its line.
    if let DiagramData::Sequence(seq) = &layout.diagram
        && seq.numbers.len() == layout.edges.len()
        && let Some(n) = seq.numbers.get(i)
    {
        boxes.push([n.x - badge, n.y - badge, n.x + badge, n.y + badge]);
    }
    boxes
}

fn note_box(seq: &SequenceData, j: usize) -> Option<Rect> {
    seq.notes.get(j).map(|n| [n.x, n.y, n.x + n.width, n.y + n.height])
}

fn top(boxes: &[Rect]) -> f32 {
    boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min)
}

fn bottom(boxes: &[Rect]) -> f32 {
    boxes.iter().map(|b| b[3]).fold(f32::MIN, f32::max)
}

/// What the renderer stacks, top to bottom: the notes before each message, then the message.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Note(usize),
    Message(usize),
}

fn rows(layout: &Layout) -> Vec<Row> {
    let DiagramData::Sequence(seq) = &layout.diagram else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0..=layout.edges.len() {
        out.extend(seq.notes.iter().enumerate().filter(|(_, n)| n.index == i).map(|(j, _)| Row::Note(j)));
        if i < layout.edges.len() {
            out.push(Row::Message(i));
        }
    }
    out
}

fn row_boxes(layout: &Layout, row: Row, badge: f32, font_size: f32) -> Vec<Rect> {
    match (row, &layout.diagram) {
        (Row::Message(i), _) => message_boxes(layout, i, badge, font_size),
        (Row::Note(j), DiagramData::Sequence(seq)) => note_box(seq, j).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Moves each row down past any row above it that it meets.
fn open_rows(layout: &mut Layout, badge: f32, font_size: f32) {
    let rows = rows(layout);
    for (k, &row) in rows.iter().enumerate() {
        let here = row_boxes(layout, row, badge, font_size);
        let mut need = 0.0f32;
        for &above in rows.iter().take(k) {
            for a in row_boxes(layout, above, badge, font_size) {
                for b in here.iter().filter(|b| meet_across(&a, b)) {
                    need = need.max(a[3] + GAP - b[1]);
                }
            }
        }
        if need > 0.0 {
            push_down(layout, k, top(&here) - 0.5, need);
        }
    }
}

/// Where in [`rows`] the first row `wanted` is (given the row and, for a note, its index), or the
/// end.
fn first_row(layout: &Layout, wanted: impl Fn(Row, usize) -> bool) -> usize {
    let DiagramData::Sequence(seq) = &layout.diagram else { return usize::MAX };
    let rows = rows(layout);
    rows.iter()
        .position(|&row| {
            let index = match row {
                Row::Note(j) => seq.notes.get(j).map_or(0, |n| n.index),
                Row::Message(i) => i,
            };
            wanted(row, index)
        })
        .unwrap_or(rows.len())
}

/// Sets frame `d` (the renderer's `d`th, drawing the source's `frame`) round what is in it: its
/// title just above the first of it, clear of what comes before; its sections' titles clear of
/// their messages; its bottom just below the last of it.
#[allow(clippy::too_many_arguments)]
fn fit_frame(
    layout: &mut Layout,
    graph: &Graph,
    places: &[NotePlace],
    frames: &[usize],
    d: usize,
    frame: &SequenceFrame,
    source_index: usize,
    badge: f32,
    font_size: f32,
) {
    let (start, end) = (frame.start_idx, frame.end_idx);
    let inside_note = |j: usize, index: usize| {
        let place = places.get(j);
        (index > start && index < end)
            || (index == start && place.is_some_and(|p| p.opens.contains(&source_index)))
            || (index == end && index > start && !place.is_some_and(|p| p.follows.contains(&source_index)))
    };
    let before_note = |j: usize, index: usize| index < start || (index == start && !inside_note(j, index));
    // The frames inside this one, and those wholly before it.
    let inner: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|&(e, &f)| {
            e != d
                && graph.sequence_frames.get(f).is_some_and(|g| {
                    g.start_idx >= start && g.end_idx <= end && (g.start_idx, std::cmp::Reverse(g.end_idx)) > (start, std::cmp::Reverse(end))
                })
        })
        .map(|(e, _)| e)
        .collect();
    let earlier: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|&(_, &f)| graph.sequence_frames.get(f).is_some_and(|g| g.end_idx <= start && g.start_idx < start))
        .map(|(e, _)| e)
        .collect();
    let frame_rect = |layout: &Layout, e: usize| match &layout.diagram {
        DiagramData::Sequence(seq) => seq.frames.get(e).map(|f| [f.x, f.y, f.x + f.width, f.y + f.height]),
        _ => None,
    };
    let content = |layout: &Layout, from: usize, to: usize, with_inner: bool| -> Vec<Rect> {
        let mut boxes: Vec<Rect> = (from..to.min(layout.edges.len())).flat_map(|i| message_boxes(layout, i, badge, font_size)).collect();
        if let DiagramData::Sequence(seq) = &layout.diagram {
            for (j, n) in seq.notes.iter().enumerate() {
                // A note at a section's first index is stacked before its divider: the section before's.
                let here = (n.index > from && n.index <= to) || (n.index == from && from == start);
                if here && inside_note(j, n.index) {
                    boxes.extend(note_box(seq, j));
                }
            }
        }
        if with_inner {
            boxes.extend(inner.iter().filter_map(|&e| frame_rect(layout, e)));
        }
        boxes
    };
    let Some(here) = frame_rect(layout, d) else { return };
    // What comes before it, where it is across.
    let mut before: Vec<Rect> = (0..start.min(layout.edges.len())).flat_map(|i| message_boxes(layout, i, badge, font_size)).collect();
    if let DiagramData::Sequence(seq) = &layout.diagram {
        for (j, n) in seq.notes.iter().enumerate() {
            if before_note(j, n.index) {
                before.extend(note_box(seq, j));
            }
        }
    }
    before.extend(earlier.iter().filter_map(|&e| frame_rect(layout, e)));
    before.retain(|b| meet_across(b, &here));
    let first = content(layout, start, end, true);
    if first.is_empty() {
        return;
    }
    // A condition taller than the tag (wrapped) hung above the frame's top: below it now.
    let conditioned = frame.sections.first().is_some_and(|s| s.label.is_some());
    if conditioned
        && let DiagramData::Sequence(seq) = &mut layout.diagram
        && let Some(f) = seq.frames.get_mut(d)
        && let Some(label) = f.section_labels.first_mut()
    {
        label.y = label.y.max(f.y + label.text.height / 2.0 + 2.0);
    }
    let title = title_height(layout, d, conditioned);
    let mut want = top(&first) - title - TITLE_GAP;
    let floor = if before.is_empty() { f32::MIN } else { bottom(&before) + GAP };
    if want < floor {
        let from = first_row(layout, |row, index| match row {
            Row::Message(i) => i >= start,
            Row::Note(j) => index > start || (index == start && inside_note(j, index)),
        });
        push_down(layout, from, top(&first) - 0.5, floor - want);
        want = floor;
    }
    move_top(layout, d, want, conditioned);
    // Each section after the first: its divider clear of the section before, its title clear of
    // its messages.
    for (k, pair) in frame.sections.windows(2).enumerate() {
        let [prev, section] = pair else { continue };
        let earlier_part = content(layout, prev.start_idx, prev.end_idx, false);
        // The section's rows: its messages, and the notes after its first.
        let from = first_row(layout, |row, index| match row {
            Row::Message(i) => i >= section.start_idx,
            Row::Note(_) => index > section.start_idx,
        });
        if let Some(divider) = divider(layout, d, k) {
            let need = if earlier_part.is_empty() { 0.0 } else { bottom(&earlier_part) + GAP - divider };
            if need > 0.0 {
                push_down(layout, from, divider - 0.5, need);
            }
        }
        let part = content(layout, section.start_idx, section.end_idx, false);
        // Only sections with a condition have a title: its place among those. A condition taller
        // than a line (wrapped) hung over the divider: below it now.
        let titled = section.label.is_some().then(|| frame.sections.iter().take(k + 1).filter(|s| s.label.is_some()).count());
        if let (Some(t), Some(line)) = (titled, divider(layout, d, k))
            && let DiagramData::Sequence(seq) = &mut layout.diagram
            && let Some(label) = seq.frames.get_mut(d).and_then(|f| f.section_labels.get_mut(t))
        {
            label.y = label.y.max(line + label.text.height / 2.0 + 3.0);
        }
        if let Some(label) = titled.and_then(|t| section_label(layout, d, t))
            && !part.is_empty()
        {
            let need = label[3] + TITLE_GAP - top(&part);
            if need > 0.0 {
                push_down(layout, from, top(&part) - 0.5, need);
            }
        }
    }
    // The bottom just below the last of it; a note written after it below that.
    let all = content(layout, start, end, true);
    widen(layout, d, &all, conditioned);
    let pad = (font_size * 0.85).max(12.0);
    let old_bottom = frame_rect(layout, d).map_or(f32::MIN, |r| r[3]);
    let new_bottom = bottom(&all) + pad;
    if !all.is_empty() {
        // Lower than it was (round a frame inside ending with it): what follows moves down too.
        if new_bottom > old_bottom {
            let from = first_row(layout, |row, index| match row {
                Row::Message(i) => i >= end,
                Row::Note(j) => index > end || (index == end && !inside_note(j, index)),
            });
            push_down(layout, from, bottom(&all) + 0.01, new_bottom - old_bottom);
        }
        if let DiagramData::Sequence(seq) = &mut layout.diagram
            && let Some(f) = seq.frames.get_mut(d)
        {
            f.height = new_bottom - f.y;
        }
    }
    let frame_bottom = frame_rect(layout, d).map_or(f32::MIN, |r| r[3]);
    let after: Vec<Rect> = match &layout.diagram {
        DiagramData::Sequence(seq) => seq
            .notes
            .iter()
            .enumerate()
            .filter(|(j, n)| n.index == end && places.get(*j).is_some_and(|p| p.follows.contains(&source_index)))
            .filter_map(|(j, _)| note_box(seq, j))
            .collect(),
        _ => Vec::new(),
    };
    if !after.is_empty() && top(&after) < frame_bottom + GAP {
        let from = first_row(layout, |row, index| match row {
            Row::Message(i) => i >= end,
            Row::Note(j) => index > end || (index == end && places.get(j).is_some_and(|p| p.follows.contains(&source_index))),
        });
        push_down(layout, from, top(&after) - 0.5, frame_bottom + GAP - top(&after));
    }
}

/// Frame `d` as wide as what is in it (`inside`: its messages' labels, its notes), as Mermaid's
/// frames hold their text: a label reaching past the lanes it joins stuck out of the frame.
fn widen(layout: &mut Layout, d: usize, inside: &[Rect], conditioned: bool) {
    // Within the drawing, which the renderer made as wide as the labels.
    let left = (inside.iter().map(|b| b[0]).fold(f32::MAX, f32::min) - GAP).max(1.0);
    let right = (inside.iter().map(|b| b[2]).fold(f32::MIN, f32::max) + GAP).min(layout.width - 1.0);
    let DiagramData::Sequence(seq) = &mut layout.diagram else { return };
    let Some(f) = seq.frames.get_mut(d) else { return };
    if inside.is_empty() {
        return;
    }
    if left < f.x {
        let dl = f.x - left;
        f.x -= dl;
        f.width += dl;
        f.label_box.0 -= dl;
        f.label.x -= dl;
        if conditioned && let Some(first) = f.section_labels.first_mut() {
            first.x -= dl;
        }
    }
    if right > f.x + f.width {
        f.width = right - f.x;
    }
}

/// The height of frame `d`'s title row: its kind's tag, and its first section's condition if it
/// has one.
fn title_height(layout: &Layout, d: usize, conditioned: bool) -> f32 {
    let DiagramData::Sequence(seq) = &layout.diagram else { return 0.0 };
    let Some(f) = seq.frames.get(d) else { return 0.0 };
    let tag = f.label_box.1 + f.label_box.3;
    let condition = f.section_labels.first().filter(|_| conditioned).map_or(f32::MIN, |l| l.y + l.text.height / 2.0);
    tag.max(condition) - f.y
}

/// The divider before section `k + 1` of frame `d`.
fn divider(layout: &Layout, d: usize, k: usize) -> Option<f32> {
    let DiagramData::Sequence(seq) = &layout.diagram else { return None };
    seq.frames.get(d)?.dividers.get(k).copied()
}

/// The box of section `k`'s condition in frame `d`.
fn section_label(layout: &Layout, d: usize, k: usize) -> Option<Rect> {
    let DiagramData::Sequence(seq) = &layout.diagram else { return None };
    let l = seq.frames.get(d)?.section_labels.get(k)?;
    Some([l.x - l.text.width / 2.0, l.y - l.text.height / 2.0, l.x + l.text.width / 2.0, l.y + l.text.height / 2.0])
}

/// Moves frame `d`'s top (and its title: its tag, and its first section's condition when it has
/// one) to `y`, its bottom staying.
fn move_top(layout: &mut Layout, d: usize, y: f32, conditioned: bool) {
    let DiagramData::Sequence(seq) = &mut layout.diagram else { return };
    let Some(f) = seq.frames.get_mut(d) else { return };
    let delta = y - f.y;
    f.y += delta;
    f.height -= delta;
    f.label_box.1 += delta;
    f.label.y += delta;
    if conditioned && let Some(first) = f.section_labels.first_mut() {
        first.y += delta;
    }
}

/// Opens the drawing: the rows from the `from`th on (as [`rows`] lists them) move down by `dy`,
/// those before staying whatever their height (an arrow to its own lane reaches below its row's
/// line); the rest moves if it is below `y0`, and grows if it spans it (a frame, a lifeline, an
/// activation).
fn push_down(layout: &mut Layout, from: usize, y0: f32, dy: f32) {
    if dy <= 0.0 || !dy.is_finite() {
        return;
    }
    let shift = |y: &mut f32| {
        if *y >= y0 {
            *y += dy;
        }
    };
    let moving: Vec<Row> = rows(layout).into_iter().skip(from).collect();
    let numbered = matches!(&layout.diagram, DiagramData::Sequence(seq) if seq.numbers.len() == layout.edges.len());
    for row in &moving {
        match *row {
            Row::Message(i) => {
                if let Some(edge) = layout.edges.get_mut(i) {
                    for p in &mut edge.points {
                        p.1 += dy;
                    }
                    for anchor in [&mut edge.label_anchor, &mut edge.start_label_anchor, &mut edge.end_label_anchor].into_iter().flatten() {
                        anchor.1 += dy;
                    }
                }
                if numbered
                    && let DiagramData::Sequence(seq) = &mut layout.diagram
                    && let Some(n) = seq.numbers.get_mut(i)
                {
                    n.y += dy;
                }
            }
            Row::Note(j) => {
                if let DiagramData::Sequence(seq) = &mut layout.diagram
                    && let Some(n) = seq.notes.get_mut(j)
                {
                    n.y += dy;
                }
            }
        }
    }
    for node in layout.nodes.values_mut() {
        shift(&mut node.y);
    }
    if let DiagramData::Sequence(seq) = &mut layout.diagram {
        for f in &mut seq.frames {
            if f.y >= y0 {
                f.y += dy;
                f.label_box.1 += dy;
                f.label.y += dy;
            } else if f.y + f.height >= y0 {
                f.height += dy;
            }
            for l in &mut f.section_labels {
                shift(&mut l.y);
            }
            for divider in &mut f.dividers {
                shift(divider);
            }
        }
        if !numbered {
            for n in &mut seq.numbers {
                shift(&mut n.y);
            }
        }
        for a in &mut seq.activations {
            if a.y >= y0 {
                a.y += dy;
            } else if a.y + a.height >= y0 {
                a.height += dy;
            }
        }
        for b in &mut seq.boxes {
            if b.y >= y0 {
                b.y += dy;
            } else if b.y + b.height >= y0 {
                b.height += dy;
            }
        }
        for l in &mut seq.lifelines {
            shift(&mut l.y1);
            shift(&mut l.y2);
        }
        for f in &mut seq.footboxes {
            shift(&mut f.y);
        }
    }
    layout.height += dy;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mermaid_rs_renderer::RenderOptions;

    /// The diagram laid out as it is drawn, with its frames, notes and numbers.
    fn laid(source: &str) -> (Layout, Graph) {
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source).unwrap_or_else(|e| panic!("{e}")).graph;
        let (layout, _, _) = super::super::laid_out(source, &RenderOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        (layout, graph)
    }

    fn seq(layout: &Layout) -> &SequenceData {
        match &layout.diagram {
            DiagramData::Sequence(seq) => seq,
            _ => panic!("not a sequence diagram"),
        }
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a[0] < b[2] - 0.5 && b[0] < a[2] - 0.5 && a[1] < b[3] - 0.5 && b[1] < a[3] - 0.5
    }

    /// The title row of each frame: its tag and its first condition.
    fn titles(layout: &Layout) -> Vec<Rect> {
        seq(layout)
            .frames
            .iter()
            .map(|f| {
                let (x, y, w, h) = f.label_box;
                let mut r = [x, y, x + w, y + h];
                if let Some(l) = f.section_labels.first() {
                    r = [
                        r[0].min(l.x - l.text.width / 2.0),
                        r[1].min(l.y - l.text.height / 2.0),
                        r[2].max(l.x + l.text.width / 2.0),
                        r[3].max(l.y + l.text.height / 2.0),
                    ];
                }
                r
            })
            .collect()
    }

    const NESTED: &str = "sequenceDiagram\n    autonumber\n    participant L as 연계 서버\n    participant C as 변환기\n    participant J as 주소 API\n    participant D as 자료\n    D-->>L: 송신 대상 목록\n    loop 기관별(A, B) 각각\n        L->>C: 행 만들기(dto, config)\n        Note over C: 전화번호 형식,<br/>기관별 필드 맞춤\n        opt B 인 경우\n            C->>J: 우편번호로 주소 조회\n            J-->>C: 시도/시군구\n        end\n        C-->>L: 변환된 행\n        L->>L: 파일 작성<br/>→ ZIP 압축\n        L->>D: 송신 결과\n        alt 성공\n            L->>D: 결과 갱신<br/>(TRSM_YN='Y',<br/>TRSM_YMD)\n        else 실패: 연결이 끊기거나 응답이 늦은 경우\n            L->>D: 실패 갱신\n        end\n    end\n    L->>L: 정리<br/>임시 파일 지우기";

    #[test]
    fn frame_titles_meet_nothing() {
        let (layout, _) = laid(NESTED);
        let s = seq(&layout);
        let titles = titles(&layout);
        assert_eq!(titles.len(), 3);
        // No two titles meet, and none meets a message or a note.
        for (i, a) in titles.iter().enumerate() {
            for b in titles.iter().skip(i + 1) {
                assert!(!overlap(a, b), "titles {a:?} and {b:?}");
            }
            for m in 0..layout.edges.len() {
                for b in message_boxes(&layout, m, 6.3, 14.0) {
                    assert!(!overlap(a, &b), "title {a:?} over message {m} {b:?}");
                }
            }
            for n in &s.notes {
                assert!(!overlap(a, &[n.x, n.y, n.x + n.width, n.y + n.height]), "title {a:?} over a note");
            }
        }
        // The note before `opt` is outside it, above it.
        let note = s.notes.first().map(|n| n.y + n.height).unwrap_or_default();
        let opt = s.frames.iter().find(|f| f.kind == mermaid_rs_renderer::ir::SequenceFrameKind::Opt).map(|f| f.y).unwrap_or_default();
        assert!(opt > note, "opt starts below the note before it");
        // A condition under its divider.
        let alt = s.frames.iter().find(|f| f.kind == mermaid_rs_renderer::ir::SequenceFrameKind::Alt);
        if let Some(alt) = alt
            && let (Some(&line), Some(l)) = (alt.dividers.first(), alt.section_labels.get(1))
        {
            assert!(l.y - l.text.height / 2.0 >= line, "else's condition below its divider");
        }
    }

    #[test]
    fn rows_meet_no_row_above_and_the_feet_stay_clear() {
        let (layout, _) = laid(NESTED);
        let rows = rows(&layout);
        let boxes: Vec<Vec<Rect>> = rows.iter().map(|&r| row_boxes(&layout, r, 6.3, 14.0)).collect();
        for (k, here) in boxes.iter().enumerate() {
            for above in boxes.iter().take(k) {
                for a in above {
                    for b in here {
                        assert!(!(meet_across(a, b) && a[3] > b[1] + 0.5), "row {k} {b:?} meets {a:?} above it");
                    }
                }
            }
        }
        let lowest = boxes.iter().flatten().map(|b| b[3]).fold(f32::MIN, f32::max);
        let feet = seq(&layout).footboxes.iter().map(|f| f.y).fold(f32::MAX, f32::min);
        assert!(feet >= lowest + GAP, "the last arrow to its own lane ends above the feet");
        // Each frame holds its own and stays in the drawing.
        for f in &seq(&layout).frames {
            assert!(f.x >= 0.0 && f.x + f.width <= layout.width && f.y + f.height <= feet, "frame inside");
        }
    }

    #[test]
    fn a_note_first_in_a_frame_is_in_it() {
        let (layout, graph) = laid("sequenceDiagram\n    A->>B: 시작\n    loop 매 분\n        Note over A: 확인\n        A->>B: 묻기\n    end");
        let places = note_places("sequenceDiagram\n    A->>B: 시작\n    loop 매 분\n        Note over A: 확인\n        A->>B: 묻기\n    end", &graph);
        assert!(places.is_some_and(|p| p.first().is_some_and(|n| n.opens == vec![0])));
        let s = seq(&layout);
        let (Some(note), Some(frame)) = (s.notes.first(), s.frames.first()) else { panic!("a note and a frame") };
        let title = titles(&layout).first().map_or(0.0, |t| t[3]);
        assert!(frame.y < note.y && title <= note.y, "the frame's title above the note in it");
        // The same note before the frame stays out of it.
        let (layout, _) = laid("sequenceDiagram\n    A->>B: 시작\n    Note over A: 확인\n    loop 매 분\n        A->>B: 묻기\n    end");
        let s = seq(&layout);
        let (Some(note), Some(frame)) = (s.notes.first(), s.frames.first()) else { panic!("a note and a frame") };
        assert!(frame.y > note.y + note.height, "the frame below the note before it");
    }

    #[test]
    fn a_source_read_otherwise_leaves_notes_before_frames() {
        let graph = mermaid_rs_renderer::parse_mermaid_strict("sequenceDiagram\n    A->>B: 하나\n    Note over A: 메모")
            .unwrap_or_else(|e| panic!("{e}"))
            .graph;
        assert!(note_places("sequenceDiagram\n    A->>B: 하나\n    A->>B: 둘\n    Note over A: 메모", &graph).is_none());
    }
}
