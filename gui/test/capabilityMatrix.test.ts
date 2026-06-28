/**
 * capabilityMatrix — the pure resolution behind the Admin capability editor.
 * These assert the grant / deny / inherit tri-state resolves exactly like the
 * server's capability algebra (`role bundle ∪ grants ∖ denies`, deny-wins) and
 * that a deny visibly overrides a would-be-allowed role default.
 */

import { describe, expect, it } from "vitest";

import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../src/data/contract";
import {
  capKey,
  overlayFromCapabilities,
  overlayStateAt,
  overlayToCapabilities,
  overlaysDiffer,
  nextOverlay,
  resolveCell,
  resolveEffective,
  roleAllows,
} from "../src/lib/capabilityMatrix";

describe("roleAllows — the role bundle baseline", () => {
  it("admin holds every action (grant-all)", () => {
    for (const action of CAPABILITY_ACTIONS) {
      expect(roleAllows("ADMIN", action)).toBe(true);
    }
  });

  it("trader holds every action except administer", () => {
    for (const action of CAPABILITY_ACTIONS) {
      expect(roleAllows("TRADER", action)).toBe(action !== "administer");
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
    // 8 non-admin actions × 2 assets = 16.
    expect(effective.length).toBe(16);
    expect(keys.has(capKey("administer", "fx_options"))).toBe(false);
    expect(keys.has(capKey("administer", "fixed_income"))).toBe(false);
    expect(keys.has(capKey("execute", "fx_options"))).toBe(true);
  });

  it("admin with no overlay = the full 9 × 2 grid", () => {
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
