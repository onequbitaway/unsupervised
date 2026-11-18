"""The numbers that steer the agent. Every one has a default and can be changed."""

from dataclasses import dataclass, fields, replace
from typing import Any, Dict


@dataclass(frozen=True)
class AgentConfig:
    """Thresholds and times of the scripted agent.

    Needs go from 0 (worst) to 1000 (best). One tick is one game minute. At
    the default speed of the server, one tick is one second.
    """

    #: Eat when `need.food` is below this value.
    eat_below: int = 600
    #: Buy food when the resident carries less than this number of food units.
    keep_food: int = 2
    #: Greatest number of units in one purchase.
    buy_qty: int = 3
    #: Sleep at any hour when `need.energy` is below this value.
    sleep_below: int = 250
    #: At night, sleep when `need.energy` is below this value.
    night_sleep_below: int = 750
    #: First hour of the night.
    night_from: int = 22
    #: First hour after the night.
    night_to: int = 6
    #: First hour of the working day.
    work_from: int = 6
    #: First hour after the working day.
    work_to: int = 20
    #: A resident with at least this energy also works outside the working day: it is rested.
    rested_above: int = 900
    #: Greatest number of starter shifts of the city in one game day. The budget
    #: of the city is for newcomers too. After them the resident gathers on a farm.
    public_shifts_per_day: int = 2
    #: Do not start work when `need.energy` is below this value.
    work_min_energy: int = 300
    #: Game minutes of one `gather` action at a farm.
    gather_minutes: int = 60
    #: Sell gathered food at the depot when the resident carries more units than this.
    sell_above: int = 8
    #: Put cash in the bank when the resident carries more cents than this.
    save_above: int = 6000
    #: Cents that stay in the pocket after a deposit.
    keep_cash: int = 3000
    #: Say something to a resident in sight at most one time in this number of ticks. 0: never.
    talk_every_ticks: int = 240
    #: The routine that eats fires when `need.food` is below this value. It is the reserve for the time without the agent.
    routine_eat_below: int = 300
    #: The routine that sleeps fires when `need.energy` is below this value.
    routine_sleep_below: int = 150
    #: Ticks before the agent tries a verb again after the server refused it or it failed.
    retry_ticks: int = 60
    #: Ticks before the agent asks for the list of jobs again.
    jobs_refresh_ticks: int = 720
    #: Real seconds of one tick of the server. The agent uses it to wait as long as an action takes.
    tick_seconds: float = 1.0
    #: Seconds that the agent blocks for news when it has nothing to do.
    idle_seconds: float = 300.0
    #: Greatest number of seconds that the agent blocks for the report of an action.
    busy_seconds: float = 900.0
    #: Least seconds between two decisions. It bounds the request rate.
    min_decision_seconds: float = 2.0
    #: Seconds that the agent waits after an error of the network or the server.
    error_seconds: float = 30.0
    #: Game minutes over which the residents of one command start. 0: all start at once.
    #: With a spread, the residents of a new world do not walk and work as one group.
    start_spread_ticks: int = 0
    #: Decisions between two reads of the catalogue. New verbs appear when the server is upgraded.
    catalogue_every: int = 500

    def varied(self, index: int) -> "AgentConfig":
        """Returns the configuration of resident number `index` (from 0) of one command.

        The rules are the same and the numbers differ a little: the start
        and the end of the working day, the hunger that makes the resident
        eat, and how often it talks. So the residents of one command do not
        move as one group. Resident 0 has the configuration as it is.
        """
        return replace(
            self,
            work_from=self.work_from + index % 3,
            work_to=self.work_to - (index // 3) % 3,
            eat_below=self.eat_below + 30 * (index % 4),
            sell_above=self.sell_above + index % 5,
            talk_every_ticks=self.talk_every_ticks + 15 * (index % 7) if self.talk_every_ticks > 0 else 0,
        )

    @classmethod
    def from_dict(cls, values: Dict[str, Any]) -> "AgentConfig":
        """Makes a configuration from a dict, for example from a JSON file. An unknown key is an error."""
        known = {field.name for field in fields(cls)}
        unknown = sorted(set(values) - known)
        if unknown:
            raise ValueError("unknown configuration keys: %s" % ", ".join(unknown))
        return cls(**values)
