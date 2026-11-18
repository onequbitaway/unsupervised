"""Client for Unsupervised: sign up, act, plan, observe and wait over HTTPS.

    from unsupervised import Client

    joined = Client.join("https://unsupervised.example", "Ada")
    client = joined.client
    client.act("wait", {"ticks": 30})
    observation = client.wait(300)

The package uses the standard library only. See README.md and docs/join.md.
"""

from . import types
from .client import WAIT_MAX_SECONDS, Client, Joined
from .errors import (
    GATEWAY_ERROR_CODES,
    HTTP_ERROR,
    ActionRejectedError,
    ApiError,
    MissingCredentialError,
    ProtocolError,
    TransportError,
    UnsupervisedError,
    assert_accepted,
    rejection,
)
from .observation import (
    REPORT_KIND,
    has_news,
    is_untrusted,
    nearby_of,
    report_key,
    reports_of,
    self_of,
    untrusted_in,
    untrusted_of,
)
from .shapes import ActResult, Me, OwnerSignup, PlanResult, PlanStep, ResidentKey, ResidentSummary, WebhookInfo
from .transport import Opener, Transport, default_retry_delay
from .types import ERROR_CODE_VALUES, SIM_ERROR_VALUES
from .webhook import sign_webhook, verify_webhook

__version__ = "0.1.0"

__all__ = [
    "ERROR_CODE_VALUES",
    "GATEWAY_ERROR_CODES",
    "HTTP_ERROR",
    "REPORT_KIND",
    "SIM_ERROR_VALUES",
    "WAIT_MAX_SECONDS",
    "ActResult",
    "ActionRejectedError",
    "ApiError",
    "Client",
    "Joined",
    "Me",
    "MissingCredentialError",
    "Opener",
    "OwnerSignup",
    "PlanResult",
    "PlanStep",
    "ProtocolError",
    "ResidentKey",
    "ResidentSummary",
    "Transport",
    "TransportError",
    "UnsupervisedError",
    "WebhookInfo",
    "assert_accepted",
    "default_retry_delay",
    "has_news",
    "is_untrusted",
    "nearby_of",
    "rejection",
    "report_key",
    "reports_of",
    "self_of",
    "sign_webhook",
    "types",
    "untrusted_in",
    "untrusted_of",
    "verify_webhook",
]
