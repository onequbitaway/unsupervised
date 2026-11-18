//! Sign-up: how an agent gets a resident and its key.
//!
//! The tool first tries the flow of `docs/design/protocol.md`, section 2:
//! `POST /v1/owners` gives an owner token, `POST /v1/residents` with that
//! token gives a resident and its key. When the server does not have these
//! routes, the tool uses the route of the development server:
//! `POST /v1/dev/residents`, where the key is the resident id.
//!
//! The protocol document does not fix the field names of the two answers.
//! This module accepts the names in [`TOKEN_NAMES`], [`KEY_NAMES`] and
//! [`ID_NAMES`], at the top of the answer or inside an `owner` or
//! `resident` object.

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::error::RequestError;
use super::http::{Urls, post_json};

/// Field names that can hold the token of an owner.
pub const TOKEN_NAMES: [&str; 4] = ["token", "owner_token", "key", "api_key"];

/// Field names that can hold the key of a resident.
pub const KEY_NAMES: [&str; 4] = ["key", "resident_key", "api_key", "token"];

/// Field names that can hold the id of a resident.
pub const ID_NAMES: [&str; 3] = ["resident", "id", "resident_id"];

/// The model name that the residents of the swarm show to viewers.
const MODEL: &str = "loadtest";

/// What an agent needs to act as its resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// Id of the resident.
    pub resident: u64,
    /// The bearer token of the resident.
    pub key: String,
}

/// The sign-up flow that a server has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    /// `POST /v1/owners`, then `POST /v1/residents`.
    Documented,
    /// `POST /v1/dev/residents`.
    Dev,
}

/// Signs agents up and remembers which flow the server has.
#[derive(Debug, Default)]
pub struct SignUp {
    /// 0: not known yet, 1: documented, 2: dev.
    flow: AtomicU8,
}

/// Looks for the first of `names` that holds a text, at the top of
/// `value` or inside its object `inner`.
fn find_text(value: &Value, inner: &str, names: &[&str]) -> Option<String> {
    let nested = value.get(inner);
    [Some(value), nested]
        .into_iter()
        .flatten()
        .find_map(|place| names.iter().find_map(|name| place.get(name)?.as_str()))
        .map(str::to_string)
}

/// Looks for the id of the new resident.
fn find_resident(value: &Value) -> Option<u64> {
    ID_NAMES.iter().find_map(|name| match value.get(name)? {
        Value::Number(id) => id.as_u64(),
        Value::Object(resident) => resident.get("id")?.as_u64(),
        _ => None,
    })
}

fn missing(what: &str, route: &str) -> RequestError {
    RequestError::local(
        "signup_answer",
        format!("the answer of {route} has no {what} under a name that this tool knows"),
    )
}

impl SignUp {
    /// Returns the flow that worked, or `None` when no agent has signed up
    /// yet.
    #[must_use]
    pub fn flow(&self) -> Option<Flow> {
        match self.flow.load(Ordering::Relaxed) {
            1 => Some(Flow::Documented),
            2 => Some(Flow::Dev),
            _ => None,
        }
    }

    fn remember(&self, flow: Flow) {
        let code = match flow {
            Flow::Documented => 1,
            Flow::Dev => 2,
        };
        self.flow.store(code, Ordering::Relaxed);
    }

    /// Creates a resident with the given name and returns its key. Returns
    /// also the time that the whole sign-up took.
    ///
    /// # Errors
    ///
    /// Fails when the server refuses the sign-up or when its answer has no
    /// key.
    pub async fn sign_up(
        &self,
        client: &reqwest::Client,
        urls: &Urls,
        name: &str,
    ) -> Result<Credential, RequestError> {
        if self.flow() != Some(Flow::Dev) {
            match documented(client, urls, name).await {
                Ok(credential) => {
                    self.remember(Flow::Documented);
                    return Ok(credential);
                }
                // The server does not have the documented routes. Other
                // errors are real: a server that has the routes and
                // refuses must not be bypassed. Many agents sign up at the
                // same time, so another agent can find the flow meanwhile:
                // an absent route is real only when the documented flow
                // worked before.
                Err(error) if error.route_absent() && self.flow() != Some(Flow::Documented) => {}
                Err(error) => return Err(error),
            }
        }
        let credential = dev(client, urls, name).await?;
        self.remember(Flow::Dev);
        Ok(credential)
    }
}

