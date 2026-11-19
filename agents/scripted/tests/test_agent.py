"""The agent against the town, through the SDK.

The requests go to the fake gateway without a network: a run of some game
days is hundreds of requests, and a socket for each of them is not needed.
`test_cli.py` and `test_readme.py` run the agent over a local HTTP server.
"""

import threading
import unittest
from typing import Any, List

import _path  # noqa: F401
from town import BANK, ENERGY, FOOD, HEALTH, SHELTER, TICKS_PER_DAY, Town, kernel_verbs

from scripted_agent import Agent, AgentConfig, State
from scripted_agent.brain import PHRASES
from unsupervised.testing import FakeGateway, FakeWorld, direct_client

DAYS = 4


class AgentCase(unittest.TestCase):
    """A town (or another world), its gateway, one resident and an agent for it."""

    def start(self, town_or_world: Any, config: AgentConfig = AgentConfig()) -> None:
        town = town_or_world if isinstance(town_or_world, Town) else None
        self.world: FakeWorld = town.world if town else town_or_world
        self.gateway = FakeGateway(self.world)
        self.resident = town.spawn() if town else self.world.spawn(self.world.sign_up()["owner"]["id"], "Ada")
        self.client = direct_client(self.gateway, self.resident.key)
        self.sleeps: List[float] = []
        # One tick is one second: the clock of the agent is the tick of the world.
        world = self.world
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

    def reports(self) -> List[Any]:
        return [entry["report"] for entry in self.resident.inbox if "report" in entry]


