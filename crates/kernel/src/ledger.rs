//! The ledger: the only store of money and goods.
//!
//! Every realm has one ledger. A balance belongs to a [`Holder`]. Value
//! enters the world only through a named faucet (`mint`, `produce`) and
//! leaves it only through a named sink (`burn`, `consume`). Everything else
//! is a transfer, so the totals can be audited at any time:
//!
//! ```text
//! held == minted - burned + wired in - wired out
//! ```
//!
//! "Wired" value is value that moved between realms: a wire, or the holdings
//! of a resident that migrated.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use unsup_protocol::{ActionError, ErrorCode, ItemId, Money, OrgId, RealmId, ResidentId, SiteId};

use crate::codec::pairs;
use crate::ids::{FaucetId, SinkId};

/// The owner of a balance.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Holder {
    /// A resident. The balance moves with the resident between zones.
    Resident(ResidentId),
    /// A site: the till of a shop, the stock of a warehouse.
    Site(SiteId),
    /// An organization.
    Org(OrgId),
    /// A numbered account, for example a bank deposit. Account 0 is
    /// [`UNCLAIMED`].
    Account(u64),
    /// A numbered escrow, for example the stake of a contract.
    Escrow(u64),
}

/// The account of the global realm that receives value with no owner: a wire
/// to a resident that no longer exists, and the holdings of a resident that
/// an administrator removed. Modules decide what happens to it.
pub const UNCLAIMED: Holder = Holder::Account(0);

/// A ledger operation was refused. Nothing changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LedgerError {
    /// The holder does not have enough.
    Insufficient {
        /// The holder that would pay.
        holder: Holder,
        /// The amount that the operation needs.
        needed: i64,
        /// The amount that the holder has.
        available: i64,
    },
    /// The amount is negative.
    InvalidAmount,
    /// A balance or a total would not fit in an `i64`.
    Overflow,
    /// A wire names a destination that cannot hold a balance (all zones, or a
    /// zone that does not exist).
    BadDestination,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::Insufficient { holder, needed, available } => {
                write!(f, "{holder:?} has {available} but needs {needed}")
            }
            LedgerError::InvalidAmount => f.write_str("the amount is negative"),
            LedgerError::Overflow => f.write_str("the amount is too large"),
            LedgerError::BadDestination => f.write_str("the destination cannot hold a balance"),
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<LedgerError> for ActionError {
    fn from(error: LedgerError) -> ActionError {
        match error {
            LedgerError::Insufficient { needed, available, .. } => {
                ActionError::new(ErrorCode::Insufficient, "Not enough to pay for this.")
                    .with_details(serde_json::json!({ "needed": needed, "available": available }))
            }
            LedgerError::InvalidAmount => {
                ActionError::invalid_args("The amount must not be negative.")
            }
            LedgerError::Overflow => ActionError::invalid_args("The amount is too large."),
            LedgerError::BadDestination => {
                ActionError::internal("The destination cannot hold a balance.")
            }
        }
    }
}

/// The totals of a ledger do not match its balances. This is a bug in the
/// kernel or corrupted state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AuditError {
    /// The realm whose ledger failed, or `None` for the whole world.
    pub realm: Option<RealmId>,
    /// The item that failed, or `None` for money.
    pub item: Option<ItemId>,
    /// The sum of the balances.
    pub held: i64,
    /// What the sum must be: created minus destroyed plus received minus
    /// sent.
    pub expected: i64,
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let place = match self.realm {
            Some(realm) => realm.to_string(),
            None => "the world".to_string(),
        };
        match self.item {
            Some(item) => write!(
                f,
                "ledger audit failed in {place}: item {item} holds {} but must hold {}",
                self.held, self.expected
            ),
            None => write!(
                f,
                "ledger audit failed in {place}: money holds {} but must hold {}",
                self.held, self.expected
            ),
        }
    }
}

impl std::error::Error for AuditError {}

/// Totals of money in one ledger, in cents.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MoneyTotals {
    /// Sum of all balances.
    pub held: i64,
    /// Created through faucets.
    pub minted: i64,
    /// Destroyed through sinks.
    pub burned: i64,
    /// Received from other realms.
    pub wired_in: i64,
    /// Sent to other realms.
    pub wired_out: i64,
}

