"""A small town for the tests of the agent.

It adds to the fake world of the SDK what the scripted agent lives on in
the real game: needs that fall each game hour, a shelter, a city hall and a
depot with starter shifts, the depot prices of the catalogue, a farm with
raw food and a bank. The verbs, their arguments, their results and their
error codes are those of the real game (`docs/design/modules`). The
end-to-end test in `crates/e2e` runs the agent against the real game.
"""

from typing import Any, Dict, List, Optional, Sequence

import _path  # noqa: F401

from unsupervised.testing import Busy, Done, FakeActionError, FakeResident, FakeVerb, FakeWorld, Pending
from unsupervised.testing import default_verbs, verb_info

FOOD = "need.food"
ENERGY = "need.energy"
HEALTH = "need.health"
NEED_MAX = 1000
TICKS_PER_DAY = 1440

#: What the needs lose in one game hour, as in the life module.
FOOD_PER_HOUR = 14
ENERGY_PER_HOUR = 40
STARVE_HEALTH_PER_HOUR = 20
#: Energy that one hour of sleep gives in a shelter, and in another place.
SLEEP_FAIR_PER_HOUR = 90
SLEEP_POOR_PER_HOUR = 60
SLEEP_MAX_TICKS = 720
EAT_TICKS = 10
#: One starter shift: its length, its wage before tax and the income tax in basis points.
SHIFT_MINUTES = 240
WAGE = 2000
INCOME_TAX_BP = 1000
#: Energy that one hour of work costs.
WORK_ENERGY_PER_HOUR = 20
#: Units of raw food that one hour of `gather` gives.
GATHER_PER_HOUR = 5
ACCOUNT_FEE = 100

ATTRS = [
    {"name": ENERGY, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means exhausted."},
    {"name": FOOD, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means starving."},
    {"name": HEALTH, "min": 0, "max": NEED_MAX, "default": NEED_MAX, "summary": "0 means death."},
]
ITEMS = [
    {"id": 0, "name": "grain", "weight": 1, "tags": ["raw"], "attrs": {"base_price": 200}, "summary": "Grain."},
    {"id": 1, "name": "vegetables", "weight": 1, "tags": ["raw", "food"], "attrs": {"nutrition": 120}, "summary": "Vegetables."},
    {"id": 2, "name": "bread", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 300}, "summary": "A loaf of bread."},
    {"id": 3, "name": "meal", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 600}, "summary": "A cooked meal."},
]  # fmt: skip
#: What the depot takes for one unit, and what it pays.
CEILING = {"grain": 500, "vegetables": 750, "bread": 2000, "meal": 4500}
FLOOR = {"grain": 80, "vegetables": 120, "bread": 320, "meal": 720}


def _site(id: int, kind: str, name: str, x0: int, y0: int, tags: Sequence[str] = ()) -> Dict[str, Any]:
    return {"id": id, "kind": kind, "name": name, "rect": {"x0": x0, "y0": y0, "x1": x0 + 6, "y1": y0 + 6}, "tags": list(tags)}


SHELTER = _site(10, "shelter", "Shelter 1", 4, 4, ["public", "spawn"])
CITY_HALL = _site(20, "city_hall", "City Hall", 20, 4, ["public"])
DEPOT = _site(30, "depot", "Depot", 40, 4, ["public"])
FARM = _site(40, "farm", "Farm 1", 40, 40)
BANK = _site(50, "bank", "Bank 1", 4, 40, ["public"])
SHOP = _site(60, "shop", "Shop 1", 20, 40)
SITES = [SHELTER, CITY_HALL, DEPOT, FARM, BANK, SHOP]
#: The centre of the shelter. A new resident appears there.
SPAWN = {"x": 7, "y": 7}

