// The WebSocket session: requests, pushed notices and reports, reconnect.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ApiError,
  Client,
  MissingCredentialError,
  SessionClosedError,
  type ActionReport,
  type Notice,
  type WebSocketLike,
} from "../src/index.js";
import { Collector } from "./support/server.js";
import { scene } from "./support/setup.js";

const instant = { reconnectDelay: () => 0 };

test("a session acts, plans, observes and pings over the WebSocket", async (t) => {
  const { client, world, resident } = await scene(t);
  const session = client.connect(instant);
  try {
    await session.ready;
    assert.equal(session.state, "open");

    const ack = await session.act("move_to", { x: 34, y: 32 });
    assert.deepEqual(ack.status, { state: "started", ends_at: 2, ticks: 2 });
    assert.match(ack.request_id ?? "", /^[0-9a-f-]{36}$/);

    const acks = await session.plan([{ verb: "wait", args: { ticks: 3 } }, { verb: "wait", args: { ticks: 1 } }]);
    assert.deepEqual(
      acks.map((one) => one.status.state),
      ["queued", "queued"],
    );
    assert.equal(resident.queue.length, 2);
    // The server stops a plan after the first refusal: no acknowledgement
    // comes for the steps after it, and they are not in the queue.
    const stopped = await session.plan([
      { verb: "wait", args: { ticks: 2 } },
      { verb: "fly" },
      { verb: "wait", args: { ticks: 7 } },
    ]);
    assert.deepEqual(
      stopped.map((one) => [one.verb, one.status.state]),
      [
        ["wait", "queued"],
        ["fly", "rejected"],
      ],
    );
    assert.equal(resident.queue.length, 3);
    assert.deepEqual(await session.plan([]), []);

    world.tell(resident.id, 7, "hello");
    const observation = await session.observe({ detail: "full" });
    assert.equal(observation.resident, resident.id);
    assert.equal(observation.next_notice, 2);
    assert.equal(client.cursor, 2);
    assert.equal(await session.ping(), undefined);
  } finally {
    session.close();
  }
});

test("the server pushes notices and reports, and each reaches its handler once", async (t) => {
  const { client, world, resident } = await scene(t);
  const notices = new Collector<Notice>();
  const reports = new Collector<ActionReport>();
  const session = client.connect({ ...instant, onNotice: notices.push, onReport: reports.push });
  try {
    await session.ready;
    world.tell(resident.id, 7, "one");
    world.notify(resident.id, "economy.wage_paid", { amount: 900 });
    await notices.atLeast(2);
    assert.deepEqual(
      notices.items.map((notice) => [notice.seq, notice.kind]),
      [
        [1, "society.said"],
        [2, "economy.wage_paid"],
      ],
    );
    assert.equal(client.cursor, 3);

    await session.act("wait", { ticks: 2 });
    world.step(2);
    await reports.atLeast(1);
    assert.deepEqual(reports.items.map((report) => [report.verb, report.ok, report.seq]), [["wait", true, 3]]);
    assert.equal(client.cursor, 4);
    assert.deepEqual(session.cursors, { since: 3, reportsSince: 4 });

    // An observation with an old cursor repeats the news, but the handlers
    // do not get anything twice: the sequence numbers tell.
    const observation = await session.observe({ since: 1 });
    assert.deepEqual(observation.notices.map((notice) => notice.seq), [1, 2]);
    assert.equal(notices.items.length, 2);
    assert.equal(reports.items.length, 1);
    // News that no push carried reaches the handlers through `observe`.
    session.subscribe(["nothing"]);
    await session.ping();
    session.subscribe([]);
    world.notify(resident.id, "economy.rent_due");
    await session.ping();
    assert.deepEqual(notices.items.map((notice) => notice.seq), [1, 2]);
    await session.observe();
    assert.deepEqual(notices.items.map((notice) => notice.seq), [1, 2, 4]);
  } finally {
    session.close();
  }
});

