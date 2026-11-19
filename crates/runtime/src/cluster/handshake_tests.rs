//! Tests of the first step of the handshake over real connections on this
//! machine: who the coordinator lets in.

use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use unsup_kernel::WorldConfig;

use super::auth::{Admission, Challenge, NO_PROOF, Proof, WRONG_SECRET};
use super::fingerprint::fingerprint;
use super::proto::{FromWorker, PROTOCOL_VERSION, ToWorker};
use super::wire::{Hash, read_frame, write_frame};
use super::{
    ClusterConfig, ClusterError, ClusterHandle, ClusterJoin, ClusterSecret, Coordinator, Worker,
    WorkerConfig, WorkerExit, run_worker,
};
use crate::durable::Plugins;

/// Greatest frame of these tests, in bytes.
const MAX_FRAME: u32 = 1 << 20;

type Started = Result<(ClusterHandle, ClusterJoin), ClusterError>;

fn plugins() -> Plugins {
    Arc::new(Vec::new())
}

fn world() -> WorldConfig {
    WorldConfig::with_seed(7)
}

fn secret(text: &str) -> ClusterSecret {
    ClusterSecret::new(text).expect("a secret that is not empty")
}

/// Starts a coordinator for one worker on a thread. It gives up after
/// `join` when nobody took the slot.
fn start(coordinator: Coordinator, join: Duration) -> JoinHandle<Started> {
    let config = ClusterConfig { join_timeout: join, ..ClusterConfig::manual(world(), 1) };
    std::thread::spawn(move || coordinator.start(plugins(), config))
}

/// Opens a coordinator on a free port of this machine, with a secret or
/// without, and returns its address.
fn listen(secret: Option<ClusterSecret>) -> (Coordinator, SocketAddr) {
    let coordinator = match secret {
        Some(secret) => Coordinator::bind_with_secret("127.0.0.1:0", secret),
        None => Coordinator::bind("127.0.0.1:0"),
    }
    .expect("a free port on this machine");
    let address = coordinator.local_addr().expect("the address");
    (coordinator, address)
}

fn worker(address: SocketAddr, secret: Option<ClusterSecret>) -> WorkerConfig {
    WorkerConfig { secret, ..WorkerConfig::new(world(), 0, address) }
}

/// Reads what the other side sends until it closes the connection or says
/// nothing for a while.
fn read_all(stream: &mut TcpStream) -> Vec<u8> {
    stream.set_read_timeout(Some(Duration::from_millis(400))).expect("a time limit");
    let mut received = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return received,
            Ok(n) => received.extend_from_slice(&chunk[..n]),
        }
    }
}

/// The `Hello` of a worker that knows the world and the plugins and asks
/// for slot 0. Before the cluster had a shared secret, this message alone
/// let a peer in.
fn hello() -> FromWorker {
    let print = fingerprint(&plugins(), &world()).expect("a fingerprint");
    FromWorker::Hello {
        protocol: PROTOCOL_VERSION,
        world: print.world,
        plugins: print.plugins,
        slot: 0,
    }
}

/// Runs one frame of a cluster and stops the cluster in order. Returns
/// the number of the frame.
fn step_and_stop(started: Started, worker: JoinHandle<Result<WorkerExit, ClusterError>>) -> u64 {
    let (handle, join) = started.expect("the cluster starts");
    let runtime = tokio::runtime::Builder::new_current_thread().build().expect("a runtime");
    let frame = runtime.block_on(handle.step(1)).expect("the cluster runs a frame");
    drop(handle);
    join.shutdown().expect("a stop in order");
    let exit = worker.join().expect("the worker thread ends").expect("a stop in order");
    // The default world has one city of 2 by 2 zones.
    assert_eq!(exit.worlds.len(), 4, "the worker had every zone of the world");
    frame
}

#[test]
fn a_peer_that_proves_nothing_is_not_welcomed() {
    for with_secret in [false, true] {
        let (coordinator, address) = listen(with_secret.then(|| secret("the secret")));
        let starting = start(coordinator, Duration::from_millis(600));

        // The peer sends the `Hello` at once, and no proof.
        let mut peer = TcpStream::connect(address).expect("connect");
        write_frame(&mut peer, &hello(), MAX_FRAME).expect("the message leaves");
        let received = read_all(&mut peer);
        let mut input = received.as_slice();
        // The coordinator spoke first, with its challenge. Then it read
        // one frame, refused the peer and said no more.
        read_frame::<_, Challenge>(&mut input, MAX_FRAME).expect("a challenge");
        let (answer, _) = read_frame::<_, Admission>(&mut input, MAX_FRAME).expect("an answer");
        assert!(
            matches!(&answer, Admission::Refused { reason } if reason == NO_PROOF),
            "{answer:?}"
        );
        assert!(input.is_empty(), "the coordinator sent more: no `Welcome` must follow");
        drop(peer);

        // The slot stays free, and the start fails because nobody took it.
        let started = starting.join().expect("the coordinator thread ends");
        assert_eq!(started.err(), Some(ClusterError::JoinTimeout { missing: vec![0] }));
    }
}

