/**
 * License gating — the three-state rail (DEC-license-gating-and-scope).
 *
 * Entitlement and commercial LICENSE are distinct gates. This proves the pure
 * `railState`/`domainRailState` resolution:
 *   • entitlement-deny ⇒ HIDDEN (info-barrier; wins over any upsell);
 *   • entitled-but-unlicensed ⇒ GATED-UPSELL (present + lock);
 *   • entitled + licensed ⇒ PRESENT;
 * and — critically — that the DEFAULT all-licensed predicate collapses the three-
 * state back to the legacy two-state (present iff accessible), so nothing changes
 * unless a class is explicitly gated. UX-only: the server still enforces every RPC.
 */

import { describe, expect, it } from "vitest";

import {
  ALL_LICENSED,
  assetOfDomain,
  domainRailState,
  LICENSE_UPSELL_TITLE,
  makeLicensePredicate,
  RAIL,
  railState,
  workspaceAccessible,
  workspaceAsset,
  type NavAuth,
} from "../src/lib/commands";

/** A NavAuth whose `can` admits exactly the given `action·asset` keys. */
function navAuth(opts: { isAdmin: boolean; allow?: ReadonlySet<string> }): NavAuth {
  return {
    isAdmin: opts.isAdmin,
    can: (action, asset) => opts.allow?.has(`${action}·${asset}`) ?? false,
  };
}

/** The signed-out identity: `can` permissive (entitled everywhere), not admin. */
const signedOut: NavAuth = { isAdmin: false, can: () => true };

describe("license gating — three-state rail (DEC-license-gating-and-scope)", () => {
  it("DEFAULTS to all-licensed: state == legacy accessibility, never gated", () => {
    for (const r of RAIL) {
      // Omit the predicate ⇒ ALL_LICENSED, the no-behavior-change default.
      const st = railState(r.id, signedOut);
      expect(st).toBe(workspaceAccessible(r.id, signedOut) ? "present" : "hidden");
      expect(st).not.toBe("gated-upsell");
    }
    // And with a real (FI-only) identity — still exactly the two-state.
    const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
    for (const r of RAIL) {
      expect(railState(r.id, fiOnly)).toBe(
        workspaceAccessible(r.id, fiOnly) ? "present" : "hidden",
      );
    }
  });

  it("ALL_LICENSED admits every asset class", () => {
    expect(ALL_LICENSED("fx_options")).toBe(true);
    expect(ALL_LICENSED("fixed_income")).toBe(true);
  });

  it("entitlement-deny HIDES even when also unlicensed (info-barrier wins)", () => {
    const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
    const noneLicensed = makeLicensePredicate(["fx_options", "fixed_income"]);
    // FX is entitlement-denied AND unlicensed ⇒ hidden, not a visible upsell.
    expect(railState("ticket", fiOnly, noneLicensed)).toBe("hidden");
    expect(railState("surface", fiOnly, noneLicensed)).toBe("hidden");
  });

  it("entitled-but-unlicensed ⇒ gated-upsell (present + lock), not hidden", () => {
    const fiUnlicensed = makeLicensePredicate(["fixed_income"]);
    // Signed-out is entitled everywhere; only FI is unlicensed.
    expect(railState("rates", signedOut, fiUnlicensed)).toBe("gated-upsell");
    expect(railState("book", signedOut, fiUnlicensed)).toBe("gated-upsell");
    // FX stays fully present (its class is licensed).
    expect(railState("ticket", signedOut, fiUnlicensed)).toBe("present");
    expect(railState("surface", signedOut, fiUnlicensed)).toBe("present");
  });

  it("Administration workspaces have no license concept (admin gate only)", () => {
    const admin = navAuth({ isAdmin: true });
    const noneLicensed = makeLicensePredicate(["fx_options", "fixed_income"]);
    expect(workspaceAsset("admin")).toBeNull();
    expect(workspaceAsset("permissions")).toBeNull();
    // Even with nothing licensed, an admin pane is present for an admin, hidden else.
    expect(railState("admin", admin, noneLicensed)).toBe("present");
    expect(railState("admin", signedOut, noneLicensed)).toBe("hidden");
  });

  it("assetOfDomain maps each domain to its commercial-license asset class", () => {
    expect(assetOfDomain("fx-options")).toBe("fx_options");
    expect(assetOfDomain("fixed-income")).toBe("fixed_income");
    expect(assetOfDomain("administration")).toBeNull();
  });

  it("workspaceAsset routes through the workspace's domain", () => {
    expect(workspaceAsset("ticket")).toBe("fx_options"); // fx-options
    expect(workspaceAsset("rates")).toBe("fixed_income"); // fixed-income
  });

  it("domainRailState is the tab-level twin of railState", () => {
    const fiUnlicensed = makeLicensePredicate(["fixed_income"]);
    // Default all-licensed ⇒ legacy: present iff accessible.
    expect(domainRailState("fixed-income", signedOut)).toBe("present");
    expect(domainRailState("administration", signedOut)).toBe("hidden");
    // FI unlicensed ⇒ gated; FX present; Administration hidden (non-admin).
    expect(domainRailState("fixed-income", signedOut, fiUnlicensed)).toBe("gated-upsell");
    expect(domainRailState("fx-options", signedOut, fiUnlicensed)).toBe("present");
    expect(domainRailState("administration", signedOut, fiUnlicensed)).toBe("hidden");
    // Entitlement-denied FI stays hidden even when unlicensed.
    const fxOnly = navAuth({ isAdmin: false, allow: new Set(["view·fx_options"]) });
    expect(domainRailState("fixed-income", fxOnly, fiUnlicensed)).toBe("hidden");
  });

  it("exposes the stable upsell title for the affordance", () => {
    expect(LICENSE_UPSELL_TITLE).toBe("license this class");
  });
});
