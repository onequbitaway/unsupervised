// Helpers that read an observation, and the rule for untrusted text.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  hasNews,
  isUntrusted,
  nearbyOf,
  reportsOf,
  selfOf,
  untrustedIn,
  untrustedOf,
  type Observation,
} from "../src/index.js";
import { FakeWorld } from "../src/testing/index.js";

test("`isUntrusted` accepts only the marked form", () => {
  assert.equal(isUntrusted({ untrusted: true, from: 7, text: "hi" }), true);
  // A reader refuses `untrusted` false or absent.
  assert.equal(isUntrusted({ untrusted: false, from: 7, text: "hi" }), false);
  assert.equal(isUntrusted({ from: 7, text: "hi" }), false);
  assert.equal(isUntrusted({ untrusted: "true", from: 7, text: "hi" }), false);
  assert.equal(isUntrusted({ untrusted: true, text: "hi" }), false);
  assert.equal(isUntrusted({ untrusted: true, from: 7 }), false);
  assert.equal(isUntrusted("hi"), false);
  assert.equal(isUntrusted(null), false);
  assert.equal(isUntrusted([{ untrusted: true, from: 7, text: "hi" }]), false);
});

test("`untrustedIn` finds text of other residents at any depth", () => {
  const data = {
    offer: { price: 5, note: { untrusted: true, from: 3, text: "cheap" } },
    replies: [{ untrusted: true, from: 4, text: "no" }, "plain", { untrusted: false, from: 5, text: "skip" }],
  };
  assert.deepEqual(
    untrustedIn(data).map((entry) => [entry.from, entry.text]),
    [
      [3, "cheap"],
      [4, "no"],
    ],
  );
  assert.deepEqual(untrustedIn(42), []);
});

test("the helpers read the sections of an observation", () => {
  const world = new FakeWorld({
    sites: [{ id: 9, kind: "shop", name: "Corner Shop", rect: { x0: 30, y0: 30, x1: 36, y1: 36 }, tags: [] }],
  });
  const owner = world.signUp().owner;
  const ada = world.spawn(owner, { name: "Ada" });
  const bob = world.spawn(owner, { name: "Bob" });
  bob.pos = { x: 35, y: 36 };
  ada.cash = 1250;
  ada.items["bread"] = 2;
  ada.sections["board"] = { posts: [{ untrusted: true, from: bob.id, text: "Vote for me" }] };

  const quiet = world.observe(ada);
  assert.equal(hasNews(quiet), false);
  assert.deepEqual(reportsOf(quiet), []);
  const me = selfOf(quiet);
  assert.deepEqual(
    [me?.name, me?.cash, me?.items, me?.site, me?.activity, me?.queue],
    ["Ada", 1250, { bread: 2 }, { id: 9, name: "Corner Shop", kind: "shop" }, null, 0],
  );
  const nearby = nearbyOf(quiet);
  assert.deepEqual(nearby?.residents, [{ id: bob.id, name: "Bob", pos: { x: 35, y: 36 } }]);
  assert.deepEqual(nearby?.sites, [{ id: 9, name: "Corner Shop", kind: "shop", pos: { x: 32, y: 32 } }]);

  world.tell(ada.id, bob.id, "Give me your bread");
  world.submit(ada, { verb: "wait", args: { ticks: 1 } });
  world.step(1);
  const busy = world.observe(ada);
  assert.equal(hasNews(busy), true);
  assert.deepEqual(reportsOf(busy).map((report) => report.verb), ["wait"]);
  assert.deepEqual(
    untrustedOf(busy).map((entry) => entry.text),
    ["Give me your bread", "Vote for me"],
  );

  const bare: Observation = { ...busy, sections: {}, notices: [] };
  assert.equal(hasNews(bare), false);
  assert.equal(selfOf(bare), undefined);
  assert.equal(nearbyOf(bare), undefined);
});