class TownTest(AgentCase):
    def test_the_agent_keeps_a_resident_alive_fed_at_work_and_saving_for_days(self) -> None:
        town = Town()
        self.start(town, AgentConfig(save_above=4000, keep_cash=2000))
        self.run_days(DAYS)

        lowest = town.lowest[self.resident.id]
        self.assertGreater(lowest[FOOD], 300, "the resident never came near starving")
        self.assertGreater(lowest[ENERGY], 100, "the resident never came near exhaustion")
        self.assertEqual(lowest[HEALTH], 1000)
        # It worked starter shifts, bought its own food at the depot, ate it and saved.
        # Two starter shifts each day: the rest of the budget of the city is for others.
        self.assertGreaterEqual(town.shifts[self.resident.id], 2 * DAYS - 1)
        self.assertLessEqual(town.shifts[self.resident.id], 2 * DAYS)
        # The needs lose 336 food points in one day. That is more than one meal for each day.
        self.assertGreaterEqual(town.meals[self.resident.id], DAYS + 2)
        self.assertGreater(town.deposits[self.resident.id], 0)
        verbs = self.sent_verbs()
        for verb in ("list_jobs", "move_to", "work", "buy", "eat", "sleep", "open_account", "deposit"):
            self.assertIn(verb, verbs)
        # Every verb that it sent is in the catalogue, and the server refused no plan.
        known = {verb["verb"] for verb in self.world.catalogue()["verbs"]}
        self.assertLessEqual(set(verbs), known)
        self.assertEqual([report for report in self.reports() if not report["ok"]], [])
        self.assertEqual(
            [routine["name"] for routine in self.agent.routines],
            ["eat meal when hungry", "eat bread when hungry", "eat vegetables when hungry", "sleep when exhausted"],
        )
        # It sleeps each night, and always at home: the sleep of the shelter is fair.
        slept = town.sleeps[self.resident.id]
        self.assertGreaterEqual(len(slept), DAYS - 1)
        self.assertEqual(set(slept), {"fair"})

    def test_the_agent_is_cheap_a_few_requests_for_each_game_hour(self) -> None:
        town = Town()
        self.start(town)
        self.run_days(DAYS)
        requests = len(self.gateway.requests)
        hours = DAYS * 24
        self.assertLess(requests, 3 * hours, "%d requests in %d game hours" % (requests, hours))
        self.assertLess(self.agent.decisions, 1.5 * hours)
        # The agent never slept to wait for the server: the long poll did the waiting.
        self.assertEqual([delay for delay in self.sleeps if delay > 2.0], [])
        # It read the map of the city one time.
        self.assertEqual(len(self.gateway.requests_to("/v1/world")), 1)

    def test_without_the_agent_the_resident_starves(self) -> None:
        # The control for the tests above: the town is hard enough to hurt a resident that does nothing.
        town = Town()
        resident = town.spawn()
        town.world.step(DAYS * TICKS_PER_DAY)
        self.assertEqual(town.lowest[resident.id][FOOD], 0)
        self.assertEqual(town.lowest[resident.id][ENERGY], 0)
        self.assertLess(resident.attrs[HEALTH], 1000)

    def test_without_budget_for_starter_shifts_the_agent_lives_from_the_farm(self) -> None:
        town = Town()
        town.budget_open = False
        self.start(town)
        self.run_days(DAYS)
        self.assertNotIn(self.resident.id, town.shifts)
        self.assertGreater(town.lowest[self.resident.id][FOOD], 300)
        self.assertEqual(town.lowest[self.resident.id][HEALTH], 1000)
        verbs = self.sent_verbs()
        self.assertIn("gather", verbs)
        # It sold what it did not need at the depot.
        self.assertIn("sell", verbs)
        self.assertGreater(self.resident.cash, 0)

    def test_the_agent_takes_a_job_that_pays_more_than_the_city(self) -> None:
        town = Town()
        town.jobs.append({"job": 7, "site": BANK["id"], "wage": 3400, "min_level": 0, "free_slots": 1})
        self.start(town)
        self.run_days(2)
        self.assertEqual(self.resident.sections["job"]["job"], 7)
        paid = [report["result"]["wage"] for report in self.reports() if report["verb"] == "work"]
        self.assertEqual(set(paid), {3400})

    def test_routines_alone_keep_a_placed_resident_fed_and_rested_without_the_agent(self) -> None:
        # The agent installs the routines. Then nobody decides for four days.
        town = Town()
        self.start(town)
        self.resident.items["bread"] = 6
        self.agent.setup()
        self.assertEqual(len(self.resident.routines), 4)
        before = len(self.gateway.requests)
        self.world.step(4 * TICKS_PER_DAY)
        self.assertEqual(len(self.gateway.requests), before)
        # The first meal comes when the food is below 300, after about 50 hours.
        self.assertGreaterEqual(town.meals[self.resident.id], 2)
        self.assertGreater(town.lowest[self.resident.id][FOOD], 250)
        # It stands in the shelter, so the sleep routine gives a fair sleep.
        self.assertEqual(self.world.site_at(self.resident.pos)["id"], SHELTER["id"])
        self.assertGreater(town.lowest[self.resident.id][ENERGY], 100)
        self.assertEqual(town.lowest[self.resident.id][HEALTH], 1000)

    def test_a_sleep_of_a_routine_holds_the_agent_only_until_the_resident_wakes(self) -> None:
        # A resident that is too tired stands in the shelter. The reserve routine of the agent
        # fires before the agent decides: the server starts the sleep, and it writes no report.
        town = Town()
        self.start(town)
        self.agent.setup()
        self.resident.attrs[ENERGY] = 100
        self.world.step(1)
        self.assertEqual(self.resident.current.verb, "sleep")
        ends = self.resident.current.ends
        self.assertEqual(ends, 1 + 600)

        busy = self.agent.step()
        self.assertEqual(busy.state, State.BUSY)
        # The agent did not wake the resident, and it waited no longer than the sleep.
        self.assertNotIn("cancel", self.sent_verbs())
        self.assertEqual(self.world.tick, ends)
        self.assertEqual(town.sleeps[self.resident.id], ["fair"])
        # The next decision comes in the tick in which the resident woke.
        self.assertNotEqual(self.agent.step().state, State.BUSY)
        self.assertEqual(self.gateway.requests_to("/v1/wait")[0].query["timeout"], "60")

    def test_two_residents_that_meet_talk_and_refused_words_make_one_silent(self) -> None:
        town = Town()
        self.start(town, AgentConfig(talk_every_ticks=60))
        town.spawn("Bo")
        self.run_days(1)
        self.assertGreaterEqual(len(town.said), 1)
        self.assertIn("say", self.sent_verbs())

        # A server that refuses the words: the agent goes on and says nothing more.
        self.gateway.text_rule = lambda text: "word" if text in PHRASES else None
        said = len(town.said)
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.run_days(2)
        self.assertEqual(len(town.said), said)
        self.assertTrue(any("refused the words" in line for line in logs.output))
        self.assertIsNone(self.agent.fatal)


