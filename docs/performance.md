# Performance

This document tells you how to measure the performance of Unsupervised, what
each number means, and what was measured. A number that is not in section 8
was not measured.

## 1. Targets

| Target | Value |
| --- | --- |
| Residents at the same time | 1,000,000 |
| Actions for each second | 200,000 |
| Acknowledgement latency, 99th percentile | below 250 ms |

A target is met only when a measurement shows it. Section 9 gives the state
of each target.

## 2. The tool

The tool is the binary `unsup-loadtest` in `crates/loadtest`. It has three
commands.

| Command | What it does | Network |
| --- | --- | --- |
| `bench` | Measures frame time, action rate and memory for a world of a given size | No |
| `soak` | Runs many ticks and checks conservation and determinism | No |
| `swarm` | Runs many virtual agents against a server and measures the acknowledgement latency | Yes |

`bench` and `soak` build the world with `unsup_server::build_world`. `swarm`
with `--spawn-server` starts the server with `unsup_server::start`. Thus all
three commands run the full game that the server runs.

Always use a release build. A debug build is many times slower, and the
summary then says `DEBUG BUILD (numbers are not valid)`.

```sh
cargo run --release -p unsup-loadtest -- <command> [options]
cargo run --release -p unsup-loadtest -- <command> --help
```

### The report

Each command prints a summary and writes one JSON file. The file has the
same head for all commands.

| Field | Content |
| --- | --- |
| `tool`, `version` | Name and version of the tool |
| `command`, `argv` | The command and its arguments |
| `started_at` | Start time in UTC |
| `machine` | Processor, logical cores, zone threads, memory, system, build kind, load average at the start |
| `load_average_end` | Load average of the last minute at the end |
| `commit` | Short hash of `HEAD`, and `dirty` when the tree has local changes |
| `result` | The numbers of the command |

- The default path is `var/loadtest/<command>-<time>.json`. Git ignores `var/`.
- Use `--out FILE` to set the path.
- The load average is the mean number of threads that wanted a processor.
  When it is near or above the number of logical cores, other programs used
  the machine too. The measured times are then too high.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command finished and found no problem |
| 1 | The command failed, or `soak` found a violation, or a size of `bench` did not finish, or `swarm` reached a limit of the machine, or `swarm --require-sustained` was not sustained |
| 2 | The command line is not correct |

## 3. Behaviour profiles

A behaviour profile tells the tool what the residents do. A profile is a
TOML file. All three commands read the same profiles.

```toml
name = "walkers"
summary = "Residents walk to random tiles near them."
observe_share = 0.25      # chance of an observation after an action
wait_share = 0.5          # swarm only: chance to wait for the end of an action

[think]                   # bench and soak: ticks between two actions
kind = "uniform"          # constant { ticks }, uniform { min, max },
min = 0                   # or exponential { mean, max }
max = 10

[[action]]
verb = "move_to"
weight = 9
args = { gen = "near_tile", radius = 12 }

[[action]]
verb = "wait"
weight = 1
args = { ticks = { gen = "int", min = 1, max = 5 } }
```

- The tool chooses an action by weight. An action with weight 9 is chosen
  nine times as often as an action with weight 1.
- The tool reads the verbs of the world from the catalogue at the start. It
  skips each verb that the world does not have, and it prints one warning
  for each. The weights of the other actions keep their ratio.
- The tool compares the argument names with the schema of the verb. It
  prints a warning for an argument that the verb needs and the profile does
  not give, and for an argument that the verb does not have.
- The command stops when the world has none of the verbs of the profile.

### Argument generators

In `args`, a table with the key `gen` is a generator. All other values are
used as they are written. A generator can be anywhere in a table or a list.

