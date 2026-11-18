//! Webhooks: the server posts the news of a resident to an address.
//!
//! Each webhook has one task. The task sleeps until the hub says that the
//! resident has news, reads the news since its cursor and posts it. One
//! delivery holds every notice and report since the last good delivery,
//! so deliveries of one resident never overlap and arrive in order.
//!
//! A delivery that fails is sent again after a wait that doubles each
//! time. After the last attempt the task rests, then starts again with
//! whatever the inbox of the resident still holds. A receiver can get a
//! notice twice; the sequence number tells it.
//!
//! All deliveries share a fixed number of workers and a bounded queue in
//! front of them. A delivery that finds the queue full counts as a failed
//! attempt.

pub mod sign;
pub(crate) mod url;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use reqwest::Url;
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use unsup_protocol::{ResidentId, SimError};
use unsup_runtime::SimHandle;
use unsup_store::{Secret, random_hex, unix_now};

use crate::config::WebhookConfig;
use crate::push::{Hub, News, Watch, cursor_now, news_since};

/// Number of random bytes in the id of a delivery.
const DELIVERY_ID_BYTES: usize = 12;

/// Numbers about webhook deliveries since the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct WebhookStats {
    /// Webhooks that are set now.
    pub webhooks: u64,
    /// Deliveries that the receiver accepted.
    pub delivered: u64,
    /// Attempts that failed: no answer in time, or an answer that was not
    /// a success.
    pub failed_attempts: u64,
    /// Attempts that found the queue full.
    pub queue_full: u64,
    /// Deliveries that the server gave up on after the last attempt.
    pub abandoned: u64,
}

#[derive(Debug, Default)]
struct Counters {
    delivered: AtomicU64,
    failed_attempts: AtomicU64,
    queue_full: AtomicU64,
    abandoned: AtomicU64,
}

/// What every webhook task shares.
struct Shared {
    config: WebhookConfig,
    sim: SimHandle,
    /// `None` when the HTTP client could not be built. No delivery is
    /// sent then; every attempt fails.
    client: Option<reqwest::Client>,
    /// One permit for each delivery that is sent now.
    workers: Semaphore,
    /// One permit for each delivery that is sent now or waits for a
    /// worker.
    places: Semaphore,
    counters: Counters,
}

/// The webhooks of all residents.
pub(crate) struct Webhooks {
    shared: Arc<Shared>,
    hub: Arc<Hub>,
    tasks: Mutex<HashMap<ResidentId, JoinHandle<()>>>,
}

impl Webhooks {
    pub(crate) fn new(config: &WebhookConfig, sim: SimHandle, hub: Arc<Hub>) -> Webhooks {
        let workers = config.workers.max(1);
        let client = match url::client(config) {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::error!(%error, "cannot build the HTTP client for webhooks");
                None
            }
        };
        Webhooks {
            shared: Arc::new(Shared {
                config: config.clone(),
                sim,
                client,
                workers: Semaphore::new(workers),
                places: Semaphore::new(workers + config.queue),
                counters: Counters::default(),
            }),
            hub,
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Sets or replaces the webhook of a resident. Only news that comes
    /// after this call is delivered.
    pub(crate) async fn set(&self, resident: ResidentId, address: &str, secret: Secret) {
        self.clear(resident);
        let url = match url::parse(address, &self.shared.config) {
            Ok(url) => url,
            Err(reason) => {
                // The address was good when it was set; the rules of this
                // server are stricter now.
                tracing::warn!(%resident, reason, "a stored webhook is not used");
                return;
            }
        };
        // Watch first, then read the cursor: news between the two steps
        // wakes the task, and the cursor is before it.
        let watch = self.hub.watch(resident);
        let Ok(cursor) = cursor_now(&self.shared.sim, resident).await else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        let task = tokio::spawn(run(shared, resident, url, secret, watch, cursor));
        let old = self.tasks.lock().unwrap_or_else(PoisonError::into_inner).insert(resident, task);
        if let Some(old) = old {
            old.abort();
        }
    }

    /// Removes the webhook of a resident.
    pub(crate) fn clear(&self, resident: ResidentId) {
        let task = self.tasks.lock().unwrap_or_else(PoisonError::into_inner).remove(&resident);
        if let Some(task) = task {
            task.abort();
        }
    }

    /// Stops every delivery.
    pub(crate) fn stop(&self) {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, task) in tasks.drain() {
            task.abort();
        }
    }

