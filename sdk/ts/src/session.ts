// The WebSocket session: requests with answers, pushed notices and reports,
// and reconnect.
//
// The frames have no correlation id in the protocol document. The session
// reads the document this way:
//
// - The server answers the requests of one connection in order.
// - An `ack` answers the pending action with the same request id. The
//   session gives every action a request id.
// - An `observation` answers the oldest pending `observe`; a `pong` answers
//   the oldest pending `ping`.
// - An `error` answers the oldest pending request of any kind.
// - `subscribe` has no answer.

import { ApiError, MissingCredentialError, ProtocolError, SessionClosedError, TransportError } from "./errors.js";
import type {
  ActionAck,
  ActionReport,
  ActionRequest,
  Notice,
  Observation,
  ObserveOpts,
  ResidentId,
} from "./generated/protocol.js";
import { REPORT_KIND, reportKey, reportsOf } from "./observation.js";
import type { ClientFrame, Me, ServerFrame } from "./types.js";
import type { ActOptions, ObserveOptions, PlanStep } from "./client.js";
import type { Value } from "./generated/protocol.js";

/** The part of a WebSocket that the session uses. The global `WebSocket` of Node 22 fits. */
export interface WebSocketLike {
  /** Sends one text frame. */
  send(data: string): void;
  /** Closes the connection. */
  close(code?: number, reason?: string): void;
  /** Listens to `open`, `message`, `close` and `error`. */
  addEventListener(type: string, listener: (event: any) => void): void;
}

/**
 * Makes a WebSocket. The second argument carries the `Authorization`
 * header. The global `WebSocket` of Node 22 accepts it.
 */
export type WebSocketFactory = (url: string, headers: Record<string, string>) => WebSocketLike;

/** What the session needs from the client. */
export interface SessionHost {
  /** Address of the server. */
  readonly baseUrl: string;
  /** The resident key. */
  residentKey: string | undefined;
  /** The notice cursor, shared with the HTTPS calls of the client. */
  cursor: number;
  /** Id of the resident, when it is known. */
  resident: ResidentId | undefined;
  /** Returns a new idempotency key. */
  nextRequestId(): string;
  /** Checks the key over HTTPS. */
  me(): Promise<Me>;
}

/** State of a session. */
export type SessionState = "connecting" | "open" | "reconnecting" | "closed";

/** Options of a session. Every option has a default. */
export interface SessionOptions {
  /** Called for each notice, once, in order. */
  onNotice?: (notice: Notice) => void;
  /** Called for each action report, once. */
  onReport?: (report: ActionReport) => void;
  /** Called each time the connection opens, after the session caught up. */
  onOpen?: () => void;
  /** Called each time the connection closes. `final` is true when the session will not connect again. */
  onClose?: (info: { final: boolean }) => void;
  /** Called for an error that no request can carry. */
  onError?: (error: Error) => void;
  /** Push only notices of these kinds. An action report has the kind `report`. Default: every kind. */
  kinds?: string[];
  /** Connect again after the connection is lost. Default: true. */
  reconnect?: boolean;
  /** Milliseconds to wait before reconnect attempt `attempt` (from 1). Default: 500 ms doubled each time, at most 30 seconds, with a random part. */
  reconnectDelay?: (attempt: number) => number;
  /** Milliseconds before a request without an answer fails. Default: 30,000. */
  requestTimeoutMs?: number;
  /** Makes the WebSocket. Default: the global `WebSocket`. */
  webSocket?: WebSocketFactory;
}

interface Pending {
  expects: "ack" | "observation" | "pong";
  /** The action of an `ack`. */
  request?: ActionRequest;
  /** Pending acknowledgements with the same `plan` number go out in one frame. */
  plan?: number;
  /** The options of an `observation`. */
  opts?: ObserveOpts;
  /** The cursor that the `observe` request carried. */
  since?: number;
  resolve: (value: any) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 30_000;
const REQUEST_TIMEOUT_MS = 30_000;
/** How many delivered reports the session remembers to drop repeats. */
const REPORT_MEMORY = 128;

function defaultReconnectDelay(attempt: number): number {
  const ceiling = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** (attempt - 1));
  return ceiling / 2 + Math.random() * (ceiling / 2);
}

