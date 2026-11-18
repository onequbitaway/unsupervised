//! What the MCP layer remembers between two calls: sessions and notice
//! cursors.
//!
//! An agent that speaks MCP never handles a notice cursor. The server
//! keeps it:
//!
//! - A client of a protocol version with the `initialize` handshake gets
//!   a session id. The cursor belongs to the session.
//! - A client of a stateless protocol version has no session. The cursor
//!   belongs to the resident key that every request carries.
//!
//! Notices and action reports share one sequence, and each of them
//! carries its number. The cursor is a number of that sequence: the
//! client saw everything below it.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use unsup_protocol::ResidentId;
use unsup_store::{StoreError, random_hex};

use crate::config::McpConfig;

/// Number of random bytes in a session id.
const SESSION_ID_BYTES: usize = 16;

/// Names the cursor of a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CursorKey {
    /// The cursor of a session.
    Session(String),
    /// The cursor of a resident without a session.
    Resident(ResidentId),
}

/// The sessions and cursors of the MCP endpoint.
#[derive(Debug)]
pub(crate) struct Sessions {
    idle: Duration,
    max: usize,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    sessions: HashMap<String, Session>,
    cursors: HashMap<ResidentId, u64>,
}

#[derive(Debug)]
struct Session {
    resident: ResidentId,
    cursor: u64,
    used: Instant,
}

