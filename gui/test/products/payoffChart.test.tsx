/**
 * Gate for the {@link PayoffChart} (GW2) — the inline-SVG payoff-at-expiry
 * mini-chart. We test the pure terminal-payoff function directly (a vanilla call
 * is 0 below its strike and rises monotonically above it) and the component's
 * honesty contract: path-dependent families render the em-dash empty state rather
 * than a fabricated curve, and a drawable structure carries an accessible
 * `aria-label` describing the shape.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { PayoffChart, payoffAt, payoffAtExpiry } from "../../src/products/PayoffChart";

/** A monotone spot grid around 1.10 for the pure-function tests. */
const GRID = Array.from({ length: 41 }, (_, i) => 1.0 + i * 0.005); // 1.000 … 1.200
const STRIKE = 1.1;
const SPOT = 1.1;

describe("payoffAtExpiry (pure)", () => {
  it("vanilla call payoff is 0 below the strike and rises above it", () => {
    const payoff = payoffAtExpiry("VANILLA", { strike: STRIKE, spot: SPOT }, GRID);
    expect(payoff).not.toBeNull();
    const ys = payoff as number[];
    expect(ys).toHaveLength(GRID.length);

    // Every below-strike sample is exactly 0; every above-strike sample is > 0
    // and equals the intrinsic s - K.
    for (let i = 0; i < GRID.length; i += 1) {
      const s = GRID[i]!;
      const y = ys[i]!;
      if (s < STRIKE) {
        expect(y).toBe(0);
      } else {
        expect(y).toBeCloseTo(Math.max(s - STRIKE, 0), 12);
      }
    }

    // Strictly non-decreasing across the grid (a vanilla call is monotone in spot).
    for (let i = 1; i < ys.length; i += 1) {
      expect(ys[i]!).toBeGreaterThanOrEqual(ys[i - 1]!);
    }
    // It genuinely rises above the strike (not a flat zero everywhere).
    expect(ys[ys.length - 1]!).toBeGreaterThan(0);
  });

  it("SELL mirrors the vanilla payoff through zero (0 below strike, negative above)", () => {
    const buy = payoffAt("VANILLA", { strike: STRIKE, spot: SPOT }, 1.15)!;
    const sell = payoffAt("VANILLA", { strike: STRIKE, spot: SPOT, side: "SELL" }, 1.15)!;
    expect(buy).toBeGreaterThan(0);
    expect(sell).toBeCloseTo(-buy, 12);
  });

  it("digital is a unit step at the strike", () => {
    expect(payoffAt("DIGITAL", { strike: STRIKE, spot: SPOT }, STRIKE - 0.01)).toBe(0);
    expect(payoffAt("DIGITAL", { strike: STRIKE, spot: SPOT }, STRIKE + 0.01)).toBe(1);
  });

  it("returns null for every path-dependent structure (honest, no fabricated curve)", () => {
    const pathDependent = [
      "ASIAN",
      "LOOKBACK",
      "CLIQUET",
      "TARF",
      "ACCUMULATOR",
      "VARIANCE_SWAP",
      "VOLATILITY_SWAP",
      "FORWARD_START",
      "QUANTO",
      "AMERICAN",
      "BASKET",
    ];
    for (const id of pathDependent) {
      expect(payoffAtExpiry(id, { strike: STRIKE, spot: SPOT }, GRID)).toBeNull();
      expect(payoffAt(id, { strike: STRIKE, spot: SPOT }, SPOT)).toBeNull();
    }
  });

  it("draws the terminal strategy / barrier families (non-null curves)", () => {
    for (const id of [
      "RISK_REVERSAL",
      "STRANGLE",
      "STRADDLE",
      "SEAGULL",
      "SINGLE_BARRIER",
      "DOUBLE_BARRIER",
      "WINDOW_BARRIER",
      "TOUCH",
    ]) {
      const payoff = payoffAtExpiry(id, { strike: STRIKE, spot: SPOT }, GRID);
      expect(payoff).not.toBeNull();
      expect(payoff as number[]).toHaveLength(GRID.length);
    }
  });
});

describe("PayoffChart (component)", () => {
  it("renders an SVG with a descriptive aria-label for a drawable structure", () => {
    render(<PayoffChart structureId="VANILLA" strike={STRIKE} spot={SPOT} />);
    const img = screen.getByRole("img");
    expect(img.tagName.toLowerCase()).toBe("svg");
    expect(img.getAttribute("aria-label")).toMatch(/vanilla payoff at expiry/i);
  });

  it("renders the honest empty state for a path-dependent structure", () => {
    render(<PayoffChart structureId="ASIAN" strike={STRIKE} spot={SPOT} />);
    const img = screen.getByRole("img");
    expect(img.tagName.toLowerCase()).not.toBe("svg");
    expect(img.getAttribute("aria-label")).toMatch(/average fixing/i);
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("renders the honest empty state for a non-positive spot rather than a broken curve", () => {
    render(<PayoffChart structureId="VANILLA" strike={STRIKE} spot={0} />);
    const img = screen.getByRole("img");
    expect(img.tagName.toLowerCase()).not.toBe("svg");
    expect(img.getAttribute("aria-label")).toMatch(/unavailable/i);
  });
});
