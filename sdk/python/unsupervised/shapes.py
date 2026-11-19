"""Shapes of the HTTPS API that the schema does not have.

The schema holds the wire types of the Rust crate `unsup-protocol`. The
request and answer bodies of the account routes are written here by hand
from `docs/design/protocol.md`, section 2.
"""

from typing import Any, Dict, List, NamedTuple, Optional, TypedDict

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


_MissionObjectiveRequired = TypedDict("_MissionObjectiveRequired", {"event": str})
_MissionObjectiveOptional = TypedDict(
    "_MissionObjectiveOptional", {"fields": Dict[str, Any], "count": int, "actor": str}, total=False
)


class MissionObjective(_MissionObjectiveRequired, _MissionObjectiveOptional):
    """One objective of a sponsor mission: a number of events of one kind.

    `event` is the kind of the event, for example `economy.goods_produced`.
    `fields` are data fields that the event must have, for example
    `{"item": "bread"}`. `count` is the number of events; the default is 1.
    `actor` is `assignee` (the default) or `team`.
    """


_MissionDefinitionRequired = TypedDict(
    "_MissionDefinitionRequired",
    {"title": str, "objectives": List[MissionObjective], "reward": int, "deadline_in": int},
)
_MissionDefinitionOptional = TypedDict(
    "_MissionDefinitionOptional", {"text": str, "mode": str, "org": Optional[int], "takers": int}, total=False
)


class MissionDefinition(_MissionDefinitionRequired, _MissionDefinitionOptional):
    """The definition of a mission: the body of `POST /v1/missions`.

    The fields are those of the arguments of the verb `mission_post`.
    `reward` is the cents that each taker gets. `deadline_in` is the game
    minutes from the publication to the deadline. `mode` is `solo` (the
    default) or `team`. `takers` is the number of takers; the default is 1.
    """


# Answer of `POST /v1/missions`: the mission that the world published.
# `frame` is the frame of the publication and `deadline` is the frame of the
# deadline. The public event `missions.mission_posted` has the same `request`.
MissionPosted = TypedDict(
    "MissionPosted", {"mission": int, "request": int, "frame": int, "deadline": int, "reward": int, "takers": int}
)


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


class ActSent(NamedTuple):
    """The answer of `Client.act` with `observe=False`: the acknowledgement alone."""

    #: What happened to the request. A refusal is here; it is not raised.
    ack: ActionAck


class PlanSent(NamedTuple):
    """The answer of `Client.plan` with `observe=False`: the acknowledgements alone."""

    #: One acknowledgement for each step that the server sent to the simulation.
    acks: List[ActionAck]
    #: Number of steps at the end of the plan that the server did not send.
    not_sent: int = 0
