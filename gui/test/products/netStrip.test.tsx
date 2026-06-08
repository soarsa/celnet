/**
 * GW2 — NetStructureStrip: the running net-economics summary shown while a trader
 * structures a multi-leg deal. Verifies the sign-by-side, scale-by-ratio
 * aggregation across a 2-leg risk-reversal; the honest-absence rule (any leg
 * missing a measure ⇒ that net reads "—", NOT a wrong partial sum); and the
 * single-leg passthrough (net == the lone leg's signed, scaled value).
 */
import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";

import {
  NetStructureStrip,
  type NetStructureLeg,
} from "../../src/products/NetStructureStrip";
import { fmtSigned } from "../../src/lib/format";

/** The strip renders one `<div>` cell per measure; read the value text by its dt label. */
function valueByLabel(label: string): string {
  const group = screen.getByRole("group");
  const dt = within(group)
    .getAllByRole("term")
    .find((t) => t.textContent === label);
  if (!dt) throw new Error(`no measure cell labelled "${label}"`);
  const dd = dt.nextElementSibling;
  if (!(dd instanceof HTMLElement)) throw new Error(`measure "${label}" has no value`);
  return dd.textContent ?? "";
}

describe("<NetStructureStrip>", () => {
  it("aggregates premium/Δ/ν/Γ with correct signs across a 2-leg risk-reversal", () => {
    // Long the 25Δ call, short the 25Δ put — the canonical RR.
    const legs: NetStructureLeg[] = [
      { side: "BUY", ratio: 1, premium: 0.004, delta: 0.25, vega: 0.18, gamma: 0.06 },
      { side: "SELL", ratio: 1, premium: 0.003, delta: -0.25, vega: 0.18, gamma: 0.06 },
    ];
    render(<NetStructureStrip legs={legs} baseCcy="EUR" quoteCcy="USD" />);

    // premium: (+1·0.004) + (−1·0.003) = 0.001 ⇒ 0.100 % (fmtPremiumPct ×100, 3dp).
    expect(valueByLabel("Premium")).toContain("0.100");
    // delta: (+0.25) + (−1·−0.25) = +0.50.
    expect(valueByLabel("Δ")).toBe(fmtSigned(0.5, 3));
    // vega: (+0.18) + (−0.18) = 0 (a long-vol leg against an equal short-vol leg).
    expect(valueByLabel("ν")).toBe(fmtSigned(0, 4));
    // gamma: (+0.06) + (−0.06) = 0.
    expect(valueByLabel("Γ")).toBe(fmtSigned(0, 4));

    // Fully-populated ⇒ no honesty note.
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("respects leg ratios when scaling each contribution", () => {
    const legs: NetStructureLeg[] = [
      { side: "BUY", ratio: 2, premium: 0.005, delta: 0.3, vega: 0.2, gamma: 0.05 },
      { side: "SELL", ratio: 1, premium: 0.004, delta: 0.1, vega: 0.2, gamma: 0.05 },
    ];
    render(<NetStructureStrip legs={legs} baseCcy="GBP" quoteCcy="USD" />);

    // premium: (2·0.005) − (1·0.004) = 0.006 ⇒ 0.600 %.
    expect(valueByLabel("Premium")).toContain("0.600");
    // delta: (2·0.3) − (1·0.1) = 0.5.
    expect(valueByLabel("Δ")).toBe(fmtSigned(0.5, 3));
    // vega: (2·0.2) − (1·0.2) = 0.2.
    expect(valueByLabel("ν")).toBe(fmtSigned(0.2, 4));
  });

  it("shows — for a net whose legs are missing that measure (not a wrong partial sum)", () => {
    const legs: NetStructureLeg[] = [
      { side: "BUY", ratio: 1, premium: 0.004, delta: 0.25, vega: 0.18, gamma: 0.06 },
      // Second leg has NO vega — so the net vega is unknowable, must read "—".
      { side: "SELL", ratio: 1, premium: 0.003, delta: -0.25, gamma: 0.06 },
    ];
    render(<NetStructureStrip legs={legs} baseCcy="EUR" quoteCcy="USD" />);

    // delta/premium/gamma are present on both legs ⇒ real numbers.
    expect(valueByLabel("Premium")).toContain("0.100");
    expect(valueByLabel("Δ")).toBe(fmtSigned(0.5, 3));
    expect(valueByLabel("Γ")).toBe(fmtSigned(0, 4));

    // vega is missing on a leg ⇒ "—", NOT the present-leg partial 0.18.
    const vegaText = valueByLabel("ν");
    expect(vegaText).toBe("—");
    expect(vegaText).not.toContain("0.18");

    // The em-dash carries an accessible reason, and the honesty note appears.
    expect(
      screen.getByRole("img", { name: /missing this value/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("note")).toBeInTheDocument();
  });

  it("single-leg structure passes the one leg's signed, scaled value straight through", () => {
    const legs: NetStructureLeg[] = [
      { side: "SELL", ratio: 1, premium: 0.0035, delta: 0.4, vega: 0.22, gamma: 0.07 },
    ];
    render(<NetStructureStrip legs={legs} baseCcy="USD" quoteCcy="JPY" />);

    // Sold ⇒ sign flips: premium (−0.0035) ⇒ −0.350 %; delta −0.4; vega −0.22; gamma −0.07.
    expect(valueByLabel("Premium")).toContain("0.350");
    expect(valueByLabel("Premium")).toMatch(/−|-/);
    expect(valueByLabel("Δ")).toBe(fmtSigned(-0.4, 3));
    expect(valueByLabel("ν")).toBe(fmtSigned(-0.22, 4));
    expect(valueByLabel("Γ")).toBe(fmtSigned(-0.07, 4));
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("is a labelled group naming the pair", () => {
    render(<NetStructureStrip legs={[]} baseCcy="EUR" quoteCcy="USD" />);
    expect(screen.getByRole("group", { name: /EUR\/USD/i })).toBeInTheDocument();
  });
});
