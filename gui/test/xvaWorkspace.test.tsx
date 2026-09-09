/**
 * XvaWorkspace — data-binding + workspace-render tests.
 *
 * The data-binding suite pins the `price_xva` wire codec (the browser JSON the
 * server decodes / the `result` object it encodes) and the offline analytic pricer
 * (`computeXvaOffline`) that reproduces the server's `compute_xva` aggregation:
 * both adjustments strictly positive on a two-sided netting set, the total identity
 * `cva − dva + fva`, and the correct monotonicity in the counterparty hazard.
 *
 * The render suite drives the REAL `XvaWorkspace` through the offline mock
 * transport (`?mock`): it prices on mount and shows the CVA/DVA/FVA figures, wires
 * the exposure fan, and surfaces an HONEST error (never a fabricated figure) when
 * the netting set is made invalid.
 */

import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { XvaWorkspace } from "../src/workspaces/XvaWorkspace";
import { priceXvaRequestToWire, xvaResultFromWire } from "../src/data/wsCodec";
import { computeXvaOffline, XvaPricingError } from "../src/data/xvaPricing";
import type { XvaPricingRequest } from "../src/data/contract";

/** A two-sided netting-set request (long call + short put) mirroring the server fixture. */
function sampleRequest(overrides: Partial<XvaPricingRequest> = {}): XvaPricingRequest {
  return {
    trades: [
      { optionType: "CALL", strike: 1.1, expiryYears: 1.0, vol: 0.12, notional: 1_000_000 },
      { optionType: "PUT", strike: 1.05, expiryYears: 1.5, vol: 0.14, notional: -1_000_000 },
    ],
    rDom: 0.03,
    rFor: 0.01,
    spot0: 1.1,
    sigma: 0.13,
    paths: 4096,
    seed: 1,
    exposureSteps: 16,
    counterparty: { pillarTimes: [], hazardRates: [0.02] },
    own: { pillarTimes: [], hazardRates: [0.015] },
    lgdCounterparty: 0.6,
    lgdOwn: 0.55,
    fundingSpread: 0.008,
    ...overrides,
  };
}

describe("price_xva wire codec (mirror of the server ws codec)", () => {
  it("encodes the request into the exact snake_case, numeric-enum body", () => {
    const wire = priceXvaRequestToWire(sampleRequest());
    const trades = wire.trades as Array<Record<string, unknown>>;
    // Option type rides as the numeric enum (CALL=0, PUT=1), not a string.
    expect(trades[0]!.option_type).toBe(0);
    expect(trades[1]!.option_type).toBe(1);
    expect(trades[0]!.strike).toBe(1.1);
    expect(trades[0]!.expiry_years).toBe(1.0);
    expect(trades[0]!.vol).toBe(0.12);
    expect(trades[1]!.notional).toBe(-1_000_000);
    // Scalar market + budget fields, snake_cased.
    expect(wire.r_dom).toBe(0.03);
    expect(wire.r_for).toBe(0.01);
    expect(wire.spot0).toBe(1.1);
    expect(wire.sigma).toBe(0.13);
    expect(wire.paths).toBe(4096);
    expect(wire.seed).toBe(1);
    expect(wire.exposure_steps).toBe(16);
    // Survival curves nested field-for-field.
    expect(wire.counterparty).toEqual({ pillar_times: [], hazard_rates: [0.02] });
    expect(wire.own).toEqual({ pillar_times: [], hazard_rates: [0.015] });
    // LGDs + funding.
    expect(wire.lgd_counterparty).toBe(0.6);
    expect(wire.lgd_own).toBe(0.55);
    expect(wire.funding_spread).toBe(0.008);
  });

  it("decodes the response frame's `result` object into an XvaResult", () => {
    const decoded = xvaResultFromWire({
      request_id: 11,
      result: { cva: 1.5, dva: 0.5, fva: 0.25, total_adjustment: 1.25 },
      correlation_id: 5,
    });
    expect(decoded).toEqual({ cva: 1.5, dva: 0.5, fva: 0.25, totalAdjustment: 1.25, buckets: [] });
  });
});

describe("computeXvaOffline — the deterministic in-app pricer", () => {
  it("prices a two-sided netting set with positive CVA & DVA and the total identity", () => {
    const r = computeXvaOffline(sampleRequest());
    expect(r.cva).toBeGreaterThan(0);
    expect(r.dva).toBeGreaterThan(0);
    // total = cva − dva + fva, to floating-point precision.
    expect(Math.abs(r.totalAdjustment - (r.cva - r.dva + r.fva))).toBeLessThan(1e-9);
  });

  it("is deterministic (same request ⇒ byte-identical result)", () => {
    expect(computeXvaOffline(sampleRequest())).toEqual(computeXvaOffline(sampleRequest()));
  });

  it("increases CVA when the counterparty hazard rises (higher default risk)", () => {
    const low = computeXvaOffline(
      sampleRequest({ counterparty: { pillarTimes: [], hazardRates: [0.01] } }),
    );
    const high = computeXvaOffline(
      sampleRequest({ counterparty: { pillarTimes: [], hazardRates: [0.06] } }),
    );
    expect(high.cva).toBeGreaterThan(low.cva);
  });

  it("rejects an empty netting set (mirrors the server refusal)", () => {
    expect(() => computeXvaOffline(sampleRequest({ trades: [] }))).toThrow(XvaPricingError);
  });
});

// ---------------------------------------------------------------------------
// workspace render — through the offline mock transport
// ---------------------------------------------------------------------------

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
});

async function renderXva(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <XvaWorkspace />
      </AppProvider>,
    );
  });
}

describe("XvaWorkspace — render, fan wiring, honest states", () => {
  it("prices on mount and shows the CVA / DVA / FVA / total figures", async () => {
    await renderXva();
    // The priced result populates all four adjustment cards with a $ figure.
    for (const label of ["Total XVA", "CVA", "DVA", "FVA"]) {
      const dt = await screen.findByText(label);
      expect(dt.parentElement?.textContent ?? "").toMatch(/\$/);
    }
  });

  it("wires the counterparty exposure fan", async () => {
    await renderXva();
    // The real XvaExposureFan renders — its live QMC profile is present.
    expect(await screen.findByText(/Counterparty exposure profile/i)).toBeInTheDocument();
    expect(screen.getByText(/Live QMC exposure profile/i)).toBeInTheDocument();
  });

  it("surfaces an honest error (no fabricated figure) when the set is made invalid", async () => {
    await renderXva();
    await screen.findByText("Total XVA");
    // Drive an invalid trade (zero vol) and recompute — the pricer refuses.
    fireEvent.change(screen.getByLabelText("trade 1 vol in percent"), {
      target: { value: "0" },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /XVA/ }));
    });
    const alert = await screen.findByRole("alert");
    expect(alert.textContent ?? "").toMatch(/vol must be/i);
    // The stale figures are cleared — never a fabricated number left on screen.
    expect(screen.queryByText("Total XVA")).not.toBeInTheDocument();
  });
});
