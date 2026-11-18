//! Operations: what an administrator needs to see and stop problems.
//!
//! One task reads the frames of the simulation. It feeds the funnel
//! detector, keeps the newest economy statistics that a module
//! published, and looks at the statistics of the simulation for the
//! alert list.

pub(crate) mod alerts;
pub(crate) mod funnel;
mod watch;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError};
use unsup_protocol::{Detail, Event, FrameOutput, ResidentId, SimError};

use self::alerts::Alerts;
use self::funnel::{Flag, Funnel, FunnelConfig, GiftEvent};
use self::watch::SimWatch;
use crate::actions::observe_opts;
use crate::state::Gateway;

/// Kind of the event that the kernel emits for a new resident.
const SPAWNED: &str = "kernel.spawned";
/// Kind of the event that the kernel emits for a resident that left.
const DESPAWNED: &str = "kernel.despawned";
/// Greatest number of series of published statistics that are kept.
const SERIES_MAX: usize = 64;

/// The detectors of abuse. The values come from the `[abuse]` table of
/// the gateway configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbuseConfig {
    /// The funnel detector.
    pub funnel: FunnelConfig,
}

/// What the gateway watches for an administrator. The values come from
/// the `[ops]` table of the gateway configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpsConfig {
    /// Time between two looks at the statistics of the simulation for the
    /// alert list, in milliseconds. 0: only a request of the alert list
    /// or of the dashboard looks.
    pub watch_interval_ms: u64,
    /// Greatest number of kinds of alerts in the list.
    pub max_alerts: usize,
    /// Kind of the event with the public economy statistics.
    pub economy_kind: String,
}

/// A function of the host that describes what only the host knows: the
/// data directory, the recovery at the start, the snapshots.
pub(crate) type HostInfo = Arc<dyn Fn() -> Value + Send + Sync>;

/// The state of the watcher.
pub(crate) struct Ops {
    pub(crate) alerts: Arc<Alerts>,
    funnel: Mutex<Funnel>,
    economy_kind: String,
    /// The newest published statistics, for each key.
    economy: Mutex<BTreeMap<String, Value>>,
    sim_watch: Mutex<SimWatch>,
    host: Mutex<Option<HostInfo>>,
    /// The last frame that the watcher handled completely.
    frame: AtomicU64,
    frames_lost: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads a whole number from a field of event data.
fn number(data: &Value, field: &str) -> Option<i64> {
    if field.is_empty() { None } else { data.get(field).and_then(Value::as_i64) }
}

/// A gift, as an event tells it.
struct Seen {
    from: ResidentId,
    to: ResidentId,
    value: (i64, i64),
    item: Option<String>,
}

fn read_gift(rule: &GiftEvent, event: &Event) -> Option<Seen> {
    let id = |field: &str| {
        number(&event.data, field).and_then(|id| u64::try_from(id).ok()).map(ResidentId)
    };
    let from = if rule.from.is_empty() { event.actor } else { id(&rule.from) }?;
    let item = event.data.get(&rule.item).and_then(Value::as_str).map(str::to_string);
    let value = (
        number(&event.data, &rule.money).unwrap_or(0),
        number(&event.data, &rule.goods).unwrap_or(0),
    );
    Some(Seen { from, to: id(&rule.to)?, value, item })
}

impl Ops {
    pub(crate) fn new(config: &OpsConfig, funnel: FunnelConfig) -> Ops {
        Ops {
            alerts: Arc::new(Alerts::new(config.max_alerts)),
            funnel: Mutex::new(Funnel::new(funnel)),
            economy_kind: config.economy_kind.clone(),
            economy: Mutex::new(BTreeMap::new()),
            sim_watch: Mutex::new(SimWatch::default()),
            host: Mutex::new(None),
            frame: AtomicU64::new(0),
            frames_lost: AtomicU64::new(0),
        }
    }

    pub(crate) fn set_host(&self, info: HostInfo) {
        *lock(&self.host) = Some(info);
    }

    /// Returns what the host says about itself, or `null`.
    pub(crate) fn host(&self) -> Value {
        let info = lock(&self.host).clone();
        info.map_or(Value::Null, |info| info())
    }

    /// Returns the last frame that the watcher handled.
    pub(crate) fn frame(&self) -> u64 {
        self.frame.load(Ordering::Acquire)
    }

    pub(crate) fn frames_lost(&self) -> u64 {
        self.frames_lost.load(Ordering::Relaxed)
    }

    pub(crate) fn flags(&self) -> Vec<Flag> {
        lock(&self.funnel).flags()
    }

    pub(crate) fn dismiss(&self, flag: u64) -> bool {
        lock(&self.funnel).dismiss(flag)
    }

    /// Returns the newest published economy statistics: the series
    /// without a key when there is one, else every series by its key.
    pub(crate) fn economy(&self) -> Value {
        let series = lock(&self.economy);
        match (series.len(), series.get("")) {
            (0, _) => Value::Null,
            (1, Some(only)) => only.clone(),
            _ => json!(*series),
        }
    }

