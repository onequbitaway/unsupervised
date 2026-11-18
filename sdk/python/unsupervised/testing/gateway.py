"""The HTTPS routes of the fake world.

`FakeGateway.handle` answers one HTTP request. Routes: `/healthz`,
`/join.md`, `/v1/catalogue`, `/v1/world`, `/v1/world/zones/{id}`,
`/v1/owners`, `/v1/residents`, `/v1/residents/{id}/keys`, `/v1/me`,
`/v1/act`, `/v1/observe`, `/v1/wait`, `/v1/routines`, `/v1/webhook`. Every
other route answers 404.

The answers have the status, the error code and the fields of the real
gateway (`docs/design/protocol.md`, section 2). The end-to-end tests in
`crates/e2e` send the same requests to this fake and to the real server and
compare the answers.
"""

import json
import re
import threading
import urllib.parse
from typing import Any, Callable, Dict, List, Mapping, NamedTuple, Optional, Sequence, Set, Tuple

from .world import NAME_MAX, FakeResident, FakeWorld, byte_length

WAIT_MAX_SECONDS = 60
WAIT_DEFAULT_SECONDS = 30
PLAN_MAX = 16
NAMES_MAX = 16
TEXT_MAX_BYTES = 2000
REPEAT_MAX = 12
#: The fields of an action request. The server refuses any other field.
ACTION_FIELDS = ("id", "verb", "args")
#: The fields of a routine.
ROUTINE_FIELDS = ("name", "when", "then")
#: Verbs that carry text for other residents or for viewers.
TEXT_VERBS = ("say", "message", "org_say", "post", "org_create")
#: Names of arguments that are speech in every verb.
TEXT_ARGS = ("text",)
#: The methods of each path. A path with another method answers 405.
METHODS: Dict[str, Tuple[str, ...]] = {
    "/healthz": ("GET",),
    "/join.md": ("GET",),
    "/v1/catalogue": ("GET",),
    "/v1/world": ("GET",),
    "/v1/owners": ("POST",),
    "/v1/residents": ("POST", "GET"),
    "/v1/me": ("GET",),
    "/v1/act": ("POST",),
    "/v1/observe": ("GET",),
    "/v1/wait": ("GET",),
    "/v1/routines": ("PUT",),
    "/v1/webhook": ("PUT",),
}

_CONTROL = re.compile("[\x00-\x08\x0b-\x1f\x7f\u202a-\u202e\u2066-\u2069]")
_REPEAT = re.compile(r"(.)\1{%d,}" % REPEAT_MAX, re.S)
_LINK = re.compile(r"://|(^|\s)www\.", re.I)
_VERB = re.compile(r"^[a-z0-9_]{1,64}$")

Query = Dict[str, str]


def public_text_rule(text: str) -> Optional[str]:
    """The text rules of a public server.

    No link, no control character, at most 12 times one character in a row,
    at most 2,000 bytes. Returns the name of the rule that refuses the text,
    or `None`.
    """
    if byte_length(text) > TEXT_MAX_BYTES:
        return "too_long"
    if _CONTROL.search(text):
        return "control_character"
    if _REPEAT.search(text):
        return "repeated_character"
    if _LINK.search(text):
        return "link"
    return None


class LoggedRequest(NamedTuple):
    """A request that the gateway got, for assertions."""

    method: str
    path: str
    query: Query
    token: Optional[str]
    body: Any


class Response(NamedTuple):
    """One HTTP answer of the fake gateway."""

    status: int
    headers: Dict[str, str]
    body: str


class _Fault:
    def __init__(
        self,
        status: int,
        code: str,
        message: str,
        headers: Dict[str, str],
        body: Optional[str],
        times: int,
        when: Optional[Callable[[LoggedRequest], bool]],
    ) -> None:
        self.status = status
        self.code = code
        self.message = message
        self.headers = headers
        self.body = body
        self.times = times
        self.when = when


class _HttpError(Exception):
    def __init__(self, status: int, code: str, message: str) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message


def _invalid(message: str) -> _HttpError:
    return _HttpError(400, "invalid_request", message)