| Generator | Value |
| --- | --- |
| `{ gen = "near_tile", radius = N }` | `{ x, y }`: a random tile within N tiles of the resident, inside its city |
| `{ gen = "near_x", radius = N }`, `{ gen = "near_y", radius = N }` | One coordinate of such a tile |
| `{ gen = "nearby_resident", radius = N }` | The id of a random other resident within N tiles |
| `{ gen = "nearby_site", radius = N, kind = "shop" }` | The id of a random site within N tiles. `kind` is optional |
| `{ gen = "site_tile", radius = N, kind = "shop" }` | `{ x, y }`: the nearest tile of such a site |
| `{ gen = "item", tag = "food", as = "name" }` | A random item of the catalogue. `tag` is optional. `as` is `name` or `id` |
| `{ gen = "int", min = A, max = B }` | A whole number from A to B |
| `{ gen = "choice", of = [...] }` | One of the listed values |
| `{ gen = "text", words = N }` | A sentence of N words |
| `{ gen = "self" }` | The id of the resident |

When a generator cannot make a value (for example, no resident is near), the
resident chooses a different action. After four tries it does nothing in
this turn.

### Built-in profiles

The built-in profiles are in `crates/loadtest/profiles`. Give `--profile` the
name of a built-in profile or the path of a file.

| Name | What the residents do | Actions for each resident |
| --- | --- | --- |
| `walkers` (default) | Walk to a random tile within 12 tiles, think 0 to 10 ticks, sometimes stand still | About one in 13 ticks |
| `rush` | Walk to a random tile within 3 tiles with almost no think time. A stress load | About one in 3.3 ticks |
| `daily_life` | Walk, talk, eat, rest, work, trade, use the bank and read missions | Depends on the verbs that the world has |

The world of this worktree has four verbs: `wait`, `cancel`, `set_routines`
and `move_to`. Thus `daily_life` uses only `move_to` and `wait` now, and it
prints a warning for each of the other 16 verbs. The argument names of those
16 verbs are not yet fixed. When a verb arrives with different argument
names, the tool prints a warning that names the argument. Change
`daily_life.toml` then.

## 4. `bench`: frame time, action rate and memory

`bench` builds a world, creates the residents, and steps the world frame by
frame. Before each frame, the residents whose turn has come choose an action
from the profile. No network is used.

```sh
cargo run --release -p unsup-loadtest -- bench --residents 100000
cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000
cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 100 --profile rush
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--residents N` | 100,000 | Number of residents. `10k` and `1M` are also correct |
| `--scale A,B,C` | none | Several sizes in sequence. Each size runs in a process of its own |
| `--frames N` | 300 | Frames that are measured |
| `--warmup N` | 60 | Frames that run before the measurement |
| `--profile NAME` | `walkers` | Behaviour profile |
| `--cities N`, `--districts N` | automatic | Size of the world. Without them the tool chooses about 2,000 residents for each zone |
| `--placement scatter` or `spawn` | `scatter` | Where the residents start. See below |
| `--seed N` | 1 | Seed of the world and of the choices of the residents |
| `--request-ids` | off | Send an idempotency key with each request |
| `--data FILE` | none | A TOML file that replaces numbers of the game, in the format of a module's `data/balance.toml` |
| `--max-memory-gib X` | automatic | Stop the run when the process holds more memory than this |
| `--time-limit-secs N` | none | With `--scale`: stop a size that takes longer |

- `scatter` moves each resident to a random tile of its city before the run.
  This is the state of a world that has run for some time.
- `spawn` leaves each resident where the game puts a new resident. The world
  of this worktree has no spawn sites, so all residents of a city then start
  on one tile.
- A size of `--scale` that fails does not stop the other sizes. The summary
  shows `NOT MEASURED` and the reason for that size.

### What the numbers mean

| Number | Meaning |
| --- | --- |
| `frame` | Time of one `World::step`: all zones tick in parallel, then the global realm, then the router |
| `inputs` | Time to apply the action requests of one frame through `World::apply`, on one thread |
| `observe` | Time to build the observations of one frame through `World::observe`, on one thread |
| `server` | `inputs + observe + frame`: the work of the simulation thread of a server for one frame |
| `generator` | Time that the tool itself needs to choose actions. A server does not do this work |
| mean, p50, p99, worst | Mean, median, 99th percentile and largest value over the measured frames. The percentiles are exact (nearest rank) |
| actions accepted, rejected | Requests that the world accepted or rejected in the measured frames |
| actions for each frame | Accepted actions divided by the measured frames |
| actions for each second of wall time | Accepted actions divided by the wall time of the measured loop. The loop includes the time of the generator |
| actions for each second of server work | Accepted actions divided by the sum of `server` |
| moves, zone changes | Position changes and residents that changed zone, for each frame |
| RSS | Resident set size of the process: the memory that is in physical memory now |
| footprint | RSS plus the pages that the system compressed or swapped. macOS: `phys_footprint`. Linux: `VmRSS + VmSwap` |
| bytes for each resident | `(value at the end - RSS before the world) / residents`, for RSS and for the footprint |
| kernel estimate | `World::memory_estimate`: the resident tables only |
| inboxes | How full the inboxes of a sample of residents are. An inbox keeps the last `kernel.inbox_max` notices and reports |

