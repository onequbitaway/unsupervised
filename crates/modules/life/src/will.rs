//! The will: the verb `write_will`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Kin, ResidentId, VerbSpec};

use crate::care::BODY_CARE;
use crate::handles::Handles;

/// Arguments of `write_will`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteWillArgs {
    /// The resident that gets your money and goods when you die. Without
    /// it, the will names nobody: your spouse inherits, else your city.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heir: Option<ResidentId>,
}

/// Registers the verb `write_will`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.instant_verb::<WriteWillArgs>(
        VerbSpec {
            verb: "write_will",
            module: "life",
            summary: "Name the resident that inherits from you, or name nobody.",
            free: true,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx, args| {
            if args.heir == Some(ctx.resident) {
                return Err(ActionError::invalid_args("You cannot be your own heir."));
            }
            realm.col_mut::<Kin>()[ctx.idx].heir = args.heir;
            realm.emit(
                EventMeta::public("life.will_written", h.drama(realm, h.p.drama_will))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "resident": ctx.resident, "heir": args.heir }),
            );
            Ok(json!({ "heir": args.heir }))
        },
    );
}
