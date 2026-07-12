// Runtime session-gating tests for `excel/src/transport/session.ts` — the shared
// signed-in identity that the task pane drives and the CELNET.* cells read.
//
// They assert the posture the add-in commits to: anonymous is permissive for
// price-preview but dealing affordances require sign-in; a signed-in user is gated
// to their effective capability set (disable + explain, never silent); the bearer
// token is installed on / cleared from the shared transport; an expired session
// denies everything and prompts re-login.

import { describe, expect, it } from "vitest";

import { UserSession } from "../src/transport/session";
import type { Connection } from "../src/transport/connection";
import type { LoginResult } from "../src/contract/access";

/** A minimal transport double recording the bearer token the session installs. */
class TokenSpy {
  tokens: (string | null)[] = [];
  setSessionToken(token: string | null): void {
    this.tokens.push(token);
  }
  get current(): string | null {
    return this.tokens.length > 0 ? this.tokens[this.tokens.length - 1]! : null;
  }
}

function asConn(spy: TokenSpy): Connection {
  return spy as unknown as Connection;
}

/** A login result whose absolute expiry is `expiresNanos` (default far future). */
function loginResult(
  caps: LoginResult["capabilities"],
  opts?: { expiresNanos?: bigint; role?: "TRADER" | "ADMIN"; token?: string },
): LoginResult {
  return {
    token: opts?.token ?? "tok-1",
    user: {
      id: "u-1",
      email: "t@celnet.com",
      displayName: "Trader",
      role: opts?.role ?? "TRADER",
      deskIds: [],
      allDesks: false,
      disabled: false,
    },
    expiresNanos: opts?.expiresNanos ?? 10_000_000_000_000_000_000n,
    capabilities: caps,
  };
}

describe("UserSession — anonymous posture", () => {
  it("is permissive for capability checks but blocks dealing affordances", () => {
    const session = new UserSession(asConn(new TokenSpy()));
    expect(session.isSignedIn()).toBe(false);
    // can() is permissive (server enforces the anonymous edge) — GUI parity.
    expect(session.can("execute", "fx_options")).toBe(true);
    // Cell affordances inherit the permissive posture.
    expect(session.canEntry("price")).toBe(true);
    expect(session.canEntry("subscribe")).toBe(true);
    // Dealing affordances require a signed-in identity.
    expect(session.canEntry("rfq")).toBe(false);
    expect(session.canEntry("book")).toBe(false);
    expect(session.canEntry("contribute")).toBe(false);
    expect(session.entryDenialReason("book")).toBe("Sign in to book a trade.");
  });
});

describe("UserSession — sign-in gating", () => {
  it("installs the token and gates to the effective set (held stays enabled, missing disabled)", () => {
    const spy = new TokenSpy();
    const session = new UserSession(asConn(spy));
    session.signIn(
      loginResult([
        { action: "view", asset: "fx_options" },
        { action: "price", asset: "fx_options" },
        // NB: no execute on fx_options — the lacking dealing capability.
      ]),
    );
    expect(session.isSignedIn()).toBe(true);
    expect(spy.current).toBe("tok-1");

    // Held capability → affordance enabled, no denial.
    expect(session.canEntry("rfq")).toBe(true); // price · fx_options held
    expect(session.entryDenialReason("rfq")).toBe("");

    // Missing capability → disabled + the honest denial sentence.
    expect(session.canEntry("book")).toBe(false); // execute · fx_options NOT held
    expect(session.entryDenialReason("book")).toBe(
      "Your permissions don't allow executing FX-options trades.",
    );

    // Cell entry on the missing asset → denied for the signed-in caller.
    expect(session.canEntry("rates")).toBe(false); // price · fixed_income NOT held
    expect(session.entryDenialReason("rates")).toBe(
      "Your permissions don't allow requesting fixed-income prices.",
    );
  });

  it("a full FX trader can deal but a missing FI capability stays denied", () => {
    const session = new UserSession(asConn(new TokenSpy()));
    session.signIn(
      loginResult([
        { action: "price", asset: "fx_options" },
        { action: "execute", asset: "fx_options" },
        { action: "stream", asset: "fx_options" },
      ]),
    );
    expect(session.canEntry("rfq")).toBe(true);
    expect(session.canEntry("book")).toBe(true);
    expect(session.canEntry("subscribe")).toBe(true);
    expect(session.canEntry("rates")).toBe(false);
  });
});

describe("UserSession — sign-out", () => {
  it("clears the identity and the transport token", () => {
    const spy = new TokenSpy();
    const session = new UserSession(asConn(spy));
    session.signIn(loginResult([{ action: "execute", asset: "fx_options" }]));
    expect(spy.current).toBe("tok-1");
    session.signOut();
    expect(session.isSignedIn()).toBe(false);
    expect(spy.current).toBe(null);
    expect(session.effectiveCapabilities()).toEqual([]);
    // Back to the anonymous posture.
    expect(session.canEntry("book")).toBe(false);
    expect(session.can("execute", "fx_options")).toBe(true);
  });
});

describe("UserSession — expiry", () => {
  it("denies everything past the absolute expiry and prompts re-login", () => {
    let now = 0n;
    const session = new UserSession(asConn(new TokenSpy()), { clock: () => now });
    session.signIn(
      loginResult([{ action: "execute", asset: "fx_options" }], { expiresNanos: 1_000n }),
    );
    now = 500n; // before expiry
    expect(session.isExpired()).toBe(false);
    expect(session.canEntry("book")).toBe(true);

    now = 1_000n; // at/after expiry
    expect(session.isExpired()).toBe(true);
    expect(session.can("execute", "fx_options")).toBe(false);
    expect(session.canEntry("book")).toBe(false);
    expect(session.entryDenialReason("book")).toBe(
      "Your session has expired — sign in again to continue.",
    );
  });
});

describe("UserSession — subscribers", () => {
  it("notifies on sign-in and sign-out", () => {
    const session = new UserSession(asConn(new TokenSpy()));
    const snaps: (string | null)[] = [];
    const unsub = session.subscribe((s) => snaps.push(s.user?.email ?? null));
    session.signIn(loginResult([]));
    session.signOut();
    unsub();
    session.signIn(loginResult([])); // ignored after unsubscribe
    expect(snaps).toEqual(["t@celnet.com", null]);
  });
});
