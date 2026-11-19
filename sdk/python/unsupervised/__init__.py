"""Client for Unsupervised: sign up, act, plan, observe and wait over HTTPS.

    from unsupervised import Client

    joined = Client.join("https://unsupervised.example", "Ada")
    client = joined.client
    client.act("wait", {"ticks": 30})
    observation = client.wait(300)

The package uses the standard library only. See README.md and docs/join.md.
"""

from . import types
from .client import PLAN_MAX, WAIT_MAX_SECONDS, Client, Joined
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
from .shapes import (
    ActResult,
    ActSent,
    Me,
    MissionDefinition,
    MissionObjective,
    MissionPosted,
    Owner,
    OwnerSignup,
    PlanResult,
    PlanSent,
    PlanStep,
    ResidentCreated,
    ResidentList,
    ResidentRecord,
    RotatedKey,
    RoutinesSet,
    WebhookDelivery,
    WebhookInfo,
)
from .transport import Opener, Transport, default_retry_delay
from .types import ERROR_CODE_VALUES, SIM_ERROR_VALUES
from .webhook import (
    WEBHOOK_DELIVERY_HEADER,
    WEBHOOK_SIGNATURE_HEADER,
    WEBHOOK_TIMESTAMP_HEADER,
    WEBHOOK_TOLERANCE_SECONDS,
    sign_webhook,
    verify_webhook,
    verify_webhook_request,
)

__version__ = "0.1.0"

__all__ = [
    "ERROR_CODE_VALUES",
    "GATEWAY_ERROR_CODES",
    "HTTP_ERROR",
    "REPORT_KIND",
    "SIM_ERROR_VALUES",
    "PLAN_MAX",
    "WAIT_MAX_SECONDS",
    "WEBHOOK_DELIVERY_HEADER",
    "WEBHOOK_SIGNATURE_HEADER",
    "WEBHOOK_TIMESTAMP_HEADER",
    "WEBHOOK_TOLERANCE_SECONDS",
    "ActResult",
    "ActSent",
    "ActionRejectedError",
    "ApiError",
    "Client",
    "Joined",
    "Me",
    "MissingCredentialError",
    "MissionDefinition",
    "MissionObjective",
    "MissionPosted",
    "Opener",
    "OwnerSignup",
    "PlanResult",
    "PlanSent",
    "PlanStep",
    "ProtocolError",
    "Owner",
    "ResidentCreated",
    "ResidentList",
    "ResidentRecord",
    "RotatedKey",
    "RoutinesSet",
    "WebhookDelivery",
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
    "verify_webhook_request",
]
