//! Observations in batches.
//!
//! An observation is the most expensive call that does not change the
//! world. The simulation thread does not build one for each call as the
//! call comes. It collects the calls that ask for one and builds all their
//! observations in one step, on all processor cores: [`observe_batch`].
//! Every observation of the simulation thread comes from that function.

use unsup_kernel::World;
use unsup_protocol::{Observation, ObserveOpts, ResidentId, SimError};

/// Builds the observations of many residents in one step. The answers are
/// in the order of `requests`, one for each request.
///
/// The kernel groups the requests by zone and builds the observations on
/// all processor cores (`World::observe_batch`). Each answer is equal to
/// the answer of `World::observe` for the same request.
pub(crate) fn observe_batch(
    world: &mut World,
    requests: &[(ResidentId, ObserveOpts)],
) -> Vec<Result<Observation, SimError>> {
    world.observe_batch(requests)
}
