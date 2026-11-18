//! The numbers of the world generator, read from the parameters.

use unsup_kernel::Realm;

use super::kinds::{District, KINDS};
use crate::names::key;

/// The parameters `life.gen.*` and `life.site.*` as tables.
#[derive(Clone, Debug)]
pub(crate) struct GenParams {
    /// Side of a block in tiles.
    pub(crate) block: i32,
    /// Width of a street in tiles.
    pub(crate) street: i32,
    /// Percent of the free tiles around a building that get a tree.
    pub(crate) margin_trees_pct: u64,
    /// Width in tiles of the free ground around the rock of a mine.
    pub(crate) mine_rim: i32,
    /// The pond of a park is this part of each side of the park.
    pub(crate) pond_divisor: i32,
    /// Greatest difference of a site value from the value of its kind, in
    /// percent.
    pub(crate) value_jitter_pct: i64,
    /// Share of each district type of [`District::RINGS`].
    pub(crate) share: [i64; 5],
    /// Zones for each site of a service kind, by kind.
    pub(crate) zones_per: [i64; KINDS.len()],
    /// Weight of each kind in the free lots of each district type.
    pub(crate) mix: [[i64; KINDS.len()]; District::ALL.len()],
    /// Start value of each kind in cents.
    pub(crate) value: [i64; KINDS.len()],
    /// Beds of each kind. 0 means that the kind has no beds.
    pub(crate) beds: [i64; KINDS.len()],
}

impl GenParams {
    /// Reads the tables. A number that the data file does not have is 0.
    pub(crate) fn read(realm: &Realm) -> GenParams {
        let get = |name: &str| realm.defs().param_id(name).map_or(0, |id| realm.param(id));
        let kinds = |pattern: &dyn Fn(&str) -> String| {
            let mut out = [0i64; KINDS.len()];
            for (slot, kind) in out.iter_mut().zip(KINDS.iter()) {
                *slot = get(&pattern(kind.name));
            }
            out
        };
        let mut mix = [[0i64; KINDS.len()]; District::ALL.len()];
        for district in District::ALL {
            mix[district.index()] =
                kinds(&|kind| format!("life.gen.mix.{}.{kind}", district.name()));
        }
        GenParams {
            block: i32::try_from(get("life.gen.block")).unwrap_or(0).max(1),
            street: i32::try_from(get("life.gen.street")).unwrap_or(0).max(1),
            margin_trees_pct: u64::try_from(get("life.gen.margin_trees_pct")).unwrap_or(0),
            mine_rim: i32::try_from(get("life.gen.mine_rim")).unwrap_or(0).max(1),
            pond_divisor: i32::try_from(get("life.gen.pond_divisor")).unwrap_or(0).max(2),
            value_jitter_pct: get("life.gen.value_jitter_pct").clamp(0, 99),
            share: District::RINGS
                .map(|district| get(&format!("life.gen.share.{}", district.name())).max(0)),
            zones_per: kinds(&|kind| format!("life.gen.zones_per.{kind}")),
            mix,
            value: kinds(&|kind| format!("life.site.{kind}.{}", key::VALUE)),
            beds: kinds(&|kind| format!("life.site.{kind}.{}", key::BEDS)),
        }
    }
}
