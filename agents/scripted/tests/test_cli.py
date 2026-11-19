"""The command line: arguments, the state file, sign-up of N residents and the run."""

import io
import json
import stat
import tempfile
import threading
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import _path  # noqa: F401
from town import Town, kernel_verbs

from scripted_agent import AgentConfig
from scripted_agent.cli import enlist, load_state, main, parse_args, resident_name, run_agents, save_state
from unsupervised import ApiError, Client
from unsupervised.testing import FakeGateway, FakeServer, FakeWorld


def factory(server: FakeServer):
    """Returns a client factory for the command line that uses no proxy settings."""
    return lambda url, *args, **options: Client(url, *args, opener=server.opener, **options)


class ArgumentsTest(unittest.TestCase):
    def test_the_defaults(self) -> None:
        args = parse_args(["--url", "http://127.0.0.1:8080"])
        self.assertEqual(
            (args.url, args.residents, args.name_prefix, args.model, args.decisions, args.config, args.verbose),
            ("http://127.0.0.1:8080", 1, None, "scripted", None, None, False),
        )
        self.assertIsNone(args.tick_seconds)
        self.assertEqual(args.state, Path.home() / ".unsupervised" / "scripted-agents.json")

    def test_every_option(self) -> None:
        args = parse_args(
            ["--url", "http://x", "--residents", "7", "--name-prefix", "bot", "--model", "rules", "--state", "/tmp/s.json"]
            + ["--decisions", "3", "--verbose", "--owner-token", "unsup_own_1", "--tick-seconds", "0.5"]
        )
        self.assertEqual(
            (args.residents, args.name_prefix, args.model, args.state, args.decisions, args.verbose),
            (7, "bot", "rules", Path("/tmp/s.json"), 3, True),
        )
        self.assertEqual((args.owner_token, args.tick_seconds), ("unsup_own_1", 0.5))

    def test_the_names_are_first_names_then_first_names_with_a_number(self) -> None:
        self.assertEqual([resident_name(index) for index in (0, 1, 23)], ["Ada", "Bo", "Yara"])
        self.assertEqual([resident_name(index) for index in (24, 25, 48)], ["Ada 2", "Bo 2", "Ada 3"])
        self.assertEqual(resident_name(4, "bot"), "bot-5")

    def test_a_missing_url_or_a_bad_count_is_an_error(self) -> None:
        bad = (["--residents", "0"], ["--residents", "many"], ["--tick-seconds", "0"])
        for argv in [[]] + [["--url", "http://x"] + options for options in bad]:
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
        self.assertEqual(self.world.owner_of(saved["owner_token"]), 1)
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

    def test_an_owner_token_of_the_operator_makes_the_residents_and_no_owner_signs_up(self) -> None:
        operator = self.world.sign_up("Operator")
        self.world.limits["residents_per_owner"] = 0
        clients = self.enlist(5, owner_token=operator["token"])
        self.assertEqual([client.me()["name"] for client in clients], ["Ada", "Bo", "Cleo", "Dev", "Esme"])
        self.assertEqual({resident.owner for resident in self.world.residents.values()}, {operator["owner"]["id"]})
        self.assertEqual(self.gateway.requests_to("/v1/owners"), [])
        # The next run with the same token uses the same residents.
        again = self.enlist(5, owner_token=operator["token"])
        self.assertEqual([client.resident_key for client in again], [client.resident_key for client in clients])
        self.assertEqual(len(self.world.residents), 5)

    def test_the_limit_of_residents_of_the_owner_is_an_error_that_names_the_code(self) -> None:
        with self.assertRaises(ApiError) as raised:
            self.enlist(4)
        self.assertEqual(raised.exception.code, "resident_limit")
        # The three residents that the owner could make are in the file: the next run uses them.
        self.assertEqual(len(load_state(self.state)["servers"][self.server.url]["residents"]), 3)

    def test_a_resident_that_left_the_world_gets_a_successor(self) -> None:
        first = self.enlist(2)
        self.world.despawn(self.world.residents[1])
        second = self.enlist(2)
        self.assertEqual(second[0].resident_key, first[1].resident_key)
        self.assertEqual([client.me()["name"] for client in second], ["Bo", "Ada"])
        self.assertEqual(sorted(self.world.residents), [2, 3])

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

    def test_with_a_start_spread_the_residents_start_one_after_the_other(self) -> None:
        clients = self.enlist(3)
        # 30 game minutes of 0.002 seconds over three residents: 0.02 seconds between two starts.
        config = AgentConfig(min_decision_seconds=0.0, start_spread_ticks=30, tick_seconds=0.002)
        started = []
        real_observe = [client.observe for client in clients]
        for index, client in enumerate(clients):
            client.observe = lambda *args, _index=index, **options: (started.append(_index), real_observe[_index](*args, **options))[1]
        agents = run_agents(clients, config, decisions=1)
        self.assertEqual([agent.decisions for agent in agents], [1, 1, 1])
        # Each resident observed for the first time in the order of the start.
        self.assertEqual(started, [0, 1, 2])
        # A stop during the spread starts no more residents.
        stop = threading.Event()
        stop.set()
        idle = run_agents(self.enlist(3), config, decisions=1, stop=stop)
        self.assertEqual([agent.decisions for agent in idle], [0, 0, 0])

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
        self.assertEqual(sorted(resident.name for resident in town.world.residents.values()), ["Ada", "Bo"])
        # Both residents acted: each read the list of jobs of the town.
        asked = [request.token for request in server.gateway.requests_to("/v1/act") if "list_jobs" in json.dumps(request.body)]
        self.assertEqual(len(set(asked)), 2)

    def test_the_command_says_which_resident_died_and_the_next_run_makes_a_successor(self) -> None:
        # A world where a resident dies in its fifth game minute.
        def death(world: FakeWorld) -> None:
            if world.tick == 5 and 1 in world.residents:
                world.die(world.residents[1])

        self.world.on_tick.append(death)
        config = self.state.parent.parent / "config.json"
        config.write_text(json.dumps({"min_decision_seconds": 0.0}))
        output = io.StringIO()
        argv = ["--url", self.server.url, "--residents", "2", "--state", str(self.state), "--decisions", "3"]
        argv += ["--config", str(config)]
        with redirect_stdout(output), self.assertLogs("scripted_agent", "ERROR"):
            status = main(argv, client_factory=factory(self.server))
        self.assertEqual(status, 1)
        self.assertEqual(
            output.getvalue().splitlines()[1:],
            [
                "an agent stopped: resident 1 left the world: it died, or an administrator removed it. "
                "Its key does not work any more. Run the command again to make a successor"
            ],
        )
        # The other resident made its decisions. The next run makes a successor for the dead one.
        self.assertEqual([resident.name for resident in self.world.residents.values()], ["Bo"])
        self.world.on_tick.clear()
        with redirect_stdout(io.StringIO()):
            self.assertEqual(main(argv, client_factory=factory(self.server)), 0)
        self.assertEqual([resident.name for resident in self.world.residents.values()], ["Bo", "Ada"])
        self.assertNotIn(1, self.world.residents)

    def test_the_command_says_what_to_do_when_the_owner_has_too_many_residents(self) -> None:
        output = io.StringIO()
        argv = ["--url", self.server.url, "--residents", "4", "--state", str(self.state)]
        with redirect_stdout(output):
            status = main(argv, client_factory=factory(self.server))
        self.assertEqual(status, 1)
        self.assertIn("resident_limit", output.getvalue())
        self.assertIn("residents_per_owner", output.getvalue())

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
