//! The read-only cache of the world.
//!
//! The cache holds what viewers ask for: the residents of every zone, the
//! number of residents per zone, the tiles and sites, the recent public
//! events, the published state and the residents that left the world. One
//! task feeds it from the frames of the simulation; every answer for a
//! viewer comes from here and costs the simulation nothing.

mod apply;
mod departed;
mod events;
mod published;
mod zones;

use std::collections::{BTreeMap, HashMap};

use axum::extract::ws::Utf8Bytes;
use unsup_protocol::{CityView, Clock, Pos, ResidentId, WorldView, ZoneId, ZoneView};

pub(crate) use self::events::FeedQuery;
pub(crate) use self::published::PUBLISHED_SUFFIX;
pub(crate) use self::zones::{Resolved, ZoneMap};

use self::departed::DepartedList;
use self::events::EventLog;
use self::published::PublishedStore;
use self::zones::ZoneCache;
use crate::config::Config;
use crate::digest::to_text;
use crate::wire::{
    FeedPage, FeedStats, PROTOCOL_VERSION, Profile, Published, PublishedRef, ServerMessage,
};

/// The read-only cache of the world.
#[derive(Debug)]
pub(crate) struct Cache {
    /// The last frame that the cache holds.
    frame: u64,
    clock: Clock,
    cities: Vec<CityView>,
    zones: BTreeMap<ZoneId, ZoneCache>,
    /// The zone of every resident.
    index: HashMap<ResidentId, ZoneId>,
    events: EventLog,
    published: PublishedStore,
    departed: DepartedList,
}

impl Cache {
    /// Makes an empty cache: no cities, no zones, no events.
    pub fn new(config: &Config) -> Cache {
        Cache {
            frame: 0,
            clock: Clock::default(),
            cities: Vec::new(),
            zones: BTreeMap::new(),
            index: HashMap::new(),
            events: EventLog::new(
                config.feed.recent_events,
                config.feed.notable_events,
                config.feed.notable_drama,
            ),
            published: PublishedStore::new(
                config.published.max_series,
                config.published.max_key_bytes,
            ),
            departed: DepartedList::new(config.profile.departed),
        }
    }

    /// Returns the last frame that the cache holds.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Replaces the layout of the world: the cities and the zones, without
    /// residents. Events, published state and departed residents stay.
    pub fn set_layout(
        &mut self,
        frame: u64,
        clock: Clock,
        cities: Vec<CityView>,
        zones: Vec<ZoneView>,
    ) {
        self.frame = frame;
        self.clock = clock;
        self.cities = cities;
        self.zones = zones.into_iter().map(|zone| (zone.id, ZoneCache::new(zone))).collect();
        self.index.clear();
    }

    /// Replaces the residents and the map of one zone with what the
    /// simulation says.
    pub fn load_zone(&mut self, view: WorldView) {
        let WorldView::Zone { zone, width, height, tiles, sites, residents } = view else {
            return;
        };
        let read_at = self.frame;
        let Some(cache) = self.zones.get_mut(&zone.id) else {
            return;
        };
        for resident in &residents {
            self.index.insert(resident.id, zone.id);
        }
        cache.load(residents, ZoneMap { width, height, tiles, sites, read_at });
    }

    /// Replaces the tiles and the sites of one zone. The residents stay.
    pub fn set_map(&mut self, view: WorldView) {
        let WorldView::Zone { zone, width, height, tiles, sites, .. } = view else {
            return;
        };
        let read_at = self.frame;
        if let Some(cache) = self.zones.get_mut(&zone.id) {
            cache.map = ZoneMap { width, height, tiles, sites, read_at };
        }
    }

    /// Returns true when the zone exists and this process hosts it.
    pub fn serves(&self, zone: ZoneId) -> bool {
        self.zones.get(&zone).is_some_and(|cache| cache.hosted)
    }

    /// Says whether a request must read the map of a zone again: the map
    /// is older than `refresh_frames` and no other request reads it now.
    /// The caller must call [`Cache::refresh_done`] afterwards.
    pub fn claim_refresh(&self, zone: ZoneId, refresh_frames: u64) -> bool {
        use std::sync::atomic::Ordering;
        self.zones.get(&zone).is_some_and(|cache| {
            cache.hosted
                && self.frame.saturating_sub(cache.map.read_at) >= refresh_frames
                && !cache.refreshing.swap(true, Ordering::AcqRel)
        })
    }

    /// Ends a refresh that [`Cache::claim_refresh`] started.
    pub fn refresh_done(&self, zone: ZoneId) {
        use std::sync::atomic::Ordering;
        if let Some(cache) = self.zones.get(&zone) {
            cache.refreshing.store(false, Ordering::Release);
        }
    }

    /// Returns the layout of the world with the current numbers of
    /// residents.
    pub fn layout(&self) -> WorldView {
        WorldView::Layout {
            frame: self.frame,
            clock: self.clock,
            cities: self.cities.clone(),
            zones: self.zones.values().map(ZoneCache::view).collect(),
        }
    }

