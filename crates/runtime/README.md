# The runtime

`unsup-runtime` drives a `World` in real time. It owns the simulation
thread, the input log, the snapshots, recovery, replay, time dilation and
rollback. The gateway and the spectator service reach the world only
through `SimHandle`. The runtime can also run one world as a cluster of
processes (section 14).

The contract is `docs/design/architecture.md`, section 6. This document
gives the formats, the rules and the limits of the implementation.

## 1. Parts

| Part | File | Job |
| --- | --- | --- |
| Simulation thread | `src/sim.rs` | Takes calls from the mailbox, records them, applies them, steps frames |
| Observations | `src/observe.rs`, `src/sim/observing.rs` | The one function that builds observations, and the batch of `observe` calls |
| Handle | `src/handle.rs` | The API for all other services |
| Mailbox types | `src/mailbox.rs` | The calls and the answers |
| Reply thread | `src/replier.rs` | Sends the answers of large batches |
| Waiting calls | `src/waiters.rs` | The `wait` calls, by resident and by deadline |
| Statistics | `src/stats.rs` | `SimStats` and its meters |
| Extension traits | `src/journal.rs` | `Journal` and `Pacer` |
| Storage | `src/storage/` | Named blobs and append-only segments |
| Input log | `src/log/` | The record format, the writer, the reader |
| Snapshots | `src/snapshot/` | The file format, the writer thread, the rotation |
| Durable journal | `src/durable.rs` | Log and snapshots together; rollback |
| Recovery and replay | `src/replay.rs` | From storage back to a world |
| Time dilation | `src/dilation.rs` | The pacer that makes slow zones skip frames |
| Outside messages | `src/external.rs` | A message from a service to the global realm |
| Cluster | `src/cluster/` | One world in several processes (section 14) |
| Numbers | `data/runtime.toml` | Every tunable number |

## 2. How to start a durable world

```rust
let storage = Arc::new(FsStorage::open("var/world")?);
let opened = DurableJournal::open(storage, plugins, DurableConfig::default(), |plugins| {
    World::new(plugins, world_config)
})?;
let config = RuntimeConfig {
    journal: Box::new(opened.journal),
    pacer: Box::new(Dilation::new(DilationConfig::default())),
    ..RuntimeConfig::default()
};
let (handle, join) = start(opened.world, config);
// ...
join.shutdown()?; // a clean stop: it writes the final snapshot
```

`DurableJournal::open` recovers the world when the directory holds one.
It calls the closure only when the directory is empty.

`RuntimeConfig::default()` has no journal and no time dilation. A server
that wants them must set them as shown.

## 3. Storage

The trait `Storage` has two kinds of data:

- A *blob* is written in one piece. The write is atomic: a reader sees the
  old blob or the new blob.
- A *segment* grows at its end. `Segment::sync` forces it to the disk.

`FsStorage` uses a directory of the local file system. It writes a blob
to a temporary file, forces the file to the disk, renames it over the
name and forces the directory to the disk. It removes temporary files of a
dead process when it opens the directory.

`FsStorage` holds a lock on the file `.lock` in its directory. A second
`FsStorage::open` for the same directory fails while the first value
lives. Two servers cannot write into one directory.

`MemStorage` keeps the data in memory, for tests. It knows which bytes
were forced to the disk. `fork` gives the disk after the process was
killed. `fork_durable` gives the disk after a loss of power. `set_fault`
makes appends, syncs or blob writes fail.

## 4. The input log

Every input (`Input::Spawn`, `Act`, `Step`, `Admin`, `Deliver`) goes to the
log before the world applies it. The simulation thread takes a batch of
calls from the mailbox, records their inputs, commits them and then
applies them.

### Files

The log is a row of segment files in the directory `log`. The name of a
segment is `<index>-<first sequence number>.seg`.

