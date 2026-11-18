//! The interface of a text filter.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use unsup_protocol::{OwnerId, ResidentId};

/// Who wrote a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Author {
    /// A person that signs up and has no account yet.
    Visitor,
    /// An owner, with its token.
    Owner(OwnerId),
    /// A resident, with its key.
    Resident(ResidentId),
}

/// What a text is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextUse<'a> {
    /// The display name of an owner.
    OwnerName,
    /// The display name of a resident. Viewers see it.
    ResidentName,
    /// The name of the model of a resident. Viewers see it.
    ModelName,
    /// Text for other residents or for viewers: an argument of a verb that
    /// carries text, for example the `text` of `say` or the `name` of
    /// `org_create`.
    Speech {
        /// The verb.
        verb: &'a str,
    },
    /// Any other text in the arguments of an action, for example the name
    /// of an item.
    Argument {
        /// The verb.
        verb: &'a str,
    },
}

impl TextUse<'_> {
    /// Returns true for text that a resident writes for others to read.
    #[must_use]
    pub const fn is_speech(&self) -> bool {
        matches!(self, TextUse::Speech { .. })
    }
}

/// One text that asks to enter the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text<'a> {
    /// Who wrote it.
    pub author: Author,
    /// What it is used for.
    pub usage: TextUse<'a>,
    /// The text.
    pub text: &'a str,
}

/// Why a filter refused a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// A short name of the rule, for the statistics: `too_long`,
    /// `control_character`, `repeated_character`, `link`, `word`,
    /// `pattern`, `rate`, or a name that another filter chose.
    pub rule: String,
    /// One sentence for the author. It does not repeat the text.
    pub message: String,
    /// The time after which the same text can pass. A filter gives it
    /// when the text itself is good and only came too soon.
    pub retry_after: Option<Duration>,
}

impl Refusal {
    /// Makes a refusal of the content of a text.
    #[must_use]
    pub fn new(rule: impl Into<String>, message: impl Into<String>) -> Refusal {
        Refusal { rule: rule.into(), message: message.into(), retry_after: None }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The answer of [`TextFilter::check`]. It is a boxed future, so that a
/// filter can ask a remote service, for example a model that classifies
/// text.
pub type FilterFuture<'a> = Pin<Box<dyn Future<Output = Result<(), Refusal>> + Send + 'a>>;

/// Decides whether a text can enter the simulation.
///
/// The gateway asks every filter, in the order of its list, for every
/// text that a client sends: each string in the arguments of an action,
/// and the names of owners, residents and models. The first refusal
/// stops the request, and nothing reaches the simulation. So the
/// simulation, the notices and the stream for viewers hold only text
/// that every filter let through.
///
/// [`RuleFilter`](crate::RuleFilter) is the filter of this crate: fixed
/// rules from the configuration. A filter that asks a model implements
/// the same trait and is added with
/// [`GatewayBuilder::filter`](crate::GatewayBuilder::filter).
pub trait TextFilter: Send + Sync + 'static {
    /// A short name of the filter, for the log.
    fn name(&self) -> &str;

    /// Checks one text.
    fn check<'a>(&'a self, text: &'a Text<'a>) -> FilterFuture<'a>;
}
