//! A bucket grid over the residents of one zone, for `nearby` and
//! `nearest`.

use std::collections::BinaryHeap;

use unsup_protocol::{Pos, Rect, ResidentId};

use crate::ids::Idx;

/// Tiles per bucket side.
const CELL: u32 = 8;

/// One resident that [`Grid::nearest`] found. The order is by distance,
/// then by id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct Near {
    /// Distance to the place of the search, in tiles.
    pub(crate) dist: u32,
    /// The resident.
    pub(crate) id: ResidentId,
    /// Its dense index.
    pub(crate) idx: u32,
}

/// The best `k` residents that a search has seen: a heap with the worst of
/// them on top.
struct Best {
    k: usize,
    heap: BinaryHeap<Near>,
}

impl Best {
    /// Returns the distance that a resident must beat or match to get in,
    /// or `None` while there is room for anyone.
    fn worst_dist(&self) -> Option<u32> {
        if self.heap.len() < self.k { None } else { self.heap.peek().map(|near| near.dist) }
    }

    /// Takes a resident when it is among the best `k` so far. The id is
    /// read only when the distance does not decide.
    fn offer(&mut self, dist: u32, idx: u32, ids: &[ResidentId]) {
        if self.heap.len() < self.k {
            self.heap.push(Near { dist, id: ids[idx as usize], idx });
        } else if let Some(mut worst) = self.heap.peek_mut()
            && dist <= worst.dist
        {
            let near = Near { dist, id: ids[idx as usize], idx };
            if near < *worst {
                *worst = near;
            }
        }
    }
}

/// Residents of one zone, bucketed by position.
///
/// Each bucket lists the dense indices of the residents whose position is in
/// its square of tiles. One more bucket holds the residents whose position
/// is outside the zone: residents that leave at the end of the tick.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    bounds: Rect,
    cols: u32,
    rows: u32,
    /// `cols * rows` buckets row by row, then the bucket of the residents
    /// outside the zone.
    cells: Vec<Vec<u32>>,
}

impl Grid {
    pub(crate) fn new(bounds: Rect) -> Grid {
        let cols = bounds.width().div_ceil(CELL).max(1);
        let rows = bounds.height().div_ceil(CELL).max(1);
        Grid { bounds, cols, rows, cells: vec![Vec::new(); (cols * rows) as usize + 1] }
    }

    /// Returns the column of buckets that holds `x`, or the nearest one
    /// when `x` is outside the zone.
    #[inline]
    fn col_of(&self, x: i32) -> u32 {
        if x <= self.bounds.x0 { 0 } else { (x.abs_diff(self.bounds.x0) / CELL).min(self.cols - 1) }
    }

    /// Returns the row of buckets that holds `y`, or the nearest one when
    /// `y` is outside the zone.
    #[inline]
    fn row_of(&self, y: i32) -> u32 {
        if y <= self.bounds.y0 { 0 } else { (y.abs_diff(self.bounds.y0) / CELL).min(self.rows - 1) }
    }

    /// Returns the bucket of the residents outside the zone.
    #[inline]
    fn outside(&self) -> usize {
        (self.cols * self.rows) as usize
    }

