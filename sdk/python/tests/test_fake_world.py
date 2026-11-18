"""The fake world: the parts that agent tests rely on and that the client tests do not reach."""

import unittest

import _path  # noqa: F401

from unsupervised.testing import Busy, Done, FakeActionError, FakeVerb, FakeWorld, Pending, verb_info

ENERGY = {"name": "need.energy", "min": 0, "max": 1000, "default": 1000, "summary": "0 means exhausted."}
BREAD = {"id": 0, "name": "bread", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 300}, "summary": "A loaf."}


def nap(resident, args, world):
    def wake():
        resident.attrs["need.energy"] = 1000
        return {"rested": True}

    return Busy(10, wake)


def snack(resident, args, world):
    if resident.items.get("bread", 0) < 1:
        raise FakeActionError("insufficient", "You have no bread.")
    resident.items["bread"] -= 1
    return Done({"ate": "bread"})


class RoutineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.world = FakeWorld(attrs=[ENERGY], items=[BREAD])
        self.world.add_verb(FakeVerb(verb_info("nap", "life", "Sleep a little."), nap))
        self.world.add_verb(FakeVerb(verb_info("snack", "life", "Eat bread."), snack))
        self.ada = self.world.spawn(self.world.sign_up()["owner"]["id"], "Ada")

    def routines(self):
        return self.world.observe(self.ada, {"detail": "full"})["sections"]["self"]["routines"]

    def reports(self):
        return self.world.observe(self.ada)["sections"]["reports"]

    def test_a_routine_fires_when_the_resident_is_idle_and_its_condition_holds(self) -> None:
        self.world.set_routines(
            self.ada,
            [{"name": "nap", "when": {"type": "attr_below", "attr": "need.energy", "value": 300}, "then": {"verb": "nap"}}],
        )
        self.world.step(5)
        self.assertIsNone(self.ada.current)
        self.assertEqual(self.routines()[0]["runs"], 0)

        self.ada.attrs["need.energy"] = 200
        self.world.step(1)
        self.assertEqual(self.ada.current.verb, "nap")
        self.world.step(10)
        self.assertEqual(self.ada.attrs["need.energy"], 1000)
        self.assertEqual((self.routines()[0]["runs"], self.routines()[0]["failures"]), (1, 0))
        # A routine action that succeeds is counted and writes no report.
        self.assertEqual(self.reports(), [])

    def test_the_first_routine_that_holds_wins_and_a_busy_resident_starts_none(self) -> None:
        self.ada.items["bread"] = 1
        self.world.set_routines(
            self.ada,
            [
                {"name": "eat", "when": {"type": "has_item", "item": "bread", "qty": 1}, "then": {"verb": "snack"}},
                {"name": "nap", "when": {"type": "always"}, "then": {"verb": "nap"}},
            ],
        )
        self.world.submit(self.ada, {"verb": "wait", "args": {"ticks": 3}})
        self.world.step(2)
        self.assertEqual((self.ada.current.verb, self.ada.items["bread"]), ("wait", 1))
        # The wait ends at tick 3. The first routine that holds fires in the same tick.
        self.world.step(1)
        self.assertEqual(self.ada.items["bread"], 0)
        # One routine per tick: the second one starts in the next tick.
        self.assertIsNone(self.ada.current)
        self.world.step(1)
        self.assertEqual(self.ada.current.verb, "nap")

    def test_a_routine_that_fails_reports_and_waits_longer_each_time(self) -> None:
        self.world.set_routines(self.ada, [{"name": "eat", "when": {"type": "always"}, "then": {"verb": "snack"}}])
        self.world.step(1)
        failures = [(report["verb"], report["ok"], report["error"]["code"], report["tick"]) for report in self.reports()]
        self.assertEqual(failures, [("snack", False, "insufficient", 1)])
        # The backoff is 30 ticks, then 60: the second failure is at tick 31, the third at tick 91.
        self.world.step(100)
        self.assertEqual([report["tick"] for report in self.reports()], [1, 31, 91])
        self.assertEqual(self.routines()[0]["failures"], 3)

    def test_a_clock_condition_follows_the_hour_and_wraps_past_midnight(self) -> None:
        night = {"type": "clock_between", "from_hour": 22, "to_hour": 6}
        day = {"type": "clock_between", "from_hour": 6, "to_hour": 22}
        seen = {}
        for hour in (0, 5, 6, 21, 22, 23):
            self.world.tick = hour * 60
            seen[hour] = (self.world.holds(self.ada, night), self.world.holds(self.ada, day))
        self.assertEqual(
            seen,
            {0: (True, False), 5: (True, False), 6: (False, True), 21: (False, True), 22: (True, False), 23: (True, False)},
        )
        both = {"type": "all", "of": [night, {"type": "attr_above", "attr": "need.energy", "value": 999}]}
        self.assertTrue(self.world.holds(self.ada, both))
        self.ada.attrs["need.energy"] = 999
        self.assertFalse(self.world.holds(self.ada, both))
        self.assertTrue(self.world.holds(self.ada, {"type": "any", "of": [both, night]}))


class EngineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.world = FakeWorld()
        self.ada = self.world.spawn(self.world.sign_up()["owner"]["id"], "Ada")

    def state(self, request):
        return self.world.submit(self.ada, request)["status"]

    def test_a_repeated_request_id_returns_the_first_answer_and_a_rejected_one_is_free_again(self) -> None:
        first = self.world.submit(self.ada, {"id": "a", "verb": "wait", "args": {"ticks": 5}})
        again = self.world.submit(self.ada, {"id": "a", "verb": "wait", "args": {"ticks": 5}})
        self.assertEqual(again, first)
        self.assertEqual(self.ada.queue, [])

        # The resident is busy, so the request goes to the queue when its arguments fit the schema.
        self.assertEqual(self.state({"id": "b", "verb": "wait"})["error"]["code"], "invalid_args")
        self.assertEqual(self.state({"id": "b", "verb": "wait", "args": {"ticks": 2}}), {"state": "queued", "position": 1})

        long_id = self.world.submit(self.ada, {"id": "x" * 65, "verb": "wait", "args": {"ticks": 1}})
        self.assertEqual((long_id["action_id"], long_id["status"]["error"]["code"]), (0, "invalid_args"))
        self.assertNotIn("request_id", long_id)

    def test_the_queue_and_the_free_verbs_have_limits(self) -> None:
        self.assertEqual(self.state({"verb": "wait", "args": {"ticks": 5}})["state"], "started")
        for position in range(1, 9):
            self.assertEqual(self.state({"verb": "wait", "args": {"ticks": 1}}), {"state": "queued", "position": position})
        self.assertEqual(self.state({"verb": "wait", "args": {"ticks": 1}})["error"]["code"], "queue_full")

        # A free verb runs while the resident is busy, four times in one tick.
        cancelled = self.state({"verb": "cancel"})
        self.assertEqual((cancelled["state"], cancelled["result"]["cleared"]), ("done", 8))
        for _ in range(3):
            self.assertEqual(self.state({"verb": "cancel"})["state"], "done")
        self.assertEqual(self.state({"verb": "cancel"})["error"]["code"], "rate_limited")
        self.world.step(1)
        self.assertEqual(self.state({"verb": "cancel"})["state"], "done")
        # Each cancelled action wrote one `interrupted` report.
        reports = self.world.observe(self.ada)["sections"]["reports"]
        self.assertEqual([report["error"]["code"] for report in reports], ["interrupted"] * 9)

    def test_a_pending_action_reports_its_answer_or_a_timeout(self) -> None:
        self.world.add_verb(FakeVerb(verb_info("ask", "bank", "Ask the bank.", scope="global"), lambda r, a, w: Pending(2, lambda: {"balance": 5})))
        self.world.add_verb(FakeVerb(verb_info("hang", "bank", "Never answered.", scope="global"), lambda r, a, w: Pending(3)))
        self.assertEqual(self.state({"verb": "ask"}), {"state": "pending"})
        self.assertEqual(self.state({"verb": "hang"}), {"state": "queued", "position": 1})
        self.assertTrue(self.world.observe(self.ada)["sections"]["self"]["activity"]["waiting"])
        self.world.step(5)
        reports = self.world.observe(self.ada)["sections"]["reports"]
        self.assertEqual(
            [(report["verb"], report["ok"], report.get("result"), report.get("error", {}).get("code")) for report in reports],
            [("ask", True, {"balance": 5}, None), ("hang", False, None, "timeout")],
        )

    def test_the_inbox_keeps_the_newest_entries_and_the_numbers_never_repeat(self) -> None:
        for i in range(40):
            self.world.notify(self.ada.id, "test.note", {"i": i})
        observation = self.world.observe(self.ada)
        self.assertEqual([notice["seq"] for notice in observation["notices"]], list(range(9, 41)))
        self.assertEqual(observation["next_notice"], 41)
        self.assertEqual(self.world.observe(self.ada, {"since_notice": 40})["notices"][0]["data"], {"i": 39})

    def test_on_tick_rules_run_once_for_each_tick(self) -> None:
        ticks = []
        self.world.on_tick.append(lambda world: ticks.append(world.tick))
        self.world.step(3)
        self.assertEqual(ticks, [1, 2, 3])


if __name__ == "__main__":
    unittest.main()
