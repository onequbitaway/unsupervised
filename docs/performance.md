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

A target is met only when a measurement shows it. Section 10 gives the state
of each target. Section 9 has the findings.

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
three commands run the full game that the server runs: the life, economy,
finance, society, crime, politics and missions modules, the runtime and the
gateway.

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
| 1 | The command failed, or the profile does not fit the verbs of the world, or `soak` found a violation, or a size of `bench` did not finish, or `swarm` reached a limit of the machine, or `swarm --require-sustained` was not sustained |
| 2 | The command line is not correct |

### Refusals are data

The game refuses many actions: a wall is in the way, the budget of the day
is spent, the resident has no cash. A refusal is a correct answer of the
world. It is not a failure of the tool or of the server. All three commands
count refusals in the same way.

| Line of the summary | Meaning |
| --- | --- |
| `rejected at once` | The world rejected the request in the acknowledgement. Nothing changed. The line gives the rate for each error code |
| `reports read` | Actions that the world accepted and that ended later. A resident reads the report of an ended action in its next observation. The line gives the rate of reports that say "failed", for each error code. An action fails in its report when the world refuses it at its turn in the queue, or when it stops before its end |
| `not sent` | Actions of a plan that were not sent, because the world rejected the walk before them |
| One line for each verb | Requests, rate of rejections and rate of failed reports of the verb, with the error codes |

The inbox of a resident keeps the last `kernel.inbox_max` notices and
reports (32). A resident that hears much speech can lose a report before it
reads it. Thus `reports read` is a sample, not a full count.

## 3. Behaviour profiles

A behaviour profile tells the tool what the residents do. A profile is a
TOML file. All three commands read the same profiles.

```toml
name = "workers"
summary = "Residents walk and do public works."
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
verb = "work"
weight = 3
at = ["city_hall", "depot"]
when = { attr_above = { "need.energy" = 100 } }
```

- The tool chooses an action by weight among the actions that are possible
  now. An action with weight 9 is chosen nine times as often as an action
  with weight 1.
- `at` lists site kinds. The action works only at a site of one of these
  kinds. A resident that is not at such a site walks there first: the tool
  sends a plan of `move_to { site }` and the action. The site is one of the
  three nearest sites of the kinds in the city of the resident. The action
  waits in the queue of the resident behind the walk.
- `when` lists conditions. All of them must be true.
- `once = true` lets a resident do the action one time in a run.

| Condition | True when |
| --- | --- |
| `cash_min = N`, `cash_max = N` | The cash of the resident in cents is at least N, or at most N |
| `carries_tag = "food"` | The resident carries an item with the tag |
| `lacks_tag = "food"` | The resident carries no item with the tag |
| `attr_below = { "need.food" = N }` | The attribute is below N |
| `attr_above = { "need.energy" = N }` | The attribute is above N |
| `did = "open_account"` | The world accepted this verb from the resident before, in this run |

The tool checks a profile against the catalogue of the world at the start.

- A verb that the world does not have is skipped. The tool prints one
  warning for each. The weights of the other actions keep their ratio.
- The command stops when an action does not fit a verb of the world: an
  argument that the verb does not have, a necessary argument that the
  profile does not give, a condition with an attribute or a tag that the
  world does not have, or `at` on a free verb. A free verb does not wait
  behind a walk.
- The tool prints a warning for a site kind of `at` that no city has.
- The command stops when the world has none of the verbs of the profile.

### Argument generators

In `args`, a table with the key `gen` is a generator. All other values are
used as they are written. A generator can be anywhere in a table or a list.

| Generator | Value |
| --- | --- |
| `{ gen = "near_tile", radius = N }` | `{ x, y }`: a random walkable tile within N tiles of the resident, in its city |
| `{ gen = "nearby_resident", radius = N }` | The id of a random other resident within N tiles |
| `{ gen = "nearby_site", radius = N, kind = "shop" }` | The id of a random site within N tiles. `kind` is optional |
| `{ gen = "carried", tag = "raw", max_qty = N }` | `{ item, qty }`: goods that the resident carries. `tag` and `max_qty` are optional |
| `{ gen = "cash", percent = N }` | N percent of the cash of the resident, in cents |
| `{ gen = "item", tag = "food", as = "name" }` | A random item of the catalogue. `tag` is optional. `as` is `name` or `id` |
| `{ gen = "int", min = A, max = B }` | A whole number from A to B |
| `{ gen = "choice", of = [...] }` | One of the listed values |
| `{ gen = "text", words = N }` | A sentence of N words |
| `{ gen = "self" }` | The id of the resident |

A walk target is always a tile on which a resident can stand, or a site id.
The tool reads the tiles and the sites of each zone one time at the start:
from `World::view` in one process, and from `GET /v1/world` and
`GET /v1/world/zones/{id}` over the network. Wall (4), water (5) and rock (8)
are not walkable (`docs/design/game-design.md`, section 2). The life module
makes each walkable tile of a zone reachable on foot.

When a generator cannot make a value (for example, no resident is near), the
resident chooses a different action. After four tries it does nothing in
this turn.

### Built-in profiles

The built-in profiles are in `crates/loadtest/profiles`. Give `--profile` the
name of a built-in profile or the path of a file.

| Name | What the residents do | Measured rate of actions |
| --- | --- | --- |
| `walkers` (default) | Walk to a random walkable tile within 12 tiles, think 0 to 10 ticks, sometimes stand still | One action in 19 ticks for each resident |
| `rush` | Walk to a random walkable tile within 3 tiles with almost no think time. A stress load | One action in 5.9 ticks |
| `daily_life` | The full game for a newcomer. See below | One request in 32 to 72 ticks |
| `still` | Stand still and observe after each action. Use it to measure the cost of one observation | One action in 4 ticks |
| `sprint` | Walk to a random walkable tile within 2 tiles, or stand still for one tick. An agent of the swarm never waits for the end of an action, and it observes after one action in fifty. A stress load for the rate of actions over the network. Use it with `swarm --acks-only` | Not measured in one process |

`daily_life` has 28 actions with 22 verbs: `move_to`, `wait`, `rest`, `work`,
`gather`, `list_jobs`, `sell`, `buy`, `eat`, `sleep`, `say`, `message`,
`post`, `relate`, `inspect`, `list_property`, `open_account`, `deposit`,
`withdraw`, `give`, `train` and `write_will`.

- A resident walks to the city hall or the depot and does one shift of
  public works for a wage.
- It walks to a farm, a forest or a mine and gathers goods.
- It sells raw goods at the depot and buys food there.
- It eats, it sleeps in a shelter, it rests in a park.
- It talks, sends messages, posts, looks at residents and sites.
- It opens a bank account one time, deposits half of its cash and withdraws.
- It gives cash to a resident beside it, trains at a school, writes a will.

The profile does not have these parts of the game: jobs of other residents,
property, business, loans, shares, the exchange, organizations, contracts,
marriage and households, `travel`, `heal`, `use_item` and `opine`. A
newcomer cannot use most of them.

A test fails when a built-in profile names a verb that the game does not
have, or an argument that the verb does not have.

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
| `--observe-sections A,B` | all sections | The sections that a resident asks for in an observation. Use it to measure the cost of one section |
| `--observe-batch N` | 0 | How the observations of one frame are built. 0: one `World::observe` for each observation, on one thread. N: calls of `World::observe_batch` with at most N requests each, on all processor cores. See "One thread or all cores" below |
| `--data FILE` | none | A TOML file that replaces numbers of the game, in the format of a module's `data/balance.toml` |
| `--max-memory-gib X` | automatic | Stop the run when the process holds more memory than this |
| `--time-limit-secs N` | none | With `--scale`: stop a size that takes longer |

- `scatter` moves each resident to a random walkable tile of its city before
  the run. This is the state of a world that has run for some time.
- `spawn` leaves each resident where the game puts a new resident: in a
  shelter or a station of its city. Many residents then stand in few sites.
