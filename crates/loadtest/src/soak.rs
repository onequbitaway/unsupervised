//! `soak`: a long run in one process that proves conservation and
//! determinism for the full game.
//!
//! The run steps the world tick by tick with a behaviour profile and stops
//! at the first violation:
//!
//! - **conservation**: `World::audit` fails. Money or goods were created or
//!   lost outside a faucet or a sink, in a ledger or on the way between two
//!   realms.
//! - **determinism**: with `verify_determinism`, a second world gets the
//!   same seed and the same inputs. The state hashes of the two worlds
//!   differ.
//! - **panic**: the kernel or a module panicked. The self-audit of a realm
//!   reports a broken ledger this way.

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Plugin, World};
use unsup_protocol::RealmId;

use crate::bench::ProfileUse;
use crate::local::{ActionCounts, Driver, DriverOptions, Placement, Shape, WorldSpec, populate};
use crate::profile::Profile;
use crate::report::thousands;

/// What to run.
#[derive(Clone, Debug)]
pub struct SoakConfig {
    /// The world.
    pub world: WorldSpec,
    /// Number of ticks.
    pub ticks: u64,
    /// What the residents do.
    pub profile: Profile,
    /// Ticks between two audits of the whole world. The last tick is always
    /// audited.
    pub audit_every: u64,
    /// True: run a second world with the same seed and inputs and compare
    /// the state hashes.
    pub verify_determinism: bool,
    /// Ticks between two comparisons of the state hash. The state before
    /// the first tick and after the last tick is always compared.
    pub hash_every: u64,
    /// True: every request carries an idempotency key.
    pub request_ids: bool,
}

/// The rule that a run broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// Money or goods were created or lost outside a faucet or a sink.
    Conservation,
    /// Two runs with the same seed and inputs have different states.
    Determinism,
    /// The kernel or a module panicked.
    Panic,
}

impl fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ViolationKind::Conservation => "conservation",
            ViolationKind::Determinism => "determinism",
            ViolationKind::Panic => "panic",
        })
    }
}

/// The first violation of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// The rule that was broken.
    pub kind: ViolationKind,
    /// The tick after which the check failed. 0 is the state before the
    /// first tick.
    pub tick: u64,
    /// What the check found.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "soak failed at tick {}: {}: {}", self.tick, self.kind, self.message)
    }
}

impl std::error::Error for Violation {}

/// Money in the whole world at the end of a run, in cents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoneyAtEnd {
    /// Sum of all balances.
    pub held: i64,
    /// Created through faucets.
    pub minted: i64,
    /// Destroyed through sinks.
    pub burned: i64,
    /// On the way between two realms.
    pub in_flight: i64,
    /// Money that each faucet created, by the name of the faucet. Only
    /// faucets that created money.
    #[serde(default)]
    pub faucets: BTreeMap<String, i64>,
    /// Money that each sink destroyed, by the name of the sink. Only sinks
    /// that destroyed money.
    #[serde(default)]
    pub sinks: BTreeMap<String, i64>,
}

/// One good in the whole world at the end of a run, in units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodsAtEnd {
    /// Sum of all stocks.
    pub held: i64,
    /// Created through faucets.
    pub produced: i64,
    /// Destroyed through sinks.
    pub consumed: i64,
    /// On the way between two realms.
    pub in_flight: i64,
}

/// Adds the money that each faucet created and each sink destroyed, over
/// the ledgers of all realms.
fn flows(world: &World) -> (BTreeMap<String, i64>, BTreeMap<String, i64>) {
    let blueprint = world.blueprint();
    let realms = || {
        let zones = world.layout().zones().iter().map(|zone| RealmId::Zone(zone.id));
        std::iter::once(RealmId::Global).chain(zones).filter_map(|id| world.realm(id))
    };
    let mut faucets = BTreeMap::new();
    for (faucet, name) in blueprint.faucets() {
        let minted: i64 = realms().map(|realm| realm.ledger().minted_by(faucet).cents()).sum();
        if minted != 0 {
            faucets.insert(name.to_string(), minted);
        }
    }
    let mut sinks = BTreeMap::new();
    for (sink, name) in blueprint.sinks() {
        let burned: i64 = realms().map(|realm| realm.ledger().burned_by(sink).cents()).sum();
        if burned != 0 {
            sinks.insert(name.to_string(), burned);
        }
    }
    (faucets, sinks)
}

