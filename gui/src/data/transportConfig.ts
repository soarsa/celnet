/**
 * Transport selection: which `CelnetTransport` the app runs against. The default
 * is ALWAYS the deterministic in-app mock/replay source, so the GUI builds and
 * runs standalone with no server (the demo, the design review, and the strict
 * typecheck all work offline). A live WebSocket transport against `celnet-server`
 * is opted into by a build-time env flag — one contract, two transports
 * (src/data/transport.ts), selected here and nowhere else.
 *
 * Vite exposes `import.meta.env.VITE_*` at build time. Set either:
 *   - `VITE_CELNET_TRANSPORT=ws` (uses `VITE_CELNET_WS_URL`, default ws://127.0.0.1:8081), or
 *   - `VITE_CELNET_WS_URL=ws://host:port` (presence alone selects the live transport).
 * Anything else (including unset) keeps the mock. No runtime mixing: a build is
 * one transport, matching the platform's single-uniform-version deploy model.
 */

import { createMockTransport } from "./mockSource";
import type { CelnetTransport } from "./transport";
import { WsTransport } from "./wsTransport";

/** The default WS endpoint when the live transport is selected without an explicit URL. */
const DEFAULT_WS_URL = "ws://127.0.0.1:8081";

/** Which transport this build selected, plus a human label for diagnostics. */
export interface TransportSelection {
  readonly transport: CelnetTransport;
  /** "mock" or "ws" — the selected mode (the ribbon shows `transport.label`). */
  readonly mode: "mock" | "ws";
}

/** Read a `VITE_*` string env var, trimmed; empty/unset ⇒ undefined. */
function envString(key: string): string | undefined {
  const v = import.meta.env[key as keyof ImportMetaEnv];
  if (typeof v !== "string") return undefined;
  const trimmed = v.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

/**
 * Resolve the configured transport. Pure of side effects beyond constructing the
 * chosen transport (the WS one dials lazily). Called once at the app root.
 */
export function resolveTransport(): TransportSelection {
  const mode = envString("VITE_CELNET_TRANSPORT");
  const url = envString("VITE_CELNET_WS_URL");
  const live = mode === "ws" || (mode === undefined && url !== undefined);
  if (live) {
    return {
      transport: new WsTransport({ url: url ?? DEFAULT_WS_URL }),
      mode: "ws",
    };
  }
  return { transport: createMockTransport(), mode: "mock" };
}
