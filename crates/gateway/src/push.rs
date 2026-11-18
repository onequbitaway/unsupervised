//! The hub that tells watchers when a resident has news.
//!
//! One task reads the frames of the simulation. Each frame lists the
//! residents that got a notice or a report. The hub wakes the watchers of
//! those residents: WebSocket connections and webhooks. A watcher then
//! reads the news with its own cursor, so a wake can never lose news; it
//! can only come late.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;
use tokio::sync::broadcast::{self, error::RecvError};
use unsup_protocol::{
    ActionReport, Detail, FrameOutput, Notice, ObserveOpts, ResidentId, SimError,
};
use unsup_runtime::SimHandle;

/// The section of an observation that holds the action reports.
const REPORTS: &str = "reports";

/// Tells watchers when their resident has news.
#[derive(Debug, Default)]
pub(crate) struct Hub {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    last_id: u64,
    watchers: HashMap<ResidentId, Vec<(u64, Arc<Notify>)>>,
}

/// A registration at the hub. Dropping it ends the registration.
#[derive(Debug)]
pub(crate) struct Watch {
    hub: Arc<Hub>,
    resident: ResidentId,
    id: u64,
    notify: Arc<Notify>,
}

impl Hub {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a watcher for the news of one resident.
    pub(crate) fn watch(self: &Arc<Self>, resident: ResidentId) -> Watch {
        let notify = Arc::new(Notify::new());
        let mut state = self.lock();
        state.last_id += 1;
        let id = state.last_id;
        state.watchers.entry(resident).or_default().push((id, Arc::clone(&notify)));
        Watch { hub: Arc::clone(self), resident, id, notify }
    }

    /// Returns the number of watchers.
    pub(crate) fn watchers(&self) -> u64 {
        self.lock().watchers.values().map(|list| list.len() as u64).sum()
    }

    /// Wakes the watchers of the residents in `woke`, which is sorted.
    fn wake(&self, woke: &[ResidentId]) {
        let state = self.lock();
        if state.watchers.len() < woke.len() {
            for (resident, list) in &state.watchers {
                if woke.binary_search(resident).is_ok() {
                    list.iter().for_each(|(_, notify)| notify.notify_one());
                }
            }
        } else {
            for resident in woke {
                if let Some(list) = state.watchers.get(resident) {
                    list.iter().for_each(|(_, notify)| notify.notify_one());
                }
            }
        }
    }

    /// Wakes every watcher. The hub does this when it lost frames: each
    /// watcher then looks for news once.
    fn wake_all(&self) {
        let state = self.lock();
        state.watchers.values().flatten().for_each(|(_, notify)| notify.notify_one());
    }

    /// Reads the frames of the simulation until it stops.
    pub(crate) async fn run(self: Arc<Self>, mut frames: broadcast::Receiver<Arc<FrameOutput>>) {
        loop {
            match frames.recv().await {
                Ok(frame) => self.wake(&frame.woke),
                Err(RecvError::Lagged(_)) => self.wake_all(),
                Err(RecvError::Closed) => return,
            }
        }
    }
}

impl Watch {
    /// Waits until the resident may have news. A wake that came before
    /// the call is kept, so the caller cannot miss one.
    pub(crate) async fn changed(&self) {
        self.notify.notified().await;
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let mut state = self.hub.lock();
        if let Some(list) = state.watchers.get_mut(&self.resident) {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                state.watchers.remove(&self.resident);
            }
        }
    }
}

/// What a resident received since a cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct News {
    pub(crate) notices: Vec<Notice>,
    pub(crate) reports: Vec<ActionReport>,
    /// The cursor for the next read.
    pub(crate) next: u64,
}

impl News {
    pub(crate) fn is_empty(&self) -> bool {
        self.notices.is_empty() && self.reports.is_empty()
    }
}

/// Reads the notices and the reports of a resident with a sequence number
/// at or above `since`.
pub(crate) async fn news_since(
    sim: &SimHandle,
    resident: ResidentId,
    since: u64,
) -> Result<News, SimError> {
    let opts = ObserveOpts {
        detail: Detail::Brief,
        since_notice: since,
        sections: Some(vec![REPORTS.to_string()]),
    };
    let mut observation = sim.observe(resident, opts).await?;
    let reports = match observation.sections.remove(REPORTS) {
        // The kernel writes this section from `ActionReport` values.
        Some(value) => serde_json::from_value(value).map_err(|_| SimError::Invalid)?,
        None => Vec::new(),
    };
    Ok(News { notices: observation.notices, reports, next: observation.next_notice })
}

/// Returns the cursor that sees only news from now on.
pub(crate) async fn cursor_now(sim: &SimHandle, resident: ResidentId) -> Result<u64, SimError> {
    let opts =
        ObserveOpts { detail: Detail::Brief, since_notice: u64::MAX, sections: Some(Vec::new()) };
    Ok(sim.observe(resident, opts).await?.next_notice)
}
