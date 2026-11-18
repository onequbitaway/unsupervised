//! The recorder, fed with the frames that the cache makes.

use std::sync::Arc;

use serde_json::Value;
use unsup_protocol::{
    CityId, CityView, Clock, Event, FrameOutput, Pos, RealmId, Rect, ResidentId, ResidentView,
    Tick, WorldView, ZoneDelta, ZoneId, ZoneView,
};

use super::*;
use crate::cache::Cache;
use crate::config::Config;

/// The limits of these tests: 10 frames are kept, a clip has 2 frames
/// before its event and 3 after it, and 2 clips can be pinned.
fn config() -> Config {
    let mut config = Config::default();
    config.clips.window_frames = 10;
    config.clips.before_frames = 2;
    config.clips.after_frames = 3;
    config.clips.max_pinned = 2;
    config.clips.auto_pin_drama = 85;
    config
}

/// A cache of one city with two zones side by side, and a recorder.
struct Rig {
    config: Config,
    cache: Cache,
    recorder: Recorder,
}

fn view(id: u64, x: i32) -> ResidentView {
    ResidentView {
        id: ResidentId(id),
        name: format!("r{id}"),
        pos: Pos::new(x, 1),
        activity: None,
        model: None,
    }
}

fn event(zone: Option<u32>, drama: u8) -> Event {
    Event {
        seq: 1,
        frame: 0,
        tick: Tick(0),
        realm: zone.map_or(RealmId::Global, |zone| RealmId::Zone(ZoneId(zone))),
        kind: "crime.theft_committed".to_string(),
        actor: Some(ResidentId(5)),
        pos: None,
        public: true,
        drama,
        data: Value::Null,
    }
}

fn moved(zone: u32, id: u64, x: i32) -> ZoneDelta {
    ZoneDelta {
        zone: ZoneId(zone),
        tick: Tick(1),
        moved: vec![(ResidentId(id), Pos::new(x, 1))],
        ..ZoneDelta::default()
    }
}

impl Rig {
    fn new(config: Config) -> Rig {
        let mut cache = Cache::new(&config);
        let zone = |id: u32| ZoneView {
            id: ZoneId(id),
            city: CityId(0),
            name: format!("Town {id}-0"),
            bounds: Rect::sized(id as i32 * 8, 0, 8, 8),
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
            vec![zone(0), zone(1)],
        );
        for id in [0, 1] {
            cache.load_zone(WorldView::Zone {
                zone: zone(id),
                width: 8,
                height: 8,
                tiles: vec![0; 64],
                sites: Vec::new(),
                residents: Vec::new(),
            });
        }
        let mut recorder = Recorder::new(&config.clips);
        recorder.reset(cache.residents_by_zone());
        Rig { config, cache, recorder }
    }

    /// Applies one frame. Returns its public events as the feed has them.
    fn frame(
        &mut self,
        number: u64,
        zones: Vec<ZoneDelta>,
        events: Vec<Event>,
    ) -> Vec<Arc<FeedEvent>> {
        let output = FrameOutput { frame: number, zones, events, ..FrameOutput::default() };
        let digest = self.cache.apply_frame(Arc::new(output), &self.config, 0);
        let events = digest.events.clone();
        for event in &events {
            self.recorder.note(event, "a line");
        }
        self.recorder.push(Arc::new(digest));
        events
    }

    /// Frames 1 to 4: resident 5 comes into zone 0 and walks east, and
    /// steals at frame 3. Resident 6 is in zone 1. Returns the theft.
    fn theft(&mut self) -> Arc<FeedEvent> {
        let enter = |zone: u32, id: u64, x: i32| ZoneDelta {
            zone: ZoneId(zone),
            tick: Tick(1),
            entered: vec![view(id, x)],
            ..ZoneDelta::default()
        };
        self.frame(1, vec![enter(0, 5, 1), enter(1, 6, 9)], vec![]);
        self.frame(2, vec![moved(0, 5, 2)], vec![]);
        let events = self.frame(3, vec![moved(0, 5, 3)], vec![event(Some(0), 55)]);
        self.frame(4, vec![moved(0, 5, 4)], vec![]);
        Arc::clone(&events[0])
    }

    /// Frames up to `last`. Zone 0 has nothing in the frame 5, and a move
    /// of resident 5 in every later frame.
    fn run_to(&mut self, from: u64, last: u64) {
        for number in from..=last {
            let zones = if number == 5 {
                vec![moved(1, 6, 10)]
            } else {
                vec![moved(0, 5, number as i32 % 8)]
            };
            self.frame(number, zones, vec![]);
        }
    }
}

