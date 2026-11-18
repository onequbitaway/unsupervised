"""The verbs that a new fake world has: the three kernel verbs and `move_to`."""

from typing import Any, Dict, List

from .world import Busy, Done, FakeActionError, FakeResident, FakeVerb, FakeWorld, Outcome, distance, verb_info


def _wait(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Outcome:
    ticks = args["ticks"]
    limit = world.limits["wait_max"]
    if ticks < 1 or ticks > limit:
        raise FakeActionError("invalid_args", "`ticks` must be from 1 to %d." % limit)
    return Busy(ticks, lambda: {"waited": ticks})


def _cancel(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Outcome:
    return Done(world.cancel(resident))


def _set_routines(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Outcome:
    return Done(world.set_routines(resident, args["routines"]))


def _move_to(resident: FakeResident, args: Dict[str, Any], world: FakeWorld) -> Outcome:
    target = {"x": args["x"], "y": args["y"]}

    def arrive() -> Dict[str, int]:
        resident.pos = dict(target)
        return dict(target)

    return Busy(max(1, distance(resident.pos, target)), arrive)


def default_verbs() -> List[FakeVerb]:
    """Returns the default verbs: `wait`, `cancel`, `set_routines` and `move_to`."""
    return [
        FakeVerb(
            verb_info(
                "wait",
                "kernel",
                "Do nothing for a number of ticks.",
                properties={"ticks": {"type": "integer", "minimum": 0, "description": "How many ticks to do nothing."}},
                required=["ticks"],
            ),
            _wait,
        ),
        FakeVerb(
            verb_info("cancel", "kernel", "Stop the current action and clear the queue of planned actions.", free=True),
            _cancel,
        ),
        FakeVerb(
            verb_info(
                "set_routines",
                "kernel",
                "Replace the routines: standing orders that run when the resident is idle.",
                free=True,
                properties={"routines": {"type": "array", "description": "The new routines, in priority order."}},
                required=["routines"],
            ),
            _set_routines,
        ),
        FakeVerb(
            verb_info(
                "move_to",
                "life",
                "Walk to a tile. One tile takes one game minute.",
                properties={
                    "x": {"type": "integer", "description": "Column of the target tile."},
                    "y": {"type": "integer", "description": "Row of the target tile."},
                },
                required=["x", "y"],
            ),
            _move_to,
        ),
    ]
