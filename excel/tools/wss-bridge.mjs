// Dev-only TLS WebSocket bridge: terminates `wss://localhost:8443` with the
// trusted `office-addin-dev-certs` localhost certificate and forwards frames
// verbatim to the plain-`ws` celnet-server edge (`ws://127.0.0.1:8081`).
//
// Why: Office serves a sideloaded add-in over HTTPS, and a webview blocks an
// insecure `ws://` opened from an `https://` origin (mixed content). The demo
// edge speaks plain `ws` (TLS is the deploy gateway's job), so for local in-Excel
// verification this bridge supplies the trusted `wss://` the add-in dials. NOT a
// product component — a developer bring-up convenience.
//
// Run:  node excel/tools/wss-bridge.mjs   (after `cargo run … --example demo_edge`)
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { createServer } from "node:https";
import { WebSocketServer, WebSocket } from "ws";

const EDGE = process.env.CELNET_EDGE_WS ?? "ws://127.0.0.1:8081";
const PORT = Number(process.env.CELNET_WSS_PORT ?? 8443);
const certDir = resolve(homedir(), ".office-addin-dev-certs");

const server = createServer({
  cert: readFileSync(resolve(certDir, "localhost.crt")),
  key: readFileSync(resolve(certDir, "localhost.key")),
});

const wss = new WebSocketServer({ server });

let nconn = 0;
wss.on("connection", (client) => {
  const id = ++nconn;
  console.log(`[${id}] client connected (add-in) → dialling edge…`);
  const up = new WebSocket(EDGE);
  const pending = [];
  let open = false;
  up.on("open", () => {
    open = true;
    console.log(`[${id}] edge UP — relaying`);
    for (const [data, binary] of pending) up.send(data, { binary });
    pending.length = 0;
  });
  client.on("message", (data, isBinary) => {
    if (open) up.send(data, { binary: isBinary });
    else pending.push([data, isBinary]);
  });
  up.on("message", (data, isBinary) => {
    if (client.readyState === WebSocket.OPEN) client.send(data, { binary: isBinary });
  });
  const closeBoth = () => {
    try { client.close(); } catch {}
    try { up.close(); } catch {}
  };
  client.on("close", closeBoth);
  up.on("close", closeBoth);
  client.on("error", closeBoth);
  up.on("error", closeBoth);
});

server.listen(PORT, () => {
  console.log(`wss bridge listening on wss://127.0.0.1:${PORT} → ${EDGE}`);
});
