/**
 * vitest globalSetup — boot the REAL `celnet-server` demo edge ONCE for the whole
 * conformance e2e run, record its WebSocket-mirror URL, and return a teardown that
 * kills the child. No mock: the spec dials this real edge over a real socket.
 */
import { writeFileSync } from "node:fs";

import { startDemoEdge } from "./demoEdge";
import { WS_URL_FILE } from "./wsUrl";

export default async function setup(): Promise<() => Promise<void>> {
  const edge = await startDemoEdge();
  writeFileSync(WS_URL_FILE, edge.wsUrl, "utf8");
  process.stdout.write(`[excel-e2e] demo edge ready at ${edge.wsUrl}\n`);
  return async () => {
    await edge.stop();
  };
}
