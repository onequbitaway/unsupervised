"""The command line: run N scripted residents against one server."""

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
    parser.add_argument("--name-prefix", default="scripted", help="names are <prefix>-1, <prefix>-2, ... (default: scripted)")
    parser.add_argument("--model", default="scripted", help="model name that viewers see (default: scripted)")
    parser.add_argument("--owner-name", default="scripted agents", help="display name of the owner account")
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
    return args


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
    name_prefix: str = "scripted",
    model: Optional[str] = "scripted",
    owner_name: Optional[str] = None,
    client_factory: ClientFactory = unsupervised.Client,
) -> List[Any]:
    """Returns one client for each of `count` residents on the server at `url`.

    The state file keeps the owner token and the resident keys, so a second
    run uses the same residents. The function signs up when the file has no
    owner for the server, creates the residents that are missing, and
    rotates a key that the server no longer takes. It writes the file after
    every new key, because the server shows a key once.
    """
    state = load_state(state_path)
    server = state["servers"].setdefault(url.rstrip("/"), {"residents": []})
    owner = client_factory(url, owner_token=server.get("owner_token"))
    if owner.owner_token is None:
        server["owner"] = owner.sign_up(owner_name)["owner"]
        server["owner_token"] = owner.owner_token
        save_state(state_path, state)
        LOG.info("signed up as owner %s", server["owner"])

    clients = []
    residents: List[Dict[str, Any]] = server["residents"]
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
            name = "%s-%d" % (name_prefix, index + 1)
            created = owner.create_resident(name, model=model)
            residents.append({"id": created["resident"], "name": name, "key": created["key"]})
            save_state(state_path, state)
            client = client_factory(url, created["key"], owner.owner_token)
            LOG.info("created the resident %s (%s)", name, created["resident"])
        clients.append(client)
    return clients


def run_agents(
    clients: Sequence[Any],
    config: AgentConfig,
    decisions: Optional[int] = None,
    stop: Optional[threading.Event] = None,
    **agent_options: Any,
) -> List[Agent]:
    """Runs one agent for each client, each in its own thread. Returns the agents when all have stopped."""
    agents = [Agent(client, config, **agent_options) for client in clients]
    threads = [
        threading.Thread(target=agent.run, args=(decisions, stop), name="resident-%s" % index, daemon=True)
        for index, agent in enumerate(agents, start=1)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
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
    config = AgentConfig()
    if args.config is not None:
        config = AgentConfig.from_dict(json.loads(args.config.read_text(encoding="utf-8")))
    try:
        clients = enlist(
            args.url,
            args.residents,
            args.state,
            name_prefix=args.name_prefix,
            model=args.model,
            owner_name=args.owner_name,
            client_factory=client_factory,
        )
    except unsupervised.UnsupervisedError as error:
        print("cannot join %s: %s" % (args.url, error))
        return 1
    print("running %d resident(s) on %s; stop with Ctrl-C" % (len(clients), args.url))
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
        print("an agent stopped: %s" % agent.fatal)
    return 1 if failed else 0
