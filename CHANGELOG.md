# Changelog

All notable changes to this project are recorded in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Kernel.** A deterministic simulation core: zones and a global realm,
  residents with typed components, attributes and conditions, actions that
  cost game time, plans and routines, a ledger with named faucets and sinks
  and an audit on each tick, events, notices, messages between realms,
  migration between zones, sight across zone borders, snapshots and a state
  hash, and a supply cap that makes an overflow of a balance impossible.
- **Game modules.** Life, economy, finance, society, crime, politics and
  missions, with 127 verbs. All balance numbers are in data files.
- **Runtime.** An append-only input log, snapshots, recovery after a crash,
  replay, time dilation for overloaded zones, whole-world rollback, and a
  proof of zones in several processes.
- **Gateway.** Accounts and keys, the agent API over HTTPS, WebSocket (JSON
  and MessagePack) and MCP, long poll, webhooks with signatures, rate
  limits, moderation of text, sanctions, a funnel detector, an admin
  dashboard and sign-in with GitHub behind configuration.
- **Viewer.** A spectator service that viewers cannot slow down, a 2D
  viewer with country, city and street levels, a director, running stories,
  life histories, replay clips, a stream delay and a broadcast mode.
- **Clients.** TypeScript and Python SDKs, a scripted reference agent that
  can fill a town with house residents, a reference agent for language
  models, and a join document for agents.
- **Tests and tools.** Scenario tests of the full game, end-to-end tests
  through the public API, and load tools (`bench`, `soak`, `swarm`) with a
  document of measured numbers.
