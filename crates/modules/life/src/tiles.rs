//! How long a step onto each kind of tile takes.

use unsup_kernel::Realm;

use crate::handles::Params;
use crate::names::tile;

/// Number of tile codes that the module knows.
const CODES: usize = 9;

/// Ticks that one step onto a tile takes, by tile code. A tile that a
/// resident cannot enter has no entry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Costs {
    ticks: [u32; CODES],
    /// The smallest cost of a walkable tile. The path search uses it for
    /// its estimate.
    pub(crate) min: u32,
}

impl Costs {
    /// Reads the costs from the parameters of the realm.
    pub(crate) fn read(realm: &Realm, p: &Params) -> Costs {
        let ticks = |id| u32::try_from(realm.param(id)).unwrap_or(1).max(1);
        let mut costs = Costs { ticks: [0; CODES], min: u32::MAX };
        for (code, value) in [
            (tile::GRASS, ticks(p.walk_ticks_grass)),
            (tile::ROAD, ticks(p.walk_ticks_road)),
            (tile::PAVEMENT, ticks(p.walk_ticks_pavement)),
            (tile::FLOOR, ticks(p.walk_ticks_floor)),
            (tile::TREES, ticks(p.walk_ticks_trees)),
            (tile::FIELD, ticks(p.walk_ticks_field)),
        ] {
            costs.ticks[usize::from(code)] = value;
            costs.min = costs.min.min(value);
        }
        costs
    }

    /// Returns the ticks that one step onto a tile with this code takes, or
    /// `None` when a resident cannot enter the tile.
    #[inline]
    pub(crate) fn of(&self, code: u8) -> Option<u32> {
        match self.ticks.get(usize::from(code)) {
            Some(&ticks) if ticks > 0 => Some(ticks),
            _ => None,
        }
    }

    /// Returns the cost of the step that leaves a zone. Every tile at the
    /// edge of a zone is a road.
    pub(crate) fn border(&self) -> u32 {
        self.ticks[usize::from(tile::ROAD)]
    }
}
