//! Scripted event sequences for the director. The numbers in the comments
//! are for the weights of `data/director.toml`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use unsup_protocol::{CityId, Clock, Pos, RealmId, ResidentId, SiteId, Tick, ZoneId};

use super::*;
use crate::config::Config;
use crate::facts::{Reader, TestAtlas};
use crate::wire::EventId;

/// An event of a script: the frame, the drama score, the actor, the zone
/// and the data.
struct Line(u64, u8, Option<u64>, Option<u32>, Value);

fn event(line: &Line, index: u32) -> Arc<FeedEvent> {
    let Line(frame, drama, actor, zone, data) = line;
    Arc::new(FeedEvent {
        id: EventId { frame: *frame, index },
        frame: *frame,
        clock: clock(*frame),
        tick: Tick(*frame),
        realm: RealmId::Global,
        zone: zone.map(ZoneId),
        // Each event has its own kind, so that the variety of kinds plays
        // no part. A script names a kind in the data to test that rule.
        kind: data
            .get("kind")
            .and_then(Value::as_str)
            .map_or_else(|| format!("test.event_{frame}_{index}"), str::to_string),
        actor: actor.map(ResidentId),
        actor_name: None,
        pos: zone.map(|_| Pos::new(5, 5)),
        drama: *drama,
        data: data.clone(),
    })
}

fn clock(frame: u64) -> Clock {
    Clock { year: 1, day: 1, hour: (frame / 60) as u8, minute: (frame % 60) as u8 }
}

/// Runs a script for `frames` frames. Returns every change of the shot
/// with its frame.
fn run(director: &mut Director, frames: u64, script: &[Line]) -> Vec<(u64, Shot)> {
    run_with(director, frames, script, &|_| None)
}

fn run_with(
    director: &mut Director,
    frames: u64,
    script: &[Line],
    story_words: &dyn Fn(u64) -> Option<StoryWords>,
) -> Vec<(u64, Shot)> {
    let atlas = TestAtlas::town();
    let reader = Reader::new(&Config::default().facts);
    let mut by_frame: BTreeMap<u64, Vec<Arc<FeedEvent>>> = BTreeMap::new();
    for line in script {
        let events = by_frame.entry(line.0).or_default();
        let index = events.len() as u32;
        events.push(event(line, index));
    }
    let mut changes = Vec::new();
    for frame in 1..=frames {
        let events = by_frame.remove(&frame).unwrap_or_default();
        let facts: Vec<_> = events.iter().map(|event| reader.facts(event)).collect();
        let lines: Vec<String> = events
            .iter()
            .map(|event| format!("event {} of drama {}", event.id, event.drama))
            .collect();
        let cues: Vec<Cue<'_>> = events
            .iter()
            .zip(&facts)
            .zip(&lines)
            .map(|((event, facts), line)| Cue {
                event,
                facts,
                line,
                story: event.data.get("story").and_then(Value::as_u64),
            })
            .collect();
        if let Some(shot) = director.frame(frame, clock(frame), &cues, &atlas, story_words) {
            changes.push((frame, shot));
        }
    }
    changes
}

fn director() -> Director {
    Director::new(&Config::default().director)
}

/// The frame, the number and the resident of each cut. A change of the
/// words of a shot is not a cut.
fn residents(changes: &[(u64, Shot)]) -> Vec<(u64, u64, Option<u64>)> {
    changes
        .iter()
        .filter(|(frame, shot)| shot.frame == *frame)
        .map(|(frame, shot)| {
            let resident = match &shot.focus {
                Focus::Resident { id, .. } => Some(id.0),
                _ => None,
            };
            (*frame, shot.seq, resident)
        })
        .collect()
}

#[test]
fn a_quiet_world_gets_wide_views_in_turn() {
    let changes = run(&mut director(), 100, &[]);
    let views: Vec<_> = changes
        .iter()
        .map(|(frame, shot)| (*frame, shot.seq, shot.zoom, shot.city, shot.caption.as_str()))
        .collect();
    assert_eq!(
        views,
        [
            (1, 1, Zoom::Country, None, "The whole country · 40 residents"),
            // `wide_hold_frames` is 30. The city with the most residents is first.
            (31, 2, Zoom::City, Some(CityId(0)), "Latent City · 30 residents"),
            (61, 3, Zoom::City, Some(CityId(1)), "Tokentown · 10 residents"),
            (91, 4, Zoom::Country, None, "The whole country · 40 residents"),
        ]
    );
    let (_, first) = &changes[0];
    assert_eq!(
        (&first.focus, first.breaking, first.event, first.pos),
        (&Focus::None, false, None, None)
    );
    assert_eq!(changes[1].1.pos, Some(Pos::new(32, 32)));
}

