"""What the agent reads from one observation."""

from typing import Any, Dict, List, Optional

from .catalogue import Catalogue

FOOD = "need.food"
ENERGY = "need.energy"


class Senses:
    """The parts of an observation that the agent uses. Every part can be absent."""

    def __init__(self, observation: Dict[str, Any], catalogue: Catalogue) -> None:
        sections = observation.get("sections", {})
        me = sections.get("self") or {}
        self.tick: int = observation.get("tick", 0)
        self.hour: int = observation.get("clock", {}).get("hour", 0)
        self.attrs: Dict[str, int] = me.get("attrs") or {}
        self.cash: int = me.get("cash") or 0
        self.items: Dict[str, int] = me.get("items") or {}
        self.pos: Optional[Dict[str, int]] = me.get("pos")
        self.zone: Optional[int] = me.get("zone")
        self.site: Optional[Dict[str, Any]] = me.get("site")
        self.activity: Optional[Dict[str, Any]] = me.get("activity")
        self.queue: int = me.get("queue") or 0
        self.conditions: Dict[str, Any] = me.get("conditions") or {}
        self.nearby_sites: List[Dict[str, Any]] = (sections.get("nearby") or {}).get("sites") or []
        reports = sections.get("reports")
        self.reports: List[Dict[str, Any]] = reports if isinstance(reports, list) else []
        job = sections.get("job")
        #: The section `job` of the economy module. Present when the resident has a job.
        self.job: Optional[Dict[str, Any]] = job if isinstance(job, dict) and job else None
        self._catalogue = catalogue

    @property
    def busy(self) -> bool:
        """True when the resident runs an action or has planned actions."""
        return self.activity is not None or self.queue > 0

    @property
    def blocked_by(self) -> Optional[str]:
        """The blocking condition of the resident, for example `jailed`, or `None`."""
        for name in sorted(self.conditions):
            if name in self._catalogue.blocking:
                return name
        return None

    def need(self, name: str) -> Optional[int]:
        """Returns the value of a need, or `None` when the world does not have it."""
        value = self.attrs.get(name)
        return value if isinstance(value, int) else None

    def food_on_hand(self) -> List[str]:
        """Returns the names of the food items that the resident carries, the most nutritious first."""
        return [item["name"] for item in self._catalogue.food() if self.items.get(item["name"], 0) > 0]

    def food_units(self) -> int:
        """Returns how many units of food the resident carries."""
        return sum(self.items.get(item["name"], 0) for item in self._catalogue.food())