impl MoneyTotals {
    /// Returns what `held` must be.
    #[must_use]
    pub fn expected(&self) -> i64 {
        self.minted
            .wrapping_sub(self.burned)
            .wrapping_add(self.wired_in)
            .wrapping_sub(self.wired_out)
    }
}

/// Totals of one item in one ledger.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ItemTotals {
    /// Sum of all balances.
    pub held: i64,
    /// Created through faucets.
    pub produced: i64,
    /// Destroyed through sinks.
    pub consumed: i64,
    /// Received from other realms.
    pub wired_in: i64,
    /// Sent to other realms.
    pub wired_out: i64,
}

impl ItemTotals {
    /// Returns what `held` must be.
    #[must_use]
    pub fn expected(&self) -> i64 {
        self.produced
            .wrapping_sub(self.consumed)
            .wrapping_add(self.wired_in)
            .wrapping_sub(self.wired_out)
    }
}

/// Totals of one ledger.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LedgerTotals {
    /// Money.
    pub money: MoneyTotals,
    /// Every item that the ledger has ever seen.
    pub items: BTreeMap<ItemId, ItemTotals>,
}

/// How much moved under one reason label.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Flow {
    /// Number of transfers.
    pub count: u64,
    /// Sum of the amounts.
    pub amount: i64,
}

/// The store of money and goods of one realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(with = "pairs")]
    money: BTreeMap<Holder, i64>,
    #[serde(with = "pairs")]
    items: BTreeMap<(Holder, ItemId), i64>,
    #[serde(with = "pairs")]
    minted: BTreeMap<FaucetId, i64>,
    #[serde(with = "pairs")]
    burned: BTreeMap<SinkId, i64>,
    #[serde(with = "pairs")]
    produced: BTreeMap<(FaucetId, ItemId), i64>,
    #[serde(with = "pairs")]
    consumed: BTreeMap<(SinkId, ItemId), i64>,
    money_in: i64,
    money_out: i64,
    #[serde(with = "pairs")]
    item_in: BTreeMap<ItemId, i64>,
    #[serde(with = "pairs")]
    item_out: BTreeMap<ItemId, i64>,
    money_flows: BTreeMap<String, Flow>,
    item_flows: BTreeMap<String, Flow>,
}

/// Adds `delta` to a counter in a map of totals.
fn bump<K: Ord>(map: &mut BTreeMap<K, i64>, key: K, delta: i64) {
    let total = map.entry(key).or_insert(0);
    *total = total.wrapping_add(delta);
}

/// Sums amounts without a panic on overflow. The audit equation holds
/// modulo 2^64, so a wrapped sum still compares correctly.
fn wsum<'a>(values: impl Iterator<Item = &'a i64>) -> i64 {
    values.fold(0i64, |acc, &v| acc.wrapping_add(v))
}

/// Records one transfer under its reason label. The label is allocated only
/// the first time it is seen.
fn record(flows: &mut BTreeMap<String, Flow>, reason: &str, amount: i64) {
    if let Some(flow) = flows.get_mut(reason) {
        flow.count += 1;
        flow.amount = flow.amount.saturating_add(amount);
    } else {
        flows.insert(reason.to_string(), Flow { count: 1, amount });
    }
}

impl Ledger {
    /// Makes an empty ledger.
    #[must_use]
    pub fn new() -> Ledger {
        Ledger::default()
    }

    // ---- Reads ---------------------------------------------------------

    /// Returns the money of a holder.
    #[must_use]
    pub fn money(&self, holder: Holder) -> Money {
        Money(self.money.get(&holder).copied().unwrap_or(0))
    }

    /// Returns how many units of an item a holder has.
    #[must_use]
    pub fn qty(&self, holder: Holder, item: ItemId) -> i64 {
        self.items.get(&(holder, item)).copied().unwrap_or(0)
    }

    /// Returns the items of a holder, ordered by item id. Items with quantity
    /// zero are left out.
    #[must_use]
    pub fn items_of(&self, holder: Holder) -> Vec<(ItemId, i64)> {
        self.items
            .range((holder, ItemId(0))..=(holder, ItemId(u16::MAX)))
            .map(|(&(_, item), &qty)| (item, qty))
            .collect()
    }

