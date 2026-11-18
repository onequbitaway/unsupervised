//! Arguments of the contract verbs, and the checks that turn the clauses
//! that a resident wrote into clauses of the engine.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::{ActionError, Realm, ResidentId, SiteId, Value};

use super::WATCHED;
use super::model::{Clause, Scalar, Side, Term};
use crate::params::{self, Params};
use crate::text;

/// One clause of an offer.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ClauseArg {
    /// `by` pays `amount` to the other party when the contract is signed.
    PayAtSigning {
        /// The party that pays.
        by: Side,
        /// Cents.
        amount: i64,
    },
    /// `by` pays `amount` to the other party at the deadline. The money is
    /// locked from the start, so the payment cannot fail.
    PayAtDeadline {
        /// The party that pays.
        by: Side,
        /// Cents.
        amount: i64,
        /// Game minutes from the signing to the deadline.
        due_in: u32,
    },
    /// `by` pays `amount` to the other party when an event happens before
    /// the deadline. When it does not happen, `by` gets the money back.
    /// This makes a bounty.
    PayOnEvent {
        /// The party that pays.
        by: Side,
        /// Cents.
        amount: i64,
        /// Kind of the event, for example `crime.arrest_made`.
        event: String,
        /// Data fields that the event must have, for example
        /// `{"target": 42}`. Values are whole numbers, booleans or short
        /// names.
        #[serde(default)]
        fields: BTreeMap<String, Value>,
        /// When given, only an event that this resident caused counts.
        #[serde(default)]
        actor: Option<ResidentId>,
        /// Game minutes from the signing to the deadline.
        due_in: u32,
    },
    /// `by` delivers goods before the deadline with `contract_claim`. When
    /// it does not, it breaches the contract.
    Deliver {
        /// The party that delivers.
        by: Side,
        /// Name of the item.
        item: String,
        /// Number of units.
        qty: i64,
        /// The site where the goods go. When absent, `by` hands them to the
        /// other party in person.
        #[serde(default)]
        site: Option<SiteId>,
        /// Game minutes from the signing to the deadline.
        due_in: u32,
    },
    /// `by` locks `amount`. The other party gets it when `by` breaches the
    /// contract. Else `by` gets it back at the end.
    Penalty {
        /// The party that locks the money.
        by: Side,
        /// Cents.
        amount: i64,
    },
}

/// Arguments of `contract_offer`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OfferArgs {
    /// The resident that can sign. When absent, the first resident that
    /// signs is the other party.
    #[serde(default)]
    pub(crate) to: Option<ResidentId>,
    /// The clauses. You are the `offerer` in them.
    pub(crate) clauses: Vec<ClauseArg>,
    /// Game minutes that the offer stays open. The default is the parameter
    /// `society.contract_offer_ttl`.
    #[serde(default)]
    pub(crate) expires_in: Option<u32>,
}

/// Arguments of `contract_sign`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct SignArgs {
    /// Id of the contract.
    pub(crate) contract: u64,
    /// Cents of your cash that you agree to lock. It must cover your part
    /// of the contract (`your_part` in the offer). What is above your part
    /// comes back.
    #[serde(default)]
    pub(crate) stake: i64,
}

/// Arguments of verbs that name only a contract.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct ContractArgs {
    /// Id of the contract.
    pub(crate) contract: u64,
}

/// Arguments of `contract_claim`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct ClaimArgs {
    /// Id of the contract.
    pub(crate) contract: u64,
    /// Number of the `deliver` clause, from 0. When absent: your first open
    /// one.
    #[serde(default)]
    pub(crate) clause: Option<u32>,
}

fn bad(message: impl Into<String>) -> ActionError {
    ActionError::invalid_args(message)
}

fn check_amount(amount: i64) -> Result<i64, ActionError> {
    if amount > 0 { Ok(amount) } else { Err(bad("An amount must be greater than zero.")) }
}

fn check_due(realm: &Realm, p: &Params, due_in: u32) -> Result<u32, ActionError> {
    let max = realm.param(p.contract_due_max);
    if due_in == 0 || i64::from(due_in) > max {
        return Err(bad(format!("`due_in` must be from 1 to {max} game minutes.")));
    }
    Ok(due_in)
}

