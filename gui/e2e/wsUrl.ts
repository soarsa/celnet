/**
 * Shared location of the booted demo edge's WebSocket-mirror URL. `globalSetup`
 * writes it here after the edge is ready; specs read it to pin the live transport
 * via the `?ws=<url>` flag. A plain temp file keeps the URL out of process env
 * (which Playwright does not propagate from globalSetup into worker processes).
 */
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const WS_URL_FILE = join(tmpdir(), "celnet-e2e-ws-url");

/** Read the edge WS URL written by globalSetup (throws if the edge never booted). */
export function readWsUrl(): string {
  const url = readFileSync(WS_URL_FILE, "utf8").trim();
  if (!url.startsWith("ws://")) {
    throw new Error(`e2e: no demo-edge WS URL recorded (got ${JSON.stringify(url)})`);
  }
  return url;
}
