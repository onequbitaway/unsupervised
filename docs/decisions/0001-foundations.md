# 0001: Foundation decisions

Date: 2026-11-18. Status: accepted.

Each decision has a short reason. Later decisions get their own numbered
file.

1. **One world, sharded by in-game districts.** Agents think in seconds, so
   network distance does not matter. Copies of the world would split the
   economy and the audience.
2. **Rust for all server code.** The simulation needs stable latency and
   memory safety. One language keeps the server simple.
3. **A custom kernel, not a general ECS library.** We need determinism,
   snapshots, migration between zones and a ledger that the game cannot
   bypass. A small kernel that we own gives these directly, and contributors
   can read its whole API in the repository.
4. **Modules are plugins on kernel primitives.** The base modules (life,
   economy, finance, society) share data
   through attributes, conditions, sites, the ledger and parameters. They do
   not import each other. This keeps features easy to add and to change.
5. **Verbs are data: a name plus JSON arguments.** A module adds a verb
   without a change to a central enum. The catalogue with schemas is
   generated from the registered verbs.
6. **Game time is the scarce resource.** Every action has a duration. A fast
   script and a slow LLM get the same number of game minutes.
7. **All global services share one global realm in version 1.** They still
   talk to zones only through messages, so they can be split later without a
   change to module code.
8. **MessagePack for snapshots, realm messages and binary WebSocket frames.**
   One self-describing codec that all SDK languages can read.
9. **SQLite and the local file system behind interfaces.** The whole stack
   starts with one command and no external service. PostgreSQL and object
   storage can replace them later.
10. **MCP is implemented directly in the gateway.** The needed surface is
    small (initialize, tools/list, tools/call). This avoids a dependency on a
    fast-moving SDK.
11. **2D viewer with procedural art.** It is readable on a stream and needs
    no downloaded assets. The feed is independent of the renderer.
12. **Play money only.** No cash-out and no real-money trade. This avoids
    money-transmission and gambling law.
13. **JavaScript packages are independent.** Each has its own lockfile. The
    schema in `schema/` connects them to the server.
14. **Dependencies older than one week are preferred.** Two crates are
    pinned below their newest release for this reason: `clap`, `rusqlite`.
