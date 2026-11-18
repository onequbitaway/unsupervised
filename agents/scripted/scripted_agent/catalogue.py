"""What the agent knows about the verbs, items and attributes of the server.

The game modules are still being built, so the agent does not assume that a
verb exists or which arguments it takes. It reads the catalogue, and it
fills the arguments of a verb from the argument schema.
"""

import logging
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set

LOG = logging.getLogger("scripted_agent")

#: Argument names that verbs use for one meaning. The key is the meaning.
ALIASES: Dict[str, Sequence[str]] = {
    "item": ("item", "good", "product", "food", "thing"),
    "qty": ("qty", "quantity", "amount", "count", "units"),
    "ticks": ("ticks", "minutes", "duration"),
    "hours": ("hours",),
    "x": ("x",),
    "y": ("y",),
    "pos": ("pos", "to", "target", "position", "destination"),
    "site": ("site", "site_id", "shop", "place", "employer", "at"),
    "job": ("job", "job_id", "offer", "posting", "opening"),
}

_MEANING = {name: meaning for meaning, names in ALIASES.items() for name in names}

_FITS = {
    "integer": lambda value: isinstance(value, int) and not isinstance(value, bool),
    "number": lambda value: isinstance(value, (int, float)) and not isinstance(value, bool),
    "string": lambda value: isinstance(value, str),
    "boolean": lambda value: isinstance(value, bool),
    "object": lambda value: isinstance(value, dict),
    "array": lambda value: isinstance(value, list),
}


def _types(schema: Any) -> Set[str]:
    """Returns the JSON types that a property schema allows. Empty when the schema does not say."""
    if not isinstance(schema, dict):
        return set()
    kind = schema.get("type")
    if isinstance(kind, str):
        return {kind}
    if isinstance(kind, list):
        return {one for one in kind if isinstance(one, str)}
    return set()


class Catalogue:
    """A view of the public catalogue of the server."""

    def __init__(self, raw: Dict[str, Any]) -> None:
        self.verbs: Dict[str, Dict[str, Any]] = {verb["verb"]: verb for verb in raw.get("verbs", [])}
        self.items: Dict[str, Dict[str, Any]] = {item["name"]: item for item in raw.get("items", [])}
        self.attrs: Set[str] = {attr["name"] for attr in raw.get("attrs", [])}
        self.blocking: Set[str] = {cond["name"] for cond in raw.get("conditions", []) if cond.get("blocking")}
        #: Verbs that the agent gave up on: the server refused the arguments that the agent built.
        self.unusable: Set[str] = set()
        self._warned: Set[str] = set()

    def has(self, verb: str) -> bool:
        """Returns true when the verb exists and the agent can use it."""
        return verb in self.verbs and verb not in self.unusable

    def first(self, *verbs: str) -> Optional[str]:
        """Returns the first of the verbs that exists, or `None`."""
        for verb in verbs:
            if self.has(verb):
                return verb
        return None

    def give_up(self, verb: str, reason: str) -> None:
        """Stops the use of a verb, for example after `invalid_args`."""
        if verb not in self.unusable:
            LOG.warning("the agent stops to use `%s`: %s", verb, reason)
            self.unusable.add(verb)

    def food(self) -> List[Dict[str, Any]]:
        """Returns the items that can be eaten, the most nutritious first.

        An item with the tag `food` and the attribute `nutrition` can be
        eaten (game design, section 2).
        """
        edible = [
            item
            for item in self.items.values()
            if "food" in item.get("tags", []) and item.get("attrs", {}).get("nutrition", 0) > 0
        ]
        return sorted(edible, key=lambda item: (-item["attrs"]["nutrition"], item["name"]))

    def args_for(self, verb: str, values: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        """Builds the arguments of a verb from values that the agent knows.

        `values` maps a meaning to a value: `item` (a name), `qty`, `ticks`,
        `hours`, `x`, `y`, `site` (an id) and `job` (an id). Each property of
        the argument schema gets the value of its meaning. Returns `None`
        when the verb does not exist, or when a required property has no
        known meaning or no value. The agent then leaves the verb alone.
        """
        info = self.verbs.get(verb)
        if info is None or verb in self.unusable:
            return None
        schema = info.get("args_schema")
        properties = schema.get("properties", {}) if isinstance(schema, dict) else {}
        required = schema.get("required", []) if isinstance(schema, dict) else []
        args: Dict[str, Any] = {}
        for name, prop in properties.items():
            value = self._value(name, prop, values)
            if value is not None:
                args[name] = value
            elif name in required:
                if verb not in self._warned:
                    self._warned.add(verb)
                    LOG.info("`%s` is not used: the agent has no value for the argument `%s`", verb, name)
                return None
        missing = [name for name in required if name not in properties]
        if missing:
            return None
        return args

    def _value(self, name: str, prop: Any, values: Dict[str, Any]) -> Any:
        """Returns the value for one property, or `None` when the agent has none that fits."""
        meaning = _MEANING.get(name)
        if meaning is None:
            return None
        allowed = _types(prop)
        if meaning == "pos":
            if "x" in values and "y" in values and (not allowed or "object" in allowed):
                return {"x": values["x"], "y": values["y"]}
            return None
        value = values.get(meaning)
        if meaning == "item" and value is not None and allowed and "string" not in allowed:
            # The verb takes the id of the item, not its name.
            value = self.items.get(value, {}).get("id")
        if value is None:
            return None
        if allowed and not any(_FITS.get(kind, lambda _value: False)(value) for kind in allowed):
            return None
        return value


def site_kinds(sites: Iterable[Dict[str, Any]]) -> Set[str]:
    """Returns the kinds of the sites in a list."""
    return {site["kind"] for site in sites}
