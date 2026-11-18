//! Payments from the ledger of the global realm.

use unsup_kernel::{Dest, Holder, Money, Realm, ResidentId};

/// Ledger label of cash on its way from a poster to the holding escrow of
/// its request.
pub(crate) const HOLD: &str = "missions.hold";
/// Ledger label of cash that comes back from the holding escrow of a
/// request.
pub(crate) const RETURN: &str = "missions.return";
/// Ledger label of the reward that goes into the escrow of a mission.
pub(crate) const ESCROW: &str = "missions.escrow";
/// Ledger label of a reward that a taker gets.
pub(crate) const REWARD: &str = "missions.reward";
/// Ledger label of the unpaid reward that goes back to the poster.
pub(crate) const REFUND: &str = "missions.refund";

/// Sends money from a holder of the global realm to a resident, wherever it
/// is. It sends at most what the holder has, and returns the cents sent.
///
/// The kernel gives a wire for a resident that no longer exists to the
/// unclaimed account, so no money is lost.
pub(crate) fn wire_to_resident(
    realm: &Realm,
    from: Holder,
    to: ResidentId,
    amount: i64,
    reason: &str,
) -> i64 {
    let amount = amount.min(realm.ledger().money(from).cents());
    if amount <= 0 {
        return 0;
    }
    match realm.wire_money(from, Dest::Resident(to), Holder::Resident(to), Money(amount), reason) {
        Ok(()) => amount,
        Err(_) => 0,
    }
}

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

/// Splits the reward of a team between the members that added events, in
/// proportion to the events of each one. No cent is lost: the cents that
/// the division leaves go one by one to the members with the most events,
/// and among equals to the lowest id.
///
/// `members` holds the events of each member, ordered by resident. Members
/// without events get nothing and are not in the result. The result is
/// ordered by resident.
pub(crate) fn split(reward: i64, members: &[(ResidentId, u32)]) -> Vec<(ResidentId, i64)> {
    let total: i128 = members.iter().map(|&(_, events)| i128::from(events)).sum();
    if total == 0 || reward <= 0 {
        return Vec::new();
    }
    let mut shares: Vec<(ResidentId, u32, i64)> = members
        .iter()
        .filter(|&&(_, events)| events > 0)
        .map(|&(resident, events)| {
            let share = i128::from(reward) * i128::from(events) / total;
            (resident, events, i64::try_from(share).unwrap_or(0))
        })
        .collect();
    let paid: i64 = shares.iter().map(|&(_, _, share)| share).sum();
    // The rest is less than the number of members with events.
    let mut rest = reward - paid;
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(shares[i].1), shares[i].0));
    for i in order {
        if rest == 0 {
            break;
        }
        shares[i].2 += 1;
        rest -= 1;
    }
    shares.into_iter().map(|(resident, _, share)| (resident, share)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(events: &[u32]) -> Vec<(ResidentId, u32)> {
        events.iter().enumerate().map(|(i, &e)| (ResidentId(i as u64 + 1), e)).collect()
    }

    fn amounts(reward: i64, events: &[u32]) -> Vec<(u64, i64)> {
        split(reward, &members(events)).into_iter().map(|(r, cents)| (r.0, cents)).collect()
    }

    #[test]
    fn a_split_follows_the_events_and_loses_no_cent() {
        // 3 of 4 events and 1 of 4 events.
        assert_eq!(amounts(1_000, &[3, 1]), [(1, 750), (2, 250)]);
        // 100 / 3 leaves one cent: the lowest id among equals gets it.
        assert_eq!(amounts(100, &[1, 1, 1]), [(1, 34), (2, 33), (3, 33)]);
        // 101 * 2 / 7 = 28 and 101 * 5 / 7 = 72: one cent is left, and the
        // member with the most events gets it.
        assert_eq!(amounts(101, &[2, 5]), [(1, 28), (2, 73)]);
        // A member without events gets nothing and is not listed.
        assert_eq!(amounts(500, &[0, 4, 0]), [(2, 500)]);
        // Two cents are left: the two members with the most events.
        assert_eq!(amounts(8, &[1, 1, 1]), [(1, 3), (2, 3), (3, 2)]);
    }

    #[test]
    fn a_split_of_any_reward_adds_up() {
        for reward in [1, 2, 7, 99, 100, 12_345, i64::MAX] {
            for events in [&[1u32][..], &[1, 2, 3], &[5, 0, 5, 1], &[1_000, 1], &[u32::MAX, 7]] {
                let shares = split(reward, &members(events));
                let sum: i128 = shares.iter().map(|&(_, cents)| i128::from(cents)).sum();
                assert_eq!(sum, i128::from(reward), "{reward} over {events:?}");
                assert!(shares.iter().all(|&(_, cents)| cents >= 0));
            }
        }
    }

    #[test]
    fn nothing_to_split() {
        assert_eq!(amounts(100, &[0, 0]), []);
        assert_eq!(amounts(0, &[1, 2]), []);
        assert_eq!(amounts(100, &[]), []);
    }
}
