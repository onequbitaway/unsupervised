//! Path search: A* over the walkable tiles of one zone.
//!
//! A resident moves in eight directions. A step costs the ticks of the tile
//! that it enters. A diagonal step is allowed only when the two tiles beside
//! it are walkable too, so a path never cuts the corner of a wall. A door is
//! a gap of one tile in a wall; a path can only go straight through it.
//!
//! The search looks at no more than a given number of tiles. It is
//! deterministic: ties are broken by the position of the tile.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use unsup_kernel::{Pos, TileMap};

use crate::tiles::Costs;

/// The eight directions of a step: north, east, south, west, then the four
/// diagonals. A path is a list of indices into this table.
pub(crate) const DIRS: [(i32, i32); 8] =
    [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)];

/// Index of the direction north in [`DIRS`].
pub(crate) const NORTH: u8 = 0;
/// Index of the direction east in [`DIRS`].
pub(crate) const EAST: u8 = 1;
/// Index of the direction south in [`DIRS`].
pub(crate) const SOUTH: u8 = 2;
/// Index of the direction west in [`DIRS`].
pub(crate) const WEST: u8 = 3;

/// Marks the start tile in the table of directions.
const NO_DIR: u8 = u8::MAX;

/// Returns the tile one step from `pos` in a direction of [`DIRS`].
#[inline]
pub(crate) fn step(pos: Pos, dir: u8) -> Pos {
    let (dx, dy) = DIRS[usize::from(dir) % DIRS.len()];
    Pos::new(pos.x + dx, pos.y + dy)
}

/// Where a search ends.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Goal {
    /// At this tile.
    Tile(Pos),
    /// At any walkable tile on one of the given edges of the zone. The
    /// search prefers the tile from which `toward` is nearest.
    Edge {
        /// The place outside the zone that the resident walks to.
        toward: Pos,
        /// The edges that count, in the order north, east, south, west.
        sides: [bool; 4],
    },
}

/// A path that the search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Route {
    /// The steps, as indices into [`DIRS`].
    pub(crate) steps: Vec<u8>,
    /// Sum of the ticks of all steps.
    pub(crate) ticks: u32,
    /// The tile where the path ends.
    pub(crate) end: Pos,
}

/// Why the search found no path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoRoute {
    /// No path exists inside the zone.
    Unreachable,
    /// The search looked at the greatest allowed number of tiles.
    TooFar,
}

/// Buffers of the search. A zone keeps one and uses it again, so a search
/// allocates only its result.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    /// Cheapest known cost to reach each tile.
    cost: Vec<u32>,
    /// Direction of the last step to each tile.
    came: Vec<u8>,
    /// Round in which `cost` and `came` of each tile were written.
    seen: Vec<u32>,
    /// Round in which each tile was finished.
    closed: Vec<u32>,
    /// Number of the current search.
    round: u32,
    /// Tiles to look at next: estimate of the whole path, estimate of the
    /// rest, tile number.
    open: BinaryHeap<Reverse<(u32, u32, u32)>>,
}

impl Scratch {
    /// Makes the buffers ready for a search over `tiles` tiles.
    fn start(&mut self, tiles: usize) {
        if self.cost.len() != tiles || self.round == u32::MAX {
            self.cost = vec![0; tiles];
            self.came = vec![NO_DIR; tiles];
            self.seen = vec![0; tiles];
            self.closed = vec![0; tiles];
            self.round = 0;
        }
        self.round += 1;
        self.open.clear();
    }
}

