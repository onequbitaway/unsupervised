//! `soak` through the library: a run that passes, and runs that must fail
//! because a test plugin breaks a rule on purpose. The failing runs prove
//! that the checker finds a violation.

use std::sync::atomic::{AtomicU64, Ordering};

use unsup_kernel::{AttrDef, Builder, Dest, Holder, Money, Plugin, RealmId, Scope, Stage, ZoneId};
use unsup_loadtest::local::{Placement, Shape, WorldSpec};
use unsup_loadtest::profile::Profile;
use unsup_loadtest::soak::{self, SoakConfig, SoakRun, ViolationKind};

fn config(seed: u64, ticks: u64, verify: bool) -> SoakConfig {
    SoakConfig {
        world: WorldSpec {
            seed,
            residents: 120,
            shape: Shape { cities: 2, districts: 2 },
            placement: Placement::Scatter,
            data: Vec::new(),
        },
        ticks,
        profile: Profile::load("walkers").expect("a built-in profile"),
        audit_every: 1,
        verify_determinism: verify,
        hash_every: 10,
        request_ids: false,
    }
}

fn run(config: &SoakConfig, extra: &[&dyn Plugin]) -> SoakRun {
    soak::run(config, extra).expect("the soak run starts")
}

#[test]
fn a_run_of_the_full_game_keeps_conservation_and_determinism() {
    let result = run(&config(1, 60, true), &[]);
    assert!(result.passed(), "violation: {:?}", result.violation);
    assert_eq!(result.ticks_run, 60);
    assert_eq!(result.residents, 120);
    assert_eq!(result.residents_end, 120);
    assert_eq!(result.zones, 8);
    // One audit before the first tick and one after each tick.
    assert_eq!(result.audits_passed, 61);
    // Hash comparisons: before the first tick, then after ticks 10 to 60.
    assert_eq!(result.hash_checks_passed, 7);
    assert!(result.actions.accepted() > 300, "{} actions", result.actions.accepted());
    assert_eq!(result.actions.rejected, 0);
    assert_eq!(result.state_hash.len(), 64, "a blake3 hash in hexadecimal");
    assert!(result.summary().contains("result: PASSED"));
    assert!(
        result.summary().starts_with("120 residents in 8 zones (2 cities of 2 x 2 districts)"),
        "{}",
        result.summary()
    );
    assert_eq!(result.verdict(), Ok(()));
    // The audit equation holds for the totals in the report.
    let money = result.money.expect("the last audit passed");
    assert_eq!(money.held + money.in_flight, money.minted - money.burned);
}

#[test]
fn the_checks_run_at_the_asked_period_and_always_at_the_end() {
    let mut config = config(1, 25, true);
    config.audit_every = 10;
    config.hash_every = 7;
    let result = run(&config, &[]);
    assert!(result.passed(), "violation: {:?}", result.violation);
    // Audits after ticks 0, 10, 20 and 25. Hashes after 0, 7, 14, 21, 25.
    assert_eq!(result.audits_passed, 4);
    assert_eq!(result.hash_checks_passed, 5);

    let unchecked = run(&SoakConfig { verify_determinism: false, ..config }, &[]);
    assert_eq!(unchecked.hash_checks_passed, 0);
    assert!(unchecked.summary().contains("determinism: not checked"));
}

#[test]
fn the_same_seed_gives_the_same_state_and_another_seed_does_not() {
    let first = run(&config(5, 40, false), &[]);
    let again = run(&config(5, 40, false), &[]);
    let other = run(&config(6, 40, false), &[]);
    assert!(first.passed() && again.passed() && other.passed());
    assert_eq!(first.state_hash, again.state_hash);
    assert_eq!(first.actions, again.actions, "the driver sends the same requests");
    assert_ne!(first.state_hash, other.state_hash);
}

/// Makes money from nothing: it wires money to the global realm and then
/// puts the ledger of the zone back as it was before the wire. The money
/// is then in the zone and on its way at the same time.
struct Counterfeit {
    at_tick: u64,
}

impl Plugin for Counterfeit {
    fn name(&self) -> &'static str {
        "counterfeit"
    }

    fn build(&self, b: &mut Builder) {
        let faucet = b.faucet("counterfeit.print");
        if b.scope() != Scope::Zone {
            return;
        }
        let at_tick = self.at_tick;
        b.system(Stage::Update, 0, "counterfeit.copy", move |realm| {
            if realm.id() != RealmId::Zone(ZoneId(0)) || realm.tick().0 != at_tick {
                return;
            }
            let till = Holder::Account(900);
            realm.ledger_mut().mint(faucet, till, Money(500));
            let before = realm.ledger().clone();
            realm
                .wire_money(till, Dest::Global, Holder::Account(901), Money(500), "counterfeit")
                .expect("the till holds the money");
            *realm.ledger_mut() = before;
        });
    }
}

#[test]
fn a_plugin_that_creates_money_outside_a_faucet_fails_the_soak_at_that_tick() {
    let result = run(&config(1, 40, false), &[&Counterfeit { at_tick: 17 }]);
    let violation = result.violation.clone().expect("the audit finds the copied money");
    assert_eq!(violation.kind, ViolationKind::Conservation);
    assert_eq!(violation.tick, 17);
    // 500 cents were minted, and 1,000 cents exist.
    assert_eq!(
        violation.message,
        "ledger audit failed in the world: money holds 1000 but must hold 500"
    );
    assert_eq!(result.ticks_run, 17, "the run stops at the first violation");
    assert_eq!(result.audits_passed, 17, "ticks 0 to 16 were clean");
    assert!(!result.passed());
    assert_eq!(
        violation.to_string(),
        "soak failed at tick 17: conservation: ledger audit failed in the world: money holds 1000 but must hold 500"
    );
    assert!(result.summary().contains("result: FAILED. soak failed at tick 17"));
    assert_eq!(result.money, None, "the totals of a world that does not balance are not shown");
    // The command line fails with this verdict.
    assert_eq!(result.verdict(), Err(violation));
}

