//! One virtual agent: it signs up, then observes, thinks and acts until
//! the run ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use unsup_kernel::Rng;
use unsup_protocol::{ActionAck, ActionRequest, ActionStatus, Observation, Pos, ResidentId};

use super::error::RequestError;
use super::http::{self, HttpLink, Urls};
use super::metrics::{Link, Metrics};
use super::signup::{self, Credential, SignUp};
use super::wire::{CityBounds, has_report, update_scene};
use super::ws::WsLink;
use crate::profile::Resolved;
use crate::random;
use crate::scene::ListScene;
use crate::think::ThinkTime;

/// First stream of the generators of the agents. Agent `n` uses stream
/// `AGENT_STREAM + n`.
const AGENT_STREAM: u64 = 0xa9e_0000_0000;

/// The times that rule the requests of an agent.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Time after which a request counts as failed.
    pub request_timeout: Duration,
    /// Pause after a failed request.
    pub retry: Duration,
    /// Failed requests in a row after which the agent gives up.
    pub max_failures: u32,
    /// Greatest time of one wait for the end of an action.
    pub wait_timeout: Duration,
    /// Time between two observations when the server has no long poll.
    pub poll_interval: Duration,
}

/// What all agents of a swarm share.
pub struct Shared {
    /// The behaviour, matched with the catalogue of the server.
    pub profile: Resolved,
    /// The tiles of each city.
    pub cities: CityBounds,
    /// Think time between two requests.
    pub think: ThinkTime,
    /// The addresses of the server.
    pub urls: Arc<Urls>,
    /// The numbers of the run.
    pub metrics: Metrics,
    /// The sign-up flow.
    pub signup: SignUp,
    /// Times and limits of requests.
    pub timing: Timing,
    /// Seed of the choices of the agents.
    pub seed: u64,
    /// When the agents stop.
    pub end: Instant,
    /// One client for all HTTP agents, or `None` when each agent has its
    /// own client and connection.
    pub shared_client: Option<reqwest::Client>,
    /// Set when the server has no WebSocket route.
    pub ws_absent: AtomicBool,
    /// Set when the server has no long poll route.
    pub wait_absent: AtomicBool,
}

/// What one agent does in a run.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Number of the agent, from 0.
    pub index: u32,
    /// When the agent starts.
    pub start: Instant,
    /// How the agent talks to the server.
    pub link: Link,
    /// The key from an earlier run, if there is one.
    pub credential: Option<Credential>,
}

/// How an agent ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// `None` when the agent ran until the end of the run. Otherwise the
    /// code of the error that made it give up.
    pub failed: Option<String>,
    /// The credential of the agent, for the credentials file.
    pub credential: Option<Credential>,
}

/// The connection of an agent.
enum Transport {
    Http(HttpLink),
    Ws(Box<WsLink>),
}

struct Agent {
    index: usize,
    shared: Arc<Shared>,
    rng: Rng,
    client: reqwest::Client,
    owns_client: bool,
    credential: Credential,
    /// The link that the plan asked for. The transport can differ: a
    /// WebSocket agent uses HTTP when the server has no WebSocket route.
    planned: Link,
    transport: Transport,
    scene: ListScene,
    /// False when the position in the scene may be old.
    scene_fresh: bool,
    /// The notice cursor: `next_notice` of the last observation.
    cursor: u64,
    sent: u64,
    failures: u32,
}

/// Runs one agent until the run ends or the agent gives up.
pub async fn run(plan: Plan, shared: Arc<Shared>) -> Outcome {
    tokio::time::sleep_until(plan.start.into()).await;
    shared.metrics.agents_started.add();
    let mut credential = plan.credential.clone();
    let failed = match Agent::start(&plan, &shared, &mut credential).await {
        Ok(mut agent) => {
            shared.metrics.agents_ready.add();
            let result = agent.live().await;
            credential = Some(agent.credential.clone());
            agent.close().await;
            result.err()
        }
        Err(error) => Some(error),
    };
    if let Some(error) = &failed {
        shared.metrics.agents_failed.add(&error.code);
    }
    Outcome { failed: failed.map(|error| error.code), credential }
}

