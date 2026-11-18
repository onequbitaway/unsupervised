"""A small model of the server, for tests without the real server.

`FakeWorld` follows `docs/design/protocol.md`: accounts and keys, the action
engine (idempotency keys, the queue, free verbs, reports), routines, the
inbox with one sequence for notices and reports, observations and the long
poll. It is not the game: it has four verbs (`wait`, `cancel`,
`set_routines`, `move_to`). A test adds the verbs that it needs with
`add_verb` and the rules that it needs with `on_tick`.

Every method takes the lock of the world, so the HTTP server threads and
the test thread can use one world.
"""

import threading
from typing import Any, Callable, Dict, List, NamedTuple, Optional, Sequence, Tuple, Union

DEFAULT_LIMITS: Dict[str, int] = {
    # The defaults of the kernel parameters that the fake world uses.
    "queue_max": 8,
    "free_actions_per_tick": 4,
    "inbox_max": 32,
    "routines_max": 8,
    "sight_radius": 12,
    "nearby_max": 12,
    "wait_max": 1440,
    "routine_backoff": 30,
}

#: Verbs that work in every condition except `frozen`.
KERNEL_VERBS = ("wait", "cancel", "set_routines")
REQUEST_ID_MAX = 64
RECENT_MAX = 32
NAME_MAX = 64
ZONE_SIZE = 64
DAYS_PER_YEAR = 28
MINUTES_PER_DAY = 1440
REPORT_KIND = "report"
#: Greatest number of times that the backoff of a failed routine doubles.
BACKOFF_DOUBLINGS_MAX = 16
#: Seconds that `parked` waits for a long poll before it gives up.
PARKED_TIMEOUT_SECONDS = 10.0

Json = Any


