"""A local HTTP server in front of the fake gateway."""

import socket
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, List, Optional

from ..client import Client
from .gateway import FakeGateway
from .world import FakeWorld


class FakeServer:
    """Serves a fake gateway on a free local port.

        with FakeServer() as server:
            client = server.client()
            client.sign_up("Grace")

    The server runs in its own thread. `close` stops it; the `with` form
    calls `close`.
    """

    def __init__(self, gateway: Optional[FakeGateway] = None) -> None:
        self.gateway = gateway if gateway is not None else FakeGateway()
        self.world: FakeWorld = self.gateway.world
        #: Delays that the clients of `client()` asked for between attempts, in seconds.
        self.sleeps: List[float] = []
        #: An opener that uses no proxy, for a `Client` that a test makes by hand.
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self._lose = 0
        self._refuse = 0
        self._lock = threading.Lock()
        server = self

        class Handler(BaseHTTPRequestHandler):
            def _serve(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length).decode("utf-8") if length else ""
                if server._take("_refuse"):
                    self._cut()
                    return
                answer = server.gateway.handle(self.command, self.path, self.headers, body)
                if server._take("_lose"):
                    self._cut()
                    return
                data = answer.body.encode("utf-8")
                self.send_response(answer.status)
                for name, value in answer.headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def _cut(self) -> None:
                # Close without an answer: the client sees a lost connection.
                self.close_connection = True
                try:
                    self.connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

            do_GET = do_POST = do_PUT = _serve

            def log_message(self, format: str, *args: Any) -> None:
                pass

        self._httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._httpd.daemon_threads = True
        #: Address of the server, for `base_url`.
        self.url = "http://127.0.0.1:%d" % self._httpd.server_address[1]
        self._thread = threading.Thread(target=self._httpd.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        self._thread.start()

    def _take(self, counter: str) -> bool:
        with self._lock:
            left = getattr(self, counter)
            if left > 0:
                setattr(self, counter, left - 1)
            return left > 0

    def lose_responses(self, count: int) -> None:
        """The next `count` requests are handled, but the answer is lost: the connection is cut."""
        with self._lock:
            self._lose = count

    def refuse_requests(self, count: int) -> None:
        """The next `count` requests are cut before the gateway sees them."""
        with self._lock:
            self._refuse = count

    def client(self, resident_key: Optional[str] = None, owner_token: Optional[str] = None, **options: Any) -> Client:
        """Returns a client for this server.

        The client uses no proxy. It does not sleep between attempts: it
        records each delay in `sleeps`. The delay is the attempt number in
        seconds, or the `Retry-After` value when the answer had one.
        """
        options.setdefault("opener", self.opener)
        options.setdefault("sleep", self.sleeps.append)
        options.setdefault("retry_delay", lambda attempt, retry_after: attempt if retry_after is None else retry_after)
        return Client(self.url, resident_key, owner_token, **options)

    def close(self) -> None:
        """Stops the server and ends every parked long poll."""
        self.world.close()
        self._httpd.shutdown()
        self._httpd.server_close()
        self._thread.join()

    def __enter__(self) -> "FakeServer":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
