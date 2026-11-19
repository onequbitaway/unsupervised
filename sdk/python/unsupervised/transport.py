"""The HTTPS transport: one JSON request with bearer authentication, typed errors and retry."""

import http.client
import json
import random
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Dict, Mapping, Optional, Protocol, Union

from .errors import HTTP_ERROR, ApiError, ProtocolError, TransportError

DEFAULT_MAX_RETRIES = 3
DEFAULT_TIMEOUT_SECONDS = 30.0
BACKOFF_BASE_SECONDS = 0.25
BACKOFF_MAX_SECONDS = 8.0

#: Statuses that mean: the server refused the request for load and did nothing.
REFUSED_STATUSES = (429, 503)
#: Statuses that mean: the answer was lost on the way.
LOST_STATUSES = (408, 502, 504)

QueryValue = Union[str, int, None]


class Opener(Protocol):
    """What sends the requests of a transport: an object with the `open` method of `urllib`.

    `urllib.request.build_opener()` makes one. The answer has `read()` and
    works in a `with` statement. An error status raises
    `urllib.error.HTTPError`.
    """

    def open(self, __request: urllib.request.Request, *, timeout: float) -> Any:
        """Sends one request and returns the answer."""


def default_retry_delay(attempt: int, retry_after: Optional[float]) -> float:
    """Returns the seconds to wait before attempt `attempt` (from 1).

    The `Retry-After` header wins when the answer had one. Otherwise the
    delay starts at 0.25 seconds, doubles for each attempt up to 8 seconds,
    and has a random part.
    """
    if retry_after is not None:
        return retry_after
    ceiling = min(BACKOFF_MAX_SECONDS, BACKOFF_BASE_SECONDS * 2 ** (attempt - 1))
    return ceiling / 2 + random.random() * (ceiling / 2)


def _retry_after(headers: Mapping[str, str]) -> Optional[float]:
    """Reads the `Retry-After` header as a number of seconds."""
    value = headers.get("Retry-After")
    if value is None:
        return None
    try:
        seconds = float(value)
    except ValueError:
        return None
    return seconds if seconds >= 0 else None


def _api_error(status: int, headers: Mapping[str, str], text: str) -> ApiError:
    """Makes the error of an answer whose status is not a success."""
    retry_after = _retry_after(headers)
    try:
        parsed = json.loads(text)
    except ValueError:
        parsed = None
    error = parsed.get("error") if isinstance(parsed, dict) else None
    if isinstance(error, dict) and isinstance(error.get("code"), str) and isinstance(error.get("message"), str):
        return ApiError(error["code"], error["message"], status, error.get("details"), retry_after)
    # The body is not the documented one: a proxy or a framework wrote it.
    summary = text.strip()[:200]
    message = "HTTP %d: %s" % (status, summary) if summary else "HTTP %d" % status
    return ApiError(HTTP_ERROR, message, status, None, retry_after)


class Transport:
    """Sends JSON requests to one server."""

    def __init__(
        self,
        base_url: str,
        *,
        max_retries: int = DEFAULT_MAX_RETRIES,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
        retry_delay: Callable[[int, Optional[float]], float] = default_retry_delay,
        sleep: Callable[[float], None] = time.sleep,
        opener: Optional[Opener] = None,
    ) -> None:
        """Makes a transport.

        `max_retries` is the greatest number of new attempts after the first
        one. `timeout` is the seconds before one attempt fails without an
        answer. `retry_delay(attempt, retry_after)` gives the seconds to wait
        before a new attempt. `sleep` waits. `opener` sends the requests; the
        default is the opener of `urllib`.
        """
        self.base_url = base_url.rstrip("/")
        self._max_retries = max_retries
        self._timeout = timeout
        self._retry_delay = retry_delay
        self._sleep = sleep
        self._opener = opener

    def url(self, path: str, query: Optional[Mapping[str, QueryValue]] = None) -> str:
        """Returns the full address of a path with its query. A `None` value is left out."""
        pairs = [(name, str(value)) for name, value in (query or {}).items() if value is not None]
        suffix = "?" + urllib.parse.urlencode(pairs) if pairs else ""
        return self.base_url + path + suffix

    def request(
        self,
        method: str,
        path: str,
        *,
        idempotent: bool,
        query: Optional[Mapping[str, QueryValue]] = None,
        body: Any = None,
        token: Optional[str] = None,
        extra_timeout: float = 0.0,
        text: bool = False,
        retry: bool = True,
    ) -> Any:
        """Sends a request and returns the parsed JSON answer.

        `idempotent` says that a repeat of the request cannot change the
        world twice. Such a request is sent again after a network failure. A
        request that is not idempotent is sent again only when the server
        refused it for load reasons (HTTP 429 or 503). With `retry=False`
        the request goes out one time: use it for a request that must not
        act twice and whose `Retry-After` can be hours. `extra_timeout` adds
        seconds to the timeout, for a long poll. With `text`, the body comes
        back as a string.

        Raises `ApiError` when the server answers with an error,
        `TransportError` when no answer comes and `ProtocolError` when the
        answer is not JSON.
        """
        url = self.url(path, query)
        headers: Dict[str, str] = {"Accept": "text/plain, text/markdown" if text else "application/json"}
        if token is not None:
            headers["Authorization"] = "Bearer " + token
        data: Optional[bytes] = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            # The same bytes go out on every attempt, so the request ids repeat.
            data = json.dumps(body, separators=(",", ":")).encode("utf-8")
        timeout = self._timeout + extra_timeout

        attempt = 0
        while True:
            last = attempt >= self._max_retries or not retry
            request = urllib.request.Request(url, data=data, headers=headers, method=method)
            try:
                opened = self._opener.open(request, timeout=timeout) if self._opener else urllib.request.urlopen(request, timeout=timeout)
                with opened as response:
                    payload = response.read().decode("utf-8")
            except urllib.error.HTTPError as answer:
                error = _api_error(answer.code, answer.headers, answer.read().decode("utf-8", "replace"))
                answer.close()
                retry = error.status in REFUSED_STATUSES or (error.status in LOST_STATUSES and idempotent)
                if last or not retry:
                    raise error from None
                attempt += 1
                self._sleep(self._retry_delay(attempt, error.retry_after))
                continue
            except (OSError, http.client.HTTPException) as cause:
                if last or not idempotent:
                    raise TransportError("%s %s got no answer" % (method, path)) from cause
                attempt += 1
                self._sleep(self._retry_delay(attempt, None))
                continue
            if text:
                return payload
            if payload == "":
                return None
            try:
                return json.loads(payload)
            except ValueError as cause:
                raise ProtocolError("%s %s did not answer with JSON" % (method, path)) from cause
