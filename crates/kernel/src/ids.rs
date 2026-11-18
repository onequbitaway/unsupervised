//! Kernel ids.
//!
//! The ids that cross the wire (`ResidentId`, `ZoneId`, `ItemId`, ...) live in
//! `unsup_protocol`. The ids here are used only inside the simulation.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The dense index of a resident in the resident table of its zone.
///
/// An `Idx` is valid only during the current tick. The kernel removes
/// residents (migration, despawn) at the end of the tick and fills the gap
/// with the last row, so an index can name another resident in the next tick.
/// State must store [`ResidentId`](unsup_protocol::ResidentId), never `Idx`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Idx(pub u32);

impl Idx {
    /// Returns the index as a `usize`, for slices.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Idx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

macro_rules! def_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
        )]
        pub struct $name(pub u16);

        impl $name {
            /// Returns the id as a `usize`, for tables.
            #[inline]
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

def_id!(
    /// Id of a parameter. It is stable for the life of a world.
    ParamId
);
def_id!(
    /// Id of an attribute. It is stable for the life of a world.
    AttrId
);
def_id!(
    /// Id of a condition. It is stable for the life of a world.
    CondId
);
def_id!(
    /// Id of a faucet: a named source of money or goods.
    FaucetId
);
def_id!(
    /// Id of a sink: a named drain of money or goods.
    SinkId
);
def_id!(
    /// Id of a verb. It is stable for the life of a world.
    VerbId
);
