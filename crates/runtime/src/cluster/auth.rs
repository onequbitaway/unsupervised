//! The shared secret of a cluster, and the first step of the handshake.
//!
//! The coordinator lets a connection in only when the other side proves
//! that it knows the secret of the cluster. The secret itself never
//! travels:
//!
//! 1. The coordinator sends a [`Challenge`]: a fresh number of 32 bytes.
//! 2. The worker answers with a [`Proof`]: a fresh number of its own and a
//!    keyed hash (BLAKE3) over its role and both numbers. The key comes
//!    from the secret.
//! 3. The coordinator computes the same hash and compares in constant
//!    time. A proof that is wrong ends the connection with
//!    [`Admission::Refused`], before the coordinator reads anything else.
//!    A proof that is right gets [`Admission::Proven`] with the proof of
//!    the coordinator, which the worker checks in the same way.
//!
//! A proof holds for one pair of numbers and for one role, so a proof that
//! somebody recorded opens no later connection, and the proof of one side
//! is no proof of the other.
//!
//! A cluster without a secret uses a fixed key that everybody knows. The
//! steps are the same, and they prove nothing. Such a cluster is allowed
//! on a loopback address only.
//!
//! The proof protects the door, not the room. The connection has no
//! encryption and no check of each message. Run a cluster on a private
//! network.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::error::ClusterError;
use super::wire::{Hash, WireError, read_frame, write_frame};

/// Context of the key that the secret gives. Part of the protocol.
const KEY_CONTEXT: &str = "unsupervised cluster 2 handshake key";
/// Context of the numbers of a challenge. It only keeps their bytes apart
/// from every other hash of the program.
const NONCE_CONTEXT: &str = "unsupervised cluster 2 handshake nonce";
/// Greatest frame of this step in bytes. The messages of the step are
/// small, so a peer that is not let in yet cannot make the coordinator
/// read much. Part of the protocol.
const STEP_FRAME_BYTES: u32 = 1024;
/// Greatest length of the reason of a refusal that a worker reads.
const REASON_BYTES: usize = 512;

/// Why the coordinator refuses a peer whose proof is wrong. The text says
/// nothing about the secret.
pub(crate) const WRONG_SECRET: &str =
    "the worker did not prove that it knows the shared secret of the cluster";
/// Why the coordinator refuses a peer that did not send a proof first.
pub(crate) const NO_PROOF: &str =
    "the first message of a connection must be the proof of the shared secret";

/// The shared secret of a cluster.
///
/// Every process of one cluster gets the same secret: the coordinator
/// with [`Coordinator::bind_with_secret`](super::Coordinator::bind_with_secret),
/// each worker with [`WorkerConfig::with_secret`](super::WorkerConfig::with_secret).
/// A worker that does not know it cannot join.
///
/// Use at least 32 random bytes, for example from a secret store. The
/// value keeps a key that is derived from the secret, not the secret. It
/// does not show the key when it is printed.
#[derive(Clone, PartialEq, Eq)]
pub struct ClusterSecret {
    key: [u8; blake3::KEY_LEN],
}

impl fmt::Debug for ClusterSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClusterSecret(..)")
    }
}

/// The two sides of a connection. A proof names the side that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// The process that listens.
    Coordinator,
    /// The process that connects.
    Worker,
}

impl Role {
    /// Returns the bytes that stand for the role in a proof.
    const fn tag(self) -> &'static [u8] {
        match self {
            Role::Coordinator => b"coordinator",
            Role::Worker => b"worker",
        }
    }
}

impl ClusterSecret {
    /// Makes the secret of a cluster from bytes that only the operator of
    /// the cluster knows.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Config`] when the secret is empty.
    pub fn new(secret: impl AsRef<[u8]>) -> Result<ClusterSecret, ClusterError> {
        let secret = secret.as_ref();
        if secret.is_empty() {
            return Err(ClusterError::Config("the shared secret of the cluster is empty".into()));
        }
        Ok(ClusterSecret { key: blake3::derive_key(KEY_CONTEXT, secret) })
    }

    /// Returns the key of a cluster without a secret. Everybody knows it.
    fn open() -> ClusterSecret {
        ClusterSecret { key: blake3::derive_key(KEY_CONTEXT, &[]) }
    }

    /// Returns the key that a process uses: that of its secret, or the one
    /// that everybody knows.
    pub(crate) fn or_open(secret: Option<&ClusterSecret>) -> ClusterSecret {
        secret.cloned().unwrap_or_else(ClusterSecret::open)
    }