fn frames(clip: &Clip) -> Vec<u64> {
    clip.frames.iter().map(|frame| frame.frame).collect()
}

#[test]
fn a_clip_has_the_event_and_the_frames_around_it() {
    let mut rig = Rig::new(config());
    let theft = rig.theft();
    assert_eq!(theft.id.to_string(), "3-0");
    // The frames after the event are not all there yet.
    let clip = rig.recorder.clip(&theft, "Bo stole").expect("a clip");
    assert_eq!((clip.from, clip.to, clip.complete, clip.pinned), (1, 4, false, false));
    assert_eq!(frames(&clip), [1, 2, 3, 4]);

    rig.run_to(5, 7);
    let clip = rig.recorder.clip(&theft, "Bo stole").expect("a clip");
    assert_eq!((clip.id, clip.zone, clip.line.as_str()), (theft.id, ZoneId(0), "Bo stole"));
    assert_eq!(clip.link, "/?clip=3-0");
    // 2 frames before the event, 3 after it. Frame 5 had nothing in zone 0.
    assert_eq!((clip.from, clip.to, clip.complete), (1, 6, true));
    assert_eq!(frames(&clip), [1, 2, 3, 4, 6]);
    // Nobody was in the zone before the first frame of the clip.
    assert_eq!(clip.residents, Vec::new());
    assert_eq!(clip.frames[0].entered, vec![view(5, 1)]);
    assert_eq!(clip.frames[1].moved, vec![(ResidentId(5), 2, 1)]);
    // The frame of the event carries the event.
    assert_eq!(clip.frames[2].events, vec![(*theft).clone()]);
    assert_eq!(clip.event, *theft);
    // The clip has only its zone: resident 6 is not in it.
    assert!(clip.frames.iter().all(|frame| frame.entered.iter().all(|r| r.id == ResidentId(5))));
}

#[test]
fn a_clip_starts_with_the_residents_of_that_time() {
    let mut rig = Rig::new(config());
    let theft = rig.theft();
    // The recorder keeps 10 frames: at frame 12 these are 3 to 12.
    rig.run_to(5, 12);
    let clip = rig.recorder.clip(&theft, "x").expect("a clip");
    assert_eq!((clip.from, clip.to, clip.complete), (3, 6, true));
    // Resident 5 came at frame 1 and was at (2, 1) after frame 2.
    assert_eq!(clip.residents, vec![view(5, 2)]);
    assert_eq!(frames(&clip), [3, 4, 6]);
    // One frame later the frame of the event is gone.
    rig.run_to(13, 13);
    assert_eq!(rig.recorder.clip(&theft, "x"), Err(NoClip::Gone));
}

#[test]
fn a_pinned_clip_stays_when_its_frames_are_gone() {
    let mut rig = Rig::new(config());
    let theft = rig.theft();
    // The clip is not complete at frame 4: the pin waits.
    assert_eq!(rig.recorder.pin(&theft, "Bo stole"), Ok(Pin::Waiting));
    assert!(rig.recorder.pinned(theft.id).is_none());
    assert!(rig.recorder.list().is_empty());
    rig.run_to(5, 5);
    assert!(rig.recorder.pinned(theft.id).is_none());
    // Frame 6 is the third frame after the event.
    rig.run_to(6, 6);
    let pinned = rig.recorder.pinned(theft.id).expect("the clip is pinned");
    assert_eq!(
        (pinned.pinned, pinned.complete, frames(&pinned)),
        (true, true, vec![1, 2, 3, 4, 6])
    );
    assert_eq!(rig.recorder.pin(&theft, "Bo stole"), Ok(Pin::Done));
    let (pins, unpins) = rig.recorder.take_unwritten();
    assert_eq!((pins.len(), pins[0].id, unpins.len()), (1, theft.id, 0));
    assert_eq!(rig.recorder.take_unwritten().0.len(), 0);

    rig.run_to(7, 30);
    // The recorder has no frame of the event any more. The pin has.
    let clip = rig.recorder.clip(&theft, "another line").expect("the pinned clip");
    assert_eq!(
        (clip.pinned, clip.line.as_str(), frames(&clip)),
        (true, "Bo stole", vec![1, 2, 3, 4, 6])
    );
    let list = rig.recorder.list();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].id, list[0].frame, list[0].drama, list[0].link.as_str()),
        (theft.id, 3, 55, "/?clip=3-0")
    );
}

