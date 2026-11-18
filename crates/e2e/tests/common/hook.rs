//! A webhook receiver: a small HTTP server that keeps each delivery, and
//! the signature check of the protocol document, written again here.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Block size of SHA-256, in bytes.
const BLOCK: usize = 64;

/// One attempt of a delivery, as the receiver got it.
#[derive(Clone, Debug)]
pub struct Delivery {
    /// The header `X-Unsupervised-Delivery`.
    pub delivery: String,
    /// The header `X-Unsupervised-Timestamp`.
    pub timestamp: String,
    /// The header `X-Unsupervised-Signature`.
    pub signature: String,
    /// The header `Content-Type`.
    pub content_type: String,
    /// The exact bytes of the body.
    pub body: Vec<u8>,
    /// The status that the receiver answered.
    pub answered: StatusCode,
}

impl Delivery {
    /// Returns the body as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a delivery is JSON")
    }
}

#[derive(Clone)]
struct Shared {
    /// Number of attempts that the receiver still refuses.
    refuse: Arc<AtomicUsize>,
    seen: mpsc::UnboundedSender<Delivery>,
}

/// A running receiver.
pub struct Receiver {
    /// The address to give to `PUT /v1/webhook`.
    pub url: String,
    seen: mpsc::UnboundedReceiver<Delivery>,
    task: JoinHandle<()>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default().to_string()
}

async fn receive(State(shared): State<Shared>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let refuse = shared
        .refuse
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
        .is_ok();
    let answered = if refuse { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::NO_CONTENT };
    // The test can be done already. Then nobody reads the delivery.
    let _ = shared.seen.send(Delivery {
        delivery: header(&headers, "x-unsupervised-delivery"),
        timestamp: header(&headers, "x-unsupervised-timestamp"),
        signature: header(&headers, "x-unsupervised-signature"),
        content_type: header(&headers, "content-type"),
        body: body.to_vec(),
        answered,
    });
    answered
}

impl Receiver {
    /// Starts a receiver on a free local port. It answers the first
    /// `refuse` attempts with 503 and each later attempt with 204.
    pub async fn start(refuse: usize) -> Receiver {
        let (seen_tx, seen) = mpsc::unbounded_channel();
        let shared = Shared { refuse: Arc::new(AtomicUsize::new(refuse)), seen: seen_tx };
        let app = Router::new().route("/hook", post(receive)).with_state(shared);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
        let addr = listener.local_addr().expect("the address of the listener");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the receiver runs");
        });
        Receiver { url: format!("http://{addr}/hook"), seen, task }
    }

    /// Waits for the next attempt of a delivery.
    pub async fn next(&mut self) -> Delivery {
        self.seen.recv().await.expect("the receiver runs")
    }

    /// Returns an attempt that arrived already, without a wait.
    pub fn arrived(&mut self) -> Option<Delivery> {
        self.seen.try_recv().ok()
    }
}

/// Computes HMAC-SHA256 of a message (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner_pad: Vec<u8> = block.iter().map(|byte| byte ^ 0x36).collect();
    let outer_pad: Vec<u8> = block.iter().map(|byte| byte ^ 0x5c).collect();
    let inner = Sha256::new().chain_update(&inner_pad).chain_update(message).finalize();
    Sha256::new().chain_update(&outer_pad).chain_update(inner).finalize().to_vec()
}

/// Returns the signature header that the protocol document gives for a
/// delivery: `v1=` and the HMAC-SHA256, in lowercase hexadecimal, of
/// `v1:<timestamp>:<body>` with the secret as the key.
pub fn signature(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut message = format!("v1:{timestamp}:").into_bytes();
    message.extend_from_slice(body);
    format!("v1={}", hex::encode(hmac_sha256(secret.as_bytes(), &message)))
}
