/**
 * Playwright globalTeardown — stop the demo edge booted in globalSetup.
 */
import { takeEdge } from "./edgeHandle";

export default async function globalTeardown(): Promise<void> {
  const edge = takeEdge();
  if (edge) await edge.stop();
}
