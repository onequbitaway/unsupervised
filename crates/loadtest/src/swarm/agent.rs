//! One virtual agent: it signs up, then observes, thinks and acts until
//! the run ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use unsup_kernel::Rng;
use unsup_protocol::{ActionAck, ActionRequest, ActionStatus, Observation, Pos, ResidentId};

use super::error::RequestError;
use super::http::{self, HttpLink, Urls};
use super::metrics::{Link, Metrics};
use super::preflight::HttpConnections;
use super::signup::{self, Credential};
use super::wire::{Ended, update_scene};
use super::ws::WsLink;
use crate::map::WorldMap;
use crate::profile::{Memory, Resolved};
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
    /// Pause after a failed request. It is also the pause after a request
    /// that a limit of the server refused, when the answer names none.
    pub retry: Duration,
    /// Failed requests in a row after which the agent gives up.
    pub max_failures: u32,
    /// Greatest time of one wait for the end of an action.
    pub wait_timeout: Duration,
}

/// What all agents of a swarm share.
pub struct Shared {
    /// The behaviour, matched with the catalogue of the server.
    pub profile: Resolved,
    /// The tiles and the sites of the world.
    pub map: Arc<WorldMap>,
    /// Think time between two requests.
    pub think: ThinkTime,
    /// The addresses of the server.
    pub urls: Arc<Urls>,
    /// The numbers of the run.
    pub metrics: Metrics,
    /// Times and limits of requests.
    pub timing: Timing,
    /// Seed of the choices of the agents.
    pub seed: u64,
    /// When the agents stop.
    pub end: Instant,
    /// One pool of connections for the agents that hold no HTTP connection
    /// of their own.
    pub pool: reqwest::Client,
    /// How the HTTP agents hold connections.
    pub http_connections: HttpConnections,
}

/// What one agent does in a run.
#[derive(Clone, Debug)]
pub struct Seat {
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
    /// The link that the seat asked for.
    planned: Link,
    transport: Transport,
    scene: ListScene,
    /// False when the position in the scene may be old.
    scene_fresh: bool,
    /// The notice cursor: `next_notice` of the last observation.
    cursor: u64,
    /// What the agent remembers about its own actions.
    memory: Memory,
    sent: u64,
    failures: u32,
}