#[test]
fn a_very_dramatic_event_is_pinned_without_a_request() {
    let mut rig = Rig::new(config());
    rig.theft();
    // Drama 84 is below `auto_pin_drama` (85).
    let quiet = rig.frame(5, vec![moved(0, 5, 5)], vec![event(Some(0), 84)]);
    let loud = rig.frame(6, vec![moved(0, 5, 6)], vec![event(Some(0), 85)]);
    // An event without a place has no clip.
    let nowhere = rig.frame(7, vec![], vec![event(None, 99)]);
    rig.run_to(8, 9);
    assert!(rig.recorder.pinned(loud[0].id).is_some());
    assert!(rig.recorder.pinned(quiet[0].id).is_none());
    assert!(rig.recorder.pinned(nowhere[0].id).is_none());
    assert_eq!(rig.recorder.clip(&nowhere[0], "x"), Err(NoClip::NoPlace));
    assert_eq!(rig.recorder.pin(&nowhere[0], "x"), Err(NoClip::NoPlace));
}

#[test]
fn the_clip_of_the_oldest_event_leaves_when_there_are_too_many_pins() {
    let mut rig = Rig::new(config());
    let first = rig.theft();
    let second = rig.frame(5, vec![moved(0, 5, 5)], vec![event(Some(0), 50)]);
    let third = rig.frame(6, vec![moved(0, 5, 6)], vec![event(Some(0), 50)]);
    rig.run_to(7, 9);
    for event in [&first, &second[0], &third[0]] {
        assert_eq!(rig.recorder.pin(event, "x"), Ok(Pin::Done));
    }
    let ids: Vec<String> = rig.recorder.list().iter().map(|clip| clip.id.to_string()).collect();
    // The newest event first. `3-0` left.
    assert_eq!(ids, ["6-0", "5-0"]);
    let (pins, unpins) = rig.recorder.take_unwritten();
    // The clip that left was never written: it is in neither list as a
    // write, and the storage is told to forget it.
    assert_eq!(pins.iter().map(|clip| clip.id.to_string()).collect::<Vec<_>>(), ["5-0", "6-0"]);
    assert_eq!(unpins, vec![first.id]);
}

#[test]
fn pinned_clips_come_back_from_the_storage() {
    let mut rig = Rig::new(config());
    let theft = rig.theft();
    let later = rig.frame(5, vec![moved(0, 5, 5)], vec![event(Some(0), 50)]);
    rig.run_to(6, 9);
    rig.recorder.pin(&theft, "Bo stole").expect("pin");
    rig.recorder.pin(&later[0], "later").expect("pin");
    let (pins, _) = rig.recorder.take_unwritten();
    let stored: Vec<Clip> = pins.iter().map(|clip| (**clip).clone()).collect();

    // A new service whose world is at frame 4: the clip of frame 5 is from
    // a time that the world does not have.
    let mut again = Recorder::new(&config().clips);
    again.restore(stored, 4);
    assert_eq!(again.list().iter().map(|clip| clip.id).collect::<Vec<_>>(), [theft.id]);
    // In this test zone 0 also changed in the frame 5.
    assert_eq!(again.pinned(theft.id).map(|clip| frames(&clip)), Some(vec![1, 2, 3, 4, 5, 6]));
    let (pins, unpins) = again.take_unwritten();
    assert_eq!((pins.len(), unpins), (0, vec![later[0].id]));
}

#[test]
fn a_recorder_that_is_off_keeps_nothing() {
    let mut config = config();
    config.clips.window_frames = 0;
    let mut rig = Rig::new(config);
    let theft = rig.theft();
    assert_eq!(rig.recorder.clip(&theft, "x"), Err(NoClip::Off));
    assert_eq!(rig.recorder.pin(&theft, "x"), Err(NoClip::Off));
}

#[test]
fn a_new_read_of_the_world_starts_the_recorder_again() {
    let mut rig = Rig::new(config());
    let theft = rig.theft();
    rig.recorder.pin(&theft, "x").expect("a pin that waits");
    // The cache lost frames and read the world again at frame 4.
    rig.recorder.reset(rig.cache.residents_by_zone());
    assert_eq!(rig.recorder.clip(&theft, "x"), Err(NoClip::Gone));
    rig.run_to(5, 9);
    assert!(rig.recorder.pinned(theft.id).is_none());
    // New events have clips again, with the residents of the new read.
    let events = rig.frame(10, vec![moved(0, 5, 7)], vec![event(Some(0), 50)]);
    let clip = rig.recorder.clip(&events[0], "x").expect("a clip");
    // The new read had resident 5 at (4, 1). The frames 6 and 7 moved it
    // to (7, 1) before the first frame of the clip.
    assert_eq!((clip.from, clip.residents.as_slice()), (8, [view(5, 7)].as_slice()));
}
