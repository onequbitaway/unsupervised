# The runtime

`unsup-runtime` drives a `World` in real time. It owns the simulation
thread, the input log, the snapshots, recovery, replay, time dilation and
rollback. The gateway and the spectator service reach the world only
through `SimHandle`.

The contract is `docs/design/architecture.md`, section 6. This document
gives the formats, the rules and the limits of the implementation.

## 1. Parts

| Part | File | Job |
| --- | --- | --- |
| Simulation thread | `src/sim.rs` | Takes calls from the mailbox, records them, applies them, steps frames |
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

The simulation thread serializes the world and computes the state hash.
A second thread writes the file. That thread first forces the log to the
disk, so a snapshot is never newer than the log on the disk.

`SimStats` has the frame of the last snapshot, the time that the
simulation thread stood still for it, the time of the write and the size.

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

The example `throughput` measures the runtime with many callers:

```sh
cargo run --release -p unsup-runtime --example throughput
```

## 11. Messages from outside

`SimHandle::send_global(&message)` sends a typed message from a service
outside the simulation to the global realm. The message type implements
`unsup_kernel::Message`. A module takes the message with
`Builder::on_message` in the global scope.

- The message becomes `Input::Deliver`. It is in the log, and a recovery
  or a replay delivers it again.
- The handler runs at the start of the next frame.
- The handler sees `RealmId::Global` as the sender. Give a message from
  outside a `KIND` of its own.
- Use the message type of the module itself. A handler panics on a payload
  that it cannot read.
- `Ok` means that the world has the message. A module answers through an
  event or a notice.

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
- The runtime cannot check that the type of an outside message is the
  type that the module registered. The kernel has no public way to ask.
- A message that no module takes is dropped by the global realm. The call
  still returns `Ok`.
- The simulation thread stands still while it serializes the world and
  computes the state hash. Both walk the whole state.
- The simulation thread looks for a due frame only between two batches. A
  journal that is slow for each input (`EveryRecord`) makes frames late
  when the batch is large.
- `FsStorage::open` takes the lock of the directory. A tool that only
  reads, such as the replay check, cannot open the directory of a server
  that runs. Copy the directory first.
- A world that does not host all realms has an outbox for other
  processes. The runtime does not send that outbox anywhere.
- The catalogue of `SimHandle::catalogue` is the one at the start. A
  rollback or a parameter change does not update it.
