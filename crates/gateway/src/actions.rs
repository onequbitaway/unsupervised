//! What the HTTP routes, the WebSocket and the MCP tools share: sending
//! actions and building observation options.

use serde_json::json;
use unsup_protocol::{
    ActionAck, ActionRequest, Detail, ObserveOpts, ResidentId, Routine, SimError, WaitFilter,
};
use unsup_runtime::SimHandle;

/// The verb that replaces the routines of a resident.
const SET_ROUTINES: &str = "set_routines";

/// Sends the requests of a plan in order. Stops after the first request
/// that the simulation rejects, so the answer has one acknowledgement for
/// each request that was sent.
pub(crate) async fn run_plan(
    sim: &SimHandle,
    resident: ResidentId,
    requests: Vec<ActionRequest>,
) -> Result<Vec<ActionAck>, SimError> {
    let mut acks = Vec::with_capacity(requests.len());
    for request in requests {
        let ack = sim.act(resident, request).await?;
        let rejected = !ack.status.is_accepted();
        acks.push(ack);
        if rejected {
            break;
        }
    }
    Ok(acks)
}

/// Replaces the routines of a resident. The simulation checks them; a
/// refusal is in the acknowledgement.
pub(crate) async fn set_routines(
    sim: &SimHandle,
    resident: ResidentId,
    routines: &[Routine],
) -> Result<ActionAck, SimError> {
    sim.act(resident, ActionRequest::new(SET_ROUTINES, json!({ "routines": routines }))).await
}

/// Builds observation options from the optional values of a request.
pub(crate) fn observe_opts(
    detail: Option<Detail>,
    since: Option<u64>,
    sections: Option<Vec<String>>,
) -> ObserveOpts {
    ObserveOpts { detail: detail.unwrap_or_default(), since_notice: since.unwrap_or(0), sections }
}

/// Builds the filter of a wait: the listed kinds, or any news.
pub(crate) fn wait_filter(kinds: Option<Vec<String>>) -> WaitFilter {
    match kinds {
        Some(kinds) => WaitFilter::Kinds { kinds },
        None => WaitFilter::Any,
    }
}
