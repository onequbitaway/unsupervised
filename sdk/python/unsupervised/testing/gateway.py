"""The HTTPS routes of the fake world.

`FakeGateway.handle` answers one HTTP request. Routes: `/healthz`,
`/join.md`, `/v1/catalogue`, `/v1/world`, `/v1/world/zones/{id}`,
`/v1/owners`, `/v1/residents`, `/v1/residents/{id}/keys`, `/v1/me`,
`/v1/act`, `/v1/observe`, `/v1/wait`, `/v1/routines`, `/v1/webhook`. Every
other route answers 404.
"""

import json
import re
import threading
import urllib.parse
from typing import Any, Callable, Dict, List, Mapping, NamedTuple, Optional, Tuple

from .world import NAME_MAX, FakeResident, FakeWorld, byte_length

WAIT_MAX_SECONDS = 60
#: HTTP status of an action error on a route that is not `/v1/act`.
ACTION_ERROR_STATUS = {"blocked": 403, "rate_limited": 429}

Query = Dict[str, str]


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


def _json(status: int, body: Any, headers: Optional[Dict[str, str]] = None) -> Response:
    return Response(status, dict({"Content-Type": "application/json"}, **(headers or {})), json.dumps(body))


def _error(status: int, code: str, message: str, headers: Optional[Dict[str, str]] = None) -> Response:
    return _json(status, {"error": {"code": code, "message": message}}, headers)


def _integer(query: Query, name: str) -> Optional[int]:
    text = query.get(name)
    if text is None:
        return None
    if not text.isdigit():
        raise _HttpError(400, "invalid", "`%s` must be a whole number." % name)
    return int(text)


def _list(query: Query, name: str) -> Optional[List[str]]:
    text = query.get(name)
    return text.split(",") if text else None


def _observe_opts(query: Query) -> Dict[str, Any]:
    opts: Dict[str, Any] = {}
    detail = query.get("detail")
    if detail is not None:
        if detail not in ("brief", "full"):
            raise _HttpError(400, "invalid", "`detail` is `brief` or `full`.")
        opts["detail"] = detail
    since = _integer(query, "since")
    if since is not None:
        opts["since_notice"] = since
    sections = _list(query, "sections")
    if sections is not None:
        opts["sections"] = sections
    return opts


