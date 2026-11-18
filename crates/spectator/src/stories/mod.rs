//! Stories: events that belong together.
//!
//! A story is a heist and its investigation, an election, a feud, a
//! bankruptcy. The rules of `data/director.toml` say which events make
//! which kind of story. An event joins an open story of its kind when it
//! names the same thread (a case, an election), or when it shares enough
//! residents with the story. Else it can open a new story.
//!
//! The stories have no clock and no randomness. The same events in the
//! same frames give the same stories with the same ids.

mod rules;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use unsup_protocol::ResidentId;

use self::rules::Thread;
pub(crate) use self::rules::headline_slots;
use crate::config::{StoriesConfig, StoryKind};
use crate::facts::{Atlas, Facts, Phrasebook, clean};
use crate::wire::{FeedEvent, Story, StoryBeat, StoryResident, StoryStatus};

/// One story with what the service needs to add events to it.
#[derive(Clone, Debug)]
struct Record {
    /// The story as viewers get it.
    story: Story,
    /// The rule of the story: an index into the rules.
    rule: usize,
    /// The thread of the story, if an event named one.
    thread: Option<Thread>,
    /// The residents of the story.
    residents: Vec<ResidentId>,
    /// The line of the most dramatic event.
    lead_line: String,
    /// True when viewers see the story.
    public: bool,
}

/// The stories. See the module documentation.
#[derive(Debug)]
pub(crate) struct Stories {
    rules: Vec<StoryKind>,
    limits: StoriesConfig,
    /// The id of the next story.
    next_id: u64,
    open: BTreeMap<u64, Record>,
    /// Closed stories, the one that closed last at the end.
    closed: VecDeque<Record>,
    /// Stories that changed since viewers were told.
    dirty: BTreeSet<u64>,
    /// The last frame that the stories saw.
    frame: u64,
}

impl Stories {
    /// Makes the stories without a story.
    pub fn new(rules: &[StoryKind], limits: &StoriesConfig) -> Stories {
        Stories {
            rules: rules.to_vec(),
            limits: limits.clone(),
            next_id: 1,
            open: BTreeMap::new(),
            closed: VecDeque::new(),
            dirty: BTreeSet::new(),
            frame: 0,
        }
    }

    /// Returns the last frame that the stories saw.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Returns true when an event belongs to a kind of story.
    pub fn wants(&self, event: &FeedEvent) -> bool {
        self.rules.iter().any(|rule| rules::matches(rule, event))
    }

    /// Gives an event to the stories. Returns the id of the first story
    /// that took it and that viewers see.
    pub fn take(
        &mut self,
        event: &FeedEvent,
        facts: &Facts,
        line: &str,
        atlas: &dyn Atlas,
        words: &Phrasebook,
    ) -> Option<u64> {
        let mut first = None;
        for rule in 0..self.rules.len() {
            if !rules::matches(&self.rules[rule], event) {
                continue;
            }
            let Some(id) = self.place(rule, event, facts) else {
                continue;
            };
            self.add(id, event, facts, line, atlas, words);
            if first.is_none() && self.open.get(&id).is_some_and(|record| record.public) {
                first = Some(id);
            }
        }
        first
    }

    /// Finds the story of a rule that takes an event, or opens one.
    fn place(&mut self, rule: usize, event: &FeedEvent, facts: &Facts) -> Option<u64> {
        let kind = &self.rules[rule];
        let thread = rules::thread_of(kind, event);
        if let Some(thread) = &thread
            && let Some((id, _)) = self
                .open
                .iter()
                .find(|(_, record)| record.rule == rule && record.thread.as_ref() == Some(thread))
        {
            return Some(*id);
        }
        // By residents: the story with the newest event wins. A story with
        // another thread is another case.
        if kind.share > 0 {
            let found = self
                .open
                .iter()
                .filter(|(_, record)| {
                    record.rule == rule
                        && (record.thread.is_none() || thread.is_none())
                        && event.frame.saturating_sub(record.story.updated) <= kind.link_frames
                        && shared(&record.residents, &facts.residents) >= kind.share
                })
                .max_by_key(|(id, record)| (record.story.updated, **id))
                .map(|(id, _)| *id);
            if let Some(id) = found {
                if let (Some(thread), Some(record)) = (thread, self.open.get_mut(&id)) {
                    record.thread = Some(thread);
                }
                return Some(id);
            }
        }
        // A story that events join by their residents needs residents.
        let enough = facts.residents.len() >= kind.share || thread.is_some();
        if event.drama < kind.open_drama || !enough {
            return None;
        }
        Some(self.open_story(rule, thread, event))
    }

