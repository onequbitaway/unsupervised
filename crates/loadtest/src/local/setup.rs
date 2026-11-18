//! Builds a world with all game modules and fills it with residents.

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use unsup_kernel::{Plugin, Rng, Site, World, WorldConfig, WorldError};
use unsup_protocol::{CityId, OwnerId, Pos, RealmId, ResidentId, SpawnSpec, ZoneId};

use super::Shape;
use crate::defaults::defaults;

/// Stream of the generator that the scatter placement uses.
const SCATTER_STREAM: u64 = 0x5ca7_7e12;

/// Where the residents are when the run starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Every resident is moved to a random tile of its city before the run.
    /// This is the state of a world that has run for a while.
    #[default]
    Scatter,
    /// Every resident stays where the game put it: at a spawn site of its
    /// city, or at the centre of the first zone when the city has none.
    Spawn,
}

impl std::str::FromStr for Placement {
    type Err = String;

    fn from_str(text: &str) -> Result<Placement, String> {
        match text {
            "scatter" => Ok(Placement::Scatter),
            "spawn" => Ok(Placement::Spawn),
            _ => Err(format!("unknown placement `{text}`; use scatter or spawn")),
        }
    }
}

impl std::fmt::Display for Placement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Placement::Scatter => "scatter",
            Placement::Spawn => "spawn",
        })
    }
}

/// What world to build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSpec {
    /// Seed of the world.
    pub seed: u64,
    /// Number of residents.
    pub residents: u32,
    /// Cities and districts.
    pub shape: Shape,
    /// Where the residents start.
    pub placement: Placement,
    /// Data files (TOML text, the format of a module's `data/balance.toml`)
    /// that replace numbers of the game, like `--data-dir` of the server.
    /// Empty for the game as it ships.
    #[serde(default)]
    pub data: Vec<String>,
}

/// A world with its residents.
pub struct Population {
    /// The world.
    pub world: World,
    /// Every resident, in ascending id order.
    pub ids: Vec<ResidentId>,
    /// Time to build the empty world, with world generation.
    pub build_time: Duration,
    /// Time to create and place the residents.
    pub populate_time: Duration,
    /// Number of spawn sites that the game made in the whole world.
    pub spawn_sites: u64,
}

/// Builds a new world with all game modules of the server, plus `extra`
/// plugins. Tests use `extra` to add a module that breaks a rule on
/// purpose.
///
/// Without extra plugins and without data overrides, this is
/// `unsup_server::build_world`: the world of the server.
///
/// # Errors
///
/// Fails when the configuration or one of its data overrides is not valid.
pub fn build_world(config: WorldConfig, extra: &[&dyn Plugin]) -> Result<World, WorldError> {
    if extra.is_empty() && config.data.is_empty() {
        config.validate().map_err(WorldError::Config)?;
        return Ok(unsup_server::build_world(config));
    }
    let modules = unsup_server::plugins();
    let mut plugins: Vec<&dyn Plugin> = modules.iter().map(AsRef::as_ref).collect();
    plugins.extend_from_slice(extra);
    World::try_new(&plugins, config)
}

/// Builds the world of `spec` and creates its residents through
/// `World::apply`, city by city in turn.
///
/// # Errors
///
/// Fails when the size is not valid for the kernel or when a resident
/// cannot be created.
pub fn populate(spec: &WorldSpec, extra: &[&dyn Plugin]) -> anyhow::Result<Population> {
    let mut config = spec.shape.to_config(spec.seed);
    config.data.clone_from(&spec.data);
    let start = Instant::now();
    let mut world = build_world(config, extra)
        .map_err(|error| anyhow!("{error}. The world would have {}", spec.shape.describe()))?;
    let build_time = start.elapsed();

    let start = Instant::now();
    let cities = u64::try_from(world.layout().cities().len()).unwrap_or(1).max(1);
    let per_owner = defaults().world.residents_per_owner.max(1);
    let mut ids = Vec::with_capacity(spec.residents as usize);
    for n in 0..u64::from(spec.residents) {
        let city = CityId(u16::try_from(n % cities).unwrap_or(0));
        let spawn = SpawnSpec {
            owner: OwnerId(1 + n / per_owner),
            name: format!("bot-{n}"),
            model: Some("loadtest".to_string()),
            city: Some(city),
        };
        let id = world
            .spawn(spawn)
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("cannot create resident number {n}"))?;
        ids.push(id);
    }
    let spawn_sites = spawn_sites(&world);
    if spec.placement == Placement::Scatter {
        scatter(&mut world, &ids, spec.seed)?;
    }
    Ok(Population { world, ids, build_time, populate_time: start.elapsed(), spawn_sites })
}

/// Counts the sites with the tag `spawn` in the whole world.
fn spawn_sites(world: &World) -> u64 {
    world
        .layout()
        .zones()
        .iter()
        .filter_map(|zone| world.realm(RealmId::Zone(zone.id)))
        .map(|realm| realm.sites().with_tag(Site::SPAWN_TAG).count() as u64)
        .sum()
}

/// Moves every resident to a random tile of its city.
///
/// This is not an action of a resident: it sets the start state, the same
/// way for the same seed. It uses the path that the kernel gives to tests
/// and administration tools (`Realm::teleport`, then `World::settle`).
fn scatter(world: &mut World, ids: &[ResidentId], seed: u64) -> anyhow::Result<()> {
    let d = &defaults().world;
    let mut rng = Rng::stream(seed, SCATTER_STREAM);
    for &id in ids {
        let home = world.locate(id).ok_or_else(|| anyhow!("resident {id} is in no zone"))?;
        let layout = world.layout();
        let city = layout
            .zone(home)
            .and_then(|zone| layout.city(zone.city))
            .ok_or_else(|| anyhow!("zone {home} has no city"))?;
        let mut target: Option<(ZoneId, Pos)> = None;
        for _ in 0..d.scatter_attempts.max(1) {
            let Some(&zone) = rng.pick(&city.zones) else { break };
            let Some(info) = layout.zone(zone) else { break };
            let b = info.bounds;
            let pos = Pos::new(
                i32::try_from(rng.range(i64::from(b.x0), i64::from(b.x1))).unwrap_or(b.x0),
                i32::try_from(rng.range(i64::from(b.y0), i64::from(b.y1))).unwrap_or(b.y0),
            );
            target = Some((zone, pos));
            let kind = world.realm(RealmId::Zone(zone)).and_then(|realm| realm.tiles().get(pos));
            if kind.is_some_and(|kind| !d.blocked_tiles.contains(&kind)) {
                break;
            }
        }
        let Some((zone, pos)) = target else { continue };
        let realm = world
            .realm(RealmId::Zone(home))
            .ok_or_else(|| anyhow!("zone {home} is not in this world"))?;
        let idx = realm.idx_of(id).ok_or_else(|| anyhow!("resident {id} is not in zone {home}"))?;
        realm
            .teleport(idx, zone, pos)
            .map_err(|error| anyhow!("cannot place resident {id} at {pos}: {error}"))?;
    }
    world.settle();
    Ok(())
}
