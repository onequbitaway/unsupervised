//! The `admin` command: calls the admin routes of a running server.

use std::time::Duration;

use anyhow::Context;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{Value, json};

/// Time limit of one call.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The options of `admin`.
#[derive(Args)]
pub struct AdminArgs {
    /// The address of the server.
    #[arg(long, env = "UNSUPERVISED_ADMIN_URL", default_value = "http://127.0.0.1:8080")]
    url: String,
    /// The token of the administrator. Prefer the environment variable:
    /// other users of the machine can read a flag.
    #[arg(long, env = "UNSUPERVISED_ADMIN_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[command(subcommand)]
    command: AdminCommand,
}

/// One call of an admin route.
#[derive(Subcommand)]
enum AdminCommand {
    /// Show the dashboard: the simulation, the journal and the snapshots,
    /// the economy, moderation, flags and alerts.
    Dashboard,
    /// Show the numbers of the simulation, the store and the gateway.
    Stats,
    /// Show the alert list.
    Alerts,
    /// Show the flags of the abuse detectors, with their evidence.
    Flags,
    /// Close a flag.
    DismissFlag {
        /// Id of the flag.
        id: u64,
    },
    /// Show every sanction in force.
    Sanctions,
    /// Mute a resident: it cannot send text.
    Mute {
        /// Id of the resident.
        resident: u64,
        /// Why. The list of sanctions shows it.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Undo `mute`.
    Unmute {
        /// Id of the resident.
        resident: u64,
    },
    /// Freeze a resident: the simulation refuses every action of it.
    Freeze {
        /// Id of the resident.
        resident: u64,
        /// Why. The list of sanctions shows it.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Undo `freeze`.
    Unfreeze {
        /// Id of the resident.
        resident: u64,
    },
    /// Remove a resident from the world.
    Despawn {
        /// Id of the resident.
        resident: u64,
    },
    /// Ban an owner: no token of the owner and no key of its residents
    /// works.
    Ban {
        /// Id of the owner.
        owner: u64,
        /// Why. The list of sanctions shows it.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Undo `ban`.
    Unban {
        /// Id of the owner.
        owner: u64,
    },
    /// Put the whole world back to the newest snapshot at or before a
    /// frame. Everything after that snapshot is lost for good.
    Rollback {
        /// The frame.
        frame: u64,
    },
    /// Change a parameter of the simulation.
    SetParam {
        /// Name of the parameter.
        name: String,
        /// New value.
        #[arg(allow_negative_numbers = true)]
        value: i64,
        /// Change the value for one city only.
        #[arg(long)]
        city: Option<u16>,
    },
}

/// Returns the body for a route that takes an optional reason.
fn reason_body(reason: Option<String>) -> Option<Value> {
    reason.map(|reason| json!({ "reason": reason }))
}

impl AdminCommand {
    /// Returns the method, the path and the body of the call.
    fn request(self) -> (Method, String, Option<Value>) {
        let get = |path: &str| (Method::GET, path.to_string(), None);
        let post = |path: String, body: Option<Value>| (Method::POST, path, body);
        match self {
            AdminCommand::Dashboard => get("/admin/dashboard"),
            AdminCommand::Stats => get("/admin/stats"),
            AdminCommand::Alerts => get("/admin/alerts"),
            AdminCommand::Flags => get("/admin/flags"),
            AdminCommand::Sanctions => get("/admin/sanctions"),
            AdminCommand::DismissFlag { id } => post(format!("/admin/flags/{id}/dismiss"), None),
            AdminCommand::Mute { resident, reason } => {
                post(format!("/admin/residents/{resident}/mute"), reason_body(reason))
            }
            AdminCommand::Unmute { resident } => {
                post(format!("/admin/residents/{resident}/unmute"), None)
            }
            AdminCommand::Freeze { resident, reason } => {
                post(format!("/admin/residents/{resident}/freeze"), reason_body(reason))
            }
            AdminCommand::Unfreeze { resident } => {
                post(format!("/admin/residents/{resident}/unfreeze"), None)
            }
            AdminCommand::Despawn { resident } => {
                post(format!("/admin/residents/{resident}/despawn"), None)
            }
            AdminCommand::Ban { owner, reason } => {
                post(format!("/admin/owners/{owner}/ban"), reason_body(reason))
            }
            AdminCommand::Unban { owner } => post(format!("/admin/owners/{owner}/unban"), None),
            AdminCommand::Rollback { frame } => {
                post("/admin/rollback".into(), Some(json!({ "frame": frame })))
            }
            AdminCommand::SetParam { name, value, city } => {
                let mut body = json!({ "name": name, "value": value });
                if let Some(city) = city {
                    body["city"] = json!(city);
                }
                post("/admin/params".into(), Some(body))
            }
        }
    }
}

/// Runs the command: sends the call, prints the answer as JSON. It fails,
/// and the process ends with an error, when the server refuses the call.
/// The token is never printed.
pub async fn run(args: AdminArgs) -> anyhow::Result<()> {
    let token = args.token.filter(|token| !token.is_empty()).context(
        "the admin token is missing: put it in the environment variable \
         UNSUPERVISED_ADMIN_TOKEN",
    )?;
    let (method, path, body) = args.command.request();
    let url = format!("{}{path}", args.url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("cannot make the HTTP client")?;
    let mut request = client.request(method, &url).bearer_auth(&token);
    if let Some(body) = &body {
        request = request.json(body);
    }
    // An error of the client can name the address. It never holds the
    // token, which travels in a header.
    let response = request.send().await.with_context(|| format!("cannot reach {url}"))?;
    let status = response.status();
    let bytes =
        response.bytes().await.with_context(|| format!("cannot read the answer of {url}"))?;
    let answer: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    if status.is_success() {
        println!("{}", serde_json::to_string_pretty(&answer).context("cannot print the answer")?);
        return Ok(());
    }
    let error = &answer["error"];
    match (error["code"].as_str(), error["message"].as_str()) {
        (Some(code), Some(message)) => {
            anyhow::bail!("{path} answered {status} ({code}): {message}")
        }
        _ => anyhow::bail!("{path} answered {status}: {answer}"),
    }
}
