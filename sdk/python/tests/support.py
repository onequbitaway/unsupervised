"""Shared set-up of the client tests."""

import unittest
from typing import Any, List, Optional

import _path  # noqa: F401

from unsupervised import Client
from unsupervised.testing import FakeGateway, FakeResident, FakeServer, FakeWorld


class ServerCase(unittest.TestCase):
    """A test case with a fake server, one owner and one resident named Ada.

    `self.client` has her key and the owner token. It does not sleep between
    attempts: `self.sleeps` records each delay. A subclass sets
    `world_options` to change the world.
    """

    world_options: dict = {}
    client_options: dict = {}

    def setUp(self) -> None:
        self.world = FakeWorld(**self.world_options)
        self.gateway = FakeGateway(self.world)
        self.server = FakeServer(self.gateway)
        self.addCleanup(self.server.close)
        self.owner = self.world.sign_up()
        self.resident: FakeResident = self.world.spawn(self.owner["owner"]["id"], "Ada")
        self.sleeps: List[float] = self.server.sleeps
        self.client: Client = self.server.client(self.resident.key, self.owner["token"], **self.client_options)

    def last_request(self, path: Optional[str] = None) -> Any:
        """Returns the newest request that the gateway got, for one path when it is given."""
        requests = self.gateway.requests if path is None else self.gateway.requests_to(path)
        return requests[-1]
