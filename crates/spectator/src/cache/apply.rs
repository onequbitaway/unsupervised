//! How one frame of the simulation changes the cache.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Arc;

use unsup_protocol::{Clock, Event, FrameOutput, ZoneId};

use super::Cache;
use super::departed::Departed;
use super::published::PUBLISHED_SUFFIX;
use super::zones::{Gone, Resolved};
use crate::config::Config;
use crate::digest::{Digest, ZoneBeat};
use crate::wire::{EventId, FeedEvent};

/// The kind of the kernel event for a resident that left the world.
const DESPAWNED: &str = "kernel.despawned";

/// Keeps the `limit` events with the highest drama scores. Of two events
/// with the same score the earlier one stays. The order does not change.
fn keep_most_dramatic(events: &mut Vec<Arc<FeedEvent>>, limit: usize) {
    if events.len() <= limit {
        return;
    }
    let mut ranked: Vec<usize> = (0..events.len()).collect();
    ranked.sort_by_key(|&at| (Reverse(events[at].drama), at));
    ranked.truncate(limit);
    ranked.sort_unstable();
    *events = ranked.into_iter().map(|at| Arc::clone(&events[at])).collect();
}

/// Returns true when a zone has something for viewers in this frame.
fn has_content(beat: &ZoneBeat, frame: &FrameOutput) -> bool {
    if !beat.events.is_empty() {
        return true;
    }
    if let Some(resolved) = &beat.resolved {
        return *resolved != Resolved::default();
    }
    beat.delta.and_then(|at| frame.zones.get(at)).is_some_and(|delta| !delta.is_empty())
}

impl Cache {
    /// Applies one frame and returns what viewers get from it.
    ///
    /// `viewers` is the number of connected viewers; the `frame` message
    /// carries it.
    pub fn apply_frame(
        &mut self,
        frame: Arc<FrameOutput>,
        config: &Config,
        viewers: u32,
    ) -> Digest {
        self.frame = frame.frame;
        self.clock = frame.clock;

        // Residents.
        let mut beats: BTreeMap<ZoneId, ZoneBeat> = BTreeMap::new();
        let mut populations = Vec::new();
        let mut gone: Vec<Gone> = Vec::new();
        for (at, delta) in frame.zones.iter().enumerate() {
            let Some(zone) = self.zones.get_mut(&delta.zone) else {
                continue;
            };
            let before = zone.population();
            let resolved = zone.apply(delta, &mut self.index, &mut gone);
            if zone.population() != before {
                populations.push((delta.zone, zone.population()));
            }
            let mut beat = ZoneBeat::new(delta.zone, delta.tick);
            beat.delta = Some(at);
            beat.resolved = resolved;
            beats.insert(delta.zone, beat);
        }
        // A resident that left one zone and is in another zone now only
        // moved. The others left the world.
        gone.retain(|gone| !self.index.contains_key(&gone.view.id));

        // Events.
        let mut headlines = Vec::new();
        for (at, event) in frame.events.iter().enumerate() {
            if !event.public {
                continue;
            }
            if event.kind.ends_with(PUBLISHED_SUFFIX) {
                if let Err(reason) = self.published.put(event, frame.frame, frame.clock) {
                    tracing::warn!(kind = %event.kind, ?reason, "a published event was not kept");
                }
                continue;
            }
            let id = EventId { frame: frame.frame, index: u32::try_from(at).unwrap_or(u32::MAX) };
            let event = Arc::new(self.feed_event(event, id, frame.clock, &gone));
            self.events.push(Arc::clone(&event));
            if event.drama >= config.feed.headline_drama {
                headlines.push(Arc::clone(&event));
            }
            if let Some(zone) = event.zone
                && let Some(cache) = self.zones.get(&zone)
            {
                beats
                    .entry(zone)
                    .or_insert_with(|| ZoneBeat::new(zone, cache.tick))
                    .events
                    .push(event);
            }
        }

        // Residents that left the world keep a profile for some time.
        for Gone { view, zone } in gone {
            let cause = frame
                .events
                .iter()
                .find(|event| event.kind == DESPAWNED && event.actor == Some(view.id))
                .and_then(|event| event.data.get("cause"))
                .and_then(|cause| cause.as_str())
                .map(str::to_string);
            self.departed.push(Departed { view, zone, frame: frame.frame, cause });
        }

        keep_most_dramatic(&mut headlines, config.socket.headlines_per_frame);
        // A zone whose residents entered and left again has nothing to say.
        let mut zones: Vec<ZoneBeat> =
            beats.into_values().filter(|beat| has_content(beat, &frame)).collect();
        for beat in &mut zones {
            keep_most_dramatic(&mut beat.events, config.socket.zone_events_per_frame);
        }
        Digest::new(frame, zones, populations, &headlines, viewers)
    }