fn takes_time(ack: &ActionAck) -> bool {
    matches!(
        ack.status,
        ActionStatus::Started { .. } | ActionStatus::Pending | ActionStatus::Queued { .. }
    )
}

impl Agent {
    /// Signs up (or takes the stored key), connects and makes the first
    /// observation. A failed step is tried again a few times.
    async fn start(
        plan: &Plan,
        shared: &Arc<Shared>,
        stored: &mut Option<Credential>,
    ) -> Result<Agent, RequestError> {
        let (client, owns_client) = match &shared.shared_client {
            Some(client) => (client.clone(), false),
            None => (http::client(shared.timing.request_timeout, true)?, true),
        };
        let mut failures = 0;
        loop {
            match Agent::try_start(plan, shared, stored, &client, owns_client).await {
                Ok(agent) => return Ok(agent),
                Err(error) => {
                    shared.metrics.error(&error);
                    if error.credentials_refused() && stored.take().is_some() {
                        // The stored key is from another world. Sign up.
                        shared.metrics.credentials_renewed.add();
                    }
                    failures += 1;
                    let pause = Instant::now() + shared.timing.retry;
                    if failures >= shared.timing.max_failures || pause >= shared.end {
                        return Err(error);
                    }
                    tokio::time::sleep_until(pause.into()).await;
                }
            }
        }
    }

    async fn try_start(
        plan: &Plan,
        shared: &Arc<Shared>,
        stored: &mut Option<Credential>,
        client: &reqwest::Client,
        owns_client: bool,
    ) -> Result<Agent, RequestError> {
        let credential = match stored {
            Some(credential) => credential.clone(),
            None => {
                let name = format!("swarm-{}-{}", shared.seed, plan.index);
                let (credential, took) =
                    signup::timed(&shared.signup, client, &shared.urls, &name).await?;
                shared.metrics.signup.record(plan.index as usize, took);
                *stored = Some(credential.clone());
                credential
            }
        };
        let transport = Agent::connect(plan.link, shared, client, &credential).await?;
        let mut agent = Agent {
            index: plan.index as usize,
            shared: Arc::clone(shared),
            rng: Rng::stream(shared.seed, AGENT_STREAM + u64::from(plan.index)),
            client: client.clone(),
            owns_client,
            scene: ListScene::new(ResidentId(credential.resident), Pos::default(), None),
            credential,
            planned: plan.link,
            transport,
            scene_fresh: false,
            cursor: 0,
            sent: 0,
            failures: 0,
        };
        agent.count(true);
        if plan.link == Link::Ws && matches!(agent.transport, Transport::Http(_)) {
            shared.metrics.ws_fallbacks.add();
        }
        match agent.observe().await {
            Ok(()) => Ok(agent),
            Err(error) => {
                agent.close().await;
                Err(error)
            }
        }
    }

    /// Adds the transport of the agent to the gauges of open connections,
    /// or takes it away.
    fn count(&self, open: bool) {
        let metrics = &self.shared.metrics;
        let gauge = match self.transport {
            Transport::Ws(_) => &metrics.ws_open,
            Transport::Http(_) if self.owns_client => &metrics.http_agents,
            Transport::Http(_) => return,
        };
        if open { gauge.up() } else { gauge.down() }
    }

    /// Replaces the transport and keeps the gauges right. The old
    /// transport is dropped, which closes its socket.
    fn install(&mut self, transport: Transport) {
        self.count(false);
        self.transport = transport;
        self.count(true);
    }

    fn http_link(&self) -> HttpLink {
        HttpLink::new(
            self.client.clone(),
            Arc::clone(&self.shared.urls),
            self.credential.key.clone(),
        )
    }

