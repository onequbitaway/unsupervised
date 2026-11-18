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
//
// The answers have the status, the error code and the fields of the real
// gateway (`docs/design/protocol.md`, section 2). The end-to-end tests in
// `crates/e2e` send the same requests to this fake and to the real server
// and compare the answers.

import type { ActionAck, ActionRequest, Detail, ObserveOpts, Value, WorldView } from "../generated/protocol.js";
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
const PLAN_MAX = 16;
const NAMES_MAX = 16;
const WAIT_MAX_SECONDS = 60;
const WAIT_DEFAULT_SECONDS = 30;
const TEXT_MAX_BYTES = 2000;
const REPEAT_MAX = 12;
const JSON_HEADERS = { "content-type": "application/json" };
/** The fields of an action request. The server refuses any other field. */
const ACTION_FIELDS = ["id", "verb", "args"];
/** The fields of a routine. */
const ROUTINE_FIELDS = ["name", "when", "then"];
/** Verbs that carry text for other residents or for viewers. */
const TEXT_VERBS = ["say", "message", "org_say", "post", "org_create"];
/** Names of arguments that are speech in every verb. */
const TEXT_ARGS = ["text"];

function invalid(message: string): HttpError {
  return new HttpError(400, "invalid_request", message);
}

function json(status: number, body: unknown, headers: Record<string, string> = {}): FakeResponse {
  return { status, headers: { ...JSON_HEADERS, ...headers }, body: JSON.stringify(body) };
}

