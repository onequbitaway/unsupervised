//! Determinism: the same seed and the same inputs give the same state hash,
//! also when the world is restored from a snapshot on the way.

mod common;

use common::{add_params, cities, plugins, site_in, world_with};
use unsup_kernel::protocol::CityId;
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Holder, Money, Pos, RealmId, ResidentId, WorldConfig, ZoneId};
use unsup_life::Walkers;

const HOUR: u32 = 60;
const DAY: u32 = 1_440;

/// The residents of the scenario, in the order in which they are spawned.
const CAST: [&str; 13] = [
    "walker",
    "hiker",
    "visitor",
    "sleeper",
    "patient",
    "student",
    "traveller",
    "giver",
    "heir",
    "testator",
    "eater",
    "keeper",
    "pauper",
];

/// Two cities, and years of one day so that residents age and die of old
/// age while the scenario runs. The chance of death of old age is high, so
/// that the random rolls decide who lives.
fn config() -> WorldConfig {
    let mut config = cities(21, &[2, 1]);
    config.days_per_year = 1;
    add_params(
        &mut config,
        &[
            ("life.old_age_from", 19),
            ("life.old_age_death_bp", 2_500),
            ("life.old_age_death_rise_bp", 500),
        ],
    );
    config
}

/// Returns the id of a member of the cast. Ids are given in spawn order,
/// from 1.
fn member(name: &str) -> ResidentId {
    let slot = CAST.iter().position(|member| *member == name).expect("a member of the cast");
    ResidentId(slot as u64 + 1)
}

/// Returns the residents that walk, in all zones.
fn walkers(w: &TestWorld<'_>) -> usize {
    w.world.layout().zones().iter().map(|zone| w.zone(zone.id).res::<Walkers>().len()).sum()
}

/// Returns how many members of the cast are alive.
fn alive(w: &TestWorld<'_>) -> usize {
    CAST.iter().filter(|name| w.is_alive(member(name))).count()
}

/// Sends every living member of the cast in the first city on a walk to
/// the north-west corner of the city. Returns how many walk.
fn gather(w: &mut TestWorld<'_>) -> usize {
    let mut on_the_way = 0;
    for name in CAST {
        let id = member(name);
        let in_first_city = w.is_alive(id)
            && w.world.layout().zone(w.zone_of(id)).is_some_and(|zone| zone.city == CityId(0));
        if in_first_city {
            w.act(id, "move_to", json!({ "x": 0, "y": 0 }));
            on_the_way += 1;
        }
    }
    on_the_way
}

/// Puts a resident in the middle of the first site of a kind in the first
/// city.
fn enter(w: &mut TestWorld<'_>, r: ResidentId, kind: &str) {
    let (_, place) = site_in(w, CityId(0), kind);
    w.set_pos(r, place.rect.centre());
}

