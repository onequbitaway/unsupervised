//! A world with residents in this process, without the network.
//!
//! `bench` and `soak` share this code: the size of the world
//! ([`Shape`]), how it is built and filled ([`populate`]), and the
//! [`Driver`] that makes every resident act by a behaviour profile.

mod counters;
mod driver;
mod scene;
mod setup;
mod shape;

pub use counters::{ActionCounts, VerbCounts};
pub use driver::{Driver, DriverOptions};
pub use scene::{LocalScene, Scratch};
pub use setup::{Placement, Population, WorldSpec, build_world, populate};
pub use shape::Shape;
