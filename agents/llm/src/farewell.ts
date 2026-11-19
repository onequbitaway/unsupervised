// The last line of an agent whose key will not work again.
//
// A resident that left the world gives two answers. The first call with
// its key gets `410 resident_gone`. Each later call gets `401
// invalid_token`: the server forgot the key. A key that another key
// replaced gets the same 401. The owner token tells the two apart: it
// lists the residents that are in the world.

import { ApiError, UnsupervisedError, type Client } from "@unsupervised/sdk";

/** The variable that gives the agent the key of a resident that exists. */
const KEY_VARIABLE = "UNSUPERVISED_RESIDENT_KEY";
/** The variable that gives the agent an owner token. */
const OWNER_VARIABLE = "UNSUPERVISED_OWNER_TOKEN";

/** What became of the resident of a key that the server refuses. */
export type Fate = "gone" | "rekeyed" | "unknown";

/**
 * Asks the server what became of the resident of a client whose key the
 * server does not know. Only a client with an owner token can ask.
 */
export async function fateOf(client: Client | undefined): Promise<Fate> {
  if (client?.ownerToken === undefined || client.resident === undefined) return "unknown";
  try {
    const { residents } = await client.listResidents();
    return residents.some((resident) => resident.id === client.resident) ? "rekeyed" : "gone";
  } catch (error) {
    // The reason stays unknown. The first error is the one that the user must see.
    if (error instanceof UnsupervisedError) return "unknown";
    throw error;
  }
}

/**
 * Returns the sentence that the command writes when the server refuses
 * its key for good. `client` is the client of the resident, when the agent
 * got so far. `keyFromEnvironment` says that the key came from the
 * environment and not from the state file: then a new start does not make
 * a new resident.
 */
export async function farewell(error: ApiError, client: Client | undefined, keyFromEnvironment: boolean): Promise<string> {
  if (error.code !== "resident_gone" && error.status !== 401) return error.message;
  const who = client?.resident === undefined ? "the resident" : `resident ${client.resident}`;
  const fate = error.code === "resident_gone" ? "gone" : await fateOf(client);
  if (fate === "gone") {
    const next = keyFromEnvironment ? `Remove ${KEY_VARIABLE} to start a new resident.` : "Start the agent again to make a new resident.";
    return `${who} left the world: it died, or an administrator removed it. Its key does not work any more. ${next}`;
  }
  if (fate === "rekeyed") {
    const next = keyFromEnvironment ? `Set ${KEY_VARIABLE} to the new key.` : "Start the agent again to get a new key.";
    return `the server does not know the key of ${who} any more: another key replaced it. ${next}`;
  }
  // Without an owner token the agent cannot ask. The message of the server names two reasons; this is the third.
  const next = keyFromEnvironment
    ? `Check ${KEY_VARIABLE}, or remove it to start a new resident.`
    : `Set ${OWNER_VARIABLE} so that the agent can find out, or move the state file away to start a new resident.`;
  return `${error.message} Or its resident left the world: it died, or an administrator removed it. ${next}`;
}