```text
header  = "UNSUPLOG" | version u32 | index u64 | first_seq u64 | check u32
record  = length u32 | check u32 | payload
payload = MessagePack with field names of { seq, input }
```

All numbers are little-endian. `check` is the first four bytes of the
blake3 hash of the bytes that it protects. A segment rolls when it is
larger than `log.segment_bytes`.

### One rule for every accident

Every input has a sequence number, from 0. A segment names the sequence
number of its first record. A new segment makes every record with the
same or a higher sequence number in an older segment void.

The log is never changed in place. The writer starts a new segment:

- when a segment is full;
- after a crash, at the first record that could not be read;
- after a write that failed, at the first input of that write;
- after a rollback, at the sequence number of the snapshot;
- after a panic of the simulation, at the input that caused it.

A reader accepts a record that it cannot read only when the record is
void or is at the end of the last segment. That is the torn record that a
crash leaves. The reader ignores it and reports it (`TornTail`). Damage
with a valid record behind it is an error (`LogError::Corrupt`). The
reader does not skip it, because that would lose inputs that the world
applied.

### Sync policy

`DurableConfig::sync` decides when the log is forced to the disk.

| Policy | When | A killed process loses | A loss of power loses |
| --- | --- | --- | --- |
| `EveryRecord` | After each input, before it is applied | Nothing | Nothing |
| `EveryBatch` | After each batch, before it is applied | Nothing | Nothing |
| `Interval(d)` | Every `d`, on another thread | Nothing | At most the inputs of the last `d` |

With every policy the operating system has the input before the world
applies it. The default is `Interval` with 50 ms.

With `EveryBatch`, one sync serves a whole batch. Raise
`RuntimeConfig::batch` to take more inputs per sync.

With `EveryRecord`, each input waits for its own sync. The simulation
thread looks for a due frame between two batches. Lower
`RuntimeConfig::batch` to a few calls, or the frames are late when the
mailbox is full.

### When a write fails

The journal refuses the input and the caller gets `SimError::Overloaded`.
The world does not apply an input that the log does not have. The journal
then starts a new segment, so that the bytes of the refused input do not
count in a recovery.

## 5. Snapshots

A snapshot is one blob in the directory `snapshots`. Its name is
`<sequence number>-<frame>.snap`.

```text
file   = header | payload
header = "UNSUPSNP" | version u32 | frame u64 | seq u64 | segment u64
       | state_hash [32] | payload_len u64 | payload_hash [32] | check u32
```

`payload` is the result of `World::snapshot`. `seq` is the number of
inputs that the world had applied. `state_hash` is `World::state_hash` at
that moment.

- The journal takes a snapshot every `snapshots.every_frames` frames.
- The journal writes the first snapshot when it makes a new world.
- A clean stop writes a final snapshot.
- The journal keeps the newest `snapshots.keep` snapshots. It deletes the
  older ones, and the log segments that only they need.

The simulation thread serializes the world and computes the state hash
in one pass over the state (`World::snapshot_with_hash`). A second thread
writes the file. That thread first forces the log to the
disk, so a snapshot is never newer than the log on the disk.

`SimStats` has the frame of the last snapshot, the time that the
simulation thread stood still for it, the time of the write and the size.

A host reads the snapshots of a storage with two functions of the module
`snapshot`. Both work while the world runs, and neither reads the state.

- `snapshot::list(storage)` returns the snapshot files, oldest first: the
  name, the sequence number and the frame of each. It reads only the
  names. A rollback can go to each of them.
- `snapshot::newest(storage)` returns the `SnapshotMeta` of the newest
  snapshot that a recovery or a rollback can use: the frame, the sequence
  number and the state hash. It reads the header of one file. It passes
  over a snapshot that a rollback made void and a file with a header that
  cannot be read.

## 6. Recovery

`recover(storage, plugins)` returns the world and a report.

1. It loads the newest snapshot that it can use. It passes over a snapshot
   that is damaged, that the plugins cannot restore, or that a rollback
   made void. The report names each one.
