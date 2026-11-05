//! A flowchart laid out in layers, as Mermaid's own renderer (dagre) lays one out: ranks along the
//! flow, nodes ordered within their rank to cross few arrows and lined up with the nodes they link
//! to, and room between ranks for the arrows' labels. An arrow leaves the side of its node, bends
//! once, and carries its label on a straight stretch; an arrow back against the flow goes round
//! below (or beside) the nodes it passes.
//!
//! mermaid-rs-renderer measures the nodes and labels and draws the result: only the positions are
//! this module's. A flowchart it can't lay out this way (subgroups, an arrow to its own node, a
//! label at an arrow's end) keeps the renderer's own layout.

use std::collections::HashMap;

use mermaid_rs_renderer::{Direction, Graph, Layout, NodeShape};

/// The space round the drawing.
const MARGIN: f32 = 8.0;
/// Between two nodes of a rank.
const NODE_GAP: f32 = 34.0;
/// Between a node and an arrow passing it in its rank, or two such arrows.
const PASS_GAP: f32 = 12.0;
/// Between two ranks, at least (Mermaid's `rankSpacing`).
const RANK_GAP: f32 = 50.0;
/// The arrow left showing on each side of a label on it.
const LABEL_MARGIN: f32 = 16.0;
/// How far from a node an arrow bends.
const JOG: f32 = 18.0;
/// How much further each next arrow of a node bends, so arrows meeting at a node don't overlap.
const STAGGER: f32 = 8.0;
/// The most arrows of a node staggered; the rest bend with the last.
const STAGGER_MAX: usize = 4;
/// Below the nodes, between the lanes of arrows going back.
const LANE_GAP: f32 = 24.0;
/// The most nodes laid out this way.
const MAX_NODES: usize = 200;
/// Passes of ordering and of lining up.
const ORDER_PASSES: usize = 8;
const ALIGN_PASSES: usize = 12;

/// A point or a size along the flow (`main`) and across it (`cross`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct At {
    main: f32,
    cross: f32,
}

fn at(main: f32, cross: f32) -> At {
    At { main, cross }
}

/// A node of the layered graph: a flowchart node, or a stand-in where an arrow crosses a rank.
#[derive(Clone, Debug)]
struct Item {
    size: At,
    rank: usize,
    /// Its centre's place across the flow.
    cross: f32,
    /// Ports can spread along its side (a box); other shapes take their arrows at the middle.
    spreads: bool,
    stand_in: bool,
}

/// An arrow: the items it goes through in rank order (two for neighbouring ranks), its label's
/// size, and whether it goes back against the flow.
#[derive(Clone, Debug)]
struct Arrow {
    path: Vec<usize>,
    label: Option<At>,
    back: bool,
}

/// The layered graph being laid out.
struct Layers {
    items: Vec<Item>,
    arrows: Vec<Arrow>,
    ranks: Vec<Vec<usize>>,
}

/// Lays out `layout` (the renderer's, for `graph`) in layers. `false` leaves it as it was.
pub(super) fn relayout(graph: &Graph, layout: &mut Layout) -> bool {
    let Some(ids) = node_ids(graph, layout) else { return false };
    let horizontal = matches!(graph.direction, Direction::LeftRight | Direction::RightLeft);
    let Some(mut layers) = build(graph, layout, &ids, horizontal) else { return false };
    order(&mut layers);
    line_up(&mut layers);
    let mains = rank_mains(&layers);
    let mut routes = Vec::with_capacity(layers.arrows.len());
    let mut lanes = 0;
    for i in 0..layers.arrows.len() {
        routes.push(route(&layers, &mains, i, &mut lanes));
    }
    write(layout, &ids, &layers, &mains, &routes, graph.direction);
    true
}

/// The flowchart's node ids in the order they were written, when this layout can take the graph.
fn node_ids(graph: &Graph, layout: &Layout) -> Option<Vec<String>> {
    let simple = graph.subgraphs.is_empty()
        && graph.nodes.len() <= MAX_NODES
        && layout.edges.len() == graph.edges.len()
        && graph
            .edges
            .iter()
            .zip(&layout.edges)
            .all(|(e, l)| e.from != e.to && e.from == l.from && e.to == l.to && e.start_label.is_none() && e.end_label.is_none())
        && layout.nodes.values().all(|n| !n.hidden)
        && graph.nodes.keys().all(|id| layout.nodes.contains_key(id));
    if !simple || graph.nodes.is_empty() {
        return None;
    }
    let mut ids: Vec<String> = graph.nodes.keys().cloned().collect();
    ids.sort_by_key(|id| (graph.node_order.get(id).copied().unwrap_or(usize::MAX), id.clone()));
    Some(ids)
}

