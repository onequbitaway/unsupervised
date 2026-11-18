# Contributing to Unsupervised

Thank you for your help. This file tells you how to set up, how the code is
organized and what a change must do before it can merge.

## Set up

You need:

- Rust (stable, 1.100 or later) with `rustfmt` and `clippy`
- Node.js 22 or later and npm
- Python 3.9 or later (only for the Python SDK and the scripted agent)
- `make`

Then:

```sh
git clone https://github.com/onequbitaway/unsupervised
cd unsupervised
make check
```

`make check` is the gate. It runs the format check, clippy with warnings as
errors, all Rust tests, and the build and tests of the JavaScript and Python
packages. A change must pass the gate before it merges.

## Where things are

Read `docs/design/architecture.md` first. In short:

- `crates/kernel` is the deterministic simulation core.
- `crates/modules/*` are the game rules. One crate per topic.
- `crates/runtime`, `crates/gateway`, `crates/spectator`, `crates/store` are
  the services around the core.
- `crates/server` puts everything together.
- `web/` is the viewer. `sdk/` and `agents/` are client code.

## How to add a feature

Follow section 8 of `docs/design/architecture.md`. The short version: pick
the module, use kernel primitives, put numbers in the data file, move value
only through the ledger, emit events, write tests with `TestWorld`.

## Rules for simulation code

These rules keep the world deterministic and fair. Clippy enforces some of
them.

1. No `HashMap` or `HashSet`. Use `BTreeMap`, `BTreeSet` or `Vec`.
2. No wall clock, files, network, threads or environment in a module.
3. Integers only. Money is cents. Rates are basis points or per-mille.
4. Randomness only from `realm.rng()`.
5. Store `ResidentId` in state. Never store an `Idx`.
6. Only the ledger changes a balance.
7. Every number that a designer could want to tune goes in a data file.

## Rules for all code

- Every public item has a doc comment.
- No `todo!()`, `unimplemented!()`, `dbg!()` or placeholder return value.
- No `unwrap()` or `expect()` outside tests, unless a comment proves that it
  cannot fail.
- Validate all input from the network before use.
- Do not log secrets, API keys or tokens.
- Keep modules small. One file has one job.

## Tests

- Test behaviour through the public API of the crate.
- Each assertion must be able to fail. Break the code once and see the test
  go red.
- Assert exact values when you know them.
- Do not sleep in tests. Step the simulation or await the event.
- No network access to the internet in tests.
- A module test suite includes: the normal path, each refusal, conservation
  of money and goods, and determinism.

## Documentation style

Write documentation in simple English (ASD-STE100 style): short sentences,
active voice, one instruction per sentence, the same word for the same thing.

## Commits and pull requests

- Use Conventional Commits: `feat(economy): add the order book`.
- Keep the subject below 72 characters. Explain the reason in the body.
- Name branches by topic: `feat/order-book`, `fix/ledger-audit`.
- One concern per pull request. Fill in the pull request template.
- Do not add tool or assistant attribution to commits, branches or pull
  requests.

## Security

Do not open a public issue for a security problem. See `SECURITY.md`.

## Licence

By contributing, you agree that your contribution is licensed under the MIT
licence of this project.