2. It applies every later input of the log.
3. It ignores a torn record at the end of the log. The report names it.

Recovery does not change the storage. `DurableJournal::open` does the
recovery and then starts a new segment.

Two cases need the operator:

- `RecoverError::Log(LogError::Corrupt)`: a record in the middle of the
  log is damaged.
- `RecoverError::Panic { seq }`: input `seq` makes the simulation panic.

In both cases, start again with `DurableConfig::drop_from = Some(seq)`.
The world then stops before that input. That input and all later inputs
leave the log for good.

A panic in a running world does not need this step. The runtime takes the
input that caused the panic out of the log before the thread stops.

## 7. Replay check

`replay(storage, plugins, options)` replays the log from a snapshot. It
reports the frames, the inputs and the final state hash. At each later
snapshot it compares the state hash of the replayed world with the hash
in the snapshot. `ReplayReport::is_consistent` is true when all hashes
match. A later snapshot that the replay cannot compare is in
`ReplayReport::skipped`.

The example `replay` is a small command line tool for it:

```sh
cargo run -p unsup-runtime --example replay
cargo run -p unsup-runtime --example replay -- record DIR [ROUNDS]
cargo run -p unsup-runtime --example replay -- check DIR [oldest|newest|FRAME]
```

A replay needs the plugins that made the world. The example knows only
the module of the tests of this crate. The server has all game modules;
its tool must call the same function.

## 8. Time dilation

`Dilation` is a `Pacer`. The kernel measures the tick time of each zone
with the clock that the runtime gives it (`World::set_clock`,
`World::timings`). Before each frame the pacer decides which zones skip
the frame.

- Each zone has a divisor `d`. The zone ticks in one frame of `d`.
- The budget of a zone tick is `dilation.budget_percent` of the frame
  interval. A zone above `d` budgets gets a higher divisor at once.
- All zones together must fit on the threads. When they do not, the
  heaviest zone gets a higher divisor.
- A divisor goes down one step at a time. It goes down only after
  `dilation.calm_ticks` measurements below `dilation.relax_percent` of the
  budget. This gap is the hysteresis.
- No divisor is above `dilation.max_divisor` (10). A zone never skips more
  than nine frames in a row.

The decision is the `skip` list of `Input::Step`. It is in the log, so a
replay skips the same zones in the same frames without a clock.

The budget comes from the frame interval. With manual stepping (a frame
interval of zero) there is no budget, and no zone skips. A host can set a
fixed budget in `DilationConfig::budget` instead.

## 9. Rollback

`SimHandle::rollback(frame)` puts the whole world back to the newest
snapshot at or before `frame`. It drops every later input from the log.
The rollback is permanent: a later recovery gives the world of the
snapshot plus the inputs after the rollback. Every waiting `wait` call
gets an answer. Viewers see the frame number go back.

**One zone cannot go back alone.** Money and goods cross zone borders. A
zone from the past and the rest of the world from the present would not
balance: value that left the zone would exist twice, and value that
entered it would be lost. The runtime refuses
`AdminCommand::RollbackZone` with `AdminReply::Error` and a message that
names the whole-world rollback. The refusal is not an input and is not in
the log.

The protocol has no admin command for the whole-world rollback. A service
calls `SimHandle::rollback`.

## 10. Load

- **Back-pressure.** The mailbox is bounded (`runtime.mailbox`). A call
  that finds it full returns `SimError::Overloaded` at once. It never
  blocks. Nothing changed, so the caller can try again later.
- **Waiting calls.** `wait` calls are stored by resident and by deadline.
  A frame looks only at the calls of the residents that got news. The
  cost of a frame does not grow with the number of calls that wait. At
  most `runtime.max_waiters` calls wait; one more gets `Overloaded`. A
  resident has at most `runtime.max_waiters_per_resident` waiting calls;
  one more ends the oldest one.
