"""A way from a `Client` to the fake gateway without a network.

A test that makes many requests does not need a socket for each of them.
`DirectOpener` takes the place of the opener of `urllib`: it gives each
request to `FakeGateway.handle` in the thread of the caller.
"""

import io
import urllib.error
import urllib.parse
import urllib.request
from email.message import Message
from typing import Any, Optional

from ..client import Client
from .gateway import FakeGateway, Response

#: The address of a client that uses a `DirectOpener`. No request goes there.
DIRECT_URL = "http://fake"


class _Answer:
    """The part of an HTTP response that the transport of the SDK reads."""

    def __init__(self, answer: Response, headers: Message) -> None:
        self.status = answer.status
        self.headers = headers
        self._body = answer.body.encode("utf-8")

    def read(self) -> bytes:
        return self._body

    def __enter__(self) -> "_Answer":
        return self

    def __exit__(self, *exc: Any) -> None:
        pass


class DirectOpener:
    """Answers the requests of a `Client` from a fake gateway, in the thread of the caller.

        client = Client("http://fake", opener=DirectOpener(gateway))

    An error status raises `urllib.error.HTTPError`, as the opener of
    `urllib` does. With manual time, a long poll blocks the caller until
    another thread makes news; use automatic time or a `FakeServer` then.
    """

    def __init__(self, gateway: FakeGateway) -> None:
        self.gateway = gateway

    def open(self, request: urllib.request.Request, timeout: Optional[float] = None) -> _Answer:
        """Gives one request to the gateway and returns its answer."""
        parts = urllib.parse.urlsplit(request.full_url)
        url = parts.path + ("?" + parts.query if parts.query else "")
        data = request.data
        body = data.decode("utf-8") if isinstance(data, bytes) else ""
        # `urllib` writes header names as `Content-type`. A real server reads them without case.
        sent = Message()
        for name, value in request.header_items():
            sent[name] = value
        answer = self.gateway.handle(request.get_method(), url, sent, body)
        headers = Message()
        for name, value in answer.headers.items():
            headers[name] = value
        if answer.status >= 400:
            raise urllib.error.HTTPError(
                request.full_url, answer.status, "error", headers, io.BytesIO(answer.body.encode("utf-8"))
            )
        return _Answer(answer, headers)


def direct_client(
    gateway: FakeGateway, resident_key: Optional[str] = None, owner_token: Optional[str] = None, **options: Any
) -> Client:
    """Returns a client that talks to the gateway without a network.

    The client does not sleep between attempts, unless `sleep` is given.
    """
    options.setdefault("opener", DirectOpener(gateway))
    options.setdefault("sleep", lambda seconds: None)
    return Client(DIRECT_URL, resident_key, owner_token, **options)
