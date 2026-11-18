//! Starter shifts: public works at a city hall or a depot.
//!
//! Every resident can work them. The wage comes from the faucet
//! `gov.public_works`. Each city has a budget for one game day; the global
//! realm counts it, because a city has several zones.
//!
//! A shift starts at once. The zone asks the global realm for the wage in
//! the same tick. The answer comes two ticks later: a grant, which the
//! shift needs to be paid, or a refusal, which stops the shift. When the
//! budget of the day is spent, the global realm tells every zone, and the
//! zones then refuse new shifts at once.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::{
    ActionCtx, ActionError, CityId, Dest, Event, Holder, Idx, Message, Realm, RealmId, ResidentId,
    Resource, Site, SiteId, TICKS_PER_DAY,
};

use crate::calc;
use crate::events;
use crate::ids::Econ;
use crate::labour;
use crate::site;

/// The message of a shift that stops because the budget is spent.
const SPENT: &str = "The public works budget of this city is spent for today. Try again tomorrow.";

/// One starter shift that runs in this zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shift {
    resident: ResidentId,
    site: SiteId,
    /// Wage in cents, before income tax.
    wage: i64,
    /// The day whose budget pays the wage.
    day: u64,
    /// True when the global realm granted the wage.
    granted: bool,
}

/// The starter shifts of one zone, by action id, and the day for which the
/// zone knows that the budget is spent.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct PublicWorks {
    shifts: BTreeMap<u64, Shift>,
    spent_day: Option<u64>,
}

impl Resource for PublicWorks {
    const NAME: &'static str = "economy.public_works";
}

/// What every city has granted today, in the global realm.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct PublicBudget {
    day: u64,
    granted: BTreeMap<CityId, i64>,
}

impl Resource for PublicBudget {
    const NAME: &'static str = "economy.public_budget";
}

/// A zone asks for the wage of one shift.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Claim {
    action: u64,
    city: CityId,
    wage: i64,
}

impl Message for Claim {
    const KIND: &'static str = "economy.public_claim";
}

/// The global realm answers a [`Claim`].
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Answer {
    action: u64,
    city: CityId,
    wage: i64,
    day: u64,
    granted: bool,
}

impl Message for Answer {
    const KIND: &'static str = "economy.public_answer";
}

/// A zone gives a granted wage back: the shift stopped before its end.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Release {
    city: CityId,
    wage: i64,
    day: u64,
}

impl Message for Release {
    const KIND: &'static str = "economy.public_release";
}

/// The global realm tells every zone whether a city can still pay a shift
/// today.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BudgetState {
    city: CityId,
    day: u64,
    open: bool,
}

impl Message for BudgetState {
    const KIND: &'static str = "economy.public_budget_state";
}

fn today(realm: &Realm) -> u64 {
    realm.frame() / TICKS_PER_DAY
}

// ---- Zone ---------------------------------------------------------------------

/// Returns the wage of a starter shift here, when the site offers them.
pub(crate) fn wage_at(econ: &Econ, realm: &Realm, site: &Site) -> Option<i64> {
    let rule = econ.rules.kind(&site.kind);
    (rule.public_works && site::is_government(site.owner))
        .then(|| realm.param(econ.ids.public_works_wage))
}

/// Starts a starter shift: checks what the zone knows, records the shift
/// and asks the global realm for the wage.
pub(crate) fn begin(
    realm: &Realm,
    ctx: &ActionCtx,
    site: &Site,
    wage: i64,
) -> Result<(), ActionError> {
    if wage <= 0 {
        return Err(ActionError::not_allowed("This city pays no public works now."));
    }
    let day = today(realm);
    let mut state = realm.res_mut::<PublicWorks>();
    if state.spent_day == Some(day) {
        return Err(ActionError::insufficient(SPENT));
    }
    state.shifts.insert(
        ctx.action_id,
        Shift { resident: ctx.resident, site: site.id, wage, day, granted: false },
    );
    drop(state);
    realm.send(Dest::Global, &Claim { action: ctx.action_id, city: site::city(realm), wage });
    Ok(())
}