class MissingVerbsTest(AgentCase):
    def test_with_only_the_kernel_verbs_the_agent_stays_idle_and_sends_no_action(self) -> None:
        self.start(FakeWorld(verbs=kernel_verbs(), time="auto"))
        decisions = [self.agent.step() for _ in range(5)]
        self.assertEqual({decision.state for decision in decisions}, {State.IDLE})
        self.assertEqual(self.sent_verbs(), [])
        self.assertEqual(self.agent.routines, [])
        self.assertEqual(self.resident.routines, [])
        paths = [request.path for request in self.gateway.requests]
        self.assertEqual(paths[:5], ["/v1/catalogue", "/v1/routines", "/v1/observe", "/v1/world", "/v1/world/zones/0"])
        self.assertEqual(set(paths[5:]), {"/v1/wait"})
        # Five decisions, each with a long poll to the next full hour.
        self.assertEqual(self.world.tick, 5 * 60)

    def test_with_some_verbs_the_agent_uses_what_exists(self) -> None:
        town = Town(verbs=["eat", "sleep"])
        self.start(town)
        self.resident.items["bread"] = 6
        self.run_days(2)
        self.assertEqual([report for report in self.reports() if not report["ok"]], [])
        self.assertLessEqual(set(self.sent_verbs()), {"eat", "sleep", "move_to"})
        self.assertGreater(town.lowest[self.resident.id][FOOD], 300)
        self.assertGreater(town.lowest[self.resident.id][ENERGY], 100)

    def test_a_new_verb_is_used_after_the_agent_reads_the_catalogue_again(self) -> None:
        town = Town(verbs=["sleep"])
        self.start(town, AgentConfig(catalogue_every=3))
        self.resident.items["bread"] = 6
        self.resident.attrs[FOOD] = 300
        for _ in range(3):
            self.agent.step()
        self.assertNotIn("eat", self.sent_verbs())
        self.assertEqual([routine["name"] for routine in self.agent.routines], ["sleep when exhausted"])
        # The server is upgraded: `eat` appears. The agent reads the catalogue again after three decisions.
        town.world.add_verb(town.verb("eat"))
        for _ in range(3):
            self.agent.step()
        self.assertEqual(self.agent.routines[0]["name"], "eat meal when hungry")
        self.assertGreaterEqual(town.meals[self.resident.id], 1)


class RoutineFallbackTest(AgentCase):
    def test_when_the_server_refuses_one_routine_the_agent_keeps_the_others(self) -> None:
        town = Town()
        self.start(town)

        def has_sleep(request: Any) -> bool:
            routines = (request.body or {}).get("routines", [])
            return request.path == "/v1/routines" and any(routine["then"]["verb"] == "sleep" for routine in routines)

        self.gateway.fail_next(400, "invalid_args", "`sleep` needs the argument `bed`.", times=100, when=has_sleep)
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.agent.setup()
        kept = ["eat meal when hungry", "eat bread when hungry", "eat vegetables when hungry"]
        self.assertEqual([routine["name"] for routine in self.agent.routines], kept)
        self.assertEqual([routine["name"] for routine in self.resident.routines], kept)
        self.assertTrue(any("the routine `sleep when exhausted` is left out" in line for line in logs.output))


