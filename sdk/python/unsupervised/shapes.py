"""Shapes of the HTTPS API that the schema does not have.

The schema holds the wire types of the Rust crate `unsup-protocol`. The
request and answer bodies of the account routes are written here by hand
from `docs/design/protocol.md`, section 2.
"""

from typing import List, NamedTuple, Optional, TypedDict

from .types import ActionAck, Observation

# Answer of `POST /v1/owners`: the id of the new owner and its token.
OwnerSignup = TypedDict("OwnerSignup", {"owner": int, "token": str})

# Answer of `POST /v1/residents` and of `POST /v1/residents/{id}/keys`.
# The server shows the key once.
ResidentKey = TypedDict("ResidentKey", {"resident": int, "key": str})

_ResidentSummaryRequired = TypedDict("_ResidentSummaryRequired", {"id": int, "name": str})
_ResidentSummaryOptional = TypedDict("_ResidentSummaryOptional", {"model": Optional[str]}, total=False)


class ResidentSummary(_ResidentSummaryRequired, _ResidentSummaryOptional):
    """One entry of `GET /v1/residents`."""


# Answer of `GET /v1/me`: the id and the name of the resident of the key.
Me = TypedDict("Me", {"resident": int, "name": str})

_WebhookInfoRequired = TypedDict("_WebhookInfoRequired", {"url": Optional[str]})
_WebhookInfoOptional = TypedDict("_WebhookInfoOptional", {"secret": str}, total=False)


class WebhookInfo(_WebhookInfoRequired, _WebhookInfoOptional):
    """Answer of `PUT /v1/webhook`: the address, and the secret that signs each delivery."""


_PlanStepRequired = TypedDict("_PlanStepRequired", {"verb": str})
_PlanStepOptional = TypedDict("_PlanStepOptional", {"args": object, "id": str}, total=False)


class PlanStep(_PlanStepRequired, _PlanStepOptional):
    """One step of a plan: a verb, its arguments and an optional idempotency key."""


class ActResult(NamedTuple):
    """The answer of `Client.act`."""

    #: What happened to the request. A refusal is here; it is not raised.
    ack: ActionAck
    #: The observation after the action, with the news since the last observation.
    observation: Observation


class PlanResult(NamedTuple):
    """The answer of `Client.plan`."""

    #: One acknowledgement for each step, in the order of the steps.
    acks: List[ActionAck]
    #: The observation after the steps, with the news since the last observation.
    observation: Observation