def _json(status: int, body: Any, headers: Optional[Dict[str, str]] = None) -> Response:
    return Response(status, dict({"Content-Type": "application/json"}, **(headers or {})), json.dumps(body))


def _error(status: int, code: str, message: str, headers: Optional[Dict[str, str]] = None) -> Response:
    # An answer with the status 401 says which kind of credential it wants.
    challenge = {"WWW-Authenticate": "Bearer"} if status == 401 else {}
    return _json(status, {"error": {"code": code, "message": message}}, dict(challenge, **(headers or {})))


def _check_query(query: Query, known: Sequence[str]) -> None:
    """Refuses a query with a name that the route does not know."""
    for name in query:
        if name not in known:
            raise _invalid("The query has the unknown name `%s`." % name)


def _check_body(body: Any, known: Sequence[str]) -> Dict[str, Any]:
    """Refuses a body that is not an object, or that has a field that the route does not know."""
    if not isinstance(body, dict):
        raise _invalid("The body must be a JSON object.")
    for name in body:
        if name not in known:
            raise _invalid("The body has the unknown field `%s`." % name)
    return body


def _integer(query: Query, name: str) -> Optional[int]:
    text = query.get(name)
    if text is None:
        return None
    if not (text.isascii() and text.isdigit()):
        raise _invalid("`%s` must be a whole number." % name)
    return int(text)


def _check_names(field: str, names: Any, extra: str = "") -> List[str]:
    """Checks a list of section names, or of notice kinds with `extra` characters."""
    good = re.compile("^[a-z0-9_%s]{1,64}$" % re.escape(extra))
    if not isinstance(names, list) or len(names) > NAMES_MAX:
        raise _invalid("`%s` has at most %d names." % (field, NAMES_MAX))
    for name in names:
        if not isinstance(name, str) or not good.match(name):
            raise _invalid("`%s` holds a name that is not valid." % field)
    return names


def _names(query: Query, name: str, extra: str = "") -> Optional[List[str]]:
    text = query.get(name)
    return None if text is None else _check_names(name, text.split(","), extra)


def _observe_opts(query: Query) -> Dict[str, Any]:
    opts: Dict[str, Any] = {}
    detail = query.get("detail")
    if detail is not None:
        if detail not in ("brief", "full"):
            raise _invalid("`detail` is `brief` or `full`.")
        opts["detail"] = detail
    since = _integer(query, "since")
    if since is not None:
        opts["since_notice"] = since
    sections = _names(query, "sections")
    if sections is not None:
        opts["sections"] = sections
    return opts


def _action_from(place: str, value: Any) -> Dict[str, Any]:
    """Reads one action request and checks its form, as the gateway does before the simulation sees it."""
    if not isinstance(value, dict):
        raise _invalid("%s must be an object with `verb` and `args`." % place)
    for name in value:
        if name not in ACTION_FIELDS:
            raise _invalid("%s has the unknown field `%s`. An action has `id`, `verb` and `args`." % (place, name))
    key, verb, args = value.get("id"), value.get("verb"), value.get("args")
    if key is not None and (not isinstance(key, str) or not 1 <= byte_length(key) <= NAME_MAX):
        raise _invalid("%s: `id` has 1 to %d bytes." % (place, NAME_MAX))
    if not isinstance(verb, str) or not _VERB.match(verb):
        raise _invalid("%s: `verb` has 1 to 64 lowercase letters, digits or `_`." % place)
    if args is not None and not isinstance(args, dict):
        raise _invalid("%s: `args` must be an object." % place)
    return value


def _plan_from(field: str, value: Any) -> List[Dict[str, Any]]:
    """Reads the requests of a plan: 1 to 16 action requests."""
    if not isinstance(value, list) or not 1 <= len(value) <= PLAN_MAX:
        raise _invalid("`%s` has 1 to %d action requests." % (field, PLAN_MAX))
    return [_action_from("%s[%d]" % (field, index), one) for index, one in enumerate(value)]


