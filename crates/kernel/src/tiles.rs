//! The tile map of a zone.

use serde::{Deserialize, Serialize};
use unsup_protocol::{Pos, Rect};

/// The tiles of one zone: one byte per tile, row by row.
///
/// The kernel gives no meaning to tile values. A new map is all zeros;
/// modules (world generation) write the kinds that they define.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileMap {
    bounds: Rect,
    #[serde(with = "crate::codec::bytes")]
    tiles: Vec<u8>,
}

impl TileMap {
    /// Makes a map of zeros that covers `bounds`.
    #[must_use]
    pub fn new(bounds: Rect) -> TileMap {
        let len = bounds.width() as usize * bounds.height() as usize;
        TileMap { bounds, tiles: vec![0; len] }
    }

    /// Returns the tiles that the map covers, in global coordinates.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Returns the number of columns.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.bounds.width()
    }

    /// Returns the number of rows.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.bounds.height()
    }

    fn slot(&self, pos: Pos) -> Option<usize> {
        if !self.bounds.contains(pos) {
            return None;
        }
        let col = pos.x.abs_diff(self.bounds.x0) as usize;
        let row = pos.y.abs_diff(self.bounds.y0) as usize;
        Some(row * self.bounds.width() as usize + col)
    }

    /// Returns the kind of the tile at `pos`, or `None` outside the map.
    #[must_use]
    pub fn get(&self, pos: Pos) -> Option<u8> {
        self.slot(pos).and_then(|i| self.tiles.get(i).copied())
    }

    /// Sets the kind of the tile at `pos`. Returns false outside the map.
    pub fn set(&mut self, pos: Pos, kind: u8) -> bool {
        match self.slot(pos).and_then(|i| self.tiles.get_mut(i)) {
            Some(tile) => {
                *tile = kind;
                true
            }
            None => false,
        }
    }

    /// Sets every tile of `rect` that is inside the map.
    pub fn fill(&mut self, rect: Rect, kind: u8) {
        let x0 = rect.x0.max(self.bounds.x0);
        let x1 = rect.x1.min(self.bounds.x1);
        let y0 = rect.y0.max(self.bounds.y0);
        let y1 = rect.y1.min(self.bounds.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.bounds.width() as usize;
        let col0 = x0.abs_diff(self.bounds.x0) as usize;
        let col1 = x1.abs_diff(self.bounds.x0) as usize;
        for y in y0..y1 {
            let row = y.abs_diff(self.bounds.y0) as usize;
            self.tiles[row * width + col0..row * width + col1].fill(kind);
        }
    }

    /// Returns how many tiles have the kind.
    #[must_use]
    pub fn count(&self, kind: u8) -> usize {
        self.tiles.iter().filter(|&&t| t == kind).count()
    }

    /// Returns all tiles, row by row.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.tiles
    }

    /// Returns the rows of `rect` that are inside the map, each as its
    /// first position and its tiles.
    fn rows_in(&self, rect: Rect) -> impl Iterator<Item = (Pos, &[u8])> + '_ {
        let x0 = rect.x0.max(self.bounds.x0);
        let x1 = rect.x1.min(self.bounds.x1);
        let y0 = rect.y0.max(self.bounds.y0);
        let y1 = rect.y1.min(self.bounds.y1);
        let width = self.bounds.width() as usize;
        let rows = if x0 < x1 { y0..y1 } else { 0..0 };
        rows.map(move |y| {
            let start =
                y.abs_diff(self.bounds.y0) as usize * width + x0.abs_diff(self.bounds.x0) as usize;
            (Pos::new(x0, y), &self.tiles[start..start + x1.abs_diff(x0) as usize])
        })
    }

    /// Returns how many tiles of `rect` inside the map have a kind in
    /// `kinds`. Bit `n` of `kinds` stands for kind `n`; a kind above 63 is
    /// never in it.
    pub(crate) fn count_in(&self, rect: Rect, kinds: u64) -> usize {
        self.rows_in(rect)
            .map(|(_, row)| row.iter().filter(|&&kind| has_kind(kinds, kind)).count())
            .sum()
    }

    /// Returns the position of tile number `n`, from 0, among the tiles of
    /// `rect` inside the map that have a kind in `kinds`, counted row by
    /// row. Returns `None` when there are not that many.
    pub(crate) fn nth_in(&self, rect: Rect, kinds: u64, n: usize) -> Option<Pos> {
        let mut left = n;
        for (first, row) in self.rows_in(rect) {
            for (column, &kind) in row.iter().enumerate() {
                if has_kind(kinds, kind) {
                    if left == 0 {
                        return Some(first.offset(column as i32, 0));
                    }
                    left -= 1;
                }
            }
        }
        None
    }
}