test("`subscribe` limits the pushed kinds", async (t) => {
  const { client, server, world, resident } = await scene(t);
  const notices = new Collector<Notice>();
  const reports = new Collector<ActionReport>();
  const session = client.connect({
    ...instant,
    kinds: ["economy.rent_due"],
    onNotice: notices.push,
    onReport: reports.push,
  });
  try {
    await session.ready;
    // A ping after the open proves that the server has the subscription.
    await session.ping();
    world.tell(resident.id, 7, "noise");
    world.notify(resident.id, "economy.rent_due");
    await notices.atLeast(1);
    assert.deepEqual(notices.items.map((notice) => notice.kind), ["economy.rent_due"]);

    session.subscribe(["report"]);
    await session.act("wait", { ticks: 1 });
    world.notify(resident.id, "economy.rent_due");
    world.step(1);
    await reports.atLeast(1);
    assert.equal(notices.items.length, 1);
    assert.deepEqual(reports.items.map((report) => report.verb), ["wait"]);

    // An empty list means every kind: the session sends `subscribe`
    // without `kinds`. A list with no name would stop every push.
    session.subscribe([]);
    await session.ping();
    assert.deepEqual(JSON.parse(server.frames.at(-2) as string), { op: "subscribe" });
    world.tell(resident.id, 7, "heard");
    await notices.atLeast(2);
    assert.deepEqual(notices.items.map((notice) => notice.kind), ["economy.rent_due", "society.said"]);
  } finally {
    session.close();
  }
});

test("after a lost connection the session connects again with its two cursors and gets what it missed, once", async (t) => {
  const { client, server, world, resident } = await scene(t);
  const notices = new Collector<Notice>();
  const reports = new Collector<ActionReport>();
  const opens = new Collector<number>();
  const closes = new Collector<boolean>();
  const session = client.connect({
    ...instant,
    onNotice: notices.push,
    onReport: reports.push,
    onOpen: () => opens.push(server.upgrades),
    onClose: (info) => closes.push(info.final),
  });
  try {
    await session.ready;
    // The first connection has no cursor: the server pushes only what comes from now on.
    assert.deepEqual(server.upgradeUrls, ["/v1/ws"]);
    world.tell(resident.id, 7, "before");
    await session.act("wait", { ticks: 1 });
    world.step(1);
    await reports.atLeast(1);
    await session.plan([
      { verb: "wait", args: { ticks: 1 } },
      { verb: "wait", args: { ticks: 1 } },
    ]);
    assert.deepEqual(session.cursors, { since: 2, reportsSince: 3 });

    server.dropSockets();
    // While the connection is down: a report, a notice, a report.
    world.step(1);
    world.tell(resident.id, 7, "during");
    world.step(1);

    await opens.atLeast(2);
    assert.deepEqual(opens.items, [1, 2]);
    assert.deepEqual(closes.items, [false]);
    assert.equal(session.state, "open");
    // The second connection sent the `seq` of the last notice plus 1, and
    // the `seq` of the last report plus 1.
    assert.deepEqual(server.upgradeUrls, ["/v1/ws", "/v1/ws?since=2&reports_since=3"]);
    await notices.atLeast(2);
    await reports.atLeast(3);
    assert.deepEqual(notices.items.map((notice) => notice.seq), [1, 4]);
    assert.deepEqual(reports.items.map((report) => report.seq), [2, 3, 5]);

    // Pushes work again, and nothing old comes twice.
    world.tell(resident.id, 7, "after");
    await notices.atLeast(3);
    assert.deepEqual(notices.items.map((notice) => notice.seq), [1, 4, 6]);
    assert.equal(reports.items.length, 3);
    assert.equal(client.cursor, 7);
  } finally {
    session.close();
  }
});

test("a session of a client with a cursor starts at that cursor", async (t) => {
  const { client, server, world, resident } = await scene(t);
  world.tell(resident.id, 7, "seen over HTTPS");
  await client.observe();
  world.tell(resident.id, 7, "not seen yet");
  const notices = new Collector<Notice>();
  const session = client.connect({ ...instant, onNotice: notices.push });
  try {
    await session.ready;
    assert.deepEqual(server.upgradeUrls, ["/v1/ws?since=2&reports_since=2"]);
    await notices.atLeast(1);
    assert.deepEqual(notices.items.map((notice) => notice.seq), [2]);
  } finally {
    session.close();
  }
});

