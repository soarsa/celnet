/**
 * capabilityMatrix — the pure resolution behind the Admin capability editor.
 * These assert the grant / deny / inherit tri-state resolves exactly like the
 * server's capability algebra (`role bundle ∪ grants ∖ denies`, deny-wins) and
 * that a deny visibly overrides a would-be-allowed role default.
 */

import { describe, expect, it } from "vitest";

import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../src/data/contract";
import {
  can,
  capKey,
  overlayFromCapabilities,
  overlayStateAt,
  overlayToCapabilities,
  overlaysDiffer,
  nextOverlay,
  resolveCell,
  resolveEffective,
  roleAllows,
  roleBaselineSummary,
} from "../src/lib/capabilityMatrix";

describe("roleAllows — the role bundle baseline", () => {
  it("admin holds every action (grant-all)", () => {
    for (const action of CAPABILITY_ACTIONS) {
      expect(roleAllows("ADMIN", action)).toBe(true);
    }
  });

  it("trader holds every action except the five held-back authorities", () => {
    // The default trader bundle withholds administer, risk_transfer, and the three
    // granular management caps (mirrors the server's default_trader_bundle).
    const heldBack = new Set([
      "administer",
      "risk_transfer",
      "risk_manage",
      "manage_pricing",
      "manage_liquidity",
    ]);
    for (const action of CAPABILITY_ACTIONS) {
      expect(roleAllows("TRADER", action)).toBe(!heldBack.has(action));
    }
  });
});

describe("resolveCell — inherit / grant / deny", () => {
  it("inherit follows the role default", () => {
    // Trader inherits price (allowed) but not administer (blocked).
    expect(resolveCell("TRADER", "price", "fx_options", "inherit").allowed).toBe(true);
    expect(resolveCell("TRADER", "administer", "fx_options", "inherit").allowed).toBe(false);
  });

  it("a grant widens a role default that would otherwise block", () => {
    const cell = resolveCell("TRADER", "administer", "fixed_income", "grant");
    expect(cell.allowed).toBe(true);
    expect(cell.roleAllows).toBe(false);
  });

  it("a deny narrows a role default that would otherwise allow", () => {
    const cell = resolveCell("TRADER", "execute", "fx_options", "deny");
    expect(cell.allowed).toBe(false);
  });

  it("deny wins over a would-be-allowed role default and flags the override", () => {
    const cell = resolveCell("ADMIN", "execute", "fixed_income", "deny");
    expect(cell.allowed).toBe(false);
    expect(cell.denyOverrides).toBe(true);
  });

  it("a deny on an already-blocked role default is not flagged as overriding", () => {
    const cell = resolveCell("TRADER", "administer", "fx_options", "deny");
    expect(cell.allowed).toBe(false);
    expect(cell.denyOverrides).toBe(false);
  });
});

describe("nextOverlay — the cell cycle", () => {
  it("cycles inherit → grant → deny → inherit", () => {
    expect(nextOverlay("inherit")).toBe("grant");
    expect(nextOverlay("grant")).toBe("deny");
    expect(nextOverlay("deny")).toBe("inherit");
  });
});

describe("overlay <-> capabilities round-trip", () => {
  it("builds a map from grants/denies and serializes it back in canonical order", () => {
    const grants = [{ action: "administer", asset: "fixed_income" } as const];
    const denies = [{ action: "execute", asset: "fx_options" } as const];
    const map = overlayFromCapabilities(grants, denies);
    expect(overlayStateAt(map, "administer", "fixed_income")).toBe("grant");
    expect(overlayStateAt(map, "execute", "fx_options")).toBe("deny");
    expect(overlayStateAt(map, "view", "fx_options")).toBe("inherit");

    const back = overlayToCapabilities(map);
    expect(back.grants).toEqual(grants);
    expect(back.denies).toEqual(denies);
  });

  it("overlaysDiffer detects a single changed cell", () => {
    const a = overlayFromCapabilities([], []);
    const b = overlayFromCapabilities([{ action: "book", asset: "fixed_income" } as const], []);
    expect(overlaysDiffer(a, b)).toBe(true);
    expect(overlaysDiffer(a, overlayFromCapabilities([], []))).toBe(false);
  });
});

