// The HTTPS transport: one JSON request with bearer authentication, typed
// errors and retry.

import { ApiError, ProtocolError, TransportError } from "./errors.js";
import type { ErrorBody } from "./types.js";

/** The `fetch` function that the transport uses. */
export type FetchLike = (input: string, init: RequestInit) => Promise<Response>;

/** Options of the transport. Every option has a default. */
export interface TransportOptions {
  /** The function that sends requests. Default: the global `fetch`. */
  fetch?: FetchLike;
  /** Greatest number of new attempts after the first one. Default: 3. */
  maxRetries?: number;
  /**
   * Milliseconds to wait before attempt `attempt` (from 1). `retryAfterSeconds`
   * is the `Retry-After` header of the answer, if there was one. Default:
   * the header when present, else 250 ms doubled for each attempt, at most 8
   * seconds, with a random part.
   */
  retryDelay?: (attempt: number, retryAfterSeconds: number | undefined) => number;
  /** Waits for a number of milliseconds. Default: a timer. */
  sleep?: (milliseconds: number) => Promise<void>;
  /** Milliseconds before one attempt fails without an answer. Default: 30,000. */
  timeoutMs?: number;
}

/** One request of the transport. */
export interface TransportRequest {
  /** HTTP method. */
  method: "GET" | "POST" | "PUT";
  /** Path from the base address, for example `/v1/observe`. */
  path: string;
  /** Query parameters. A parameter with the value `undefined` is left out. */
  query?: Record<string, string | number | undefined>;
  /** JSON body. */
  body?: unknown;
  /** Bearer token. */
  token?: string;
  /**
   * True when a repeat of the request cannot change the world twice. Such a
   * request is sent again after a network failure. A request that is not
   * idempotent is sent again only when the server refused it for load
   * reasons (HTTP 429 or 503).
   */
  idempotent: boolean;
  /**
   * `false`: the request goes out one time. No new attempt follows a
   * refusal for load or a lost answer. Use it for a request that must not
   * act twice and whose `Retry-After` can be hours. Default: `true`.
   */
  retry?: boolean;
  /** Extra milliseconds on top of `timeoutMs`, for a long poll. */
  extraTimeoutMs?: number;
  /** Stops the request when the caller gives up. */
  signal?: AbortSignal;
  /** Return the body as text and do not parse it. */
  text?: boolean;
}

const DEFAULT_MAX_RETRIES = 3;
const DEFAULT_TIMEOUT_MS = 30_000;
const BACKOFF_BASE_MS = 250;
const BACKOFF_MAX_MS = 8_000;

/** The default delay before a new attempt. */
export function defaultRetryDelay(attempt: number, retryAfterSeconds: number | undefined): number {
  if (retryAfterSeconds !== undefined) return retryAfterSeconds * 1000;
  const ceiling = Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * 2 ** (attempt - 1));
  return ceiling / 2 + Math.random() * (ceiling / 2);
}

function defaultSleep(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

/** Reads the `Retry-After` header as a number of seconds. */
function retryAfter(response: Response): number | undefined {
  const header = response.headers.get("retry-after");
  if (header === null) return undefined;
  const seconds = Number(header);
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : undefined;
}

/** Returns true when the value is the documented error body. */
function isErrorBody(value: unknown): value is { error: ErrorBody } {
  if (typeof value !== "object" || value === null) return false;
  const error = (value as { error?: unknown }).error;
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as ErrorBody).code === "string" &&
    typeof (error as ErrorBody).message === "string"
  );
}

/** Makes the error of an answer whose status is not a success. */
async function apiError(response: Response): Promise<ApiError> {
  const options = { status: response.status, retryAfterSeconds: retryAfter(response) };
  const text = await response.text().catch(() => "");
  try {
    const parsed: unknown = JSON.parse(text);
    if (isErrorBody(parsed)) return new ApiError(parsed.error, options);
  } catch {
    // The body is not JSON: a proxy or a framework wrote it.
  }
  const message = text.trim() === "" ? `HTTP ${response.status}` : `HTTP ${response.status}: ${text.trim().slice(0, 200)}`;
  return new ApiError({ code: "http_error", message }, options);
}

/** Sends JSON requests to one server. */
export class Transport {
  /** Address of the server, without a slash at the end. */
  readonly baseUrl: string;
  readonly #fetch: FetchLike;
  readonly #maxRetries: number;
  readonly #retryDelay: (attempt: number, retryAfterSeconds: number | undefined) => number;
  readonly #sleep: (milliseconds: number) => Promise<void>;
  readonly #timeoutMs: number;

  /** Makes a transport for the server at `baseUrl`. */
  constructor(baseUrl: string, options: TransportOptions = {}) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.#fetch = options.fetch ?? ((input, init) => fetch(input, init));
    this.#maxRetries = options.maxRetries ?? DEFAULT_MAX_RETRIES;
    this.#retryDelay = options.retryDelay ?? defaultRetryDelay;
    this.#sleep = options.sleep ?? defaultSleep;
    this.#timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  /** Returns the full address of a path with its query. */
  url(path: string, query?: Record<string, string | number | undefined>): string {
    const params = new URLSearchParams();
    for (const [name, value] of Object.entries(query ?? {})) {
      if (value !== undefined) params.set(name, String(value));
    }
    const suffix = params.size > 0 ? `?${params.toString()}` : "";
    return `${this.baseUrl}${path}${suffix}`;
  }

  /**
   * Sends a request and returns the parsed JSON answer.
   *
   * @throws ApiError when the server answers with an error.
   * @throws TransportError when no answer comes.
   * @throws ProtocolError when the answer is not JSON.
   */
  async request<T>(request: TransportRequest): Promise<T> {
    const url = this.url(request.path, request.query);
    const headers: Record<string, string> = { accept: request.text === true ? "text/plain, text/markdown" : "application/json" };
    if (request.token !== undefined) headers["authorization"] = `Bearer ${request.token}`;
    let body: string | undefined;
    if (request.body !== undefined) {
      headers["content-type"] = "application/json";
      // The same text goes out on every attempt, so the request ids repeat.
      body = JSON.stringify(request.body);
    }
    const timeout = this.#timeoutMs + (request.extraTimeoutMs ?? 0);

    for (let attempt = 0; ; attempt += 1) {
      const last = attempt >= this.#maxRetries || request.retry === false;
      let response: Response;
      try {
        const limit = AbortSignal.timeout(timeout);
        const signal = request.signal === undefined ? limit : AbortSignal.any([request.signal, limit]);
        const init: RequestInit = { method: request.method, headers, signal };
        if (body !== undefined) init.body = body;
        response = await this.#fetch(url, init);
      } catch (cause) {
        if (request.signal?.aborted === true) throw cause;
        if (request.idempotent && !last) {
          await this.#sleep(this.#retryDelay(attempt + 1, undefined));
          continue;
        }
        throw new TransportError(`${request.method} ${request.path} got no answer`, { cause });
      }
      if (response.ok) {
        const text = await response.text();
        if (request.text === true) return text as T;
        if (text === "") return undefined as T;
        try {
          return JSON.parse(text) as T;
        } catch (cause) {
          throw new ProtocolError(`${request.method} ${request.path} did not answer with JSON`, { cause });
        }
      }
      const error = await apiError(response);
      const refused = response.status === 429 || response.status === 503;
      const lost = response.status === 408 || response.status === 502 || response.status === 504;
      if (!last && (refused || (lost && request.idempotent))) {
        await this.#sleep(this.#retryDelay(attempt + 1, error.retryAfterSeconds));
        continue;
      }
      throw error;
    }
  }
}