test("a request without an answer goes out again after the reconnect and acts once", async (t) => {
  const { client, server, resident } = await scene(t);
  const session = client.connect(instant);
  try {
    await session.ready;
    // The session asked where the inbox ends. That answer is there after a ping.
    await session.ping();
    // The server handles the action, but the acknowledgement is lost with the connection.
    server.loseFrames(1);
    const handled = server.frameHandled();
    const acting = session.act("wait", { ticks: 9 });
    await handled;
    const first = resident.current?.action_id;
    assert.equal(typeof first, "number");
    server.dropSockets();

    const ack = await acting;
    assert.equal(server.upgrades, 2);
    assert.deepEqual(ack.status, { state: "started", ends_at: 9, ticks: 9 });
    // The repeat got the first answer: the same action, and nothing in the queue.
    assert.equal(ack.action_id, first);
    assert.equal(resident.current?.action_id, first);
    assert.equal(resident.queue.length, 0);
  } finally {
    session.close();
  }
});

test("an `error` frame fails the oldest request without an answer", async (t) => {
  const { client, gateway, resident } = await scene(t);
  const session = client.connect(instant);
  try {
    await session.ready;
    gateway.failNextFrame({ code: "rate_limited", message: "Too many frames." });
    await assert.rejects(session.ping(), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.deepEqual([error.code, error.status, error.isTransient], ["rate_limited", undefined, true]);
      return true;
    });
    // The session still works.
    assert.equal(await session.ping(), undefined);

    // A text that a filter refuses: the action did not reach the world.
    await assert.rejects(session.act("say", { text: "read www.example.com" }), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.deepEqual([error.code, error.isTextRefusal], ["text_refused", true]);
      return true;
    });
    // One refused text stops a whole plan, with one error.
    const plan = session.plan([{ verb: "wait", args: { ticks: 1 } }, { verb: "say", args: { text: "www.example.com" } }]);
    await assert.rejects(plan, (error: unknown) => error instanceof ApiError && error.code === "text_refused");
    assert.equal(resident.current, undefined);
    // A muted resident cannot speak. Every other action works.
    gateway.muted.add(resident.id);
    await assert.rejects(session.act("say", { text: "hello" }), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.equal(error.code, "muted");
      return true;
    });
    assert.equal((await session.act("wait", { ticks: 1 })).status.state, "started");
  } finally {
    session.close();
  }
});

test("a resident that left the world ends the session", async (t) => {
  const { client, world, resident } = await scene(t);
  const errors = new Collector<Error>();
  const closes = new Collector<boolean>();
  const session = client.connect({ ...instant, onError: errors.push, onClose: (info) => closes.push(info.final) });
  await session.ready;
  await session.ping();
  world.despawn(resident);
  await closes.atLeast(1);
  assert.deepEqual(closes.items, [true]);
  assert.equal(session.state, "closed");
  assert.equal(errors.items.length, 1);
  assert.ok(errors.items[0] instanceof ApiError);
  assert.equal((errors.items[0] as ApiError).code, "resident_gone");
  assert.equal((errors.items[0] as ApiError).endsCredential, true);
  await assert.rejects(session.ping(), SessionClosedError);
});

test("a session with a wrong key fails with the error of the server and does not try again", async (t) => {
  const { server } = await scene(t);
  const errors = new Collector<Error>();
  const closes = new Collector<boolean>();
  const client = new Client({ baseUrl: server.url, residentKey: "wrong-key" });
  // Count the sockets that the session makes. The WebSocket of Node can
  // send its own second upgrade request, so the server count says less.
  let made = 0;
  const session = client.connect({
    ...instant,
    onError: errors.push,
    onClose: (info) => closes.push(info.final),
    webSocket: (url, headers) => {
      made += 1;
      const Socket = WebSocket as unknown as new (url: string, options: object) => WebSocketLike;
      return new Socket(url, { headers });
    },
  });
  await assert.rejects(session.ready, (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["invalid_token", 401]);
    return true;
  });
  assert.equal(session.state, "closed");
  assert.equal(made, 1);
  assert.ok(server.upgrades >= 1);
  assert.deepEqual(closes.items, [true]);
  assert.equal(errors.items.length, 1);
  await assert.rejects(session.ping(), SessionClosedError);
});

