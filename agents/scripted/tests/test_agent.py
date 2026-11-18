"""The agent against the town, through the SDK.

The requests go to the fake gateway without a network: a run of some game
days is hundreds of requests, and a socket for each of them is not needed.
`test_cli.py` and `test_readme.py` run the agent over a local HTTP server.
"""

import threading
import unittest
from typing import Any, List
from unittest import mock

import _path  # noqa: F401
from town import ENERGY, FOOD, HEALTH, SHELTER, TICKS_PER_DAY, WORKS, Town, kernel_verbs

from scripted_agent import Agent, AgentConfig, Brain, State
from unsupervised.testing import FakeGateway, FakeWorld, direct_client

DAYS = 4


class AgentCase(unittest.TestCase):
    """A town (or another world), its gateway, one resident and an agent for it."""

    def start(self, world: FakeWorld, config: AgentConfig = AgentConfig()) -> None:
        self.world = world
        self.gateway = FakeGateway(world)
        self.resident = world.spawn(world.sign_up()["owner"], "Ada")
        self.client = direct_client(self.gateway, self.resident.key)
        self.sleeps: List[float] = []
        # One tick is one second: the clock of the agent is the tick of the world.
        self.agent = Agent(self.client, config, sleep=self.sleeps.append, clock=lambda: float(world.tick))

    def run_days(self, days: int) -> None:
        while self.world.tick < days * TICKS_PER_DAY:
            self.agent.step()

    def sent_verbs(self) -> List[str]:
        """Returns the verbs of every action that the agent asked for."""
        verbs = []
        for request in self.gateway.requests_to("/v1/act"):
            steps = request.body["plan"] if "plan" in request.body else [request.body]
            verbs += [step["verb"] for step in steps]
        return verbs

    def rejected(self) -> List[Any]:
        """Returns the refused actions of the agent: what `POST /v1/act` rejected."""
        return [report for report in self.reports() if not report["ok"]]

    def reports(self) -> List[Any]:
        return [entry["report"] for entry in self.resident.inbox if "report" in entry]


