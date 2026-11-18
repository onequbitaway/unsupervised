//! The per-resident state of the module: private labels, open proposals and
//! the meters of the rate limits.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Component, ResidentId};

/// Game minutes in one hour.
pub(crate) const HOUR: u64 = 60;
/// Game minutes in one day.
pub(crate) const DAY: u64 = unsup_kernel::TICKS_PER_DAY;

/// A counter for a rate limit: how many times something happened in the
/// current window. Windows are counted in frames, so a meter is right in
/// every zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Meter {
    window: u64,
    used: u32,
}

impl Meter {
    /// Returns true when one more use fits in the window that holds
    /// `frame`. `length` is the length of a window in frames.
    pub(crate) fn has_room(&self, frame: u64, length: u64, limit: i64) -> bool {
        let used = if self.window == frame / length { i64::from(self.used) } else { 0 };
        used < limit
    }

    /// Counts one use in the window that holds `frame`.
    pub(crate) fn count(&mut self, frame: u64, length: u64) {
        let window = frame / length;
        if self.window != window {
            self.window = window;
            self.used = 0;
        }
        self.used = self.used.saturating_add(1);
    }

    fn is_unused(&self) -> bool {
        *self == Meter::default()
    }
}

/// A private label that a resident gives to another resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Label {
    Friend,
    Rival,
    Ally,
    Enemy,
}

impl Label {
    /// Every label, in the order of the observation.
    pub(crate) const ALL: [Label; 4] = [Label::Friend, Label::Rival, Label::Ally, Label::Enemy];

    /// Returns the name of the label.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Label::Friend => "friend",
            Label::Rival => "rival",
            Label::Ally => "ally",
            Label::Enemy => "enemy",
        }
    }
}

/// What another resident asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Ask {
    /// To marry.
    Marriage,
    /// To join its household.
    Household,
}

impl Ask {
    /// Returns the name of the question.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Ask::Marriage => "marriage",
            Ask::Household => "household",
        }
    }
}

/// An open proposal or invitation that a resident received. It is a copy
/// for the observation; the global realm decides what is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Suit {
    pub(crate) from: ResidentId,
    pub(crate) ask: Ask,
    /// The last frame in which the proposal can be accepted.
    pub(crate) expires: u64,
}

/// The rate limits of a resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Say,
    Message,
    OrgSay,
    Post,
    Opinion,
}

impl Channel {
    /// Returns the length of one window of the limit, in frames.
    pub(crate) fn window(self) -> u64 {
        match self {
            Channel::Say | Channel::Message | Channel::OrgSay => HOUR,
            Channel::Post | Channel::Opinion => DAY,
        }
    }

    /// Returns the name of the window for a message to the agent.
    pub(crate) fn window_name(self) -> &'static str {
        if self.window() == HOUR { "hour" } else { "day" }
    }
}

/// The state behind [`Social`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SocialState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) labels: Vec<(ResidentId, Label)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) suits: Vec<Suit>,
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) says: Meter,
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) messages: Meter,
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) org_says: Meter,
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) posts: Meter,
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) opinions: Meter,
    /// Kept contracts that raised the standing of the resident today.
    #[serde(default, skip_serializing_if = "Meter::is_unused")]
    pub(crate) rewards: Meter,
    /// The residents that this resident gave an opinion about, with the day
    /// of the opinion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) opined: Vec<(ResidentId, u64)>,
}

impl SocialState {
    /// Returns the counter of a rate limit.
    pub(crate) fn meter(&self, channel: Channel) -> Meter {
        match channel {
            Channel::Say => self.says,
            Channel::Message => self.messages,
            Channel::OrgSay => self.org_says,
            Channel::Post => self.posts,
            Channel::Opinion => self.opinions,
        }
    }

    /// Returns the counter of a rate limit for a change.
    pub(crate) fn meter_mut(&mut self, channel: Channel) -> &mut Meter {
        match channel {
            Channel::Say => &mut self.says,
            Channel::Message => &mut self.messages,
            Channel::OrgSay => &mut self.org_says,
            Channel::Post => &mut self.posts,
            Channel::Opinion => &mut self.opinions,
        }
    }
}

/// The state of the society module for one resident. It moves with the
/// resident between zones.
///
/// Most residents never speak, so the state is behind a pointer: a resident
/// that did nothing costs eight bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Social(Option<Box<SocialState>>);

impl Component for Social {
    const NAME: &'static str = "society.social";
}

impl Social {
    /// Returns the state when the resident has one.
    pub(crate) fn get(&self) -> Option<&SocialState> {
        self.0.as_deref()
    }

    /// Returns the state for a change. It makes an empty one first when the
    /// resident has none.
    pub(crate) fn make(&mut self) -> &mut SocialState {
        self.0.get_or_insert_with(Box::default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_counts_per_window() {
        let mut meter = Meter::default();
        assert!(meter.has_room(125, HOUR, 2));
        meter.count(125, HOUR);
        meter.count(130, HOUR);
        assert!(!meter.has_room(179, HOUR, 2));
        assert!(meter.has_room(179, HOUR, 3));
        // Frame 180 starts the next hour.
        assert!(meter.has_room(180, HOUR, 2));
        meter.count(180, HOUR);
        assert!(meter.has_room(181, HOUR, 2));
        assert!(!meter.has_room(181, HOUR, 1));
        assert!(!Meter::default().has_room(0, HOUR, 0));
    }

    #[test]
    fn a_resident_without_state_costs_one_pointer() {
        assert_eq!(size_of::<Social>(), size_of::<usize>());
        let mut social = Social::default();
        assert!(social.get().is_none());
        social.make().labels.push((ResidentId(7), Label::Friend));
        assert_eq!(social.get().map(|s| s.labels.len()), Some(1));
    }
}
