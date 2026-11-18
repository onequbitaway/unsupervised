//! An agent over HTTPS: the routes of a resident key.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::link::{Inbox, Link};
use super::{World, id, send};

/// A resident that a test controls over HTTPS.
///
/// The agent keeps the notice cursor like a real client: it sends
/// `next_notice` of the last observation as `since`, so each answer has
/// only the news that is new.
pub struct Agent {
    /// The id of the resident.
    pub id: u64,
    /// The key of the resident.
    pub key: String,
    /// `next_notice` of the last observation that the agent read.
    pub cursor: u64,
    /// The news that arrived so far.
    pub inbox: Inbox,
    base: String,
    http: reqwest::Client,
}

impl Agent {
    /// Makes an agent for a resident that exists.
    pub fn new(world: &World, id: u64, key: &str) -> Agent {
        Agent {
            id,
            key: key.to_string(),
            cursor: 0,
            inbox: Inbox::default(),
            base: world.base.clone(),
            http: world.http.clone(),
        }
    }

    /// Signs up an owner, makes a resident and returns an agent for it.
    pub async fn join(world: &World, name: &str) -> Agent {
        let (id, key) = world.newcomer(name).await;
        Agent::new(world, id, &key)
    }

    /// Sends a request with the key of the resident. Returns the status
    /// and the body.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> (StatusCode, Value) {
        let request =
            self.http.request(method, format!("{}{path}", self.base)).bearer_auth(&self.key);
        send(match body {
            Some(body) => request.json(body),
            None => request,
        })
        .await
    }

    /// Sends the body of `POST /v1/act` with the cursor of the agent.
    /// Reads the news of the answer. Returns the acknowledgements.
    ///
    /// # Panics
    ///
    /// Panics when the server does not answer 200.
    pub async fn send_act(&mut self, body: &Value) -> Vec<Value> {
        let path = format!("/v1/act?since={}&sections=self,reports", self.cursor);
        let (status, answer) = self.call(Method::POST, &path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "POST /v1/act {body}: {answer}");
        self.read(&answer["observation"]);
        answer["acks"].as_array().expect("`acks` is a list").clone()
    }

    /// Takes the news of an observation and moves the cursor.
    fn read(&mut self, observation: &Value) {
        assert_eq!(observation["resident"], self.id, "the observation is that of the resident");
        self.inbox.observation(observation);
        self.cursor = id(&observation["next_notice"], "next_notice");
    }

    /// Calls the long poll with the cursor of the agent and reads the
    /// news of the answer. Returns the observation. The caller runs the
    /// world.
    pub async fn wait(&mut self, query: &str) -> Value {
        let path = format!("/v1/wait?since={}{query}", self.cursor);
        let (status, observation) = self.call(Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {observation}");
        self.read(&observation);
        observation
    }
}

impl Link for Agent {
    async fn act(&mut self, verb: &str, args: Value) -> Value {
        let mut acks = self.send_act(&json!({ "verb": verb, "args": args })).await;
        assert_eq!(acks.len(), 1, "one request gets one acknowledgement: {acks:?}");
        acks.remove(0)
    }

    async fn plan(&mut self, requests: Vec<Value>) -> Vec<Value> {
        self.send_act(&json!({ "plan": requests })).await
    }

    async fn observe(&mut self, detail: &str, sections: &[&str]) -> Value {
        let mut path = format!("/v1/observe?detail={detail}&since={}", self.cursor);
        if !sections.is_empty() {
            // The reports are news. The agent reads them with every
            // observation, or the cursor would pass them.
            let mut names = sections.to_vec();
            if !names.contains(&"reports") {
                names.push("reports");
            }
            path.push_str(&format!("&sections={}", names.join(",")));
        }
        let (status, observation) = self.call(Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {observation}");
        self.read(&observation);
        observation
    }

    async fn news(&mut self, world: &World) {
        let before = self.inbox.seqs().len();
        // The long poll of 60 seconds ends when news comes. The world
        // runs until then.
        world.until(self.wait("&timeout=60")).await;
        assert!(self.inbox.seqs().len() > before, "the long poll ended with news");
    }

    fn inbox(&self) -> &Inbox {
        &self.inbox
    }
}