Read these notes before you use the numbers.

- The memory of a world grows until the inboxes of the residents are full.
  When the summary says that the inboxes are less than 95 % full, the memory
  was still growing. Use more warm-up frames to measure the memory of a
  world that has run for a long time.
- When the system has too little free memory, it compresses or swaps pages
  of the process. The RSS is then too low and the times are too high. The
  summary then shows `WARNING: the system compressed or swapped ...` and
  `UNDER MEMORY PRESSURE`. Do not use the times of such a run.
- The run stops when the process holds more memory than the limit. The
  default limit is the smaller of 80 % of the installed memory and 90 % of
  the memory that was free at the start. This protects the machine.
- `inputs` and `observe` run on one thread, as they do in the server. `frame`
  uses one thread for each logical core.
- One tick is one frame. A server steps one frame for each second by
  default. The last line of the summary gives the share of that second that
  the slowest frame used.

## 5. `soak`: conservation and determinism

`soak` steps the world tick by tick with a behaviour profile. It stops at the
first violation.

```sh
cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000
cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --verify-determinism
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--residents N` | 2,000 | Number of residents |
| `--ticks N` | 2,000 | Number of ticks |
| `--audit-every K` | 1 | Ticks between two audits of the whole world. The last tick is always audited |
| `--verify-determinism` | off | Run a second world with the same seed and the same inputs, and compare the state hashes |
| `--hash-every K` | 50 | Ticks between two comparisons of the state hash |

`soak` also takes `--profile`, `--seed`, `--cities`, `--districts`,
`--placement`, `--request-ids` and `--data`, with the same meaning as for
`bench`.

| Check | What fails | Meaning |
| --- | --- | --- |
| Conservation | `World::audit` | Money or goods were created or lost outside a faucet or a sink, in a ledger or between two realms |
| Determinism | The state hashes of the two worlds are different | The state depends on something that is not the seed or the inputs |
| Panic | The kernel or a module panicked | A bug. The self-audit of a realm reports a broken ledger in this way |

- When a check fails, the summary shows `result: FAILED` with the tick, the
  check and the message. The exit code is 1.
- The tick of a conservation violation is the tick of the first audit that
  failed. With `--audit-every 1` it is the tick of the violation.
- The tick of a determinism violation is the tick of the first comparison
  that failed. The message gives the last tick at which the two worlds were
  equal. Use `--hash-every 1` to find the exact tick.

| Number | Meaning |
| --- | --- |
| ticks | Ticks that ran, of the ticks that the command asked for |
| ticks for each second | Ticks divided by the wall time. With `--verify-determinism`, two worlds run, so the value is about half |
| audits passed | Audits of the whole world that found no error |
| comparisons passed | State hash comparisons that found the two worlds equal |
| money at the end | `held + on the way = minted - burned`, in cents |
| dropped messages | Messages without a target that the world dropped |
| state hash | blake3 hash of the state after the last tick |

## 6. `swarm`: load over the network

`swarm` starts N virtual agents. Each agent is one task. Each agent signs up,
then it thinks, observes, acts and sometimes waits, until the run ends.