    /// Opens an empty story. The oldest open story closes when there are
    /// too many.
    fn open_story(&mut self, rule: usize, thread: Option<Thread>, event: &FeedEvent) -> u64 {
        while self.open.len() >= self.limits.max_open {
            let oldest = self
                .open
                .iter()
                .min_by_key(|(id, record)| (record.story.updated, **id))
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => self.close(id),
                None => break,
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        let kind = &self.rules[rule];
        self.open.insert(
            id,
            Record {
                story: Story {
                    id,
                    kind: kind.kind.clone(),
                    label: kind.label.clone(),
                    headline: String::new(),
                    latest: String::new(),
                    status: StoryStatus::Open,
                    started: event.frame,
                    started_clock: event.clock,
                    updated: event.frame,
                    updated_clock: event.clock,
                    drama: 0,
                    events: 0,
                    residents: Vec::new(),
                    zone: None,
                    lead: event.id,
                    timeline: Vec::new(),
                },
                rule,
                thread,
                residents: Vec::new(),
                lead_line: String::new(),
                public: false,
            },
        );
        id
    }

    /// Adds an event to an open story.
    fn add(
        &mut self,
        id: u64,
        event: &FeedEvent,
        facts: &Facts,
        line: &str,
        atlas: &dyn Atlas,
        words: &Phrasebook,
    ) {
        let Some(record) = self.open.get_mut(&id) else {
            return;
        };
        let kind = &self.rules[record.rule];
        let story = &mut record.story;
        story.events += 1;
        story.updated = event.frame;
        story.updated_clock = event.clock;
        story.latest = line.to_string();
        if event.zone.is_some() {
            story.zone = event.zone;
        }
        // The first event leads until a more dramatic one comes.
        if story.events == 1 || event.drama > story.drama {
            story.drama = event.drama;
            story.lead = event.id;
            record.lead_line = line.to_string();
        }
        story.timeline.push(StoryBeat {
            event: event.id,
            frame: event.frame,
            clock: event.clock,
            kind: event.kind.clone(),
            drama: event.drama,
            line: line.to_string(),
            zone: event.zone,
            actor: event.actor,
        });
        if story.timeline.len() > self.limits.timeline {
            // The first entry stays: it says how the story began.
            story.timeline.remove(1);
        }
        for resident in &facts.residents {
            // A story remembers more residents than it lists.
            if !record.residents.contains(resident)
                && record.residents.len() < self.limits.remembered
            {
                record.residents.push(*resident);
            }
            let name = words.resident(*resident, atlas);
            let listed = story.residents.iter().position(|listed| listed.id == *resident);
            match listed {
                Some(at) => story.residents[at].name = name,
                None if story.residents.len() < self.limits.residents => {
                    story.residents.push(StoryResident { id: *resident, name });
                }
                None => {}
            }
        }
        let headline =
            rules::headline(&kind.headline, &record.lead_line, &story.latest, story.events);
        story.headline = clean(&headline, words.limits().line_chars);
        record.public = story.events >= kind.min_events as u64;
        if record.public {
            self.dirty.insert(id);
        }
    }

    /// Closes a story.
    fn close(&mut self, id: u64) {
        let Some(mut record) = self.open.remove(&id) else {
            return;
        };
        if !record.public {
            // Viewers never saw it. It leaves without a word.
            return;
        }
        record.story.status = StoryStatus::Closed;
        self.dirty.insert(id);
        self.closed.push_back(record);
        while self.closed.len() > self.limits.max_closed {
            if let Some(gone) = self.closed.pop_front() {
                self.dirty.remove(&gone.story.id);
            }
        }
    }

    /// Ends a frame: stories without a new event for `close_frames` frames
    /// close.
    pub fn end_frame(&mut self, frame: u64) {
        self.frame = frame;
        let quiet: Vec<u64> = self
            .open
            .iter()
            .filter(|(_, record)| {
                frame.saturating_sub(record.story.updated) >= self.limits.close_frames
            })
            .map(|(id, _)| *id)
            .collect();
        for id in quiet {
            self.close(id);
        }
    }

    fn record(&self, id: u64) -> Option<&Record> {
        self.open.get(&id).or_else(|| self.closed.iter().find(|record| record.story.id == id))
    }

    /// Returns a story that viewers see.
    pub fn get(&self, id: u64) -> Option<&Story> {
        self.record(id).filter(|record| record.public).map(|record| &record.story)
    }

    /// Returns the word for the kind and the headline of a story that
    /// viewers see.
    pub fn words(&self, id: u64) -> Option<(String, String)> {
        self.get(id).map(|story| (story.label.clone(), story.headline.clone()))
    }

    /// Returns the stories that changed since the last call, at most
    /// `max`: the most dramatic first, then the oldest. The others wait
    /// for the next call.
    pub fn take_updates(&mut self, max: usize) -> Vec<Story> {
        let mut changed: Vec<&Story> = self.dirty.iter().filter_map(|id| self.get(*id)).collect();
        changed.sort_by(|a, b| b.drama.cmp(&a.drama).then(a.id.cmp(&b.id)));
        let updates: Vec<Story> = changed.into_iter().take(max).cloned().collect();
        if updates.len() < self.dirty.len() {
            for story in &updates {
                self.dirty.remove(&story.id);
            }
            // A story that viewers do not see has nothing to tell.
            let hidden: Vec<u64> =
                self.dirty.iter().copied().filter(|id| self.get(*id).is_none()).collect();
            for id in hidden {
                self.dirty.remove(&id);
            }
        } else {
            self.dirty.clear();
        }
        updates
    }

    /// Returns the stories that viewers see: open ones first, then by the
    /// frame of the newest event, newest first. `open_only` leaves the
    /// closed stories out.
    pub fn list(&self, limit: usize, open_only: bool) -> Vec<Story> {
        let closed = self.closed.iter().filter(|_| !open_only);
        let mut stories: Vec<&Story> = self
            .open
            .values()
            .chain(closed)
            .filter(|record| record.public)
            .map(|record| &record.story)
            .collect();
        stories.sort_by(|a, b| {
            let rank = |story: &Story| story.status == StoryStatus::Closed;
            rank(a).cmp(&rank(b)).then(b.updated.cmp(&a.updated)).then(b.id.cmp(&a.id))
        });
        stories.into_iter().take(limit).cloned().collect()
    }
}

/// Counts the residents that are in both lists.
fn shared(story: &[ResidentId], event: &[ResidentId]) -> usize {
    event.iter().filter(|resident| story.contains(resident)).count()
}

#[cfg(test)]
mod tests;
