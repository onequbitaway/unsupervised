//! The life module: the physical world and the body of a resident.
//!
//! This crate is a plugin for the kernel. It builds the map of every city,
//! lets residents walk and travel, gives them needs that they must meet to
//! stay alive, skills, an age, a death and an estate.
//!
//! The rules and the formulas are in `docs/design/modules/life.md`. The
//! numbers are in `data/balance.toml`. The shared names (site kinds, tile
//! codes, attributes, conditions, verbs, events) come from
//! `docs/design/game-design.md`; this crate has them in [`names`].
//!
//! # Map of the crate
//!
//! - `worldgen`: districts, roads, sites and tiles of every zone.
//! - `path`, `walk`, `travel`: the verbs `move_to` and `travel`.
//! - `needs`, `reflex`: the hourly pass over food, energy, health and mood.
//! - `eat`, `care`: the verbs `eat`, `use_item`, `sleep`, `rest`, `heal`.
//! - `skills`: the level formula and the verb `train`.
//! - `age`, `death`: ageing, death and estates. The verb `write_will`.
//! - `give`, `carry`, `inspect`: the verbs `give`, `drop`, `pick_up` and
//!   `inspect`.
//! - `observe`, `spawn`, `shelters`: the observation sections and the start
//!   of a life.
//!
//! # Example
//!
//! ```
//! use unsup_kernel::testing::TestWorld;
//!
//! let life = unsup_life::plugin();
//! let mut world = TestWorld::new(&[&life]);
//! let ada = world.spawn("Ada");
//! // A new resident has full needs and is 18 years old.
//! assert_eq!(world.attr(ada, "need.food"), 1_000);
//! assert_eq!(world.attr(ada, "age.years"), 18);
//! // One game hour later it is a little more hungry.
//! world.step(60);
//! assert_eq!(world.attr(ada, "need.food"), 1_000 - 14);
//! ```

mod age;
mod body;
mod care;
mod carry;
mod death;
mod eat;
mod give;
mod handles;
mod inspect;
mod math;
pub mod names;
mod needs;
mod observe;
mod path;
mod reflex;
mod shelters;
mod sites;
mod skills;
mod spawn;
mod tiles;
mod travel;
mod walk;
mod will;
mod worldgen;

use unsup_kernel::{Builder, Kin, Plugin};

pub use crate::body::{Bed, Body};
pub use crate::care::{
    HealArgs, RestArgs, SleepArgs, SleepQuality, sleep_quality, sleep_quality_in,
};
pub use crate::carry::CarryArgs;
pub use crate::death::{Estate, Estates, die};
pub use crate::eat::{EatArgs, UseItemArgs};
pub use crate::give::{GIFT_RECEIVED, GiveArgs};
pub use crate::inspect::InspectArgs;
pub use crate::needs::{NEED_EMPTY, NEED_LOW};
pub use crate::reflex::default_routines;
pub use crate::shelters::{Shelter, Shelters};
pub use crate::sites::{Place, SiteIndex};
pub use crate::skills::{TrainArgs, XP_PER_LEVEL_SQUARED, skill_attr, skill_level, xp_for_level};
pub use crate::travel::{Station, Stations, TravelArgs};
pub use crate::walk::{MoveToArgs, PathBuffers, Walkers};
pub use crate::will::WriteWillArgs;
pub use crate::worldgen::{District, ZoneFacts};

/// The life module.
#[derive(Clone, Copy, Debug, Default)]
pub struct LifePlugin;

impl Plugin for LifePlugin {
    fn name(&self) -> &'static str {
        "life"
    }

    fn build(&self, b: &mut Builder) {
        let handles = handles::Handles::register(b);
        b.component::<Kin>();
        b.component::<Body>();

        worldgen::register(b);
        sites::register(b);
        shelters::register(b);
        spawn::register(b, handles);
        walk::register(b, handles);
        travel::register(b, handles);
        needs::register(b, handles);
        age::register(b, handles);
        death::register(b);
        eat::register(b, handles);
        care::register(b, handles);
        skills::register(b, handles);
        give::register(b, handles);
        carry::register(b, handles);
        will::register(b, handles);
        inspect::register(b, handles);
        observe::register(b, handles);
    }
}

/// Returns the plugin of the life module.
#[must_use]
pub fn plugin() -> LifePlugin {
    LifePlugin
}
