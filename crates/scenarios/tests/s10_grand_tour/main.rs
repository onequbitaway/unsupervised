//! Story 10: a grand tour of the catalogue.
//!
//! The residents of the republic use every verb of the game with success
//! at least once. The test fails when the catalogue has a verb that the
//! tour never completed: a new verb needs a place in the tour.
//!
//! The tour starts with the founding of the republic
//! (`unsup_scenarios::republic`) in a world with two cities, and goes on
//! for four more game days. Each part of it is one file:
//!
//! - `daily`: the kernel verbs, the body, the map and the journey.
//! - `business`: a restaurant, its workers, its goods and its customers.
//! - `home`: a home with a tenant, a marriage and a household.
//! - `money`: the bank, the exchange, shares and the central bank.
//! - `groups`: a guild, its contracts and its missions.
//! - `law`: crimes, the police, the court and the jail.
//! - `state`: a recall, an election campaign and laws.

mod business;
mod daily;
mod groups;
mod home;
mod law;
mod money;
mod state;

use serde_json::json;
use unsup_kernel::{CityId, OrgId, ResidentId};
use unsup_scenarios::republic::{feed, found};
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic};

/// The first city: the capital of the republic.
pub const CITY: CityId = CityId(0);

/// What the parts of the tour share.
pub struct Tour {
    /// The residents of the republic.
    pub cast: Republic,
    /// The guild that the tour founds.
    pub guild: OrgId,
}

/// Sends the residents that can walk to the shelter. They eat, sleep for
/// `ticks` game minutes and eat again. A resident in custody stays where
/// it is: the jail feeds it.
pub fn rest(g: &mut Game, residents: &[ResidentId], ticks: u32) {
    let free: Vec<ResidentId> = residents
        .iter()
        .copied()
        .filter(|r| !g.has_condition(*r, "jailed") && !g.has_condition(*r, "detained"))
        .collect();
    let shelter = g.site("shelter");
    g.walk_all(&free, &shelter);
    feed(g, &free);
    let tired: Vec<ResidentId> =
        free.iter().copied().filter(|r| g.attr(*r, "need.energy") < 1_000).collect();
    g.each(&tired, "sleep", json!({ "ticks": ticks }));
    feed(g, &free);
}

fn story(g: &mut Game) {
    let cast = found(g);
    let everyone = cast.everyone();
    // Milo leaves for the other city on the way. The others stay.
    let home_cast: Vec<ResidentId> =
        everyone.iter().copied().filter(|r| *r != cast.rival).collect();

    // Day 6. Three signatures open an early election for mayor. Its
    // campaign day is day 8 and its voting day is day 9.
    assert_eq!(g.day(), 6);
    let recall = state::start_recall(g, &cast);
    state::laws(g, &cast);
    money::bank(g, &cast);
    let place = business::open(g, &cast);
    rest(g, &everyone, 480);

    // Day 7.
    assert_eq!(g.day(), 7);
    daily::run(g, &cast);
    business::trade(g, &cast, &place);
    let guild = groups::guild(g, &cast);
    let tour = Tour { cast, guild };
    home::run(g, &tour);

    // Day 8: the campaign day.
    assert_eq!(g.day(), 8);
    state::campaign(g, &tour.cast, recall);
    rest(g, &home_cast, 360);
    money::credit(g, &tour);
    money::markets(g, &tour);
    groups::contracts(g, &tour);
    groups::missions(g, &tour);

    // Day 9: the polls are open from 06:00 to 20:00.
    if g.frame() < Game::frame_at(9, 6, 5) {
        g.run_to(9, 6, 5);
    }
    assert_eq!(g.day(), 9);
    state::vote(g, &tour.cast, recall, &home_cast);
    rest(g, &home_cast, 360);
    // The crimes go on into the night, past the count of 20:00.
    let robbery = law::crimes(g, &tour, &place);
    state::count(g, &tour.cast, home_cast.len());
    law::court(g, &tour, robbery);
    business::close(g, &tour.cast, &place);
    state::dismissals(g, &tour.cast);

    // Every verb of the catalogue ended with success at least once.
    let catalogue: Vec<String> =
        g.world.world.catalogue().verbs.into_iter().map(|verb| verb.verb).collect();
    assert_eq!(
        catalogue.len(),
        127,
        "the catalogue changed: give the new verb a place in the tour"
    );
    let missing: Vec<&String> =
        catalogue.iter().filter(|verb| !g.completed_verbs().contains(*verb)).collect();
    assert!(missing.is_empty(), "the tour never completed these verbs: {missing:?}");
    // And the tour used no verb that the catalogue does not have.
    assert_eq!(g.completed_verbs().len(), catalogue.len());

    // Nobody died on the way, and nothing was made or lost outside the
    // faucets and the sinks.
    for resident in tour.cast.everyone() {
        assert!(g.is_alive(resident), "resident {resident} died");
    }
    g.audit();
}

#[test]
fn the_tour_completes_every_verb_of_the_catalogue() {
    assert_deterministic(&Setup::shaped(1, 2, 2), story);
}
