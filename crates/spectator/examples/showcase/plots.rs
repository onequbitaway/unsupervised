//! The stories of the showcase: events that belong together, so that the
//! director and the stories of the spectator service have something to
//! show. A crime with its investigation, a feud, a bankruptcy, an election
//! with its candidates, careers, and the published state of the economy
//! and of the boards. Nothing here is the game.

use serde_json::{Value, json};
use unsup_protocol::ResidentId;

use crate::numbers::Numbers;

/// One event that the driver sends at a frame.
pub struct Step {
    /// The frame at which the event happens.
    pub due: u64,
    /// The resident that acts.
    pub actor: ResidentId,
    /// The verb: `happen`, `declare` or `rob`.
    pub verb: &'static str,
    /// The arguments of the verb.
    pub args: Value,
}

fn step(
    due: u64,
    actor: ResidentId,
    verb: &'static str,
    kind: &str,
    drama: u8,
    data: Value,
) -> Step {
    Step { due, actor, verb, args: json!({ "kind": kind, "drama": drama, "data": data }) }
}

/// The names of the models of the showcase, for the board of the models.
const MODELS: [&str; 3] = ["atlas-2", "quill-mini", "nova-pro"];

/// Returns the steps that start at a frame. `pick` gives a resident for a
/// number: the same number gives the same resident while nobody dies.
pub fn start(frame: u64, numbers: &Numbers, pick: &dyn Fn(u64) -> ResidentId) -> Vec<Step> {
    let plots = &numbers.plots;
    let drama = &numbers.plot_drama;
    let mut steps = Vec::new();
    let at = |every: u64, offset: u64| every > 0 && frame % every == offset % every;
    let turn = |every: u64| frame / every.max(1);

    if at(plots.crime_every, 12) {
        let n = turn(plots.crime_every);
        let (thief, victim, officer) = (pick(n * 7 + 1), pick(n * 7 + 2), pick(n * 13 + 5));
        let case = 1_000 + n;
        // The events have the data of the crime module
        // (`docs/design/modules/crime.md`, section 12). Every third crime
        // is a heist: a burglary with a large loss.
        let heist = n % 3 == 2;
        if heist {
            let data = json!({
                "case": case, "kind": "burglary", "city": 0, "site": null,
                "value": 450_000 + 90_000 * (n % 7), "attempt": false,
            });
            steps.push(step(
                frame,
                thief,
                "happen",
                "crime.theft_committed",
                drama.heist_committed,
                data,
            ));
        } else {
            // A theft needs the victim in the same district. When it is
            // not there, the warrant starts the story.
            let args = json!({ "other": victim, "case": case });
            steps.push(Step { due: frame, actor: thief, verb: "rob", args });
        }
        steps.push(step(
            frame + plots.investigation_after,
            officer,
            "happen",
            "crime.warrant_issued",
            drama.investigation_opened,
            json!({ "case": case, "kind": "theft", "city": 0 }),
        ));
        steps.push(step(
            frame + plots.arrest_after,
            officer,
            "happen",
            "crime.arrest_made",
            drama.arrest_made,
            json!({
                "officer": officer, "target": thief, "case": case, "cases": [case], "city": 0,
                "watch": false,
            }),
        ));
        steps.push(step(
            frame + plots.trial_after,
            thief,
            "happen",
            "crime.verdict_given",
            drama.trial_ended,
            json!({
                "case": case, "kind": "theft", "city": 0, "target": thief, "guilty": n % 2 == 0,
                "by": "jury", "judge": null, "fine": 3_000, "jail_ticks": 240,
                "restitution": 4_500, "votes_guilty": 3, "votes_not_guilty": 2,
            }),
        ));
    }
    if at(plots.feud_every, 40) {
        let n = turn(plots.feud_every);
        let (a, b) = (pick(n * 11 + 3), pick(n * 11 + 4));
        let opinion = |from: ResidentId, about: ResidentId| json!({ "from": from, "about": about, "verdict": "condemn" });
        steps.push(step(
            frame,
            a,
            "happen",
            "society.opinion_given",
            drama.opinion_given,
            opinion(a, b),
        ));
        steps.push(step(
            frame + plots.feud_step,
            b,
            "happen",
            "society.contract_breached",
            drama.contract_breached,
            json!({ "contract": n, "by": b, "parties": [a, b] }),
        ));
        steps.push(step(
            frame + 2 * plots.feud_step,
            b,
            "happen",
            "society.opinion_given",
            drama.opinion_given,
            opinion(b, a),
        ));
    }

    if at(plots.bankruptcy_every, 95) {
        let n = turn(plots.bankruptcy_every);
        let who = pick(n * 17 + 9);
        steps.push(step(
            frame,
            who,
            "happen",
            "finance.loan_defaulted",
            drama.loan_defaulted,
            json!({ "resident": who, "outstanding": 120_000 + 15_000 * (n % 5) }),
        ));
        steps.push(step(
            frame + plots.bankruptcy_step,
            who,
            "happen",
            "economy.property_foreclosed",
            drama.property_foreclosed,
            json!({ "from": { "Resident": who }, "reason": "loan_default" }),
        ));
        steps.push(step(
            frame + 2 * plots.bankruptcy_step,
            who,
            "happen",
            "economy.tenant_evicted",
            drama.tenant_evicted,
            json!({ "tenant": who }),
        ));
    }

    if at(plots.career_every, 3) {
        let n = turn(plots.career_every);
        let who = pick(n * 5 + 2);
        let (kind, score, data) = if n % 2 == 0 {
            ("economy.worker_hired", drama.worker_hired, json!({ "resident": who, "wage": 1_200 }))
        } else {
            ("economy.lease_started", drama.lease_started, json!({ "tenant": who, "rent": 800 }))
        };
        steps.push(step(frame, who, "happen", kind, score, data));
    }
    steps
}