- **Observations.** An observation is the most expensive read. The
  simulation thread does not answer an `observe` call as it comes. It
  takes a batch of calls from the mailbox (`runtime.batch`), applies the
  inputs of the batch, and then builds the observations of all `observe`
  calls of the batch in one step. That step is one function:
  `observe_batch` in `src/observe.rs`. Every observation of the simulation
  thread comes from it, also that of a `wait` call; a `wait` call gets
  its observation on its own, when the call is answered. Today the function
  asks the world for one observation after the other. When the kernel
  builds a batch on all cores (`World::observe_batch`), only the body of
  that function changes.
- **What an observation shows.** The world after every input of the same
  batch, also after an input that came later than the `observe` call. The
  world had that state between the call and its answer. A caller that
  waits for the answer to its action before it asks for an observation
  always sees that action.
- **Answers.** A batch with `runtime.reply_handoff` answers or more goes
  to a reply thread.
- **Statistics.** `SimHandle::stats` returns `SimStats`.

| `SimStats` field | Meaning |
| --- | --- |
| `p50_frame_micros`, `p95_frame_micros`, `p99_frame_micros` | Frame time over the last `runtime.stats_window_frames` frames |
| `inputs_per_second` | Inputs applied in the last full window of `runtime.rate_window_ms` |
| `mailbox_depth`, `mailbox_capacity` | Calls in the mailbox now, and the limit |
| `overloaded` | Calls refused with `overloaded` |
| `waiting` | `wait` calls that wait now |
| `dilated_zones`, `zone_ticks_skipped` | Zones under time dilation now; zone ticks skipped since the start |
| `journal_records`, `journal_lag`, `journal_errors` | Inputs in the log; inputs not forced to the disk yet; inputs refused by the journal |
| `snapshots`, `snapshot_errors` | Snapshots written; snapshots that failed |
| `last_snapshot_frame`, `last_snapshot_micros`, `last_snapshot_write_micros`, `last_snapshot_bytes` | The last snapshot: frame, stall of the simulation thread, time of the write, size |
| `rollbacks` | Rollbacks since the start |
| `intake_micros`, `commit_micros`, `apply_micros`, `reply_micros` | Where the simulation thread spent its time |
| `observations`, `observe_batches`, `observe_micros` | Observations that the simulation thread built; the steps in which it built them; the time for them. The time for the `observe` calls is also in `apply_micros` |

The example `throughput` measures the runtime with many callers. It also
measures observations: alone, and one after each action. Each row names
the share of the time of the simulation thread that went to observations
and the number of observations in one step.

```sh
cargo run --release -p unsup-runtime --example throughput
```

## 11. Messages from outside

`SimHandle::post_global(&message)` sends a typed message from a service
outside the simulation to the global realm. The message type implements
`unsup_kernel::Message`. A module takes the message with
`Builder::on_message` in the global scope.

- The kernel makes the envelope (`Envelope::module`).
- The message becomes `Input::Deliver`. It is in the log, and a recovery
  or a replay delivers it again.
- The handler runs at the start of the next frame.
- The handler sees `RealmId::Global` as the sender. Give a message from
  outside a `KIND` of its own.
- `Ok` means that the world has the message. A module answers through an
  event or a notice.

The runtime refuses a message before it is an input. The world does not
change and the log does not get the message. `PostError` gives the reason:

| `PostError` | Reason |
| --- | --- |
| `Unreadable { kind, reason }` | The message cannot be written as MessagePack, or it does not read back |
| `NoHandler { kind }` | No module handles this kind in the global realm (`Blueprint::handles_message`) |
| `Sim(SimError::NotHosted)` | Another process hosts the global realm |
| `Sim(SimError::Overloaded)` | The mailbox is full, or the journal cannot record the message |
| `Sim(SimError::Shutdown)` | The simulation stopped |

