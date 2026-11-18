//! Observations in batches.
//!
//! An observation is the most expensive call that does not change the
//! world. The simulation thread does not build one for each call as the
//! call comes. It collects the calls that ask for one and builds all their
//! observations in one step: [`observe_batch`]. Every observation of the
//! simulation thread comes from that function.

use unsup_kernel::World;
use unsup_protocol::{Observation, ObserveOpts, ResidentId, SimError};

/// Builds the observations of many residents in one step. The answers are
/// in the order of `requests`, one for each request.
///
/// This is the one place to change when the kernel can build observations
/// on all cores. Today the function asks the world for one observation
/// after the other, on the simulation thread. With
/// `World::observe_batch(&mut self, &[(ResidentId, ObserveOpts)])` its body
/// is `world.observe_batch(requests)`. It takes the world as `&mut` for
/// that call.
pub(crate) fn observe_batch(
    world: &mut World,
    requests: &[(ResidentId, ObserveOpts)],
) -> Vec<Result<Observation, SimError>> {
    requests.iter().map(|(resident, opts)| world.observe(*resident, opts)).collect()
}
