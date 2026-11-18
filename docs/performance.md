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
of each target. Section 9 has the open findings.

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
finance and society modules, the runtime and the gateway.

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
| `daily_life` | The full game for a newcomer. See below | One request in 32 to 64 ticks |
| `still` | Stand still and observe after each action. Use it to measure the cost of one observation | One action in 4 ticks |

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
| `observe` | Time to build the observations of one frame through `World::observe`, on one thread |
| `server` | `inputs + observe + frame`: the work of the simulation thread of a server for one frame |
| `generator` | Time that the tool itself needs to choose actions. A server does not do this work |
| mean, p50, p99, worst | Mean, median, 99th percentile and largest value over the measured frames. The percentiles are exact (nearest rank) |
| actions accepted, rejected | Requests that the world accepted or rejected in the measured frames |
| actions for each frame | Accepted actions divided by the measured frames |
| actions for each second of wall time | Accepted actions divided by the wall time of the measured loop. The loop includes the time of the generator |
| actions for each second of server work | Accepted actions divided by the sum of `server` |
| one request costs, one observation costs | The sum of `inputs` divided by the requests, and the sum of `observe` divided by the observations, in microseconds |
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
- A share of the residents (`observe_share`) observes after an action. Over
  HTTP, each answer of `POST /v1/act` and each answer of the long poll has
  an observation. Thus a server builds more observations for each action
  than `bench` does with the built-in profiles.

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
   WebSocket. The tool measures the acknowledgement latency.
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
| simulation thread | With `--spawn-server`: the share of the window in which the one simulation thread worked, from the counters of the runtime: frames, inputs and observations between frames, and the calls themselves. The runtime does not count the answers to long polls after a frame, so the real share is larger |

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

This section has only numbers that were measured in this worktree. All other
numbers are **not measured**.

### Conditions

| Item | Value |
| --- | --- |
| Date | 2026-11-18 (UTC) |
| Machine | Apple M5 Pro, 18 logical cores, 48 GiB memory, macOS 27.2 |
| Build | Release. 18 zone threads |
| Commit | `566b75e` with the local changes of the load tools. The kernel, the modules, the runtime and the gateway are those of `566b75e` |
| World | The full game: life, economy, finance and society. 88 verbs |
| Seed | 1 |

Other programs used the machine during all measurements. The load average
was between 3 and 65 with 18 logical cores, and each table gives the load
average of each run. Thus most times in this section are too high, and the
values near a limit change from run to run. Measure again on a machine that
does nothing else before you compare with a target.

### `bench`

Placement `scatter`. One tick for each frame. "us" is microseconds.

| Residents | Profile | Zones | Frames (warm-up + measured) | Frame ms: mean / p50 / p99 / worst | Inputs ms, mean | Observe ms, mean | Server ms: mean / worst | Actions for each frame | Actions for each second of wall time | Rejected at once | us for one request / one observation | Footprint at the end | Footprint bytes for each resident | Inboxes full | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10,000 | walkers | 4 | 60 + 300 | 0.94 / 0.94 / 1.27 / 1.32 | 3.54 | 5.70 | 10.18 / 15.33 | 523 | 49,946 | 0.00 % | 6.78 / 43.56 | 213 MiB | 21,446 | 57 % | 25.5 to 13.5 | A |
| 100,000 | walkers | 49 | 60 + 300 | 6.07 / 5.81 / 11.46 / 31.08 | 39.51 | 84.24 | 129.83 / 153.90 | 5,183 | 38,505 | 0.00 % | 7.62 / 64.90 | 2,391 MiB | 24,982 | 58 % | 25.5 to 13.5 | A |
| 250,000 | walkers | 128 | 60 + 300 | 13.12 / 12.23 / 28.12 / 46.14 | 91.89 | 192.36 | 297.37 / 458.07 | 12,971 | 42,058 | 0.00 % | 7.08 / 59.31 | 6,071 MiB | 25,426 | 57 % | 17.0 to 15.7 | B |
| 500,000 | walkers | 256 | 60 + 300 | 29.69 / 26.56 / 69.97 / 122.96 | 210.92 | 479.28 | 719.90 / 2,360.36 | 25,942 | 34,529 | 0.00 % | 8.13 / 73.83 | 12,621 MiB | 26,450 | 57 % | 17.0 to 15.7 | B, under memory pressure |
| 1,000,000 | walkers | 512 | 60 + 300 | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | 25.5 to 13.5 | A |
| 1,000,000 | walkers | 512 | 20 + 100 | 57.00 / 52.78 / 101.38 / 144.47 | 427.80 | 829.92 | 1,314.72 / 2,024.93 | 52,240 | 37,783 | 0.00 % | 8.19 / 63.53 | 9,224 MiB | 9,662 | 18 % | 64.7 to 17.0 | C |
| 1,000,000 | rush | 512 | 10 + 40 | 75.69 / 71.54 / 153.94 / 153.94 | 556.85 | 1,111.50 | 1,744.04 / 3,366.14 | 170,323 | 88,902 | 0.00 % | 3.27 / 65.36 | 13,265 MiB | 13,899 | 25 % | 17.5 to 10.6 | R |
| 10,000 | daily_life | 4 | 60 + 300 | 5.45 / 4.22 / 17.36 / 28.02 | 4.85 | 11.29 | 21.60 / 75.32 | 290 | 12,854 | 7.19 % | 15.52 / 95.30 | 120 MiB | 11,595 | 100 % | 15.7 to 11.7 | D |
| 100,000 | daily_life | 49 | 60 + 300 | 26.50 / 27.93 / 42.63 / 52.50 | 36.39 | 87.68 | 150.57 / 436.49 | 1,467 | 9,114 | 5.70 % | 23.39 / 151.71 | 1,627 MiB | 16,968 | 100 % | 15.7 to 11.7 | D |
| 1,000,000 | daily_life | 512 | 60 + 300 | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | not measured | 15.7 to 11.7 | D |
| 1,000,000 | daily_life | 512 | 20 + 100 | 139.61 / 134.90 / 197.19 / 207.00 | 202.15 | 516.36 | 858.12 / 1,604.75 | 17,564 | 18,341 | 0.50 % | 11.45 / 80.83 | 13,415 MiB | 14,057 | 99 % | 11.7 to 17.5 | E |

