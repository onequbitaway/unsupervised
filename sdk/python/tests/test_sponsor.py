"""Missions of a sponsor: `post_mission` with an owner token."""

import unittest

import _path  # noqa: F401
from support import ServerCase

from unsupervised import ApiError, MissingCredentialError, TransportError

BREAD = {
    "title": "Bread for the fair",
    "text": "The town needs bread.",
    "objectives": [{"event": "economy.goods_produced", "fields": {"item": "bread"}, "count": 10}],
    "reward": 5000,
    "deadline_in": 2880,
    "takers": 2,
}


class SponsorTest(ServerCase):
    def refusal(self, mission: dict, client=None) -> ApiError:
        """Returns the error of a mission that the server must refuse."""
        with self.assertRaises(ApiError) as raised:
            (client or self.client).post_mission(mission)
        return raised.exception

    def missions_sent(self) -> int:
        return len(self.gateway.requests_to("/v1/missions"))

    def test_an_owner_publishes_a_mission_with_its_token_and_the_answer_names_the_mission(self) -> None:
        self.world.step(7)
        posted = self.client.post_mission(BREAD)
        expected = {"mission": posted["mission"], "request": 1, "frame": 7, "deadline": 7 + 2880, "reward": 5000}
        self.assertEqual(posted, dict(expected, takers=2))
        self.assertEqual([(one["mission"], one["owner"]) for one in self.world.missions], [(posted["mission"], 1)])

        sent = self.last_request()
        self.assertEqual((sent.method, sent.path, sent.query), ("POST", "/v1/missions", {}))
        self.assertEqual(sent.token, self.owner["token"])
        self.assertEqual(sent.body, BREAD)

        # A mission without the optional fields: one taker.
        small = {"title": "One loaf", "objectives": [{"event": "economy.goods_produced"}], "reward": 100}
        answer = self.client.post_mission(dict(small, deadline_in=10))
        self.assertEqual((answer["request"], answer["takers"], answer["deadline"]), (2, 1, 17))

    def test_a_mission_needs_an_owner_token(self) -> None:
        before = len(self.gateway.requests)
        with self.assertRaises(MissingCredentialError):
            self.server.client(self.resident.key).post_mission(BREAD)
        self.assertEqual(len(self.gateway.requests), before)

        wrong = self.refusal(BREAD, self.server.client(owner_token=self.resident.key))
        self.assertEqual((wrong.code, wrong.status), ("wrong_token_kind", 403))
        self.assertEqual(self.world.missions, [])

    def test_the_limit_of_one_owner_answers_with_retry_after_and_the_client_does_not_wait_for_it(self) -> None:
        clock = [1000.0]
        self.gateway.now = lambda: clock[0]
        for _ in range(3):
            self.client.post_mission(BREAD)
            clock[0] += 60
        before = self.missions_sent()
        limited = self.refusal(BREAD)
        self.assertEqual((limited.code, limited.status, limited.is_transient), ("rate_limited", 429, True))
        # The first mission counts for 24 hours. Three minutes of them are over.
        self.assertEqual(limited.retry_after, 86400 - 180)
        # One request went out: the client did not sleep for a day and did not try again.
        self.assertEqual(self.missions_sent(), before + 1)
        self.assertEqual(self.sleeps, [])

        # The limit counts for each owner, and a mission is free again after its 24 hours.
        other = self.server.client()
        other.sign_up("Bob")
        self.assertEqual(other.post_mission(BREAD)["request"], 4)
        clock[0] = 1000.0 + 86400
        self.assertEqual(self.client.post_mission(BREAD)["request"], 5)

    def test_the_server_refuses_a_mission_with_the_code_of_the_check_that_failed(self) -> None:
        cases = [
            ({"sponsor": "Acme"}, "invalid_request", 400),
            ({"title": " "}, "invalid_request", 400),
            ({"objectives": []}, "invalid_request", 400),
            # 500,001 cents of new money: more than the server gives to one mission.
            ({"reward": 166667, "takers": 3}, "invalid_request", 400),
            ({"title": "read www.example.com"}, "text_refused", 422),
            ({"objectives": [{"event": "see http://example.com"}]}, "text_refused", 422),
            # The missions module refuses these: the numbers are out of their ranges.
            ({"reward": 99}, "invalid_args", 400),
            ({"deadline_in": 9}, "invalid_args", 400),
            ({"takers": 51, "reward": 100}, "invalid_args", 400),
            ({"org": 4}, "invalid_args", 400),
        ]
        for change, code, status in cases:
            error = self.refusal(dict(BREAD, **change))
            self.assertEqual((error.code, error.status), (code, status), change)
        self.gateway.sponsors["enabled"] = False
        off = self.refusal(BREAD)
        self.assertEqual((off.code, off.status, off.ends_credential), ("missions_disabled", 403, False))
        self.assertEqual(self.world.missions, [])

        # A mission that was refused does not count for the limit of the owner.
        self.gateway.sponsors.update(enabled=True, missions_per_day=1)
        self.assertEqual(self.client.post_mission(BREAD)["request"], 1)
        self.assertEqual(self.refusal(BREAD).code, "rate_limited")

    def test_a_mission_without_an_answer_is_not_sent_again(self) -> None:
        # The world took the mission and did not answer in time.
        self.gateway.fail_next(503, "answer_timeout", "Look in `/v1/feed` for the request 41.")
        late = self.refusal(BREAD)
        self.assertEqual((late.code, late.status), ("answer_timeout", 503))
        self.assertIn("request 41", late.message)
        # A refusal for load is not repeated either: only the caller knows when to send a mission again.
        self.gateway.fail_next(503, "overloaded")
        self.assertEqual(self.refusal(BREAD).code, "overloaded")
        self.assertEqual(self.missions_sent(), 2)

        # The answer is lost on the way: the server published the mission.
        self.server.lose_responses(1)
        with self.assertRaises(TransportError):
            self.client.post_mission(BREAD)
        self.assertEqual(len(self.world.missions), 1)
        self.assertEqual(self.missions_sent(), 3)
        self.assertEqual(self.sleeps, [])


if __name__ == "__main__":
    unittest.main()
