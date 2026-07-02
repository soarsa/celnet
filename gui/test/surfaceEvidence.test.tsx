/**
 * SurfaceWorkspace publish-evidence trail — the "show me WHY a publish is
 * blocked" region (mockup 01's evidence trail, CRITIQUE-ROUND2 gap). Pins the
 * pure derivation (`brokerImpliedPillarVol` + `deriveSurfaceEvidence`) against
 * the REAL ladder calibration (no mocks of our own functionality) and the
 * rendered `SurfaceEvidenceTrail` behaviour: auto-open exactly when the gate is
 * blocked, honest null cells for unquoted 10Δ pillars, and per-tenor arb-gate
 * pass/fail rows that localize the violation.
 */
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import {
  SurfaceEvidenceTrail,
  brokerImpliedPillarVol,
  deriveSurfaceEvidence,
} from "../src/workspaces/SurfaceWorkspace";
import { calibrateLadder } from "../src/data/surface";
import { DEFAULT_CONVENTIONS } from "../src/data/seed";
import type { BrokerQuoteSet, CcyPair, Smile } from "../src/data/contract";

afterEach(cleanup);

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const NOW = 1_717_000_000_000_000_000n;

/** A well-formed broker quote set (put skew, positive butterfly). */
function quote(overrides: Partial<BrokerQuoteSet> = {}): BrokerQuoteSet {
  return {
    tenorYears: 0.25,
    atmVol: 0.08,
    rr25: -0.003,
    bf25: 0.0016,
    rr10: -0.0055,
    bf10: 0.0037,
    hasTenDelta: true,
    ...overrides,
  };
}

/** A calendar-arb-free 3-tenor ladder (ATM total variance rises in T). */
function cleanLadder(): Smile[] {
  return calibrateLadder(
    PAIR,
    [
      quote({ tenorYears: 30 / 365, atmVol: 0.075 }),
      quote({ tenorYears: 0.25, atmVol: 0.08 }),
      quote({ tenorYears: 1.0, atmVol: 0.09 }),
    ],
    DEFAULT_CONVENTIONS,
    NOW,
  );
}

/** A ladder whose 1Y ATM total variance FALLS below the 3M's ⇒ calendar arb. */
function calendarArbLadder(): Smile[] {
  return calibrateLadder(
    PAIR,
    [quote({ tenorYears: 0.25, atmVol: 0.2 }), quote({ tenorYears: 1.0, atmVol: 0.05 })],
    DEFAULT_CONVENTIONS,
    NOW,
  );
}

describe("brokerImpliedPillarVol — the market side of the residual", () => {
  const q = quote();

  it("ATM pillar is the quoted ATM exactly", () => {
    expect(brokerImpliedPillarVol(q, 0.5)).toBeCloseTo(q.atmVol, 12);
    expect(brokerImpliedPillarVol(q, -0.5)).toBeCloseTo(q.atmVol, 12);
  });

  it("25Δ pillars follow the broker decomposition atm + bf25 ± rr25/2", () => {
    expect(brokerImpliedPillarVol(q, 0.25)).toBeCloseTo(q.atmVol + q.bf25 + q.rr25 / 2, 12);
    expect(brokerImpliedPillarVol(q, -0.25)).toBeCloseTo(q.atmVol + q.bf25 - q.rr25 / 2, 12);
  });

  it("10Δ pillars follow atm + bf10 ± rr10/2 when the ladder quotes 10Δ", () => {
    expect(brokerImpliedPillarVol(q, 0.1)).toBeCloseTo(q.atmVol + q.bf10 + q.rr10 / 2, 12);
    expect(brokerImpliedPillarVol(q, -0.1)).toBeCloseTo(q.atmVol + q.bf10 - q.rr10 / 2, 12);
  });

  it("is honestly null off the quoted pillars: 3-pt ladders at 10Δ, non-pillar deltas", () => {
    expect(brokerImpliedPillarVol(quote({ hasTenDelta: false }), 0.1)).toBeNull();
    expect(brokerImpliedPillarVol(quote({ hasTenDelta: false }), -0.1)).toBeNull();
    expect(brokerImpliedPillarVol(q, 0.35)).toBeNull();
    expect(brokerImpliedPillarVol(q, Number.NaN)).toBeNull();
  });
});

