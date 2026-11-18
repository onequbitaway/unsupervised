//! The studio: the parts of the show that read every event.
//!
//! The director, the stories, the chronicle and the recorder of the clips
//! all get the same events in the same order, after the delay of the
//! stream. The studio reads each event once (its residents, its place,
//! its line) and gives it to each part that wants it.

use std::sync::Arc;

use unsup_protocol::{Clock, ResidentView, ZoneId};

use crate::chronicle::{Chapters, Chronicle};
use crate::clips::Recorder;
use crate::config::Config;
use crate::digest::Digest;
use crate::director::{Cue, Director};
use crate::facts::{Atlas, Facts, Phrasebook, Reader};
use crate::stories::Stories;
use crate::vault::{Restored, Work};
use crate::wire::{DirectorState, FeedEvent, Shot, Story};

/// What the show has for viewers after one frame.
#[derive(Debug, Default)]
pub(crate) struct ShowBeat {
    /// The shot of the director, when it changed.
    pub shot: Option<Shot>,
    /// The stories that changed.
    pub stories: Vec<Story>,
}

/// An event of a frame with what the studio read from it.
struct Read {
    event: Arc<FeedEvent>,
    facts: Facts,
    line: String,
    story: Option<u64>,
}

/// The studio. See the module documentation.
#[derive(Debug)]
pub(crate) struct Studio {
    reader: Reader,
    words: Phrasebook,
    /// The director.
    pub director: Director,
    /// The stories.
    pub stories: Stories,
    /// The chronicle.
    pub chronicle: Chronicle,
    /// The recorder of the clips.
    pub recorder: Recorder,
    /// Lowest drama score of an event that the director counts.
    director_drama: u8,
    /// Lowest drama score of an event that the chronicle keeps.
    chronicle_drama: u8,
    /// Lowest drama score of an event whose clip is pinned without a
    /// request.
    pin_drama: u8,
    chapters: Chapters,
    updates_per_frame: usize,
    /// The last frame that the studio saw.
    frame: u64,
}

impl Studio {
    /// Makes the studio. The configuration must be valid
    /// ([`Config::validate`]).
    pub fn new(config: &Config) -> Studio {
        // `Config::validate` read every template. A configuration that
        // was not checked gets the names of the kinds as words.
        let words = Phrasebook::new(&config.phrases, &config.facts).unwrap_or_else(|error| {
            tracing::error!(%error, "the phrases are not valid; events get their kind as words");
            Phrasebook::plain(&config.facts)
        });
        let chapters = Chapters::new(&config.chapters);
        Studio {
            reader: Reader::new(&config.facts),
            words,
            director: Director::new(&config.director),
            stories: Stories::new(&config.story_kinds, &config.stories),
            chronicle: Chronicle::new(&config.chronicle, chapters.clone()),
            recorder: Recorder::new(&config.clips),
            director_drama: config.director.min_drama,
            chronicle_drama: config.chronicle.min_drama,
            pin_drama: config.clips.auto_pin_drama,
            chapters,
            updates_per_frame: config.stories.updates_per_frame,
            frame: 0,
        }
    }

    /// Returns true when a part of the show wants the event. Most events
    /// are routine (a shift, a meal): nobody wants them, and the studio
    /// does not read them.
    fn wanted(&self, event: &FeedEvent) -> bool {
        event.drama >= self.director_drama.min(self.chronicle_drama).min(self.pin_drama)
            || self.chapters.of(&event.kind).is_some()
            || self.stories.wants(event)
    }

    /// Takes the public events of one frame, in the order of the feed, and
    /// returns what the show has for viewers.
    pub fn frame(
        &mut self,
        frame: u64,
        clock: Clock,
        events: &[Arc<FeedEvent>],
        atlas: &dyn Atlas,
    ) -> ShowBeat {
        self.frame = frame;
        let mut reads: Vec<Read> = Vec::new();
        for event in events {
            if !self.wanted(event) {
                continue;
            }
            let facts = self.reader.facts(event);
            let line = self.words.line(event, atlas);
            let site = facts.site.or_else(|| {
                let zone = event.zone?;
                atlas.site_at(zone, event.pos?).map(|site| site.id)
            });
            let story = self.stories.take(event, &facts, &line, atlas, &self.words);
            self.chronicle.take(event, &facts, &line, site);
            self.recorder.note(event, &line);
            reads.push(Read { event: Arc::clone(event), facts, line, story });
        }
        self.stories.end_frame(frame);
        let cues: Vec<Cue<'_>> = reads
            .iter()
            .map(|read| Cue {
                event: &read.event,
                facts: &read.facts,
                line: &read.line,
                story: read.story,
            })
            .collect();
        let stories = &self.stories;
        let shot = self.director.frame(frame, clock, &cues, atlas, &|id| stories.words(id));
        ShowBeat { shot, stories: self.stories.take_updates(self.updates_per_frame) }
    }

    /// Gives the recorder the frame that viewers get.
    pub fn record(&mut self, digest: Arc<Digest>) {
        self.recorder.push(digest);
    }

    /// The cache read the whole world again: the recorder starts again
    /// from the residents of now.
    pub fn reset(&mut self, zones: impl Iterator<Item = (ZoneId, Vec<ResidentView>)>) {
        self.recorder.reset(zones);
    }

    /// Puts what the storage had into the studio. `max_frame` is the frame
    /// of the world at the start: later data is from a time that the world
    /// does not have.
    pub fn restore(&mut self, restored: Restored, max_frame: u64) {
        let kept = self.chronicle.restore(restored.records, max_frame);
        if kept > 0 {
            tracing::info!(records = kept, "the chronicle was read from the storage");
        }
        self.recorder.restore(restored.clips, max_frame);
    }

    /// Returns what the storage does not have yet. With `rewrite`, the
    /// work has everything that the chronicle holds, for a new journal.
    pub fn take_work(&mut self, rewrite: bool) -> Work {
        let records = self.chronicle.take_unwritten();
        let (pins, unpins) = self.recorder.take_unwritten();
        Work { records, rewrite: rewrite.then(|| self.chronicle.records()), pins, unpins }
    }

    /// Returns the answer of `/v1/director`.
    pub fn director_state(&self) -> DirectorState {
        DirectorState { frame: self.frame, shot: self.director.shot().clone() }
    }

    /// Writes one line for an event.
    pub fn line(&self, event: &FeedEvent, atlas: &dyn Atlas) -> String {
        self.words.line(event, atlas)
    }
}
