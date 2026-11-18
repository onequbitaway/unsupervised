//! Rate limits of the verbs that a resident can repeat without a cost in
//! game time.

use unsup_kernel::{ActionCtx, ActionError, ErrorCode, ParamId, Realm};

use crate::social::{Channel, Social};

/// Checks that a resident can use a rate-limited verb once more in the
/// current window, then runs `go`. When `go` succeeds, the use is counted.
///
/// `limit` is the parameter with the number of uses in one window.
pub(crate) fn limited<T>(
    realm: &Realm,
    ctx: &ActionCtx,
    channel: Channel,
    limit: ParamId,
    go: impl FnOnce() -> Result<T, ActionError>,
) -> Result<T, ActionError> {
    let frame = realm.frame();
    let limit = realm.param(limit);
    let meter =
        realm.col::<Social>()[ctx.idx].get().map(|state| state.meter(channel)).unwrap_or_default();
    if !meter.has_room(frame, channel.window(), limit) {
        return Err(ActionError::new(
            ErrorCode::RateLimited,
            format!("You can do this {limit} times in one game {}.", channel.window_name()),
        ));
    }
    let out = go()?;
    realm.col_mut::<Social>()[ctx.idx].make().meter_mut(channel).count(frame, channel.window());
    Ok(out)
}
