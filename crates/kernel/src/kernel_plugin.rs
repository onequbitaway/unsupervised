//! The kernel's own plugin: its parameters, the condition `frozen` and the
//! built-in verbs `wait`, `cancel` and `set_routines`.
//!
//! The kernel registers them through the same [`Builder`] that modules use.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_protocol::{ActionError, Routine};

use crate::defs::CondDef;
use crate::plugin::{Begin, Builder, NoArgs, Plugin, VerbSpec};

/// Arguments of the verb `wait`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WaitArgs {
    /// How many ticks to do nothing. One tick is one game minute. At least 1
    /// and at most the parameter `kernel.wait_max`.
    pub ticks: u32,
}

/// Arguments of the verb `set_routines`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetRoutinesArgs {
    /// The new routines, in priority order. They replace all routines of the
    /// resident. An empty list removes them.
    pub routines: Vec<Routine>,
}

/// Name of the faucet that tests and administration tools use to give a
/// holder money or goods. No game rule uses it.
pub const GRANT_FAUCET: &str = "kernel.grant";

/// The plugin of the kernel. It is always built first.
pub(crate) struct KernelPlugin;

impl Plugin for KernelPlugin {
    fn name(&self) -> &'static str {
        "kernel"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(include_str!("../data/balance.toml"));
        b.condition(
            CondDef::new("frozen")
                .blocking()
                .summary("An administrator stopped the resident. It cannot act."),
        );

        b.faucet(GRANT_FAUCET);

        let wait_max = b.param_id("kernel.wait_max");
        b.verb::<WaitArgs>(
            VerbSpec {
                verb: "wait",
                module: "kernel",
                summary: "Do nothing for a number of ticks.",
                free: false,
                allowed_in: &[],
            },
            move |realm, _ctx, args| {
                let max = realm.param(wait_max);
                if args.ticks == 0 || i64::from(args.ticks) > max {
                    return Err(ActionError::invalid_args(format!(
                        "`ticks` must be from 1 to {max}."
                    )));
                }
                Ok(Begin::Busy { ticks: args.ticks })
            },
            |_realm, _ctx, args| Ok(json!({ "waited": args.ticks })),
        );
        b.instant_verb::<NoArgs>(
            VerbSpec {
                verb: "cancel",
                module: "kernel",
                summary: "Stop the current action and clear the queue of planned actions.",
                free: true,
                allowed_in: &[],
            },
            |realm, ctx, _args| Ok(realm.cancel_actions(ctx.idx)),
        );
        b.instant_verb::<SetRoutinesArgs>(
            VerbSpec {
                verb: "set_routines",
                module: "kernel",
                summary: "Replace the routines: standing orders that run when the resident is idle.",
                free: true,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                realm.set_routines(ctx.idx, &args.routines)?;
                Ok(json!({ "routines": args.routines.len() }))
            },
        );
        for verb in ["wait", "cancel", "set_routines"] {
            b.allow_everywhere(verb);
        }
    }
}
