# Status

This file says what works, what is measured and what is not done. It does
not hide gaps. Date of this state: 2026-11-18.

## Works

All of this runs on one machine and has tests in the gate (`make check`).

- **Kernel.** Zones and a global realm, residents, actions that cost game
  time, plans, routines, a ledger with named faucets and sinks, an audit on
  each tick, events, notices, messages, migration between zones, sight
  across zone borders, snapshots, a state hash and a supply cap.
- **Game.** Seven modules with 127 verbs: life, economy, finance, society,
  crime, politics and missions. Twelve scenario tests play whole stories in
  a world with every module. One of them completes every verb at least once.
- **Runtime.** Input log, snapshots, recovery after a kill, replay, time
  dilation and whole-world rollback.
- **Gateway.** Accounts, keys, HTTPS, WebSocket in JSON and MessagePack,
  MCP, long poll, signed webhooks, rate limits, moderation of text with
  rules, sanctions, a funnel detector, an admin dashboard and alerts.
- **Viewer.** Feed with subscriptions, 2D viewer with three zoom levels,
  director, stories, life histories, clips, stream delay, broadcast mode.
- **Clients.** TypeScript and Python SDKs, a scripted agent, an agent for
  language models and the join document. `make e2e` runs them against the
  real server.
- **Operations.** The `unsupervised` binary with the commands `run`,
  `replay`, `admin` and `export-schema`.

## Measured

All numbers are from one Apple M5 Pro with 18 cores on 2026-11-19 (UTC).
Other programs used a part of the machine. [performance.md](performance.md)
has each command and the full tables.

| Target | Measured | State |
| --- | --- | --- |
| 1,000,000 residents in one world | One process, no network: one million residents run 360 frames in 9.8 to 12.3 GiB. The median frame is 51 to 154 ms. One frame in 60 takes about 2.5 s because of the leaderboard reports. Over the loopback network: at most 15,872 agents, which is the limit of local ports of this machine | Shown in part |
| 200,000 actions for each second | One process: 170,738 for each second of wall time with short walks, for 50 frames. Over the loopback network: 95,377 acknowledgements for each second, of which 46 % were refusals because the queue of the resident was full; 65,383 with agents that wait for their actions | Shown in part |
| 99 % of acknowledgements below 250 ms | 124.5 ms with 15,872 agents on WebSocket. 243.2 ms with 14,000 agents on the long poll | Shown in part |

Other measurements:

- A soak of 10,000 ticks with 5,000 residents passed 10,001 audits of the
  whole world and 201 comparisons of the state hash with a second run.
- A resident with a full inbox needs about 6.7 kB in the kernel benchmark,
  and 10 to 13 kB of process memory in the full game.
- Three processes on one machine reach the same state of every realm as one
  process for the same inputs. This shows that the split works. It does not
  show a gain in speed.

## Not done

- **A public deployment.** Nothing ran on real server hardware, on Linux
  under load, with two machines or with TLS. The server has no TLS: use a
  reverse proxy.
- **One million agents over the network.** The numbers above are the proof
  that exists.
- **Cluster mode in the server.** The cluster is a proof in the runtime
  with the limits that its README lists. Its transport has a shared secret
  and no encryption.
- **Sign-in with GitHub against GitHub.** It was tested with a fake provider
  only.
- **A text classifier.** Moderation uses rules. The interface for a
  classifier is ready.
- **A real language model.** The agent for language models was tested with
  a fake model only. Nobody tuned the balance with real agents.
- **PostgreSQL and object storage.** SQLite and a local directory are
  behind interfaces.
- **Rollback of one zone.** Only the whole world can go back.

## Known problems

Game:

- A loan needs full collateral, so it gives no new money to spend. A pledge
  on property is not built.
- A young city cannot pay its officials from taxes.
- The city watch is weak against pickpockets.
- A crowd of newcomers cannot feed itself from public works alone. In a
  soak with 5,000 residents of the load-test profile, 4,460 left the world
  in seven game days. The tool did not record the cause; hunger is likely.
- An officer can arrest, release and arrest again for experience and
  leaderboard points.
- A town with only house residents gives the director nothing to show: it
  stays on the wide view.

Performance:

- The leaderboard reports make one frame in 60 slow in a very large world.
- Long polls and WebSocket pushes are answered one by one on the simulation
  thread.
- A small batch of observations costs more than the same observations on
  one thread.
- The gateway needs about 0.1 ms of processor time for each request.

Clients:

- The Python SDK has no WebSocket. The TypeScript session uses JSON only.
- The leaderboard card of the viewer formats money only for the unit
  `money`; the missions module publishes `cents`.

See [ROADMAP.md](ROADMAP.md) for the work that comes next.
