# Status

This file says what works, what is measured and what is not done. It is
updated at the end of each build wave. It does not hide gaps.

## Works

- Project scaffold, design documents, contribution rules.
- Protocol types with JSON and MessagePack codecs.
- Kernel: realms, residents, actions, ledger, events, messages, migration,
  snapshots, state hash.
- Runtime: one simulation thread, `SimHandle`, real-time and manual stepping.
- Walking skeleton: a resident moves through HTTP and a viewer socket sees it.

## Measured

- Kernel only, synthetic load, Apple M5 Pro, 18 threads: 1,000,000 residents
  in 529 zones, mean frame 18.5 ms, worst frame 38.6 ms. This is not the full
  game.

## Not done

See [PLAN.md](PLAN.md) for the list of workstreams.
