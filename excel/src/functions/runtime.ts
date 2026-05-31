/**
 * The custom-function runtime singletons: the one WS connection (the single
 * multiplexed session, docs §3.2) and the ref-counted stream registry, shared by
 * every CELNET.* cell in the workbook.
 *
 * The endpoint is read from a workbook setting (set in the task pane) or a
 * `<meta>`/global injected at sideload time, defaulting to the local dev edge.
 * The connection uses the browser WebSocket factory; the same Connection/registry
 * run in the node harness with the `ws` factory (the clean transport seam).
 */

import { Connection } from "../transport/connection";
import { browserWebSocketFactory } from "../transport/socket";
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
