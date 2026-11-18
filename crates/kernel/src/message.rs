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
use crate::realm::{Membership, Transfer};

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
/// The host of a world sees envelopes only when the world does not host
/// every realm: `World::take_external` returns the envelopes for realms in
/// other processes, and `World::deliver_external` takes the envelopes that
/// other processes sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// The realm that sent the message.
    pub from: RealmId,
    /// Sequence number in the sending realm.
    pub seq: u64,
    /// Where the message goes.
    pub to: Dest,
    pub(crate) body: Body,
}

impl Envelope {
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
        }
    }
}
