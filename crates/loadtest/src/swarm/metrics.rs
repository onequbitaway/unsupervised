//! The numbers that all agents of a swarm write to.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use unsup_protocol::ActionAck;

use super::error::RequestError;
use super::wire::Ended;
use crate::local::ActionCounts;
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
    /// What the agents did in the window: acknowledgements by state and by
    /// verb, rejections by error code, and the reports that agents read.
    actions: Mutex<ActionCounts>,
    /// Requests without a usable answer in the window, by code.
    pub errors_steady: NamedCounts,
    /// Requests without a usable answer outside the window, by code: in
    /// the ramp-up, and while the run stops.
    pub errors_ramp: NamedCounts,
    /// Requests in the window that a limit of the server refused (status
    /// 429), by code. They are not in the failed requests.
    pub throttled_steady: NamedCounts,
    /// Requests outside the window that a limit of the server refused.
    pub throttled_ramp: NamedCounts,
    /// Number of requests in the window that a limit of the server
    /// refused.
    pub throttled: Counter,
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
            actions: Mutex::new(ActionCounts::default()),
            errors_steady: NamedCounts::default(),
            errors_ramp: NamedCounts::default(),
            throttled_steady: NamedCounts::default(),
            throttled_ramp: NamedCounts::default(),
            throttled: Counter::default(),
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

    /// Runs `change` on the action counts of the window.
    fn count(&self, change: impl FnOnce(&mut ActionCounts)) {
        // A poisoned lock still holds valid counts.
        let mut actions = match self.actions.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        change(&mut actions);
    }

    /// Returns a copy of the action counts of the window.
    #[must_use]
    pub fn actions(&self) -> ActionCounts {
        match self.actions.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Counts the acknowledgements of one request with its latency. A plan
    /// has several acknowledgements; each has the latency of the request.
    /// `requests` is the number of actions that the agent sent: the server
    /// stops a plan at the first rejection.
    pub fn acks(
        &self,
        agent: usize,
        link: Link,
        acks: &[ActionAck],
        requests: usize,
        latency: Duration,
    ) {
        for _ in acks {
            self.acks_total.add();
        }
        if !self.is_steady() {
            return;
        }
        for _ in acks {
            self.acks_steady.add();
            self.ack.record(agent, latency);
            match link {
                Link::Http => self.ack_http.record(agent, latency),
                Link::Ws => self.ack_ws.record(agent, latency),
            }
        }
        self.count(|actions| {
            for ack in acks {
                actions.record(ack);
            }
            actions.not_sent += requests.saturating_sub(acks.len()) as u64;
        });
    }

    /// Counts one observation with its latency.
    pub fn observation(&self, agent: usize, latency: Duration) {
        if self.is_steady() {
            self.observes_steady.add();
            self.observe.record(agent, latency);
        }
    }

    /// Counts how actions ended, from the reports that an agent read.
    pub fn ended(&self, ended: &[Ended]) {
        if ended.is_empty() || !self.is_steady() {
            return;
        }
        self.count(|actions| {
            for end in ended {
                actions.record_end(&end.verb, end.failed.as_deref());
            }
        });
    }

    /// Counts one request without a usable answer, and keeps its message
    /// when it is the first of its code. A request that a limit of the
    /// server refused is counted in a class of its own: it is not a failed
    /// request. A failure that shows a limit of this machine is kept too;
    /// the run then stops.
    pub fn error(&self, error: &RequestError) {
        self.error_examples.keep(&error.code, &error.message);
        if error.throttled() {
            if self.is_steady() {
                self.throttled.add();
                self.throttled_steady.add(&error.code);
            } else {
                self.throttled_ramp.add(&error.code);
            }
            return;
        }
        if self.is_steady() {
            self.failed_steady.add();
            self.errors_steady.add(&error.code);
        } else {
            self.errors_ramp.add(&error.code);
        }
        if error.machine_limit() {
            // Only the first one is kept.
            let _ = self.machine_limit.set(error.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_by_a_limit_of_the_server_is_not_a_failed_request() {
        let metrics = Metrics::new(Duration::from_secs(10), 3).expect("valid bounds");
        let limited = RequestError::from_status(429, b"", Some(2));
        let broken = RequestError::from_status(503, b"", None);

        // Before the window opens, both are counted outside it.
        metrics.error(&limited);
        metrics.error(&broken);
        assert_eq!(metrics.throttled_ramp.get().get("http_429"), Some(&1));
        assert_eq!(metrics.errors_ramp.get().get("http_503"), Some(&1));
        assert_eq!((metrics.throttled.get(), metrics.failed_steady.get()), (0, 0));

        metrics.set_steady(true);
        metrics.error(&limited);
        metrics.error(&limited);
        metrics.error(&broken);
        assert_eq!(metrics.throttled.get(), 2);
        assert_eq!(metrics.throttled_steady.get().get("http_429"), Some(&2));
        assert_eq!(metrics.failed_steady.get(), 1, "only the 503 is a failed request");
        assert_eq!(metrics.errors_steady.get().get("http_429"), None);
        assert!(metrics.error_examples.get().contains_key("http_429"));
    }

    #[test]
    fn the_ends_of_actions_are_counted_in_the_window() {
        let metrics = Metrics::new(Duration::from_secs(10), 3).expect("valid bounds");
        let ended = [
            Ended { verb: "work".into(), failed: None },
            Ended { verb: "work".into(), failed: Some("interrupted".into()) },
        ];
        metrics.ended(&ended);
        assert_eq!(metrics.actions().reports_finished, 0, "the window is not open");
        metrics.set_steady(true);
        metrics.ended(&ended);
        let actions = metrics.actions();
        assert_eq!((actions.reports_finished, actions.reports_failed), (1, 1));
        assert_eq!(actions.failed_by_code.get("interrupted"), Some(&1));
        assert_eq!(actions.by_verb["work"].failed, 1);
    }

    #[test]
    fn each_acknowledgement_of_a_plan_has_the_latency_of_its_request() {
        use unsup_protocol::{ActionError, ActionStatus, ErrorCode, Tick};
        let metrics = Metrics::new(Duration::from_secs(10), 3).expect("valid bounds");
        let ack = |verb: &str, status| ActionAck {
            request_id: None,
            action_id: 1,
            verb: verb.into(),
            status,
        };
        let walk = ack("move_to", ActionStatus::Started { ends_at: Tick(5), ticks: 5 });
        let work = ack("work", ActionStatus::Queued { position: 1 });
        let refused = ack(
            "move_to",
            ActionStatus::Rejected {
                error: ActionError {
                    code: ErrorCode::NotFound,
                    message: String::new(),
                    details: serde_json::Value::Null,
                },
            },
        );
        metrics.set_steady(true);
        metrics.acks(3, Link::Ws, &[walk, work], 2, Duration::from_millis(4));
        // The simulation rejected the walk: the server did not send the
        // action behind it.
        metrics.acks(3, Link::Http, &[refused], 2, Duration::from_millis(9));
        assert_eq!(metrics.acks_steady.get(), 3);
        assert_eq!(metrics.ack.summary().count, 3);
        assert_eq!((metrics.ack_ws.summary().count, metrics.ack_http.summary().count), (2, 1));
        let actions = metrics.actions();
        assert_eq!((actions.sent, actions.started, actions.queued, actions.rejected), (3, 1, 1, 1));
        assert_eq!(actions.not_sent, 1);
        assert_eq!(actions.rejected_by_code.get("not_found"), Some(&1));
    }
}
