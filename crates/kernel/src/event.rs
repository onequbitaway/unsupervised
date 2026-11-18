//! Events: who hears about a fact.

use unsup_protocol::{Pos, ResidentId};

/// The description of an event, given to `Realm::emit` with the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventMeta {
    /// Kind of the event: `<module>.<what_happened>`, for example
    /// `crime.theft_committed`.
    pub kind: &'static str,
    /// The resident that caused the event, if any.
    pub actor: Option<ResidentId>,
    /// Where the event happened, if it has a place.
    pub pos: Option<Pos>,
    /// Who hears about the event.
    pub audience: Audience,
    /// How interesting the event is for viewers, 0 to 100. The director uses
    /// it. Values above 100 are read as 100.
    pub drama: u8,
}

impl EventMeta {
    /// Makes the description of an event that only handlers see.
    #[must_use]
    pub fn silent(kind: &'static str) -> EventMeta {
        EventMeta { kind, actor: None, pos: None, audience: Audience::Silent, drama: 0 }
    }

    /// Makes the description of an event that viewers see.
    #[must_use]
    pub fn public(kind: &'static str, drama: u8) -> EventMeta {
        EventMeta { kind, actor: None, pos: None, audience: Audience::Public, drama }
    }

    /// Sets the resident that caused the event.
    #[must_use]
    pub fn actor(mut self, actor: ResidentId) -> EventMeta {
        self.actor = Some(actor);
        self
    }

    /// Sets the place of the event.
    #[must_use]
    pub fn at(mut self, pos: Pos) -> EventMeta {
        self.pos = Some(pos);
        self
    }

    /// Sets who hears about the event.
    #[must_use]
    pub fn audience(mut self, audience: Audience) -> EventMeta {
        self.audience = audience;
        self
    }
}

/// Who hears about an event. Event handlers always see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Viewers see it. No resident gets a notice.
    Public,
    /// Viewers see it. Every resident of the zone within this many tiles of
    /// the place of the event gets a notice, except the actor. The place is
    /// `pos`, or the position of the actor when `pos` is absent.
    Nearby(u32),
    /// Only these residents get a notice, in whatever zone they are. Viewers
    /// do not see it.
    Private(Vec<ResidentId>),
    /// Only event handlers see it.
    Silent,
}
