//! The chronicle: a bounded history of public events for each resident
//! and for each place.
//!
//! The chronicle keeps the events of the chapters of a life story (born,
//! jobs, homes, deals, crimes, offices, death) and every other event with
//! a drama score of at least `chronicle.min_drama`. It keeps one line for
//! each event, written with the names of that time, not the whole event.
//!
//! The chronicle lives in memory. The service also writes it to a storage
//! (see [`archive`]), so that the life stories are there again after a
//! restart.

pub(crate) mod archive;
mod chapters;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{ResidentId, SiteId, ZoneId};

pub(crate) use self::chapters::{Chapter, Chapters};
use crate::config::ChronicleConfig;
use crate::facts::Facts;
use crate::wire::{FeedEvent, LifeEntry, LifeStory};

/// One event of the chronicle with its chapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// The event, as viewers get it.
    #[serde(flatten)]
    pub entry: LifeEntry,
    /// The chapter of the event, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<Chapter>,
    /// The age that the event says, for a death.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<u64>,
}

/// One event with the residents and the place that it is about. This is
/// also one line of the journal on the storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// The resident that the event is about: its actor, or the first
    /// resident that its data names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<ResidentId>,
    /// The other residents of the event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<ResidentId>,
    /// The site of the event, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<SiteId>,
    /// The event.
    #[serde(with = "shared")]
    pub entry: Arc<Entry>,
}

/// Writes and reads a shared entry as the entry itself.
mod shared {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Entry;