/// Starts one action of every kind and stops in the middle of them: walks
/// in one zone and across zones, a search for a site, a sleep, a hospital
/// stay, a lesson, a journey, a meal and a death with an estate.
fn setup(w: &mut TestWorld<'_>) {
    // The zones learn about the stations and the sites of the other zones.
    w.step(1);
    for (slot, name) in CAST.iter().enumerate() {
        let id = w.spawn(name);
        assert_eq!(id, member(name), "resident {slot} has another id");
        if *name != "pauper" {
            w.give_money(id, Money(20_000));
            w.give_item(id, "bread", 6);
        }
    }

    // A walk inside one zone, a walk across the city, and a walk to a site
    // of another zone that the global realm must find first.
    w.set_pos(member("walker"), Pos::new(0, 0));
    w.set_pos(member("hiker"), Pos::new(0, 0));
    w.set_pos(member("visitor"), Pos::new(0, 0));
    w.act_ok(member("walker"), "move_to", json!({ "x": 40, "y": 0 }));
    w.act_ok(member("hiker"), "move_to", json!({ "x": 127, "y": 127 }));
    let far = w.zone(ZoneId(3)).sites().iter().next().expect("zone 3 has a site").id;
    w.act_ok(member("visitor"), "move_to", json!({ "site": far }));

    w.set_attr(member("sleeper"), "need.energy", 300);
    w.act_ok(member("sleeper"), "sleep", json!({}));

    enter(w, member("patient"), "hospital");
    w.set_attr(member("patient"), "need.health", 400);
    w.act_ok(member("patient"), "heal", json!({}));

    enter(w, member("student"), "school");
    w.act_ok(member("student"), "train", json!({ "skill": "cooking", "ticks": 90 }));

    enter(w, member("traveller"), "station");
    w.act_ok(member("traveller"), "travel", json!({ "city": 1 }));

    let (giver, heir) = (member("giver"), member("heir"));
    w.set_pos(heir, w.pos(giver));
    w.act_ok(giver, "give", json!({ "to": heir, "money": 1_500, "item": "bread", "qty": 2 }));

    // The testator dies at the next hourly pass and leaves all to the heir.
    let testator = member("testator");
    w.act_ok(testator, "write_will", json!({ "heir": heir }));
    w.set_attr(testator, "need.food", 0);
    w.set_attr(testator, "need.health", 15);
    // The pauper dies with it and leaves an estate with nothing in it. The
    // global realm waits for holdings that never come.
    let pauper = member("pauper");
    w.act_ok(pauper, "write_will", json!({ "heir": heir }));
    w.set_attr(pauper, "need.food", 0);
    w.set_attr(pauper, "need.health", 15);

    w.set_attr(member("eater"), "need.food", 500);
    w.act_ok(member("eater"), "eat", json!({ "item": "bread" }));

    // The keeper leaves goods in a home and plans to rest.
    enter(w, member("keeper"), "home");
    w.act_ok(member("keeper"), "drop", json!({ "item": "bread", "qty": 3 }));
    w.act_ok(member("keeper"), "rest", json!({ "ticks": 45 }));

    // Stop in the middle of all of it.
    w.step(7);
    assert_eq!(walkers(w), 3, "three walks are on the way");
    assert!(w.has_condition(member("sleeper"), "asleep"));
    assert!(w.has_condition(member("patient"), "hospitalized"));
    assert_eq!(w.activity(member("traveller")), Some("travel"));
    assert_eq!(w.activity(member("keeper")), Some("rest"));
}

/// Lets the actions end, starts more, and runs three game days: every
/// hourly and daily pass runs, residents starve, age and die.
fn run(w: &mut TestWorld<'_>) {
    w.step(20);
    // A walk that is stopped, and a new one from where the walker stands.
    w.act_ok(member("walker"), "cancel", json!({}));
    w.act_ok(member("walker"), "move_to", json!({ "x": 3, "y": 60 }));
    w.step(5 * HOUR);

    // The first hours ended as the scenario wants.
    assert_eq!(w.pos(member("walker")), Pos::new(3, 60));
    assert_eq!(w.pos(member("hiker")), Pos::new(127, 127));
    assert_eq!(w.zone_of(member("visitor")), ZoneId(3));
    assert_eq!(w.attr(member("patient"), "need.health"), 1_000);
    assert!(w.attr(member("student"), "skill.cooking") > 0);
    let other_city = w.world.layout().zone(w.zone_of(member("traveller"))).map(|zone| zone.city);
    assert_eq!(other_city, Some(CityId(1)));
    assert!(!w.is_alive(member("testator")));
    assert!(!w.is_alive(member("pauper")));
    // The heir has its own money, the gift and the estate of the testator.
    assert_eq!(w.cash(member("heir")), Money(20_000 + 1_500 + 20_000));
    assert_eq!(w.qty(Holder::Resident(member("heir")), "bread"), 6 + 2 + 6);
    assert_eq!(walkers(w), 0);
    w.assert_conserved();

    // Three days: three birthdays, and a chance of death in each day.
    let before = alive(w);
    assert_eq!(before, CAST.len() - 2);
    w.step(DAY);
    // The old residents of the first city walk to one corner of it. Those
    // that cross a zone border get their age and their chance of the day
    // when they arrive.
    assert!(gather(w) >= 3, "too few residents of the first city are alive");
    w.step(2 * DAY);
    let after = alive(w);
    assert!(after < before, "nobody died in three days of old age");
    w.assert_conserved();
}

#[test]
fn the_same_inputs_give_the_same_hash_also_after_a_restore() {
    let hash = assert_deterministic(&plugins(), &config(), setup, run);
    // Another seed gives another world.
    let mut other = config();
    other.seed += 1;
    assert_ne!(assert_deterministic(&plugins(), &other, |w| drop(w.step(1)), |_| {}), hash);
}

