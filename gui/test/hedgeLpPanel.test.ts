/**
 * Pure hedging-LP-panel resolver tests (client mirror of the server
 * `celnet_hedge_routing::HedgeLpPanel::effective_lps` + `resolve_lp_panel`, 12e08c8):
 * include-or-all minus exclude, unknown-id / empty-set validation, and the
 * most-specific-wins (instrument > book > desk) scope resolution.
 */
import { describe, expect, it } from "vitest";

import type { HedgeLpPanel } from "../src/data/contract";
import {
  KNOWN_LPS,
  effectiveLps,
  resolveLpPanelForScope,
  validateLpPanel,
} from "../src/lib/hedgeLpPanel";

const panel = (p: Partial<HedgeLpPanel>): HedgeLpPanel => ({
  scopeKind: "book",
  scopeId: "fi-rates-emea",
  include: [],
  exclude: [],
  ...p,
});

describe("effectiveLps — include/exclude resolution", () => {
  it("empty include starts from ALL known LPs", () => {
    expect(effectiveLps(panel({}))).toEqual([...KNOWN_LPS]);
  });
  it("a non-empty include is the base set, in order", () => {
    expect(effectiveLps(panel({ include: ["LP-3", "LP-1"] }))).toEqual(["LP-3", "LP-1"]);
  });
  it("subtracts the exclude set from all-known", () => {
    expect(effectiveLps(panel({ exclude: ["LP-4"] }))).toEqual(["LP-1", "LP-2", "LP-3"]);
  });
  it("include AND exclude interact — exclude wins over include", () => {
    expect(effectiveLps(panel({ include: ["LP-1", "LP-2", "LP-3"], exclude: ["LP-2"] }))).toEqual([
      "LP-1",
      "LP-3",
    ]);
  });
  it("dedups a repeated include preserving first-seen order", () => {
    expect(effectiveLps(panel({ include: ["LP-2", "LP-2", "LP-1"] }))).toEqual(["LP-2", "LP-1"]);
  });
});

describe("validateLpPanel — the server write-boundary checks", () => {
  it("accepts a well-formed panel (no defects)", () => {
    expect(validateLpPanel(panel({ include: ["LP-1"], exclude: ["LP-2"] }))).toEqual([]);
  });
  it("rejects an unknown include id", () => {
    const errs = validateLpPanel(panel({ include: ["LP-9"] }));
    expect(errs.some((e) => e.includes("LP-9"))).toBe(true);
  });
  it("rejects an unknown exclude id", () => {
    const errs = validateLpPanel(panel({ exclude: ["LP-X"] }));
    expect(errs.some((e) => e.includes("LP-X"))).toBe(true);
  });
  it("rejects a panel whose effective set is empty", () => {
    const errs = validateLpPanel(panel({ include: ["LP-1"], exclude: ["LP-1"] }));
    expect(errs.some((e) => e.toLowerCase().includes("empty"))).toBe(true);
  });
});

describe("resolveLpPanelForScope — most-specific-wins (instrument > book > desk)", () => {
  const panels: HedgeLpPanel[] = [
    { scopeKind: "desk", scopeId: "emea", include: [], exclude: ["LP-4"] },
    { scopeKind: "book", scopeId: "fi-rates-emea", include: ["LP-1", "LP-2", "LP-3"], exclude: ["LP-2"] },
    { scopeKind: "instrument", scopeId: "US10Y", include: ["LP-1"], exclude: [] },
  ];

  it("prefers the instrument panel when it matches", () => {
    const p = resolveLpPanelForScope(panels, { instrument: "US10Y", book: "fi-rates-emea", desk: "emea" });
    expect(p?.scopeKind).toBe("instrument");
  });
  it("falls back to the book panel when no instrument matches", () => {
    const p = resolveLpPanelForScope(panels, { instrument: "OIS-5Y", book: "fi-rates-emea", desk: "emea" });
    expect(p?.scopeKind).toBe("book");
    expect(effectiveLps(p as HedgeLpPanel)).toEqual(["LP-1", "LP-3"]);
  });
  it("falls back to the desk panel when neither instrument nor book matches", () => {
    const p = resolveLpPanelForScope(panels, { instrument: "X", book: "other", desk: "emea" });
    expect(p?.scopeKind).toBe("desk");
  });
  it("returns null when no scope matches (caller falls back to per-rule / full panel)", () => {
    expect(resolveLpPanelForScope(panels, { instrument: "X", book: "Y", desk: "Z" })).toBeNull();
  });
});