/// Takes the record of a starter shift when the action is one.
pub(crate) fn take(realm: &Realm, action: u64) -> Option<Shift> {
    realm.res_mut::<PublicWorks>().shifts.remove(&action)
}

fn release(realm: &Realm, shift: &Shift) {
    if shift.granted {
        let city = site::city(realm);
        realm.send(Dest::Global, &Release { city, wage: shift.wage, day: shift.day });
    }
}

/// Ends a starter shift: mints the wage, withholds the income tax and
/// trains `labor`.
pub(crate) fn finish(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    shift: &Shift,
) -> Result<Value, ActionError> {
    let ids = &econ.ids;
    let pos = realm.pos(ctx.idx);
    let here = realm.sites().at(pos).map(|s| s.id);
    if here != Some(shift.site) {
        release(realm, shift);
        return Err(ActionError::not_here("You left the site before the shift ended."));
    }
    if !shift.granted {
        return Err(ActionError::insufficient(
            "The city did not confirm the wage in time. The shift is not paid.",
        ));
    }
    let worker = Holder::Resident(ctx.resident);
    let tax = calc::share(shift.wage, realm.param(ids.income_tax_bp));
    realm.ledger_mut().mint(ids.public_works, worker, unsup_kernel::Money(shift.wage));
    realm.wire_money(
        worker,
        Dest::Global,
        site::city_holder(realm),
        unsup_kernel::Money(tax),
        "income_tax",
    )?;
    let minutes = i64::try_from(realm.tick().since(ctx.started)).unwrap_or(0);
    let rule =
        econ.rules.kind(&realm.sites().get(shift.site).map(|s| s.kind.clone()).unwrap_or_default());
    let xp = labour::finish_work(econ, realm, ctx.idx, &rule.skill, minutes);
    events::shift_completed(
        realm,
        ctx.resident,
        shift.site,
        pos,
        shift.wage,
        tax,
        &rule.skill.name,
    );
    Ok(json!({
        "site": shift.site,
        "wage": shift.wage,
        "tax": tax,
        "net": shift.wage - tax,
        "skill": rule.skill.name,
        "xp": xp,
        "public_works": true,
    }))
}

/// Handles the answer of the global realm to a claim.
pub(crate) fn on_answer(realm: &Realm, answer: Answer) {
    let mut state = realm.res_mut::<PublicWorks>();
    if answer.granted {
        match state.shifts.get_mut(&answer.action) {
            Some(shift) => {
                shift.granted = true;
                shift.day = answer.day;
            }
            None => {
                // The shift stopped before the answer came.
                drop(state);
                let Answer { city, wage, day, .. } = answer;
                realm.send(Dest::Global, &Release { city, wage, day });
            }
        }
        return;
    }
    if answer.city == site::city(realm) {
        state.spent_day = Some(answer.day);
    }
    let shift = state.shifts.remove(&answer.action);
    drop(state);
    if let Some(shift) = shift
        && let Some(idx) = realm.idx_of(shift.resident)
        && realm.activity(idx).is_some_and(|a| a.action_id == answer.action)
    {
        realm.interrupt(idx, SPENT);
    }
}

/// Handles the news that a city can or cannot pay more shifts today.
pub(crate) fn on_budget_state(realm: &Realm, news: &BudgetState) {
    if news.city != site::city(realm) {
        return;
    }
    let mut state = realm.res_mut::<PublicWorks>();
    if news.open {
        if state.spent_day == Some(news.day) {
            state.spent_day = None;
        }
    } else {
        state.spent_day = Some(news.day);
    }
}

