"""Routines: standing orders that keep the resident alive between decisions.

The server runs a routine when the resident is idle and the condition of the
routine holds. A routine uses one fixed action, so it cannot walk to the
right place or buy food. The decisions of the agent do that. A routine that
fails writes a report, and the report wakes the agent.
"""

from typing import Any, Dict, List

from .catalogue import Catalogue
from .config import AgentConfig
from .senses import ENERGY, FOOD

#: How many kinds of food get their own eat routine.
FOOD_ROUTINES_MAX = 2


def build_routines(catalogue: Catalogue, config: AgentConfig) -> List[Dict[str, Any]]:
    """Returns the routines that fit the catalogue, in priority order.

    A routine is left out when its verb, its attribute or its item does not
    exist: the server refuses a list that names something unknown.
    """
    routines: List[Dict[str, Any]] = []

    if FOOD in catalogue.attrs:
        for item in catalogue.food()[:FOOD_ROUTINES_MAX]:
            args = catalogue.args_for("eat", {"item": item["name"], "qty": 1})
            if args is not None:
                routines.append(
                    {
                        "name": "eat %s when hungry" % item["name"],
                        "when": {
                            "type": "all",
                            "of": [
                                {"type": "attr_below", "attr": FOOD, "value": config.eat_below},
                                {"type": "has_item", "item": item["name"], "qty": 1},
                            ],
                        },
                        "then": {"verb": "eat", "args": args},
                    }
                )

    sleep = catalogue.first("sleep", "rest")
    if sleep is not None and ENERGY in catalogue.attrs:
        args = catalogue.args_for(sleep, {"ticks": config.sleep_ticks, "hours": config.sleep_ticks // 60})
        if args is not None:
            routines.append(
                {
                    "name": "sleep when tired",
                    "when": {
                        "type": "any",
                        "of": [
                            {"type": "attr_below", "attr": ENERGY, "value": config.sleep_below},
                            {
                                "type": "all",
                                "of": [
                                    {"type": "clock_between", "from_hour": config.night_from, "to_hour": config.night_to},
                                    {"type": "attr_below", "attr": ENERGY, "value": config.night_sleep_below},
                                ],
                            },
                        ],
                    },
                    "then": {"verb": sleep, "args": args},
                }
            )

    if catalogue.has("work"):
        args = catalogue.args_for("work", {"ticks": config.work_ticks, "hours": config.work_ticks // 60})
        if args is not None:
            parts: List[Dict[str, Any]] = [
                {"type": "clock_between", "from_hour": config.work_from, "to_hour": config.work_to}
            ]
            if ENERGY in catalogue.attrs:
                parts.append({"type": "attr_above", "attr": ENERGY, "value": config.work_min_energy})
            routines.append({"name": "work in the day", "when": {"type": "all", "of": parts}, "then": {"verb": "work", "args": args}})

    return routines