```sh
cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents 1000
cargo run --release -p unsup-loadtest -- swarm --url http://127.0.0.1:8080 --agents 1000 --duration 60
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--url URL` | none | Address of a server that runs |
| `--spawn-server` | off | Start the server of the project in this process on a free port |
| `--agents N` | 100 | Number of agents |
| `--ramp-up S` | 10 | Seconds in which the agents start, one after the other |
| `--duration S` | 60 | Seconds of the measurement window after the ramp-up |
| `--think KIND` | `lognormal` | Think time between two requests: `constant`, `exponential` or `lognormal` |
| `--think-mean S` | 5 | Mean think time in seconds |
| `--think-sigma X` | 1 | Shape of the log-normal think time |
| `--ws-share X` | 0.5 | Share of the agents that use the WebSocket, from 0 to 1. The other agents send HTTP requests |
| `--http-connections per-agent` or `shared` | `per-agent` | `per-agent`: each HTTP agent keeps one connection open. `shared`: all HTTP agents use one pool |
| `--profile NAME` | `walkers` | Behaviour profile |
| `--credentials FILE` | none | Use the keys in the file again, and write the keys of this run to it. Keep the file private |
| `--request-timeout S` | 30 | Seconds after which a request has failed |
| `--tick-ms N` | 1000 | With `--spawn-server`: real milliseconds for one game minute |
| `--cities N`, `--districts N` | automatic | With `--spawn-server`: size of the world |
| `--p99-target-ms X` | 250 | Target for the 99th percentile of the acknowledgement latency |
| `--max-error-share X` | 0.001 | Largest share of failed requests of a sustained run |
| `--require-sustained` | off | Make the exit code 1 when the load was not sustained |
| `--stats-path PATH` | `/v1/stats` | Route with the statistics of the server |

### What an agent does

1. The agent signs up. It tries the flow of `docs/design/protocol.md`
   section 2 first: `POST /v1/owners`, then `POST /v1/residents` with the
   owner token. When the server does not have these routes, the agent uses
   `POST /v1/dev/residents`, where the key is the resident id. With
   `--credentials`, the agent uses its stored key and does not sign up. When
   the server refuses a stored key, the agent signs up again.
2. The agent connects. A WebSocket agent opens `GET /v1/ws`. When the server
   does not have that route, the agent uses HTTP requests, and the summary
   counts it.
3. The agent observes once. It is then *ready*.
4. The agent thinks for a time from the think time distribution. The
   log-normal distribution imitates an agent that a language model drives:
   most answers come fast and some take a long time.
5. The agent observes when its view is old, or with the chance
   `observe_share` of the profile.
6. The agent chooses an action from the profile and sends it. The tool
   measures the acknowledgement latency.
7. When the action takes time, the agent waits for its end with the chance
   `wait_share` of the profile. An HTTP agent uses the long poll
   (`GET /v1/wait`). A WebSocket agent reads the report that the server
   pushes. When the server has no long poll, the agent observes again and
   again.
8. After a failed request, the agent waits one second and tries again. After
   five failed requests in a row, the agent gives up.

This build sends plain HTTP and plain WebSocket only. It refuses an
`https://` address with a clear message. For a server behind TLS, run the
swarm on the same machine as the server, or through a local proxy.

### Machine limits

Each agent with a connection of its own needs one open file and one local
port. A server in the same process needs one more open file for each
connection.

- Before the run starts, the tool compares these needs with the limits of
  the system. When the run does not fit, the tool stops and tells you what
  to change.
- During the run, the tool stops at the first failure that shows a limit of
  the machine: no free local port (`no_local_port`) or too many open files
  (`too_many_open_files`). The summary then gives the limit, the connections
  that were open, and what to change. The exit code is 1. Such a run says
  nothing about the server.

What you can change:

- Raise the limit of open files with `ulimit -n N` in the same shell.
- Use `--http-connections shared` when the machine has too few local ports.
  One client address can hold only as many connections to one server address
  as the system has local ports (16,384 on macOS, about 28,000 on Linux).
  Use `--ws-share 0` too, because each WebSocket agent needs a port.
- Wait one minute between two large runs. A local port stays in use for
  about 30 seconds after its connection closes.
- Start the agents from more than one machine for a larger load.

The summary shows the limit of open files, the open files of the process at
the peak, and the number of WebSocket connections that were really open.

### What the numbers mean

All latencies and rates are those of the measurement window. The window
starts after the ramp-up.

