//! What an event says: who is in it, where it was, and one line of words.
//!
//! The director, the stories and the chronicle all read events through
//! this module, so that they agree about the residents and the place of an
//! event and use the same words for it.

mod phrase;
mod subjects;
mod text;

use unsup_protocol::{CityId, Pos, ResidentId, SiteId, ZoneId};

pub(crate) use self::phrase::{Phrasebook, Template};
pub(crate) use self::subjects::{Facts, Reader};
pub(crate) use self::text::clean;

/// A site, as the atlas knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SiteRef {
    /// Id of the site.
    pub id: SiteId,
    /// Name of the site. It can be text that a resident chose.
    pub name: String,
    /// The zone of the site.
    pub zone: ZoneId,
    /// The middle of the site.
    pub centre: Pos,
}

/// A city with its number of residents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CityRef {
    /// Id of the city.
    pub id: CityId,
    /// Name of the city.
    pub name: String,
    /// The middle of the city.
    pub centre: Pos,
    /// Residents in the city now.
    pub residents: u64,
}

/// What the service knows about the world when it reads an event: names
/// and places. The cache is the atlas of the running service; tests use a
/// small one.
pub(crate) trait Atlas {
    /// Returns the name of a resident: one that lives, or one that the
    /// service still remembers.
    fn resident_name(&self, id: ResidentId) -> Option<String>;
    /// Returns where a living resident is.
    fn resident_place(&self, id: ResidentId) -> Option<(ZoneId, Pos)>;
    /// Returns a site by its id.
    fn site(&self, id: SiteId) -> Option<SiteRef>;
    /// Returns the site that holds a position of a zone.
    fn site_at(&self, zone: ZoneId, pos: Pos) -> Option<SiteRef>;
    /// Returns the name and the city of a zone.
    fn zone(&self, zone: ZoneId) -> Option<(String, CityId)>;
    /// Returns the name of a city.
    fn city_name(&self, city: CityId) -> Option<String>;
    /// Returns every city with its number of residents, in ascending id
    /// order.
    fn cities(&self) -> Vec<CityRef>;
}

/// An atlas for tests: what a test puts into it, and nothing else.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestAtlas {
    /// Names of residents.
    pub names: std::collections::BTreeMap<ResidentId, String>,
    /// Places of living residents.
    pub places: std::collections::BTreeMap<ResidentId, (ZoneId, Pos)>,
    /// Sites.
    pub sites: Vec<SiteRef>,
    /// Zones: name and city.
    pub zones: std::collections::BTreeMap<ZoneId, (String, CityId)>,
    /// Cities.
    pub cities: Vec<CityRef>,
}

#[cfg(test)]
impl TestAtlas {
    /// Makes a country of two cities with two zones each, four named
    /// residents and one site in zone 0.
    pub fn town() -> TestAtlas {
        let mut atlas = TestAtlas::default();
        for (id, name) in [(1, "Ada"), (2, "Bo"), (3, "Cleo"), (4, "Dev")] {
            atlas.names.insert(ResidentId(id), name.to_string());
            atlas.places.insert(ResidentId(id), (ZoneId(0), Pos::new(id as i32, 1)));
        }
        atlas.sites.push(SiteRef {
            id: SiteId(900),
            name: "First Bank".to_string(),
            zone: ZoneId(0),
            centre: Pos::new(10, 10),
        });
        for (zone, name, city) in
            [(0, "Old Town", 0), (1, "Docks", 0), (2, "Mill Row", 1), (3, "Hill", 1)]
        {
            atlas.zones.insert(ZoneId(zone), (name.to_string(), CityId(city)));
        }
        atlas.cities = vec![
            CityRef {
                id: CityId(0),
                name: "Latent City".to_string(),
                centre: Pos::new(32, 32),
                residents: 30,
            },
            CityRef {
                id: CityId(1),
                name: "Tokentown".to_string(),
                centre: Pos::new(160, 32),
                residents: 10,
            },
        ];
        atlas
    }
}

#[cfg(test)]
impl Atlas for TestAtlas {
    fn resident_name(&self, id: ResidentId) -> Option<String> {
        self.names.get(&id).cloned()
    }

    fn resident_place(&self, id: ResidentId) -> Option<(ZoneId, Pos)> {
        self.places.get(&id).copied()
    }

    fn site(&self, id: SiteId) -> Option<SiteRef> {
        self.sites.iter().find(|site| site.id == id).cloned()
    }

    fn site_at(&self, zone: ZoneId, pos: Pos) -> Option<SiteRef> {
        self.sites.iter().find(|site| site.zone == zone && site.centre == pos).cloned()
    }

    fn zone(&self, zone: ZoneId) -> Option<(String, CityId)> {
        self.zones.get(&zone).cloned()
    }

    fn city_name(&self, city: CityId) -> Option<String> {
        self.cities.iter().find(|known| known.id == city).map(|known| known.name.clone())
    }

    fn cities(&self) -> Vec<CityRef> {
        self.cities.clone()
    }
}

/// Makes a feed event for tests.
#[cfg(test)]
pub(crate) fn test_event(
    frame: u64,
    index: u32,
    kind: &str,
    drama: u8,
    actor: Option<u64>,
    data: serde_json::Value,
) -> std::sync::Arc<crate::wire::FeedEvent> {
    use unsup_protocol::{Clock, RealmId, Tick};
    std::sync::Arc::new(crate::wire::FeedEvent {
        id: crate::wire::EventId { frame, index },
        frame,
        clock: Clock { year: 1, day: 1, hour: (frame / 60 % 24) as u8, minute: (frame % 60) as u8 },
        tick: Tick(frame),
        realm: RealmId::Zone(ZoneId(0)),
        zone: Some(ZoneId(0)),
        kind: kind.to_string(),
        actor: actor.map(ResidentId),
        actor_name: actor.and_then(|id| TestAtlas::town().names.get(&ResidentId(id)).cloned()),
        pos: Some(Pos::new(5, 5)),
        drama,
        data,
    })
}
