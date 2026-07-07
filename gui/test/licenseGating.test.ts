/**
 * License gating — the per-workspace three-state rail (DEC-license-gating-and-scope,
 * re-homed by fe-fi-migration #6 onto the single class-parametric rail).
 *
 * Entitlement and commercial LICENSE are distinct gates. This proves the pure
 * `railState` resolution over `workspaceAssets`:
 *   • not reachable (entitlement-deny) ⇒ HIDDEN (info-barrier; wins over any upsell);
 *   • reachable but NONE of the entitled served classes is licensed ⇒ GATED-UPSELL;
 *   • reachable + at least one entitled served class licensed ⇒ PRESENT — a
 *     cross-asset row stays present while EITHER served class is licensed, its
 *     unlicensed lens gated per-lens INSIDE the class-parametric pane;
 * and — critically — that the DEFAULT all-licensed predicate collapses the three-
 * state back to the two-state (present iff reachable), so nothing changes unless a
 * class is explicitly gated. UX-only: the server still enforces every RPC.
 */

import { describe, expect, it } from "vitest";

import {
  ALL_LICENSED,
  domainAccessible,
  DOMAINS,
  LICENSE_UPSELL_TITLE,
  makeLicensePredicate,
  RAIL,
  railState,
  workspaceAccessible,
  workspaceAssets,
  type Domain,
  type LicensePredicate,
  type NavAuth,
  type RailState,
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

describe("license gating — per-workspace three-state rail (fe-fi-migration #6)", () => {
  it("DEFAULTS to all-licensed: state == reachability, never gated", () => {
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
    // Stream is FX-only; a FI-only identity cannot view FX ⇒ hidden, not a visible upsell.
    expect(railState("stream", fiOnly, noneLicensed)).toBe("hidden");
  });

  it("a SINGLE-asset workspace whose one class is unlicensed ⇒ gated-upsell", () => {
    const fiUnlicensed = makeLicensePredicate(["fixed_income"]);
    const fxUnlicensed = makeLicensePredicate(["fx_options"]);
    // Quoting (FI-only) unlicensed ⇒ gated-upsell; Stream (FX-only) stays present.
    expect(railState("quoting", signedOut, fiUnlicensed)).toBe("gated-upsell");
    expect(railState("stream", signedOut, fiUnlicensed)).toBe("present");
    // Symmetrically: Stream (FX-only) unlicensed ⇒ gated-upsell; Quoting present.
    expect(railState("stream", signedOut, fxUnlicensed)).toBe("gated-upsell");
    expect(railState("quoting", signedOut, fxUnlicensed)).toBe("present");
  });

  it("a CROSS-asset workspace stays PRESENT while EITHER served class is licensed", () => {
    const fiUnlicensed = makeLicensePredicate(["fixed_income"]);
    const fxUnlicensed = makeLicensePredicate(["fx_options"]);
    // Ticket / Market Data / Risk / Book serve both classes: one class unlicensed
    // still leaves the row present (the unlicensed lens is gated per-lens inside).
    for (const id of ["ticket", "surface", "risk", "book"] as const) {
      expect(railState(id, signedOut, fiUnlicensed)).toBe("present");
      expect(railState(id, signedOut, fxUnlicensed)).toBe("present");
    }
  });

  it("a cross-asset workspace is gated-upsell only when ALL served classes are unlicensed", () => {
    const noneLicensed = makeLicensePredicate(["fx_options", "fixed_income"]);
    for (const id of ["ticket", "surface", "risk", "book"] as const) {
      expect(railState(id, signedOut, noneLicensed)).toBe("gated-upsell");
    }
  });

  it("a cross-asset row's state reflects only the ENTITLED classes", () => {
    const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
    const fiUnlicensed = makeLicensePredicate(["fixed_income"]);
    // A FI-only trader reaches the cross-asset Book via FI; with FI unlicensed the
    // ONLY class they can use is unlicensed ⇒ gated-upsell (FX's license is moot —
    // they cannot view FX, so it never rescues the row for them).
    expect(railState("book", fiOnly, fiUnlicensed)).toBe("gated-upsell");
    // With FI licensed it is present.
    expect(railState("book", fiOnly, ALL_LICENSED)).toBe("present");
  });

  it("Administration workspaces have no license concept (admin gate only)", () => {
    const admin = navAuth({ isAdmin: true });
    const noneLicensed = makeLicensePredicate(["fx_options", "fixed_income"]);
    expect(workspaceAssets("admin")).toEqual([]);
    expect(workspaceAssets("permissions")).toEqual([]);
    // Even with nothing licensed, an admin pane is present for an admin, hidden else.
    expect(railState("admin", admin, noneLicensed)).toBe("present");
    expect(railState("admin", signedOut, noneLicensed)).toBe("hidden");
  });

  it("workspaceAssets maps each workspace to the class(es) it serves", () => {
    expect(new Set(workspaceAssets("ticket"))).toEqual(
      new Set(["fx_options", "fixed_income"]),
    );
    expect(workspaceAssets("quoting")).toEqual(["fixed_income"]);
    expect(workspaceAssets("stream")).toEqual(["fx_options"]);
  });

  it("exposes the stable upsell title for the affordance", () => {
    expect(LICENSE_UPSELL_TITLE).toBe("license this class");
  });
});

describe("license gating — the top-level DOMAIN tab three-state (fe-fi-migration re-add)", () => {
  // The Shell derives a domain tab's three-state as: HIDDEN when the domain is
  // inaccessible (entitlement / non-admin); GATED-UPSELL when a trading domain's
  // class is unlicensed; else PRESENT. Admin has no license concept. This mirrors
  // that derivation over the pure lib predicates so the composition is guarded.
  const domainState = (d: Domain, auth: NavAuth, licensed: LicensePredicate): RailState => {
    if (!domainAccessible(d, auth)) return "hidden";
    if (d !== "admin" && !licensed(d)) return "gated-upsell";
    return "present";
  };
  const signedOut: NavAuth = { isAdmin: false, can: () => true };

  it("defaults (all-licensed, signed out): both trading tabs present, Administration hidden", () => {
    for (const { id } of DOMAINS) {
      const st = domainState(id, signedOut, ALL_LICENSED);
      expect(st).toBe(id === "admin" ? "hidden" : "present");
    }
  });

  it("an unlicensed trading class ⇒ its tab is GATED-UPSELL (still discoverable), the other stays present", () => {
    const fxUnlicensed = makeLicensePredicate(["fx_options"]);
    expect(domainState("fx_options", signedOut, fxUnlicensed)).toBe("gated-upsell");
    expect(domainState("fixed_income", signedOut, fxUnlicensed)).toBe("present");
  });

  it("entitlement-deny HIDES a trading tab (wins over any upsell)", () => {
    const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
    // No `view·fx_options` ⇒ the FX tab is hidden regardless of license.
    expect(domainState("fx_options", fiOnly, ALL_LICENSED)).toBe("hidden");
    expect(domainState("fx_options", fiOnly, makeLicensePredicate(["fx_options"]))).toBe("hidden");
    expect(domainState("fixed_income", fiOnly, ALL_LICENSED)).toBe("present");
  });

  it("the Administration tab has NO license concept — present iff admin, never gated", () => {
    const admin = navAuth({ isAdmin: true });
    const nonAdmin = navAuth({ isAdmin: false });
    const noneLicensed = makeLicensePredicate(["fx_options", "fixed_income"]);
    expect(domainState("admin", admin, noneLicensed)).toBe("present");
    expect(domainState("admin", nonAdmin, noneLicensed)).toBe("hidden");
    expect(domainState("admin", admin, ALL_LICENSED)).not.toBe("gated-upsell");
  });
});
