"""The command line: run N scripted residents against one server.

An operator uses it to run the house residents of a new world: residents
that work, eat, sleep and talk, so that the world is not empty.
"""

import argparse
import json
import logging
import os
import threading
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence

from .agent import Agent
from .config import AgentConfig
from .sdk import unsupervised

LOG = logging.getLogger("scripted_agent")

DEFAULT_STATE = Path.home() / ".unsupervised" / "scripted-agents.json"
#: First names of the house residents. A number follows when the list is used up.
NAMES = (
    "Ada", "Bo", "Cleo", "Dev", "Esme", "Finn", "Gus", "Hana", "Ines", "Jun", "Kofi", "Lena",
    "Milo", "Nia", "Omar", "Pia", "Quin", "Rosa", "Sven", "Tara", "Uma", "Vito", "Wren", "Yara",
)  # fmt: skip
#: Permission bits of the state file: only the user reads and writes it. It holds keys.
STATE_MODE = 0o600

ClientFactory = Callable[..., Any]


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    """Reads the command line."""
    parser = argparse.ArgumentParser(
        prog="scripted_agent",
        description="Run scripted residents of Unsupervised. They eat, sleep, find work and buy food without a language model.",
    )
    parser.add_argument("--url", required=True, help="address of the server, for example http://127.0.0.1:8080")
    parser.add_argument("--residents", type=int, default=1, help="how many residents to run (default: 1)")
    parser.add_argument(
        "--name-prefix",
        default=None,
        help="names are <prefix>-1, <prefix>-2, ... (default: first names such as Ada, Bo, Cleo)",
    )
    parser.add_argument("--model", default="scripted", help="model name that viewers see (default: scripted)")
    parser.add_argument("--owner-name", default="scripted agents", help="display name of the owner account")
    parser.add_argument(
        "--owner-token",
        default=os.environ.get("UNSUPERVISED_OWNER_TOKEN"),
        help="owner token that makes the residents (default: the variable UNSUPERVISED_OWNER_TOKEN, "
        "then the state file, then a new owner)",
    )
    parser.add_argument(
        "--start-spread",
        type=int,
        default=None,
        metavar="MINUTES",
        help="game minutes over which the residents start, one after the other (default: 0, all at once)",
    )
    parser.add_argument(
        "--tick-seconds",
        type=float,
        default=None,
        help="real seconds of one game minute of the server (default: 1)",
    )
    parser.add_argument(
        "--state",
        type=Path,
        default=DEFAULT_STATE,
        help="file that keeps the owner token and the resident keys (default: %s)" % DEFAULT_STATE,
    )
    parser.add_argument("--decisions", type=int, default=None, help="stop each resident after this number of decisions")
    parser.add_argument("--config", type=Path, default=None, help="JSON file with values of AgentConfig")
    parser.add_argument("--verbose", action="store_true", help="log every state change")
    args = parser.parse_args(argv)
    if args.residents < 1:
        parser.error("--residents must be at least 1")
    if args.tick_seconds is not None and args.tick_seconds <= 0:
        parser.error("--tick-seconds must be greater than 0")
    if args.start_spread is not None and args.start_spread < 0:
        parser.error("--start-spread must be 0 or more")
    return args


def resident_name(index: int, prefix: Optional[str] = None) -> str:
    """Returns the name of resident number `index` (from 0)."""
    if prefix is not None:
        return "%s-%d" % (prefix, index + 1)
    name, round_number = NAMES[index % len(NAMES)], index // len(NAMES)
    return name if round_number == 0 else "%s %d" % (name, round_number + 1)


def load_state(path: Path) -> Dict[str, Any]:
    """Reads the state file. A file that does not exist gives an empty state."""
    if not path.exists():
        return {"servers": {}}
    state = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(state, dict) or not isinstance(state.get("servers"), dict):
        raise ValueError("%s is not a state file of the scripted agent" % path)
    return state


