// The WebSocket session: requests with answers, pushed notices and reports,
// and reconnect (`docs/design/protocol.md`, "WebSocket frames (agents)").
//
// The frames have no correlation id. The session uses these rules of the
// server:
//
// - The server answers the requests of one connection in order.
// - An `ack` answers the pending action with the same request id. The
//   session gives every action a request id.
// - A plan gets one `ack` for each request up to the first refusal, and
//   nothing for the requests after it.
// - An `observation` answers the oldest pending `observe`; a `pong` answers
//   the oldest pending `ping`.
// - An `error` answers the oldest pending request of any kind.
// - `subscribe` has no answer.
// - A report and a notice have a `seq` of one shared sequence. The session
//   keeps the last `seq` of each of the two. After a lost connection it
//   connects again with `since` and `reports_since`, so the server pushes
//   what the session missed, and nothing a second time.

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
import { REPORT_KIND, reportsOf } from "./observation.js";
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
  /** Called for each notice, once, in the order of `seq`. */
  onNotice?: (notice: Notice) => void;
  /** Called for each action report, once, in the order of `seq`. */
  onReport?: (report: ActionReport) => void;
  /** Called each time the connection opens. */
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
  /** The actions of an `ack` entry: one for `act`, all steps for `plan`. */
  requests?: ActionRequest[];
  /** True when the entry is a plan: it goes out as one `plan` frame. */
  plan?: boolean;
  /** The acknowledgements that came so far, in the order of `requests`. */
  acks?: ActionAck[];
  /** The options of an `observation`. */
  opts?: ObserveOpts;
  /** True for the observation that the session asks for itself, to learn where the inbox ends. */
  internal?: boolean;
  resolve: (value: any) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 30_000;