class FakeActionError(Exception):
    """An action error that a fake verb raises to refuse or to fail."""

    def __init__(self, code: str, message: str, details: Json = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details

    def to_wire(self) -> Dict[str, Json]:
        """Returns the wire form of the error."""
        error: Dict[str, Json] = {"code": self.code, "message": self.message}
        if self.details is not None:
            error["details"] = self.details
        return error


class Done(NamedTuple):
    """The action finished at once."""

    result: Json = None


class Busy(NamedTuple):
    """The action occupies the resident for a number of ticks.

    `finish` gives the result at the end; it can raise `FakeActionError`.
    """

    ticks: int
    finish: Optional[Callable[[], Json]] = None


class Pending(NamedTuple):
    """The action waits for another party for a number of ticks. The acknowledgement is `pending`."""

    ticks: int
    finish: Optional[Callable[[], Json]] = None


Outcome = Union[Done, Busy, Pending]


class FakeVerb(NamedTuple):
    """A verb of the fake world: its catalogue entry and the function that starts it."""

    info: Dict[str, Json]
    begin: Callable[["FakeResident", Dict[str, Json], "FakeWorld"], Outcome]


def verb_info(
    verb: str,
    module: str,
    summary: str,
    *,
    free: bool = False,
    scope: str = "zone",
    properties: Optional[Dict[str, Json]] = None,
    required: Sequence[str] = (),
) -> Dict[str, Json]:
    """Returns the catalogue entry of a verb with an argument schema."""
    schema: Dict[str, Json] = {"type": "object"}
    if properties:
        schema["properties"] = properties
    if required:
        schema["required"] = list(required)
    return {"verb": verb, "module": module, "summary": summary, "free": free, "scope": scope, "args_schema": schema}


class _Running:
    def __init__(
        self,
        action_id: int,
        verb: str,
        request_id: Optional[str],
        started: int,
        ends: int,
        waiting: bool,
        finish: Optional[Callable[[], Json]],
        routine: Optional[int],
    ) -> None:
        self.action_id = action_id
        self.verb = verb
        self.request_id = request_id
        self.started = started
        self.ends = ends
        self.waiting = waiting
        self.finish = finish
        self.routine = routine


class _Queued(NamedTuple):
    action_id: int
    verb: str
    request_id: Optional[str]
    args: Dict[str, Json]


class _Slot:
    """A routine with its counters."""

    def __init__(self, routine: Dict[str, Json]) -> None:
        self.routine = routine
        self.retry_at = 0
        self.failures = 0
        self.runs = 0


class FakeResident:
    """A resident of the fake world. A test can read and change every field."""

    def __init__(self, id: int, owner: int, name: str, model: Optional[str], pos: Dict[str, int]) -> None:
        self.id = id
        self.owner = owner
        self.name = name
        self.model = model
        self.city = 0
        self.zone = 0
        self.pos = dict(pos)
        #: Cash in cents.
        self.cash = 0
        #: Carried goods: quantity by item name.
        self.items: Dict[str, int] = {}
        #: Attributes by name.
        self.attrs: Dict[str, int] = {}
        #: Conditions by name. The value is the end tick, or `None` for no end.
        self.conditions: Dict[str, Optional[int]] = {}
        #: Extra observation sections, as a game module writes them.
        self.sections: Dict[str, Json] = {}
        self.current: Optional[_Running] = None
        self.queue: List[_Queued] = []
        self.slots: List[_Slot] = []
        #: Inbox entries: `{"seq", "notice"}` or `{"seq", "report"}`.
        self.inbox: List[Dict[str, Json]] = []
        self.next_seq = 1
        #: The last accepted request ids with their acknowledgements.
        self.recent: List[Tuple[str, Dict[str, Json]]] = []
        self.key = ""
        self.webhook: Optional[Dict[str, str]] = None
        self.free_tick = -1
        self.free_used = 0
        self.routine_tick = -1

    @property
    def routines(self) -> List[Dict[str, Json]]:
        """The routines as the owner wrote them."""
        return [slot.routine for slot in self.slots]

    @property
    def idle(self) -> bool:
        """True when the resident has no running action and an empty queue."""
        return self.current is None and not self.queue


class _Waiter:
    def __init__(self, resident: FakeResident, since: int, kinds: Optional[Sequence[str]]) -> None:
        self.resident = resident
        self.since = since
        self.kinds = kinds
        self.expired = False


def byte_length(text: str) -> int:
    """Returns the length of a string in bytes."""
    return len(text.encode("utf-8"))


def distance(a: Dict[str, int], b: Dict[str, int]) -> int:
    """Distance in tiles. A diagonal step counts as one."""
    return max(abs(a["x"] - b["x"]), abs(a["y"] - b["y"]))


def contains(rect: Dict[str, int], pos: Dict[str, int]) -> bool:
    """Returns true when the position is inside the rectangle."""
    return rect["x0"] <= pos["x"] < rect["x1"] and rect["y0"] <= pos["y"] < rect["y1"]


def nearest(rect: Dict[str, int], pos: Dict[str, int]) -> Dict[str, int]:
    """Returns the tile of the rectangle that is nearest to the position."""
    return {
        "x": min(max(pos["x"], rect["x0"]), rect["x1"] - 1),
        "y": min(max(pos["y"], rect["y0"]), rect["y1"] - 1),
    }


def hour_between(hour: int, start: int, end: int) -> bool:
    """True when `hour` is from `start` (inclusive) to `end` (exclusive). The range can wrap past midnight."""
    if start < end:
        return start <= hour < end
    return start > end and (hour >= start or hour < end)


_JSON_TYPES: Dict[str, Callable[[Json], bool]] = {
    "integer": lambda value: isinstance(value, int) and not isinstance(value, bool),
    "number": lambda value: isinstance(value, (int, float)) and not isinstance(value, bool),
    "string": lambda value: isinstance(value, str),
    "boolean": lambda value: isinstance(value, bool),
    "array": lambda value: isinstance(value, list),
    "object": lambda value: isinstance(value, dict),
}


def check_args(info: Dict[str, Json], args: Json) -> Dict[str, Json]:
    """Checks arguments against the top level of the schema of a verb. Returns them as a dict."""
    given = {} if args is None else args
    name = info["verb"]
    if not isinstance(given, dict):
        raise FakeActionError("invalid_args", "The arguments of `%s` must be an object." % name)
    schema = info.get("args_schema") or {}
    for key in schema.get("required", []):
        if key not in given:
            raise FakeActionError("invalid_args", "`%s` needs the argument `%s`." % (name, key))
    for key, value in given.items():
        kind = (schema.get("properties", {}).get(key) or {}).get("type")
        fits = _JSON_TYPES.get(kind)
        if fits is not None and not fits(value):
            raise FakeActionError("invalid_args", "The argument `%s` of `%s` must be of type %s." % (key, name, kind))
    return given


class FakeWorld:
    """The fake world. See the head of this module."""

    def __init__(
        self,
        *,
        limits: Optional[Dict[str, int]] = None,
        verbs: Optional[Sequence[FakeVerb]] = None,
        attrs: Sequence[Dict[str, Json]] = (),
        items: Sequence[Dict[str, Json]] = (),
        conditions: Optional[Sequence[Dict[str, Json]]] = None,
        sites: Sequence[Dict[str, Json]] = (),
        spawn: Optional[Dict[str, int]] = None,
        time: str = "manual",
    ) -> None:
        """Makes an empty world at tick 0.

        `time` says how time passes during a long poll. `manual`: a `wait`
        call parks until there is news or `expire_waits` is called. `auto`:
        a `wait` call steps the world, one tick for each second of its
        timeout, until there is news.
        """
        from .verbs import default_verbs

        if time not in ("manual", "auto"):
            raise ValueError("`time` is `manual` or `auto`")
        self.lock = threading.RLock()
        self._changed = threading.Condition(self.lock)
        #: Local tick. One tick is one game minute. The frame number is the same.
        self.tick = 0
        self.limits = dict(DEFAULT_LIMITS, **(limits or {}))
        self.residents: Dict[int, FakeResident] = {}
        self.sites = list(sites)
        self.attrs = list(attrs)
        self.items = list(items)
        self.conditions = list(
            conditions
            if conditions is not None
            else [{"name": "frozen", "blocking": True, "summary": "An administrator stopped the resident. It cannot act."}]
        )
        self.time = time
        #: Rules of a test. Each runs once per tick, before the actions advance.
        self.on_tick: List[Callable[["FakeWorld"], None]] = []
        self._verbs: Dict[str, FakeVerb] = {}
        self._owners: Dict[str, int] = {}
        self._keys: Dict[str, int] = {}
        self._waiters: List[_Waiter] = []
        self._spawn = spawn or {"x": ZONE_SIZE // 2, "y": ZONE_SIZE // 2}
        self._ids = 0
        self._owner = 0
        self._secrets = 0
        self._closed = False
        for verb in verbs if verbs is not None else default_verbs():
            self.add_verb(verb)

    # ---- Catalogue --------------------------------------------------------

    def add_verb(self, verb: FakeVerb) -> None:
        """Adds a verb, or replaces the verb with the same name."""
        with self.lock:
            self._verbs[verb.info["verb"]] = verb

    def remove_verb(self, name: str) -> None:
        """Removes a verb. A later request for it gets `unknown_verb`."""
        with self.lock:
            self._verbs.pop(name, None)

    def catalogue(self) -> Dict[str, Json]:
        """Returns the public catalogue."""
        with self.lock:
            verbs = sorted((verb.info for verb in self._verbs.values()), key=lambda info: (info["module"], info["verb"]))
            params = [
                {"name": "kernel." + name, "value": value, "policy": False, "summary": "The kernel parameter %s." % name}
                for name, value in sorted(self.limits.items())
            ]
            return {
                "verbs": verbs,
                "params": params,
                "items": list(self.items),
                "attrs": list(self.attrs),
                "conditions": list(self.conditions),
            }

    # ---- Accounts ---------------------------------------------------------

    def sign_up(self) -> Dict[str, Json]:
        """Signs up an owner. Returns `{"owner", "token"}`."""
        with self.lock:
            self._owner += 1
            token = "owner-token-%d" % self._owner
            self._owners[token] = self._owner
            return {"owner": self._owner, "token": token}

    def owner_of(self, token: Optional[str]) -> Optional[int]:
        """Returns the owner of a token, if the token is known."""
        return None if token is None else self._owners.get(token)

    def resident_of(self, key: Optional[str]) -> Optional[FakeResident]:
        """Returns the resident of a key, if the key is known."""
        resident = None if key is None else self._keys.get(key)
        return None if resident is None else self.residents.get(resident)

    def spawn(self, owner: int, name: str, model: Optional[str] = None) -> FakeResident:
        """Creates a resident for an owner."""
        with self.lock:
            self._ids += 1
            resident = FakeResident(self._ids, owner, name, model, self._spawn)
            resident.attrs = {attr["name"]: attr["default"] for attr in self.attrs}
            self.residents[resident.id] = resident
            self.rotate_key(resident)
            return resident

    def rotate_key(self, resident: FakeResident) -> str:
        """Gives a resident a new key. The old key stops to work."""
        with self.lock:
            self._keys.pop(resident.key, None)
            self._secrets += 1
            resident.key = "resident-key-%d-%d" % (resident.id, self._secrets)
            self._keys[resident.key] = resident.id
            return resident.key

    def set_webhook(self, resident: FakeResident, url: Optional[str]) -> Optional[str]:
        """Sets or removes the webhook of a resident. Returns the new secret."""
        with self.lock:
            if url is None:
                resident.webhook = None
                return None
            self._secrets += 1
            resident.webhook = {"url": url, "secret": "webhook-secret-%d-%d" % (resident.id, self._secrets)}
            return resident.webhook["secret"]

    # ---- Actions ----------------------------------------------------------

    def submit(self, resident: FakeResident, request: Dict[str, Json]) -> Dict[str, Json]:
        """Handles one action request and returns the acknowledgement."""
        with self.lock:
            key = request.get("id")
            verb = request["verb"]
            if key is not None:
                if byte_length(key) > REQUEST_ID_MAX:
                    error = FakeActionError("invalid_args", "The request id is longer than %d bytes." % REQUEST_ID_MAX)
                    return {"action_id": 0, "verb": verb, "status": {"state": "rejected", "error": error.to_wire()}}
                for known, ack in resident.recent:
                    if known == key:
                        return ack
            self._ids += 1
            action_id = self._ids
            status = self._status(resident, verb, key, request.get("args"), action_id)
            ack = {"action_id": action_id, "verb": verb, "status": status}
            if key is not None:
                ack["request_id"] = key
                # A rejected request changed nothing, so its key can be used again.
                if status["state"] != "rejected":
                    if len(resident.recent) >= RECENT_MAX:
                        resident.recent.pop(0)
                    resident.recent.append((key, ack))
            return ack

    def _status(
        self, resident: FakeResident, name: str, request_id: Optional[str], args: Json, action_id: int
    ) -> Dict[str, Json]:
        try:
            verb = self._verbs.get(name)
            if verb is None:
                raise FakeActionError("unknown_verb", "No verb is named `%s`." % name)
            self._check_conditions(resident, name)
            checked = check_args(verb.info, args)
            if verb.info["free"]:
                if resident.free_tick != self.tick:
                    resident.free_tick = self.tick
                    resident.free_used = 0
                limit = self.limits["free_actions_per_tick"]
                if resident.free_used >= limit:
                    raise FakeActionError("rate_limited", "At most %d free actions in one tick." % limit)
                resident.free_used += 1
                outcome = verb.begin(resident, checked, self)
                if not isinstance(outcome, Done):
                    raise FakeActionError("internal", "The free verb `%s` did not finish at once." % name)
                return _done(outcome.result)
            if not resident.idle:
                limit = self.limits["queue_max"]
                if len(resident.queue) >= limit:
                    raise FakeActionError("queue_full", "The queue holds at most %d planned actions." % limit)
                resident.queue.append(_Queued(action_id, name, request_id, checked))
                return {"state": "queued", "position": len(resident.queue)}
            return self._begin(resident, verb, request_id, checked, action_id, None)
        except FakeActionError as error:
            return {"state": "rejected", "error": error.to_wire()}

    def _check_conditions(self, resident: FakeResident, verb: str) -> None:
        for name, until in resident.conditions.items():
            if until is not None and until <= self.tick:
                continue
            blocking = any(cond["name"] == name and cond["blocking"] for cond in self.conditions)
            if name == "frozen" or (blocking and verb not in KERNEL_VERBS):
                raise FakeActionError(
                    "blocked", "You cannot use `%s` while you are %s." % (verb, name), {"condition": name}
                )

    def _begin(
        self,
        resident: FakeResident,
        verb: FakeVerb,
        request_id: Optional[str],
        args: Dict[str, Json],
        action_id: int,
        routine: Optional[int],
    ) -> Dict[str, Json]:
        outcome = verb.begin(resident, args, self)
        if isinstance(outcome, Done):
            return _done(outcome.result)
        waiting = isinstance(outcome, Pending)
        ticks = max(1, outcome.ticks)
        resident.current = _Running(
            action_id, verb.info["verb"], request_id, self.tick, self.tick + ticks, waiting, outcome.finish, routine
        )
        if waiting:
            return {"state": "pending"}
        return {"state": "started", "ends_at": self.tick + ticks, "ticks": ticks}

    def cancel(self, resident: FakeResident) -> Dict[str, Json]:
        """Stops the running action and clears the queue, as the verb `cancel` does."""
        with self.lock:
            cleared = len(resident.queue)
            for queued in resident.queue:
                self._report(
                    resident,
                    queued.action_id,
                    queued.verb,
                    queued.request_id,
                    FakeActionError("interrupted", "The action was removed from the queue."),
                )
            resident.queue = []
            running = resident.current
            resident.current = None
            if running is not None:
                self._report(
                    resident,
                    running.action_id,
                    running.verb,
                    running.request_id,
                    FakeActionError("interrupted", "You cancelled the action."),
                )
            return {"cancelled": running.action_id if running else None, "cleared": cleared}

    def set_routines(self, resident: FakeResident, routines: Json) -> Dict[str, Json]:
        """Replaces the routines of a resident, as the verb `set_routines` does."""
        with self.lock:
            limit = self.limits["routines_max"]
            if not isinstance(routines, list):
                raise FakeActionError("invalid_args", "`routines` must be a list.")
            if len(routines) > limit:
                raise FakeActionError("invalid_args", "A resident has at most %d routines." % limit)
            slots = []
            for routine in routines:
                if byte_length(routine["name"]) > NAME_MAX:
                    raise FakeActionError("invalid_args", "A routine name has at most %d bytes." % NAME_MAX)
                self._check_condition(routine["when"])
                name = routine["then"]["verb"]
                verb = self._verbs.get(name)
                if verb is None:
                    raise FakeActionError("unknown_verb", "No verb is named `%s`." % name)
                if name in ("set_routines", "cancel"):
                    raise FakeActionError("invalid_args", "A routine cannot use the verb `%s`." % name)
                check_args(verb.info, routine["then"].get("args"))
                then = {key: value for key, value in routine["then"].items() if key != "id"}
                slots.append(_Slot(dict(routine, then=then)))
            resident.slots = slots
            if resident.current is not None:
                resident.current.routine = None
            return {"routines": len(slots)}

    def _check_condition(self, condition: Dict[str, Json]) -> None:
        kind = condition.get("type")
        if kind == "always":
            return
        if kind in ("attr_below", "attr_above"):
            if not any(attr["name"] == condition["attr"] for attr in self.attrs):
                raise FakeActionError("invalid_args", "No attribute is named `%s`." % condition["attr"])
        elif kind == "clock_between":
            if condition["from_hour"] > 23 or condition["to_hour"] > 24:
                raise FakeActionError("invalid_args", "`from_hour` is from 0 to 23 and `to_hour` is from 0 to 24.")
        elif kind == "has_item":
            if not any(item["name"] == condition["item"] for item in self.items):
                raise FakeActionError("invalid_args", "No item is named `%s`." % condition["item"])
        elif kind in ("all", "any"):
            for part in condition["of"]:
                self._check_condition(part)
        else:
            raise FakeActionError("invalid_args", "The condition type is not known.")

    def holds(self, resident: FakeResident, condition: Dict[str, Json]) -> bool:
        """Evaluates a routine condition for a resident."""
        kind = condition["type"]
        if kind == "always":
            return True
        if kind == "attr_below":
            return resident.attrs.get(condition["attr"], 0) < condition["value"]
        if kind == "attr_above":
            return resident.attrs.get(condition["attr"], 0) > condition["value"]
        if kind == "clock_between":
            return hour_between(self.clock()["hour"], condition["from_hour"], condition["to_hour"])
        if kind == "has_item":
            return resident.items.get(condition["item"], 0) >= condition["qty"]
        if kind == "all":
            return all(self.holds(resident, part) for part in condition["of"])
        return any(self.holds(resident, part) for part in condition["of"])

    # ---- Time -------------------------------------------------------------

    def step(self, ticks: int = 1) -> None:
        """Runs a number of ticks: rules run, actions end, queued actions start, routines fire."""
        with self.lock:
            for _ in range(ticks):
                self.tick += 1
                for rule in self.on_tick:
                    rule(self)
                for resident in sorted(self.residents.values(), key=lambda one: one.id):
                    self._advance(resident)

    def _advance(self, resident: FakeResident) -> None:
        running = resident.current
        if running is not None and running.ends <= self.tick:
            resident.current = None
            self._finish(resident, running)
        while resident.current is None and resident.queue:
            queued = resident.queue.pop(0)
            try:
                verb = self._verbs.get(queued.verb)
                if verb is None:
                    raise FakeActionError("unknown_verb", "No verb is named `%s`." % queued.verb)
                self._check_conditions(resident, queued.verb)
                status = self._begin(resident, verb, queued.request_id, queued.args, queued.action_id, None)
                # The acknowledgement was `queued`, so an action that ends at once reports too.
                if status["state"] == "done":
                    self._report(resident, queued.action_id, queued.verb, queued.request_id, status.get("result"))
            except FakeActionError as error:
                self._report(resident, queued.action_id, queued.verb, queued.request_id, error)
        self._try_routines(resident)

    def _finish(self, resident: FakeResident, running: _Running) -> None:
        result: Json
        try:
            if running.waiting and running.finish is None:
                raise FakeActionError("timeout", "The action waited too long for an answer.")
            result = running.finish() if running.finish else None
        except FakeActionError as error:
            result = error
        failed = isinstance(result, FakeActionError)
        if running.routine is not None and running.routine < len(resident.slots):
            self._note_routine(resident.slots[running.routine], not failed, instant=False)
        # A routine action that succeeds is only counted. A failure reports.
        if running.routine is None or failed:
            self._report(resident, running.action_id, running.verb, running.request_id, result)

    def _try_routines(self, resident: FakeResident) -> None:
        """Fires the first routine of an idle resident whose condition holds. At most one per tick."""
        if not resident.slots or not resident.idle or resident.routine_tick == self.tick:
            return
        if "frozen" in resident.conditions:
            return
        for index, slot in enumerate(resident.slots):
            if slot.retry_at <= self.tick and self.holds(resident, slot.routine["when"]):
                break
        else:
            return
        resident.routine_tick = self.tick
        self._ids += 1
        action_id = self._ids
        then = slot.routine["then"]
        name = then["verb"]
        try:
            verb = self._verbs.get(name)
            if verb is None:
                raise FakeActionError("unknown_verb", "No verb is named `%s`." % name)
            self._check_conditions(resident, name)
            status = self._begin(resident, verb, None, check_args(verb.info, then.get("args")), action_id, index)
        except FakeActionError as error:
            self._note_routine(slot, False, instant=True)
            self._report(resident, action_id, name, None, error)
            return
        if status["state"] == "done":
            self._note_routine(slot, True, instant=True)

    def _note_routine(self, slot: _Slot, ok: bool, instant: bool) -> None:
        backoff = max(1, self.limits["routine_backoff"])
        if ok:
            slot.failures = 0
            slot.runs += 1
            if instant:
                slot.retry_at = self.tick + backoff
        else:
            doublings = min(slot.failures, BACKOFF_DOUBLINGS_MAX)
            slot.failures += 1
            slot.retry_at = self.tick + backoff * 2**doublings

    def _report(
        self, resident: FakeResident, action_id: int, verb: str, request_id: Optional[str], result: Json
    ) -> None:
        report: Dict[str, Json] = {"action_id": action_id, "verb": verb, "tick": self.tick, "ok": True}
        if request_id is not None:
            report["request_id"] = request_id
        if isinstance(result, FakeActionError):
            report["ok"] = False
            report["error"] = result.to_wire()
        elif result is not None:
            report["result"] = result
        self._push(resident, {"report": report})

    # ---- Inbox ------------------------------------------------------------

    def notify(self, to: int, kind: str, data: Json = None, sender: Optional[int] = None) -> int:
        """Writes a notice to the inbox of a resident. Returns its sequence number."""
        with self.lock:
            resident = self.residents[to]
            notice: Dict[str, Json] = {"seq": resident.next_seq, "tick": self.tick, "kind": kind}
            if sender is not None:
                notice["from"] = sender
            if data is not None:
                notice["data"] = data
            return self._push(resident, {"notice": notice})

    def tell(self, to: int, sender: int, text: str, kind: str = "society.speech") -> int:
        """Writes a notice with text of another resident. The text is marked `untrusted`."""
        return self.notify(to, kind, {"untrusted": True, "from": sender, "text": text}, sender)

    def _push(self, resident: FakeResident, entry: Dict[str, Json]) -> int:
        seq = resident.next_seq
        entry["seq"] = seq
        resident.next_seq += 1
        resident.inbox.append(entry)
        del resident.inbox[: max(0, len(resident.inbox) - max(1, self.limits["inbox_max"]))]
        self._changed.notify_all()
        return seq

    # ---- Observation ------------------------------------------------------

    def clock(self) -> Dict[str, int]:
        """Returns the calendar time."""
        day, minute = divmod(self.tick, MINUTES_PER_DAY)
        return {
            "year": day // DAYS_PER_YEAR + 1,
            "day": day % DAYS_PER_YEAR + 1,
            "hour": minute // 60,
            "minute": minute % 60,
        }

    def observe(self, resident: FakeResident, opts: Optional[Dict[str, Json]] = None) -> Dict[str, Json]:
        """Returns the observation of a resident. It changes nothing."""
        opts = opts or {}
        with self.lock:
            since = opts.get("since_notice", 0)
            wanted = opts.get("sections")
            notices = [entry["notice"] for entry in resident.inbox if entry["seq"] >= since and "notice" in entry]
            reports = [entry["report"] for entry in resident.inbox if entry["seq"] >= since and "report" in entry]
            sections: Dict[str, Json] = {
                "self": self._self(resident, opts.get("detail") == "full"),
                "nearby": self._nearby(resident),
                "reports": reports,
            }
            sections.update(resident.sections)
            if wanted is not None:
                sections = {name: value for name, value in sections.items() if name in wanted}
            return {
                "frame": self.tick,
                "tick": self.tick,
                "clock": self.clock(),
                "resident": resident.id,
                "sections": sections,
                "notices": notices,
                "next_notice": resident.next_seq,
            }

    def site_at(self, pos: Dict[str, int]) -> Optional[Dict[str, Json]]:
        """Returns the site that holds a position, if there is one."""
        for site in self.sites:
            if contains(site["rect"], pos):
                return site
        return None

    def _self(self, resident: FakeResident, full: bool) -> Dict[str, Json]:
        out: Dict[str, Json] = {
            "id": resident.id,
            "name": resident.name,
            "pos": dict(resident.pos),
            "zone": resident.zone,
            "city": resident.city,
        }
        site = self.site_at(resident.pos)
        if site is not None:
            out["site"] = {"id": site["id"], "name": site["name"], "kind": site["kind"]}
        running = resident.current
        out["activity"] = (
            None
            if running is None
            else {
                "verb": running.verb,
                "action_id": running.action_id,
                "started": running.started,
                "ends": running.ends,
                "waiting": running.waiting,
            }
        )
        out["queue"] = len(resident.queue)
        out["conditions"] = {
            name: until for name, until in resident.conditions.items() if until is None or until > self.tick
        }
        out["attrs"] = dict(resident.attrs)
        out["cash"] = resident.cash
        out["items"] = dict(resident.items)
        if full:
            out["zone_name"] = "Fake District"
            out["owner"] = resident.owner
            out["model"] = resident.model
            out["born"] = 0
            out["queued"] = [{"verb": queued.verb, "action_id": queued.action_id} for queued in resident.queue]
            out["routines"] = [dict(slot.routine, runs=slot.runs, failures=slot.failures) for slot in resident.slots]
            out["memberships"] = []
        return out

    def _nearby(self, resident: FakeResident) -> Dict[str, Json]:
        radius = self.limits["sight_radius"]
        limit = self.limits["nearby_max"]
        seen = sorted(
            (
                other
                for other in self.residents.values()
                if other.id != resident.id and distance(other.pos, resident.pos) <= radius
            ),
            key=lambda other: (distance(other.pos, resident.pos), other.id),
        )
        residents = []
        for other in seen[:limit]:
            entry: Dict[str, Json] = {"id": other.id, "name": other.name, "pos": dict(other.pos)}
            if other.current is not None:
                entry["activity"] = other.current.verb
            residents.append(entry)
        sites = sorted(
            (
                (distance(nearest(site["rect"], resident.pos), resident.pos), site["id"], site)
                for site in self.sites
                if distance(nearest(site["rect"], resident.pos), resident.pos) <= radius
            ),
            key=lambda entry: entry[:2],
        )
        out: Dict[str, Json] = {"residents": residents}
        if len(seen) > limit:
            out["more_residents"] = len(seen) - limit
        out["sites"] = [
            {"id": site["id"], "name": site["name"], "kind": site["kind"], "pos": nearest(site["rect"], resident.pos)}
            for _, _, site in sites[:limit]
        ]
        if len(sites) > limit:
            out["more_sites"] = len(sites) - limit
        return out

    # ---- Long poll --------------------------------------------------------

    def has_news(self, resident: FakeResident, since: int, kinds: Optional[Sequence[str]] = None) -> bool:
        """Returns true when the inbox has an entry from `since` that the kinds let through."""
        for entry in resident.inbox:
            if entry["seq"] < since:
                continue
            if not kinds:
                return True
            kind = entry["notice"]["kind"] if "notice" in entry else REPORT_KIND
            if kind in kinds:
                return True
        return False

    def wait(
        self,
        resident: FakeResident,
        timeout: int,
        since: Optional[int] = None,
        kinds: Optional[Sequence[str]] = None,
    ) -> Dict[str, Json]:
        """The long poll.

        Returns an observation when the resident has news from `since`, or
        without news when the timeout ends. Without `since`, only news
        after the call counts.
        """
        with self.lock:
            start = resident.next_seq if since is None else since
            if not self.has_news(resident, start, kinds):
                if self.time == "auto":
                    for _ in range(timeout):
                        self.step(1)
                        if self.has_news(resident, start, kinds):
                            break
                else:
                    waiter = _Waiter(resident, start, kinds)
                    self._waiters.append(waiter)
                    self._changed.notify_all()
                    while not (self.has_news(resident, start, kinds) or waiter.expired or self._closed):
                        self._changed.wait()
                    self._waiters.remove(waiter)
            return self.observe(resident, {"since_notice": start})

    def parked(self, count: int = 1) -> None:
        """Blocks until `count` long polls are parked. A test calls it before it makes news."""
        with self.lock:
            if not self._changed.wait_for(lambda: len(self._waiters) >= count, PARKED_TIMEOUT_SECONDS):
                raise AssertionError("no long poll was parked")

    def expire_waits(self) -> None:
        """Ends every parked long poll, as the timeout does."""
        with self.lock:
            for waiter in self._waiters:
                waiter.expired = True
            self._changed.notify_all()

    def close(self) -> None:
        """Ends every parked long poll for good. The server calls it when it stops."""
        with self.lock:
            self._closed = True
            self._changed.notify_all()


def _done(result: Json) -> Dict[str, Json]:
    return {"state": "done"} if result is None else {"state": "done", "result": result}
