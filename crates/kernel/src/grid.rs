//! A bucket grid over the residents of one zone, for `nearby`.

use unsup_protocol::{Pos, Rect};

use crate::ids::Idx;

/// Tiles per bucket side.
const CELL: u32 = 8;

/// Residents of one zone, bucketed by position.
///
/// Each bucket lists the dense indices of the residents whose position is in
/// it. A position outside the zone (a resident that leaves at the end of the
/// tick) goes to the nearest border bucket.
#[derive(Clone, Debug, Default)]
pub(crate) struct Grid {
    bounds: Rect,
    cols: u32,
    rows: u32,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    pub(crate) fn new(bounds: Rect) -> Grid {
        let cols = bounds.width().div_ceil(CELL).max(1);
        let rows = bounds.height().div_ceil(CELL).max(1);
        Grid { bounds, cols, rows, cells: vec![Vec::new(); (cols * rows) as usize] }
    }

    #[inline]
    fn col_of(&self, x: i32) -> u32 {
        if x <= self.bounds.x0 { 0 } else { (x.abs_diff(self.bounds.x0) / CELL).min(self.cols - 1) }
    }

    #[inline]
    fn row_of(&self, y: i32) -> u32 {
        if y <= self.bounds.y0 { 0 } else { (y.abs_diff(self.bounds.y0) / CELL).min(self.rows - 1) }
    }

    #[inline]
    fn cell_of(&self, pos: Pos) -> usize {
        (self.row_of(pos.y) * self.cols + self.col_of(pos.x)) as usize
    }

    /// Adds a resident.
    pub(crate) fn insert(&mut self, idx: u32, pos: Pos) {
        let cell = self.cell_of(pos);
        self.cells[cell].push(idx);
    }

    /// Removes a resident that is at `pos`.
    pub(crate) fn remove(&mut self, idx: u32, pos: Pos) {
        let cell = self.cell_of(pos);
        let list = &mut self.cells[cell];
        if let Some(at) = list.iter().position(|&i| i == idx) {
            list.swap_remove(at);
        }
    }

    /// Moves a resident from `old` to `new`.
    pub(crate) fn relocate(&mut self, idx: u32, old: Pos, new: Pos) {
        let from = self.cell_of(old);
        let to = self.cell_of(new);
        if from != to {
            let list = &mut self.cells[from];
            if let Some(at) = list.iter().position(|&i| i == idx) {
                list.swap_remove(at);
            }
            self.cells[to].push(idx);
        }
    }

    /// Changes the dense index of the resident at `pos` from `old` to `new`.
    /// The resident table calls it after a swap-remove.
    pub(crate) fn rename(&mut self, old: u32, new: u32, pos: Pos) {
        let cell = self.cell_of(pos);
        if let Some(slot) = self.cells[cell].iter_mut().find(|i| **i == old) {
            *slot = new;
        }
    }

    /// Appends to `out` every resident within `radius` tiles of `pos`
    /// (Chebyshev distance). `positions` is the position column. The order of
    /// the result is not defined; the caller sorts it.
    pub(crate) fn query(&self, pos: Pos, radius: u32, positions: &[Pos], out: &mut Vec<Idx>) {
        let r = i32::try_from(radius).unwrap_or(i32::MAX);
        let c0 = self.col_of(pos.x.saturating_sub(r));
        let c1 = self.col_of(pos.x.saturating_add(r));
        let r0 = self.row_of(pos.y.saturating_sub(r));
        let r1 = self.row_of(pos.y.saturating_add(r));
        for row in r0..=r1 {
            for col in c0..=c1 {
                for &idx in &self.cells[(row * self.cols + col) as usize] {
                    if positions[idx as usize].dist(pos) <= radius {
                        out.push(Idx(idx));
                    }
                }
            }
        }
    }

    /// Returns an estimate of the heap bytes that the grid uses.
    pub(crate) fn memory_estimate(&self) -> usize {
        self.cells.iter().map(|c| c.capacity() * 4 + size_of::<Vec<u32>>()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    /// The slow, obviously correct answer.
    fn brute(positions: &[Pos], pos: Pos, radius: u32) -> Vec<u32> {
        (0..positions.len() as u32).filter(|&i| positions[i as usize].dist(pos) <= radius).collect()
    }

    fn query_sorted(grid: &Grid, positions: &[Pos], pos: Pos, radius: u32) -> Vec<u32> {
        let mut out = Vec::new();
        grid.query(pos, radius, positions, &mut out);
        let mut out: Vec<u32> = out.into_iter().map(|idx| idx.0).collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn query_matches_brute_force_also_outside_the_zone() {
        let bounds = Rect::sized(64, 128, 64, 64);
        let mut rng = Rng::seeded(5);
        let mut grid = Grid::new(bounds);
        let mut positions = Vec::new();
        for i in 0..400u32 {
            // Some positions are outside the zone on every side.
            let pos = Pos::new(rng.range(54, 138) as i32, rng.range(118, 202) as i32);
            positions.push(pos);
            grid.insert(i, pos);
        }
        for _ in 0..200 {
            let pos = Pos::new(rng.range(40, 150) as i32, rng.range(100, 210) as i32);
            let radius = rng.below(40) as u32;
            assert_eq!(
                query_sorted(&grid, &positions, pos, radius),
                brute(&positions, pos, radius)
            );
        }
        assert_eq!(query_sorted(&grid, &positions, Pos::new(90, 150), u32::MAX).len(), 400);
        assert!(grid.memory_estimate() > 0);
    }

    #[test]
    fn relocate_remove_and_rename_keep_the_index_right() {
        let bounds = Rect::sized(0, 0, 64, 64);
        let mut rng = Rng::seeded(6);
        let mut grid = Grid::new(bounds);
        let mut positions: Vec<Pos> = Vec::new();
        for i in 0..100u32 {
            let pos = Pos::new(rng.range(0, 64) as i32, rng.range(0, 64) as i32);
            positions.push(pos);
            grid.insert(i, pos);
        }
        for _ in 0..500 {
            let i = rng.below(positions.len() as u64) as u32;
            let new = Pos::new(rng.range(-5, 70) as i32, rng.range(-5, 70) as i32);
            grid.relocate(i, positions[i as usize], new);
            positions[i as usize] = new;
        }
        // Swap-remove half of the residents, the way the resident table does.
        while positions.len() > 50 {
            let i = rng.below(positions.len() as u64) as u32;
            let last = positions.len() as u32 - 1;
            grid.remove(i, positions[i as usize]);
            if i != last {
                grid.rename(last, i, positions[last as usize]);
            }
            positions.swap_remove(i as usize);
        }
        for radius in [0, 3, 9, 30, 100] {
            let pos = Pos::new(32, 32);
            assert_eq!(
                query_sorted(&grid, &positions, pos, radius),
                brute(&positions, pos, radius)
            );
        }
    }

    #[test]
    fn a_zone_smaller_than_one_bucket_works() {
        let mut grid = Grid::new(Rect::sized(0, 0, 5, 5));
        let positions = [Pos::new(1, 1), Pos::new(4, 4)];
        grid.insert(0, positions[0]);
        grid.insert(1, positions[1]);
        assert_eq!(query_sorted(&grid, &positions, Pos::new(0, 0), 1), [0]);
        assert_eq!(query_sorted(&grid, &positions, Pos::new(0, 0), 4), [0, 1]);
    }
}
