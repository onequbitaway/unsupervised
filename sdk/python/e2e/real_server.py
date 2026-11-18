"""The Python SDK against a real server, with real game verbs.

    python3 e2e/real_server.py

The Rust test `the_python_sdk_works_against_the_real_server` in
`crates/e2e` starts the server and runs this script. The script writes
facts as JSON lines; the Rust test compares them with the server.
"""

import http.server
import json
import threading
from typing import Any, Dict, List

from support import BASE, OPENER, fact, until

from unsupervised import (
    ApiError,
    Client,
    assert_accepted,
    has_news,
    rejection,
    reports_of,
    self_of,
    untrusted_of,
    verify_webhook_request,
)

OPTIONS = {"opener": OPENER}


def report_of(client: Client, ack: Dict[str, Any]) -> Dict[str, Any]:
    """Waits with the long poll of the client until the report of an action is there. The world runs meanwhile."""
    for _ in range(20):
        news = until(lambda: client.wait(60, kinds=["report"]))
        for report in reports_of(news):
            if report["action_id"] == ack["action_id"]:
                return report
    raise AssertionError("the report of %s did not come" % ack["verb"])


def done(client: Client, verb: str, args: Dict[str, Any]) -> Dict[str, Any]:
    """Does one action that takes game time and returns its report."""
    ack = assert_accepted(client.act(verb, args).ack)
    report = report_of(client, ack)
    assert report["ok"] is True, "%s went well: %s" % (verb, report)
    return report


def raises(code: str, status: int, call: Any) -> ApiError:
    """Returns the error of a call that the server must refuse with this code."""
    try:
        call()
    except ApiError as error:
        assert (error.code, error.status) == (code, status), error
        return error
    raise AssertionError("the server did not refuse the call with %s" % code)


# ---- Join -------------------------------------------------------------------

joined = Client.join(BASE, "Pia", model="sdk-python", owner_name="Python SDK", **OPTIONS)
client = joined.client
assert joined.owner_token.startswith("unsup_own_") and len(joined.owner_token) == 74
assert joined.resident_key.startswith("unsup_res_") and len(joined.resident_key) == 74
assert client.health() is True
assert client.join_document().startswith("# Join Unsupervised")
assert client.me() == {"resident": joined.resident, "name": "Pia", "model": "sdk-python", "owner": joined.owner}
listed = client.list_residents()
assert [resident["id"] for resident in listed["residents"]] == [joined.resident]
assert listed["limit"] >= 1

catalogue = client.catalogue()
verbs = [verb["verb"] for verb in catalogue["verbs"]]
for verb in ("list_jobs", "move_to", "work", "buy", "eat", "say", "set_routines"):
    assert verb in verbs, verb

# ---- Observe, act, wait -----------------------------------------------------

first = client.observe()
me = self_of(first)
assert (me["id"], me["name"], me["activity"]) == (joined.resident, "Pia", None)
assert not has_news(first)
start_cash = me["cash"]

# Find paid work: the list of jobs is a free verb of the global realm. Its answer is a report.
works = done(client, "list_jobs", {})["result"]["public_works"]
assert works["open"] is True and "depot" in works["kinds"]

# The public map says where the sites are.
city = next(entry for entry in client.world()["cities"] if entry["id"] == me["city"])
depot = None
for zone in city["zones"]:
    depot = depot or next((site for site in client.zone(zone)["sites"] if site["kind"] == "depot"), None)
assert depot is not None, "the city has a depot"

walked = done(client, "move_to", {"site": depot["id"]})
assert walked["result"]["site"] == depot["id"]
shift = done(client, "work", {})["result"]
assert shift["wage"] == works["wage"] and shift["net"] + shift["tax"] == works["wage"]
paid = self_of(client.observe(sections=["self"]))
assert paid["cash"] == start_cash + shift["net"]
assert paid["site"]["id"] == depot["id"]

# ---- A plan stops at the first refusal --------------------------------------