fn check_fields(
    realm: &Realm,
    p: &Params,
    fields: &BTreeMap<String, Value>,
) -> Result<Vec<(String, Scalar)>, ActionError> {
    let max = params::count(realm, p.contract_match_max);
    if fields.len() > max {
        return Err(bad(format!("An event clause compares at most {max} data fields.")));
    }
    let name_max = params::count(realm, p.contract_field_name_max);
    let text_max = params::count(realm, p.contract_field_text_max);
    fields
        .iter()
        .map(|(name, value)| {
            if !text::is_ident(name, name_max) {
                return Err(bad(format!("`{name}` is not the name of a data field.")));
            }
            let scalar = match value {
                Value::Bool(v) => Scalar::Bool(*v),
                Value::Number(v) => Scalar::Int(v.as_i64().ok_or_else(|| {
                    bad(format!("The value of `{name}` must be a whole number."))
                })?),
                Value::String(v) if text::is_token(v, text_max) => Scalar::Text(v.clone()),
                _ => {
                    return Err(bad(format!(
                        "The value of `{name}` must be a whole number, a boolean or a short name without spaces."
                    )));
                }
            };
            Ok((name.clone(), scalar))
        })
        .collect()
}

/// Checks the clauses of an offer and returns them in the form of the
/// engine. It reads only definitions and parameters, so it runs in a zone.
pub(crate) fn check_clauses(
    realm: &Realm,
    p: &Params,
    clauses: &[ClauseArg],
) -> Result<Vec<Clause>, ActionError> {
    let max = params::count(realm, p.contract_clauses_max);
    if clauses.is_empty() || clauses.len() > max {
        return Err(bad(format!("A contract has from 1 to {max} clauses.")));
    }
    clauses
        .iter()
        .map(|clause| {
            Ok(match clause {
                ClauseArg::PayAtSigning { by, amount } => {
                    Clause::new(*by, Term::PayAtSigning { amount: check_amount(*amount)? }, 0)
                }
                ClauseArg::PayAtDeadline { by, amount, due_in } => Clause::new(
                    *by,
                    Term::PayAtDeadline { amount: check_amount(*amount)? },
                    check_due(realm, p, *due_in)?,
                ),
                ClauseArg::PayOnEvent { by, amount, event, fields, actor, due_in } => {
                    let kind_max = params::count(realm, p.contract_event_kind_max);
                    if !text::is_event_kind(event, kind_max)
                        || !WATCHED.iter().any(|w| event.starts_with(w))
                    {
                        return Err(bad(format!(
                            "`{event}` is not an event that a contract can wait for. Use a kind that starts with one of: {}.",
                            WATCHED.join(", ")
                        )));
                    }
                    Clause::new(
                        *by,
                        Term::PayOnEvent {
                            amount: check_amount(*amount)?,
                            event: event.clone(),
                            fields: check_fields(realm, p, fields)?,
                            actor: *actor,
                        },
                        check_due(realm, p, *due_in)?,
                    )
                }
                ClauseArg::Deliver { by, item, qty, site, due_in } => {
                    let Some(item) = realm.defs().item_id(item) else {
                        return Err(bad(format!("No item is named `{item}`.")));
                    };
                    if *qty <= 0 {
                        return Err(bad("`qty` must be greater than zero."));
                    }
                    Clause::new(
                        *by,
                        Term::Deliver { item, qty: *qty, site: *site },
                        check_due(realm, p, *due_in)?,
                    )
                }
                ClauseArg::Penalty { by, amount } => {
                    Clause::new(*by, Term::Penalty { amount: check_amount(*amount)? }, 0)
                }
            })
        })
        .collect()
}

/// Returns the cents that one side locks, or an error when the sum does not
/// fit.
pub(crate) fn part_of(clauses: &[Clause], side: Side) -> Result<i64, ActionError> {
    clauses
        .iter()
        .filter(|clause| clause.by == side)
        .try_fold(0i64, |sum, clause| sum.checked_add(clause.term.stake()))
        .ok_or_else(|| bad("The amounts are too large."))
}
