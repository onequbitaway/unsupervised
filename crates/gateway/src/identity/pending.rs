//! The sign-ins that were started and wait for the answer of the service.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use unsup_store::{StoreError, TokenHash, random_hex};

/// Number of random bytes in a state value and in a proof key.
const RANDOM_BYTES: usize = 32;

/// One sign-in that waits.
#[derive(Debug)]
struct Waiting {
    /// The proof key (PKCE verifier) that goes to the token endpoint.
    verifier: String,
    expires: Instant,
}

/// The sign-ins that wait, by the hash of their state value.
///
/// A state is good one time and for a limited time. The table has a
/// greatest size, so that a client cannot fill the memory with starts.
#[derive(Debug)]
pub(crate) struct PendingStates {
    ttl: Duration,
    max: usize,
    waiting: Mutex<HashMap<TokenHash, Waiting>>,
}

impl PendingStates {
    pub(crate) fn new(ttl: Duration, max: usize) -> PendingStates {
        PendingStates { ttl, max: max.max(1), waiting: Mutex::new(HashMap::new()) }
    }

    /// Starts a sign-in at the time `now`. Returns the state value and
    /// the proof key, or `None` when too many sign-ins wait.
    pub(crate) fn start(&self, now: Instant) -> Result<Option<(String, String)>, StoreError> {
        let state = random_hex(RANDOM_BYTES)?;
        let verifier = random_hex(RANDOM_BYTES)?;
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        if waiting.len() >= self.max {
            waiting.retain(|_, entry| entry.expires > now);
        }
        if waiting.len() >= self.max {
            return Ok(None);
        }
        let entry = Waiting { verifier: verifier.clone(), expires: now + self.ttl };
        waiting.insert(TokenHash::of(&state), entry);
        Ok(Some((state, verifier)))
    }

    /// Ends the sign-in with this state value and returns its proof key.
    /// Returns `None` for a state that this table did not make, that was
    /// used already, or that is too old.
    pub(crate) fn finish(&self, state: &str, now: Instant) -> Option<String> {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = waiting.remove(&TokenHash::of(state))?;
        (entry.expires > now).then_some(entry.verifier)
    }

    /// Returns the number of sign-ins that wait.
    pub(crate) fn len(&self) -> usize {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::PendingStates;

    #[test]
    fn a_state_is_good_one_time_and_for_a_limited_time() {
        let table = PendingStates::new(Duration::from_secs(600), 8);
        let now = Instant::now();
        let (state, verifier) = table.start(now).expect("random bytes").expect("room");
        assert_eq!((state.len(), verifier.len()), (64, 64));
        assert_ne!(state, verifier);
        assert_eq!(table.finish("not-a-state", now), None);
        assert_eq!(table.finish(&state, now + Duration::from_secs(599)), Some(verifier));
        assert_eq!(table.finish(&state, now), None, "the state was used");

        let (late, _) = table.start(now).expect("random bytes").expect("room");
        assert_eq!(table.finish(&late, now + Duration::from_secs(600)), None, "too old");
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn a_full_table_takes_no_start_until_a_state_is_too_old() {
        let table = PendingStates::new(Duration::from_secs(10), 2);
        let now = Instant::now();
        let (first, _) = table.start(now).expect("random bytes").expect("room");
        table.start(now + Duration::from_secs(5)).expect("random bytes").expect("room");
        assert_eq!(table.start(now + Duration::from_secs(6)).expect("random bytes"), None);
        // The first state is too old now, and makes room.
        assert!(table.start(now + Duration::from_secs(10)).expect("random bytes").is_some());
        assert_eq!(table.finish(&first, now + Duration::from_secs(10)), None);
        assert_eq!(table.len(), 2);
    }
}