#[test]
fn a_shot_stays_for_its_least_time() {
    // Ada at frame 3, a stronger event of Bo at frame 8. Neither is
    // breaking news (71).
    let script = [Line(3, 50, Some(1), None, json!({})), Line(8, 65, Some(2), None, json!({}))];
    let changes = run(&mut director(), 40, &script);
    assert_eq!(
        residents(&changes),
        [
            (1, 1, None),
            // The wide view stays 4 frames (`wide_min_frames`), then Ada.
            (5, 2, Some(1)),
            // Bo has the higher score from frame 8 on. The shot of Ada
            // stays 12 frames (`min_hold_frames`): 5 + 12 = 17. There Ada
            // has 50 * 76 / 90 = 42 and Bo 65 * 81 / 90 = 58, which is
            // above 130 percent of 42.
            (17, 3, Some(2)),
        ]
    );
    let (_, ada) = &changes[1];
    assert_eq!(
        (ada.zoom, ada.caption.as_str(), ada.event, ada.drama, ada.breaking),
        (Zoom::Street, "event 3-0 of drama 50", Some(EventId { frame: 3, index: 0 }), 50, false)
    );
    // Ada has no place in the event. The atlas knows where she is.
    assert_eq!(
        (ada.zone, ada.pos, ada.city),
        (Some(ZoneId(0)), Some(Pos::new(1, 1)), Some(CityId(0)))
    );
    assert_eq!(ada.focus, Focus::Resident { id: ResidentId(1), name: Some("Ada".into()) });
    // 65 is at least `close_drama` (55).
    assert_eq!(changes[2].1.zoom, Zoom::Close);
}

#[test]
fn a_challenger_needs_a_clearly_higher_score() {
    // Bo has 55 against the 50 of Ada: not 130 percent. Ada keeps the
    // shot until her score is below `keep_score`.
    let script = [Line(3, 50, Some(1), None, json!({})), Line(8, 55, Some(2), None, json!({}))];
    let changes = run(&mut director(), 80, &script);
    assert_eq!(
        residents(&changes),
        [
            (1, 1, None),
            (5, 2, Some(1)),
            // 50 * (90 - age) / 90 is below 12 from the age 69: frame 72.
            // Bo then has 55 * 26 / 90 = 15, less than a shot needs (30).
            (72, 3, None),
        ]
    );
    // The second wide view is the first city.
    assert_eq!(
        (changes[2].1.zoom, changes[2].1.caption.as_str()),
        (Zoom::City, "Latent City · 30 residents")
    );
}

#[test]
fn breaking_news_cuts_in_at_once() {
    let script = [
        Line(3, 50, Some(1), None, json!({})),
        // Ada is on air from frame 5. Breaking news at frame 7.
        Line(7, 80, Some(3), None, json!({})),
        // Less dramatic news 2 frames later: the first news holds.
        Line(9, 75, Some(4), None, json!({})),
        // More dramatic: it cuts in.
        Line(11, 90, Some(2), None, json!({})),
        // As dramatic, not more: it does not cut in.
        Line(13, 90, Some(4), None, json!({})),
        // 8 frames (`breaking_hold_frames`) after frame 11: any breaking
        // news cuts in.
        Line(19, 72, Some(1), None, json!({})),
    ];
    let changes = run(&mut director(), 22, &script);
    assert_eq!(
        residents(&changes),
        [(1, 1, None), (5, 2, Some(1)), (7, 3, Some(3)), (11, 4, Some(2)), (19, 5, Some(1))]
    );
    let flags: Vec<bool> = changes.iter().map(|(_, shot)| shot.breaking).collect();
    assert_eq!(flags, [false, false, true, true, true]);
    assert_eq!(changes[2].1.zoom, Zoom::Close);
    assert_eq!(changes[2].1.caption, "event 7-0 of drama 80");
}