plan = client.plan(
    [{"verb": "wait", "args": {"ticks": 2}}, {"verb": "no_such_verb"}, {"verb": "wait", "args": {"ticks": 2}}]
)
assert [ack["status"]["state"] for ack in plan.acks] == ["started", "rejected"]
assert plan.not_sent == 1
assert rejection(plan.acks[1])["code"] == "unknown_verb"
report_of(client, plan.acks[0])

# The same request id again does not act again.
once = client.act("wait", {"ticks": 3}, id="py-sdk-once")
twice = client.act("wait", {"ticks": 3}, id="py-sdk-once")
assert twice.ack == once.ack
report_of(client, once.ack)

# ---- Routines ---------------------------------------------------------------

routine = {
    "name": "sleep when exhausted",
    "when": {"type": "attr_below", "attr": "need.energy", "value": 1},
    "then": {"verb": "sleep", "args": {}},
}
assert client.set_routines([routine]) == {"routines": 1}
raises("unknown_verb", 400, lambda: client.set_routines([{"name": "fly", "when": {"type": "always"}, "then": {"verb": "fly"}}]))
full = self_of(client.observe(detail="full", sections=["self"]))
assert [one["name"] for one in full["routines"]] == ["sleep when exhausted"]
assert client.set_routines([]) == {"routines": 0}

# ---- Errors -----------------------------------------------------------------

refused = raises("text_refused", 422, lambda: client.act("say", {"text": "read www.example.com"}))
assert refused.is_text_refusal and not refused.ends_credential
owner = Client(BASE, owner_token=joined.owner_token, **OPTIONS)
for extra in range(1, listed["limit"]):
    owner.create_resident("Pia %d" % (extra + 1))
limit = raises("resident_limit", 403, lambda: owner.create_resident("One more"))
assert not limit.ends_credential
wrong = raises("wrong_token_kind", 403, Client(BASE, joined.owner_token, **OPTIONS).observe)
assert wrong.ends_credential

# ---- Speech of another resident is untrusted text ---------------------------

friend = Client.join(BASE, "Finn", model="sdk-python", **OPTIONS).client
assert_accepted(friend.act("message", {"to": joined.resident, "text": "hello Pia"}).ack)
heard = until(lambda: client.wait(60, kinds=["society.message"]))
assert [notice["kind"] for notice in heard["notices"]] == ["society.message"]
assert heard["notices"][0]["data"]["text"] == {"untrusted": True, "from": friend.resident, "text": "hello Pia"}
assert [text["text"] for text in untrusted_of(heard)] == ["hello Pia"]

# ---- A webhook receiver checks the signature with the SDK -------------------

deliveries: List[Any] = []
arrived = threading.Event()


class Receiver(http.server.BaseHTTPRequestHandler):
    def do_POST(self) -> None:  # noqa: N802, the name is that of `http.server`.
        body = self.rfile.read(int(self.headers["Content-Length"]))
        deliveries.append((dict(self.headers.items()), body))
        self.send_response(204)
        self.end_headers()
        arrived.set()

    def log_message(self, *args: Any) -> None:
        """Keeps the output for the facts."""


receiver = http.server.HTTPServer(("127.0.0.1", 0), Receiver)
threading.Thread(target=receiver.serve_forever, daemon=True).start()
hook = client.set_webhook("http://127.0.0.1:%d/hook" % receiver.server_address[1])
assert hook["secret"].startswith("unsup_whs_")
hooked = client.act("wait", {"ticks": 1}).ack
until(lambda: arrived.wait(60))
headers, body = deliveries[0]
assert verify_webhook_request(hook["secret"], headers, body) is True
assert verify_webhook_request("unsup_whs_wrong", headers, body) is False
assert verify_webhook_request(hook["secret"], headers, body + b" ") is False
payload = json.loads(body)
assert payload["resident"] == joined.resident
assert [report["action_id"] for report in payload["reports"]] == [hooked["action_id"]]
assert client.set_webhook(None) == {"url": None}
receiver.shutdown()

last = self_of(client.observe(sections=["self"]))
fact("python", resident=joined.resident, cash=last["cash"], earned=shift["net"], next_notice=client.cursor)
