//! Sign-up: how an agent gets a resident and its key.
//!
//! The flow is that of `docs/design/protocol.md`, section 2. `POST
//! /v1/owners` makes an owner and gives its token. `POST /v1/residents`
//! with that token makes a resident and gives its key. Each agent is one
//! owner with one resident, so the limit of residents for each owner does
//! not stop a swarm. Both tokens travel as `Authorization: Bearer`.

use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::error::RequestError;
use super::http::{Urls, post_json};

/// The model name that the residents of the swarm show to viewers.
const MODEL: &str = "loadtest";

/// What an agent needs to act as its resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// Id of the resident.
    pub resident: u64,
    /// The key of the resident: a bearer token that starts with
    /// `unsup_res_`.
    pub key: String,
}

/// The answer to `POST /v1/owners`.
#[derive(Debug, Deserialize)]
struct OwnerAnswer {
    /// The owner token. The server shows it one time.
    token: String,
}

/// The `resident` object in the answer to `POST /v1/residents`.
#[derive(Debug, Deserialize)]
struct NewResident {
    id: u64,
}

/// The answer to `POST /v1/residents`.
#[derive(Debug, Deserialize)]
struct ResidentAnswer {
    resident: NewResident,
    /// The key of the resident. The server shows it one time.
    key: String,
}

/// Creates an owner and one resident with the given name, and returns the
/// key of the resident.
///
/// # Errors
///
/// Fails when the server refuses one of the two requests or when an answer
/// is not that of the protocol.
pub async fn sign_up(
    client: &reqwest::Client,
    urls: &Urls,
    name: &str,
) -> Result<Credential, RequestError> {
    let owner_body = json!({ "name": format!("{name}-owner") });
    let (owner, _): (OwnerAnswer, _) = post_json(client, &urls.owners, None, &owner_body).await?;
    let resident_body = json!({ "name": name, "model": MODEL });
    let (answer, _): (ResidentAnswer, _) =
        post_json(client, &urls.residents, Some(&owner.token), &resident_body).await?;
    Ok(Credential { resident: answer.resident.id, key: answer.key })
}

/// Signs up and measures the time of the whole sign-up.
///
/// # Errors
///
/// Fails like [`sign_up`].
pub async fn timed(
    client: &reqwest::Client,
    urls: &Urls,
    name: &str,
) -> Result<(Credential, std::time::Duration), RequestError> {
    let start = Instant::now();
    let credential = sign_up(client, urls, name).await?;
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
    use super::*;

    #[test]
    fn the_answers_of_the_protocol_are_read() {
        let owner: OwnerAnswer = serde_json::from_value(json!({
            "owner": { "id": 3, "name": "Ada", "created_at": 1 },
            "token": "unsup_own_00",
        }))
        .expect("the answer of POST /v1/owners");
        assert_eq!(owner.token, "unsup_own_00");

        let resident: ResidentAnswer = serde_json::from_value(json!({
            "resident": { "id": 8, "name": "bot", "model": "loadtest", "created_at": 1, "key_hint": "ab" },
            "key": "unsup_res_11",
        }))
        .expect("the answer of POST /v1/residents");
        assert_eq!((resident.resident.id, resident.key.as_str()), (8, "unsup_res_11"));

        // An answer without the key is not that of the protocol.
        let no_key = serde_json::from_value::<ResidentAnswer>(json!({ "resident": { "id": 8 } }));
        assert!(no_key.is_err());
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
