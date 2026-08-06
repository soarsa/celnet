/**
 * Acceptance consolidation + capability-gating tests. The incoming-quote-acceptance
 * builder is CONSOLIDATED into the single "Risk" host (`riskdashboard`) as its
 * "Acceptance" tab — it no longer carries a standalone rail row. The `acceptance` id
 * stays a valid deep-link ALIAS resolving to that host, so `?view=acceptance` opens the
 * merged surface straight on its Acceptance tab.
 *
 * Two gates now stack: reaching the host ROW follows the host's `risk_manage·FI` viewCap
 * (so the deep-link's reachability resolves via the alias host), while the Acceptance
 * TAB inside keeps its NARROW `manage_acceptance·FI` gate (asserted in
 * riskDashboardWorkspace.test.tsx). `can` is permissive signed-out.
 */
import { describe, expect, it } from "vitest";

import {
  CONSOLIDATED_WORKSPACE_ALIAS,
  RAIL,
  railForDomain,
  workspaceAccessible,
  workspaceAssets,
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

describe("acceptance consolidation into the Risk host", () => {
  it("has NO standalone rail row — it is the Risk host's Acceptance tab", () => {
    expect(RAIL.some((r) => r.id === "acceptance")).toBe(false);
  });

  it("resolves as a deep-link alias to the `riskdashboard` host", () => {
    expect(CONSOLIDATED_WORKSPACE_ALIAS.acceptance).toBe("riskdashboard");
    // Still a navigable id — assets resolve via the alias (fixed_income) rather than
    // throwing "not in RAIL".
    expect(workspaceAssets("acceptance")).toEqual(["fixed_income"]);
  });

  it("maps to the Fixed Income domain (via the host), not its own tab / admin / analytics", () => {
    expect(workspaceDomains("acceptance")).toEqual(["fixed_income"]);
    // The host row (not a standalone `acceptance` row) is the FI Risk entry.
    expect(railForDomain("fixed_income").some((r) => r.id === "riskdashboard")).toBe(true);
    expect(railForDomain("fixed_income").some((r) => r.id === "acceptance")).toBe(false);
  });

  it("manage_acceptance is held back from the default trader bundle", () => {
    expect(TRADER_HELD_BACK_ACTIONS.has("manage_acceptance")).toBe(true);
  });
});

describe("acceptance deep-link reachability (resolves via the Risk host)", () => {
  it("is HIDDEN from a signed-in FI trader lacking risk management", () => {
    const trader = navAuth(["view·fixed_income", "execute·fixed_income", "book·fixed_income"]);
    expect(workspaceAccessible("acceptance", trader)).toBe(false);
  });

  it("is VISIBLE to a risk_manage·fixed_income holder (the host's viewCap)", () => {
    // The deep-link resolves via the host's `risk_manage·FI` viewCap; the Acceptance
    // TAB then applies its own `manage_acceptance` gate inside the host.
    const riskMgr = navAuth(["view·fixed_income", "risk_manage·fixed_income"]);
    expect(workspaceAccessible("acceptance", riskMgr)).toBe(true);
  });

  it("is VISIBLE to an admin (grant-all)", () => {
    expect(workspaceAccessible("acceptance", admin)).toBe(true);
  });
});
