//! The director: it chooses what the broadcast shows.
//!
//! The director reads every public event. It scores the subjects of the
//! events (residents, sites, districts) by the drama of their events, by
//! how many events gather around them and by how fresh the events are. It
//! then chooses a shot: a subject, a zoom level and a caption.
//!
//! The rules, with the weights of `data/director.toml`:
//!
//! - A shot stays for a least time (`min_hold_frames`).
//! - Breaking news (`breaking_drama`) cuts in at once.
//! - After the least time, a subject with a clearly higher score takes the
//!   shot (`switch_percent`), and a shot whose subject went quiet ends
//!   (`keep_score`). No shot stays longer than `max_hold_frames` when
//!   another subject can take it.
//! - A subject that just left the air has a lower score for some time, so
//!   that the broadcast does not stay on one thing (`variety_frames`). The
//!   same holds for the kind of event of a shot that just started: after a
//!   death, the next shot is rather a theft than one more death. Breaking
//!   news of one kind does not cut in again and again
//!   (`breaking_kind_frames`).
//! - When no subject deserves a shot, the director shows a wide view: the
//!   country, then each city in turn.
//!
//! The director has no clock and no randomness. The same events in the
//! same frames give the same shots.

mod score;
mod shot;

use std::collections::BTreeMap;

use unsup_protocol::{Clock, ZoneId};

pub(crate) use self::score::Subject;
use self::score::{Beat, Track, after_penalty};
use crate::config::DirectorConfig;
use crate::facts::{Atlas, Facts};
use crate::wire::{FeedEvent, Focus, Shot, Zoom};

/// One event of a frame with what the service read from it.
pub(crate) struct Cue<'a> {
    /// The event.
    pub event: &'a std::sync::Arc<FeedEvent>,
    /// Who and what the event is about.
    pub facts: &'a Facts,
    /// One line about the event.
    pub line: &'a str,
    /// The story of the event, if it is part of one.
    pub story: Option<u64>,
}

/// The words of a story for a shot: the word for its kind and its
/// headline.
pub(crate) type StoryWords = (String, String);

/// The director. See the module documentation.
#[derive(Debug)]
pub(crate) struct Director {
    config: DirectorConfig,
    /// The subjects with events that still count.
    tracks: BTreeMap<Subject, Track>,
    /// The subject on air. `None`: a wide view.
    on_air: Option<Subject>,
    shot: Shot,
    /// The frame at which each subject left the air.
    aired: BTreeMap<Subject, u64>,
    /// The district that left the air last, and the frame.
    aired_zone: Option<(ZoneId, u64)>,
    /// Number of wide views so far. It chooses the next one.
    wide_turns: u64,
    /// The frame at which a shot of each kind of event last started.
    kinds: BTreeMap<String, u64>,
    /// The frame and the drama score of the last breaking news of each
    /// kind of event.
    news: BTreeMap<String, (u64, u8)>,
}

impl Director {
    /// Makes a director. Until the first frame the shot is an empty wide
    /// view with the number 0.
    pub fn new(config: &DirectorConfig) -> Director {
        Director {
            config: config.clone(),
            tracks: BTreeMap::new(),
            on_air: None,
            shot: Shot {
                seq: 0,
                frame: 0,
                clock: Clock::default(),
                zone: None,
                city: None,
                pos: None,
                focus: Focus::None,
                zoom: Zoom::Country,
                caption: String::new(),
                breaking: false,
                event: None,
                kind: None,
                drama: 0,
                story: None,
                label: None,
                headline: None,
            },
            aired: BTreeMap::new(),
            aired_zone: None,
            wide_turns: 0,
            kinds: BTreeMap::new(),
            news: BTreeMap::new(),
        }
    }

    /// Returns the shot on air.
    pub fn shot(&self) -> &Shot {
        &self.shot
    }