- A size of `--scale` that fails does not stop the other sizes. The summary
  shows `NOT MEASURED` and the reason for that size.

### What the numbers mean

| Number | Meaning |
| --- | --- |
| `frame` | Time of one `World::step`: all zones tick in parallel, then the global realm, then the router |
| `inputs` | Time to apply the action requests of one frame through `World::apply`, on one thread |
| `observe` | Time to build the observations of one frame. Without `--observe-batch`: through `World::observe`, on one thread. With it: through `World::observe_batch`, on all processor cores; the number is the time that the calling thread waits |
| `server` | `inputs + observe + frame`: the time that one thread needs for the work of one frame. See "One thread or all cores" below |
| `generator` | Time that the tool itself needs to choose actions. A server does not do this work |
| mean, p50, p99, worst | Mean, median, 99th percentile and largest value over the measured frames. The percentiles are exact (nearest rank) |
| actions accepted, rejected | Requests that the world accepted or rejected in the measured frames |
| actions for each frame | Accepted actions divided by the measured frames |
| actions for each second of wall time | Accepted actions divided by the wall time of the measured loop. The loop includes the time of the generator |
| actions for each second of server work | Accepted actions divided by the sum of `server` |
| one request costs, one observation costs | The sum of `inputs` divided by the requests, and the sum of `observe` divided by the observations, in microseconds. With `--observe-batch` the second number is time of the calling thread, not processor time: the cores share the work |
| moves, zone changes | Position changes and residents that changed zone, for each frame |
| RSS | Resident set size of the process: the memory that is in physical memory now |
| footprint | The memory that the system charges to the process, with the pages that it compressed or swapped. macOS: `phys_footprint`. Linux: `VmRSS + VmSwap`. On macOS the footprint can be smaller than the RSS; the tool uses the larger of the two for its memory limit |
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
- `inputs` runs on one thread, as it does in the server. `frame` uses one
  thread for each logical core.
- One tick is one frame. A server steps one frame for each second by
  default. The last line of the summary gives the share of that second that
  the slowest frame used.
- A share of the residents (`observe_share`) observes after an action. Over
  HTTP, each answer of `POST /v1/act` has an observation, unless the
  request has `observe=false`. A waiting call (the long poll) and a
  WebSocket cost more observations. Thus a server builds more observations
  for each action than `bench` does with the built-in profiles.

### One thread or all cores

The simulation thread of the server does not build one observation for each
call. It takes at most 256 calls from its mailbox (`batch` in
`crates/runtime/data/runtime.toml`), applies the actions of that batch, and
then builds the observations of all `observe` calls of the batch in one call
of `World::observe_batch`. The kernel builds them on all processor cores:
one thread for each zone. The observation of a `wait` call (the long poll)
is built alone, when the call is answered.

`bench` can measure both ways.

- Without `--observe-batch`, each observation is one `World::observe` on one
  thread. `observe` is then processor time. `server` is an upper limit for
  the simulation thread of a server: no batch helps.
- With `--observe-batch 256`, the observations of a frame are built in calls
  of at most 256 requests. This is the largest batch that the server builds:
  a batch of the mailbox with `observe` calls only. `server` is then the
  work of the simulation thread when all observation requests come in full
  batches. A batch of the mailbox that also has actions holds fewer
  observations, and its gain is smaller.

The two ways give the same observations. A test of the tool proves that the
counts of a run are equal in both ways.

## 5. `soak`: conservation and determinism

`soak` steps the world tick by tick with a behaviour profile. It stops at the
first violation.

```sh
cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --profile daily_life
cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --profile daily_life --verify-determinism
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
| money faucets, money sinks | The money that each faucet created and each sink destroyed, by name. A run with the walkers profile has none |
| goods at the end | Kinds of goods, and `held = produced - consumed` in units. The report file has each good |
| dropped messages | Messages without a target that the world dropped |
| state hash | blake3 hash of the state after the last tick |

The conservation check has a meaning only when money moves. Use the profile
`daily_life`: its residents earn wages (faucet `gov.public_works`), sell
goods to the depot (faucet `export.sale`) and buy food from the depot (sink
`import.purchase`).

## 6. `swarm`: load over the network

`swarm` starts N virtual agents. Each agent is one task. Each agent signs up,
then it thinks, observes, acts and sometimes waits, until the run ends.

```sh
cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents 1000 --profile daily_life
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
| `--acks-only` | off | The HTTP agents send `POST /v1/act?observe=false`. The answer has only the acknowledgements, and the server builds no observation for it. An agent then keeps the view that it has; it observes with the chance `observe_share` of the profile and when it waits for the end of an action |
| `--profile NAME` | `walkers` | Behaviour profile |
| `--credentials FILE` | none | Use the keys in the file again, and write the keys of this run to it. Keep the file private |
| `--request-timeout S` | 30 | Seconds after which a request has failed |
| `--tick-ms N` | 1000 | With `--spawn-server`: real milliseconds for one game minute |
| `--cities N`, `--districts N` | automatic | With `--spawn-server`: size of the world |
| `--p99-target-ms X` | 250 | Target for the 99th percentile of the acknowledgement latency |
| `--max-error-share X` | 0.001 | Largest share of failed requests of a sustained run |
| `--require-sustained` | off | Make the exit code 1 when the load was not sustained |
| `--stats-path PATH` | `/v1/spectate/stats` | A route with numbers of the server that needs no token |

### What an agent does

1. The agent signs up with the flow of `docs/design/protocol.md`, section 2:
   `POST /v1/owners`, then `POST /v1/residents` with the owner token. Each
   agent is one owner with one resident. With `--credentials`, the agent
   uses its stored key and does not sign up. When the server does not know a
   stored key (`401`), the agent signs up again.
2. The agent connects. Each request has the key of the resident as
   `Authorization: Bearer`. A WebSocket agent opens `GET /v1/ws` with the
   key in the handshake.
3. The agent observes once. It is then *ready*.
4. The agent thinks for a time from the think time distribution. The
   log-normal distribution imitates an agent that a language model drives:
   most answers come fast and some take a long time.
5. The agent observes when its view is old, or with the chance
   `observe_share` of the profile.
6. The agent chooses an action from the profile and sends it. An action at a
   site is a plan: `{ "plan": [...] }` over HTTP and a `plan` frame on the
   WebSocket. The tool measures the acknowledgement latency. The answer of
   `POST /v1/act` has a new observation, and the agent uses it as its view.
   With `--acks-only` the answer has none. The acknowledgement of a
   WebSocket has no observation in both cases.
7. When the action takes time, the agent waits for its end with the chance
   `wait_share` of the profile. An HTTP agent uses the long poll
   (`GET /v1/wait` with `kinds=report`). A WebSocket agent reads the report
   that the server pushes.
8. After a failed request, the agent waits one second and tries again. After
   five failed requests in a row, the agent gives up.
9. After a request that a limit of the server refused (`429`), the agent
   waits as long as the header `Retry-After` says, and goes on.

The tool reads the catalogue and the map of the world before the first
agent starts: `GET /v1/catalogue`, `GET /v1/world` and
`GET /v1/world/zones/{id}` for each zone.

This build sends plain HTTP and plain WebSocket only. It refuses an
`https://` address with a clear message. For a server behind TLS, run the
swarm on the same machine as the server, or through a local proxy.

### Limits of the server

The gateway has limits for each key, each owner and each client address
(`docs/design/protocol.md`, section 4). A swarm comes from one address, so
the default limits refuse it.

- With `--spawn-server`, the tool turns these limits off in the
  configuration of the server that it starts: residents for each owner,
  requests for each key and each address, sign-ups, open `wait` calls and
  WebSocket connections for each resident. It changes nothing else.
- With `--url`, the tool changes nothing on the server. A request that a
  limit refuses (status `429`, codes `rate_limited` and
  `too_many_connections`) is counted in a class of its own. It is not a
  failed request. The summary gives the counts by code, and the run is
  `NOT SUSTAINED` with the limit as the reason.

