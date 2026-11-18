//! `leaderboard`: a resident reads a board and its own place on it.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, GlobalCtx, Realm, Value};

use super::score;
use super::state::{Board, Boards, Standing};
use crate::params::{self, Params};
use crate::read_spec;
use crate::season::Seasons;

/// Which list of a board `leaderboard` returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
enum By {
    /// The best residents.
    #[default]
    Resident,
    /// The best owners.
    Owner,
    /// The best organizations.
    Org,
    /// The best models. The model is what the owner of a resident declared.
    Model,
}

/// Arguments of `leaderboard`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
struct BoardArgs {
    /// The board: `wealth`, `influence`, `notoriety`, `law`, `missions` or
    /// `longevity`.
    board: Board,
    /// Which list: `resident` (the default), `owner`, `org` or `model`.
    #[serde(default)]
    by: By,
    /// A season that ended. The answer is its final standing from the hall
    /// of fame. The season that runs now when absent.
    #[serde(default)]
    season: Option<u32>,
}

/// Puts the list that the caller asked for into the answer, under the name
/// that the published board uses for it.
fn put_list(out: &mut Value, standing: &Standing, by: By) {
    match by {
        By::Resident => out["top"] = standing.top_value(),
        By::Owner => out["by_owner"] = standing.owners_value(),
        By::Org => out["by_org"] = standing.orgs_value(),
        By::Model => out["by_model"] = standing.models_value(),
    }
    if matches!(by, By::Resident | By::Model) {
        // Every `model` is the text that the owner of a resident gave.
        out["model_source"] = json!("self_reported");
    }
}

fn read(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &BoardArgs,
) -> Result<Value, ActionError> {
    let seasons = realm.res::<Seasons>();
    let board = args.board;
    let current = seasons.current(realm, p);
    if let Some(season) = args.season
        && season != current
    {
        let Some(record) = seasons.hall.iter().find(|record| record.season == season) else {
            return Err(ActionError::not_found(format!(
                "The hall of fame has no season {season}."
            )));
        };
        let standing = record.standing(board).cloned().unwrap_or_default();
        let mut out = json!({
            "board": board.name(),
            "season": season,
            "final": true,
            "started": record.started,
            "ended": record.ended,
            "unit": board.unit(),
            "residents": standing.residents,
        });
        put_list(&mut out, &standing, args.by);
        // A season that ended keeps only its top list.
        if let Some(entry) = standing.top.iter().find(|entry| entry.resident == ctx.resident) {
            out["you"] = json!({ "rank": entry.rank, "score": entry.score });
        }
        return Ok(out);
    }
    let boards = realm.res::<Boards>();
    let scores = score::scores(realm, &boards, board);
    // Only the list that the caller asked for is built.
    let (top, group_top) = match args.by {
        By::Resident => (params::count(realm, p.board_top), 0),
        _ => (0, params::count(realm, p.board_groups)),
    };
    let standing = score::standing(realm, &boards, &scores, top, group_top);
    let mut out = json!({
        "board": board.name(),
        "season": current,
        "season_ends": seasons.ends(realm, p),
        "unit": board.unit(),
        "residents": standing.residents,
    });
    put_list(&mut out, &standing, args.by);
    if let Some((rank, score)) = score::rank_of(&scores, ctx.resident) {
        out["you"] = json!({ "rank": rank, "score": score });
    }
    Ok(out)
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<BoardArgs>(
        read_spec(
            "leaderboard",
            "Read a leaderboard: the best residents, owners, organizations or models, and your own place.",
        ),
        move |realm, ctx, args| read(realm, &p, ctx, args),
    );
}
