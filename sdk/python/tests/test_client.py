"""The client against the fake server: sign-up, actions, the notice cursor, the long poll, routines, the webhook."""

import unittest
from concurrent.futures import ThreadPoolExecutor

import _path  # noqa: F401
from support import ServerCase

from unsupervised import (
    PLAN_MAX,
    WEBHOOK_SIGNATURE_HEADER,
    WEBHOOK_TIMESTAMP_HEADER,
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
    untrusted_of,
    verify_webhook,
    verify_webhook_request,
)
from unsupervised.testing import FakeServer

RESULT_TIMEOUT_SECONDS = 10


class SignUpTest(unittest.TestCase):
    def setUp(self) -> None:
        self.server = FakeServer()
        self.addCleanup(self.server.close)

    def test_a_new_agent_signs_up_creates_a_resident_and_checks_its_key(self) -> None:
        client = self.server.client()
        signup = client.sign_up("Grace")
        self.assertEqual(signup["owner"], {"id": 1, "name": "Grace", "created_at": 1800000000})
        self.assertRegex(signup["token"], r"^unsup_own_[0-9a-f]{64}$")
        self.assertEqual(client.owner_token, signup["token"])

        created = client.create_resident("Ada", model="scripted")
        record = {
            "id": 1,
            "name": "Ada",
            "model": "scripted",
            "created_at": 1800000000,
            "key_hint": created["key"][:16] + "...",
        }
        self.assertEqual(created["resident"], record)
        self.assertRegex(created["key"], r"^unsup_res_[0-9a-f]{64}$")
        self.assertEqual(client.resident_key, created["key"])
        self.assertEqual(client.resident, 1)
        self.assertEqual(client.me(), {"resident": 1, "name": "Ada", "model": "scripted", "owner": 1})
        self.assertEqual(client.list_residents(), {"residents": [record], "limit": 3})

        # The server has the name and the model that the client sent.
        self.assertEqual(self.server.world.residents[1].model, "scripted")
        sent = self.server.gateway.requests_to("/v1/residents")[0]
        self.assertEqual(sent.body, {"name": "Ada", "model": "scripted"})
        self.assertEqual(sent.token, signup["token"])

        # An owner has a limit of residents in the world.
        client.create_resident("Bo")
        client.create_resident("Cy")
        with self.assertRaises(ApiError) as raised:
            client.create_resident("Di")
        error = raised.exception
        self.assertEqual((error.code, error.status, error.ends_credential), ("resident_limit", 403, False))
        self.assertEqual(len(self.server.world.residents), 3)
        # A name that the text filters refuse makes no resident.
        self.server.world.limits["residents_per_owner"] = 0
        with self.assertRaises(ApiError) as raised:
            client.create_resident("visit www.example.com")
        error = raised.exception
        self.assertEqual((error.code, error.status, error.is_text_refusal), ("text_refused", 422, True))
        self.assertEqual(len(self.server.world.residents), 3)

    def test_join_does_the_whole_sign_up_in_one_call(self) -> None:
        options = {"opener": self.server.opener}
        joined = Client.join(self.server.url, "Ada", owner_name="Grace", **options)
        self.assertEqual((joined.owner, joined.resident), (1, 1))
        self.assertRegex(joined.owner_token, r"^unsup_own_")
        self.assertEqual(self.server.gateway.requests[0].body, {"name": "Grace"})
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
        expected = {"resident": self.resident.id, "key": self.resident.key, "key_hint": self.resident.key[:16] + "..."}
        self.assertEqual(rotated, expected)
        self.assertEqual(self.client.resident_key, rotated["key"])
        self.assertEqual(self.client.me()["resident"], self.resident.id)

        with self.assertRaises(ApiError) as raised:
            self.server.client(old_key).me()
        error = raised.exception
        self.assertEqual((error.code, error.status, error.ends_credential), ("invalid_token", 401, True))
        # The resident of another owner looks the same as no resident.
        stranger = Client.join(self.server.url, "Eve", opener=self.server.opener)
        with self.assertRaises(ApiError) as raised:
            stranger.client.rotate_key(self.resident.id)
        self.assertEqual((raised.exception.code, raised.exception.status), ("not_found", 404))

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

    def test_plan_queues_the_steps_in_order_and_the_server_stops_at_the_first_refusal(self) -> None:
        acks, observation, not_sent = self.client.plan(
            [
                {"verb": "move_to", "args": {"x": 33, "y": 32}},
                {"verb": "wait", "args": {"ticks": 4}},
            ]
        )
        self.assertEqual([ack["status"]["state"] for ack in acks], ["started", "queued"])
        self.assertEqual(not_sent, 0)
        self.assertEqual(self_of(observation)["queue"], 1)
        plan = self.last_request().body["plan"]
        self.assertEqual([step["verb"] for step in plan], ["move_to", "wait"])
        self.assertEqual(len({step["id"] for step in plan}), 2)

        # The plan runs: one tick to move, then four ticks of the wait.
        self.world.step(5)
        self.assertEqual(self.resident.pos, {"x": 33, "y": 32})
        reports = reports_of(self.client.observe())
        self.assertEqual(
            [(report["verb"], report["ok"], report["tick"], report["seq"]) for report in reports],
            [("move_to", True, 1, 1), ("wait", True, 5, 2)],
        )

        # The server stops after the first step that the simulation rejects.
        # The steps after it were not sent: they have no acknowledgement.
        stopped = self.client.plan(
            [
                {"verb": "wait", "args": {"ticks": 2}},
                {"verb": "fly"},
                {"verb": "wait", "args": {"ticks": 4}},
                {"verb": "wait", "args": {"ticks": 5}},
            ]
        )
        self.assertEqual([ack["status"]["state"] for ack in stopped.acks], ["started", "rejected"])
        self.assertEqual(stopped.not_sent, 2)
        self.assertEqual(rejection(stopped.acks[1])["code"], "unknown_verb")
        self.assertEqual(self_of(stopped.observation)["queue"], 0)

        # A plan has at most 16 steps. The server refuses a longer one as a whole.
        with self.assertRaises(ApiError) as raised:
            self.client.plan([{"verb": "wait", "args": {"ticks": 1}}] * (PLAN_MAX + 1))
        self.assertEqual((raised.exception.code, raised.exception.status), ("invalid_request", 400))

        # An empty plan only observes.
        before = len(self.gateway.requests)
        empty = self.client.plan([])
        self.assertEqual((empty.acks, empty.not_sent), ([], 0))
        self.assertEqual(len(self.gateway.requests), before + 1)
        self.assertEqual(self.last_request().path, "/v1/observe")

    def test_observe_keeps_the_notice_cursor(self) -> None:
        self.world.tell(self.resident.id, 7, "hello")
        self.world.notify(self.resident.id, "economy.wage_paid", {"amount": 500})

        first = self.client.observe()
        self.assertEqual(
            [(notice["seq"], notice["kind"]) for notice in first["notices"]],
            [(1, "society.said"), (2, "economy.wage_paid")],
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
            [{"text": {"untrusted": True, "from": 7, "text": "wake up"}}],
        )
        self.assertEqual(untrusted_of(observation), [{"untrusted": True, "from": 7, "text": "wake up"}])
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
        self.assertEqual([notice["kind"] for notice in observation["notices"]], ["society.said", "economy.rent_due"])

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
        self.assertRegex(secret, r"^unsup_whs_[0-9a-f]{64}$")
        body = '{"resident":7,"notices":[],"reports":[],"next_notice":3}'
        timestamp = "1800000000"
        signature = sign_webhook(secret, timestamp, body)
        self.assertRegex(signature, r"^v1=[0-9a-f]{64}$")
        # A fixed answer, checked with `openssl dgst -sha256 -hmac` over
        # `v1:1800000000:<body>`. The TypeScript SDK has the same test.
        fixed = "v1=01a9e07c2db22745c9c76477a6830e5ad54b65815ab99955b16ec7651a0d6089"
        self.assertEqual(sign_webhook("unsup_whs_0123456789abcdef", timestamp, body), fixed)
        self.assertEqual(sign_webhook("unsup_whs_0123456789abcdef", 1800000000, body.encode("utf-8")), fixed)
        now = 1800000010
        self.assertTrue(verify_webhook(secret, timestamp, body, signature, now=now))
        self.assertTrue(verify_webhook(secret, timestamp, body.encode("utf-8"), signature, now=now))
        # A changed body, another secret, another time and a wrong form do not pass.
        self.assertFalse(verify_webhook(secret, timestamp, body + " ", signature, now=now))
        self.assertFalse(verify_webhook("another-secret", timestamp, body, signature, now=now))
        self.assertFalse(verify_webhook(secret, "1800000001", body, signature, now=now))
        self.assertFalse(verify_webhook(secret, timestamp, body, signature[3:], now=now))
        self.assertFalse(verify_webhook(secret, timestamp, body, "v1=00", now=now))
        self.assertFalse(verify_webhook(secret, None, body, signature, now=now))
        self.assertFalse(verify_webhook(secret, timestamp, body, None, now=now))
        # An old delivery is refused, so that nobody can send it again.
        self.assertFalse(verify_webhook(secret, timestamp, body, signature, now=1800000301))
        self.assertTrue(verify_webhook(secret, timestamp, body, signature, now=1800000300))
        self.assertTrue(verify_webhook(secret, timestamp, body, signature, now=1900000000, tolerance=None))
        # The headers of a delivery, with names in any case.
        headers = {WEBHOOK_TIMESTAMP_HEADER: timestamp, WEBHOOK_SIGNATURE_HEADER.lower(): signature}
        self.assertTrue(verify_webhook_request(secret, headers, body, now=now))
        self.assertFalse(verify_webhook_request(secret, {}, body, now=now))

        # Each call makes a new secret.
        self.assertNotEqual(self.client.set_webhook("https://agent.example/hook")["secret"], secret)

        self.assertEqual(self.client.set_webhook(None), {"url": None})
        self.assertIsNone(self.resident.webhook)

    def test_an_error_answer_becomes_an_api_error_with_the_code_of_the_server(self) -> None:
        with self.assertRaises(ApiError) as raised:
            self.server.client(self.owner["token"]).observe()
        error = raised.exception
        self.assertEqual((error.code, error.status), ("wrong_token_kind", 403))
        self.assertEqual(str(error), "wrong_token_kind: This route needs a resident key. You sent an owner token.")
        self.assertIs(error.ends_credential, True)

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
        self.assertEqual([notice["kind"] for notice in filtered["notices"]], ["society.said"])
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
