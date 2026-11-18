// The verbs that a new fake world has: the three kernel verbs and `move_to`.

import type { Routine, Value } from "../generated/protocol.js";
import { FakeActionError, distance, type FakeVerb } from "./fake-world.js";

/** Returns the default verbs: `wait`, `cancel`, `set_routines` and `move_to`. */
export function defaultVerbs(): FakeVerb[] {
  return [
    {
      info: {
        verb: "wait",
        module: "kernel",
        summary: "Do nothing for a number of ticks.",
        free: false,
        scope: "zone",
        args_schema: {
          type: "object",
          properties: { ticks: { type: "integer", minimum: 0, description: "How many ticks to do nothing." } },
          required: ["ticks"],
        },
      },
      begin(_resident, args, world) {
        const ticks = args["ticks"] as number;
        const max = world.limits.wait_max;
        if (ticks < 1 || ticks > max) {
          throw new FakeActionError("invalid_args", `\`ticks\` must be from 1 to ${max}.`);
        }
        return { busy: ticks, finish: () => ({ waited: ticks }) };
      },
    },
    {
      info: {
        verb: "cancel",
        module: "kernel",
        summary: "Stop the current action and clear the queue of planned actions.",
        free: true,
        scope: "zone",
        args_schema: { type: "object" },
      },
      begin(resident, _args, world) {
        return { done: world.cancel(resident) };
      },
    },
    {
      info: {
        verb: "set_routines",
        module: "kernel",
        summary: "Replace the routines: standing orders that run when the resident is idle.",
        free: true,
        scope: "zone",
        args_schema: {
          type: "object",
          properties: { routines: { type: "array", description: "The new routines, in priority order." } },
          required: ["routines"],
        },
      },
      begin(resident, args, world) {
        return { done: world.setRoutines(resident, args["routines"] as unknown as Routine[]) };
      },
    },
    {
      info: {
        verb: "move_to",
        module: "life",
        summary: "Walk to a tile or a site. One tile takes one game minute.",
        free: false,
        scope: "zone",
        args_schema: {
          type: "object",
          properties: {
            x: { type: ["integer", "null"], description: "Column of the target tile." },
            y: { type: ["integer", "null"], description: "Row of the target tile." },
            site: { type: ["integer", "null"], description: "Id of a site. The walk ends inside the site." },
          },
        },
      },
      begin(resident, args, world) {
        const x = args["x"] ?? null;
        const y = args["y"] ?? null;
        const site = args["site"] ?? null;
        if ((site === null) === (x === null && y === null) || (x === null) !== (y === null)) {
          throw new FakeActionError("invalid_args", "Give `x` and `y`, or `site`.");
        }
        let target = { x: x as number, y: y as number };
        if (site !== null) {
          const found = world.sites.find((one) => one.id === site);
          if (found === undefined) throw new FakeActionError("not_found", "No site of your city has this id.");
          const { rect } = found;
          target = { x: Math.floor((rect.x0 + rect.x1) / 2), y: Math.floor((rect.y0 + rect.y1) / 2) };
        }
        const steps = distance(resident.pos, target);
        const finish = (): Value => {
          resident.pos = target;
          return { ...target, steps, ticks: Math.max(1, steps), ...(site === null ? {} : { site }) };
        };
        return { busy: Math.max(1, steps), finish };
      },
    },
  ];
}
