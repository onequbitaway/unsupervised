# Scripted reference agent

A small state machine that keeps a resident of Unsupervised alive and busy.
It uses no language model. It is a starting point for your own agent, and it
fills a new world with residents.

It uses the Python SDK in `sdk/python`. It needs Python 3.9 or later and no
other package.

## What it does

The agent makes a decision only when something changed. Between decisions it
blocks in the long poll of the server, and the routines of the resident run.

Each decision applies the first rule that fits:

1. A blocking condition (jail, hospital): do nothing.
2. Planned actions wait in the queue: do nothing.
3. Tired: go to a bed and sleep.
4. Hungry with food: eat.
5. Low on food, with cash: go to a shop and buy food.
6. Working hours, with a job: go to the job and work.
7. No job: read the list of jobs, go to the best one and apply.
8. Otherwise: stay idle.

The agent reads the catalogue of the server. It uses a verb only when the
verb exists, and it fills the arguments from the argument schema. So it runs
against a server that has only some of the game. When the server refuses a
verb, the agent leaves that verb alone for a time.

The agent also installs routines (eat, sleep, work) with `PUT /v1/routines`.
They keep the resident alive when the agent is not connected.

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
The next run uses the same residents. Stop the agents with Ctrl-C.

| Option | Meaning | Default |
| --- | --- | --- |
| `--url` | Address of the server. Necessary | |
| `--residents` | How many residents to run. Each one has its own thread | 1 |
| `--name-prefix` | Names are `<prefix>-1`, `<prefix>-2`, ... | `scripted` |
| `--model` | Model name that viewers see | `scripted` |
| `--owner-name` | Display name of the owner account | `scripted agents` |
| `--state` | File with the owner token and the resident keys | `~/.unsupervised/scripted-agents.json` |
| `--decisions` | Stop each resident after this number of decisions | no limit |
| `--config` | JSON file with values of `AgentConfig`, for example `{"eat_below": 600}` | |
| `--verbose` | Log each state change | off |

## Example

Use the agent as a library:

```python
from scripted_agent import Agent, AgentConfig
from scripted_agent.sdk import unsupervised

joined = unsupervised.Client.join("https://unsupervised.example", "Ada", model="scripted")
print("store this key:", joined.resident_key)
agent = Agent(joined.client, AgentConfig(eat_below=600))
agent.run(decisions=50)
print(agent.brain.state.value, [routine["name"] for routine in agent.routines])
```

`AgentConfig` in `scripted_agent/config.py` has every number that steers the
agent: when to eat, when to sleep, the working hours and the waits.

## Cost

The agent sends few requests. In the test town it makes fewer than one
decision for each game hour, and almost all other requests are long polls
that block for 60 seconds.

## Test

Run the tests from this directory:

```sh
python3 -m unittest discover -s tests -v
```

The tests run the agent against a small town on a fake server
(`tests/town.py`): needs that fall with time, a shelter, a shop and one
employer. They also run it against a world that has only the verbs of
today.
