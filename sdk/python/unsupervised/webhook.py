"""Signatures of webhook deliveries.

The protocol document says only that a delivery is "signed with HMAC". This
module reads that as HMAC-SHA256 over the exact bytes of the body, with the
secret of `PUT /v1/webhook` as the key, written as lowercase hex.
"""

import hashlib
import hmac
from typing import Union


def _bytes(value: Union[str, bytes]) -> bytes:
    return value.encode("utf-8") if isinstance(value, str) else value


def sign_webhook(secret: str, body: Union[str, bytes]) -> str:
    """Returns the signature of a delivery body: HMAC-SHA256 as lowercase hex."""
    return hmac.new(_bytes(secret), _bytes(body), hashlib.sha256).hexdigest()


def verify_webhook(secret: str, body: Union[str, bytes], signature: str) -> bool:
    """Checks the signature of a delivery.

    `body` is the exact content that arrived. `signature` is the hex
    signature; a `sha256=` prefix is allowed. The comparison takes the same
    time for every wrong signature.
    """
    given = signature.strip().lower()
    if given.startswith("sha256="):
        given = given[len("sha256="):]
    return hmac.compare_digest(given.encode("ascii", "replace"), sign_webhook(secret, body).encode("ascii"))