/// The result of a soak run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SoakRun {
    /// Number of residents at the start.
    pub residents: u64,
    /// Number of residents at the end.
    pub residents_end: u64,
    /// Number of cities.
    pub cities: u32,
    /// Districts on one side of a city.
    pub districts: u32,
    /// Number of zones.
    pub zones: u64,
    /// Where the residents started.
    pub placement: Placement,
    /// The profile.
    pub profile: ProfileUse,
    /// Ticks that the command asked for.
    pub ticks: u64,
    /// Ticks that ran. Fewer than `ticks` after a violation.
    pub ticks_run: u64,
    /// Ticks between two audits.
    pub audit_every: u64,
    /// Audits of the whole world that passed.
    pub audits_passed: u64,
    /// True when a second world ran for the determinism check.
    pub verify_determinism: bool,
    /// Ticks between two comparisons of the state hash.
    pub hash_every: u64,
    /// Comparisons of the state hash that found the two worlds equal.
    pub hash_checks_passed: u64,
    /// The first violation, or `None` when the run passed.
    pub violation: Option<Violation>,
    /// What the residents did.
    pub actions: ActionCounts,
    /// Wall time of the tick loop in seconds.
    pub wall_secs: f64,
    /// Ticks for each second of wall time.
    pub ticks_per_second: f64,
    /// Money in the world at the end. `None` when the last audit failed.
    pub money: Option<MoneyAtEnd>,
    /// Number of different goods that the world has seen.
    pub goods_kinds: u64,
    /// Each good that the world has seen, by the name of the item. Empty
    /// when the last audit failed.
    #[serde(default)]
    pub goods: BTreeMap<String, GoodsAtEnd>,
    /// Messages without a target that the world dropped.
    pub dropped_messages: u64,
    /// State hash at the end, in hexadecimal.
    pub state_hash: String,
}

impl SoakRun {
    /// Returns true when the run found no violation.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.violation.is_none()
    }

    /// Returns the verdict of the run as a result, for a caller that must
    /// fail when the run failed.
    ///
    /// # Errors
    ///
    /// Returns the first violation of the run.
    pub fn verdict(&self) -> Result<(), Violation> {
        self.violation.clone().map_or(Ok(()), Err)
    }

    /// Returns the summary of the run.
    #[must_use]
    pub fn summary(&self) -> String {
        let shape = Shape { cities: self.cities, districts: self.districts };
        let mut out = format!(
            "{} residents in {} ({}), placement {}\n",
            thousands(self.residents),
            Shape::zones_text(self.zones),
            shape.describe(),
            self.placement
        );
        out.push_str(&format!("  {}\n", self.profile.line()));
        out.push_str(&format!(
            "  {} of {} ticks in {:.2} s ({:.0} ticks for each second)\n",
            thousands(self.ticks_run),
            thousands(self.ticks),
            self.wall_secs,
            self.ticks_per_second
        ));
        out.push_str(&format!(
            "  actions: {} accepted, {} rejected, {} observations\n",
            thousands(self.actions.accepted()),
            thousands(self.actions.rejected),
            thousands(self.actions.observes),
        ));
        out.push_str(&self.actions.refusal_lines());
        out.push_str(&format!(
            "  conservation: {} audits of the whole world passed (one every {} ticks)\n",
            thousands(self.audits_passed),
            self.audit_every
        ));
        if self.verify_determinism {
            out.push_str(&format!(
                "  determinism: {} comparisons of two identical runs passed (one every {} ticks)\n",
                thousands(self.hash_checks_passed),
                self.hash_every
            ));
        } else {
            out.push_str("  determinism: not checked (use --verify-determinism)\n");
        }
        if let Some(money) = &self.money {
            out.push_str(&format!(
                "  money at the end: {} held + {} on the way = {} minted - {} burned (cents)\n",
                money.held, money.in_flight, money.minted, money.burned
            ));
            out.push_str(&format!("  money faucets: {:?}\n", money.faucets));
            out.push_str(&format!("  money sinks: {:?}\n", money.sinks));
            let (produced, consumed, held) = self.goods.values().fold((0, 0, 0), |sum, good| {
                (sum.0 + good.produced, sum.1 + good.consumed, sum.2 + good.held + good.in_flight)
            });
            out.push_str(&format!(
                "  goods at the end: {} kinds, {held} units held = {produced} produced - {consumed} consumed\n",
                self.goods_kinds
            ));
        }
        out.push_str(&format!(
            "  residents at the end: {}; dropped messages: {}; state hash {}\n",
            thousands(self.residents_end),
            self.dropped_messages,
            self.state_hash
        ));
        for warning in &self.profile.warnings {
            out.push_str(&format!("  warning: {warning}\n"));
        }
        match &self.violation {
            None => out.push_str("  result: PASSED\n"),
            Some(violation) => out.push_str(&format!("  result: FAILED. {violation}\n")),
        }
        out
    }
}

