//! The zone side of the statistics: a report of the ledger of the zone.

use unsup_kernel::{Blueprint, FaucetId, Holder, Ledger, Realm, SinkId};

use super::state::ZoneReport;

/// Returns the units of goods that a faucet has created in a ledger, all
/// items together.
pub(crate) fn produced(ledger: &Ledger, defs: &Blueprint, faucet: FaucetId) -> i64 {
    defs.items().fold(0i64, |sum, (item, _)| sum.saturating_add(ledger.produced_by(faucet, item)))
}

/// Returns the units of goods that a sink has destroyed in a ledger, all
/// items together.
pub(crate) fn consumed(ledger: &Ledger, defs: &Blueprint, sink: SinkId) -> i64 {
    defs.items().fold(0i64, |sum, (item, _)| sum.saturating_add(ledger.consumed_by(sink, item)))
}

/// Reads the ledger of a zone into a report. With `with_residents`, the
/// report also lists the cash of every resident of the zone.
pub(crate) fn report(realm: &Realm, with_residents: bool) -> ZoneReport {
    let ledger = realm.ledger();
    let defs = realm.defs();
    let mut report = ZoneReport { frame: realm.frame(), ..ZoneReport::default() };
    for (holder, money) in ledger.balances() {
        let part = match holder {
            Holder::Resident(_) => &mut report.cash,
            Holder::Site(_) => &mut report.tills,
            Holder::Escrow(_) => &mut report.escrow,
            Holder::Org(_) | Holder::Account(_) => &mut report.other,
        };
        *part += money.0;
    }
    report.minted = defs
        .faucets()
        .map(|(faucet, _)| (faucet.0, ledger.minted_by(faucet).0))
        .filter(|&(_, cents)| cents != 0)
        .collect();
    report.burned = defs
        .sinks()
        .map(|(sink, _)| (sink.0, ledger.burned_by(sink).0))
        .filter(|&(_, cents)| cents != 0)
        .collect();
    report.produced = defs
        .faucets()
        .map(|(faucet, _)| (faucet.0, produced(&ledger, defs, faucet)))
        .filter(|&(_, units)| units != 0)
        .collect();
    report.consumed = defs
        .sinks()
        .map(|(sink, _)| (sink.0, consumed(&ledger, defs, sink)))
        .filter(|&(_, units)| units != 0)
        .collect();
    if with_residents {
        report.residents =
            realm.ids().iter().map(|&id| (id.0, ledger.money(Holder::Resident(id)).0)).collect();
    }
    report
}
