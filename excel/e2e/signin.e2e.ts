/**
 * Excel REAL-EDGE sign-in + capability-gating e2e — proves end-to-end, against a
 * REAL booted `celnet-server` demo edge over a REAL WebSocket, that the add-in's
 * interactive `AuthService.Login` round-trips, captures the caller's effective
 * capabilities, and that the production `UserSession` gates affordances on them.
 *
 * NO FakeSocket, NO mock: the same `Connection` + `authCodec` the task pane uses.
 *
 * The demo edge seeds exactly ONE user — the all-capability administrator
 * `admin@celnet.com` / `password` (crates/celnet-server/src/config/identity.rs) —
 * and the add-in transport has no admin-provisioning path, so the LIVE half asserts
 * the held-capability case (an admin can deal) and the failure case (bad creds are
 * rejected). The MISSING-capability case (a user lacking FX execute → the book
 * affordance disabled with an explanation) is asserted by driving the SAME
 * production `UserSession` gating with a restricted effective set — the exact
 * scenario, with the restricted set crafted because the edge seeds no such user.
 * The deny-by-capability logic itself is the production code path, not a stub.
 */
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Connection, TransportError } from "../src/transport/connection";
import { UserSession } from "../src/transport/session";
import type { LoginResult } from "../src/contract/access";
import { can } from "../src/contract/access";
import { nodeWebSocketFactory } from "./nodeSocket";
import { readWsUrl } from "./wsUrl";

const SEED_ADMIN_EMAIL = "admin@celnet.com";
const SEED_ADMIN_PASSWORD = "password";
const LOGIN_TIMEOUT_MS = 30_000;

let conn: Connection;

beforeAll(async () => {
  const url = readWsUrl();
  conn = new Connection({
    url,
    factory: nodeWebSocketFactory(),
    requestTimeoutMs: LOGIN_TIMEOUT_MS,
    stalenessWindowMs: 10 * 60_000,
  });
  const deadline = Date.now() + 30_000;
  while (!conn.isOpen()) {
    if (Date.now() > deadline) throw new Error(`e2e: socket to ${url} never opened`);
    await new Promise((r) => setTimeout(r, 25));
  }
});

afterAll(() => {
  conn?.close();
});

describe("AuthService.Login over the real WS mirror", () => {
  it("signs in the seeded admin and captures a token + effective capabilities", async () => {
    const result: LoginResult = await conn.login(SEED_ADMIN_EMAIL, SEED_ADMIN_PASSWORD);
    expect(result.token.length).toBeGreaterThan(0);
    expect(result.user.email).toBe(SEED_ADMIN_EMAIL);
    expect(result.user.role).toBe("ADMIN");
    expect(result.capabilities.length).toBeGreaterThan(0);
    // The admin bundle is grant-all → it holds the dealing capabilities.
    expect(can(result.capabilities, "execute", "fx_options")).toBe(true);
    expect(can(result.capabilities, "price", "fx_options")).toBe(true);

    // The production session, driven by the LIVE login result, enables dealing.
    const session = new UserSession(conn);
    session.signIn(result);
    expect(session.isSignedIn()).toBe(true);
    expect(session.canEntry("book")).toBe(true); // execute · fx_options held
    expect(session.entryDenialReason("book")).toBe("");

    // Sign out invalidates server-side (best-effort) and clears locally.
    await conn.logout().catch(() => false);
    session.signOut();
    expect(session.isSignedIn()).toBe(false);
  });

  it("rejects a bad password (the failure path is surfaced, never silent)", async () => {
    await expect(conn.login(SEED_ADMIN_EMAIL, "wrong-password")).rejects.toBeInstanceOf(TransportError);
  });

  it("gates a signed-in user LACKING FX execute: book disabled + explained, price stays enabled", () => {
    // The edge seeds only an all-capability admin, so the restricted effective set is
    // crafted here; the GATING is the production `UserSession` path (not a stub).
    const restricted: LoginResult = {
      token: "live-session-token",
      user: { id: "u-r", email: "restricted@celnet.com", displayName: "R", role: "TRADER", disabled: false },
      expiresNanos: 10_000_000_000_000_000_000n,
      capabilities: [
        { action: "view", asset: "fx_options" },
        { action: "price", asset: "fx_options" },
        // deliberately NO execute · fx_options
      ],
    };
    const session = new UserSession(conn);
    session.signIn(restricted);
    // A held capability stays enabled.
    expect(session.canEntry("rfq")).toBe(true);
    // The lacking capability disables with the honest explanation (never a no-op).
    expect(session.canEntry("book")).toBe(false);
    expect(session.entryDenialReason("book")).toBe(
      "Your permissions don't allow executing FX-options trades.",
    );
    session.signOut();
  });
});
