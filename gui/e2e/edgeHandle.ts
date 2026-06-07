/**
 * A tiny module-scoped slot holding the booted demo edge between globalSetup and
 * globalTeardown (Playwright runs both in the same Node process). Kept in its own
 * module so neither file reaches for a `var`-typed global.
 */
import type { DemoEdge } from "./demoEdge";

let edge: DemoEdge | undefined;

export function setEdge(e: DemoEdge): void {
  edge = e;
}

export function takeEdge(): DemoEdge | undefined {
  const e = edge;
  edge = undefined;
  return e;
}