    /// Opens the connection of the agent. A WebSocket agent uses HTTP when
    /// the server has no WebSocket route.
    async fn connect(
        link: Link,
        shared: &Shared,
        client: &reqwest::Client,
        credential: &Credential,
    ) -> Result<Transport, RequestError> {
        let http = || {
            Transport::Http(HttpLink::new(
                client.clone(),
                Arc::clone(&shared.urls),
                credential.key.clone(),
            ))
        };
        if link == Link::Http {
            return Ok(http());
        }
        if !shared.ws_absent.load(Ordering::Relaxed) {
            let timeout = shared.timing.request_timeout;
            match WsLink::connect(&shared.urls.ws, &credential.key, timeout).await {
                Ok(socket) => return Ok(Transport::Ws(Box::new(socket))),
                Err(error) if error.route_absent() => {
                    if !shared.ws_absent.swap(true, Ordering::Relaxed) {
                        eprintln!(
                            "swarm: warning: the server has no WebSocket route (GET /v1/ws); \
                             the WebSocket agents use HTTP requests"
                        );
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(http())
    }

    fn link(&self) -> Link {
        match self.transport {
            Transport::Http(_) => Link::Http,
            Transport::Ws(_) => Link::Ws,
        }
    }

    fn see(&mut self, observation: &Observation) {
        self.scene_fresh = update_scene(&mut self.scene, observation, &self.shared.cities);
        self.cursor = observation.next_notice;
    }

    /// Asks for an observation and updates the scene.
    async fn observe(&mut self) -> Result<(), RequestError> {
        let (observation, latency) = match &mut self.transport {
            Transport::Http(link) => link.observe(self.cursor).await?,
            Transport::Ws(link) => link.observe(self.cursor).await?,
        };
        self.shared.metrics.observation(self.index, latency);
        self.see(&observation);
        Ok(())
    }

    /// Sends one action and counts its acknowledgement.
    async fn act(&mut self, request: &ActionRequest) -> Result<ActionAck, RequestError> {
        let link = self.link();
        let (ack, latency) = match &mut self.transport {
            Transport::Http(http) => {
                let (answer, latency) = http.act(request).await?;
                let ack = answer.acks.into_iter().next().ok_or_else(|| {
                    RequestError::local("decode", "the answer has no acknowledgement")
                })?;
                match &answer.observation {
                    Some(observation) => self.see(observation),
                    None => self.scene_fresh = false,
                }
                (ack, latency)
            }
            Transport::Ws(ws) => {
                let answer = ws.act(request).await?;
                // An acknowledgement on the socket carries no observation.
                self.scene_fresh = false;
                answer
            }
        };
        self.shared.metrics.ack(self.index, link, &ack, latency);
        Ok(ack)
    }

    /// Passes time until `deadline`. A WebSocket agent reads what the
    /// server pushes meanwhile.
    async fn idle(&mut self, deadline: Instant) -> Result<(), RequestError> {
        match &mut self.transport {
            Transport::Http(_) => {
                tokio::time::sleep_until(deadline.into()).await;
                Ok(())
            }
            Transport::Ws(link) => {
                let result = link.idle_until(deadline).await;
                for _ in 0..link.take_pushed() {
                    self.shared.metrics.pushed.add();
                }
                result
            }
        }
    }

    /// Waits for the end of the running action: the long poll over HTTP, a
    /// pushed report on the WebSocket, or repeated observations when the
    /// server has no long poll.
    async fn wait_for_report(&mut self) -> Result<(), RequestError> {
        let timing = self.shared.timing;
        let deadline = (Instant::now() + timing.wait_timeout).min(self.shared.end);
        if self.shared.metrics.is_steady() {
            self.shared.metrics.waits_steady.add();
        }
        if let Transport::Ws(link) = &mut self.transport {
            let result = link.wait_report(deadline).await;
            for _ in 0..link.take_pushed() {
                self.shared.metrics.pushed.add();
            }
            self.scene_fresh = false;
            return result.map(|_| ());
        }
        if !self.shared.wait_absent.load(Ordering::Relaxed)
            && let Transport::Http(link) = &self.transport
        {
            let left = deadline.saturating_duration_since(Instant::now());
            match link.wait(self.cursor, left, timing.request_timeout).await {
                Ok(observation) => {
                    self.shared.metrics.long_polls.add();
                    self.see(&observation);
                    return Ok(());
                }
                Err(error) if error.route_absent() => {
                    if !self.shared.wait_absent.swap(true, Ordering::Relaxed) {
                        eprintln!(
                            "swarm: warning: the server has no long poll (GET /v1/wait); \
                             agents that wait for the end of an action observe again and again"
                        );
                    }
                }
                Err(error) => return Err(error),
            }
        }
        // No long poll: look again and again until the report is there.
        loop {
            let next = Instant::now() + timing.poll_interval;
            if next >= deadline {
                return Ok(());
            }
            tokio::time::sleep_until(next.into()).await;
            let before = self.cursor;
            let Transport::Http(link) = &self.transport else { return Ok(()) };
            let (observation, latency) = link.observe(before).await?;
            self.shared.metrics.observation(self.index, latency);
            self.see(&observation);
            if has_report(&observation) {
                return Ok(());
            }
        }
    }

    /// One round: think, look when needed, act, and sometimes wait for the
    /// end of the action.
    async fn round(&mut self) -> Result<(), RequestError> {
        let think = self.shared.think.sample(&mut self.rng);
        let wake =
            Instant::now().checked_add(think).map_or(self.shared.end, |at| at.min(self.shared.end));
        self.idle(wake).await?;
        if Instant::now() >= self.shared.end {
            return Ok(());
        }
        let profile = &self.shared.profile;
        if !self.scene_fresh || random::chance(&mut self.rng, profile.observe_share) {
            self.observe().await?;
        }
        let shared = Arc::clone(&self.shared);
        let Some(choice) = shared.profile.pick(&mut self.rng, &mut self.scene) else {
            // Nothing is possible here and now. Think again.
            return Ok(());
        };
        self.sent += 1;
        let request = ActionRequest {
            id: Some(format!("lt{}-{}", self.index, self.sent)),
            verb: choice.verb,
            args: choice.args,
        };
        let ack = self.act(&request).await?;
        if takes_time(&ack) && random::chance(&mut self.rng, shared.profile.wait_share) {
            self.wait_for_report().await?;
        }
        Ok(())
    }

    /// Lives until the run ends. Returns the error that made the agent
    /// give up, if it did.
    async fn live(&mut self) -> Result<(), RequestError> {
        while Instant::now() < self.shared.end {
            let Err(error) = self.round().await else {
                self.failures = 0;
                continue;
            };
            self.fail(error.clone())?;
            if let Err(again) = self.recover(&error).await {
                self.fail(again)?;
            }
        }
        Ok(())
    }

    /// Counts one failed request. Gives the error back when the agent has
    /// failed too often in a row.
    fn fail(&mut self, error: RequestError) -> Result<(), RequestError> {
        self.shared.metrics.error(&error);
        self.failures += 1;
        if self.failures >= self.shared.timing.max_failures { Err(error) } else { Ok(()) }
    }

    /// Pauses after a failed request and repairs what the failure broke: a
    /// refused key is replaced, and a WebSocket is opened again, because
    /// a socket is not usable after an error or a missed answer.
    async fn recover(&mut self, error: &RequestError) -> Result<(), RequestError> {
        let pause = (Instant::now() + self.shared.timing.retry).min(self.shared.end);
        tokio::time::sleep_until(pause.into()).await;
        if Instant::now() >= self.shared.end {
            return Ok(());
        }
        let shared = Arc::clone(&self.shared);
        self.scene_fresh = false;
        if matches!(self.transport, Transport::Ws(_)) {
            // Close the socket now. Until a new one is open, the agent has
            // a working HTTP link.
            let http = Transport::Http(self.http_link());
            self.install(http);
        }
        if error.credentials_refused() {
            let name = format!("swarm-{}-{}-again", shared.seed, self.index);
            self.credential = shared.signup.sign_up(&self.client, &shared.urls, &name).await?;
            shared.metrics.credentials_renewed.add();
            self.scene.me = ResidentId(self.credential.resident);
            self.cursor = 0;
        }
        let transport =
            Agent::connect(self.planned, &shared, &self.client, &self.credential).await?;
        self.install(transport);
        Ok(())
    }

    async fn close(self) {
        self.count(false);
        if let Transport::Ws(link) = self.transport {
            link.close().await;
        }
    }
}