/// Writes bytes as lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// One world with the driver of its residents.
struct Side {
    world: World,
    driver: Driver,
}

impl Side {
    fn new(config: &SoakConfig, extra: &[&dyn Plugin]) -> anyhow::Result<(Side, ProfileUse)> {
        let population = populate(&config.world, extra)?;
        let world = population.world;
        let mut profile = config.profile.resolve(&world.catalogue())?;
        profile.check_map(&population.map);
        let profile_use = ProfileUse::of(&profile);
        let options = DriverOptions {
            seed: config.world.seed,
            request_ids: config.request_ids,
            observe_sections: Vec::new(),
            observe_batch: 0,
        };
        let driver = Driver::new(&world, population.map, population.ids, profile, options);
        Ok((Side { world, driver }, profile_use))
    }

    /// Runs one tick: the residents act, then the world steps one frame.
    /// A panic of the kernel or of a module comes back as its message.
    fn tick(&mut self) -> Result<(), String> {
        let (world, driver) = (&mut self.world, &mut self.driver);
        // After a panic the run stops and the world is not used again, so
        // state that the panic left half-changed cannot be observed.
        catch_unwind(AssertUnwindSafe(|| {
            driver.turn(world);
            world.step();
        }))
        .map_err(|panic| {
            panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_string()))
                .unwrap_or_else(|| "a panic without a message".to_string())
        })
    }
}

/// Compares the state hashes of the two worlds.
fn compare(
    first: &World,
    second: &World,
    tick: u64,
    last_equal: Option<u64>,
) -> Result<(), Violation> {
    let (a, b) = (first.state_hash(), second.state_hash());
    if a == b {
        return Ok(());
    }
    let since = match last_equal {
        Some(tick) => format!("The last equal comparison was after tick {tick}."),
        None => "No comparison before this one was equal.".to_string(),
    };
    Err(Violation {
        kind: ViolationKind::Determinism,
        tick,
        message: format!(
            "two runs with the same seed and the same inputs have different state hashes \
             (first run {}, second run {}). {since}",
            hex(&a),
            hex(&b)
        ),
    })
}