#[test]
fn a_world_that_is_restored_many_times_stays_the_same() {
    let mut straight = world_with(config());
    setup(&mut straight);
    let mut restored = world_with(config());
    setup(&mut restored);
    // A restore after every frame of the first two hours: inside walks,
    // sleeps, journeys, the search for a site and the wait of an estate.
    // Then a restore now and then for two days.
    let often = std::iter::repeat_n(1, 2 * HOUR as usize);
    for frames in often.chain([119, 240, 1_440, 1_441]) {
        straight.step(frames);
        restored.step(frames);
        restored.snapshot_restore();
        assert_eq!(straight.hash(), restored.hash(), "at frame {}", straight.world.frame());
    }
    // The residents are old now. Each arrival in a zone gives the chance
    // of death of the day to a resident that did not have it yet. After a
    // restore, the next arrival must not be one more chance.
    let old: Vec<ResidentId> =
        CAST.iter().map(|name| member(name)).filter(|id| straight.is_alive(*id)).collect();
    assert!(old.len() >= 2, "{} residents are alive", old.len());
    for hop in 0..4 {
        let to = if hop % 2 == 0 { Pos::new(64, 0) } else { Pos::new(63, 0) };
        for w in [&mut straight, &mut restored] {
            let living: Vec<ResidentId> =
                old.iter().copied().filter(|id| w.is_alive(*id)).collect();
            for id in living {
                assert!(w.attr(id, "age.years") >= 19);
                w.set_pos(id, to);
            }
            w.step(3);
        }
        restored.snapshot_restore();
        assert_eq!(straight.hash(), restored.hash(), "after hop {hop}");
    }
    assert_eq!(walkers(&straight), walkers(&restored));
    assert_eq!(alive(&straight), alive(&restored));
}

#[test]
fn a_walk_goes_on_after_a_restore_as_if_nothing_happened() {
    let walk = |restore_at: Option<u32>| {
        let mut w = world_with(WorldConfig::with_seed(4));
        let ada = w.spawn_at("Ada", Pos::new(0, 0));
        w.step(1);
        w.act_ok(ada, "move_to", json!({ "x": 100, "y": 90 }));
        let mut frames = 0u32;
        while w.activity(ada).is_some() {
            if Some(frames) == restore_at {
                w.snapshot_restore();
            }
            w.step(1);
            frames += 1;
            assert!(frames < 1_000, "the walk does not end");
        }
        assert_eq!(w.pos(ada), Pos::new(100, 90));
        (frames, w.hash())
    };
    let plain = walk(None);
    // In the first zone, at the border and in the last zone.
    for at in [1, 30, 64, 65, 66, 120] {
        assert!(at < plain.0, "the walk takes {} frames", plain.0);
        assert_eq!(walk(Some(at)), plain, "a restore after {at} frames");
    }
}

#[test]
fn the_buffers_of_the_path_search_are_not_state() {
    // Two worlds: one searched many paths before, the other did not. The
    // searches are refused, so they change nothing but the buffers.
    let mut used = world_with(WorldConfig::with_seed(4));
    let mut fresh = world_with(WorldConfig::with_seed(4));
    let (ada, bob) = (used.spawn_at("Ada", Pos::new(0, 0)), fresh.spawn_at("Ada", Pos::new(0, 0)));
    assert_eq!(ada, bob);
    let wall = {
        let realm = used.world.realm(RealmId::Zone(ZoneId(0))).expect("zone 0");
        let tiles = realm.tiles();
        let bounds = tiles.bounds();
        (bounds.y0..bounds.y1)
            .flat_map(|y| (bounds.x0..bounds.x1).map(move |x| Pos::new(x, y)))
            .find(|pos| tiles.get(*pos) == Some(unsup_life::names::tile::WALL))
            .expect("zone 0 has a wall")
    };
    for _ in 0..3 {
        used.act_err(ada, "move_to", json!({ "x": wall.x, "y": wall.y }));
        used.act_ok(ada, "move_to", json!({ "x": 30, "y": 0 }));
        used.act_ok(ada, "cancel", json!({}));
        used.step(1);
        fresh.step(1);
    }
    // The cancelled walks left reports in the inbox of Ada, so the two
    // worlds differ there. The walks that follow must not differ.
    for w in [&mut used, &mut fresh] {
        w.act_ok(ada, "move_to", json!({ "x": 50, "y": 63 }));
    }
    let path = |w: &mut TestWorld<'_>| {
        let mut seen = Vec::new();
        while w.activity(ada).is_some() {
            w.step(1);
            seen.push(w.pos(ada));
        }
        seen
    };
    let (first, second) = (path(&mut used), path(&mut fresh));
    assert!(first.len() > 50, "{} steps", first.len());
    assert_eq!(first, second);
}