#[test]
fn a_peer_with_a_made_up_proof_is_refused_before_its_hello_is_read() {
    let (coordinator, address) = listen(Some(secret("the secret")));
    let starting = start(coordinator, Duration::from_millis(600));

    let mut peer = TcpStream::connect(address).expect("connect");
    let (challenge, _) = read_frame::<_, Challenge>(&mut peer, MAX_FRAME).expect("a challenge");
    // The peer answers in the right form. It cannot compute the hash, so
    // it sends the challenge back as its proof. Its `Hello` follows in the
    // same moment.
    let guess = Proof { nonce: Hash([9; 32]), proof: challenge.nonce };
    write_frame(&mut peer, &guess, MAX_FRAME).expect("the proof leaves");
    write_frame(&mut peer, &hello(), MAX_FRAME).expect("the hello leaves");
    let received = read_all(&mut peer);
    let mut input = received.as_slice();
    let (answer, _) = read_frame::<_, Admission>(&mut input, MAX_FRAME).expect("an answer");
    assert!(
        matches!(&answer, Admission::Refused { reason } if reason == WRONG_SECRET),
        "{answer:?}"
    );
    // Nothing answers the `Hello`: the coordinator did not read it.
    assert!(read_frame::<_, ToWorker>(&mut input, MAX_FRAME).is_err());
    assert!(input.is_empty());
    drop(peer);
    let started = starting.join().expect("the coordinator thread ends");
    assert_eq!(started.err(), Some(ClusterError::JoinTimeout { missing: vec![0] }));
}

#[test]
fn a_worker_with_the_wrong_secret_is_refused_and_the_right_one_joins() {
    let (coordinator, address) = listen(Some(secret("the secret of the cluster")));
    let starting = start(coordinator, Duration::from_secs(60));
    let refused = Err(ClusterError::Refused { reason: WRONG_SECRET.to_string() });

    // Another secret, a secret that differs in one letter, and no secret.
    for wrong in [Some(secret("another secret")), Some(secret("the secret of the clusteR")), None] {
        let joined = Worker::connect(plugins(), worker(address, wrong)).map(|w| w.zones().len());
        assert_eq!(joined, refused);
    }

    // The slot is still free for the worker that knows the secret.
    let right = worker(address, None).with_secret(secret("the secret of the cluster"));
    let serving = std::thread::spawn(move || run_worker(plugins(), right));
    let started = starting.join().expect("the coordinator thread ends");
    assert_eq!(step_and_stop(started, serving), 1);
}

#[test]
fn a_cluster_without_a_secret_runs_on_a_loopback_address() {
    let (coordinator, address) = listen(None);
    let starting = start(coordinator, Duration::from_secs(60));

    // A worker with a secret does not fit a coordinator without one.
    let joined = Worker::connect(plugins(), worker(address, Some(secret("a secret"))))
        .map(|worker| worker.zones().len());
    assert_eq!(joined, Err(ClusterError::Refused { reason: WRONG_SECRET.to_string() }));

    let serving = std::thread::spawn(move || run_worker(plugins(), worker(address, None)));
    let started = starting.join().expect("the coordinator thread ends");
    assert_eq!(step_and_stop(started, serving), 1);
}

#[test]
fn without_a_secret_only_a_loopback_address_is_allowed() {
    let refusal = |address: &str| match Coordinator::bind(address) {
        Err(ClusterError::Config(message)) => message,
        other => panic!("`{address}` was not refused as a configuration: {other:?}"),
    };
    assert_eq!(
        refusal("0.0.0.0:0"),
        "0.0.0.0:0 is not a loopback address, and the cluster has no shared secret; every \
         process that reaches the address could join. Use `Coordinator::bind_with_secret`, or \
         listen on 127.0.0.1"
    );
    assert!(refusal("[::]:0").starts_with("[::]:0 is not a loopback address"));
    // An address of a documentation network. This machine does not have
    // it, so nothing could listen there anyway.
    assert!(refusal("203.0.113.9:0").starts_with("203.0.113.9:0 is not a loopback address"));

    // With a secret the rule does not refuse that address. The operating
    // system does, because the machine does not have it.
    let with_secret = Coordinator::bind_with_secret("203.0.113.9:0", secret("a secret"));
    assert!(matches!(with_secret, Err(ClusterError::Io(_))), "{with_secret:?}");

    // The loopback addresses work without a secret.
    for local in ["127.0.0.1:0", "[::1]:0"] {
        let coordinator = Coordinator::bind(local).expect("a loopback address is allowed");
        assert!(coordinator.local_addr().expect("the address").ip().is_loopback());
    }
}

#[test]
fn a_worker_without_a_secret_does_not_call_a_coordinator_that_is_not_local() {
    let far: SocketAddr = "203.0.113.9:7000".parse().expect("an address");
    let joined = Worker::connect(plugins(), worker(far, None)).map(|worker| worker.zones().len());
    assert_eq!(
        joined,
        Err(ClusterError::Config(
            "the coordinator at 203.0.113.9:7000 is not on a loopback address, and this worker \
             has no shared secret; give it the secret of the cluster"
                .into()
        ))
    );
}
