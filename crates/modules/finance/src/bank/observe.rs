//! The observation section `bank`: balance, loan and next instalment, and
//! the shares of the resident.

use serde_json::{Map, Value, json};
use unsup_kernel::{Builder, ObserveOpts, Realm, ResidentId};

use super::loans::{collateral, credit_limit, ticks_to_collection};
use super::state::Bank;
use crate::params::Params;
use crate::shares;
use crate::util::{calendar_day, day_of};

/// Registers the global observer of the section `bank`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.observe_global("bank", 30, move |realm, resident, opts| section(realm, &p, resident, opts));
}

fn section(realm: &Realm, p: &Params, resident: ResidentId, opts: &ObserveOpts) -> Option<Value> {
    let bank = realm.res::<Bank>();
    let account = bank.account(resident);
    let shares = shares::observe(realm, p, resident, opts);
    if account.is_none() && shares.is_none() {
        return None;
    }
    let mut out = Map::new();
    if let Some(account) = account {
        let today = day_of(realm);
        out.insert("balance".into(), json!(Bank::balance(realm, resident)));
        if let Some(loan) = &account.loan {
            let (principal, interest) = loan.next_instalment(realm.layout().days_per_year());
            out.insert(
                "loan".into(),
                json!({
                    "principal": loan.principal,
                    "rate_bp": loan.rate_bp,
                    "owed": loan.owed(),
                    "next_instalment": principal + interest,
                    "next_in_ticks": ticks_to_collection(realm, loan),
                    "days_left": loan.days_left,
                    "missed": loan.missed,
                    "held": collateral(realm, p, loan.principal),
                }),
            );
        }
        if today < account.bankrupt_until {
            out.insert("bankrupt_until".into(), calendar_day(realm, account.bankrupt_until));
        }
        if opts.is_full() {
            out.insert(
                "credit".into(),
                json!({
                    "limit": credit_limit(realm, p, account, today),
                    "seasoned_balance": account.seasoned(),
                    "loans_repaid": account.loans_repaid,
                    "defaults": account.defaults,
                }),
            );
        }
    }
    if let Some(shares) = shares {
        out.insert("shares".into(), shares);
    }
    Some(Value::Object(out))
}
