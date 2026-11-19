"""The loop of one resident: observe, decide, plan, wait."""

import logging
import threading
import time
from typing import Any, Callable, Dict, List, Optional

from .brain import Brain, Decision, Site
from .catalogue import Catalogue
from .config import AgentConfig
from .routines import build_routines
from .sdk import unsupervised

LOG = logging.getLogger("scripted_agent")

#: After this number of decisions in a row without a wait, the agent waits.
FREE_DECISIONS_MAX = 3
#: Least seconds of one long poll.
WAIT_MIN_SECONDS = 1.0
#: The states of an acknowledgement after which a report follows.
REPORT_FOLLOWS = ("started", "queued", "pending")


class Agent:
    """Runs the state machine for one resident through a `Client` of the SDK."""

    def __init__(
        self,
        client: Any,
        config: Optional[AgentConfig] = None,
        *,
        variety: int = 0,
        sleep: Callable[[float], None] = time.sleep,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        """Makes an agent for the resident of `client`.

        `variety` is a small number that differs between the residents of
        one command: they then choose different places of the same worth.
        `sleep` and `clock` exist for tests: the agent sleeps only to keep
        `min_decision_seconds` between decisions and to wait after an error.
        """
        self.client = client
        self.config = config or AgentConfig()
        self.variety = variety
        self.brain: Optional[Brain] = None
        #: The routines that the server accepted.
        self.routines: List[Dict[str, Any]] = []
        #: How many decisions the agent made.
        self.decisions = 0
        #: The last error that stopped the agent, if one did.
        self.fatal: Optional[Exception] = None
        #: One sentence that says why the agent stopped, when `fatal` is set.
        self.farewell: Optional[str] = None
        self._sleep = sleep
        self._clock = clock
        self._observation: Optional[Dict[str, Any]] = None
        self._city: Optional[int] = None
        self._city_sites: List[Site] = []
        self._free_decisions = 0
        self._last_decision: Optional[float] = None
        self._catalogue_at = 0
        self._installed = False

    # ---- Set-up -----------------------------------------------------------

    def setup(self) -> None:
        """Reads the catalogue and installs the routines that fit it."""
        catalogue = Catalogue(self.client.catalogue())
        if self.brain is None:
            self.brain = Brain(catalogue, self.config, self.variety)
        else:
            self.brain.use_catalogue(catalogue)
        self._catalogue_at = self.decisions
        wanted = build_routines(catalogue, self.config)
        if not self._installed or wanted != self.routines:
            self.routines = self._install(wanted)
            self._installed = True

    def _install(self, routines: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """Sends the routines. Returns the list that the server accepted.

        The server refuses a whole list for one routine that it does not
        like. The agent then tries the routines one by one and keeps the
        good ones.
        """
        try:
            self.client.set_routines(routines)
            return routines
        except unsupervised.ApiError as error:
            if error.ends_credential or error.is_transient or not routines:
                raise
            LOG.warning("the server refused the routines (%s); the agent tries them one by one", error)
        accepted: List[Dict[str, Any]] = []
        for routine in routines:
            try:
                self.client.set_routines(accepted + [routine])
                accepted.append(routine)
            except unsupervised.ApiError as error:
                if error.ends_credential or error.is_transient:
                    raise
                LOG.warning("the routine `%s` is left out: %s", routine["name"], error)
        self.client.set_routines(accepted)
        return accepted

    # ---- Places -----------------------------------------------------------

    def sites(self, observation: Dict[str, Any]) -> List[Site]:
        """Returns the sites of the city of the resident.

        The observation shows only the sites within sight. The public map
        has all of them, so the agent reads it one time for each city:
        `GET /v1/world`, then one zone view for each district.
        """
        me = observation.get("sections", {}).get("self") or {}
        city = me.get("city")
        if city is not None and city != self._city:
            sites: List[Site] = []
            try:
                layout = self.client.world()
                zones = next(entry["zones"] for entry in layout["cities"] if entry["id"] == city)
                for zone in zones:
                    for site in self.client.zone(zone, residents=False).get("sites", []):
                        rect = site["rect"]
                        middle = {"x": (rect["x0"] + rect["x1"]) // 2, "y": (rect["y0"] + rect["y1"]) // 2}
                        sites.append({"id": site["id"], "kind": site["kind"], "name": site.get("name"), "pos": middle})
                self._city, self._city_sites = city, sites
            except (unsupervised.UnsupervisedError, KeyError, StopIteration, TypeError) as error:
                LOG.info("no map of city %s (%s); the agent uses the sites within sight", city, error)
        if self._city == city and self._city_sites:
            return self._city_sites
        return (observation.get("sections", {}).get("nearby") or {}).get("sites") or []

    # ---- Loop -------------------------------------------------------------

    def step(self) -> Decision:
        """Makes one decision: decide, plan the actions, then block for news."""
        if self.brain is None or self.decisions - self._catalogue_at >= self.config.catalogue_every:
            self.setup()
        assert self.brain is not None
        if self._observation is None:
            self._observation = self.client.observe()
        decision = self.brain.decide(self._observation, self.sites(self._observation))
        if decision.say is not None:
            self._say(decision.say)
        wait_ticks = decision.wait_ticks
        if decision.steps:
            result = self.client.plan(decision.steps)
            self._observation = result.observation
            self.brain.learn(decision.steps, result.acks)
            # A report comes only for an action that runs, waits in the
            # queue or waits for an answer. When the server refused the
            # plan, or all of it finished at once, no report will end the
            # wait: the agent decides again.
            if not any(ack["status"]["state"] in REPORT_FOLLOWS for ack in result.acks):
                wait_ticks = 0
        self.decisions += 1
        if wait_ticks <= 0:
            self._free_decisions += 1
            if self._free_decisions >= FREE_DECISIONS_MAX:
                wait_ticks = 60
        if wait_ticks > 0:
            self._free_decisions = 0
            self._pace()
            seconds = min(self.config.busy_seconds, max(WAIT_MIN_SECONDS, wait_ticks * self.config.tick_seconds))
            # Only a report wakes the agent. Talk of other residents does not.
            self._observation = self.client.wait(seconds, [unsupervised.REPORT_KIND])
        return decision

    def _say(self, words: str) -> None:
        """Says words aloud. A refusal of the words makes the agent silent; it does not stop it."""
        assert self.brain is not None
        try:
            ack, self._observation = self.client.act("say", {"text": words})
            if ack["status"]["state"] == "rejected":
                self.brain.learn([{"verb": "say"}], [ack])
        except unsupervised.ApiError as error:
            if not error.is_text_refusal:
                raise
            LOG.warning("the server refused the words of the resident (%s); it says nothing more", error.code)
            self.brain.refused_words()

    def _pace(self) -> None:
        """Keeps `min_decision_seconds` between two decisions."""
        now = self._clock()
        if self._last_decision is not None:
            early = self._last_decision + self.config.min_decision_seconds - now
            if early > 0:
                self._sleep(early)
                now = self._clock()
        self._last_decision = now

    def run(self, decisions: Optional[int] = None, stop: Optional[threading.Event] = None) -> int:
        """Runs until `decisions` decisions are made, `stop` is set, or the key stops to work.

        An error of the network or the server does not stop the agent: it
        waits `error_seconds` and goes on. An error that ends the key stops
        it: the key is unknown, the resident left the world, or the owner
        is banned. `fatal` then has the error and `farewell` has one
        sentence that says what happened. Returns the number of decisions.
        """
        while (decisions is None or self.decisions < decisions) and not (stop is not None and stop.is_set()):
            try:
                self.step()
            except unsupervised.ApiError as error:
                if error.ends_credential:
                    self._stop(error, self._why_the_key_ended(error))
                    break
                self._recover(error)
            except unsupervised.MissingCredentialError as error:
                self._stop(error, str(error))
                break
            except unsupervised.UnsupervisedError as error:
                self._recover(error)
        return self.decisions

    def _stop(self, error: Exception, farewell: str) -> None:
        LOG.error("%s; the agent stops", farewell)
        self.fatal = error
        self.farewell = farewell

    def _why_the_key_ended(self, error: Any) -> str:
        """Returns one sentence for an error that ends the key of the resident.

        A resident that left the world gives two answers. The first call
        with its key gets `410 resident_gone`. Each later call gets
        `401 invalid_token`: the server forgot the key. A key that another
        key replaced gets the same 401. The owner token tells the two
        apart: it lists the residents that are in the world.
        """
        resident = getattr(self.client, "resident", None)
        who = "the resident" if resident is None else "resident %s" % resident
        fate = "gone" if error.code == "resident_gone" else None
        if fate is None and error.status == 401 and resident is not None and getattr(self.client, "owner_token", None):
            try:
                alive = [record["id"] for record in self.client.list_residents()["residents"]]
                fate = "rekeyed" if resident in alive else "gone"
            except unsupervised.UnsupervisedError:
                # The reason stays unknown. The first error is the one that the user must see.
                pass
        if fate == "gone":
            return (
                "%s left the world: it died, or an administrator removed it. Its key does not work any more. "
                "Run the command again to make a successor" % who
            )
        if fate == "rekeyed":
            return (
                "the server does not know the key of %s any more: another key replaced it. "
                "Run the command again to get a new key" % who
            )
        return "the key of %s does not work any more (%s)" % (who, error)

    def _recover(self, error: Exception) -> None:
        LOG.warning("%s; the agent waits %s seconds", error, self.config.error_seconds)
        self._observation = None
        self._sleep(self.config.error_seconds)
