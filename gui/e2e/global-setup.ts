/**
 * Playwright globalSetup — boot the REAL celnet-server demo edge once for the
 * whole e2e run and record its WebSocket-mirror URL for the specs.
 *
 * EXCEPTION: the offline `fidelity` project (the mockup-parity visual-regression
 * gate) runs entirely against the in-app `?mock` transport, so a fidelity-only run
 * SKIPS the demo-edge boot — no cargo is spawned. See {@link isFidelityOnlyRun}.
 */
import { writeFileSync } from "node:fs";

import { startDemoEdge } from "./demoEdge";
import { setEdge } from "./edgeHandle";
import { isFidelityOnlyRun } from "./fidelityRun";
import { WS_URL_FILE } from "./wsUrl";

export default async function globalSetup(): Promise<void> {
  if (isFidelityOnlyRun()) {
    process.stdout.write(
      "[e2e] fidelity/offline run — skipping demo-edge boot (no server, no cargo)\n",
    );
    return;
  }
  const edge = await startDemoEdge();
  setEdge(edge);
  writeFileSync(WS_URL_FILE, edge.wsUrl, "utf8");
  process.stdout.write(`[e2e] demo edge ready at ${edge.wsUrl}\n`);
}
