//! Routines: standing orders that run when the resident is idle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::action::ActionRequest;

/// A standing order. When the resident is idle and `when` holds, the server
/// starts `then`. Routines are checked in list order; the first that holds
/// wins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Routine {
    /// Label chosen by the owner. It has no effect.
    pub name: String,
    /// When the routine fires.
    pub when: Condition,
    /// The action to start. Its `id` is ignored.
    pub then: ActionRequest,
}

/// A test on the state of a resident. Conditions are data, not code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// Always true.
    Always,
    /// True when the attribute is less than `value`.
    AttrBelow {
        /// Name of the attribute.
        attr: String,
        /// Threshold.
        value: i64,
    },
    /// True when the attribute is greater than `value`.
    AttrAbove {
        /// Name of the attribute.
        attr: String,
        /// Threshold.
        value: i64,
    },
    /// True when the hour of the day is at or after `from_hour` and before
    /// `to_hour`. The range wraps past midnight when `from_hour` is greater
    /// than `to_hour`. It is never true when the two are equal.
    ClockBetween {
        /// First hour of the range, 0 to 23.
        from_hour: u8,
        /// First hour after the range, 0 to 24.
        to_hour: u8,
    },
    /// True when the resident carries at least `qty` of the item.
    HasItem {
        /// Name of the item.
        item: String,
        /// Least quantity.
        qty: i64,
    },
    /// True when every condition in `of` is true.
    All {
        /// The conditions.
        of: Vec<Condition>,
    },
    /// True when at least one condition in `of` is true.
    Any {
        /// The conditions.
        of: Vec<Condition>,
    },
}
