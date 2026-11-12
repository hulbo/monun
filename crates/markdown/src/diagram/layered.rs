//! A flowchart laid out in layers, as Mermaid's own renderer (dagre) lays one out: ranks along the
//! flow, nodes ordered within their rank to cross few arrows and lined up with the nodes they link
//! to, and room between ranks for the arrows' labels. An arrow leaves the side of its node, bends
//! once, and carries its label on a straight stretch; an arrow back against the flow goes round
//! below (or beside) the nodes it passes; an arrow to its own node loops out of its side.
//!
//! Subgroups (`subgraph … end`, nested too) are laid out with the rest: the whole flowchart takes
//! ranks together, a group's members stay side by side in each rank, with room where a rank passes
//! into or out of a group (and for its title), and whatever isn't a group's is moved out of its
//! frame, which then holds just its members.
//!
//! An arrow from or to a group (`daily ==> monthly`) is laid out as one from its last node or to its
//! first, and drawn from or to the group's frame.
//!
//! mermaid-rs-renderer measures the nodes and labels and draws the result: only the positions are
//! this module's. A flowchart it can't lay out this way (groups that overlap without one holding
//! the other, a label at an arrow's end, an arrow between a group and what is in it) keeps the
//! renderer's own layout.

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
/// Inside a group's frame, round its members.
const GROUP_PAD: f32 = 16.0;
/// A group's title: the renderer draws it 12 below the frame's top; this much more below it.
const TITLE_BELOW: f32 = 10.0;
/// Between two ranks with a group's frame between them and no label, outside the frames.
const FRAME_GAP: f32 = 28.0;
/// Passes moving what isn't a group's out of its frame.
const CLEAR_PASSES: usize = 24;
/// How far an arrow to its own node goes out of the node, and how far apart it leaves and comes back.
const LOOP_OUT: f32 = 22.0;
const LOOP_HALF: f32 = 20.0;
/// The most an arrow crossing ranks is moved across the flow to run on straight.
const STRAIGHTEN: f32 = 48.0;
/// Between an arrow to its own node and its label, beside it.
const LOOP_LABEL_GAP: f32 = 4.0;

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
    /// The groups it is in, outermost first.
    groups: Vec<usize>,
    /// Room kept after it across the flow (for an arrow looping out of it and its label).
    room: f32,
}

/// An arrow: the items it goes through in rank order (two for neighbouring ranks), its label's
/// size and the stand-in carrying it (else it is on the first stretch), whether it goes back
/// against the flow, and whether it goes to its own node.
#[derive(Clone, Debug)]
struct Arrow {
    path: Vec<usize>,
    label: Option<At>,
    label_at: Option<usize>,
    back: bool,
    looped: bool,
    /// The groups it leaves and enters by their frames (an arrow from or to a group), cut there.
    cut: [Option<usize>; 2],
}

/// A group of nodes: its members (items), the group holding it, its title's size (x by y) and
/// its frame once placed (main and cross, low and high).
#[derive(Clone, Debug)]
struct Group {
    members: Vec<usize>,
    parent: Option<usize>,
    title: (f32, f32),
    frame: [f32; 4],
}

/// Which side of a group's frame its title takes, by the flowchart's direction: the title is drawn
/// at the frame's top on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TitleSide {
    MainLow,
    MainHigh,
    CrossLow,
}

/// The layered graph being laid out.
struct Layers {
    items: Vec<Item>,
    arrows: Vec<Arrow>,
    ranks: Vec<Vec<usize>>,
    groups: Vec<Group>,
    title_side: TitleSide,
}

/// Lays out `layout` (the renderer's, for `graph`) in layers. `false` leaves it as it was.
pub(super) fn relayout(graph: &Graph, layout: &mut Layout) -> bool {
    let Some(ids) = node_ids(graph, layout) else { return false };
    let horizontal = matches!(graph.direction, Direction::LeftRight | Direction::RightLeft);
    let Some(mut layers) = build(graph, layout, &ids, horizontal) else { return false };
    order(&mut layers);
    line_up(&mut layers);
    let mains = rank_mains(&layers);
    clear_groups(&mut layers, &mains);
    let mut routes = Vec::with_capacity(layers.arrows.len());
    let mut lanes = 0;
    for (i, arrow) in layers.arrows.iter().enumerate() {
        let mut route = route(&layers, &mains, i, &mut lanes);
        cut(&mut route, &layers, arrow.cut);
        routes.push(route);
    }
    write(layout, &ids, &layers, &mains, &routes, graph.direction);
    true
}

/// The flowchart's node ids in the order they were written, when this layout can take the graph.
fn node_ids(graph: &Graph, layout: &Layout) -> Option<Vec<String>> {
    let simple = graph.nodes.len() <= MAX_NODES
        && layout.edges.len() == graph.edges.len()
        && graph
            .edges
            .iter()
            .zip(&layout.edges)
            .all(|(e, l)| e.from == l.from && e.to == l.to && e.start_label.is_none() && e.end_label.is_none())
        // Nothing hidden but a group's stand-in, for an arrow to the group.
        && layout.nodes.iter().all(|(id, n)| match n.anchor_subgraph {
            Some(_) => graph.subgraphs.iter().any(|s| s.id.as_deref() == Some(id.as_str())),
            None => !n.hidden,
        })
        && graph.nodes.keys().all(|id| layout.nodes.contains_key(id))
        && graph.subgraphs.iter().all(|s| !s.nodes.is_empty() && s.nodes.iter().all(|n| graph.nodes.contains_key(n)));
    if !simple {
        return None;
    }
    // Groups either hold one another or have no node in common.
    for (i, a) in graph.subgraphs.iter().enumerate() {
        for b in graph.subgraphs.iter().skip(i + 1) {
            let shared = a.nodes.iter().filter(|n| b.nodes.contains(n)).count();
            if shared > 0 && shared != a.nodes.len().min(b.nodes.len()) {
                return None;
            }
        }
    }
    // A group's stand-in is the group's frame, not a node.
    let mut ids: Vec<String> = graph.nodes.keys().filter(|id| layout.nodes.get(*id).is_some_and(|n| n.anchor_subgraph.is_none())).cloned().collect();
    ids.sort_by_key(|id| (graph.node_order.get(id).copied().unwrap_or(usize::MAX), id.clone()));
    (!ids.is_empty()).then_some(ids)
}

