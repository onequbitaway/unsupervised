"""The client against the fake server: sign-up, actions, the notice cursor, the long poll, routines, the webhook."""

import unittest
from concurrent.futures import ThreadPoolExecutor

import _path  # noqa: F401
from support import ServerCase

from unsupervised import (
    ActionRejectedError,
    ApiError,
    Client,
    MissingCredentialError,
    assert_accepted,
    has_news,
    rejection,
    reports_of,
    self_of,
    sign_webhook,
    verify_webhook,
)
from unsupervised.testing import FakeServer

RESULT_TIMEOUT_SECONDS = 10


class SignUpTest(unittest.TestCase):
    def setUp(self) -> None:
        self.server = FakeServer()
        self.addCleanup(self.server.close)

    def test_a_new_agent_signs_up_creates_a_resident_and_checks_its_key(self) -> None:
        client = self.server.client()
        self.assertEqual(client.sign_up("Grace"), {"owner": 1, "token": "owner-token-1"})
        self.assertEqual(client.owner_token, "owner-token-1")

        created = client.create_resident("Ada", model="scripted")
        self.assertEqual(created["resident"], 1)
        self.assertEqual(client.resident_key, created["key"])
        self.assertEqual(client.me(), {"resident": 1, "name": "Ada"})
        self.assertEqual(client.list_residents(), [{"id": 1, "name": "Ada", "model": "scripted"}])

        # The server has the name and the model that the client sent.
        self.assertEqual(self.server.world.residents[1].model, "scripted")
        sent = self.server.gateway.requests_to("/v1/residents")[0]
        self.assertEqual(sent.body, {"name": "Ada", "model": "scripted"})
        self.assertEqual(sent.token, "owner-token-1")

    def test_join_does_the_whole_sign_up_in_one_call(self) -> None:
        options = {"opener": self.server.opener}
        joined = Client.join(self.server.url, "Ada", owner_name="Grace", **options)
        self.assertEqual((joined.owner, joined.owner_token, joined.resident), (1, "owner-token-1", 1))
        self.assertEqual(joined.resident_key, self.server.world.residents[1].key)
        self.assertEqual(joined.client.me()["name"], "Ada")

        # With an owner token, no new owner signs up.
        second = Client.join(self.server.url, "Bob", owner_token=joined.owner_token, **options)
        self.assertEqual((second.owner, second.resident), (None, 2))
        self.assertEqual(self.server.world.residents[2].owner, 1)
        self.assertEqual(len(self.server.gateway.requests_to("/v1/owners")), 1)

    def test_a_call_without_the_token_that_it_needs_sends_nothing(self) -> None:
        client = self.server.client()
        for call in (
            client.observe,
            lambda: client.act("wait", {"ticks": 1}),
            lambda: client.set_routines([]),
            lambda: client.create_resident("Ada"),
            client.list_residents,
        ):
            with self.assertRaises(MissingCredentialError):
                call()
        self.assertEqual(self.server.gateway.requests, [])