#[test]
fn an_event_below_the_breaking_score_does_not_cut_in() {
    let script = [Line(3, 50, Some(1), None, json!({})), Line(7, 70, Some(3), None, json!({}))];
    let changes = run(&mut director(), 16, &script);
    assert_eq!(residents(&changes), [(1, 1, None), (5, 2, Some(1))]);
}

#[test]
fn breaking_news_without_a_place_gets_a_wide_view() {
    let script = [
        Line(3, 85, None, None, json!({ "city": 1 })),
        // A subject that deserves a shot. The news stays for 12 frames.
        Line(4, 50, Some(1), None, json!({})),
        Line(40, 85, None, None, json!({})),
    ];
    let changes = run(&mut director(), 41, &script);
    let views: Vec<_> = changes
        .iter()
        .map(|(frame, shot)| (*frame, shot.zoom, shot.city, shot.breaking, shot.caption.as_str()))
        .collect();
    assert_eq!(
        views,
        [
            (1, Zoom::Country, None, false, "The whole country · 40 residents"),
            (3, Zoom::City, Some(CityId(1)), true, "event 3-0 of drama 85"),
            (15, Zoom::Street, Some(CityId(0)), false, "event 4-0 of drama 50"),
            (40, Zoom::Country, None, true, "event 40-0 of drama 85"),
        ]
    );
    assert_eq!(changes[1].1.pos, Some(Pos::new(160, 32)));
}

#[test]
fn the_camera_does_not_go_straight_back_to_a_subject() {
    let script = [
        Line(3, 60, Some(1), None, json!({})),
        // Bo takes the shot at frame 17: 70 * 89 / 90 = 69 against the
        // 60 * 76 / 90 = 50 of Ada.
        Line(16, 70, Some(2), None, json!({})),
        // Ada has news again. At frame 29 her score is 132, far above
        // 130 percent of the 59 of Bo. But she just left the air: her bid
        // is 132 less 56 percent, 58.
        Line(20, 70, Some(1), None, json!({})),
    ];
    let changes = run(&mut director(), 100, &script);
    assert_eq!(
        residents(&changes),
        [
            (1, 1, None),
            (5, 2, Some(1)),
            (17, 3, Some(2)),
            // Bo keeps the shot until his score is below 12, at the age
            // 75. The bid of Ada never reaches 130 percent of his score,
            // and falls below 30 at frame 74.
            (91, 4, None),
        ]
    );
}

#[test]
fn without_the_loss_for_variety_the_camera_goes_back() {
    // The same script as above, with no loss: Ada takes the shot back at
    // the first frame after the least time of Bo.
    let mut config = Config::default().director;
    config.repeat_penalty_percent = 0;
    let script = [
        Line(3, 60, Some(1), None, json!({})),
        Line(16, 70, Some(2), None, json!({})),
        Line(20, 70, Some(1), None, json!({})),
    ];
    let changes = run(&mut Director::new(&config), 40, &script);
    assert_eq!(
        residents(&changes)[..4],
        [(1, 1, None), (5, 2, Some(1)), (17, 3, Some(2)), (29, 4, Some(1))]
    );
}

#[test]
fn the_next_shot_is_rather_another_kind_of_event() {
    // Ada dies at frame 3 and gets the shot. Then Bo dies (70) and Cleo
    // steals (65). Neither is breaking news. After the least time of the
    // shot, Bo has 63 and Cleo 58. The theft gets the shot: the death of
    // Bo loses 46 percent, because a shot of a death started 12 frames
    // ago.
    let died = json!({ "kind": "life.resident_died" });
    let script = [
        Line(3, 50, Some(1), None, died.clone()),
        Line(8, 70, Some(2), None, died.clone()),
        Line(8, 65, Some(3), None, json!({ "kind": "crime.theft_committed" })),
    ];
    let changes = run(&mut director(), 20, &script);
    assert_eq!(residents(&changes), [(1, 1, None), (5, 2, Some(1)), (17, 3, Some(3))]);

    // Without that rule the more dramatic event wins.
    let mut config = Config::default().director;
    config.kind_penalty_percent = 0;
    let changes = run(&mut Director::new(&config), 20, &script);
    assert_eq!(residents(&changes), [(1, 1, None), (5, 2, Some(1)), (17, 3, Some(2))]);
}