    pub fn serialize<S: Serializer>(entry: &Arc<Entry>, serializer: S) -> Result<S::Ok, S::Error> {
        Entry::serialize(entry, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<Entry>, D::Error> {
        Entry::deserialize(deserializer).map(Arc::new)
    }
}

/// The events of one resident, oldest first.
#[derive(Clone, Debug, Default)]
struct ResidentLog {
    /// The events with the part of the resident in each: true when the
    /// event is about this resident.
    entries: VecDeque<(Arc<Entry>, bool)>,
}

/// The events of one place, oldest first.
#[derive(Clone, Debug, Default)]
struct PlaceLog {
    entries: VecDeque<Arc<Entry>>,
}

/// Returns the frame of the newest event of a log.
fn newest<T>(entries: &VecDeque<T>, frame: impl Fn(&T) -> u64) -> u64 {
    entries.back().map_or(0, frame)
}

/// The chronicle. See the module documentation.
#[derive(Debug)]
pub(crate) struct Chronicle {
    limits: ChronicleConfig,
    chapters: Chapters,
    residents: BTreeMap<ResidentId, ResidentLog>,
    places: BTreeMap<SiteId, PlaceLog>,
    /// Records that the storage does not have yet.
    unwritten: Vec<Record>,
}

impl Chronicle {
    /// Makes an empty chronicle.
    pub fn new(limits: &ChronicleConfig, chapters: Chapters) -> Chronicle {
        Chronicle {
            limits: limits.clone(),
            chapters,
            residents: BTreeMap::new(),
            places: BTreeMap::new(),
            unwritten: Vec::new(),
        }
    }

    /// Gives an event to the chronicle. `site` is the site of the event:
    /// the one that the data names, or the one at the place of the event.
    pub fn take(&mut self, event: &FeedEvent, facts: &Facts, line: &str, site: Option<SiteId>) {
        let chapter = self.chapters.of(&event.kind);
        if chapter.is_none() && event.drama < self.limits.min_drama {
            return;
        }
        if facts.residents.is_empty() && site.is_none() {
            return;
        }
        let entry = Arc::new(Entry {
            entry: LifeEntry {
                event: event.id,
                frame: event.frame,
                clock: event.clock,
                kind: event.kind.clone(),
                drama: event.drama,
                line: line.to_string(),
                zone: event.zone,
                site,
            },
            chapter,
            age: (chapter == Some(Chapter::Death))
                .then(|| event.data.get("age").and_then(Value::as_u64))
                .flatten(),
        });
        let record = Record {
            about: facts.residents.first().copied(),
            others: facts.residents.iter().skip(1).copied().collect(),
            place: site,
            entry,
        };
        self.put(&record);
        self.unwritten.push(record);
    }

    /// Adds a record to the logs of its residents and of its place.
    fn put(&mut self, record: &Record) {
        let about = record.about.iter().map(|resident| (resident, true));
        let others = record.others.iter().map(|resident| (resident, false));
        for (resident, is_about) in about.chain(others) {
            let log = self.residents.entry(*resident).or_default();
            log.entries.push_back((Arc::clone(&record.entry), is_about));
            if log.entries.len() > self.limits.resident_events {
                // The arrival and the end of a life stay.
                let spare = log.entries.iter().position(|(entry, about)| {
                    !(*about && matches!(entry.chapter, Some(Chapter::Born | Chapter::Death)))
                });
                log.entries.remove(spare.unwrap_or(0));
            }
        }
        if let Some(site) = record.place {
            let log = self.places.entry(site).or_default();
            log.entries.push_back(Arc::clone(&record.entry));
            if log.entries.len() > self.limits.place_events {
                log.entries.pop_front();
            }
        }
        self.keep_bounds();
    }

    /// Forgets the residents and the places with the oldest events when
    /// there are too many. It forgets some more than it must, so that it
    /// does not search at every event.
    fn keep_bounds(&mut self) {
        fn trim<K: Copy + Ord, V>(
            map: &mut BTreeMap<K, V>,
            max: usize,
            one_in: usize,
            newest: impl Fn(&V) -> u64,
        ) {
            if map.len() <= max {
                return;
            }
            let extra = map.len() - max + max / one_in.max(1);
            let mut ages: Vec<(u64, K)> =
                map.iter().map(|(key, log)| (newest(log), *key)).collect();
            ages.sort_unstable();
            for (_, key) in ages.into_iter().take(extra) {
                map.remove(&key);
            }
        }
        let one_in = self.limits.forget_one_in;
        trim(&mut self.residents, self.limits.residents, one_in, |log| {
            newest(&log.entries, |(entry, _)| entry.entry.frame)
        });
        trim(&mut self.places, self.limits.places, one_in, |log| {
            newest(&log.entries, |entry| entry.entry.frame)
        });
    }

    /// Puts records from the storage into an empty chronicle. Records of
    /// frames after `max_frame` are from a time that the world does not
    /// have: they are left out.
    pub fn restore(&mut self, records: Vec<Record>, max_frame: u64) -> usize {
        let mut kept = 0;
        for record in records {
            if record.entry.entry.frame <= max_frame {
                self.put(&record);
                kept += 1;
            }
        }
        kept
    }

    /// Returns the records that the storage does not have yet, and forgets
    /// them.
    pub fn take_unwritten(&mut self) -> Vec<Record> {
        std::mem::take(&mut self.unwritten)
    }

    /// Returns everything that the chronicle holds as records, in the
    /// order of the events, for a new journal.
    pub fn records(&self) -> Vec<Record> {
        let mut records: BTreeMap<crate::wire::EventId, Record> = BTreeMap::new();
        let blank = |entry: &Arc<Entry>| Record {
            about: None,
            others: Vec::new(),
            place: None,
            entry: Arc::clone(entry),
        };
        for (resident, log) in &self.residents {
            for (entry, about) in &log.entries {
                let record = records.entry(entry.entry.event).or_insert_with(|| blank(entry));
                if *about {
                    record.about = Some(*resident);
                } else {
                    record.others.push(*resident);
                }
            }
        }
        for (site, log) in &self.places {
            for entry in &log.entries {
                let record = records.entry(entry.entry.event).or_insert_with(|| blank(entry));
                record.place = Some(*site);
            }
        }
        records.into_values().collect()
    }

    /// Returns the life story of a resident. A resident without a history
    /// has an empty one.
    pub fn life(&self, resident: ResidentId) -> LifeStory {
        let mut life = LifeStory::default();
        let Some(log) = self.residents.get(&resident) else {
            return life;
        };
        for (entry, about) in &log.entries {
            let line = entry.entry.clone();
            // The arrival and the death of another resident are moments in
            // the life of this one.
            match entry.chapter {
                Some(Chapter::Born) if *about => {
                    life.born.get_or_insert(line);
                }
                Some(Chapter::Death) if *about => {
                    if life.death.is_none() {
                        life.age = entry.age;
                        life.death = Some(line);
                    }
                }
                Some(Chapter::Jobs) => life.jobs.push(line),
                Some(Chapter::Homes) => life.homes.push(line),
                Some(Chapter::Deals) => life.deals.push(line),
                Some(Chapter::Crimes) => life.crimes.push(line),
                Some(Chapter::Offices) => life.offices.push(line),
                Some(Chapter::Born | Chapter::Death) | None => life.moments.push(line),
            }
        }
        life
    }

    /// Returns the newest `limit` events of a place, oldest first, and the
    /// zone of the place. `None`: the chronicle has no event of the place.
    pub fn place(&self, site: SiteId, limit: usize) -> Option<(Option<ZoneId>, Vec<LifeEntry>)> {
        let log = self.places.get(&site)?;
        let zone = log.entries.iter().rev().find_map(|entry| entry.entry.zone);
        let skip = log.entries.len().saturating_sub(limit);
        Some((zone, log.entries.iter().skip(skip).map(|entry| entry.entry.clone()).collect()))
    }

    /// Returns the number of residents with a history.
    #[cfg(test)]
    pub fn residents(&self) -> usize {
        self.residents.len()
    }
}

#[cfg(test)]
mod tests;