class FakeGateway:
    """The gateway of a fake world."""

    def __init__(self, world: Optional[FakeWorld] = None) -> None:
        #: The world behind the gateway.
        self.world = world if world is not None else FakeWorld()
        #: Every request that came in, in order.
        self.requests: List[LoggedRequest] = []
        #: The text of `GET /join.md`.
        self.join_document = "# Join\n"
        #: True: `POST /v1/act` does not read `since` and answers with the whole inbox.
        self.act_ignores_since = False
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
        authorization = headers.get("Authorization") or headers.get("authorization")
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
            if bad_body:
                raise _HttpError(400, "invalid", "The body is not JSON.")
            return self._route(request)
        except _HttpError as error:
            return _error(error.status, error.code, error.message)

    def _route(self, request: LoggedRequest) -> Response:
        world = self.world
        method, path, query, body = request.method, request.path, request.query, request.body
        route = "%s %s" % (method, path)
        keys = re.match(r"^/v1/residents/(\d+)/keys$", path)
        zone = re.match(r"^/v1/world/zones/(\d+)$", path)

        if route == "GET /healthz":
            return Response(200, {"Content-Type": "text/plain"}, "ok")
        if route == "GET /join.md":
            return Response(200, {"Content-Type": "text/markdown"}, self.join_document)
        if route == "GET /v1/catalogue":
            return _json(200, world.catalogue())
        if route == "GET /v1/world":
            return _json(200, self._layout())
        if method == "GET" and zone:
            if zone.group(1) != "0":
                raise _HttpError(404, "not_found", "No zone has the id %s." % zone.group(1))
            return _json(200, self._zone())

        if route == "POST /v1/owners":
            return _json(201, world.sign_up())
        if route == "POST /v1/residents":
            owner = self._owner(request.token)
            spec = body if isinstance(body, dict) else {}
            name = spec.get("name")
            model = spec.get("model")
            if not isinstance(name, str) or not 1 <= byte_length(name) <= NAME_MAX:
                raise _HttpError(400, "invalid", "`name` must have 1 to %d bytes." % NAME_MAX)
            if model is not None and (not isinstance(model, str) or byte_length(model) > NAME_MAX):
                raise _HttpError(400, "invalid", "`model` must have at most %d bytes." % NAME_MAX)
            if spec.get("city") not in (None, 0):
                raise _HttpError(400, "invalid", "The fake world has one city: 0.")
            resident = world.spawn(owner, name, model)
            return _json(201, {"resident": resident.id, "key": resident.key})
        if route == "GET /v1/residents":
            owner = self._owner(request.token)
            with world.lock:
                residents = []
                for resident in world.residents.values():
                    if resident.owner == owner:
                        entry: Dict[str, Any] = {"id": resident.id, "name": resident.name}
                        if resident.model is not None:
                            entry["model"] = resident.model
                        residents.append(entry)
            return _json(200, {"residents": residents})
        if method == "POST" and keys:
            owner = self._owner(request.token)
            target = world.residents.get(int(keys.group(1)))
            if target is None:
                raise _HttpError(404, "unknown_resident", "No living resident has this id.")
            if target.owner != owner:
                raise _HttpError(403, "forbidden", "The resident belongs to another owner.")
            return _json(200, {"resident": target.id, "key": world.rotate_key(target)})

        if route == "GET /v1/me":
            resident = self._resident(request.token)
            return _json(200, {"resident": resident.id, "name": resident.name})
        if route == "POST /v1/act":
            resident = self._resident(request.token)
            requests = body["plan"] if isinstance(body, dict) and isinstance(body.get("plan"), list) else [body]
            for one in requests:
                if not isinstance(one, dict) or not isinstance(one.get("verb"), str):
                    raise _HttpError(400, "invalid", "An action request needs a `verb`.")
            with world.lock:
                acks = [world.submit(resident, one) for one in requests]
                opts = {} if self.act_ignores_since else _observe_opts(query)
                return _json(200, {"acks": acks, "observation": world.observe(resident, opts)})
        if route == "GET /v1/observe":
            return _json(200, world.observe(self._resident(request.token), _observe_opts(query)))
        if route == "GET /v1/wait":
            resident = self._resident(request.token)
            timeout = _integer(query, "timeout")
            timeout = WAIT_MAX_SECONDS if timeout is None else timeout
            if timeout > WAIT_MAX_SECONDS:
                raise _HttpError(400, "invalid", "`timeout` is at most %d seconds." % WAIT_MAX_SECONDS)
            return _json(200, world.wait(resident, timeout, _integer(query, "since"), _list(query, "kinds")))
        if route == "PUT /v1/routines":
            resident = self._resident(request.token)
            routines = body.get("routines") if isinstance(body, dict) else None
            if not isinstance(routines, list):
                raise _HttpError(400, "invalid", "The body needs `routines`: a list.")
            status = world.submit(resident, {"verb": "set_routines", "args": {"routines": routines}})["status"]
            if status["state"] == "rejected":
                code = status["error"]["code"]
                raise _HttpError(ACTION_ERROR_STATUS.get(code, 400), code, status["error"]["message"])
            return _json(200, status.get("result"))
        if route == "PUT /v1/webhook":
            resident = self._resident(request.token)
            target_url = body.get("url") if isinstance(body, dict) else ""
            if target_url is not None and not (isinstance(target_url, str) and re.match(r"^https?://", target_url)):
                raise _HttpError(400, "invalid", "`url` must be an http or https address, or null.")
            secret = world.set_webhook(resident, target_url)
            return _json(200, {"url": None} if secret is None else {"url": target_url, "secret": secret})
        raise _HttpError(404, "not_found", "The fake gateway has no route %s." % route)

    def _owner(self, token: Optional[str]) -> int:
        owner = self.world.owner_of(token)
        if owner is not None:
            return owner
        if self.world.resident_of(token) is not None:
            raise _HttpError(403, "forbidden", "This route needs an owner token, not a resident key.")
        raise _HttpError(401, "unauthorized", "Send an owner token: `Authorization: Bearer <token>`.")

    def _resident(self, key: Optional[str]) -> FakeResident:
        resident = self.world.resident_of(key)
        if resident is not None:
            return resident
        if self.world.owner_of(key) is not None:
            raise _HttpError(403, "forbidden", "This route needs a resident key, not an owner token.")
        raise _HttpError(401, "unauthorized", "Send a resident key: `Authorization: Bearer <key>`.")

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

    def _zone(self) -> Dict[str, Any]:
        with self.world.lock:
            zone, bounds = self._zone_view()
            side = bounds["x1"] - bounds["x0"]
            residents = []
            for resident in self.world.residents.values():
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