class ErrorTest(AgentCase):
    def test_an_error_of_the_server_does_not_stop_the_agent(self) -> None:
        town = Town()
        self.start(town, AgentConfig(error_seconds=30.0))
        self.agent.step()
        self.gateway.fail_next(500, "internal", "The server failed.", when=lambda request: request.path == "/v1/wait")
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.assertEqual(self.agent.run(decisions=6), 6)
        self.assertIsNone(self.agent.fatal)
        self.assertIn(30.0, self.sleeps)
        self.assertTrue(any("internal: The server failed." in line for line in logs.output))

    def test_a_key_that_ends_stops_the_agent(self) -> None:
        gone = (
            "resident 1 left the world: it died, or an administrator removed it. Its key does not work any more. "
            "Run the command again to make a successor"
        )
        rekeyed = (
            "the server does not know the key of resident 1 any more: another key replaced it. "
            "Run the command again to get a new key"
        )
        banned = "the key of resident 1 does not work any more (banned: An administrator banned the owner of this token.)"

        def use_the_key_elsewhere() -> None:
            # Another client of the resident gets the one `resident_gone`. The agent gets `invalid_token`.
            self.world.die(self.resident)
            self.assertEqual(self.gateway.handle("GET", "/v1/me", {"Authorization": "Bearer " + self.resident.key}).status, 410)

        cases = [
            # The server answers `resident_gone` to the first call after a death, and `invalid_token` after it.
            ("resident_gone", lambda: self.world.die(self.resident), True, gone),
            ("invalid_token", use_the_key_elsewhere, True, gone),
            ("invalid_token", lambda: self.world.despawn(self.resident), True, gone),
            ("invalid_token", lambda: self.world.rotate_key(self.resident), True, rekeyed),
            ("banned", lambda: self.gateway.banned.add(self.resident.owner), True, banned),
        ]
        self.assertEqual(len(cases), 5)
        for code, end, with_owner_token, farewell in cases:
            with self.subTest(code=code, farewell=farewell):
                town = Town()
                self.start(town)
                if with_owner_token:
                    self.client.owner_token = next(
                        token for token, owner in self.world._owners.items() if owner == self.resident.owner
                    )
                self.agent.step()
                end()
                with self.assertLogs("scripted_agent", "ERROR") as logs:
                    made = self.agent.run(decisions=50)
                self.assertLess(made, 50)
                self.assertEqual(self.agent.fatal.code, code)
                self.assertEqual(self.agent.farewell, farewell)
                self.assertEqual([record.getMessage() for record in logs.records], [farewell + "; the agent stops"])

    def test_without_an_owner_token_an_unknown_key_keeps_the_words_of_the_server(self) -> None:
        self.start(Town())
        self.agent.step()
        self.world.despawn(self.resident)
        with self.assertLogs("scripted_agent", "ERROR"):
            self.agent.run(decisions=50)
        self.assertEqual(
            self.agent.farewell,
            "the key of resident 1 does not work any more "
            "(invalid_token: The server does not know this token. It is wrong or it was replaced.)",
        )

    def test_a_mute_does_not_stop_the_agent(self) -> None:
        town = Town()
        self.start(town, AgentConfig(talk_every_ticks=60))
        town.spawn("Bo")
        self.gateway.muted.add(self.resident.id)
        with self.assertLogs("scripted_agent", "WARNING"):
            self.run_days(1)
        self.assertIsNone(self.agent.fatal)
        self.assertEqual(town.said, [])
        self.assertGreaterEqual(town.shifts[self.resident.id], 2)

    def test_run_stops_when_the_stop_event_is_set(self) -> None:
        self.start(FakeWorld(verbs=kernel_verbs(), time="auto"))
        stop = threading.Event()
        stop.set()
        self.assertEqual(self.agent.run(stop=stop), 0)
        self.assertEqual(self.gateway.requests, [])


if __name__ == "__main__":
    unittest.main()
