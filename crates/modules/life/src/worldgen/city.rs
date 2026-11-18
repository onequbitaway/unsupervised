//! The plan of a city: the type of each district, and the sites that each
//! district must have so that the city has every kind of site.
//!
//! The plan is a pure function of the seed and the shape of the city. Every
//! zone of the city computes the same plan and builds its own part.

use unsup_kernel::{CityInfo, Rng};

use super::kinds::{District, KINDS, Kind, Role};
use super::params::GenParams;
use super::zone::{MIN_LOT, spans};

/// First random stream of the city plans. Realms use the streams from 0.
const CITY_STREAM: u64 = 1 << 40;

/// Position of each district type in [`District::RINGS`], in the order in
/// which a city gets them when it has fewer zones than types: the centre,
/// then homes, then food, then work, then shops.
const NEED: [usize; 5] = [0, 2, 4, 3, 1];

/// The plan of one city.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CityPlan {
    /// Type of each district, in the order of `CityInfo::zones`.
    pub(crate) districts: Vec<District>,
    /// Kinds of the sites that each district must have, as positions in
    /// [`KINDS`], in the order of `CityInfo::zones`.
    pub(crate) required: Vec<Vec<usize>>,
}

/// Makes the plan of a city.
pub(crate) fn plan_city(
    seed: u64,
    city: &CityInfo,
    zone_size: u32,
    params: &GenParams,
) -> CityPlan {
    let districts = districts(seed, city, params);
    let required = required(&districts, zone_size, params);
    CityPlan { districts, required }
}

/// Gives every zone of a city a district type. The centre is downtown. The
/// other types follow from the centre to the edge: commercial, residential,
/// industrial, rural. Zones at the same distance from the centre come in a
/// seeded random order.
fn districts(seed: u64, city: &CityInfo, params: &GenParams) -> Vec<District> {
    let zones = city.zones.len();
    if zones <= 1 {
        return vec![District::Mixed; zones];
    }
    let side = i64::from(city.districts.max(1));
    // Distance of a zone from the centre of the city, in half zones.
    let ring = |slot: usize| {
        let col = slot as i64 % side;
        let row = slot as i64 / side;
        (2 * col - (side - 1)).abs().max((2 * row - (side - 1)).abs())
    };
    let mut rng = Rng::stream(seed, CITY_STREAM + u64::from(city.id.0));
    let mut order: Vec<usize> = (0..zones).collect();
    rng.shuffle(&mut order);
    order.sort_by_key(|&slot| ring(slot));

    let mut out = vec![District::Mixed; zones];
    let mut next = order.into_iter();
    for (district, count) in District::RINGS.into_iter().zip(seats(zones, &params.share)) {
        for slot in next.by_ref().take(count) {
            out[slot] = district;
        }
    }
    out
}

/// Returns how many zones each type of [`District::RINGS`] gets. Each type
/// gets one zone when there are enough. The rest follows the shares, by
/// largest remainder.
fn seats(zones: usize, share: &[i64; 5]) -> [usize; 5] {
    let mut seats = [0usize; 5];
    for &slot in NEED.iter().take(zones) {
        seats[slot] = 1;
    }
    let Some(rest) = zones.checked_sub(seats.len()).filter(|&rest| rest > 0) else {
        return seats;
    };
    let total: i64 = share.iter().sum();
    if total <= 0 {
        // No shares: the rest are homes.
        seats[2] += rest;
        return seats;
    }
    let mut given = 0usize;
    let mut remainders = [(0i64, 0usize); 5];
    for (slot, &part) in share.iter().enumerate() {
        let exact = rest as i64 * part;
        let whole = (exact / total) as usize;
        seats[slot] += whole;
        given += whole;
        remainders[slot] = (exact % total, slot);
    }
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, slot) in remainders.iter().take(rest - given) {
        seats[slot] += 1;
    }
    seats
}

/// Returns the room of one zone in quarters of a block, and whether a block
/// can be cut into lots.
fn room(zone_size: u32, params: &GenParams) -> (u32, bool) {
    let side = spans(i32::try_from(zone_size).unwrap_or(i32::MAX), params.block, params.street);
    let blocks = (side.len() * side.len()) as u32;
    let cut = side.iter().all(|&(_, length)| length >= 2 * MIN_LOT);
    (if cut { blocks * 4 } else { blocks }, cut)
}

/// Returns how many sites of a kind a city with `zones` zones must have.
fn quota(kind: &Kind, zones_per: i64, zones: usize) -> usize {
    match kind.role {
        Role::Unique | Role::Filler => 1,
        Role::Service => {
            let per = usize::try_from(zones_per).unwrap_or(1).max(1);
            zones.div_ceil(per).max(1)
        }
    }
}

/// Spreads the sites that a city must have over its districts. A site goes
/// to a district of the type that its kind prefers, to one that does not
/// have the kind yet, and to the one with the most room left.
fn required(districts: &[District], zone_size: u32, params: &GenParams) -> Vec<Vec<usize>> {
    let (capacity, cut) = room(zone_size, params);
    let mut out = vec![Vec::new(); districts.len()];
    let mut load = vec![0u32; districts.len()];
    for (index, kind) in KINDS.iter().enumerate() {
        let need = if cut { kind.size.quarters() } else { 1 };
        let rank = |district: District| {
            kind.prefers.iter().position(|d| *d == district).unwrap_or(kind.prefers.len())
        };
        for _ in 0..quota(kind, params.zones_per[index], districts.len()) {
            let best =
                (0..districts.len()).filter(|&zone| load[zone] + need <= capacity).min_by_key(
                    |&zone| (out[zone].contains(&index), rank(districts[zone]), load[zone], zone),
                );
            if let Some(zone) = best {
                out[zone].push(index);
                load[zone] += need;
            }
        }
    }
    out
}
