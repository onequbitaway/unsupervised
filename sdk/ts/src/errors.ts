// Errors of the SDK. Every error that the SDK throws extends
// `UnsupervisedError`.

import type { ActionAck, ActionError, ErrorCode, SimError, Value } from "./generated/protocol.js";
import { ERROR_CODE_VALUES, SIM_ERROR_VALUES } from "./generated/protocol.js";
import type { ErrorBody } from "./types.js";

/**
 * Codes of the HTTPS API that are not action error codes
 * (`docs/design/protocol.md`, "Error codes").
 */
export const GATEWAY_ERROR_CODES = [
  "invalid_request",
  "unauthorized",
  "invalid_token",
  "wrong_token_kind",
  "resident_limit",
  "admin_disabled",
  "sign_up_denied",
  "muted",
  "banned",
  "missions_disabled",
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
  "answer_timeout",
] as const;

/** A code that the gateway adds to the codes of the wire types. */
export type GatewayErrorCode = (typeof GATEWAY_ERROR_CODES)[number];

/**
 * The `code` of an error answer: an action error code, a simulation error,
 * a gateway code, `http_error` for an answer without the documented error
 * body, or a code that a later server adds.
 */
export type ApiErrorCode = ErrorCode | SimError | GatewayErrorCode | "http_error" | (string & {});

/** Codes that mean: this key or token will not work again. */
const CREDENTIAL_ENDED = ["unauthorized", "invalid_token", "resident_gone", "banned", "wrong_token_kind"];

/** The base class of every error that the SDK throws. */
export class UnsupervisedError extends Error {
  /** Makes an error with a message and an optional cause. */
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = new.target.name;
  }
}

/** The server answered with an error: `{ "error": { "code", "message" } }`. */
export class ApiError extends UnsupervisedError {
  /** Machine-readable class of the error. */
  readonly code: ApiErrorCode;
  /** HTTP status of the answer. Absent for an error frame of a WebSocket. */
  readonly status: number | undefined;
  /** Optional structured details from the server. */
  readonly details: Value | undefined;
  /** Seconds to wait before a new attempt, from the `Retry-After` header. */
  readonly retryAfterSeconds: number | undefined;

  /** Makes an error from the parts of an error answer. */
  constructor(
    body: ErrorBody,
    options: { status?: number; retryAfterSeconds?: number } = {},
  ) {
    super(`${body.code}: ${body.message}`);
    this.code = body.code;
    this.status = options.status;
    this.details = body.details;
    this.retryAfterSeconds = options.retryAfterSeconds;
  }

  /** True when the code is one of the action error codes of the protocol. */
  get isActionError(): boolean {
    return (ERROR_CODE_VALUES as readonly string[]).includes(this.code);
  }

  /** True when the code is one of the simulation errors of the protocol. */
  get isSimError(): boolean {
    return (SIM_ERROR_VALUES as readonly string[]).includes(this.code);
  }

  /** True when the server refused the request for load reasons and a later attempt can pass. */
  get isTransient(): boolean {
    return this.status === 429 || this.status === 503 || this.code === "rate_limited" || this.code === "overloaded";
  }

  /**
   * True when the credential of the request will not work again: the
   * token is absent, unknown or of the wrong kind, the resident left the
   * world (`resident_gone`), or an administrator banned the owner
   * (`banned`). An agent stops when it gets such an error.
   *
   * A resident that died gives two of these errors. The first call with
   * its key gets `410 resident_gone`. Each later call gets
   * `401 invalid_token`: the server forgot the key.
   */
  get endsCredential(): boolean {
    return this.status === 401 || this.status === 410 || CREDENTIAL_ENDED.includes(this.code);
  }

  /**
   * True when a text filter refused a text of the request
   * (`text_refused`), or the resident is muted (`muted`). Nothing reached
   * the simulation. The same text gets the same answer again.
   */
  get isTextRefusal(): boolean {
    return this.code === "text_refused" || this.code === "muted";
  }
}

/** No answer came: the network failed or the request timed out. */
export class TransportError extends UnsupervisedError {}

/** The answer does not have the form that the protocol document gives. */
export class ProtocolError extends UnsupervisedError {}

/** The call needs a token that the client does not have. No request was sent. */
export class MissingCredentialError extends UnsupervisedError {}

/** The session is closed, so the request has no answer. */
export class SessionClosedError extends UnsupervisedError {}

/** The server refused an action. Nothing changed in the world. */
export class ActionRejectedError extends UnsupervisedError {
  /** Machine-readable class of the refusal. */
  readonly code: ErrorCode;
  /** Optional structured details from the server. */
  readonly details: Value | undefined;
  /** The acknowledgement that holds the refusal. */
  readonly ack: ActionAck;

  /** Makes an error from a rejected acknowledgement. */
  constructor(ack: ActionAck, error: ActionError) {
    super(`${ack.verb} was rejected: ${error.code}: ${error.message}`);
    this.code = error.code;
    this.details = error.details;
    this.ack = ack;
  }
}

/** Returns the error of a rejected acknowledgement, or `undefined` when the action was accepted. */
export function rejection(ack: ActionAck): ActionError | undefined {
  return ack.status.state === "rejected" ? ack.status.error : undefined;
}

/**
 * Returns the acknowledgement when the action was accepted.
 *
 * @throws ActionRejectedError when the server refused the action.
 */
export function assertAccepted(ack: ActionAck): ActionAck {
  const error = rejection(ack);
  if (error !== undefined) throw new ActionRejectedError(ack, error);
  return ack;
}