/// Handles `kernel.action_interrupted`: a starter shift that stops gives
/// its wage back to the budget.
pub(crate) fn on_interrupted(realm: &Realm, event: &Event) {
    let Some(action) = event.data.get("action_id").and_then(Value::as_u64) else {
        return;
    };
    if let Some(shift) = take(realm, action) {
        release(realm, &shift);
    }
}

/// Handles a resident that leaves the zone in the middle of a starter
/// shift. The shift cannot end in another zone.
pub(crate) fn on_depart(realm: &Realm, idx: Idx) {
    let Some(activity) = realm.activity(idx) else {
        return;
    };
    if let Some(shift) = take(realm, activity.action_id) {
        release(realm, &shift);
    }
}

// ---- Global -------------------------------------------------------------------

fn roll(realm: &Realm, budget: &mut PublicBudget) {
    let day = today(realm);
    if budget.day != day {
        budget.day = day;
        budget.granted.clear();
    }
}

/// Returns true when the city can pay one more shift today. A budget of
/// an earlier day counts as not used.
fn has_room(econ: &Econ, realm: &Realm, city: CityId, budget: &PublicBudget) -> bool {
    let granted = if budget.day == today(realm) {
        budget.granted.get(&city).copied().unwrap_or(0)
    } else {
        0
    };
    let left = realm.param_in(econ.ids.public_works_budget, city) - granted;
    left >= realm.param_in(econ.ids.public_works_wage, city).max(1)
}

/// What `list_jobs` says about the starter shifts of a city: the wage, the
/// length of a shift, the site kinds that offer them, and whether the
/// budget of today has room for one more. `None` when the city pays none.
pub(crate) fn offer(econ: &Econ, realm: &Realm, city: CityId) -> Option<Value> {
    let wage = realm.param_in(econ.ids.public_works_wage, city);
    if wage <= 0 {
        return None;
    }
    let open = has_room(econ, realm, city, &realm.res::<PublicBudget>());
    Some(json!({
        "wage": wage,
        "minutes": realm.param_in(econ.ids.shift_minutes, city),
        "kinds": econ.rules.public_works_kinds(),
        "open": open,
    }))
}

/// Tells every zone whether the city can pay one more shift today.
fn announce(econ: &Econ, realm: &Realm, city: CityId, budget: &PublicBudget) {
    let open = has_room(econ, realm, city, budget);
    realm.send(Dest::AllZones, &BudgetState { city, day: budget.day, open });
}

/// Handles a claim in the global realm: grants the wage when the budget of
/// the day has room for it.
pub(crate) fn on_claim(econ: &Econ, realm: &Realm, from: RealmId, claim: &Claim) {
    let Some(zone) = from.zone() else {
        return;
    };
    let mut budget = realm.res_mut::<PublicBudget>();
    roll(realm, &mut budget);
    let limit = realm.param_in(econ.ids.public_works_budget, claim.city);
    let granted = budget.granted.get(&claim.city).copied().unwrap_or(0);
    let ok = claim.wage > 0 && granted.saturating_add(claim.wage) <= limit;
    if ok {
        budget.granted.insert(claim.city, granted + claim.wage);
    }
    let answer = Answer {
        action: claim.action,
        city: claim.city,
        wage: claim.wage,
        day: budget.day,
        granted: ok,
    };
    realm.send(Dest::Zone(zone), &answer);
    if !ok || !has_room(econ, realm, claim.city, &budget) {
        announce(econ, realm, claim.city, &budget);
    }
}

/// Handles a wage that a zone gives back.
pub(crate) fn on_release(econ: &Econ, realm: &Realm, release: &Release) {
    let mut budget = realm.res_mut::<PublicBudget>();
    roll(realm, &mut budget);
    if release.day != budget.day {
        return;
    }
    let granted = budget.granted.get(&release.city).copied().unwrap_or(0);
    let left = (granted - release.wage).max(0);
    if left == 0 {
        budget.granted.remove(&release.city);
    } else {
        budget.granted.insert(release.city, left);
    }
    announce(econ, realm, release.city, &budget);
}