/// Runs one agent until the run ends or the agent gives up.
pub async fn run(seat: Seat, shared: Arc<Shared>) -> Outcome {
    tokio::time::sleep_until(seat.start.into()).await;
    shared.metrics.agents_started.add();
    let mut credential = seat.credential.clone();
    let failed = match Agent::start(&seat, &shared, &mut credential).await {
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

/// Returns the time that an agent waits after the server refused a request
/// by one of its limits: the time that the answer names, else `retry`.
fn limit_pause(error: &RequestError, retry: Duration) -> Duration {
    error.retry_after_secs.map_or(retry, Duration::from_secs).max(retry)
}

impl Agent {
    /// Signs up (or takes the stored key), connects and makes the first
    /// observation. A failed step is tried again a few times. A step that
    /// a limit of the server refused is tried again after the time that
    /// the server names, until the run ends.
    async fn start(
        seat: &Seat,
        shared: &Arc<Shared>,
        stored: &mut Option<Credential>,
    ) -> Result<Agent, RequestError> {
        // An HTTP agent keeps one connection of its own open, like a real
        // agent. A WebSocket agent holds its socket; it signs up through
        // the pool, so that it does not keep a second connection.
        let own = seat.link == Link::Http && shared.http_connections == HttpConnections::PerAgent;
        let client = match own {
            true => http::client(shared.timing.request_timeout, true)?,
            false => shared.pool.clone(),
        };
        let owns_client = own;
        let mut failures = 0;
        loop {
            let error = match Agent::try_start(seat, shared, stored, &client, owns_client).await {
                Ok(agent) => return Ok(agent),
                Err(error) => error,
            };
            shared.metrics.error(&error);
            if error.credentials_refused() && stored.take().is_some() {
                // The stored key is from another world. Sign up.
                shared.metrics.credentials_renewed.add();
            }
            let pause = if error.throttled() {
                limit_pause(&error, shared.timing.retry)
            } else {
                failures += 1;
                shared.timing.retry
            };
            let again = Instant::now() + pause;
            if failures >= shared.timing.max_failures || again >= shared.end {
                return Err(error);
            }
            tokio::time::sleep_until(again.into()).await;
        }
    }

    async fn try_start(
        seat: &Seat,
        shared: &Arc<Shared>,
        stored: &mut Option<Credential>,
        client: &reqwest::Client,
        owns_client: bool,
    ) -> Result<Agent, RequestError> {
        let credential = match stored {
            Some(credential) => credential.clone(),
            None => {
                let name = format!("swarm-{}-{}", shared.seed, seat.index);
                let (credential, took) = signup::timed(client, &shared.urls, &name).await?;
                shared.metrics.signup.record(seat.index as usize, took);
                *stored = Some(credential.clone());
                credential
            }
        };
        let transport = Agent::connect(seat.link, shared, client, &credential).await?;
        let mut scene = ListScene::new(ResidentId(credential.resident), Pos::default(), None);
        scene.map = Some(Arc::clone(&shared.map));
        let mut agent = Agent {
            index: seat.index as usize,
            shared: Arc::clone(shared),
            rng: Rng::stream(shared.seed, AGENT_STREAM + u64::from(seat.index)),
            client: client.clone(),
            owns_client,
            scene,
            credential,
            planned: seat.link,
            transport,
            scene_fresh: false,
            cursor: 0,
            memory: Memory::default(),
            sent: 0,
            failures: 0,
        };
        agent.count(true);
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

    /// Opens the connection of the agent: the WebSocket of its resident,
    /// or an HTTP link that sends its key with each request.
    async fn connect(
        link: Link,
        shared: &Shared,
        client: &reqwest::Client,
        credential: &Credential,
    ) -> Result<Transport, RequestError> {
        match link {
            Link::Http => Ok(Transport::Http(HttpLink::new(
                client.clone(),
                Arc::clone(&shared.urls),
                credential.key.clone(),
            ))),
            Link::Ws => {
                let timeout = shared.timing.request_timeout;
                let socket = WsLink::connect(&shared.urls.ws, &credential.key, timeout).await?;
                Ok(Transport::Ws(Box::new(socket)))
            }
        }
    }

    fn link(&self) -> Link {
        match self.transport {
            Transport::Http(_) => Link::Http,
            Transport::Ws(_) => Link::Ws,
        }
    }

    /// Takes an observation: the scene, the notice cursor, and over HTTP
    /// the reports of ended actions. A WebSocket agent gets each report as
    /// a pushed frame, so it does not count the reports of an observation.
    fn see(&mut self, observation: &Observation) {
        self.scene_fresh = update_scene(&mut self.scene, observation, self.shared.profile.items());
        self.cursor = observation.next_notice;
        if matches!(self.transport, Transport::Http(_)) {
            self.shared.metrics.ended(&Ended::list(observation));
        }
    }

    /// Counts what the server pushed on the WebSocket since the last call.
    fn count_pushed(&mut self) {
        if let Transport::Ws(link) = &mut self.transport {
            for _ in 0..link.take_pushed() {
                self.shared.metrics.pushed.add();
            }
            self.shared.metrics.ended(&link.take_ended());
        }
    }

    /// Asks for an observation and updates the scene.
    async fn observe(&mut self) -> Result<(), RequestError> {
        let answer = match &mut self.transport {
            Transport::Http(link) => link.observe(self.cursor).await,
            Transport::Ws(link) => link.observe(self.cursor).await,
        };
        self.count_pushed();
        let (observation, latency) = answer?;
        self.shared.metrics.observation(self.index, latency);
        self.see(&observation);
        Ok(())
    }

    /// Sends one action, or a plan of a walk and an action, and counts the
    /// acknowledgements. Returns the acknowledgements.
    async fn act(&mut self, requests: &[ActionRequest]) -> Result<Vec<ActionAck>, RequestError> {
        let link = self.link();
        let (acks, latency) = match &mut self.transport {
            Transport::Http(http) => {
                let (answer, latency) = http.act(requests, self.cursor).await?;
                self.see(&answer.observation);
                (answer.acks, latency)
            }
            Transport::Ws(ws) => {
                let answer = ws.act(requests).await;
                // An acknowledgement on the socket carries no observation.
                self.scene_fresh = false;
                self.count_pushed();
                answer?
            }
        };
        if acks.is_empty() {
            return Err(RequestError::local("decode", "the answer has no acknowledgement"));
        }
        self.shared.metrics.acks(self.index, link, &acks, requests.len(), latency);
        Ok(acks)
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
                self.count_pushed();
                result
            }
        }
    }

    /// Waits for the end of the running action: the long poll over HTTP,
    /// or a pushed report on the WebSocket.
    async fn wait_for_report(&mut self) -> Result<(), RequestError> {
        let timing = self.shared.timing;
        let deadline = (Instant::now() + timing.wait_timeout).min(self.shared.end);
        if self.shared.metrics.is_steady() {
            self.shared.metrics.waits_steady.add();
        }
        match &mut self.transport {
            Transport::Ws(link) => {
                let result = link.wait_report(deadline).await;
                self.count_pushed();
                self.scene_fresh = false;
                result.map(|_| ())
            }
            Transport::Http(link) => {
                let left = deadline.saturating_duration_since(Instant::now());
                let observation = link.wait(self.cursor, left, timing.request_timeout).await?;
                self.shared.metrics.long_polls.add();
                self.see(&observation);
                Ok(())
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
        let shared = Arc::clone(&self.shared);
        if !self.scene_fresh || random::chance(&mut self.rng, shared.profile.observe_share) {
            self.observe().await?;
        }
        let Some(mut choice) = shared.profile.pick(&mut self.rng, &mut self.scene, self.memory)
        else {
            // Nothing is possible here and now. Think again.
            return Ok(());
        };
        // An action at a site is a plan: the walk, then the action.
        let mut requests: Vec<ActionRequest> = choice.walk().into_iter().collect();
        requests.push(choice.take_request());
        for request in &mut requests {
            self.sent += 1;
            request.id = Some(format!("lt{}-{}", self.index, self.sent));
        }
        let acks = self.act(&requests).await?;
        let Some(last) = acks.last() else { return Ok(()) };
        if acks.len() == requests.len() && last.status.is_accepted() {
            choice.remember(&mut self.memory);
        }
        if takes_time(last) && random::chance(&mut self.rng, shared.profile.wait_share) {
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
            if error.throttled() {
                // A limit of the server refused the request. The agent
                // waits as long as the server asks and goes on.
                self.shared.metrics.error(&error);
                let pause = limit_pause(&error, self.shared.timing.retry);
                let until = Instant::now().checked_add(pause).unwrap_or(self.shared.end);
                self.idle(until.min(self.shared.end)).await?;
                continue;
            }
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
            self.credential = signup::sign_up(&self.client, &shared.urls, &name).await?;
            shared.metrics.credentials_renewed.add();
            self.scene.me = ResidentId(self.credential.resident);
            self.cursor = 0;
            self.memory = Memory::default();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_after_a_limit_is_the_time_that_the_server_names() {
        let retry = Duration::from_millis(500);
        let named = RequestError::from_status(429, b"", Some(3));
        assert_eq!(limit_pause(&named, retry), Duration::from_secs(3));
        // Without a time, and with a time of zero, the agent waits as
        // after a failed request: it must not send again at once.
        let silent = RequestError::from_status(429, b"", None);
        assert_eq!(limit_pause(&silent, retry), retry);
        let zero = RequestError::from_status(429, b"", Some(0));
        assert_eq!(limit_pause(&zero, retry), retry);
    }
}
