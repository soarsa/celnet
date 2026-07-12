// Wire-codec parity tests for `excel/src/contract/authCodec.ts` — the add-in's
// `AuthService.Login` / `Logout` encode/decode against the single `celnet.wire`
// contract. They assert the request body shape, the full LoginResult decode
// (token + user + expiry + capabilities), the numeric UserRole mapping (no
// accidental admin), and the 64-bit expiry recovered exactly.

import { describe, expect, it } from "vitest";

import {
  loginRequestToWire,
  loginResultFromWire,
  logoutRequestToWire,
  userDescFromWire,
  userRoleFromWire,
} from "../src/contract/authCodec";
import { isUserOnDesk } from "../src/contract/access";

describe("loginRequestToWire", () => {
  it("encodes the email + password body verbatim", () => {
    expect(loginRequestToWire("trader@celnet.com", "s3cret")).toEqual({
      email: "trader@celnet.com",
      password: "s3cret",
    });
  });
});

describe("userRoleFromWire", () => {
  it("maps tag 1 → ADMIN and every other tag → TRADER", () => {
    expect(userRoleFromWire(1)).toBe("ADMIN");
    expect(userRoleFromWire(0)).toBe("TRADER");
    expect(userRoleFromWire(7)).toBe("TRADER"); // unknown tag never grants admin
  });
});

describe("userDescFromWire", () => {
  it("decodes a full descriptor as deskless when desk_ids is absent", () => {
    const u = userDescFromWire({
      id: "u-1",
      email: "a@celnet.com",
      display_name: "Ada",
      role: 1,
      disabled: false,
    });
    expect(u).toEqual({
      id: "u-1",
      email: "a@celnet.com",
      displayName: "Ada",
      role: "ADMIN",
      deskIds: [],
      allDesks: false,
      disabled: false,
    });
    expect("deskId" in u).toBe(false);
  });

  it("carries a non-empty desk set (Set membership), filtering blanks/non-strings", () => {
    const u = userDescFromWire({
      id: "u-2",
      email: "b@x",
      display_name: "Bo",
      role: 0,
      desk_ids: ["fx-emea", "", "fx-apac", 7],
      all_desks: false,
    });
    expect(u.deskIds).toEqual(["fx-emea", "fx-apac"]);
    expect(u.allDesks).toBe(false);
    expect(u.role).toBe("TRADER");
    expect(isUserOnDesk(u, "fx-apac")).toBe(true);
    expect(isUserOnDesk(u, "fx-us")).toBe(false);
  });

  it("decodes all_desks membership (All) with an empty desk set", () => {
    const u = userDescFromWire({
      id: "u-3",
      email: "c@x",
      display_name: "Cy",
      role: 0,
      desk_ids: [],
      all_desks: true,
    });
    expect(u.deskIds).toEqual([]);
    expect(u.allDesks).toBe(true);
    // An all-desks user is on EVERY desk, including ones it never names.
    expect(isUserOnDesk(u, "fx-emea")).toBe(true);
    expect(isUserOnDesk(u, "anything")).toBe(true);
  });
});

describe("loginResultFromWire", () => {
  it("decodes token, user, expiry and the caller's effective capabilities", () => {
    const result = loginResultFromWire({
      session_token: "tok-abc",
      expires_nanos: 1_900_000_000_000_000_000,
      user: { id: "u-9", email: "c@celnet.com", display_name: "Cy", role: 0, disabled: false },
      capabilities: [
        { action: "view", asset: "fx_options" },
        { action: "price", asset: "fx_options" },
        { action: "execute", asset: "fx_options" },
      ],
    });
    expect(result.token).toBe("tok-abc");
    expect(result.user.displayName).toBe("Cy");
    expect(result.user.role).toBe("TRADER");
    expect(typeof result.expiresNanos).toBe("bigint");
    expect(result.expiresNanos).toBe(1_900_000_000_000_000_000n);
    expect(result.capabilities).toHaveLength(3);
    expect(result.capabilities[2]).toEqual({ action: "execute", asset: "fx_options" });
  });

  it("defaults absent capabilities to an empty (deny-all) set", () => {
    const result = loginResultFromWire({
      session_token: "tok",
      expires_nanos: 0,
      user: { id: "u", email: "d@x", display_name: "D", role: 0, disabled: false },
    });
    expect(result.capabilities).toEqual([]);
  });

  it("throws when the authenticated user is missing", () => {
    expect(() => loginResultFromWire({ session_token: "tok" })).toThrow(/missing the authenticated user/);
  });
});

describe("logoutRequestToWire", () => {
  it("is an empty body (the bearer token identifies the session)", () => {
    expect(logoutRequestToWire()).toEqual({});
  });
});
