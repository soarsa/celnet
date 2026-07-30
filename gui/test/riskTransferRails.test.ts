/**
 * Risk-transfer rail + capability-gating tests. The three surfaces are Fixed-Income
 * rail rows; the ticket + inbox gate reachability on the NARROW `risk_transfer`
 * capability (not the default trader bundle), while the audit trail stays `view` for
 * any FI trader. `can` is permissive signed-out, so the rails still render pre-login.
 */
import { describe, expect, it } from "vitest";

import {
  RAIL,
  WORKSPACE_CAPABILITY,
  railForDomain,
  workspaceAccessible,
  type NavAuth,
  type WorkspaceId,
} from "../src/lib/commands";
import type { CapabilityAction, CapabilityAsset } from "../src/data/contract";

/** Build a signed-in NavAuth from an allow-set of "action·asset" tokens. */
function navAuth(allow: Iterable<string>): NavAuth {
  const set = new Set(allow);
  return {
    isAdmin: false,
    can: (action: CapabilityAction, asset: CapabilityAsset) => set.has(`${action}·${asset}`),
  };
}

/** Signed-out: `can` is permissive (everything true). */
const signedOut: NavAuth = { isAdmin: false, can: () => true };

const TRANSFER_ROWS: WorkspaceId[] = ["risktransfer", "transferinbox", "transferaudit"];

describe("risk-transfer rail rows", () => {
  it("adds the three FI rows in RAIL, each fixed-income, with distinct glyphs", () => {
    const rows = RAIL.filter((r) => TRANSFER_ROWS.includes(r.id));
    expect(rows.map((r) => r.id)).toEqual(TRANSFER_ROWS); // RAIL order, after riskrouting
    for (const r of rows) {
      expect(r.assets).toEqual(["fixed_income"]);
      expect(r.subtitle && r.subtitle.length > 0).toBe(true);
    }
    // Glyphs are unique across the whole rail (no collision with existing rows).
    const glyphs = RAIL.map((r) => r.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  it("places all three under the Fixed Income domain immediately after riskrouting", () => {
    const fi = railForDomain("fixed_income").map((r) => r.id);
    const i = fi.indexOf("riskrouting");
    expect(fi.slice(i + 1, i + 4)).toEqual(TRANSFER_ROWS);
  });
});

describe("WORKSPACE_CAPABILITY gating", () => {
  it("gates initiate/accept surfaces on `risk_transfer`; audit defaults to `view`", () => {
    expect(WORKSPACE_CAPABILITY.risktransfer).toBe("risk_transfer");
    expect(WORKSPACE_CAPABILITY.transferinbox).toBe("risk_transfer");
    expect(WORKSPACE_CAPABILITY.transferaudit).toBeUndefined();
  });

  it("hides ticket + inbox from an FI trader lacking risk_transfer, but shows the audit trail", () => {
    const trader = navAuth(["view·fixed_income"]); // can view FI, but NOT risk_transfer
    expect(workspaceAccessible("risktransfer", trader)).toBe(false);
    expect(workspaceAccessible("transferinbox", trader)).toBe(false);
    expect(workspaceAccessible("transferaudit", trader)).toBe(true);
  });

  it("shows all three to a user holding risk_transfer·fixed_income", () => {
    const granted = navAuth(["view·fixed_income", "risk_transfer·fixed_income"]);
    for (const id of TRANSFER_ROWS) expect(workspaceAccessible(id, granted)).toBe(true);
  });

  it("shows all three signed-out (permissive `can` — rails render pre-login)", () => {
    for (const id of TRANSFER_ROWS) expect(workspaceAccessible(id, signedOut)).toBe(true);
  });
});
