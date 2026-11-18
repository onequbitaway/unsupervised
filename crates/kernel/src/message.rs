//! Messages between realms.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{
    ActionError, CityId, Event, ItemId, OrgId, OwnerId, Pos, RealmId, ResidentId, ZoneId,
};

use crate::codec::{self, DecodeError};
use crate::ids::{ParamId, VerbId};
use crate::ledger::Holder;
use crate::plugin::Message;
use crate::realm::{BorderNews, Membership, Transfer};

fn is_false(v: &bool) -> bool {
    !*v
}

/// Where a message goes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Dest {
    /// The global realm.
    Global,
    /// One zone.
    Zone(ZoneId),
    /// The zone in which a resident is when the message is routed.
    Resident(ResidentId),
    /// Every zone.
    AllZones,
}

/// A message in transit between two realms.
///
/// The host of a world sees envelopes in two cases. When the world does not
/// host every realm, `World::take_external` returns the envelopes for realms
/// in other processes, and `World::deliver_external` takes the envelopes
/// that other processes sent. When a service outside the simulation has
/// something to tell a module, it makes an envelope with
/// [`Envelope::module`] and gives it to `World::deliver_external`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// The realm that sent the message.
    pub from: RealmId,
    /// Sequence number in the sending realm.
    pub seq: u64,
    /// Where the message goes.
    pub to: Dest,
    pub(crate) body: Body,
    /// True when the envelope came into this world through
    /// `Input::Deliver`. A module message of such an envelope that no
    /// handler can take is dropped and counted. It is not written when it
    /// is false, so an envelope of this process has the bytes that it
    /// always had.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) foreign: bool,
}

impl Envelope {
    /// Makes an envelope with a module message, for a sender that is not a
    /// realm of this world: a service outside the simulation, or a tool.
    ///
    /// `from` is the realm that the handler sees as the sender. A service
    /// names the realm where its message enters the world, for example
    /// `RealmId::Global`. The sequence number is 0. Give the envelope to
    /// `World::deliver_external` (`Input::Deliver`); it is delivered in the
    /// next tick of its target.
    ///
    /// `Blueprint::handles_message` tells if a handler will take the
    /// message. A message that no handler takes is dropped and counted in
    /// `World::dropped_messages`.
    ///
    /// # Panics
    ///
    /// Panics when the `Serialize` implementation of the message fails, as
    /// `Realm::send` does. That is a bug in the type.
    #[must_use]
    pub fn module<M: Message>(from: RealmId, to: Dest, message: &M) -> Envelope {
        Envelope {
            from,
            seq: 0,
            to,
            body: Body::Module {
                kind: Cow::Borrowed(M::KIND),
                payload: codec::encode(M::KIND, message),
            },
            foreign: false,
        }
    }

    /// Makes an envelope of this process with a body.
    pub(crate) fn new(from: RealmId, seq: u64, to: Dest, body: Body) -> Envelope {
        Envelope { from, seq, to, body, foreign: false }
    }

    /// Returns the kind of the message: `Message::KIND` of a module message,
    /// or a name that starts with `kernel.` for a kernel message.
    #[must_use]
    pub fn kind(&self) -> &str {
        match &self.body {
            Body::Module { kind, .. } => kind,
            Body::Kernel(msg) => msg.kind(),
        }
    }

    /// Writes the envelope as MessagePack, for transport to another process.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        codec::encode("envelope", self)
    }

    /// Reads an envelope that [`Envelope::to_bytes`] wrote.
    pub fn from_bytes(bytes: &[u8]) -> Result<Envelope, DecodeError> {
        codec::decode("envelope", bytes)
    }
}

/// The content of an envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Body {
    /// A module message: the kind and the encoded payload.
    Module {
        kind: Cow<'static, str>,
        #[serde(with = "crate::codec::bytes")]
        payload: Vec<u8>,
    },
    /// A message of the kernel itself.
    Kernel(Box<KernelMsg>),
}

/// A request to run a global verb, sent from a zone to the global realm.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GlobalAct {
    pub(crate) resident: ResidentId,
    pub(crate) action_id: u64,
    pub(crate) verb: VerbId,
    pub(crate) request_id: Option<String>,
    pub(crate) args: Value,
    pub(crate) zone: ZoneId,
    pub(crate) pos: Pos,
    pub(crate) free: bool,
}

/// The answer to a [`GlobalAct`], sent to the zone of the resident.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ActionDone {
    pub(crate) resident: ResidentId,
    pub(crate) action_id: u64,
    pub(crate) verb: VerbId,
    pub(crate) request_id: Option<String>,
    pub(crate) free: bool,
    pub(crate) result: Result<Value, ActionError>,
}

/// A change of the resident directory in the global realm.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum DirUpdate {
    Spawned { id: ResidentId, owner: OwnerId, name: String, zone: ZoneId },
    Moved { id: ResidentId, zone: ZoneId },
    Despawned { id: ResidentId },
}

/// The messages that the kernel sends to itself across realms.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum KernelMsg {
    /// A resident that migrates, with everything it owns.
    Arrive(Box<Transfer>),
    /// A notice for a resident in another zone.
    Notice {
        to: ResidentId,
        kind: String,
        from: Option<ResidentId>,
        data: Value,
    },
    GlobalAct(GlobalAct),
    ActionDone(ActionDone),
    /// `Realm::complete_action` for a resident in another realm.
    Complete {
        resident: ResidentId,
        action_id: u64,
        result: Result<Value, ActionError>,
    },
    /// Money for a holder in the target realm.
    WireMoney {
        to: Holder,
        amount: i64,
    },
    /// Goods for a holder in the target realm.
    WireItem {
        to: Holder,
        item: ItemId,
        qty: i64,
    },
    /// A parameter changed. `value` is `None` when a city override was
    /// removed.
    ParamSet {
        param: ParamId,
        city: Option<CityId>,
        value: Option<i64>,
    },
    /// Zone events for the handlers of the global realm.
    Events(Vec<Event>),
    Directory(DirUpdate),
    /// A membership was granted (`Some`) or revoked (`None`).
    Membership {
        resident: ResidentId,
        org: OrgId,
        grant: Option<Membership>,
    },
    /// What changed among the residents of the sending zone that are near
    /// the border of the target zone.
    Border(Vec<BorderNews>),
}

impl KernelMsg {
    fn kind(&self) -> &'static str {
        match self {
            KernelMsg::Arrive(_) => "kernel.arrive",
            KernelMsg::Notice { .. } => "kernel.notice",
            KernelMsg::GlobalAct(_) => "kernel.global_act",
            KernelMsg::ActionDone(_) => "kernel.action_done",
            KernelMsg::Complete { .. } => "kernel.complete",
            KernelMsg::WireMoney { .. } => "kernel.wire_money",
            KernelMsg::WireItem { .. } => "kernel.wire_item",
            KernelMsg::ParamSet { .. } => "kernel.param_set",
            KernelMsg::Events(_) => "kernel.events",
            KernelMsg::Directory(_) => "kernel.directory",
            KernelMsg::Membership { .. } => "kernel.membership",
            KernelMsg::Border(_) => "kernel.border",
        }
    }
}