    /// Returns every money balance that is not zero, ordered by holder.
    pub fn balances(&self) -> impl Iterator<Item = (Holder, Money)> + '_ {
        self.money.iter().map(|(&holder, &cents)| (holder, Money(cents)))
    }

    /// Returns every item balance that is not zero, ordered by holder and
    /// item.
    pub fn stocks(&self) -> impl Iterator<Item = (Holder, ItemId, i64)> + '_ {
        self.items.iter().map(|(&(holder, item), &qty)| (holder, item, qty))
    }

    /// Returns the money that a faucet created.
    #[must_use]
    pub fn minted_by(&self, faucet: FaucetId) -> Money {
        Money(self.minted.get(&faucet).copied().unwrap_or(0))
    }

    /// Returns the money that a sink destroyed.
    #[must_use]
    pub fn burned_by(&self, sink: SinkId) -> Money {
        Money(self.burned.get(&sink).copied().unwrap_or(0))
    }

    /// Returns how many units of an item a faucet created.
    #[must_use]
    pub fn produced_by(&self, faucet: FaucetId, item: ItemId) -> i64 {
        self.produced.get(&(faucet, item)).copied().unwrap_or(0)
    }

    /// Returns how many units of an item a sink destroyed.
    #[must_use]
    pub fn consumed_by(&self, sink: SinkId, item: ItemId) -> i64 {
        self.consumed.get(&(sink, item)).copied().unwrap_or(0)
    }

    /// Returns the money transfers per reason label, ordered by label.
    pub fn money_flows(&self) -> impl Iterator<Item = (&str, Flow)> + '_ {
        self.money_flows.iter().map(|(reason, &flow)| (reason.as_str(), flow))
    }

    /// Returns the item transfers per reason label, ordered by label.
    pub fn item_flows(&self) -> impl Iterator<Item = (&str, Flow)> + '_ {
        self.item_flows.iter().map(|(reason, &flow)| (reason.as_str(), flow))
    }

    // ---- Transfers -----------------------------------------------------

    /// Moves money between two holders of this ledger.
    ///
    /// `reason` is a short fixed label such as `wage` or `rent`. The ledger
    /// keeps the count and the sum per label, so do not put ids or names in
    /// it. An amount of zero does nothing. A failed transfer changes nothing.
    pub fn transfer_money(
        &mut self,
        from: Holder,
        to: Holder,
        amount: Money,
        reason: &str,
    ) -> Result<(), LedgerError> {
        let amount = amount.0;
        if amount < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.money.get(&from).copied().unwrap_or(0);
        if available < amount {
            return Err(LedgerError::Insufficient { holder: from, needed: amount, available });
        }
        if amount == 0 || from == to {
            return Ok(());
        }
        let target = self.money.get(&to).copied().unwrap_or(0);
        let new_target = target.checked_add(amount).ok_or(LedgerError::Overflow)?;
        self.set_money(from, available - amount);
        self.set_money(to, new_target);
        record(&mut self.money_flows, reason, amount);
        Ok(())
    }

    /// Moves units of an item between two holders of this ledger. The rules
    /// are those of [`Ledger::transfer_money`].
    pub fn transfer_item(
        &mut self,
        from: Holder,
        to: Holder,
        item: ItemId,
        qty: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        if qty < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.qty(from, item);
        if available < qty {
            return Err(LedgerError::Insufficient { holder: from, needed: qty, available });
        }
        if qty == 0 || from == to {
            return Ok(());
        }
        let new_target = self.qty(to, item).checked_add(qty).ok_or(LedgerError::Overflow)?;
        self.set_qty(from, item, available - qty);
        self.set_qty(to, item, new_target);
        record(&mut self.item_flows, reason, qty);
        Ok(())
    }

    // ---- Faucets and sinks ---------------------------------------------

    /// Creates money. An amount of zero or less creates nothing, and so does
    /// an amount that would not fit.
    pub fn mint(&mut self, faucet: FaucetId, to: Holder, amount: Money) {
        let amount = amount.0;
        if amount <= 0 {
            return;
        }
        let balance = self.money.get(&to).copied().unwrap_or(0);
        let total = self.minted.get(&faucet).copied().unwrap_or(0);
        if let (Some(balance), Some(total)) =
            (balance.checked_add(amount), total.checked_add(amount))
        {
            self.set_money(to, balance);
            self.minted.insert(faucet, total);
        }
    }

    /// Destroys money of a holder.
    pub fn burn(&mut self, from: Holder, sink: SinkId, amount: Money) -> Result<(), LedgerError> {
        let amount = amount.0;
        if amount < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.money.get(&from).copied().unwrap_or(0);
        if available < amount {
            return Err(LedgerError::Insufficient { holder: from, needed: amount, available });
        }
        if amount == 0 {
            return Ok(());
        }
        self.set_money(from, available - amount);
        bump(&mut self.burned, sink, amount);
        Ok(())
    }

    /// Creates units of an item. A quantity of zero or less creates nothing,
    /// and so does a quantity that would not fit.
    pub fn produce(&mut self, faucet: FaucetId, to: Holder, item: ItemId, qty: i64) {
        if qty <= 0 {
            return;
        }
        let balance = self.qty(to, item);
        let total = self.produced_by(faucet, item);
        if let (Some(balance), Some(total)) = (balance.checked_add(qty), total.checked_add(qty)) {
            self.set_qty(to, item, balance);
            self.produced.insert((faucet, item), total);
        }
    }

    /// Destroys units of an item of a holder.
    pub fn consume(
        &mut self,
        from: Holder,
        sink: SinkId,
        item: ItemId,
        qty: i64,
    ) -> Result<(), LedgerError> {
        if qty < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.qty(from, item);
        if available < qty {
            return Err(LedgerError::Insufficient { holder: from, needed: qty, available });
        }
        if qty == 0 {
            return Ok(());
        }
        self.set_qty(from, item, available - qty);
        bump(&mut self.consumed, (sink, item), qty);
        Ok(())
    }

    // ---- Totals and audit ----------------------------------------------

    /// Returns the totals of the ledger. The `held` numbers are computed
    /// from the balances.
    #[must_use]
    pub fn totals(&self) -> LedgerTotals {
        let money = MoneyTotals {
            held: wsum(self.money.values()),
            minted: wsum(self.minted.values()),
            burned: wsum(self.burned.values()),
            wired_in: self.money_in,
            wired_out: self.money_out,
        };
        let mut items: BTreeMap<ItemId, ItemTotals> = BTreeMap::new();
        for (&(_, item), &qty) in &self.items {
            let t = items.entry(item).or_default();
            t.held = t.held.wrapping_add(qty);
        }
        for (&(_, item), &qty) in &self.produced {
            let t = items.entry(item).or_default();
            t.produced = t.produced.wrapping_add(qty);
        }
        for (&(_, item), &qty) in &self.consumed {
            let t = items.entry(item).or_default();
            t.consumed = t.consumed.wrapping_add(qty);
        }
        for (&item, &qty) in &self.item_in {
            items.entry(item).or_default().wired_in = qty;
        }
        for (&item, &qty) in &self.item_out {
            items.entry(item).or_default().wired_out = qty;
        }
        LedgerTotals { money, items }
    }

    /// Checks that the balances match the totals:
    /// `held == minted - burned + wired in - wired out`, for money and for
    /// every item. The error has no realm; the caller adds it.
    pub fn audit(&self) -> Result<(), AuditError> {
        let held = wsum(self.money.values());
        let expected = wsum(self.minted.values())
            .wrapping_sub(wsum(self.burned.values()))
            .wrapping_add(self.money_in)
            .wrapping_sub(self.money_out);
        if held != expected {
            return Err(AuditError { realm: None, item: None, held, expected });
        }
        if self.items.is_empty()
            && self.produced.is_empty()
            && self.item_in.is_empty()
            && self.item_out.is_empty()
        {
            return Ok(());
        }
        // Net per item: balances minus what the totals explain. Item ids are
        // small, so a dense table is faster than a map.
        let mut net: Vec<i64> = Vec::new();
        let mut add = |item: ItemId, delta: i64| {
            let slot = usize::from(item.0);
            if slot >= net.len() {
                net.resize(slot + 1, 0);
            }
            net[slot] = net[slot].wrapping_add(delta);
        };
        for (&(_, item), &qty) in &self.items {
            add(item, qty);
        }
        for (&(_, item), &qty) in &self.produced {
            add(item, -qty);
        }
        for (&(_, item), &qty) in &self.consumed {
            add(item, qty);
        }
        for (&item, &qty) in &self.item_in {
            add(item, -qty);
        }
        for (&item, &qty) in &self.item_out {
            add(item, qty);
        }
        if let Some(slot) = net.iter().position(|&n| n != 0) {
            let item = ItemId(slot as u16);
            let totals = self.totals().items.get(&item).copied().unwrap_or_default();
            return Err(AuditError {
                realm: None,
                item: Some(item),
                held: totals.held,
                expected: totals.expected(),
            });
        }
        Ok(())
    }

    // ---- Kernel only ---------------------------------------------------

    fn set_money(&mut self, holder: Holder, cents: i64) {
        if cents == 0 {
            self.money.remove(&holder);
        } else {
            self.money.insert(holder, cents);
        }
    }

    fn set_qty(&mut self, holder: Holder, item: ItemId, qty: i64) {
        if qty == 0 {
            self.items.remove(&(holder, item));
        } else {
            self.items.insert((holder, item), qty);
        }
    }

    /// Takes money from a holder to send it to another realm.
    pub(crate) fn wire_out_money(
        &mut self,
        from: Holder,
        amount: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        if amount < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.money.get(&from).copied().unwrap_or(0);
        if available < amount {
            return Err(LedgerError::Insufficient { holder: from, needed: amount, available });
        }
        self.set_money(from, available - amount);
        self.money_out = self.money_out.wrapping_add(amount);
        if amount > 0 {
            record(&mut self.money_flows, reason, amount);
        }
        Ok(())
    }

    /// Gives a holder money that came from another realm.
    pub(crate) fn wire_in_money(&mut self, to: Holder, amount: i64) {
        let balance = self.money.get(&to).copied().unwrap_or(0);
        self.set_money(to, balance.saturating_add(amount));
        self.money_in = self.money_in.wrapping_add(amount);
    }

    /// Takes units of an item from a holder to send them to another realm.
    pub(crate) fn wire_out_item(
        &mut self,
        from: Holder,
        item: ItemId,
        qty: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        if qty < 0 {
            return Err(LedgerError::InvalidAmount);
        }
        let available = self.qty(from, item);
        if available < qty {
            return Err(LedgerError::Insufficient { holder: from, needed: qty, available });
        }
        self.set_qty(from, item, available - qty);
        if qty > 0 {
            bump(&mut self.item_out, item, qty);
            record(&mut self.item_flows, reason, qty);
        }
        Ok(())
    }

    /// Gives a holder units of an item that came from another realm.
    pub(crate) fn wire_in_item(&mut self, to: Holder, item: ItemId, qty: i64) {
        let balance = self.qty(to, item);
        self.set_qty(to, item, balance.saturating_add(qty));
        bump(&mut self.item_in, item, qty);
    }

    /// Removes everything that a holder has and counts it as sent to another
    /// realm. Returns the money and the items.
    pub(crate) fn take_holdings(&mut self, holder: Holder) -> (i64, Vec<(ItemId, i64)>) {
        let money = self.money.remove(&holder).unwrap_or(0);
        self.money_out = self.money_out.wrapping_add(money);
        let items = self.items_of(holder);
        for &(item, qty) in &items {
            self.items.remove(&(holder, item));
            bump(&mut self.item_out, item, qty);
        }
        (money, items)
    }

    /// Gives a holder money and items that came from another realm.
    pub(crate) fn put_holdings(&mut self, holder: Holder, money: i64, items: &[(ItemId, i64)]) {
        if money != 0 {
            self.wire_in_money(holder, money);
        }
        for &(item, qty) in items {
            self.wire_in_item(holder, item, qty);
        }
    }

    /// Moves everything that `from` has to `to` in this ledger.
    pub(crate) fn move_holdings(&mut self, from: Holder, to: Holder, reason: &str) {
        if from == to {
            return;
        }
        let money = self.money(from);
        // Neither transfer can fail: the amounts are the whole balance.
        let _ = self.transfer_money(from, to, money, reason);
        for (item, qty) in self.items_of(from) {
            let _ = self.transfer_item(from, to, item, qty, reason);
        }
    }

    /// Changes a balance without a faucet or a sink. Only tests call it, to
    /// prove that the audit sees a wrong balance.
    pub(crate) fn corrupt_money(&mut self, holder: Holder, delta: i64) {
        let balance = self.money.get(&holder).copied().unwrap_or(0);
        self.set_money(holder, balance + delta);
    }

    /// Changes an item balance without a faucet or a sink. Only tests call
    /// it.
    pub(crate) fn corrupt_item(&mut self, holder: Holder, item: ItemId, delta: i64) {
        let balance = self.qty(holder, item);
        self.set_qty(holder, item, balance + delta);
    }

    /// Returns an estimate of the heap bytes that the balances use.
    pub(crate) fn memory_estimate(&self) -> usize {
        self.money.len() * (size_of::<Holder>() + 8 + 8)
            + self.items.len() * (size_of::<Holder>() + 16 + 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANN: Holder = Holder::Resident(ResidentId(1));
    const BOB: Holder = Holder::Resident(ResidentId(2));
    const SHOP: Holder = Holder::Site(SiteId(9));
    const WAGES: FaucetId = FaucetId(0);
    const FARM: FaucetId = FaucetId(1);
    const TAX: SinkId = SinkId(0);
    const EATEN: SinkId = SinkId(1);
    const BREAD: ItemId = ItemId(3);
    const COAL: ItemId = ItemId(4);

    fn funded() -> Ledger {
        let mut ledger = Ledger::new();
        ledger.mint(WAGES, ANN, Money(1_000));
        ledger.produce(FARM, ANN, BREAD, 5);
        ledger
    }

    #[test]
    fn transfers_move_value_and_keep_the_total() {
        let mut ledger = funded();
        ledger.transfer_money(ANN, BOB, Money(300), "gift").unwrap();
        ledger.transfer_item(ANN, SHOP, BREAD, 2, "stock").unwrap();
        assert_eq!((ledger.money(ANN), ledger.money(BOB)), (Money(700), Money(300)));
        assert_eq!((ledger.qty(ANN, BREAD), ledger.qty(SHOP, BREAD)), (3, 2));
        assert_eq!(ledger.totals().money.held, 1_000);
        assert_eq!(ledger.totals().items[&BREAD].held, 5);
        assert_eq!(ledger.audit(), Ok(()));
        assert_eq!(ledger.items_of(ANN), [(BREAD, 3)]);
        assert_eq!(ledger.items_of(BOB), []);
    }

    #[test]
    fn a_failed_transfer_changes_nothing() {
        let mut ledger = funded();
        let before = ledger.clone();
        assert_eq!(
            ledger.transfer_money(ANN, BOB, Money(1_001), "gift"),
            Err(LedgerError::Insufficient { holder: ANN, needed: 1_001, available: 1_000 })
        );
        assert_eq!(
            ledger.transfer_money(ANN, BOB, Money(-1), "gift"),
            Err(LedgerError::InvalidAmount)
        );
        assert_eq!(
            ledger.transfer_item(ANN, BOB, BREAD, 6, "gift"),
            Err(LedgerError::Insufficient { holder: ANN, needed: 6, available: 5 })
        );
        assert!(ledger.transfer_item(ANN, BOB, COAL, 1, "gift").is_err());
        assert_eq!(
            ledger.transfer_item(ANN, BOB, BREAD, -2, "gift"),
            Err(LedgerError::InvalidAmount)
        );
        assert!(ledger.burn(BOB, TAX, Money(1)).is_err());
        assert_eq!(ledger.burn(ANN, TAX, Money(-1)), Err(LedgerError::InvalidAmount));
        assert!(ledger.consume(ANN, EATEN, BREAD, 9).is_err());
        assert_eq!(ledger.consume(ANN, EATEN, BREAD, -1), Err(LedgerError::InvalidAmount));
        assert_eq!(ledger, before);
    }

    #[test]
    fn overflow_is_refused() {
        let mut ledger = Ledger::new();
        ledger.mint(WAGES, ANN, Money(i64::MAX));
        ledger.mint(FARM, BOB, Money(10));
        // The faucet total of WAGES is full, so this mints nothing.
        ledger.mint(WAGES, BOB, Money(1));
        assert_eq!(ledger.money(BOB), Money(10));
        assert_eq!(ledger.transfer_money(BOB, ANN, Money(5), "gift"), Err(LedgerError::Overflow));
        assert_eq!(ledger.money(BOB), Money(10));
    }

    #[test]
    fn zero_and_self_transfers_do_nothing() {
        let mut ledger = funded();
        let before = ledger.clone();
        ledger.transfer_money(ANN, BOB, Money(0), "gift").unwrap();
        ledger.transfer_money(ANN, ANN, Money(10), "gift").unwrap();
        ledger.transfer_item(ANN, ANN, BREAD, 1, "gift").unwrap();
        ledger.mint(WAGES, BOB, Money(0));
        ledger.mint(WAGES, BOB, Money(-5));
        ledger.produce(FARM, BOB, BREAD, 0);
        ledger.burn(ANN, TAX, Money(0)).unwrap();
        assert_eq!(ledger, before);
        // A holder cannot give itself more than it has, even though nothing
        // would move.
        assert!(ledger.transfer_money(ANN, ANN, Money(5_000), "gift").is_err());
    }

    #[test]
    fn faucets_and_sinks_keep_their_own_totals() {
        let mut ledger = funded();
        ledger.mint(FARM, BOB, Money(50));
        ledger.burn(ANN, TAX, Money(100)).unwrap();
        ledger.consume(ANN, EATEN, BREAD, 2).unwrap();
        assert_eq!(ledger.minted_by(WAGES), Money(1_000));
        assert_eq!(ledger.minted_by(FARM), Money(50));
        assert_eq!(ledger.burned_by(TAX), Money(100));
        assert_eq!(ledger.burned_by(SinkId(7)), Money(0));
        assert_eq!(ledger.produced_by(FARM, BREAD), 5);
        assert_eq!(ledger.consumed_by(EATEN, BREAD), 2);
        let totals = ledger.totals();
        assert_eq!(
            totals.money,
            MoneyTotals { held: 950, minted: 1_050, burned: 100, wired_in: 0, wired_out: 0 }
        );
        assert_eq!(totals.money.expected(), 950);
        assert_eq!(
            totals.items[&BREAD],
            ItemTotals { held: 3, produced: 5, consumed: 2, wired_in: 0, wired_out: 0 }
        );
        assert_eq!(ledger.audit(), Ok(()));
    }

    #[test]
    fn wires_and_holdings_are_counted() {
        let mut a = funded();
        let mut b = Ledger::new();
        a.wire_out_money(ANN, 400, "deposit").unwrap();
        b.wire_in_money(SHOP, 400);
        a.wire_out_item(ANN, BREAD, 1, "ship").unwrap();
        b.wire_in_item(SHOP, BREAD, 1);
        assert!(a.wire_out_money(ANN, 601, "deposit").is_err());
        assert!(a.wire_out_item(ANN, BREAD, 5, "ship").is_err());
        assert_eq!(a.wire_out_money(ANN, -1, "deposit"), Err(LedgerError::InvalidAmount));
        assert_eq!(
            a.money_flows().collect::<Vec<_>>(),
            [("deposit", Flow { count: 1, amount: 400 })]
        );

        let (money, items) = a.take_holdings(ANN);
        assert_eq!((money, items.clone()), (600, vec![(BREAD, 4)]));
        b.put_holdings(ANN, money, &items);
        assert_eq!((a.money(ANN), a.qty(ANN, BREAD)), (Money(0), 0));
        assert_eq!((b.money(ANN), b.qty(ANN, BREAD), b.money(SHOP)), (Money(600), 4, Money(400)));
        assert_eq!(a.audit(), Ok(()));
        assert_eq!(b.audit(), Ok(()));
        assert_eq!(a.totals().money.wired_out, 1_000);
        assert_eq!(b.totals().money.wired_in, 1_000);
        assert_eq!(b.totals().items[&BREAD].wired_in, 5);
        // Across the two ledgers nothing was created or lost.
        assert_eq!(a.totals().money.held + b.totals().money.held, 1_000);

        b.move_holdings(ANN, BOB, "inherit");
        assert_eq!((b.money(BOB), b.qty(BOB, BREAD), b.money(ANN)), (Money(600), 4, Money(0)));
        assert_eq!(b.audit(), Ok(()));
    }

    #[test]
    fn audit_sees_a_wrong_balance() {
        let mut ledger = funded();
        ledger.corrupt_money(BOB, 1);
        assert_eq!(
            ledger.audit(),
            Err(AuditError { realm: None, item: None, held: 1_001, expected: 1_000 })
        );
        ledger.corrupt_money(BOB, -1);
        assert_eq!(ledger.audit(), Ok(()));
        ledger.corrupt_item(BOB, BREAD, -2);
        assert_eq!(
            ledger.audit(),
            Err(AuditError { realm: None, item: Some(BREAD), held: 3, expected: 5 })
        );
        ledger.corrupt_item(BOB, BREAD, 2);
        ledger.corrupt_item(BOB, COAL, 1);
        let error = ledger.audit().unwrap_err();
        assert_eq!((error.item, error.held, error.expected), (Some(COAL), 1, 0));
        assert!(error.to_string().contains("item 4 holds 1 but must hold 0"), "{error}");
    }

    #[test]
    fn reasons_are_totalled() {
        let mut ledger = funded();
        ledger.transfer_money(ANN, BOB, Money(100), "rent").unwrap();
        ledger.transfer_money(ANN, BOB, Money(50), "rent").unwrap();
        ledger.transfer_money(BOB, ANN, Money(10), "gift").unwrap();
        ledger.transfer_item(ANN, BOB, BREAD, 2, "gift").unwrap();
        let flows: Vec<_> = ledger.money_flows().collect();
        assert_eq!(
            flows,
            [("gift", Flow { count: 1, amount: 10 }), ("rent", Flow { count: 2, amount: 150 })]
        );
        assert_eq!(
            ledger.item_flows().collect::<Vec<_>>(),
            [("gift", Flow { count: 1, amount: 2 })]
        );
        assert_eq!(ledger.balances().collect::<Vec<_>>(), [(ANN, Money(860)), (BOB, Money(140))]);
        assert_eq!(ledger.stocks().collect::<Vec<_>>(), [(ANN, BREAD, 3), (BOB, BREAD, 2)]);
    }

    #[test]
    fn ledger_survives_serialization() {
        let mut ledger = funded();
        ledger.transfer_money(ANN, SHOP, Money(250), "purchase").unwrap();
        ledger.burn(ANN, TAX, Money(10)).unwrap();
        ledger.wire_out_item(ANN, BREAD, 1, "ship").unwrap();
        let bytes = crate::codec::encode("ledger", &ledger);
        let back: Ledger = crate::codec::decode("ledger", &bytes).unwrap();
        assert_eq!(back, ledger);
        // Empty balances are never stored, so equal states have equal bytes.
        let mut other = funded();
        other.transfer_money(ANN, BOB, Money(5), "x").unwrap();
        other.transfer_money(BOB, ANN, Money(5), "x").unwrap();
        assert_eq!(other.balances().count(), 1);
    }

    #[test]
    fn ledger_errors_become_action_errors() {
        let error: ActionError =
            LedgerError::Insufficient { holder: ANN, needed: 500, available: 120 }.into();
        assert_eq!(error.code, ErrorCode::Insufficient);
        assert_eq!(error.details, serde_json::json!({ "needed": 500, "available": 120 }));
        assert_eq!(ActionError::from(LedgerError::InvalidAmount).code, ErrorCode::InvalidArgs);
        assert_eq!(ActionError::from(LedgerError::BadDestination).code, ErrorCode::Internal);
    }
}
