//! The verbs of the module.
//!
//! Every verb that changes something costs game time
//! (`docs/design/game-design.md`, section 5). Most verbs here are *timed*:
//! one function checks the request and applies it. `begin` runs it in the
//! phase [`Phase::Check`], so that a request that cannot work is refused at
//! once and changes nothing. `finish` runs it again in the phase
//! [`Phase::Apply`], because the world can change while the resident is
//! busy.

use std::sync::Arc;

use unsup_kernel::{ActionCtx, ActionError, Args, Begin, Builder, ParamId, Realm, Value, VerbSpec};

pub(crate) mod business;
pub(crate) mod gather;
pub(crate) mod jobs;
pub(crate) mod lease;
pub(crate) mod produce;
pub(crate) mod property;
pub(crate) mod shop;
pub(crate) mod till;
pub(crate) mod work;

/// Name of the module in the verb catalogue.
pub(crate) const MODULE: &str = "economy";

/// Conditions in which a resident can still read the boards.
const READ_ALLOWED_IN: &[&str] = &["hospitalized", "detained", "jailed"];

/// The two runs of a timed verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Only check. Nothing may change.
    Check,
    /// Check again and apply.
    Apply,
}

impl Phase {
    /// Returns true in the phase that only checks.
    pub(crate) fn is_check(self) -> bool {
        self == Phase::Check
    }
}

/// Makes the description of a verb that occupies the resident.
pub(crate) fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free: false, allowed_in: &[] }
}

/// Makes the description of a free verb that only reads.
pub(crate) fn query(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free: true, allowed_in: READ_ALLOWED_IN }
}

/// Turns a number of minutes from a parameter into ticks.
pub(crate) fn ticks(minutes: i64) -> u32 {
    u32::try_from(minutes.max(1)).unwrap_or(u32::MAX)
}

/// Registers a timed verb. It lasts as many ticks as the parameter
/// `minutes` says.
pub(crate) fn timed<A: Args>(
    b: &mut Builder,
    spec: VerbSpec,
    minutes: ParamId,
    run: impl Fn(&Realm, &ActionCtx, &A, Phase) -> Result<Value, ActionError> + Send + Sync + 'static,
) {
    let apply = Arc::new(run);
    let check = Arc::clone(&apply);
    b.verb::<A>(
        spec,
        move |realm, ctx, args| {
            check(realm, ctx, args, Phase::Check)?;
            Ok(Begin::Busy { ticks: ticks(realm.param(minutes)) })
        },
        move |realm, ctx, args| apply(realm, ctx, args, Phase::Apply),
    );
}