    /// Takes the events of one frame and chooses the shot. Returns the
    /// shot when it changed: a cut (a new number) or new words (the same
    /// number).
    ///
    /// `story_words` gives the label and the headline of a story now.
    pub fn frame(
        &mut self,
        frame: u64,
        clock: Clock,
        cues: &[Cue<'_>],
        atlas: &dyn Atlas,
        story_words: &dyn Fn(u64) -> Option<StoryWords>,
    ) -> Option<Shot> {
        let before = self.shot.clone();
        self.take(cues, atlas);
        self.prune(frame);
        if !self.breaking_news(frame, clock, cues, atlas) {
            self.choose(frame, clock, atlas);
        }
        self.refresh_words(frame, story_words);
        (self.shot != before).then(|| self.shot.clone())
    }

    /// Adds the events of a frame to the tracks of their subjects.
    fn take(&mut self, cues: &[Cue<'_>], atlas: &dyn Atlas) {
        for cue in cues {
            if cue.event.drama < self.config.min_drama {
                continue;
            }
            for (subject, share) in self.subjects_of(cue, atlas) {
                self.tracks.entry(subject).or_default().beats.push_back(Beat {
                    event: std::sync::Arc::clone(cue.event),
                    line: cue.line.to_string(),
                    story: cue.story,
                    share,
                });
            }
        }
    }

    /// Returns the subjects of an event with the share of each: the first
    /// resident, the other residents, the site and the district.
    fn subjects_of(&self, cue: &Cue<'_>, atlas: &dyn Atlas) -> Vec<(Subject, u64)> {
        let mut subjects = Vec::new();
        for (at, resident) in cue.facts.residents.iter().enumerate() {
            let share = if at == 0 { 100 } else { self.config.party_share_percent };
            subjects.push((Subject::Resident(*resident), share));
        }
        let site = cue.facts.site.or_else(|| {
            let zone = cue.event.zone?;
            atlas.site_at(zone, cue.event.pos?).map(|site| site.id)
        });
        if let Some(site) = site {
            subjects.push((Subject::Place(site), self.config.place_share_percent));
        }
        if let Some(zone) = cue.event.zone {
            subjects.push((Subject::Zone(zone), self.config.zone_share_percent));
        }
        subjects
    }

    /// Removes what counts no more, and keeps the number of subjects below
    /// the limit.
    fn prune(&mut self, frame: u64) {
        let fresh = self.config.fresh_frames;
        self.tracks.retain(|_, track| !track.prune(frame, fresh));
        while self.tracks.len() > self.config.max_subjects {
            let oldest = self
                .tracks
                .iter()
                .filter(|(subject, _)| Some(**subject) != self.on_air)
                .min_by_key(|(subject, track)| (track.newest(), **subject))
                .map(|(subject, _)| *subject);
            match oldest {
                Some(subject) => self.tracks.remove(&subject),
                None => break,
            };
        }
        let variety = self.config.variety_frames;
        self.aired.retain(|_, left| frame.saturating_sub(*left) < variety);
        if self.aired_zone.is_some_and(|(_, left)| frame.saturating_sub(left) >= variety) {
            self.aired_zone = None;
        }
        self.kinds.retain(|_, started| frame.saturating_sub(*started) < variety);
        let tired = self.config.breaking_kind_frames;
        self.news.retain(|_, (cut, _)| frame.saturating_sub(*cut) < tired);
    }

    /// Returns true when an event must not cut in as breaking news: news
    /// of its kind cut in a short time ago, and this event is not more
    /// dramatic.
    fn old_news(&self, event: &FeedEvent, frame: u64) -> bool {
        self.news.get(&event.kind).is_some_and(|(cut, drama)| {
            frame.saturating_sub(*cut) < self.config.breaking_kind_frames && event.drama <= *drama
        })
    }

    /// Cuts to the most dramatic breaking news of the frame, if there is
    /// one that may take the shot. Returns true when it cut.
    fn breaking_news(
        &mut self,
        frame: u64,
        clock: Clock,
        cues: &[Cue<'_>],
        atlas: &dyn Atlas,
    ) -> bool {
        let mut news: Option<&Cue<'_>> = None;
        for cue in cues {
            if cue.event.drama >= self.config.breaking_drama
                && !self.old_news(cue.event, frame)
                && news.is_none_or(|best| cue.event.drama > best.event.drama)
            {
                news = Some(cue);
            }
        }
        let Some(cue) = news else {
            return false;
        };
        let held = frame.saturating_sub(self.shot.frame);
        let holds = self.shot.breaking
            && held < self.config.breaking_hold_frames
            && self.shot.drama >= cue.event.drama;
        if holds {
            return false;
        }
        let subject = self.subjects_of(cue, atlas).first().map(|(subject, _)| *subject);
        let beat = Beat {
            event: std::sync::Arc::clone(cue.event),
            line: cue.line.to_string(),
            story: cue.story,
            share: 100,
        };
        let shot = shot::of_beat(subject, &beat, cue.facts.city, atlas, &self.config);
        self.cut(frame, clock, subject, Shot { breaking: true, ..shot });
        true
    }

    /// Returns the score of a subject for the choice of the next shot:
    /// its score, less the loss for variety.
    fn bid(&self, subject: Subject, track: &Track, frame: u64) -> u64 {
        let score = track.score(frame, &self.config);
        let variety = self.config.variety_frames;
        let score = if let Some(left) = self.aired.get(&subject) {
            let since = frame.saturating_sub(*left);
            after_penalty(score, self.config.repeat_penalty_percent, since, variety)
        } else {
            match self.aired_zone {
                Some((zone, left)) if track.zone() == Some(zone) => {
                    let since = frame.saturating_sub(left);
                    after_penalty(score, self.config.zone_penalty_percent, since, variety)
                }
                _ => score,
            }
        };
        // The kind of the event that would be the caption.
        let started = track
            .lead(frame, self.config.fresh_frames)
            .and_then(|lead| self.kinds.get(&lead.event.kind));
        match started {
            Some(started) => {
                let since = frame.saturating_sub(*started);
                after_penalty(score, self.config.kind_penalty_percent, since, variety)
            }
            None => score,
        }
    }

    /// Returns the subject that is not on air and has the highest bid, when
    /// that bid is enough for a shot. Of two with the same bid, the first
    /// in the order of [`Subject`] wins.
    fn challenger(&self, frame: u64) -> Option<(Subject, u64)> {
        let mut best: Option<(Subject, u64)> = None;
        for (subject, track) in &self.tracks {
            if Some(*subject) == self.on_air {
                continue;
            }
            let bid = self.bid(*subject, track, frame);
            if bid >= self.config.cut_score && best.is_none_or(|(_, top)| bid > top) {
                best = Some((*subject, bid));
            }
        }
        best
    }

    /// Applies the rules of the shot: hold, switch, end, wide view.
    fn choose(&mut self, frame: u64, clock: Clock, atlas: &dyn Atlas) {
        let held = frame.saturating_sub(self.shot.frame);
        let config = &self.config;
        let Some(subject) = self.on_air else {
            // A wide view, or no shot yet. Breaking news without a place
            // stays as long as any other shot.
            let least =
                if self.shot.breaking { config.min_hold_frames } else { config.wide_min_frames };
            let ready = self.shot.seq == 0 || held >= least;
            match self.challenger(frame) {
                Some((next, _)) if ready => self.cut_to(frame, clock, next, atlas),
                _ if self.shot.seq == 0 || held >= config.wide_hold_frames => {
                    self.cut_wide(frame, clock, atlas);
                }
                _ => {}
            }
            return;
        };
        if held < config.min_hold_frames {
            return;
        }
        let own = self.tracks.get(&subject).map_or(0, |track| track.score(frame, config));
        let challenger = self.challenger(frame);
        let quiet = own < config.keep_score;
        let long = held >= config.max_hold_frames;
        match challenger {
            Some((next, bid))
                if quiet || long || bid.saturating_mul(100) >= own * config.switch_percent =>
            {
                self.cut_to(frame, clock, next, atlas);
            }
            None if quiet => self.cut_wide(frame, clock, atlas),
            _ => {}
        }
    }

    /// Cuts to a subject with the event that speaks for it.
    fn cut_to(&mut self, frame: u64, clock: Clock, subject: Subject, atlas: &dyn Atlas) {
        let fresh = self.config.fresh_frames;
        let Some(beat) = self.tracks.get(&subject).and_then(|track| track.lead(frame, fresh))
        else {
            return;
        };
        let shot = shot::of_beat(Some(subject), beat, None, atlas, &self.config);
        self.cut(frame, clock, Some(subject), shot);
    }

    /// Cuts to the next wide view.
    fn cut_wide(&mut self, frame: u64, clock: Clock, atlas: &dyn Atlas) {
        let shot = shot::wide(self.wide_turns, atlas);
        self.wide_turns += 1;
        self.cut(frame, clock, None, shot);
    }

    /// Puts a new shot on air.
    fn cut(&mut self, frame: u64, clock: Clock, subject: Option<Subject>, shot: Shot) {
        if let Some(old) = self.on_air
            && Some(old) != subject
        {
            self.aired.insert(old, frame);
            if let Some(zone) = self.shot.zone {
                self.aired_zone = Some((zone, frame));
            }
        }
        if let Some(subject) = subject {
            self.aired.remove(&subject);
        }
        if let Some(kind) = &shot.kind {
            self.kinds.insert(kind.clone(), frame);
            if shot.breaking {
                self.news.insert(kind.clone(), (frame, shot.drama));
            }
        }
        self.on_air = subject;
        self.shot = Shot { seq: self.shot.seq + 1, frame, clock, ..shot };
    }

    /// Gives the shot on air the words of now: the line of a new event of
    /// its subject that weighs more than the event of the caption, and the
    /// headline of its story.
    fn refresh_words(&mut self, frame: u64, story_words: &dyn Fn(u64) -> Option<StoryWords>) {
        let fresh = self.config.fresh_frames;
        if let Some(subject) = self.on_air
            && let Some(lead) = self.tracks.get(&subject).and_then(|track| track.lead(frame, fresh))
            && lead.event.frame == frame
            && Some(lead.event.id) != self.shot.event
            && self.shot.frame != frame
        {
            self.shot.caption.clone_from(&lead.line);
            self.shot.event = Some(lead.event.id);
            self.shot.kind = Some(lead.event.kind.clone());
            self.shot.drama = lead.event.drama;
            self.shot.story = lead.story;
        }
        let words = self.shot.story.and_then(story_words);
        let (label, headline) = match words {
            Some((label, headline)) => (Some(label), Some(headline)),
            None => (None, None),
        };
        self.shot.label = label;
        self.shot.headline = headline;
    }
}

#[cfg(test)]
mod tests;