/// Returns the steps of the campaign of an election that is decided at
/// the frame `decided`: the candidates register in the frames before it.
pub fn campaign(
    decided: u64,
    city: u64,
    numbers: &Numbers,
    pick: &dyn Fn(u64) -> ResidentId,
) -> Vec<Step> {
    let plots = &numbers.plots;
    let count = plots.candidates.max(1);
    (0..count)
        .map(|n| {
            let candidate = pick(decided * 3 + n);
            let due = decided.saturating_sub(plots.campaign_frames)
                + n * (plots.campaign_frames / (count + 1));
            step(
                due,
                candidate,
                "declare",
                "politics.candidate_registered",
                numbers.plot_drama.candidate_registered,
                json!({
                    "gov": "city", "city": city, "office": "mayor", "kind": "regular",
                    "candidate": candidate, "candidates": n + 1, "fee": 5_000,
                }),
            )
        })
        .collect()
}

/// Returns the statistics of the economy, in the shape of the finance
/// module (`docs/design/modules/finance.md`, section 7). The numbers move
/// a little with the frame.
pub fn stats(frame: u64, residents: u64) -> Value {
    let wave = (frame / 30) % 12;
    json!({
        "day": frame / 1_440,
        "money_supply": 250_000 * residents as i64 + 40_000 * wave as i64,
        "price_index_bp": 10_000 + 35 * wave,
        "gini_bp": 3_900 + 20 * wave,
        "residents": residents,
        "loans": { "count": residents / 6, "outstanding": 9_000_000 + 250_000 * wave, "written_off": 420_000 },
        "base_rate_bp": 450 + 25 * (wave % 4),
    })
}

/// Returns the board of the richest residents, in the shape of the
/// missions module (`docs/design/modules/missions.md`, section 10.3): the
/// top residents, and the sums for each declared model.
pub fn boards(frame: u64, numbers: &Numbers, pick: &dyn Fn(u64) -> ResidentId) -> Vec<Value> {
    let rows = numbers.plots.board_rows;
    let turn = frame / numbers.plots.boards_every.max(1);
    let top: Vec<Value> = (0..rows)
        .map(|rank| {
            let resident = pick(rank * 31 + 1);
            json!({
                "rank": rank + 1,
                "resident": resident,
                "owner": 1 + resident.0 % 20,
                "model": MODELS[(rank % 3) as usize],
                "score": 9_800_000 - 870_000 * rank as i64 + 12_000 * ((turn + rank) % 9) as i64,
            })
        })
        .collect();
    let by_model: Vec<Value> = MODELS
        .iter()
        .enumerate()
        .map(|(rank, model)| {
            let score = 7_100_000 - 900_000 * rank as i64 + 10_000 * ((turn + rank as u64) % 4) as i64;
            json!({ "rank": rank + 1, "model": model, "score": score, "residents": 3, "mean": score / 3 })
        })
        .collect();
    vec![json!({
        "key": "wealth", "season": 1, "unit": "cents", "residents": rows,
        "model_source": "self_reported", "top": top, "by_owner": [], "by_org": [],
        "by_model": by_model,
    })]
}
