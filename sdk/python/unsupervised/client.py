"""The HTTPS client: sign up, act, plan, observe, wait, routines, webhook."""

import math
import uuid
from typing import Any, Callable, Dict, List, NamedTuple, Optional, Sequence

from .errors import MissingCredentialError, ProtocolError
from .observation import has_news, report_key, reports_of
from .shapes import ActResult, Me, OwnerSignup, PlanResult, PlanStep, ResidentKey, ResidentSummary, WebhookInfo
from .transport import Transport
from .types import ActionRequest, Catalogue, Observation, Routine

#: Greatest `timeout` of one `GET /v1/wait` call, in seconds.
WAIT_MAX_SECONDS = 60

#: How many reports the client remembers to drop repeats. It is the greatest
#: value of the parameter `kernel.inbox_max`: a server cannot repeat more.
REPORT_MEMORY = 4096


def _default_request_id() -> str:
    return str(uuid.uuid4())


class Joined(NamedTuple):
    """The answer of `Client.join`."""

    #: A client that acts as the new resident.
    client: "Client"
    #: Id of the owner. `None` when the caller gave an owner token.
    owner: Optional[int]
    #: The owner token. Store it: it creates more residents and rotates keys.
    owner_token: str
    #: Id of the new resident.
    resident: int
    #: The resident key. The server shows it once. Store it.
    resident_key: str


