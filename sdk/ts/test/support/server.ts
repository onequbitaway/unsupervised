// A local HTTP and WebSocket server in front of the fake gateway. The tests
// of the client use it, so that `fetch` and `WebSocket` do real work.

import { createHash } from "node:crypto";
import http from "node:http";
import type { AddressInfo } from "node:net";
import type { Duplex } from "node:stream";

import { FakeGateway, type FakeSocket, type FakeWorld } from "../../src/testing/index.js";

const WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const OP_TEXT = 0x1;
const OP_CLOSE = 0x8;
const OP_PING = 0x9;
const OP_PONG = 0xa;

/** Encodes one unmasked WebSocket frame, as a server sends it. */
function encodeFrame(opcode: number, payload: Buffer): Buffer {
  const length = payload.length;
  let header: Buffer;
  if (length < 126) {
    header = Buffer.from([0x80 | opcode, length]);
  } else if (length < 65_536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(length), 2);
  }
  return Buffer.concat([header, payload]);
}

/** Takes complete frames of a client from a buffer. Returns the frames and the rest. */
function decodeFrames(buffer: Buffer): { frames: { opcode: number; payload: Buffer }[]; rest: Buffer } {
  const frames: { opcode: number; payload: Buffer }[] = [];
  let offset = 0;
  for (;;) {
    if (buffer.length - offset < 2) break;
    const opcode = (buffer[offset] as number) & 0x0f;
    const second = buffer[offset + 1] as number;
    const masked = (second & 0x80) !== 0;
    let length = second & 0x7f;
    let cursor = offset + 2;
    if (length === 126) {
      if (buffer.length - cursor < 2) break;
      length = buffer.readUInt16BE(cursor);
      cursor += 2;
    } else if (length === 127) {
      if (buffer.length - cursor < 8) break;
      length = Number(buffer.readBigUInt64BE(cursor));
      cursor += 8;
    }
    const maskLength = masked ? 4 : 0;
    if (buffer.length - cursor < maskLength + length) break;
    const mask = buffer.subarray(cursor, cursor + maskLength);
    cursor += maskLength;
    const payload = Buffer.from(buffer.subarray(cursor, cursor + length));
    if (masked) {
      for (let i = 0; i < payload.length; i += 1) {
        payload[i] = (payload[i] as number) ^ (mask[i % 4] as number);
      }
    }
    frames.push({ opcode, payload });
    offset = cursor + length;
  }
  return { frames, rest: buffer.subarray(offset) };
}

/** A running test server. */
export interface TestServer {
  /** Address of the server, for `baseUrl`. */
  readonly url: string;
  readonly gateway: FakeGateway;
  readonly world: FakeWorld;
  /** How many WebSocket upgrades the server saw, also the refused ones. */
  readonly upgrades: number;
  /** The path and the query of each WebSocket upgrade, in order. */
  readonly upgradeUrls: string[];
  /** The text of each WebSocket frame that a client sent, in order. */
  readonly frames: string[];
  /** The next `count` HTTP requests are handled, but the answer is lost: the connection is cut. */
  loseResponses(count: number): void;
  /** The next `count` HTTP requests are cut before the gateway sees them. */
  refuseRequests(count: number): void;
  /** The next `count` WebSocket frames for clients are lost. */
  loseFrames(count: number): void;
  /** Cuts every WebSocket connection without a close frame. */
  dropSockets(): void;
  /** Resolves when the gateway handled the next frame of a client. */
  frameHandled(): Promise<void>;
  /** Stops the server. */
  close(): Promise<void>;
}

