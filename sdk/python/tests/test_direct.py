"""The client without a network: `DirectOpener` gives each request to the fake gateway."""

import socket
import unittest
from unittest import mock

import _path  # noqa: F401

from unsupervised import ApiError, Client, reports_of
from unsupervised.testing import DIRECT_URL, DirectOpener, FakeGateway, FakeWorld, direct_client


class DirectTest(unittest.TestCase):
    def setUp(self) -> None:
        self.world = FakeWorld(time="auto")
        self.gateway = FakeGateway(self.world)
        # A test of this class that opens a socket fails.
        patcher = mock.patch.object(socket.socket, "connect", side_effect=AssertionError("the client opened a socket"))
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_a_client_joins_acts_and_waits_without_a_socket(self) -> None:
        joined = Client.join(DIRECT_URL, "Ada", model="scripted", opener=DirectOpener(self.gateway))
        client = joined.client
        self.assertEqual(client.me(), {"resident": joined.resident, "name": "Ada"})
        ack, observation = client.act("move_to", {"x": 34, "y": 32})
        self.assertEqual(ack["status"], {"state": "started", "ends_at": 2, "ticks": 2})
        self.assertEqual(observation["resident"], joined.resident)
        news = client.wait(120, kinds=["report"])
        self.assertEqual([(report["verb"], report["ok"]) for report in reports_of(news)], [("move_to", True)])
        # The gateway saw the same requests that a server sees: the token, the query and the body.
        sent = self.gateway.requests_to("/v1/act")[0]
        self.assertEqual(sent.token, joined.resident_key)
        self.assertEqual((sent.body["verb"], sent.body["args"]), ("move_to", {"x": 34, "y": 32}))
        self.assertEqual(self.gateway.requests_to("/v1/wait")[0].query, {"timeout": "60", "since": "1", "kinds": "report"})

    def test_an_error_answer_has_its_code_its_status_and_its_headers(self) -> None:
        client = direct_client(self.gateway, "wrong-key")
        with self.assertRaises(ApiError) as raised:
            client.me()
        self.assertEqual((raised.exception.code, raised.exception.status), ("unauthorized", 401))

        # A refusal for load goes out again after the time of `Retry-After`, as over a network.
        resident = self.world.spawn(self.world.sign_up()["owner"], "Ada")
        sleeps = []
        client = direct_client(self.gateway, resident.key, sleep=sleeps.append)
        self.gateway.fail_next(429, "rate_limited", headers={"Retry-After": "7"})
        self.assertEqual(client.me()["name"], "Ada")
        self.assertEqual(sleeps, [7.0])

    def test_the_text_routes_work(self) -> None:
        self.gateway.join_document = "# How to join\n"
        client = direct_client(self.gateway)
        self.assertEqual(client.join_document(), "# How to join\n")
        self.assertTrue(client.health())


if __name__ == "__main__":
    unittest.main()