class TownTest(AgentCase):
    def test_the_agent_keeps_a_resident_alive_fed_and_at_work_for_days(self) -> None:
        town = Town()
        self.start(town.world)
        self.run_days(DAYS)

        lowest = town.lowest[self.resident.id]
        self.assertGreater(lowest[FOOD], 200, "the resident never came near starving")
        self.assertGreater(lowest[ENERGY], 100, "the resident never came near exhaustion")
        self.assertEqual(lowest[HEALTH], 1000)
        # It found the better job, worked most of the working hours and bought its own food.
        self.assertEqual(self.resident.sections["job"]["id"], 1)
        self.assertGreaterEqual(town.shifts[self.resident.id], 12)
        self.assertGreaterEqual(town.meals[self.resident.id], 3)
        self.assertGreater(self.resident.cash, 0)
        # Every verb that it sent is in the catalogue.
        known = {verb["verb"] for verb in self.world.catalogue()["verbs"]}
        self.assertLessEqual(set(self.sent_verbs()), known)
        self.assertEqual(
            self.agent.routines and [routine["name"] for routine in self.agent.routines],
            ["eat bread when hungry", "eat apple when hungry", "sleep when tired", "work in the day"],
        )

    def test_a_server_that_repeats_the_inbox_with_each_action_does_not_confuse_the_agent(self) -> None:
        # The protocol document gives `POST /v1/act` no cursor. A server can answer with all old reports.
        town = Town()
        self.start(town.world)
        self.gateway.act_ignores_since = True
        # The resident is hungry and has too little cash for any food, so its purchases fail.
        self.resident.cash = 100
        self.resident.attrs[FOOD] = 400
        with mock.patch.object(Brain, "_failed", autospec=True, side_effect=Brain._failed) as failed:
            for _ in range(8):
                self.agent.step()
        read = [call.args[1] for call in failed.call_args_list]
        real = [report["verb"] for report in self.reports() if not report["ok"]]
        self.assertGreaterEqual(real.count("buy"), 1)
        # The agent read each failure one time, though the server sent it with every later action.
        self.assertEqual(read.count("buy"), real.count("buy"))
        self.assertGreater(len(self.gateway.requests_to("/v1/act")), real.count("buy") + 1)

    def test_the_agent_is_cheap_a_few_requests_for_each_game_hour(self) -> None:
        town = Town()
        self.start(town.world)
        self.run_days(DAYS)
        requests = len(self.gateway.requests)
        hours = DAYS * 24
        # A long poll of 60 seconds is one request for each game hour at the least.
        self.assertGreaterEqual(requests, hours)
        self.assertLess(requests, 2.5 * hours, "%d requests in %d game hours" % (requests, hours))
        self.assertLess(self.agent.decisions, hours)
        # The agent never slept to wait for the server: the long poll did the waiting.
        self.assertEqual([delay for delay in self.sleeps if delay > 2.0], [])

    def test_without_the_agent_the_resident_starves(self) -> None:
        # The control for the test above: the town is hard enough to kill a resident that does nothing.
        town = Town()
        resident = town.spawn()
        town.world.step(DAYS * TICKS_PER_DAY)
        self.assertEqual(town.lowest[resident.id][FOOD], 0)
        self.assertEqual(town.lowest[resident.id][ENERGY], 0)
        self.assertLess(resident.attrs[HEALTH], 1000)

    def test_routines_alone_keep_a_placed_resident_going_between_decisions(self) -> None:
        # The agent decides once and installs the routines. Then nobody decides for three days.
        town = Town()
        self.start(town.world)
        self.resident.items["bread"] = 8
        self.resident.sections["job"] = {"id": 1, "site": WORKS["id"], "wage": 600}
        self.agent.setup()
        self.assertEqual(len(self.resident.routines), 4)
        before = len(self.gateway.requests)
        self.world.step(3 * TICKS_PER_DAY)
        self.assertEqual(len(self.gateway.requests), before)
        # The eat routine works anywhere, so the resident does not starve.
        self.assertGreater(town.lowest[self.resident.id][FOOD], 200)
        self.assertGreaterEqual(town.meals[self.resident.id], 3)
        # It stands in the shelter, so the sleep routine works too.
        self.assertEqual(self.world.site_at(self.resident.pos)["id"], SHELTER["id"])
        self.assertGreater(town.lowest[self.resident.id][ENERGY], 100)
        # The work routine fails there (`not_here`) and waits longer each time: a few reports, not hundreds.
        failures = [report for report in self.reports() if not report["ok"]]
        self.assertEqual({(report["verb"], report["error"]["code"]) for report in failures}, {("work", "not_here")})
        self.assertLessEqual(len(failures), 8)


