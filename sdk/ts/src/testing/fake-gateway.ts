// The HTTPS routes and the WebSocket frames of the fake world.
//
// `FakeGateway.handle` answers one HTTP request without a network. The
// `fetch` property has the form of the global `fetch`, so a `Client` can use
// it: `new Client({ baseUrl: "http://fake", fetch: gateway.fetch })`.
//
// Routes: `/healthz`, `/join.md`, `/v1/catalogue`, `/v1/world`,
// `/v1/world/zones/{id}`, `/v1/owners`, `/v1/residents`,
// `/v1/residents/{id}/keys`, `/v1/me`, `/v1/act`, `/v1/observe`, `/v1/wait`,
// `/v1/routines`, `/v1/webhook`. Every other route answers 404.

import type { ActionRequest, Detail, ObserveOpts, Value, WorldView } from "../generated/protocol.js";
import type { FetchLike } from "../http.js";
import type { ClientFrame, ErrorBody, ServerFrame } from "../types.js";
import { FakeWorld, byteLength, type FakeResident, type InboxEntry } from "./fake-world.js";

/** One HTTP request for the fake gateway. */
export interface FakeRequest {
  method: string;
  /** Path and query, for example `/v1/observe?since=3`. */
  url: string;
  /** Headers with lowercase names. */
  headers?: Record<string, string>;
  body?: string | undefined;
  signal?: AbortSignal | undefined;
}

/** One HTTP answer of the fake gateway. */
export interface FakeResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/** A request that the gateway got, for assertions. */
export interface LoggedRequest {
  method: string;
  path: string;
  query: Record<string, string>;
  token: string | undefined;
  body: unknown;
}

/** An answer that the gateway gives in the place of the real one. */
export interface Fault {
  status: number;
  /** Error code of the body. Default: `overloaded`. */
  code?: string;
  message?: string;
  headers?: Record<string, string>;
  /** A body that is not the documented error body. */
  body?: string;
  /** How many requests get the fault. Default: 1. */
  times?: number;
  /** Only requests that pass this test get the fault. */
  when?: (request: LoggedRequest) => boolean;
}

/** One WebSocket connection of the fake gateway. */
export interface FakeSocket {
  /** Gives the gateway a text frame from the client. */
  receive(text: string): void;
  /** Ends the connection. */
  close(): void;
}

class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

const NAME_MAX = 64;
const WAIT_MAX_SECONDS = 60;
const JSON_HEADERS = { "content-type": "application/json" };
/** HTTP status of an action error on a route that is not `/v1/act`. */
const ACTION_ERROR_STATUS: Record<string, number> = { blocked: 403, rate_limited: 429 };

function json(status: number, body: unknown, headers: Record<string, string> = {}): FakeResponse {
  return { status, headers: { ...JSON_HEADERS, ...headers }, body: JSON.stringify(body) };
}

function errorResponse(status: number, error: ErrorBody, headers: Record<string, string> = {}): FakeResponse {
  return json(status, { error }, headers);
}

function integer(query: URLSearchParams, name: string): number | undefined {
  const text = query.get(name);
  if (text === null) return undefined;
  if (!/^\d+$/.test(text)) throw new HttpError(400, "invalid", `\`${name}\` must be a whole number.`);
  return Number(text);
}

function list(query: URLSearchParams, name: string): string[] | undefined {
  const text = query.get(name);
  return text === null || text === "" ? undefined : text.split(",");
}

function observeOpts(query: URLSearchParams): ObserveOpts {
  const opts: ObserveOpts = {};
  const detail = query.get("detail");
  if (detail !== null) {
    if (detail !== "brief" && detail !== "full") {
      throw new HttpError(400, "invalid", "`detail` is `brief` or `full`.");
    }
    opts.detail = detail as Detail;
  }
  const since = integer(query, "since");
  if (since !== undefined) opts.since_notice = since;
  const sections = list(query, "sections");
  if (sections !== undefined) opts.sections = sections;
  return opts;
}

/** The gateway of a fake world. */
export class FakeGateway {
  /** The world behind the gateway. */
  readonly world: FakeWorld;
  /** Every request that came in, in order. */
  readonly requests: LoggedRequest[] = [];
  /** The text of `GET /join.md`. */
  joinDocument = "# Join\n";
  /** True: `POST /v1/act` does not read `since` and answers with the whole inbox. */
  actIgnoresSince = false;
  #faults: Fault[] = [];
  #frameFaults: ErrorBody[] = [];