def save_state(path: Path, state: Dict[str, Any]) -> None:
    """Writes the state file so that only the user can read it."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    descriptor = os.open(str(temporary), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, STATE_MODE)
    with os.fdopen(descriptor, "w", encoding="utf-8") as file:
        json.dump(state, file, indent=2)
        file.write("\n")
    os.replace(str(temporary), str(path))


def enlist(
    url: str,
    count: int,
    state_path: Path,
    *,
    name_prefix: Optional[str] = None,
    model: Optional[str] = "scripted",
    owner_name: str = "scripted agents",
    owner_token: Optional[str] = None,
    client_factory: ClientFactory = unsupervised.Client,
) -> List[Any]:
    """Returns one client for each of `count` residents on the server at `url`.

    The state file keeps the owner token and the resident keys, so a second
    run uses the same residents. The function uses `owner_token` when it is
    given, else the token of the file, else it signs up. It creates the
    residents that are missing, makes a new resident for one that left the
    world, and rotates a key that the server no longer takes. It writes the
    file after every new key, because the server shows a key once.

    Raises `ApiError` with the code `resident_limit` when the owner cannot
    have `count` residents: the operator raises the limit of the server.
    """
    state = load_state(state_path)
    server = state["servers"].setdefault(url.rstrip("/"), {"residents": []})
    if owner_token is not None and owner_token != server.get("owner_token"):
        # The residents of the file belong to another owner.
        server.update({"owner_token": owner_token, "residents": []})
        server.pop("owner", None)
    owner = client_factory(url, owner_token=server.get("owner_token"))
    if owner.owner_token is None:
        server["owner"] = owner.sign_up(owner_name)["owner"]["id"]
        server["owner_token"] = owner.owner_token
        save_state(state_path, state)
        LOG.info("signed up as owner %s", server["owner"])

    # The residents of the owner that are in the world. A resident of the
    # file that is not in this list died, or the server has a new world.
    alive = {record["id"] for record in owner.list_residents()["residents"]}
    kept = [record for record in server["residents"] if record["id"] in alive]
    if len(kept) != len(server["residents"]):
        LOG.info("%d residents of the state file left the world", len(server["residents"]) - len(kept))
        server["residents"] = kept
        save_state(state_path, state)

    clients = []
    residents: List[Dict[str, Any]] = server["residents"]
    used = {record["name"] for record in residents}
    next_name = 0
    for index in range(count):
        if index < len(residents):
            record = residents[index]
            client = client_factory(url, record["key"], owner.owner_token)
            try:
                client.me()
            except unsupervised.ApiError as error:
                if error.status != 401:
                    raise
                record["key"] = owner.rotate_key(record["id"])["key"]
                save_state(state_path, state)
                client = client_factory(url, record["key"], owner.owner_token)
                LOG.info("rotated the key of resident %s", record["id"])
        else:
            name = resident_name(next_name, name_prefix)
            while name in used:
                next_name += 1
                name = resident_name(next_name, name_prefix)
            used.add(name)
            created = owner.create_resident(name, model=model)
            residents.append({"id": created["resident"]["id"], "name": name, "key": created["key"]})
            save_state(state_path, state)
            client = client_factory(url, created["key"], owner.owner_token)
            LOG.info("created the resident %s (%s)", name, created["resident"]["id"])
        clients.append(client)
    return clients


def run_agents(
    clients: Sequence[Any],
    config: AgentConfig,
    decisions: Optional[int] = None,
    stop: Optional[threading.Event] = None,
    **agent_options: Any,
) -> List[Agent]:
    """Runs one agent for each client, each in its own thread. Returns the agents when all have stopped.

    Each agent gets the configuration with small changes and its own
    `variety`, so that the residents do not move as one group.
    """
    agents = [
        Agent(client, config.varied(index), variety=index, **agent_options) for index, client in enumerate(clients)
    ]
    threads = [
        threading.Thread(target=agent.run, args=(decisions, stop), name="resident-%s" % index, daemon=True)
        for index, agent in enumerate(agents, start=1)
    ]
    # The residents start one after the other, over `start_spread_ticks` game minutes.
    pause = config.start_spread_ticks * config.tick_seconds / max(1, len(threads))
    halt = stop if stop is not None else threading.Event()
    for thread in threads:
        if halt.is_set():
            break
        thread.start()
        if pause > 0:
            halt.wait(pause)
    for thread in threads:
        if thread.is_alive():
            thread.join()
    return agents


def main(argv: Optional[Sequence[str]] = None, *, client_factory: ClientFactory = unsupervised.Client) -> int:
    """Runs the command. Returns the exit status: 0, or 1 when an agent stopped on an error.

    `client_factory` makes the clients; a test gives one that needs no proxy settings.
    """
    args = parse_args(argv)
    logging.basicConfig(
        level=logging.INFO if args.verbose else logging.WARNING,
        format="%(asctime)s %(threadName)s %(message)s",
    )
    values: Dict[str, Any] = {}
    if args.config is not None:
        values = json.loads(args.config.read_text(encoding="utf-8"))
    if args.tick_seconds is not None:
        values["tick_seconds"] = args.tick_seconds
    if args.start_spread is not None:
        values["start_spread_ticks"] = args.start_spread
    config = AgentConfig.from_dict(values)
    try:
        clients = enlist(
            args.url,
            args.residents,
            args.state,
            name_prefix=args.name_prefix,
            model=args.model,
            owner_name=args.owner_name,
            owner_token=args.owner_token,
            client_factory=client_factory,
        )
    except unsupervised.ApiError as error:
        print("cannot join %s: %s" % (args.url, error))
        if error.code == "resident_limit":
            print(
                "The owner has its greatest number of residents. Raise `residents_per_owner` in the "
                "`[gateway.limits]` table of the server configuration, or run fewer residents."
            )
        return 1
    except unsupervised.UnsupervisedError as error:
        print("cannot join %s: %s" % (args.url, error))
        return 1
    print("running %d resident(s) on %s; stop with Ctrl-C" % (len(clients), args.url), flush=True)
    stop = threading.Event()
    result: List[Agent] = []
    runner = threading.Thread(target=lambda: result.extend(run_agents(clients, config, args.decisions, stop)), daemon=True)
    runner.start()
    try:
        while runner.is_alive():
            runner.join(0.5)
    except KeyboardInterrupt:
        # A long poll can take a minute to return. The threads are daemons, so the process ends now.
        stop.set()
        print("stopped")
        return 0
    failed = [agent for agent in result if agent.fatal is not None]
    for agent in failed:
        print("an agent stopped: %s" % (agent.farewell or agent.fatal))
    return 1 if failed else 0