    #[inline]
    fn cell_of(&self, pos: Pos) -> usize {
        if self.bounds.contains(pos) {
            (self.row_of(pos.y) * self.cols + self.col_of(pos.x)) as usize
        } else {
            self.outside()
        }
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

    /// Returns the columns and the rows of buckets that the square of
    /// `radius` tiles around `pos` touches: first column, last column,
    /// first row, last row.
    fn span(&self, pos: Pos, radius: u32) -> (u32, u32, u32, u32) {
        let r = i32::try_from(radius).unwrap_or(i32::MAX);
        (
            self.col_of(pos.x.saturating_sub(r)),
            self.col_of(pos.x.saturating_add(r)),
            self.row_of(pos.y.saturating_sub(r)),
            self.row_of(pos.y.saturating_add(r)),
        )
    }

    /// Appends to `out` every resident within `radius` tiles of `pos`
    /// (Chebyshev distance). `positions` is the position column. The order of
    /// the result is not defined; the caller sorts it.
    pub(crate) fn query(&self, pos: Pos, radius: u32, positions: &[Pos], out: &mut Vec<Idx>) {
        let (c0, c1, r0, r1) = self.span(pos, radius);
        let buckets = (r0..=r1)
            .flat_map(|row| (c0..=c1).map(move |col| (row * self.cols + col) as usize))
            .chain(std::iter::once(self.outside()));
        for bucket in buckets {
            for &idx in &self.cells[bucket] {
                if positions[idx as usize].dist(pos) <= radius {
                    out.push(Idx(idx));
                }
            }
        }
    }

    /// Returns how many residents are within `radius` tiles of `pos`.
    ///
    /// A bucket that lies wholly inside the square of the search is counted
    /// by its length, so a crowd on one tile costs nothing to count.
    pub(crate) fn count(&self, pos: Pos, radius: u32, positions: &[Pos]) -> usize {
        let within = |bucket: usize| {
            self.cells[bucket]
                .iter()
                .filter(|&&idx| positions[idx as usize].dist(pos) <= radius)
                .count()
        };
        let (c0, c1, r0, r1) = self.span(pos, radius);
        let reach = i64::from(radius);
        let (x, y) = (i64::from(pos.x), i64::from(pos.y));
        let side = i64::from(CELL);
        let mut total = within(self.outside());
        for row in r0..=r1 {
            let top = i64::from(self.bounds.y0) + i64::from(row) * side;
            let rows_inside = top >= y - reach && top + side - 1 <= y + reach;
            for col in c0..=c1 {
                let left = i64::from(self.bounds.x0) + i64::from(col) * side;
                let bucket = (row * self.cols + col) as usize;
                if rows_inside && left >= x - reach && left + side - 1 <= x + reach {
                    total += self.cells[bucket].len();
                } else {
                    total += within(bucket);
                }
            }
        }
        total
    }

    /// Finds the `k` residents nearest to `pos` among those within `radius`
    /// tiles, and writes them to `out`: nearest first, and residents at the
    /// same distance in `ResidentId` order. `positions` and `ids` are the
    /// position and the id columns. `out` is cleared first; its buffer is
    /// reused.
    ///
    /// The search does not sort the residents in sight. It reads the
    /// buckets ring by ring around the bucket of `pos`, and stops when no
    /// bucket that is left can hold a resident as near as the `k` that it
    /// has.
    pub(crate) fn nearest(
        &self,
        pos: Pos,
        radius: u32,
        k: usize,
        positions: &[Pos],
        ids: &[ResidentId],
        out: &mut Vec<Near>,
    ) {
        let mut buffer = std::mem::take(out);
        buffer.clear();
        let mut best = Best { k, heap: BinaryHeap::from(buffer) };
        if k > 0 {
            self.search(pos, radius, positions, ids, &mut best);
        }
        *out = best.heap.into_sorted_vec();
    }

    fn search(
        &self,
        pos: Pos,
        radius: u32,
        positions: &[Pos],
        ids: &[ResidentId],
        best: &mut Best,
    ) {
        let read = |bucket: usize, best: &mut Best| {
            for &idx in &self.cells[bucket] {
                let dist = positions[idx as usize].dist(pos);
                if dist <= radius {
                    best.offer(dist, idx, ids);
                }
            }
        };
        // The residents outside the zone have no place in the rings.
        read(self.outside(), best);
        let (c0, c1, r0, r1) = self.span(pos, radius);
        let (qc, qr) = (self.col_of(pos.x), self.row_of(pos.y));
        let last = (qc - c0).max(c1 - qc).max(qr - r0).max(r1 - qr);
        for ring in 0..=last {
            // The top and the bottom row of the ring, then its two sides
            // without the corners.
            let cols = qc.saturating_sub(ring).max(c0)..=(qc + ring).min(c1);
            if qr >= r0 + ring {
                for col in cols.clone() {
                    read(((qr - ring) * self.cols + col) as usize, best);
                }
            }
            if ring > 0 && qr + ring <= r1 {
                for col in cols {
                    read(((qr + ring) * self.cols + col) as usize, best);
                }
            }
            if ring > 0 {
                let rows = (qr + 1).saturating_sub(ring).max(r0)..=(qr + ring - 1).min(r1);
                if qc >= c0 + ring {
                    for row in rows.clone() {
                        read((row * self.cols + qc - ring) as usize, best);
                    }
                }
                if qc + ring <= c1 {
                    for row in rows {
                        read((row * self.cols + qc + ring) as usize, best);
                    }
                }
            }
            // A resident in a bucket that is not read yet is at least
            // `beyond` tiles away. When the worst of the best `k` is nearer
            // than that, nothing can replace it or tie with it.
            if let Some(worst) = best.worst_dist()
                && i64::from(worst) < self.beyond(pos, qc, qr, ring, (c0, c1, r0, r1))
            {
                break;
            }
        }
    }

    /// Returns the least distance from `pos` to a tile of a bucket outside
    /// the rings `0..=ring` around the bucket `(qc, qr)`, among the buckets
    /// of `span`. It is `i64::MAX` when no such bucket is left.
    fn beyond(&self, pos: Pos, qc: u32, qr: u32, ring: u32, span: (u32, u32, u32, u32)) -> i64 {
        let (c0, c1, r0, r1) = span;
        let side = i64::from(CELL);
        let (x, y) = (i64::from(pos.x), i64::from(pos.y));
        let (x0, y0) = (i64::from(self.bounds.x0), i64::from(self.bounds.y0));
        let mut least = i64::MAX;
        if qc + ring < c1 {
            // The first column of tiles to the east of the rings.
            least = least.min(x0 + i64::from(qc + ring + 1) * side - x);
        }
        if qc > c0 + ring {
            // The last column of tiles to the west of the rings.
            least = least.min(x - (x0 + i64::from(qc - ring) * side - 1));
        }
        if qr + ring < r1 {
            least = least.min(y0 + i64::from(qr + ring + 1) * side - y);
        }
        if qr > r0 + ring {
            least = least.min(y - (y0 + i64::from(qr - ring) * side - 1));
        }
        least
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

    /// The slow, obviously correct nearest search: sort everybody in sight
    /// by distance and id, and take the first `k`.
    fn brute_nearest(
        positions: &[Pos],
        ids: &[ResidentId],
        pos: Pos,
        radius: u32,
        k: usize,
    ) -> Vec<Near> {
        let mut all: Vec<Near> = (0..positions.len() as u32)
            .map(|idx| Near { dist: positions[idx as usize].dist(pos), id: ids[idx as usize], idx })
            .filter(|near| near.dist <= radius)
            .collect();
        all.sort_unstable();
        all.truncate(k);
        all
    }

    /// Makes a grid over `bounds` with `n` residents at random tiles of
    /// `area`, with ids in a random order.
    fn crowd(bounds: Rect, area: Rect, n: u32, rng: &mut Rng) -> (Grid, Vec<Pos>, Vec<ResidentId>) {
        let mut grid = Grid::new(bounds);
        let mut positions = Vec::new();
        let mut ids: Vec<ResidentId> = (1..=u64::from(n)).map(|i| ResidentId(i * 3)).collect();
        rng.shuffle(&mut ids);
        for i in 0..n {
            let pos = Pos::new(
                rng.range(i64::from(area.x0), i64::from(area.x1)) as i32,
                rng.range(i64::from(area.y0), i64::from(area.y1)) as i32,
            );
            positions.push(pos);
            grid.insert(i, pos);
        }
        (grid, positions, ids)
    }

    #[test]
    fn nearest_matches_a_full_sort_also_outside_the_zone() {
        let bounds = Rect::sized(64, 128, 64, 64);
        let mut rng = Rng::seeded(21);
        // Some residents and some searches are outside the zone on every
        // side. Few tiles and many residents give many equal distances.
        let (grid, positions, ids) = crowd(bounds, Rect::new(54, 118, 138, 202), 900, &mut rng);
        let mut found = Vec::new();
        let mut cut = 0;
        for _ in 0..600 {
            let pos = Pos::new(rng.range(40, 150) as i32, rng.range(100, 210) as i32);
            let radius = rng.below(40) as u32;
            let k = rng.below(30) as usize;
            grid.nearest(pos, radius, k, &positions, &ids, &mut found);
            let expected = brute_nearest(&positions, &ids, pos, radius, k);
            assert_eq!(found, expected, "pos {pos}, radius {radius}, k {k}");
            let in_sight = brute(&positions, pos, radius).len();
            assert_eq!(grid.count(pos, radius, &positions), in_sight, "pos {pos}, radius {radius}");
            if in_sight > k {
                cut += 1;
            }
        }
        assert!(cut > 300, "most searches saw more residents than they took: {cut}");
        // A dense town: about five residents on each tile, so that the
        // `k` nearest often end in the middle of a group at one distance.
        // Then the search can stop only when no bucket that is left can
        // hold a resident at that distance.
        let town = Rect::sized(64, 128, 20, 20);
        let (dense, dense_positions, dense_ids) = crowd(bounds, town, 2_000, &mut rng);
        for _ in 0..1_500 {
            let pos = Pos::new(rng.range(60, 88) as i32, rng.range(124, 152) as i32);
            let radius = rng.range(1, 14) as u32;
            let k = rng.range(1, 60) as usize;
            dense.nearest(pos, radius, k, &dense_positions, &dense_ids, &mut found);
            let expected = brute_nearest(&dense_positions, &dense_ids, pos, radius, k);
            assert_eq!(found, expected, "pos {pos}, radius {radius}, k {k}");
        }
        // No limit on the distance or on the number: everybody, in order.
        grid.nearest(Pos::new(90, 150), u32::MAX, usize::MAX, &positions, &ids, &mut found);
        assert_eq!(found, brute_nearest(&positions, &ids, Pos::new(90, 150), u32::MAX, 900));
        assert_eq!(found.len(), 900);
        assert_eq!(grid.count(Pos::new(90, 150), u32::MAX, &positions), 900);
    }

    #[test]
    fn nearest_breaks_ties_by_id_in_a_crowd_on_one_tile() {
        let bounds = Rect::sized(0, 0, 64, 64);
        let mut rng = Rng::seeded(22);
        let spot = Pos::new(37, 21);
        let (mut grid, mut positions, mut ids) =
            crowd(bounds, Rect::new(spot.x, spot.y, spot.x + 1, spot.y + 1), 5_000, &mut rng);
        // One resident with the lowest id of all, one tile away: the crowd
        // on the tile comes first.
        positions.push(spot.offset(1, 0));
        ids.push(ResidentId(1));
        grid.insert(5_000, spot.offset(1, 0));
        let mut found = Vec::new();
        grid.nearest(spot, 12, 12, &positions, &ids, &mut found);
        let lowest: Vec<ResidentId> = (1..=12).map(|i| ResidentId(i * 3)).collect();
        assert_eq!(found.iter().map(|near| near.id).collect::<Vec<_>>(), lowest);
        assert!(found.iter().all(|near| near.dist == 0 && positions[near.idx as usize] == spot));
        assert_eq!(grid.count(spot, 12, &positions), 5_001);
        // From 3 tiles away the crowd is still nearer than nobody else but
        // the single resident.
        let from = spot.offset(4, 0);
        grid.nearest(from, 12, 3, &positions, &ids, &mut found);
        assert_eq!(
            found.iter().map(|near| (near.dist, near.id)).collect::<Vec<_>>(),
            [(3, ResidentId(1)), (4, ResidentId(3)), (4, ResidentId(6))]
        );
        assert_eq!(found, brute_nearest(&positions, &ids, from, 12, 3));
        // Out of reach of the crowd.
        grid.nearest(spot.offset(14, 0), 12, 3, &positions, &ids, &mut found);
        assert_eq!(found, []);
        assert_eq!(grid.count(spot.offset(14, 0), 12, &positions), 0);
        assert_eq!(grid.count(spot.offset(13, 0), 12, &positions), 1);
    }

    #[test]
    fn nearest_takes_nobody_when_asked_for_nobody() {
        let bounds = Rect::sized(0, 0, 16, 16);
        let mut rng = Rng::seeded(23);
        let (grid, positions, ids) = crowd(bounds, bounds, 50, &mut rng);
        let mut found = vec![Near { dist: 1, id: ResidentId(1), idx: 0 }];
        grid.nearest(Pos::new(8, 8), 100, 0, &positions, &ids, &mut found);
        assert_eq!(found, []);
        // A radius of zero finds the residents on the tile itself.
        let pos = positions[7];
        grid.nearest(pos, 0, 50, &positions, &ids, &mut found);
        assert_eq!(found, brute_nearest(&positions, &ids, pos, 0, 50));
        assert!(found.iter().any(|near| near.idx == 7));
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
        let ids: Vec<ResidentId> = (0..positions.len() as u64).map(ResidentId).collect();
        let mut found = Vec::new();
        for radius in [0, 3, 9, 30, 100] {
            let pos = Pos::new(32, 32);
            assert_eq!(
                query_sorted(&grid, &positions, pos, radius),
                brute(&positions, pos, radius)
            );
            assert_eq!(grid.count(pos, radius, &positions), brute(&positions, pos, radius).len());
            grid.nearest(pos, radius, 7, &positions, &ids, &mut found);
            assert_eq!(found, brute_nearest(&positions, &ids, pos, radius, 7));
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
        let ids = [ResidentId(9), ResidentId(4)];
        let mut found = Vec::new();
        grid.nearest(Pos::new(0, 0), 4, 2, &positions, &ids, &mut found);
        assert_eq!(
            found,
            [
                Near { dist: 1, id: ResidentId(9), idx: 0 },
                Near { dist: 4, id: ResidentId(4), idx: 1 }
            ]
        );
        assert_eq!(grid.count(Pos::new(0, 0), 3, &positions), 1);
    }
}
