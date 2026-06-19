/**
 * fixClientConfig — generate a ready-to-use FIX client config for a managed
 * inbound acceptor, and the small browser helpers that download the FIX spec
 * artifacts.
 *
 * A connection row describes an acceptor (venue) the counterparty connects INTO.
 * A client's session is the mirror image of the venue's: the client's
 * `SenderCompID` is the venue's `TargetCompID` and vice-versa (the acceptor
 * authenticates the peer by this swap — see `celnet-fix-api.md` §2). This module
 * derives a QuickFIX-family initiator `.cfg` from a {@link FixConnection} with the
 * CompIDs already swapped and the host/port filled in, so an operator hands a
 * counterparty a config that connects on the first try.
 *
 * The static spec artifacts (the QuickFIX data dictionary and the API guide) are
 * served from the GUI's public dir under `/fix/`; the URLs are exported here so
 * the workspace can link/download them.
 */

import type { FixConnection } from "../data/contract";

/** The served path of the QuickFIX FIX 4.4 data dictionary (the machine spec). */
export const FIX_DICTIONARY_URL = "/fix/celnet-fix44.xml";
/** The served path of the FIX API guide (the human/agent spec). */
export const FIX_API_GUIDE_URL = "/fix/celnet-fix-api.md";
/** The data-dictionary filename a generated client config references. */
export const FIX_DICTIONARY_FILENAME = "celnet-fix44.xml";

/** The default client heartbeat interval (seconds) the generated config uses. */
const DEFAULT_HEARTBEAT_SECS = 30;

/**
 * Split a `host:port` bind address into its parts. An acceptor often binds a
 * wildcard host (`0.0.0.0` / `::` / empty) which a client cannot connect to, so
 * the connect host falls back to loopback — the operator edits it to the routable
 * address for a remote counterparty (noted in the generated file).
 */
function splitBindAddr(bindAddr: string): { host: string; port: string } {
  const idx = bindAddr.lastIndexOf(":");
  const rawHost = idx >= 0 ? bindAddr.slice(0, idx) : "";
  const port = idx >= 0 ? bindAddr.slice(idx + 1) : "";
  const wildcard = rawHost === "" || rawHost === "0.0.0.0" || rawHost === "::" || rawHost === "[::]";
  return { host: wildcard ? "127.0.0.1" : rawHost, port };
}

/**
 * Build a QuickFIX-family initiator `.cfg` for connecting to `conn` as a client.
 * The CompIDs are swapped (client SenderCompID = the venue's TargetCompID, and
 * vice-versa) and the connect host/port come from the venue's bind address.
 */
export function buildFixClientConfig(conn: FixConnection): string {
  const { host, port } = splitBindAddr(conn.bindAddr);
  // The client is the mirror of the venue: swap the CompIDs.
  const clientSender = conn.targetCompId;
  const clientTarget = conn.senderCompId;
  const wildcardNote =
    host === "127.0.0.1" && !conn.bindAddr.startsWith("127.0.0.1")
      ? "; NOTE: the acceptor binds a wildcard/0.0.0.0 host — set SocketConnectHost\n; to its routable address for a remote counterparty.\n"
      : "";

  return (
    `; QuickFIX initiator config for the celnet FIX acceptor "${conn.name}"` +
    `${conn.desk ? ` (desk ${conn.desk})` : ""}.\n` +
    `; Generated from the Connections workspace. Drop ${FIX_DICTIONARY_FILENAME} alongside\n` +
    `; this file (Download "FIX dictionary" from the same workspace). See celnet-fix-api.md\n` +
    `; for the RFQ -> Quote -> NewOrderSingle -> ExecutionReport message flow.\n` +
    wildcardNote +
    `\n` +
    `[DEFAULT]\n` +
    `ConnectionType=initiator\n` +
    `ReconnectInterval=5\n` +
    `FileStorePath=store\n` +
    `FileLogPath=log\n` +
    `\n` +
    `[SESSION]\n` +
    `BeginString=FIX.4.4\n` +
    `# Your CompID is the venue's TargetCompID; the venue's is its SenderCompID.\n` +
    `SenderCompID=${clientSender}\n` +
    `TargetCompID=${clientTarget}\n` +
    `SocketConnectHost=${host}\n` +
    `SocketConnectPort=${port}\n` +
    `HeartBtInt=${DEFAULT_HEARTBEAT_SECS}\n` +
    `ResetOnLogon=Y\n` +
    `DataDictionary=${FIX_DICTIONARY_FILENAME}\n` +
    `UseDataDictionary=Y\n`
  );
}

/** A filesystem-safe `.cfg` filename for a connection's client config. */
export function fixClientConfigFilename(conn: FixConnection): string {
  const slug = conn.id.replace(/[^a-zA-Z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "connection";
  return `celnet-fix-${slug}.cfg`;
}

/**
 * Trigger a browser download of `text` as `filename`. Creates a temporary object
 * URL + anchor and revokes the URL afterwards. No-op outside a DOM (e.g. SSR).
 */
export function downloadText(filename: string, text: string, mime = "text/plain"): void {
  if (typeof document === "undefined" || typeof URL.createObjectURL !== "function") return;
  const blob = new Blob([text], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
