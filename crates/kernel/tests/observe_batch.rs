//! `World::observe_batch` builds many observations on many threads. Each
//! answer is what `World::observe` gives for the same request
//! (architecture.md, section 5.6).

mod common;

use common::{Demo, world};
use serde_json::json;
use unsup_kernel::protocol::{Detail, Observation, ObserveOpts, SimError};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Pos, RealmId, ResidentId, Rng, World, WorldConfig, ZoneId};

/// Makes a world with residents in all four zones that work, speak and pay,
/// so that an observation has neighbours, notices and reports.
fn busy_world(residents: u64) -> (TestWorld<'static>, Vec<ResidentId>) {
    let mut w = world();
    let mut rng = Rng::seeded(51);
    let ids: Vec<ResidentId> = (0..residents)
        .map(|i| {
            let pos = Pos::new(rng.range(0, 128) as i32, rng.range(0, 128) as i32);
            w.spawn_at(&format!("r{i}"), pos)
        })
        .collect();
    for round in 0..6 {
        for (i, &id) in ids.iter().enumerate() {
            match (i + round) % 4 {
                0 => drop(w.act(id, "work", json!({ "ticks": 1 + i % 3 }))),
                1 => {
                    drop(w.act(id, "shout", json!({ "text": format!("round {round} from {id}") })))
                }
                2 => drop(w.act(id, "pay", json!({ "to": ids[(i + 7) % ids.len()], "amount": 5 }))),
                _ => drop(w.act(id, "deposit", json!({ "amount": 3 }))),
            }
        }
        w.step(1);
    }
    (w, ids)
}

/// The options that the requests of a test use, in turn.
fn options(n: usize) -> ObserveOpts {
    match n % 5 {
        0 => ObserveOpts::default(),
        1 => ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() },
        2 => ObserveOpts { since_notice: 3, ..ObserveOpts::default() },
        3 => ObserveOpts {
            sections: Some(vec!["nearby".into(), "bank".into()]),
            ..ObserveOpts::default()
        },
        _ => ObserveOpts {
            sections: Some(vec!["self".into()]),
            since_notice: 99,
            ..ObserveOpts::default()
        },
    }
}

/// What `World::observe` gives for each request, one call at a time.
fn one_by_one(
    world: &World,
    requests: &[(ResidentId, ObserveOpts)],
) -> Vec<Result<Observation, SimError>> {
    requests.iter().map(|(resident, opts)| world.observe(*resident, opts)).collect()
}

#[test]
fn a_batch_gives_what_observe_gives_for_each_request() {
    let (mut w, ids) = busy_world(120);
    let gone = ids[5];
    w.despawn(gone);
    w.step(1);
    // Every resident with every kind of options, in a mixed order, with
    // repeats, a resident that left the world and one that never lived.
    let mut rng = Rng::seeded(52);
    let mut requests: Vec<(ResidentId, ObserveOpts)> =
        (0..600).map(|n| (ids[rng.below(ids.len() as u64) as usize], options(n))).collect();
    requests.extend(ids.iter().enumerate().map(|(n, &id)| (id, options(n))));
    requests.push((ResidentId(9_999), ObserveOpts::default()));
    requests.push((gone, options(1)));
    requests.push(requests[0].clone());

    let before = w.hash();
    let expected = one_by_one(&w.world, &requests);
    let got = w.world.observe_batch(&requests);
    assert_eq!(got.len(), requests.len());
    for (n, (got, expected)) in got.iter().zip(&expected).enumerate() {
        assert_eq!(got, expected, "request {n}: resident {}", requests[n].0);
    }
    assert_eq!(w.hash(), before, "observations do not change state");

    // The batch is not empty of content: the answers have the sections of
    // the zone and of the global realm, and the two errors.
    let ok: Vec<&Observation> = got.iter().filter_map(|answer| answer.as_ref().ok()).collect();
    assert_eq!(ok.len(), requests.len() - 9, "resident 6 is asked 7 times and is gone");
    let zones: std::collections::BTreeSet<ZoneId> =
        ok.iter().map(|o| w.zone_of(o.resident)).collect();
    assert_eq!(zones.len(), 4, "the batch covers every zone");
    let full = ok.iter().find(|o| o.sections.len() == 5).expect("a request for all sections");
    assert_eq!(
        full.sections.keys().map(String::as_str).collect::<Vec<_>>(),
        ["bank", "demo", "nearby", "reports", "self"]
    );
    assert!(ok.iter().any(|o| !o.notices.is_empty()));
    assert!(ok.iter().any(|o| o.sections.get("reports").is_some_and(|r| r != &json!([]))));
    assert!(ok.iter().any(|o| o.sections.keys().eq(["bank", "nearby"])));
    assert_eq!(got[requests.len() - 3].as_ref().err(), Some(&SimError::UnknownResident));
    assert_eq!(got[requests.len() - 2].as_ref().err(), Some(&SimError::UnknownResident));
    assert_eq!(got[requests.len() - 1], got[0]);
}

