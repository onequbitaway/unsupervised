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

    const acks = await session.plan([{ verb: "wait", args: { ticks: 3 } }, { verb: "fly" }, { verb: "wait", args: { ticks: 1 } }]);
    assert.deepEqual(
      acks.map((one) => one.status.state),
      ["queued", "rejected", "queued"],
    );
    assert.equal(resident.queue.length, 2);

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
        [1, "society.speech"],
        [2, "economy.wage_paid"],
      ],
    );
    assert.equal(client.cursor, 3);

    await session.act("wait", { ticks: 2 });
    world.step(2);
    await reports.atLeast(1);
    assert.deepEqual(reports.items.map((report) => [report.verb, report.ok]), [["wait", true]]);

    // An observation after the pushes repeats the report (a report has no
    // sequence number), but the handlers do not get anything twice.
    const observation = await session.observe();
    assert.deepEqual(observation.notices, []);
    assert.equal(notices.items.length, 2);
    assert.equal(reports.items.length, 1);
  } finally {
    session.close();
  }
});

test("`subscribe` limits the pushed kinds", async (t) => {
  const { client, world, resident } = await scene(t);
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
  } finally {
    session.close();
  }
});

test("after a lost connection the session connects again and reads what it missed, once", async (t) => {
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
    world.tell(resident.id, 7, "before");
    await session.act("wait", { ticks: 2 });
    await notices.atLeast(1);

    server.dropSockets();
    // While the connection is down: a notice arrives and the action ends.
    world.tell(resident.id, 7, "during");
    world.step(2);

    await opens.atLeast(2);
    assert.deepEqual(opens.items, [1, 2]);
    assert.deepEqual(closes.items, [false]);
    assert.equal(session.state, "open");
    assert.deepEqual(notices.items.map((notice) => (notice.data as { text: string }).text), ["before", "during"]);
    assert.deepEqual(reports.items.map((report) => report.verb), ["wait"]);

    // Pushes work again, and nothing old comes twice.
    world.tell(resident.id, 7, "after");
    await notices.atLeast(3);
    assert.deepEqual(notices.items.map((notice) => notice.seq), [1, 2, 4]);
    assert.equal(reports.items.length, 1);
  } finally {
    session.close();
  }
});

test("a request without an answer goes out again after the reconnect and acts once", async (t) => {
  const { client, server, resident } = await scene(t);
  const session = client.connect(instant);
  try {
    await session.ready;
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
  const { client, gateway } = await scene(t);
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
  } finally {
    session.close();
  }
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
    assert.deepEqual([error.code, error.status], ["unauthorized", 401]);
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