### Machine limits

Each agent with a connection of its own needs one open file and one local
port. A server in the same process needs one more open file for each
connection. A WebSocket agent holds its socket and signs up through a shared
pool of connections.

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
- Wait one minute between two large runs. A local port stays in use for
  about 30 seconds after its connection closes.
- Start the agents from more than one machine for a larger load.

### What the numbers mean

All latencies and rates are those of the measurement window. The window
starts after the ramp-up.

| Number | Meaning |
| --- | --- |
| ack latency | HTTP: time from the send of `POST /v1/act` to the last byte of the answer. WebSocket: time from the send of the frame to the last `ack` frame. Each acknowledgement of a plan has the latency of its request |
| p50, p95, p99, max | Percentiles from a histogram with three significant digits |
| observe latency | Time of one observation |
| acknowledgements for each second | Acknowledgements in the window divided by its length. The summary also gives the least and the most in one second |
| acknowledgements by state | `started`, `done`, `queued`, `pending`, `rejected` |
| rejected at once, reports read, the lines for each verb | See "Refusals are data" in section 2. An HTTP agent reads reports in observations. A WebSocket agent counts the reports that the server pushes |
| failed requests | Requests without a usable answer, by code: the `error.code` of the server (for example `overloaded`), `http_<status>`, `timeout`, `connect`, `transport`, `closed`, `decode`. The summary shows the message of the first failure of each code |
| error share | Failed requests divided by all requests in the window |
| refused by a limit of the server | Requests with status `429`, by code. They are not failed requests |
| agents ready | Agents that signed up, connected and got their first observation |
| agents gave up | Agents that stopped after five failed requests in a row, by the code of the last error |
| WebSockets open at the peak | Largest number of WebSocket connections that were open at the same time. This number is exact |
| pushed frames | Notices and reports that the server pushed on WebSockets, in the whole run |
| server | With `--spawn-server`: residents, frames in the window, mean frame time, 99th percentile of the last frames, worst frame, inputs that the simulation applied. With `--url`: the answer of the statistics route at the start and at the end of the window, in the report file |
| simulation thread | With `--spawn-server`: the share of the window in which the one simulation thread worked, from the counters of the runtime: frames, inputs and observations between frames, and the calls themselves. The time of the observations for the long polls that a frame woke is not in these counters, so the real share is larger |
| observations of the simulation thread | With `--spawn-server`: the observations that the simulation thread built in the window, the steps in which it built them, and the time of these steps. One step builds the observations of all `observe` calls of one batch of calls, or the observation of one `wait` call. Many observations for each step show that the batches work. This time includes the observations for the long polls |

The result is `SUSTAINED` when all of these are true:

- every agent became ready, and no agent gave up;
- at least one action was acknowledged in the window;
- no limit of the server refused a request;
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
- The server puts a new resident in a shelter or a station of its city. A
  city has one shelter for each two zones and few stations. Thus the agents
  of a swarm start in few sites.
- A city has one depot and one city hall. With `daily_life`, many residents
  of a city walk to these two sites.

## 8. Results

This section has only numbers that were measured in this worktree. Each
number is copied from the output of a tool. All other numbers are **not
measured**.

### Conditions

| Item | Value |
| --- | --- |
| Date | 2026-11-19, 00:24 to 01:55 (UTC) |
| Machine | Apple M5 Pro, 18 logical cores, 48 GiB memory, macOS 27.2 |
| Build | Release. 18 zone threads |
| Commit | `ba673bd`. Bench run A used the clean commit. All later runs used the local changes of the load tools of section 4 and section 6 (`--observe-batch`, `--acks-only`, the profile `sprint`). The kernel, the modules, the runtime and the gateway are those of `ba673bd` |
| World | The full game: the life, economy, finance, society, crime, politics and missions modules |
| Order | One measurement at a time |

Other programs used the machine during all measurements. Before the first
run, `top -l 2 -n 12 -o cpu` showed a security scanner, an editor and the
window system with about three and a half processor cores together. The
load average before a run was between 2.3 and 10.5 with 18 logical cores.
Each table gives the load average at the start and at the end of each run.
The load average at the end of a `swarm` run is high because the swarm and
the server use the processors themselves. Measure again on a machine that
does nothing else before you compare with a target.

### `bench`

Placement `scatter`. One tick for each frame. "us" is microseconds. The
column "Observations" says how they were built: on one thread, or in
batches of 256 on all cores (section 4, "One thread or all cores").

| Residents | Profile | Zones | Frames (warm-up + measured) | Observations | Frame ms: mean / p50 / p99 / worst | Inputs ms, mean | Observe ms, mean | Server ms: mean / worst | Actions for each frame | Actions for each second of wall time | Rejected at once | us for one request / one observation | Footprint at the end | Footprint bytes for each resident | RSS at the end | RSS bytes for each resident | Inboxes full | Fullest zone | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10,000 | walkers | 4 | 60 + 300 | one thread | 1.07 / 0.87 / 11.19 / 11.48 | 3.27 | 3.40 | 7.73 / 17.97 | 523 | 66,020 | 0.00 % | 6.25 / 25.97 | 93 MiB | 8,876 | 104 MiB | 10,033 | 57 % | 2,953 | 8.0 to 5.8 | A |
| 100,000 | walkers | 49 | 60 + 300 | one thread | 8.40 / 5.21 / 180.09 / 182.27 | 35.63 | 41.85 | 85.88 / 260.01 | 5,183 | 58,228 | 0.00 % | 6.87 / 32.24 | 750 MiB | 7,772 | 1,044 MiB | 10,858 | 58 % | 2,606 | 8.0 to 5.8 | A |
| 1,000,000 | walkers | 512 | 60 + 300 | one thread | 95.35 / 54.10 / 2,452.50 / 2,517.18 | 422.00 | 495.44 | 1,012.79 / 3,438.08 | 51,912 | 48,459 | 0.00 % | 8.13 / 38.18 | 8,487 MiB | 8,890 | 10,823 MiB | 11,339 | 56 % | 2,607 | 8.0 to 5.8 | A |
| 10,000 | walkers | 4 | 60 + 300 | batches of 256 | 0.95 / 0.76 / 10.77 / 10.96 | 3.13 | 2.11 | 6.19 / 16.34 | 523 | 82,182 | 0.00 % | 5.99 / 16.14 | 81 MiB | 7,564 | 113 MiB | 10,890 | 57 % | 2,953 | 5.5 to 7.9 | B |
| 100,000 | walkers | 49 | 60 + 300 | batches of 256 | 7.82 / 4.82 / 169.09 / 175.22 | 35.02 | 15.95 | 58.78 / 228.16 | 5,183 | 83,833 | 0.00 % | 6.76 / 12.29 | 811 MiB | 8,411 | 1,106 MiB | 11,507 | 58 % | 2,606 | 5.5 to 7.9 | B |
| 1,000,000 | walkers | 512 | 60 + 300 | batches of 256 | 91.87 / 50.76 / 2,420.28 / 2,478.26 | 406.80 | 168.71 | 667.37 / 3,059.00 | 51,912 | 71,927 | 0.00 % | 7.84 / 13.00 | 7,599 MiB | 7,958 | 10,016 MiB | 10,493 | 56 % | 2,607 | 5.5 to 7.9 | B |
| 10,000 | daily_life | 4 | 60 + 300 | one thread | 3.96 / 3.71 / 14.47 / 17.77 | 3.41 | 3.59 | 10.96 / 27.01 | 290 | 25,001 | 7.21 % | 10.92 / 30.31 | 98 MiB | 9,329 | 110 MiB | 10,551 | 100 % | 3,891 | 8.6 to 6.2 | D |
| 100,000 | daily_life | 49 | 60 + 300 | one thread | 27.17 / 25.13 / 191.33 / 249.47 | 26.07 | 21.77 | 75.02 / 298.38 | 1,467 | 17,658 | 5.66 % | 16.77 / 37.77 | 893 MiB | 9,264 | 1,181 MiB | 12,291 | 100 % | 16,409 | 8.6 to 6.2 | D |
| 1,000,000 | daily_life | 512 | 60 + 300 | one thread | 193.68 / 154.00 / 2,487.95 / 2,538.43 | 324.09 | 247.84 | 765.60 / 3,246.39 | 13,105 | 14,933 | 5.16 % | 23.45 / 48.53 | 10,151 MiB | 10,634 | 12,554 MiB | 13,154 | 100 % | 19,103 | 8.6 to 6.2 | D |
| 10,000 | daily_life | 4 | 60 + 300 | batches of 256 | 3.70 / 3.50 / 13.42 / 13.96 | 3.16 | 2.77 | 9.63 / 21.50 | 290 | 28,377 | 7.21 % | 10.11 / 23.41 | 97 MiB | 9,170 | 129 MiB | 12,522 | 100 % | 3,891 | 6.9 to 8.6 | E |
| 100,000 | daily_life | 49 | 60 + 300 | batches of 256 | 26.03 / 24.10 / 187.10 / 251.68 | 24.74 | 11.75 | 62.52 / 288.12 | 1,467 | 20,876 | 5.66 % | 15.91 / 20.38 | 939 MiB | 9,746 | 1,246 MiB | 12,970 | 100 % | 16,409 | 6.9 to 8.6 | E |
| 1,000,000 | daily_life | 512 | 60 + 300 | batches of 256 | 185.23 / 144.38 / 2,549.50 / 2,622.43 | 320.07 | 92.92 | 598.23 / 3,137.64 | 13,105 | 18,503 | 5.16 % | 23.16 / 18.19 | 8,587 MiB | 8,994 | 10,945 MiB | 11,466 | 100 % | 19,103 | 6.9 to 8.6 | E |
| 1,000,000 | rush | 512 | 10 + 40 | batches of 256 | 70.15 / 68.91 / 105.88 / 105.88 | 567.87 | 215.08 | 853.10 / 931.89 | 170,323 | 170,738 | 0.00 % | 3.33 / 12.65 | 6,588 MiB | 6,899 | 8,467 MiB | 8,869 | 25 % | 2,451 | 7.5 to 8.6 | R |