  /** Makes a gateway for a world. A new world when none is given. */
  constructor(world: FakeWorld = new FakeWorld()) {
    this.world = world;
  }

  /** A `fetch` function that answers from this gateway. */
  readonly fetch: FetchLike = async (input, init) => {
    const url = new URL(input);
    const headers: Record<string, string> = {};
    new Headers(init.headers).forEach((value, name) => {
      headers[name.toLowerCase()] = value;
    });
    const answer = await this.handle({
      method: init.method ?? "GET",
      url: `${url.pathname}${url.search}`,
      headers,
      body: typeof init.body === "string" ? init.body : undefined,
      signal: init.signal ?? undefined,
    });
    return new Response(answer.body, { status: answer.status, headers: answer.headers });
  };

  /** Makes the next matching request (or requests) fail with the given answer. */
  failNext(fault: Fault): void {
    this.#faults.push({ ...fault });
  }

  /** Makes the next WebSocket frame of a client get an `error` frame. */
  failNextFrame(error: ErrorBody): void {
    this.#frameFaults.push(error);
  }

  /** Answers one HTTP request. */
  async handle(request: FakeRequest): Promise<FakeResponse> {
    const url = new URL(request.url, "http://fake");
    const authorization = request.headers?.["authorization"];
    const token = authorization?.startsWith("Bearer ") === true ? authorization.slice(7).trim() : undefined;
    let body: unknown;
    let badBody = false;
    if (request.body !== undefined && request.body !== "") {
      try {
        body = JSON.parse(request.body);
      } catch {
        badBody = true;
      }
    }
    const logged: LoggedRequest = {
      method: request.method,
      path: url.pathname,
      query: Object.fromEntries(url.searchParams),
      token,
      body,
    };
    this.requests.push(logged);

    const fault = this.#faults.find((candidate) => candidate.when === undefined || candidate.when(logged));
    if (fault !== undefined) {
      fault.times = (fault.times ?? 1) - 1;
      if (fault.times <= 0) this.#faults.splice(this.#faults.indexOf(fault), 1);
      if (fault.body !== undefined) {
        return { status: fault.status, headers: { "content-type": "text/plain", ...fault.headers }, body: fault.body };
      }
      const error = { code: fault.code ?? "overloaded", message: fault.message ?? "The fake gateway refused the request." };
      return errorResponse(fault.status, error, fault.headers);
    }
    try {
      if (badBody) throw new HttpError(400, "invalid", "The body is not JSON.");
      return await this.#route(logged, url.searchParams, request.signal);
    } catch (error) {
      if (error instanceof HttpError) {
        return errorResponse(error.status, { code: error.code, message: error.message });
      }
      throw error;
    }
  }

  async #route(request: LoggedRequest, query: URLSearchParams, signal: AbortSignal | undefined): Promise<FakeResponse> {
    const { method, path } = request;
    const route = `${method} ${path}`;
    const world = this.world;
    const keys = /^\/v1\/residents\/(\d+)\/keys$/.exec(path);
    const zone = /^\/v1\/world\/zones\/(\d+)$/.exec(path);