/// The size of `w` × `h` along and across the flow.
fn oriented(w: f32, h: f32, horizontal: bool) -> At {
    if horizontal { at(w, h) } else { at(h, w) }
}

/// The groups of the flowchart, each with the group holding it (the smallest larger one holding
/// all its nodes), and each node's groups, outermost first.
fn groups(graph: &Graph, layout: &Layout, index: &HashMap<&str, usize>) -> (Vec<Group>, Vec<Vec<usize>>) {
    let sets: Vec<Vec<usize>> = graph.subgraphs.iter().map(|s| s.nodes.iter().filter_map(|n| index.get(n.as_str()).copied()).collect()).collect();
    let mut groups: Vec<Group> = Vec::with_capacity(sets.len());
    for (g, members) in sets.iter().enumerate() {
        let parent = sets
            .iter()
            .enumerate()
            .filter(|(o, other)| *o != g && other.len() > members.len() && members.iter().all(|m| other.contains(m)))
            .min_by_key(|(_, other)| other.len())
            .map(|(o, _)| o);
        let title = layout
            .subgraphs
            .iter()
            .find(|s| s.nodes == graph.subgraphs.get(g).map(|sg| sg.nodes.clone()).unwrap_or_default())
            .filter(|s| !s.label.trim().is_empty())
            .map_or((0.0, 0.0), |s| (s.label_block.width, s.label_block.height));
        groups.push(Group { members: members.clone(), parent, title, frame: [0.0; 4] });
    }
    let mut of_node: Vec<Vec<usize>> = vec![Vec::new(); index.len()];
    for (g, group) in groups.iter().enumerate() {
        for &m in &group.members {
            if let Some(list) = of_node.get_mut(m) {
                list.push(g);
            }
        }
    }
    // Outermost first: a group holding another has more members.
    for list in &mut of_node {
        list.sort_by_key(|g| std::cmp::Reverse(groups.get(*g).map_or(0, |gr| gr.members.len())));
    }
    (groups, of_node)
}

/// The node standing for `group` at an arrow's end: of its nodes, the last no arrow leads from to
/// another of them (`from`, the arrow leaving the group), or the first none leads to.
fn standing_for(group: &Group, plain: &[(usize, usize)], from: bool) -> Option<usize> {
    let members = &group.members;
    let inner = |m: usize| plain.iter().any(|&(a, b)| a != b && if from { a == m && members.contains(&b) } else { b == m && members.contains(&a) });
    if from {
        members.iter().rev().find(|&&m| !inner(m)).or(members.last()).copied()
    } else {
        members.iter().find(|&&m| !inner(m)).or(members.first()).copied()
    }
}

/// The groups two lists of groups share, outermost first.
fn shared(a: &[usize], b: &[usize]) -> Vec<usize> {
    a.iter().zip(b).take_while(|(x, y)| x == y).map(|(x, _)| *x).collect()
}

