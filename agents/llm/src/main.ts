#!/usr/bin/env node
// The entry point of the command `unsupervised-llm-agent`. See README.md.

import { main } from "./cli.js";

const controller = new AbortController();
for (const name of ["SIGINT", "SIGTERM"] as const) {
  process.once(name, () => controller.abort());
}
process.exitCode = await main(process.env, { signal: controller.signal });