class ClientTest(ServerCase):
    def test_a_rotated_key_replaces_the_old_key(self) -> None:
        old_key = self.resident.key
        self.client.me()
        rotated = self.client.rotate_key(self.resident.id)
        self.assertNotEqual(rotated["key"], old_key)
        self.assertEqual(self.client.resident_key, rotated["key"])
        self.assertEqual(self.client.me()["resident"], self.resident.id)

        with self.assertRaises(ApiError) as raised:
            self.server.client(old_key).me()
        self.assertEqual((raised.exception.code, raised.exception.status), ("unauthorized", 401))

    def test_the_public_routes_need_no_token(self) -> None:
        self.gateway.join_document = "# How to join\n"
        client = self.server.client()
        self.assertIs(client.health(), True)
        self.assertEqual(client.join_document(), "# How to join\n")
        verbs = ["%s.%s" % (verb["module"], verb["verb"]) for verb in client.catalogue()["verbs"]]
        self.assertEqual(verbs, ["kernel.cancel", "kernel.set_routines", "kernel.wait", "life.move_to"])
        layout = client.world()
        self.assertEqual((layout["type"], [zone["id"] for zone in layout["zones"]]), ("layout", [0]))
        zone = client.zone(0)
        self.assertEqual((zone["type"], [resident["name"] for resident in zone["residents"]]), ("zone", ["Ada"]))
        self.assertTrue(all(request.token is None for request in self.gateway.requests))
        self.assertEqual(len(self.gateway.requests), 5)

    def test_a_base_url_with_a_slash_at_the_end_works(self) -> None:
        client = Client(self.server.url + "/", self.resident.key, opener=self.server.opener)
        self.assertEqual(client.base_url, self.server.url)
        self.assertEqual(client.me()["name"], "Ada")

    def test_act_returns_the_acknowledgement_and_the_observation_and_sends_an_idempotency_key(self) -> None:
        ack, observation = self.client.act("move_to", {"x": 35, "y": 30})
        self.assertEqual(ack["verb"], "move_to")
        self.assertEqual(ack["status"], {"state": "started", "ends_at": 3, "ticks": 3})
        self.assertEqual(self_of(observation)["activity"]["verb"], "move_to")
        self.assertEqual(observation["resident"], self.resident.id)

        body = self.last_request().body
        self.assertRegex(body["id"], r"^[0-9a-f-]{36}$")
        self.assertEqual(ack["request_id"], body["id"])
        self.assertEqual((body["verb"], body["args"]), ("move_to", {"x": 35, "y": 30}))

        # A key of the caller goes out as it is.
        second = self.client.act("wait", {"ticks": 2}, id="my-key")
        self.assertEqual(second.ack["request_id"], "my-key")
        self.assertEqual(second.ack["status"], {"state": "queued", "position": 1})

    def test_a_refused_action_comes_back_as_data_with_its_error_code(self) -> None:
        unknown = self.client.act("fly")
        self.assertEqual(unknown.ack["status"]["state"], "rejected")
        self.assertEqual(rejection(unknown.ack)["code"], "unknown_verb")
        with self.assertRaises(ActionRejectedError) as raised:
            assert_accepted(unknown.ack)
        self.assertEqual(raised.exception.code, "unknown_verb")
        self.assertIs(raised.exception.ack, unknown.ack)

        self.assertEqual(rejection(self.client.act("wait", {"ticks": 0}).ack)["code"], "invalid_args")

        self.resident.conditions["frozen"] = None
        error = rejection(self.client.act("wait", {"ticks": 1}).ack)
        self.assertEqual((error["code"], error["details"]), ("blocked", {"condition": "frozen"}))

        del self.resident.conditions["frozen"]
        accepted = self.client.act("wait", {"ticks": 1})
        self.assertIsNone(rejection(accepted.ack))
        self.assertIs(assert_accepted(accepted.ack), accepted.ack)

    def test_plan_queues_the_steps_in_order_and_gives_one_acknowledgement_for_each(self) -> None:
        acks, observation = self.client.plan(
            [
                {"verb": "move_to", "args": {"x": 33, "y": 32}},
                {"verb": "fly"},
                {"verb": "wait", "args": {"ticks": 4}},
            ]
        )
        self.assertEqual([ack["status"]["state"] for ack in acks], ["started", "rejected", "queued"])
        self.assertEqual(self_of(observation)["queue"], 1)
        plan = self.last_request().body["plan"]
        self.assertEqual([step["verb"] for step in plan], ["move_to", "fly", "wait"])
        self.assertEqual(len({step["id"] for step in plan}), 3)

        # The plan runs: one tick to move, then four ticks of the wait.
        self.world.step(5)
        self.assertEqual(self.resident.pos, {"x": 33, "y": 32})
        reports = reports_of(self.client.observe())
        self.assertEqual(
            [(report["verb"], report["ok"], report["tick"]) for report in reports],
            [("move_to", True, 1), ("wait", True, 5)],
        )

        # An empty plan only observes.
        before = len(self.gateway.requests)
        self.assertEqual(self.client.plan([]).acks, [])
        self.assertEqual(len(self.gateway.requests), before + 1)
        self.assertEqual(self.last_request().path, "/v1/observe")

    def test_observe_keeps_the_notice_cursor(self) -> None:
        self.world.tell(self.resident.id, 7, "hello")
        self.world.notify(self.resident.id, "economy.wage_paid", {"amount": 500})

        first = self.client.observe()
        self.assertEqual(
            [(notice["seq"], notice["kind"]) for notice in first["notices"]],
            [(1, "society.speech"), (2, "economy.wage_paid")],
        )
        self.assertEqual(self.client.cursor, 3)
        self.assertNotIn("since", self.last_request().query)

        # Nothing new: the next observation has no notice, and the request carried the cursor.
        second = self.client.observe()
        self.assertEqual(second["notices"], [])
        self.assertEqual(self.last_request().query["since"], "3")
        self.assertFalse(has_news(second))

        self.world.notify(self.resident.id, "economy.rent_due")
        third = self.client.observe(detail="full", sections=["self"])
        self.assertEqual([notice["seq"] for notice in third["notices"]], [3])
        self.assertEqual(list(third["sections"]), ["self"])
        self.assertEqual(self_of(third)["owner"], 1)
        self.assertEqual(self.last_request().query, {"detail": "full", "since": "3", "sections": "self"})

        # An explicit `since` reads older notices again.
        again = self.client.observe(since=2)
        self.assertEqual([notice["seq"] for notice in again["notices"]], [2, 3])

    def test_act_moves_the_cursor_and_the_client_drops_notices_that_a_server_repeats(self) -> None:
        self.world.tell(self.resident.id, 7, "one")
        self.client.observe()
        self.world.tell(self.resident.id, 7, "two")

        acted = self.client.act("wait", {"ticks": 1})
        self.assertEqual(self.last_request().query["since"], "2")
        self.assertEqual([notice["seq"] for notice in acted.observation["notices"]], [2])
        self.assertEqual(self.client.cursor, 3)

        # A server that does not read `since` on this route sends the whole inbox.
        self.gateway.act_ignores_since = True
        self.world.tell(self.resident.id, 7, "three")
        repeated = self.client.act("wait", {"ticks": 1})
        self.assertEqual([notice["seq"] for notice in repeated.observation["notices"]], [3])
        self.assertEqual(self.client.cursor, 4)

        # The same for reports. They have no sequence number, so the client
        # remembers the reports that it returned and drops them from a repeat.
        self.world.step(1)
        self.assertEqual([(report["verb"], report["tick"]) for report in reports_of(self.client.observe())], [("wait", 1)])
        after_report = self.client.act("wait", {"ticks": 1})
        self.assertEqual(reports_of(after_report.observation), [])
        # A report that the client did not return before stays, also when it comes with an action.
        self.world.step(1)
        with_news = self.client.act("wait", {"ticks": 1})
        self.assertEqual([(report["verb"], report["tick"]) for report in reports_of(with_news.observation)], [("wait", 2)])
        # `observe` has a cursor in the protocol document. An explicit `since` reads old reports again.
        self.assertEqual([report["tick"] for report in reports_of(self.client.observe(since=1))], [1, 2])

    def test_wait_blocks_until_a_notice_arrives(self) -> None:
        self.client.observe()
        with ThreadPoolExecutor(1) as pool:
            waiting = pool.submit(self.client.wait, 30)
            self.world.parked()
            self.assertEqual(self.last_request().query, {"timeout": "30", "since": "1"})
            self.assertFalse(waiting.done())
            self.world.tell(self.resident.id, 7, "wake up")
            observation = waiting.result(RESULT_TIMEOUT_SECONDS)
        self.assertEqual(
            [notice["data"] for notice in observation["notices"]],
            [{"untrusted": True, "from": 7, "text": "wake up"}],
        )
        self.assertEqual(self.client.cursor, 2)

    def test_wait_returns_the_report_of_an_action_that_ended(self) -> None:
        self.client.act("wait", {"ticks": 3})
        with ThreadPoolExecutor(1) as pool:
            waiting = pool.submit(self.client.wait, 30, ["report"])
            self.world.parked()
            self.world.step(3)
            reports = reports_of(waiting.result(RESULT_TIMEOUT_SECONDS))
        self.assertEqual([(report["verb"], report["ok"], report["result"]) for report in reports], [("wait", True, {"waited": 3})])

    def test_wait_with_kinds_sleeps_through_other_notices(self) -> None:
        with ThreadPoolExecutor(1) as pool:
            waiting = pool.submit(self.client.wait, 30, ["economy.rent_due", "report"])
            self.world.parked()
            self.assertEqual(self.last_request().query["kinds"], "economy.rent_due,report")
            self.world.tell(self.resident.id, 7, "noise")
            self.assertFalse(waiting.done())
            self.world.notify(self.resident.id, "economy.rent_due")
            observation = waiting.result(RESULT_TIMEOUT_SECONDS)
        # The observation holds everything since the cursor, also the notice that did not wake the call.
        self.assertEqual([notice["kind"] for notice in observation["notices"]], ["society.speech", "economy.rent_due"])

    def test_a_wait_that_runs_out_returns_an_observation_without_news(self) -> None:
        with ThreadPoolExecutor(1) as pool:
            waiting = pool.submit(self.client.wait, 20)
            self.world.parked()
            self.world.expire_waits()
            observation = waiting.result(RESULT_TIMEOUT_SECONDS)
        self.assertFalse(has_news(observation))
        self.assertEqual(len(self.gateway.requests_to("/v1/wait")), 1)

    def test_set_webhook_sets_the_address_and_the_secret_signs_deliveries(self) -> None:
        info = self.client.set_webhook("https://agent.example/hook")
        self.assertEqual(info["url"], "https://agent.example/hook")
        self.assertEqual(info["secret"], self.resident.webhook["secret"])

        secret = info["secret"]
        body = '{"seq":1,"tick":5,"kind":"economy.wage_paid"}'
        signature = sign_webhook(secret, body)
        self.assertRegex(signature, r"^[0-9a-f]{64}$")
        # A fixed answer, checked with `openssl dgst -sha256 -hmac`. The TypeScript SDK has the same test.
        self.assertEqual(
            sign_webhook("webhook-secret-1-2", body),
            "99f547faf5157681638e12e4b1c66d90a08e6237a6c7136b2f56f4d059e08d0b",
        )
        self.assertEqual(sign_webhook("webhook-secret-1-2", body.encode("utf-8")), sign_webhook("webhook-secret-1-2", body))
        self.assertTrue(verify_webhook(secret, body, signature))
        self.assertTrue(verify_webhook(secret, body, "sha256=" + signature))
        self.assertFalse(verify_webhook(secret, body + " ", signature))
        self.assertFalse(verify_webhook("another-secret", body, signature))
        self.assertFalse(verify_webhook(secret, body, "00"))

        self.assertEqual(self.client.set_webhook(None), {"url": None})
        self.assertIsNone(self.resident.webhook)

    def test_an_error_answer_becomes_an_api_error_with_the_code_of_the_server(self) -> None:
        with self.assertRaises(ApiError) as raised:
            self.server.client(self.owner["token"]).observe()
        error = raised.exception
        self.assertEqual((error.code, error.status), ("forbidden", 403))
        self.assertEqual(str(error), "forbidden: This route needs a resident key, not an owner token.")

        # An answer without the documented body keeps its status.
        self.gateway.fail_next(400, body="Failed to parse the request body")
        with self.assertRaises(ApiError) as raised:
            self.client.observe()
        error = raised.exception
        self.assertEqual((error.code, error.status), ("http_error", 400))
        self.assertEqual(str(error), "http_error: HTTP 400: Failed to parse the request body")


