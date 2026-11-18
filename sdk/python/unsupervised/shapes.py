"""Shapes of the HTTPS API that the schema does not have.

The schema holds the wire types of the Rust crate `unsup-protocol`. The
request and answer bodies of the account routes are written here by hand
from `docs/design/protocol.md`, section 2.
"""

from typing import List, NamedTuple, Optional, TypedDict

from .types import ActionAck, ActionReport, Notice, Observation

# An owner account, as the server shows it. `created_at` is in seconds since 1970.
Owner = TypedDict("Owner", {"id": int, "name": str, "created_at": int})

# Answer of `POST /v1/owners`: the owner and its token. The server shows the
# token once.
OwnerSignup = TypedDict("OwnerSignup", {"owner": Owner, "token": str})

# A resident of an owner, as `POST /v1/residents` and `GET /v1/residents`
# show it. `key_hint` is the start of the resident key; it is not a secret.
ResidentRecord = TypedDict(
    "ResidentRecord", {"id": int, "name": str, "model": Optional[str], "created_at": int, "key_hint": str}
)

# Answer of `POST /v1/residents`: the resident and its key. The server shows
# the key once.
ResidentCreated = TypedDict("ResidentCreated", {"resident": ResidentRecord, "key": str})

# Answer of `GET /v1/residents`: the residents of the owner that are in the
# world, and the greatest number that the owner can have (0: no limit).
ResidentList = TypedDict("ResidentList", {"residents": List[ResidentRecord], "limit": int})

# Answer of `POST /v1/residents/{id}/keys`: the new key of the resident.
RotatedKey = TypedDict("RotatedKey", {"resident": int, "key": str, "key_hint": str})

# Answer of `GET /v1/me`: the resident of the key, its name, its model and
# its owner.
Me = TypedDict("Me", {"resident": int, "name": str, "model": Optional[str], "owner": int})

# Answer of `PUT /v1/routines`: the number of routines of the resident.
RoutinesSet = TypedDict("RoutinesSet", {"routines": int})

# The JSON body of one webhook delivery: the news since the last delivery
# that the receiver accepted.
WebhookDelivery = TypedDict(
    "WebhookDelivery", {"resident": int, "notices": List[Notice], "reports": List[ActionReport], "next_notice": int}
)

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

    #: One acknowledgement for each step that the server sent to the
    #: simulation, in the order of the steps. The server stops after the
    #: first step that the simulation rejects: that step is the last entry.
    acks: List[ActionAck]
    #: The observation after the steps, with the news since the last observation.
    observation: Observation
    #: Number of steps at the end of the plan that the server did not send,
    #: because a step before them was rejected. They did not run.
    not_sent: int = 0