/// The flow of the protocol document: an owner, then a resident.
async fn documented(
    client: &reqwest::Client,
    urls: &Urls,
    name: &str,
) -> Result<Credential, RequestError> {
    let owner_body = json!({ "name": format!("{name}-owner") });
    let (owner, _): (Value, _) = post_json(client, &urls.owners, None, &owner_body).await?;
    let token = find_text(&owner, "owner", &TOKEN_NAMES)
        .ok_or_else(|| missing("owner token", "POST /v1/owners"))?;
    let resident_body = json!({ "name": name, "model": MODEL });
    let (answer, _): (Value, _) =
        post_json(client, &urls.residents, Some(&token), &resident_body).await?;
    let resident =
        find_resident(&answer).ok_or_else(|| missing("resident id", "POST /v1/residents"))?;
    let key = find_text(&answer, "resident", &KEY_NAMES)
        .ok_or_else(|| missing("resident key", "POST /v1/residents"))?;
    Ok(Credential { resident, key })
}

/// The flow of the development server: the key is the resident id.
async fn dev(
    client: &reqwest::Client,
    urls: &Urls,
    name: &str,
) -> Result<Credential, RequestError> {
    let body = json!({ "name": name, "model": MODEL });
    let (answer, _): (Value, _) = post_json(client, &urls.dev_residents, None, &body).await?;
    let resident =
        find_resident(&answer).ok_or_else(|| missing("resident id", "POST /v1/dev/residents"))?;
    Ok(Credential { resident, key: resident.to_string() })
}

/// Signs up and measures the time of the whole sign-up.
///
/// # Errors
///
/// Fails like [`SignUp::sign_up`].
pub async fn timed(
    signup: &SignUp,
    client: &reqwest::Client,
    urls: &Urls,
    name: &str,
) -> Result<(Credential, std::time::Duration), RequestError> {
    let start = Instant::now();
    let credential = signup.sign_up(client, urls, name).await?;
    Ok((credential, start.elapsed()))
}

/// The credentials of a swarm in a file, so that a later run can use the
/// same residents again. The file holds keys: keep it private.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialFile {
    /// The address of the server that gave the keys.
    pub server: String,
    /// One entry for each agent, in agent order.
    pub agents: Vec<Credential>,
}

impl CredentialFile {
    /// Reads the credentials for `server` from a file. A file that does
    /// not exist gives none. A file of another server gives none and a
    /// warning.
    ///
    /// # Errors
    ///
    /// Fails when the file exists but cannot be read or is not valid.
    pub fn load(path: &Path, server: &str) -> anyhow::Result<(Vec<Credential>, Option<String>)> {
        if !path.exists() {
            return Ok((Vec::new(), None));
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let file: CredentialFile = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a credentials file", path.display()))?;
        if file.server == server {
            Ok((file.agents, None))
        } else {
            let warning = format!(
                "{} holds keys of {}, not of {server}; the agents sign up again",
                path.display(),
                file.server
            );
            Ok((Vec::new(), Some(warning)))
        }
    }

    /// Writes the credentials to a file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot make the directory {}", dir.display()))?;
        }
        let text = serde_json::to_string(self).context("cannot write the credentials")?;
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use tokio::net::TcpListener;
    use tokio::sync::watch;

    use super::*;

    #[test]
    fn the_answer_of_the_development_server_is_read() {
        let answer = json!({ "resident": 42 });
        assert_eq!(find_resident(&answer), Some(42));
        assert_eq!(find_text(&answer, "resident", &KEY_NAMES), None);
    }

    #[test]
    fn several_shapes_of_the_documented_answers_are_read() {
        let flat = json!({ "resident": 7, "key": "rk_flat" });
        assert_eq!(find_resident(&flat), Some(7));
        assert_eq!(find_text(&flat, "resident", &KEY_NAMES).as_deref(), Some("rk_flat"));

        let nested = json!({ "resident": { "id": 8, "name": "bot" }, "key": "rk_nested" });
        assert_eq!(find_resident(&nested), Some(8));
        assert_eq!(find_text(&nested, "resident", &KEY_NAMES).as_deref(), Some("rk_nested"));

        let inside = json!({ "resident": { "id": 9, "key": "rk_inside" } });
        assert_eq!(find_resident(&inside), Some(9));
        assert_eq!(find_text(&inside, "resident", &KEY_NAMES).as_deref(), Some("rk_inside"));

        let owner = json!({ "owner": { "id": 3 }, "token": "ot_1" });
        assert_eq!(find_text(&owner, "owner", &TOKEN_NAMES).as_deref(), Some("ot_1"));
        let owner_inside = json!({ "owner": { "id": 3, "token": "ot_2" } });
        assert_eq!(find_text(&owner_inside, "owner", &TOKEN_NAMES).as_deref(), Some("ot_2"));

        assert_eq!(find_resident(&json!({ "name": "x" })), None);
        assert_eq!(find_text(&json!({ "id": 3 }), "owner", &TOKEN_NAMES), None);
    }

