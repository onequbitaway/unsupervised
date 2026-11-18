"""A small town for the tests of the agent.

It adds to the fake world of the SDK what the scripted agent lives on:
needs that fall with time, a shelter with beds, a shop that sells food and
one public job. The verbs and their arguments follow the names of the game
design document; the real modules can differ in detail.
"""

import math
from typing import Any, Dict, List, Optional, Sequence

import _path  # noqa: F401

from unsupervised.testing import Busy, Done, FakeActionError, FakeResident, FakeVerb, FakeWorld
from unsupervised.testing import default_verbs, verb_info

FOOD = "need.food"
ENERGY = "need.energy"
HEALTH = "need.health"
NEED_MAX = 1000

#: Ticks in which `need.food` falls by one point.
FOOD_FALL_TICKS = 3
#: Ticks in which `need.energy` falls by one point while the resident is awake.
ENERGY_FALL_TICKS = 2
#: Points of `need.energy` that one tick of sleep gives.
SLEEP_GAIN = 3
#: Ticks that one meal takes.
EAT_TICKS = 5
#: Ticks of one shift, and the wage for it in cents.
SHIFT_TICKS = 120
WAGE = 600
#: Ticks that an application takes.
APPLY_TICKS = 5
TICKS_PER_DAY = 1440

ATTRS = [
    {"name": ENERGY, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means exhausted."},
    {"name": FOOD, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means starving."},
    {"name": HEALTH, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means death."},
]
ITEMS = [
    {"id": 0, "name": "apple", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 120}, "summary": "An apple."},
    {"id": 1, "name": "bread", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 300}, "summary": "A loaf of bread."},
    {"id": 2, "name": "coal", "weight": 5, "tags": ["fuel"], "attrs": {}, "summary": "A lump of coal."},
]
PRICES = {"apple": 150, "bread": 250}

SHELTER = {"id": 10, "kind": "shelter", "name": "Public Shelter", "rect": {"x0": 4, "y0": 4, "x1": 10, "y1": 10}, "tags": ["public", "spawn"]}
SHOP = {"id": 20, "kind": "shop", "name": "Corner Shop", "rect": {"x0": 40, "y0": 4, "x1": 46, "y1": 10}, "tags": []}
WORKS = {"id": 30, "kind": "office", "name": "Public Works", "rect": {"x0": 40, "y0": 40, "x1": 48, "y1": 48}, "tags": ["public"]}
SITES = [SHELTER, SHOP, WORKS]
#: The centre of the shelter. A new resident appears there.
SPAWN = {"x": 7, "y": 7}

JOBS = [
    {"id": 1, "site": WORKS["id"], "title": "Street cleaner", "wage": WAGE},
    {"id": 2, "site": SHOP["id"], "title": "Shelf filler", "wage": WAGE - 200},
]
CONDITIONS = [
    {"name": "frozen", "blocking": True, "summary": "An administrator stopped the resident."},
    {"name": "jailed", "blocking": True, "summary": "The resident serves a sentence."},
]


class Town:
    """A fake world with the rules of the town, and counters for assertions."""

    def __init__(self, *, time: str = "auto", verbs: Optional[Sequence[str]] = None) -> None:
        """Makes the town. `verbs` limits the town verbs that exist; the default is all of them."""
        self.world = FakeWorld(attrs=ATTRS, items=ITEMS, sites=SITES, conditions=CONDITIONS, spawn=SPAWN, time=time)
        #: The lowest value that each need reached, by resident id.
        self.lowest: Dict[int, Dict[str, int]] = {}
        #: Finished shifts and meals, by resident id.
        self.shifts: Dict[int, int] = {}
        self.meals: Dict[int, int] = {}
        self._town_verbs = self._verbs()
        for name in self._town_verbs if verbs is None else verbs:
            self.world.add_verb(self._town_verbs[name])
        self.world.on_tick.append(self._needs)

    def verb(self, name: str) -> FakeVerb:
        """Returns one town verb, for a test that adds it to the world later."""
        return self._town_verbs[name]

    def spawn(self, name: str = "Ada") -> FakeResident:
        """Adds a resident with a new owner."""
        return self.world.spawn(self.world.sign_up()["owner"], name)

    # ---- Rules ------------------------------------------------------------

    def _needs(self, world: FakeWorld) -> None:
        for resident in world.residents.values():
            attrs = resident.attrs
            asleep = resident.current is not None and resident.current.verb == "sleep"
            if world.tick % FOOD_FALL_TICKS == 0:
                attrs[FOOD] = max(0, attrs[FOOD] - 1)
            if asleep:
                attrs[ENERGY] = min(NEED_MAX, attrs[ENERGY] + SLEEP_GAIN)
            elif world.tick % ENERGY_FALL_TICKS == 0:
                attrs[ENERGY] = max(0, attrs[ENERGY] - 1)
            if attrs[FOOD] == 0 or attrs[ENERGY] == 0:
                attrs[HEALTH] = max(0, attrs[HEALTH] - 1)
            lowest = self.lowest.setdefault(resident.id, {FOOD: NEED_MAX, ENERGY: NEED_MAX, HEALTH: NEED_MAX})
            for name in lowest:
                lowest[name] = min(lowest[name], attrs[name])

    def _in(self, resident: FakeResident, site: Dict[str, Any], what: str) -> None:
        here = self.world.site_at(resident.pos)
        if here is None or here["id"] != site["id"]:
            raise FakeActionError("not_here", "You must be in %s to %s." % (site["name"], what))

    # ---- Verbs ------------------------------------------------------------

    def _verbs(self) -> Dict[str, FakeVerb]:
        def eat(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            item = next((one for one in ITEMS if one["name"] == args["item"]), None)
            if item is None or "food" not in item["tags"]:
                raise FakeActionError("invalid_args", "`%s` is not food." % args["item"])
            if resident.items.get(item["name"], 0) < 1:
                raise FakeActionError("insufficient", "You have no %s." % item["name"])

            def swallow() -> Dict[str, Any]:
                resident.items[item["name"]] -= 1
                resident.attrs[FOOD] = min(NEED_MAX, resident.attrs[FOOD] + item["attrs"]["nutrition"])
                self.meals[resident.id] = self.meals.get(resident.id, 0) + 1
                return {"item": item["name"], "food": resident.attrs[FOOD]}

            return Busy(EAT_TICKS, swallow)

        def sleep(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            self._in(resident, SHELTER, "sleep")
            ticks = max(1, math.ceil((NEED_MAX - resident.attrs[ENERGY]) / SLEEP_GAIN))
            return Busy(ticks, lambda: {"slept": ticks})

        def buy(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            self._in(resident, SHOP, "buy")
            price = PRICES.get(args["item"])
            if price is None:
                raise FakeActionError("not_found", "The shop does not sell `%s`." % args["item"])
            total = price * args["qty"]
            if args["qty"] < 1:
                raise FakeActionError("invalid_args", "`qty` must be at least 1.")
            if resident.cash < total:
                raise FakeActionError("insufficient", "You need %d cents." % total, {"missing": total - resident.cash})
            resident.cash -= total
            resident.items[args["item"]] = resident.items.get(args["item"], 0) + args["qty"]
            return Done({"item": args["item"], "qty": args["qty"], "paid": total})

        def list_jobs(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            return Done({"jobs": [dict(job) for job in JOBS]})

        def apply_job(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            job = next((one for one in JOBS if one["id"] == args["job"]), None)
            if job is None:
                raise FakeActionError("not_found", "No job has the id %s." % args["job"])
            site = next(one for one in SITES if one["id"] == job["site"])
            self._in(resident, site, "apply")

            def hired() -> Dict[str, Any]:
                resident.sections["job"] = {"id": job["id"], "site": job["site"], "title": job["title"], "wage": job["wage"]}
                return {"job": job["id"]}

            return Busy(APPLY_TICKS, hired)

        def work(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            job = resident.sections.get("job")
            if job is None:
                raise FakeActionError("not_allowed", "You have no job.")
            site = next(one for one in SITES if one["id"] == job["site"])
            self._in(resident, site, "work")

            def paid() -> Dict[str, Any]:
                resident.cash += job["wage"]
                self.shifts[resident.id] = self.shifts.get(resident.id, 0) + 1
                return {"wage": job["wage"]}

            return Busy(SHIFT_TICKS, paid)

        item = {"type": "string", "description": "Name of the item."}
        return {
            "eat": FakeVerb(verb_info("eat", "life", "Eat one unit of a food item.", properties={"item": item}, required=["item"]), eat),
            "sleep": FakeVerb(verb_info("sleep", "life", "Sleep in a bed until rested."), sleep),
            "buy": FakeVerb(
                verb_info(
                    "buy",
                    "economy",
                    "Buy goods in the shop where you are.",
                    properties={"item": item, "qty": {"type": "integer", "description": "How many units."}},
                    required=["item", "qty"],
                ),
                buy,
            ),
            "list_jobs": FakeVerb(verb_info("list_jobs", "economy", "List open jobs.", free=True), list_jobs),
            "apply_job": FakeVerb(
                verb_info(
                    "apply_job",
                    "economy",
                    "Apply for a job at its site.",
                    properties={"job": {"type": "integer", "description": "Id of the job."}},
                    required=["job"],
                ),
                apply_job,
            ),
            "work": FakeVerb(verb_info("work", "economy", "Work one shift at your job."), work),
        }


def kernel_verbs() -> List[FakeVerb]:
    """Returns the verbs that exist today: the kernel verbs and `move_to`."""
    return default_verbs()