function defaultWebSocket(url: string, headers: Record<string, string>): WebSocketLike {
  // The WebSocket of Node takes an options object in the place of the
  // protocol list. The type of the browser API does not know that.
  const Constructor = WebSocket as unknown as new (url: string, options: { headers: Record<string, string> }) => WebSocketLike;
  return new Constructor(url, { headers });
}

/**
 * A WebSocket session of one resident.
 *
 * The server pushes notices and reports; the session gives each of them to
 * `onNotice` or `onReport` once. When the connection is lost, the session
 * connects again, sends the unanswered requests again with the same request
 * ids, and reads what it missed with the notice cursor.
 */
export class Session {
  /** Resolves when the first connection is open. Rejects when the session closes before that. */
  readonly ready: Promise<void>;
  readonly #host: SessionHost;
  readonly #options: SessionOptions;
  #state: SessionState = "connecting";
  #socket: WebSocketLike | undefined;
  #pending: Pending[] = [];
  #kinds: string[] | undefined;
  #attempts = 0;
  #everOpen = false;
  #plans = 0;
  #seenReports: string[] = [];
  #timer: ReturnType<typeof setTimeout> | undefined;
  #readyResolve!: () => void;
  #readyReject!: (error: Error) => void;

  /** Opens a session for the resident of `host`. `Client.connect` calls this. */
  constructor(host: SessionHost, options: SessionOptions = {}) {
    this.#host = host;
    this.#options = options;
    this.#kinds = options.kinds;
    this.ready = new Promise<void>((resolve, reject) => {
      this.#readyResolve = resolve;
      this.#readyReject = reject;
    });
    // A caller that does not await `ready` still gets `onError`.
    this.ready.catch(() => undefined);
    if (host.residentKey === undefined) {
      this.#finish(new MissingCredentialError("A session needs a resident key."));
    } else {
      this.#open();
    }
  }

  /** State of the session. */
  get state(): SessionState {
    return this.#state;
  }

  /** Asks for one action. Resolves with the acknowledgement. A refusal is not thrown. */
  act(verb: string, args?: Value, options: ActOptions = {}): Promise<ActionAck> {
    const request = this.#request({ verb, ...(args === undefined ? {} : { args }), ...options });
    return this.#enqueue({ expects: "ack", request });
  }

  /** Asks for several actions in order. Resolves with one acknowledgement for each step. */
  plan(steps: PlanStep[]): Promise<ActionAck[]> {
    this.#plans += 1;
    const plan = this.#plans;
    const acks = steps.map((step) => this.#enqueue<ActionAck>({ expects: "ack", request: this.#request(step), plan }, false));
    this.#flush();
    return Promise.all(acks);
  }

  /** Returns the current observation. It moves the cursor. */
  observe(options: ObserveOptions = {}): Promise<Observation> {
    const since = options.since ?? this.#host.cursor;
    const opts: ObserveOpts = { since_notice: since };
    if (options.detail !== undefined) opts.detail = options.detail;
    if (options.sections !== undefined) opts.sections = options.sections;
    return this.#enqueue({ expects: "observation", opts, since });
  }

  /**
   * Limits the pushed notices to these kinds. An action report has the kind
   * `report`. An empty list means every kind. The session sends the list
   * again after a reconnect.
   */
  subscribe(kinds: string[]): void {
    this.#kinds = kinds;
    if (this.#state === "open") this.#send({ op: "subscribe", kinds });
  }

  /** Sends a `ping`. Resolves when the `pong` arrives. */
  ping(): Promise<void> {
    return this.#enqueue({ expects: "pong" });
  }

  /** Closes the session. Requests without an answer fail with `SessionClosedError`. */
  close(): void {
    this.#finish(new SessionClosedError("The session was closed."));
  }

  // ---- Requests ---------------------------------------------------------------

  #request(step: PlanStep): ActionRequest {
    const request: ActionRequest = { id: step.id ?? this.#host.nextRequestId(), verb: step.verb };
    if (step.args !== undefined && step.args !== null) request.args = step.args;
    return request;
  }

  #enqueue<T>(entry: Pick<Pending, "expects" | "request" | "plan" | "opts" | "since">, flush = true): Promise<T> {
    if (this.#state === "closed") {
      return Promise.reject(new SessionClosedError("The session is closed."));
    }
    return new Promise<T>((resolve, reject) => {
      const pending: Pending = {
        ...entry,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.#drop(pending);
          reject(new TransportError(`The server did not answer the ${entry.expects} request in time.`));
        }, this.#options.requestTimeoutMs ?? REQUEST_TIMEOUT_MS),
      };
      this.#pending.push(pending);
      if (flush && this.#state === "open") this.#transmit([pending]);
    });
  }

  /** Sends the pending entries of the newest plan. */
  #flush(): void {
    if (this.#state !== "open") return;
    this.#transmit(this.#pending.filter((pending) => pending.plan === this.#plans));
  }

  /** Sends the frames of pending entries, in order. Entries of one plan share a frame. */
  #transmit(entries: Pending[]): void {
    const sent = new Set<number>();
    for (const entry of entries) {
      if (entry.expects === "pong") {
        this.#send({ op: "ping" });
      } else if (entry.expects === "observation") {
        this.#send({ op: "observe", opts: entry.opts ?? {} });
      } else if (entry.plan === undefined) {
        this.#send({ op: "act", request: entry.request as ActionRequest });
      } else if (!sent.has(entry.plan)) {
        sent.add(entry.plan);
        const requests = entries
          .filter((other) => other.plan === entry.plan)
          .map((other) => other.request as ActionRequest);
        this.#send({ op: "plan", requests });
      }
    }
  }

