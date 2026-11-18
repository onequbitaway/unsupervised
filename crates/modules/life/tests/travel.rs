//! Journeys between cities.

mod common;

use common::{cities, result, site_in, treasury, world_with};
use unsup_kernel::protocol::{ActionStatus, CityId, ErrorCode};
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Money, Pos, ResidentId};
use unsup_life::Stations;

/// Two cities: one of four zones and one of one zone.
fn two_cities() -> TestWorld<'static> {
    let mut w = world_with(cities(9, &[2, 1]));
    // The zones learn about the stations of the other city in the first
    // frame.
    w.step(1);
    w
}

/// Puts a resident in the first station of the first city.
fn at_station(w: &mut TestWorld<'_>, name: &str) -> ResidentId {
    let (_, station) = site_in(w, CityId(0), "station");
    w.spawn_at(name, station.rect.centre())
}

#[test]
fn every_zone_knows_the_stations_of_every_city() {
    let w = two_cities();
    for zone in w.world.layout().zones() {
        let realm = w.zone(zone.id);
        let stations = realm.res::<Stations>();
        for city in [CityId(0), CityId(1)] {
            let (in_zone, station) = site_in(&w, city, "station");
            let known: Vec<_> = stations.of_city(city).collect();
            assert_eq!(known.len(), 1, "zone {} city {city}", zone.id);
            assert_eq!((known[0].site, known[0].zone), (station.id, in_zone));
            assert_eq!(known[0].pos, station.rect.centre());
        }
    }
}

#[test]
fn a_journey_costs_the_fare_takes_time_and_ends_in_a_station_of_the_other_city() {
    let mut w = two_cities();
    let ada = at_station(&mut w, "Ada");
    w.give_money(ada, Money(6_000));
    let start = w.pos(ada);

    let status = w.act_ok(ada, "travel", json!({ "city": 1 }));
    assert!(matches!(status, ActionStatus::Started { ticks: 120, .. }), "{status:?}");
    // The fare leaves at once and reaches the treasury of the city of
    // departure in the same frame.
    assert_eq!(w.cash(ada), Money(3_500));
    w.step(1);
    assert_eq!(w.money(treasury(0)), Money(2_500));
    assert_eq!(w.money(treasury(1)), Money::ZERO);
    w.step(118);
    assert_eq!(w.pos(ada), start);
    w.step(1);

    let (zone, station) = site_in(&w, CityId(1), "station");
    assert_eq!(w.zone_of(ada), zone);
    assert_eq!(w.pos(ada), station.rect.centre());
    assert_eq!(
        result(&w, ada),
        json!({
            "city": 1,
            "zone": zone,
            "site": station.id,
            "x": station.rect.centre().x,
            "y": station.rect.centre().y,
        })
    );
    let totals = w.assert_conserved();
    assert_eq!(totals.money.held, 6_000);

    // And back: the other city gets this fare.
    w.act_ok(ada, "travel", json!({ "city": 0 }));
    w.run_until_idle(ada, 130);
    assert_eq!(w.pos(ada), start);
    assert_eq!(w.cash(ada), Money(1_000));
    w.step(1);
    assert_eq!(w.money(treasury(1)), Money(2_500));
}

#[test]
fn travel_refusals() {
    let mut w = two_cities();
    let ada = at_station(&mut w, "Ada");

    // No money for the fare.
    let error = w.act_err(ada, "travel", json!({ "city": 1 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 2_500, "available": 0 }));
    w.give_money(ada, Money(5_000));
    // The city where the resident is, and a city that does not exist.
    assert_eq!(w.act_err(ada, "travel", json!({ "city": 0 })).code, ErrorCode::InvalidArgs);
    assert_eq!(w.act_err(ada, "travel", json!({ "city": 7 })).code, ErrorCode::NotFound);
    assert_eq!(w.act_err(ada, "travel", json!({})).code, ErrorCode::InvalidArgs);
    // Not in a station.
    w.step(1);
    w.set_pos(ada, Pos::new(0, 0));
    assert_eq!(w.act_err(ada, "travel", json!({ "city": 1 })).code, ErrorCode::NotHere);
    assert_eq!(w.cash(ada), Money(5_000));
    assert_eq!(w.activity(ada), None);
    w.step(1);
    assert_eq!(w.money(treasury(0)), Money::ZERO);
}

#[test]
fn a_journey_that_is_cut_short_stays_in_the_city_and_the_fare_is_gone() {
    let mut w = two_cities();
    let ada = at_station(&mut w, "Ada");
    w.give_money(ada, Money(2_500));
    let start = w.pos(ada);
    w.act_ok(ada, "travel", json!({ "city": 1 }));
    w.step(60);
    w.act_ok(ada, "cancel", json!({}));
    w.step(100);
    assert_eq!(w.pos(ada), start);
    assert_eq!(w.cash(ada), Money::ZERO);
    assert_eq!(w.money(treasury(0)), Money(2_500));
    w.assert_conserved();
}

#[test]
fn walking_does_not_leave_the_city() {
    let mut w = two_cities();
    let ada = at_station(&mut w, "Ada");
    let other = site_in(&w, CityId(1), "station").1.rect.centre();
    let error = w.act_err(ada, "move_to", json!({ "x": other.x, "y": other.y }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    // A site of another city, by id: the answer of the global realm fails
    // the walk.
    let station = site_in(&w, CityId(1), "station").1.id;
    let ack = w.act(ada, "move_to", json!({ "site": station }));
    assert_eq!(ack.status, ActionStatus::Pending);
    w.run_until_idle(ada, 10);
    let failed = w.last_report(ada).expect("a report");
    assert_eq!(failed.error.expect("an error").code, ErrorCode::NotAllowed);
}

#[test]
fn a_traveller_that_something_moves_out_of_the_station_does_not_leave() {
    let mut w = two_cities();
    let ada = at_station(&mut w, "Ada");
    w.give_money(ada, Money(2_500));
    w.act_ok(ada, "travel", json!({ "city": 1 }));
    w.step(60);
    // Something takes the traveller out and does not stop the journey.
    w.set_pos(ada, Pos::new(0, 0));
    w.run_until_idle(ada, 100);
    let failed = w.last_report(ada).expect("a report").error.expect("an error");
    assert_eq!(failed.code, ErrorCode::Conflict);
    assert_eq!(w.pos(ada), Pos::new(0, 0));
    w.assert_conserved();
}