#[test]
fn breaking_news_of_one_kind_does_not_cut_in_again_and_again() {
    let died = json!({ "kind": "life.resident_died" });
    let script = [
        Line(3, 80, Some(1), None, died.clone()),
        // 10 frames later, after `breaking_hold_frames`: the same kind
        // with the same drama does not cut in. Another kind does.
        Line(13, 80, Some(2), None, died.clone()),
        Line(14, 75, Some(3), None, json!({ "kind": "politics.election_decided" })),
        // A more dramatic event of the kind cuts in.
        Line(24, 90, Some(4), None, died.clone()),
        // 120 frames (`breaking_kind_frames`) after the last one, the kind
        // is news again.
        Line(143, 80, Some(2), None, died.clone()),
        Line(144, 80, Some(1), None, died),
    ];
    let changes = run(&mut director(), 144, &script);
    let breaking: Vec<_> = changes
        .iter()
        .filter(|(frame, shot)| shot.breaking && shot.frame == *frame)
        .map(|(frame, shot)| (*frame, shot.caption.as_str()))
        .collect();
    assert_eq!(
        breaking,
        [
            (3, "event 3-0 of drama 80"),
            (14, "event 14-0 of drama 75"),
            (24, "event 24-0 of drama 90"),
            (144, "event 144-0 of drama 80"),
        ]
    );
}

#[test]
fn no_shot_stays_longer_than_the_greatest_time_when_another_waits() {
    // Ada and Bo both have an event in every fifth frame. Ada has the
    // higher score all the time.
    let mut script = Vec::new();
    for frame in (5..200).step_by(5) {
        script.push(Line(frame, 50, Some(1), None, json!({})));
        script.push(Line(frame, 35, Some(2), None, json!({})));
    }
    let changes = run(&mut director(), 199, &script);
    assert_eq!(
        residents(&changes),
        [
            (1, 1, None),
            (5, 2, Some(1)),
            // `max_hold_frames` is 60.
            (65, 3, Some(2)),
            (125, 4, Some(1)),
            (185, 5, Some(2)),
        ]
    );
    // The words of a shot follow the news of its subject. The number of
    // the shot stays.
    let mut all = director();
    let every = run(&mut all, 20, &script);
    let words: Vec<_> =
        every.iter().map(|(frame, shot)| (*frame, shot.seq, shot.caption.as_str())).collect();
    assert_eq!(
        words,
        [
            (1, 1, "The whole country · 40 residents"),
            (5, 2, "event 5-0 of drama 50"),
            (10, 2, "event 10-0 of drama 50"),
            (15, 2, "event 15-0 of drama 50"),
            (20, 2, "event 20-0 of drama 50"),
        ]
    );
}

#[test]
fn events_that_gather_win_a_shot_that_one_event_does_not_get() {
    // One event of drama 25 is below the score for a shot (30).
    let alone = run(&mut director(), 20, &[Line(3, 25, Some(1), None, json!({}))]);
    assert_eq!(residents(&alone), [(1, 1, None)]);
    // Two events of drama 20 around one resident: 40 and 25 percent more.
    let pair =
        [Line(3, 20, Some(2), None, json!({})), Line(4, 20, None, None, json!({ "victim": 2 }))];
    let together = run(&mut director(), 20, &pair);
    assert_eq!(residents(&together), [(1, 1, None), (5, 2, Some(2))]);

    // Six events of drama 20 of six residents in one district. No
    // resident gets a shot. The district does: 120 * 25 percent, doubled.
    let crowd: Vec<Line> = (11..17).map(|id| Line(4, 20, Some(id), Some(2), json!({}))).collect();
    let changes = run(&mut director(), 20, &crowd);
    assert_eq!(changes.len(), 2, "{changes:?}");
    let (frame, shot) = &changes[1];
    assert_eq!(
        (*frame, shot.zone, shot.city, shot.zoom, &shot.focus),
        (
            5,
            Some(ZoneId(2)),
            Some(CityId(1)),
            Zoom::Street,
            &Focus::Place { site: None, name: None, pos: Pos::new(5, 5) }
        )
    );
}

