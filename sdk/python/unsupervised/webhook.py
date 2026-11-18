"""Signatures of webhook deliveries (`docs/design/protocol.md`, "Webhooks").

Each delivery has three headers: the id of the delivery, the time of the
attempt, and the signature. The signature is `v1=` and the HMAC-SHA256, as
lowercase hexadecimal, of `v1:<timestamp>:<body>` with the secret of
`PUT /v1/webhook` as the key.
"""

import hashlib
import hmac
import time
from typing import Mapping, Optional, Union

#: Name of the header with the id of the delivery. It is the same for each attempt.
WEBHOOK_DELIVERY_HEADER = "X-Unsupervised-Delivery"
#: Name of the header with the time of the attempt, in seconds since 1970.
WEBHOOK_TIMESTAMP_HEADER = "X-Unsupervised-Timestamp"
#: Name of the header with the signature.
WEBHOOK_SIGNATURE_HEADER = "X-Unsupervised-Signature"
#: Greatest age of a delivery that `verify_webhook` accepts by default, in seconds.
WEBHOOK_TOLERANCE_SECONDS = 300

_VERSION = "v1"


def _bytes(value: Union[str, bytes]) -> bytes:
    return value.encode("utf-8") if isinstance(value, str) else value


def sign_webhook(secret: str, timestamp: Union[str, int], body: Union[str, bytes]) -> str:
    """Returns the value of the signature header for a delivery.

    It is `v1=` and the HMAC-SHA256 of `v1:<timestamp>:<body>`. `body` is
    the exact content of the delivery.
    """
    message = ("%s:%s:" % (_VERSION, timestamp)).encode("ascii") + _bytes(body)
    return "%s=%s" % (_VERSION, hmac.new(_bytes(secret), message, hashlib.sha256).hexdigest())


def verify_webhook(
    secret: str,
    timestamp: Union[str, int, None],
    body: Union[str, bytes],
    signature: Optional[str],
    *,
    tolerance: Optional[float] = WEBHOOK_TOLERANCE_SECONDS,
    now: Optional[float] = None,
) -> bool:
    """Checks a webhook delivery.

    `timestamp` is the header `X-Unsupervised-Timestamp`, `signature` is the
    header `X-Unsupervised-Signature`, and `body` is the exact content that
    arrived, before any JSON parsing. The delivery passes when the signature
    is that of the secret and the timestamp is at most `tolerance` seconds
    away from `now` (default: the clock of the machine). An old delivery is
    refused, so that nobody can send it again. `tolerance=None` turns that
    check off. A delivery without a timestamp or a signature does not pass.
    The comparison takes the same time for every wrong signature.
    """
    if timestamp is None or signature is None:
        return False
    stamp = str(timestamp).strip()
    if not (stamp.isascii() and stamp.isdigit()):
        return False
    if tolerance is not None:
        current = time.time() if now is None else now
        if abs(current - int(stamp)) > tolerance:
            return False
    expected = sign_webhook(secret, stamp, body)
    return hmac.compare_digest(signature.strip().encode("ascii", "replace"), expected.encode("ascii"))


def verify_webhook_request(
    secret: str,
    headers: Mapping[str, str],
    body: Union[str, bytes],
    *,
    tolerance: Optional[float] = WEBHOOK_TOLERANCE_SECONDS,
    now: Optional[float] = None,
) -> bool:
    """Checks a delivery from its HTTP headers and its body.

    `headers` is the header mapping of the request. The names can have any
    case, as in `http.server` and in most frameworks.
    """
    lowered = {name.lower(): value for name, value in headers.items()}
    return verify_webhook(
        secret,
        lowered.get(WEBHOOK_TIMESTAMP_HEADER.lower()),
        body,
        lowered.get(WEBHOOK_SIGNATURE_HEADER.lower()),
        tolerance=tolerance,
        now=now,
    )