/// The layered graph: ranks from the arrows (those closing a loop turned round), and stand-ins
/// where an arrow spans several ranks.
fn build(graph: &Graph, layout: &Layout, ids: &[String], horizontal: bool) -> Option<Layers> {
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let (groups, of_node) = groups(graph, layout, &index);
    let mut items = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        let n = layout.nodes.get(id)?;
        let spreads = matches!(n.shape, NodeShape::Rectangle | NodeShape::RoundRect | NodeShape::Subroutine);
        let groups = of_node.get(i).cloned().unwrap_or_default();
        items.push(Item { size: oriented(n.width, n.height, horizontal), rank: 0, cross: 0.0, spreads, stand_in: false, groups, room: 0.0 });
    }
    // An arrow to or from a group goes from or to a node of the group standing for it, the arrow
    // then cut at the group's frame.
    let group_ids: HashMap<&str, usize> = graph.subgraphs.iter().enumerate().filter_map(|(g, s)| Some((s.id.as_deref()?, g))).collect();
    let plain: Vec<(usize, usize)> = graph.edges.iter().filter_map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?))).collect();
    let end_of = |id: &str, from: bool| -> Option<(usize, Option<usize>)> {
        if let Some(&i) = index.get(id) {
            return Some((i, None));
        }
        let g = *group_ids.get(id)?;
        Some((standing_for(groups.get(g)?, &plain, from)?, Some(g)))
    };
    let mut pairs = Vec::with_capacity(graph.edges.len());
    let mut cuts = Vec::with_capacity(graph.edges.len());
    for e in &graph.edges {
        let ((from, from_group), (to, to_group)) = (end_of(&e.from, true)?, end_of(&e.to, false)?);
        // Between a group and what is in it there is no arrow from frame to frame.
        let holds = |g: Option<usize>, item: usize| g.and_then(|g| groups.get(g)).is_some_and(|g| g.members.contains(&item));
        if holds(from_group, to) || holds(to_group, from) {
            return None;
        }
        pairs.push((from, to));
        cuts.push([from_group, to_group]);
    }
    // Arrows to their own node take no part in the ranks.
    let ranked: Vec<(usize, usize)> = pairs.iter().copied().filter(|(a, b)| a != b).collect();
    let back_ranked = loops(items.len(), &ranked);
    let ranks = ranking(items.len(), &ranked, &back_ranked)?;
    for (item, rank) in items.iter_mut().zip(&ranks) {
        item.rank = *rank;
    }
    let mut back_of = back_ranked.into_iter();
    let mut arrows = Vec::with_capacity(pairs.len());
    for (i, &(from, to)) in pairs.iter().enumerate() {
        let label = layout.edges.get(i).and_then(|e| e.label.as_ref()).map(|t| oriented(t.width, t.height, horizontal));
        if from == to {
            if let Some(item) = items.get_mut(from) {
                item.room = item.room.max(LOOP_OUT + label.map_or(0.0, |l| LOOP_LABEL_GAP + l.cross));
            }
            arrows.push(Arrow { path: vec![from, from], label, label_at: None, back: false, looped: true, cut: [None; 2] });
            continue;
        }
        let backwards = back_of.next().unwrap_or(false);
        let mut path = vec![from];
        let (a, b) = (ranks.get(from).copied().unwrap_or(0), ranks.get(to).copied().unwrap_or(0));
        if !backwards {
            // A stand-in is in the groups both ends are in.
            let groups =
                shared(&items.get(from).map(|i| i.groups.clone()).unwrap_or_default(), &items.get(to).map(|i| i.groups.clone()).unwrap_or_default());
            for rank in a + 1..b {
                items.push(Item { size: at(0.0, 2.0), rank, cross: 0.0, spreads: false, stand_in: true, groups: groups.clone(), room: 0.0 });
                path.push(items.len() - 1);
            }
        }
        path.push(to);
        // An arrow crossing ranks carries its label on its middle stand-in, which takes its size so
        // the ranks keep room for it.
        let mut label_at = None;
        if let Some(size) = label
            && path.len() > 2
            && let Some(&middle) = path.get(path.len() / 2)
            && let Some(item) = items.get_mut(middle)
        {
            item.size = size;
            label_at = Some(middle);
        }
        let cut = cuts.get(i).copied().unwrap_or([None; 2]);
        arrows.push(Arrow { path, label, label_at, back: backwards || b <= a, looped: false, cut });
    }
    let count = items.iter().map(|i| i.rank).max().unwrap_or(0) + 1;
    let mut by_rank = vec![Vec::new(); count];
    for (i, item) in items.iter().enumerate() {
        if let Some(rank) = by_rank.get_mut(item.rank) {
            rank.push(i);
        }
    }
    let title_side = match graph.direction {
        Direction::TopDown => TitleSide::MainLow,
        Direction::BottomTop => TitleSide::MainHigh,
        Direction::LeftRight | Direction::RightLeft => TitleSide::CrossLow,
    };
    Some(Layers { items, arrows, ranks: by_rank, groups, title_side })
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
    for arrow in layers.arrows.iter().filter(|a| !a.back && !a.looped) {
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

/// Orders each rank by the average place of its items' neighbours, sweeping down and up the ranks;
/// a group's items stay together, the group placed by their average.
fn order(layers: &mut Layers) {
    for pass in 0..ORDER_PASSES {
        let down = pass % 2 == 0;
        let count = layers.ranks.len();
        for step in 1..count {
            let r = if down { step } else { count - 1 - step };
            let place: HashMap<usize, f32> = place_in_rank(layers, if down { r - 1 } else { r + 1 });
            let Some(rank) = layers.ranks.get(r).cloned() else { continue };
            let keyed: Vec<(f32, usize)> = rank
                .iter()
                .enumerate()
                .map(|(pos, &item)| {
                    let near: Vec<f32> = neighbours(layers, item, down).iter().filter_map(|n| place.get(n).copied()).collect();
                    let key = if near.is_empty() { pos as f32 } else { near.iter().sum::<f32>() / near.len() as f32 };
                    (key, item)
                })
                .collect();
            let sorted = by_groups(layers, keyed, 0);
            if let Some(slot) = layers.ranks.get_mut(r) {
                *slot = sorted;
            }
        }
    }
}

/// An item of a rank with the key it is ordered by.
type Keyed = (f32, usize);

/// `keyed` items sorted by key, the items of each group (at nesting `depth` and below) kept
/// together and the group sorted by its items' average key.
fn by_groups(layers: &Layers, keyed: Vec<Keyed>, depth: usize) -> Vec<usize> {
    // Blocks: one per group at this depth, one per item in none.
    let mut blocks: Vec<(Option<usize>, Vec<Keyed>)> = Vec::new();
    for (key, item) in keyed {
        let group = layers.items.get(item).and_then(|i| i.groups.get(depth)).copied();
        match blocks.iter_mut().find(|(g, _)| group.is_some() && *g == group) {
            Some((_, list)) => list.push((key, item)),
            None => blocks.push((group, vec![(key, item)])),
        }
    }
    let key = |list: &[Keyed]| list.iter().map(|(k, _)| k).sum::<f32>() / list.len().max(1) as f32;
    blocks.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
    let mut out = Vec::new();
    for (group, list) in blocks {
        if group.is_some() {
            out.extend(by_groups(layers, list, depth + 1));
        } else {
            out.extend(list.into_iter().map(|(_, i)| i));
        }
    }
    out
}

/// The places of a rank's items, `0, 1, 2…`.
fn place_in_rank(layers: &Layers, r: usize) -> HashMap<usize, f32> {
    layers.ranks.get(r).map(|rank| rank.iter().enumerate().map(|(pos, &i)| (i, pos as f32)).collect()).unwrap_or_default()
}

/// The room between two neighbours of a rank, `a` before `b`: more where they are on either side
/// of a group's frame (its padding, and its title when that is on the side `b` enters).
fn gap(layers: &Layers, a: &Item, b: &Item) -> f32 {
    let base = if a.stand_in || b.stand_in { PASS_GAP } else { NODE_GAP };
    let common = shared(&a.groups, &b.groups).len();
    let leaving = a.groups.len().saturating_sub(common);
    let entering: Vec<usize> = b.groups.iter().skip(common).copied().collect();
    let titles: f32 =
        if layers.title_side == TitleSide::CrossLow { entering.iter().filter_map(|g| layers.groups.get(*g)).map(title_room).sum() } else { 0.0 };
    base + GROUP_PAD * (leaving + entering.len()) as f32 + titles
}

/// The room a group's title takes at its frame's top (the renderer draws it 12 below the top).
fn title_room(group: &Group) -> f32 {
    if group.title.1 > 0.0 { 12.0 + group.title.1 + TITLE_BELOW - GROUP_PAD } else { 0.0 }
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
                    median(places)
                })
                .collect();
            settle(layers, r, &wanted);
        }
    }
}

/// The middle of `places` (between the middle two of an even count), NaN for none. The median
/// rather than the mean keeps a chain straight where a node also leads off to one side (a decision's
/// "no").
fn median(mut places: Vec<f32>) -> f32 {
    places.sort_by(f32::total_cmp);
    let n = places.len();
    match (places.get(n.saturating_sub(1) / 2), places.get(n / 2)) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        _ => f32::NAN,
    }
}

