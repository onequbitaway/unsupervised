//! The signature of a webhook delivery.
//!
//! Every delivery has three headers:
//!
//! - `X-Unsupervised-Delivery`: an id of the delivery. It stays the same
//!   when the server sends the delivery again.
//! - `X-Unsupervised-Timestamp`: the time of the attempt, in seconds
//!   since 1970.
//! - `X-Unsupervised-Signature`: `v1=` and the HMAC-SHA256, in lowercase
//!   hexadecimal, of the text `v1:<timestamp>:<body>` with the secret of
//!   the webhook as the key.
//!
//! A receiver computes the same value with [`verify`] and refuses a
//! delivery whose signature differs or whose timestamp is old.

use sha2::{Digest, Sha256};
use unsup_store::constant_time_eq;

/// Name of the header with the id of the delivery.
pub const DELIVERY_HEADER: &str = "x-unsupervised-delivery";
/// Name of the header with the time of the attempt.
pub const TIMESTAMP_HEADER: &str = "x-unsupervised-timestamp";
/// Name of the header with the signature.
pub const SIGNATURE_HEADER: &str = "x-unsupervised-signature";

/// The version label of the signature scheme.
const VERSION: &str = "v1";
/// Block size of SHA-256, in bytes.
const BLOCK: usize = 64;

/// Computes HMAC-SHA256 of a message (RFC 2104).
#[must_use]
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // A key longer than one block is replaced by its hash.
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new().chain_update(block.map(|b| b ^ 0x36)).chain_update(message);
    let outer = Sha256::new().chain_update(block.map(|b| b ^ 0x5c)).chain_update(inner.finalize());
    outer.finalize().into()
}

/// Returns the value of the signature header for a delivery.
#[must_use]
pub fn sign(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut message = format!("{VERSION}:{timestamp}:").into_bytes();
    message.extend_from_slice(body);
    format!("{VERSION}={}", hex::encode(hmac_sha256(secret.as_bytes(), &message)))
}

/// Returns true when `signature` is the signature header of this
/// delivery. The comparison takes the same time for every wrong value.
#[must_use]
pub fn verify(secret: &str, timestamp: u64, body: &[u8], signature: &str) -> bool {
    constant_time_eq(sign(secret, timestamp, body).as_bytes(), signature.as_bytes())
}