describe("resolveEffective — full enumeration matches the server algebra", () => {
  it("trader with no overlay = every action except administer, both assets", () => {
    const effective = resolveEffective("TRADER", new Map());
    const keys = new Set(effective.map((c) => capKey(c.action, c.asset)));
    // 9 non-admin actions × 2 assets = 18.
    expect(effective.length).toBe(18);
    expect(keys.has(capKey("administer", "fx_options"))).toBe(false);
    expect(keys.has(capKey("administer", "fixed_income"))).toBe(false);
    expect(keys.has(capKey("execute", "fx_options"))).toBe(true);
  });

  it("admin with no overlay = the full 14 × 2 grid", () => {
    const effective = resolveEffective("ADMIN", new Map());
    expect(effective.length).toBe(CAPABILITY_ACTIONS.length * CAPABILITY_ASSETS.length);
  });

  it("grants widen and denies narrow (deny-wins) in the resolved set", () => {
    // Trader: grant administer·FI, deny execute·FX.
    const map = overlayFromCapabilities(
      [{ action: "administer", asset: "fixed_income" } as const],
      [{ action: "execute", asset: "fx_options" } as const],
    );
    const keys = new Set(resolveEffective("TRADER", map).map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("administer", "fixed_income"))).toBe(true); // widened
    expect(keys.has(capKey("execute", "fx_options"))).toBe(false); // narrowed (deny-wins)
    expect(keys.has(capKey("administer", "fx_options"))).toBe(false); // untouched role default
  });

  it("a deny on an admin removes exactly that capability and nothing else", () => {
    const map = overlayFromCapabilities([], [{ action: "book", asset: "fx_options" } as const]);
    const keys = new Set(resolveEffective("ADMIN", map).map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("book", "fx_options"))).toBe(false);
    expect(keys.has(capKey("book", "fixed_income"))).toBe(true);
  });
});

describe("roleBaselineSummary — the honest per-asset role-baseline chip counts", () => {
  it("admin holds every action on both assets (14/14 · 14/14)", () => {
    const summary = roleBaselineSummary("ADMIN");
    expect(summary).toEqual([
      { asset: "fx_options", allowed: CAPABILITY_ACTIONS.length, total: CAPABILITY_ACTIONS.length },
      { asset: "fixed_income", allowed: CAPABILITY_ACTIONS.length, total: CAPABILITY_ACTIONS.length },
    ]);
  });

  it("trader holds every action except the five held-back authorities on both assets (9/14 · 9/14)", () => {
    const summary = roleBaselineSummary("TRADER");
    const total = CAPABILITY_ACTIONS.length;
    expect(summary).toEqual([
      { asset: "fx_options", allowed: total - 5, total },
      { asset: "fixed_income", allowed: total - 5, total },
    ]);
    // Sanity: administer + risk_transfer + risk_manage + manage_pricing +
    // manage_liquidity are the five dropped actions, per asset (14 total → 9 held).
    expect(total).toBe(14);
  });

  it("emits one entry per asset in canonical order", () => {
    expect(roleBaselineSummary("TRADER").map((s) => s.asset)).toEqual(CAPABILITY_ASSETS);
  });
});

describe("can — the affordance-gating membership selector", () => {
  it("returns true exactly for a capability present in the effective set", () => {
    const caps = [
      { action: "price", asset: "fx_options" },
      { action: "execute", asset: "fx_options" },
    ] as const;
    expect(can(caps, "price", "fx_options")).toBe(true);
    expect(can(caps, "execute", "fx_options")).toBe(true);
  });

  it("returns false when the action is held on a DIFFERENT asset class", () => {
    // A user who may execute FX must NOT thereby be able to execute fixed income.
    const caps = [{ action: "execute", asset: "fx_options" }] as const;
    expect(can(caps, "execute", "fx_options")).toBe(true);
    expect(can(caps, "execute", "fixed_income")).toBe(false);
  });

  it("returns false for an absent action and for an empty (deny-all) set", () => {
    const caps = [{ action: "view", asset: "fixed_income" }] as const;
    expect(can(caps, "book", "fixed_income")).toBe(false);
    expect(can([], "price", "fx_options")).toBe(false);
  });

  it("agrees with resolveEffective for a deny-narrowed trader", () => {
    // Deny execute·fixed_income on a TRADER: every OTHER cell stays held, only
    // that one affordance is gated off — exactly what the GUI disables.
    const map = overlayFromCapabilities(
      [],
      [{ action: "execute", asset: "fixed_income" } as const],
    );
    const effective = resolveEffective("TRADER", map);
    expect(can(effective, "execute", "fixed_income")).toBe(false);
    expect(can(effective, "execute", "fx_options")).toBe(true);
    expect(can(effective, "price", "fixed_income")).toBe(true);
    expect(can(effective, "book", "fixed_income")).toBe(true);
  });
});
