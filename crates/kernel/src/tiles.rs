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
    fn an_empty_map_has_no_tiles() {
        let mut map = TileMap::default();
        assert_eq!(map.get(Pos::new(0, 0)), None);
        assert!(!map.set(Pos::new(0, 0), 1));
        assert!(map.as_slice().is_empty());
    }
}
