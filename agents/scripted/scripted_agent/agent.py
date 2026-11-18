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
#: HTTP statuses that mean: the key does not work. The agent stops.
FATAL_STATUSES = (401, 403)


class Agent:
    """Runs the state machine for one resident through a `Client` of the SDK."""

    def __init__(
        self,
        client: Any,
        config: Optional[AgentConfig] = None,
        *,
        sleep: Callable[[float], None] = time.sleep,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        """Makes an agent for the resident of `client`.

        `sleep` and `clock` exist for tests: the agent sleeps only to keep
        `min_decision_seconds` between decisions and to wait after an error.
        """
        self.client = client
        self.config = config or AgentConfig()
        self.brain: Optional[Brain] = None
        #: The routines that the server accepted.
        self.routines: List[Dict[str, Any]] = []
        #: How many decisions the agent made.
        self.decisions = 0
        #: The last error that stopped the agent, if one did.
        self.fatal: Optional[Exception] = None
        self._sleep = sleep
        self._clock = clock
        self._observation: Optional[Dict[str, Any]] = None
        self._zone: Optional[int] = None
        self._zone_sites: List[Site] = []
        self._free_decisions = 0
        self._last_decision: Optional[float] = None
        self._catalogue_at = 0
        self._installed = False
        self._wanted: List[Dict[str, Any]] = []

    # ---- Set-up -----------------------------------------------------------

    def setup(self) -> None:
        """Reads the catalogue and installs the routines that fit it."""
        catalogue = Catalogue(self.client.catalogue())
        if self.brain is None:
            self.brain = Brain(catalogue, self.config)
        else:
            self.brain.use_catalogue(catalogue)
        self._catalogue_at = self.decisions
        self._zone = None
        wanted = build_routines(self.brain.catalogue, self.config)
        # The first set-up always writes the list: the resident can have routines of an earlier run.
        if not self._installed or wanted != self._wanted:
            self.routines = self._install(wanted)
            self._wanted = wanted
            self._installed = True
            LOG.info("routines: %s", ", ".join(routine["name"] for routine in self.routines) or "none")

    def _install(self, wanted: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """Installs routines and returns the ones that the server took.

        The server refuses a whole list for one bad routine. Then the agent
        adds the routines one by one and keeps those that pass.
        """
        try:
            self.client.set_routines(wanted)
            return wanted
        except unsupervised.ApiError as error:
            if error.status in FATAL_STATUSES or error.is_transient:
                raise
            LOG.warning("the server refused the routines (%s); the agent tries them one by one", error)
        accepted: List[Dict[str, Any]] = []
        for routine in wanted:
            try:
                self.client.set_routines(accepted + [routine])
                accepted.append(routine)
            except unsupervised.ApiError as error:
                if error.status in FATAL_STATUSES or error.is_transient:
                    raise
                LOG.warning("the routine `%s` is left out: %s", routine["name"], error)
        self.client.set_routines(accepted)
        return accepted

    # ---- Places -----------------------------------------------------------

    def sites(self, observation: Dict[str, Any]) -> List[Site]:
        """Returns the known sites of the zone of the resident.

        The observation shows only the sites within sight. The public view of
        the zone has all of them, so the agent reads it once for each zone.
        """
        me = observation.get("sections", {}).get("self") or {}
        zone = me.get("zone")
        if zone is not None and zone != self._zone:
            self._zone = zone
            self._zone_sites = []
            try:
                view = self.client.zone(zone)
                for site in view.get("sites", []):
                    rect = site["rect"]
                    centre = {"x": (rect["x0"] + rect["x1"]) // 2, "y": (rect["y0"] + rect["y1"]) // 2}
                    self._zone_sites.append({"id": site["id"], "kind": site["kind"], "name": site.get("name"), "pos": centre})
            except (unsupervised.UnsupervisedError, KeyError, TypeError) as error:
                LOG.info("no view of zone %s (%s); the agent uses the sites within sight", zone, error)
        known = {site["id"] for site in self._zone_sites}
        nearby = (observation.get("sections", {}).get("nearby") or {}).get("sites") or []
        return self._zone_sites + [site for site in nearby if site.get("id") not in known]

    # ---- Loop -------------------------------------------------------------

    def step(self) -> Decision:
        """Makes one decision: decide, plan the actions, then block for news."""
        if self.brain is None or self.decisions - self._catalogue_at >= self.config.catalogue_every:
            self.setup()
        assert self.brain is not None
        if self._observation is None:
            self._observation = self.client.observe()
        decision = self.brain.decide(self._observation, self.sites(self._observation))
        if decision.steps:
            acks, self._observation = self.client.plan(decision.steps)
            self.brain.learn(decision.steps, acks)
        self.decisions += 1
        wait_seconds = decision.wait_seconds
        if wait_seconds <= 0:
            self._free_decisions += 1
            if self._free_decisions >= FREE_DECISIONS_MAX:
                wait_seconds = self.config.idle_seconds
        if wait_seconds > 0:
            self._free_decisions = 0
            self._pace()
            # Only a report wakes the agent. Talk of other residents does not.
            self._observation = self.client.wait(wait_seconds, [unsupervised.REPORT_KIND])
        return decision

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
        waits `error_seconds` and goes on. Returns the number of decisions.
        """
        while (decisions is None or self.decisions < decisions) and not (stop is not None and stop.is_set()):
            try:
                self.step()
            except unsupervised.ApiError as error:
                if error.status in FATAL_STATUSES:
                    LOG.error("the server refused the key (%s); the agent stops", error)
                    self.fatal = error
                    break
                self._recover(error)
            except unsupervised.MissingCredentialError as error:
                LOG.error("%s; the agent stops", error)
                self.fatal = error
                break
            except unsupervised.UnsupervisedError as error:
                self._recover(error)
        return self.decisions

    def _recover(self, error: Exception) -> None:
        LOG.warning("%s; the agent waits %s seconds", error, self.config.error_seconds)
        self._observation = None
        self._sleep(self.config.error_seconds)
