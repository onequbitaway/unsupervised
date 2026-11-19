"""Compares the fake gateway of this SDK with a real server.

The script sends the probes of `sdk/conformance/probes.json` to both and
compares the answers: the status, the error code and the form of the body.
See `sdk/conformance/README.md`.

    python3 e2e/conformance.py

The Rust test `the_python_fake_answers_like_the_real_server` in
`crates/e2e` starts the server and runs this script.
"""

import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from support import ADMIN, BASE, OPENER, admin, fact, step, until

from unsupervised.testing import FakeGateway

PROBES = json.loads((Path(__file__).resolve().parents[2] / "conformance" / "probes.json").read_text(encoding="utf-8"))
UNKNOWN_KEY = "unsup_res_00000000000000000000000000000000000000000000000000000000000000ff"
#: Game minutes of one day, and the greatest number of days that a resident without food lives in a probe.
DAY = 1440
DAYS_MAX = 30

Send = Callable[[str, str, Dict[str, str], Optional[str]], Tuple[int, str]]


def type_name(value: Any) -> str:
    """Returns the name of a JSON type, as the TypeScript script writes it."""
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    return "string"


def shape_of(value: Any, path: str, probe: Dict[str, Any]) -> Any:
    """Returns the form of a JSON value: the names of its fields and the type of each value."""
    ignore = probe.get("ignore", [])
    if path in ignore:
        return "(not compared)"
    if value is None:
        return "null"
    if isinstance(value, list):
        entries = value[:1] if probe.get("lists") == "first" else value
        return [shape_of(entry, path, probe) for entry in entries]
    if isinstance(value, dict):
        out = {}
        for name in sorted(value):
            inner = name if path == "" else "%s.%s" % (path, name)
            if inner not in ignore:
                out[name] = shape_of(value[name], inner, probe)
        return out
    return type_name(value)


def at(value: Any, path: str) -> Any:
    """Reads the value at a path such as `resident.id`."""
    for name in path.split("."):
        value = value.get(name) if isinstance(value, dict) else None
    return value


class Side:
    """One side of the comparison: the fake or the real server."""

    def __init__(self, name: str, send: Send, during: Callable[[Callable[[], Any]], Any]) -> None:
        self.name = name
        self.send = send
        #: Runs a request whose answer needs a frame of the world.
        self.during = during
        self.saved: Dict[str, Any] = {}

    def request(self, method: str, path: str, token: Optional[str], body: Any = None, raw: Any = None) -> Tuple[int, Any]:
        """Sends a request. Returns the status and the parsed body."""
        headers: Dict[str, str] = {}
        if token is not None:
            headers["Authorization"] = "Bearer %s" % token
        text: Optional[str] = None
        if raw is not None:
            text, headers["Content-Type"] = raw
        elif body is not None:
            text, headers["Content-Type"] = json.dumps(body), "application/json"
        status, answer = self.send(method, path, headers, text)
        try:
            parsed = json.loads(answer) if answer else None
        except ValueError:
            parsed = {"(not JSON)": answer[:80]}
        return status, parsed

    def token(self, auth: str) -> Optional[str]:
        return {
            "owner": self.saved.get("owner_token"),
            "resident": self.saved.get("resident_key"),
            "other": self.saved.get("other_key"),
            "unknown": UNKNOWN_KEY,
        }.get(auth)

    def set_up(self) -> None:
        """Makes the second owner with its resident, for the probes that need another party."""
        _, owner = self.request("POST", "/v1/owners", None, {"name": "Other owner"})
        _, resident = self.request("POST", "/v1/residents", owner["token"], {"name": "Bo"})
        self.saved["other"] = resident["resident"]["id"]
        self.saved["other_key"] = resident["key"]

    def probe(self, probe: Dict[str, Any]) -> Dict[str, Any]:
        """Runs one probe that is a request."""
        path = probe["path"].replace("{resident}", str(self.saved.get("resident")))
        path = path.replace("{other}", str(self.saved.get("other")))
        raw = (probe["raw"], probe["content_type"]) if "raw" in probe else None

        def send() -> Tuple[int, Any]:
            return self.request(probe["method"], path, self.token(probe["auth"]), probe.get("body"), raw)

        status, body = self.during(send) if probe.get("runs") is True else send()
        for name, where in probe.get("save", {}).items():
            self.saved[name] = at(body, where)
        error = body.get("error") if isinstance(body, dict) else None
        seen: Dict[str, Any] = {"status": status, "code": error.get("code") if isinstance(error, dict) else None}
        if probe.get("shape") is True:
            seen["shape"] = shape_of(body, "", probe)
        if "count" in probe:
            entries = at(body, probe["count"])
            seen["count"] = len(entries) if isinstance(entries, list) else "(no list)"
        return seen


# ---- The fake ---------------------------------------------------------------

