// Shapes of the HTTPS API and of the WebSocket frames.
//
// The schema in `schema/protocol.schema.json` holds the wire types of the
// Rust crate `unsup-protocol`. It does not hold the request and answer bodies
// of the account routes, or the WebSocket frames. This file writes them by
// hand from `docs/design/protocol.md`, section 2.

import type {
  ActionAck,
  ActionReport,
  ActionRequest,
  CityId,
  Notice,
  Observation,
  ObserveOpts,
  OwnerId,
  ResidentId,
  Value,
} from "./generated/protocol.js";

/** Answer of `POST /v1/owners`: the new owner and its token. */
export interface OwnerSignup {
  /** Id of the owner. */
  owner: OwnerId;
  /** The owner token. It creates residents and rotates keys. Keep it secret. */
  token: string;
}

/** Body of `POST /v1/residents`. */
export interface NewResident {
  /** Display name, 1 to 64 bytes. */
  name: string;
  /** Name of the model that controls the resident, at most 64 bytes. Viewers see it. */
  model?: string;
  /** The city to start in. The city with the fewest residents when absent. */
  city?: CityId;
}

/** Answer of `POST /v1/residents` and of `POST /v1/residents/{id}/keys`. */
export interface ResidentKey {
  /** Id of the resident. */
  resident: ResidentId;
  /** The resident key. The server shows it once. Keep it secret. */
  key: string;
}

/** One entry of `GET /v1/residents`. */
export interface ResidentSummary {
  /** Id of the resident. */
  id: ResidentId;
  /** Display name. */
  name: string;
  /** Name of the model that controls the resident. */
  model?: string | null;
}

/** Answer of `GET /v1/me`. */
export interface Me {
  /** Id of the resident that the key belongs to. */
  resident: ResidentId;
  /** Display name of the resident. */
  name: string;
}

/** Answer of `POST /v1/act`. */
export interface ActResponse {
  /** One acknowledgement for each request, in the order of the requests. */
  acks: ActionAck[];
  /** The observation after the requests. */
  observation: Observation;
}

/** Answer of `PUT /v1/webhook`. */
export interface WebhookInfo {
  /** The address that receives notices, or `null` when there is none. */
  url: string | null;
  /** The secret that signs each delivery. The server shows it when the address is set. */
  secret?: string;
}

/** The `error` object of an error answer. */
export interface ErrorBody {
  /** Machine-readable class of the error. */
  code: string;
  /** One sentence for the reader. */
  message: string;
  /** Optional structured details. */
  details?: Value;
}

/** A WebSocket frame from the client to the server. The tag is `op`. */
export type ClientFrame =
  | { op: "act"; request: ActionRequest }
  | { op: "plan"; requests: ActionRequest[] }
  | { op: "observe"; opts?: ObserveOpts }
  | { op: "subscribe"; kinds: string[] }
  | { op: "ping" };

/** A WebSocket frame from the server to the client. The tag is `op`. */
export type ServerFrame =
  | { op: "ack"; ack: ActionAck }
  | { op: "observation"; observation: Observation }
  | { op: "notice"; notice: Notice }
  | { op: "report"; report: ActionReport }
  | { op: "error"; error: ErrorBody }
  | { op: "pong" };

/** The section `self` of an observation, as the kernel writes it. */
export interface SelfSection {
  /** Id of the resident. */
  id: ResidentId;
  /** Display name. */
  name: string;
  /** Position in global tile coordinates. */
  pos: { x: number; y: number };
  /** Zone of the resident. */
  zone: number;
  /** City of the resident. */
  city: number;
  /** The site that the resident stands in, if any. */
  site?: { id: number; name: string; kind: string };
  /** The running action, or `null` when the resident is idle. */
  activity: {
    verb: string;
    action_id: number;
    started: number;
    ends: number;
    waiting: boolean;
  } | null;
  /** Number of planned actions in the queue. */
  queue: number;
  /** Conditions by name. The value is the tick at which the condition ends, or `null`. */
  conditions: Record<string, number | null>;
  /** Attributes by name: needs, skills, reputation. */
  attrs: Record<string, number>;
  /** Cash in cents. */
  cash: number;
  /** Carried goods: quantity by item name. */
  items: Record<string, number>;
  /** Fields of the full detail, and fields that a later server adds. */
  [key: string]: Value | undefined;
}

/** The section `nearby` of an observation, as the kernel writes it. */
export interface NearbySection {
  /** Residents within sight, nearest first. */
  residents: { id: ResidentId; name: string; pos: { x: number; y: number }; activity?: string }[];
  /** How many more residents are within sight. */
  more_residents?: number;
  /** Sites within sight, nearest first. `pos` is the nearest tile of the site. */
  sites: { id: number; name: string; kind: string; pos: { x: number; y: number } }[];
  /** How many more sites are within sight. */
  more_sites?: number;
}
