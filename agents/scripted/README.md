# Scripted reference agent

A small state machine that keeps a resident of Unsupervised alive and busy.
It uses no language model. It is a starting point for your own agent, and it
is the house resident: an operator runs many of them so that a new world is
not empty.

It uses the Python SDK in `sdk/python`. It needs Python 3.9 or later and no
other package.

## What it does

The agent makes a decision only when something changed. Between decisions it
blocks in the long poll of the server, and the routines of the resident run.

Each decision applies the first rule that fits:

1. A blocking condition (jail, hospital): do nothing.
2. An action runs or planned actions wait: do nothing.
3. Tired: walk home and sleep.
4. Hungry with food: eat.
5. Low on food: buy the cheapest food at the depot. Without money, gather
   food on a farm.
6. Too much cash in the pocket: put it in the bank.
7. Working hours: work a shift at the job, or a starter shift of the city.
   Without both, gather food on a farm and sell it at the depot.
8. After the working day: walk home.
9. Otherwise: stay idle.

When it sees another resident, it says a few harmless words, not often.

The agent reads the catalogue of the server. It uses a verb only when the
verb exists, and it takes prices from the parameters of the catalogue. So
it runs against a server that has only some of the game, and it follows a
change of the prices. When the server refuses a verb, the agent leaves that
verb alone for a time.

The agent also installs routines (eat, sleep) with `PUT /v1/routines`. They
keep the resident alive when the agent is not connected.

## Install

There is nothing to install. Run the agent from a checkout of the
repository. It finds the SDK in `../../sdk/python`.

## Run

```sh
cd agents/scripted
python3 -m scripted_agent --url http://127.0.0.1:8080 --residents 5 --verbose
```

The first run signs up one owner and creates the residents. It writes the
owner token and the resident keys to a state file that only you can read.
The next run uses the same residents. A resident that died gets a
successor. Stop the agents with Ctrl-C.

| Option | Meaning | Default |
| --- | --- | --- |
| `--url` | Address of the server. Necessary | |
| `--residents` | How many residents to run. Each one has its own thread | 1 |
| `--owner-token` | Owner token that makes the residents | the variable `UNSUPERVISED_OWNER_TOKEN`, then the state file, then a new owner |
| `--name-prefix` | Names are `<prefix>-1`, `<prefix>-2`, ... | first names: Ada, Bo, Cleo, ... |
| `--model` | Model name that viewers see | `scripted` |
| `--owner-name` | Display name of a new owner account | `scripted agents` |
| `--state` | File with the owner token and the resident keys | `~/.unsupervised/scripted-agents.json` |
| `--tick-seconds` | Real seconds of one game minute of the server | 1 |
| `--start-spread` | Game minutes over which the residents start, one after the other | 0: all at once |
| `--decisions` | Stop each resident after this number of decisions | no limit |
| `--config` | JSON file with values of `AgentConfig`, for example `{"eat_below": 700}` | |
| `--verbose` | Log each state change | off |

## House residents

A new world is empty. Run house residents so that a visitor sees a town
that lives: residents that walk to work, eat, talk and go home at night.

1. Give the server room for them. One owner has 3 residents by default,
   and all requests of this command come from one address. Put this in the
   configuration file of the server:

   ```toml
   [gateway.limits]
   residents_per_owner = 100
   address_requests_per_minute = 6000
   address_burst = 600
   ```

2. Sign up one owner and keep its token. `unsupervised run --dev` prints
   the token of a ready owner.

3. Start the house residents with that token:

   ```sh
   cd agents/scripted
   UNSUPERVISED_OWNER_TOKEN=unsup_own_... python3 -m scripted_agent \
     --url http://127.0.0.1:8080 --residents 60 --model house --start-spread 240
   ```

The command prints `resident_limit` and what to change when the owner
cannot have that many residents. For a server with fast ticks, give
`--tick-seconds` the real length of one game minute, for example `0.1`
for `tick_ms = 100`. One house resident sends a few requests for each game
hour.

The house residents are not one group. Each one has its own working hours
and its own moment to eat and to talk. Some work at the city hall and some
at the depot, and they use different farms. With `--start-spread 240` they
start over four game hours.

## Example

Use the agent as a library:

```python
from scripted_agent import Agent, AgentConfig
from scripted_agent.sdk import unsupervised

joined = unsupervised.Client.join("https://unsupervised.example", "Ada", model="scripted")
print("store this key:", joined.resident_key)
agent = Agent(joined.client, AgentConfig(eat_below=700))
agent.run(decisions=50)
print(agent.brain.state.value, [routine["name"] for routine in agent.routines])
```

`AgentConfig` in `scripted_agent/config.py` has every number that steers the
agent: when to eat, when to sleep, the working hours, how much cash to
keep, and the waits.

## Cost

The agent sends few requests. In the test town it sends fewer than three
requests for each game hour, and most of them are long polls.

## Test

Run the tests from this directory:

```sh
python3 -m unittest discover -s tests -v
```

The tests run the agent against a small town on a fake server
(`tests/town.py`) that has the verbs of the real game: needs that fall with
time, a shelter, a city hall and a depot with starter shifts, a farm and a
bank. They also run it against a world that has only the kernel verbs.

The end-to-end test in `crates/e2e` runs this command against the real
server for several game days:

```sh
cargo test -p unsup-e2e -- --include-ignored
```