Commands:

- A: `cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000`
- B: `cargo run --release -p unsup-loadtest -- bench --scale 250k,500k`
- C: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 100 --warmup 20`
- R: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 40 --warmup 10 --profile rush`
- D: `cargo run --release -p unsup-loadtest -- bench --scale 10000,100000,1000000 --profile daily_life`
- E: `cargo run --release -p unsup-loadtest -- bench --residents 1M --frames 100 --warmup 20 --profile daily_life`

What these runs show:

- One million residents with 360 frames did not fit in the free memory of
  the machine. Command A stopped that size after 257 of 360 frames: the
  process held 18.2 GiB, and the limit was 18.1 GiB (90 % of the memory that
  was free at the start). Command D stopped it after 222 of 360 frames at
  17.0 GiB with a limit of 16.9 GiB. The largest size that ran with 360
  frames is 500,000 residents, and the system compressed or swapped 935 MiB
  of that run, so its times are too high.
- One million residents ran for 120 frames (commands C and E) and for 50
  frames (command R). With `walkers` the mean frame time was 57.0 ms and the
  worst frame was 144.5 ms. With `daily_life` the mean was 139.6 ms and the
  worst was 207.0 ms.
- The world rejected no request of `walkers` and `rush`, and every walk
  whose report a resident read reached its target. Each walk target is a
  walkable tile from the map.
- `inputs` and `observe` run on one thread. One request costs 6.8 to 8.2
  microseconds in `World::apply` with `walkers`, where a walk needs a path
  search. One observation costs 44 to 74 microseconds with `walkers` and 81
  to 152 microseconds with `daily_life`. With `walkers`, one action in four
  is followed by an observation. With `daily_life`, one in two.
- The largest action rate was 88,902 accepted actions for each second of
  wall time (run R). With `daily_life` it was 18,341 (run E).
- With one million residents, the work of the simulation thread for one
  frame was 1,315 ms (mean) with `walkers`, 858 ms with `daily_life` and
  1,744 ms with `rush`. A server that steps one frame for each second cannot
  do this work in time on this machine in this state. The observations take
  about 60 % of that work.
- With `daily_life`, the fullest zone had 16,420 of 100,000 residents: the
  residents of a city walk to its one depot and its one city hall.

### Cost of one observation

Profile `still`: each resident stands still and observes after each action.
One city of one zone. 10 warm-up frames and 60 measured frames. Each value
is the mean time of one `World::observe` in microseconds, for the placement
`scatter` and then for `spawn`.

