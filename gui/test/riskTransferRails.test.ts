/**
 * Risk-transfer rail + capability-gating tests. The three former surfaces (initiate
 * ticket · inbox · audit) are CONSOLIDATED into ONE Fixed-Income rail row, "Risk
 * Transfer", a tabbed shell (mirroring the Risk Dashboard + Pricing merges). The retired
 * `transferinbox` / `transferaudit` ids keep no rail row of their own — they are
 * deep-link ALIASES resolving to the `risktransfer` host and opening its folded tab.
 *
 * The write-class tabs (initiate + inbox) still gate on the NARROW `risk_transfer`
 * capability, while the audit tab — and hence the consolidated rail row itself — sits on
 * the `view·FI` floor, so a booking-only FI trader reaches the surface for the audit
 * trail and simply sees the two write tabs hidden. `can` is permissive signed-out.
 */
import { describe, expect, it } from "vitest";

import {
  CONSOLIDATED_WORKSPACE_ALIAS,
  RAIL,
  WORKSPACE_CAPABILITY,
  railForDomain,
  workspaceAccessible,
  workspaceAssets,
  type NavAuth,
  type WorkspaceId,
} from "../src/lib/commands";
import type { CapabilityAction, CapabilityAsset } from "../src/data/contract";

/** Build a signed-in NavAuth from an allow-set of "action·asset" tokens. */
function navAuth(allow: Iterable<string>): NavAuth {
  const set = new Set(allow);
  return {
    isAdmin: false,
    signedIn: true,
    can: (action: CapabilityAction, asset: CapabilityAsset) => set.has(`${action}·${asset}`),
  };
}

/** Signed-out: `can` is permissive (everything true). */
const signedOut: NavAuth = { isAdmin: false, can: () => true };

const RETIRED_ROWS: WorkspaceId[] = ["transferinbox", "transferaudit"];

describe("consolidated Risk Transfer rail row", () => {
  it("keeps ONE fixed-income row (Risk Transfer); the inbox + audit rows are retired", () => {
    const transfers = RAIL.filter((r) => r.section === "transfers");
    expect(transfers.map((r) => r.id)).toEqual(["risktransfer"]);
    const host = transfers[0]!;
    expect(host.label).toBe("Risk Transfer");
    expect(host.assets).toEqual(["fixed_income"]);
    // The two former standalone rows no longer exist in RAIL.
    for (const id of RETIRED_ROWS) {
      expect(RAIL.some((r) => r.id === id)).toBe(false);
    }
    // Glyphs stay unique across the whole rail (no collision after the removal).
    const glyphs = RAIL.map((r) => r.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  it("keeps the consolidated row under Fixed Income once the Risk host is hoisted away", () => {
    const fi = railForDomain("fixed_income").map((r) => r.id);
    // Risk Routing + Acceptance are CONSOLIDATED into the "riskdashboard" Risk host
    // (their own tabs), and that host is itself hoisted to the top-level Risk tab — so
    // none of the three are FI rail rows. Risk Transfer STAYS on the FI rail: moving
    // risk between portfolios is an FI desk workflow, not firm-wide risk management.
    expect(fi.includes("riskrouting")).toBe(false);
    expect(fi.includes("acceptance")).toBe(false);
    expect(fi.includes("riskdashboard")).toBe(false);
    expect(fi.includes("risktransfer")).toBe(true);
    // The retired transfer ids are not standalone rail rows under any domain either.
    for (const id of RETIRED_ROWS) expect(fi.includes(id)).toBe(false);
  });
});

describe("retired ids resolve to the host as consolidated aliases", () => {
  it("maps transferinbox + transferaudit onto the risktransfer host", () => {
    expect(CONSOLIDATED_WORKSPACE_ALIAS.transferinbox).toBe("risktransfer");
    expect(CONSOLIDATED_WORKSPACE_ALIAS.transferaudit).toBe("risktransfer");
  });

  it("resolves the aliases' assets through the host row (still fixed-income)", () => {
    for (const id of RETIRED_ROWS) expect(workspaceAssets(id)).toEqual(["fixed_income"]);
  });
});

describe("WORKSPACE_CAPABILITY gating after the merge", () => {
  it("host defaults to `view`; the inbox deep-link stays `risk_transfer`; audit is `view`", () => {
    // The consolidated host is absent from the map (defaults to `view`) so the
    // audit-floor surface stays reachable by any FI trader.
    expect(WORKSPACE_CAPABILITY.risktransfer).toBeUndefined();
    // The accept-class inbox deep-link keeps its narrow gate.
    expect(WORKSPACE_CAPABILITY.transferinbox).toBe("risk_transfer");
    expect(WORKSPACE_CAPABILITY.transferaudit).toBeUndefined();
  });

  it("shows the consolidated row + audit deep-link to a view-only FI trader; hides the inbox deep-link", () => {
    const trader = navAuth(["view·fixed_income"]); // can view FI, but NOT risk_transfer
    // The host row is reachable (its audit tab is on the view floor).
    expect(workspaceAccessible("risktransfer", trader)).toBe(true);
    expect(workspaceAccessible("transferaudit", trader)).toBe(true);
    // The accept-class inbox deep-link stays gated.
    expect(workspaceAccessible("transferinbox", trader)).toBe(false);
  });

  it("keeps every id reachable for a holder of risk_transfer·fixed_income", () => {
    const granted = navAuth(["view·fixed_income", "risk_transfer·fixed_income"]);
    for (const id of ["risktransfer", ...RETIRED_ROWS] as WorkspaceId[]) {
      expect(workspaceAccessible(id, granted)).toBe(true);
    }
  });

  it("keeps every id reachable signed-out (permissive `can` — rails render pre-login)", () => {
    for (const id of ["risktransfer", ...RETIRED_ROWS] as WorkspaceId[]) {
      expect(workspaceAccessible(id, signedOut)).toBe(true);
    }
  });
});
