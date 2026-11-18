//! `relate`: a private label for another resident.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, ResidentId, VerbSpec};

use crate::params::{self, Params};
use crate::social::{Label, Social};
use crate::spec;

/// The label in the arguments of `relate`.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
enum LabelArg {
    /// A resident that you like.
    Friend,
    /// A resident that you compete with.
    Rival,
    /// A resident that works with you.
    Ally,
    /// A resident that you fight.
    Enemy,
    /// Remove the label.
    None,
}

impl LabelArg {
    fn label(self) -> Option<Label> {
        match self {
            LabelArg::Friend => Some(Label::Friend),
            LabelArg::Rival => Some(Label::Rival),
            LabelArg::Ally => Some(Label::Ally),
            LabelArg::Enemy => Some(Label::Enemy),
            LabelArg::None => None,
        }
    }
}

/// Arguments of `relate`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct RelateArgs {
    /// The resident that gets the label.
    resident: ResidentId,
    /// `friend`, `rival`, `ally`, `enemy`, or `none` to remove the label.
    label: LabelArg,
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.instant_verb::<RelateArgs>(
        VerbSpec {
            free: true,
            ..spec("relate", "Give another resident a private label: friend, rival, ally or enemy. Only you see it.")
        },
        move |realm, ctx, args| {
            if args.resident == ctx.resident {
                return Err(ActionError::invalid_args("You cannot label yourself."));
            }
            let max = params::count(realm, p.relations_max);
            let mut column = realm.col_mut::<Social>();
            let (known, count) = column[ctx.idx].get().map_or((false, 0), |state| {
                (state.labels.iter().any(|(id, _)| *id == args.resident), state.labels.len())
            });
            let Some(label) = args.label.label() else {
                if known {
                    column[ctx.idx].make().labels.retain(|(id, _)| *id != args.resident);
                }
                return Ok(json!({ "resident": args.resident, "label": null }));
            };
            // A refused action must not change anything, so check first.
            if !known && count >= max {
                return Err(ActionError::conflict(format!(
                    "You keep at most {max} labels. Remove one first."
                )));
            }
            let labels = &mut column[ctx.idx].make().labels;
            labels.retain(|(id, _)| *id != args.resident);
            let at = labels.partition_point(|(id, _)| *id < args.resident);
            labels.insert(at, (args.resident, label));
            Ok(json!({ "resident": args.resident, "label": label.name() }))
        },
    );
}
