# Build plan

This file is the working memory of the build. It is updated after each event.

## Conventions

- Local path: `~/Documents/GitHub/unsupervised`. Remote: `onequbitaway/unsupervised` (private).
- Toolchain: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` before cargo.
- Gate: `make check`.
- Each writer works in a git worktree under `.worktrees/<name>` on branch
  `feat/<name>`. Writers do not commit. The integrator checks, commits and merges.
- Commits: Conventional Commits, author is the repository owner, no tool
  attribution.

## Wave 0 (serial)

| Step | State |
| --- | --- |
| Repository, scaffold, licence, CI, gate | done |
| Design documents, landscape | done |
| Protocol + kernel + runtime skeleton | done |
| Walking skeleton: server binary, minimal gateway, minimal viewer | done |
| Freeze contracts, first commit and push | done |

## Wave 1 (parallel)

| Workstream | Branch | Owns | State |
| --- | --- | --- | --- |
| runtime | `feat/runtime` | `crates/runtime` | running |
| life | `feat/life` | `crates/modules/life`, `docs/design/modules/life.md` | running |
| economy | `feat/economy` | `crates/modules/economy`, `docs/design/modules/economy.md` | running |
| finance | `feat/finance` | `crates/modules/finance`, `docs/design/modules/finance.md` | running |
| society | `feat/society` | `crates/modules/society`, `docs/design/modules/society.md` | running |
| gateway | `feat/gateway` | `crates/gateway`, `crates/store`, `crates/server` (not `plugins.rs`) | running |
| viewer | `feat/viewer` | `web/`, `crates/spectator` | running |
| sdk | `feat/sdk` | `sdk/`, `agents/`, `docs/join.md` | running |
| loadtest | `feat/loadtest` | `crates/loadtest` | running |

## Wave 2 (parallel, after wave 1 merges)

| Workstream | Branch | Owns | State |
| --- | --- | --- | --- |
| crime | `feat/crime` | `crates/modules/crime` | todo |
| politics | `feat/politics` | `crates/modules/politics` | todo |
| missions | `feat/missions` | `crates/modules/missions` | todo |
| director | `feat/director` | `crates/spectator`, `web/` | todo |
| ops | `feat/ops` | `crates/runtime` (multi-process), moderation, admin tools | todo |

## Wave 3 (integration)

Scenario tests through the public API, soak run, performance measurements,
final documents, final gate.

## Log

- 2026-11-18: repository created; scaffold and design documents written.
- 2026-11-18: kernel, protocol and runtime landed (158 tests). Walking skeleton passes: a resident moves through HTTP and a viewer socket sees it. Contracts frozen.