    /// What the test server of the next test shares with the test.
    #[derive(Clone)]
    struct Gate {
        /// Set when the sign-up of the slow agent reached the server.
        slow_arrived: Arc<watch::Sender<bool>>,
        /// Set by the test when the slow agent can get its answer.
        release: Arc<watch::Sender<bool>>,
        residents: Arc<AtomicU64>,
    }

    /// A server without the documented route: it answers 404 without a
    /// body, like a web server does for a route that it does not have. The
    /// answer for the agent `slow` waits until the test releases it.
    async fn owners(State(gate): State<Gate>, Json(body): Json<Value>) -> StatusCode {
        if body["name"].as_str().is_some_and(|name| name.starts_with("slow")) {
            gate.slow_arrived.send_replace(true);
            let mut release = gate.release.subscribe();
            // The sender lives as long as the server.
            let _ = release.wait_for(|released| *released).await;
        }
        StatusCode::NOT_FOUND
    }

    async fn dev_residents(State(gate): State<Gate>) -> Json<Value> {
        Json(json!({ "resident": gate.residents.fetch_add(1, Ordering::Relaxed) + 1 }))
    }

    #[tokio::test]
    async fn an_agent_whose_sign_up_is_under_way_when_another_finds_the_flow_still_signs_up() {
        let gate = Gate {
            slow_arrived: Arc::new(watch::Sender::new(false)),
            release: Arc::new(watch::Sender::new(false)),
            residents: Arc::new(AtomicU64::new(0)),
        };
        let app = Router::new()
            .route("/v1/owners", post(owners))
            .route("/v1/dev/residents", post(dev_residents))
            .with_state(gate.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let urls = Arc::new(Urls::new(&base, "/v1/stats").expect("a valid address"));
        let client = crate::swarm::http::client(std::time::Duration::from_secs(30), false)
            .expect("a client");
        let signup = Arc::new(SignUp::default());

        // The slow agent starts first. Its request for the documented
        // route is under way.
        let slow = {
            let (signup, client, urls) = (Arc::clone(&signup), client.clone(), Arc::clone(&urls));
            tokio::spawn(async move { signup.sign_up(&client, &urls, "slow").await })
        };
        let mut arrived = gate.slow_arrived.subscribe();
        arrived.wait_for(|arrived| *arrived).await.expect("the server runs");
        assert_eq!(signup.flow(), None, "nobody has signed up yet");

        // The fast agent finds that the server has only the development
        // route.
        let fast = signup.sign_up(&client, &urls, "fast").await.expect("the fast agent signs up");
        assert_eq!(fast, Credential { resident: 1, key: "1".into() });
        assert_eq!(signup.flow(), Some(Flow::Dev));

        // Now the slow agent gets its 404. It must use the development
        // route too, not fail.
        gate.release.send_replace(true);
        let slow = slow.await.expect("the task ends").expect("the slow agent signs up");
        assert_eq!(slow, Credential { resident: 2, key: "2".into() });
        assert_eq!(signup.flow(), Some(Flow::Dev));
        server.abort();
    }

    #[test]
    fn credentials_survive_a_round_trip_and_belong_to_one_server() {
        let dir = std::env::temp_dir().join(format!("unsup-credentials-{}", std::process::id()));
        let path = dir.join("agents.json");
        let (none, warning) = CredentialFile::load(&path, "http://a").expect("no file is fine");
        assert!(none.is_empty());
        assert_eq!(warning, None);

        let file = CredentialFile {
            server: "http://a".into(),
            agents: vec![Credential { resident: 1, key: "k1".into() }],
        };
        file.save(&path).expect("the file is written");
        let (same, warning) = CredentialFile::load(&path, "http://a").expect("the file is read");
        assert_eq!(same, file.agents);
        assert_eq!(warning, None);
        let (other, warning) = CredentialFile::load(&path, "http://b").expect("the file is read");
        assert!(other.is_empty());
        assert!(warning.expect("a warning").contains("http://b"));
        std::fs::remove_dir_all(&dir).expect("the temporary directory is removed");
    }
}
