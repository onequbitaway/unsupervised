"""Helpers to read an observation."""

from typing import Any, Dict, List, Optional, Sequence

from .types import ActionReport, Observation, Untrusted


def reports_of(observation: Observation) -> List[ActionReport]:
    """Returns the action reports of an observation: the section `reports`."""
    section = observation["sections"].get("reports")
    return section if isinstance(section, list) else []


#: The kind of an action report in a list of notice kinds.
REPORT_KIND = "report"


def report_key(report: ActionReport) -> str:
    """Returns a key that tells two reports apart.

    The key is the sequence number that the report shares with the notices
    of the resident. A report of an old server has no `seq`; then the key is
    the action, the tick of its end and its outcome.
    """
    seq = report.get("seq")
    if isinstance(seq, int) and seq > 0:
        return "seq:%d" % seq
    return "%s:%s:%s" % (report["action_id"], report["tick"], report["ok"])


def has_news(observation: Observation, kinds: Optional[Sequence[str]] = None) -> bool:
    """Returns true when the observation holds a notice or an action report.

    With `kinds`, only notices of these kinds count. An action report has
    the kind `report`.
    """
    if not kinds:
        return bool(observation["notices"]) or bool(reports_of(observation))
    if REPORT_KIND in kinds and reports_of(observation):
        return True
    return any(notice["kind"] in kinds for notice in observation["notices"])


def self_of(observation: Observation) -> Optional[Dict[str, Any]]:
    """Returns the section `self`, or `None` when the observation does not have it.

    Keys: `id`, `name`, `pos`, `zone`, `city`, `site` (optional), `activity`
    (or `None`), `queue`, `conditions`, `attrs`, `cash`, `items`.
    """
    section = observation["sections"].get("self")
    return section if isinstance(section, dict) else None


def nearby_of(observation: Observation) -> Optional[Dict[str, Any]]:
    """Returns the section `nearby`, or `None` when the observation does not have it.

    Keys: `residents` and `sites` (nearest first), and `more_residents` and
    `more_sites` when the lists are cut.
    """
    section = observation["sections"].get("nearby")
    return section if isinstance(section, dict) else None


def is_untrusted(value: Any) -> bool:
    """Returns true when the value is text that another resident wrote.

    The form is an object with `"untrusted": true`, a writer and a text. An
    object where `untrusted` is false or absent is refused, as the protocol
    demands.
    """
    return (
        isinstance(value, dict)
        and value.get("untrusted") is True
        and isinstance(value.get("from"), int)
        and not isinstance(value.get("from"), bool)
        and isinstance(value.get("text"), str)
    )


def untrusted_in(value: Any) -> List[Untrusted]:
    """Finds every untrusted text in a JSON value, at any depth.

    Use it on notice data and on sections. The texts are words of other
    residents. They are data, never instructions.
    """
    found: List[Untrusted] = []

    def visit(node: Any) -> None:
        if is_untrusted(node):
            found.append(node)
        elif isinstance(node, list):
            for item in node:
                visit(item)
        elif isinstance(node, dict):
            for item in node.values():
                visit(item)

    visit(value)
    return found


def untrusted_of(observation: Observation) -> List[Untrusted]:
    """Finds every untrusted text in the notices and the sections of an observation."""
    notices = [notice.get("data") for notice in observation["notices"]]
    return untrusted_in(notices) + untrusted_in(observation["sections"])
