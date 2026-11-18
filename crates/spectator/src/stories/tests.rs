//! Scripted event sequences for the stories, with the rules of
//! `data/director.toml`.

use serde_json::{Value, json};
use unsup_protocol::ZoneId;

use super::*;
use crate::config::Config;
use crate::facts::{Reader, TestAtlas, test_event};
use crate::wire::EventId;

/// The stories with everything that they need to read an event.
struct Desk {
    stories: Stories,
    reader: Reader,
    words: Phrasebook,
    atlas: TestAtlas,
}

impl Desk {
    fn new() -> Desk {
        Desk::with(Config::default())
    }

    fn with(config: Config) -> Desk {
        Desk {
            stories: Stories::new(&config.story_kinds, &config.stories),
            reader: Reader::new(&config.facts),
            words: Phrasebook::new(&config.phrases, &config.facts).expect("phrases"),
            atlas: TestAtlas::town(),
        }
    }

    /// Gives one event to the stories and ends its frame.
    fn event(
        &mut self,
        frame: u64,
        kind: &str,
        drama: u8,
        actor: Option<u64>,
        data: Value,
    ) -> Option<u64> {
        let event = test_event(frame, 0, kind, drama, actor, data);
        let facts = self.reader.facts(&event);
        let line = self.words.line(&event, &self.atlas);
        let story = self.stories.take(&event, &facts, &line, &self.atlas, &self.words);
        self.stories.end_frame(frame);
        story
    }

    fn story(&self, id: u64) -> &Story {
        self.stories.get(id).expect("a story that viewers see")
    }
}

fn names(story: &Story) -> Vec<&str> {
    story.residents.iter().map(|resident| resident.name.as_str()).collect()
}

fn lines(story: &Story) -> Vec<(u64, &str)> {
    story.timeline.iter().map(|beat| (beat.frame, beat.line.as_str())).collect()
}

#[test]
fn a_heist_and_its_investigation_are_one_story() {
    let mut desk = Desk::new();
    let theft = json!({ "thief": 2, "victim": 1, "amount": 4500 });
    assert_eq!(desk.event(10, "crime.theft_committed", 55, Some(2), theft), Some(1));
    // The investigation names the thief. It brings the case number.
    let opened = json!({ "officer": 3, "suspect": 2, "case": 4 });
    assert_eq!(desk.event(20, "crime.investigation_opened", 30, Some(3), opened), Some(1));
    // The arrest names only the case.
    assert_eq!(desk.event(30, "crime.arrest_made", 50, None, json!({ "case": 4 })), Some(1));
    // Another thief and another victim: another story.
    let other = json!({ "thief": 4, "victim": 9, "amount": 100 });
    assert_eq!(desk.event(31, "crime.theft_committed", 45, Some(4), other), Some(2));
    // A trial with another case number and other residents is no part of
    // the first story, and its drama is too low to open one (41).
    assert_eq!(desk.event(32, "crime.trial_opened", 30, None, json!({ "case": 5 })), None);
    assert_eq!(
        desk.event(40, "crime.trial_ended", 50, Some(4), json!({ "case": 4, "accused": 2 })),
        Some(1)
    );

    let story = desk.story(1);
    assert_eq!(
        (story.kind.as_str(), story.label.as_str(), story.status, story.events, story.drama),
        ("crime", "Crime", StoryStatus::Open, 4, 55)
    );
    // The headline is the line of the most dramatic event.
    assert_eq!(story.headline, "Bo stole $45.00 from Ada");
    assert_eq!(story.latest, "The trial of Bo ended (no verdict)");
    assert_eq!(story.lead, EventId { frame: 10, index: 0 });
    assert_eq!((story.started, story.updated, story.zone), (10, 40, Some(ZoneId(0))));
    assert_eq!(names(story), ["Bo", "Ada", "Cleo", "Dev"]);
    assert_eq!(
        lines(story),
        [
            (10, "Bo stole $45.00 from Ada"),
            (20, "Cleo opened an investigation against Bo"),
            (30, "A suspect was arrested"),
            (40, "The trial of Bo ended (no verdict)"),
        ]
    );
    assert_eq!(names(desk.story(2)), ["Dev", "resident 9"]);
    assert_eq!(
        desk.stories.words(1),
        Some(("Crime".to_string(), "Bo stole $45.00 from Ada".to_string()))
    );
}