/// Returns true when bit `kind` of `kinds` is set.
#[inline]
fn has_kind(kinds: u64, kind: u8) -> bool {
    kind < 64 && kinds >> kind & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_and_bounds() {
        let mut map = TileMap::new(Rect::sized(64, -8, 4, 3));
        assert_eq!((map.width(), map.height(), map.as_slice().len()), (4, 3, 12));
        assert_eq!(map.get(Pos::new(64, -8)), Some(0));
        assert!(map.set(Pos::new(67, -6), 9));
        assert_eq!(map.get(Pos::new(67, -6)), Some(9));
        assert_eq!(map.as_slice()[11], 9);
        assert!(!map.set(Pos::new(68, -6), 9));
        assert_eq!(map.get(Pos::new(63, -8)), None);
        assert_eq!(map.get(Pos::new(64, -5)), None);
        assert_eq!(map.bounds(), Rect::new(64, -8, 68, -5));
    }

    #[test]
    fn fill_clips_to_the_map() {
        let mut map = TileMap::new(Rect::sized(0, 0, 4, 4));
        map.fill(Rect::new(2, 1, 99, 3), 7);
        assert_eq!(map.as_slice(), [0, 0, 0, 0, 0, 0, 7, 7, 0, 0, 7, 7, 0, 0, 0, 0]);
        assert_eq!(map.count(7), 4);
        map.fill(Rect::new(10, 10, 20, 20), 5);
        map.fill(Rect::new(3, 3, 3, 9), 5);
        assert_eq!(map.count(5), 0);
    }

    #[test]
    fn tiles_of_some_kinds_are_counted_and_found_row_by_row() {
        // Kinds 1 and 3 in a 5 x 4 map at (10, 20):
        //   . 1 . 3 .
        //   9 . . . 1
        //   . . 3 . .
        //   1 . . . 70
        let mut map = TileMap::new(Rect::sized(10, 20, 5, 4));
        for (x, y, kind) in [
            (11, 20, 1),
            (13, 20, 3),
            (10, 21, 9),
            (14, 21, 1),
            (12, 22, 3),
            (10, 23, 1),
            (14, 23, 70),
        ] {
            assert!(map.set(Pos::new(x, y), kind));
        }
        let kinds = 1 << 1 | 1 << 3;
        let all = Rect::sized(10, 20, 5, 4);
        assert_eq!(map.count_in(all, kinds), 5);
        let found: Vec<Pos> = (0..5).map(|n| map.nth_in(all, kinds, n).unwrap()).collect();
        assert_eq!(
            found,
            [
                Pos::new(11, 20),
                Pos::new(13, 20),
                Pos::new(14, 21),
                Pos::new(12, 22),
                Pos::new(10, 23)
            ]
        );
        assert_eq!(map.nth_in(all, kinds, 5), None);
        // A part of the map, and a rectangle that is partly outside it.
        let part = Rect::new(12, 21, 15, 23);
        assert_eq!(map.count_in(part, kinds), 2);
        assert_eq!(map.nth_in(part, kinds, 1), Some(Pos::new(12, 22)));
        let wide = Rect::new(-100, 22, 100, 100);
        assert_eq!(map.count_in(wide, kinds), 2);
        assert_eq!(map.nth_in(wide, kinds, 1), Some(Pos::new(10, 23)));
        // Kind 0 is a kind like any other. Kinds above 63 are never found.
        assert_eq!(map.count_in(all, 1), 13);
        assert_eq!(map.count_in(all, u64::MAX), 19);
        assert_eq!(map.count_in(all, 0), 0);
        // Outside the map, and an empty rectangle.
        assert_eq!(map.count_in(Rect::new(15, 20, 30, 30), u64::MAX), 0);
        assert_eq!(map.nth_in(Rect::new(0, 0, 10, 20), u64::MAX, 0), None);
        assert_eq!(map.count_in(Rect::new(12, 21, 12, 23), u64::MAX), 0);
        assert_eq!(map.count_in(Rect::new(14, 23, 10, 20), u64::MAX), 0);
    }

    #[test]
    fn an_empty_map_has_no_tiles() {
        let mut map = TileMap::default();
        assert_eq!(map.get(Pos::new(0, 0)), None);
        assert!(!map.set(Pos::new(0, 0), 1));
        assert!(map.as_slice().is_empty());
    }
}