| Residents in the zone | All sections: scatter / spawn | Only `nearby`: scatter / spawn | Only `self`: scatter / spawn |
| --- | --- | --- | --- |
| 200 | 11.17 / 13.02 | 8.41 / 9.87 | 1.07 / 1.14 |
| 2,000 | 18.41 / 19.19 | 18.04 / 15.65 | 1.45 / 1.56 |
| 5,000 | 34.06 / 27.65 | 28.65 / 22.98 | 1.76 / 2.67 |
| 10,000 | 53.39 / 44.55 | 50.02 / 38.48 | 2.92 / 2.19 |
| 20,000 | 91.21 / 61.78 | 87.13 / 63.45 | 2.11 / 2.42 |

Command, with N, the placement and the sections:
`cargo run --release -p unsup-loadtest -- bench --residents N --cities 1 --districts 1 --profile still --placement scatter --frames 60 --warmup 10 --observe-sections nearby`

The load average was between 6 and 18. A first run of 10,000 residents
with all sections and `scatter` gave 101.79 microseconds, so one value can
be wrong by a factor of two.

The same profile in a city of more zones, with all sections:

| Residents | Zones | scatter | spawn |
| --- | --- | --- | --- |
| 20,000 | 9 | 42.49 | 43.62 |
| 100,000 | 49 | 44.54 | 55.36 |

### `soak`

An audit after each tick. One city.

| Residents | Zones | Profile | Placement | Ticks | Determinism check | Result | Audits passed | Hash comparisons passed | Actions accepted | Rejected at once | Money minted, cents | Money burned, cents | Ticks for each second | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2,000 | 1 | daily_life | scatter | 2,000 | yes | PASSED | 2,001 | 41 | 126,186 | 7,647 (5.71 %) | 919,300 | 149,000 | 221 | 10.6 to 9.8 | F |
| 2,000 | 1 | daily_life | scatter | 2,000 | no | PASSED | 2,001 | not measured | 126,186 | 7,647 (5.71 %) | 919,300 | 149,000 | 525 | 9.8 to 9.5 | G |
| 2,000 | 1 | daily_life | spawn | 2,000 | yes | PASSED | 2,001 | 41 | 125,984 | 7,766 (5.81 %) | 922,520 | 136,750 | 233 | 9.5 to 8.6 | H |
| 20,000 | 9 | daily_life | scatter | 2,000 | yes | PASSED | 2,001 | 41 | 907,993 | 53,700 (5.58 %) | 3,423,460 | 144,250 | 14 | 8.6 to 8.2 | I |
| 2,000 | 1 | walkers | scatter | 2,000 | yes | PASSED | 2,001 | 41 | 207,309 | 0 | 0 | 0 | 259 | 8.2 to 8.0 | J |

Commands:

- F: `cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --verify-determinism --profile daily_life`
- G: command F without `--verify-determinism`
- H: command F with `--placement spawn`
- I: command F with `--residents 20000`
- J: `cargo run --release -p unsup-loadtest -- soak --ticks 2000 --residents 2000 --verify-determinism`

What these runs show:

- Run F had real money and balanced on each of the 2,001 audits. The
  faucets made 919,300 cents: 519,300 from `export.sale` and 400,000 from
  `gov.public_works`. The sink `import.purchase` destroyed 149,000 cents.
  770,300 cents were held at the end, and nothing was on the way.
- The 400,000 cents of wages are the public works budget of two game days
  for one city (200,000 for each day). The run has 2,000 ticks, and a day
  has 1,440.
- Run F produced 5,379 units of 7 goods and consumed 4,635. 744 units were
  held at the end.
- The two worlds of run F had the same state hash at each of the 41
  comparisons. The runs F and G end with the same state hash
  (`739abe7a...`).
- The refusals of run F by verb: the world rejected 32.8 % of the `work`
  requests at once with `insufficient`. Of the `work` reports that
  residents read, 98.4 % say `insufficient`: the request waited in the
  queue behind the walk, and the world refused it at its turn. The city
  pays 100 shifts in one day, and 2,000 residents ask for them. The world
  rejected 11.6 % of the `gather` requests at once, and 55.3 % of the
  `gather` reports say `insufficient`. The run does not tell which rule
  refused them: an empty node or too little energy. No `move_to` was
  rejected and no walk failed.
- The tests of the tool prove that the checks find a violation: a test
  plugin that copies money fails the run at the tick of the copy, and a test
  plugin that writes a number from outside the simulation into the state
  fails the determinism check.

### `swarm`

