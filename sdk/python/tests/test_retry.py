"""Retry: which requests go out again, how long the client waits, and that a repeat does not act twice."""

import unittest

import _path  # noqa: F401
from support import ServerCase

from unsupervised import ApiError, TransportError, default_retry_delay


class RetryTest(ServerCase):
    def test_a_request_that_the_server_refused_for_load_goes_out_again(self) -> None:
        self.gateway.fail_next(503, "overloaded", times=2)
        self.assertEqual(self.client.observe()["resident"], 1)
        self.assertEqual(len(self.gateway.requests), 3)
        # The test delay is the attempt number in seconds.
        self.assertEqual(self.sleeps, [1, 2])

    def test_the_client_waits_as_long_as_retry_after_says(self) -> None:
        self.gateway.fail_next(429, "rate_limited", headers={"Retry-After": "7"})
        self.client.observe()
        self.assertEqual(len(self.gateway.requests), 2)
        self.assertEqual(self.sleeps, [7.0])

    def test_an_error_that_a_repeat_cannot_cure_is_raised_at_once(self) -> None:
        self.gateway.fail_next(400, "invalid", times=5)
        with self.assertRaises(ApiError) as raised:
            self.client.observe()
        self.assertEqual(raised.exception.code, "invalid")
        self.assertEqual(len(self.gateway.requests), 1)
        self.assertEqual(self.sleeps, [])

    def test_an_action_whose_answer_was_lost_is_sent_again_with_the_same_id_and_runs_once(self) -> None:
        self.server.lose_responses(1)
        ack = self.client.act("move_to", {"x": 40, "y": 32}).ack

        sent = self.gateway.requests_to("/v1/act")
        self.assertEqual(len(sent), 2)
        self.assertEqual(sent[0].body["id"], sent[1].body["id"])
        self.assertEqual(ack["request_id"], sent[0].body["id"])
        self.assertEqual(self.sleeps, [1])

        # The server acted on the first request. The repeat got the first answer.
        self.assertEqual(ack["status"], {"state": "started", "ends_at": 8, "ticks": 8})
        self.assertEqual(self.resident.current.action_id, ack["action_id"])
        self.assertEqual(self.resident.queue, [])

    def test_a_plan_whose_answer_was_lost_does_not_queue_its_steps_twice(self) -> None:
        self.server.lose_responses(1)
        acks = self.client.plan([{"verb": "wait", "args": {"ticks": 5}}, {"verb": "wait", "args": {"ticks": 5}}]).acks
        self.assertEqual([ack["status"]["state"] for ack in acks], ["started", "queued"])
        self.assertEqual(len(self.resident.queue), 1)

    def test_a_sign_up_without_an_answer_is_not_sent_again_because_it_has_no_idempotency_key(self) -> None:
        client = self.server.client()
        self.server.lose_responses(1)
        with self.assertRaises(TransportError):
            client.sign_up("Grace")
        self.assertEqual(len(self.gateway.requests_to("/v1/owners")), 1)
        self.assertEqual(self.sleeps, [])

        # A refusal for load is safe to repeat: the server did nothing.
        self.gateway.fail_next(503, "overloaded")
        signup = client.sign_up("Grace")
        self.assertEqual(len(self.gateway.requests_to("/v1/owners")), 3)
        # The lost sign-up made owner 2 on the server; this one is owner 3.
        self.assertEqual(signup["owner"]["id"], 3)
        self.assertEqual(self.world.owner_of(signup["token"]), 3)

    def test_the_default_delay_doubles_stays_below_its_ceiling_and_follows_retry_after(self) -> None:
        for attempt in range(1, 13):
            ceiling = min(8.0, 0.25 * 2 ** (attempt - 1))
            for _ in range(20):
                delay = default_retry_delay(attempt, None)
                self.assertTrue(ceiling / 2 <= delay <= ceiling, "attempt %d: %s" % (attempt, delay))
        self.assertEqual(default_retry_delay(1, 3.0), 3.0)
        self.assertEqual(default_retry_delay(9, 0.0), 0.0)


class FewRetriesTest(ServerCase):
    client_options = {"max_retries": 2}

    def test_the_client_gives_up_after_max_retries_new_attempts_and_raises_the_last_error(self) -> None:
        self.gateway.fail_next(503, "overloaded", "The simulation is overloaded.", times=10)
        with self.assertRaises(ApiError) as raised:
            self.client.observe()
        error = raised.exception
        self.assertEqual(
            (error.code, error.status, error.is_sim_error, error.is_transient), ("overloaded", 503, True, True)
        )
        self.assertEqual(len(self.gateway.requests), 3)
        self.assertEqual(self.sleeps, [1, 2])

    def test_a_read_without_an_answer_goes_out_again_and_fails_when_no_attempt_gets_through(self) -> None:
        self.server.refuse_requests(1)
        self.assertEqual(self.client.me()["name"], "Ada")
        self.assertEqual(self.sleeps, [1])

        self.server.refuse_requests(3)
        with self.assertRaises(TransportError) as raised:
            self.client.me()
        self.assertEqual(str(raised.exception), "GET /v1/me got no answer")
        # The server cut the three attempts before the gateway saw them.
        self.assertEqual(len(self.gateway.requests_to("/v1/me")), 1)
        self.assertEqual(self.sleeps, [1, 1, 2])


if __name__ == "__main__":
    unittest.main()
