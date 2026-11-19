// The fake world: the parts that agent tests rely on and that the client
// tests do not reach. The routines run as the kernel runs them.

import assert from "node:assert/strict";
import { test } from "node:test";

import type { ActionReport, AttrInfo, Condition, ItemInfo, Routine, Value } from "../src/index.js";
import { FakeActionError, FakeWorld, type FakeResident, type FakeVerb } from "../src/testing/index.js";

const ENERGY: AttrInfo = { name: "need.energy", min: 0, max: 1000, default: 1000, summary: "0 means exhausted." };
const BREAD: ItemInfo = { id: 0, name: "bread", weight: 1, tags: ["food"], attrs: { nutrition: 300 }, summary: "A loaf." };

function verb(name: string, begin: FakeVerb["begin"]): FakeVerb {
  return { info: { verb: name, module: "life", summary: `Do ${name}.`, free: false, scope: "zone", args_schema: { type: "object" } }, begin };
}

/** A world with a verb that takes time (`nap`) and one that ends at once (`snack`), and one resident. */
function town(): { world: FakeWorld; ada: FakeResident } {
  const world = new FakeWorld({ attrs: [ENERGY], items: [BREAD] });
  world.addVerb(
    verb("nap", (resident) => ({
      busy: 10,
      finish: () => {
        resident.attrs["need.energy"] = 1000;
        return { rested: true };
      },
    })),
  );
  world.addVerb(
    verb("snack", (resident) => {
      if ((resident.items["bread"] ?? 0) < 1) throw new FakeActionError("insufficient", "You have no bread.");
      resident.items["bread"] = (resident.items["bread"] ?? 0) - 1;
      return { done: { ate: "bread" } };
    }),
  );
  return { world, ada: world.spawn(world.signUp().owner.id, { name: "Ada" }) };
}

/** Returns the routines of a resident as the full observation shows them. */
function shown(world: FakeWorld, resident: FakeResident): { name: string; runs: number; failures: number }[] {
  const me = world.observe(resident, { detail: "full" }).sections["self"] as { routines: { name: string; runs: number; failures: number }[] };
  return me.routines;
}

function reports(world: FakeWorld, resident: FakeResident): ActionReport[] {
  return world.observe(resident).sections["reports"] as unknown as ActionReport[];
}

/** Returns the verb of the running action. A function, because a test reads it again after the world moved. */
function doing(resident: FakeResident): string | undefined {
  return resident.current?.verb;
}

function routine(name: string, when: Condition, then: string, args?: Value): Routine {
  return { name, when, then: { verb: then, ...(args === undefined ? {} : { args }) } };
}

test("a routine fires when the resident is idle and its condition holds, and a success writes no report", () => {
  const { world, ada } = town();
  world.setRoutines(ada, [routine("nap", { type: "attr_below", attr: "need.energy", value: 300 }, "nap")]);
  world.step(5);
  assert.equal(doing(ada), undefined);
  assert.deepEqual(shown(world, ada), [{ ...ada.routines[0], runs: 0, failures: 0 }]);

  ada.attrs["need.energy"] = 200;
  world.step(1);
  assert.equal(doing(ada), "nap");
  world.step(10);
  assert.equal(ada.attrs["need.energy"], 1000);
  assert.deepEqual([shown(world, ada)[0]?.runs, shown(world, ada)[0]?.failures], [1, 0]);
  // A routine action that goes well is counted and writes no report.
  assert.deepEqual(reports(world, ada), []);
});

test("the first routine that holds wins, a busy resident starts none, and one routine fires in one tick", () => {
  const { world, ada } = town();
  ada.items["bread"] = 1;
  world.setRoutines(ada, [
    routine("eat", { type: "has_item", item: "bread", qty: 1 }, "snack"),
    routine("nap", { type: "always" }, "nap"),
  ]);
  world.submit(ada, { verb: "wait", args: { ticks: 3 } });
  world.step(2);
  assert.deepEqual([doing(ada), ada.items["bread"]], ["wait", 1]);
  // The wait ends at tick 3. The first routine that holds fires in the same tick.
  world.step(1);
  assert.equal(ada.items["bread"], 0);
  assert.equal(doing(ada), undefined);
  // One routine in one tick: the second one starts in the next tick.
  world.step(1);
  assert.deepEqual([doing(ada), ada.current?.started], ["nap", 4]);
});

test("a routine that fails reports and waits longer each time", () => {
  const { world, ada } = town();
  world.setRoutines(ada, [routine("eat", { type: "always" }, "snack")]);
  world.step(1);
  assert.deepEqual(
    reports(world, ada).map((report) => [report.verb, report.ok, report.error?.code, report.tick, report.request_id]),
    [["snack", false, "insufficient", 1, undefined]],
  );
  // The backoff is 30 ticks, then 60: the second failure is at tick 31, the third at tick 91.
  world.step(100);
  assert.deepEqual(reports(world, ada).map((report) => report.tick), [1, 31, 91]);
  assert.equal(shown(world, ada)[0]?.failures, 3);
});

test("a routine whose action ends at once waits before it fires again, and new routines start their counts at zero", () => {
  const { world, ada } = town();
  ada.items["bread"] = 5;
  world.setRoutines(ada, [routine("eat", { type: "always" }, "snack")]);
  world.step(61);
  // Ticks 1, 31 and 61: the backoff of a routine that finished at once.
  assert.equal(ada.items["bread"], 2);
  assert.equal(shown(world, ada)[0]?.runs, 3);
  world.setRoutines(ada, [routine("eat", { type: "always" }, "snack")]);
  assert.equal(shown(world, ada)[0]?.runs, 0);
});

test("a frozen resident starts no routine", () => {
  const { world, ada } = town();
  world.setRoutines(ada, [routine("nap", { type: "always" }, "nap")]);
  ada.conditions["frozen"] = null;
  world.step(3);
  assert.equal(doing(ada), undefined);
  delete ada.conditions["frozen"];
  world.step(1);
  assert.equal(doing(ada), "nap");
});

test("a clock condition follows the hour and wraps past midnight", () => {
  const { world, ada } = town();
  const night: Condition = { type: "clock_between", from_hour: 22, to_hour: 6 };
  const day: Condition = { type: "clock_between", from_hour: 6, to_hour: 22 };
  const seen: Record<number, boolean[]> = {};
  for (const hour of [0, 5, 6, 21, 22, 23]) {
    world.tick = hour * 60;
    seen[hour] = [world.holds(ada, night), world.holds(ada, day)];
  }
  assert.deepEqual(seen, { 0: [true, false], 5: [true, false], 6: [false, true], 21: [false, true], 22: [true, false], 23: [true, false] });
  const both: Condition = { type: "all", of: [night, { type: "attr_above", attr: "need.energy", value: 999 }] };
  assert.equal(world.holds(ada, both), true);
  ada.attrs["need.energy"] = 999;
  assert.equal(world.holds(ada, both), false);
  assert.equal(world.holds(ada, { type: "any", of: [both, night] }), true);
});