describe("deriveSurfaceEvidence — residual grid, term slice, arb-gate rows", () => {
  it("columns are the wing order 10ΔP…ATM…10ΔC; rows ascend in tenor", () => {
    const ev = deriveSurfaceEvidence(cleanLadder());
    expect(ev.wings).toEqual(["10ΔP", "25ΔP", "ATM", "25ΔC", "10ΔC"]);
    expect(ev.rows.map((r) => r.tenor)).toEqual(["1M", "3M", "1Y"]);
    const years = ev.rows.map((r) => r.tenorYears);
    expect([...years].sort((a, b) => a - b)).toEqual(years);
  });

  it("each residual is (implied − model)·100 vol pts against the CALIBRATED point", () => {
    const smiles = cleanLadder();
    const ev = deriveSurfaceEvidence(smiles);
    const smile = smiles.find((s) => s.tenorYears === 0.25)!;
    const row = ev.rows.find((r) => r.tenorYears === 0.25)!;
    const q = smile.brokerQuotes;
    // 10ΔP: implied from the raw broker decomposition, model from the smile point.
    const model = smile.points.find((p) => p.delta === -0.1)!.vol;
    const implied = q.atmVol + q.bf10 - q.rr10 / 2;
    const cell = row.cells.find((c) => c.wing === "10ΔP")!;
    expect(cell.residual).not.toBeNull();
    expect(cell.residual!).toBeCloseTo((implied - model) * 100, 10);
  });

  it("the default market-hedge fit reproduces ATM and 25Δ exactly (zero residual) but not 10Δ", () => {
    const ev = deriveSurfaceEvidence(cleanLadder());
    for (const row of ev.rows) {
      const at = (wing: string): number | null =>
        row.cells.find((c) => c.wing === wing)?.residual ?? null;
      expect(at("ATM")).toBeCloseTo(0, 9);
      expect(at("25ΔP")).toBeCloseTo(0, 9);
      expect(at("25ΔC")).toBeCloseTo(0, 9);
      // The wing construction bends the far wing away from the raw quote — a
      // real fit residual, which is exactly what the evidence must surface.
      expect(Math.abs(at("10ΔP")!)).toBeGreaterThan(1e-6);
      expect(Math.abs(at("10ΔC")!)).toBeGreaterThan(1e-6);
    }
  });

  it("worst cell + symmetric domain: |worst| equals maxAbsResidual", () => {
    const ev = deriveSurfaceEvidence(cleanLadder());
    expect(ev.worst).not.toBeNull();
    expect(Math.abs(ev.worst!.residual)).toBeCloseTo(ev.maxAbsResidual, 12);
    expect(ev.wings).toContain(ev.worst!.wing);
    expect(ev.rows.map((r) => r.tenor)).toContain(ev.worst!.tenor);
  });

  it("a 3-pt ladder's 10Δ cells are honestly null (no fabricated market vol)", () => {
    const smiles = calibrateLadder(
      PAIR,
      [quote({ tenorYears: 0.25, hasTenDelta: false })],
      DEFAULT_CONVENTIONS,
      NOW,
    );
    const ev = deriveSurfaceEvidence(smiles);
    const row = ev.rows[0]!;
    expect(row.cells.find((c) => c.wing === "10ΔP")!.residual).toBeNull();
    expect(row.cells.find((c) => c.wing === "10ΔC")!.residual).toBeNull();
    expect(row.cells.find((c) => c.wing === "ATM")!.residual).toBeCloseTo(0, 9);
  });

  it("the term slice carries the calibrated ATM per tenor; arb-gate rows localize a calendar arb", () => {
    const ev = deriveSurfaceEvidence(calendarArbLadder());
    expect(ev.rows.map((r) => r.atmVol)).toEqual([0.2, 0.05]);
    const threeM = ev.rows.find((r) => r.tenorYears === 0.25)!;
    const oneY = ev.rows.find((r) => r.tenorYears === 1.0)!;
    expect(threeM.calendarOk).toBe(true);
    expect(oneY.calendarOk).toBe(false);
    expect(oneY.note).toMatch(/calendar/i);
  });

  it("a butterfly breach flags exactly the offending tenor's row", () => {
    const smiles = calibrateLadder(
      PAIR,
      [
        quote({ tenorYears: 0.25 }),
        quote({ tenorYears: 1.0, atmVol: 0.09, bf25: -0.03, bf10: -0.05 }),
      ],
      DEFAULT_CONVENTIONS,
      NOW,
    );
    const ev = deriveSurfaceEvidence(smiles);
    expect(ev.rows.find((r) => r.tenorYears === 0.25)!.butterflyOk).toBe(true);
    expect(ev.rows.find((r) => r.tenorYears === 1.0)!.butterflyOk).toBe(false);
  });

  it("no smiles ⇒ an empty, honest evidence set (nothing invented)", () => {
    expect(deriveSurfaceEvidence([])).toEqual({
      wings: [],
      rows: [],
      maxAbsResidual: 0,
      worst: null,
    });
  });
});