`SimHandle::send_global` is the same call with the error as a `SimError`.
`Unreadable` and `NoHandler` become `SimError::Invalid`, and the log of the
server names the reason.

## 12. Numbers

All tunable numbers are in `data/runtime.toml`. The default values of
`RuntimeConfig`, `DurableConfig` and `DilationConfig` come from that file.
A host changes a number through the fields of those types.

## 13. Limits

- A refused input can come back in one case. The write of its record
  fails, the storage also refuses the new segment, and the process dies
  before the next write. A recovery then applies the record.
- An input that is larger than `log.max_record_bytes` gets
  `SimError::Overloaded`, not `SimError::Invalid`.
- The runtime checks the kind of an outside message, not its type. A
  message with the right kind and other fields gets into the log. The
  kernel then drops it in the global realm and counts it in
  `World::dropped_messages`.
- The simulation thread stands still while it serializes the world and
  computes the state hash. One pass over the whole state gives both.
- The simulation thread looks for a due frame only between two batches. A
  journal that is slow for each input (`EveryRecord`) makes frames late
  when the batch is large.
- `FsStorage::open` takes the lock of the directory. A tool that only
  reads, such as the replay check, cannot open the directory of a server
  that runs. Copy the directory first.
- `start` drives one world and does not send its outbox for other
  processes anywhere. A world that does not host all realms needs the
  cluster (section 14).
- The catalogue of `SimHandle::catalogue` is the one at the start. A
  rollback or a parameter change does not update it.

## 14. The cluster

A cluster runs one world in several processes. The state of every realm is
the state that one process reaches with the same inputs. So more
processes are a change of the deployment, not of the modules.

### Topology

```text
        gateway, spectator
               |  ClusterHandle (a SimHandle)
        +-------------+
        | coordinator |   global realm, frame clock, directory, router
        +-------------+
          |    |    |     TCP, one connection per worker
     +------+ +------+ +------+
     |worker| |worker| |worker|   each hosts a set of zones
     +------+ +------+ +------+
```

- The **coordinator** hosts the global realm. It owns the frame clock and
  the mailbox of the handle. It routes every message between two realms.
- A **worker** hosts a set of zones. `ClusterConfig::workers` gives each
  worker slot its zones; `partition` splits the zones in rows of numbers.
- Workers talk only to the coordinator. A message from a zone of one
  worker to a zone of the same worker also goes through the coordinator.
- Each realm has a kernel `World` of its own, which hosts only that realm.
  The kernel routes a message between two realms of one world by itself,
  in an order that depends on the set of realms in that world. With one
  realm per world, every message leaves through `World::take_external`,
  and one router gives every realm its messages in the order of the single
  process.

| Part | File | Job |
| --- | --- | --- |
| Coordinator | `src/cluster/coordinator/` | Start, mailbox loop, frames, inputs, reads |
| Worker | `src/cluster/worker.rs`, `session.rs` | Handshake, and the loop that answers the coordinator |
| One realm | `src/cluster/cell.rs` | A world with one realm, its journal and its seals |
| Router | `src/cluster/plan.rs` | Which zone gets which message, in which order |
| Directory | `src/cluster/lookup.rs` | In which zone a resident is |
| Transport | `src/cluster/wire.rs`, `proto.rs`, `link.rs` | Frames, messages, the connection to a worker |
| Seals | `src/cluster/seals.rs` | The points to which the cluster can recover |
| Handle | `src/cluster/handle.rs` | `ClusterHandle` and `ClusterJoin` |

### How to start

```rust
let coordinator = Coordinator::bind("127.0.0.1:7000")?;
// In each worker process:
//   run_worker(plugins, WorkerConfig::new(world_config, slot, address))?;
let config = ClusterConfig::real_time(world_config, 3, Duration::from_secs(1));
let (handle, join) = coordinator.start(plugins, config)?;
// `handle` has every method of `SimHandle`. `handle.sim()` is a `SimHandle`.
join.shutdown()?; // a stop in order: every process writes its final snapshots
```

