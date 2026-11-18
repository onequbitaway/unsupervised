// What the scripts of this directory share: the address of the real
// server, the clock of the test, and the output for the Rust test.
//
// The Rust test (`crates/e2e`) starts the real server and gives:
//
//   UNSUPERVISED_URL   the address of the server
//   E2E_CLOCK_URL      a service that runs frames of the world:
//                      `POST <url>/step?frames=N`
//   E2E_ADMIN_TOKEN    the admin token of the server

import assert from "node:assert/strict";

function need(name) {
  const value = process.env[name];
  if (value === undefined || value === "") {
    console.error(`Set ${name}. The Rust test in crates/e2e gives it.`);
    process.exit(2);
  }
  return value;
}

/** The address of the real server. */
export const BASE = need("UNSUPERVISED_URL").replace(/\/+$/, "");
/** The admin token of the real server. */
export const ADMIN = process.env.E2E_ADMIN_TOKEN ?? "";

/** Runs frames of the real world. One frame is one game minute. */
export async function step(frames) {
  const response = await fetch(`${need("E2E_CLOCK_URL")}/step?frames=${frames}`, { method: "POST" });
  assert.equal(response.status, 200, "the clock of the test answers");
  return (await response.json()).frame;
}

/** Runs the world, one frame at a time, until `promise` is done. Returns what it gave. */
export async function until(promise, limit = 5_000) {
  let done = false;
  const marked = promise.finally(() => {
    done = true;
  });
  for (let frames = 0; !done; frames += 1) {
    assert.ok(frames < limit, `the awaited news came in ${limit} frames`);
    await step(1);
    // Let the answer of the server arrive before the next frame.
    await new Promise((resolve) => setImmediate(resolve));
  }
  return marked;
}

/** Sends a request with the admin token. */
export async function admin(path, body) {
  const response = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { authorization: `Bearer ${ADMIN}`, ...(body === undefined ? {} : { "content-type": "application/json" }) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  assert.equal(response.status, 200, `POST ${path} works`);
  return response.json();
}

/** Writes one fact for the Rust test: a JSON object on one line. */
export function fact(name, fields = {}) {
  console.log(JSON.stringify({ fact: name, ...fields }));
}