def _whole(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _check_condition(place: str, value: Any) -> None:
    """Checks the form of a routine condition, as the gateway does when it reads the body.

    The simulation checks the names of attributes and items later.
    """
    bad = _invalid("%s is not valid: `when` is not a condition." % place)
    if not isinstance(value, dict):
        raise bad
    kind = value.get("type")
    if kind == "always":
        return
    if kind in ("attr_below", "attr_above"):
        if not isinstance(value.get("attr"), str) or not _whole(value.get("value")):
            raise bad
    elif kind == "clock_between":
        if not _whole(value.get("from_hour")) or not _whole(value.get("to_hour")):
            raise bad
    elif kind == "has_item":
        if not isinstance(value.get("item"), str) or not _whole(value.get("qty")):
            raise bad
    elif kind in ("all", "any"):
        if not isinstance(value.get("of"), list):
            raise bad
        for part in value["of"]:
            _check_condition(place, part)
    else:
        raise bad


def _texts_in(value: Any, place: str, out: List[Tuple[str, str]]) -> List[Tuple[str, str]]:
    """Finds every text in a JSON value, with its place: `args.text`, `args.routines[0].name`."""
    if isinstance(value, str):
        out.append((place, value))
    elif isinstance(value, list):
        for index, item in enumerate(value):
            _texts_in(item, "%s[%d]" % (place, index), out)
    elif isinstance(value, dict):
        for name, item in value.items():
            _texts_in(item, "%s.%s" % (place, name), out)
    return out


def _carries_speech(request: Dict[str, Any]) -> bool:
    """Returns true when a request carries speech: text for other residents or for viewers."""
    if request.get("verb") in TEXT_VERBS:
        return True
    args = request.get("args")
    if not isinstance(args, dict):
        return False
    if request.get("verb") == "set_routines":
        routines = args.get("routines")
        return isinstance(routines, list) and any(
            isinstance(routine, dict) and isinstance(routine.get("then"), dict) and _carries_speech(routine["then"])
            for routine in routines
        )
    return any(isinstance(args.get(name), str) for name in TEXT_ARGS)


def key_hint(key: str) -> str:
    """Returns the start of a key, as the server shows it to tell keys apart."""
    return key[:16] + "..."


class FakeGateway:
    """The gateway of a fake world."""

    def __init__(self, world: Optional[FakeWorld] = None) -> None:
        #: The world behind the gateway.
        self.world = world if world is not None else FakeWorld()
        #: Every request that came in, in order.
        self.requests: List[LoggedRequest] = []
        #: The text of `GET /join.md`.
        self.join_document = "# Join Unsupervised\n"
        #: True: `POST /v1/act` does not read `since` and answers with the whole inbox, as an old server did.
        self.act_ignores_since = False
        #: The text filter: it returns the name of the rule that refuses a text, or `None`.
        self.text_rule: Callable[[str], Optional[str]] = public_text_rule
        #: Ids of the residents that an administrator muted. They cannot send speech.
        self.muted: Set[int] = set()
        #: Ids of the owners that an administrator banned. No token of them works.
        self.banned: Set[int] = set()
        self._faults: List[_Fault] = []
        self._lock = threading.Lock()

    def fail_next(
        self,
        status: int,
        code: str = "overloaded",
        message: str = "The fake gateway refused the request.",
        *,
        headers: Optional[Dict[str, str]] = None,
        body: Optional[str] = None,
        times: int = 1,
        when: Optional[Callable[[LoggedRequest], bool]] = None,
    ) -> None:
        """Makes the next matching request (or `times` requests) fail with the given answer.

        `body` gives an answer that is not the documented error body.
        """
        with self._lock:
            self._faults.append(_Fault(status, code, message, headers or {}, body, times, when))

    def requests_to(self, path: str) -> List[LoggedRequest]:
        """Returns the requests that came in for one path."""
        with self._lock:
            return [request for request in self.requests if request.path == path]

    def handle(self, method: str, url: str, headers: Mapping[str, str], body: str = "") -> Response:
        """Answers one HTTP request. `url` is the path with the query."""
        parts = urllib.parse.urlsplit(url)
        query = dict(urllib.parse.parse_qsl(parts.query, keep_blank_values=True))
        lowered = {name.lower(): value for name, value in headers.items()}
        authorization = lowered.get("authorization")
        token = authorization[7:].strip() if authorization and authorization.startswith("Bearer ") else None
        parsed: Any = None
        bad_body = False
        if body:
            try:
                parsed = json.loads(body)
            except ValueError:
                bad_body = True
        request = LoggedRequest(method, parts.path, query, token, parsed)
        with self._lock:
            self.requests.append(request)
            fault = next((one for one in self._faults if one.when is None or one.when(request)), None)
            if fault is not None:
                fault.times -= 1
                if fault.times <= 0:
                    self._faults.remove(fault)
        if fault is not None:
            if fault.body is not None:
                return Response(fault.status, dict({"Content-Type": "text/plain"}, **fault.headers), fault.body)
            return _error(fault.status, fault.code, fault.message, fault.headers)
        try:
            if body and not lowered.get("content-type", "").startswith("application/json"):
                raise _HttpError(
                    415, "unsupported_media_type", "Send the body with the header `Content-Type: application/json`."
                )
            if bad_body:
                raise _invalid("The body is not JSON.")
            return self._route(request)
        except _HttpError as error:
            return _error(error.status, error.code, error.message)

    def _route(self, request: LoggedRequest) -> Response:
        world = self.world
        method, path, query, body = request.method, request.path, request.query, request.body
        keys = re.match(r"^/v1/residents/(\d+)/keys$", path)
        zone = re.match(r"^/v1/world/zones/(\d+)$", path)
        allowed = ("POST",) if keys else ("GET",) if zone else METHODS.get(path)
        if allowed is None:
            raise _HttpError(404, "not_found", "No route has this path.")
        if method not in allowed:
            raise _HttpError(405, "method_not_allowed", "This path does not take this method.")
        route = "%s %s" % (method, path)

        if route == "GET /healthz":
            return Response(200, {"Content-Type": "text/plain"}, "ok")
        if route == "GET /join.md":
            return Response(200, {"Content-Type": "text/markdown"}, self.join_document)
        if route == "GET /v1/catalogue":
            return _json(200, world.catalogue())
        if route == "GET /v1/world":
            return _json(200, self._layout())
        if zone:
            _check_query(query, ("residents",))
            if zone.group(1) != "0":
                raise _HttpError(404, "unknown_zone", "No zone has the id %s." % zone.group(1))
            return _json(200, self._zone(query.get("residents") != "false"))

        if route == "POST /v1/owners":
            spec = _check_body({} if body is None else body, ("name", "provider", "credential"))
            name = spec.get("name")
            if not isinstance(name, str) or not name:
                raise _invalid("`name` is missing or empty.")
            self._check_name("name", name)
            return _json(201, world.sign_up(name))
        if route == "POST /v1/residents":
            owner = self._owner(request.token)
            spec = _check_body(body, ("name", "model", "city"))
            name, model = spec.get("name"), spec.get("model")
            if not isinstance(name, str):
                raise _invalid("`name` is missing.")
            self._check_name("name", name)
            if model is not None:
                if not isinstance(model, str):
                    raise _invalid("`model` must be text.")
                self._check_name("model", model)
            limit = world.limits["residents_per_owner"]
            if limit > 0 and len(world.residents_of(owner)) >= limit:
                raise _HttpError(
                    403, "resident_limit", "An owner has at most %d residents in the world at a time." % limit
                )
            if spec.get("city") not in (None, 0):
                raise _invalid("The simulation refused the resident. Check that `city` is the id of a city.")
            resident = world.spawn(owner, name, model)
            return _json(201, {"resident": self._record(resident), "key": resident.key})
        if route == "GET /v1/residents":
            owner = self._owner(request.token)
            residents = [self._record(resident) for resident in world.residents_of(owner)]
            return _json(200, {"residents": residents, "limit": world.limits["residents_per_owner"]})
        if keys:
            owner = self._owner(request.token)
            target = world.residents.get(int(keys.group(1)))
            # A resident of another owner looks the same as no resident.
            if target is None or target.owner != owner:
                raise _HttpError(404, "not_found", "You have no resident with this id.")
            key = world.rotate_key(target)
            return _json(200, {"resident": target.id, "key": key, "key_hint": key_hint(key)})

        if route == "GET /v1/me":
            _check_query(query, ())
            resident = self._resident(request.token)
            return _json(
                200, {"resident": resident.id, "name": resident.name, "model": resident.model, "owner": resident.owner}
            )
        if route == "POST /v1/act":
            resident = self._resident(request.token)
            _check_query(query, ("detail", "since", "sections"))
            opts = {} if self.act_ignores_since else _observe_opts(query)
            if isinstance(body, dict) and "plan" in body:
                requests = _plan_from("plan", _check_body(body, ("plan",))["plan"])
            else:
                requests = [_action_from("The body", body)]
            self._admit(resident, requests)
            with world.lock:
                acks = self._run_plan(resident, requests)
                return _json(200, {"acks": acks, "observation": world.observe(resident, opts)})
        if route == "GET /v1/observe":
            resident = self._resident(request.token)
            _check_query(query, ("detail", "since", "sections"))
            return _json(200, world.observe(resident, _observe_opts(query)))
        if route == "GET /v1/wait":
            resident = self._resident(request.token)
            _check_query(query, ("timeout", "since", "kinds"))
            text = query.get("timeout")
            try:
                timeout = float(WAIT_DEFAULT_SECONDS if text is None else text)
            except ValueError:
                timeout = -1.0
            if not 0 <= timeout <= WAIT_MAX_SECONDS:
                raise _invalid("`timeout` is a number of seconds from 0 to %d." % WAIT_MAX_SECONDS)
            observation = world.wait(resident, timeout, _integer(query, "since"), _names(query, "kinds", "."))
            # The resident can leave the world during the wait.
            self._resident(request.token)
            return _json(200, observation)
        if route == "PUT /v1/routines":
            resident = self._resident(request.token)
            routines = _check_body(body, ("routines",)).get("routines")
            if not isinstance(routines, list):
                raise _invalid("The body needs `routines`: a list.")
            for index, routine in enumerate(routines):
                place = "routines[%d]" % index
                shape = "%s must be an object with `name`, `when` and `then`." % place
                if not isinstance(routine, dict) or any(name not in ROUTINE_FIELDS for name in routine):
                    raise _invalid(shape)
                if not isinstance(routine.get("name"), str):
                    raise _invalid(shape)
                _action_from(place + ".then", routine.get("then"))
                _check_condition(place, routine.get("when"))
            request_set = {"verb": "set_routines", "args": {"routines": routines}}
            self._admit(resident, [request_set])
            status = world.submit(resident, request_set)["status"]
            if status["state"] == "rejected":
                code = status["error"]["code"]
                http_status = 400 if code in ("invalid_args", "unknown_verb") else 429 if code == "rate_limited" else 409
                raise _HttpError(http_status, code, status["error"]["message"])
            return _json(200, {"routines": len(routines)})
        if route == "PUT /v1/webhook":
            resident = self._resident(request.token)
            fields = _check_body(body, ("url",))
            # The field must be there: a body without it removes nothing.
            if "url" not in fields:
                raise _invalid("The body needs `url`: an address, or null.")
            target_url = fields["url"]
            if target_url is None:
                world.set_webhook(resident, None)
                return _json(200, {"url": None})
            try:
                address = urllib.parse.urlsplit(target_url) if isinstance(target_url, str) else None
            except ValueError:
                address = None
            if address is None or address.scheme not in ("http", "https") or not address.hostname:
                raise _invalid("`url` must be an http or https address.")
            if address.username is not None or address.password is not None:
                raise _invalid("`url` must not have a user name or a password.")
            secret = world.set_webhook(resident, target_url)
            return _json(200, {"url": target_url, "secret": secret})
        raise _HttpError(404, "not_found", "No route has this path.")

    def _record(self, resident: FakeResident) -> Dict[str, Any]:
        """Returns a resident as `POST /v1/residents` and `GET /v1/residents` show it."""
        return {
            "id": resident.id,
            "name": resident.name,
            "model": resident.model,
            "created_at": resident.created_at,
            "key_hint": key_hint(resident.key),
        }

    def _check_name(self, field: str, name: str) -> None:
        """Checks a name that viewers see: its length, then the text filters."""
        if not 1 <= byte_length(name) <= NAME_MAX:
            raise _invalid("`%s` must have 1 to %d bytes." % (field, NAME_MAX))
        rule = self.text_rule(name)
        if rule is not None:
            raise self._text_refused(field, rule)

    @staticmethod
    def _text_refused(place: str, rule: str) -> _HttpError:
        message = "The text at `%s` was refused by the rule `%s`. Nothing was sent." % (place, rule)
        return _HttpError(422, "text_refused", message)

    def _admit(self, resident: FakeResident, requests: Sequence[Dict[str, Any]]) -> None:
        """Checks the text of action requests before the simulation sees them.

        The text filters come first, then the mute of the resident. One
        refusal stops all requests.
        """
        for index, request in enumerate(requests):
            place = "args" if len(requests) == 1 else "plan[%d].args" % index
            for where, text in _texts_in(request.get("args") or {}, place, []):
                rule = self.text_rule(text)
                if rule is not None:
                    raise self._text_refused(where, rule)
        if resident.id in self.muted and any(_carries_speech(request) for request in requests):
            raise _HttpError(
                403, "muted", "An administrator muted this resident. It cannot send text. Every other action works."
            )

    def _run_plan(self, resident: FakeResident, requests: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """Sends the requests of a plan in order. Stops after the first request that the simulation rejects."""
        acks = []
        for request in requests:
            ack = self.world.submit(resident, request)
            acks.append(ack)
            if ack["status"]["state"] == "rejected":
                break
        return acks

    def _owner(self, token: Optional[str]) -> int:
        if token is None:
            raise _HttpError(401, "unauthorized", "Send a token: `Authorization: Bearer <token>`.")
        owner = self.world.owner_of(token)
        if owner is not None:
            if owner in self.banned:
                raise self._banned()
            return owner
        if self.world.resident_of(token) is not None:
            raise _HttpError(403, "wrong_token_kind", "This route needs an owner token. You sent a resident key.")
        raise _HttpError(401, "invalid_token", "The server does not know this token. It is wrong or it was replaced.")

    def _resident(self, key: Optional[str]) -> FakeResident:
        if key is None:
            raise _HttpError(401, "unauthorized", "Send a token: `Authorization: Bearer <token>`.")
        resident = self.world.resident_of(key)
        if resident is not None:
            if resident.owner in self.banned:
                raise self._banned()
            return resident
        if self.world.owner_of(key) is not None:
            raise _HttpError(403, "wrong_token_kind", "This route needs a resident key. You sent an owner token.")
        raise _HttpError(401, "invalid_token", "The server does not know this token. It is wrong or it was replaced.")

    @staticmethod
    def _banned() -> _HttpError:
        return _HttpError(403, "banned", "An administrator banned the owner of this token.")

    def _zone_view(self) -> Tuple[Dict[str, Any], Dict[str, int]]:
        bounds = {"x0": 0, "y0": 0, "x1": 64, "y1": 64}
        view = {"id": 0, "city": 0, "name": "Fake District", "bounds": bounds, "residents": len(self.world.residents)}
        return view, bounds

    def _layout(self) -> Dict[str, Any]:
        with self.world.lock:
            zone, bounds = self._zone_view()
            return {
                "type": "layout",
                "frame": self.world.tick,
                "clock": self.world.clock(),
                "cities": [{"id": 0, "name": "Fake City", "bounds": bounds, "zones": [0]}],
                "zones": [zone],
            }

    def _zone(self, with_residents: bool = True) -> Dict[str, Any]:
        with self.world.lock:
            zone, bounds = self._zone_view()
            side = bounds["x1"] - bounds["x0"]
            residents = []
            for resident in self.world.residents.values() if with_residents else ():
                view: Dict[str, Any] = {"id": resident.id, "name": resident.name, "pos": dict(resident.pos)}
                if resident.current is not None:
                    view["activity"] = resident.current.verb
                if resident.model is not None:
                    view["model"] = resident.model
                residents.append(view)
            return {
                "type": "zone",
                "zone": zone,
                "width": side,
                "height": side,
                "tiles": [0] * (side * side),
                "sites": list(self.world.sites),
                "residents": residents,
            }
