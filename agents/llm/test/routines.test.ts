// The wait of the agent and the routines of the resident.
//
// The server starts a routine when the resident is idle: in the tick in
// which the last planned action ends. It writes no report when the action
// of a routine goes well. So the observation cannot tell the agent that
// its plan ended: the resident is busy again. The agent follows the
// action ids of its own plan.

import assert from "node:assert/strict";
import { test } from "node:test";

import type { AttrInfo, Routine } from "@unsupervised/sdk";
import { FakeActionError, type FakeVerb } from "@unsupervised/sdk/testing";

import { answer, scene } from "./support/scene.js";

const ENERGY = "need.energy";
const ATTRS: AttrInfo[] = [{ name: ENERGY, min: 0, max: 1000, default: 1000, summary: "Energy." }];
/** Ticks of one shift, and the energy that it costs. A resident is tired after it. */
const SHIFT = { ticks: 240, energy: 800 };
/** Ticks of one sleep. */
const NIGHT = 720;

const WORK: FakeVerb = {
  info: { verb: "work", module: "economy", summary: "Work one shift.", free: false, scope: "zone", args_schema: { type: "object" } },
  begin: (resident) => ({
    busy: SHIFT.ticks,
    finish: () => {
      resident.attrs[ENERGY] = (resident.attrs[ENERGY] ?? 0) - SHIFT.energy;
      return { wage: 2800 };
    },
  }),
};

const SLEEP: FakeVerb = {
  info: { verb: "sleep", module: "life", summary: "Sleep.", free: false, scope: "zone", args_schema: { type: "object" } },
  begin: (resident) => ({
    busy: NIGHT,
    finish: () => {
      resident.attrs[ENERGY] = 1000;
      return { quality: "poor" };
    },
  }),
};

/** The default routine of the game that sleeps where the resident stands. */
const SLEEP_WHEN_TIRED: Routine = {
  name: "sleep when tired",
  when: { type: "attr_below", attr: ENERGY, value: 250 },
  then: { verb: "sleep", args: {} },
};

/** A scene with the game verbs of these tests and a resident that has the routine. */
function town(answers: string[], limits: { inbox_max?: number } = {}): ReturnType<typeof scene> {
  const made = scene({ world: { attrs: ATTRS, limits }, answers, agent: { minDecisionSeconds: 0 } });
  made.world.addVerb(WORK);
  made.world.addVerb(SLEEP);
  made.world.setRoutines(made.resident, [SLEEP_WHEN_TIRED]);
  return made;
}

test("the report of the last planned action ends the wait, also when a routine starts an action in the same tick", async () => {
  const { agent, provider, world, resident } = town([
    answer({ actions: [{ verb: "work" }], wait_seconds: 3600 }),
    answer({ actions: [{ verb: "wait", args: { ticks: 5 } }], wait_seconds: 3600 }),
    answer({ wait_seconds: 60 }),
  ]);
  await agent.turn();
  // The shift ended at tick 240, and the routine put the tired resident to sleep in that tick.
  assert.equal(resident.current?.verb, "sleep");
  assert.equal(resident.current?.started, SHIFT.ticks);
  // The agent did not take the sleep for a part of its plan: it did not wait the hour that the model gave.
  assert.equal(world.tick, SHIFT.ticks);

  // The model sees that the resident sleeps. Its next action waits in the queue until the resident wakes.
  const second = await agent.turn();
  assert.equal(provider.data(2)["observation"].self.activity.verb, "sleep");
  assert.deepEqual(provider.data(2)["reports"], [{ verb: "work", ok: true, tick: SHIFT.ticks, result: { wage: 2800 } }]);
  assert.deepEqual(second.acks.map((ack) => [ack.verb, ack.status.state]), [["wait", "queued"]]);
  // The sleep wrote no report. The report of the planned action ended the wait, five ticks after the sleep.
  assert.equal(world.tick, SHIFT.ticks + NIGHT + 5);
  await agent.turn();
  assert.deepEqual(
    provider.data(3)["reports"].map((report: { verb: string; ok: boolean }) => [report.verb, report.ok]),
    [["wait", true]],
  );
});

test("a report of an action that the agent did not plan ends the wait only when the action failed", async () => {
  const well = town([answer({ wait_seconds: 120 })]);
  // Another client of the same resident starts an action. A server can also report each action of a routine.
  well.world.submit(well.resident, { verb: "wait", args: { ticks: 20 } });
  await well.agent.turn();
  assert.equal(well.world.tick, 120);

  const failed = town([answer({ wait_seconds: 120 })]);
  failed.world.addVerb({
    info: { verb: "dig", module: "economy", summary: "Dig.", free: false, scope: "zone", args_schema: { type: "object" } },
    begin: () => {
      throw new FakeActionError("not_here", "There is no ground to dig here.");
    },
  });
  failed.world.setRoutines(failed.resident, [{ name: "dig", when: { type: "always" }, then: { verb: "dig", args: {} } }]);
  await failed.agent.turn();
  // The routine fired in the first tick and failed. The model must know of it.
  assert.equal(failed.world.tick, 1);
});

test("a report that the inbox lost does not hold the agent in its later waits", async () => {
  const { agent, world, resident } = town(
    [
      answer({ actions: [{ verb: "wait", args: { ticks: 2 } }], wait_seconds: 600 }),
      answer({ actions: [{ verb: "wait", args: { ticks: 3 } }], wait_seconds: 600 }),
    ],
    { inbox_max: 1 },
  );
  // A notice comes in the tick of the report. The inbox keeps one entry, so the report is lost.
  world.addVerb({
    info: { verb: "listen", module: "society", summary: "Listen.", free: false, scope: "zone", args_schema: { type: "object" } },
    begin: () => ({ done: { heard: world.tell(resident.id, 2, "Good day.") } }),
  });
  world.setRoutines(resident, [{ name: "listen", when: { type: "always" }, then: { verb: "listen", args: {} } }]);
  await agent.turn();
  // No report ended the wait: the agent waited the whole time of the model.
  assert.equal(world.tick, 600);

  world.setRoutines(resident, []);
  await agent.turn();
  // The lost action is forgotten: the report of the new plan ends the wait.
  assert.equal(world.tick, 603);
});

test("a report that comes with the answer to the plan starts the next turn at once", async () => {
  const { client, provider, notes, world } = town([answer({ actions: [{ verb: "wait", args: { ticks: 1 } }], wait_seconds: 600 })]);
  // A server in real time: one tick passes before the server builds the observation of its answer.
  const live = {
    catalogue: () => client.catalogue(),
    observe: () => client.observe(),
    wait: (options: Parameters<typeof client.wait>[0]) => client.wait(options),
    plan: async (steps: Parameters<typeof client.plan>[0]) => {
      const since = client.cursor;
      const result = await client.plan(steps);
      world.step(1);
      return { ...result, observation: await client.observe({ since }) };
    },
  };
  const { LlmAgent } = await import("../src/index.js");
  const agent = new LlmAgent({ world: live, provider, notes, now: () => world.tick * 1000, minDecisionSeconds: 0 });
  const result = await agent.turn();
  assert.deepEqual(result.acks.map((ack) => ack.status.state), ["started"]);
  // The plan ended in the answer. A wait of 600 seconds would bring no news.
  assert.equal(result.waitedSeconds, 0);
  assert.equal(world.tick, 1);
});