  #send(frame: ClientFrame): void {
    this.#socket?.send(JSON.stringify(frame));
  }

  #drop(pending: Pending): void {
    clearTimeout(pending.timer);
    const index = this.#pending.indexOf(pending);
    if (index >= 0) this.#pending.splice(index, 1);
  }

  #settle(pending: Pending | undefined, value: unknown): boolean {
    if (pending === undefined) return false;
    this.#drop(pending);
    pending.resolve(value);
    return true;
  }

  // ---- Connection -------------------------------------------------------------

  #open(): void {
    const url = `${this.#host.baseUrl.replace(/^http/, "ws")}/v1/ws`;
    const headers = { authorization: `Bearer ${this.#host.residentKey}` };
    let socket: WebSocketLike;
    try {
      socket = (this.#options.webSocket ?? defaultWebSocket)(url, headers);
    } catch (cause) {
      this.#finish(new TransportError("The WebSocket could not be made.", { cause }));
      return;
    }
    this.#socket = socket;
    let opened = false;
    const lost = (): void => {
      if (this.#socket !== socket) return;
      this.#socket = undefined;
      void this.#onClose(opened);
    };
    socket.addEventListener("open", () => {
      if (this.#socket !== socket) return;
      opened = true;
      this.#onOpen();
    });
    socket.addEventListener("message", (event: { data: unknown }) => {
      if (this.#socket === socket) this.#onMessage(event.data);
    });
    socket.addEventListener("close", lost);
    socket.addEventListener("error", () => {
      // A connection that is open gives `close` after `error`. A connection
      // that fails before it opens gives only `error` in Node 22, and
      // `error` then `close` in later versions. The first of them counts.
      if (opened || this.#socket !== socket) return;
      lost();
      try {
        // Node 22 leaves the failed socket in the state "connecting".
        socket.close();
      } catch {
        // A socket that cannot close is closed.
      }
    });
  }

  #onOpen(): void {
    const first = !this.#everOpen;
    this.#everOpen = true;
    this.#state = "open";
    this.#attempts = 0;
    if (this.#kinds !== undefined) this.#send({ op: "subscribe", kinds: this.#kinds });
    // Unanswered requests go out again. Their request ids make that safe.
    this.#transmit(this.#pending);
    if (first) {
      this.#readyResolve();
      this.#options.onOpen?.();
      return;
    }
    // Read what came while the connection was down.
    this.observe().then(
      () => this.#options.onOpen?.(),
      (error: Error) => this.#options.onError?.(error),
    );
  }

  async #onClose(opened: boolean): Promise<void> {
    if (this.#state === "closed") return;
    const again = this.#options.reconnect !== false;
    this.#state = "reconnecting";
    if (!opened) {
      // A WebSocket does not say why the server refused it. Ask over HTTPS.
      try {
        await this.#host.me();
      } catch (error) {
        const refused = error instanceof ApiError && (error.status === 401 || error.status === 403);
        if (refused || error instanceof MissingCredentialError) {
          this.#finish(error);
          return;
        }
      }
      if (this.#state !== "reconnecting") return;
    }
    if (!again) {
      this.#finish(new SessionClosedError("The connection was lost."));
      return;
    }
    this.#options.onClose?.({ final: false });
    this.#attempts += 1;
    const delay = (this.#options.reconnectDelay ?? defaultReconnectDelay)(this.#attempts);
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      if (this.#state === "reconnecting") this.#open();
    }, delay);
  }

  /** Ends the session for good. */
  #finish(error: Error): void {
    if (this.#state === "closed") return;
    this.#state = "closed";
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    const socket = this.#socket;
    this.#socket = undefined;
    socket?.close(1000, "closed by the client");
    for (const pending of this.#pending.splice(0)) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#readyReject(error);
    if (!(error instanceof SessionClosedError)) this.#options.onError?.(error);
    this.#options.onClose?.({ final: true });
  }

  // ---- Frames -----------------------------------------------------------------

  #onMessage(data: unknown): void {
    if (typeof data !== "string") {
      this.#options.onError?.(new ProtocolError("The session reads text frames only."));
      return;
    }
    let frame: ServerFrame;
    try {
      frame = JSON.parse(data) as ServerFrame;
    } catch (cause) {
      this.#options.onError?.(new ProtocolError("A frame was not JSON.", { cause }));
      return;
    }
    switch (frame.op) {
      case "ack": {
        const id = frame.ack.request_id;
        const acks = this.#pending.filter((pending) => pending.expects === "ack");
        const match = id == null ? acks[0] : acks.find((pending) => pending.request?.id === id);
        // An ack without a pending request repeats an answer that came before a reconnect.
        this.#settle(match, frame.ack);
        return;
      }
      case "observation": {
        const pending = this.#pending.find((entry) => entry.expects === "observation");
        const observation = frame.observation;
        this.#deliver(observation, pending?.since ?? this.#host.cursor);
        this.#settle(pending, observation);
        return;
      }
      case "pong":
        this.#settle(this.#pending.find((entry) => entry.expects === "pong"), undefined);
        return;
      case "notice":
        this.#notice(frame.notice);
        return;
      case "report":
        this.#report(frame.report);
        return;
      case "error": {
        const error = new ApiError(frame.error);
        const pending = this.#pending[0];
        if (pending === undefined) {
          this.#options.onError?.(error);
        } else {
          this.#drop(pending);
          pending.reject(error);
        }
        return;
      }
      default:
        this.#options.onError?.(new ProtocolError(`The frame \`${String((frame as { op?: unknown }).op)}\` is not known.`));
    }
  }

  /** Gives the news of an observation to the handlers and moves the cursor. */
  #deliver(observation: Observation, since: number): void {
    if (typeof observation?.next_notice !== "number" || !Array.isArray(observation.notices)) {
      this.#options.onError?.(new ProtocolError("An `observation` frame did not hold an observation."));
      return;
    }
    observation.notices = observation.notices.filter((notice) => notice.seq >= since);
    for (const notice of observation.notices) this.#notice(notice);
    for (const report of reportsOf(observation)) this.#report(report);
    this.#host.cursor = observation.next_notice;
    this.#host.resident = observation.resident;
  }

  /** Returns true when the handlers want notices of this kind. */
  #wants(kind: string): boolean {
    return this.#kinds === undefined || this.#kinds.length === 0 || this.#kinds.includes(kind);
  }

  #notice(notice: Notice): void {
    // The cursor is the next sequence number that the session has not seen.
    if (notice.seq < this.#host.cursor) return;
    this.#host.cursor = notice.seq + 1;
    if (this.#wants(notice.kind)) this.#options.onNotice?.(notice);
  }

  #report(report: ActionReport): void {
    const key = reportKey(report);
    if (this.#seenReports.includes(key)) return;
    this.#seenReports.push(key);
    if (this.#seenReports.length > REPORT_MEMORY) this.#seenReports.shift();
    if (this.#wants(REPORT_KIND)) this.#options.onReport?.(report);
  }
}
