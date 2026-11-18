//! Scenario tests of the full game in one process.
//!
//! The tests in this crate build a world with every game module, as the
//! server does, and play whole stories through verbs: a newcomer that
//! survives, a heist that ends in jail, an election that changes a law.
//! They are the proof that the modules work together.
//!
//! The library part is the support for those tests:
//!
//! - [`Game`] is a world with every game module. It sends the verbs of a
//!   resident, runs frames until the report is there, keeps the public
//!   events and audits the whole world after every frame.
//! - [`Setup`] says what world a story needs.
//! - [`assert_deterministic`] plays a story three times and compares the
//!   state hashes: two plain runs, and one run that restores the world
//!   from its own snapshot again and again.
//!
//! A story uses verbs for every step of the game. It can place a resident
//! and let time pass. Money comes from the faucets of the game.

mod acting;
mod determinism;
mod game;
mod places;
mod reads;
pub mod republic;
mod time;

pub use determinism::{assert_deterministic, assert_deterministic_in_republic};
pub use game::{Game, Setup};
pub use republic::Republic;
