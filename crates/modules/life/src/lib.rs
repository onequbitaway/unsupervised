//! Map, movement, needs, skills, housing and ageing.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, ErrorCode, Plugin, Pos, VerbSpec};

/// The life module.
#[derive(Clone, Copy, Debug, Default)]
pub struct LifePlugin;

/// Arguments of `move_to`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct MoveToArgs {
    /// Column of the target tile.
    pub x: i32,
    /// Row of the target tile.
    pub y: i32,
}

impl Plugin for LifePlugin {
    fn name(&self) -> &'static str {
        "life"
    }

    fn build(&self, b: &mut Builder) {
        b.verb::<MoveToArgs>(
            VerbSpec {
                verb: "move_to",
                module: "life",
                summary: "Walk to a tile. One tile takes one game minute.",
                free: false,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                let target = Pos::new(args.x, args.y);
                if realm.layout().zone_at(target).is_none() {
                    return Err(ActionError::invalid_args("the target is outside the world"));
                }
                Ok(Begin::Busy { ticks: realm.pos(ctx.idx).dist(target).max(1) })
            },
            |realm, ctx, args| {
                realm
                    .set_pos(ctx.idx, Pos::new(args.x, args.y))
                    .map_err(|_| ActionError::new(ErrorCode::Conflict, "cannot move there"))?;
                Ok(json!({ "x": args.x, "y": args.y }))
            },
        );
    }
}

/// Returns the plugin of the life module.
#[must_use]
pub fn plugin() -> LifePlugin {
    LifePlugin
}
