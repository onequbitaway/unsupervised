"""The checks of a sponsor mission, for the route `POST /v1/missions` of the fake gateway.

The real server does them in this order (`docs/design/protocol.md`,
"Missions of sponsors"): the switch of the administrator, the form and the
sizes, the money of one mission, the text filters, the limit of missions of
one owner. Then the missions module checks the ranges of the numbers. This
module has the form, the sizes, the money and the ranges. The fake does not
know the event kinds of the game: it takes each kind.
"""

from typing import Any, Dict, NamedTuple, Optional

from .world import byte_length

#: The limits of the server for sponsor missions: the `[sponsors]` table of
#: its configuration. `enabled` false: the route answers `403
#: missions_disabled`. `missions_per_day`: missions that one owner can
#: publish in 24 hours; 0 is no limit. `max_mission_cents`: the greatest sum
#: of new money for one mission, the reward times the takers; 0 is no limit.
DEFAULT_SPONSORS: Dict[str, Any] = {"enabled": True, "missions_per_day": 3, "max_mission_cents": 500000}

#: The defaults of the parameters of the missions module that bound a definition.
SIZES: Dict[str, int] = {
    "title_max": 80,
    "text_max": 1000,
    "objectives_max": 4,
    "match_max": 4,
    "event_kind_max": 64,
    "field_name_max": 32,
    "field_text_max": 64,
    "count_max": 1000,
    "reward_min": 100,
    "reward_max": 10000000,
    "takers_max": 50,
    "deadline_min": 10,
    "deadline_max": 40320,
}

MISSION_FIELDS = ("title", "text", "objectives", "mode", "org", "reward", "deadline_in", "takers")
OBJECTIVE_FIELDS = ("event", "fields", "count", "actor")
#: The greatest value of 32 bits: the greatest count that the server reads.
COUNT_MAX = 0xFFFFFFFF


class MissionRefusal(NamedTuple):
    """Why the route refuses a mission: the HTTP status, the error code and one sentence."""

    status: int
    code: str
    message: str


def _whole(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _count(value: Any) -> bool:
    return _whole(value) and 0 <= value <= COUNT_MAX


def _invalid(message: str) -> MissionRefusal:
    return MissionRefusal(400, "invalid_request", message)


def _wrong(field: str) -> MissionRefusal:
    return _invalid("The body is not a mission: `%s` is missing or has a wrong type." % field)


def mission_form(body: Any, limits: Dict[str, Any]) -> Optional[MissionRefusal]:
    """Checks the form and the sizes of a definition and the money that it asks for.

    The route does this before the mission reaches the world. Returns the
    refusal, or `None` for a definition that passes.
    """
    if not isinstance(body, dict):
        return _invalid("The body must be an object: the definition of a mission.")
    for name in body:
        if name not in MISSION_FIELDS:
            return _invalid("The body is not a mission: unknown field `%s`." % name)
    title, text, objectives = body.get("title"), body.get("text"), body.get("objectives")
    if not isinstance(title, str):
        return _wrong("title")
    if "text" in body and not isinstance(text, str):
        return _wrong("text")
    if not isinstance(objectives, list):
        return _wrong("objectives")
    if "mode" in body and body["mode"] not in ("solo", "team"):
        return _wrong("mode")
    if body.get("org") is not None and not _count(body["org"]):
        return _wrong("org")
    if not _whole(body.get("reward")):
        return _wrong("reward")
    if not _count(body.get("deadline_in")):
        return _wrong("deadline_in")
    if "takers" in body and not _count(body["takers"]):
        return _wrong("takers")
    for index, objective in enumerate(objectives):
        place = "objectives[%d]" % index
        if not isinstance(objective, dict) or any(name not in OBJECTIVE_FIELDS for name in objective):
            return _wrong(place)
        if not isinstance(objective.get("event"), str):
            return _wrong(place + ".event")
        if "fields" in objective and not isinstance(objective["fields"], dict):
            return _wrong(place + ".fields")
        if "count" in objective and not _count(objective["count"]):
            return _wrong(place + ".count")
        if "actor" in objective and objective["actor"] not in ("assignee", "team"):
            return _wrong(place + ".actor")

    if not title.strip():
        return _invalid("`title` must not be empty.")
    if byte_length(title) > SIZES["title_max"]:
        return _invalid("`title` has at most %d bytes." % SIZES["title_max"])
    if isinstance(text, str) and byte_length(text) > SIZES["text_max"]:
        return _invalid("`text` has at most %d bytes." % SIZES["text_max"])
    if not 1 <= len(objectives) <= SIZES["objectives_max"]:
        return _invalid("`objectives` has from 1 to %d objectives." % SIZES["objectives_max"])
    for index, objective in enumerate(objectives):
        place = "`objectives[%d]`" % index
        fields = objective.get("fields") or {}
        if byte_length(objective["event"]) > SIZES["event_kind_max"]:
            return _invalid("%s: `event` has at most %d bytes." % (place, SIZES["event_kind_max"]))
        if len(fields) > SIZES["match_max"]:
            return _invalid("%s: `fields` has at most %d data fields." % (place, SIZES["match_max"]))
        if any(byte_length(name) > SIZES["field_name_max"] for name in fields):
            return _invalid("%s: the name of a data field has at most %d bytes." % (place, SIZES["field_name_max"]))
        if any(isinstance(value, str) and byte_length(value) > SIZES["field_text_max"] for value in fields.values()):
            return _invalid(
                "%s: a text value of a data field has at most %d bytes." % (place, SIZES["field_text_max"])
            )

    # A reward below zero is no money. The module refuses it.
    total = max(0, body["reward"]) * body.get("takers", 1)
    limit = limits["max_mission_cents"]
    if limit > 0 and total > limit:
        return _invalid(
            "`reward` times `takers` is more than %d cents. That is the greatest sum for one sponsor mission "
            "on this server." % limit
        )
    return None


def mission_ranges(definition: Dict[str, Any]) -> Optional[MissionRefusal]:
    """Checks what the missions module checks when the mission reaches the world.

    These are the ranges of the numbers, and the fields that only a team
    mission can have. Returns the refusal, or `None`.
    """

    def out_of_range(name: str, value: int, least: int, greatest: int, unit: str) -> Optional[MissionRefusal]:
        if least <= value <= greatest:
            return None
        return MissionRefusal(400, "invalid_args", "`%s` must be from %d to %d%s." % (name, least, greatest, unit))

    team = definition.get("mode") == "team"
    if definition.get("org") is not None and not team:
        return MissionRefusal(400, "invalid_args", "Only a team mission can name an organization.")
    for objective in definition["objectives"]:
        refusal = out_of_range("count", objective.get("count", 1), 1, SIZES["count_max"], "")
        if refusal is not None:
            return refusal
        if objective.get("actor") == "team" and not team:
            return MissionRefusal(400, "invalid_args", "Only a team mission can count the events of team members.")
    return (
        out_of_range("reward", definition["reward"], SIZES["reward_min"], SIZES["reward_max"], " cents")
        or out_of_range("takers", definition.get("takers", 1), 1, SIZES["takers_max"], "")
        or out_of_range(
            "deadline_in", definition["deadline_in"], SIZES["deadline_min"], SIZES["deadline_max"], " game minutes"
        )
    )
