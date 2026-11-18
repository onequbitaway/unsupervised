//! The chronicle: what it keeps, the life story, and the journal.

use std::sync::Arc;

use serde_json::{Value, json};
use unsup_protocol::{ResidentId, SiteId, ZoneId};
use unsup_runtime::storage::{Fault, MemStorage, Storage};

use super::archive::Archive;
use super::*;
use crate::config::Config;
use crate::facts::{Phrasebook, Reader, TestAtlas, test_event};
use crate::wire::EventId;

/// A chronicle with everything that it needs to read an event.
struct Desk {
    chronicle: Chronicle,
    reader: Reader,
    words: Phrasebook,
    atlas: TestAtlas,
    /// The frame and the index of the last event, for the event ids.
    last: (u64, u32),
}

impl Desk {
    fn new() -> Desk {
        Desk::with(&Config::default())
    }

    fn with(config: &Config) -> Desk {
        Desk {
            chronicle: Chronicle::new(&config.chronicle, Chapters::new(&config.chapters)),
            reader: Reader::new(&config.facts),
            words: Phrasebook::new(&config.phrases, &config.facts).expect("phrases"),
            atlas: TestAtlas::town(),
            last: (0, 0),
        }
    }

    fn event(&mut self, frame: u64, kind: &str, drama: u8, actor: Option<u64>, data: Value) {
        self.last = if self.last.0 == frame { (frame, self.last.1 + 1) } else { (frame, 0) };
        let event = test_event(frame, self.last.1, kind, drama, actor, data);
        let facts = self.reader.facts(&event);
        let line = self.words.line(&event, &self.atlas);
        self.chronicle.take(&event, &facts, &line, facts.site);
    }

    /// The life of Ada: an arrival, a job, a home, a loan, a crime against
    /// her, an office and a death.
    fn life_of_ada(&mut self) {
        self.event(1, "kernel.spawned", 10, Some(1), json!({ "resident": 1, "name": "Ada" }));
        self.event(
            5,
            "economy.worker_hired",
            12,
            None,
            json!({ "resident": 1, "site": 900, "job": 3, "wage": 1200 }),
        );
        self.event(
            6,
            "economy.shift_completed",
            2,
            Some(1),
            json!({ "resident": 1, "site": 900, "wage": 1200 }),
        );
        self.event(
            9,
            "economy.lease_started",
            12,
            None,
            json!({ "site": 901, "tenant": 1, "rent": 800 }),
        );
        self.event(
            12,
            "finance.loan_granted",
            20,
            Some(1),
            json!({ "resident": 1, "amount": 50_000 }),
        );
        self.event(
            20,
            "crime.theft_committed",
            55,
            Some(2),
            json!({ "thief": 2, "victim": 1, "amount": 4500 }),
        );
        self.event(
            30,
            "politics.election_decided",
            85,
            Some(1),
            json!({ "city": 0, "office": "mayor", "winner": 1 }),
        );
        self.event(31, "town.fair_opened", 30, Some(1), json!({}));
        self.event(
            40,
            "life.resident_died",
            80,
            Some(1),
            json!({ "resident": 1, "cause": "old_age", "age": 81, "heir": { "Resident": 3 } }),
        );
        self.event(
            40,
            "kernel.despawned",
            30,
            Some(1),
            json!({ "resident": 1, "name": "Ada", "cause": "old_age" }),
        );
    }
}

fn lines(entries: &[LifeEntry]) -> Vec<(u64, &str)> {
    entries.iter().map(|entry| (entry.frame, entry.line.as_str())).collect()
}