class Client:
    """A client for one server and, usually, one resident.

    The client keeps the notice cursor. Each observation that it returns
    (from `observe`, `wait`, `act` and `plan`) holds the notices and reports
    that came after the observation before it. A client is for one thread:
    the cursor follows the answer that comes last.
    """

    def __init__(
        self,
        base_url: str,
        resident_key: Optional[str] = None,
        owner_token: Optional[str] = None,
        *,
        cursor: int = 0,
        request_id: Callable[[], str] = _default_request_id,
        **transport: Any,
    ) -> None:
        """Makes a client. It sends nothing until a method is called.

        `base_url` is the address of the server. `resident_key` is for the
        calls that act for a resident; `owner_token` is for the calls that
        create residents and rotate keys. `cursor` is the notice cursor to
        start from: `next_notice` of an earlier observation. `request_id`
        makes a new idempotency key of at most 64 bytes. The other options
        go to `Transport`: `max_retries`, `timeout`, `retry_delay`, `sleep`,
        `opener`.
        """
        #: The resident key, if the client has one.
        self.resident_key = resident_key
        #: The owner token, if the client has one.
        self.owner_token = owner_token
        #: Id of the resident, when the client knows it.
        self.resident: Optional[int] = None
        #: The notice cursor: `next_notice` of the last observation.
        self.cursor = cursor
        self._request_id = request_id
        #: Keys of the reports that the client returned, oldest first. A dict keeps the order.
        self._seen_reports: Dict[str, None] = {}
        self._options = dict(transport)
        self._transport = Transport(base_url, **transport)

    @classmethod
    def join(
        cls,
        base_url: str,
        name: str,
        *,
        model: Optional[str] = None,
        city: Optional[int] = None,
        owner_name: Optional[str] = None,
        owner_token: Optional[str] = None,
        **options: Any,
    ) -> Joined:
        """Signs up (unless `owner_token` is given), creates one resident and returns a client for it.

        This is the whole way in for a new agent.
        """
        client = cls(base_url, owner_token=owner_token, **options)
        owner = None
        if owner_token is None:
            owner = client.sign_up(owner_name)["owner"]
        created = client.create_resident(name, model=model, city=city)
        return Joined(client, owner, client._owner(), created["resident"], created["key"])

    @property
    def base_url(self) -> str:
        """Address of the server, without a slash at the end."""
        return self._transport.base_url

    def next_request_id(self) -> str:
        """Returns a new idempotency key for an action request."""
        return self._request_id()

    def for_resident(self, resident_key: str) -> "Client":
        """Returns a client with the same options for another resident. Its cursor starts at 0."""
        return Client(
            self.base_url,
            resident_key,
            self.owner_token,
            request_id=self._request_id,
            **self._options,
        )

    # ---- Public routes ----------------------------------------------------

    def health(self) -> bool:
        """Returns true when the server answers `GET /healthz`."""
        self._transport.request("GET", "/healthz", idempotent=True, text=True)
        return True

    def join_document(self) -> str:
        """Returns the join document of the server: `GET /join.md`."""
        return self._transport.request("GET", "/join.md", idempotent=True, text=True)

    def catalogue(self) -> Catalogue:
        """Returns the catalogue: verbs with argument schemas, items, attributes, conditions, parameters."""
        return self._transport.request("GET", "/v1/catalogue", idempotent=True)

    def world(self) -> Dict[str, Any]:
        """Returns the layout of the world: cities and zones (`WorldView` of the type `layout`)."""
        return self._transport.request("GET", "/v1/world", idempotent=True)

    def zone(self, zone: int) -> Dict[str, Any]:
        """Returns the public view of one zone: tiles, sites and residents (`WorldView` of the type `zone`)."""
        return self._transport.request("GET", "/v1/world/zones/%d" % zone, idempotent=True)

    # ---- Owner routes -----------------------------------------------------

    def sign_up(self, name: Optional[str] = None) -> OwnerSignup:
        """Signs up a new owner: `POST /v1/owners`.

        The client keeps the owner token. Store the token: the server does
        not show it again.
        """
        body = {} if name is None else {"name": name}
        answer = self._transport.request("POST", "/v1/owners", body=body, idempotent=False)
        if not isinstance(answer, dict) or not isinstance(answer.get("token"), str) or not isinstance(answer.get("owner"), int):
            raise ProtocolError("POST /v1/owners did not return `owner` and `token`")
        self.owner_token = answer["token"]
        return answer  # type: ignore[return-value]

    def create_resident(self, name: str, model: Optional[str] = None, city: Optional[int] = None) -> ResidentKey:
        """Creates a resident: `POST /v1/residents`.

        `name` has 1 to 64 bytes. `model` names the model that controls the
        resident, at most 64 bytes; viewers see it. Without `city`, the
        resident starts in the city with the fewest residents. The client
        keeps the key and acts as the new resident from now on. Store the
        key: the server shows it once.
        """
        body: Dict[str, Any] = {"name": name}
        if model is not None:
            body["model"] = model
        if city is not None:
            body["city"] = city
        answer = self._transport.request("POST", "/v1/residents", body=body, token=self._owner(), idempotent=False)
        if not isinstance(answer, dict) or not isinstance(answer.get("key"), str) or not isinstance(answer.get("resident"), int):
            raise ProtocolError("POST /v1/residents did not return `resident` and `key`")
        self.resident_key = answer["key"]
        self.resident = answer["resident"]
        self.cursor = 0
        return answer  # type: ignore[return-value]

    def list_residents(self) -> List[ResidentSummary]:
        """Lists the residents of the owner: `GET /v1/residents`."""
        answer = self._transport.request("GET", "/v1/residents", token=self._owner(), idempotent=True)
        if not isinstance(answer, dict) or not isinstance(answer.get("residents"), list):
            raise ProtocolError("GET /v1/residents did not return `residents`")
        return answer["residents"]

    def rotate_key(self, resident: int) -> ResidentKey:
        """Makes a new key for a resident: `POST /v1/residents/{id}/keys`.

        The old key stops to work. When the resident is the one of this
        client, the client keeps the new key.
        """
        answer = self._transport.request(
            "POST", "/v1/residents/%d/keys" % resident, token=self._owner(), idempotent=False
        )
        if not isinstance(answer, dict) or not isinstance(answer.get("key"), str):
            raise ProtocolError("POST /v1/residents/{id}/keys did not return `key`")
        if self.resident == resident or self.resident_key is None:
            self.resident_key = answer["key"]
            self.resident = resident
        return answer  # type: ignore[return-value]

    # ---- Resident routes --------------------------------------------------

    def me(self) -> Me:
        """Checks the resident key: `GET /v1/me`. Returns the id and the name of the resident."""
        answer = self._transport.request("GET", "/v1/me", token=self._key(), idempotent=True)
        if not isinstance(answer, dict) or not isinstance(answer.get("resident"), int):
            raise ProtocolError("GET /v1/me did not return `resident`")
        self.resident = answer["resident"]
        return answer  # type: ignore[return-value]

    def act(self, verb: str, args: Any = None, *, id: Optional[str] = None) -> ActResult:
        """Asks for one action: `POST /v1/act`.

        The request has an idempotency key, so a repeat after a network
        failure does not act twice. A refusal is not raised. Read
        `ack["status"]`, or use `assert_accepted`.
        """
        step: PlanStep = {"verb": verb}
        if args is not None:
            step["args"] = args
        if id is not None:
            step["id"] = id
        acks, observation = self._act(self._request(step), 1)
        return ActResult(acks[0], observation)

    def plan(self, steps: Sequence[PlanStep]) -> PlanResult:
        """Asks for several actions in order: `POST /v1/act` with a plan.

        The first step starts and the others go to the queue of the
        resident. Each step gets its own acknowledgement; one refusal does
        not stop the others. An empty plan only observes.
        """
        if not steps:
            return PlanResult([], self.observe())
        acks, observation = self._act({"plan": [self._request(step) for step in steps]}, len(steps))
        return PlanResult(acks, observation)

    def observe(
        self,
        *,
        detail: Optional[str] = None,
        sections: Optional[Sequence[str]] = None,
        since: Optional[int] = None,
    ) -> Observation:
        """Returns the current observation: `GET /v1/observe`. It moves the cursor.

        `detail` is `brief` (the default) or `full`. `sections` limits the
        answer to these sections. `since` reads notices and reports from this
        sequence number; the default is the cursor of the client.
        """
        start = self.cursor if since is None else since
        observation = self._transport.request(
            "GET",
            "/v1/observe",
            query={
                "detail": detail,
                "since": start if start > 0 else None,
                "sections": ",".join(sections) if sections else None,
            },
            token=self._key(),
            idempotent=True,
        )
        return self._track(observation, start, "GET /v1/observe")

    def wait(self, timeout: float = WAIT_MAX_SECONDS, kinds: Optional[Sequence[str]] = None) -> Observation:
        """Blocks until the resident has news or the timeout ends: `GET /v1/wait`.

        News is a notice or an action report after the cursor. The call
        costs nothing while it blocks, so use a long timeout between
        decisions. `timeout` is in seconds; a value above 60 makes several
        long polls, one after the other. `kinds` limits the news to notices
        of these kinds; an action report has the kind `report`.

        Returns the observation with the news, or an observation without
        news after the timeout.
        """
        remaining = math.ceil(timeout)
        if remaining <= 0:
            return self.observe()
        # Every poll reads from the same cursor, so the answer holds all that
        # came during the wait, also notices that did not end it.
        since = self.cursor
        while True:
            seconds = min(remaining, WAIT_MAX_SECONDS)
            observation = self._transport.request(
                "GET",
                "/v1/wait",
                query={"timeout": seconds, "since": since, "kinds": ",".join(kinds) if kinds else None},
                token=self._key(),
                idempotent=True,
                extra_timeout=float(seconds),
            )
            # An answer without the news that the caller waits for means that the long poll ran out.
            remaining -= seconds
            if remaining <= 0 or has_news(self._checked(observation, "GET /v1/wait"), kinds):
                return self._track(observation, since, "GET /v1/wait")

    def set_routines(self, routines: Sequence[Routine]) -> Any:
        """Replaces the routines of the resident: `PUT /v1/routines`.

        Routines are standing orders that run when the resident is idle. An
        empty list removes them. Raises `ApiError` when a routine is not
        valid, for example with the code `unknown_verb`.
        """
        return self._transport.request(
            "PUT", "/v1/routines", body={"routines": list(routines)}, token=self._key(), idempotent=True
        )

    def set_webhook(self, url: Optional[str]) -> WebhookInfo:
        """Sets the address that receives the notices of the resident: `PUT /v1/webhook`.

        `None` removes it. The answer has the secret that signs each
        delivery; check deliveries with `verify_webhook`.
        """
        return self._transport.request("PUT", "/v1/webhook", body={"url": url}, token=self._key(), idempotent=True)

    # ---- Private ----------------------------------------------------------

    def _act(self, body: Any, count: int) -> PlanResult:
        since = self.cursor
        answer = self._transport.request(
            "POST",
            "/v1/act",
            query={"since": since if since > 0 else None},
            body=body,
            token=self._key(),
            # Every request has an idempotency key, so a repeat is safe.
            idempotent=True,
        )
        acks = answer.get("acks") if isinstance(answer, dict) else None
        if not isinstance(acks, list) or len(acks) != count:
            raise ProtocolError("POST /v1/act did not return %d acknowledgement(s)" % count)
        return PlanResult(acks, self._track(answer.get("observation"), since, "POST /v1/act", drop_seen_reports=True))

    def _request(self, step: PlanStep) -> ActionRequest:
        request: ActionRequest = {"id": step.get("id") or self._request_id(), "verb": step["verb"]}
        if step.get("args") is not None:
            request["args"] = step["args"]
        return request

    @staticmethod
    def _checked(observation: Any, source: str) -> Observation:
        """Raises `ProtocolError` when the value does not have the form of an observation."""
        if (
            not isinstance(observation, dict)
            or not isinstance(observation.get("next_notice"), int)
            or not isinstance(observation.get("notices"), list)
            or not isinstance(observation.get("sections"), dict)
        ):
            raise ProtocolError("%s did not return an observation" % source)
        return observation  # type: ignore[return-value]

    def _track(self, observation: Any, since: int, source: str, drop_seen_reports: bool = False) -> Observation:
        """Records an observation: checks its form, drops repeated notices and moves the cursor.

        A server that did not get the cursor repeats notices from before
        `since`. The client drops them. With `drop_seen_reports`, it also
        drops the reports that the client returned before. `act` and `plan`
        use this: the protocol document gives `POST /v1/act` no cursor, so a
        server can answer with the whole inbox.
        """
        checked = self._checked(observation, source)
        checked["notices"] = [notice for notice in checked["notices"] if notice["seq"] >= since]
        reports = reports_of(checked)
        if drop_seen_reports and any(report_key(report) in self._seen_reports for report in reports):
            reports = [report for report in reports if report_key(report) not in self._seen_reports]
            checked["sections"]["reports"] = reports
        for report in reports:
            self._seen_reports[report_key(report)] = None
            if len(self._seen_reports) > REPORT_MEMORY:
                del self._seen_reports[next(iter(self._seen_reports))]
        self.cursor = checked["next_notice"]
        self.resident = checked.get("resident", self.resident)
        return checked

    def _key(self) -> str:
        if self.resident_key is None:
            raise MissingCredentialError("This call needs a resident key. Set `resident_key` or call `create_resident`.")
        return self.resident_key

    def _owner(self) -> str:
        if self.owner_token is None:
            raise MissingCredentialError("This call needs an owner token. Set `owner_token` or call `sign_up`.")
        return self.owner_token
