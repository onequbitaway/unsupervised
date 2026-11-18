# Conformance probes for the fake servers

Each SDK has a fake server for its tests: `FakeGateway` in `sdk/ts` and in
`sdk/python`. A fake is useful only while it answers like the real server.
`probes.json` is a list of requests. The scripts `sdk/ts/e2e/conformance.mjs`
and `sdk/python/e2e/conformance.py` send each request to the fake of their
SDK and to a real server, and compare the two answers.

A Rust test in `crates/e2e` starts the real server and runs the scripts:

```sh
cargo test -p unsup-e2e -- --include-ignored
```

## What a probe is

A probe is one JSON object. The scripts read these fields:

| Field | Meaning |
| --- | --- |
| `name` | What the probe shows. It is in the message of a failure |
| `status`, `code` | The HTTP status and the error code that the real server gives. A probe that gets another answer fails, also when the fake gives the same one |
| `method`, `path` | The request. `{resident}` in the path is the id of the resident |
| `auth` | The token: `none`, `owner`, `resident`, `other` (a resident key of another owner) or `unknown` |
| `body` | The JSON body. `raw` and `content_type` send a body that is not JSON |
| `save` | Values of the answer to keep: `owner_token`, `resident_key`, `resident`. The value is the path in the answer |
| `shape` | `true`: compare the names and the types of all fields of the answer |
| `ignore` | Paths that the game decides, for example `sections.self.attrs`. They are not compared. A list adds nothing to a path |
| `lists` | `first`: compare each list by its first entry. Without it, the lists have the same length and each entry is compared |
| `count` | The path of a list. The two lists have the same length |
| `advance` | No request: this number of game minutes pass |
| `tell` | No request: another resident sends this text to the resident, as a private message |
| `sanction` | No request: an administrator acts on the resident or on its owner: `mute`, `unmute`, `ban`, `unban`, `despawn` |

Each probe compares the HTTP status and the `code` of an error answer. A
probe with `shape` also compares the form of the body: the names of the
fields and the type of each value.

The real server of a run moves only when a script asks: the Rust test gives
the scripts a small clock service (`E2E_CLOCK_URL`) that runs frames of the
world. So a run is the same each time.

Add a probe when a fake gets a new route or a new refusal.
