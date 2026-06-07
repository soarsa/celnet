/**
 * Playwright globalSetup — boot the REAL celnet-server demo edge once for the
 * whole e2e run and record its WebSocket-mirror URL for the specs.
 */
import { writeFileSync } from "node:fs";

import { startDemoEdge } from "./demoEdge";
import { setEdge } from "./edgeHandle";
import { WS_URL_FILE } from "./wsUrl";

export default async function globalSetup(): Promise<void> {
  const edge = await startDemoEdge();
  setEdge(edge);
  writeFileSync(WS_URL_FILE, edge.wsUrl, "utf8");
  process.stdout.write(`[e2e] demo edge ready at ${edge.wsUrl}\n`);
}
