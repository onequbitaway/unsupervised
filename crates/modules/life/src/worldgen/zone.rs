//! The plan of one zone: roads, blocks, lots, and the tiles and the
//! rectangle of every site.
//!
//! The plan is a pure function of the seed, the zone, its district type and
//! the sites that it must have.
//!
//! The rules:
//!
//! - The tiles at the edge of the zone are road. So a resident can cross
//!   every border between two zones of a city at every tile.
//! - Streets cut the rest of the zone into blocks. Every block has a road
//!   on each of its four sides.
//! - A block is one large lot, or two medium lots, or four small lots, or a
//!   mix. So every lot touches a road.
//! - A building stands one tile inside its lot. Its door is in the middle of
//!   a wall that faces a road, and pavement goes from the door to the road.

use unsup_kernel::{Pos, Rect, Rng, ZoneInfo};

use super::kinds::{District, KINDS, Shape, Size};
use super::params::GenParams;
use crate::names::tile;

/// First random stream of the zone plans. Realms use the streams from 0 and
/// city plans the streams from `1 << 40`.
const ZONE_STREAM: u64 = 2 << 40;

/// Shortest side of a lot in tiles: a building of three by three tiles and
/// one free tile on each side.
pub(crate) const MIN_LOT: i32 = 5;

/// Shortest side of a building in tiles: one tile of floor between walls.
const MIN_BUILDING: u32 = 3;

/// The tiles of a zone while it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Canvas {
    bounds: Rect,
    tiles: Vec<u8>,
}

impl Canvas {
    fn new(bounds: Rect, code: u8) -> Canvas {
        Canvas { bounds, tiles: vec![code; bounds.width() as usize * bounds.height() as usize] }
    }

    fn slot(&self, pos: Pos) -> Option<usize> {
        self.bounds.contains(pos).then(|| {
            pos.y.abs_diff(self.bounds.y0) as usize * self.bounds.width() as usize
                + pos.x.abs_diff(self.bounds.x0) as usize
        })
    }

    /// Returns the code of a tile, or `None` outside the zone.
    pub(crate) fn get(&self, pos: Pos) -> Option<u8> {
        self.slot(pos).map(|slot| self.tiles[slot])
    }

    fn set(&mut self, pos: Pos, code: u8) {
        if let Some(slot) = self.slot(pos) {
            self.tiles[slot] = code;
        }
    }

    fn fill(&mut self, rect: Rect, code: u8) {
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                self.set(Pos::new(x, y), code);
            }
        }
    }

    /// Returns every tile with its position, row by row.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Pos, u8)> + '_ {
        let width = self.bounds.width().max(1) as usize;
        self.tiles.iter().enumerate().map(move |(slot, &code)| {
            let pos = Pos::new(
                self.bounds.x0 + (slot % width) as i32,
                self.bounds.y0 + (slot / width) as i32,
            );
            (pos, code)
        })
    }
}

/// One site of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SitePlan {
    /// Position of the kind in [`KINDS`].
    pub(crate) kind: usize,
    /// The tiles of the site: the walls of a building, or the whole lot of
    /// open land.
    pub(crate) rect: Rect,
    /// Start value in cents.
    pub(crate) value: i64,
}

/// The plan of one zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ZonePlan {
    pub(crate) canvas: Canvas,
    pub(crate) sites: Vec<SitePlan>,
}

/// A piece of a block that gets one site.
#[derive(Clone, Copy, Debug)]
struct Lot {
    rect: Rect,
    /// How often the block was cut to make the lot: 0, 1 or 2.
    level: u8,
    /// Which sides of the lot touch a road: north, east, south, west.
    roads: [bool; 4],
}

/// Cuts one side of a zone into blocks. Returns the offset from the edge of
/// the zone and the length of each block. The first and the last tile are
/// the road at the edge; `street` tiles of road lie between two blocks.
pub(crate) fn spans(len: i32, block: i32, street: i32) -> Vec<(i32, i32)> {
    let inside = len - 2;
    if inside < 1 {
        return Vec::new();
    }
    let block = block.max(1);
    let street = street.max(1);
    let count = ((inside + street) / (block + street)).max(1);
    let ground = inside - (count - 1) * street;
    let base = ground / count;
    let extra = ground % count;
    let mut out = Vec::with_capacity(count as usize);
    let mut offset = 1;
    for index in 0..count {
        let length = base + i32::from(index < extra);
        out.push((offset, length));
        offset += length + street;
    }
    out
}

