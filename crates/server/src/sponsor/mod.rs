//! Missions of human sponsors: the route `POST /v1/missions`.
//!
//! A sponsor is an owner, not a resident. The owner sends the definition
//! of a mission, and the missions module publishes it with new money as
//! the reward. The gateway knows no game module, so this route is code of
//! the server: the gateway authenticates the owner and applies its limits
//! (`unsup_gateway::OwnerRoute`), and the [`Desk`] does the rest:
//!
//! 1. The switch of the administrator (`sponsors.enabled`).
//! 2. The shape and the sizes of the definition, and the limit of new
//!    money for one mission.
//! 3. The text filters, for every text of the definition and for the name
//!    of the owner. That name is the name of the sponsor: the client
//!    cannot choose another one.
//! 4. The limit of missions for one owner in 24 hours.
//! 5. The message `unsup_missions::SponsorMission` for the global realm,
//!    and the wait for the public event that answers it.
//!
//! The module checks the definition again and enforces the limit of new
//! money for all sponsors in one game day.

mod answer;
mod config;
mod quota;
mod sizes;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::Context;
use axum::http::StatusCode;
use serde_json::{Value, json};
use unsup_gateway::{
    ApiError, Author, Gateway, OwnerCall, OwnerRoute, OwnerRouteFuture, TextUse, retry_after_secs,
};
use unsup_missions::{MissionSpec, SponsorMission};
use unsup_protocol::ErrorCode;
use unsup_runtime::{PostError, SimHandle};
use unsup_store::{Owner, random_hex};

use self::answer::Verdict;
pub use self::config::SponsorConfig;
use self::quota::Quota;
use self::sizes::Sizes;

/// The path of the route.
pub(crate) const PATH: &str = "/v1/missions";

/// The verb with which a resident publishes a mission. The text filters
/// treat the text of a sponsor as the arguments of this verb.
const VERB: &str = "mission_post";

/// The request numbers stay below 2^53, so that every JSON reader keeps
/// them exact.
const REQUEST_MASK: u64 = (1 << 53) - 1;
/// Random bytes in the first request number.
const REQUEST_SEED_BYTES: usize = 6;

/// Takes the missions of human sponsors and sends them to the world.
pub(crate) struct Desk {
    sim: SimHandle,
    config: SponsorConfig,
    /// The sizes that the missions module permits. `None` when the
    /// sponsor missions are turned off.
    sizes: Option<Sizes>,
    quota: Quota,
    /// The number of the next request. The answer of the world carries
    /// the number.
    next_request: AtomicU64,
}

/// The answer for a server whose sponsor missions are turned off.
fn disabled() -> ApiError {
    ApiError::forbidden(
        "missions_disabled",
        "The administrator of this server turned the sponsor missions off.",
    )
}

/// Converts the refusal of the missions module into an answer. The code
/// and the message are those of the module.
fn refused(code: &str, message: String) -> ApiError {
    let code = serde_json::from_value::<ErrorCode>(json!(code)).ok();
    let status = match code {
        Some(ErrorCode::InvalidArgs) => StatusCode::BAD_REQUEST,
        Some(ErrorCode::RateLimited) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::CONFLICT,
    };
    ApiError::new(status, code.map_or("refused", ErrorCode::as_str), message)
}

impl Desk {
    /// Makes the desk for a world.
    ///
    /// # Errors
    ///
    /// Fails when the sponsor missions are on and the world has no
    /// missions module, and when the system gives no random bytes.
    pub(crate) fn new(sim: SimHandle, config: SponsorConfig) -> anyhow::Result<Desk> {
        let sizes = if config.enabled {
            let sizes = Sizes::read(&sim.catalogue()).map_err(|name| {
                anyhow::anyhow!(
                    "the world has no parameter `{name}`: sponsor missions need the missions \
                     module. Set `sponsors.enabled = false` to start without them"
                )
            })?;
            Some(sizes)
        } else {
            None
        };
        // A number of an earlier run of the server cannot come again by
        // chance: the first number is random.
        let seed = random_hex(REQUEST_SEED_BYTES).context("cannot make a request number")?;
        let first = u64::from_str_radix(&seed, 16).context("cannot read the request number")?;
        Ok(Desk {
            sim,
            quota: Quota::new(config.missions_per_day),
            config,
            sizes,
            next_request: AtomicU64::new(first),
        })
    }

