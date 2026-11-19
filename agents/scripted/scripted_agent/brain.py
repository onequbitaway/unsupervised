"""The state machine of the agent.

`Brain.decide` reads one observation and returns the state of the agent and
the actions to plan. It does no input or output, so tests call it directly.

The rules, in priority order:

1. A blocking condition (jail, hospital): do nothing.
2. An action runs or planned actions wait: do nothing until it ends. This
   is also the rule for an action that a routine started, for example a
   sleep: the agent does not wake the resident.
3. Tired: walk home and sleep.
4. Hungry with food: eat.
5. Low on food: buy the cheapest food at the depot, or gather it on a farm.
6. Too much cash in the pocket: put it in the bank.
7. Working hours: work a shift at the job, or a starter shift of the city;
   without both, gather food on a farm and sell it at the depot.
8. After the working day: walk home.
9. Otherwise: stay idle. The routines of the resident run when it is idle.

The agent knows the real game: starter shifts are in the answer of
`list_jobs`, the depot sells food at the prices of the catalogue, and a
walk takes the id of a site. A rule applies only when its verb is in the
catalogue. When the server refuses an action or the action fails, the agent
leaves the verb alone for a time and the next rule gets its turn.
"""

import enum
import logging
from typing import Any, Dict, List, NamedTuple, Optional, Sequence

from .catalogue import Catalogue
from .config import AgentConfig
from .senses import ENERGY, FOOD, Senses

LOG = logging.getLogger("scripted_agent")

#: Site kinds with a bed that everybody can use.
BED_KINDS = ("shelter",)
#: The site kind that sells every basic good at a fixed price.
DEPOT = "depot"
#: The site kind where raw food grows.
FARM = "farm"
#: The site kind that keeps deposits.
BANK = "bank"
#: Greatest number of times that the pause after a failure doubles.
PAUSE_DOUBLINGS_MAX = 4
#: Ticks of one game day.
DAY_TICKS = 1440
#: Ticks for which the agent waits for the report of a plan, when it cannot know better.
PLAN_TICKS = 720
#: What a resident says to a resident that it sees. The words are harmless and have no link.
PHRASES = (
    "Good day.",
    "How is the work today?",
    "The depot has food for a fair price.",
    "The city hall has shifts for everybody.",
    "I go home before the night.",
    "The farms have vegetables.",
    "Did you eat today?",
    "I save for a home of my own.",
    "The shelter has a free bed.",
    "A shift pays well this year.",
    "See you at the depot.",
    "Who gets your vote for mayor?",
)

Step = Dict[str, Any]
Site = Dict[str, Any]


class State(enum.Enum):
    """What the agent does now."""

    BLOCKED = "blocked"
    BUSY = "busy"
    SLEEP = "sleep"
    EAT = "eat"
    SHOP = "shop"
    SAVE = "save"
    WORK = "work"
    GATHER = "gather"
    HOME = "home"
    JOB_HUNT = "job_hunt"
    IDLE = "idle"


class Decision(NamedTuple):
    """The result of one decision."""

    #: The state of the agent.
    state: State
    #: Actions to plan, in order. Can be empty.
    steps: List[Step]
    #: Game minutes to block for news after the plan. 0 means: decide again at once.
    wait_ticks: int
    #: One sentence for the log.
    reason: str
    #: Words to say aloud now, or `None`. The agent sends them apart from the plan.
    say: Optional[str] = None


def hour_between(hour: int, start: int, end: int) -> bool:
    """True when `hour` is from `start` (inclusive) to `end` (exclusive). The range can wrap past midnight."""
    if start < end:
        return start <= hour < end
    return start > end and (hour >= start or hour < end)


def distance(a: Dict[str, int], b: Dict[str, int]) -> int:
    """Distance in tiles. A diagonal step counts as one."""
    return max(abs(a["x"] - b["x"]), abs(a["y"] - b["y"]))