#[test]
fn a_life_story_has_its_chapters() {
    let mut desk = Desk::new();
    desk.life_of_ada();
    let life = desk.chronicle.life(ResidentId(1));
    let born = life.born.as_ref().expect("the arrival");
    assert_eq!(
        (born.frame, born.kind.as_str(), born.line.as_str()),
        (1, "kernel.spawned", "Ada arrived in Old Town")
    );
    assert_eq!(lines(&life.jobs), [(5, "Ada got a job at First Bank")]);
    assert_eq!(lines(&life.homes), [(9, "Ada rented a place for $8.00 a day")]);
    assert_eq!(lines(&life.deals), [(12, "Ada got a loan of $500.00")]);
    assert_eq!(lines(&life.crimes), [(20, "Bo stole $45.00 from Ada")]);
    assert_eq!(lines(&life.offices), [(30, "Ada won the election for mayor in Latent City")]);
    // An event without a chapter and with enough drama is a moment. The
    // shift (drama 2) is not kept.
    assert_eq!(lines(&life.moments), [(31, "Fair opened: Ada")]);
    let death = life.death.as_ref().expect("the death");
    // The death is the first event of the end, not the removal after it.
    assert_eq!(
        (death.frame, death.kind.as_str(), death.line.as_str()),
        (40, "life.resident_died", "Ada died at the age of 81 (old age)")
    );
    assert_eq!(life.age, Some(81));
    assert_eq!(death.event, EventId { frame: 40, index: 0 });
    assert_eq!((born.zone, life.jobs[0].site), (Some(ZoneId(0)), Some(SiteId(900))));

    // The theft is also in the life of the thief. The death of Ada is a
    // moment in the life of her heir, not his own death.
    let bo = desk.chronicle.life(ResidentId(2));
    assert_eq!(lines(&bo.crimes), [(20, "Bo stole $45.00 from Ada")]);
    assert_eq!((bo.born.is_some(), bo.death.is_some()), (false, false));
    let heir = desk.chronicle.life(ResidentId(3));
    assert_eq!(heir.death, None);
    assert_eq!(lines(&heir.moments), [(40, "Ada died at the age of 81 (old age)")]);
    assert_eq!(desk.chronicle.life(ResidentId(99)), LifeStory::default());
}

#[test]
fn a_place_has_a_history() {
    let mut desk = Desk::new();
    desk.life_of_ada();
    let (zone, events) = desk.chronicle.place(SiteId(900), 10).expect("the bank has a history");
    assert_eq!(zone, Some(ZoneId(0)));
    assert_eq!(lines(&events), [(5, "Ada got a job at First Bank")]);
    desk.event(
        50,
        "economy.business_closed",
        25,
        None,
        json!({ "site": 900, "kind": "bank", "reason": "owner" }),
    );
    desk.event(
        60,
        "economy.business_opened",
        25,
        Some(2),
        json!({ "site": 900, "kind": "shop", "owner": { "Resident": 2 } }),
    );
    let (_, events) = desk.chronicle.place(SiteId(900), 2).expect("a history");
    // The newest two, oldest first.
    assert_eq!(
        lines(&events),
        [(50, "The bank at First Bank closed (owner)"), (60, "Bo opened a shop at First Bank")]
    );
    assert_eq!(desk.chronicle.place(SiteId(5), 10), None);
}

#[test]
fn the_history_of_a_resident_is_bounded_and_keeps_the_arrival() {
    let mut config = Config::default();
    config.chronicle.resident_events = 4;
    config.chronicle.place_events = 2;
    let mut desk = Desk::with(&config);
    desk.event(1, "kernel.spawned", 10, Some(1), json!({ "resident": 1 }));
    for frame in 2..=8 {
        desk.event(frame, "economy.worker_hired", 12, None, json!({ "resident": 1, "site": 900 }));
    }
    let life = desk.chronicle.life(ResidentId(1));
    assert_eq!(life.born.as_ref().map(|born| born.frame), Some(1));
    assert_eq!(life.jobs.iter().map(|job| job.frame).collect::<Vec<_>>(), [6, 7, 8]);
    let (_, events) = desk.chronicle.place(SiteId(900), 10).expect("a history");
    assert_eq!(events.iter().map(|event| event.frame).collect::<Vec<_>>(), [7, 8]);
}

#[test]
fn the_chronicle_forgets_the_residents_with_the_oldest_events() {
    let mut config = Config::default();
    config.chronicle.residents = 32;
    let mut desk = Desk::with(&config);
    for id in 1..=33u64 {
        desk.event(id, "kernel.spawned", 10, Some(id), json!({ "resident": id }));
    }
    // One too many: the chronicle forgets that one and 2 more (32 / 16).
    assert_eq!(desk.chronicle.residents(), 30);
    assert_eq!(desk.chronicle.life(ResidentId(3)), LifeStory::default());
    assert!(desk.chronicle.life(ResidentId(4)).born.is_some());
    assert!(desk.chronicle.life(ResidentId(33)).born.is_some());
}

/// A world signature for the tests.
const WORLD: u64 = 77;

fn open(storage: &MemStorage, world: u64) -> (Archive, Vec<Record>) {
    Archive::open(Arc::new(storage.clone()), world).expect("the storage works")
}

