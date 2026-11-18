//! The election calendar. It follows `realm.clock()`, so it counts frames:
//! time dilation of a zone does not move a deadline.
//!
//! A *day* in this module is the number of a game day of the world, from 0:
//! day 0 is day 1 of year 1.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Realm, TICKS_PER_DAY, Value};

use crate::model::Gov;
use crate::params::{self, Params};

/// Game minutes in one game day.
pub(crate) const DAY: u64 = TICKS_PER_DAY;

/// Returns the number of the current game day, from 0.
pub(crate) fn today(realm: &Realm) -> u64 {
    let clock = realm.clock();
    let years = u64::from(clock.year.saturating_sub(1));
    years * u64::from(realm.layout().days_per_year()) + u64::from(clock.day.saturating_sub(1))
}

/// Returns the frame at which a day starts.
pub(crate) fn day_start(day: u64) -> u64 {
    day.saturating_mul(DAY)
}

/// Returns a day as residents see it in the clock: `year` and `day`, both
/// from 1.
pub(crate) fn date(realm: &Realm, day: u64) -> Value {
    let clock = realm.layout().clock_at(day_start(day));
    json!({ "year": clock.year, "day": clock.day })
}

/// Returns a frame as residents see it in the clock.
pub(crate) fn moment(realm: &Realm, frame: u64) -> Value {
    let clock = realm.layout().clock_at(frame);
    json!({ "year": clock.year, "day": clock.day, "hour": clock.hour, "minute": clock.minute })
}

/// Returns the game minutes from now to a frame, or 0 when the frame is not
/// in the future.
pub(crate) fn minutes_left(realm: &Realm, frame: u64) -> u64 {
    frame.saturating_sub(realm.frame())
}

/// The windows of one election, in frames. Each window ends where the next
/// one starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Windows {
    /// Registration opens.
    pub(crate) opens: u64,
    /// Registration closes and the campaign starts.
    pub(crate) campaign_from: u64,
    /// The campaign ends and the polls open.
    pub(crate) polls_open: u64,
    /// The polls close and the votes are counted.
    pub(crate) polls_close: u64,
}

/// Returns the windows of an election of a government with the given voting
/// day, from the parameters as they are now.
pub(crate) fn windows(realm: &Realm, p: &Params, gov: Gov, vote_day: u64) -> Windows {
    let start = day_start(vote_day);
    let open = params::amount(realm, p.polls_open_minute, gov);
    let close = params::amount(realm, p.polls_close_minute, gov).max(open);
    let campaign = params::amount(realm, p.campaign_days, gov);
    let registration = params::amount(realm, p.registration_days, gov);
    let campaign_from = start.saturating_sub(campaign.saturating_mul(DAY));
    Windows {
        opens: campaign_from.saturating_sub(registration.saturating_mul(DAY)),
        campaign_from,
        polls_open: start.saturating_add(open),
        polls_close: start.saturating_add(close),
    }
}

/// Returns the number of days of the term of a government, at least one.
pub(crate) fn term_days(realm: &Realm, p: &Params, gov: Gov) -> u64 {
    let param = match gov {
        Gov::Nation => p.nation_term_days,
        Gov::City(_) => p.city_term_days,
    };
    params::amount(realm, param, gov).max(1)
}

/// Returns the first regular voting day of a government that is not before
/// `today`: the first voting day of the data file, then one each term.
pub(crate) fn first_vote_day(realm: &Realm, p: &Params, gov: Gov, today: u64) -> u64 {
    let param = match gov {
        Gov::Nation => p.nation_first_vote_day,
        Gov::City(_) => p.city_first_vote_day,
    };
    // The parameter counts days from 1, like the clock.
    let first = params::amount(realm, param, gov).saturating_sub(1);
    if first >= today {
        return first;
    }
    let term = term_days(realm, p, gov);
    first + (today - first).div_ceil(term) * term
}

/// Returns the voting day of an early election that is called today: the
/// first day that leaves the rest of today and the whole registration and
/// campaign windows before it.
pub(crate) fn early_vote_day(realm: &Realm, p: &Params, gov: Gov, today: u64) -> u64 {
    today
        .saturating_add(params::amount(realm, p.registration_days, gov))
        .saturating_add(params::amount(realm, p.campaign_days, gov))
        .saturating_add(1)
}