const REQUEST_TIMEOUT_MS = 30_000;
/** Codes of an `error` frame that end the session: the key will not work again. */
const FATAL_CODES = ["resident_gone", "banned", "unauthorized", "invalid_token"];
/** A cursor behind every sequence number: an observation with it has no news. */
const NO_NEWS = Number.MAX_SAFE_INTEGER;

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
 * connects again with its two cursors, sends the unanswered requests again
 * with the same request ids, and gets what it missed.
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
  /** Where the inbox ended when the session started: the cursor for news that no push carried yet. */
  #base: number | undefined;
  /** The `seq` of the last notice that the session took. */
  #lastNotice: number | undefined;
  /** The `seq` of the last report that the session took. */
  #lastReport: number | undefined;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #readyResolve!: () => void;
  #readyReject!: (error: Error) => void;

  /** Opens a session for the resident of `host`. `Client.connect` calls this. */
  constructor(host: SessionHost, options: SessionOptions = {}) {
    this.#host = host;
    this.#options = options;
    this.#kinds = options.kinds;
    // A client that observed before has a cursor: the session starts there.
    if (host.cursor > 0) this.#base = host.cursor;
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

  /**
   * The cursors that the next connection sends: `since` is the `seq` of the
   * last notice plus 1, and `reportsSince` is the `seq` of the last report
   * plus 1. `undefined` while the session does not know where the inbox
   * ends.
   */
  get cursors(): { since: number; reportsSince: number } | undefined {
    const since = this.#lastNotice === undefined ? this.#base : this.#lastNotice + 1;
    const reportsSince = this.#lastReport === undefined ? this.#base : this.#lastReport + 1;
    return since === undefined || reportsSince === undefined ? undefined : { since, reportsSince };
  }

  /** Asks for one action. Resolves with the acknowledgement. A refusal is not thrown. */
  async act(verb: string, args?: Value, options: ActOptions = {}): Promise<ActionAck> {
    const request = this.#request({ verb, ...(args === undefined ? {} : { args }), ...options });
    const acks = await this.#enqueue<ActionAck[]>({ expects: "ack", requests: [request], acks: [] });
    return acks[0] as ActionAck;
  }

  /**
   * Asks for several actions in order. Resolves with one acknowledgement
   * for each step that the server sent to the simulation. The server stops
   * after the first step that the simulation rejects: the list then ends
   * with that refusal.
   */
  plan(steps: PlanStep[]): Promise<ActionAck[]> {
    if (steps.length === 0) return Promise.resolve([]);
    const requests = steps.map((step) => this.#request(step));
    return this.#enqueue<ActionAck[]>({ expects: "ack", requests, plan: true, acks: [] });
  }

  /**
   * Returns the current observation. Its notices and reports go to the
   * handlers too, unless a push gave them already.
   */
  observe(options: ObserveOptions = {}): Promise<Observation> {
    const opts: ObserveOpts = { since_notice: options.since ?? this.#host.cursor };
    if (options.detail !== undefined) opts.detail = options.detail;
    if (options.sections !== undefined) opts.sections = options.sections;
    return this.#enqueue({ expects: "observation", opts });
  }

  /**
   * Limits the pushed notices to these kinds. An action report has the kind
   * `report`. An empty list means every kind. The session sends the list
   * again after a reconnect.
   */
  subscribe(kinds: string[]): void {
    this.#kinds = kinds.length === 0 ? undefined : kinds;
    if (this.#state === "open") this.#sendSubscription();
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

  #enqueue<T>(entry: Omit<Pending, "resolve" | "reject" | "timer">): Promise<T> {
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
      if (this.#state === "open") this.#transmit(pending);
    });
  }

  /** Sends the frame of one pending entry. */
  #transmit(entry: Pending): void {
    if (entry.expects === "pong") {
      this.#send({ op: "ping" });
    } else if (entry.expects === "observation") {
      this.#send({ op: "observe", opts: entry.opts ?? {} });
    } else if (entry.plan === true) {
      this.#send({ op: "plan", requests: entry.requests as ActionRequest[] });
    } else {
      this.#send({ op: "act", request: (entry.requests as ActionRequest[])[0] as ActionRequest });
    }
  }

  #send(frame: ClientFrame): void {
    this.#socket?.send(JSON.stringify(frame));
  }

  /** Sends the kinds that the caller chose. Without kinds, the server pushes everything. */
  #sendSubscription(): void {
    this.#send(this.#kinds === undefined ? { op: "subscribe" } : { op: "subscribe", kinds: this.#kinds });
  }

  #drop(pending: Pending): void {
    clearTimeout(pending.timer);
    const index = this.#pending.indexOf(pending);
    if (index >= 0) this.#pending.splice(index, 1);
  }

  #settle(pending: Pending | undefined, value: unknown): void {
    if (pending === undefined) return;
    this.#drop(pending);
    pending.resolve(value);
  }

  // ---- Connection -------------------------------------------------------------

  #open(): void {
    const cursors = this.cursors;
    const query = cursors === undefined ? "" : `?since=${cursors.since}&reports_since=${cursors.reportsSince}`;
    const url = `${this.#host.baseUrl.replace(/^http/, "ws")}/v1/ws${query}`;
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
    if (this.#kinds !== undefined) this.#sendSubscription();
    // Unanswered requests go out again, in their order. Their request ids
    // make that safe.
    for (const entry of this.#pending) this.#transmit(entry);
    // Without a cursor the server pushes only the news that comes from now
    // on. The session asks where the inbox ends, so that a later connection
    // has a cursor. The answer has no news: the cursor of the request is
    // behind every number.
    if (this.cursors === undefined) {
      const learn = this.#enqueue<Observation>({
        expects: "observation",
        opts: { since_notice: NO_NEWS, sections: [] },
        internal: true,
      });
      learn.catch((error: Error) => {
        if (!(error instanceof SessionClosedError)) this.#options.onError?.(error);
      });
    }
    if (first) this.#readyResolve();
    this.#options.onOpen?.();
  }

  async #onClose(opened: boolean): Promise<void> {
    if (this.#state === "closed") return;
    const again = this.#options.reconnect !== false;
    this.#state = "reconnecting";
    // The observation that the session asked for itself has no caller. The
    // next connection asks again.
    for (const entry of this.#pending.filter((pending) => pending.internal === true)) this.#drop(entry);
    if (!opened) {
      // A WebSocket does not say why the server refused it. Ask over HTTPS.
      try {
        await this.#host.me();
      } catch (error) {
        const refused = error instanceof ApiError && (error.endsCredential || error.status === 403);
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
      case "ack":
        this.#ack(frame.ack);
        return;
      case "observation": {
        const pending = this.#pending.find((entry) => entry.expects === "observation");
        const observation = frame.observation;
        if (typeof observation?.next_notice !== "number" || !Array.isArray(observation.notices)) {
          this.#options.onError?.(new ProtocolError("An `observation` frame did not hold an observation."));
          return;
        }
        if (pending?.internal === true) {
          // Only the end of the inbox counts. The pushes carry the news.
          this.#base ??= observation.next_notice;
        } else {
          this.#deliver(observation);
        }
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
        const pending = this.#pending.find((entry) => entry.internal !== true);
        if (pending !== undefined) {
          this.#drop(pending);
          pending.reject(error);
        }
        if (FATAL_CODES.includes(error.code)) {
          // The server closes the connection, and no new one can open.
          this.#finish(error);
        } else if (pending === undefined) {
          this.#options.onError?.(error);
        }
        return;
      }
      default:
        this.#options.onError?.(new ProtocolError(`The frame \`${String((frame as { op?: unknown }).op)}\` is not known.`));
    }
  }

  /** Takes one acknowledgement: finds its request among the pending actions. */
  #ack(ack: ActionAck): void {
    const id = ack.request_id;
    const entries = this.#pending.filter((pending) => pending.expects === "ack");
    const entry = id == null ? entries[0] : entries.find((pending) => pending.requests?.some((request) => request.id === id));
    // An ack without a pending request repeats an answer that came before a reconnect.
    if (entry === undefined) return;
    const requests = entry.requests as ActionRequest[];
    const acks = entry.acks as ActionAck[];
    const index = id == null ? acks.length : requests.findIndex((request) => request.id === id);
    acks[index] = ack;
    // The server stops a plan after the first refusal: no more acks come.
    const complete = index === requests.length - 1 || ack.status.state === "rejected";
    if (complete) this.#settle(entry, acks.slice(0, index + 1));
  }

  /** Gives the news of an observation to the handlers, unless a push gave it already. */
  #deliver(observation: Observation): void {
    for (const report of reportsOf(observation)) this.#report(report);
    for (const notice of observation.notices) this.#notice(notice);
    this.#host.cursor = Math.max(this.#host.cursor, observation.next_notice);
    this.#host.resident = observation.resident;
  }

  /** Returns true when the handlers want notices of this kind. */
  #wants(kind: string): boolean {
    return this.#kinds === undefined || this.#kinds.includes(kind);
  }

  #notice(notice: Notice): void {
    // The notices come in the order of `seq`. A lower number came already.
    if (this.#lastNotice !== undefined && notice.seq <= this.#lastNotice) return;
    this.#lastNotice = notice.seq;
    this.#host.cursor = Math.max(this.#host.cursor, notice.seq + 1);
    if (this.#wants(notice.kind)) this.#options.onNotice?.(notice);
  }

  #report(report: ActionReport): void {
    const seq = report.seq ?? 0;
    // The reports come in the order of `seq`. A lower number came already.
    if (this.#lastReport !== undefined && seq <= this.#lastReport) return;
    this.#lastReport = seq;
    this.#host.cursor = Math.max(this.#host.cursor, seq + 1);
    if (this.#wants(REPORT_KIND)) this.#options.onReport?.(report);
  }
}