| Number | Meaning |
| --- | --- |
| ack latency | HTTP: time from the send of `POST /v1/act` to the last byte of the answer. WebSocket: time from the send of the `act` frame to the `ack` frame |
| p50, p95, p99, max | Percentiles from a histogram with three significant digits |
| observe latency | Time of one observation |
| acknowledgements for each second | Acknowledgements in the window divided by its length. The summary also gives the least and the most in one second |
| acknowledgements by state | `started`, `done`, `queued`, `pending`, `rejected` |
| rejected by the game | Rejections by error code. A rejection is a correct answer, not a failure of the server |
| failed requests | Requests without a usable answer, by code: the `error.code` of the server (for example `overloaded`), `http_<status>`, `timeout`, `connect`, `transport`, `closed`, `decode`. The summary shows the message of the first failure of each code |
| error share | Failed requests divided by all requests in the window |
| agents ready | Agents that signed up, connected and got their first observation |
| agents gave up | Agents that stopped after five failed requests in a row, by the code of the last error |
| WebSockets open at the peak | Largest number of WebSocket connections that were open at the same time. This number is exact |
| open files at the peak | Open files of the process, sampled each second |
| server | With `--spawn-server`: residents, frames in the window, mean and worst frame time, inputs that the simulation applied. With `--url`: the answer of the statistics route at the start and at the end of the window, in the report file |

The result is `SUSTAINED` when all of these are true:

- every agent became ready, and no agent gave up;
- at least one action was acknowledged in the window;
- the error share is at or below `--max-error-share`;
- the 99th percentile of the acknowledgement latency is at or below
  `--p99-target-ms`.

If not, the result is `NOT SUSTAINED` and the summary gives each reason.

## 7. How to find the largest load

1. Run `swarm` with a number of agents and `--duration 60`.
2. When the result is `SUSTAINED`, increase the number of agents.
3. When the result is `NOT SUSTAINED`, read the reasons. Decrease the number
   of agents.
4. Record the largest number that was `SUSTAINED`, with the report file.

Read these notes before you use the numbers.

- The swarm and a server from `--spawn-server` use the same processors. For
  the capacity of a server alone, start the server on one machine and the
  swarm on a different machine.
- The server puts a new resident on a spawn site of its city. The world of
  this worktree has no spawn sites, so all residents of a city start on one
  tile. The cost of an observation grows with the number of residents in
  sight. With many agents in one city, the result then shows this crowd and
  not the capacity of the server. Use `--districts 1 --cities N` with about
  200 agents for each city to spread the residents.

## 8. Results

This section has only numbers that were measured in this worktree. All other
numbers are **not measured**.

### Conditions

| Item | Value |
| --- | --- |
| Date | 2026-11-18 (UTC) |
| Machine | Apple M5 Pro, 18 logical cores, 48 GiB memory, macOS 27.2 |
| Build | Release. 18 zone threads |
| Commit | `fb9cabe` with the local changes of the load tools. The kernel, the modules, the runtime and the gateway are those of `fb9cabe` |
| World | Four verbs: `wait`, `cancel`, `set_routines`, `move_to`. No money, no goods, no spawn sites |
| Seed | 1 |

Other programs used the machine during all measurements. The load average
was between 5 and 32 with 18 logical cores, and each table gives the load
average of each run. Thus most times in this section are too high, and the
values near a limit change from run to run. Measure again on a machine that
does nothing else before you compare with a target.

The load tools changed on the day of the measurements. No change touched the
code that makes the load or measures it.

### `bench`

Placement `scatter`. One tick for each frame.