/// How much an item's wish counts when items of a rank crowd one another: a node leading on (or a
/// stand-in, an arrow running through) far more than a node at the end of a branch, so a chain
/// runs straight and the branch's last node moves aside.
fn weight(layers: &Layers, item: usize) -> f32 {
    let links = neighbours(layers, item, true).len() + neighbours(layers, item, false).len();
    if links > 1 { 100.0 } else { 1.0 }
}

/// Puts rank `r`'s items as near where `wanted` says (NaN: where they are) as their order and the
/// room between them let: the places with the least weighted squared distance from the wishes
/// (pooling neighbours that crowd one another, as isotonic regression does).
fn settle(layers: &mut Layers, r: usize, wanted: &[f32]) {
    let Some(rank) = layers.ranks.get(r).cloned() else { return };
    // Each item's least distance from the first, and its wish less that.
    let mut offset = 0.0;
    let mut offsets: Vec<f32> = Vec::with_capacity(rank.len());
    let mut prev: Option<&Item> = None;
    // Pools of neighbours: their common wish (less offsets), weight, and how many.
    let mut pools: Vec<(f32, f32, usize)> = Vec::with_capacity(rank.len());
    for (pos, &i) in rank.iter().enumerate() {
        let Some(item) = layers.items.get(i) else { return };
        if let Some(p) = prev {
            offset += p.size.cross / 2.0 + p.room + gap(layers, p, item) + item.size.cross / 2.0;
        }
        offsets.push(offset);
        prev = Some(item);
        let (wish, w) = match wanted.get(pos).copied().filter(|w| w.is_finite()) {
            Some(wish) => (wish, weight(layers, i)),
            None => (item.cross, 0.01),
        };
        pools.push((wish - offset, w, 1));
        // A pool wishing to be before the one before it joins it.
        while let [.., (a, wa, na), (b, wb, nb)] = pools[..]
            && a > b
        {
            pools.truncate(pools.len() - 2);
            pools.push(((a * wa + b * wb) / (wa + wb), wa + wb, na + nb));
        }
    }
    let mut pos = 0;
    for (value, _, n) in pools {
        for _ in 0..n {
            if let (Some(&i), Some(off)) = (rank.get(pos), offsets.get(pos))
                && let Some(item) = layers.items.get_mut(i)
            {
                item.cross = value + off;
            }
            pos += 1;
        }
    }
}

/// Where each rank's middle is along the flow: each rank as deep as its deepest node, with room
/// after it for the labels of the arrows leaving it, and round the groups starting and ending at it.
fn rank_mains(layers: &Layers) -> Vec<f32> {
    let count = layers.ranks.len();
    let depth: Vec<f32> = (0..count)
        .map(|r| layers.ranks.get(r).map_or(0.0, |rank| rank.iter().filter_map(|&i| layers.items.get(i)).map(|i| i.size.main).fold(0.0, f32::max)))
        .collect();
    // The room for the labels of the arrows leaving each rank, if any.
    let mut labels: Vec<Option<f32>> = vec![None; count];
    for arrow in layers.arrows.iter().filter(|a| !a.back && !a.looped && a.label_at.is_none()) {
        if let (Some(label), Some(&first)) = (arrow.label, arrow.path.first()) {
            let r = layers.items.get(first).map_or(0, |i| i.rank);
            let siblings = layers.arrows.iter().filter(|a| !a.back && !a.looped && a.path.first() == Some(&first)).count();
            let stagger = STAGGER * siblings.saturating_sub(1).min(STAGGER_MAX) as f32;
            if let Some(slot) = labels.get_mut(r) {
                *slot = Some(slot.unwrap_or(RANK_GAP).max(label.main + 2.0 * LABEL_MARGIN + JOG + stagger));
            }
        }
    }
    // A group's frame, and its title, before its first rank and after its last.
    let mut before = vec![0.0f32; count];
    let mut after = vec![0.0f32; count];
    for group in &layers.groups {
        let ranks = group.members.iter().filter_map(|&m| layers.items.get(m)).map(|i| i.rank);
        let (Some(first), Some(last)) = (ranks.clone().min(), ranks.max()) else { continue };
        let title = title_room(group);
        let (start, end) = match layers.title_side {
            TitleSide::MainLow => (GROUP_PAD + title, GROUP_PAD),
            TitleSide::MainHigh => (GROUP_PAD, GROUP_PAD + title),
            TitleSide::CrossLow => (GROUP_PAD, GROUP_PAD),
        };
        if let Some(b) = before.get_mut(first) {
            *b += start;
        }
        if let Some(a) = after.get_mut(last) {
            *a += end;
        }
    }
    let mut mains = Vec::with_capacity(count);
    let mut edge = before.first().copied().unwrap_or(0.0);
    for r in 0..count {
        let d = depth.get(r).copied().unwrap_or(0.0);
        mains.push(edge + d / 2.0);
        let frames = after.get(r).copied().unwrap_or(0.0) + before.get(r + 1).copied().unwrap_or(0.0);
        // Labels go between the frames; without, frames come closer than nodes.
        let between = match labels.get(r).copied().flatten() {
            Some(room) => room,
            None if frames > 0.0 => FRAME_GAP,
            None => RANK_GAP,
        };
        edge += d + frames + between;
    }
    mains
}

/// The frame of `group` (main and cross, low and high) round its members and the groups it holds,
/// with its padding and its title.
fn frame_of(layers: &Layers, mains: &[f32], g: usize) -> Option<[f32; 4]> {
    let group = layers.groups.get(g)?;
    let mut f = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for &m in &group.members {
        let item = layers.items.get(m)?;
        let main = mains.get(item.rank).copied()?;
        f[0] = f[0].min(main - item.size.main / 2.0);
        f[1] = f[1].min(item.cross - item.size.cross / 2.0);
        f[2] = f[2].max(main + item.size.main / 2.0);
        f[3] = f[3].max(item.cross + item.size.cross / 2.0 + item.room);
    }
    // The groups inside it, with their own frames.
    for (c, child) in layers.groups.iter().enumerate() {
        if child.parent == Some(g)
            && let Some(cf) = frame_of(layers, mains, c)
        {
            f = [f[0].min(cf[0]), f[1].min(cf[1]), f[2].max(cf[2]), f[3].max(cf[3])];
        }
    }
    let title = title_room(group);
    let mut frame = [f[0] - GROUP_PAD, f[1] - GROUP_PAD, f[2] + GROUP_PAD, f[3] + GROUP_PAD];
    match layers.title_side {
        TitleSide::MainLow => frame[0] -= title,
        TitleSide::MainHigh => frame[2] += title,
        TitleSide::CrossLow => frame[1] -= title,
    }
    // As wide as its title.
    let wide = group.title.0 + 2.0 * GROUP_PAD;
    let (across, along) = if layers.title_side == TitleSide::CrossLow { (frame[2] - frame[0], 0) } else { (frame[3] - frame[1], 1) };
    if across < wide {
        let grow = (wide - across) / 2.0;
        if along == 0 {
            frame[0] -= grow;
            frame[2] += grow;
        } else {
            frame[1] -= grow;
            frame[3] += grow;
        }
    }
    Some(frame)
}

