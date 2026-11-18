// Shared set-up of the client tests.

import type { TestContext } from "node:test";

import { Client, type ClientOptions } from "../../src/index.js";
import type { FakeResident } from "../../src/testing/index.js";
import { FakeGateway, FakeWorld, type FakeWorldOptions } from "../../src/testing/index.js";
import { startServer, type TestServer } from "./server.js";

/** A server, a resident of its world and a client for the resident. */
export interface Scene {
  server: TestServer;
  world: FakeWorld;
  gateway: FakeGateway;
  client: Client;
  resident: FakeResident;
  /** Delays that the client asked for between attempts, in milliseconds. */
  sleeps: number[];
}

/**
 * Starts a server with one owner and one resident named Ada, and a client
 * with her key. The client does not sleep between attempts: it records the
 * delay. The server stops when the test ends.
 */
export async function scene(
  t: TestContext,
  options: FakeWorldOptions = {},
  client: Partial<ClientOptions> = {},
): Promise<Scene> {
  const world = new FakeWorld(options);
  const gateway = new FakeGateway(world);
  const server = await startServer(gateway);
  t.after(() => server.close());
  const owner = world.signUp();
  const resident = world.spawn(owner.owner, { name: "Ada" });
  const sleeps: number[] = [];
  return {
    server,
    world,
    gateway,
    resident,
    sleeps,
    client: new Client({
      baseUrl: server.url,
      residentKey: resident.key,
      ownerToken: owner.token,
      sleep: async (milliseconds) => {
        sleeps.push(milliseconds);
      },
      retryDelay: (attempt, retryAfterSeconds) => (retryAfterSeconds === undefined ? attempt : retryAfterSeconds * 1000),
      ...client,
    }),
  };
}
