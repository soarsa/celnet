/**
 * The custom-function runtime singletons: the one WS connection (the single
 * multiplexed session, docs §3.2), the ref-counted stream registry, and the
 * signed-in user session — all shared by every CELNET.* cell AND the task pane in
 * the workbook (the add-in runs them in one Office shared runtime, so a sign-in in
 * the pane authenticates the cells too).
 *
 * The endpoint is read from a workbook setting (set in the task pane) or a
 * `<meta>`/global injected at sideload time, defaulting to the local dev edge.
 * The connection uses the browser WebSocket factory; the same Connection/registry
 * run in the node harness with the `ws` factory (the clean transport seam).
 */

import { Connection } from "../transport/connection";
import { browserWebSocketFactory } from "../transport/socket";
import { UserSession } from "../transport/session";
import { RatesStreamRegistry } from "./ratesStreamRegistry";
import { SeriesRegistry } from "./seriesRegistry";
import { StreamRegistry } from "./streamRegistry";

/** The default local dev endpoint (the celnet-server WS mirror prints its ws://). */
const DEFAULT_ENDPOINT = "ws://127.0.0.1:8081";

/** Resolve the configured endpoint: a global set by the task pane, else default. */
function resolveEndpoint(): string {
  const g = globalThis as unknown as { CELNET_WS_ENDPOINT?: string };
  return typeof g.CELNET_WS_ENDPOINT === "string" && g.CELNET_WS_ENDPOINT.length > 0
    ? g.CELNET_WS_ENDPOINT
    : DEFAULT_ENDPOINT;
}

let connection: Connection | null = null;
let registry: StreamRegistry | null = null;
let seriesRegistry: SeriesRegistry | null = null;
let ratesStreamRegistry: RatesStreamRegistry | null = null;
let session: UserSession | null = null;

/** The shared connection (lazily opened on first use). */
export function getConnection(): Connection {
  if (!connection) {
    connection = new Connection({
      url: resolveEndpoint(),
      factory: browserWebSocketFactory(),
    });
  }
  return connection;
}

/** The shared stream registry over the shared connection. */
export function getRegistry(): StreamRegistry {
  if (!registry) {
    registry = new StreamRegistry(getConnection());
  }
  return registry;
}

/** The shared market-series (trend) registry over the shared connection. */
export function getSeriesRegistry(): SeriesRegistry {
  if (!seriesRegistry) {
    seriesRegistry = new SeriesRegistry(getConnection());
  }
  return seriesRegistry;
}

/** The shared fixed-income (linear-rates) streaming registry over the shared connection. */
export function getRatesStreamRegistry(): RatesStreamRegistry {
  if (!ratesStreamRegistry) {
    ratesStreamRegistry = new RatesStreamRegistry(getConnection());
  }
  return ratesStreamRegistry;
}

/**
 * The shared signed-in user session over the shared connection. The task pane
 * drives sign-in/out on it; the CELNET.* cell functions read it to gate their
 * affordances for the signed-in caller (anonymous stays permissive).
 */
export function getSession(): UserSession {
  if (!session) {
    session = new UserSession(getConnection());
  }
  return session;
}
