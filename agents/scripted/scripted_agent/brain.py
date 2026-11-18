"""The state machine of the agent.

`Brain.decide` reads one observation and returns the state of the agent and
the actions to plan. It does no input or output, so tests call it directly.

The rules, in priority order:

1. A blocking condition (jail, hospital): do nothing.
2. Planned actions wait in the queue: do nothing.
3. Tired: go to a bed and sleep.
4. Hungry with food: eat.
5. Hungry or low on food, with cash: go to a shop and buy food.
6. Working hours, with a job: go to the job and work.
7. No job: read the list of jobs, go to the best one and apply.
8. Otherwise: stay idle. The routines of the resident run when it is idle.

A rule applies only when its verb is in the catalogue. When the server
refuses an action or the action fails, the agent leaves the verb alone for
a time and the next rule gets its turn.
"""

import enum
import logging
from typing import Any, Dict, List, NamedTuple, Optional, Sequence, Set

from .catalogue import Catalogue
from .config import AgentConfig
from .senses import ENERGY, FOOD, Senses

LOG = logging.getLogger("scripted_agent")

#: Site kinds with a bed that everybody can use, then kinds that need a lease.
BED_KINDS = ("shelter", "home", "apartment")
#: Site kinds that sell food.
SHOP_KINDS = ("shop", "restaurant", "depot")
#: Result keys that can hold the list of jobs.
JOB_LIST_KEYS = ("jobs", "offers", "openings", "postings")
#: Greatest number of times that the pause after a failure doubles.
PAUSE_DOUBLINGS_MAX = 4
#: Error codes of `work` that mean: the resident has no job.
NO_JOB_CODES = ("not_allowed", "not_found", "conflict")
#: Error codes of `buy` that mean: try another item.
OTHER_ITEM_CODES = ("insufficient", "not_found")

Step = Dict[str, Any]
Site = Dict[str, Any]


class State(enum.Enum):
    """What the agent does now."""

    BLOCKED = "blocked"
    BUSY = "busy"
    SLEEP = "sleep"
    EAT = "eat"
    SHOP = "shop"
    WORK = "work"
    JOB_HUNT = "job_hunt"
    IDLE = "idle"


class Decision(NamedTuple):
    """The result of one decision."""

    #: The state of the agent.
    state: State
    #: Actions to plan, in order. Can be empty.
    steps: List[Step]
    #: Seconds to block for news after the plan. 0 means: decide again at once.
    wait_seconds: int
    #: One sentence for the log.
    reason: str


def hour_between(hour: int, start: int, end: int) -> bool:
    """True when `hour` is from `start` (inclusive) to `end` (exclusive). The range can wrap past midnight."""
    if start < end:
        return start <= hour < end
    return start > end and (hour >= start or hour < end)


def distance(a: Dict[str, int], b: Dict[str, int]) -> int:
    """Distance in tiles. A diagonal step counts as one."""
    return max(abs(a["x"] - b["x"]), abs(a["y"] - b["y"]))


def read_jobs(result: Any) -> List[Dict[str, Any]]:
    """Finds the list of jobs in the result of `list_jobs`.

    The result can be the list itself, or an object that holds the list
    under one of `JOB_LIST_KEYS`, or under any other key.
    """
    if isinstance(result, dict):
        lists = [result[key] for key in JOB_LIST_KEYS if isinstance(result.get(key), list)]
        lists += [value for value in result.values() if isinstance(value, list)]
        result = lists[0] if lists else []
    if not isinstance(result, list):
        return []
    return [job for job in result if isinstance(job, dict) and job_id(job) is not None]


def job_id(job: Dict[str, Any]) -> Any:
    """Returns the id of a job record."""
    for key in ("id", "job", "job_id"):
        if isinstance(job.get(key), (int, str)) and not isinstance(job.get(key), bool):
            return job[key]
    return None