| Residents | Profile | Zones | Frames (warm-up + measured) | Frame ms: mean / p50 / p99 / worst | Inputs ms, mean | Observe ms, mean | Server ms: mean / worst | Actions for each frame | Actions for each second of wall time | Footprint at the end | Footprint bytes for each resident | Inboxes full | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10,000 | walkers | 4 | 60 + 300 | 0.42 / 0.41 / 0.62 / 0.71 | 0.24 | 4.74 | 5.40 / 7.78 | 789 | 139,177 | 265 MiB | 27,276 | 88 % | 7.0 to 22.3 | A |
| 100,000 | walkers | 49 | 60 + 300 | 2.44 / 2.20 / 3.70 / 33.79 | 5.02 | 50.40 | 57.86 / 89.26 | 7,834 | 126,201 | 2,965 MiB | 31,041 | 87 % | 7.0 to 22.3 | A |
| 250,000 | walkers | 128 | 60 + 300 | 10.52 / 7.83 / 43.34 / 86.62 | 28.53 | 195.61 | 234.66 / 1,307.13 | 19,584 | 76,333 | 6,079 MiB | 25,477 | 87 % | 30.4 to 28.7 | B |
| 500,000 | walkers | 256 | 60 + 300 | 20.24 / 15.93 / 81.02 / 175.66 | 60.64 | 354.43 | 435.31 / 1,420.06 | 39,169 | 81,104 | 11,985 MiB | 25,124 | 87 % | 30.4 to 28.7 | B |
| 1,000,000 | walkers | 512 | 60 + 300 | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | 7.0 to 22.3 | A |
| 1,000,000 | walkers | 512 | 20 + 100 | 30.86 / 28.31 / 96.54 / 101.60 | 116.01 | 598.40 | 745.27 / 1,338.98 | 78,544 | 93,757 | 10,469 MiB | 10,972 | 28 % | 29.0 to 26.1 | C |
| 1,000,000 | walkers | 512 | 20 + 100 | 55.81 / 53.54 / 74.19 / 179.66 | 143.38 | 665.93 | 865.12 / 1,720.10 | 78,544 | 81,442 | 9,127 MiB | 9,566 | 28 % | 22.3 to 21.8 | C |
| 1,000,000 | rush | 512 | 10 + 40 | 181.48 / 177.92 / 215.50 / 215.50 | 510.08 | 1,140.10 | 1,831.65 / 3,279.09 | 306,369 | 139,543 | 15,449 MiB | 16,195 | 47 % | 21.8 to 22.2 | R |

Commands:

- A: `cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000`
- B: `cargo run --release -p unsup-loadtest -- bench --scale 250k,500k`
- C: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 100 --warmup 20`
- R: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 40 --warmup 10 --profile rush`

The 10,000 and 100,000 residents of command A ran first, when the load
average was about 7. Command C ran two times.

What these runs show:

- One million residents with 360 frames did not fit in the free memory of
  the machine. Command A stopped the size after 312 of 360 frames: the
  process held 21.4 GiB, and the limit was 21.3 GiB (90 % of the memory that
  was free at the start). The largest size that ran with 360 frames is
  500,000 residents.
- One million residents ran in command C for 120 frames. The mean frame
  time was 30.9 ms in one run and 55.8 ms in the other. The worst frame was
  101.6 ms and 179.7 ms. The inboxes were only 28 % full, so the memory of
  these runs is not the memory of a world that has run for a long time.
- A resident costs 25,000 to 31,000 bytes when the inboxes are 87 % full.
  The value changes from run to run. Thus one million residents need at
  least 24 GiB. One earlier run of one million residents with 360 frames
  confirmed this. That run used a build of the tool that did not yet stop at
  the footprint limit. The footprint was 23.9 GiB at the end and 25.9 GiB at
  the peak. The system compressed or swapped 8.8 GiB of it, so the times of
  that run are not valid and are not in the table. To do such a run now, add
  `--max-memory-gib 40`.
- The system compressed or swapped 1.7 GiB of the memory of run R (11 % of
  its footprint). The summary of that run shows the warning. Its times are
  too high.
- `inputs` and `observe` run on one thread. One action costs 0.3
  microseconds in `World::apply` with 10,000 residents, and 1.5 to 1.8
  microseconds with one million residents. One observation costs 24 to 40
  microseconds in `World::observe`. With `walkers`, one action in four is
  followed by an observation. With `rush`, one action in ten.
- The largest action rate was 139,543 accepted actions for each second of
  wall time (run R).
- With one million residents and `walkers`, the work of the simulation
  thread for one frame was 745 ms and 865 ms (mean), and 1,339 ms and
  1,720 ms (worst). A server that steps one frame for each second cannot do
  this work in time on this machine in this state. The observations take
  about 80 % of that work.

### `soak`

Profile `walkers`, placement `scatter`, an audit after each tick.