class MissingVerbsTest(AgentCase):
    def test_with_only_the_kernel_verbs_the_agent_stays_idle_and_sends_no_action(self) -> None:
        self.start(FakeWorld(verbs=kernel_verbs(), time="auto"))
        decisions = [self.agent.step() for _ in range(5)]
        self.assertEqual({decision.state for decision in decisions}, {State.IDLE})
        self.assertEqual(self.sent_verbs(), [])
        self.assertEqual(self.agent.routines, [])
        self.assertEqual(self.resident.routines, [])
        paths = [request.path for request in self.gateway.requests]
        self.assertEqual(paths[:4], ["/v1/catalogue", "/v1/routines", "/v1/observe", "/v1/world/zones/0"])
        self.assertEqual(set(paths[4:]), {"/v1/wait"})
        # Five decisions, each with five minutes of long poll.
        self.assertEqual(self.world.tick, 5 * 300)

    def test_with_some_verbs_the_agent_uses_what_exists(self) -> None:
        town = Town(verbs=["eat", "sleep"])
        self.start(town.world)
        self.resident.items["bread"] = 6
        self.run_days(2)
        self.assertEqual(self.rejected(), [])
        self.assertLessEqual(set(self.sent_verbs()), {"eat", "sleep", "move_to"})
        self.assertEqual([routine["name"] for routine in self.agent.routines], ["eat bread when hungry", "eat apple when hungry", "sleep when tired"])
        self.assertGreater(town.lowest[self.resident.id][FOOD], 200)
        self.assertGreater(town.lowest[self.resident.id][ENERGY], 100)
        self.assertEqual(town.lowest[self.resident.id][HEALTH], 1000)

    def test_a_verb_whose_arguments_the_agent_cannot_fill_is_never_sent(self) -> None:
        town = Town()
        # In this world `eat` needs an argument that the agent does not know.
        eat = next(verb for verb in town.world.catalogue()["verbs"] if verb["verb"] == "eat")
        eat["args_schema"] = {"type": "object", "properties": {"ritual": {"type": "string"}}, "required": ["ritual"]}
        self.start(town.world)
        self.resident.items["bread"] = 6
        self.resident.attrs[FOOD] = 300
        self.run_days(1)
        self.assertNotIn("eat", self.sent_verbs())
        self.assertNotIn("eat bread when hungry", [routine["name"] for routine in self.agent.routines])
        self.assertIn("sleep when tired", [routine["name"] for routine in self.agent.routines])
        # The other rules still ran.
        self.assertIn("apply_job", self.sent_verbs())

    def test_a_new_verb_is_used_after_the_agent_reads_the_catalogue_again(self) -> None:
        town = Town(verbs=["sleep"])
        self.start(town.world, AgentConfig(catalogue_every=3))
        self.resident.items["bread"] = 6
        self.resident.attrs[FOOD] = 300
        for _ in range(3):
            self.agent.step()
        self.assertNotIn("eat", self.sent_verbs())
        self.assertEqual([routine["name"] for routine in self.agent.routines], ["sleep when tired"])
        # The server is upgraded: `eat` appears. The agent reads the catalogue again after three decisions.
        town.world.add_verb(town.verb("eat"))
        for _ in range(3):
            self.agent.step()
        self.assertEqual([routine["name"] for routine in self.agent.routines][:2], ["eat bread when hungry", "eat apple when hungry"])
        self.assertGreaterEqual(town.meals[self.resident.id], 1)


class RoutineFallbackTest(AgentCase):
    def test_when_the_server_refuses_one_routine_the_agent_keeps_the_others(self) -> None:
        town = Town()
        self.start(town.world)

        def has_work(request: Any) -> bool:
            routines = (request.body or {}).get("routines", [])
            return request.path == "/v1/routines" and any(routine["then"]["verb"] == "work" for routine in routines)

        self.gateway.fail_next(400, "invalid_args", "`work` needs the argument `contract`.", times=100, when=has_work)
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.agent.setup()
        kept = ["eat bread when hungry", "eat apple when hungry", "sleep when tired"]
        self.assertEqual([routine["name"] for routine in self.agent.routines], kept)
        self.assertEqual([routine["name"] for routine in self.resident.routines], kept)
        self.assertTrue(any("the routine `work in the day` is left out" in line for line in logs.output))


class ErrorTest(AgentCase):
    def test_an_error_of_the_server_does_not_stop_the_agent(self) -> None:
        town = Town()
        self.start(town.world, AgentConfig(error_seconds=30.0))
        self.agent.step()
        self.gateway.fail_next(500, "internal", "The server failed.", when=lambda request: request.path == "/v1/wait")
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.assertEqual(self.agent.run(decisions=6), 6)
        self.assertIsNone(self.agent.fatal)
        self.assertIn(30.0, self.sleeps)
        self.assertTrue(any("internal: The server failed." in line for line in logs.output))

    def test_a_key_that_stops_to_work_stops_the_agent(self) -> None:
        town = Town()
        self.start(town.world)
        self.agent.step()
        self.world.rotate_key(self.resident)
        with self.assertLogs("scripted_agent", "ERROR"):
            made = self.agent.run(decisions=50)
        self.assertLess(made, 50)
        self.assertEqual(self.agent.fatal.code, "unauthorized")

    def test_run_stops_when_the_stop_event_is_set(self) -> None:
        self.start(FakeWorld(verbs=kernel_verbs(), time="auto"))
        stop = threading.Event()
        stop.set()
        self.assertEqual(self.agent.run(stop=stop), 0)
        self.assertEqual(self.gateway.requests, [])


if __name__ == "__main__":
    unittest.main()
