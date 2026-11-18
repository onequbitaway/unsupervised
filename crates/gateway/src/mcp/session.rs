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

    /// Moves the cursor forward. It never moves back.
    pub(crate) fn advance(&self, key: &CursorKey, to: u64) {
        let mut state = self.lock();
        match key {
            CursorKey::Session(id) => {
                if let Some(session) = state.sessions.get_mut(id) {
                    session.cursor = session.cursor.max(to);
                }
            }
            CursorKey::Resident(resident) => {
                let cursor = state.cursors.entry(*resident).or_insert(0);
                *cursor = (*cursor).max(to);
            }
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