| Residents | Zones | Ticks | Determinism check | Result | Audits passed | Hash comparisons passed | Actions accepted | Ticks for each second | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2,000 | 1 | 2,000 | no | PASSED | 2,001 | not measured | 318,524 | 429 | 24.5 to 23.2 | D |
| 2,000 | 1 | 2,000 | yes | PASSED | 2,001 | 41 | 318,524 | 268 | 23.2 to 22.3 | E |
| 20,000 | 9 | 2,000 | yes | PASSED | 2,001 | 41 | 3,146,107 | 21 | 26.5 to 31.1 | F |

Commands:

- D: `cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000`
- E: command D with `--verify-determinism`
- F: `cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 20000 --verify-determinism`

What these runs show:

- The runs D and E end with the same state hash (`19f83af4...`).
- The world of this worktree has no faucet. The money at the end is 0 held
  and 0 minted, and the world has no goods. Thus the conservation check
  shows only that nothing made value from nothing. Run `soak` again with the
  profile `daily_life` when the economy modules are in the world.
- The tests of the tool prove that the checks find a violation: a test
  plugin that copies money fails the run at the tick of the copy, and a test
  plugin that writes a number from outside the simulation into the state
  fails the determinism check.

### `swarm`

All runs: `--spawn-server` (server and agents in one process, loopback
network), profile `walkers`, log-normal think time with a mean of 5 seconds,
a window of 60 seconds, one frame for each second. The server of this
worktree has no WebSocket route and no long poll. Thus all agents sent HTTP
requests, and an agent that waited for the end of an action observed one
time each second.

| Agents | World (cities x districts) | HTTP connections | Ramp-up s | Result | Ack latency ms: p50 / p95 / p99 / max | Acks for each second | Observations for each second | Failed requests | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 5,000 | 3 x 1 (automatic) | per-agent | 30 | SUSTAINED | 0.36 / 3.64 / 13.68 / 106.43 | 551 | 2,396 | 0 | 17.3 to 24.8 | G |
| 7,500 | 1 x 2 (automatic) | per-agent | 30 | NOT SUSTAINED | 1.59 / 215.81 / 647.68 / 858.62 | 825 | 3,543 | 0 | 18.6 to 21.6 | G |
| 10,000 | 1 x 2 (automatic) | per-agent | 30 | NOT SUSTAINED | 81.28 / 1,211.39 / 1,998.85 / 2,213.89 | 1,073 | 4,063 | 0 | 26.0 to 22.5 | G |
| 15,000 | 1 x 3 (automatic) | per-agent | 30 | NOT SUSTAINED | 3,379.20 / 6,914.05 / 7,536.64 / 11,960.32 | 1,111 | 2,124 | 0 | 22.9 to 20.9 | G |
| 15,000 | 75 x 1 | per-agent | 30 | SUSTAINED | 0.69 / 7.27 / 23.84 / 119.61 | 1,649 | 7,184 | 0 | 18.7 to 20.0 | H |
| 20,000 | 100 x 1 | shared | 60 | NOT SUSTAINED | 0.67 / 45.05 / 272.89 / 386.05 | 2,152 | 9,717 | 793 (0.111 %) | 16.1 to 19.9 | I |
| 20,000 | 100 x 1 | shared | 60 | SUSTAINED | 0.50 / 1.69 / 8.45 / 57.70 | 2,148 | 9,831 | 0 | 10.0 to 18.5 | I |
| 25,000 | 125 x 1 | shared | 60 | SUSTAINED | 0.62 / 6.10 / 18.25 / 160.51 | 2,687 | 12,250 | 337 (0.038 %) | 20.0 to 14.7 | I |
| 30,000 | 150 x 1 | shared | 60 | NOT SUSTAINED | 0.86 / 146.81 / 512.51 / 625.15 | 3,216 | 14,531 | 963 (0.090 %) | 17.4 to 19.6 | I without `--ws-share 0` |
| 30,000 | 150 x 1 | shared | 60 | SUSTAINED | 0.61 / 1.43 / 2.00 / 30.66 | 3,225 | 14,771 | 0 | 10.1 to 12.3 | I |
| 40,000 | 200 x 1 | shared | 60 | SUSTAINED | 1.02 / 2.37 / 3.23 / 20.41 | 4,295 | 19,665 | 0 | 22.0 to 19.7 | I |
| 50,000 | 250 x 1 | shared | 60 | SUSTAINED | 1.30 / 3.00 / 3.87 / 18.34 | 5,364 | 24,589 | 0 | 5.3 to 8.2 | I |
| 60,000 | 300 x 1 | shared | 60 | STOPPED after 34 s of the window: no free local port | 2.63 / 334.08 / 675.33 / 1,083.39 | 6,318 | 27,585 | 295 (0.026 %) | 9.6 to 13.3 | I |
| 100,000 | 500 x 1 | shared | 90 | STOPPED in the ramp-up: no free local port | not measured | not measured | not measured | not measured | 10.1 to 8.7 | I with `--ramp-up 90` |
| 150,000 | 750 x 1 | shared | 90 | STOPPED in the ramp-up: no free local port | not measured | not measured | not measured | not measured | 8.9 to 7.9 | I with `--ramp-up 90` |

