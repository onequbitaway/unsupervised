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
    """Walks to a tile (`x` and `y`) or into a site (`site`), as the verb of the life module does."""
    x, y, site = args.get("x"), args.get("y"), args.get("site")
    if (site is None) == (x is None and y is None) or (x is None) != (y is None):
        raise FakeActionError("invalid_args", "Give `x` and `y`, or `site`.")
    if site is not None:
        found = next((one for one in world.sites if one["id"] == site), None)
        if found is None:
            raise FakeActionError("not_found", "No site of your city has this id.")
        rect = found["rect"]
        target = {"x": (rect["x0"] + rect["x1"]) // 2, "y": (rect["y0"] + rect["y1"]) // 2}
    else:
        target = {"x": x, "y": y}
    steps = distance(resident.pos, target)

    def arrive() -> Dict[str, int]:
        resident.pos = dict(target)
        result = dict(target, steps=steps, ticks=max(1, steps))
        if site is not None:
            result["site"] = site
        return result

    return Busy(max(1, steps), arrive)


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
                "Walk to a tile or a site. One tile takes one game minute.",
                properties={
                    "x": {"type": ["integer", "null"], "description": "Column of the target tile."},
                    "y": {"type": ["integer", "null"], "description": "Row of the target tile."},
                    "site": {"type": ["integer", "null"], "description": "Id of a site. The walk ends inside the site."},
                },
            ),
            _move_to,
        ),
    ]