/// The size of `w` × `h` along and across the flow.
fn oriented(w: f32, h: f32, horizontal: bool) -> At {
    if horizontal { at(w, h) } else { at(h, w) }
}

/// The layered graph: ranks from the arrows (those closing a loop turned round), and stand-ins
/// where an arrow spans several ranks.
fn build(graph: &Graph, layout: &Layout, ids: &[String], horizontal: bool) -> Option<Layers> {
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        let n = layout.nodes.get(id)?;
        let spreads = matches!(n.shape, NodeShape::Rectangle | NodeShape::RoundRect | NodeShape::Subroutine);
        items.push(Item { size: oriented(n.width, n.height, horizontal), rank: 0, cross: 0.0, spreads, stand_in: false });
    }
    let mut pairs = Vec::with_capacity(graph.edges.len());
    for e in &graph.edges {
        pairs.push((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?));
    }
    let back = loops(items.len(), &pairs);
    let ranks = ranking(items.len(), &pairs, &back)?;
    for (item, rank) in items.iter_mut().zip(&ranks) {
        item.rank = *rank;
    }
    let mut arrows = Vec::with_capacity(pairs.len());
    for (i, &(from, to)) in pairs.iter().enumerate() {
        let label = layout.edges.get(i).and_then(|e| e.label.as_ref()).map(|t| oriented(t.width, t.height, horizontal));
        let backwards = back.get(i).copied().unwrap_or(false);
        let mut path = vec![from];
        let (a, b) = (ranks.get(from).copied().unwrap_or(0), ranks.get(to).copied().unwrap_or(0));
        if !backwards {
            for rank in a + 1..b {
                items.push(Item { size: at(0.0, 2.0), rank, cross: 0.0, spreads: false, stand_in: true });
                path.push(items.len() - 1);
            }
        }
        path.push(to);
        arrows.push(Arrow { path, label, back: backwards || b <= a });
    }
    let count = items.iter().map(|i| i.rank).max().unwrap_or(0) + 1;
    let mut by_rank = vec![Vec::new(); count];
    for (i, item) in items.iter().enumerate() {
        if let Some(rank) = by_rank.get_mut(item.rank) {
            rank.push(i);
        }
    }
    Some(Layers { items, arrows, ranks: by_rank })
}

/// The arrows that close a loop, found depth first in the order the nodes were written.
fn loops(n: usize, pairs: &[(usize, usize)]) -> Vec<bool> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, &(from, _)) in pairs.iter().enumerate() {
        if let Some(list) = out.get_mut(from) {
            list.push(i);
        }
    }
    let mut back = vec![false; pairs.len()];
    // 0 unseen, 1 on the path being walked, 2 done.
    let mut state = vec![0u8; n];
    for start in 0..n {
        if state.get(start) != Some(&0) {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        if let Some(s) = state.get_mut(start) {
            *s = 1;
        }
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let Some(&edge) = out.get(node).and_then(|l| l.get(*next)) else {
                if let Some(s) = state.get_mut(node) {
                    *s = 2;
                }
                stack.pop();
                continue;
            };
            *next += 1;
            let to = pairs.get(edge).map_or(node, |p| p.1);
            match state.get(to) {
                Some(1) => {
                    if let Some(b) = back.get_mut(edge) {
                        *b = true;
                    }
                }
                Some(0) => {
                    if let Some(s) = state.get_mut(to) {
                        *s = 1;
                    }
                    stack.push((to, 0));
                }
                _ => {}
            }
        }
    }
    back
}