All runs: `--spawn-server` (server and agents in one process, loopback
network, the limits of the gateway off through the configuration),
log-normal think time with a mean of 5 seconds, a window of 60 seconds, one
frame for each second. Each agent signed up with `POST /v1/owners` and
`POST /v1/residents`. WebSocket agents used `GET /v1/ws`. HTTP agents waited
in the long poll `GET /v1/wait`. No request was refused by a limit of the
server in any run.

| Agents | Link | Profile | World (cities x districts) | HTTP connections | Ramp-up s | Result | Ack latency ms: p50 / p95 / p99 / max | Acks for each second | Observations for each second | Failed requests | Simulation thread at work, at least | Load average, start to end | Command |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 5,000 | WebSocket | daily_life | 3 x 1 | none | 30 | SUSTAINED | 0.12 / 0.80 / 9.29 / 44.26 | 366 | 265 | 0 | 3.6 % | 7.4 to 7.6 | W |
| 5,000 | WebSocket | daily_life | 3 x 1 | none | 30 | SUSTAINED | 0.11 / 0.32 / 6.36 / 50.62 | 366 | 265 | 0 | 2.9 % | 6.2 to 5.0 | W |
| 10,000 | WebSocket | daily_life | 1 x 2 | none | 30 | SUSTAINED | 0.12 / 19.60 / 64.06 / 491.77 | 703 | 497 | 0 | 10.4 % | 25.4 to 9.7 | W |
| 15,000 | WebSocket | daily_life | 1 x 3 | none | 30 | SUSTAINED | 0.15 / 36.06 / 132.09 / 800.25 | 1,032 | 724 | 0 | 15.3 % | 6.4 to 8.8 | W |
| 15,800 | WebSocket | daily_life | 1 x 3 | none | 30 | STOPPED after 1 s of the window: no free local port | not measured | not measured | not measured | not measured | not measured | 7.0 to 18.0 | W |
| 15,000 | WebSocket | walkers | 1 x 3 | none | 30 | SUSTAINED | 0.10 / 0.49 / 3.85 / 60.83 | 1,376 | 1,347 | 0 | 14.2 % | 20.2 to 14.3 | W |
| 5,000 | HTTP | daily_life | 3 x 1 | per-agent | 30 | SUSTAINED | 0.37 / 9.79 / 83.58 / 168.45 | 363 | 135 | 0 | 3.4 % | 14.1 to 15.7 | H |
| 6,000 | HTTP | daily_life | 3 x 1 | per-agent | 30 | SUSTAINED | 0.27 / 15.21 / 94.72 / 157.95 | 436 | 162 | 0 | 3.0 % | 4.9 to 3.2 | H |
| 7,500 | HTTP | daily_life | 1 x 2 | per-agent | 30 | NOT SUSTAINED | 0.36 / 198.14 / 342.53 / 463.10 | 524 | 190 | 0 | 5.1 % | 8.2 to 9.6 | H |
| 10,000 | HTTP | daily_life | 1 x 2 | per-agent | 30 | NOT SUSTAINED | 70.02 / 1,243.13 / 2,007.04 / 3,108.86 | 664 | 245 | 0 | 12.6 % | 25.5 to 33.5 | H |
| 15,000 | HTTP | daily_life | 1 x 3 | per-agent | 30 | NOT SUSTAINED | 48.06 / 744.96 / 967.68 / 1,546.24 | 1,022 | 398 | 0 | 11.2 % | 23.7 to 18.6 | H |
| 15,000 | HTTP | daily_life | 1 x 3 | per-agent | 30 | NOT SUSTAINED | 65.79 / 1,069.06 / 3,084.29 / 3,938.30 | 990 | 382 | 0 | 13.4 % | 19.5 to 24.6 | H |
| 20,000 | HTTP | daily_life | 1 x 3 | shared | 60 | STOPPED after 7 s of the window: no free local port | not measured | not measured | not measured | not measured | not measured | 8.6 to 11.8 | S |
| 10,000 | HTTP, no wait | daily_life with `wait_share = 0` | 1 x 2 | per-agent | 30 | SUSTAINED | 0.69 / 3.58 / 7.11 / 92.99 | 2,435 | 1,009 | 0 | 34.4 % | 17.1 to 10.6 | N |
| 10,000 | half WebSocket, half HTTP | daily_life | 1 x 2 | per-agent | 30 | NOT SUSTAINED | 5.05 / 705.53 / 1,880.06 / 2,650.11 | 697 | 372 | 0 in the window, 53 in the ramp-up | 13.8 % | 11.7 to 21.1 | M |
| 15,000 | HTTP | walkers | 1 x 3 | per-agent | 30 | SUSTAINED | 0.69 / 31.05 / 85.95 / 305.41 | 1,360 | 340 | 0 | 20.0 % | 21.6 to 21.3 | H |
| 25,000 | HTTP | walkers | 3 x 2 | shared | 60 | SUSTAINED | 2.62 / 125.31 / 186.88 / 287.74 | 2,221 | 556 | 0 | 40.9 % | 3.9 to 4.9 | S |
| 30,000 | HTTP | walkers | 1 x 4 | shared | 60 | STOPPED after 30 s of the window: no free local port, and no acknowledgement in the window | not measured | not measured | not measured | not measured | not measured | 4.0 to 6.9 | S |

