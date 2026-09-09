/**
 * Per-deal internalise / auto-hedge provenance surfacing.
 *
 * The server stamps an OPTIONAL `internalise` object on every FI lift that ran the
 * internalise evaluation (`Deal.internalise`); it is absent on every other deal.
 * These tests pin the three seams that surface it in the trader GUI:
 *
 *  (a) codec — `dealFromWire` decodes the snake_case `deal.internalise` object when
 *      present (all six fields + the `hedge_band` string → enum, unknown → green),
 *      and leaves it undefined when the key is absent/null (never fabricated);
 *  (b) DealTicket detail — the "Internalise & auto-hedge" section renders the
 *      decision, sign-aware edge, tolerance and DV01 split for a deal that carries
 *      it, and is entirely ABSENT for a deal that does not;
 *  (c) DealsBlotter row — an internalised-green fill shows the "Internalised" badge,
 *      a below-tolerance back-to-back shows "B2B" with the losing treatment, and a
 *      deal WITHOUT provenance renders unchanged (a plain "—", no badge).
 *
 * All three run against the REAL modules — the real codec, the real DealTicket, and
 * the real DealsBlotterWorkspace under the real AppProvider + offline MockTransport.
 */

import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { DealsBlotterWorkspace } from "../src/workspaces/DealsBlotterWorkspace";
import { DealTicket } from "../src/workspaces/DealTicket";
import { createMockTransport } from "../src/data/mockSource";
import { dealFromWire } from "../src/data/wsCodec";
import type { CelnetTransport } from "../src/data/transport";
import type { Deal, ListDealsResponse } from "../src/data/contract";

/** A minimal wire Deal, optionally carrying an `internalise` object. */
function wireDeal(internalise?: Record<string, unknown>): Record<string, unknown> {
  const base: Record<string, unknown> = {
    deal_id: "deal-1",
    request_id: "req-1",
    kind: 1,
    counterparty: "Citadel",
    desk: "g10-rates",
    instrument: { ois: { tenor_years: 5, fixed_rate: 0.04, notional: 1e7, side: 0 } },
    curve_set: { currency: "USD", reference_date: { year: 2026, month: 6, day: 26 }, ois_pillars: [] },
    side: 0,
    notional: 1e7,
    price: 0.0405,
    executed_at_nanos: 1_700_000_000_000_000_000n,
    trader: "Sam",
    position_id: 7,
  };
  if (internalise !== undefined) base["internalise"] = internalise;
  return base;
}

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  // UNMOUNT rather than wiping innerHTML. Blanking the body tears the DOM out from under
  // React, so the next unmount of anything portalled (the ticket is a modal) throws "the
  // node to be removed is not a child of this node".
  cleanup();
});

describe("dealFromWire — optional internalise provenance", () => {
  it("decodes the internalise object (fields + hedge_band → enum) when present", () => {
    const deal = dealFromWire(
      wireDeal({
        internalised: true,
        internal_dv01: 8200,
        external_dv01: 0,
        edge_bps: 1.85,
        within_tolerance: true,
        hedge_band: "green",
      }),
    );
    expect(deal.internalise).toBeDefined();
    expect(deal.internalise?.internalised).toBe(true);
    expect(deal.internalise?.internalDv01).toBe(8200);
    expect(deal.internalise?.externalDv01).toBe(0);
    expect(deal.internalise?.edgeBps).toBeCloseTo(1.85, 10);
    expect(deal.internalise?.withinTolerance).toBe(true);
    expect(deal.internalise?.hedgeBand).toBe("green");
  });

  it("decodes a losing back-to-back (external, negative edge, off-tolerance, red band)", () => {
    const deal = dealFromWire(
      wireDeal({
        internalised: false,
        internal_dv01: 1000,
        external_dv01: 18000,
        edge_bps: -0.58,
        within_tolerance: false,
        hedge_band: "red",
      }),
    );
    expect(deal.internalise?.internalised).toBe(false);
    expect(deal.internalise?.edgeBps).toBeLessThan(0);
    expect(deal.internalise?.withinTolerance).toBe(false);
    expect(deal.internalise?.hedgeBand).toBe("red");
  });

  it("falls back to the green band for an unknown hedge_band string", () => {
    const deal = dealFromWire(
      wireDeal({
        internalised: true,
        internal_dv01: 1,
        external_dv01: 0,
        edge_bps: 0,
        within_tolerance: true,
        hedge_band: "chartreuse",
      }),
    );
    expect(deal.internalise?.hedgeBand).toBe("green");
  });

  it("leaves internalise undefined when the key is absent — never fabricated", () => {
    expect(dealFromWire(wireDeal()).internalise).toBeUndefined();
  });

  it("leaves internalise undefined when the key is present-with-null", () => {
    const wire = wireDeal();
    wire["internalise"] = null;
    expect(dealFromWire(wire).internalise).toBeUndefined();
  });
});

