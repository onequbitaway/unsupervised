//! Journeys between cities: the verb `travel`.
//!
//! A resident travels from a station to a station of another city. Every
//! zone keeps a list of all stations of the world. A zone with a station
//! tells all zones about it when the world is made.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Begin, Builder, CityId, Dest, EventMeta, Holder, Message, Money, OrgId, Pos,
    Realm, Resource, Scope, SiteId, VerbSpec, ZoneId,
};

use crate::handles::Handles;
use crate::names::site;
use crate::sites::{site_here, standing_tile};

/// One station.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Station {
    /// The city of the station.
    pub city: CityId,
    /// The site.
    pub site: SiteId,
    /// The zone of the station.
    pub zone: ZoneId,
    /// The tile inside the station where a traveller arrives.
    pub pos: Pos,
}

impl Message for Station {
    const KIND: &'static str = "life.station";
}

/// Every station of the world, sorted by city and site. Each zone has a
/// copy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stations {
    list: Vec<Station>,
}

impl Stations {
    /// Returns the stations of a city.
    pub fn of_city(&self, city: CityId) -> impl Iterator<Item = &Station> + '_ {
        self.list.iter().filter(move |station| station.city == city)
    }

    fn add(&mut self, station: Station) {
        if let Err(slot) = self.list.binary_search(&station) {
            self.list.insert(slot, station);
        }
    }
}

impl Resource for Stations {
    const NAME: &'static str = "life.stations";
}

/// Arguments of `travel`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TravelArgs {
    /// Id of the city to go to.
    pub city: CityId,
}

/// Tells every zone about the stations of this zone. World generation
/// calls it.
pub(crate) fn announce(realm: &Realm) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let stations: Vec<Station> = {
        let sites = realm.sites();
        let tiles = realm.tiles();
        sites
            .of_kind(site::STATION)
            .filter_map(|place| {
                let pos = standing_tile(&tiles, place.rect)?;
                Some(Station { city: info.city, site: place.id, zone: info.id, pos })
            })
            .collect()
    };
    for station in stations {
        realm.res_mut::<Stations>().add(station);
        realm.send(Dest::AllZones, &station);
    }
}

/// Registers the verb `travel` and the list of stations.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.resource::<Stations>();
        b.on_message::<Station>(|realm, _from, station| {
            realm.res_mut::<Stations>().add(station);
        });
    }
    b.verb::<TravelArgs>(
        VerbSpec {
            verb: "travel",
            module: "life",
            summary: "Go from the station where you are to a station of another city. You pay the fare first.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| {
            let Some(zone) = realm.zone_info() else {
                return Err(ActionError::internal("`travel` needs a zone."));
            };
            if realm.layout().city(args.city).is_none() {
                return Err(ActionError::not_found(format!("No city has the id {}.", args.city)));
            }
            if args.city == zone.city {
                return Err(ActionError::invalid_args("You are in that city already."));
            }
            if site_here(realm, ctx.idx).is_none_or(|place| place.kind != site::STATION) {
                return Err(ActionError::not_here("You must be in a station."));
            }
            if realm.res::<Stations>().of_city(args.city).next().is_none() {
                return Err(ActionError::not_found(format!(
                    "City {} has no station that is known here.",
                    args.city
                )));
            }
            let fare = Money(realm.param(h.p.travel_fare));
            realm.wire_money(
                Holder::Resident(ctx.resident),
                Dest::Global,
                Holder::Org(OrgId::city(zone.city)),
                fare,
                "fare",
            )?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.travel_ticks) })
        },
        move |realm, ctx, args| {
            // Something else can move a resident while it waits for the
            // journey: an arrest, for example.
            if site_here(realm, ctx.idx).is_none_or(|place| place.kind != site::STATION) {
                return Err(ActionError::conflict(
                    "You left the station before the journey started.",
                ));
            }
            let from = realm.zone_info().map(|zone| zone.city);
            // Travellers spread over the stations of the city by their id.
            let station = {
                let stations = realm.res::<Stations>();
                let count = stations.of_city(args.city).count();
                let pick = (ctx.resident.0 % count.max(1) as u64) as usize;
                stations.of_city(args.city).nth(pick).copied()
            };
            let Some(station) = station else {
                return Err(ActionError::not_found(format!("City {} has no station.", args.city)));
            };
            realm.teleport(ctx.idx, station.zone, station.pos).map_err(|error| {
                ActionError::internal(format!("The journey failed: {error}."))
            })?;
            realm.emit(
                EventMeta::public("life.traveled", h.drama(realm, h.p.drama_traveled))
                    .actor(ctx.resident)
                    .at(station.pos),
                &json!({
                    "resident": ctx.resident,
                    "from_city": from,
                    "to_city": args.city,
                    "site": station.site,
                }),
            );
            Ok(json!({
                "city": args.city,
                "zone": station.zone,
                "site": station.site,
                "x": station.pos.x,
                "y": station.pos.y,
            }))
        },
    );
}