def job_site(job: Dict[str, Any]) -> Any:
    """Returns the id of the site of a job record, when the record has one."""
    for key in ("site", "site_id", "employer"):
        value = job.get(key)
        if isinstance(value, dict):
            value = value.get("id")
        if isinstance(value, int) and not isinstance(value, bool):
            return value
    return None


class Brain:
    """The state machine. One brain serves one resident."""

    def __init__(self, catalogue: Catalogue, config: Optional[AgentConfig] = None) -> None:
        self.catalogue = catalogue
        self.config = config or AgentConfig()
        #: The state of the last decision.
        self.state = State.IDLE
        #: Tick until which a verb is left alone, by verb.
        self.pause_until: Dict[str, int] = {}
        #: Failures in a row, by verb.
        self.failures: Dict[str, int] = {}
        #: The jobs of the last `list_jobs`, and the tick of that answer.
        self.jobs: List[Dict[str, Any]] = []
        self.jobs_tick: Optional[int] = None
        #: Ids of jobs that refused the resident.
        self.bad_jobs: Set[Any] = set()
        #: The job that the resident applied for or has.
        self.job: Optional[Dict[str, Any]] = None
        self.employed = False
        #: Index of the food item to buy. It moves on when a purchase fails.
        self.food_choice = 0
        self._tick = 0

    def use_catalogue(self, catalogue: Catalogue) -> None:
        """Takes a new catalogue. Verbs that the agent gave up on stay given up."""
        catalogue.unusable |= self.catalogue.unusable
        self.catalogue = catalogue

    # ---- Decide -----------------------------------------------------------

    def decide(self, observation: Dict[str, Any], sites: Sequence[Site] = ()) -> Decision:
        """Returns what to do for one observation.

        `sites` are the known sites of the zone: `{"id", "kind", "pos"}`,
        where `pos` is a tile inside the site.
        """
        senses = Senses(observation, self.catalogue)
        self._tick = senses.tick
        self._read_reports(senses)
        known = list(sites) or senses.nearby_sites
        decision = self._decide(senses, known)
        if decision.state is not self.state:
            LOG.info("state %s -> %s: %s", self.state.value, decision.state.value, decision.reason)
        self.state = decision.state
        return decision

    def _decide(self, senses: Senses, sites: Sequence[Site]) -> Decision:
        config = self.config
        blocked = senses.blocked_by
        if blocked is not None:
            return Decision(State.BLOCKED, [], config.idle_seconds, "the resident is %s" % blocked)
        if senses.queue > 0:
            return Decision(State.BUSY, [], config.busy_seconds, "planned actions wait in the queue")
        # The verb of the running action. A plan goes to the queue behind it.
        running = senses.activity["verb"] if senses.activity else None
        for rule in (self._sleep, self._eat, self._shop, self._work, self._job_hunt):
            decision = rule(senses, sites, running)
            if decision is not None:
                return decision
        if running is not None:
            return Decision(State.BUSY, [], config.busy_seconds, "the resident does `%s`" % running)
        return Decision(State.IDLE, [], config.idle_seconds, "nothing to do; the routines run")

    def _sleep(self, senses: Senses, sites: Sequence[Site], running: Optional[str]) -> Optional[Decision]:
        config = self.config
        verb = self.catalogue.first("sleep", "rest")
        energy = senses.need(ENERGY)
        if verb is None or energy is None or running == verb or self._paused(verb):
            return None
        night = hour_between(senses.hour, config.night_from, config.night_to)
        if not (energy < config.sleep_below or (night and energy < config.night_sleep_below)):
            return None
        args = self.catalogue.args_for(verb, {"ticks": config.sleep_ticks, "hours": config.sleep_ticks // 60})
        if args is None:
            return None
        steps = self._go(senses, sites, BED_KINDS) + [{"verb": verb, "args": args}]
        return Decision(State.SLEEP, steps, config.busy_seconds, "energy is %d" % energy)

    def _eat(self, senses: Senses, sites: Sequence[Site], running: Optional[str]) -> Optional[Decision]:
        food = senses.need(FOOD)
        on_hand = senses.food_on_hand()
        if food is None or food >= self.config.eat_below or not on_hand or running == "eat" or self._paused("eat"):
            return None
        args = self.catalogue.args_for("eat", {"item": on_hand[0], "qty": 1})
        if args is None:
            return None
        return Decision(State.EAT, [{"verb": "eat", "args": args}], self.config.busy_seconds, "food is %d" % food)

    def _shop(self, senses: Senses, sites: Sequence[Site], running: Optional[str]) -> Optional[Decision]:
        config = self.config
        foods = self.catalogue.food()
        if not foods or senses.need(FOOD) is None or running == "buy" or self._paused("buy"):
            return None
        if senses.food_units() >= config.keep_food or senses.cash <= 0:
            return None
        item = foods[self.food_choice % len(foods)]
        shop = self._nearest(senses, sites, SHOP_KINDS)
        values = {"item": item["name"], "qty": config.buy_qty, "site": shop["id"] if shop else None}
        args = self.catalogue.args_for("buy", values)
        if args is None:
            return None
        steps = self._go(senses, sites, SHOP_KINDS) + [{"verb": "buy", "args": args}]
        reason = "the resident carries %d food units" % senses.food_units()
        return Decision(State.SHOP, steps, config.busy_seconds, reason)

    def _work(self, senses: Senses, sites: Sequence[Site], running: Optional[str]) -> Optional[Decision]:
        config = self.config
        if not self.catalogue.has("work") or running == "work" or self._paused("work"):
            return None
        if not hour_between(senses.hour, config.work_from, config.work_to):
            return None
        energy = senses.need(ENERGY)
        if energy is not None and energy < config.work_min_energy:
            return None
        job = senses.job or self.job
        # When jobs are explicit, a resident without a job looks for one first.
        if self.catalogue.has("apply_job") and senses.job is None and not self.employed:
            return None
        values = {"ticks": config.work_ticks, "hours": config.work_ticks // 60}
        if job is not None:
            values.update({"job": job_id(job), "site": job_site(job)})
        args = self.catalogue.args_for("work", values)
        if args is None:
            return None
        steps = self._go_to_job(senses, sites, job) + [{"verb": "work", "args": args}]
        return Decision(State.WORK, steps, config.busy_seconds, "it is %02d:00, a working hour" % senses.hour)

    def _job_hunt(self, senses: Senses, sites: Sequence[Site], running: Optional[str]) -> Optional[Decision]:
        config = self.config
        if not self.catalogue.has("apply_job") or senses.job is not None or self.employed or running == "apply_job":
            return None
        fresh = self.jobs_tick is not None and senses.tick - self.jobs_tick < config.jobs_refresh_ticks
        if not fresh:
            args = self.catalogue.args_for("list_jobs", {})
            if args is None or self._paused("list_jobs"):
                return None
            return Decision(State.JOB_HUNT, [{"verb": "list_jobs", "args": args}], 0, "read the list of jobs")
        open_jobs = [job for job in self.jobs if job_id(job) not in self.bad_jobs]
        if not open_jobs or self._paused("apply_job"):
            return None
        job = max(open_jobs, key=lambda one: one.get("wage", 0) if isinstance(one.get("wage"), int) else 0)
        args = self.catalogue.args_for("apply_job", {"job": job_id(job), "site": job_site(job)})
        if args is None:
            return None
        self.job = job
        steps = self._go_to_job(senses, sites, job) + [{"verb": "apply_job", "args": args}]
        return Decision(State.JOB_HUNT, steps, config.busy_seconds, "apply for the job %s" % job_id(job))

    # ---- Places -----------------------------------------------------------

    def _nearest(self, senses: Senses, sites: Sequence[Site], kinds: Sequence[str]) -> Optional[Site]:
        """Returns the nearest known site of the first kind that has one."""
        for kind in kinds:
            matches = [site for site in sites if site.get("kind") == kind and isinstance(site.get("pos"), dict)]
            if matches:
                if senses.pos is None:
                    return matches[0]
                return min(matches, key=lambda site: (distance(site["pos"], senses.pos), site.get("id", 0)))
        return None

    def _move(self, target: Optional[Dict[str, int]]) -> List[Step]:
        if target is None:
            return []
        args = self.catalogue.args_for("move_to", {"x": target["x"], "y": target["y"]})
        return [] if args is None else [{"verb": "move_to", "args": args}]

    def _go(self, senses: Senses, sites: Sequence[Site], kinds: Sequence[str]) -> List[Step]:
        """Returns the step that walks to a site of one of the kinds. Empty when the resident is in one."""
        if senses.site is not None and senses.site.get("kind") in kinds:
            return []
        site = self._nearest(senses, sites, kinds)
        return self._move(site["pos"] if site else None)

    def _go_to_job(self, senses: Senses, sites: Sequence[Site], job: Optional[Dict[str, Any]]) -> List[Step]:
        """Returns the step that walks to the site of a job. Empty when the place is not known."""
        if job is None:
            return []
        wanted = job_site(job)
        if wanted is not None and senses.site is not None and senses.site.get("id") == wanted:
            return []
        for site in sites:
            if wanted is not None and site.get("id") == wanted and isinstance(site.get("pos"), dict):
                return self._move(site["pos"])
        pos = job.get("pos")
        if isinstance(pos, dict) and "x" in pos and "y" in pos and pos != senses.pos:
            return self._move(pos)
        return []

    # ---- Learn ------------------------------------------------------------

    def learn(self, steps: Sequence[Step], acks: Sequence[Dict[str, Any]]) -> None:
        """Reads the acknowledgements of a plan that the agent sent."""
        for step, ack in zip(steps, acks):
            status = ack["status"]
            verb = step["verb"]
            if status["state"] == "rejected":
                self._failed(verb, status["error"])
            elif status["state"] == "done":
                self._succeeded(verb, status.get("result"))

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
        if verb == "list_jobs":
            self.jobs = read_jobs(result)
            self.jobs_tick = self._tick
        elif verb == "apply_job":
            self.employed = True
        elif verb == "quit_job":
            self.employed = False

    def _failed(self, verb: str, error: Dict[str, Any], own: bool = True) -> None:
        """Reads a refusal or a failure.

        `own` is false for an action that a routine started. A routine fires
        the moment the resident is idle, often in the wrong place. Such a
        failure tells the agent what is wrong, but it does not pause the verb:
        the next plan of the agent walks to the place first.
        """
        code = error.get("code", "")
        message = error.get("message", "")
        if code == "interrupted":
            return
        if code in ("unknown_verb", "invalid_args"):
            # The catalogue changed, or the agent built arguments that the verb does not take.
            self.catalogue.give_up(verb, "%s: %s" % (code, message))
            return
        if verb == "work" and code in NO_JOB_CODES:
            self.employed = False
            self.job = None
            self.jobs_tick = None
        if verb == "apply_job" and self.job is not None:
            self.bad_jobs.add(job_id(self.job))
            self.job = None
        if verb == "buy" and code in OTHER_ITEM_CODES:
            self.food_choice += 1
        if not own:
            return
        failures = self.failures.get(verb, 0) + 1
        self.failures[verb] = failures
        # The first `not_here` gets a new try at once: the next plan walks to the place.
        if code == "not_here" and failures == 1 and self.catalogue.has("move_to"):
            return
        pause = self.config.retry_ticks * 2 ** min(failures - 1, PAUSE_DOUBLINGS_MAX)
        self.pause_until[verb] = self._tick + pause
        LOG.info("`%s` failed (%s: %s); the agent leaves it alone for %d ticks", verb, code, message, pause)

    def _paused(self, verb: str) -> bool:
        return self.pause_until.get(verb, 0) > self._tick