    if (route === "GET /healthz") return { status: 200, headers: { "content-type": "text/plain" }, body: "ok" };
    if (route === "GET /join.md") {
      return { status: 200, headers: { "content-type": "text/markdown" }, body: this.joinDocument };
    }
    if (route === "GET /v1/catalogue") return json(200, world.catalogue());
    if (route === "GET /v1/world") return json(200, this.#layout());
    if (method === "GET" && zone !== null) {
      if (zone[1] !== "0") throw new HttpError(404, "not_found", `No zone has the id ${zone[1]}.`);
      return json(200, this.#zone());
    }

    if (route === "POST /v1/owners") return json(201, world.signUp());
    if (route === "POST /v1/residents") {
      const owner = this.#owner(request.token);
      const spec = (request.body ?? {}) as { name?: unknown; model?: unknown; city?: unknown };
      if (typeof spec.name !== "string" || spec.name === "" || byteLength(spec.name) > NAME_MAX) {
        throw new HttpError(400, "invalid", `\`name\` must have 1 to ${NAME_MAX} bytes.`);
      }
      if (spec.model !== undefined && (typeof spec.model !== "string" || byteLength(spec.model) > NAME_MAX)) {
        throw new HttpError(400, "invalid", `\`model\` must have at most ${NAME_MAX} bytes.`);
      }
      if (spec.city !== undefined && spec.city !== 0) {
        throw new HttpError(400, "invalid", "The fake world has one city: 0.");
      }
      const resident = world.spawn(owner, {
        name: spec.name,
        ...(spec.model === undefined ? {} : { model: spec.model }),
      });
      return json(201, { resident: resident.id, key: resident.key });
    }
    if (route === "GET /v1/residents") {
      const owner = this.#owner(request.token);
      const residents = [...world.residents.values()]
        .filter((resident) => resident.owner === owner)
        .map((resident) => ({
          id: resident.id,
          name: resident.name,
          ...(resident.model === undefined ? {} : { model: resident.model }),
        }));
      return json(200, { residents });
    }
    if (method === "POST" && keys !== null) {
      const owner = this.#owner(request.token);
      const resident = world.residents.get(Number(keys[1]));
      if (resident === undefined) throw new HttpError(404, "unknown_resident", "No living resident has this id.");
      if (resident.owner !== owner) throw new HttpError(403, "forbidden", "The resident belongs to another owner.");
      return json(200, { resident: resident.id, key: world.rotateKey(resident) });
    }

    if (route === "GET /v1/me") {
      const resident = this.#resident(request.token);
      return json(200, { resident: resident.id, name: resident.name });
    }
    if (route === "POST /v1/act") {
      const resident = this.#resident(request.token);
      const body = request.body as { plan?: unknown } | undefined;
      const requests = Array.isArray(body?.plan) ? body.plan : [body];
      for (const one of requests) {
        if (typeof (one as ActionRequest | undefined)?.verb !== "string") {
          throw new HttpError(400, "invalid", "An action request needs a `verb`.");
        }
      }
      const acks = (requests as ActionRequest[]).map((one) => world.submit(resident, one));
      const opts = this.actIgnoresSince ? {} : observeOpts(query);
      return json(200, { acks, observation: world.observe(resident, opts) });
    }
    if (route === "GET /v1/observe") {
      return json(200, world.observe(this.#resident(request.token), observeOpts(query)));
    }
    if (route === "GET /v1/wait") {
      const resident = this.#resident(request.token);
      const timeout = integer(query, "timeout") ?? WAIT_MAX_SECONDS;
      if (timeout > WAIT_MAX_SECONDS) {
        throw new HttpError(400, "invalid", `\`timeout\` is at most ${WAIT_MAX_SECONDS} seconds.`);
      }
      const since = integer(query, "since");
      const kinds = list(query, "kinds");
      const waiting = world.wait(resident, {
        timeout,
        ...(since === undefined ? {} : { since }),
        ...(kinds === undefined ? {} : { kinds }),
      });
      if (signal === undefined) return json(200, await waiting);
      const aborted = new Promise<never>((_resolve, reject) => {
        signal.addEventListener("abort", () => {
          world.abandonWaits(resident);
          reject(signal.reason as Error);
        });
      });
      return json(200, await Promise.race([waiting, aborted]));
    }
    if (route === "PUT /v1/routines") {
      const resident = this.#resident(request.token);
      const routines = (request.body as { routines?: unknown } | undefined)?.routines;
      if (!Array.isArray(routines)) throw new HttpError(400, "invalid", "The body needs `routines`: a list.");
      const ack = world.submit(resident, { verb: "set_routines", args: { routines: routines as Value } });
      if (ack.status.state === "rejected") {
        const { code, message } = ack.status.error;
        throw new HttpError(ACTION_ERROR_STATUS[code] ?? 400, code, message);
      }
      return json(200, ack.status.state === "done" ? (ack.status.result ?? null) : null);
    }
    if (route === "PUT /v1/webhook") {
      const resident = this.#resident(request.token);
      const target = (request.body as { url?: unknown } | undefined)?.url;
      if (target !== null && (typeof target !== "string" || !/^https?:\/\//.test(target))) {
        throw new HttpError(400, "invalid", "`url` must be an http or https address, or null.");
      }
      const secret = world.setWebhook(resident, target);
      return json(200, secret === undefined ? { url: null } : { url: target, secret });
    }
    throw new HttpError(404, "not_found", `The fake gateway has no route ${route}.`);
  }

  #owner(token: string | undefined): number {
    const owner = this.world.ownerOf(token);
    if (owner !== undefined) return owner;
    if (this.world.residentOf(token) !== undefined) {
      throw new HttpError(403, "forbidden", "This route needs an owner token, not a resident key.");
    }
    throw new HttpError(401, "unauthorized", "Send an owner token: `Authorization: Bearer <token>`.");
  }

  #resident(key: string | undefined): FakeResident {
    const resident = this.world.residentOf(key);
    if (resident !== undefined) return resident;
    if (this.world.ownerOf(key) !== undefined) {
      throw new HttpError(403, "forbidden", "This route needs a resident key, not an owner token.");
    }
    throw new HttpError(401, "unauthorized", "Send a resident key: `Authorization: Bearer <key>`.");
  }

  #zoneView(): { id: number; city: number; name: string; bounds: { x0: number; y0: number; x1: number; y1: number }; residents: number } {
    return {
      id: 0,
      city: 0,
      name: "Fake District",
      bounds: { x0: 0, y0: 0, x1: 64, y1: 64 },
      residents: this.world.residents.size,
    };
  }

  #layout(): WorldView {
    const zone = this.#zoneView();
    return {
      type: "layout",
      frame: this.world.tick,
      clock: this.world.clock(),
      cities: [{ id: 0, name: "Fake City", bounds: zone.bounds, zones: [0] }],
      zones: [zone],
    };
  }

