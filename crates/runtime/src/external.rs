//! Messages from a service outside the simulation.
//!
//! A service such as the gateway is not a realm, but it can have something
//! to tell the world: a human sponsor posts a mission. The service sends a
//! typed message through [`SimHandle::send_global`]. The runtime puts the
//! message in an envelope and gives it to the world as `Input::Deliver`, so
//! the message is in the input log and a replay delivers it again.
//!
//! The kernel makes envelopes only inside a realm. From outside, the one
//! public door is `Envelope::from_bytes`, which reads the transport format
//! of an envelope (MessagePack with field names). This module writes that
//! format.
//!
//! [`SimHandle::send_global`]: crate::SimHandle::send_global

use serde::{Serialize, Serializer};
use unsup_kernel::{Dest, Envelope, Message};
use unsup_protocol::RealmId;

/// A byte string. It is written as a MessagePack binary, like the payload
/// of an envelope in the kernel.
struct Binary<'a>(&'a [u8]);

impl Serialize for Binary<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

/// The content of an envelope with a module message, in the transport
/// format of the kernel.
#[derive(Serialize)]
enum Body<'a> {
    Module { kind: &'a str, payload: Binary<'a> },
}

/// An envelope in the transport format of the kernel.
#[derive(Serialize)]
struct Wire<'a> {
    from: RealmId,
    seq: u64,
    to: Dest,
    body: Body<'a>,
}

/// Puts a message for the global realm into an envelope.
///
/// The message has no sending realm. The envelope names the global realm
/// as its sender, because that is where the message enters the world.
///
/// The error is text for the log. It says why the message cannot be sent:
/// the type cannot be written as MessagePack, or it cannot be read back.
pub(crate) fn envelope_for_global<M: Message>(message: &M) -> Result<Envelope, String> {
    let payload = rmp_serde::to_vec_named(message)
        .map_err(|e| format!("the message cannot be written as MessagePack: {e}"))?;
    // The handler in the global realm panics on a payload that it cannot
    // read. A type that does not read back what it wrote must stay outside.
    rmp_serde::from_slice::<M>(&payload)
        .map_err(|e| format!("the message does not read back from MessagePack: {e}"))?;
    let wire = Wire {
        from: RealmId::Global,
        seq: 0,
        to: Dest::Global,
        body: Body::Module { kind: M::KIND, payload: Binary(&payload) },
    };
    let bytes = rmp_serde::to_vec_named(&wire)
        .map_err(|e| format!("the envelope cannot be written as MessagePack: {e}"))?;
    let envelope = Envelope::from_bytes(&bytes).map_err(|e| e.to_string())?;
    // A kernel that changed its transport format must not get a message
    // that means something else.
    if envelope.kind() != M::KIND || envelope.to != Dest::Global {
        return Err("the kernel read the envelope as another message".into());
    }
    Ok(envelope)
}