#[test]
fn an_election_is_one_story_for_each_city_and_office() {
    let mut desk = Desk::new();
    let run = |city: u64, who: u64| json!({ "city": city, "office": "mayor", "candidate": who });
    assert_eq!(desk.event(5, "politics.candidate_registered", 20, Some(1), run(0, 1)), Some(1));
    assert_eq!(desk.event(6, "politics.candidate_registered", 20, Some(2), run(0, 2)), Some(1));
    assert_eq!(desk.event(7, "politics.candidate_registered", 20, Some(3), run(1, 3)), Some(2));
    let won = json!({ "city": 0, "office": "mayor", "winner": 2 });
    assert_eq!(desk.event(90, "politics.election_decided", 85, Some(2), won), Some(1));
    let story = desk.story(1);
    assert_eq!((story.kind.as_str(), story.events, story.drama), ("election", 3, 85));
    // The headline of an election is its newest line.
    assert_eq!(story.headline, "Bo won the election for mayor in Latent City");
    assert_eq!(names(story), ["Ada", "Bo"]);
    assert_eq!(names(desk.story(2)), ["Cleo"]);
    // The candidates of the other city share no thread with the first.
    assert_eq!(desk.story(2).events, 1);
}

#[test]
fn a_feud_needs_two_events_between_the_same_two_residents() {
    let mut desk = Desk::new();
    let breach = json!({ "contract": 1, "by": 1, "parties": [1, 2] });
    // One event is not a feud yet: viewers do not see a story.
    assert_eq!(desk.event(10, "society.contract_breached", 40, Some(1), breach), None);
    assert!(desk.stories.list(10, false).is_empty());
    assert!(desk.stories.take_updates(10).is_empty());
    // Praise is no part of a feud. An opinion about another resident is
    // not between the same two.
    let opinion = |from: u64, about: u64, verdict: &str| json!({ "from": from, "about": about, "verdict": verdict });
    assert_eq!(desk.event(20, "society.opinion_given", 12, Some(2), opinion(2, 1, "praise")), None);
    assert_eq!(
        desk.event(21, "society.opinion_given", 12, Some(2), opinion(2, 3, "condemn")),
        None
    );
    assert!(desk.stories.list(10, false).is_empty());
    // The second event between Ada and Bo makes the feud.
    assert_eq!(
        desk.event(30, "society.opinion_given", 12, Some(2), opinion(2, 1, "condemn")),
        Some(1)
    );
    let story = desk.story(1);
    assert_eq!((story.kind.as_str(), story.label.as_str(), story.events), ("feud", "Feud", 2));
    assert_eq!(story.headline, "Bo spoke in public about Ada (condemn)");
    assert_eq!(names(story), ["Ada", "Bo"]);
    assert_eq!(
        lines(story),
        [(10, "Ada broke a contract"), (30, "Bo spoke in public about Ada (condemn)")]
    );
    let updates = desk.stories.take_updates(10);
    assert_eq!(updates.iter().map(|story| story.id).collect::<Vec<_>>(), [1]);
    assert!(desk.stories.take_updates(10).is_empty());
}

#[test]
fn a_bankruptcy_gathers_the_default_and_what_follows() {
    let mut desk = Desk::new();
    let default = json!({ "resident": 1, "outstanding": 250_000 });
    assert_eq!(desk.event(10, "finance.loan_defaulted", 60, Some(1), default), Some(1));
    let foreclosed =
        json!({ "site": 900, "city": 0, "from": { "Resident": 1 }, "reason": "loan_default" });
    assert_eq!(desk.event(11, "economy.property_foreclosed", 55, None, foreclosed), Some(1));
    assert_eq!(
        desk.event(300, "economy.tenant_evicted", 45, None, json!({ "site": 900, "tenant": 1 })),
        Some(1)
    );
    // The eviction of another resident opens its own story.
    assert_eq!(
        desk.event(301, "economy.tenant_evicted", 45, None, json!({ "site": 900, "tenant": 3 })),
        Some(2)
    );
    let story = desk.story(1);
    assert_eq!((story.kind.as_str(), story.events), ("bankruptcy", 3));
    assert_eq!(story.headline, "Ada defaulted on a loan of $2,500.00");
    assert_eq!(story.latest, "Ada was evicted from First Bank");
    assert_eq!(names(story), ["Ada"]);
}