test("a session without a key does not connect", async (t) => {
  const { server } = await scene(t);
  const session = new Client({ baseUrl: server.url }).connect(instant);
  await assert.rejects(session.ready, MissingCredentialError);
  assert.equal(server.upgrades, 0);
});

test("`close` fails the requests without an answer and stops the reconnect", async (t) => {
  const { client, server } = await scene(t);
  const closes = new Collector<boolean>();
  const session = client.connect({ ...instant, onClose: (info) => closes.push(info.final) });
  await session.ready;
  // The answer of this request never comes.
  server.loseFrames(1);
  const pending = session.ping();
  session.close();
  await assert.rejects(pending, SessionClosedError);
  assert.equal(session.state, "closed");
  assert.deepEqual(closes.items, [true]);
  await assert.rejects(session.act("wait", { ticks: 1 }), SessionClosedError);
  assert.equal(server.upgrades, 1);
});

test("with `reconnect: false` a lost connection ends the session", async (t) => {
  const { client, server } = await scene(t);
  const closes = new Collector<boolean>();
  const session = client.connect({ reconnect: false, onClose: (info) => closes.push(info.final) });
  await session.ready;
  const pending = session.ping();
  server.loseFrames(1);
  server.dropSockets();
  await assert.rejects(pending, SessionClosedError);
  await closes.atLeast(1);
  assert.deepEqual(closes.items, [true]);
  assert.equal(session.state, "closed");
  assert.equal(server.upgrades, 1);
});

/** A socket that a test drives by hand: it sends nothing and gives only the events that the test fires. */
class ScriptedSocket implements WebSocketLike {
  closed = 0;
  readonly sent: string[] = [];
  readonly #listeners = new Map<string, ((event: unknown) => void)[]>();

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.closed += 1;
  }

  addEventListener(type: string, listener: (event: unknown) => void): void {
    this.#listeners.set(type, [...(this.#listeners.get(type) ?? []), listener]);
  }

  /** Gives an event to the listeners, as the runtime does. */
  fire(type: "open" | "close" | "error"): void {
    for (const listener of this.#listeners.get(type) ?? []) listener({ type });
  }
}

// The time limit turns a session that never connects again into a failed test.
test("a connection that fails with `error` and no `close` is tried again", { timeout: 10_000 }, async (t) => {
  // Node 22 gives only `error` when the server refuses the upgrade or is not there.
  const { client } = await scene(t);
  const sockets: ScriptedSocket[] = [];
  const made = new Collector<ScriptedSocket>();
  const closes = new Collector<boolean>();
  const session = client.connect({
    ...instant,
    onClose: (info) => closes.push(info.final),
    webSocket: () => {
      const socket = new ScriptedSocket();
      sockets.push(socket);
      made.push(socket);
      return socket;
    },
  });
  try {
    (await made.atLeast(1))[0]?.fire("error");
    // The key is good (the session asks over HTTPS), so the session connects again.
    const second = (await made.atLeast(2))[1] as ScriptedSocket;
    assert.equal(session.state, "reconnecting");
    assert.deepEqual(closes.items, [false]);
    // The failed socket is released.
    assert.equal(sockets[0]?.closed, 1);

    second.fire("open");
    await session.ready;
    assert.equal(session.state, "open");

    // A later runtime gives `close` after `error`. The second event of one failure changes nothing.
    sockets[0]?.fire("close");
    assert.equal(session.state, "open");
    assert.equal(sockets.length, 2);
  } finally {
    session.close();
  }
});

test("an `error` on an open connection waits for the `close` that follows", { timeout: 10_000 }, async (t) => {
  const { client } = await scene(t);
  const made = new Collector<ScriptedSocket>();
  const session = client.connect({
    ...instant,
    webSocket: () => {
      const socket = new ScriptedSocket();
      made.push(socket);
      return socket;
    },
  });
  try {
    const first = (await made.atLeast(1))[0] as ScriptedSocket;
    first.fire("open");
    await session.ready;
    first.fire("error");
    assert.equal(session.state, "open");
    assert.equal(made.items.length, 1);

    first.fire("close");
    assert.equal(session.state, "reconnecting");
    assert.equal((await made.atLeast(2)).length, 2);
  } finally {
    session.close();
  }
});