/// Runs the checks that are due after `tick`. Returns the first violation.
fn check(
    config: &SoakConfig,
    first: &Side,
    second: Option<&Side>,
    tick: u64,
    run: &mut SoakRun,
    last_equal: &mut Option<u64>,
) -> Result<(), Violation> {
    let last = tick == config.ticks;
    if last || tick.is_multiple_of(config.audit_every.max(1)) {
        first.world.audit().map_err(|error| Violation {
            kind: ViolationKind::Conservation,
            tick,
            message: error.to_string(),
        })?;
        run.audits_passed += 1;
    }
    if let Some(second) = second
        && (last || tick == 0 || tick.is_multiple_of(config.hash_every.max(1)))
    {
        compare(&first.world, &second.world, tick, *last_equal)?;
        *last_equal = Some(tick);
        run.hash_checks_passed += 1;
    }
    Ok(())
}

/// Runs a soak test. `extra` plugins are added to the game modules of the
/// server; tests use them to break a rule on purpose.
///
/// A violation is not an `Err`: it is in [`SoakRun::violation`], with the
/// numbers of the run up to it. The command line turns it into a failed
/// exit code.
///
/// # Errors
///
/// Fails when the world cannot be built or when it has none of the verbs of
/// the profile.
pub fn run(config: &SoakConfig, extra: &[&dyn Plugin]) -> anyhow::Result<SoakRun> {
    let (mut first, profile_use) = Side::new(config, extra)?;
    let mut second = match config.verify_determinism {
        true => Some(Side::new(config, extra)?.0),
        false => None,
    };
    for warning in &profile_use.warnings {
        eprintln!("soak: warning: {warning}");
    }
    let mut run = SoakRun {
        residents: u64::from(config.world.residents),
        cities: config.world.shape.cities,
        districts: config.world.shape.districts,
        zones: first.world.layout().zones().len() as u64,
        placement: config.world.placement,
        profile: profile_use,
        ticks: config.ticks,
        audit_every: config.audit_every.max(1),
        verify_determinism: config.verify_determinism,
        hash_every: config.hash_every.max(1),
        ..SoakRun::default()
    };

    let mut last_equal = None;
    let start = Instant::now();
    // The state before the first tick: world generation and the creation of
    // the residents must already be the same in both worlds.
    let mut violation = check(config, &first, second.as_ref(), 0, &mut run, &mut last_equal).err();
    let mut tick = 0;
    while violation.is_none() && tick < config.ticks {
        tick += 1;
        let stepped = first.tick().and_then(|()| second.as_mut().map_or(Ok(()), Side::tick));
        violation = match stepped {
            Err(message) => Some(Violation { kind: ViolationKind::Panic, tick, message }),
            Ok(()) => {
                run.ticks_run = tick;
                check(config, &first, second.as_ref(), tick, &mut run, &mut last_equal).err()
            }
        };
    }
    run.wall_secs = start.elapsed().as_secs_f64();
    run.ticks_per_second =
        if run.wall_secs > 0.0 { run.ticks_run as f64 / run.wall_secs } else { 0.0 };
    run.actions = first.driver.take_counts();

    // After a panic the world may be half-changed. It is not read again.
    let panicked = violation.as_ref().is_some_and(|v| v.kind == ViolationKind::Panic);
    if !panicked {
        run.residents_end = first.world.residents() as u64;
        run.dropped_messages = first.world.dropped_messages();
        run.state_hash = hex(&first.world.state_hash());
        if let Ok(totals) = first.world.audit() {
            let (faucets, sinks) = flows(&first.world);
            run.money = Some(MoneyAtEnd {
                held: totals.money.held,
                minted: totals.money.minted,
                burned: totals.money.burned,
                in_flight: totals.money.in_flight,
                faucets,
                sinks,
            });
            run.goods_kinds = totals.items.len() as u64;
            let blueprint = first.world.blueprint();
            for (item, good) in &totals.items {
                run.goods.insert(
                    blueprint.item_name(*item).to_string(),
                    GoodsAtEnd {
                        held: good.held,
                        produced: good.produced,
                        consumed: good.consumed,
                        in_flight: good.in_flight,
                    },
                );
            }
        }
    }
    run.violation = violation;
    Ok(run)
}
