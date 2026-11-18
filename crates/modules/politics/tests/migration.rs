//! A world that was made without this module gets its offices and its
//! calendar when it is restored with it.

mod common;

use common::{CITY, DAY, can, election_of, events, plugins, register, rich, seat, section, vote};
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Plugin, WorldConfig};
use unsup_politics::roles;
use unsup_society::SocietyPlugin;

static SOCIETY: SocietyPlugin = SocietyPlugin;

#[test]
fn a_snapshot_without_the_module_restores_with_offices_and_a_calendar() {
    // An old world: the society module only. It runs for nine days.
    let old: [&'static dyn Plugin; 1] = [&SOCIETY];
    let mut before = TestWorld::with_config(&old, WorldConfig::default());
    let ann = rich(&mut before, "Ann", 20_000);
    let bob = rich(&mut before, "Bob", 0);
    before.step(u32::try_from(9 * DAY + 100).expect("a short test"));
    assert!(!before.observe(ann).sections.contains_key("politics"));
    let bytes = before.world.snapshot();

    // The same world with this module. The first frame defines the roles
    // and makes the calendar from the day of the world.
    let mut w = TestWorld::restore(&plugins(), &bytes);
    let frames = w.step(1);
    assert_eq!(events(&frames, "politics.government_published").len(), 2);
    // Day 10: the first voting days of the data file are over. The next
    // city voting day on their grid is day 17, the next national one is
    // day 33.
    assert_eq!(
        section(&w, ann)["next_vote"],
        json!({ "city": { "year": 1, "day": 17 }, "nation": { "year": 2, "day": 5 } })
    );
    assert!(events(&frames, "politics.election_called").is_empty(), "day 15 opens the next one");
    // The roles exist: an office can be filled and gives its permissions.
    seat(&mut w, CITY, roles::MAYOR, &[bob]);
    assert!(can(&w, bob, CITY, "gov.appoint"));

    // The calendar runs from here like in a new world.
    let frames = w.step(u32::try_from(5 * DAY).expect("a short test"));
    let called = events(&frames, "politics.election_called");
    assert_eq!(called.len(), 2);
    assert_eq!(called[0].frame, 14 * DAY);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Ann.");
    w.step(u32::try_from(16 * DAY + 360 - w.world.frame()).expect("a short test"));
    vote(&mut w, ann, mayor, ann);
    w.step(u32::try_from(16 * DAY + 1_201 - w.world.frame()).expect("a short test"));
    assert!(can(&w, ann, CITY, "gov.sign"));
    assert!(!can(&w, bob, CITY, "gov.sign"));
    w.assert_conserved();
}