`Coordinator::start` returns when every worker has joined and every realm
is ready. A worker can start before the coordinator: it tries to connect
until `WorkerConfig::connect_timeout` has passed.

A `ClusterHandle` is a `SimHandle` with more calls: `cluster_stats`,
`hashes`, `snapshots` and `audit`. The coordinator sends each call to the
process that hosts the resident or the realm. `audit` checks the ledgers
of every world and the union of all processes: nothing is created or lost,
also not on the way between two processes.

### Transport and handshake

A connection carries frames. A frame is a length of four bytes
(little-endian) and a payload. The payload is one protocol message as
MessagePack with field names. A frame larger than
`cluster.max_frame_bytes` stops the cluster. A kernel envelope travels as
the bytes of `Envelope::to_bytes`.

The coordinator asks and the worker answers: each message of the
coordinator has exactly one answer. The handshake:

1. The worker sends `Hello`: the protocol version, the hash of the world
   configuration, the hash of the set of plugins, and its slot.
2. The coordinator checks them. It answers `Welcome` with the zones of the
   slot, or `Refuse` with the reason. The reason says which of the four
   is wrong.
3. The worker sends the seals that the storage of each zone holds.
4. When all workers have joined, the coordinator sends `Begin`: make new
   worlds, or recover to one seal.
5. The worker makes or recovers its worlds and sends `Ready`.

The hash of the world configuration covers every field but `hosted`. The
hash of the set of plugins covers the plugin names in their order and the
catalogue: verbs with argument schemas, parameters with values, items,
attributes and conditions.

### The frame protocol

One process runs a frame in three steps (`World::apply` in the kernel).
The cluster runs the same three steps. The coordinator waits for every
answer of a step before it starts the next step.

1. **Zones.** The coordinator sends `Step` with the list of zones that
   skip the frame. Each worker ticks its zones in parallel. It answers
   with the viewer output of each zone, the tick time of each zone and
   every message that each zone sent.
2. **Global realm.** The coordinator gives the global realm the messages
   of the zones for it, in zone order. Then the global realm ticks.
3. **Exchange.** The coordinator makes one batch for each zone:
   - first the residents that arrive, in the order of the zones that they
     left;
   - then the messages of the global realm, in sending order;
   - then the messages of the zones, in zone order, each zone in sending
     order.

   A message for a resident goes to the zone in which the resident is
   after the arrivals. Each worker gives its zones their batches and
   answers. Then every realm writes a seal.

The order of the messages comes from the zone numbers and the sending
order. It does not come from the time at which a worker answers. So the
result does not depend on timing, and it does not depend on the split of
the zones over the workers.

Time dilation works as in one process. The workers measure the zone tick
times. The pacer of the coordinator (`ClusterConfig::pacer`) decides the
`skip` list, and every realm records it in its log.

### Inputs between frames

The coordinator applies one input at a time. It sends the input to the
world of the right realm and waits for the answer. Then it gives every
message that the input made to its target, one after the other. The
caller gets its answer after that. So all worlds see inputs and their
messages in one order, as one process does.

- A spawn goes to the world of the spawn zone (`ClusterConfig::spawn_zone`).
- An action and a command for a resident go to the zone of the resident.
- A parameter change and a message from outside go to the global realm.
- An observation comes from the zone of the resident.
- The layout view comes from the coordinator. A zone view comes from the
  worker of the zone.
- The coordinator keeps the `wait` calls. After a frame it asks each
  worker once for the observations of the residents that got news.

### The directory

The kernel keeps the directory of residents in the global realm. Zones
tell it about each spawn, migration and despawn. It is exact after the
tick of the global realm. Between two frames, the coordinator adds what
the inputs since that tick did: the worker of a zone reports each resident
that arrives.

