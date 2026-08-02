/**
 * `simulate` capability vocabulary + gating.
 *
 * The counterparty-simulator GUI surface was removed (the top-bar ticket + popout
 * are gone), but `Action::Simulate` remains a first-class SERVER capability on the
 * frozen contract. These tests pin that the client's capability vocabulary still
 * mirrors the server's `Action::ALL` ordering and that `can(...)` / the denial
 * tooltip resolve `simulate·<asset>` correctly — so the raw capability matrix
 * (which enumerates every action) keeps rendering the row even with no dedicated
 * component surface.
 */

import { describe, expect, it } from "vitest";

import { CAPABILITY_ACTIONS } from "../src/data/contract";
import { can, capabilityDenialTitle } from "../src/lib/capabilityMatrix";

describe("simulate capability vocabulary", () => {
  it("includes `simulate` in the canonical action list (kernel parity)", () => {
    expect(CAPABILITY_ACTIONS).toContain("simulate");
    // Canonical order: …book, risk_transfer, simulate, administer (mirroring
    // `Action::ALL`, which inserts `RiskTransfer` between `Book` and `Simulate`).
    expect(CAPABILITY_ACTIONS.indexOf("risk_transfer")).toBe(
      CAPABILITY_ACTIONS.indexOf("book") + 1,
    );
    expect(CAPABILITY_ACTIONS.indexOf("simulate")).toBe(
      CAPABILITY_ACTIONS.indexOf("risk_transfer") + 1,
    );
    expect(CAPABILITY_ACTIONS.indexOf("administer")).toBe(
      CAPABILITY_ACTIONS.indexOf("simulate") + 1,
    );
  });

  it("`can` admits a held simulate cap and denies an empty set", () => {
    const held = [{ action: "simulate", asset: "fixed_income" } as const];
    expect(can(held, "simulate", "fixed_income")).toBe(true);
    expect(can(held, "simulate", "fx_options")).toBe(false);
    expect(can([], "simulate", "fixed_income")).toBe(false);
  });

  it("the denial tooltip explains the simulator (never a silent grey-out)", () => {
    const title = capabilityDenialTitle("simulate", "fixed_income");
    expect(title).toMatch(/permissions don't allow/i);
    expect(title).toMatch(/simulator/i);
  });
});
