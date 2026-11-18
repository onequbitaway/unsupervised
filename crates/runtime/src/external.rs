//! Messages from a service outside the simulation.
//!
//! A service such as the gateway is not a realm, but it can have something
//! to tell the world: a human sponsor posts a mission. The service sends a
//! typed message through [`SimHandle::post_global`]. The runtime puts the
//! message in an envelope and gives it to the world as `Input::Deliver`, so
//! the message is in the input log and a replay delivers it again.
//!
//! The kernel makes the envelope (`Envelope::module`) and tells which kinds
//! of message a module handles (`Blueprint::handles_message`). The runtime
//! refuses a message that nobody would take before it is in the input log.
//!
//! [`SimHandle::post_global`]: crate::SimHandle::post_global

use std::fmt;

use unsup_kernel::{Blueprint, Dest, Envelope, Message, Scope};
use unsup_protocol::{RealmId, SimError};

/// A message from outside did not get into the world. Nothing changed and
/// nothing is in the input log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostError {
    /// The message cannot be written as MessagePack, or it does not read
    /// back as what was written. A handler could not read it.
    Unreadable {
        /// `Message::KIND` of the message.
        kind: String,
        /// What went wrong.
        reason: String,
    },
    /// No module handles messages of this kind in the global realm.
    NoHandler {
        /// `Message::KIND` of the message.
        kind: String,
    },
    /// The simulation did not take the message: the global realm is in
    /// another process, the mailbox is full, the journal failed or the
    /// simulation stopped.
    Sim(SimError),
}

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostError::Unreadable { kind, reason } => {
                write!(f, "the message `{kind}` cannot be sent: {reason}")
            }
            PostError::NoHandler { kind } => write!(
                f,
                "no module handles the message kind `{kind}` in the global realm; \
                 a module registers a handler with `Builder::on_message`"
            ),
            PostError::Sim(error) => write!(f, "the simulation did not take the message: {error}"),
        }
    }
}

impl std::error::Error for PostError {}

impl From<SimError> for PostError {
    fn from(error: SimError) -> PostError {
        PostError::Sim(error)
    }
}

impl From<PostError> for SimError {
    /// A message that cannot be read and a kind that nobody handles are
    /// both `SimError::Invalid`.
    fn from(error: PostError) -> SimError {
        match error {
            PostError::Unreadable { .. } | PostError::NoHandler { .. } => SimError::Invalid,
            PostError::Sim(error) => error,
        }
    }
}

/// Puts a message for the global realm into an envelope.
///
/// The message has no sending realm. The envelope names the global realm
/// as its sender, because that is where the message enters the world.
pub(crate) fn envelope_for_global<M: Message>(message: &M) -> Result<Envelope, PostError> {
    let unreadable = |reason: String| PostError::Unreadable { kind: M::KIND.to_string(), reason };
    // `Envelope::module` panics on a type that cannot be written. A
    // service must get an error instead, so the type is tried first.
    let payload = rmp_serde::to_vec_named(message)
        .map_err(|e| unreadable(format!("it cannot be written as MessagePack: {e}")))?;
    // A type that does not read back what it wrote must stay outside: the
    // kernel would drop the message in the global realm, after it is in
    // the input log.
    rmp_serde::from_slice::<M>(&payload)
        .map_err(|e| unreadable(format!("it does not read back from MessagePack: {e}")))?;
    Ok(Envelope::module(RealmId::Global, Dest::Global, message))
}

/// Returns the kind of the first message that no handler of the global
/// realm takes.
pub(crate) fn unhandled(blueprint: &Blueprint, messages: &[Envelope]) -> Option<String> {
    messages
        .iter()
        .find(|envelope| !blueprint.handles_message(envelope.kind(), Scope::Global))
        .map(|envelope| envelope.kind().to_string())
}