/// Each node's rank: one after its furthest predecessor (arrows closing a loop count turned
/// round), and a node nothing leads to just before the first node it leads to.
fn ranking(n: usize, pairs: &[(usize, usize)], back: &[bool]) -> Option<Vec<usize>> {
    let flow: Vec<(usize, usize)> = pairs.iter().zip(back).map(|(&(a, b), &turned)| if turned { (b, a) } else { (a, b) }).collect();
    let mut into = vec![0usize; n];
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &flow {
        *into.get_mut(b)? += 1;
        out.get_mut(a)?.push(b);
    }
    let mut rank = vec![0usize; n];
    let mut ready: Vec<usize> = (0..n).filter(|&i| into.get(i) == Some(&0)).collect();
    let mut sorted = Vec::with_capacity(n);
    while let Some(a) = ready.pop() {
        sorted.push(a);
        let r = rank.get(a).copied().unwrap_or(0);
        for &b in out.get(a)? {
            let rb = rank.get_mut(b)?;
            *rb = (*rb).max(r + 1);
            let left = into.get_mut(b)?;
            *left -= 1;
            if *left == 0 {
                ready.push(b);
            }
        }
    }
    if sorted.len() != n {
        return None;
    }
    let sources: Vec<bool> = (0..n).map(|i| !flow.iter().any(|&(_, b)| b == i)).collect();
    for &a in sorted.iter().rev() {
        let next = out.get(a)?.iter().filter_map(|&b| rank.get(b).copied()).min();
        if sources.get(a) == Some(&true)
            && let Some(next) = next
        {
            *rank.get_mut(a)? = next.saturating_sub(1);
        }
    }
    Some(rank)
}

/// The items next to `item` in the neighbouring rank (`before` the one before it, else after).
fn neighbours(layers: &Layers, item: usize, before: bool) -> Vec<usize> {
    let mut found = Vec::new();
    for arrow in layers.arrows.iter().filter(|a| !a.back) {
        for pair in arrow.path.windows(2) {
            if let [a, b] = *pair {
                if before && b == item {
                    found.push(a);
                } else if !before && a == item {
                    found.push(b);
                }
            }
        }
    }
    found
}