#[test]
fn a_batch_is_the_same_on_one_thread_and_on_many() {
    let (mut w, ids) = busy_world(60);
    let requests: Vec<(ResidentId, ObserveOpts)> =
        ids.iter().enumerate().map(|(n, &id)| (id, options(n))).collect();
    let expected = one_by_one(&w.world, &requests);
    for threads in [1, 2, 7] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
        let world = &mut w.world;
        let got = pool.install(|| world.observe_batch(&requests));
        assert_eq!(got, expected, "{threads} threads");
    }
}

#[test]
fn small_batches_and_batches_for_one_zone() {
    let (mut w, ids) = busy_world(40);
    assert_eq!(w.world.observe_batch(&[]), []);
    // One request, and one that fails.
    let one = [(ids[3], options(1))];
    assert_eq!(w.world.observe_batch(&one), one_by_one(&w.world, &one));
    let unknown = [(ResidentId(777), ObserveOpts::default())];
    assert_eq!(w.world.observe_batch(&unknown), [Err(SimError::UnknownResident)]);
    // Only failures.
    let nobody = [unknown[0].clone(), (ResidentId(778), options(3))];
    assert_eq!(
        w.world.observe_batch(&nobody),
        [Err(SimError::UnknownResident), Err(SimError::UnknownResident)]
    );
    // All requests for the residents of one zone.
    let zone3: Vec<(ResidentId, ObserveOpts)> = ids
        .iter()
        .filter(|&&id| w.zone_of(id) == ZoneId(3))
        .enumerate()
        .map(|(n, &id)| (id, options(n)))
        .collect();
    assert!(zone3.len() > 3, "{} residents in zone 3", zone3.len());
    assert_eq!(w.world.observe_batch(&zone3), one_by_one(&w.world, &zone3));
}

#[test]
fn a_batch_in_a_world_that_hosts_a_part_of_the_realms() {
    // This world hosts zones 0 and 1, and not the global realm.
    let hosted = vec![RealmId::Zone(ZoneId(0)), RealmId::Zone(ZoneId(1))];
    let config = WorldConfig { hosted: Some(hosted), ..WorldConfig::default() };
    let mut w = TestWorld::with_config(&[&Demo], config);
    let ada = w.spawn_at("Ada", Pos::new(20, 20));
    let bea = w.spawn_at("Bea", Pos::new(80, 20));
    let cyd = w.spawn_at("Cyd", Pos::new(30, 30));
    w.step(1);
    // Cyd walks to zone 2, which another process hosts.
    w.with_resident(cyd, |realm, idx| realm.set_pos(idx, Pos::new(30, 70)).expect("zone 2 exists"));
    w.world.settle();
    let requests = [
        (ada, options(0)),
        (cyd, options(0)),
        (bea, options(1)),
        (ResidentId(50), options(0)),
        (ada, options(3)),
    ];
    let got = w.world.observe_batch(&requests);
    assert_eq!(got, one_by_one(&w.world, &requests));
    assert_eq!(got[1].as_ref().err(), Some(&SimError::NotHosted));
    assert_eq!(got[3].as_ref().err(), Some(&SimError::UnknownResident));
    // Without the global realm there is no section `bank`.
    let sections = |n: usize| -> Vec<String> {
        got[n].as_ref().expect("an observation").sections.keys().cloned().collect()
    };
    assert_eq!(sections(0), ["demo", "nearby", "reports", "self"]);
    assert_eq!(sections(4), ["nearby"]);
}

#[test]
fn a_batch_without_observers_in_the_global_realm() {
    // The kernel alone: the sections `self`, `nearby` and `reports`.
    let mut w = TestWorld::new(&[]);
    let ids: Vec<ResidentId> =
        (0..30).map(|i| w.spawn_at(&format!("r{i}"), Pos::new(4 * i, 100 - 3 * i))).collect();
    w.step(2);
    let requests: Vec<(ResidentId, ObserveOpts)> =
        ids.iter().enumerate().map(|(n, &id)| (id, options(n))).collect();
    let got = w.world.observe_batch(&requests);
    assert_eq!(got, one_by_one(&w.world, &requests));
    let first = got[0].as_ref().expect("an observation");
    assert_eq!(
        first.sections.keys().map(String::as_str).collect::<Vec<_>>(),
        ["nearby", "reports", "self"]
    );
}
