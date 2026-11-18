"""Routines: standing orders that keep the resident alive without the agent.

The server runs a routine when the resident is idle and the condition of the
routine holds. A routine uses one fixed action, so it cannot walk to the
right place or buy food. The decisions of the agent do that. A routine that
fails writes a report, and the report wakes the agent. A routine that
succeeds writes none.
"""

from typing import Any, Dict, List

from .catalogue import Catalogue
from .config import AgentConfig
from .senses import ENERGY, FOOD

#: How many kinds of food get their own eat routine.
FOOD_ROUTINES_MAX = 3


def build_routines(catalogue: Catalogue, config: AgentConfig) -> List[Dict[str, Any]]:
    """Returns the routines that fit the catalogue, in priority order.

    The routines are the reserve for the time without the agent: they eat
    the food that the resident carries and they sleep where it stands. Their
    thresholds are below those of the agent, so that the agent acts first
    while it runs. A routine is left out when its verb, its attribute or its
    item does not exist: the server refuses a list that names something
    unknown.
    """
    routines: List[Dict[str, Any]] = []

    if FOOD in catalogue.attrs:
        for item in catalogue.food()[:FOOD_ROUTINES_MAX]:
            args = catalogue.args_for("eat", {"item": item["name"]})
            if args is not None:
                routines.append(
                    {
                        "name": "eat %s when hungry" % item["name"],
                        "when": {
                            "type": "all",
                            "of": [
                                {"type": "attr_below", "attr": FOOD, "value": config.routine_eat_below},
                                {"type": "has_item", "item": item["name"], "qty": 1},
                            ],
                        },
                        "then": {"verb": "eat", "args": args},
                    }
                )

    if catalogue.has("sleep") and ENERGY in catalogue.attrs:
        routines.append(
            {
                "name": "sleep when exhausted",
                "when": {"type": "attr_below", "attr": ENERGY, "value": config.routine_sleep_below},
                # Without arguments the resident sleeps until its energy is full.
                "then": {"verb": "sleep", "args": {}},
            }
        )

    return routines