/** A hand-built FI deal carrying the given internalise provenance (or none). */
function fiDeal(over: Partial<Deal>, internalise?: Deal["internalise"]): Deal {
  const d: Deal = {
    dealId: "deal-x",
    requestId: "req-x",
    kind: "RFQ",
    counterparty: "Citadel",
    desk: "g10-rates",
    productKind: "OIS",
    instrument: { tenorYears: 5, fixedRate: 0.04, notional: 1e7, direction: "PAY_FIXED" },
    curveSet: { currency: "USD", referenceDate: { year: 2026, month: 6, day: 26 }, pillars: [] },
    side: "BUY",
    notional: 1e7,
    price: 0.0405,
    executedAtNanos: 1_700_000_000_000_000_000n,
    trader: "Sam",
    ...over,
  };
  if (internalise !== undefined) d.internalise = internalise;
  return d;
}

describe("DealTicket — internalise detail section", () => {
  it("renders the decision, positive edge, tolerance and DV01 split for an internalised green fill", () => {
    render(
      <DealTicket
        deal={fiDeal({}, {
          internalised: true,
          internalDv01: 60_000,
          externalDv01: 0,
          edgeBps: 1.85,
          withinTolerance: true,
          hedgeBand: "green",
        })}
        onClose={() => {}}
      />,
    );
    const section = screen.getByRole("heading", { name: /Internalise & auto-hedge/i }).closest("section");
    expect(section).not.toBeNull();
    const scope = within(section as HTMLElement);
    expect(scope.getByText("Internalised")).toBeInTheDocument();
    expect(scope.getByText("+1.85 bp")).toBeInTheDocument();
    expect(scope.getByText("Green")).toBeInTheDocument();
    // Within tolerance → "Yes" (no losing marker).
    expect(scope.getByText("Yes")).toBeInTheDocument();
    expect(scope.queryByText(/No — losing/)).toBeNull();
  });

  it("renders the losing/B2B treatment for a below-tolerance external fill", () => {
    render(
      <DealTicket
        deal={fiDeal({}, {
          internalised: false,
          internalDv01: 5_000,
          externalDv01: 55_000,
          edgeBps: -0.58,
          withinTolerance: false,
          hedgeBand: "red",
        })}
        onClose={() => {}}
      />,
    );
    const section = screen.getByRole("heading", { name: /Internalise & auto-hedge/i }).closest("section");
    const scope = within(section as HTMLElement);
    expect(scope.getByText("B2B")).toBeInTheDocument();
    expect(scope.getByText("−0.58 bp")).toBeInTheDocument();
    expect(scope.getByText(/No — losing/)).toBeInTheDocument();
    expect(scope.getByText("Red")).toBeInTheDocument();
  });

  it("omits the internalise section entirely for a deal without provenance", () => {
    render(<DealTicket deal={fiDeal({})} onClose={() => {}} />);
    expect(screen.queryByRole("heading", { name: /Internalise & auto-hedge/i })).toBeNull();
    // The rest of the ticket is unchanged — the economics section still renders.
    expect(screen.getByRole("heading", { name: /Economics/i })).toBeInTheDocument();
  });
});

/** A transport whose deals blotter returns a fixed FI mix: green, losing-B2B, none. */
function blotterTransport(): CelnetTransport {
  const t = createMockTransport();
  const green = fiDeal(
    { dealId: "deal-green", counterparty: "Millennium Capital" },
    {
      internalised: true,
      internalDv01: 60_000,
      externalDv01: 0,
      edgeBps: 1.85,
      withinTolerance: true,
      hedgeBand: "green",
    },
  );
  const losing = fiDeal(
    { dealId: "deal-losing", counterparty: "Balyasny" },
    {
      internalised: false,
      internalDv01: 5_000,
      externalDv01: 55_000,
      edgeBps: -0.58,
      withinTolerance: false,
      hedgeBand: "red",
    },
  );
  const plain = fiDeal({ dealId: "deal-plain", counterparty: "Point72" });
  const deals: Deal[] = [green, losing, plain];
  t.listDeals = async (): Promise<ListDealsResponse> => ({ deals });
  return t;
}

describe("DealsBlotterWorkspace — internalise row badge (FI domain)", () => {
  it("shows the Internalised badge, the B2B losing badge, and an unchanged no-provenance row", async () => {
    window.history.replaceState(null, "", "/?mock&dom=fixed_income");
    await act(async () => {
      render(
        <AppProvider transport={blotterTransport()}>
          <DealsBlotterWorkspace />
        </AppProvider>,
      );
    });
    await settle();

    // The internalised-green fill row carries the "Internalised" badge.
    const greenRow = screen.getByText("Millennium Capital").closest("tr") as HTMLElement;
    expect(within(greenRow).getByText("Internalised")).toBeInTheDocument();

    // The below-tolerance external fill row carries the "B2B" badge, marked losing.
    const losingRow = screen.getByText("Balyasny").closest("tr") as HTMLElement;
    const b2b = within(losingRow).getByText("B2B");
    expect(b2b).toBeInTheDocument();
    expect(b2b.closest("[data-losing]")?.getAttribute("data-losing")).toBe("true");
    expect(b2b.getAttribute("data-band")).toBe("red");

    // The no-provenance fill row renders unchanged — no badge, plain em dash.
    const plainRow = screen.getByText("Point72").closest("tr") as HTMLElement;
    expect(within(plainRow).queryByText("Internalised")).toBeNull();
    expect(within(plainRow).queryByText("B2B")).toBeNull();
  });
});
