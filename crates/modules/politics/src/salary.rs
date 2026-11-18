//! Salaries. At the start of each game day every government pays its
//! officials for the day that ended. No funds, no pay.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, EventMeta, Holder, Realm, Scope, Stage};

use crate::calendar;
use crate::model::Gov;
use crate::offices;
use crate::params::{self, Params};
use crate::pay;
use crate::state::Politics;

/// Ledger label of a salary.
const SALARY: &str = "politics.salary";

/// Order of the salary system among the systems of the module. It runs
/// after the calendar, so that the winners of an election that ends at
/// midnight get the pay of their first day one day later.
const ORDER_SALARIES: i32 = 30;

/// Pays the officials of one government from its treasury: the elected
/// offices first, then the posts, each in id order. An official whose
/// salary the treasury cannot pay in full gets nothing for the day.
fn pay_government(realm: &Realm, p: &Params, gov: Gov) {
    let treasury = Holder::Org(gov.org());
    let (mut paid, mut unpaid, mut total) = (0_u32, 0_u32, 0_i64);
    for seat in offices::seats(realm, p, gov) {
        if seat.salary <= 0 {
            continue;
        }
        for holder in offices::holders(realm, seat.org, seat.role) {
            let facts = gov.describe_with(json!({ "office": seat.name, "amount": seat.salary }));
            if pay::wire_exact(realm, treasury, holder, seat.salary, SALARY) {
                paid += 1;
                total += seat.salary;
                realm.notify(holder, "politics.salary_paid", None, facts);
            } else {
                unpaid += 1;
                realm.notify(holder, "politics.salary_unpaid", None, facts);
            }
        }
    }
    if paid + unpaid == 0 {
        return;
    }
    let drama = if unpaid > 0 { p.drama_salaries_unpaid } else { p.drama_routine };
    realm.emit(
        EventMeta::public("politics.salaries_paid", params::drama(realm, drama)),
        &gov.describe_with(json!({
            "paid": paid,
            "unpaid": unpaid,
            "total": total,
            "treasury": realm.ledger().money(treasury),
        })),
    );
}

/// Pays the salaries once for every new day. A global system calls it
/// every frame.
fn run(realm: &Realm, p: &Params) {
    let today = calendar::today(realm);
    {
        let mut state = realm.res_mut::<Politics>();
        if state.salary_day >= today {
            return;
        }
        state.salary_day = today;
    }
    for gov in offices::governments(realm) {
        pay_government(realm, p, gov);
    }
}

/// Registers the system.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.system(Stage::Update, ORDER_SALARIES, "politics.salaries", move |realm| run(realm, &p));
    }
}