Commands:

- A: `cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000`
- B: command A with `--observe-batch 256`
- D: `cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000 --profile daily_life`
- E: command D with `--observe-batch 256`
- R: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 40 --warmup 10 --profile rush --observe-batch 256`

The load average of a `--scale` command is that of the whole command: the
start of the first size and the end of the last size.

What these runs show:

- One million residents ran for the default 360 frames with both profiles.
  The process held 9.8 GiB to 12.3 GiB at the end (RSS). The memory limit
  of the tool was 17.7 GiB to 24.8 GiB, and the system compressed or
  swapped nothing of a run.
- At one million residents the median frame took 50.8 ms to 54.1 ms with
  `walkers` and 144.4 ms to 154.0 ms with `daily_life`. 95 of 100 frames
  stayed below 72.3 ms and below 203.0 ms.
- Some frames are much longer. With one million residents the 99th
  percentile of the frame time was 2,420 ms to 2,550 ms and the worst frame
  2,478 ms to 2,622 ms, in each of the four runs. With 100,000 residents
  the worst frame was 175 ms to 252 ms. The cause is the leaderboards of
  the missions module, once in 60 frames. See "The frame of the
  leaderboards" below and finding 9.4. Run R has 50 frames, and none of
  them is such a frame.
- `inputs` runs on one thread. With one million `walkers` it was 407 ms to
  422 ms for the 51,912 requests of one frame: 7.8 to 8.1 microseconds for
  one request, because a walk needs a path search. With `daily_life` it was
  320 ms to 324 ms for 13,818 requests: 23.2 to 23.5 microseconds for one.
  With `rush` (short walks) one request cost 3.33 microseconds.
- One observation on one thread cost 26 to 38 microseconds with `walkers`
  and 30 to 49 microseconds with `daily_life`. In batches of 256 it cost 12
  to 16 microseconds of the calling thread with `walkers` and 18 to 23
  microseconds with `daily_life`. With one million residents, the
  observations of one frame took 495 ms on one thread and 169 ms in batches
  (`walkers`), and 248 ms and 93 ms (`daily_life`).
- With batches, the work of one thread for one frame of one million
  residents was 667 ms (mean) with `walkers`, 598 ms with `daily_life` and
  853 ms with `rush`. 95 of 100 frames stayed below 682 ms (`walkers`) and
  817 ms (`daily_life`). The worst frames were 3,059 ms and 3,138 ms: the
  frames of the leaderboards.
- The largest action rate was that of run R: 170,738 accepted actions for
  each second of wall time, and 199,650 for each second of server work. The
  slowest of its 40 frames used 93.2 % of one second.
- The world rejected no request of `walkers` and `rush`, and no walk failed
  in a report that a resident read.
- With `daily_life`, the fullest zone had 16,409 of 100,000 residents and
  19,103 of one million: the residents of a city walk to its one depot and
  its one city hall.
- The footprint was smaller than the RSS in each run. The tool uses the
  larger of the two for its memory limit. The same world gave different
  memory numbers in two runs: `daily_life` with one million residents had
  an RSS of 12,554 MiB in run D and 10,945 MiB in run E. The cause was not
  examined.

### The frame of the leaderboards

`walkers`, 100,000 residents in 49 zones, 60 warm-up frames and 300
measured frames. Each run changes parameters of the missions module with a
data file.

| `missions.report_period` | `missions.board_period` | Frame ms: mean / p50 / p99 / worst | Load average, start to end |
| --- | --- | --- | --- |
| 60 (the default) | 60 (the default) | 8.40 / 5.21 / 180.09 / 182.27 | 8.0 to 5.8 (run A) |
| 60 | 1,000,000 | 5.77 / 5.12 / 30.81 / 32.33 | 5.8 to 6.3 |
| 100,000 | 60 | 5.45 / 5.19 / 9.29 / 15.78 | 6.3 to 5.9 |
| 100,000 | 1,000,000 | 5.45 / 5.22 / 8.47 / 17.16 | 5.8 to 5.8 |

Command: `cargo run --release -p unsup-loadtest -- bench --residents 100000 --data FILE`.
The file has the two parameters in the format of a module's
`data/balance.toml`.

- Each zone reports its residents to the leaderboards each
  `missions.report_period` frames (60), and all zones do it in the same
  frame. The global realm publishes the boards each
  `missions.board_period` frames (60).
- Without the reports, the long frame is gone: the worst frame is 15.8 ms,
  not 182.3 ms. Without the publication only, the worst frame is 32.3 ms.
- With one million residents this frame takes 2.4 to 2.6 seconds (runs A,
  B, D and E).

### The size of a batch of observations

Each value is the mean time of one observation for the calling thread, in
microseconds.

`walkers`, 100,000 residents in 49 zones, placement `scatter`, 1,298
observations in one frame:

| One thread | Batch of 16 | 64 | 256 | 1,024 | 4,096 | 16,384 |
| --- | --- | --- | --- | --- | --- | --- |
| 32.24 (run A) | 33.42 | 16.29 | 11.21 | 9.93 | 9.17 | 9.35 |

`walkers`, 10,000 residents in 4 zones, placement `spawn`, 135 observations
in one frame:

| One thread | Batch of 1 | 2 | 4 | 8 | 16 | 64 | 256 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 22.82 | 23.03 | 67.36 | 65.81 | 54.03 | 40.15 | 24.36 | 20.04 |

Commands: `cargo run --release -p unsup-loadtest -- bench --residents 100000 --observe-batch N`
and `cargo run --release -p unsup-loadtest -- bench --residents 10000 --cities 1 --districts 2 --placement spawn --observe-batch N`.
The load average was between 6.0 and 10.3.

- A batch of 2 to 16 requests costs more for each observation than one
  thread does: up to three times as much in the world with 4 zones.
- In the world with 4 zones, a batch must have about 64 requests to cost
  the same as one thread. In the world with 49 zones, a batch of 64 costs
  half.
- The largest gain in these runs is a factor of 3.5 (32.24 to 9.17
  microseconds). The kernel example below shows a factor of 13 to 15 with
  batches of 100,000 requests in a world that does not move.

### Cost of one observation

Profile `still`: each resident stands still and observes after each action.
One city of one zone. 10 warm-up frames and 60 measured frames. Each value
is the mean time of one `World::observe` on one thread in microseconds, for
the placement `scatter` and then for `spawn`.

| Residents in the zone | All sections: scatter / spawn | Only `nearby`: scatter / spawn | Only `self`: scatter / spawn |
| --- | --- | --- | --- |
| 200 | 11.75 / 11.92 | 8.02 / 8.26 | 1.16 / 1.22 |
| 2,000 | 13.22 / 13.33 | 8.95 / 8.96 | 1.17 / 1.22 |
| 5,000 | 14.19 / 14.46 | 10.24 / 8.96 | 1.25 / 1.35 |
| 10,000 | 15.79 / 14.29 | 11.12 / 10.13 | 1.43 / 1.36 |
| 20,000 | 18.54 / 16.08 | 14.12 / 11.55 | 1.53 / 1.80 |

Command, with N, the placement and the sections:
`cargo run --release -p unsup-loadtest -- bench --residents N --cities 1 --districts 1 --profile still --placement scatter --frames 60 --warmup 10 --observe-sections nearby`

The load average was 4.4 at the start and 4.3 at the end of these 30 runs.
Each value is one run.

### `soak`

An audit after each tick, and two identical worlds side by side.

| Residents | Zones | Profile | Ticks | Result | Audits passed | Hash comparisons passed | Actions accepted | Rejected at once | Money minted, cents | Money burned, cents | Residents at the end | Ticks for each second | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2,000 | 1 | daily_life | 2,000 | PASSED | 2,001 | 41 | 119,128 | 7,564 (5.97 %) | 1,061,140 | 199,500 | 2,000 | 259 | 8.0 to 7.9 | F |
| 5,000 | 3 | daily_life | 10,000 | PASSED | 10,001 | 201 | 1,010,710 | 60,483 (5.65 %) | 9,906,700 | 3,145,750 | 540 | 120 | 7.5 to 7.0 | L |

Commands:

- F: `cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --verify-determinism --profile daily_life`
- L: `cargo run --release -p unsup-loadtest -- soak --ticks 10000 --residents 5000 --verify-determinism --profile daily_life`

What these runs show:

- Both runs had real money and balanced on each audit. In run F the faucets
  made 1,061,140 cents: 501,140 from `export.sale` and 560,000 from
  `gov.public_works`. The sink `import.purchase` destroyed 199,500 cents.
  861,640 cents were held at the end, and nothing was on the way.
- The 560,000 cents of wages are the public works budget of two game days
  for one city (280,000 for each day: 100 shifts of 2,800). Run F has 2,000
  ticks, and a day has 1,440.
- The two worlds of each run had the same state hash at each comparison.
  The hash at the end of run F is `0aaa8723...`, and that of run L is
  `731ca6ef...`.
- Run L is the longest run of this document: 10,000 ticks are 6.9 game
  days. It has the first city election, the national election and six
  ends of a day.
- In run L, 4,460 of the 5,000 residents left the world before the end,
  and the world dropped 6 messages without a target. The checks passed
  through all of that. The tool does not record why a resident left. The
  goods numbers of the run give the likely cause: the residents got 10,290
  units of vegetables and 488 units of bread in 6.9 game days, which is
  about two units of food for each resident, and they sent 1,626 `eat`
  requests. A resident without food dies after about 122 game hours. See
  finding 9.7.
- The refusals of run F by verb: the world rejected 33.4 % of the `work`
  requests at once with `insufficient`. Of the `work` reports that
  residents read, 98.4 % say `insufficient`: the city pays 100 shifts in
  one day, and 2,000 residents ask for them. The world rejected 11.9 % of
  the `gather` requests at once, and 54.9 % of the `gather` reports say
  `insufficient`. No `move_to` was rejected and no walk failed.
- The tests of the tool prove that the checks find a violation: a test
  plugin that copies money fails the run at the tick of the copy, and a test
  plugin that writes a number from outside the simulation into the state
  fails the determinism check.

### `swarm`

All runs: `--spawn-server` (server and agents in one process, loopback
network, plain HTTP and plain WebSocket, the limits of the gateway off
through the configuration) and a window of 60 seconds. Each agent signed up
with `POST /v1/owners` and `POST /v1/residents`. No request was refused by
a limit of the server in any run.

Runs W and H: log-normal think time with a mean of 5 seconds, one frame for
each second, ramp-up 30 seconds, each agent with a connection of its own.
WebSocket agents used `GET /v1/ws`. HTTP agents waited in the long poll
`GET /v1/wait`.

| Agents | Link | Profile | World (cities x districts) | Result | Ack latency ms: p50 / p95 / p99 / max | Acks for each second | Observations of the agents for each second | Failed requests | Simulation thread at work, at least | Observations of the simulation thread in the window: number, steps, time | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15,000 | WebSocket | daily_life | 1 x 3 | SUSTAINED | 0.18 / 24.82 / 116.29 / 528.38 | 1,031 | 724 | 0 | 18.8 % | 792,419 in 34,054 steps, 8,675 ms | 9.6 to 23.8 | W |
| 15,872 | WebSocket | daily_life | 1 x 3 | SUSTAINED | 0.18 / 28.89 / 124.48 / 657.41 | 1,086 | 763 | 0 | 20.2 % | 844,215 in 35,450 steps, 9,405 ms | 7.6 to 6.3 | W |
| 16,200 | WebSocket | daily_life | not started | REFUSED before the start: the system has 16,384 local ports | not measured | not measured | not measured | not measured | not measured | not measured | 8.8 | W |
| 12,500 | HTTP, long poll | daily_life | 2 x 2 | SUSTAINED | 0.51 / 119.04 / 200.57 / 265.21 | 872 | 317 | 0 | 9.1 % | 548,415 in 531,624 steps, 14,676 ms | 2.3 to 2.7 | H |
| 14,000 | HTTP, long poll | daily_life | 2 x 2 | SUSTAINED | 0.53 / 161.53 / 243.20 / 295.94 | 977 | 355 | 0 | 9.3 % | 625,852 in 603,480 steps, 16,784 ms | 2.3 to 2.4 | H |
| 15,000 | HTTP, long poll | daily_life | 1 x 3 | NOT SUSTAINED | 0.59 / 171.52 / 251.78 / 307.71 | 1,023 | 369 | 0 | 10.3 % | 657,841 in 633,732 steps, 17,418 ms | 3.4 to 3.1 | H |

Runs A: 10,000 HTTP agents with `--acks-only` (`POST /v1/act?observe=false`),
exponential think time, 100 ms for each tick, one city of 2 x 2 districts,
ramp-up 20 seconds.

| Profile | Mean think time, s | HTTP connections | Result | Ack latency ms: p50 / p95 / p99 / max | Acks for each second | Rejected at once (`queue_full`) | Observations of the agents for each second | Failed requests | Simulation thread at work, at least | Observations of the simulation thread in the window: number, steps, time | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| sprint | 0.2 | shared | SUSTAINED | 0.64 / 4.08 / 7.67 / 85.50 | 49,475 | 10.7 % | 993 | 0 | 31.2 % | 59,595 in 48,459 steps, 4,887 ms | 5.0 to 10.5 | A |
| sprint | 0.15 | shared | SUSTAINED | 0.88 / 6.39 / 9.48 / 96.64 | 65,383 | 22.4 % | 1,325 | 1,073 (0.027 %) | 41.7 % | 79,516 in 60,989 steps, 7,823 ms | 7.6 to 14.0 | A |
| sprint | 0.1 | shared | NOT SUSTAINED | 7.62 / 424.96 / 778.75 / 20,971.52 | 51,277 | 26.3 % | 1,041 | 846 (0.027 %) | 33.9 % | 62,547 in 47,258 steps, 6,636 ms | 7.1 to 13.3 | A |
| sprint | 0.1 | per-agent | SUSTAINED | 52.29 / 91.84 / 110.14 / 213.25 | 61,215 | 17.5 % | 1,232 | 0 | 88.5 % | 73,967 in 22,333 steps, 28,683 ms | 8.0 to 38.2 | A |
| sprint, `observe_share` 0 | 0.1 | per-agent | SUSTAINED | 2.36 / 10.02 / 17.77 / 95.55 | 95,377 | 46.0 % | 0 | 0 | 42.6 % | 7 in 6 steps, 1 ms | 8.8 to 18.7 | A |
| sprint, `observe_share` 0 | 0.05 | per-agent | NOT SUSTAINED | 94.27 / 677.38 / 1,110.02 / 16,941.06 | 39,876 | 10.7 % | 7 | 28 (0.001 %) | 21.6 % | 375 in 317 steps, 30 ms | 8.9 to 11.5 | A |

One more run A had 20,000 agents on a shared pool with a mean think time of
0.2 seconds. It stopped in the ramp-up: the system had no free local port.
16,114 agents were ready then (load average 10.1 to 15.3).

Commands (N is the number of agents, T the mean think time, C the
connections, P the profile):

- W: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents N --ws-share 1 --profile daily_life --ramp-up 30 --duration 60`
- H: command W with `--ws-share 0`
- A: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents 10000 --ws-share 0 --http-connections C --acks-only --profile P --think exponential --think-mean T --tick-ms 100 --ramp-up 20 --duration 60`

"sprint, `observe_share` 0" is a copy of `crates/loadtest/profiles/sprint.toml`
in which `observe_share` is 0: the agents never observe after the first
time.

One run had the server in a process of its own: the `unsupervised` binary
with `run --dev --tick-ms 100`, one city of 2 x 2 districts and the same
limits off, and command A with `--url` in place of `--spawn-server`
(sprint with `observe_share` 0, think time 0.1 seconds, per-agent).

| Result | Ack latency ms: p50 / p95 / p99 / max | Acks for each second | Processor use of the server process | Processor use of the swarm process | Load average, start to end |
| --- | --- | --- | --- | --- | --- |
| SUSTAINED | 11.12 / 45.98 / 77.69 / 194.69 | 85,864 | 889 %, 1,009 % and 1,058 % of one core (three samples of `ps`) | 319 %, 311 % and 333 % | 9.9 to 34.3 |

What these runs show:

- **WebSocket.** The largest number of agents that this machine sustained
  is 15,872, with `daily_life` (99th percentile 124.5 ms). The limit is the
  machine, not the server: one client address has 16,384 local ports, and
  the tool refuses a run that needs more than 15,872 connections. The
  simulation thread worked at least 20.2 % of the window.
- **HTTP with the long poll.** The largest number that was sustained is
  14,000 (99th percentile 243.2 ms). 15,000 agents were not sustained: the
  99th percentile was 251.8 ms, 1.8 ms above the target. The limit is the
  server. With 15,000 agents the simulation thread built 657,841
  observations in the window, in 633,732 steps, and that took 17.4 seconds
  of the 60 seconds. The agents asked for at most 83,519 of them (22,156
  observations and 61,363 acknowledged actions). The others are for
  waiting calls: such a call gets an observation each time its resident
  gets a notice, also when the notice is not of the kind that the call
  waits for. See finding 9.2.
- **Actions without an observation.** The largest rate that was sustained
  is 95,377 acknowledgements for each second (99th percentile 17.8 ms),
  with agents that never observe. The world rejected 46.0 % of these
  requests at once with `queue_full`: an agent that sends ten actions in
  one second fills the queue of its resident. The world accepted 51,519
  actions for each second. With the profile `sprint`, whose agents observe
  after one action in fifty, the largest sustained rate is 65,383
  acknowledgements for each second (99th percentile 9.5 ms), of which the
  world accepted 50,742. In each of the four sustained runs A the world
  accepted 44,163 to 51,519 actions for each second: 10,000 residents do
  not do more.
- **Where the action rate stops.** At 85,864 acknowledgements for each
  second the server process used 9 to 11 processor cores and the swarm
  process 3. The load average was above the number of cores. The
  simulation thread worked 42.6 % of the window at 95,377 acknowledgements
  for each second. So the processors of this machine are the limit, and
  most of the work is in the gateway, not in the simulation. See finding
  9.5.
- **A few observations cost much.** The two runs with a think time of 0.1
  seconds and per-agent connections differ only in `observe_share`. With
  1,232 observations of the agents for each second, the simulation thread
  spent 28.7 seconds of the window on 73,967 observations (388 microseconds
  for each), worked 88.5 % of the window, and the rate fell from 95,377 to
  61,215. The steps had 3.3 observations on the average. See finding 9.2.
- **Overload.** With a think time of 0.05 seconds the rate fell to 39,876
  for each second and the 99th percentile was 1,110 ms. The simulation
  thread worked 21.6 % of the window and the load average was 11.5: neither
  was the limit. With a think time of 0.1 seconds on a shared pool, the
  rate changed between 28,792 and 97,334 in one second, and 846 requests
  failed with a connection that the server reset or closed. The cause of
  both was not examined.
- **Local ports.** A shared pool does not protect a run from the limit of
  local ports. The pool opens a connection for each request that is under
  way, and 20,000 agents with a short think time have more requests under
  way than the system has ports. Each agent has at most one request under
  way, so a run with 15,000 agents or fewer cannot reach that limit.
- **WebSocket pushes.** With 15,000 WebSocket agents the simulation thread
  built 792,419 observations in 60 seconds, and the agents asked for only
  43,425 of them. The gateway reads the news of a resident with one
  `observe` call each time a frame has news for a resident with a socket.
  That is 0.88 observations for each socket in one second with
  `daily_life`. These calls come in batches: 23.3 observations in one
  step, 10.9 microseconds for each.

### Kernel and runtime examples

These examples measure parts of the system alone, in one process, without
the network and without the game modules. They are not measurements of the
game.

`cargo run --release -p unsup-kernel --example bench_observe -- nearby`
(load average 5.2): the mean time of one `World::observe` on one thread.

| Place | All kernel sections | Only `nearby` | Only `self` |
| --- | --- | --- | --- |
| Town of 18,000 residents in 9 zones, observers in the zone in the middle | 12.64 us | 11.76 us | 1.26 us |
| The same town, observers in all zones | 15.94 us | 12.34 us | 0.94 us |
| Crowd of 15,000 residents on one tile | 29.56 us | 27.81 us | 1.87 us |

`cargo run --release -p unsup-kernel --example bench_observe -- batch 1000000`,
and the same with `global` (load average 5.1 to 7.0): every resident of a
world of one million residents in 529 zones observes once. The world does
not step between the observations. One call of `World::observe_batch` has
100,000 requests.

| Way | Without observers in the global realm | With 4 observers in the global realm |
| --- | --- | --- |
| `World::observe`, one thread | 49,794 for each second (20.08 us each) | 52,285 (19.13 us) |
| `World::observe_batch`, one thread | 74,248 (13.47 us) | 77,748 (12.86 us) |
| `World::observe_batch`, 18 threads, first round | 655,088 (1.53 us of wall time), 13.2 times `World::observe` | 664,356 (1.51 us), 12.7 times |
| `World::observe_batch`, 18 threads, second round | 744,405 (1.34 us), 14.9 times | 682,440 (1.47 us), 13.1 times |

`cargo run --release -p unsup-kernel --example bench_memory -- 100000 NEWS`
(load average 5.8 to 6.3): 100,000 residents in 64 zones, `kernel.inbox_max`
32. The resident set size for each resident, and the estimate of the kernel.

| State | RSS bytes for each resident | Kernel estimate |
| --- | --- | --- |
| Idle residents, empty inboxes | 956 | 601 |
| One routine and one running action each, empty inboxes | 1,413 | 889 |
| Full inboxes, `mixed`: speech notices of 80 bytes and action reports | 6,599 | 5,529 |
| The same after 64 more frames: every entry replaced | 6,634 | 5,529 |
| Full inboxes, `speech` only | 8,068, and 8,078 after 64 more frames | 7,001 |
| Full inboxes, `reports` only | 4,511, and 4,539 after 64 more frames | 3,737 |
| Empty inboxes, `keys`: the answers to 32 requests with an idempotency key | 6,694 | 4,185 |

`cargo run --release -p unsup-runtime --example throughput` (load average
5.8 to 5.4): 4,096 callers send calls through one `SimHandle` as fast as
they get answers ("saturated"), or at half of that rate. A frame runs each
50 ms. The module of this example has one action that costs the kernel 0.15
microseconds, so these numbers are the limit of the runtime, not of the
game.

| Setting | Calls for each second | p50 ms | p99 ms | Note |
| --- | --- | --- | --- | --- |
| Journal off, saturated | 3,348,432 | 1.19 | 1.55 | Apply 87 % of the simulation thread |
| Journal off, half load | 1,668,967 | 1.66 | 2.75 | |
| `observe`, saturated | 414,655 | 9.68 | 14.89 | Observations 98 % of the simulation thread, 255.9 in one step |
| `observe`, half load | 206,750 | 0.82 | 1.78 | 119.1 in one step |
| Act then observe, saturated | 302,708 | 13.50 | 14.46 | 127.9 in one step |
| Act then observe, half load | 150,968 | 0.92 | 1.79 | 55.7 in one step |
| Journal, sync each 50 ms, saturated | 2,415,201 | 1.50 | 5.38 | |
| Journal, sync each 50 ms, half load | 1,173,881 | 1.26 | 7.25 | |
| Journal, sync each batch, saturated | 64,070 | 64.73 | 77.18 | Commit 87 % of the simulation thread |
| Journal, sync each batch of 4,096, saturated | 495,277 | 8.03 | 13.45 | |
| Journal, sync each record, 64 callers, saturated | 298 | 200.02 | 225.79 | |

The example also printed:

- The kernel alone applied 6,773,490 actions for each second on one thread,
  and built 186,132 observations for each second on one thread (5.37
  microseconds each), with its small module.
- A snapshot of this small world had 586.0 KiB. The simulation thread stood
  still for 0.83 ms, and the write took 10.06 ms.
- With 4,096 callers and a mailbox of 1,024 places, 563,375 calls were
  refused with `overloaded` in 3 seconds, and 3,411,778 calls for each
  second were answered.
- 200,000 waiting calls: a frame with them took 0.214 ms, and a frame
  without them 0.488 ms. A frame that woke 1,999 calls took 33.598 ms: 16.8
  microseconds for each call.

Not explained:

- In the bench runs, the kernel estimate of the memory for each resident
  (5,135 to 8,788 bytes) is smaller than the RSS for each resident (10,033
  to 13,154 bytes).
- The bench runs with a batch of 256 need 12 to 23 microseconds of the
  calling thread for one observation. The kernel example needs 1.3 to 1.5
  microseconds with batches of 100,000 in a world that does not move. The
  runs do not tell how much of the difference is the size of the batch,
  the number of requests for each zone, or the steps of the world between
  the batches.

## 9. Findings

The first measurement of this tool gave three findings. Since then the
kernel keeps an inbox as packed bytes, finds the nearest residents without
a sort of all residents in sight, puts newcomers on different tiles of a
spawn site, and builds the observations of a batch on all cores. The
runtime answers the `observe` calls of one batch of its mailbox in one
step, and `POST /v1/act?observe=false` exists. This section gives the state
of the three findings after these changes, and four new findings. The
numbers are those of section 8.

### 9.1 The cost of the `nearby` section

State: smaller, not closed. The cost still grows with the number of
residents in the zone, but much less.

- With 200 residents in a zone, one observation costs 11.8 to 11.9
  microseconds, and `nearby` alone 8.0 to 8.3. With 20,000 residents in a
  zone it costs 16.1 to 18.5 microseconds, and `nearby` alone 11.6 to 14.1.
  Before the changes, 20,000 residents in a zone cost 62 to 91
  microseconds.
- A crowd of 15,000 residents on one tile costs 29.6 microseconds for one
  observation in the kernel example, of which `nearby` is 27.8.
- The `self` section alone stays between 1 and 2 microseconds.
- In the bench run with one million `daily_life` residents, one zone had
  19,103 residents, and one observation cost 48.5 microseconds on one
  thread on the average of the whole world. Before the changes it was 80.8
  microseconds with 120 frames.

Request that is left: none for the target. The answer has at most
`kernel.nearby_max` residents (12), and the cost for 20,000 residents in a
zone is now 1.4 to 1.6 times the cost for 200.

### 9.2 One simulation thread serves all observations

State: closed in part. One thread still applies the inputs and runs the
frames. The observations of the `observe` calls of one batch of calls are
now built in one step on all cores. The observation of a waiting call is
still built alone.

What the batches give:

- In one process, with batches of 256, the observations of one frame of one
  million residents took 169 ms, not 495 ms (`walkers`), and 93 ms, not
  248 ms (`daily_life`). The work of one thread for one frame fell from
  1,013 ms to 667 ms and from 766 ms to 598 ms.
- Before the changes that work was 858 ms to 1,744 ms. A server that steps
  one frame for each second can now do the work of 95 of 100 frames in time
  at this load, on this machine.
- In the runtime example, 4,096 callers got 414,655 observations for each
  second, with 255.9 observations in one step.

What is still open:

- **Small batches cost more than no batch.** A batch of 2 to 16 requests
  needs 40 to 67 microseconds for one observation in a world with 4 zones,
  where one thread needs 23. Over the network the steps are small: 1.2 to
  3.3 observations in one step in the runs with `--acks-only`, where one
  observation cost 82 to 388 microseconds of the simulation thread. With
  1,232 observations for each second the simulation thread worked 88.5 %
  of the window, and without them 42.6 %.
- **The long poll.** A waiting call gets an observation, alone, each time
  its resident gets a notice. The runtime then looks whether the notice is
  of a kind that the call waits for. With 15,000 HTTP agents that wait for
  reports, the simulation thread built 657,841 observations in 60 seconds,
  which took 17.4 seconds. At most 83,519 of them were for the agents. The
  long polls of the whole run got 41,303 answers. The work for the waiting
  calls comes after each frame, before the thread takes the next call: the
  95th percentile of the acknowledgement latency was 171.5 ms. The runtime
  now counts this time (`observe_micros`), but it is in no other counter.
- **WebSocket pushes.** Each socket costs one `observe` call for each frame
  with news for its resident: 0.88 observations for each socket in one
  second with `daily_life`. With 15,000 sockets that is 8.7 seconds of the
  simulation thread in 60 seconds. For one million sockets it would be
  more than the thread has. That number is a projection, not a
  measurement.
- **The inputs.** `World::apply` runs on the one thread. With one million
  `walkers` the inputs of one frame take 407 ms, more than the frame
  itself (92 ms) and the observations (169 ms).

Requests:

- Build a batch of observations on the calling thread when it is small or
  when all its residents are in few zones. The numbers of section 8 put the
  limit near 64 requests for 4 zones.
- Decide from the notice whether a waiting call has news, and build the
  observation only for a call that gets an answer. Build the observations
  of all calls that one frame wakes in one step.
- Give a socket its news without a full `observe` call for each frame.

### 9.3 Memory for each resident

State: closed for the target on this machine.

- One million residents ran for 360 frames with both profiles. The process
  held 9.8 GiB to 12.3 GiB (RSS). Before the changes, one million
  residents did not fit in the 17 to 18 GiB that were free.
- With `walkers` and inboxes that are 56 % to 58 % full, a resident costs
  10,033 to 11,507 bytes of RSS (7,564 to 8,890 bytes of footprint). Before
  the changes the footprint was 21,400 to 26,500 bytes.
- With `daily_life` and full inboxes, a resident costs 10,551 to 13,154
  bytes of RSS (8,994 to 10,634 bytes of footprint). Before the changes the
  footprint was 11,600 to 17,000 bytes.
- In the kernel example, a full inbox costs 3,554 to 7,122 bytes for each
  resident, and the memory does not grow when every entry is replaced:
  6,599 bytes and 6,634 bytes for each resident.

What is still open:

- The estimate of the kernel (`World::memory_estimate`) is 5,135 to 8,788
  bytes for each resident in the bench runs, about half to two thirds of
  the RSS.
- The memory of one million residents in a world that has run for more
  than 360 frames was not measured.

### 9.4 The frame of the leaderboards (new)

Once in 60 frames, one frame of a world with one million residents takes
2.4 to 2.6 seconds. With one frame for each second, the world is then more
than one second late, each game hour.

- Each zone reports its residents to the leaderboards of the missions
  module in the same frame, and the global realm takes all reports in that
  frame. With 100,000 residents the frame takes 182 ms. Without the
  reports it takes 16 ms (section 8, "The frame of the leaderboards").
- The world of the first measurement did not have the missions module. Its
  worst frame with one million residents was 144.5 ms.

Request for the missions module: do not let all zones report in one frame
(the kernel has an offset for periodic systems: the zone number), and
bound the work of the global realm for the boards in one frame.

### 9.5 The gateway uses most of the processors (new)

Over HTTP without observations, the largest sustained rate was 95,377
acknowledgements for each second. The simulation thread worked 42.6 % of
that window. The limit was the processors of the machine.

- With the server in a process of its own, the server used 9 to 11
  processor cores at 85,864 acknowledgements for each second. That is 104
  to 123 microseconds of processor time for one request. The swarm used 3
  cores.
- One request costs the simulation 3 to 8 microseconds in `World::apply`.
- A sample of the server process for 5 seconds (`sample`) showed its
  threads mostly in wait and wake-up calls of the system, under two call
  paths: the blocking thread pool of the async runtime, which the gateway
  uses for each call to the store, and the lock of the store in memory in
  `authenticate`. This is one sample of one run. It is a hint, not a
  proof. The store in a SQLite file was not measured.

Request for the gateway and the store: check a key without a hand-over to
another thread and without one lock for all requests, for example with a
cache of the keys that were checked.

### 9.6 Limits of the load machine (new)

- One client address has 16,384 local ports. No run can have more than
  15,872 agents with a connection of their own.
- A shared pool of connections opens one connection for each request that
  is under way. A run with 20,000 agents and a short think time stopped at
  the limit of local ports for that reason.
- The server accepts connections from a queue of 128 places
  (`kern.ipc.somaxconn`). A burst of new connections gets resets: 846 and
  1,073 failed requests in two runs with a shared pool.
- With a think time of 0.05 seconds the rate fell to 39,876
  acknowledgements for each second while the simulation thread and the
  processors had time left. The cause was not examined. It can be in the
  swarm or in the gateway.

A larger load needs agents on more than one machine, or more than one
address of the server.

### 9.7 A crowd of newcomers cannot feed itself (new)

In the long soak, 4,460 of 5,000 `daily_life` residents left the world in
6.9 game days. This is a finding about the game and about the profile, not
about speed.

- A city pays 100 shifts of public works each day, and the 1,667 residents
  of each city asked for them: 68,951 of the 70,306 `work` reports that
  residents read say that the action failed (98.1 %), 68,904 of them with
  `insufficient`.
- The residents got about two units of food each in the whole run.
- The profile has no jobs of other residents and no business. So its
  residents have no other income.

Consequences:

- A `soak` or a `bench` run of more than about five game days measures a
  world that gets empty.
- A new world with thousands of newcomers in each city, as the scale target
  needs, has the same economy. The run does not tell whether real agents
  would do better.

## 10. State of the targets

"Shows" means that a measurement of section 8 reached the target. "Shows
in part" means that a measurement reached it in a setting that is not the
whole target. "Does not show" means that no measurement reached it.

| Target | Setting | What was measured | State |
| --- | --- | --- | --- |
| 1,000,000 residents at the same time | One process, without the network | One million residents ran for 360 frames with `walkers` and with `daily_life`. Memory: 9.8 GiB to 12.3 GiB of RSS. Median frame: 50.8 ms to 154.0 ms. The work of one thread for one frame, with observations in batches of 256: 598 ms to 667 ms (mean), 682 ms to 817 ms (95th percentile). One frame in 60 took 2.4 to 2.6 seconds | Shows in part: the world fits in memory and most frames fit in one second, but one frame in each game hour does not |
| | Loopback network | The largest swarm had 15,872 agents (WebSocket). One client machine has 16,384 local ports | Does not show |
| 200,000 actions for each second | One process, without the network | 170,738 accepted actions for each second of wall time, and 199,650 for each second of server work, with one million residents that take short walks (`rush`), for 40 frames without a frame of the leaderboards. With `walkers`: 71,927. With `daily_life`: 18,503 | Shows in part: the rate is 85 % of the target in wall time, with no network, no journal, a short run and a load of short walks |
| | Loopback network, `POST /v1/act?observe=false` | 95,377 acknowledgements for each second from 10,000 agents that never observe, of which the world accepted 51,519. 65,383 acknowledgements for each second with one observation for each 50 actions. The server process used 9 to 11 of the 18 cores at 85,864 acknowledgements for each second | Does not show: 48 % of the target, and the world rejected 46 % of those requests with `queue_full` |
| Acknowledgement below 250 ms at the 99th percentile | Loopback network, server and agents on one machine | WebSocket: 124.5 ms with 15,872 `daily_life` agents (1,086 acknowledgements for each second). HTTP with the long poll: 243.2 ms with 14,000 agents, 251.8 ms with 15,000. HTTP without observations: 17.8 ms at 95,377 acknowledgements for each second, and 1,110 ms when the agents sent twice as fast | Shows in part: up to 15,872 WebSocket agents, up to 14,000 HTTP agents with the long poll, and up to 95,377 acknowledgements for each second. No run had more agents |

What the measurements say about the distance to the targets. These lines
are conclusions, not measurements.

- **Memory** is no longer the limit for one million residents on a machine
  with 48 GiB.
- **The frame** fits, apart from the frame of the leaderboards (finding
  9.4).
- **The simulation thread** has room at the measured loads, but not for one
  million connected agents: the observations for long polls and for
  sockets are built for each resident with news (finding 9.2).
- **The action rate** over the network is limited by the processor time of
  the gateway for one request (finding 9.5), not by the simulation.
- **The number of agents** in a test is limited by the load machine
  (finding 9.6). A test of the scale target needs several load machines.

Not measured:

- A machine that does nothing else.
- Linux.
- A server and a swarm on different machines, and more than one swarm
  machine. No run had more than 15,872 agents with a connection of their
  own, or more than 16,114 agents in all.
- TLS. This build of the swarm sends plain HTTP only. No number of this
  document is for HTTPS.
- The server with its world on the disk. Each `swarm` run had the journal
  and the accounts in memory. The runtime example has numbers for the
  journal alone.
- A foreign server with its limits on, at scale. The tests of the tool run
  a small swarm against a server with limits and check the `429` class.
- One million residents for more than 360 frames, and their memory in a
  world that has run for a long time.
- The largest number of HTTP agents that walk without the long poll, and a
  swarm with both links.
- The parts of the game that `daily_life` does not use (section 3).
- A window longer than 60 seconds over the network.
- The MCP endpoint and webhooks.
- The measured rate of actions of the profile `sprint` in one process.