Commands (N is the number of agents, C is the number of cities):

- G: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents N --ramp-up 30 --duration 60`
- H: command G with `--districts 1 --cities C`
- I: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents N --ramp-up 60 --duration 60 --districts 1 --cities C --http-connections shared --ws-share 0`

What these runs show:

- The largest number of agents that this machine sustained for 60 seconds
  is 50,000: the 99th percentile of the acknowledgement latency was 3.9 ms,
  with 5,364 acknowledgements and 24,589 observations for each second.
- 60,000 agents were not sustained. The latency went up (99th percentile
  675 ms). Requests then stayed open longer, the shared pool opened more
  connections, and the system had no free local port. The tool stopped the
  run after 34 seconds of the window. The runs with 100,000 and 150,000
  agents stopped in the ramp-up for the same reason, with about 85,000 and
  90,000 agents ready.
- With one connection for each agent, one macOS client machine can run at
  most 15,872 agents (16,384 local ports). 15,000 such agents were
  sustained with a 99th percentile of 23.8 ms.
- With the automatic world, all residents of a city start on one tile. Runs
  with 7,500 or more agents in one city were not sustained: with 15,000
  agents on one tile the median latency was 3.4 seconds. The same 15,000
  agents in 75 cities had a 99th percentile of 23.8 ms. The cause is the
  cost of an observation, which grows with the number of residents in sight.
- A result near the limit depends on the other programs on the machine.
  20,000 and 30,000 agents were not sustained in one run each. They were
  sustained in a later run, when the load average was lower.
- The agents made at most 6,318 acknowledgements for each second. These
  runs do not show the largest action rate of the server.

## 9. State of the targets

| Target | State | What was measured |
| --- | --- | --- |
| 1,000,000 residents | Not shown | One million residents ran for 120 frames in one process, two times. The mean frame time was 30.9 ms and 55.8 ms. The worst frame was 101.6 ms and 179.7 ms. A run with inboxes that are 87 % full needs about 24 GiB and did not fit in the free memory of the machine. The work of the simulation thread for one frame was 745 ms and 865 ms (mean). No run had one million residents over the network |
| 200,000 actions for each second | Not shown | In one process: 76,000 to 139,000 accepted actions for each second of wall time with `walkers`, and 139,543 with `rush` and one million residents. Over the network: 5,364 acknowledgements for each second with 50,000 agents. This is the load that the agents made, not the largest rate of the server |
| Acknowledgement below 250 ms at the 99th percentile | Shown only for 50,000 agents on loopback | Server and agents on one machine, loopback, HTTP only: 3.9 ms with 50,000 agents on a shared pool of connections, and 23.8 ms with 15,000 agents that held one connection each. Not sustained with 60,000 agents |

Not measured:

- A machine that does nothing else.
- Linux.
- A server and a swarm on different machines, and more than one swarm
  machine. One macOS client machine can hold 15,872 connections to one
  server address.
- TLS. This build of the swarm sends plain HTTP only.
- WebSocket agents, the long poll and the sign-up with owners and keys
  against the server of the project. The server of this worktree does not
  have these routes. The tests of the tool use them against a test gateway.
- The largest action rate of the server over the network. The agents of all
  runs had a think time of 5 seconds.
- The verbs of `daily_life` other than `move_to` and `wait`.
- Conservation with money and goods in the world.
- Runs longer than 2,000 ticks or 60 seconds.
