"""The state machine, one observation at a time. No server is used."""

import unittest
from typing import Any, Dict, List, Optional

import _path  # noqa: F401
from town import BANK, CITY_HALL, DEPOT, FARM, SHELTER, SHOP, SITES, Town, kernel_verbs

from scripted_agent import AgentConfig, Brain, Catalogue, State
from unsupervised.testing import FakeWorld

#: The sites as the agent knows them: the id, the kind and the middle.
KNOWN_SITES = [
    {
        "id": site["id"],
        "kind": site["kind"],
        "pos": {"x": (site["rect"]["x0"] + site["rect"]["x1"]) // 2, "y": (site["rect"]["y0"] + site["rect"]["y1"]) // 2},
    }
    for site in SITES
]
HOME = {"site": SHELTER["id"], "kind": "shelter", "zone": 0, "pos": {"x": 7, "y": 7}, "sleep": "fair"}
WORKS = {"kinds": ["city_hall", "depot"], "minutes": 240, "open": True, "wage": 2000}


def observation(
    *,
    hour: int = 12,
    minute: int = 0,
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
    bank: Optional[Dict[str, Any]] = None,
    reports: Optional[List[Dict[str, Any]]] = None,
    neighbours: Optional[List[Dict[str, Any]]] = None,
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
    sections: Dict[str, Any] = {
        "self": me,
        "home": HOME,
        "nearby": {"residents": neighbours or [], "sites": []},
        "reports": reports or [],
    }
    if job is not None:
        sections["job"] = job
    if bank is not None:
        sections["bank"] = bank
    return {
        "frame": 0,
        "tick": hour * 60 + minute if tick is None else tick,
        "clock": {"year": 1, "day": 1, "hour": hour, "minute": minute},
        "resident": 1,
        "sections": sections,
        "notices": [],
        "next_notice": 1,
    }


def jobs_report(jobs: Optional[List[Dict[str, Any]]] = None, works: Optional[Dict[str, Any]] = WORKS) -> Dict[str, Any]:
    """Returns the report of `list_jobs`, as the real game writes it."""
    result: Dict[str, Any] = {"city": 0, "jobs": jobs or [], "more": 0}
    if works is not None:
        result["public_works"] = works
    return {"seq": 1, "action_id": 5, "request_id": "r1", "verb": "list_jobs", "tick": 1, "ok": True, "result": result}


def failed(verb: str, code: str, request_id: Optional[str] = "r1") -> Dict[str, Any]:
    report: Dict[str, Any] = {"seq": 2, "action_id": 6, "verb": verb, "tick": 1, "ok": False}
    report["error"] = {"code": code, "message": "The game refused it."}
    if request_id is not None:
        report["request_id"] = request_id
    return report


def walk(site: Dict[str, Any]) -> Dict[str, Any]:
    return {"verb": "move_to", "args": {"site": site["id"]}}


class BrainCase(unittest.TestCase):
    def setUp(self) -> None:
        self.town = Town()
        self.brain = Brain(Catalogue(self.town.world.catalogue()), AgentConfig(talk_every_ticks=0))

    def decide(self, **fields: Any):
        return self.brain.decide(observation(**fields), KNOWN_SITES)

    def with_jobs(self, **fields: Any):
        """Decides for an observation that also has the answer of `list_jobs`."""
        fields.setdefault("reports", [jobs_report()])
        return self.decide(**fields)


class RulesTest(BrainCase):
    def test_a_blocking_condition_or_a_running_action_means_wait(self) -> None:
        jailed = self.decide(conditions={"jailed": 900}, food=100)
        self.assertEqual((jailed.state, jailed.steps), (State.BLOCKED, []))
        # A condition that does not block changes nothing.
        self.assertNotEqual(self.decide(conditions={"asleep": 900}).state, State.BLOCKED)
        busy = self.decide(activity="work", food=100, items={"bread": 1})
        self.assertEqual((busy.state, busy.steps), (State.BUSY, []))
        self.assertEqual(self.decide(queue=2).state, State.BUSY)

    def test_a_tired_resident_walks_home_and_sleeps_until_rested(self) -> None:
        decision = self.decide(energy=200)
        self.assertEqual(decision.state, State.SLEEP)
        self.assertEqual(decision.steps, [walk(SHELTER), {"verb": "sleep", "args": {}}])
        # At home it sleeps without a walk.
        self.assertEqual(self.decide(energy=200, site=SHELTER).steps, [{"verb": "sleep", "args": {}}])
        # In the day a resident with some energy does not sleep. At night it does.
        self.assertNotEqual(self.decide(energy=700, hour=12).state, State.SLEEP)
        self.assertEqual(self.decide(energy=700, hour=23).state, State.SLEEP)
        self.assertEqual(self.decide(energy=700, hour=2).state, State.SLEEP)
        self.assertNotEqual(self.decide(energy=800, hour=2).state, State.SLEEP)

    def test_a_hungry_resident_with_food_eats(self) -> None:
        decision = self.decide(food=500, items={"vegetables": 3})
        self.assertEqual((decision.state, decision.steps), (State.EAT, [{"verb": "eat", "args": {}}]))
        self.assertNotEqual(self.decide(food=700, items={"vegetables": 3}).state, State.EAT)
        # Grain is not food.
        self.assertNotEqual(self.decide(food=500, items={"grain": 3}, cash=0).state, State.EAT)

    def test_a_resident_low_on_food_buys_the_cheapest_nutrition_at_the_depot(self) -> None:
        # Vegetables cost 750 for 120 points, bread 2000 for 300: vegetables are cheaper for one point.
        decision = self.decide(cash=5000)
        self.assertEqual(decision.state, State.SHOP)
        buy = {"verb": "buy", "args": {"item": "vegetables", "qty": 3, "max_price": 750}}
        self.assertEqual(decision.steps, [walk(DEPOT), buy])
        # It buys what the cash pays for.
        self.assertEqual(self.decide(cash=1600, site=DEPOT).steps, [dict(buy, args=dict(buy["args"], qty=2))])
        # With food for now, it buys nothing.
        self.assertNotEqual(self.decide(cash=5000, items={"bread": 2}).state, State.SHOP)

    def test_the_prices_come_from_the_catalogue(self) -> None:
        raw = self.town.world.catalogue()
        for param in raw["params"]:
            if param["name"] == "economy.depot_ceiling.bread":
                param["value"] = 600
        brain = Brain(Catalogue(raw), AgentConfig(talk_every_ticks=0))
        decision = brain.decide(observation(cash=5000), KNOWN_SITES)
        self.assertEqual(decision.steps[1]["args"], {"item": "bread", "qty": 3, "max_price": 600})

    def test_a_hungry_resident_without_money_gathers_food_on_a_farm(self) -> None:
        decision = self.decide(food=500, cash=100)
        self.assertEqual(decision.state, State.GATHER)
        self.assertEqual(decision.steps, [walk(FARM), {"verb": "gather", "args": {"item": "vegetables", "minutes": 60}}])
        # A resident that is not hungry yet looks for paid work first.
        self.assertEqual(self.decide(food=900, cash=100).state, State.JOB_HUNT)

    def test_in_working_hours_the_agent_reads_the_jobs_then_works_a_starter_shift(self) -> None:
        first = self.decide(items={"bread": 2})
        self.assertEqual((first.state, first.steps), (State.JOB_HUNT, [{"verb": "list_jobs", "args": {}}]))
        # The answer is a report. The city hall is nearer than the depot.
        second = self.with_jobs(items={"bread": 2})
        self.assertEqual(second.state, State.WORK)
        self.assertEqual(second.steps, [walk(CITY_HALL), {"verb": "work", "args": {}}])
        self.assertEqual(self.decide(items={"bread": 2}, site=DEPOT, pos={"x": 43, "y": 7}).steps, [{"verb": "work", "args": {}}])
        # Outside the working hours, and without energy, it does not work.
        self.assertEqual(self.decide(items={"bread": 2}, hour=21, energy=800, site=SHELTER).state, State.IDLE)
        # A rested resident also works at night.
        self.assertEqual(self.decide(items={"bread": 2}, hour=21, energy=950).state, State.WORK)
        # Too tired for a shift, with energy for one hour on a farm.
        self.assertEqual(self.decide(items={"bread": 2}, energy=400).state, State.GATHER)
        # Too tired for any work: it goes home to sleep, before the sleep routine fires at the workplace.
        self.assertEqual(self.decide(items={"bread": 2}, energy=280).state, State.SLEEP)

    def test_after_the_starter_shifts_of_the_day_the_agent_leaves_the_budget_to_others(self) -> None:
        shift = {"seq": 3, "action_id": 8, "request_id": "r", "verb": "work", "tick": 1, "ok": True}
        shift["result"] = {"public_works": True, "wage": 2000, "net": 1800, "tax": 200}
        self.assertEqual(self.with_jobs(items={"bread": 2}).state, State.WORK)
        self.assertEqual(self.decide(items={"bread": 2}, reports=[shift]).state, State.WORK)
        self.assertEqual(self.decide(items={"bread": 2}, reports=[shift]).state, State.GATHER)
        # The next day has new shifts.
        tomorrow = observation(items={"bread": 2})
        tomorrow["clock"]["day"] = 2
        self.assertEqual(self.brain.decide(tomorrow, KNOWN_SITES).state, State.WORK)

    def test_after_the_working_day_the_resident_walks_home(self) -> None:
        evening = self.decide(items={"bread": 2}, hour=21, energy=800, site=DEPOT)
        self.assertEqual((evening.state, evening.steps), (State.HOME, [walk(SHELTER)]))
        # At home it stays. In the working day it does not go home.
        self.assertEqual(self.decide(items={"bread": 2}, hour=21, energy=800, site=SHELTER).steps, [])
        self.assertNotEqual(self.with_jobs(items={"bread": 2}, hour=12, site=DEPOT).state, State.HOME)

    def test_a_resident_with_a_job_works_there(self) -> None:
        job = {"job": 7, "site": SHOP["id"], "zone": 0, "kind": "shop", "wage": 2600, "shift_minutes": 240}
        decision = self.decide(items={"bread": 2}, job=job)
        self.assertEqual((decision.state, decision.steps), (State.WORK, [walk(SHOP), {"verb": "work", "args": {}}]))

    def test_the_agent_takes_a_job_that_pays_more_than_a_starter_shift(self) -> None:
        offers = [
            {"job": 3, "site": SHOP["id"], "wage": 2600, "min_level": 0, "free_slots": 1},
            {"job": 4, "site": FARM["id"], "wage": 9000, "min_level": 2, "free_slots": 1},
            {"job": 5, "site": FARM["id"], "wage": 1500, "min_level": 0, "free_slots": 3},
        ]
        decision = self.decide(items={"bread": 2}, reports=[jobs_report(offers)])
        # Job 4 needs a level that a newcomer does not have, and job 5 pays less than the city.
        self.assertEqual(decision.state, State.JOB_HUNT)
        self.assertEqual(decision.steps, [walk(SHOP), {"verb": "apply_job", "args": {"job": 3}}])

    def test_without_budget_for_starter_shifts_the_agent_gathers_and_sells(self) -> None:
        decision = self.decide(items={"bread": 2}, reports=[jobs_report(works=dict(WORKS, open=False))])
        self.assertEqual(decision.state, State.GATHER)
        self.assertEqual(decision.steps[0], walk(FARM))
        # With more vegetables than it needs, it sells a part at the depot and keeps a reserve.
        sale = self.decide(items={"vegetables": 15})
        self.assertEqual(sale.state, State.SHOP)
        self.assertEqual(sale.steps, [walk(DEPOT), {"verb": "sell", "args": {"item": "vegetables", "qty": 11}}])

    def test_a_resident_with_much_cash_opens_an_account_and_saves(self) -> None:
        decision = self.decide(cash=9000, items={"bread": 2})
        self.assertEqual(decision.state, State.SAVE)
        # The fee of the account stays out of the deposit.
        deposit = {"verb": "deposit", "args": {"amount": 9000 - 3000 - 100}}
        self.assertEqual(decision.steps, [walk(BANK), {"verb": "open_account", "args": {}}, deposit])
        # With an account, it only deposits.
        again = self.decide(cash=9000, items={"bread": 2}, bank={"balance": 5900}, site=BANK)
        self.assertEqual(again.steps, [{"verb": "deposit", "args": {"amount": 6000}}])
        self.assertNotEqual(self.decide(cash=6000, items={"bread": 2}).state, State.SAVE)

    def test_an_idle_agent_waits_to_the_next_full_hour(self) -> None:
        decision = self.decide(hour=21, minute=40, items={"bread": 2}, energy=800, site=SHELTER)
        self.assertEqual((decision.state, decision.wait_ticks), (State.IDLE, 20))


class VarietyTest(unittest.TestCase):
    def brain(self, variety: int) -> Brain:
        return Brain(Catalogue(Town().world.catalogue()), AgentConfig(talk_every_ticks=0), variety)

    def test_residents_of_one_command_choose_different_places_of_the_same_worth(self) -> None:
        chosen = set()
        for variety in range(4):
            decision = self.brain(variety).decide(observation(items={"bread": 2}, reports=[jobs_report()]), KNOWN_SITES)
            self.assertEqual(decision.state, State.WORK)
            chosen.add(decision.steps[0]["args"]["site"])
        # The city hall and the depot both have starter shifts.
        self.assertEqual(chosen, {CITY_HALL["id"], DEPOT["id"]})
        # A resident that stands in one of them works there.
        here = self.brain(1).decide(observation(items={"bread": 2}, reports=[jobs_report()], site=CITY_HALL), KNOWN_SITES)
        self.assertEqual(here.steps, [{"verb": "work", "args": {}}])

    def test_residents_of_one_command_use_different_farms(self) -> None:
        sites = KNOWN_SITES + [{"id": 41, "kind": "farm", "pos": {"x": 60, "y": 60}}]
        farms = {self.brain(variety).decide(observation(food=500), sites).steps[0]["args"]["site"] for variety in range(2)}
        self.assertEqual(farms, {FARM["id"], 41})

    def test_the_configuration_of_each_resident_differs_a_little(self) -> None:
        config = AgentConfig()
        self.assertEqual(config.varied(0), config)
        varied = [config.varied(index) for index in range(12)]
        self.assertGreater(len({(one.work_from, one.work_to, one.eat_below) for one in varied}), 6)
        for one in varied:
            # The working day stays long enough for two shifts, and the resident eats before it is hungry.
            self.assertGreaterEqual(one.work_to - one.work_from, 10)
            self.assertGreater(one.eat_below, config.routine_eat_below)
        self.assertEqual(AgentConfig(talk_every_ticks=0).varied(5).talk_every_ticks, 0)


class LearnTest(BrainCase):
    def test_a_refused_verb_is_left_alone_for_a_time_that_doubles(self) -> None:
        self.with_jobs(items={"bread": 2})
        refused = self.decide(items={"bread": 2}, reports=[failed("work", "not_here")], tick=720)
        self.assertNotEqual(refused.state, State.WORK)
        self.assertEqual(self.brain.pause_until["work"], 720 + 60)
        self.decide(items={"bread": 2}, reports=[failed("work", "not_here")], tick=800)
        self.assertEqual(self.brain.pause_until["work"], 800 + 120)
        # After the pause the agent tries again.
        self.assertEqual(self.with_jobs(items={"bread": 2}, tick=721 + 200 + 720, hour=12).state, State.WORK)

    def test_a_shift_without_budget_closes_the_starter_shifts_until_the_next_list(self) -> None:
        self.with_jobs(items={"bread": 2})
        self.decide(items={"bread": 2}, reports=[failed("work", "insufficient")])
        self.assertIs(self.brain.public_works["open"], False)
        self.assertIsNone(self.brain.jobs_tick)

    def test_arguments_that_the_server_refuses_end_the_use_of_the_verb(self) -> None:
        with self.assertLogs("scripted_agent", "WARNING"):
            self.decide(reports=[failed("buy", "invalid_args")])
        self.assertFalse(self.brain.catalogue.has("buy"))
        self.assertNotEqual(self.decide(cash=5000).state, State.SHOP)

    def test_a_failure_of_a_routine_does_not_pause_the_verb(self) -> None:
        self.decide(reports=[failed("sleep", "conflict", request_id=None)])
        self.assertEqual(self.brain.pause_until, {})

    def test_a_farm_with_nothing_left_sends_the_agent_to_the_next_farm(self) -> None:
        second = {"id": 41, "kind": "farm", "pos": {"x": 60, "y": 60}}
        sites = KNOWN_SITES + [second]
        first = self.brain.decide(observation(food=500), sites)
        self.assertEqual(first.steps[0], walk(FARM))
        self.brain.decide(observation(food=500, reports=[failed("gather", "insufficient")]), sites)
        later = self.brain.decide(observation(food=500, tick=2000), sites)
        self.assertEqual(later.steps[0], {"verb": "move_to", "args": {"site": 41}})

    def test_the_acknowledgements_of_a_short_plan_are_read_as_far_as_they_go(self) -> None:
        steps = [{"verb": "move_to"}, {"verb": "open_account"}, {"verb": "deposit"}]
        error = {"code": "not_here", "message": "You are not at a bank."}
        acks = [
            {"action_id": 1, "verb": "move_to", "status": {"state": "started", "ends_at": 9, "ticks": 9}},
            {"action_id": 2, "verb": "open_account", "status": {"state": "rejected", "error": error}},
        ]
        self.brain.learn(steps, acks)
        self.assertIn("open_account", self.brain.pause_until)
        self.assertNotIn("deposit", self.brain.pause_until)


class TalkTest(unittest.TestCase):
    def setUp(self) -> None:
        self.brain = Brain(Catalogue(Town().world.catalogue()), AgentConfig(talk_every_ticks=240))
        self.bo = [{"id": 2, "name": "Bo", "pos": {"x": 21, "y": 20}}]

    def test_the_agent_greets_a_resident_in_sight_but_not_too_often(self) -> None:
        alone = self.brain.decide(observation(items={"bread": 2}), KNOWN_SITES)
        self.assertIsNone(alone.say)
        first = self.brain.decide(observation(items={"bread": 2}, neighbours=self.bo), KNOWN_SITES)
        self.assertIsInstance(first.say, str)
        self.assertNotIn("http", first.say)
        soon = self.brain.decide(observation(items={"bread": 2}, neighbours=self.bo, tick=800), KNOWN_SITES)
        self.assertIsNone(soon.say)
        later = self.brain.decide(observation(items={"bread": 2}, neighbours=self.bo, tick=1000), KNOWN_SITES)
        self.assertIsInstance(later.say, str)
        self.assertNotEqual(later.say, first.say)

    def test_residents_of_one_command_do_not_talk_at_the_same_moment(self) -> None:
        catalogue = Catalogue(Town().world.catalogue())
        first_words = []
        for variety in range(4):
            brain = Brain(catalogue, AgentConfig(talk_every_ticks=240), variety)
            tick = next(
                tick
                for tick in range(720, 1000)
                if brain.decide(observation(items={"bread": 2}, neighbours=self.bo, tick=tick), KNOWN_SITES).say
            )
            first_words.append(tick)
        self.assertEqual(len(set(first_words)), 4, first_words)

    def test_after_a_refusal_of_its_words_the_agent_is_silent(self) -> None:
        self.brain.refused_words()
        decision = self.brain.decide(observation(items={"bread": 2}, neighbours=self.bo), KNOWN_SITES)
        self.assertIsNone(decision.say)


class MissingVerbsTest(unittest.TestCase):
    def test_with_only_the_kernel_verbs_every_observation_gives_idle(self) -> None:
        brain = Brain(Catalogue(FakeWorld(verbs=kernel_verbs()).catalogue()), AgentConfig())
        for fields in ({}, {"food": 10}, {"energy": 10}, {"cash": 90000}, {"hour": 3}):
            decision = brain.decide(observation(**fields), KNOWN_SITES)
            self.assertEqual((decision.state, decision.steps), (State.IDLE, []), fields)


if __name__ == "__main__":
    unittest.main()