CONDITIONS = [
    {"name": "frozen", "blocking": True, "summary": "An administrator stopped the resident."},
    {"name": "jailed", "blocking": True, "summary": "The resident serves a sentence."},
    {"name": "asleep", "blocking": False, "summary": "The resident sleeps."},
]
#: The verbs of the town, without the kernel verbs and `move_to`.
TOWN_VERBS = ("eat", "sleep", "buy", "sell", "list_jobs", "apply_job", "work", "gather", "open_account", "deposit", "say")


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
        #: The quality of each finished sleep, by resident id.
        self.sleeps: Dict[int, List[str]] = {}
        #: Money in the bank, by resident id.
        self.deposits: Dict[int, int] = {}
        #: Words that residents said aloud.
        self.said: List[str] = []
        #: The jobs of employers. A test adds one: `{"job", "site", "wage", "min_level", "free_slots"}`.
        self.jobs: List[Dict[str, Any]] = []
        #: True while the city has budget for starter shifts.
        self.budget_open = True
        self._town_verbs = self._verbs()
        for name in TOWN_VERBS if verbs is None else verbs:
            self.world.add_verb(self._town_verbs[name])
        self._params()
        self.world.on_tick.append(self._needs)

    def verb(self, name: str) -> FakeVerb:
        """Returns one town verb, for a test that adds it to the world later."""
        return self._town_verbs[name]

    def spawn(self, name: str = "Ada") -> FakeResident:
        """Adds a resident with a new owner. Its home is the shelter, as for a newcomer of the real game."""
        resident = self.world.spawn(self.world.sign_up()["owner"]["id"], name)
        resident.sections["home"] = {"site": SHELTER["id"], "kind": "shelter", "zone": 0, "pos": dict(SPAWN), "sleep": "fair"}
        return resident

    def _params(self) -> None:
        """Puts the parameters of the game into the catalogue: the depot prices and the fee of the bank."""
        catalogue = self.world.catalogue
        extra = [("economy.depot_ceiling.%s" % item, price) for item, price in CEILING.items()]
        extra += [("economy.depot_floor.%s" % item, price) for item, price in FLOOR.items()]
        extra.append(("finance.account_open_fee", ACCOUNT_FEE))

        def with_params() -> Dict[str, Any]:
            out = catalogue()
            out["params"] = out["params"] + [
                {"name": name, "value": value, "policy": True, "summary": "A parameter of the town."} for name, value in extra
            ]
            return out

        self.world.catalogue = with_params  # type: ignore[method-assign]

    # ---- Rules ------------------------------------------------------------

    def _needs(self, world: FakeWorld) -> None:
        """Changes the needs one time in each game hour, as the life module does."""
        for resident in world.residents.values():
            attrs = resident.attrs
            if world.tick % 60 == 0:
                asleep = "asleep" in resident.conditions
                attrs[FOOD] = max(0, attrs[FOOD] - FOOD_PER_HOUR)
                if not asleep:
                    attrs[ENERGY] = max(0, attrs[ENERGY] - ENERGY_PER_HOUR)
                if attrs[FOOD] == 0:
                    attrs[HEALTH] = max(0, attrs[HEALTH] - STARVE_HEALTH_PER_HOUR)
            lowest = self.lowest.setdefault(resident.id, {FOOD: NEED_MAX, ENERGY: NEED_MAX, HEALTH: NEED_MAX})
            for name in lowest:
                lowest[name] = min(lowest[name], attrs[name])

    def _at(self, resident: FakeResident) -> Optional[Dict[str, Any]]:
        return self.world.site_at(resident.pos)

    def _in(self, resident: FakeResident, kinds: Sequence[str], message: str) -> Dict[str, Any]:
        here = self._at(resident)
        if here is None or here["kind"] not in kinds:
            raise FakeActionError("not_here", message)
        return here

    # ---- Verbs ------------------------------------------------------------

    def _verbs(self) -> Dict[str, FakeVerb]:
        def food_item(name: str) -> Dict[str, Any]:
            item = next((one for one in ITEMS if one["name"] == name), None)
            if item is None:
                raise FakeActionError("not_found", "No item is named `%s`." % name)
            if "food" not in item["tags"]:
                raise FakeActionError("not_allowed", "`%s` is not food." % name)
            return item

        def eat(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            if resident.attrs[FOOD] >= NEED_MAX:
                raise FakeActionError("conflict", "You are not hungry.")
            if args.get("item") is not None:
                item = food_item(args["item"])
            else:
                # The carried food with the most nutrition that is not wasted, else the smallest.
                carried = [one for one in ITEMS if "food" in one["tags"] and resident.items.get(one["name"], 0) > 0]
                room = NEED_MAX - resident.attrs[FOOD]
                fits = [one for one in carried if one["attrs"]["nutrition"] <= room]
                item = (
                    max(fits, key=lambda one: one["attrs"]["nutrition"])
                    if fits
                    else min(carried, key=lambda one: one["attrs"]["nutrition"], default=None)
                )
            if item is None or resident.items.get(item["name"], 0) < 1:
                raise FakeActionError("insufficient", "You carry no such food.")

            def swallow() -> Dict[str, Any]:
                resident.items[item["name"]] -= 1
                if resident.items[item["name"]] == 0:
                    del resident.items[item["name"]]
                resident.attrs[FOOD] = min(NEED_MAX, resident.attrs[FOOD] + item["attrs"]["nutrition"])
                self.meals[resident.id] = self.meals.get(resident.id, 0) + 1
                return {"item": item["name"], "nutrition": item["attrs"]["nutrition"], "food": resident.attrs[FOOD]}

            return Busy(EAT_TICKS, swallow)

        def sleep(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            if resident.attrs[ENERGY] >= NEED_MAX:
                raise FakeActionError("conflict", "Your energy is full.")
            here = self._at(resident)
            fair = here is not None and here["kind"] == "shelter"
            rate = SLEEP_FAIR_PER_HOUR if fair else SLEEP_POOR_PER_HOUR
            missing = NEED_MAX - resident.attrs[ENERGY]
            ticks = args.get("ticks") or min(SLEEP_MAX_TICKS, max(30, -(-missing * 60 // rate)))
            resident.conditions["asleep"] = world.tick + ticks

            def wake() -> Dict[str, Any]:
                resident.conditions.pop("asleep", None)
                resident.attrs[ENERGY] = min(NEED_MAX, resident.attrs[ENERGY] + rate * ticks // 60)
                quality = "fair" if fair else "poor"
                self.sleeps.setdefault(resident.id, []).append(quality)
                return {"ticks": ticks, "quality": quality, "energy": resident.attrs[ENERGY], "mood": 0}

            return Busy(ticks, wake)

        def buy(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            here = self._in(resident, ("depot",), "You are not at a shop, a restaurant or the depot.")
            price = CEILING.get(args["item"])
            if price is None:
                raise FakeActionError("not_found", "The depot does not trade `%s`." % args["item"])
            if args["qty"] < 1:
                raise FakeActionError("invalid_args", "`qty` must be at least 1.")
            if args.get("max_price") is not None and price > args["max_price"]:
                raise FakeActionError("conflict", "The price is %d cents." % price)
            total = price * args["qty"]
            if resident.cash < total:
                raise FakeActionError("insufficient", "You need %d cents." % total)

            def take() -> Dict[str, Any]:
                resident.cash -= total
                resident.items[args["item"]] = resident.items.get(args["item"], 0) + args["qty"]
                return {"item": args["item"], "qty": args["qty"], "site": here["id"], "tax": 0, "total": total, "unit_price": price}

            return Busy(1, take)

        def sell(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            here = self._in(resident, ("depot",), "You are not at a shop, a restaurant or the depot.")
            price = FLOOR.get(args["item"])
            if price is None:
                raise FakeActionError("not_found", "The depot does not trade `%s`." % args["item"])
            if not 1 <= args["qty"] <= resident.items.get(args["item"], 0):
                raise FakeActionError("insufficient", "You do not carry %d units." % args["qty"])

            def give() -> Dict[str, Any]:
                resident.items[args["item"]] -= args["qty"]
                resident.cash += price * args["qty"]
                return {"item": args["item"], "qty": args["qty"], "site": here["id"], "total": price * args["qty"], "unit_price": price}

            return Busy(1, give)

        def list_jobs(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            # A free verb of the global realm: the answer comes one frame later, in a report.
            def answer() -> Dict[str, Any]:
                works = {"kinds": ["city_hall", "depot"], "minutes": SHIFT_MINUTES, "open": self.budget_open, "wage": WAGE}
                return {"city": 0, "jobs": [dict(job) for job in self.jobs], "more": 0, "public_works": works}

            return Pending(1, answer)

        def apply_job(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            job = next((one for one in self.jobs if one["job"] == args["job"]), None)
            if job is None:
                raise FakeActionError("not_found", "No job has the id %s." % args["job"])
            here = self._at(resident)
            if here is None or here["id"] != job["site"]:
                raise FakeActionError("not_here", "You are not at the site of the job.")

            def hired() -> Dict[str, Any]:
                resident.sections["job"] = {
                    "job": job["job"], "site": job["site"], "zone": 0, "kind": here["kind"],
                    "wage": job["wage"], "shift_minutes": SHIFT_MINUTES,
                }  # fmt: skip
                return {"job": job["job"], "site": job["site"], "wage": job["wage"]}

            return Busy(5, hired)

        def work(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            here = self._at(resident)
            job = resident.sections.get("job")
            employed = job is not None and here is not None and here["id"] == job["site"]
            if not employed:
                if here is None or here["kind"] not in ("city_hall", "depot"):
                    message = "You have no job at this site. Use `apply_job` here, or work at a city hall or a depot."
                    raise FakeActionError("not_allowed" if here is not None else "not_here", message)
                if not self.budget_open:
                    raise FakeActionError("insufficient", "The city has no budget for a starter shift today.")
            wage = job["wage"] if employed else WAGE

            def paid() -> Dict[str, Any]:
                tax = wage * INCOME_TAX_BP // 10000
                resident.cash += wage - tax
                resident.attrs[ENERGY] = max(0, resident.attrs[ENERGY] - WORK_ENERGY_PER_HOUR * SHIFT_MINUTES // 60)
                self.shifts[resident.id] = self.shifts.get(resident.id, 0) + 1
                result = {"net": wage - tax, "site": here["id"], "skill": "labor", "tax": tax, "wage": wage, "xp": 39}
                if not employed:
                    result["public_works"] = True
                return result

            return Busy(SHIFT_MINUTES, paid)

        def gather(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            self._in(resident, ("farm",), "You are not at a farm, a mine or a forest.")
            item = args.get("item") or "grain"
            if item not in ("grain", "vegetables"):
                raise FakeActionError("not_found", "This site does not give `%s`." % item)
            minutes = args.get("minutes") or 60

            def harvest() -> Dict[str, Any]:
                units = GATHER_PER_HOUR * minutes // 60
                resident.items[item] = resident.items.get(item, 0) + units
                resident.attrs[ENERGY] = max(0, resident.attrs[ENERGY] - WORK_ENERGY_PER_HOUR * minutes // 60)
                return {"item": item, "qty": units, "minutes": minutes}

            return Busy(minutes, harvest)

        def open_account(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            self._in(resident, ("bank",), "You are not at a bank.")
            if resident.cash < ACCOUNT_FEE:
                raise FakeActionError("insufficient", "The account costs %d cents." % ACCOUNT_FEE)

            def opened() -> Dict[str, Any]:
                if resident.id in self.deposits:
                    raise FakeActionError("conflict", "You have an account.")
                resident.cash -= ACCOUNT_FEE
                self.deposits[resident.id] = 0
                resident.sections["bank"] = {"balance": 0}
                return {"fee": ACCOUNT_FEE}

            return Pending(2, opened)

        def deposit(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            self._in(resident, ("bank",), "You are not at a bank.")
            amount = args["amount"]
            if amount <= 0:
                raise FakeActionError("invalid_args", "The amount must be greater than zero.")
            if resident.cash < amount:
                raise FakeActionError("insufficient", "You carry %d cents." % resident.cash)

            def booked() -> Dict[str, Any]:
                if resident.id not in self.deposits:
                    raise FakeActionError("not_found", "You have no account.")
                resident.cash -= amount
                self.deposits[resident.id] += amount
                resident.sections["bank"] = {"balance": self.deposits[resident.id]}
                return {"amount": amount, "balance": self.deposits[resident.id]}

            return Pending(2, booked)

        def say(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Any:
            heard = 0
            for other in world.residents.values():
                if other.id != resident.id and max(abs(other.pos["x"] - resident.pos["x"]), abs(other.pos["y"] - resident.pos["y"])) <= 8:
                    world.tell(other.id, resident.id, args["text"])
                    heard += 1
            self.said.append(args["text"])
            return Done({"heard_by": heard})

        text = {"type": "string"}
        whole = {"type": "integer"}
        maybe_text = {"type": ["string", "null"]}
        maybe_whole = {"type": ["integer", "null"]}
        trade = {"item": text, "qty": whole}
        return {
            "eat": FakeVerb(verb_info("eat", "life", "Eat one unit of food that you carry.", properties={"item": maybe_text}), eat),
            "sleep": FakeVerb(verb_info("sleep", "life", "Sleep to get energy back.", properties={"ticks": maybe_whole}), sleep),
            "buy": FakeVerb(
                verb_info("buy", "economy", "Buy goods at the depot.", properties=dict(trade, max_price=maybe_whole), required=["item", "qty"]),
                buy,
            ),
            "sell": FakeVerb(
                verb_info("sell", "economy", "Sell goods to the depot.", properties=dict(trade, min_price=maybe_whole), required=["item", "qty"]),
                sell,
            ),
            "list_jobs": FakeVerb(verb_info("list_jobs", "economy", "List the jobs of the city.", free=True, scope="global"), list_jobs),
            "apply_job": FakeVerb(
                verb_info("apply_job", "economy", "Take a job at the site.", properties={"job": whole}, required=["job"]), apply_job
            ),
            "work": FakeVerb(verb_info("work", "economy", "Work one shift."), work),
            "gather": FakeVerb(
                verb_info("gather", "economy", "Take raw goods from the node.", properties={"item": maybe_text, "minutes": maybe_whole}),
                gather,
            ),
            "open_account": FakeVerb(verb_info("open_account", "finance", "Open a bank account."), open_account),
            "deposit": FakeVerb(
                verb_info("deposit", "finance", "Put cash into your account.", properties={"amount": whole}, required=["amount"]), deposit
            ),
            "say": FakeVerb(
                verb_info("say", "society", "Say something aloud.", free=True, properties={"text": text}, required=["text"]), say
            ),
        }


def kernel_verbs() -> List[FakeVerb]:
    """Returns the verbs of every world: the kernel verbs and `move_to`."""
    return default_verbs()
