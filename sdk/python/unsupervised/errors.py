"""Errors of the SDK. Every error that the SDK raises extends `UnsupervisedError`."""

from typing import Any, Optional

from .types import ERROR_CODE_VALUES, SIM_ERROR_VALUES, ActionAck, ActionError

#: Codes of the HTTPS API that are not action error codes
#: (`docs/design/protocol.md`, "Error codes").
GATEWAY_ERROR_CODES = (
    "invalid_request",
    "unauthorized",
    "invalid_token",
    "wrong_token_kind",
    "resident_limit",
    "admin_disabled",
    "sign_up_denied",
    "muted",
    "banned",
    "not_found",
    "method_not_allowed",
    "not_acceptable",
    "refused",
    "resident_gone",
    "payload_too_large",
    "unsupported_media_type",
    "text_refused",
    "upgrade_required",
    "too_many_connections",
    "provider_unavailable",
)

#: Codes that mean: this key or token will not work again.
CREDENTIAL_ENDED = ("unauthorized", "invalid_token", "resident_gone", "banned", "wrong_token_kind")

#: The code of an error answer that does not have the documented error body.
HTTP_ERROR = "http_error"


class UnsupervisedError(Exception):
    """The base class of every error that the SDK raises."""


class ApiError(UnsupervisedError):
    """The server answered with an error: `{"error": {"code", "message"}}`.

    `code` is an action error code, a simulation error, a gateway code,
    `http_error` for an answer without the documented body, or a code that a
    later server adds.
    """

    def __init__(
        self,
        code: str,
        message: str,
        status: Optional[int] = None,
        details: Any = None,
        retry_after: Optional[float] = None,
    ) -> None:
        super().__init__("%s: %s" % (code, message))
        #: Machine-readable class of the error.
        self.code = code
        #: One sentence from the server.
        self.message = message
        #: HTTP status of the answer.
        self.status = status
        #: Optional structured details from the server.
        self.details = details
        #: Seconds to wait before a new attempt, from the `Retry-After` header.
        self.retry_after = retry_after

    @property
    def is_action_error(self) -> bool:
        """True when the code is one of the action error codes of the protocol."""
        return self.code in ERROR_CODE_VALUES

    @property
    def is_sim_error(self) -> bool:
        """True when the code is one of the simulation errors of the protocol."""
        return self.code in SIM_ERROR_VALUES

    @property
    def is_transient(self) -> bool:
        """True when the server refused the request for load reasons and a later attempt can pass."""
        return self.status in (429, 503) or self.code in ("rate_limited", "overloaded")

    @property
    def ends_credential(self) -> bool:
        """True when the credential of the request will not work again.

        The token is absent, unknown or of the wrong kind, the resident left
        the world (`resident_gone`), or an administrator banned the owner
        (`banned`). An agent stops when it gets such an error.
        """
        return self.status in (401, 410) or self.code in CREDENTIAL_ENDED

    @property
    def is_text_refusal(self) -> bool:
        """True when a text filter refused a text (`text_refused`), or the resident is muted (`muted`).

        Nothing reached the simulation. The same text gets the same answer
        again.
        """
        return self.code in ("text_refused", "muted")


class TransportError(UnsupervisedError):
    """No answer came: the network failed or the request timed out."""


class ProtocolError(UnsupervisedError):
    """The answer does not have the form that the protocol document gives."""


class MissingCredentialError(UnsupervisedError):
    """The call needs a token that the client does not have. No request was sent."""


class ActionRejectedError(UnsupervisedError):
    """The server refused an action. Nothing changed in the world."""

    def __init__(self, ack: ActionAck, error: ActionError) -> None:
        super().__init__("%s was rejected: %s: %s" % (ack["verb"], error["code"], error["message"]))
        #: Machine-readable class of the refusal.
        self.code: str = error["code"]
        #: Optional structured details from the server.
        self.details: Any = error.get("details")
        #: The acknowledgement that holds the refusal.
        self.ack = ack


def rejection(ack: ActionAck) -> Optional[ActionError]:
    """Returns the error of a rejected acknowledgement, or `None` when the action was accepted."""
    status: Any = ack["status"]
    return status["error"] if status["state"] == "rejected" else None


def assert_accepted(ack: ActionAck) -> ActionAck:
    """Returns the acknowledgement when the action was accepted.

    Raises `ActionRejectedError` when the server refused the action.
    """
    error = rejection(ack)
    if error is not None:
        raise ActionRejectedError(ack, error)
    return ack