    fn publish(&self, event: &Event) {
        let key = match event.data.get("key") {
            Some(Value::String(key)) => key.clone(),
            Some(key @ (Value::Number(_) | Value::Bool(_))) => key.to_string(),
            _ => String::new(),
        };
        let mut series = lock(&self.economy);
        if series.len() >= SERIES_MAX && !series.contains_key(&key) {
            return;
        }
        series.insert(key, json!({ "frame": event.frame, "tick": event.tick, "data": event.data }));
    }
}

impl Gateway {
    /// Looks at the statistics of the simulation and raises the alerts
    /// for what is new since the last look.
    pub(crate) async fn look_at_sim(&self) {
        let ops = &self.inner.ops;
        match self.sim().stats().await {
            Ok(stats) => lock(&ops.sim_watch).look(&stats, &ops.alerts),
            Err(SimError::Shutdown) if !*self.inner.stopping.borrow() => {
                lock(&ops.sim_watch).stopped(&ops.alerts);
            }
            // A busy simulation shows in its own counters at the next look.
            Err(_) => {}
        }
    }

    /// Finds out when a resident appeared, for a giver that the detector
    /// does not know: it appeared before this server started.
    async fn appearance(&self, resident: ResidentId) -> Option<u64> {
        let opts = observe_opts(Some(Detail::Full), Some(u64::MAX), Some(vec!["self".into()]));
        let observation = self.sim().observe(resident, opts).await.ok()?;
        let born = observation.sections.get("self")?.get("born")?.as_u64()?;
        // `born` is a tick of the zone. The age in ticks is the age in
        // frames, as long as the zone was not slowed down.
        Some(observation.frame.saturating_sub(observation.tick.0.saturating_sub(born)))
    }

    /// Gives one gift to the detector, and writes the owners into the
    /// flag that it raised or extended.
    async fn gift(&self, rule: &GiftEvent, event: &Event) {
        let ops = &self.inner.ops;
        let Some(seen) = read_gift(rule, event) else {
            return;
        };
        if !lock(&ops.funnel).knows(seen.from)
            && let Some(frame) = self.appearance(seen.from).await
        {
            lock(&ops.funnel).appeared(seen.from, frame);
        }
        let flagged = {
            let mut funnel = lock(&ops.funnel);
            let before = funnel.flags().len();
            let flag = funnel.gift(event.frame, seen.from, seen.to, seen.value, seen.item);
            flag.map(|id| (id, funnel.flags().len() > before))
        };
        let Some((flag, new)) = flagged else {
            return;
        };
        if new {
            tracing::warn!(flag, receiver = %seen.to, "abuse: a funnel was flagged");
        }
        let unknown = lock(&ops.funnel).without_owner(flag);
        for resident in unknown {
            if let Ok(Some(record)) = self.store(move |store| store.resident(resident)).await {
                lock(&ops.funnel).set_owner(flag, resident, record.owner);
            }
        }
    }

    /// Handles one frame of the simulation.
    async fn watch_frame(&self, frame: &FrameOutput) {
        let ops = &self.inner.ops;
        let last = ops.frame();
        if last > 0 && frame.frame <= last {
            // The world went back to a snapshot.
            lock(&ops.funnel).rolled_back();
        }
        for event in &frame.events {
            match event.kind.as_str() {
                SPAWNED => {
                    if let Some(resident) = event.actor {
                        lock(&ops.funnel).appeared(resident, event.frame);
                    }
                }
                DESPAWNED => {
                    if let Some(resident) = event.actor {
                        lock(&ops.funnel).left(resident);
                    }
                }
                kind if kind == ops.economy_kind => ops.publish(event),
                kind => {
                    let rule = lock(&ops.funnel).rule(kind).cloned();
                    if let Some(rule) = rule {
                        self.gift(&rule, event).await;
                    }
                }
            }
        }
        ops.frame.store(frame.frame, Ordering::Release);
    }

    /// Reads the frames of the simulation until it stops or the gateway
    /// stops.
    pub(crate) async fn watch(self, mut frames: broadcast::Receiver<Arc<FrameOutput>>) {
        let every = self.config().ops.watch_interval_ms;
        // An interval of zero would never end its first tick.
        let mut look = tokio::time::interval(Duration::from_millis(every.max(1)));
        look.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                frame = frames.recv() => match frame {
                    Ok(frame) => self.watch_frame(&frame).await,
                    Err(RecvError::Lagged(lost)) => {
                        let ops = &self.inner.ops;
                        ops.frames_lost.fetch_add(lost, Ordering::Relaxed);
                        ops.alerts.raise(
                            "frames_lost",
                            format!(
                                "The watcher of the gateway lost {lost} frames. The funnel \
                                 detector did not see their events."
                            ),
                        );
                    }
                    Err(RecvError::Closed) => {
                        // The simulation is gone. When the server did not
                        // stop it, that is an alert.
                        self.look_at_sim().await;
                        return;
                    }
                },
                _ = look.tick(), if every > 0 => self.look_at_sim().await,
                () = self.stopped() => return,
            }
        }
    }
}
