"""What the scripts of this directory share.

The Rust test (`crates/e2e`) starts the real server and gives:

    UNSUPERVISED_URL   the address of the server
    E2E_CLOCK_URL      a service that runs frames of the world:
                       `POST <url>/step?frames=N`
    E2E_ADMIN_TOKEN    the admin token of the server
"""

import json
import os
import sys
import threading
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, Optional

# The scripts run from a checkout: the SDK is the directory above this one.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def need(name: str) -> str:
    """Returns an environment variable that the Rust test gives."""
    value = os.environ.get(name, "")
    if not value:
        sys.exit("Set %s. The Rust test in crates/e2e gives it." % name)
    return value


#: The address of the real server.
BASE = need("UNSUPERVISED_URL").rstrip("/")
#: The admin token of the real server.
ADMIN = os.environ.get("E2E_ADMIN_TOKEN", "")

#: An opener that ignores the proxy settings of the machine: the server is local.
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def step(frames: int) -> int:
    """Runs frames of the real world. One frame is one game minute. Returns the last frame."""
    request = urllib.request.Request("%s/step?frames=%d" % (need("E2E_CLOCK_URL"), frames), data=b"", method="POST")
    with OPENER.open(request, timeout=60) as response:
        return json.loads(response.read())["frame"]


def until(call: Callable[[], Any], limit: int = 5000) -> Any:
    """Runs the world, one frame at a time, until `call` returns. Returns what it gave.

    `call` blocks for news of the world, for example a long poll. It runs
    in a second thread while this thread runs the frames.
    """
    result: Dict[str, Any] = {}

    def run() -> None:
        try:
            result["value"] = call()
        except BaseException as error:  # The caller gets the error of the thread.
            result["error"] = error

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    frames = 0
    while thread.is_alive():
        if frames >= limit:
            raise AssertionError("the awaited news did not come in %d frames" % limit)
        step(1)
        frames += 1
        # Let the answer of the server arrive before the next frame.
        thread.join(0.001)
    if "error" in result:
        raise result["error"]
    return result["value"]


def admin(path: str, body: Optional[Dict[str, Any]] = None) -> Any:
    """Sends a `POST` with the admin token. Returns the JSON answer."""
    headers = {"Authorization": "Bearer %s" % ADMIN}
    data = b""
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method="POST")
    with OPENER.open(request, timeout=60) as response:
        return json.loads(response.read())


def fact(name: str, **fields: Any) -> None:
    """Writes one fact for the Rust test: a JSON object on one line."""
    print(json.dumps(dict({"fact": name}, **fields)), flush=True)
