//! The docket: what every resident can read about the cases of a city.
//!
//! `case_list` and `case_view` are free queries of the registry. They
//! name the suspect of a case only after a warrant, an arrest or a verdict
//! did so. Before that the perpetrator is secret.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{ActionError, Builder, CityId, Realm, ResidentId};

use super::registry::{Case, Cases, Status};
use crate::ids::Handles;
use crate::verbs::free_spec;

/// Arguments of `case_list`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CaseListArgs {
    /// Only cases with this status: `open`, `wanted`, `held`, `trial`,
    /// `convicted`, `acquitted`, `cold` or `moot`. Without it, the cases
    /// that are not closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Arguments of `case_view`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CaseViewArgs {
    /// The case to read.
    pub case: u64,
}

/// Every status, for the check of an argument.
const STATUSES: [Status; 8] = [
    Status::Open,
    Status::Wanted,
    Status::Held,
    Status::Trial,
    Status::Convicted,
    Status::Acquitted,
    Status::Cold,
    Status::Moot,
];

/// Returns the short form of a case for a list.
fn brief(case: &Case, frame: u64) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("case".into(), json!(case.id));
    out.insert("kind".into(), json!(case.kind));
    out.insert("status".into(), json!(case.status.name()));
    out.insert("age".into(), json!(frame.saturating_sub(case.opened)));
    out.insert("zone".into(), json!(case.zone));
    out.insert("pos".into(), json!(case.pos));
    if let Some(site) = case.site {
        out.insert("site".into(), json!(site));
    }
    out.insert("evidence".into(), json!(case.evidence.total()));
    if let Some(suspect) = case.suspect() {
        out.insert("suspect".into(), json!(suspect));
    }
    if let Some(trial) = case.trial.as_ref().filter(|_| case.status == Status::Trial) {
        out.insert("decided_in".into(), json!(trial.deadline.saturating_sub(frame)));
    }
    out
}

/// The verb `case_list`: the cases of the city where the resident is,
/// newest first, at most `crime.case_list_max`.
fn list(
    realm: &Realm,
    h: &Handles,
    city: CityId,
    args: &CaseListArgs,
) -> Result<Value, ActionError> {
    let only = match &args.status {
        None => None,
        Some(name) => match STATUSES.into_iter().find(|status| status.name() == name) {
            Some(status) => Some(status),
            None => {
                let names: Vec<&str> = STATUSES.iter().map(|status| status.name()).collect();
                return Err(ActionError::invalid_args(format!("No status is named `{name}`."))
                    .with_details(json!({ "statuses": names })));
            }
        },
    };
    let frame = realm.frame();
    let cases = realm.res::<Cases>();
    let most = h.count(realm, h.p.case_list_max);
    let mut found: Vec<&Case> = cases
        .iter()
        .filter(|case| {
            case.city == city
                && case.is_visible()
                && only.map_or(!case.status.is_closed(), |status| case.status == status)
        })
        .collect();
    // The newest case first.
    found.sort_unstable_by_key(|case| std::cmp::Reverse((case.opened, case.id)));
    let more = found.len().saturating_sub(most);
    let shown: Vec<Value> =
        found.into_iter().take(most).map(|case| Value::Object(brief(case, frame))).collect();
    Ok(json!({ "city": city, "cases": shown, "more": more }))
}

/// The verb `case_view`: one case with its evidence, its trial and its
/// sentence.
fn view(
    realm: &Realm,
    h: &Handles,
    reader: ResidentId,
    args: &CaseViewArgs,
) -> Result<Value, ActionError> {
    let frame = realm.frame();
    let cases = realm.res::<Cases>();
    let Some(case) = cases.get(args.case).filter(|case| case.is_visible()) else {
        return Err(ActionError::not_found("No case has this number."));
    };
    let mut out = brief(case, frame);
    out.insert("city".into(), json!(case.city));
    out.insert("value".into(), json!(case.value));
    out.insert("attempt".into(), json!(case.attempt));
    out.insert("witnesses".into(), json!(case.witnesses));
    out.insert("reported".into(), json!(case.reported));
    out.insert("victim".into(), json!(case.victim));
    out.insert("evidence_by_source".into(), json!(case.evidence));
    out.insert(
        "thresholds".into(),
        json!({
            "warrant": realm.param_in(h.p.warrant_evidence, case.city),
            "charge": realm.param_in(h.p.charge_evidence, case.city),
            "convict": realm.param_in(h.p.convict_evidence, case.city),
        }),
    );
    if let Some(trial) = &case.trial {
        let seat = trial.jury.iter().find(|seat| seat.resident == reader);
        let mut court = Map::new();
        court.insert("charged_by".into(), json!(trial.officer));
        court.insert("jury".into(), json!(trial.summoned));
        court.insert("jurors".into(), json!(trial.jurors()));
        court.insert("votes".into(), json!(trial.votes_cast()));
        if let Some(seat) = seat {
            court.insert("you_are_juror".into(), json!(true));
            court.insert("your_vote".into(), json!(seat.vote));
        }
        out.insert("trial".into(), Value::Object(court));
    }
    if let Some(sentence) = &case.sentence {
        out.insert(
            "sentence".into(),
            json!({
                "guilty": sentence.guilty, "by": sentence.by.name(), "judge": sentence.judge,
                "fine": sentence.fine, "jail_ticks": sentence.jail_ticks,
                "restitution": sentence.restitution,
            }),
        );
    }
    Ok(Value::Object(out))
}

/// Registers the two queries.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.global_verb::<CaseListArgs>(
        free_spec(
            "case_list",
            "Read the docket of the city where you are: the cases that are not closed, with their place, evidence and status. A suspect is named only after a warrant.",
        ),
        move |realm, ctx, args| {
            let city = realm.layout().zone(ctx.zone).map_or(CityId(0), |zone| zone.city);
            list(realm, &h, city, args)
        },
    );
    b.global_verb::<CaseViewArgs>(
        free_spec(
            "case_view",
            "Read one case: its evidence by source, its trial and its sentence.",
        ),
        move |realm, ctx, args| view(realm, &h, ctx.resident, args),
    );
}