class Brain:
    """The state machine. One brain serves one resident."""

    def __init__(self, catalogue: Catalogue, config: Optional[AgentConfig] = None, variety: int = 0) -> None:
        """Makes a brain. `variety` is a small number that differs between the residents of one command.

        Residents with different numbers choose different places of the
        same worth (the city hall or the depot for a shift, one farm or
        the next) and talk at different times.
        """
        self.catalogue = catalogue
        self.config = config or AgentConfig()
        self.variety = variety
        #: The state of the last decision.
        self.state = State.IDLE
        #: Tick until which a verb is left alone, by verb.
        self.pause_until: Dict[str, int] = {}
        #: Failures in a row, by verb.
        self.failures: Dict[str, int] = {}
        #: The jobs of the last `list_jobs`, and the tick of that answer.
        self.jobs: List[Dict[str, Any]] = []
        self.jobs_tick: Optional[int] = None
        #: The starter shifts of the city, from `list_jobs`: `wage`, `minutes`, `kinds`, `open`.
        self.public_works: Optional[Dict[str, Any]] = None
        #: True when the resident has a bank account.
        self.has_account = False
        #: True when the server refused the words of the resident. It says nothing more.
        self.silent = False
        #: The farm that the agent uses, as a place in the list of farms by distance.
        #: A farm that had nothing to gather sends the agent to the next one.
        self.farm_choice = variety
        #: Starter shifts that the resident worked, by game day `(year, day)`.
        self.public_shifts: Dict[Any, int] = {}
        self._day: Any = (0, 0)
        self._talk_tick: Optional[int] = None
        self._words_said = 0
        self._tick = 0

    def use_catalogue(self, catalogue: Catalogue) -> None:
        """Takes a new catalogue. Verbs that the agent gave up on stay given up."""
        catalogue.unusable |= self.catalogue.unusable
        self.catalogue = catalogue

    # ---- Decide -----------------------------------------------------------

    def decide(self, observation: Dict[str, Any], sites: Sequence[Site] = ()) -> Decision:
        """Returns what to do for one observation.

        `sites` are the sites of the city: `{"id", "kind", "pos"}`, where
        `pos` is the middle of the site.
        """
        senses = Senses(observation, self.catalogue)
        self._tick = senses.tick
        self._day = (senses.clock.get("year", 0), senses.clock.get("day", 0))
        self._read_reports(senses)
        if senses.bank is not None:
            self.has_account = True
        decision = self._decide(senses, list(sites) or senses.nearby_sites)
        if decision.state is not self.state:
            LOG.info("state %s -> %s: %s", self.state.value, decision.state.value, decision.reason)
        self.state = decision.state
        words = self._words(senses)
        return decision._replace(say=words) if words is not None else decision

    def _decide(self, senses: Senses, sites: Sequence[Site]) -> Decision:
        blocked = senses.blocked_by
        if blocked is not None:
            return Decision(State.BLOCKED, [], self._idle_ticks(senses), "the resident is %s" % blocked)
        if senses.activity is not None or senses.queue > 0:
            verb = senses.activity["verb"] if senses.activity else "the plan"
            return Decision(State.BUSY, [], self._busy_ticks(senses), "the resident does `%s`" % verb)
        for rule in (self._sleep, self._eat, self._food, self._save, self._work, self._home):
            decision = rule(senses, sites)
            if decision is not None:
                return decision
        return Decision(State.IDLE, [], self._idle_ticks(senses), "nothing to do; the routines run")

    def _busy_ticks(self, senses: Senses) -> int:
        """Returns the game minutes to wait for a resident that is busy.

        The report of an action ends the wait of the agent. An action that
        a routine started writes no report when it goes well: the sleep of
        the reserve routine, for example. So the agent waits until the tick
        at which the running action ends, and no longer. With planned
        actions in the queue, or with an action that waits for an answer of
        the world, a report comes: the agent waits for it.
        """
        activity = senses.activity
        if senses.queue > 0 or activity is None or activity.get("waiting") or not isinstance(activity.get("ends"), int):
            return PLAN_TICKS
        return max(1, min(PLAN_TICKS, activity["ends"] - senses.tick))

    def _idle_ticks(self, senses: Senses) -> int:
        """Returns the game minutes to the next full hour: the needs and the clock change then."""
        minute = int(senses.clock.get("minute", 0))
        return max(1, 60 - minute)

    def _sleep(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        config = self.config
        energy = senses.need(ENERGY)
        if not self.catalogue.has("sleep") or energy is None or self._paused("sleep"):
            return None
        night = hour_between(senses.hour, config.night_from, config.night_to)
        # Too tired for any work: go home now, before the sleep routine fires in a bad place.
        spent = energy < self._energy_for(config.gather_minutes)
        if not (energy < config.sleep_below or spent or (night and energy < config.night_sleep_below)):
            return None
        # Without arguments the resident sleeps until its energy is full.
        steps = self._go(senses, self._home_site(senses, sites)) + [{"verb": "sleep", "args": {}}]
        return Decision(State.SLEEP, steps, PLAN_TICKS, "energy is %d" % energy)

    def _home_site(self, senses: Senses, sites: Sequence[Site]) -> Optional[int]:
        """Returns the id of the site where the resident sleeps: its home, else the nearest shelter."""
        if senses.home and isinstance(senses.home.get("site"), int):
            return senses.home["site"]
        bed = self._nearest(senses, sites, BED_KINDS)
        return bed["id"] if bed else None

    def _home(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        """After the working day the resident walks home, so that the night starts in its bed."""
        if hour_between(senses.hour, self.config.work_from, self.config.work_to):
            return None
        # Without a verb to sleep, a home is a place like any other.
        if not self.catalogue.has("sleep") or not self.catalogue.has("move_to") or self._paused("move_to"):
            return None
        home = self._home_site(senses, sites)
        if home is None or (senses.site is not None and senses.site.get("id") == home):
            return None
        return Decision(State.HOME, self._go(senses, home), PLAN_TICKS, "the working day is over")

    def _eat(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        food = senses.need(FOOD)
        if food is None or food >= self.config.eat_below or not senses.food_on_hand():
            return None
        if not self.catalogue.has("eat") or self._paused("eat"):
            return None
        # Without an item the resident eats the food that fits its hunger best.
        return Decision(State.EAT, [{"verb": "eat", "args": {}}], PLAN_TICKS, "food is %d" % food)

    def _food(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        """Gets food when the resident carries too little: from the depot, or from a farm."""
        config = self.config
        if senses.need(FOOD) is None or senses.food_units() >= config.keep_food:
            return None
        bought = self._buy(senses, sites)
        if bought is not None:
            return bought
        # No money for food. A hungry resident does not wait for a wage: it gathers.
        if senses.need(FOOD) < config.eat_below:
            return self._gather(senses, sites, "the resident is hungry and has no money for food")
        return None

    def _buy(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        config = self.config
        depot = self._nearest(senses, sites, (DEPOT,))
        if depot is None or not self.catalogue.has("buy") or self._paused("buy"):
            return None
        # The food with the most nutrition for one cent, at the prices of the depot.
        offers = []
        for item in self.catalogue.food():
            price = self.catalogue.depot_price(item["name"])
            if price is not None and price <= senses.cash:
                offers.append((price / item["attrs"]["nutrition"], item["name"], price))
        if not offers:
            return None
        _, name, price = min(offers)
        qty = max(1, min(config.buy_qty, senses.cash // price))
        buy = {"verb": "buy", "args": {"item": name, "qty": qty, "max_price": price}}
        reason = "the resident carries %d food units" % senses.food_units()
        return Decision(State.SHOP, self._go(senses, depot["id"]) + [buy], PLAN_TICKS, reason)

    def _gather(self, senses: Senses, sites: Sequence[Site], reason: str) -> Optional[Decision]:
        """Gathers raw food on a farm of the city. It needs no money and no job."""
        crops = self.catalogue.raw_food()
        farms = [site for site in sites if site.get("kind") == FARM and isinstance(site.get("pos"), dict)]
        if not crops or not farms or not self.catalogue.has("gather") or self._paused("gather"):
            return None
        energy = senses.need(ENERGY)
        if energy is not None and energy < max(self.config.sleep_below, self._energy_for(self.config.gather_minutes)):
            return None
        if senses.pos is not None:
            farms.sort(key=lambda site: (distance(site["pos"], senses.pos), site["id"]))
        farm = farms[self.farm_choice % len(farms)]
        crop = max(crops, key=lambda item: item["attrs"]["nutrition"])["name"]
        gather = {"verb": "gather", "args": {"item": crop, "minutes": self.config.gather_minutes}}
        return Decision(State.GATHER, self._go(senses, farm["id"]) + [gather], PLAN_TICKS, reason)

    def _save(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        config = self.config
        bank = self._nearest(senses, sites, (BANK,))
        if bank is None or senses.cash <= config.save_above or not self.catalogue.has("deposit"):
            return None
        if self._paused("deposit") or self._paused("open_account"):
            return None
        steps = self._go(senses, bank["id"])
        if not self.has_account:
            if not self.catalogue.has("open_account"):
                return None
            steps.append({"verb": "open_account", "args": {}})
        fee = self.catalogue.params.get("finance.account_open_fee", 0) if not self.has_account else 0
        amount = senses.cash - config.keep_cash - fee
        if amount <= 0:
            return None
        steps.append({"verb": "deposit", "args": {"amount": amount}})
        return Decision(State.SAVE, steps, PLAN_TICKS, "the resident carries %d cents" % senses.cash)

    def _energy_for(self, minutes: int) -> int:
        """Returns the energy that the resident needs before work of this length.

        After the work the energy must still be above the mark of the sleep
        routine, with room for the walk home. Else the routine fires at the
        workplace, and the resident sleeps badly there. The costs of one
        hour come from the catalogue.
        """
        params = self.catalogue.params
        hourly = params.get("life.energy_decay_per_hour", 40) + params.get("economy.energy_per_hour", 20)
        walk_home = 2 * params.get("life.energy_decay_per_hour", 40)
        needed = self.config.routine_sleep_below + walk_home + hourly * minutes // 60
        return max(self.config.work_min_energy, needed)

    def _work(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        config = self.config
        energy = senses.need(ENERGY)
        rested = energy is None or energy >= config.rested_above
        if not hour_between(senses.hour, config.work_from, config.work_to) and not rested:
            return None
        shift = (self.public_works or {}).get("minutes") or self.catalogue.params.get("economy.shift_minutes", 240)
        if energy is not None and energy < self._energy_for(shift):
            # Too tired for a shift. A short time on a farm can still fit.
            if energy >= self._energy_for(config.gather_minutes):
                return self._gather(senses, sites, "too tired for a shift; one hour on a farm fits")
            return None
        # Sell what the resident gathered and does not need.
        sale = self._sell(senses, sites)
        if sale is not None:
            return sale
        if self.catalogue.has("work") and not self._paused("work"):
            if senses.job is not None and isinstance(senses.job.get("site"), int):
                steps = self._go(senses, senses.job["site"]) + [{"verb": "work", "args": {}}]
                return Decision(State.WORK, steps, PLAN_TICKS, "a shift at the job")
            fresh = self.jobs_tick is not None and senses.tick - self.jobs_tick < config.jobs_refresh_ticks
            if not fresh and self.catalogue.has("list_jobs") and not self._paused("list_jobs"):
                # The answer of `list_jobs` is a report: it comes one frame later.
                return Decision(State.JOB_HUNT, [{"verb": "list_jobs", "args": {}}], PLAN_TICKS, "read the list of jobs")
            hired = self._apply(senses, sites)
            if hired is not None:
                return hired
            works = self.public_works
            day = (senses.clock.get("year", 0), senses.clock.get("day", 0))
            left = config.public_shifts_per_day - self.public_shifts.get(day, 0)
            if works and works.get("open") and works.get("wage", 0) > 0 and left > 0:
                place = self._shift_site(senses, sites, works.get("kinds") or ())
                if place is not None:
                    steps = self._go(senses, place["id"]) + [{"verb": "work", "args": {}}]
                    return Decision(State.WORK, steps, PLAN_TICKS, "a starter shift of the city")
        return self._gather(senses, sites, "no paid work now; the farms have food to sell")

    def _shift_site(self, senses: Senses, sites: Sequence[Site], kinds: Sequence[str]) -> Optional[Site]:
        """Returns the site for a starter shift: the nearest site of each kind, chosen by `variety`.

        A resident that stands in such a site stays there.
        """
        if senses.site is not None and senses.site.get("kind") in kinds:
            here = [site for site in sites if site.get("id") == senses.site.get("id")]
            if here:
                return here[0]
        choices = [site for site in (self._nearest(senses, sites, (kind,)) for kind in kinds) if site is not None]
        if not choices:
            return None
        choices.sort(key=lambda site: site.get("id", 0))
        return choices[self.variety % len(choices)]

    def _apply(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        """Takes the best job of the list that a newcomer can do and that pays more than a starter shift."""
        if not self.catalogue.has("apply_job") or self._paused("apply_job"):
            return None
        floor = (self.public_works or {}).get("wage", 0)
        open_jobs = [
            job
            for job in self.jobs
            if job.get("min_level", 0) == 0 and job.get("free_slots", 1) > 0 and job.get("wage", 0) > floor
        ]
        if not open_jobs:
            return None
        job = max(open_jobs, key=lambda one: (one["wage"], -one["job"]))
        steps = self._go(senses, job["site"]) + [{"verb": "apply_job", "args": {"job": job["job"]}}]
        return Decision(State.JOB_HUNT, steps, PLAN_TICKS, "apply for the job %s" % job["job"])

    def _sell(self, senses: Senses, sites: Sequence[Site]) -> Optional[Decision]:
        config = self.config
        depot = self._nearest(senses, sites, (DEPOT,))
        if depot is None or not self.catalogue.has("sell") or self._paused("sell"):
            return None
        for item in self.catalogue.raw_food():
            spare = senses.items.get(item["name"], 0) - config.sell_above
            if spare > 0:
                sell = {"verb": "sell", "args": {"item": item["name"], "qty": spare + config.sell_above // 2}}
                reason = "the resident carries %d %s" % (senses.items[item["name"]], item["name"])
                return Decision(State.SHOP, self._go(senses, depot["id"]) + [sell], PLAN_TICKS, reason)
        return None

    def _words(self, senses: Senses) -> Optional[str]:
        """Returns words for a resident in sight, at most one time in `talk_every_ticks`."""
        every = self.config.talk_every_ticks
        if every <= 0 or self.silent or not senses.nearby_residents or not self.catalogue.has("say"):
            return None
        if "asleep" in senses.conditions or self._paused("say"):
            return None
        if self._talk_tick is None:
            # The first words come after a time that differs between residents.
            self._talk_tick = senses.tick - every + (self.variety * 37) % every
        if senses.tick - self._talk_tick < every:
            return None
        self._talk_tick = senses.tick
        self._words_said += 1
        return PHRASES[(self.variety * 5 + self._words_said) % len(PHRASES)]

    def refused_words(self) -> None:
        """Notes that the server refused the words of the resident. It says nothing more."""
        self.silent = True

    # ---- Places -----------------------------------------------------------

    def _nearest(
        self, senses: Senses, sites: Sequence[Site], kinds: Sequence[str], any_kind: bool = False
    ) -> Optional[Site]:
        """Returns the nearest site of the first kind that the city has.

        With `any_kind`, every kind of the list is as good as the others.
        """
        groups = [tuple(kinds)] if any_kind else [(kind,) for kind in kinds]
        for group in groups:
            matches = [site for site in sites if site.get("kind") in group and isinstance(site.get("pos"), dict)]
            if matches:
                if senses.pos is None:
                    return matches[0]
                return min(matches, key=lambda site: (distance(site["pos"], senses.pos), site.get("id", 0)))
        return None

    def _go(self, senses: Senses, site: Optional[int]) -> List[Step]:
        """Returns the step that walks to a site. Empty when the resident is there, or when no site is known."""
        if site is None or not self.catalogue.has("move_to"):
            return []
        if senses.site is not None and senses.site.get("id") == site:
            return []
        return [{"verb": "move_to", "args": {"site": site}}]

    # ---- Learn ------------------------------------------------------------

    def learn(self, steps: Sequence[Step], acks: Sequence[Dict[str, Any]]) -> None:
        """Reads the acknowledgements of a plan that the agent sent.

        The server stops a plan after the first refusal, so `acks` can be
        shorter than `steps`.
        """
        for step, ack in zip(steps, acks):
            status = ack["status"]
            if status["state"] == "rejected":
                self._failed(step["verb"], status["error"])
            elif status["state"] == "done":
                self._succeeded(step["verb"], status.get("result"))

    def _read_reports(self, senses: Senses) -> None:
        for report in senses.reports:
            if report.get("ok"):
                self._succeeded(report["verb"], report.get("result"))
            else:
                # The SDK gives every request of the agent an id. A report without one is from a routine.
                self._failed(report["verb"], report.get("error") or {}, own=report.get("request_id") is not None)

    def _succeeded(self, verb: str, result: Any) -> None:
        self.failures.pop(verb, None)
        self.pause_until.pop(verb, None)
        if verb == "list_jobs" and isinstance(result, dict):
            jobs = result.get("jobs")
            self.jobs = [job for job in jobs if isinstance(job, dict)] if isinstance(jobs, list) else []
            works = result.get("public_works")
            self.public_works = works if isinstance(works, dict) else None
            self.jobs_tick = self._tick
        elif verb == "open_account":
            self.has_account = True
        elif verb == "work" and isinstance(result, dict) and result.get("public_works"):
            # Only the count of today is kept.
            self.public_shifts = {self._day: self.public_shifts.get(self._day, 0) + 1}

    def _failed(self, verb: str, error: Dict[str, Any], own: bool = True) -> None:
        """Reads a refusal or a failure.

        `own` is false for an action that a routine started. Such a failure
        tells the agent what is wrong, but it does not pause the verb.
        """
        code = error.get("code", "")
        message = error.get("message", "")
        if code in ("unknown_verb", "invalid_args"):
            # The catalogue changed, or the agent built arguments that the verb does not take.
            self.catalogue.give_up(verb, "%s: %s" % (code, message))
            return
        if verb == "work" and code in ("insufficient", "interrupted"):
            # The budget of the city for starter shifts is spent, or the employer cannot pay.
            if self.public_works is not None:
                self.public_works = dict(self.public_works, open=False)
            self.jobs_tick = None
        elif code == "interrupted":
            return
        if verb == "open_account" and code == "conflict":
            self.has_account = True
            return
        if verb == "deposit" and code == "not_found":
            self.has_account = False
        if verb == "gather" and code in ("insufficient", "not_allowed"):
            self.farm_choice += 1
        if verb == "apply_job":
            self.jobs_tick = None
        if not own:
            return
        failures = self.failures.get(verb, 0) + 1
        self.failures[verb] = failures
        pause = self.config.retry_ticks * 2 ** min(failures - 1, PAUSE_DOUBLINGS_MAX)
        self.pause_until[verb] = self._tick + pause
        LOG.info("`%s` failed (%s: %s); the agent leaves it alone for %d ticks", verb, code, message, pause)

    def _paused(self, verb: str) -> bool:
        return self.pause_until.get(verb, 0) > self._tick
