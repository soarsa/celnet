/**
 * Regression: the WS transport must NOT reconnect-loop when the server rejects a
 * held bearer token.
 *
 * The firm-wide bug: after a server restart / blue-green cutover empties the
 * session registry, the client's held `session_token` is invalid. The edge closes
 * any session whose `authenticate` frame carries a presented-but-invalid token
 * (`crates/celnet-server/src/services/stream.rs` — a bad credential is a hard
 * close), emitting an unsolicited `error` frame first. Before this fix the
 * transport re-sent the same dead token on every reconnect's `authenticate` frame,
 * so the edge closed it every cycle and the socket reconnect-looped at the base
 * backoff (~4/sec) forever — the deployed "connection closed; reconnecting" banner.
 *
 * The fix: on that unsolicited "invalid or expired session token" error the
 * transport drops the dead token (so the NEXT reconnect authenticates anonymously,
 * which the edge keeps open) and fires `onSessionExpired` (so the app drops to
 * sign-in). These tests pin both behaviours with a fake WebSocket.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { WsTransport } from "../src/data/wsTransport";

/** A minimal fake of the browser WebSocket surface `WsConnection` touches. */
class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  static instances: FakeWebSocket[] = [];

  readyState = FakeWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  readonly sent: string[] = [];

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = FakeWebSocket.CLOSED;
  }

  /** Test helper: drive the open handshake. */
  fireOpen(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  /** Test helper: deliver a server → client text frame. */
  fireMessage(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }

  /** Test helper: the edge closed the socket. */
  fireClose(): void {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.();
  }
}

/** The exact server rejection frame (an unsolicited `error`, no correlation id). */
const INVALID_SESSION_FRAME = {
  correlation_id: null,
  message: "invalid or expired session token — re-authenticate via AuthService.Login",
  type: "error",
};

/** Parse an `authenticate` frame's `session_token` (undefined when anonymous). */
function authTokenOf(raw: string): string | undefined {
  const f = JSON.parse(raw) as { type?: string; session_token?: string };
  return f.type === "authenticate" ? f.session_token : undefined;
}

/** The most-recent `authenticate` frame a socket sent, or undefined. */
function lastAuth(ws: FakeWebSocket): { present: boolean; token: string | undefined } {
  for (let i = ws.sent.length - 1; i >= 0; i--) {
    const f = JSON.parse(ws.sent[i]) as { type?: string; session_token?: string };
    if (f.type === "authenticate") return { present: true, token: f.session_token };
  }
  return { present: false, token: undefined };
}

describe("WS transport — session expiry breaks the reconnect loop", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("clears the dead token so the NEXT reconnect authenticates anonymously", () => {
    const transport = new WsTransport({ url: "ws://test/" });
    const first = FakeWebSocket.instances[0];
    expect(first).toBeDefined();

    // Open + authenticate anonymously (no token yet).
    first.fireOpen();
    expect(authTokenOf(first.sent[0])).toBeUndefined();

    // A login installs a bearer token — re-authenticated on the open socket.
    transport.setSessionToken("dead-token");
    expect(lastAuth(first).token).toBe("dead-token");

    const expired = vi.fn();
    transport.onSessionExpired(expired);

    // The edge rejects the token (unsolicited error), then closes the socket.
    first.fireMessage(INVALID_SESSION_FRAME);
    expect(expired).toHaveBeenCalledTimes(1);
    first.fireClose();

    // The transport reconnects at the base backoff…
    vi.advanceTimersByTime(250);
    const second = FakeWebSocket.instances[1];
    expect(second).toBeDefined();
    second.fireOpen();

    // …and the reconnect's `authenticate` no longer carries the dead token, so the
    // edge keeps it open (loop broken).
    const auth = lastAuth(second);
    expect(auth.present).toBe(true);
    expect(auth.token).toBeUndefined();

    // A re-delivered rejection does not re-fire expiry (token already cleared).
    second.fireMessage(INVALID_SESSION_FRAME);
    expect(expired).toHaveBeenCalledTimes(1);
  });

  it("does NOT treat an anonymous session's rejection as an expiry", () => {
    const transport = new WsTransport({ url: "ws://test/" });
    const ws = FakeWebSocket.instances[0];
    ws.fireOpen();

    const expired = vi.fn();
    transport.onSessionExpired(expired);

    // Same message class, but we hold NO token — an ordinary anonymous denial, not
    // a session expiry. Must never drop the user to sign-in.
    ws.fireMessage(INVALID_SESSION_FRAME);
    expect(expired).not.toHaveBeenCalled();
  });
});