    /// Returns everything in one zone, or `None` when the zone does not
    /// exist or another process hosts it.
    pub fn zone(&self, zone: ZoneId, with_residents: bool) -> Option<WorldView> {
        let cache = self.zones.get(&zone).filter(|cache| cache.hosted)?;
        Some(WorldView::Zone {
            zone: cache.view(),
            width: cache.map.width,
            height: cache.map.height,
            tiles: cache.map.tiles.clone(),
            sites: cache.map.sites.clone(),
            residents: if with_residents { cache.sorted_residents() } else { Vec::new() },
        })
    }

    /// Returns the `world` message for one viewer.
    pub fn world_message(&self, dropped: u64, viewers: u32) -> Utf8Bytes {
        to_text(&ServerMessage::World {
            protocol: PROTOCOL_VERSION,
            frame: self.frame,
            clock: self.clock,
            populations: self.zones.values().map(|z| (z.info().id, z.population())).collect(),
            dropped,
            viewers,
        })
    }

    /// Returns the `snapshot` message of a zone. Viewers that ask for the
    /// same zone before it changes share one serialization.
    pub fn snapshot_message(&self, zone: ZoneId) -> Option<Utf8Bytes> {
        let cache = self.zones.get(&zone).filter(|cache| cache.hosted)?;
        if let Some(text) = cache.snapshot_memo() {
            return Some(text);
        }
        let text = to_text(&ServerMessage::Snapshot {
            zone,
            frame: self.frame,
            residents: cache.sorted_residents(),
        });
        cache.set_snapshot_memo(text.clone());
        Some(text)
    }

    /// Answers a question to the event log.
    pub fn feed(&self, query: &FeedQuery) -> FeedPage {
        self.events.page(query)
    }

    /// Returns the profile of a resident: one that lives, or one of the
    /// last residents that left the world.
    pub fn profile(&self, id: ResidentId, events: usize) -> Option<Profile> {
        let (view, zone, alive, frame, cause) = match self.index.get(&id) {
            Some(zone) => {
                let view = self.zones.get(zone)?.residents.get(&id)?;
                (view, *zone, true, self.frame, None)
            }
            None => {
                let departed = self.departed.get(id)?;
                (&departed.view, departed.zone, false, departed.frame, departed.cause.clone())
            }
        };
        let info = self.zones.get(&zone)?.info();
        let city_name = self
            .cities
            .iter()
            .find(|city| city.id == info.city)
            .map(|city| city.name.clone())
            .unwrap_or_default();
        Some(Profile {
            id,
            name: view.name.clone(),
            model: view.model.clone(),
            alive,
            zone,
            zone_name: info.name.clone(),
            city: info.city,
            city_name,
            pos: view.pos,
            activity: view.activity.clone(),
            frame,
            cause,
            events: self.events.about(id, events),
        })
    }

    /// Returns one series of published state.
    pub fn published(&self, kind: &str, key: Option<&str>) -> Option<Published> {
        self.published.get(kind, key).cloned()
    }

    /// Returns every series of one kind of published state.
    pub fn published_kind(&self, kind: &str) -> Vec<Published> {
        self.published.of_kind(kind)
    }

    /// Returns one line for every series of published state.
    pub fn published_index(&self) -> Vec<PublishedRef> {
        self.published.index()
    }

    /// Returns the numbers of the cache for `/v1/spectate/stats`.
    pub fn stats(&self, viewers: u32, dropped_frames: u64, resyncs: u64) -> FeedStats {
        FeedStats {
            frame: self.frame,
            residents: self.index.len() as u64,
            viewers,
            dropped_frames,
            resyncs,
        }
    }

    /// Returns the name of a resident that lives.
    fn name_of(&self, id: ResidentId) -> Option<&str> {
        let zone = self.index.get(&id)?;
        self.zones.get(zone)?.residents.get(&id).map(|view| view.name.as_str())
    }

    /// Returns the zone that holds a position.
    fn zone_at(&self, pos: Pos) -> Option<ZoneId> {
        let city = self.cities.iter().find(|city| city.bounds.contains(pos))?;
        // A city is a square of zones, row by row. Try the zone that the
        // arithmetic gives, then every zone of the city.
        let side = city.zones.len().isqrt();
        let guess = (side > 0)
            .then(|| {
                let size = (city.bounds.width() as usize / side).max(1);
                let col = pos.x.abs_diff(city.bounds.x0) as usize / size;
                let row = pos.y.abs_diff(city.bounds.y0) as usize / size;
                city.zones.get(row * side + col).copied()
            })
            .flatten();
        let holds = |zone: &ZoneId| {
            self.zones.get(zone).is_some_and(|cache| cache.info().bounds.contains(pos))
        };
        guess.filter(holds).or_else(|| city.zones.iter().copied().find(|zone| holds(zone)))
    }
}
