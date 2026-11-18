//! Moderation: every text passes the filters before it enters the
//! simulation.
//!
//! The gateway is the only door for text. A client sends text in three
//! places: the arguments of an action, the routines (which hold actions)
//! and the names of owners, residents and models. Each of them comes
//! here first. A text that a filter refuses stops the whole request, so
//! the simulation, the notices and the stream for viewers never hold it.
//!
//! - [`TextFilter`] is the interface of a filter.
//! - [`RuleFilter`] is the filter with fixed rules.
//! - A classifier that asks a model is not in this crate. It implements
//!   [`TextFilter`] and goes into the list of
//!   [`GatewayBuilder::filter`](crate::GatewayBuilder::filter).

mod filter;
mod links;
mod pattern;
mod rules;
mod texts;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use unsup_protocol::{ActionRequest, ResidentId};

pub use self::filter::{Author, FilterFuture, Refusal, Text, TextFilter, TextUse};
pub use self::rules::{LinkRule, ModerationConfig, RuleFilter};
use crate::error::ApiError;
use crate::limit::retry_after_secs;
use crate::state::Gateway;

/// The filters of a gateway and the numbers about them.
pub(crate) struct Moderation {
    pub(crate) config: ModerationConfig,
    filters: Vec<Arc<dyn TextFilter>>,
    /// Texts that the filters looked at.
    checked: AtomicU64,
    /// Texts that a filter refused, for each rule.
    refused: Mutex<BTreeMap<String, u64>>,
    /// Requests with speech that a muted resident sent.
    muted: AtomicU64,
}

impl Moderation {
    pub(crate) fn new(config: ModerationConfig, filters: Vec<Arc<dyn TextFilter>>) -> Moderation {
        Moderation {
            config,
            filters,
            checked: AtomicU64::new(0),
            refused: Mutex::new(BTreeMap::new()),
            muted: AtomicU64::new(0),
        }
    }

    /// Asks every filter about one text. The first refusal counts.
    pub(crate) async fn check(&self, text: &Text<'_>) -> Result<(), Refusal> {
        self.checked.fetch_add(1, Ordering::Relaxed);
        for filter in &self.filters {
            if let Err(refusal) = filter.check(text).await {
                let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
                *refused.entry(refusal.rule.clone()).or_insert(0) += 1;
                return Err(refusal);
            }
        }
        Ok(())
    }

    /// Returns the numbers for the dashboard.
    pub(crate) fn stats(&self) -> Value {
        let refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner).clone();
        json!({
            "checked": self.checked.load(Ordering::Relaxed),
            "refused": refused.values().sum::<u64>(),
            "refused_by_rule": refused,
            "muted_requests": self.muted.load(Ordering::Relaxed),
        })
    }
}

/// Returns true when a request carries speech: text for other residents
/// or for viewers.
pub(crate) fn carries_speech(config: &ModerationConfig, request: &ActionRequest) -> bool {
    texts::collect(config, request).iter().any(|text| text.speech)
}

/// Converts a refusal into the answer for the client. `place` says where
/// the text is.
fn refused(place: &str, refusal: &Refusal) -> ApiError {
    match refusal.retry_after {
        Some(wait) => {
            let mut error = ApiError::rate_limited(retry_after_secs(wait));
            error.message.clone_from(&refusal.message);
            error
        }
        None => ApiError::text_refused(format!("{place}: {}", refusal.message)),
    }
}

impl Gateway {
    /// Checks the text of action requests before they go to the
    /// simulation: the mute of the resident, then every filter for every
    /// text. One refusal stops all requests.
    pub(crate) async fn admit(
        &self,
        resident: ResidentId,
        requests: &[ActionRequest],
    ) -> Result<(), ApiError> {
        let moderation = &self.inner.moderation;
        let found: Vec<(usize, texts::Found<'_>)> = requests
            .iter()
            .enumerate()
            .flat_map(|(index, request)| {
                texts::collect(&moderation.config, request).into_iter().map(move |f| (index, f))
            })
            .collect();
        if found.iter().any(|(_, text)| text.speech) && self.inner.sanctions.is_muted(resident) {
            moderation.muted.fetch_add(1, Ordering::Relaxed);
            return Err(ApiError::muted());
        }
        for (index, entry) in &found {
            let usage = if entry.speech {
                TextUse::Speech { verb: entry.verb }
            } else {
                TextUse::Argument { verb: entry.verb }
            };
            let text = Text { author: Author::Resident(resident), usage, text: entry.text };
            if let Err(refusal) = moderation.check(&text).await {
                let place = if requests.len() > 1 {
                    format!("Action {}: `{}`", index + 1, entry.path)
                } else {
                    format!("`{}`", entry.path)
                };
                return Err(refused(&place, &refusal));
            }
        }
        Ok(())
    }

    /// Checks a name before the server keeps it. `field` names it in the
    /// answer.
    pub(crate) async fn admit_name(
        &self,
        author: Author,
        usage: TextUse<'_>,
        field: &str,
        text: &str,
    ) -> Result<(), ApiError> {
        let text = Text { author, usage, text };
        self.inner
            .moderation
            .check(&text)
            .await
            .map_err(|refusal| refused(&format!("`{field}`"), &refusal))
    }
}
