// ONE CONTRACT — the `AuthService.Login` / `AuthService.Logout` JSON codec for the
// add-in, the exact field-for-field mirror of the server's auth frames (the same
// single, current `celnet.wire` contract the GUI decodes in
// `gui/src/data/wsCodec.ts`). Login is the one call made while ANONYMOUS (no token
// to present); the reply carries the bearer `session_token`, the authenticated
// `user`, the absolute `expires_nanos`, and the caller's OWN resolved effective
// `capabilities`. Kept in its own module (not folded into the large `wsCodec.ts`
// mirror) so the auth surface stays cohesive and self-contained.

import type { WireObject } from "./wsCodec";
import type { Capability, CapabilityAction, CapabilityAsset, LoginResult, UserDesc, UserRole } from "./access";

// --- local scalar accessors (defensive against a malformed frame) ------------
// Self-contained so this module does not depend on `wsCodec`'s private helpers.

function str(o: WireObject, key: string): string {
  const v = o[key];
  return typeof v === "string" ? v : "";
}

function enumNum(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

/** A 64-bit wire integer (JSON number, or an oversized literal recovered as bigint). */
function numToBigInt(o: WireObject, key: string): bigint {
  const v = o[key];
  if (typeof v === "number" && Number.isFinite(v)) return BigInt(Math.trunc(v));
  if (typeof v === "bigint") return v;
  if (typeof v === "string" && v.length > 0) {
    try {
      return BigInt(v);
    } catch {
      return 0n;
    }
  }
  return 0n;
}

// --- UserRole (numeric enum on the wire) -------------------------------------

const USER_ROLE_ADMIN = 1;

/**
 * Wire enum tag → domain role. Any non-admin tag is `TRADER` so a defaulted or
 * unknown tag can NEVER be accidentally promoted to admin (mirrors the GUI).
 */
export function userRoleFromWire(tag: number): UserRole {
  return tag === USER_ROLE_ADMIN ? "ADMIN" : "TRADER";
}

/**
 * A user descriptor from its wire form. `desk_ids` is a repeated string (always
 * present, `[]` when none); `all_desks` a bool. Empty `desk_ids` + `all_desks:false`
 * ⇒ deskless (receives no desk-routed traffic).
 */
export function userDescFromWire(o: WireObject): UserDesc {
  const rawDeskIds = o["desk_ids"];
  const deskIds = Array.isArray(rawDeskIds)
    ? rawDeskIds.filter((d): d is string => typeof d === "string" && d.length > 0)
    : [];
  return {
    id: str(o, "id"),
    email: str(o, "email"),
    displayName: str(o, "display_name"),
    role: userRoleFromWire(enumNum(o, "role")),
    deskIds,
    allDesks: o["all_desks"] === true,
    disabled: o["disabled"] === true,
  };
}

// --- capabilities -------------------------------------------------------------

function capabilityFromWire(o: WireObject): Capability {
  return {
    action: str(o, "action") as CapabilityAction,
    asset: str(o, "asset") as CapabilityAsset,
  };
}

function capabilityListFromWire(o: WireObject, key: string): Capability[] {
  const arr = o[key];
  return Array.isArray(arr) ? (arr as WireObject[]).map(capabilityFromWire) : [];
}

// --- login / logout -----------------------------------------------------------

/** Encode the login request body (`AuthService.Login`). */
export function loginRequestToWire(email: string, password: string): WireObject {
  return { email, password };
}

/** Decode a `login_result` frame into the issued session. */
export function loginResultFromWire(o: WireObject): LoginResult {
  const user = o["user"];
  if (!user || typeof user !== "object") {
    throw new Error("login response is missing the authenticated user");
  }
  return {
    token: str(o, "session_token"),
    user: userDescFromWire(user as WireObject),
    expiresNanos: numToBigInt(o, "expires_nanos"),
    capabilities: capabilityListFromWire(o, "capabilities"),
  };
}

/** The logout body is empty — the bearer token (added by the connection) identifies the session. */
export function logoutRequestToWire(): WireObject {
  return {};
}