    pub(crate) fn stats(&self) -> WebhookStats {
        let counters = &self.shared.counters;
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        // A task ends by itself when its resident leaves the world.
        tasks.retain(|_, task| !task.is_finished());
        WebhookStats {
            webhooks: tasks.len() as u64,
            delivered: counters.delivered.load(Ordering::Relaxed),
            failed_attempts: counters.failed_attempts.load(Ordering::Relaxed),
            queue_full: counters.queue_full.load(Ordering::Relaxed),
            abandoned: counters.abandoned.load(Ordering::Relaxed),
        }
    }
}

/// The task of one webhook.
async fn run(
    shared: Arc<Shared>,
    resident: ResidentId,
    url: Url,
    secret: Secret,
    watch: Watch,
    mut cursor: u64,
) {
    loop {
        watch.changed().await;
        // Deliver until the inbox has nothing new.
        loop {
            let news = match news_since(&shared.sim, resident, cursor).await {
                Ok(news) => news,
                Err(SimError::UnknownResident | SimError::Shutdown) => return,
                // The simulation is busy. The next wake tries again.
                Err(_) => break,
            };
            if news.is_empty() {
                // After a rollback of the world the inbox ends before the
                // cursor. The cursor goes back with it, or the webhook
                // would miss the news up to the old place.
                cursor = cursor.min(news.next);
                break;
            }
            if shared.deliver(resident, &url, &secret, &news).await {
                cursor = news.next;
            } else {
                // The receiver is down. Rest, then look again: the inbox
                // still holds the news, as far as its size permits.
                tokio::time::sleep(shared.config.rest()).await;
            }
        }
    }
}

impl Shared {
    /// Posts one delivery, with every attempt that the configuration
    /// permits. Returns true when the receiver accepted it.
    async fn deliver(&self, resident: ResidentId, url: &Url, secret: &Secret, news: &News) -> bool {
        let body = json!({
            "resident": resident,
            "notices": news.notices,
            "reports": news.reports,
            "next_notice": news.next,
        })
        .to_string();
        // Without an id the receiver cannot match two attempts, so a
        // delivery is not sent without one.
        let Ok(delivery) = random_hex(DELIVERY_ID_BYTES) else {
            return false;
        };
        let attempts = self.config.attempts.max(1);
        for attempt in 1..=attempts {
            if self.attempt(url, secret, &delivery, &body).await {
                self.counters.delivered.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            if attempt < attempts {
                tokio::time::sleep(self.config.backoff(attempt)).await;
            }
        }
        self.counters.abandoned.fetch_add(1, Ordering::Relaxed);
        // The address can hold a secret path, so the log has the host only.
        tracing::warn!(%resident, host = url.host_str(), attempts, "webhook delivery abandoned");
        false
    }

    /// Sends one attempt. Returns true when the receiver answered with a
    /// success status.
    async fn attempt(&self, url: &Url, secret: &Secret, delivery: &str, body: &str) -> bool {
        let Ok(_place) = self.places.try_acquire() else {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let (Ok(_worker), Some(client)) = (self.workers.acquire().await, &self.client) else {
            self.counters.failed_attempts.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let timestamp = unix_now();
        let sent = client
            .post(url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(sign::DELIVERY_HEADER, delivery)
            .header(sign::TIMESTAMP_HEADER, timestamp)
            .header(sign::SIGNATURE_HEADER, sign::sign(secret.reveal(), timestamp, body.as_bytes()))
            .body(body.to_string())
            .send()
            .await;
        let accepted = sent.is_ok_and(|response| response.status().is_success());
        if !accepted {
            self.counters.failed_attempts.fetch_add(1, Ordering::Relaxed);
        }
        accepted
    }
}
