"""The numbers that steer the agent. Every one has a default and can be changed."""

from dataclasses import dataclass, fields
from typing import Any, Dict


@dataclass(frozen=True)
class AgentConfig:
    """Thresholds and times of the scripted agent.

    Needs go from 0 (worst) to 1000 (best). One tick is one game minute. At
    the default speed of the server, one tick is one second.
    """

    #: Eat when `need.food` is below this value.
    eat_below: int = 500
    #: Buy food when the resident carries less than this number of food units.
    keep_food: int = 2
    #: How many units to buy in one purchase.
    buy_qty: int = 2
    #: Sleep at any hour when `need.energy` is below this value.
    sleep_below: int = 250
    #: At night, sleep when `need.energy` is below this value.
    night_sleep_below: int = 700
    #: First hour of the night.
    night_from: int = 22
    #: First hour after the night.
    night_to: int = 6
    #: Length of one sleep in ticks, for a `sleep` verb that takes a duration.
    sleep_ticks: int = 480
    #: First hour of the working day, for the work routine.
    work_from: int = 8
    #: First hour after the working day, for the work routine.
    work_to: int = 18
    #: Do not start work when `need.energy` is below this value.
    work_min_energy: int = 300
    #: Length of one shift in ticks, for a `work` verb that takes a duration.
    work_ticks: int = 120
    #: Ticks before the agent tries a verb again after the server refused it or it failed.
    retry_ticks: int = 60
    #: Ticks before the agent asks for the list of jobs again.
    jobs_refresh_ticks: int = 720
    #: Seconds that the agent blocks for news when it has nothing to do.
    idle_seconds: int = 300
    #: Seconds that the agent blocks for the report of an action that it started.
    busy_seconds: int = 600
    #: Least seconds between two decisions. It bounds the request rate.
    min_decision_seconds: float = 2.0
    #: Seconds that the agent waits after an error of the network or the server.
    error_seconds: float = 30.0
    #: Decisions between two reads of the catalogue. New verbs appear when the server is upgraded.
    catalogue_every: int = 200

    @classmethod
    def from_dict(cls, values: Dict[str, Any]) -> "AgentConfig":
        """Makes a configuration from a dict, for example from a JSON file. An unknown key is an error."""
        known = {field.name for field in fields(cls)}
        unknown = sorted(set(values) - known)
        if unknown:
            raise ValueError("unknown configuration keys: %s" % ", ".join(unknown))
        return cls(**values)
