/**
 * Transport selection: which `CelnetTransport` the app runs against. The default
 * is ALWAYS the **live** WebSocket transport against the running `celnet-server`
 * WS JSON mirror (ws://127.0.0.1:8081) — the GUI consumes real, server-computed
 * prices, Greeks, surfaces and scenarios out of the box, adding no pricing of its
 * own (every number is the server's). One contract, two transports
 * (src/data/transport.ts); the wire codec (src/data/wsCodec.ts) is the SAME
 * type-tagged snake_case JSON the Excel add-in speaks, mirroring the server's
 * `crates/celnet-server/src/ws/codec.rs` field-for-field.
 *
 * The deterministic in-app mock/replay source is NOT the default. It is an
 * explicit, clearly-labelled OFFLINE opt-in for a no-server demo / design review,
 * selected only by:
 *   - the URL flag `?mock` (e.g. http://localhost:5173/?mock), or
 *   - the build-time env `VITE_CELNET_TRANSPORT=mock`.
 *
 * The live endpoint is configurable (without changing the live default):
 *   - URL param `?ws=ws://host:port`, or
 *   - build-time env `VITE_CELNET_WS_URL=ws://host:port`.
 * No runtime mixing: a session is one transport, matching the platform's
 * single-uniform-version deploy model (CLAUDE.md rule 9).
 */

import { createMockTransport } from "./mockSource";
import type { CelnetTransport } from "./transport";
import { WsTransport } from "./wsTransport";

/** The default WS endpoint the live transport dials when none is configured. */
const DEFAULT_WS_URL = "ws://127.0.0.1:8081";

/** Which transport this session selected, plus a human label for diagnostics. */
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

/** The current URL's query string, or empty when not in a browser context. */
function searchParams(): URLSearchParams {
  if (typeof window === "undefined" || typeof window.location === "undefined") {
    return new URLSearchParams();
  }
  return new URLSearchParams(window.location.search);
}

/**
 * Resolve the configured transport. The DEFAULT is the live WS mirror; mock is an
 * explicit offline opt-in. Pure of side effects beyond constructing the chosen
 * transport (the WS one dials lazily on the first session). Called once at the app
 * root.
 */
export function resolveTransport(): TransportSelection {
  const params = searchParams();
  const envMode = envString("VITE_CELNET_TRANSPORT");

  // Explicit offline opt-in: `?mock` in the URL, or `VITE_CELNET_TRANSPORT=mock`.
  const mockRequested =
    params.has("mock") || params.get("transport") === "mock" || envMode === "mock";
  if (mockRequested) {
    return { transport: createMockTransport(), mode: "mock" };
  }

  // Live by default. The endpoint is configurable but the mode stays live.
  const url = params.get("ws") ?? envString("VITE_CELNET_WS_URL") ?? DEFAULT_WS_URL;
  return { transport: new WsTransport({ url }), mode: "ws" };
}
