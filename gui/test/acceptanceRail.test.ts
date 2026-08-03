/**
 * Acceptance rail + capability-gating tests. The incoming-quote-acceptance builder is a
 * Fixed-Income rail row in the "Risk" section, gated at the rail on the NARROW
 * `manage_acceptance` capability × FI (held back from the default trader bundle), so
 * only a granted acceptance-policy author (or an admin) sees it; an ordinary FI trader
 * does not. Mirrors the hedging viewCap gating.
 */
import { describe, expect, it } from "vitest";

import {
  RAIL,
  railForDomain,
  workspaceAccessible,
  workspaceDomains,
  type NavAuth,
} from "../src/lib/commands";
import type { CapabilityAction, CapabilityAsset } from "../src/data/contract";
import { TRADER_HELD_BACK_ACTIONS } from "../src/lib/capabilityMatrix";

/** Build a signed-in NavAuth from an allow-set of "action·asset" tokens. */
function navAuth(allow: Iterable<string>): NavAuth {
  const set = new Set(allow);
  return {
    isAdmin: false,
    signedIn: true,
    can: (action: CapabilityAction, asset: CapabilityAsset) => set.has(`${action}·${asset}`),
  };
}

const admin: NavAuth = { isAdmin: true, signedIn: true, can: () => true };

describe("acceptance rail row", () => {
  it("is a single fixed-income row in the Risk section with a manage_acceptance viewCap", () => {
    const row = RAIL.find((r) => r.id === "acceptance");
    expect(row).toBeDefined();
    expect(row?.assets).toEqual(["fixed_income"]);
    expect(row?.section).toBe("risk");
    expect(row?.viewCap).toEqual({ action: "manage_acceptance", asset: "fixed_income" });
    expect(row?.subtitle && row.subtitle.length > 0).toBe(true);
  });

  it("has a glyph unique across the whole rail", () => {
    const glyphs = RAIL.map((r) => r.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  it("maps to the Fixed Income domain (not its own tab, not admin/analytics)", () => {
    expect(workspaceDomains("acceptance")).toEqual(["fixed_income"]);
    expect(railForDomain("fixed_income").some((r) => r.id === "acceptance")).toBe(true);
  });

  it("manage_acceptance is held back from the default trader bundle", () => {
    expect(TRADER_HELD_BACK_ACTIONS.has("manage_acceptance")).toBe(true);
  });
});

describe("acceptance rail visibility", () => {
  it("is HIDDEN from a signed-in FI trader lacking manage_acceptance", () => {
    const trader = navAuth(["view·fixed_income", "execute·fixed_income", "book·fixed_income"]);
    expect(workspaceAccessible("acceptance", trader)).toBe(false);
  });

  it("is VISIBLE to a holder of manage_acceptance·fixed_income (delegated off admin)", () => {
    const author = navAuth(["view·fixed_income", "manage_acceptance·fixed_income"]);
    expect(workspaceAccessible("acceptance", author)).toBe(true);
  });

  it("is VISIBLE to an admin (grant-all)", () => {
    expect(workspaceAccessible("acceptance", admin)).toBe(true);
  });
});