/// Cuts a lot in two across its longer side. Returns `None` when the lot is
/// a quarter of a block already or when the halves would be too small.
fn split(lot: &Lot, rng: &mut Rng) -> Option<(Lot, Lot)> {
    let r = lot.rect;
    let (width, height) = (r.x1 - r.x0, r.y1 - r.y0);
    if lot.level >= Size::Small.level() || width.max(height) < 2 * MIN_LOT {
        return None;
    }
    let across = if width == height { rng.chance(1, 2) } else { width > height };
    let level = lot.level + 1;
    let [north, east, south, west] = lot.roads;
    Some(if across {
        let cut = r.x0 + width / 2;
        (
            Lot {
                rect: Rect::new(r.x0, r.y0, cut, r.y1),
                level,
                roads: [north, false, south, west],
            },
            Lot {
                rect: Rect::new(cut, r.y0, r.x1, r.y1),
                level,
                roads: [north, east, south, false],
            },
        )
    } else {
        let cut = r.y0 + height / 2;
        (
            Lot {
                rect: Rect::new(r.x0, r.y0, r.x1, cut),
                level,
                roads: [north, east, false, west],
            },
            Lot {
                rect: Rect::new(r.x0, cut, r.x1, r.y1),
                level,
                roads: [false, east, south, west],
            },
        )
    })
}

/// The lots of a zone that have no site yet, by level.
struct FreeLots {
    by_level: [Vec<Lot>; 3],
}

impl FreeLots {
    /// Takes a lot of a level. When none is free, it cuts a larger lot and
    /// keeps the other half. A lot that is too small to cut is given whole.
    fn take(&mut self, level: u8, rng: &mut Rng) -> Option<Lot> {
        if let Some(lot) = self.by_level[usize::from(level)].pop() {
            return Some(lot);
        }
        let parent = self.take(level.checked_sub(1)?, rng)?;
        Some(match split(&parent, rng) {
            Some((first, second)) => {
                self.by_level[usize::from(level)].push(second);
                first
            }
            None => parent,
        })
    }
}

/// Draws a zone.
struct Painter<'a> {
    canvas: Canvas,
    rng: Rng,
    params: &'a GenParams,
    district: District,
    sites: Vec<SitePlan>,
}

impl Painter<'_> {
    /// Puts a site of a random kind on a free lot. The kind comes from the
    /// mix of the district, among the kinds that fit on the lot.
    fn fill(&mut self, lot: Lot) {
        let params = self.params;
        let weights = &params.mix[self.district.index()];
        let fits = |kind: usize| KINDS[kind].size.level() >= lot.level && weights[kind] > 0;
        let total: i64 =
            (0..KINDS.len()).filter(|&kind| fits(kind)).map(|kind| weights[kind]).sum();
        if total <= 0 {
            // Nothing of this district fits here. The lot stays green.
            self.scatter(lot.rect, tile::TREES);
            return;
        }
        let mut roll = self.rng.range(0, total);
        for kind in (0..KINDS.len()).filter(|&kind| fits(kind)) {
            if roll < weights[kind] {
                self.fill_with(kind, lot);
                return;
            }
            roll -= weights[kind];
        }
    }

    /// Puts a site of a given kind on a lot. A lot that is larger than the
    /// kind needs is cut, and the other half gets a random site.
    fn fill_with(&mut self, kind: usize, lot: Lot) {
        if KINDS[kind].size.level() > lot.level
            && let Some((first, second)) = split(&lot, &mut self.rng)
        {
            self.fill_with(kind, first);
            self.fill(second);
            return;
        }
        self.place(kind, &lot);
    }

    /// Draws a site on a lot and adds it to the plan.
    fn place(&mut self, kind: usize, lot: &Lot) {
        let r = lot.rect;
        let rect = match KINDS[kind].shape {
            Shape::Building => match self.building(kind, lot) {
                Some(walls) => walls,
                None => return,
            },
            Shape::Field => {
                self.canvas.fill(r, tile::FIELD);
                r
            }
            Shape::Woods => {
                self.canvas.fill(r, tile::TREES);
                r
            }
            Shape::Quarry => {
                let rim = self.params.mine_rim;
                let rock = Rect::new(r.x0 + rim, r.y0 + rim, r.x1 - rim, r.y1 - rim);
                if rock.is_empty() {
                    self.canvas.set(r.centre(), tile::ROCK);
                } else {
                    self.canvas.fill(rock, tile::ROCK);
                }
                r
            }
            Shape::Garden => {
                self.scatter(r, tile::TREES);
                let part = self.params.pond_divisor;
                let (width, height) = ((r.x1 - r.x0) / part, (r.y1 - r.y0) / part);
                let centre = r.centre();
                let pond = Rect::new(
                    centre.x - width / 2,
                    centre.y - height / 2,
                    centre.x - width / 2 + width.max(1),
                    centre.y - height / 2 + height.max(1),
                );
                if width >= 1 && height >= 1 {
                    self.canvas.fill(pond, tile::WATER);
                }
                r
            }
        };
        let base = self.params.value[kind];
        let jitter = self.params.value_jitter_pct;
        let value =
            base.saturating_add(base.saturating_mul(self.rng.range(-jitter, jitter + 1)) / 100);
        self.sites.push(SitePlan { kind, rect, value: value.max(0) });
    }

    /// Draws a building on a lot: walls, floor, doors and the pavement in
    /// front of each door. Returns the rectangle of the walls, or `None`
    /// when the lot has no room for a building.
    fn building(&mut self, kind: usize, lot: &Lot) -> Option<Rect> {
        let r = lot.rect;
        let inset = i32::from(r.x1 - r.x0 >= MIN_LOT && r.y1 - r.y0 >= MIN_LOT);
        let walls = Rect::new(r.x0 + inset, r.y0 + inset, r.x1 - inset, r.y1 - inset);
        if walls.width() < MIN_BUILDING || walls.height() < MIN_BUILDING {
            return None;
        }
        self.scatter(r, tile::TREES);
        self.canvas.fill(walls, tile::WALL);
        self.canvas
            .fill(Rect::new(walls.x0 + 1, walls.y0 + 1, walls.x1 - 1, walls.y1 - 1), tile::FLOOR);
        let mut sides: Vec<usize> = (0..4).filter(|&side| lot.roads[side]).collect();
        if sides.is_empty() {
            sides = vec![0, 1, 2, 3];
        }
        let front = sides[self.rng.below(sides.len() as u64) as usize];
        self.door(walls, r, front);
        let back = (front + 2) % 4;
        if KINDS[kind].size == Size::Large && lot.roads[back] {
            self.door(walls, r, back);
        }
        Some(walls)
    }

    /// Opens a door in the middle of one wall and paves the way from the
    /// door to the edge of the lot.
    fn door(&mut self, walls: Rect, lot: Rect, side: usize) {
        let middle = walls.centre();
        let (door, (dx, dy)) = match side {
            0 => (Pos::new(middle.x, walls.y0), (0, -1)),
            1 => (Pos::new(walls.x1 - 1, middle.y), (1, 0)),
            2 => (Pos::new(middle.x, walls.y1 - 1), (0, 1)),
            _ => (Pos::new(walls.x0, middle.y), (-1, 0)),
        };
        self.canvas.set(door, tile::FLOOR);
        let mut way = Pos::new(door.x + dx, door.y + dy);
        while lot.contains(way) {
            self.canvas.set(way, tile::PAVEMENT);
            way = Pos::new(way.x + dx, way.y + dy);
        }
    }

    /// Changes some of the grass tiles of a rectangle to another code. The
    /// parameter `life.gen.margin_trees_pct` says how many.
    fn scatter(&mut self, rect: Rect, code: u8) {
        let percent = self.params.margin_trees_pct;
        if percent == 0 {
            return;
        }
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let pos = Pos::new(x, y);
                if self.canvas.get(pos) == Some(tile::GRASS) && self.rng.chance(percent, 100) {
                    self.canvas.set(pos, code);
                }
            }
        }
    }
}

