/**
 * GreeksStrip class-correct rho labelling (multi-asset wave). All 14 Greeks are
 * always shown; only the two rate-rho labels change with the asset class so an
 * equity/crypto never reads "rho foreign" — the wire fields (rhoDom = rate rho,
 * rhoFor = carry rho) are unchanged, the labels carry the carry's real identity.
 */
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { GreeksStrip } from "../src/components/GreeksStrip";
import type { Greeks } from "../src/data/contract";

afterEach(cleanup);

const G: Greeks = {
  price: 1, deltaSpot: 0.5, deltaForward: 0.5, gamma: 0.1, vega: 0.2, theta: -0.01,
  rhoDom: 0.03, rhoFor: 0.02, vanna: 0.001, volga: 0.002, charm: 0.0001,
  speed: -0.1, zomma: 0.05, color: -0.0002,
};

/** Expand the strip so the secondary (rho) Greeks render. */
function renderExpanded(assetClass: Parameters<typeof GreeksStrip>[0]["assetClass"]) {
  render(<GreeksStrip greeks={G} assetClass={assetClass} />);
  fireEvent.click(screen.getByRole("button", { name: /toggle full greeks/i }));
}

describe("GreeksStrip — class-correct rho labels", () => {
  it("FX shows a two-rate pair (domestic / foreign)", () => {
    renderExpanded("FX");
    expect(screen.getByTitle("rho domestic")).toBeTruthy();
    expect(screen.getByTitle("rho foreign")).toBeTruthy();
  });

  it("EQUITY shows rate + dividend-yield rho, never 'rho foreign'", () => {
    renderExpanded("EQUITY");
    expect(screen.getByTitle("rho (rate)")).toBeTruthy();
    expect(screen.getByTitle("rho (dividend yield)")).toBeTruthy();
    expect(screen.queryByTitle("rho foreign")).toBeNull();
    expect(screen.queryByTitle("rho domestic")).toBeNull();
  });

  it("CRYPTO shows rate + funding rho", () => {
    renderExpanded("CRYPTO");
    expect(screen.getByTitle("rho (funding)")).toBeTruthy();
    expect(screen.queryByTitle("rho foreign")).toBeNull();
  });

  it("COMMODITY shows rate + net-carry rho", () => {
    renderExpanded("COMMODITY");
    expect(screen.getByTitle("rho (net carry)")).toBeTruthy();
  });

  it("always shows the 6 class-independent secondary Greeks (all 14 present)", () => {
    renderExpanded("EQUITY");
    for (const label of ["vanna", "volga", "charm", "speed", "zomma", "color"]) {
      expect(screen.getByTitle(label)).toBeTruthy();
    }
  });
});