#[test]
fn a_rare_audit_still_finds_the_violation_later() {
    let mut config = config(1, 40, false);
    config.audit_every = 10;
    let result = run(&config, &[&Counterfeit { at_tick: 17 }]);
    let violation = result.violation.expect("the audit finds the copied money");
    assert_eq!(violation.kind, ViolationKind::Conservation);
    assert_eq!(violation.tick, 20, "the first audit after tick 17");
}

#[test]
fn the_same_world_without_the_broken_plugin_passes() {
    // The control for the tests above: a plugin that only mints through
    // its faucet breaks nothing.
    struct Honest;
    impl Plugin for Honest {
        fn name(&self) -> &'static str {
            "honest"
        }
        fn build(&self, b: &mut Builder) {
            let faucet = b.faucet("honest.print");
            if b.scope() == Scope::Zone {
                b.system(Stage::Update, 0, "honest.print", move |realm| {
                    realm.ledger_mut().mint(faucet, Holder::Account(900), Money(5));
                });
            }
        }
    }
    let result = run(&config(1, 40, true), &[&Honest]);
    assert!(result.passed(), "violation: {:?}", result.violation);
    let money = result.money.expect("the last audit passed");
    // 8 zones mint 5 cents in each of 40 ticks.
    assert_eq!(money.minted, 8 * 40 * 5);
    assert_eq!(money.held, 8 * 40 * 5);
}

/// Counts every tick of every world of this process. Two worlds never see
/// the same numbers.
static DRIFT: AtomicU64 = AtomicU64::new(0);

/// Breaks determinism: from `from_tick` on, it writes a number that
/// depends on the process, not on the seed and the inputs, into the state.
struct Drift {
    from_tick: u64,
}

impl Plugin for Drift {
    fn name(&self) -> &'static str {
        "drift"
    }

    fn build(&self, b: &mut Builder) {
        let attr = b.attr(AttrDef::new("drift.noise").range(0, i64::MAX).default(0));
        if b.scope() != Scope::Zone {
            return;
        }
        let from_tick = self.from_tick;
        b.system(Stage::Update, 0, "drift.noise", move |realm| {
            if realm.tick().0 < from_tick {
                return;
            }
            let noise = DRIFT.fetch_add(1, Ordering::Relaxed);
            if let Some(value) = realm.attrs_mut(attr).first_mut() {
                *value = i64::try_from(noise).expect("the counter is small");
            }
        });
    }
}

#[test]
fn a_plugin_that_breaks_determinism_fails_the_soak_at_the_next_comparison() {
    let result = run(&config(1, 60, true), &[&Drift { from_tick: 23 }]);
    let violation = result.violation.clone().expect("the two runs differ");
    assert_eq!(violation.kind, ViolationKind::Determinism);
    // Hashes are compared every 10 ticks: 20 is still equal, 30 is not.
    assert_eq!(violation.tick, 30);
    assert_eq!(result.hash_checks_passed, 3, "ticks 0, 10 and 20");
    assert!(violation.message.contains("different state hashes"), "{}", violation.message);
    assert!(
        violation.message.contains("The last equal comparison was after tick 20."),
        "{}",
        violation.message
    );
    assert_eq!(result.ticks_run, 30);
    // Money was never touched: the audits all passed.
    assert_eq!(result.audits_passed, 31);

    // Without the comparison nobody sees it: the option is needed.
    let unchecked = run(&config(1, 60, false), &[&Drift { from_tick: 23 }]);
    assert!(unchecked.passed());
}

#[test]
fn a_comparison_at_every_tick_finds_the_first_tick_that_differs() {
    let mut config = config(1, 40, true);
    config.hash_every = 1;
    let result = run(&config, &[&Drift { from_tick: 23 }]);
    let violation = result.violation.expect("the two runs differ");
    assert_eq!((violation.kind, violation.tick), (ViolationKind::Determinism, 23));
    assert!(violation.message.contains("after tick 22"), "{}", violation.message);
}

/// Panics in one tick, as the self-audit of a realm does for a broken
/// ledger.
struct Bomb {
    at_tick: u64,
}

impl Plugin for Bomb {
    fn name(&self) -> &'static str {
        "bomb"
    }

    fn build(&self, b: &mut Builder) {
        if b.scope() != Scope::Global {
            return;
        }
        let at_tick = self.at_tick;
        b.system(Stage::Update, 0, "bomb.tick", move |realm| {
            assert!(realm.tick().0 != at_tick, "the bomb plugin went off at tick {at_tick}");
        });
    }
}

#[test]
fn a_panic_in_a_module_fails_the_soak_with_the_panic_message() {
    let result = run(&config(1, 40, false), &[&Bomb { at_tick: 9 }]);
    let violation = result.violation.clone().expect("the panic is a violation");
    assert_eq!(violation.kind, ViolationKind::Panic);
    assert_eq!(violation.tick, 9);
    assert_eq!(violation.message, "the bomb plugin went off at tick 9");
    assert_eq!(result.ticks_run, 8, "tick 9 did not finish");
    assert_eq!(result.audits_passed, 9);
    assert_eq!(result.state_hash, "", "a world that panicked is not read again");
}