#[test]
fn a_story_closes_when_it_goes_quiet() {
    let mut config = Config::default();
    config.stories.close_frames = 100;
    let mut desk = Desk::with(config);
    let theft = |thief: u64| json!({ "thief": thief, "victim": 1, "amount": 4500 });
    assert_eq!(desk.event(10, "crime.theft_committed", 55, Some(2), theft(2)), Some(1));
    assert_eq!(desk.stories.take_updates(10).len(), 1);
    desk.stories.end_frame(109);
    assert_eq!(desk.story(1).status, StoryStatus::Open);
    assert!(desk.stories.take_updates(10).is_empty());
    desk.stories.end_frame(110);
    assert_eq!(desk.story(1).status, StoryStatus::Closed);
    // Viewers hear that it closed.
    let updates = desk.stories.take_updates(10);
    assert_eq!(
        updates.iter().map(|s| (s.id, s.status)).collect::<Vec<_>>(),
        [(1, StoryStatus::Closed)]
    );
    // A closed story takes no event: the same residents make a new story.
    assert_eq!(desk.event(120, "crime.theft_committed", 55, Some(2), theft(2)), Some(2));
    assert_eq!(desk.event(121, "crime.theft_committed", 55, Some(3), theft(3)), Some(2));
    // Open stories come first, then the closed ones.
    let listed: Vec<(u64, StoryStatus)> =
        desk.stories.list(10, false).iter().map(|s| (s.id, s.status)).collect();
    assert_eq!(listed, [(2, StoryStatus::Open), (1, StoryStatus::Closed)]);
    let open: Vec<u64> = desk.stories.list(10, true).iter().map(|s| s.id).collect();
    assert_eq!(open, [2]);
    assert_eq!(desk.stories.list(1, false).len(), 1);
    assert_eq!(desk.stories.frame(), 121);
}

#[test]
fn an_event_joins_by_residents_only_a_story_that_is_fresh() {
    let mut config = Config::default();
    config.stories.close_frames = 10_000;
    let mut desk = Desk::with(config);
    let theft = json!({ "thief": 2, "victim": 1, "amount": 4500 });
    assert_eq!(desk.event(10, "crime.theft_committed", 55, Some(2), theft.clone()), Some(1));
    // `link_frames` of a crime is 720.
    assert_eq!(desk.event(730, "crime.theft_committed", 55, Some(2), theft.clone()), Some(1));
    assert_eq!(desk.event(1451, "crime.theft_committed", 55, Some(2), theft), Some(2));
}

#[test]
fn the_stories_are_bounded() {
    let mut config = Config::default();
    config.stories.max_open = 2;
    config.stories.max_closed = 1;
    config.stories.timeline = 3;
    config.stories.residents = 2;
    let mut desk = Desk::with(config);
    let theft = |thief: u64, victim: u64| json!({ "thief": thief, "victim": victim, "amount": 1 });
    // Story 1 gets five events. The timeline keeps the first and the
    // newest two.
    for frame in 1..=5 {
        assert_eq!(
            desk.event(frame, "crime.theft_committed", 50, Some(1), theft(1, 10 + frame)),
            Some(1)
        );
    }
    let story = desk.story(1);
    assert_eq!(story.events, 5);
    assert_eq!(story.timeline.iter().map(|beat| beat.frame).collect::<Vec<_>>(), [1, 4, 5]);
    assert_eq!(names(story), ["Ada", "resident 11"]);
    // A later event with a resident that the story does not list still
    // joins: the story remembers more residents than it lists.
    assert_eq!(desk.event(6, "crime.fence_used", 20, Some(15), json!({})), Some(1));

    assert_eq!(desk.event(7, "crime.theft_committed", 50, Some(2), theft(2, 20)), Some(2));
    // A third story: the open story with the oldest event closes.
    assert_eq!(desk.event(8, "crime.theft_committed", 50, Some(3), theft(3, 30)), Some(3));
    assert_eq!(desk.story(1).status, StoryStatus::Closed);
    // A fourth: story 2 closes, and the service forgets story 1.
    assert_eq!(desk.event(9, "crime.theft_committed", 50, Some(4), theft(4, 40)), Some(4));
    assert!(desk.stories.get(1).is_none());
    assert_eq!(desk.story(2).status, StoryStatus::Closed);

    // Updates come in pieces, the most dramatic first.
    let mut desk = Desk::new();
    for (frame, drama) in [(1, 45), (2, 60), (3, 50)] {
        desk.event(frame, "crime.theft_committed", drama, Some(frame), theft(frame, 100 + frame));
    }
    let first: Vec<u64> = desk.stories.take_updates(2).iter().map(|s| s.id).collect();
    assert_eq!(first, [2, 3]);
    let second: Vec<u64> = desk.stories.take_updates(2).iter().map(|s| s.id).collect();
    assert_eq!(second, [1]);
    assert!(desk.stories.take_updates(2).is_empty());
}

#[test]
fn the_same_events_give_the_same_stories() {
    let run = || {
        let mut desk = Desk::new();
        for frame in 1..200u64 {
            let a = 1 + frame % 4;
            let b = 1 + (frame * 7) % 5;
            let kind = ["crime.theft_committed", "society.opinion_given", "finance.loan_defaulted"]
                [(frame % 3) as usize];
            let data = json!({ "thief": a, "victim": b, "from": a, "about": b, "verdict": "condemn", "resident": a });
            desk.event(frame, kind, (frame * 13 % 90) as u8, Some(a), data);
        }
        desk.stories.list(100, false)
    };
    let first = run();
    assert!(first.len() > 3, "{}", first.len());
    assert_eq!(first, run());
}
