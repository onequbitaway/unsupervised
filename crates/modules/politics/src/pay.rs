//! Payments from the ledger of the global realm.

use unsup_kernel::{Dest, Holder, Money, Realm, ResidentId};

/// Moves money between two holders of the global realm. It moves at most
/// what the holder has, and returns the cents moved.
pub(crate) fn transfer(realm: &Realm, from: Holder, to: Holder, amount: i64, reason: &str) -> i64 {
    let amount = amount.min(realm.ledger().money(from).cents());
    if amount <= 0 {
        return 0;
    }
    match realm.ledger_mut().transfer_money(from, to, Money(amount), reason) {
        Ok(()) => amount,
        Err(_) => 0,
    }
}

/// Sends exactly `amount` cents from a holder of the global realm to the
/// cash of a resident, wherever it is. It sends nothing when the holder has
/// less. Returns true when the money left.
///
/// The kernel gives a wire for a resident that no longer exists to the
/// unclaimed account, so no money is lost.
pub(crate) fn wire_exact(
    realm: &Realm,
    from: Holder,
    to: ResidentId,
    amount: i64,
    reason: &str,
) -> bool {
    if amount <= 0 || realm.ledger().money(from).cents() < amount {
        return false;
    }
    realm.wire_money(from, Dest::Resident(to), Holder::Resident(to), Money(amount), reason).is_ok()
}