The kernel of each world also decides if a resident exists. A world knows
a resident when the resident is in it or has passed through it. So the
coordinator passes every resident that migrates through the world of the
global realm on the way to its new zone. That world then knows every
resident that has changed zone, and the global realm can answer them.

A message for a resident that left the world goes to the world of the zone
in which it left. The kernel there applies its rule for a resident that is
gone: a wire goes to the unclaimed account of the global realm, and
everything else is dropped.

### Failure rules

A cluster never goes on without one of its processes.

- The coordinator sees that a worker is gone when the connection closes,
  when a write fails, or when an answer does not come in
  `ClusterConfig::reply_timeout`. A thread reads each connection, so the
  coordinator also sees it while nobody calls the cluster.
- The coordinator then tells every other worker to stop, with the reason.
  It runs no more frames. Every call fails with `SimError::Shutdown`, and
  the stream of frames ends. `ClusterJoin::shutdown` returns the error,
  for example `ClusterError::WorkerLost { slot, reason }`.
- A worker stops when the coordinator tells it to, and when its connection
  closes. `run_worker` returns the error.
- A panic in a zone stops the worker of the zone. The worker first takes
  the input that caused the panic out of its log and tells the coordinator
  the zone and the panic message (`ClusterError::Failed`).
- A journal that cannot record an input of a resident refuses that input:
  the caller gets `SimError::Overloaded` and nothing changes. A journal
  that cannot record a frame or a message between realms stops the
  cluster. One realm cannot refuse a frame that the others ran.
- A message that the frame protocol does not allow stops the cluster
  (`ClusterError::Protocol`).

### Reconnect rules

- Before the cluster runs, a worker tries to connect until its time limit.
- A connection that fails the handshake is refused. Its slot stays free
  for the right worker. A second worker for a slot that is taken is
  refused.
- When a slot is still free after `ClusterConfig::join_timeout`, the start
  fails. The workers that joined stop.
- The coordinator closes its socket when the cluster runs. No worker joins
  a cluster that runs.
- A connection that breaks is final. A process cannot connect again and
  go on. Start every process again; the cluster then recovers.

### Input log, snapshots and recovery

Each process has one storage (`ClusterDurable`). Each realm of the process
has a directory in it: `global`, `zone-00000`, `zone-00001` and so on. A
directory holds the input log and the snapshots of that realm, in the
formats of sections 4 and 5, and the file `seals`. Every message that a
realm gets from another realm is an input in its log.

A **seal** is a point at which every realm had taken every message that
was sent to it. The coordinator numbers the seals. Each realm writes down,
for each seal, the number of inputs in its log at that point.

```text
file   = "UNSUPSEA" | version u32 | record*
record = epoch u64 | frame u64 | seq u64 | check u32
```

- Seal 0 is the start of a new world, after world generation.
- Every frame ends with a seal.
- A stop in order writes one more seal when inputs came after the last
  frame. So a stop in order loses nothing.
- A snapshot is taken at a seal.
- A realm keeps the newest `cluster.seals_keep` seals.

When the processes die, they stop at different inputs. At the next start
each process reports the seals that it can recover to: the log must hold
every input before the seal, and a snapshot at or before it must exist.
The coordinator takes the newest seal that every realm has. Every realm
goes back to that seal: it restores a snapshot, replays its log up to the
seal and drops the later inputs for good. The whole cluster is then at one
frame.

- Inputs after the last seal are lost when a process dies: at most the
  inputs since the last frame. A stop in order loses none.
- The start fails when no seal is common to all realms, and when a realm
  has a world that ran frames but no seal that can be used. The cluster
  does not start a new world next to an old one. The error names the
  realm.
- A start that was cut off before seal 0 was in every realm starts again
  as a new world. No input came before that seal.

### Numbers

`SimHandle::stats` works for a cluster. Frame times are those of a whole
cluster frame. Journal numbers are sums over all realms. `ClusterStats`
has the numbers that only a cluster has:

