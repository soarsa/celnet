/**
 * The node `ws`-backed `WebSocketFactory` for the conformance e2e.
 *
 * The add-in transport (`src/transport`) is environment-agnostic: it takes a
 * `WebSocketFactory` returning a `WebSocketLike`. In Excel that factory wraps the
 * browser global `WebSocket` (`browserWebSocketFactory`); in this node harness it
 * wraps the OSS `ws` package, adapted to the SAME `WebSocketLike` surface. This is
 * the ONLY shim — it supplies the missing browser socket, never our own code. The
 * REAL `Connection` runs over a REAL socket to a REAL edge.
 */
import WebSocket from "ws";

import type { WebSocketFactory, WebSocketLike } from "../src/transport/socket";

/** A `WebSocketFactory` backed by the node `ws` package (text frames only). */
export function nodeWebSocketFactory(): WebSocketFactory {
  return (url: string): WebSocketLike => {
    const ws = new WebSocket(url);
    const like: WebSocketLike = {
      send: (data: string) => ws.send(data),
      close: () => ws.close(),
      get readyState() {
        return ws.readyState;
      },
      onopen: null,
      onclose: null,
      onerror: null,
      onmessage: null,
    };
    ws.on("open", () => like.onopen?.());
    ws.on("close", () => like.onclose?.());
    ws.on("error", () => like.onerror?.());
    ws.on("message", (data: WebSocket.RawData, isBinary: boolean) => {
      // The mirror is text JSON only; decode the frame and drop any binary.
      if (!isBinary) like.onmessage?.(data.toString());
    });
    return like;
  };
}