impl Sessions {
    pub(crate) fn new(config: &McpConfig) -> Sessions {
        Sessions {
            idle: Duration::from_secs(config.session_idle_secs),
            max: config.max_sessions.max(1),
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens a session for a resident and returns its id. When the table
    /// is full, the sessions that were idle too long go, and then the one
    /// that was idle longest.
    pub(crate) fn open(&self, resident: ResidentId, now: Instant) -> Result<String, StoreError> {
        let id = random_hex(SESSION_ID_BYTES)?;
        let mut state = self.lock();
        if state.sessions.len() >= self.max {
            let idle = self.idle;
            state.sessions.retain(|_, s| now.saturating_duration_since(s.used) < idle);
        }
        if state.sessions.len() >= self.max {
            let oldest =
                state.sessions.iter().min_by_key(|(_, s)| s.used).map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                state.sessions.remove(&oldest);
            }
        }
        state.sessions.insert(id.clone(), Session { resident, cursor: 0, used: now });
        Ok(id)
    }

    /// Returns true when the session exists, belongs to the resident and
    /// was not idle too long. Marks it as used now.
    pub(crate) fn touch(&self, id: &str, resident: ResidentId, now: Instant) -> bool {
        let mut state = self.lock();
        let Some(session) = state.sessions.get_mut(id) else {
            return false;
        };
        if session.resident != resident {
            return false;
        }
        if now.saturating_duration_since(session.used) < self.idle {
            session.used = now;
            true
        } else {
            state.sessions.remove(id);
            false
        }
    }

    /// Ends a session of a resident. Returns false when there was none.
    pub(crate) fn close(&self, id: &str, resident: ResidentId) -> bool {
        let mut state = self.lock();
        let owned = state.sessions.get(id).is_some_and(|session| session.resident == resident);
        owned && state.sessions.remove(id).is_some()
    }

    /// Returns the cursor: the `next_notice` of the last observation that
    /// the caller got, or 0.
    pub(crate) fn cursor(&self, key: &CursorKey) -> u64 {
        let state = self.lock();
        match key {
            CursorKey::Session(id) => state.sessions.get(id).map_or(0, |session| session.cursor),
            CursorKey::Resident(resident) => state.cursors.get(resident).copied().unwrap_or(0),
        }
    }

    /// Takes the news of one observation for the caller: moves the cursor
    /// to `next`, the end of the observation, and returns the sequence
    /// number below which the caller shows nothing. `since` is the cursor
    /// that the observation was made with.
    ///
    /// Two calls of one client can run at the same time and read the same
    /// news. The call that comes here second gets a higher number back,
    /// and drops what the first call showed. So each notice and each
    /// report is shown one time.
    ///
    /// When `next` is below `since`, the world went back to a snapshot.
    /// The cursor goes back with it.
    pub(crate) fn claim(&self, key: &CursorKey, since: u64, next: u64) -> u64 {
        let mut state = self.lock();
        let cursor = match key {
            CursorKey::Session(id) => state.sessions.get_mut(id).map(|session| &mut session.cursor),
            CursorKey::Resident(resident) => Some(state.cursors.entry(*resident).or_insert(0)),
        };
        // The session ended while the call ran. Nothing remembers a cursor.
        let Some(cursor) = cursor else {
            return since;
        };
        if next < since {
            *cursor = next;
            return next;
        }
        let shown_before = (*cursor).max(since);
        *cursor = (*cursor).max(next);
        shown_before
    }

    /// Returns every resident that has a cursor.
    pub(crate) fn residents(&self) -> Vec<ResidentId> {
        let state = self.lock();
        let mut residents: Vec<ResidentId> = state
            .sessions
            .values()
            .map(|session| session.resident)
            .chain(state.cursors.keys().copied())
            .collect();
        residents.sort_unstable();
        residents.dedup();
        residents
    }

    /// Moves every cursor of a resident back to `end`, the end of its
    /// inbox, when the cursor is behind that end. The gateway calls it
    /// after a rollback of the world: the inbox is shorter then, and the
    /// news that comes next gets numbers that the cursor already passed.
    pub(crate) fn clamp(&self, resident: ResidentId, end: u64) {
        let mut state = self.lock();
        for session in state.sessions.values_mut().filter(|s| s.resident == resident) {
            session.cursor = session.cursor.min(end);
        }
        if let Some(cursor) = state.cursors.get_mut(&resident) {
            *cursor = (*cursor).min(end);
        }
    }

    /// Forgets everything about a resident that left the world.
    pub(crate) fn forget(&self, resident: ResidentId) {
        let mut state = self.lock();
        state.sessions.retain(|_, session| session.resident != resident);
        state.cursors.remove(&resident);
    }

    /// Returns the number of open sessions.
    pub(crate) fn len(&self) -> usize {
        self.lock().sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use unsup_protocol::ResidentId;

    use super::{CursorKey, Sessions};
    use crate::config::GatewayConfig;

    fn sessions() -> Sessions {
        Sessions::new(&GatewayConfig::default().mcp)
    }

    #[test]
    fn two_calls_with_the_same_cursor_share_the_news_without_a_repeat() {
        let sessions = sessions();
        let key = CursorKey::Resident(ResidentId(1));
        // Both calls read the cursor 0. The first one saw the news up to
        // 5, the second one, a moment later, up to 7.
        assert_eq!(sessions.claim(&key, 0, 5), 0, "the first call shows everything from 0");
        assert_eq!(sessions.claim(&key, 0, 7), 5, "the second call shows only 5 and 6");
        assert_eq!(sessions.cursor(&key), 7);
        // A call that ends late does not move the cursor back, and shows
        // nothing: everything below 7 was shown.
        assert_eq!(sessions.claim(&key, 0, 6), 7);
        assert_eq!(sessions.cursor(&key), 7);
    }

    #[test]
    fn a_rollback_of_the_world_moves_the_cursor_back() {
        let sessions = sessions();
        let id = sessions.open(ResidentId(1), Instant::now()).expect("random bytes");
        let key = CursorKey::Session(id);
        assert_eq!(sessions.claim(&key, 0, 9), 0);
        // The inbox now ends at 4, before the cursor 9 of the call.
        assert_eq!(sessions.claim(&key, 9, 4), 4);
        assert_eq!(sessions.cursor(&key), 4);
        assert_eq!(sessions.claim(&key, 4, 6), 4, "the news after the rollback is shown");
    }

    #[test]
    fn a_session_that_ended_keeps_no_cursor() {
        let sessions = sessions();
        let key = CursorKey::Session("gone".into());
        assert_eq!(sessions.claim(&key, 3, 8), 3);
        assert_eq!(sessions.cursor(&key), 0);
    }
}