| `ClusterStats` field | Meaning |
| --- | --- |
| `frame`, `epoch` | The last frame and the last seal |
| `workers` | Per worker: zones, residents, bytes sent and received, time of the last step |
| `migrations`, `messages` | Residents that changed zone, and messages that the coordinator routed |
| `dropped_messages` | Messages that the kernel dropped in any world |
| `last_zones_micros`, `last_global_micros`, `last_exchange_micros` | The three steps of the last frame |
| `zones_micros`, `global_micros`, `exchange_micros` | The same, summed over all frames |

### The example

The example `cluster` runs one load in one process and then in a cluster.
The coordinator is the example process. Each worker is a child process.
The load is a crowd that walks across the zone borders.

```sh
cargo run --release -p unsup-runtime --example cluster
cargo run --release -p unsup-runtime --example cluster -- WORKERS RESIDENTS FRAMES DISTRICTS ZONE_SIZE
cargo run --release -p unsup-runtime --example cluster -- crash
```

It prints the frame time and the input rate of both runs and the time of
each step of a cluster frame. Then it compares the state of every realm of
both runs. `crash` kills one worker process and shows that the cluster
stops.

All processes of the example share the processors of one machine. One
process already ticks its zones in parallel on those processors. So the
example cannot show that a cluster is faster. It shows that the cluster
gives the same world, and what the protocol costs for each frame and each
input.

### Limits

The first six limits come from the kernel, which this crate must not
change. The tests in `tests/cluster_limits.rs` hold each of them in place:
each test shows what one process does and what the cluster does instead.

- **A world treats a resident that it has never seen as gone.** The kernel
  of the sender drops a notice or a module message for such a resident
  and sends a wire to the unclaimed account. The coordinator does not get
  the message. So:
  - A zone can reach a living resident only when the resident is in the
    zone or has been in it.
  - The global realm can reach a living resident only after the resident
    has changed zone one time. Before that, a verb of the global realm
    gets no answer and ends with its timeout.

  `ClusterStats::dropped_messages` counts these messages.
- **One world gives out the resident ids.** So every resident is made in
  the spawn zone. A spawn in a city that the spawn zone is not in fails
  with `SimError::NotHosted`. A spawn without a city goes to the city of
  the spawn zone, not to the emptiest city. Only spawn sites of the spawn
  zone are used.
- **A zone gets its own messages first.** The world of a zone gives a
  message of the zone to itself (`Dest::Zone` of its own number, its copy
  of `Dest::AllZones`) before the coordinator brings the messages of the
  global realm and of the lower zones. One process gives them in zone
  order. The inbox order of that zone can differ in such a frame.
- **An observation has no section of the global realm.** The kernel builds
  those sections only in a world that hosts the zone of the resident.
- **A viewer sees a resident enter a zone one frame after it left the
  other zone.** The world of a zone gives its viewer output only when it
  runs a frame, and the resident arrives after that. The state is not
  affected.
- The state bytes of a realm differ from one process while the realm holds
  a message of another realm in its inbox. The kernel marks such a
  message. The realm takes the message in its next tick, and the bytes are
  the same again. `ClusterHandle::hashes` gives the hashes of worlds with
  one realm each. They compare between clusters, not with a world of one
  process.
- The handshake cannot see a change in the code of a plugin that does not
  change the catalogue, for example another number in the code of a
  system.
- The coordinator applies one input at a time and waits for its answer.
  The input rate is at most one round trip per input. Every message
  between two zones passes through the coordinator.
- `SimHandle::rollback` answers `RollbackError::Unsupported` for a
  cluster.
- Each realm has a journal of its own: with `DurableJournal`, one snapshot
  thread and, with the `Interval` policy, one sync thread per realm.
- A process cannot join again after its connection broke. All processes
  start again.
- The transport has no encryption and no login. Use it on one machine or
  on a private network.
