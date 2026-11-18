"""The command line: arguments, the state file, sign-up of N residents and the run."""

import io
import json
import stat
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import _path  # noqa: F401
from town import Town, kernel_verbs

from scripted_agent import AgentConfig
from scripted_agent.cli import enlist, load_state, main, parse_args, run_agents, save_state
from unsupervised import Client
from unsupervised.testing import FakeGateway, FakeServer, FakeWorld


def factory(server: FakeServer):
    """Returns a client factory for the command line that uses no proxy settings."""
    return lambda url, *args, **options: Client(url, *args, opener=server.opener, **options)


class ArgumentsTest(unittest.TestCase):
    def test_the_defaults(self) -> None:
        args = parse_args(["--url", "http://127.0.0.1:8080"])
        self.assertEqual(
            (args.url, args.residents, args.name_prefix, args.model, args.decisions, args.config, args.verbose),
            ("http://127.0.0.1:8080", 1, "scripted", "scripted", None, None, False),
        )
        self.assertEqual(args.state, Path.home() / ".unsupervised" / "scripted-agents.json")

    def test_every_option(self) -> None:
        args = parse_args(
            ["--url", "http://x", "--residents", "7", "--name-prefix", "bot", "--model", "rules", "--state", "/tmp/s.json", "--decisions", "3", "--verbose"]
        )
        self.assertEqual(
            (args.residents, args.name_prefix, args.model, args.state, args.decisions, args.verbose),
            (7, "bot", "rules", Path("/tmp/s.json"), 3, True),
        )

    def test_a_missing_url_or_a_bad_count_is_an_error(self) -> None:
        for argv in ([], ["--url", "http://x", "--residents", "0"], ["--url", "http://x", "--residents", "many"]):
            with self.subTest(argv=argv), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
                parse_args(argv)
            self.assertEqual(raised.exception.code, 2)

    def test_the_configuration_refuses_a_key_that_it_does_not_know(self) -> None:
        self.assertEqual(AgentConfig.from_dict({"eat_below": 350}).eat_below, 350)
        with self.assertRaisesRegex(ValueError, "unknown configuration keys: eat_bellow"):
            AgentConfig.from_dict({"eat_bellow": 350})


class ServerCase(unittest.TestCase):
    def setUp(self) -> None:
        self.world = FakeWorld(verbs=kernel_verbs(), time="auto")
        self.gateway = FakeGateway(self.world)
        self.server = FakeServer(self.gateway)
        self.addCleanup(self.server.close)
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name) / "nested" / "agents.json"

    def enlist(self, count: int, **options):
        return enlist(self.server.url, count, self.state, client_factory=factory(self.server), **options)


class EnlistTest(ServerCase):
    def test_the_first_run_signs_up_and_creates_the_residents(self) -> None:
        clients = self.enlist(3, name_prefix="bot", model="rules", owner_name="Grace")
        self.assertEqual([client.me()["name"] for client in clients], ["bot-1", "bot-2", "bot-3"])
        self.assertEqual({resident.model for resident in self.world.residents.values()}, {"rules"})
        self.assertEqual({resident.owner for resident in self.world.residents.values()}, {1})

        saved = load_state(self.state)["servers"][self.server.url]
        self.assertEqual(saved["owner_token"], "owner-token-1")
        self.assertEqual([record["name"] for record in saved["residents"]], ["bot-1", "bot-2", "bot-3"])
        self.assertEqual([record["key"] for record in saved["residents"]], [client.resident_key for client in clients])
        # The file holds keys: only the user can read it.
        self.assertEqual(stat.S_IMODE(self.state.stat().st_mode), 0o600)

    def test_a_second_run_uses_the_same_residents_and_adds_the_missing_ones(self) -> None:
        first = self.enlist(2)
        second = self.enlist(3)
        self.assertEqual([client.resident_key for client in second[:2]], [client.resident_key for client in first])
        self.assertEqual(len(self.world.residents), 3)
        self.assertEqual(len(self.gateway.requests_to("/v1/owners")), 1)
        # A smaller count uses the first residents and removes none.
        third = self.enlist(1)
        self.assertEqual([client.me()["resident"] for client in third], [1])
        self.assertEqual(len(load_state(self.state)["servers"][self.server.url]["residents"]), 3)

    def test_a_key_that_the_server_no_longer_takes_is_rotated(self) -> None:
        self.enlist(1)
        resident = self.world.residents[1]
        self.world.rotate_key(resident)
        clients = self.enlist(1)
        self.assertEqual(clients[0].me()["resident"], 1)
        self.assertEqual(clients[0].resident_key, resident.key)
        self.assertEqual(load_state(self.state)["servers"][self.server.url]["residents"][0]["key"], resident.key)
        self.assertEqual(len(self.world.residents), 1)

    def test_each_server_has_its_own_part_of_the_state_file(self) -> None:
        self.enlist(1)
        other = FakeServer()
        self.addCleanup(other.close)
        enlist(other.url, 2, self.state, client_factory=factory(other))
        servers = load_state(self.state)["servers"]
        self.assertEqual({url: len(entry["residents"]) for url, entry in servers.items()}, {self.server.url: 1, other.url: 2})

    def test_a_file_that_is_not_a_state_file_is_refused(self) -> None:
        save_state(self.state, {"servers": {}})
        self.assertEqual(load_state(self.state), {"servers": {}})
        self.state.write_text(json.dumps(["not", "a", "state"]))
        with self.assertRaisesRegex(ValueError, "is not a state file"):
            load_state(self.state)


class RunTest(ServerCase):
    def test_run_agents_runs_every_resident(self) -> None:
        clients = self.enlist(3)
        agents = run_agents(clients, AgentConfig(min_decision_seconds=0.0), decisions=4)
        self.assertEqual([agent.decisions for agent in agents], [4, 4, 4])
        self.assertEqual([agent.fatal for agent in agents], [None, None, None])
        # Each resident made its own long polls.
        waits = {request.token for request in self.gateway.requests_to("/v1/wait")}
        self.assertEqual(waits, {client.resident_key for client in clients})

    def test_the_command_joins_runs_and_returns_zero(self) -> None:
        town = Town()
        server = FakeServer(FakeGateway(town.world))
        self.addCleanup(server.close)
        config = self.state.parent.parent / "config.json"
        config.write_text(json.dumps({"min_decision_seconds": 0.0, "idle_seconds": 120}))
        output = io.StringIO()
        argv = ["--url", server.url, "--residents", "2", "--state", str(self.state), "--decisions", "5", "--config", str(config)]
        with redirect_stdout(output):
            status = main(argv, client_factory=factory(server))
        self.assertEqual(status, 0)
        self.assertEqual(output.getvalue(), "running 2 resident(s) on %s; stop with Ctrl-C\n" % server.url)
        self.assertEqual(sorted(resident.name for resident in town.world.residents.values()), ["scripted-1", "scripted-2"])
        # Both residents acted: each applied for the job in the town.
        applied = [request.token for request in server.gateway.requests_to("/v1/act") if "apply_job" in json.dumps(request.body)]
        self.assertEqual(len(set(applied)), 2)

    def test_the_command_fails_when_the_server_is_not_there(self) -> None:
        self.server.refuse_requests(100)
        output = io.StringIO()
        with redirect_stdout(output):
            status = main(["--url", self.server.url, "--state", str(self.state)], client_factory=factory(self.server))
        self.assertEqual(status, 1)
        self.assertEqual(output.getvalue(), "cannot join %s: POST /v1/owners got no answer\n" % self.server.url)
        self.assertFalse(self.state.exists())


if __name__ == "__main__":
    unittest.main()
