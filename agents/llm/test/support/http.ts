// A local HTTP server for the tests that start the command as a process.

import http from "node:http";
import type { AddressInfo } from "node:net";

/** One request, as the handler gets it. */
export interface HttpRequest {
  method: string;
  /** Path and query. */
  url: string;
  /** Headers with lowercase names. */
  headers: Record<string, string>;
  body: string;
}

/** One answer of the handler. */
export interface HttpAnswer {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/** A running server. */
export interface LocalServer {
  /** Address of the server, without a slash at the end. */
  readonly url: string;
  /** Stops the server and cuts its connections. */
  close(): Promise<void>;
}

/** Starts a server on a free local port. `handler` answers each request. */
export async function serve(handler: (request: HttpRequest) => Promise<HttpAnswer> | HttpAnswer): Promise<LocalServer> {
  const server = http.createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => {
      const headers: Record<string, string> = {};
      for (const [name, value] of Object.entries(request.headers)) {
        if (typeof value === "string") headers[name] = value;
      }
      void Promise.resolve(
        handler({ method: request.method ?? "GET", url: request.url ?? "/", headers, body: Buffer.concat(chunks).toString("utf8") }),
      ).then((answer) => {
        response.writeHead(answer.status, answer.headers);
        response.end(answer.body);
      });
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}`,
    close() {
      server.closeAllConnections();
      return new Promise((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
      });
    },
  };
}