    /// Checks the sum of new money that the mission asks for.
    fn check_money(&self, spec: &MissionSpec) -> Result<(), ApiError> {
        let limit = self.config.max_mission_cents;
        if limit == 0 {
            return Ok(());
        }
        // A reward below zero is no money. The module refuses it.
        let reward = u64::try_from(spec.reward).unwrap_or(0);
        match reward.checked_mul(u64::from(spec.takers)) {
            Some(total) if total <= limit => Ok(()),
            _ => Err(ApiError::invalid(format!(
                "`reward` times `takers` is more than {limit} cents. That is the greatest sum \
                 for one sponsor mission on this server."
            ))),
        }
    }

    /// Sends the mission to the world and waits for its answer. Returns
    /// the number of the request and the answer.
    async fn send(&self, owner: &Owner, spec: MissionSpec) -> Result<(u64, Verdict), ApiError> {
        let request = self.next_request.fetch_add(1, Ordering::Relaxed) & REQUEST_MASK;
        // The answer is an event of the next frame. The subscription is
        // made first, so that the event cannot pass before it.
        let mut frames = self.sim.frames();
        let message = SponsorMission { request, sponsor: owner.name.clone(), mission: spec };
        self.sim.post_global(&message).await.map_err(|error| match error {
            PostError::Sim(error) => ApiError::from(error),
            other => {
                tracing::error!(error = %other, "a sponsor mission could not be sent");
                ApiError::internal()
            }
        })?;
        let verdict = answer::wait(&mut frames, request, self.config.answer_timeout()).await;
        match &verdict {
            Verdict::Posted { frame, data } => {
                tracing::info!(owner = %owner.id, request, frame, mission = %data["mission"], "sponsor mission published");
            }
            Verdict::Refused { code, .. } => {
                tracing::info!(owner = %owner.id, request, code, "sponsor mission refused by the world");
            }
            Verdict::Unknown => {
                tracing::warn!(owner = %owner.id, request, "no answer to a sponsor mission in time");
            }
            Verdict::Stopped => {}
        }
        Ok((request, verdict))
    }

    /// Answers one request of an owner.
    async fn publish(
        &self,
        gateway: &Gateway,
        call: OwnerCall,
    ) -> Result<(StatusCode, Value), ApiError> {
        let Some(sizes) = &self.sizes else {
            return Err(disabled());
        };
        let OwnerCall { owner, body } = call;
        // A list would read as a definition too, field by field.
        if !body.is_object() {
            return Err(ApiError::invalid(
                "The body must be an object: the definition of a mission.",
            ));
        }
        let spec: MissionSpec = serde_json::from_value(body.clone())
            .map_err(|error| ApiError::invalid(format!("The body is not a mission: {error}.")))?;
        sizes.check(&spec, &owner.name).map_err(ApiError::invalid)?;
        self.check_money(&spec)?;

        // Viewers and residents read the name of the sponsor and every
        // text of the mission.
        let author = Author::Owner(owner.id);
        gateway
            .check_text(author, TextUse::OwnerName, "The name of the owner", &owner.name)
            .await?;
        gateway.check_texts(author, VERB, &body).await?;

        let slot = self.quota.reserve(owner.id, Instant::now()).map_err(|wait| {
            let seconds = retry_after_secs(wait);
            let mut error = ApiError::rate_limited(seconds);
            error.message = format!(
                "An owner can publish {} sponsor missions in 24 hours. Try again in {seconds} \
                 seconds.",
                self.quota.per_day()
            );
            error
        })?;
        let (request, verdict) = match self.send(&owner, spec).await {
            Ok(answer) => answer,
            Err(error) => {
                // The world did not take the mission.
                self.quota.release(slot);
                return Err(error);
            }
        };
        match verdict {
            Verdict::Posted { frame, data } => Ok((
                StatusCode::CREATED,
                json!({
                    "mission": data["mission"],
                    "request": data["request"],
                    "frame": frame,
                    "deadline": data["deadline"],
                    "reward": data["reward"],
                    "takers": data["takers"],
                }),
            )),
            Verdict::Refused { code, message } => {
                self.quota.release(slot);
                Err(refused(&code, message))
            }
            // The mission is in the world. Its place in the limit stays.
            Verdict::Unknown => Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "answer_timeout",
                format!(
                    "The simulation took the mission and did not answer in time. Do not send \
                     it again at once: look in `/v1/feed` for an event `missions.mission_posted` \
                     or `missions.sponsor_refused` with the request {request}."
                ),
            )),
            Verdict::Stopped => Err(ApiError::shutdown()),
        }
    }
}

impl OwnerRoute for Desk {
    fn call<'a>(&'a self, gateway: &'a Gateway, call: OwnerCall) -> OwnerRouteFuture<'a> {
        Box::pin(self.publish(gateway, call))
    }
}