/// Orders each rank by the average place of its items' neighbours, sweeping down and up the ranks.
fn order(layers: &mut Layers) {
    for pass in 0..ORDER_PASSES {
        let down = pass % 2 == 0;
        let count = layers.ranks.len();
        for step in 1..count {
            let r = if down { step } else { count - 1 - step };
            let place: HashMap<usize, f32> = place_in_rank(layers, if down { r - 1 } else { r + 1 });
            let Some(rank) = layers.ranks.get(r).cloned() else { continue };
            let mut keyed: Vec<(f32, usize)> = rank
                .iter()
                .enumerate()
                .map(|(pos, &item)| {
                    let near: Vec<f32> = neighbours(layers, item, down).iter().filter_map(|n| place.get(n).copied()).collect();
                    let key = if near.is_empty() { pos as f32 } else { near.iter().sum::<f32>() / near.len() as f32 };
                    (key, item)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Some(slot) = layers.ranks.get_mut(r) {
                *slot = keyed.into_iter().map(|(_, item)| item).collect();
            }
        }
    }
}

/// The places of a rank's items, `0, 1, 2…`.
fn place_in_rank(layers: &Layers, r: usize) -> HashMap<usize, f32> {
    layers.ranks.get(r).map(|rank| rank.iter().enumerate().map(|(pos, &i)| (i, pos as f32)).collect()).unwrap_or_default()
}

/// The room between two neighbours of a rank.
fn gap(a: &Item, b: &Item) -> f32 {
    if a.stand_in || b.stand_in { PASS_GAP } else { NODE_GAP }
}

/// Places each rank's items across the flow, in their order, as near the middle of the items they
/// link to as the room between them lets: a node leading to two sits between them, a chain runs
/// straight.
fn line_up(layers: &mut Layers) {
    for r in 0..layers.ranks.len() {
        let wanted: Vec<f32> = vec![f32::NAN; layers.ranks.get(r).map_or(0, Vec::len)];
        settle(layers, r, &wanted);
    }
    for pass in 0..ALIGN_PASSES {
        let down = pass % 2 == 0;
        let count = layers.ranks.len();
        for step in 0..count {
            let r = if down { step } else { count - 1 - step };
            let Some(rank) = layers.ranks.get(r).cloned() else { continue };
            let wanted: Vec<f32> = rank
                .iter()
                .map(|&item| {
                    let mut near = neighbours(layers, item, true);
                    near.extend(neighbours(layers, item, false));
                    let places: Vec<f32> = near.iter().filter_map(|&n| layers.items.get(n)).map(|n| n.cross).collect();
                    if places.is_empty() { f32::NAN } else { places.iter().sum::<f32>() / places.len() as f32 }
                })
                .collect();
            settle(layers, r, &wanted);
        }
    }
}

/// Puts rank `r`'s items where `wanted` says (NaN: where they are), keeping their order and the
/// room between them, the whole rank moved to be nearest what was wanted.
fn settle(layers: &mut Layers, r: usize, wanted: &[f32]) {
    let Some(rank) = layers.ranks.get(r).cloned() else { return };
    let mut places: Vec<f32> = Vec::with_capacity(rank.len());
    let mut prev: Option<&Item> = None;
    for (pos, &i) in rank.iter().enumerate() {
        let Some(item) = layers.items.get(i) else { return };
        let want = wanted.get(pos).copied().filter(|w| w.is_finite()).unwrap_or(item.cross);
        let place = match (prev, places.last()) {
            (Some(p), Some(&last)) => want.max(last + p.size.cross / 2.0 + gap(p, item) + item.size.cross / 2.0),
            _ => want,
        };
        places.push(place);
        prev = Some(item);
    }
    let shift = rank
        .iter()
        .zip(&places)
        .enumerate()
        .map(|(pos, (&i, place))| {
            wanted.get(pos).copied().filter(|w| w.is_finite()).unwrap_or(layers.items.get(i).map_or(*place, |it| it.cross)) - place
        })
        .sum::<f32>()
        / places.len().max(1) as f32;
    for (&i, place) in rank.iter().zip(places) {
        if let Some(item) = layers.items.get_mut(i) {
            item.cross = place + shift;
        }
    }
}

/// Where each rank's middle is along the flow: each rank as deep as its deepest node, with room
/// after it for the labels of the arrows leaving it.
fn rank_mains(layers: &Layers) -> Vec<f32> {
    let count = layers.ranks.len();
    let depth: Vec<f32> = (0..count)
        .map(|r| layers.ranks.get(r).map_or(0.0, |rank| rank.iter().filter_map(|&i| layers.items.get(i)).map(|i| i.size.main).fold(0.0, f32::max)))
        .collect();
    let mut room = vec![RANK_GAP; count];
    for arrow in layers.arrows.iter().filter(|a| !a.back) {
        if let (Some(label), Some(&first)) = (arrow.label, arrow.path.first()) {
            let r = layers.items.get(first).map_or(0, |i| i.rank);
            let siblings = layers.arrows.iter().filter(|a| !a.back && a.path.first() == Some(&first)).count();
            let stagger = STAGGER * siblings.saturating_sub(1).min(STAGGER_MAX) as f32;
            if let Some(slot) = room.get_mut(r) {
                *slot = slot.max(label.main + 2.0 * LABEL_MARGIN + JOG + stagger);
            }
        }
    }
    let mut mains = Vec::with_capacity(count);
    let mut edge = 0.0;
    for r in 0..count {
        let d = depth.get(r).copied().unwrap_or(0.0);
        mains.push(edge + d / 2.0);
        edge += d + room.get(r).copied().unwrap_or(RANK_GAP);
    }
    mains
}

/// An arrow's route in (main, cross) points, and its label's centre.
struct Route {
    points: Vec<At>,
    label: Option<At>,
}

/// Where on its side of `item` the `k`th of `n` arrows meets it, `k` counted across the flow.
fn port(item: &Item, k: usize, n: usize) -> f32 {
    if !item.spreads || n < 2 {
        return item.cross;
    }
    let step = (item.size.cross * 0.6 / (n - 1) as f32).min(18.0);
    item.cross + (k as f32 - (n - 1) as f32 / 2.0) * step
}

/// The port of arrow `i` on `item`: leaving it (`out`) or arriving, its place among the item's
/// arrows on that side set by where they come from or go.
fn port_of(layers: &Layers, item: usize, i: usize, out: bool) -> f32 {
    let mut ends: Vec<(f32, usize)> = Vec::new();
    for (j, arrow) in layers.arrows.iter().enumerate().filter(|(_, a)| !a.back) {
        let next = if out {
            (arrow.path.first() == Some(&item)).then(|| arrow.path.get(1)).flatten()
        } else {
            (arrow.path.last() == Some(&item)).then(|| arrow.path.iter().rev().nth(1)).flatten()
        };
        if let Some(cross) = next.and_then(|&n| layers.items.get(n)).map(|n| n.cross) {
            ends.push((cross, j));
        }
    }
    ends.sort_by(|a, b| a.0.total_cmp(&b.0));
    let k = ends.iter().position(|e| e.1 == i).unwrap_or(0);
    layers.items.get(item).map_or(0.0, |it| port(it, k, ends.len()))
}

/// The points of arrow `i`, with its label's centre; `lanes` counts the lanes taken by arrows going
/// back.
fn route(layers: &Layers, mains: &[f32], i: usize, lanes: &mut usize) -> Route {
    let Some(arrow) = layers.arrows.get(i) else { return Route { points: Vec::new(), label: None } };
    if arrow.back {
        return back_route(layers, mains, arrow, lanes);
    }
    let main_of = |item: usize| layers.items.get(item).and_then(|it| mains.get(it.rank)).copied().unwrap_or(0.0);
    let half = |item: usize| layers.items.get(item).map_or(0.0, |it| it.size.main / 2.0);
    let (Some(&first), Some(&last)) = (arrow.path.first(), arrow.path.last()) else { return Route { points: Vec::new(), label: None } };
    let start = at(main_of(first) + half(first), port_of(layers, first, i, true));
    let mut end = at(main_of(last) - half(last), port_of(layers, last, i, false));
    // A last bend smaller than a corner reads as a kink: the arrow comes in straight, still on the box.
    if let Some(target) = layers.items.get(last)
        && arrow.path.len() == 2
        && (start.cross - end.cross).abs() < 8.0
        && (start.cross - target.cross).abs() < target.size.cross / 2.0 - 4.0
    {
        end.cross = start.cross;
    }
    let mut stops: Vec<At> = arrow
        .path
        .iter()
        .skip(1)
        .take(arrow.path.len().saturating_sub(2))
        .filter_map(|&d| layers.items.get(d).map(|it| at(main_of(d), it.cross)))
        .collect();
    stops.push(end);
    let (outs, out_k) = fan(layers, first, i, true);
    let (ins, in_k) = fan(layers, last, i, false);
    let last_stop = stops.len().saturating_sub(1);
    let mut points = vec![start];
    let mut label = None;
    for (n, mut stop) in stops.into_iter().enumerate() {
        let from = points.last().copied().unwrap_or(start);
        // A stand-in is this arrow's alone: one nearly in line with the arrow so far takes its line,
        // rather than a kink smaller than a corner.
        if n < last_stop && (stop.cross - from.cross).abs() < PASS_GAP - 2.0 {
            stop.cross = from.cross;
        }
        let labelled = if n == 0 { arrow.label } else { None };
        let bend = match (labelled, n == 0, n == last_stop) {
            (Some(_), _, _) if ins > 1 && outs < 2 => Bend::End(staggered(in_k)),
            (Some(_), _, _) => Bend::Start(staggered(out_k)),
            (None, _, true) if ins > 1 => Bend::End(staggered(in_k)),
            (None, true, _) if outs > 1 => Bend::Start(staggered(out_k)),
            _ => Bend::Middle,
        };
        let (segment, middle) = elbow(from, stop, bend);
        points.extend(segment);
        if labelled.is_some() {
            label = Some(middle);
        }
    }
    Route { points, label }
}

/// How far from its node the `k`th arrow of a fan bends.
fn staggered(k: usize) -> f32 {
    JOG + STAGGER * k.min(STAGGER_MAX) as f32
}

/// How many arrows leave `item` (`out`) or arrive at it, and arrow `i`'s place among them: the
/// arrow going furthest across the flow first, as it bends nearest the node and so crosses none
/// of the others.
fn fan(layers: &Layers, item: usize, i: usize, out: bool) -> (usize, usize) {
    let mut reach: Vec<(f32, usize)> = Vec::new();
    for (j, arrow) in layers.arrows.iter().enumerate().filter(|(_, a)| !a.back) {
        let other = if out {
            (arrow.path.first() == Some(&item)).then(|| arrow.path.get(1)).flatten()
        } else {
            (arrow.path.last() == Some(&item)).then(|| arrow.path.iter().rev().nth(1)).flatten()
        };
        if let Some(cross) = other.and_then(|&o| layers.items.get(o)).map(|o| o.cross) {
            let here = port_of(layers, item, j, out);
            reach.push(((cross - here).abs(), j));
        }
    }
    reach.sort_by(|a, b| b.0.total_cmp(&a.0));
    (reach.len(), reach.iter().position(|r| r.1 == i).unwrap_or(0))
}

/// Where an arrow between two ranks bends: this far after its start, this far before its end, or
/// halfway.
#[derive(Clone, Copy, PartialEq)]
enum Bend {
    Start(f32),
    End(f32),
    Middle,
}

/// The points after `a` of a segment to `b`, bending once across the flow, and the middle of its
/// longest straight run along the flow (where a label goes).
fn elbow(a: At, b: At, bend: Bend) -> (Vec<At>, At) {
    if (a.cross - b.cross).abs() < 0.5 {
        return (vec![b], at((a.main + b.main) / 2.0, a.cross));
    }
    let room = (b.main - a.main).max(0.0);
    let turn = match bend {
        Bend::Start(d) => a.main + d.min(room / 2.0),
        Bend::End(d) => b.main - d.min(room / 2.0),
        Bend::Middle => (a.main + b.main) / 2.0,
    };
    let middle = if matches!(bend, Bend::End(_)) { at((a.main + turn) / 2.0, a.cross) } else { at((turn + b.main) / 2.0, b.cross) };
    (vec![at(turn, a.cross), at(turn, b.cross), b], middle)
}

/// An arrow against the flow: out of the far side of its start, along a lane beyond every node it
/// passes, and into the far side of its end.
fn back_route(layers: &Layers, mains: &[f32], arrow: &Arrow, lanes: &mut usize) -> Route {
    let (Some(a), Some(b)) = (arrow.path.first().and_then(|&i| layers.items.get(i)), arrow.path.last().and_then(|&i| layers.items.get(i))) else {
        return Route { points: Vec::new(), label: None };
    };
    let (lo, hi) = (a.rank.min(b.rank), a.rank.max(b.rank));
    let beyond = layers
        .items
        .iter()
        .filter(|it| !it.stand_in && it.rank >= lo && it.rank <= hi)
        .map(|it| it.cross + it.size.cross / 2.0)
        .fold(f32::MIN, f32::max);
    *lanes += 1;
    let label_room = arrow.label.map_or(0.0, |l| l.cross / 2.0);
    let lane = beyond + LANE_GAP * *lanes as f32 + label_room;
    let (ma, mb) = (mains.get(a.rank).copied().unwrap_or(0.0), mains.get(b.rank).copied().unwrap_or(0.0));
    let points = vec![at(ma, a.cross + a.size.cross / 2.0), at(ma, lane), at(mb, lane), at(mb, b.cross + b.size.cross / 2.0)];
    Route { points, label: arrow.label.map(|_| at((ma + mb) / 2.0, lane)) }
}

/// An arrow's points and its label's centre, in x and y.
type Placed = (Vec<(f32, f32)>, Option<(f32, f32)>);

/// The point in the drawing's own x and y, before it is moved into its margin.
fn to_xy(p: At, direction: Direction) -> (f32, f32) {
    match direction {
        Direction::LeftRight => (p.main, p.cross),
        Direction::RightLeft => (-p.main, p.cross),
        Direction::TopDown => (p.cross, p.main),
        Direction::BottomTop => (p.cross, -p.main),
    }
}

/// Writes the positions into the renderer's layout, the drawing moved into its margin.
fn write(layout: &mut Layout, ids: &[String], layers: &Layers, mains: &[f32], routes: &[Route], direction: Direction) {
    let mut boxes: Vec<[f32; 4]> = Vec::new();
    let mut centres: Vec<(f32, f32)> = Vec::with_capacity(ids.len());
    for (id, item) in ids.iter().zip(&layers.items) {
        let (x, y) = to_xy(at(mains.get(item.rank).copied().unwrap_or(0.0), item.cross), direction);
        centres.push((x, y));
        if let Some(n) = layout.nodes.get(id) {
            boxes.push([x - n.width / 2.0, y - n.height / 2.0, x + n.width / 2.0, y + n.height / 2.0]);
        }
    }
    let routes_xy: Vec<Placed> =
        routes.iter().map(|r| (r.points.iter().map(|&p| to_xy(p, direction)).collect(), r.label.map(|p| to_xy(p, direction)))).collect();
    for ((points, label), edge) in routes_xy.iter().zip(&layout.edges) {
        boxes.extend(points.iter().map(|&(x, y)| [x, y, x, y]));
        if let (Some((x, y)), Some(t)) = (label, &edge.label) {
            boxes.push([x - t.width / 2.0, y - t.height / 2.0, x + t.width / 2.0, y + t.height / 2.0]);
        }
    }
    let left = boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min);
    let top = boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min);
    let (dx, dy) = (MARGIN - left, MARGIN - top);
    for (id, &(x, y)) in ids.iter().zip(&centres) {
        if let Some(n) = layout.nodes.get_mut(id) {
            n.x = x + dx - n.width / 2.0;
            n.y = y + dy - n.height / 2.0;
        }
    }
    for ((points, label), edge) in routes_xy.into_iter().zip(layout.edges.iter_mut()) {
        edge.points = points.into_iter().map(|(x, y)| (x + dx, y + dy)).collect();
        edge.label_anchor = label.map(|(x, y)| (x + dx, y + dy)).or(edge.label_anchor);
    }
    layout.width = boxes.iter().map(|b| b[2]).fold(0.0, f32::max) + dx + MARGIN;
    layout.height = boxes.iter().map(|b| b[3]).fold(0.0, f32::max) + dy + MARGIN;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The renderer's layout of `source`, then this module's.
    fn laid(source: &str) -> (Layout, bool) {
        let mut graph = mermaid_rs_renderer::parse_mermaid_strict(source).unwrap_or_else(|e| panic!("{source}: {e}")).graph;
        for edge in &mut graph.edges {
            if let Some(label) = edge.label.as_mut() {
                *label = super::super::unquote(label);
            }
        }
        let options = mermaid_rs_renderer::RenderOptions::default();
        let mut layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &options.layout);
        let ours = relayout(&graph, &mut layout);
        (layout, ours)
    }

    const SAMPLES: [&str; 5] = [
        "flowchart LR\n    U[\"업로드<br>로컬 NAS /share/files\"] -->|\"sendFile 배치<br>ECM 등록 · 로컬 삭제\"| E[\"ECM 문서\"]\n    E -->|\"ECM 자동 변환\"| P[\"PDF 변환본\"]\n    E -->|\"OCR 마스킹 배치<br>editDocument 버전업\"| M[\"마스킹본<br>ECM 최신 버전\"]\n    P -->|\"뷰어 주석 저장<br>changePdf\"| P2[\"주석 PDF\"]\n    M --> D1[\"downloadFile<br>/api/cmmn/file/{fileId}\"]\n    P2 --> D2[\"downloadPdfFile<br>/api/cmmn/file/{fileId}/pdf\"]",
        "flowchart LR\n    A[\"운영 레거시 DB\"] -->|\"1 시험 이관<br>공고 1개 사람 + 첨부 메타\"| B[\"개발 DB\"]\n    N[\"운영 레거시 NAS\"] -->|\"2 그 파일만 임시 복사\"| L[\"로컬 폴더\"]\n    B --> APP[\"3 로컬 isbm\"]\n    L --> APP\n    APP -->|\"4 해시 대조 · 이력 확인\"| X[\"5 같은 날 삭제\"]",
        "flowchart TD\n    A[시작] --> B{확인}\n    B -->|예| C[처리]\n    B -->|아니오| D[다시 보기]\n    D --> B",
        "flowchart TD\n    A((원)) --> B([둥근 모양])\n    A --> C{조건}\n    C -->|예| D[(DB)]\n    C -->|아니오| E>깃발]\n    D --> F[끝]\n    E --> F\n    B --> F",
        "flowchart RL\n    A[하나] -->|첫째| B[둘]\n    B --> C[셋]\n    A --> C",
    ];

    fn meet(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] < b[2] - 0.5 && b[0] < a[2] - 0.5 && a[1] < b[3] - 0.5 && b[1] < a[3] - 0.5
    }

    fn node_boxes(layout: &Layout) -> Vec<[f32; 4]> {
        layout.nodes.values().map(|n| [n.x, n.y, n.x + n.width, n.y + n.height]).collect()
    }

    #[test]
    fn nodes_and_labels_never_overlap_and_stay_in_the_drawing() {
        for source in SAMPLES {
            let (layout, ours) = laid(source);
            assert!(ours, "{source}: laid out in layers");
            let nodes = node_boxes(&layout);
            for (i, a) in nodes.iter().enumerate() {
                assert!(a[0] >= 0.0 && a[1] >= 0.0 && a[2] <= layout.width && a[3] <= layout.height, "{source}: node inside");
                for b in nodes.iter().skip(i + 1) {
                    assert!(!meet(*a, *b), "{source}: nodes {a:?} and {b:?} overlap");
                }
            }
            let labels: Vec<[f32; 4]> = layout
                .edges
                .iter()
                .filter_map(|e| Some((e.label.as_ref()?, e.label_anchor?)))
                .map(|(t, (x, y))| [x - t.width / 2.0, y - t.height / 2.0, x + t.width / 2.0, y + t.height / 2.0])
                .collect();
            for (i, l) in labels.iter().enumerate() {
                assert!(!nodes.iter().any(|n| meet(*l, *n)), "{source}: label {l:?} over a node");
                assert!(!labels.iter().skip(i + 1).any(|o| meet(*l, *o)), "{source}: labels overlap");
                assert!(l[0] >= 0.0 && l[1] >= 0.0 && l[2] <= layout.width && l[3] <= layout.height, "{source}: label inside");
            }
        }
    }

    /// The distance from a point to a polyline.
    fn off_line(p: (f32, f32), points: &[(f32, f32)]) -> f32 {
        points
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx * dx + dy * dy;
                let t = if len > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0) } else { 0.0 };
                ((a.0 + t * dx - p.0).powi(2) + (a.1 + t * dy - p.1).powi(2)).sqrt()
            })
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn a_label_sits_on_its_arrow_and_an_arrow_joins_its_nodes() {
        for source in SAMPLES {
            let (layout, _) = laid(source);
            for e in &layout.edges {
                assert!(e.points.len() >= 2, "{source}: {} -> {} has a route", e.from, e.to);
                if let Some(anchor) = e.label_anchor.filter(|_| e.label.is_some()) {
                    assert!(off_line(anchor, &e.points) < 0.5, "{source}: the label of {} -> {} is on it", e.from, e.to);
                }
                for (end, id) in [(e.points.first(), &e.from), (e.points.last(), &e.to)] {
                    let (Some(&(x, y)), Some(n)) = (end, layout.nodes.get(id)) else { continue };
                    let near = x >= n.x - 0.5 && x <= n.x + n.width + 0.5 && y >= n.y - 0.5 && y <= n.y + n.height + 0.5;
                    assert!(near, "{source}: {} -> {} ends on {id}", e.from, e.to);
                }
            }
        }
    }

    #[test]
    fn ranks_follow_the_arrows() {
        let (layout, _) = laid(SAMPLES[0]);
        let x = |id: &str| layout.nodes.get(id).map(|n| n.x).unwrap_or_default();
        assert!(x("U") < x("E") && x("E") < x("P") && x("P") < x("P2") && x("P2") < x("D2"), "left to right along the flow");
        let mid = |id: &str| layout.nodes.get(id).map(|n| n.x + n.width / 2.0).unwrap_or_default();
        assert!((mid("P") - mid("M")).abs() < 1.0, "the two ECM outcomes share a rank");
        let y = |id: &str| layout.nodes.get(id).map(|n| n.y + n.height / 2.0).unwrap_or_default();
        assert!(y("P") < y("E") && y("E") < y("M"), "the document sits between the two it leads to");
        assert!((y("U") - y("E")).abs() < 1.0, "a chain runs straight");
        let (td, _) = laid(SAMPLES[2]);
        let top = |id: &str| td.nodes.get(id).map(|n| n.y).unwrap_or_default();
        assert!(top("A") < top("B") && top("B") < top("C"), "top down");
    }

    #[test]
    fn what_it_cant_lay_out_keeps_the_renderers_layout() {
        for source in ["flowchart LR\n    subgraph S[묶음]\n    A --> B\n    end\n    B --> C", "flowchart LR\n    A --> A"] {
            assert!(!laid(source).1, "{source}");
        }
    }
}
