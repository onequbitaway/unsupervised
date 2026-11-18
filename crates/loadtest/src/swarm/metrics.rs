//! The numbers that all agents of a swarm write to.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use unsup_protocol::{ActionAck, ActionStatus};

use super::error::RequestError;
use crate::stats::{self, Counts, Latency};

/// How an agent talks to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// One HTTP request for each call.
    Http,
    /// Frames on one WebSocket.
    Ws,
}

/// A counter that many tasks add to.
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// Adds one.
    pub fn add(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the count.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A number that goes up and down, with its greatest value.
#[derive(Debug, Default)]
pub struct Gauge {
    now: AtomicU64,
    peak: AtomicU64,
    total: AtomicU64,
}

impl Gauge {
    /// Counts one more.
    pub fn up(&self) {
        let now = self.now.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(now, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts one less.
    pub fn down(&self) {
        // The gauge never goes below zero: `down` follows an `up`.
        let _ = self.now.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
    }

    /// Returns the value now.
    #[must_use]
    pub fn now(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }

    /// Returns the greatest value so far.
    #[must_use]
    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }

    /// Returns how often the gauge went up.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
}

/// Counts by name behind a lock.
#[derive(Debug, Default)]
pub struct NamedCounts(Mutex<Counts>);

impl NamedCounts {
    /// Adds one to the count of `name`.
    pub fn add(&self, name: &str) {
        // A poisoned lock still holds valid counts.
        let mut counts = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        stats::bump(&mut counts, name, 1);
    }

    /// Returns a copy of the counts.
    #[must_use]
    pub fn get(&self) -> Counts {
        match self.0.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// The first message of each error code, as an example for the operator.
#[derive(Debug, Default)]
pub struct Examples(Mutex<BTreeMap<String, String>>);

impl Examples {
    /// Keeps `message` when `code` has no example yet.
    pub fn keep(&self, code: &str, message: &str) {
        // A poisoned lock still holds valid examples.
        let mut examples = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !examples.contains_key(code) {
            examples.insert(code.to_string(), message.to_string());
        }
    }

    /// Returns a copy of the examples, by code.
    #[must_use]
    pub fn get(&self) -> BTreeMap<String, String> {
        match self.0.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Everything that the agents count. Latencies and the counts with
/// `steady` in their name hold only the measurement window, after the
/// ramp-up.
#[derive(Debug)]
pub struct Metrics {
    steady: AtomicBool,
    /// Acknowledgement latency of all agents in the window.
    pub ack: Latency,
    /// Acknowledgement latency of the HTTP agents in the window.
    pub ack_http: Latency,
    /// Acknowledgement latency of the WebSocket agents in the window.
    pub ack_ws: Latency,
    /// Latency of observations in the window.
    pub observe: Latency,
    /// Latency of one sign-up (all its requests), in the whole run.
    pub signup: Latency,
    /// Acknowledgements in the window.
    pub acks_steady: Counter,
    /// Acknowledgements in the whole run.
    pub acks_total: Counter,
    /// Acknowledgements in the window by state (`done`, `started`, ...).
    pub ack_states: NamedCounts,
    /// Rejected actions in the window by error code of the game.
    pub rejections: NamedCounts,
    /// Requests without a usable answer in the window, by code.
    pub errors_steady: NamedCounts,
    /// Requests without a usable answer outside the window, by code: in
    /// the ramp-up, and while the run stops.
    pub errors_ramp: NamedCounts,
    /// The first message of each error code, in the whole run.
    pub error_examples: Examples,
    /// The first failure that showed a limit of this machine. The run
    /// stops when it is set.
    pub machine_limit: OnceLock<RequestError>,
    /// Number of failed requests in the window.
    pub failed_steady: Counter,
    /// Observations in the window.
    pub observes_steady: Counter,
    /// Waits for the end of an action in the window.
    pub waits_steady: Counter,
    /// Long polls that the server answered, in the whole run.
    pub long_polls: Counter,
    /// Notices and reports that the server pushed on a WebSocket.
    pub pushed: Counter,
    /// Agents that started.
    pub agents_started: Counter,
    /// Agents that signed up (or reused a key), connected and got their
    /// first observation.
    pub agents_ready: Counter,
    /// Agents that gave up, by reason.
    pub agents_failed: NamedCounts,
    /// Agents that signed up again because the server refused a stored key.
    pub credentials_renewed: Counter,
    /// Open WebSocket connections.
    pub ws_open: Gauge,
    /// Agents that hold an HTTP client of their own.
    pub http_agents: Gauge,
    /// WebSocket agents that use HTTP because the server has no WebSocket
    /// route.
    pub ws_fallbacks: Counter,
}

impl Metrics {
    /// Makes empty metrics. The histograms hold latencies up to `max`.
    ///
    /// # Errors
    ///
    /// Fails when the histogram bounds are not valid.
    pub fn new(max: Duration, sigfig: u8) -> Result<Metrics, String> {
        Ok(Metrics {
            steady: AtomicBool::new(false),
            ack: Latency::new(max, sigfig)?,
            ack_http: Latency::new(max, sigfig)?,
            ack_ws: Latency::new(max, sigfig)?,
            observe: Latency::new(max, sigfig)?,
            signup: Latency::new(max, sigfig)?,
            acks_steady: Counter::default(),
            acks_total: Counter::default(),
            ack_states: NamedCounts::default(),
            rejections: NamedCounts::default(),
            errors_steady: NamedCounts::default(),
            errors_ramp: NamedCounts::default(),
            error_examples: Examples::default(),
            machine_limit: OnceLock::new(),
            failed_steady: Counter::default(),
            observes_steady: Counter::default(),
            waits_steady: Counter::default(),
            long_polls: Counter::default(),
            pushed: Counter::default(),
            agents_started: Counter::default(),
            agents_ready: Counter::default(),
            agents_failed: NamedCounts::default(),
            credentials_renewed: Counter::default(),
            ws_open: Gauge::default(),
            http_agents: Gauge::default(),
            ws_fallbacks: Counter::default(),
        })
    }

    /// Opens or closes the measurement window.
    pub fn set_steady(&self, steady: bool) {
        self.steady.store(steady, Ordering::Relaxed);
    }

    /// Returns true inside the measurement window.
    #[must_use]
    pub fn is_steady(&self) -> bool {
        self.steady.load(Ordering::Relaxed)
    }

    /// Counts one acknowledgement with its latency.
    pub fn ack(&self, agent: usize, link: Link, ack: &ActionAck, latency: Duration) {
        self.acks_total.add();
        if !self.is_steady() {
            return;
        }
        self.acks_steady.add();
        self.ack.record(agent, latency);
        match link {
            Link::Http => self.ack_http.record(agent, latency),
            Link::Ws => self.ack_ws.record(agent, latency),
        }
        let state = match &ack.status {
            ActionStatus::Done { .. } => "done",
            ActionStatus::Started { .. } => "started",
            ActionStatus::Pending => "pending",
            ActionStatus::Queued { .. } => "queued",
            ActionStatus::Rejected { error } => {
                self.rejections.add(error.code.as_str());
                "rejected"
            }
        };
        self.ack_states.add(state);
    }

    /// Counts one observation with its latency.
    pub fn observation(&self, agent: usize, latency: Duration) {
        if self.is_steady() {
            self.observes_steady.add();
            self.observe.record(agent, latency);
        }
    }

    /// Counts one request without a usable answer, and keeps its message
    /// when it is the first of its code. A failure that shows a limit of
    /// this machine is kept too; the run then stops.
    pub fn error(&self, error: &RequestError) {
        if self.is_steady() {
            self.failed_steady.add();
            self.errors_steady.add(&error.code);
        } else {
            self.errors_ramp.add(&error.code);
        }
        self.error_examples.keep(&error.code, &error.message);
        if error.machine_limit() {
            // Only the first one is kept.
            let _ = self.machine_limit.set(error.clone());
        }
    }
}
