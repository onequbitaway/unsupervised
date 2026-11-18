"""Test support: a small model of the server.

    from unsupervised.testing import FakeServer

    with FakeServer() as server:
        client = server.client()
        client.sign_up()
        client.create_resident("Ada")
        client.act("wait", {"ticks": 5})
        server.world.step(5)

A test that makes many requests can leave out the network:

    from unsupervised.testing import FakeGateway, direct_client

    gateway = FakeGateway()
    client = direct_client(gateway)

The model follows `docs/design/protocol.md`. It is not the game.
"""

from .direct import DIRECT_URL, DirectOpener, direct_client
from .gateway import FakeGateway, LoggedRequest, Response
from .server import FakeServer
from .verbs import default_verbs
from .world import (
    DEFAULT_LIMITS,
    Busy,
    Done,
    FakeActionError,
    FakeResident,
    FakeVerb,
    FakeWorld,
    Outcome,
    Pending,
    byte_length,
    contains,
    distance,
    nearest,
    verb_info,
)

__all__ = [
    "DEFAULT_LIMITS",
    "DIRECT_URL",
    "Busy",
    "DirectOpener",
    "Done",
    "FakeActionError",
    "FakeGateway",
    "FakeResident",
    "FakeServer",
    "FakeVerb",
    "FakeWorld",
    "LoggedRequest",
    "Outcome",
    "Pending",
    "Response",
    "byte_length",
    "contains",
    "default_verbs",
    "direct_client",
    "distance",
    "nearest",
    "verb_info",
]