/** Starts a server on a free local port. */
export async function startServer(gateway: FakeGateway = new FakeGateway()): Promise<TestServer> {
  let lose = 0;
  let refuse = 0;
  let loseFrames = 0;
  let upgrades = 0;
  const upgradeUrls: string[] = [];
  const frames: string[] = [];
  let frameWaiters: (() => void)[] = [];
  const sockets = new Map<Duplex, FakeSocket>();

  const server = http.createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => {
      if (refuse > 0) {
        refuse -= 1;
        request.socket.destroy();
        return;
      }
      const headers: Record<string, string> = {};
      for (const [name, value] of Object.entries(request.headers)) {
        if (typeof value === "string") headers[name] = value;
      }
      const body = Buffer.concat(chunks).toString("utf8");
      void gateway
        .handle({ method: request.method ?? "GET", url: request.url ?? "/", headers, body })
        .then((answer) => {
          if (lose > 0) {
            lose -= 1;
            request.socket.destroy();
            return;
          }
          response.writeHead(answer.status, answer.headers);
          response.end(answer.body);
        });
    });
  });

  server.on("upgrade", (request, socket) => {
    upgrades += 1;
    const authorization = request.headers.authorization;
    const key = authorization?.startsWith("Bearer ") === true ? authorization.slice(7) : undefined;
    // A frame that the gateway pushes at once waits for the handshake.
    let handshake = false;
    const early: string[] = [];
    const send = (text: string): void => {
      if (!handshake) {
        early.push(text);
        return;
      }
      if (loseFrames > 0) {
        loseFrames -= 1;
        return;
      }
      socket.write(encodeFrame(OP_TEXT, Buffer.from(text, "utf8")));
    };
    // The query of the upgrade request has the cursors of a client that connects again.
    const target = new URL(request.url ?? "/", "http://fake");
    const cursor = (name: string): number | undefined => {
      const text = target.searchParams.get(name);
      return text === null ? undefined : Number(text);
    };
    const since = cursor("since");
    const reportsSince = cursor("reports_since");
    upgradeUrls.push(`${target.pathname}${target.search}`);
    const connection =
      target.pathname === "/v1/ws"
        ? gateway.connect(key, send, {
            ...(since === undefined ? {} : { since }),
            ...(reportsSince === undefined ? {} : { reportsSince }),
          })
        : undefined;
    if (connection === undefined) {
      socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
      return;
    }
    const accept = createHash("sha1")
      .update(`${request.headers["sec-websocket-key"]}${WEBSOCKET_GUID}`)
      .digest("base64");
    socket.write(
      `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    handshake = true;
    early.splice(0).forEach(send);
    sockets.set(socket, connection);
    let buffer: Buffer = Buffer.alloc(0);
    socket.on("data", (chunk: Buffer) => {
      const decoded = decodeFrames(Buffer.concat([buffer, chunk]));
      buffer = decoded.rest;
      for (const frame of decoded.frames) {
        if (frame.opcode === OP_TEXT) {
          frames.push(frame.payload.toString("utf8"));
          connection.receive(frame.payload.toString("utf8"));
          for (const waiter of frameWaiters.splice(0)) waiter();
        } else if (frame.opcode === OP_PING) {
          socket.write(encodeFrame(OP_PONG, frame.payload));
        } else if (frame.opcode === OP_CLOSE) {
          socket.end(encodeFrame(OP_CLOSE, frame.payload));
        }
      }
    });
    const forget = (): void => {
      connection.close();
      sockets.delete(socket);
    };
    socket.on("close", forget);
    socket.on("error", forget);
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}`,
    gateway,
    world: gateway.world,
    get upgrades() {
      return upgrades;
    },
    upgradeUrls,
    frames,
    loseResponses(count) {
      lose = count;
    },
    refuseRequests(count) {
      refuse = count;
    },
    loseFrames(count) {
      loseFrames = count;
    },
    dropSockets() {
      for (const [socket, connection] of sockets) {
        connection.close();
        socket.destroy();
      }
      sockets.clear();
    },
    frameHandled() {
      return new Promise((resolve) => {
        frameWaiters.push(resolve);
      });
    },
    close() {
      frameWaiters = [];
      for (const socket of sockets.keys()) socket.destroy();
      sockets.clear();
      gateway.world.expireWaits();
      server.closeAllConnections();
      return new Promise((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
      });
    },
  };
}

/** Collects values and lets a test await a number of them. */
export class Collector<T> {
  readonly items: T[] = [];
  #waiters: { count: number; resolve: (items: T[]) => void }[] = [];

  /** Adds a value. */
  readonly push = (item: T): void => {
    this.items.push(item);
    const ready = this.#waiters.filter((waiter) => this.items.length >= waiter.count);
    this.#waiters = this.#waiters.filter((waiter) => !ready.includes(waiter));
    for (const waiter of ready) waiter.resolve(this.items);
  };

  /** Resolves with the values when there are at least `count` of them. */
  atLeast(count: number): Promise<T[]> {
    if (this.items.length >= count) return Promise.resolve(this.items);
    return new Promise((resolve) => {
      this.#waiters.push({ count, resolve });
    });
  }
}