class LongWaitTest(ServerCase):
    world_options = {"time": "auto"}

    def waits(self) -> list:
        return [request.query["timeout"] for request in self.gateway.requests_to("/v1/wait")]

    def test_a_long_wait_makes_several_long_polls_and_stops_at_the_first_news(self) -> None:
        # No news: 150 seconds are three polls, and game time passed.
        quiet = self.client.wait(150)
        self.assertFalse(has_news(quiet))
        self.assertEqual(self.waits(), ["60", "60", "30"])
        self.assertEqual(self.world.tick, 150)

        # News in the second poll: the third poll is not made.
        self.client.act("wait", {"ticks": 90})
        del self.gateway.requests[:]
        busy = self.client.wait(600)
        self.assertEqual(self.waits(), ["60", "60"])
        self.assertEqual([report["verb"] for report in reports_of(busy)], ["wait"])
        self.assertEqual(self.world.tick, 240)
        self.assertIsNone(self.resident.current)

        # With kinds, a notice of another kind does not end the long wait.
        del self.gateway.requests[:]
        self.world.tell(self.resident.id, 7, "noise")
        filtered = self.client.wait(120, ["report"])
        self.assertEqual(self.waits(), ["60", "60"])
        self.assertFalse(has_news(filtered, ["report"]))
        # The answer still holds the notice that came during the wait.
        self.assertEqual([notice["kind"] for notice in filtered["notices"]], ["society.speech"])
        self.assertEqual(self.client.observe()["notices"], [])

        # A timeout of zero only observes.
        del self.gateway.requests[:]
        self.client.wait(0)
        self.assertEqual([request.path for request in self.gateway.requests], ["/v1/observe"])


