//! Public statistics: money supply, price index, wealth inequality and the
//! flow of money and of goods through each faucet and sink.
//!
//! Each zone reports its ledger totals to the global realm on a period.
//! The global realm publishes the event `finance.stats_published` once per
//! game day.

mod gini;
mod prices;
mod publish;
mod state;
mod zone;

use unsup_kernel::{Builder, Dest, Scope, Stage, TICKS_PER_DAY};

pub use self::gini::gini_bp;
pub use self::state::{
    FlowStat, LoanStats, MoneyParts, PricePoint, PriceTrack, Published, Stats, ZoneReport,
};
use crate::params::Params;

/// Registers the statistics.
pub(crate) fn register(b: &mut Builder, p: Params) {
    match b.scope() {
        Scope::Zone => {
            // Every zone reports in the same frames, so that the reports
            // of one frame count every resident exactly once. The report
            // at the start of a day also lists the cash of each resident,
            // for the Gini coefficient.
            b.system(Stage::Post, 100, "finance.zone_report", move |realm| {
                let frame = realm.frame();
                let period = u64::try_from(realm.param(p.stats_report_period).max(1)).unwrap_or(1);
                let day_start = frame.is_multiple_of(TICKS_PER_DAY);
                if day_start || frame.is_multiple_of(period) {
                    realm.send(Dest::Global, &zone::report(realm, day_start));
                }
            });
        }
        Scope::Global => {
            b.resource::<Stats>();
            b.on_message::<ZoneReport>(|realm, from, report| {
                if let Some(zone) = from.zone() {
                    realm.res_mut::<Stats>().zones.insert(zone.0, report);
                }
            });
            b.on_event("economy.goods_sold", |realm, event| {
                realm.res_mut::<Stats>().record_event(realm.defs(), event, "unit_price");
            });
            b.on_event("finance.order_filled", |realm, event| {
                realm.res_mut::<Stats>().record_event(realm.defs(), event, "price");
            });
            b.system(Stage::Post, 100, "finance.stats_publish", move |realm| {
                publish::run(realm, &p);
            });
        }
    }
}
