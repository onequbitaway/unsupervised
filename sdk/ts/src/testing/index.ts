// Test support: a small model of the server that needs no network.
//
//   import { FakeGateway } from "@unsupervised/sdk/testing";
//   const gateway = new FakeGateway();
//   const client = new Client({ baseUrl: "http://fake", fetch: gateway.fetch });

export { DEFAULT_LIMITS, FakeActionError, FakeWorld, byteLength, distance } from "./fake-world.js";
export type {
  FakeLimits,
  FakeResident,
  FakeVerb,
  FakeWorldOptions,
  InboxEntry,
  InboxListener,
  VerbOutcome,
} from "./fake-world.js";
export { FakeGateway } from "./fake-gateway.js";
export type { FakeRequest, FakeResponse, FakeSocket, Fault, LoggedRequest } from "./fake-gateway.js";
export { defaultVerbs } from "./verbs.js";