describe("SurfaceEvidenceTrail — the collapsible WHERE/WHY region", () => {
  it("auto-opens when the publish gate is blocked and names the failing tenor", () => {
    render(<SurfaceEvidenceTrail smiles={calendarArbLadder()} />);
    const toggle = screen.getByRole("button", { name: /evidence/i });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(toggle.textContent).toMatch(/publish blocked/i);

    const region = screen.getByRole("region", { name: /publish evidence/i });
    expect(within(region).getByRole("table", { name: /implied minus model/i })).toBeTruthy();
    expect(within(region).getByRole("img", { name: /ATM vol term structure/i })).toBeTruthy();

    // The arb-gate table localizes the violation: exactly one calendar "✕ fail".
    const gate = within(region).getByRole("table", { name: /arbitrage gate/i });
    expect(within(gate).getAllByText("✕ fail")).toHaveLength(1);
    const failRow = within(gate).getByText("✕ fail").closest("tr")!;
    expect(within(failRow).getByText("1Y")).toBeTruthy();
  });

  it("stays collapsed while arb-free, and opens/closes on the trader's toggle", () => {
    render(<SurfaceEvidenceTrail smiles={cleanLadder()} />);
    const toggle = screen.getByRole("button", { name: /evidence/i });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("region", { name: /publish evidence/i })).toBeNull();

    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const region = screen.getByRole("region", { name: /publish evidence/i });
    // Arb-free ⇒ every gate cell passes (2 checks × 3 tenors) and none fail.
    const gate = within(region).getByRole("table", { name: /arbitrage gate/i });
    expect(within(gate).getAllByText("✓ pass")).toHaveLength(6);
    expect(within(gate).queryByText("✕ fail")).toBeNull();

    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
  });

  it("unquoted 10Δ pillars render as em-dash cells, never a fabricated 0", () => {
    const smiles = calibrateLadder(
      PAIR,
      [quote({ tenorYears: 0.25, hasTenDelta: false })],
      DEFAULT_CONVENTIONS,
      NOW,
    );
    render(<SurfaceEvidenceTrail smiles={smiles} />);
    fireEvent.click(screen.getByRole("button", { name: /evidence/i }));
    const resid = screen.getByRole("table", { name: /implied minus model/i });
    const dashes = within(resid)
      .getAllByTitle(/no broker quote at this pillar/i)
      .map((el) => el.textContent);
    expect(dashes).toEqual(["—", "—"]);
  });

  it("renders an honest empty state with no smiles (no table, no chart)", () => {
    render(<SurfaceEvidenceTrail smiles={[]} />);
    fireEvent.click(screen.getByRole("button", { name: /evidence/i }));
    expect(screen.getByText(/no calibrated smiles — nothing to evidence/i)).toBeTruthy();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByRole("img")).toBeNull();
  });
});