gateway = FakeGateway()
gateway.world.time = "auto"


def send_fake(method: str, path: str, headers: Dict[str, str], body: Optional[str]) -> Tuple[int, str]:
    answer = gateway.handle(method, path, headers, body or "")
    return answer.status, answer.body


def sanction_fake(kind: str) -> None:
    resident = gateway.world.residents[fake.saved["resident"]]
    if kind == "mute":
        gateway.muted.add(resident.id)
    elif kind == "unmute":
        gateway.muted.discard(resident.id)
    elif kind == "ban":
        gateway.banned.add(resident.owner)
    elif kind == "unban":
        gateway.banned.discard(resident.owner)
    elif kind == "despawn":
        gateway.world.despawn(resident)
    else:
        raise ValueError("the sanction %s is not known" % kind)


# The fake answers at once: its world does not have to run.
fake = Side("fake", send_fake, lambda request: request())

# ---- The real server --------------------------------------------------------


def send_real(method: str, path: str, headers: Dict[str, str], body: Optional[str]) -> Tuple[int, str]:
    data = body.encode("utf-8") if body is not None else None
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with OPENER.open(request, timeout=30) as response:
            return response.status, response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8")


def tell_real(text: str) -> None:
    action = {"verb": "message", "args": {"to": real.saved["resident"], "text": text}}
    status, sent = real.request("POST", "/v1/act", real.saved["other_key"], action)
    if status != 200 or sent["acks"][0]["status"]["state"] != "done":
        raise AssertionError("the other resident cannot send a message: %s" % sent)
    # A message to another district arrives in the next frame.
    step(2)


def sanction_real(kind: str) -> None:
    if not ADMIN:
        raise AssertionError("Set E2E_ADMIN_TOKEN for the sanctions.")
    if kind in ("mute", "unmute", "despawn"):
        admin("/admin/residents/%s/%s" % (real.saved["resident"], kind))
    else:
        admin("/admin/owners/%s/%s" % (real.saved["owner"], kind))


def starve_real() -> None:
    """Runs game days until the other resident is dead. The public profile says when.

    No call with the key of the resident is made: the first one is a probe.
    """
    for days in range(DAYS_MAX + 1):
        with OPENER.open("%s/v1/residents/%s/profile" % (BASE, real.saved["other"]), timeout=30) as response:
            if json.loads(response.read())["alive"] is False:
                return
        if days < DAYS_MAX:
            step(DAY)
    raise AssertionError("a resident without food died in %d game days" % DAYS_MAX)


real = Side("real", send_real, until)

# ---- Compare ----------------------------------------------------------------

mismatches: List[Dict[str, Any]] = []
requests = 0
fake.set_up()
real.set_up()
for probe in PROBES:
    if "advance" in probe:
        gateway.world.step(probe["advance"])
        step(probe["advance"])
    elif "tell" in probe:
        gateway.world.tell(fake.saved["resident"], fake.saved["other"], probe["tell"], "society.message")
        tell_real(probe["tell"])
    elif "starve" in probe:
        if probe["starve"] != "other":
            raise ValueError("the probes starve only the other resident, not %s" % probe["starve"])
        gateway.world.die(gateway.world.residents[fake.saved["other"]])
        starve_real()
    elif "sanction" in probe:
        if probe["sanction"] == "ban":
            # The owner id of the real side is read while the key still works.
            real.saved["owner"] = real.request("GET", "/v1/me", real.saved["resident_key"])[1]["owner"]
        sanction_fake(probe["sanction"])
        sanction_real(probe["sanction"])
    else:
        requests += 1
        mine, theirs = fake.probe(probe), real.probe(probe)
        if json.dumps(mine, sort_keys=True) != json.dumps(theirs, sort_keys=True):
            mismatches.append({"probe": probe["name"], "fake": mine, "real": theirs})
        # The probe says which answer it is about. Another answer of both
        # sides would compare well and show nothing.
        expected = ("%s %s" % (probe.get("status"), probe.get("code", ""))).strip()
        got = ("%s %s" % (theirs["status"], theirs["code"] or "")).strip()
        if expected != got:
            name = "%s (the probe expects %s)" % (probe["name"], expected)
            mismatches.append({"probe": name, "fake": mine, "real": theirs})
        if os.environ.get("E2E_PRINT") == "1":
            print("%s\t%s" % (got, probe["name"]), file=sys.stderr)

for mismatch in mismatches:
    print("MISMATCH: %s\n  fake: %s\n  real: %s" % (
        mismatch["probe"], json.dumps(mismatch["fake"], sort_keys=True), json.dumps(mismatch["real"], sort_keys=True)
    ), file=sys.stderr)
fact("conformance", sdk="python", probes=requests, mismatches=len(mismatches))
sys.exit(0 if not mismatches else 1)