    /// Makes the viewer form of an event.
    fn feed_event(&self, event: &Event, id: EventId, clock: Clock, gone: &[Gone]) -> FeedEvent {
        let zone = event
            .realm
            .zone()
            .or_else(|| event.pos.and_then(|pos| self.zone_at(pos)))
            .filter(|zone| self.zones.contains_key(zone));
        let actor_name = event.actor.and_then(|actor| {
            self.name_of(actor).map(str::to_string).or_else(|| {
                gone.iter().find(|gone| gone.view.id == actor).map(|gone| gone.view.name.clone())
            })
        });
        FeedEvent {
            id,
            frame: id.frame,
            clock,
            tick: event.tick,
            realm: event.realm,
            zone,
            kind: event.kind.clone(),
            actor: event.actor,
            actor_name,
            pos: event.pos,
            drama: event.drama,
            data: event.data.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use unsup_protocol::{
        CityId, CityView, Pos, RealmId, Rect, ResidentId, ResidentView, Tick, WorldView, ZoneDelta,
        ZoneView,
    };

    use super::*;
    use crate::cache::FeedQuery;
    use crate::wire::ServerMessage;

    /// A cache of one city with two zones side by side, 8 tiles each.
    fn cache(config: &Config) -> Cache {
        let mut cache = Cache::new(config);
        let zone = |id: u32, x0: i32| ZoneView {
            id: ZoneId(id),
            city: CityId(0),
            name: format!("Town {id}-0"),
            bounds: Rect::sized(x0, 0, 8, 8),
            residents: 0,
        };
        cache.set_layout(
            0,
            Clock::default(),
            vec![CityView {
                id: CityId(0),
                name: "Town".into(),
                bounds: Rect::sized(0, 0, 16, 8),
                zones: vec![ZoneId(0), ZoneId(1)],
            }],
            vec![zone(0, 0), zone(1, 8)],
        );
        for id in [0, 1] {
            cache.load_zone(WorldView::Zone {
                zone: zone(id, id as i32 * 8),
                width: 8,
                height: 8,
                tiles: vec![0; 64],
                sites: Vec::new(),
                residents: Vec::new(),
            });
        }
        cache
    }

    fn resident(id: u64, x: i32) -> ResidentView {
        ResidentView {
            id: ResidentId(id),
            name: format!("r{id}"),
            pos: Pos::new(x, 1),
            activity: None,
            model: None,
        }
    }

    fn event(realm: RealmId, kind: &str, drama: u8, actor: Option<u64>, pos: Option<Pos>) -> Event {
        Event {
            seq: 1,
            frame: 1,
            tick: Tick(1),
            realm,
            kind: kind.to_string(),
            actor: actor.map(ResidentId),
            pos,
            public: true,
            drama,
            data: Value::Null,
        }
    }

    fn frame(number: u64, zones: Vec<ZoneDelta>, events: Vec<Event>) -> Arc<FrameOutput> {
        Arc::new(FrameOutput { frame: number, zones, events, ..FrameOutput::default() })
    }

    fn enter(zone: u32, residents: Vec<ResidentView>) -> ZoneDelta {
        ZoneDelta { zone: ZoneId(zone), tick: Tick(1), entered: residents, ..ZoneDelta::default() }
    }

    fn message(text: &str) -> ServerMessage {
        serde_json::from_str(text).expect("a server message")
    }

    #[test]
    fn a_global_event_with_a_place_goes_to_the_zone_of_the_place() {
        let config = Config::default();
        let mut cache = cache(&config);
        let digest = cache.apply_frame(
            frame(
                1,
                vec![enter(1, vec![resident(5, 9)])],
                vec![
                    event(RealmId::Global, "bank.robbed", 80, Some(5), Some(Pos::new(12, 3))),
                    event(RealmId::Global, "law.passed", 20, None, None),
                    event(RealmId::Global, "far.away", 20, None, Some(Pos::new(500, 500))),
                ],
            ),
            &config,
            3,
        );
        // Zone 0 has nothing. Zone 1 has the new resident and the event.
        assert_eq!(digest.zone_text(ZoneId(0)), None);
        let ServerMessage::Delta { entered, events, .. } =
            message(&digest.zone_text(ZoneId(1)).expect("zone 1 changed"))
        else {
            panic!("not a delta");
        };
        assert_eq!(entered, vec![resident(5, 9)]);
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].id.to_string().as_str(), events[0].zone, events[0].actor_name.as_deref()),
            ("1-0", Some(ZoneId(1)), Some("r5"))
        );
        // Only the headline goes to every viewer.
        let ServerMessage::Frame { headlines, populations, viewers, .. } =
            message(&digest.frame_text())
        else {
            panic!("not a frame");
        };
        assert_eq!(headlines.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["bank.robbed"]);
        assert_eq!(populations, vec![(ZoneId(1), 1)]);
        assert_eq!(viewers, 3);
        // All three are in the feed. The two without a zone say so.
        let page = cache.feed(&FeedQuery { limit: 10, ..FeedQuery::default() });
        assert_eq!(
            page.events.iter().map(|e| (e.kind.as_str(), e.zone)).collect::<Vec<_>>(),
            [("bank.robbed", Some(ZoneId(1))), ("law.passed", None), ("far.away", None)]
        );
    }

    #[test]
    fn a_frame_keeps_only_the_most_dramatic_events() {
        let mut config = Config::default();
        config.socket.headlines_per_frame = 2;
        config.socket.zone_events_per_frame = 2;
        let mut cache = cache(&config);
        let zone = RealmId::Zone(ZoneId(0));
        let digest = cache.apply_frame(
            frame(
                1,
                Vec::new(),
                vec![
                    event(zone, "a", 75, None, None),
                    event(zone, "b", 90, None, None),
                    event(zone, "c", 75, None, None),
                    event(zone, "d", 99, None, None),
                ],
            ),
            &config,
            0,
        );
        let ServerMessage::Frame { headlines, .. } = message(&digest.frame_text()) else {
            panic!("not a frame");
        };
        assert_eq!(headlines.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        let ServerMessage::Delta { events, tick, .. } =
            message(&digest.zone_text(ZoneId(0)).expect("zone 0 has events"))
        else {
            panic!("not a delta");
        };
        assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        // The zone had no delta: the tick is the last one that the cache knows.
        assert_eq!(tick, Tick(0));
        // The feed keeps all four.
        assert_eq!(cache.feed(&FeedQuery { limit: 10, ..FeedQuery::default() }).events.len(), 4);
    }

    #[test]
    fn equal_drama_keeps_the_earlier_event() {
        let make = |kind: &str, drama: u8| {
            Arc::new(FeedEvent {
                id: EventId::START,
                frame: 0,
                clock: Clock::default(),
                tick: Tick(0),
                realm: RealmId::Global,
                zone: None,
                kind: kind.into(),
                actor: None,
                actor_name: None,
                pos: None,
                drama,
                data: Value::Null,
            })
        };
        let mut events = vec![make("a", 5), make("b", 9), make("c", 5), make("d", 5)];
        keep_most_dramatic(&mut events, 3);
        assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    }

    #[test]
    fn a_resident_that_leaves_the_world_keeps_a_profile() {
        let config = Config::default();
        let mut cache = cache(&config);
        cache.apply_frame(
            frame(1, vec![enter(0, vec![resident(5, 2), resident(6, 3)])], vec![]),
            &config,
            0,
        );
        let mut despawned =
            event(RealmId::Zone(ZoneId(0)), "kernel.despawned", 30, Some(5), Some(Pos::new(2, 1)));
        despawned.data = json!({ "resident": 5, "name": "r5", "cause": "old_age" });
        // Resident 5 leaves the world. Resident 6 walks to zone 1; the
        // delta of zone 0 (`left`) comes before the delta of zone 1.
        let digest = cache.apply_frame(
            frame(
                2,
                vec![
                    ZoneDelta {
                        zone: ZoneId(0),
                        tick: Tick(2),
                        left: vec![ResidentId(5), ResidentId(6)],
                        ..ZoneDelta::default()
                    },
                    enter(1, vec![resident(6, 8)]),
                ],
                vec![despawned],
            ),
            &config,
            0,
        );
        let ServerMessage::Delta { events, .. } =
            message(&digest.zone_text(ZoneId(0)).expect("zone 0 changed"))
        else {
            panic!("not a delta");
        };
        assert_eq!(events[0].actor_name.as_deref(), Some("r5"));

        let gone = cache.profile(ResidentId(5), 10).expect("a profile of the departed");
        assert_eq!(
            (gone.alive, gone.zone, gone.frame, gone.cause.as_deref(), gone.name.as_str()),
            (false, ZoneId(0), 2, Some("old_age"), "r5")
        );
        assert_eq!(
            gone.events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            ["kernel.despawned"]
        );
        let moved = cache.profile(ResidentId(6), 10).expect("a profile of the living");
        assert_eq!((moved.alive, moved.zone, moved.pos), (true, ZoneId(1), Pos::new(8, 1)));
        assert_eq!(cache.profile(ResidentId(7), 10), None);
        assert_eq!(cache.stats(0, 0, 0).residents, 1);
    }

    #[test]
    fn a_published_event_is_state_and_not_news() {
        let config = Config::default();
        let mut cache = cache(&config);
        let mut stats = event(RealmId::Global, "finance.stats_published", 0, None, None);
        stats.data = json!({ "money_supply": 5 });
        let digest = cache.apply_frame(frame(1, vec![], vec![stats]), &config, 0);
        assert!(cache.feed(&FeedQuery { limit: 10, ..FeedQuery::default() }).events.is_empty());
        let ServerMessage::Frame { headlines, .. } = message(&digest.frame_text()) else {
            panic!("not a frame");
        };
        assert!(headlines.is_empty());
        let published = cache.published("finance.stats_published", None).expect("the stats");
        assert_eq!((published.frame, &published.data), (1, &json!({ "money_supply": 5 })));
    }
}
