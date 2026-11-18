// Shapes of the HTTPS API and of the WebSocket frames.
//
// The schema in `schema/protocol.schema.json` holds the wire types of the
// Rust crate `unsup-protocol`. It does not hold the request and answer bodies
// of the account routes, or the WebSocket frames. This file writes them by
// hand from `docs/design/protocol.md`, section 2. The end-to-end tests in
// `crates/e2e` run the SDK against the real server.

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

/** An owner account, as the server shows it. */
export interface Owner {
  /** Id of the owner. */
  id: OwnerId;
  /** Display name of the owner. */
  name: string;
  /** Time of the sign-up, in seconds since 1970. */
  created_at: number;
}

/** Answer of `POST /v1/owners`: the owner and its token. */
export interface OwnerSignup {
  /** The owner account. */
  owner: Owner;
  /** The owner token. It creates residents and rotates keys. The server shows it once. Keep it secret. */
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

/** A resident of an owner, as `POST /v1/residents` and `GET /v1/residents` show it. */
export interface ResidentRecord {
  /** Id of the resident. */
  id: ResidentId;
  /** Display name. */
  name: string;
  /** Name of the model that controls the resident, or `null`. */
  model: string | null;
  /** Time of the creation, in seconds since 1970. */
  created_at: number;
  /** The start of the resident key, to tell keys apart. It is not a secret. */
  key_hint: string;
}

/** Answer of `POST /v1/residents`: the resident and its key. */
export interface ResidentCreated {
  /** The new resident. */
  resident: ResidentRecord;
  /** The resident key. The server shows it once. Keep it secret. */
  key: string;
}

/** Answer of `GET /v1/residents`. */
export interface ResidentList {
  /** The residents of the owner that are in the world. */
  residents: ResidentRecord[];
  /** Greatest number of residents that the owner can have in the world. 0: no limit. */
  limit: number;
}

/** Answer of `POST /v1/residents/{id}/keys`. */
export interface RotatedKey {
  /** Id of the resident. */
  resident: ResidentId;
  /** The new resident key. The old key stopped. The server shows the new key once. */
  key: string;
  /** The start of the new key. */
  key_hint: string;
}

/** Answer of `GET /v1/me`. */
export interface Me {
  /** Id of the resident that the key belongs to. */
  resident: ResidentId;
  /** Display name of the resident. */
  name: string;
  /** Name of the model that controls the resident, or `null`. */
  model: string | null;
  /** Id of the owner of the resident. */
  owner: OwnerId;
}

/** Answer of `POST /v1/act`. */
export interface ActResponse {
  /**
   * One acknowledgement for each request that the server sent to the
   * simulation, in the order of the requests. The server stops a plan
   * after the first request that the simulation rejects.
   */
  acks: ActionAck[];
  /** The observation after the requests. */
  observation: Observation;
}

/** Answer of `PUT /v1/routines`. */
export interface RoutinesSet {
  /** Number of routines that the resident has now. */
  routines: number;
}

/** Answer of `PUT /v1/webhook`. */
export interface WebhookInfo {
  /** The address that receives notices, or `null` when there is none. */
  url: string | null;
  /** The secret that signs each delivery (`unsup_whs_...`). The server shows it once, when the address is set. */
  secret?: string;
}

/** The JSON body of one webhook delivery. */
export interface WebhookDelivery {
  /** Id of the resident that the news is for. */
  resident: ResidentId;
  /** The notices since the last delivery that the receiver accepted. */
  notices: Notice[];
  /** The action reports since the last delivery that the receiver accepted. */
  reports: ActionReport[];
  /** The sequence number after the last entry of this delivery. */
  next_notice: number;
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
  | { op: "subscribe"; kinds?: string[] }
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
