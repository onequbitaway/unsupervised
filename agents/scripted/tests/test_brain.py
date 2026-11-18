"""The state machine, one observation at a time. No server is used."""

import unittest
from typing import Any, Dict, List, Optional

import _path  # noqa: F401
from town import JOBS, SHELTER, SHOP, SITES, WORKS, Town, kernel_verbs

from scripted_agent import AgentConfig, Brain, Catalogue, State
from unsupervised.testing import FakeWorld

#: The sites as the agent knows them: the id, the kind and a tile inside.
KNOWN_SITES = [
    {"id": site["id"], "kind": site["kind"], "pos": {"x": (site["rect"]["x0"] + site["rect"]["x1"]) // 2, "y": (site["rect"]["y0"] + site["rect"]["y1"]) // 2}}
    for site in SITES
]
SHELTER_POS = {"x": 7, "y": 7}
SHOP_POS = {"x": 43, "y": 7}
WORKS_POS = {"x": 44, "y": 44}


def observation(
    *,
    hour: int = 12,
    food: int = 1000,
    energy: int = 1000,
    cash: int = 0,
    items: Optional[Dict[str, int]] = None,
    site: Optional[Dict[str, Any]] = None,
    pos: Optional[Dict[str, int]] = None,
    activity: Optional[str] = None,
    queue: int = 0,
    conditions: Optional[Dict[str, Any]] = None,
    job: Optional[Dict[str, Any]] = None,
    reports: Optional[List[Dict[str, Any]]] = None,
    tick: Optional[int] = None,
) -> Dict[str, Any]:
    """Builds an observation with the fields that the agent reads."""
    me: Dict[str, Any] = {
        "id": 1,
        "name": "Ada",
        "pos": pos or {"x": 20, "y": 20},
        "zone": 0,
        "city": 0,
        "activity": None if activity is None else {"verb": activity, "action_id": 9, "started": 0, "ends": 99, "waiting": False},
        "queue": queue,
        "conditions": conditions or {},
        "attrs": {"need.food": food, "need.energy": energy, "need.health": 1000},
        "cash": cash,
        "items": items or {},
    }
    if site is not None:
        me["site"] = {"id": site["id"], "name": site["name"], "kind": site["kind"]}
    sections: Dict[str, Any] = {"self": me, "nearby": {"residents": [], "sites": []}, "reports": reports or []}
    if job is not None:
        sections["job"] = job
    return {
        "frame": 0,
        "tick": hour * 60 if tick is None else tick,
        "clock": {"year": 1, "day": 1, "hour": hour, "minute": 0},
        "resident": 1,
        "sections": sections,
        "notices": [],
        "next_notice": 1,
    }


def ack(state: str, **fields: Any) -> Dict[str, Any]:
    return {"action_id": 1, "verb": "any", "status": dict({"state": state}, **fields)}


def failed(verb: str, code: str, routine: bool = False) -> Dict[str, Any]:
    """Builds a failed report. The requests of the agent have an id; the actions of a routine have none."""
    report = {"action_id": 1, "verb": verb, "tick": 0, "ok": False, "error": {"code": code, "message": "No."}}
    if not routine:
        report["request_id"] = "an-id-of-the-agent"
    return report


def done(verb: str, result: Any = None) -> Dict[str, Any]:
    return {"action_id": 1, "verb": verb, "tick": 0, "ok": True, "result": result}


class BrainCase(unittest.TestCase):
    def setUp(self) -> None:
        self.brain = Brain(Catalogue(Town().world.catalogue()), AgentConfig())

    def decide(self, **fields: Any):
        return self.brain.decide(observation(**fields), KNOWN_SITES)

    def steps(self, **fields: Any) -> List[Dict[str, Any]]:
        return self.decide(**fields).steps


class NeedsTest(BrainCase):
    def test_a_blocking_condition_stops_every_action(self) -> None:
        decision = self.decide(conditions={"jailed": 500}, food=10, energy=10, items={"bread": 1})
        self.assertEqual((decision.state, decision.steps, decision.wait_seconds), (State.BLOCKED, [], 300))
        # A condition that does not block changes nothing.
        self.assertEqual(self.decide(conditions={"wanted": None}, food=10, items={"bread": 1}).state, State.EAT)

    def test_planned_actions_in_the_queue_are_left_to_run(self) -> None:
        decision = self.decide(activity="move_to", queue=1, food=10, items={"bread": 1})
        self.assertEqual((decision.state, decision.steps, decision.wait_seconds), (State.BUSY, [], 600))

    def test_a_tired_resident_walks_to_a_bed_and_sleeps(self) -> None:
        decision = self.decide(energy=200)
        self.assertEqual(decision.state, State.SLEEP)
        self.assertEqual(decision.steps, [{"verb": "move_to", "args": SHELTER_POS}, {"verb": "sleep", "args": {}}])
        # In the shelter there is no walk.
        self.assertEqual(self.steps(energy=200, site=SHELTER), [{"verb": "sleep", "args": {}}])

    def test_at_night_the_resident_sleeps_earlier(self) -> None:
        self.assertEqual(self.decide(hour=23, energy=650, site=SHELTER).state, State.SLEEP)
        self.assertEqual(self.decide(hour=3, energy=650, site=SHELTER).state, State.SLEEP)
        self.assertNotEqual(self.decide(hour=21, energy=650, site=SHELTER).state, State.SLEEP)
        self.assertNotEqual(self.decide(hour=23, energy=700, site=SHELTER).state, State.SLEEP)

    def test_a_hungry_resident_eats_the_best_food_that_it_carries(self) -> None:
        decision = self.decide(food=499, items={"apple": 2, "bread": 1, "coal": 4})
        self.assertEqual((decision.state, decision.steps), (State.EAT, [{"verb": "eat", "args": {"item": "bread"}}]))
        self.assertEqual(self.steps(food=499, items={"apple": 2, "coal": 4}), [{"verb": "eat", "args": {"item": "apple"}}])
        # Not hungry: no meal.
        self.assertNotEqual(self.decide(food=500, items={"bread": 3}, hour=20).state, State.EAT)

    def test_sleep_comes_before_food_and_food_before_work(self) -> None:
        both = self.decide(energy=100, food=100, items={"bread": 1}, site=SHELTER)
        self.assertEqual(both.state, State.SLEEP)
        self.brain.employed = True
        self.assertEqual(self.decide(food=100, items={"bread": 3}).state, State.EAT)

    def test_a_resident_low_on_food_with_cash_walks_to_the_shop_and_buys(self) -> None:
        decision = self.decide(cash=900, items={"bread": 1})
        self.assertEqual(decision.state, State.SHOP)
        self.assertEqual(
            decision.steps,
            [{"verb": "move_to", "args": SHOP_POS}, {"verb": "buy", "args": {"item": "bread", "qty": 2}}],
        )
        self.assertEqual(self.steps(cash=900, site=SHOP), [{"verb": "buy", "args": {"item": "bread", "qty": 2}}])
        # Enough food, or no cash: no purchase.
        self.assertNotEqual(self.decide(cash=900, items={"bread": 1, "apple": 1}, hour=20).state, State.SHOP)
        self.assertNotEqual(self.decide(cash=0, hour=20).state, State.SHOP)

    def test_a_purchase_that_fails_for_money_tries_the_next_food(self) -> None:
        self.brain.decide(observation(cash=200, site=SHOP, reports=[failed("buy", "insufficient")]), KNOWN_SITES)
        # `buy` rests for 60 ticks. Then the agent asks for the cheaper item.
        later = self.brain.decide(observation(cash=200, site=SHOP, tick=12 * 60 + 60), KNOWN_SITES)
        self.assertEqual(later.steps, [{"verb": "buy", "args": {"item": "apple", "qty": 2}}])


class WorkTest(BrainCase):
    def test_without_a_job_the_resident_reads_the_jobs_then_applies_for_the_best_one(self) -> None:
        first = self.decide()
        self.assertEqual((first.state, first.steps, first.wait_seconds), (State.JOB_HUNT, [{"verb": "list_jobs", "args": {}}], 0))
        self.brain.learn(first.steps, [ack("done", result={"jobs": JOBS})])

        second = self.decide()
        self.assertEqual(second.state, State.JOB_HUNT)
        # The street cleaner pays more than the shelf filler.
        self.assertEqual(second.steps, [{"verb": "move_to", "args": WORKS_POS}, {"verb": "apply_job", "args": {"job": 1}}])

        # The application succeeds: the report says so, and the section `job` appears.
        job = {"id": 1, "site": WORKS["id"], "wage": 600}
        third = self.decide(reports=[done("apply_job", {"job": 1})], job=job, site=WORKS)
        self.assertEqual((third.state, third.steps), (State.WORK, [{"verb": "work", "args": {}}]))

    def test_a_job_that_refuses_the_resident_is_not_tried_again(self) -> None:
        self.brain.learn(self.steps(), [ack("done", result={"jobs": JOBS})])
        self.assertEqual(self.steps()[-1], {"verb": "apply_job", "args": {"job": 1}})
        decision = self.decide(reports=[failed("apply_job", "not_allowed")], tick=12 * 60 + 100)
        self.assertEqual(decision.steps, [], "`apply_job` rests after a failure")
        retry = self.decide(tick=12 * 60 + 200)
        self.assertEqual(retry.steps, [{"verb": "move_to", "args": SHOP_POS}, {"verb": "apply_job", "args": {"job": 2}}])

    def test_with_a_job_the_resident_walks_to_it_and_works_in_working_hours(self) -> None:
        job = {"id": 1, "site": WORKS["id"], "wage": 600}
        decision = self.decide(job=job, hour=9)
        self.assertEqual(decision.state, State.WORK)
        self.assertEqual(decision.steps, [{"verb": "move_to", "args": WORKS_POS}, {"verb": "work", "args": {}}])
        # Outside working hours, or too tired, the resident does not start a shift.
        self.assertEqual(self.decide(job=job, hour=19).state, State.IDLE)
        self.assertEqual(self.decide(job=job, hour=7).state, State.IDLE)
        self.assertEqual(self.decide(job=job, hour=9, energy=299).state, State.IDLE)

    def test_work_that_fails_for_want_of_a_job_starts_a_new_search(self) -> None:
        self.brain.employed = True
        self.assertEqual(self.decide().state, State.WORK)
        decision = self.decide(reports=[failed("work", "not_allowed")])
        self.assertEqual((decision.state, decision.steps), (State.JOB_HUNT, [{"verb": "list_jobs", "args": {}}]))

    def test_the_list_of_jobs_is_read_from_several_result_forms(self) -> None:
        for result in ({"jobs": JOBS}, {"offers": JOBS}, {"page": 1, "rows": JOBS}, JOBS):
            with self.subTest(result=type(result).__name__ if isinstance(result, list) else list(result)):
                brain = Brain(Catalogue(Town().world.catalogue()))
                brain.learn([{"verb": "list_jobs"}], [ack("done", result=result)])
                self.assertEqual([job["id"] for job in brain.jobs], [1, 2])
        self.brain.learn(self.steps(), [ack("done", result={"jobs": []})])
        self.assertEqual(self.decide().state, State.IDLE, "no open job: nothing to apply for")
        # The agent reads the list again after `jobs_refresh_ticks`.
        self.assertEqual(self.decide(tick=12 * 60 + 719).state, State.IDLE)
        self.assertEqual(self.steps(tick=12 * 60 + 720), [{"verb": "list_jobs", "args": {}}])


class BusyTest(BrainCase):
    def test_a_plan_goes_to_the_queue_behind_a_running_action(self) -> None:
        job = {"id": 1, "site": WORKS["id"], "wage": 600}
        # A shift runs. The resident is hungry and has bread: the meal is planned behind the shift.
        decision = self.decide(activity="work", job=job, site=WORKS, food=300, items={"bread": 2})
        self.assertEqual((decision.state, decision.steps), (State.EAT, [{"verb": "eat", "args": {"item": "bread"}}]))
        # Nothing is urgent: the shift runs on, and no second shift is planned.
        quiet = self.decide(activity="work", job=job, site=WORKS)
        self.assertEqual((quiet.state, quiet.steps, quiet.wait_seconds), (State.BUSY, [], 600))
        # The running action is not planned a second time.
        self.assertEqual(self.decide(activity="sleep", job=job, energy=100, site=SHELTER, hour=20).state, State.BUSY)
        self.assertEqual(self.decide(activity="eat", job=job, food=100, items={"bread": 2}, hour=20).state, State.BUSY)
        # A resident without a job plans its search behind the running action.
        self.assertEqual(self.steps(activity="sleep", energy=100, site=SHELTER, hour=20), [{"verb": "list_jobs", "args": {}}])


class FailureTest(BrainCase):
    def setUp(self) -> None:
        super().setUp()
        # The resident has a job, so a rule that rests leaves the agent idle in the evening.
        self.brain.employed = True

    def test_arguments_that_the_server_refuses_end_the_use_of_the_verb(self) -> None:
        decision = self.decide(food=100, items={"bread": 1}, hour=20)
        self.assertEqual(decision.state, State.EAT)
        rejected = ack("rejected", error={"code": "invalid_args", "message": "`eat` needs the argument `bites`."})
        with self.assertLogs("scripted_agent", "WARNING") as logs:
            self.brain.learn(decision.steps, [rejected])
        self.assertIn("the agent stops to use `eat`", logs.output[0])
        self.assertFalse(self.brain.catalogue.has("eat"))
        after = self.decide(food=100, items={"bread": 3}, hour=20)
        self.assertEqual((after.state, after.steps), (State.IDLE, []))

    def test_the_first_not_here_gets_a_new_try_with_a_walk_and_the_second_a_pause(self) -> None:
        tired = dict(energy=100, hour=20)
        first = self.decide(reports=[failed("sleep", "not_here")], **tired)
        self.assertEqual(first.steps, [{"verb": "move_to", "args": SHELTER_POS}, {"verb": "sleep", "args": {}}])
        second = self.decide(reports=[failed("sleep", "not_here")], **tired)
        self.assertEqual((second.state, second.steps), (State.IDLE, []))
        # The pause after the second failure in a row is two times 60 ticks.
        self.assertEqual(self.decide(tick=20 * 60 + 119, **tired).steps, [])
        self.assertEqual(self.decide(tick=20 * 60 + 120, **tired).state, State.SLEEP)

    def test_a_failure_pauses_the_verb_and_a_success_ends_the_pause(self) -> None:
        self.assertEqual(self.decide().state, State.WORK)
        self.assertEqual(self.decide(reports=[failed("work", "blocked")]).state, State.IDLE)
        self.assertEqual(self.decide(tick=12 * 60 + 59).state, State.IDLE)
        self.assertEqual(self.decide(tick=12 * 60 + 60).state, State.WORK)
        # Three failures in a row: 60, 120, then 240 ticks.
        for _ in range(2):
            self.decide(reports=[failed("work", "blocked")])
        self.assertEqual(self.brain.pause_until["work"], 12 * 60 + 240)
        self.decide(reports=[done("work", {"wage": 600})])
        self.assertEqual(self.brain.failures, {})
        self.assertEqual(self.decide().state, State.WORK)

    def test_a_failure_of_a_routine_action_does_not_pause_the_verb(self) -> None:
        # The work routine fired in the shelter, three times. The agent still walks to the job.
        reports = [failed("work", "not_here", routine=True)] * 3
        decision = self.decide(reports=reports, job={"id": 1, "site": WORKS["id"]}, site=SHELTER)
        self.assertEqual(decision.state, State.WORK)
        self.assertEqual(decision.steps, [{"verb": "move_to", "args": WORKS_POS}, {"verb": "work", "args": {}}])
        self.assertEqual((self.brain.failures, self.brain.pause_until), ({}, {}))
        # It still learns from it: a routine that works without a job starts a search.
        hunt = self.decide(reports=[failed("work", "not_allowed", routine=True)])
        self.assertEqual((hunt.state, self.brain.employed), (State.JOB_HUNT, False))

    def test_an_interrupted_action_is_not_a_failure(self) -> None:
        self.assertEqual(self.decide(reports=[failed("work", "interrupted")]).state, State.WORK)
        self.assertEqual(self.brain.pause_until, {})


class MissingVerbsTest(unittest.TestCase):
    def test_with_only_the_kernel_verbs_the_agent_stays_idle_and_plans_nothing(self) -> None:
        brain = Brain(Catalogue(FakeWorld(verbs=kernel_verbs()).catalogue()))
        for fields in (dict(), dict(food=0, energy=0, items={"bread": 5}, cash=5000), dict(hour=23, energy=10)):
            decision = brain.decide(observation(**fields), KNOWN_SITES)
            self.assertEqual((decision.state, decision.steps, decision.wait_seconds), (State.IDLE, [], 300))

    def test_each_rule_needs_only_its_own_verb(self) -> None:
        cases = [
            # (town verbs, observation, expected state, expected verbs)
            (["eat"], dict(food=100, items={"bread": 1}, energy=100), State.EAT, ["eat"]),
            (["sleep"], dict(food=100, items={"bread": 1}, energy=100), State.SLEEP, ["move_to", "sleep"]),
            (["buy"], dict(cash=900), State.SHOP, ["move_to", "buy"]),
            # `work` without `apply_job`: the world has no explicit jobs, so the agent just works.
            (["work"], dict(), State.WORK, ["work"]),
            (["apply_job"], dict(), State.IDLE, []),
            (["list_jobs", "apply_job"], dict(), State.JOB_HUNT, ["list_jobs"]),
        ]
        for verbs, fields, state, expected in cases:
            with self.subTest(verbs=verbs):
                brain = Brain(Catalogue(Town(verbs=verbs).world.catalogue()))
                decision = brain.decide(observation(**fields), KNOWN_SITES)
                self.assertEqual((decision.state, [step["verb"] for step in decision.steps]), (state, expected))

    def test_without_move_to_the_agent_acts_where_it_is(self) -> None:
        raw = Town().world.catalogue()
        raw["verbs"] = [verb for verb in raw["verbs"] if verb["verb"] != "move_to"]
        brain = Brain(Catalogue(raw))
        self.assertEqual(brain.decide(observation(energy=100), KNOWN_SITES).steps, [{"verb": "sleep", "args": {}}])

    def test_without_a_known_site_the_agent_acts_where_it_is(self) -> None:
        brain = Brain(Catalogue(Town().world.catalogue()))
        self.assertEqual(brain.decide(observation(energy=100), []).steps, [{"verb": "sleep", "args": {}}])

    def test_a_new_catalogue_keeps_what_the_agent_gave_up_on(self) -> None:
        brain = Brain(Catalogue(Town(verbs=["eat"]).world.catalogue()))
        brain.catalogue.give_up("eat", "test")
        brain.use_catalogue(Catalogue(Town().world.catalogue()))
        self.assertFalse(brain.catalogue.has("eat"))
        self.assertTrue(brain.catalogue.has("sleep"))


if __name__ == "__main__":
    unittest.main()