    /// Computes the proof of one side for one pair of numbers.
    fn prove(&self, role: Role, challenge: &Hash, reply: &Hash) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        let tag = role.tag();
        hasher.update(&(tag.len() as u64).to_le_bytes());
        hasher.update(tag);
        hasher.update(&challenge.0);
        hasher.update(&reply.0);
        hasher.finalize()
    }

    /// Returns true when `proof` is the proof of `role` for the pair of
    /// numbers. The comparison takes the same time for every wrong proof.
    fn accepts(&self, role: Role, challenge: &Hash, reply: &Hash, proof: &Hash) -> bool {
        // `blake3::Hash` compares in constant time. An array does not.
        self.prove(role, challenge, reply) == blake3::Hash::from_bytes(proof.0)
    }
}

/// Makes a number of 32 bytes that no connection had before and that
/// nobody outside can guess.
///
/// The runtime has no random source of its own. The standard library has
/// one: each `RandomState` gets keys from the random source of the
/// operating system. The number mixes the output of two of them with the
/// clock, the process id and a counter.
fn nonce() -> Hash {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    let clock = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let mut hasher = blake3::Hasher::new_derive_key(NONCE_CONTEXT);
    hasher.update(&RandomState::new().hash_one(count).to_le_bytes());
    hasher.update(&RandomState::new().hash_one(clock).to_le_bytes());
    hasher.update(&clock.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&count.to_le_bytes());
    Hash(*hasher.finalize().as_bytes())
}

/// The first message of a connection, from the coordinator.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct Challenge {
    /// A fresh number.
    pub nonce: Hash,
}

/// The first message of a worker.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct Proof {
    /// A fresh number of the worker.
    pub nonce: Hash,
    /// The keyed hash over the role of the worker and both numbers.
    pub proof: Hash,
}

/// What the coordinator makes of a [`Proof`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Admission {
    /// The worker knows the secret. This is the proof of the coordinator.
    Proven { proof: Hash },
    /// The connection ends.
    Refused { reason: String },
}

/// Returns the first address that is not a loopback address.
pub(crate) fn not_loopback(addresses: &[SocketAddr]) -> Option<SocketAddr> {
    addresses.iter().copied().find(|address| !address.ip().is_loopback())
}

/// Sets how long one read and one write of the step can take. `None`
/// takes the limits away.
fn limit(stream: &TcpStream, patience: Option<Duration>) -> std::io::Result<()> {
    let patience = patience.filter(|limit| !limit.is_zero());
    stream.set_read_timeout(patience)?;
    stream.set_write_timeout(patience)
}

/// Tells a peer why it is not let in. Returns the reason.
fn refuse(stream: &mut TcpStream, reason: &str) -> String {
    let refused = Admission::Refused { reason: reason.to_string() };
    // The peer may be gone. That is the same result.
    let _ = write_frame(stream, &refused, STEP_FRAME_BYTES);
    reason.to_string()
}

/// The coordinator side of the step. It sends the challenge, reads one
/// small frame and checks the proof in it. Nothing else is read from a
/// peer that this function does not let in.
///
/// # Errors
///
/// The text says why the connection is not used. The peer got a refusal
/// with a reason when it sent a frame that is no proof or a wrong proof.
pub(crate) fn admit(
    stream: &mut TcpStream,
    secret: &ClusterSecret,
    patience: Duration,
) -> Result<(), String> {
    let text = |error: &dyn fmt::Display| error.to_string();
    // A connection of a listener that does not block can be the same.
    stream.set_nonblocking(false).map_err(|e| text(&e))?;
    stream.set_nodelay(true).map_err(|e| text(&e))?;
    limit(stream, Some(patience)).map_err(|e| text(&e))?;
    let challenge = Challenge { nonce: nonce() };
    write_frame(stream, &challenge, STEP_FRAME_BYTES).map_err(|e| text(&e))?;
    let answer = match read_frame::<_, Proof>(stream, STEP_FRAME_BYTES) {
        Ok((answer, _)) => answer,
        // Something came, and it is not a proof: a frame that is too
        // large, or another message.
        Err(WireError::TooLarge { .. } | WireError::Decode(_)) => {
            return Err(refuse(stream, NO_PROOF));
        }
        Err(error) => return Err(text(&error)),
    };
    if !secret.accepts(Role::Worker, &challenge.nonce, &answer.nonce, &answer.proof) {
        return Err(refuse(stream, WRONG_SECRET));
    }
    let proof = secret.prove(Role::Coordinator, &challenge.nonce, &answer.nonce);
    let proven = Admission::Proven { proof: Hash(*proof.as_bytes()) };
    write_frame(stream, &proven, STEP_FRAME_BYTES).map_err(|e| text(&e))?;
    limit(stream, None).map_err(|e| text(&e))
}