  #zone(): WorldView {
    const zone = this.#zoneView();
    const side = zone.bounds.x1 - zone.bounds.x0;
    return {
      type: "zone",
      zone,
      width: side,
      height: side,
      tiles: new Array<number>(side * side).fill(0),
      sites: this.world.sites,
      residents: [...this.world.residents.values()].map((resident) => ({
        id: resident.id,
        name: resident.name,
        pos: resident.pos,
        ...(resident.current === undefined ? {} : { activity: resident.current.verb }),
        ...(resident.model === undefined ? {} : { model: resident.model }),
      })),
    };
  }

  // ---- WebSocket ------------------------------------------------------------

  /**
   * Opens a WebSocket connection for the resident of `key`. `send` gets
   * every text frame for the client. Returns `undefined` when the key is
   * not known; a real server then refuses the upgrade with HTTP 401.
   */
  connect(key: string | undefined, send: (text: string) => void): FakeSocket | undefined {
    const resident = this.world.residentOf(key);
    if (resident === undefined) return undefined;
    const world = this.world;
    let kinds: string[] | undefined;
    const emit = (frame: ServerFrame): void => send(JSON.stringify(frame));
    const wants = (kind: string): boolean => kinds === undefined || kinds.length === 0 || kinds.includes(kind);
    const stop = world.listen(resident.id, (entry: InboxEntry) => {
      if ("notice" in entry) {
        if (wants(entry.notice.kind)) emit({ op: "notice", notice: entry.notice });
      } else if (wants("report")) {
        emit({ op: "report", report: entry.report });
      }
    });
    const invalid = (message: string): void => emit({ op: "error", error: { code: "invalid", message } });
    return {
      close: () => {
        stop();
      },
      receive: (text: string) => {
        const fault = this.#frameFaults.shift();
        if (fault !== undefined) {
          emit({ op: "error", error: fault });
          return;
        }
        let frame: ClientFrame;
        try {
          frame = JSON.parse(text) as ClientFrame;
        } catch {
          invalid("The frame is not JSON.");
          return;
        }
        switch (frame.op) {
          case "act":
            emit({ op: "ack", ack: world.submit(resident, frame.request) });
            return;
          case "plan":
            for (const request of frame.requests) emit({ op: "ack", ack: world.submit(resident, request) });
            return;
          case "observe":
            emit({ op: "observation", observation: world.observe(resident, frame.opts ?? {}) });
            return;
          case "subscribe":
            kinds = frame.kinds;
            return;
          case "ping":
            emit({ op: "pong" });
            return;
          default:
            invalid(`The frame \`${String((frame as { op?: unknown }).op)}\` is not known.`);
        }
      },
    };
  }
}