/// Moves whatever isn't a group's out of its frame (and the rest of its rank on that side with it),
/// until no frame holds anything not its own; then keeps the frames.
fn clear_groups(layers: &mut Layers, mains: &[f32]) {
    if layers.groups.is_empty() {
        return;
    }
    for _ in 0..CLEAR_PASSES {
        let mut moved = false;
        for g in 0..layers.groups.len() {
            let Some(frame) = frame_of(layers, mains, g) else { continue };
            let middle = (frame[1] + frame[3]) / 2.0;
            for r in 0..layers.ranks.len() {
                let Some(rank) = layers.ranks.get(r).cloned() else { continue };
                let main = mains.get(r).copied().unwrap_or(0.0);
                // Where the group's own items are in this rank, if it has any: what isn't its goes
                // out on the side it is on of them.
                let own = rank.iter().position(|&i| layers.items.get(i).is_some_and(|it| it.groups.contains(&g)));
                for (pos, &i) in rank.iter().enumerate() {
                    let Some(item) = layers.items.get(i) else { continue };
                    if item.groups.contains(&g) {
                        continue;
                    }
                    let (lo, hi) = (main - item.size.main / 2.0, main + item.size.main / 2.0);
                    if hi <= frame[0] || lo >= frame[2] {
                        continue;
                    }
                    let (a, b) = (item.cross - item.size.cross / 2.0, item.cross + item.size.cross / 2.0 + item.room);
                    let clear = if item.stand_in { PASS_GAP } else { NODE_GAP / 2.0 };
                    if b + clear <= frame[1] || a - clear >= frame[3] {
                        continue;
                    }
                    // Out of the nearer side, and the rest of the rank beyond it with it.
                    let low = own.map_or(item.cross < middle, |own| pos < own);
                    let (shift, range): (f32, Vec<usize>) = if low {
                        (frame[1] - clear - b, rank.iter().take(pos + 1).copied().collect())
                    } else {
                        (frame[3] + clear - a, rank.iter().skip(pos).copied().collect())
                    };
                    for j in range {
                        if let Some(it) = layers.items.get_mut(j) {
                            it.cross += shift;
                        }
                    }
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    for g in 0..layers.groups.len() {
        if let Some(frame) = frame_of(layers, mains, g)
            && let Some(group) = layers.groups.get_mut(g)
        {
            group.frame = frame;
        }
    }
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
    for (j, arrow) in layers.arrows.iter().enumerate().filter(|(_, a)| !a.back && !a.looped) {
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
    if arrow.looped {
        return loop_route(layers, mains, arrow);
    }
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
        let through = arrow.path.get(n + 1).copied();
        let near = (stop.cross - from.cross).abs() < PASS_GAP - 2.0;
        if n < last_stop
            && (near || (stop.cross - from.cross).abs() < STRAIGHTEN && through.is_some_and(|s| clear_line(layers, mains, s, from.cross)))
        {
            stop.cross = from.cross;
        }
        if n < last_stop && arrow.label.is_some() && arrow.label_at.is_some() && arrow.path.get(n + 1) == arrow.label_at.as_ref() {
            label = Some(stop);
        }
        let labelled = if n == 0 && arrow.label_at.is_none() { arrow.label } else { None };
        // Into and out of a stand-in carrying a label the arrow comes straight, bending away from it.
        let to_label = arrow.label_at.is_some() && arrow.path.get(n + 1) == arrow.label_at.as_ref();
        let from_label = arrow.label_at.is_some() && arrow.path.get(n) == arrow.label_at.as_ref();
        let bend = match (labelled, n == 0, n == last_stop) {
            (Some(_), _, _) if ins > 1 && outs < 2 => Bend::End(staggered(in_k)),
            (Some(_), _, _) => Bend::Start(staggered(out_k)),
            (None, _, true) if ins > 1 => Bend::End(staggered(in_k)),
            (None, true, _) if outs > 1 => Bend::Start(staggered(out_k)),
            _ if to_label => Bend::Start(JOG),
            _ if from_label => Bend::End(JOG),
            _ => Bend::Middle,
        };
        let (segment, run) = elbow(from, stop, bend);
        points.extend(segment);
        if labelled.is_some() {
            // In the middle of the run, or of its part outside the frames it passes.
            let (mut lo, mut hi) = (run.from, run.to);
            if let Some((free_lo, free_hi)) = arrow.path.get(1).and_then(|&next| free_stretch(layers, mains, first, next))
                && free_lo.max(lo) < free_hi.min(hi)
            {
                (lo, hi) = (free_lo.max(lo), free_hi.min(hi));
            }
            label = Some(at((lo + hi) / 2.0, run.cross));
        }
    }
    Route { points, label }
}

/// Whether the arrow through stand-in `s` can pass at `cross` instead, running on straight: it is a
/// plain stand-in (not carrying a label), and nothing else in its rank, and no frame of a group it
/// isn't in, is that near.
fn clear_line(layers: &Layers, mains: &[f32], s: usize, cross: f32) -> bool {
    let Some(item) = layers.items.get(s) else { return false };
    if item.size.main > 0.0 {
        return false;
    }
    let Some(rank) = layers.ranks.get(item.rank) else { return false };
    let free_of_items = rank.iter().filter(|&&o| o != s).filter_map(|&o| layers.items.get(o)).all(|o| {
        let (lo, hi) = (o.cross - o.size.cross / 2.0 - PASS_GAP, o.cross + o.size.cross / 2.0 + o.room + PASS_GAP);
        cross <= lo || cross >= hi
    });
    let main = mains.get(item.rank).copied().unwrap_or(0.0);
    let free_of_frames = layers.groups.iter().enumerate().filter(|(g, _)| !item.groups.contains(g)).all(|(_, g)| {
        let f = g.frame;
        main < f[0] || main > f[2] || cross <= f[1] - PASS_GAP || cross >= f[3] + PASS_GAP
    });
    // Inside the frames it is in, still.
    let inside_own = item.groups.iter().filter_map(|g| layers.groups.get(*g)).all(|g| cross > g.frame[1] && cross < g.frame[3]);
    free_of_items && free_of_frames && inside_own
}

/// An arrow from a node to itself: out of its side across the flow, round, and back in, its label
/// on the far stretch.
fn loop_route(layers: &Layers, mains: &[f32], arrow: &Arrow) -> Route {
    let Some(item) = arrow.path.first().and_then(|&i| layers.items.get(i)) else { return Route { points: Vec::new(), label: None } };
    let main = mains.get(item.rank).copied().unwrap_or(0.0);
    let side = item.cross + item.size.cross / 2.0;
    let half = LOOP_HALF.min(item.size.main / 2.0 - 6.0).max(4.0);
    let out = side + LOOP_OUT;
    let points = vec![at(main - half, side), at(main - half, out), at(main + half, out), at(main + half, side)];
    // The label beside the loop, where it shows whole.
    Route { points, label: arrow.label.map(|l| at(main, out + LOOP_LABEL_GAP + l.cross / 2.0)) }
}

/// An arrow from or to a group cut where it leaves the frame of the one and enters that of the other.
fn cut(route: &mut Route, layers: &Layers, groups: [Option<usize>; 2]) {
    let [from, to] = groups.map(|g| g.and_then(|g| layers.groups.get(g)).map(|g| g.frame));
    if let Some(frame) = from {
        route.points = out_of(&route.points, frame);
    }
    if let Some(frame) = to {
        let mut back: Vec<At> = route.points.iter().rev().copied().collect();
        back = out_of(&back, frame);
        back.reverse();
        route.points = back;
    }
}

/// `points` from where they leave `frame` (main and cross, low and high), as they are if they
/// don't start inside it.
fn out_of(points: &[At], frame: [f32; 4]) -> Vec<At> {
    let [m0, c0, m1, c1] = frame;
    let inside = |p: &At| p.main > m0 + 0.5 && p.main < m1 - 0.5 && p.cross > c0 + 0.5 && p.cross < c1 - 0.5;
    let Some(k) = points.iter().position(|p| !inside(p)).filter(|&k| k > 0) else { return points.to_vec() };
    let (Some(a), Some(b)) = (points.get(k - 1), points.get(k)) else { return points.to_vec() };
    // The segments run along or across the flow: where this one meets the frame.
    let edge = if (a.main - b.main).abs() < 0.01 {
        at(a.main, if b.cross > a.cross { c1 } else { c0 })
    } else {
        at(if b.main > a.main { m1 } else { m0 }, a.cross)
    };
    let mut out = vec![edge];
    out.extend(points.iter().skip(k).copied().filter(|p| (p.main - edge.main).abs() > 0.01 || (p.cross - edge.cross).abs() > 0.01));
    if out.len() < 2 { points.to_vec() } else { out }
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
    for (j, arrow) in layers.arrows.iter().enumerate().filter(|(_, a)| !a.back && !a.looped) {
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

/// A straight run of an arrow along the flow: from and to along it, at this place across it.
#[derive(Clone, Copy)]
struct Run {
    from: f32,
    to: f32,
    cross: f32,
}

/// The points after `a` of a segment to `b`, bending once across the flow, and its longest
/// straight run along the flow (where a label goes).
fn elbow(a: At, b: At, bend: Bend) -> (Vec<At>, Run) {
    if (a.cross - b.cross).abs() < 0.5 {
        return (vec![b], Run { from: a.main, to: b.main, cross: a.cross });
    }
    let room = (b.main - a.main).max(0.0);
    let turn = match bend {
        Bend::Start(d) => a.main + d.min(room / 2.0),
        Bend::End(d) => b.main - d.min(room / 2.0),
        Bend::Middle => (a.main + b.main) / 2.0,
    };
    let run =
        if matches!(bend, Bend::End(_)) { Run { from: a.main, to: turn, cross: a.cross } } else { Run { from: turn, to: b.main, cross: b.cross } };
    (vec![at(turn, a.cross), at(turn, b.cross), b], run)
}

/// Along the flow, the stretch between `a` and the next item `b` of an arrow outside the frames of
/// the groups only one of them is in.
fn free_stretch(layers: &Layers, mains: &[f32], a: usize, b: usize) -> Option<(f32, f32)> {
    let (ia, ib) = (layers.items.get(a)?, layers.items.get(b)?);
    let mut lo = mains.get(ia.rank)? + ia.size.main / 2.0;
    let mut hi = mains.get(ib.rank)? - ib.size.main / 2.0;
    // A frame ending only beyond `b` (or starting before `a`) is left by its side instead.
    for g in ia.groups.iter().filter(|g| !ib.groups.contains(g)) {
        let end = layers.groups.get(*g)?.frame[2];
        if end < hi {
            lo = lo.max(end);
        }
    }
    for g in ib.groups.iter().filter(|g| !ia.groups.contains(g)) {
        let start = layers.groups.get(*g)?.frame[0];
        if start > lo {
            hi = hi.min(start);
        }
    }
    Some((lo, hi))
}

/// An arrow against the flow: out of the far side of its start, along a lane beyond every node and
/// group frame it passes, and into the far side of its end.
fn back_route(layers: &Layers, mains: &[f32], arrow: &Arrow, lanes: &mut usize) -> Route {
    let (Some(a), Some(b)) = (arrow.path.first().and_then(|&i| layers.items.get(i)), arrow.path.last().and_then(|&i| layers.items.get(i))) else {
        return Route { points: Vec::new(), label: None };
    };
    let (lo, hi) = (a.rank.min(b.rank), a.rank.max(b.rank));
    let (ma, mb) = (mains.get(a.rank).copied().unwrap_or(0.0), mains.get(b.rank).copied().unwrap_or(0.0));
    let (main_lo, main_hi) = (ma.min(mb), ma.max(mb));
    let beyond = layers
        .items
        .iter()
        .filter(|it| (!it.stand_in || it.size.main > 0.0) && it.rank >= lo && it.rank <= hi)
        .map(|it| it.cross + it.size.cross / 2.0 + it.room)
        .chain(layers.groups.iter().filter(|g| g.frame[2] >= main_lo && g.frame[0] <= main_hi).map(|g| g.frame[3]))
        .fold(f32::MIN, f32::max);
    *lanes += 1;
    let label_room = arrow.label.map_or(0.0, |l| l.cross / 2.0);
    let lane = beyond + LANE_GAP * *lanes as f32 + label_room;
    // Out of the far side of its start, or, with something beyond that side, out of its end side and
    // round; into the far side of its end, or with something beyond, into its start side.
    let up = a.rank > b.rank;
    let port = |it: &Item| it.cross + if it.spreads { it.size.cross * 0.3 } else { 0.0 };
    let mut points = Vec::with_capacity(6);
    let from = match arrow.path.first() {
        Some(&i) if up && walled(layers, mains, i) => {
            let below = ma + a.size.main / 2.0;
            points.extend([at(below, port(a)), at(below + JOG, port(a))]);
            below + JOG
        }
        _ => {
            points.push(at(ma, a.cross + a.size.cross / 2.0));
            ma
        }
    };
    points.push(at(from, lane));
    let to = match arrow.path.last() {
        Some(&i) if up && walled(layers, mains, i) => {
            let above = mb - b.size.main / 2.0;
            points.extend([at(above - JOG, lane), at(above - JOG, port(b)), at(above, port(b))]);
            above - JOG
        }
        _ => {
            points.extend([at(mb, lane), at(mb, b.cross + b.size.cross / 2.0)]);
            mb
        }
    };
    Route { points, label: arrow.label.map(|_| at((from + to) / 2.0, lane)) }
}

/// Whether anything is beyond `item` across the flow in its rank: a node, a label, or the frame of a
/// group it isn't in.
fn walled(layers: &Layers, mains: &[f32], item: usize) -> bool {
    let Some(it) = layers.items.get(item) else { return false };
    let main = mains.get(it.rank).copied().unwrap_or(0.0);
    let far = it.cross + it.size.cross / 2.0;
    let rank = layers.ranks.get(it.rank).map(Vec::as_slice).unwrap_or_default();
    let others =
        rank.iter().filter(|&&o| o != item).filter_map(|&o| layers.items.get(o)).any(|o| (!o.stand_in || o.size.main > 0.0) && o.cross > it.cross);
    let frames = layers.groups.iter().enumerate().any(|(g, group)| {
        let [m0, c0, m1, _] = group.frame;
        !it.groups.contains(&g) && m0 <= main && m1 >= main && c0 >= far
    });
    others || frames
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

/// A frame (main and cross, low and high) as x, y low and high.
fn frame_xy(f: [f32; 4], direction: Direction) -> [f32; 4] {
    let (x0, y0) = to_xy(at(f[0], f[1]), direction);
    let (x1, y1) = to_xy(at(f[2], f[3]), direction);
    [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
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
    let frames: Vec<[f32; 4]> = layers.groups.iter().map(|g| frame_xy(g.frame, direction)).collect();
    boxes.extend(frames.iter().copied());
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
    // Each group's frame, matched to the renderer's by its nodes.
    for sub in &mut layout.subgraphs {
        let Some(g) = layers
            .groups
            .iter()
            .position(|g| g.members.len() == sub.nodes.len() && g.members.iter().all(|&m| ids.get(m).is_some_and(|id| sub.nodes.contains(id))))
        else {
            continue;
        };
        if let Some(f) = frames.get(g) {
            sub.x = f[0] + dx;
            sub.y = f[1] + dy;
            sub.width = f[2] - f[0];
            sub.height = f[3] - f[1];
        }
    }
    // A group's stand-in for arrows to it is its frame.
    let subs: Vec<[f32; 4]> = layout.subgraphs.iter().map(|s| [s.x, s.y, s.width, s.height]).collect();
    for n in layout.nodes.values_mut() {
        if let Some(&[x, y, w, h]) = n.anchor_subgraph.and_then(|s| subs.get(s)) {
            (n.x, n.y, n.width, n.height) = (x, y, w, h);
        }
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

    const SAMPLES: [&str; 9] = [
        "flowchart LR\n    U[\"업로드<br>로컬 NAS /share/files\"] -->|\"sendFile 배치<br>ECM 등록 · 로컬 삭제\"| E[\"ECM 문서\"]\n    E -->|\"ECM 자동 변환\"| P[\"PDF 변환본\"]\n    E -->|\"OCR 마스킹 배치<br>editDocument 버전업\"| M[\"마스킹본<br>ECM 최신 버전\"]\n    P -->|\"뷰어 주석 저장<br>changePdf\"| P2[\"주석 PDF\"]\n    M --> D1[\"downloadFile<br>/api/cmmn/file/{fileId}\"]\n    P2 --> D2[\"downloadPdfFile<br>/api/cmmn/file/{fileId}/pdf\"]",
        "flowchart LR\n    A[\"운영 레거시 DB\"] -->|\"1 시험 이관<br>공고 1개 사람 + 첨부 메타\"| B[\"개발 DB\"]\n    N[\"운영 레거시 NAS\"] -->|\"2 그 파일만 임시 복사\"| L[\"로컬 폴더\"]\n    B --> APP[\"3 로컬 isbm\"]\n    L --> APP\n    APP -->|\"4 해시 대조 · 이력 확인\"| X[\"5 같은 날 삭제\"]",
        "flowchart TD\n    A[시작] --> B{확인}\n    B -->|예| C[처리]\n    B -->|아니오| D[다시 보기]\n    D --> B",
        "flowchart TD\n    A((원)) --> B([둥근 모양])\n    A --> C{조건}\n    C -->|예| D[(DB)]\n    C -->|아니오| E>깃발]\n    D --> F[끝]\n    E --> F\n    B --> F",
        "flowchart RL\n    A[하나] -->|첫째| B[둘]\n    B --> C[셋]\n    A --> C",
        // An arrow to its own node, in a chain left to right.
        "flowchart LR\n    L[\"원본 전체\"] -->|\"준비 - 오래 걸려도 됨\"| T[\"임시 표<br>다섯 개 + 키\"]\n    T -->|\"검증 - 칸마다 비교\"| T\n    T -->|\"전환 - 짧게 한 번\"| S[\"운영 표<br>바꿀 것만\"]\n    S --> V[\"조회 뷰\"]\n    V --> P[\"화면 여럿\"]",
        // Groups, arrows between them both ways, and one group of a single node.
        "flowchart TB\n    subgraph day[\"매일\"]\n        A[(신청)] --> R1[(요청)]\n        R1 -->|API| S1[(결과)]\n    end\n    subgraph check[\"보강\"]\n        Q{{\"조회\"}} --> T[(확인)]\n    end\n    A -. \"후보\" .-> Q\n    T -. \"가드\" .-> R1\n    subgraph month[\"매월\"]\n        S1 --> R2[(월 요청)]\n        R2 --> C{\"한도\"}\n        C -.-> P[(이력)]\n        C -->|통과| G[(대상)]\n    end\n    subgraph stop[\"중단\"]\n        X[(중단 표)]\n    end\n    G -. \"등록\" .-> X\n    X -. \"막기\" .-> OK[승인]\n    G --> OK",
        // Groups inside groups.
        "flowchart LR\n    subgraph OUT[바깥]\n        A --> B\n        subgraph IN[안]\n            C --> D\n        end\n    end\n    B --> C\n    D --> E\n    Z --> A",
        // Arrows from and to groups, one going back.
        "flowchart TB\n    subgraph day[\"매일\"]\n        A[신청] --> B[요청]\n    end\n    subgraph month[\"매월\"]\n        C[후보] -->|6 미만| D[선정]\n        C -->|6 이상| E[제외]\n    end\n    day ==>|적재 후| month\n    month -. 보호 .-> P[삭제]\n    P -. 대상 .-> day\n    X[수동 구제] --> D",
    ];

    fn meet(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] < b[2] - 0.5 && b[0] < a[2] - 0.5 && a[1] < b[3] - 0.5 && b[1] < a[3] - 0.5
    }

    fn node_boxes(layout: &Layout) -> Vec<[f32; 4]> {
        layout.nodes.values().filter(|n| n.anchor_subgraph.is_none()).map(|n| [n.x, n.y, n.x + n.width, n.y + n.height]).collect()
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

    #[test]
    fn a_group_frames_its_nodes_and_nothing_else() {
        for source in SAMPLES {
            let (layout, _) = laid(source);
            for sub in &layout.subgraphs {
                let frame = [sub.x, sub.y, sub.x + sub.width, sub.y + sub.height];
                assert!(
                    frame[0] >= 0.0 && frame[1] >= 0.0 && frame[2] <= layout.width + 0.5 && frame[3] <= layout.height + 0.5,
                    "{source}: frame inside"
                );
                for n in layout.nodes.values().filter(|n| n.anchor_subgraph.is_none()) {
                    let b = [n.x, n.y, n.x + n.width, n.y + n.height];
                    if sub.nodes.contains(&n.id) {
                        assert!(b[0] >= frame[0] && b[1] >= frame[1] && b[2] <= frame[2] && b[3] <= frame[3], "{source}: {} in {}", n.id, sub.label);
                    } else {
                        assert!(!meet(b, frame), "{source}: {} is out of {}", n.id, sub.label);
                    }
                }
                // Its title is above its nodes.
                let title_bottom = sub.y + 12.0 + sub.label_block.height;
                for n in layout.nodes.values().filter(|n| sub.nodes.contains(&n.id)) {
                    assert!(n.y >= title_bottom - 0.5, "{source}: {} below the title of {}", n.id, sub.label);
                }
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
                if let (Some(anchor), Some(t)) = (e.label_anchor, e.label.as_ref()) {
                    // An arrow to its own node has its label beside it.
                    let near = if e.from == e.to { t.width.max(t.height) / 2.0 + LOOP_LABEL_GAP + 0.5 } else { 0.5 };
                    assert!(off_line(anchor, &e.points) < near, "{source}: the label of {} -> {} is on it", e.from, e.to);
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
        // An arrow to its own node doesn't break the chain: it runs on, left to right, in a line.
        let (lr, _) = laid(SAMPLES[5]);
        let x = |id: &str| lr.nodes.get(id).map(|n| n.x).unwrap_or_default();
        assert!(x("L") < x("T") && x("T") < x("S") && x("S") < x("V") && x("V") < x("P"), "the chain goes on");
        let y = |id: &str| lr.nodes.get(id).map(|n| n.y + n.height / 2.0).unwrap_or_default();
        assert!(["T", "S", "V", "P"].iter().all(|id| (y(id) - y("L")).abs() < 1.0), "in a line");
        // Groups follow the arrows too: a group led to sits after the one leading to it.
        let (g, _) = laid(SAMPLES[6]);
        let top = |id: &str| g.nodes.get(id).map(|n| n.y).unwrap_or_default();
        assert!(top("A") < top("R2") && top("R2") < top("G") && top("G") < top("X") && top("X") < top("OK"));
        // An arrow from a group to a group puts the one after the other.
        let (g, _) = laid(SAMPLES[8]);
        let top = |id: &str| g.nodes.get(id).map(|n| n.y).unwrap_or_default();
        assert!(top("B") < top("C") && top("E") < top("P"), "after the group led from");
        let frame = |label: &str| g.subgraphs.iter().find(|s| s.label == label).map(|s| (s.y, s.y + s.height)).unwrap_or_default();
        assert!(frame("매일").1 < frame("매월").0, "the frames one after the other");
    }

    #[test]
    fn what_it_cant_lay_out_keeps_the_renderers_layout() {
        // More nodes than it lays out.
        let mut long = String::from("flowchart LR\n");
        for i in 0..MAX_NODES {
            long.push_str(&format!("    N{i} --> N{}\n", i + 1));
        }
        // An arrow from a group to a node in it.
        let to_group = "flowchart LR\n    subgraph G[묶음]\n        A --> B\n    end\n    G --> A";
        for source in [long.as_str(), to_group] {
            let (_, ours) = laid(source);
            assert!(!ours, "{source}");
        }
    }
}
