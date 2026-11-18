//! The world of a story: every game module, as the server builds it.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use unsup_kernel::Plugin;
use unsup_kernel::protocol::{Event, OwnerId, SpawnSpec};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ResidentId, WorldConfig};
use unsup_server::WorldShape;

/// Returns the game modules of the server. The list lives as long as the
/// process, so that every world of a test can borrow it.
pub(crate) fn plugins() -> &'static [&'static dyn Plugin] {
    static PLUGINS: OnceLock<Vec<&'static dyn Plugin>> = OnceLock::new();
    PLUGINS.get_or_init(|| {
        unsup_server::plugins()
            .into_iter()
            .map(|plugin| {
                let plugin: &'static dyn Plugin = Box::leak(plugin);
                plugin
            })
            .collect()
    })
}

/// What world a story needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// The seed, the number of cities and the districts of each city.
    pub shape: WorldShape,
    /// Parameters that the story changes, as the server does with
    /// `--data-dir`. A story changes a parameter only when it says why.
    pub params: Vec<(String, i64)>,
    /// The world is restored from its own snapshot each time this many
    /// frames ran. `None`: never.
    pub restore_every: Option<u64>,
}

impl Setup {
    /// A world with one city of 2 by 2 districts.
    #[must_use]
    pub fn town(seed: u64) -> Setup {
        Setup::shaped(seed, 1, 2)
    }

    /// A world with a number of cities and districts on each side of a
    /// city.
    #[must_use]
    pub fn shaped(seed: u64, cities: u32, districts: u32) -> Setup {
        Setup {
            shape: WorldShape { seed, cities, districts },
            params: Vec::new(),
            restore_every: None,
        }
    }

    /// Changes the start value of a parameter.
    #[must_use]
    pub fn param(mut self, name: &str, value: i64) -> Setup {
        self.params.retain(|(other, _)| other != name);
        self.params.push((name.to_string(), value));
        self
    }

    /// Returns the kernel configuration of the world.
    #[must_use]
    pub fn config(&self) -> WorldConfig {
        let mut config = self.shape.to_config();
        if !self.params.is_empty() {
            let data: String = self
                .params
                .iter()
                .map(|(name, value)| format!("[params.\"{name}\"]\nvalue = {value}\n"))
                .collect();
            config.data.push(data);
        }
        config
    }
}

/// A world with every game module, and what a story needs to play in it.
pub struct Game {
    /// The world. Use it for a read that the helpers do not cover.
    pub world: TestWorld<'static>,
    pub(crate) events: Vec<Event>,
    pub(crate) completed: BTreeSet<String>,
    pub(crate) restore_every: Option<u64>,
    owners: u64,
}

impl Game {
    /// Builds the world of a setup. No frame has run.
    #[must_use]
    pub fn new(setup: &Setup) -> Game {
        Game {
            world: TestWorld::with_config(plugins(), setup.config()),
            events: Vec::new(),
            completed: BTreeSet::new(),
            restore_every: setup.restore_every,
            owners: 0,
        }
    }

    /// Wraps a world that exists already, for example one that was
    /// restored from a snapshot.
    #[must_use]
    pub(crate) fn from_world(world: TestWorld<'static>, restore_every: Option<u64>) -> Game {
        // New residents get owners that no resident of the world has.
        let owners = 1_000_000;
        Game { world, events: Vec::new(), completed: BTreeSet::new(), restore_every, owners }
    }

    /// Creates a resident in the first city. Each resident has its own
    /// owner, as residents of different people have.
    pub fn spawn(&mut self, name: &str) -> ResidentId {
        self.spawn_in(name, CityId(0))
    }

    /// Creates a resident in a city, with its own owner.
    pub fn spawn_in(&mut self, name: &str, city: CityId) -> ResidentId {
        self.owners += 1;
        self.spawn_owned(name, city, OwnerId(self.owners))
    }

    /// Creates a resident of an owner in a city.
    ///
    /// # Panics
    ///
    /// Panics when the world refuses the resident.
    pub fn spawn_owned(&mut self, name: &str, city: CityId, owner: OwnerId) -> ResidentId {
        let spec = SpawnSpec { owner, name: name.to_string(), model: None, city: Some(city) };
        match self.world.world.spawn(spec) {
            Ok(id) => id,
            Err(e) => panic!("cannot spawn `{name}`: {e}"),
        }
    }

    /// Returns the public events of all frames so far, oldest first.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns the public events of one kind, oldest first.
    #[must_use]
    pub fn events_of(&self, kind: &str) -> Vec<&Event> {
        self.events.iter().filter(|event| event.kind == kind).collect()
    }

    /// Returns the newest public event of one kind.
    ///
    /// # Panics
    ///
    /// Panics when no such event happened.
    #[must_use]
    pub fn last_event(&self, kind: &str) -> &Event {
        match self.events.iter().rev().find(|event| event.kind == kind) {
            Some(event) => event,
            None => panic!("no event `{kind}` happened"),
        }
    }

    /// Returns the verbs that ended with success at least once.
    #[must_use]
    pub fn completed_verbs(&self) -> &BTreeSet<String> {
        &self.completed
    }
}
