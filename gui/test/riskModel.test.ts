import { describe, expect, it } from "vitest";

import type { HedgingModelBinding } from "../src/data/contract";
import {
  explainRiskModel,
  modelUsesBudget,
  resolveRiskModel,
  HEDGING_MODELS,
  HEDGING_MODEL_LABEL,
} from "../src/lib/riskModel";

const bind = (
  scopeKind: HedgingModelBinding["scopeKind"],
  scopeId: string,
  model: HedgingModelBinding["model"],
  dv01Budget = 0,
): HedgingModelBinding => ({ scopeKind, scopeId, model, dv01Budget });

describe("risk model resolution — most-specific-wins", () => {
  it("an UNBOUND portfolio is Custom, not 'no hedging'", () => {
    const r = resolveRiskModel([], { deskId: "emea", bookId: "fi-rates" });
    expect(r.model).toBe(0);
    expect(r.source).toBeNull();
    // The budget is null (nothing imposed), NOT zero — zero would mean warehouse nothing.
    expect(r.budget).toBeNull();
    expect(explainRiskModel(r, "fi-rates")).toContain("authored for this scope governs");
  });

  it("a book binding OVERRIDES its desk (the toxic-book case)", () => {
    const bindings = [bind("desk", "emea", 2, 25_000), bind("book", "fi-marex", 1)];
    const r = resolveRiskModel(bindings, { deskId: "emea", bookId: "fi-marex" });
    expect(r.model).toBe(1); // back-to-back wins over the desk's internalise
    expect(r.source?.scopeKind).toBe("book");
    // Back-to-back warehouses nothing, so it imposes no budget even though the desk set one.
    expect(r.budget).toBeNull();
    expect(explainRiskModel(r, "fi-marex")).toContain("set on this portfolio");
  });

  it("a portfolio with no binding of its own INHERITS the desk, and says so", () => {
    const bindings = [bind("desk", "emea", 2, 25_000)];
    const r = resolveRiskModel(bindings, { deskId: "emea", bookId: "fi-rates" });
    expect(r.model).toBe(2);
    expect(r.source?.scopeKind).toBe("desk");
    expect(r.budget).toBe(25_000);
    expect(explainRiskModel(r, "fi-rates")).toContain('inherited from desk “emea”');
  });

  it("an instrument binding beats BOTH book and desk", () => {
    const bindings = [
      bind("desk", "emea", 2, 25_000),
      bind("book", "fi-rates", 2, 10_000),
      bind("instrument", "US10Y", 1),
    ];
    const r = resolveRiskModel(bindings, {
      deskId: "emea",
      bookId: "fi-rates",
      instrumentId: "US10Y",
    });
    expect(r.model).toBe(1);
    expect(r.source?.scopeKind).toBe("instrument");
  });

  it("marks exactly ONE trace step effective, and orders least → most specific", () => {
    const bindings = [bind("desk", "emea", 2, 25_000), bind("book", "fi-rates", 1)];
    const r = resolveRiskModel(bindings, {
      deskId: "emea",
      bookId: "fi-rates",
      instrumentId: "US10Y",
    });
    expect(r.trace.map((s) => s.scopeKind)).toEqual(["desk", "book", "instrument"]);
    expect(r.trace.filter((s) => s.effective)).toHaveLength(1);
    // The unbound instrument step is still SHOWN (so the trader sees it was considered)
    // but is not effective.
    expect(r.trace[2]!.binding).toBeNull();
    expect(r.trace[2]!.effective).toBe(false);
    expect(r.trace[1]!.effective).toBe(true);
  });

  it("a desk-less portfolio simply has no desk step", () => {
    const r = resolveRiskModel([bind("book", "fi-rates", 1)], { deskId: null, bookId: "fi-rates" });
    expect(r.trace.map((s) => s.scopeKind)).toEqual(["book"]);
    expect(r.model).toBe(1);
  });

  it("matches scope ids case-insensitively, as the server does", () => {
    const r = resolveRiskModel([bind("book", "FI-Rates", 1)], {
      deskId: null,
      bookId: "fi-rates",
    });
    expect(r.model).toBe(1);
  });
});

describe("risk model — the DV01 budget inherit sentinel", () => {
  it("a blank/zero budget means INHERIT the configured threshold, never a zero cap", () => {
    const r = resolveRiskModel([bind("book", "fi-rates", 2, 0)], {
      deskId: null,
      bookId: "fi-rates",
    });
    expect(r.model).toBe(2);
    // The distinction that matters: null (inherit) rather than 0 (warehouse nothing).
    expect(r.budget).toBeNull();
    expect(explainRiskModel(r, "fi-rates")).toContain("configured warehouse threshold");
  });

  it("a NEGATIVE or non-finite budget is treated as blank, not as a cap", () => {
    for (const bad of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
      const r = resolveRiskModel([bind("book", "b", 2, bad)], { deskId: null, bookId: "b" });
      expect(r.budget).toBeNull();
    }
  });

  it("only the internalise model reads a budget at all", () => {
    expect(modelUsesBudget(0)).toBe(false);
    expect(modelUsesBudget(1)).toBe(false);
    expect(modelUsesBudget(2)).toBe(true);
    // A budget set against back-to-back is ignored rather than silently applied.
    const r = resolveRiskModel([bind("book", "b", 1, 99_000)], { deskId: null, bookId: "b" });
    expect(r.budget).toBeNull();
  });
});

describe("risk model — the selector tables", () => {
  it("labels every model in the selector, in wire order", () => {
    expect(HEDGING_MODELS).toEqual([0, 1, 2]);
    for (const m of HEDGING_MODELS) expect(HEDGING_MODEL_LABEL[m]).toBeTruthy();
  });
});
