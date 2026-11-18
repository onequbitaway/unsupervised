//! The daily publication of the statistics.

use std::collections::BTreeMap;

use unsup_kernel::{EventMeta, Holder, Realm, UNCLAIMED};

use super::gini::gini_bp;
use super::state::{FlowStat, LoanStats, MoneyParts, Published, Stats};
use super::zone::{consumed, produced};
use crate::bank::Bank;
use crate::params::Params;
use crate::util::{day_of, drama};

/// Publishes the statistics once for every day that ended. A global
/// system calls it every tick, after the bank did its daily work.
pub(crate) fn run(realm: &Realm, p: &Params) {
    let today = day_of(realm);
    if realm.res::<Stats>().day >= today {
        return;
    }
    let mut stats = realm.res_mut::<Stats>();
    stats.day = today;
    let published = build(realm, p, &mut stats, today);
    realm.emit(
        EventMeta::public("finance.stats_published", drama(realm, p.drama_routine)),
        &published,
    );
    stats.latest = Some(published);
}

/// Adds the totals of the zones to the totals of the global ledger and
/// gives each the flow since the publication before. A faucet or a sink
/// with a total of zero is left out: the kernel has one list of names for
/// money and for goods, and most names are used for only one of them.
fn flows(
    own: impl Iterator<Item = (u16, String, i64)>,
    zones: impl Iterator<Item = (u16, i64)>,
    before: Option<&BTreeMap<String, FlowStat>>,
) -> BTreeMap<String, FlowStat> {
    let mut by_id: BTreeMap<u16, (String, i64)> =
        own.map(|(id, name, amount)| (id, (name, amount))).collect();
    for (id, amount) in zones {
        if let Some(entry) = by_id.get_mut(&id) {
            entry.1 = entry.1.saturating_add(amount);
        }
    }
    by_id
        .into_values()
        .filter(|&(_, total)| total != 0)
        .map(|(name, total)| {
            let earlier = before.and_then(|map| map.get(&name)).map_or(0, |flow| flow.total);
            (name, FlowStat { total, day: total - earlier })
        })
        .collect()
}

fn build(realm: &Realm, p: &Params, stats: &mut Stats, today: u64) -> Published {
    let ledger = realm.ledger();
    let defs = realm.defs();
    let bank = realm.res::<Bank>();

    // Where the money is: the global ledger now, and the zones as they
    // last reported.
    let mut money = MoneyParts::default();
    for (holder, cents) in ledger.balances() {
        let part = match holder {
            Holder::Resident(_) => &mut money.cash,
            Holder::Site(_) => &mut money.tills,
            Holder::Org(_) => &mut money.treasuries,
            Holder::Escrow(_) => &mut money.escrow,
            holder if holder == UNCLAIMED => &mut money.other,
            Holder::Account(_) => &mut money.deposits,
        };
        *part += cents.0;
    }
    for report in stats.zones.values() {
        money.cash += report.cash;
        money.tills += report.tills;
        money.escrow += report.escrow;
        money.other += report.other;
    }

    // The flow through each faucet and sink. The money supply is what the
    // faucets created minus what the sinks destroyed, so money in messages
    // between realms is in it.
    let before = stats.latest.as_ref();
    let faucets = flows(
        defs.faucets().map(|(id, name)| (id.0, name.to_string(), ledger.minted_by(id).0)),
        stats.zones.values().flat_map(|report| report.minted.iter().copied()),
        before.map(|published| &published.faucets),
    );
    let sinks = flows(
        defs.sinks().map(|(id, name)| (id.0, name.to_string(), ledger.burned_by(id).0)),
        stats.zones.values().flat_map(|report| report.burned.iter().copied()),
        before.map(|published| &published.sinks),
    );
    let goods_faucets = flows(
        defs.faucets().map(|(id, name)| (id.0, name.to_string(), produced(&ledger, defs, id))),
        stats.zones.values().flat_map(|report| report.produced.iter().copied()),
        before.map(|published| &published.goods_faucets),
    );
    let goods_sinks = flows(
        defs.sinks().map(|(id, name)| (id.0, name.to_string(), consumed(&ledger, defs, id))),
        stats.zones.values().flat_map(|report| report.consumed.iter().copied()),
        before.map(|published| &published.goods_sinks),
    );
    let created: i64 = faucets.values().map(|flow| flow.total).sum();
    let destroyed: i64 = sinks.values().map(|flow| flow.total).sum();
    let money_supply = created - destroyed;
    money.in_transit = money_supply
        - (money.cash
            + money.deposits
            + money.treasuries
            + money.tills
            + money.escrow
            + money.other);

    // Wealth of each resident: cash in its zone plus its bank deposit. The
    // cash lists come with the zone reports of this frame, so each
    // resident is in one list. They are used once and not kept.
    let frame = realm.frame();
    let mut wealth: Vec<i64> = Vec::new();
    for report in stats.zones.values_mut() {
        let residents = std::mem::take(&mut report.residents);
        if report.frame == frame {
            wealth.extend(
                residents
                    .iter()
                    .map(|&(resident, cents)| cents + ledger.money(Holder::Account(resident)).0),
            );
        }
    }
    let gini = gini_bp(&mut wealth);

    let (prices, price_index_bp) = stats.close_prices(defs);
    Published {
        day: today,
        money_supply,
        money,
        price_index_bp,
        prices,
        gini_bp: gini,
        residents: wealth.len() as u64,
        faucets,
        sinks,
        goods_faucets,
        goods_sinks,
        loans: LoanStats {
            count: bank.loan_count() as u64,
            outstanding: bank.outstanding_principal(),
            written_off: bank.written_off(),
        },
        base_rate_bp: realm.param(p.base_rate_bp),
    }
}