/// Makes the plan of one zone. `required` holds the kinds that the zone
/// must have, as positions in [`KINDS`].
pub(crate) fn plan_zone(
    seed: u64,
    zone: &ZoneInfo,
    district: District,
    required: &[usize],
    params: &GenParams,
) -> ZonePlan {
    let bounds = zone.bounds;
    let mut rng = Rng::stream(seed, ZONE_STREAM + u64::from(zone.id.0));
    let mut canvas = Canvas::new(bounds, tile::ROAD);

    let side =
        |len: u32| spans(i32::try_from(len).unwrap_or(i32::MAX), params.block, params.street);
    let mut blocks = Vec::new();
    for &(top, height) in &side(bounds.height()) {
        for &(left, width) in &side(bounds.width()) {
            let rect = Rect::new(
                bounds.x0 + left,
                bounds.y0 + top,
                bounds.x0 + left + width,
                bounds.y0 + top + height,
            );
            canvas.fill(rect, tile::GRASS);
            blocks.push(Lot { rect, level: 0, roads: [true; 4] });
        }
    }
    rng.shuffle(&mut blocks);
    let mut free = FreeLots { by_level: [blocks, Vec::new(), Vec::new()] };
    let mut painter = Painter { canvas, rng, params, district, sites: Vec::new() };

    // The sites that the zone must have come first, the largest first, so
    // that each finds a lot of its size.
    let mut required = required.to_vec();
    required.sort_by_key(|&kind| KINDS[kind].size.level());
    for kind in required {
        if let Some(lot) = free.take(KINDS[kind].size.level(), &mut painter.rng) {
            painter.place(kind, &lot);
        }
    }
    // The lots that are left get what the district is made of. Pieces of
    // blocks come first, whole blocks last.
    for level in [2, 1, 0] {
        for lot in std::mem::take(&mut free.by_level[level]) {
            painter.fill(lot);
        }
    }
    ZonePlan { canvas: painter.canvas, sites: painter.sites }
}
