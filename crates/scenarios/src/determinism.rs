//! The determinism check of a story.

use crate::game::{Game, Setup};
use crate::republic::{Republic, republic};

/// Frames between two restores in the run that restores. The number is
/// odd, so that the restores come at moments of every kind: in the middle
/// of an action, of a walk, of a wait for the global realm.
const RESTORE_EVERY: u64 = 331;

/// Plays a story three times and checks that it is deterministic. `start`
/// makes the world of each run.
fn check<T>(
    setup: &Setup,
    start: impl Fn(&Setup) -> (Game, T),
    story: impl Fn(&mut Game, &T),
) -> String {
    let (mut first, cast) = start(setup);
    story(&mut first, &cast);
    first.audit();

    let (mut second, cast) = start(setup);
    story(&mut second, &cast);
    assert_eq!(first.hash(), second.hash(), "two runs of the same story differ");

    let restoring = Setup { restore_every: Some(RESTORE_EVERY), ..setup.clone() };
    let (mut third, cast) = start(&restoring);
    story(&mut third, &cast);
    assert_eq!(first.hash(), third.hash(), "a run that was restored from its snapshots differs");
    first.hash()
}

/// Plays a story three times in a new world and checks that it is
/// deterministic.
///
/// - Two plain runs must end with the same state hash.
/// - A run in which the world is restored from its own snapshot every few
///   hundred frames must end with that hash too.
///
/// The story is played in full each time, with all its assertions. Returns
/// the hash.
///
/// # Panics
///
/// Panics when the hashes differ.
pub fn assert_deterministic(setup: &Setup, story: impl Fn(&mut Game)) -> String {
    check(setup, |setup| (Game::new(setup), ()), |g, ()| story(g))
}

/// Plays a story three times in the republic of [`republic`] and checks
/// that it is deterministic, as [`assert_deterministic`] does. Each run
/// starts from the snapshot of the founding.
///
/// # Panics
///
/// Panics when the hashes differ.
pub fn assert_deterministic_in_republic(
    setup: &Setup,
    story: impl Fn(&mut Game, &Republic),
) -> String {
    check(setup, republic, story)
}