#[test]
fn a_site_can_be_the_subject() {
    // Three events at the bank, each of another resident, with drama 20.
    // A resident has 20. The bank has 60 * 70 percent, and 50 percent more.
    let script: Vec<Line> =
        (11..14).map(|id| Line(4, 20, Some(id), None, json!({ "site": 900 }))).collect();
    let changes = run(&mut director(), 10, &script);
    assert_eq!(changes.len(), 2, "{changes:?}");
    let shot = &changes[1].1;
    assert_eq!(
        shot.focus,
        Focus::Place {
            site: Some(SiteId(900)),
            name: Some("First Bank".into()),
            pos: Pos::new(10, 10)
        }
    );
    assert_eq!(
        (shot.zone, shot.pos, shot.zoom),
        (Some(ZoneId(0)), Some(Pos::new(10, 10)), Zoom::Street)
    );
}

#[test]
fn a_shot_carries_the_words_of_its_story() {
    let words = |id: u64| (id == 7).then(|| ("Crime".to_string(), "The bank job".to_string()));
    let script = [
        Line(3, 50, Some(1), None, json!({ "story": 7 })),
        Line(9, 40, Some(2), None, json!({ "story": 8 })),
    ];
    let changes = run_with(&mut director(), 20, &script, &words);
    let ada = &changes[1].1;
    assert_eq!(
        (ada.story, ada.label.as_deref(), ada.headline.as_deref()),
        (Some(7), Some("Crime"), Some("The bank job"))
    );
    // A new headline of the story changes the words, not the number.
    let later =
        |id: u64| (id == 7).then(|| ("Crime".to_string(), "The bank job: an arrest".to_string()));
    let mut director = director();
    let first = run_with(&mut director, 6, &script[..1], &words);
    assert_eq!(first.last().map(|(frame, shot)| (*frame, shot.seq)), Some((5, 2)));
    let atlas = TestAtlas::town();
    let update = director.frame(7, clock(7), &[], &atlas, &later).expect("the words changed");
    assert_eq!((update.seq, update.headline.as_deref()), (2, Some("The bank job: an arrest")));
    assert_eq!(director.frame(8, clock(8), &[], &atlas, &later), None);
}

#[test]
fn the_same_events_give_the_same_shots() {
    let mut script = Vec::new();
    for frame in 2..300u64 {
        // A fixed mix of subjects and drama scores.
        let drama = ((frame * 37) % 97) as u8;
        let actor = 1 + (frame * 7) % 4;
        let zone = (frame % 3 == 0).then_some((frame % 4) as u32);
        if frame % 2 == 0 {
            script.push(Line(frame, drama, Some(actor), zone, json!({ "victim": 1 + frame % 3 })));
        }
    }
    let first = run(&mut director(), 300, &script);
    let second = run(&mut director(), 300, &script);
    assert!(first.len() > 10, "{}", first.len());
    assert_eq!(first, second);
    // Numbers grow by one with each cut, and never with new words.
    for pair in first.windows(2) {
        let (a, b) = (&pair[0].1, &pair[1].1);
        assert!(b.seq == a.seq || b.seq == a.seq + 1, "{a:?} {b:?}");
        assert_eq!(b.seq == a.seq, b.frame == a.frame, "{a:?} {b:?}");
    }
}

#[test]
fn the_director_keeps_a_bounded_number_of_subjects() {
    let mut config = Config::default().director;
    config.max_subjects = 5;
    let mut director = Director::new(&config);
    let script: Vec<Line> = (100..140).map(|id| Line(3, 20, Some(id), None, json!({}))).collect();
    run(&mut director, 6, &script);
    assert_eq!(director.tracks.len(), 5);
    // The subjects leave when their events count no more.
    run_from(&mut director, 7, 100);
    assert_eq!(director.tracks.len(), 0);
}

/// Runs frames without events.
fn run_from(director: &mut Director, from: u64, to: u64) {
    let atlas = TestAtlas::town();
    for frame in from..=to {
        director.frame(frame, clock(frame), &[], &atlas, &|_| None);
    }
}
