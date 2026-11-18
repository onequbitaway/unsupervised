//! The state and the published form of the statistics.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Message, Resource};

/// What one zone tells the statistics about its ledger. The zone sends it
/// to the global realm on a period.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneReport {
    /// The frame in which the zone made the report.
    pub frame: u64,
    /// Cash of the residents in the zone, in cents.
    pub cash: i64,
    /// Money of the sites in the zone (tills), in cents.
    pub tills: i64,
    /// Money in escrows of the zone, in cents.
    pub escrow: i64,
    /// Money of every other holder in the zone, in cents.
    pub other: i64,
    /// Money that each faucet has created in the zone: faucet id and cents.
    pub minted: Vec<(u16, i64)>,
    /// Money that each sink has destroyed in the zone: sink id and cents.
    pub burned: Vec<(u16, i64)>,
    /// Goods that each faucet has created in the zone: faucet id and
    /// units, all items together.
    pub produced: Vec<(u16, i64)>,
    /// Goods that each sink has destroyed in the zone: sink id and units,
    /// all items together.
    pub consumed: Vec<(u16, i64)>,
    /// The cash of every resident in the zone: resident id and cents. Only
    /// the report at the start of a day has this list, and the statistics
    /// drop it after use.
    pub residents: Vec<(u64, i64)>,
}

impl Message for ZoneReport {
    const KIND: &'static str = "finance.zone_report";
}

/// The prices of one item that the statistics have seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceTrack {
    /// The mean price of the first day on which the item was sold. The
    /// price index compares later prices with it.
    pub reference: i64,
    /// The mean price of the last day on which the item was sold.
    pub last: i64,
    /// Cents paid for the item since the last publication.
    pub day_value: i64,
    /// Units sold since the last publication.
    pub day_qty: i64,
}

/// Where the money is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoneyParts {
    /// Cash of residents.
    pub cash: i64,
    /// Bank deposits.
    pub deposits: i64,
    /// Treasuries of organizations.
    pub treasuries: i64,
    /// Tills of sites.
    pub tills: i64,
    /// Escrows: orders, contracts, missions, estates.
    pub escrow: i64,
    /// Every other holder, for example the unclaimed account.
    pub other: i64,
    /// Money in messages between realms, and the error of zone reports
    /// that are not from this frame.
    pub in_transit: i64,
}

/// The price of one item in the published statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PricePoint {
    /// The mean price of the last day on which the item was sold.
    pub last: i64,
    /// The mean price of the first day on which the item was sold.
    pub reference: i64,
    /// True when the item is in the basket of the price index.
    pub basket: bool,
}

/// The flow through one faucet or one sink: cents for money, units for
/// goods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowStat {
    /// The flow since the world began.
    pub total: i64,
    /// The flow since the publication before this one.
    pub day: i64,
}

/// The loans of the bank in the published statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoanStats {
    /// Number of loans.
    pub count: u64,
    /// Principal that is not yet repaid, in cents.
    pub outstanding: i64,
    /// Principal that the bank gave up after defaults, since the world
    /// began, in cents.
    pub written_off: i64,
}

/// The public statistics of one game day: the data of the event
/// `finance.stats_published`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Published {
    /// Number of game days that have ended since the world began.
    pub day: u64,
    /// All money in the world, in cents: created minus destroyed.
    pub money_supply: i64,
    /// Where the money is.
    pub money: MoneyParts,
    /// The price index in basis points. 10,000 is the level of the
    /// reference prices.
    pub price_index_bp: i64,
    /// The prices of every item that was ever sold, by item name.
    pub prices: BTreeMap<String, PricePoint>,
    /// The Gini coefficient of the wealth of residents in basis points: 0
    /// when everybody has the same, 10,000 when one resident has all.
    pub gini_bp: i64,
    /// Number of residents that the Gini coefficient counts.
    pub residents: u64,
    /// Money that each faucet has created, by faucet name. A faucet that
    /// never created money is absent.
    pub faucets: BTreeMap<String, FlowStat>,
    /// Money that each sink has destroyed, by sink name. A sink that never
    /// destroyed money is absent.
    pub sinks: BTreeMap<String, FlowStat>,
    /// Units of goods that each faucet has created, all items together, by
    /// faucet name. A faucet that never created goods is absent.
    pub goods_faucets: BTreeMap<String, FlowStat>,
    /// Units of goods that each sink has destroyed, all items together, by
    /// sink name. A sink that never destroyed goods is absent.
    pub goods_sinks: BTreeMap<String, FlowStat>,
    /// The loans of the bank.
    pub loans: LoanStats,
    /// The yearly base rate of the central bank in basis points.
    pub base_rate_bp: i64,
}

/// The statistics. They live in the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// The newest report of each zone, by zone id.
    pub(crate) zones: BTreeMap<u32, ZoneReport>,
    /// The prices of each item, by item name.
    pub(crate) prices: BTreeMap<String, PriceTrack>,
    /// The last day whose statistics were published.
    pub(crate) day: u64,
    /// The statistics that were published last.
    pub(crate) latest: Option<Published>,
}

impl Resource for Stats {
    const NAME: &'static str = "finance.stats";
}

impl Stats {
    /// Returns the statistics that were published last, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&Published> {
        self.latest.as_ref()
    }

    /// Returns the newest report of a zone, if it sent one.
    #[must_use]
    pub fn zone_report(&self, zone: u32) -> Option<&ZoneReport> {
        self.zones.get(&zone)
    }

    /// Returns the prices that the statistics have seen for an item.
    #[must_use]
    pub fn price(&self, item: &str) -> Option<&PriceTrack> {
        self.prices.get(item)
    }
}