Commands (N is the number of agents, P is the profile). The world is the one
that the tool chooses:

- W: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents N --ws-share 1 --profile P --ramp-up 30 --duration 60`
- H: command W with `--ws-share 0`
- S: `cargo run --release -p unsup-loadtest -- swarm --spawn-server --agents N --ws-share 0 --http-connections shared --profile P --ramp-up 60 --duration 60`
- M: command W without `--ws-share`
- N: command H with a copy of `daily_life.toml` in which `wait_share` is 0

What these runs show:

- With the WebSocket, the largest number of agents that this machine
  sustained for 60 seconds is 15,000, with `daily_life` (99th percentile
  132.1 ms) and with `walkers` (3.9 ms). The limit is the machine, not the
  server: one macOS client address has 16,384 local ports, and 15,800
  agents stopped at that limit.
- Without the WebSocket, the largest number with `daily_life` is 6,000 (99th
  percentile 94.7 ms). 7,500 agents were not sustained (342.5 ms). With
  `walkers` it is 25,000 on a shared pool of connections (186.9 ms). 30,000
  agents stopped at the limit of local ports.
- The same load is slower over HTTP than on the WebSocket. 15,000 agents
  with `daily_life` made about 1,000 acknowledgements for each second on
  both links. The 99th percentile was 132.1 ms on the WebSocket and 967.7 ms
  and 3,084.3 ms over HTTP.
- Run N is run H with 10,000 agents that do not wait for the end of an
  action. It made 3.7 times the acknowledgements (2,435 for each second) and
  its 99th percentile was 7.1 ms, not 2,007.0 ms. This fits the long poll
  as the cause of the slow HTTP runs. It is one pair of runs, and the load
  average was higher during run H. In run N the world rejected
  29.8 % of the requests, most of them with `queue_full`: an agent that does
  not wait fills the queue of its resident.
- The acknowledgement rate of all runs follows the number of agents and the
  think time of 5 seconds. The largest was 2,435 for each second. These runs
  do not show the largest action rate of the server.
- A result near the limit depends on the other programs on the machine. The
  two HTTP runs with 15,000 agents differ by a factor of three at the 99th
  percentile.

Not explained:

- One more run of command H with 10,000 agents used a copy of `daily_life`
  without `say`, `message` and `post`. 670 agents gave up after timeouts and
  the 99th percentile was 1,816.6 ms. The load average went from 20.7 to
  37.5 during that run. The run is not in the table, and nothing follows
  from it.
- The run with half WebSocket and half HTTP agents had 53 failed requests
  in the ramp-up (41 `connect`, 12 `transport`) and none in the window. No
  other run with 10,000 agents had them. The cause was not examined.
- In the bench runs with `daily_life`, the kernel estimate of the memory
  for each resident (19,000 bytes) is larger than the measured footprint
  for each resident (11,600 to 17,000 bytes).

## 9. Open findings

The first measurement of this tool, against a world with four verbs, gave
three requests for the kernel and the runtime. All three are still open.
The numbers below are those of section 8.

### 9.1 The cost of the `nearby` section

The `nearby` section is most of the cost of an observation, and its cost
grows with the number of residents in the zone.

- With 200 residents in a zone, one observation costs 11 microseconds, and
  `nearby` alone 8. With 20,000 residents in a zone it costs 62 to 91
  microseconds, and `nearby` alone 63 to 87. The `self` section alone stays
  between 1 and 3 microseconds.
- The answer has at most `kernel.nearby_max` residents (12). The cost grows
  when the answer does not.
- A city has one depot and one city hall. In the bench run with 100,000
  `daily_life` residents, one zone of 49 had 16,420 residents, and one
  observation cost 152 microseconds on the average of the whole world.
- The first measurement had 24 to 40 microseconds for one observation, and
  3.4 seconds median latency with 15,000 agents on one tile. The game now
  has spawn sites, so that case did not occur again.

Request: make the cost of `nearby` depend on the size of the answer, not on
the number of residents around the resident.

### 9.2 One simulation thread serves all observations

One thread applies the inputs, builds each observation and runs the frames.
`World::observe` needs only `&World`, but the runtime calls it on that
thread.

- In `bench` with one million residents, the work of that thread for one
  frame was 858 ms to 1,744 ms (mean), of which the frame itself was 57 ms
  to 140 ms. The observations were 516 ms to 1,112 ms of it.
- Over the network, the thread worked at least 40.9 % of the window with
  25,000 HTTP agents that walk, at 2,221 acknowledgements for each second.
- The long poll makes it worse. The runtime answers the waiting calls of a
  frame on the simulation thread, each with a full observation, before the
  thread takes the next call (`crates/runtime/src/sim.rs`, `wake`). The
  runtime does not count that time. The measurement fits: the same 10,000
  HTTP agents had a 99th percentile of 2,007.0 ms with the long poll and
  7.1 ms without it. No measurement here isolates the time of `wake`.

Request: build observations outside the simulation thread, between two
frames, and count the time of the long poll answers in the statistics.

### 9.3 Memory for each resident

- With `walkers` and inboxes that are 57 % full, a resident costs 21,400 to
  26,500 bytes (footprint). With `daily_life` and full inboxes, a resident
  costs 11,600 to 17,000 bytes after 360 frames.
- One million residents did not fit in the memory that was free on this
  machine (about 19 to 20 GiB of 48 GiB). The `walkers` run stopped at
  18.2 GiB after 257 frames and the `daily_life` run at 17.0 GiB after 222
  frames. Both still grew.
- After 120 frames, one million `daily_life` residents had an RSS of
  14.1 GiB with inboxes that were 99 % full. The same world held 17.0 GiB
  after 222 frames. So the memory still grows when the inboxes are full.
  The cause was not measured. The memory of one million residents in a
  world that has run for a long time was not measured.

Request: lower the memory of an inbox entry and of a resident, and give
`World::memory_estimate` the parts that it does not count.

## 10. State of the targets

| Target | State | What was measured |
| --- | --- | --- |
| 1,000,000 residents | Not shown | One million residents ran in one process for 120 frames with `walkers` and with `daily_life`, and for 50 frames with `rush`. The mean frame time was 57.0 ms, 139.6 ms and 75.7 ms. The worst frame was 144.5 ms, 207.0 ms and 153.9 ms. A run of 360 frames did not fit in the free memory of the machine. The work of the simulation thread for one frame was 858 ms to 1,744 ms (mean), which is too much for one frame each second. No run had one million residents over the network |
| 200,000 actions for each second | Not shown | In one process: 34,500 to 49,900 accepted actions for each second of wall time with `walkers`, 9,100 to 18,300 with `daily_life`, and 88,902 with `rush` and one million residents. Over the network: 2,435 acknowledgements for each second with 10,000 agents. That is the load that the agents made, not the largest rate of the server |
| Acknowledgement below 250 ms at the 99th percentile | Shown only up to 15,000 WebSocket agents and 6,000 HTTP agents with `daily_life` on loopback | Server and agents on one machine. WebSocket: 132.1 ms with 15,000 `daily_life` agents, 3.9 ms with 15,000 agents that walk. HTTP with the long poll: 94.7 ms with 6,000 `daily_life` agents, and not sustained with 7,500 (342.5 ms). HTTP with agents that walk: 186.9 ms with 25,000 agents on a shared pool |

Not measured:

- A machine that does nothing else.
- Linux.
- A server and a swarm on different machines, and more than one swarm
  machine. One macOS client machine can hold about 15,000 connections to one
  server address, so no run had more than 15,000 WebSocket agents.
- TLS. This build of the swarm sends plain HTTP only.
- A foreign server with its limits on, at scale. The tests of the tool run
  a small swarm against a server with limits and check the `429` class.
- The largest action rate of the server over the network. The agents of all
  runs had a think time of 5 seconds.
- One million residents for more than 120 frames, and their memory in a
  world that has run for a long time.
- The parts of the game that `daily_life` does not use (section 3).
- Deaths. No resident died in a run: a full resident lives 122 game hours
  without food, and the longest run had 33 game hours.
- Runs longer than 2,000 ticks or 60 seconds.
- The MCP endpoint and webhooks.