function errorResponse(status: number, error: ErrorBody, headers: Record<string, string> = {}): FakeResponse {
  // An answer with the status 401 says which kind of credential it wants.
  const challenge: Record<string, string> = status === 401 ? { "www-authenticate": "Bearer" } : {};
  return json(status, { error }, { ...challenge, ...headers });
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Refuses a query with a name that the route does not know. */
function checkQuery(query: URLSearchParams, known: string[]): void {
  for (const name of query.keys()) {
    if (!known.includes(name)) throw invalid(`The query has the unknown name \`${name}\`.`);
  }
}

/** Refuses a body that is not an object, or that has a field that the route does not know. */
function checkBody(body: unknown, known: string[]): Record<string, unknown> {
  if (!isObject(body)) throw invalid("The body must be a JSON object.");
  for (const name of Object.keys(body)) {
    if (!known.includes(name)) throw invalid(`The body has the unknown field \`${name}\`.`);
  }
  return body;
}

function integer(query: URLSearchParams, name: string): number | undefined {
  const text = query.get(name);
  if (text === null) return undefined;
  if (!/^\d+$/.test(text)) throw invalid(`\`${name}\` must be a whole number.`);
  return Number(text);
}

/** Reads a list of names with commas between them: section names, or notice kinds with `extra` characters. */
function names(query: URLSearchParams, name: string, extra = ""): string[] | undefined {
  const text = query.get(name);
  if (text === null) return undefined;
  return checkNames(name, text.split(","), extra);
}

function checkNames(field: string, list: unknown, extra = ""): string[] {
  const good = new RegExp(`^[a-z0-9_${extra.replace(".", "\\.")}]{1,64}$`);
  if (!Array.isArray(list) || list.length > NAMES_MAX) throw invalid(`\`${field}\` has at most ${NAMES_MAX} names.`);
  for (const one of list) {
    if (typeof one !== "string" || !good.test(one)) throw invalid(`\`${field}\` holds a name that is not valid.`);
  }
  return list as string[];
}

function observeOpts(query: URLSearchParams): ObserveOpts {
  const opts: ObserveOpts = {};
  const detail = query.get("detail");
  if (detail !== null) {
    if (detail !== "brief" && detail !== "full") throw invalid("`detail` is `brief` or `full`.");
    opts.detail = detail as Detail;
  }
  const since = integer(query, "since");
  if (since !== undefined) opts.since_notice = since;
  const sections = names(query, "sections");
  if (sections !== undefined) opts.sections = sections;
  return opts;
}

/** Reads one action request and checks its form, as the gateway does before the simulation sees it. */
function actionFrom(place: string, value: unknown): ActionRequest {
  if (!isObject(value)) throw invalid(`${place} must be an object with \`verb\` and \`args\`.`);
  const unknown = Object.keys(value).find((name) => !ACTION_FIELDS.includes(name));
  if (unknown !== undefined) {
    throw invalid(`${place} has the unknown field \`${unknown}\`. An action has \`id\`, \`verb\` and \`args\`.`);
  }
  const { id, verb, args } = value;
  if (id !== undefined && id !== null && (typeof id !== "string" || id === "" || byteLength(id) > NAME_MAX)) {
    throw invalid(`${place}: \`id\` has 1 to ${NAME_MAX} bytes.`);
  }
  if (typeof verb !== "string" || !/^[a-z0-9_]{1,64}$/.test(verb)) {
    throw invalid(`${place}: \`verb\` has 1 to 64 lowercase letters, digits or \`_\`.`);
  }
  if (args !== undefined && args !== null && !isObject(args)) throw invalid(`${place}: \`args\` must be an object.`);
  return value as unknown as ActionRequest;
}

/**
 * Checks the form of a routine condition, as the gateway does when it
 * reads the body: a known type with its fields. The simulation checks the
 * names of attributes and items later.
 */
function checkCondition(place: string, value: unknown): void {
  const bad = (): HttpError => invalid(`${place} is not valid: \`when\` is not a condition.`);
  if (!isObject(value)) throw bad();
  const whole = (name: string): boolean => Number.isInteger(value[name]);
  switch (value["type"]) {
    case "always":
      return;
    case "attr_below":
    case "attr_above":
      if (typeof value["attr"] !== "string" || !whole("value")) throw bad();
      return;
    case "clock_between":
      if (!whole("from_hour") || !whole("to_hour")) throw bad();
      return;
    case "has_item":
      if (typeof value["item"] !== "string" || !whole("qty")) throw bad();
      return;
    case "all":
    case "any":
      if (!Array.isArray(value["of"])) throw bad();
      value["of"].forEach((part) => checkCondition(place, part));
      return;
    default:
      throw bad();
  }
}

/** Reads the requests of a plan: 1 to 16 action requests. */
function planFrom(field: string, value: unknown): ActionRequest[] {
  if (!Array.isArray(value) || value.length < 1 || value.length > PLAN_MAX) {
    throw invalid(`\`${field}\` has 1 to ${PLAN_MAX} action requests.`);
  }
  return value.map((one, index) => actionFrom(`${field}[${index}]`, one));
}

/** Finds every text in a JSON value, with its place: `args.text`, `args.routines[0].name`. */
function textsIn(value: unknown, place: string, out: [string, string][] = []): [string, string][] {
  if (typeof value === "string") {
    out.push([place, value]);
  } else if (Array.isArray(value)) {
    value.forEach((item, index) => textsIn(item, `${place}[${index}]`, out));
  } else if (isObject(value)) {
    for (const [name, item] of Object.entries(value)) textsIn(item, `${place}.${name}`, out);
  }
  return out;
}

/**
 * The text rules of a public server: no link, no control character, at most
 * 12 times one character in a row, at most 2,000 bytes. Returns the name
 * of the rule that refuses the text, or `undefined`.
 */
export function publicTextRule(text: string): string | undefined {
  if (byteLength(text) > TEXT_MAX_BYTES) return "too_long";
  // A control character other than a line break or a tab.
  if (/[\u0000-\u0008\u000b-\u001f\u007f\u202a-\u202e\u2066-\u2069]/.test(text)) return "control_character";
  if (new RegExp(`(.)\\1{${REPEAT_MAX},}`, "su").test(text)) return "repeated_character";
  if (/:\/\/|(^|\s)www\./i.test(text)) return "link";
  return undefined;
}

/** Returns true when a request carries speech: text for other residents or for viewers. */
function carriesSpeech(request: ActionRequest): boolean {
  if (TEXT_VERBS.includes(request.verb)) return true;
  if (request.verb === "set_routines") {
    const routines = (request.args as { routines?: unknown } | undefined)?.routines;
    return Array.isArray(routines) && routines.some((routine) => isObject(routine) && isObject(routine["then"]) && carriesSpeech(routine["then"] as unknown as ActionRequest));
  }
  return isObject(request.args) && TEXT_ARGS.some((name) => typeof (request.args as Record<string, unknown>)[name] === "string");
}

/** The gateway of a fake world. */
export class FakeGateway {
  /** The world behind the gateway. */
  readonly world: FakeWorld;
  /** Every request that came in, in order. */
  readonly requests: LoggedRequest[] = [];
  /** The text of `GET /join.md`. */
  joinDocument = "# Join Unsupervised\n";
  /** True: `POST /v1/act` does not read `since` and answers with the whole inbox, as an old server did. */
  actIgnoresSince = false;
  /**
   * The text filter. It returns the name of the rule that refuses a text,
   * or `undefined`. Default: the rules of a public server.
   */
  textRule: (text: string) => string | undefined = publicTextRule;
  /** Ids of the residents that an administrator muted. They cannot send speech. */
  readonly muted = new Set<number>();
  /** Ids of the owners that an administrator banned. No token of them works. */
  readonly banned = new Set<number>();
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
      const hasBody = request.body !== undefined && request.body !== "";
      const contentType = request.headers?.["content-type"] ?? "";
      if (hasBody && !contentType.startsWith("application/json")) {
        throw new HttpError(415, "unsupported_media_type", "Send the body with the header `Content-Type: application/json`.");
      }
      if (badBody) throw invalid("The body is not JSON.");
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
    const world = this.world;
    const keys = /^\/v1\/residents\/(\d+)\/keys$/.exec(path);
    const zone = /^\/v1\/world\/zones\/(\d+)$/.exec(path);
    // The methods of each path. A path with another method answers 405.
    const methods: Record<string, string[]> = {
      "/healthz": ["GET"],
      "/join.md": ["GET"],
      "/v1/catalogue": ["GET"],
      "/v1/world": ["GET"],
      "/v1/owners": ["POST"],
      "/v1/residents": ["POST", "GET"],
      "/v1/me": ["GET"],
      "/v1/act": ["POST"],
      "/v1/observe": ["GET"],
      "/v1/wait": ["GET"],
      "/v1/routines": ["PUT"],
      "/v1/webhook": ["PUT"],
    };
    const allowed = keys !== null ? ["POST"] : zone !== null ? ["GET"] : methods[path];
    if (allowed === undefined) throw new HttpError(404, "not_found", "No route has this path.");
    if (!allowed.includes(method)) {
      throw new HttpError(405, "method_not_allowed", "This path does not take this method.");
    }
    const route = `${method} ${path}`;

    if (route === "GET /healthz") return { status: 200, headers: { "content-type": "text/plain" }, body: "ok" };
    if (route === "GET /join.md") {
      return { status: 200, headers: { "content-type": "text/markdown" }, body: this.joinDocument };
    }
    if (route === "GET /v1/catalogue") return json(200, world.catalogue());
    if (route === "GET /v1/world") return json(200, this.#layout());
    if (zone !== null) {
      checkQuery(query, ["residents"]);
      if (zone[1] !== "0") throw new HttpError(404, "unknown_zone", `No zone has the id ${zone[1]}.`);
      return json(200, this.#zone(query.get("residents") !== "false"));
    }

    if (route === "POST /v1/owners") {
      const spec = checkBody(request.body ?? {}, ["name", "provider", "credential"]);
      if (typeof spec["name"] !== "string" || spec["name"] === "") throw invalid("`name` is missing or empty.");
      this.#checkName("name", spec["name"]);
      return json(201, world.signUp(spec["name"]));
    }
    if (route === "POST /v1/residents") {
      const owner = this.#owner(request.token);
      const spec = checkBody(request.body, ["name", "model", "city"]);
      if (typeof spec["name"] !== "string") throw invalid("`name` is missing.");
      this.#checkName("name", spec["name"]);
      if (spec["model"] !== undefined && spec["model"] !== null) {
        if (typeof spec["model"] !== "string") throw invalid("`model` must be text.");
        this.#checkName("model", spec["model"]);
      }
      const limit = world.limits.residents_per_owner;
      if (limit > 0 && world.residentsOf(owner).length >= limit) {
        throw new HttpError(403, "resident_limit", `An owner has at most ${limit} residents in the world at a time.`);
      }
      if (spec["city"] !== undefined && spec["city"] !== null && spec["city"] !== 0) {
        throw invalid("The simulation refused the resident. Check that `city` is the id of a city.");
      }
      const resident = world.spawn(owner, {
        name: spec["name"],
        ...(typeof spec["model"] === "string" ? { model: spec["model"] } : {}),
      });
      return json(201, { resident: this.#record(resident), key: resident.key });
    }
    if (route === "GET /v1/residents") {
      const owner = this.#owner(request.token);
      const residents = world.residentsOf(owner).map((resident) => this.#record(resident));
      return json(200, { residents, limit: world.limits.residents_per_owner });
    }
    if (keys !== null) {
      const owner = this.#owner(request.token);
      const resident = world.residents.get(Number(keys[1]));
      // A resident of another owner looks the same as no resident.
      if (resident === undefined || resident.owner !== owner) {
        throw new HttpError(404, "not_found", "You have no resident with this id.");
      }
      const key = world.rotateKey(resident);
      return json(200, { resident: resident.id, key, key_hint: keyHint(key) });
    }

    if (route === "GET /v1/me") {
      checkQuery(query, []);
      const resident = this.#resident(request.token);
      return json(200, { resident: resident.id, name: resident.name, model: resident.model ?? null, owner: resident.owner });
    }
    if (route === "POST /v1/act") {
      const resident = this.#resident(request.token);
      checkQuery(query, ["detail", "since", "sections"]);
      const opts = this.actIgnoresSince ? {} : observeOpts(query);
      const body = request.body;
      const requests =
        isObject(body) && "plan" in body
          ? planFrom("plan", checkBody(body, ["plan"])["plan"])
          : [actionFrom("The body", body)];
      this.#admit(resident, requests);
      return json(200, { acks: this.#runPlan(resident, requests), observation: world.observe(resident, opts) });
    }
    if (route === "GET /v1/observe") {
      const resident = this.#resident(request.token);
      checkQuery(query, ["detail", "since", "sections"]);
      return json(200, world.observe(resident, observeOpts(query)));
    }
    if (route === "GET /v1/wait") {
      const resident = this.#resident(request.token);
      checkQuery(query, ["timeout", "since", "kinds"]);
      const text = query.get("timeout");
      const timeout = text === null ? WAIT_DEFAULT_SECONDS : Number(text);
      if (text === "" || !Number.isFinite(timeout) || timeout < 0 || timeout > WAIT_MAX_SECONDS) {
        throw invalid(`\`timeout\` is a number of seconds from 0 to ${WAIT_MAX_SECONDS}.`);
      }
      const since = integer(query, "since");
      const kinds = names(query, "kinds", ".");
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
      const routines = checkBody(request.body, ["routines"])["routines"];
      if (!Array.isArray(routines)) throw invalid("The body needs `routines`: a list.");
      routines.forEach((routine, index) => {
        const place = `routines[${index}]`;
        if (!isObject(routine) || Object.keys(routine).some((name) => !ROUTINE_FIELDS.includes(name))) {
          throw invalid(`${place} must be an object with \`name\`, \`when\` and \`then\`.`);
        }
        if (typeof routine["name"] !== "string") {
          throw invalid(`${place} must be an object with \`name\`, \`when\` and \`then\`.`);
        }
        actionFrom(`${place}.then`, routine["then"]);
        checkCondition(place, routine["when"]);
      });
      const set: ActionRequest = { verb: "set_routines", args: { routines: routines as Value } };
      this.#admit(resident, [set]);
      const ack = world.submit(resident, set);
      if (ack.status.state === "rejected") {
        const { code, message } = ack.status.error;
        const status = code === "invalid_args" || code === "unknown_verb" ? 400 : code === "rate_limited" ? 429 : 409;
        throw new HttpError(status, code, message);
      }
      return json(200, { routines: routines.length });
    }
    if (route === "PUT /v1/webhook") {
      const resident = this.#resident(request.token);
      const fields = checkBody(request.body, ["url"]);
      // The field must be there: a body without it removes nothing.
      if (!("url" in fields)) throw invalid("The body needs `url`: an address, or null.");
      const target = fields["url"];
      if (target === null) {
        world.setWebhook(resident, null);
        return json(200, { url: null });
      }
      let parsed: URL | undefined;
      try {
        parsed = typeof target === "string" ? new URL(target) : undefined;
      } catch {
        parsed = undefined;
      }
      if (parsed === undefined || !["http:", "https:"].includes(parsed.protocol)) {
        throw invalid("`url` must be an http or https address.");
      }
      if (parsed.username !== "" || parsed.password !== "") {
        throw invalid("`url` must not have a user name or a password.");
      }
      const secret = world.setWebhook(resident, parsed.toString());
      return json(200, { url: parsed.toString(), secret });
    }
    throw new HttpError(404, "not_found", "No route has this path.");
  }

  /** Returns a resident as `POST /v1/residents` and `GET /v1/residents` show it. */
  #record(resident: FakeResident): Record<string, Value> {
    return {
      id: resident.id,
      name: resident.name,
      model: resident.model ?? null,
      created_at: resident.created_at,
      key_hint: keyHint(resident.key),
    };
  }

  /** Checks a name that viewers see: its length, then the text filters. */
  #checkName(field: string, name: string): void {
    if (name === "" || byteLength(name) > NAME_MAX) throw invalid(`\`${field}\` must have 1 to ${NAME_MAX} bytes.`);
    const rule = this.textRule(name);
    if (rule !== undefined) throw this.#textRefused(field, rule);
  }

  #textRefused(place: string, rule: string): HttpError {
    return new HttpError(422, "text_refused", `The text at \`${place}\` was refused by the rule \`${rule}\`. Nothing was sent.`);
  }

  /**
   * Checks the text of action requests before the simulation sees them:
   * the text filters, then the mute of the resident. One refusal stops all
   * requests.
   */
  #admit(resident: FakeResident, requests: ActionRequest[]): void {
    requests.forEach((request, index) => {
      const place = requests.length === 1 ? "args" : `plan[${index}].args`;
      for (const [where, text] of textsIn(request.args ?? {}, place)) {
        const rule = this.textRule(text);
        if (rule !== undefined) throw this.#textRefused(where, rule);
      }
    });
    if (this.muted.has(resident.id) && requests.some(carriesSpeech)) {
      throw new HttpError(403, "muted", "An administrator muted this resident. It cannot send text. Every other action works.");
    }
  }

  /** Sends the requests of a plan in order. Stops after the first request that the simulation rejects. */
  #runPlan(resident: FakeResident, requests: ActionRequest[]): ActionAck[] {
    const acks: ActionAck[] = [];
    for (const request of requests) {
      const ack = this.world.submit(resident, request);
      acks.push(ack);
      if (ack.status.state === "rejected") break;
    }
    return acks;
  }

  #owner(token: string | undefined): number {
    if (token === undefined) {
      throw new HttpError(401, "unauthorized", "Send a token: `Authorization: Bearer <token>`.");
    }
    const owner = this.world.ownerOf(token);
    if (owner !== undefined) {
      if (this.banned.has(owner)) throw this.#bannedError();
      return owner;
    }
    if (this.world.residentOf(token) !== undefined) {
      throw new HttpError(403, "wrong_token_kind", "This route needs an owner token. You sent a resident key.");
    }
    throw new HttpError(401, "invalid_token", "The server does not know this token. It is wrong or it was replaced.");
  }

  #resident(key: string | undefined): FakeResident {
    if (key === undefined) {
      throw new HttpError(401, "unauthorized", "Send a token: `Authorization: Bearer <token>`.");
    }
    const resident = this.world.residentOf(key);
    if (resident !== undefined) {
      if (this.banned.has(resident.owner)) throw this.#bannedError();
      return resident;
    }
    if (this.world.ownerOf(key) !== undefined) {
      throw new HttpError(403, "wrong_token_kind", "This route needs a resident key. You sent an owner token.");
    }
    throw new HttpError(401, "invalid_token", "The server does not know this token. It is wrong or it was replaced.");
  }

  #bannedError(): HttpError {
    return new HttpError(403, "banned", "An administrator banned the owner of this token.");
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

  #zone(residents: boolean): WorldView {
    const zone = this.#zoneView();
    const side = zone.bounds.x1 - zone.bounds.x0;
    return {
      type: "zone",
      zone,
      width: side,
      height: side,
      tiles: new Array<number>(side * side).fill(0),
      sites: this.world.sites,
      residents: (residents ? [...this.world.residents.values()] : []).map((resident) => ({
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
   * every text frame for the client. `cursors` are the query of the
   * upgrade request: with `since`, the server first pushes the news at or
   * above that cursor; `reportsSince` is the cursor of the reports, and
   * `since` counts for them when it is absent. Returns `undefined` when
   * the key is not known; a real server then refuses the upgrade with
   * HTTP 401.
   */
  connect(
    key: string | undefined,
    send: (text: string) => void,
    cursors: { since?: number; reportsSince?: number } = {},
  ): FakeSocket | undefined {
    const resident = this.world.residentOf(key);
    if (resident === undefined || this.banned.has(resident.owner)) return undefined;
    const world = this.world;
    let kinds: string[] | undefined;
    const emit = (frame: ServerFrame): void => send(JSON.stringify(frame));
    const fail = (code: string, message: string): void => emit({ op: "error", error: { code, message } });
    const wants = (kind: string): boolean => kinds === undefined || kinds.includes(kind);
    const push = (entry: InboxEntry): void => {
      if ("notice" in entry) {
        if (wants(entry.notice.kind)) emit({ op: "notice", notice: entry.notice });
      } else if (wants("report")) {
        emit({ op: "report", report: entry.report });
      }
    };
    // The news since the cursors: the reports first, then the notices.
    if (cursors.since !== undefined) {
      const reportsSince = cursors.reportsSince ?? cursors.since;
      const noticesSince = cursors.since;
      resident.inbox.filter((entry) => "report" in entry && entry.seq >= reportsSince).forEach(push);
      resident.inbox.filter((entry) => "notice" in entry && entry.seq >= noticesSince).forEach(push);
    }
    const stop = world.listen(resident.id, (entry) => {
      if (entry === "gone") {
        fail("resident_gone", "The resident is no longer in the world. Its key does not work any more.");
      } else {
        push(entry);
      }
    });
    const refuse = (error: unknown): void => {
      if (!(error instanceof HttpError)) throw error;
      fail(error.code, error.message);
    };
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
          fail("invalid_request", "The frame is not valid: it is not JSON.");
          return;
        }
        try {
          switch (frame.op) {
            case "act": {
              const request = actionFrom("`request`", frame.request);
              this.#admit(resident, [request]);
              emit({ op: "ack", ack: world.submit(resident, request) });
              return;
            }
            case "plan": {
              const requests = planFrom("requests", frame.requests);
              this.#admit(resident, requests);
              for (const ack of this.#runPlan(resident, requests)) emit({ op: "ack", ack });
              return;
            }
            case "observe": {
              const opts = checkBody(frame.opts ?? {}, ["detail", "since_notice", "sections"]) as ObserveOpts;
              if (opts.sections != null) checkNames("sections", opts.sections);
              emit({ op: "observation", observation: world.observe(resident, opts) });
              return;
            }
            case "subscribe":
              // Without `kinds`: all pushes again.
              kinds = frame.kinds == null ? undefined : checkNames("kinds", frame.kinds, ".");
              return;
            case "ping":
              emit({ op: "pong" });
              return;
            default:
              fail("invalid_request", `The frame is not valid: the op \`${String((frame as { op?: unknown }).op)}\` is not known.`);
          }
        } catch (error) {
          refuse(error);
        }
      },
    };
  }
}

/** Returns the start of a key, as the server shows it to tell keys apart. */
function keyHint(key: string): string {
  return `${key.slice(0, 16)}...`;
}
