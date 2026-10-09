/**
 * The WebSocket seam — one transport, two hosts.
 *
 * The add-in's transport must run unchanged in two places (GUIDE.md: a clean
 * seam so the same transport runs in the browser-host and in a node harness):
 *  - inside the Office.js custom-function runtime / task pane, where the global
 *    browser `WebSocket` exists;
 *  - inside a node verify/unit harness, where it does not, and the `ws` package
 *    provides an API-compatible socket.
 *
 * Rather than depend on either concrete type, the transport takes a
 * `WebSocketFactory`: a function that opens a `WebSocketLike` for a URL. The
 * browser supplies `(url) => new WebSocket(url)`; the node harness supplies a
 * factory backed by the `ws` package. The transport never references a global,
 * so it is environment-agnostic and unit-testable with an in-memory fake.
 */

/** The minimal text-only WebSocket surface the transport relies on. */
export interface WebSocketLike {
  /** Send a UTF-8 text frame (the mirror is text JSON only). */
  send(data: string): void;
  /** Close the socket. */
  close(): void;
  /** Current ready state; OPEN === 1 per the WHATWG/RFC contract. */
  readonly readyState: number;
  onopen: (() => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
  /** Receives a text frame; non-string data is ignored by the transport. */
  onmessage: ((data: string) => void) | null;
}

/** WebSocket.OPEN — the single numeric constant the transport checks. */
export const WS_OPEN = 1;

/** Opens a `WebSocketLike` for an endpoint URL. */
export type WebSocketFactory = (url: string) => WebSocketLike;

/**
 * The browser factory: adapt the global `WebSocket` to `WebSocketLike`. The
 * browser delivers messages via a `MessageEvent`; we unwrap `.data` to the raw
 * string and drop non-string frames (the mirror is text JSON only).
 */
export function browserWebSocketFactory(): WebSocketFactory {
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
    ws.onopen = () => like.onopen?.();
    ws.onclose = () => like.onclose?.();
    ws.onerror = () => like.onerror?.();
    ws.onmessage = (ev: MessageEvent<unknown>) => {
      if (typeof ev.data === "string") like.onmessage?.(ev.data);
    };
    return like;
  };
}