class RoutinesTest(ServerCase):
    world_options = {"attrs": [{"name": "need.energy", "min": 0, "max": 1000, "default": 1000, "summary": "0 means exhausted."}]}

    def test_set_routines_replaces_the_routines_and_reports_a_routine_that_is_not_valid(self) -> None:
        routines = [
            {
                "name": "rest when tired",
                "when": {"type": "attr_below", "attr": "need.energy", "value": 300},
                "then": {"verb": "wait", "args": {"ticks": 60}},
            }
        ]
        self.assertEqual(self.client.set_routines(routines), {"routines": 1})
        self.assertEqual((self.last_request().method, self.last_request().body), ("PUT", {"routines": routines}))
        self.assertEqual(self.resident.routines, routines)
        full = self.client.observe(detail="full")
        self.assertEqual(self_of(full)["routines"], [dict(routines[0], runs=0, failures=0)])

        with self.assertRaises(ApiError) as raised:
            self.client.set_routines([{"name": "fly", "when": {"type": "always"}, "then": {"verb": "fly"}}])
        error = raised.exception
        self.assertEqual((error.code, error.status, error.is_action_error), ("unknown_verb", 400, True))
        # The refused list changed nothing.
        self.assertEqual(self.resident.routines, routines)
        self.assertEqual(self.client.set_routines([]), {"routines": 0})
        self.assertEqual(self.resident.routines, [])


if __name__ == "__main__":
    unittest.main()