/// The worker side of the step. It answers the challenge of the
/// coordinator and checks the proof of the coordinator.
///
/// # Errors
///
/// [`ClusterError::Refused`] when the coordinator does not accept the
/// proof: the two processes have different secrets.
/// [`ClusterError::Protocol`] when the coordinator does not prove that it
/// knows the secret. [`ClusterError::CoordinatorLost`] when the connection
/// fails.
pub(crate) fn present(stream: &mut TcpStream, secret: &ClusterSecret) -> Result<(), ClusterError> {
    let lost = |error: &WireError| ClusterError::CoordinatorLost { reason: error.to_string() };
    let (challenge, _) =
        read_frame::<_, Challenge>(stream, STEP_FRAME_BYTES).map_err(|e| lost(&e))?;
    let reply = nonce();
    let proof = secret.prove(Role::Worker, &challenge.nonce, &reply);
    let answer = Proof { nonce: reply, proof: Hash(*proof.as_bytes()) };
    write_frame(stream, &answer, STEP_FRAME_BYTES).map_err(|e| lost(&e))?;
    match read_frame::<_, Admission>(stream, STEP_FRAME_BYTES).map_err(|e| lost(&e))? {
        (Admission::Proven { proof }, _)
            if secret.accepts(Role::Coordinator, &challenge.nonce, &reply, &proof) =>
        {
            Ok(())
        }
        (Admission::Proven { .. }, _) => Err(ClusterError::Protocol(
            "the coordinator did not prove that it knows the shared secret of the cluster".into(),
        )),
        (Admission::Refused { mut reason }, _) => {
            // The text comes from a process that proved nothing.
            if reason.len() > REASON_BYTES {
                let cut = (0..=REASON_BYTES).rev().find(|&at| reason.is_char_boundary(at));
                reason.truncate(cut.unwrap_or(0));
            }
            Err(ClusterError::Refused { reason })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> ClusterSecret {
        ClusterSecret::new(text).expect("a secret that is not empty")
    }

    fn number(byte: u8) -> Hash {
        Hash([byte; 32])
    }

    fn proof_of(secret: &ClusterSecret, role: Role, challenge: u8, reply: u8) -> Hash {
        Hash(*secret.prove(role, &number(challenge), &number(reply)).as_bytes())
    }

    #[test]
    fn a_proof_holds_for_one_secret_one_role_and_one_pair_of_numbers() {
        let key = secret("correct horse battery staple");
        let proof = proof_of(&key, Role::Worker, 1, 2);
        assert!(key.accepts(Role::Worker, &number(1), &number(2), &proof));
        // Another secret, the other role, another challenge, another reply.
        assert!(!secret("correct horse battery stapler").accepts(
            Role::Worker,
            &number(1),
            &number(2),
            &proof
        ));
        assert!(!key.accepts(Role::Coordinator, &number(1), &number(2), &proof));
        assert!(!key.accepts(Role::Worker, &number(3), &number(2), &proof));
        assert!(!key.accepts(Role::Worker, &number(1), &number(3), &proof));
        // The two numbers do not change places.
        assert!(!key.accepts(Role::Worker, &number(2), &number(1), &proof));
        // One wrong bit is enough.
        let mut bent = proof;
        bent.0[31] ^= 1;
        assert!(!key.accepts(Role::Worker, &number(1), &number(2), &bent));
    }

    #[test]
    fn the_same_secret_gives_the_same_key_in_every_process() {
        assert_eq!(secret("s3cret"), secret("s3cret"));
        assert_ne!(secret("s3cret"), secret("s3cres"));
        assert_eq!(
            proof_of(&secret("s3cret"), Role::Coordinator, 4, 5),
            proof_of(&secret("s3cret"), Role::Coordinator, 4, 5)
        );
        // No secret is a key of its own, which every such process has.
        assert_eq!(ClusterSecret::or_open(None), ClusterSecret::or_open(None));
        assert_ne!(ClusterSecret::or_open(None), secret("s3cret"));
        assert_eq!(ClusterSecret::or_open(Some(&secret("s3cret"))), secret("s3cret"));
    }

    #[test]
    fn an_empty_secret_is_refused_and_a_secret_is_not_printed() {
        assert_eq!(
            ClusterSecret::new(""),
            Err(ClusterError::Config("the shared secret of the cluster is empty".into()))
        );
        let printed = format!("{:?}", secret("hunter2"));
        assert_eq!(printed, "ClusterSecret(..)");
    }

    #[test]
    fn each_challenge_is_new() {
        let mut seen: Vec<Hash> = (0..1_000).map(|_| nonce()).collect();
        seen.sort_unstable_by_key(|number| number.0);
        seen.dedup();
        assert_eq!(seen.len(), 1_000);
    }

    #[test]
    fn only_a_loopback_address_is_one() {
        let address = |text: &str| text.parse::<SocketAddr>().expect("an address");
        let local = [address("127.0.0.1:7000"), address("[::1]:7000"), address("127.8.9.1:1")];
        assert_eq!(not_loopback(&local), None);
        for open in ["0.0.0.0:7000", "[::]:7000", "10.0.0.5:7000", "192.168.1.2:7000"] {
            let list = [address("127.0.0.1:7000"), address(open)];
            assert_eq!(not_loopback(&list), Some(address(open)), "{open}");
        }
    }
}