#[test]
fn the_chronicle_is_there_again_after_a_restart() {
    let storage = MemStorage::new();
    let mut desk = Desk::new();
    desk.life_of_ada();
    let before = desk.chronicle.life(ResidentId(1));
    let (mut archive, found) = open(&storage, WORLD);
    assert!(found.is_empty());
    // The first write is a new journal.
    assert!(archive.wants_rewrite(1000));
    let unwritten = desk.chronicle.take_unwritten();
    assert_eq!(unwritten.len(), 9);
    archive.rewrite(&unwritten[..4]).expect("write");
    assert!(!archive.wants_rewrite(1000));
    archive.append(&unwritten[4..]).expect("append");
    assert!(desk.chronicle.take_unwritten().is_empty());

    // A new service reads the journal.
    let (_, records) = open(&storage, WORLD);
    assert_eq!(records, unwritten);
    let mut again = Desk::new();
    assert_eq!(again.chronicle.restore(records.clone(), 1000), 9);
    assert_eq!(again.chronicle.life(ResidentId(1)), before);
    assert_eq!(again.chronicle.life(ResidentId(3)), desk.chronicle.life(ResidentId(3)));
    assert_eq!(again.chronicle.place(SiteId(900), 10), desk.chronicle.place(SiteId(900), 10));
    // What was restored is not written a second time.
    assert!(again.chronicle.take_unwritten().is_empty());

    // The world is at frame 20: later events did not happen in it.
    let mut early = Desk::new();
    assert_eq!(early.chronicle.restore(records, 20), 5);
    let life = early.chronicle.life(ResidentId(1));
    assert_eq!(
        (life.born.is_some(), life.crimes.len(), life.offices.len(), life.death.is_some()),
        (true, 1, 0, false)
    );
}

#[test]
fn a_journal_of_another_world_is_not_read() {
    let storage = MemStorage::new();
    let mut desk = Desk::new();
    desk.life_of_ada();
    let (mut archive, _) = open(&storage, WORLD);
    archive.rewrite(&desk.chronicle.take_unwritten()).expect("write");
    let (mut other, records) = open(&storage, WORLD + 1);
    assert!(records.is_empty());
    // The new world writes its own journal. The old one goes.
    other.rewrite(&[]).expect("write");
    assert_eq!(storage.list("chronicle").expect("list"), ["chronicle/journal-0000000002.jsonl"]);
    assert!(open(&storage, WORLD).1.is_empty());
}

#[test]
fn a_torn_line_at_the_end_loses_only_that_line() {
    let storage = MemStorage::new();
    let mut desk = Desk::new();
    desk.life_of_ada();
    let records = desk.chronicle.take_unwritten();
    let (mut archive, _) = open(&storage, WORLD);
    archive.rewrite(&records[..3]).expect("write");
    // The disk fills up in the middle of the next lines.
    storage.set_fault(Some(Fault::Append));
    assert!(archive.append(&records[3..5]).is_err());
    storage.set_fault(None);
    // The journal is not safe to continue: the next write is a new one.
    assert!(archive.wants_rewrite(1000));
    let (_, found) = open(&storage, WORLD);
    assert_eq!(found.len(), 3 + 1, "one whole line of the failed write is there");
    assert_eq!(found[..3], records[..3]);
    archive.rewrite(&records).expect("write");
    assert_eq!(open(&storage, WORLD).1, records);
    assert_eq!(storage.list("chronicle").expect("list").len(), 1);
}

#[test]
fn a_rewrite_holds_what_the_chronicle_holds() {
    let mut config = Config::default();
    config.chronicle.resident_events = 5;
    let mut desk = Desk::with(&config);
    desk.life_of_ada();
    // Ada keeps 5 events: her arrival, the election, the fair and the two
    // events of her end. Other residents and the places keep theirs.
    let records = desk.chronicle.records();
    let frames: Vec<u64> = records.iter().map(|record| record.entry.entry.frame).collect();
    assert_eq!(frames, [1, 5, 9, 20, 30, 31, 40, 40]);
    // The theft left the life of Ada. Bo, the thief, still has it.
    let theft = &records[3];
    assert_eq!((theft.about, theft.others.as_slice()), (Some(ResidentId(2)), [].as_slice()));
    // The job at frame 5 left the life of Ada. The bank still has it.
    assert_eq!((records[1].about, records[1].place), (None, Some(SiteId(900))));
    // The death names the heir.
    assert_eq!(
        (records[6].about, records[6].others.as_slice()),
        (Some(ResidentId(1)), [ResidentId(3)].as_slice())
    );
    let mut again = Desk::with(&config);
    again.chronicle.restore(records, 1000);
    assert_eq!(again.chronicle.life(ResidentId(1)), desk.chronicle.life(ResidentId(1)));
    assert_eq!(again.chronicle.records(), desk.chronicle.records());
}
