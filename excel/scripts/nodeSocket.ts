/**
 * The node `ws`-backed WebSocketLike factory — the SAME transport seam the
 * browser uses (src/transport/socket.ts), so the headless verify harness drives
 * the EXACT code path Excel's custom functions call, only with the node socket.
 */

import WebSocket from "ws";
import { type WebSocketFactory, type WebSocketLike } from "../src/transport/socket";

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
    ws.on("message", (data: WebSocket.RawData) => like.onmessage?.(data.toString()));
    return like;
  };
}