/// Finds the cheapest path from `from` to the goal inside the zone of
/// `tiles`. The start tile may be of any kind. `limit` is the greatest
/// number of tiles that the search looks at.
pub(crate) fn find(
    tiles: &TileMap,
    costs: &Costs,
    from: Pos,
    goal: Goal,
    limit: usize,
    scratch: &mut Scratch,
) -> Result<Route, NoRoute> {
    let bounds = tiles.bounds();
    if !bounds.contains(from) {
        return Err(NoRoute::Unreachable);
    }
    let width = bounds.width() as usize;
    let codes = tiles.as_slice();
    let slot_of =
        |p: Pos| p.y.abs_diff(bounds.y0) as usize * width + p.x.abs_diff(bounds.x0) as usize;
    let pos_of = |slot: usize| {
        Pos::new(bounds.x0 + (slot % width) as i32, bounds.y0 + (slot / width) as i32)
    };
    let aim = match goal {
        Goal::Tile(tile) => tile,
        Goal::Edge { toward, .. } => toward,
    };
    let rest = |p: Pos| p.dist(aim).saturating_mul(costs.min);
    let reached = |p: Pos| match goal {
        Goal::Tile(tile) => p == tile,
        Goal::Edge { sides, .. } => {
            (sides[usize::from(NORTH)] && p.y == bounds.y0)
                || (sides[usize::from(EAST)] && p.x == bounds.x1 - 1)
                || (sides[usize::from(SOUTH)] && p.y == bounds.y1 - 1)
                || (sides[usize::from(WEST)] && p.x == bounds.x0)
        }
    };

    scratch.start(codes.len());
    let round = scratch.round;
    let start = slot_of(from);
    scratch.cost[start] = 0;
    scratch.came[start] = NO_DIR;
    scratch.seen[start] = round;
    scratch.open.push(Reverse((rest(from), rest(from), start as u32)));

    let mut looked = 0usize;
    while let Some(Reverse((_, _, slot))) = scratch.open.pop() {
        let slot = slot as usize;
        if scratch.closed[slot] == round {
            continue;
        }
        scratch.closed[slot] = round;
        let here = pos_of(slot);
        if reached(here) {
            return Ok(trace(scratch, slot_of, here));
        }
        looked += 1;
        if looked > limit {
            return Err(NoRoute::TooFar);
        }
        let so_far = scratch.cost[slot];
        for (dir, &(dx, dy)) in DIRS.iter().enumerate() {
            let next = Pos::new(here.x + dx, here.y + dy);
            if !bounds.contains(next) {
                continue;
            }
            let next_slot = slot_of(next);
            let Some(ticks) = costs.of(codes[next_slot]) else {
                continue;
            };
            if scratch.closed[next_slot] == round {
                continue;
            }
            // A diagonal step needs both tiles beside it. They are inside
            // the zone because `here` and `next` are.
            if dx != 0
                && dy != 0
                && (costs.of(codes[slot_of(Pos::new(next.x, here.y))]).is_none()
                    || costs.of(codes[slot_of(Pos::new(here.x, next.y))]).is_none())
            {
                continue;
            }
            let cost = so_far.saturating_add(ticks);
            if scratch.seen[next_slot] != round || cost < scratch.cost[next_slot] {
                scratch.seen[next_slot] = round;
                scratch.cost[next_slot] = cost;
                scratch.came[next_slot] = dir as u8;
                let ahead = rest(next);
                scratch.open.push(Reverse((cost.saturating_add(ahead), ahead, next_slot as u32)));
            }
        }
    }
    Err(NoRoute::Unreachable)
}

/// Reads the path to the tile `end` back from the table of directions.
/// `slot_of` gives the number of a tile.
fn trace(scratch: &Scratch, slot_of: impl Fn(Pos) -> usize, end: Pos) -> Route {
    let ticks = scratch.cost[slot_of(end)];
    let mut steps = Vec::new();
    let mut at = end;
    // Every tile on the way was written in this search, so the walk back
    // ends at the start tile, which has no direction.
    while let Some(&(dx, dy)) = DIRS.get(usize::from(scratch.came[slot_of(at)])) {
        steps.push(scratch.came[slot_of(at)]);
        at = Pos::new(at.x - dx, at.y - dy);
    }
    steps.reverse();
    Route { steps, ticks, end }
}
