/**
 * BOND security identity + ESP deal-kind on the Deals blotter.
 *
 * The server now threads a bond's identity (`instrument_id` / `display_name`, the SAME
 * descriptor the Agg Book tile shows) onto a fill and gains an `ESP` deal kind (a
 * market-data streaming lift, wire tag 3). These tests pin the two GUI seams:
 *
 *  (a) codec — `dealFromWire` decodes `deal.instrument.bond.{instrument_id,display_name}`
 *      onto `Deal.{bondSecurityId,bondDisplayName}` (present only for BOND; empty
 *      `display_name` ⇒ absent, never fabricated), and decodes `kind: 3` → `"ESP"`;
 *  (b) DealsBlotter — a BOND ESP row shows the security descriptor + id in the SECURITY
 *      cell and the ESP type chip; an OIS row shows no security (a plain em dash) and
 *      its RFQ chip, all against the REAL blotter under the real AppProvider.
 */
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { DealsBlotterWorkspace } from "../src/workspaces/DealsBlotterWorkspace";
import { createMockTransport } from "../src/data/mockSource";
import { dealFromWire } from "../src/data/wsCodec";
import type { CelnetTransport } from "../src/data/transport";
import type { Deal, ListDealsResponse } from "../src/data/contract";

/** A wire BOND deal, `kind` overridable (RFQ=1/IOI=2/ESP=3), bond identity overridable. */
function wireBondDeal(
  kind: number,
  bond: Record<string, unknown>,
): Record<string, unknown> {
  return {
    deal_id: "deal-bond-1",
    request_id: "req-1",
    kind,
    counterparty: "Citadel",
    desk: "g10-rates",
    instrument: {
      bond: {
        coupon_rate: 0.04375,
        coupon_frequency: 1,
        day_count: 2,
        maturity_date: { year: 2036, month: 5, day: 15 },
        redemption: 100,
        side: 0,
        ...bond,
      },
    },
    curve_set: { currency: "USD", reference_date: { year: 2026, month: 6, day: 26 }, ois_pillars: [] },
    side: 0,
    notional: 6e7,
    price: 97.41,
    executed_at_nanos: 1_700_000_000_000_000_000n,
    trader: "Sam",
  };
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
  document.body.innerHTML = "";
});

describe("dealFromWire — BOND identity + ESP kind", () => {
  it("threads instrument_id + display_name onto a BOND deal and decodes kind 3 → ESP", () => {
    const deal = dealFromWire(
      wireBondDeal(3, { instrument_id: "91282CJP7", display_name: "UST 10Y 4.375%" }),
    );
    expect(deal.productKind).toBe("BOND");
    expect(deal.kind).toBe("ESP");
    expect(deal.bondSecurityId).toBe("91282CJP7");
    expect(deal.bondDisplayName).toBe("UST 10Y 4.375%");
  });

  it("leaves display_name absent (never fabricated) when the bond is outside refdata", () => {
    const deal = dealFromWire(
      wireBondDeal(1, { instrument_id: "999999999", display_name: "" }),
    );
    expect(deal.bondSecurityId).toBe("999999999");
    expect(deal.bondDisplayName).toBeUndefined();
  });

  it("carries no bond identity for an OIS fill (RFQ) — the fields stay undefined", () => {
    const deal = dealFromWire({
      deal_id: "deal-ois-1",
      request_id: "req-2",
      kind: 1,
      counterparty: "Nordea",
      desk: "g10-rates",
      instrument: { ois: { tenor_years: 5, fixed_rate: 0.04, notional: 1e7, side: 0 } },
      curve_set: { currency: "USD", reference_date: { year: 2026, month: 6, day: 26 }, ois_pillars: [] },
      side: 0,
      notional: 1e7,
      price: 0.0405,
      executed_at_nanos: 1_700_000_000_000_000_000n,
      trader: "Sam",
    });
    expect(deal.productKind).toBe("OIS");
    expect(deal.kind).toBe("RFQ");
    expect(deal.bondSecurityId).toBeUndefined();
    expect(deal.bondDisplayName).toBeUndefined();
  });
});

/** A hand-built FI deal (BOND or OIS) with optional bond identity. */
function fiDeal(over: Partial<Deal>): Deal {
  return {
    dealId: "deal-x",
    requestId: "req-x",
    kind: "RFQ",
    counterparty: "Point72",
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
}

function blotterTransport(): CelnetTransport {
  const t = createMockTransport();
  const bondEsp = fiDeal({
    dealId: "deal-bond-esp",
    counterparty: "Millennium Capital",
    kind: "ESP",
    productKind: "BOND",
    instrument: { tenorYears: 10, fixedRate: 0.04375, notional: 6e7, direction: "PAY_FIXED" },
    bondSecurityId: "91282CJP7",
    bondDisplayName: "UST 10Y 4.375%",
  });
  const ois = fiDeal({ dealId: "deal-ois", counterparty: "Point72" });
  const deals: Deal[] = [bondEsp, ois];
  t.listDeals = async (): Promise<ListDealsResponse> => ({ deals });
  return t;
}

describe("DealsBlotterWorkspace — BOND security cell + ESP type chip (FI domain)", () => {
  it("shows the descriptor + id for a BOND ESP fill, and no security for an OIS RFQ fill", async () => {
    window.history.replaceState(null, "", "/?mock&dom=fixed_income");
    await act(async () => {
      render(
        <AppProvider transport={blotterTransport()}>
          <DealsBlotterWorkspace />
        </AppProvider>,
      );
    });
    await settle();

    // The BOND ESP row: the SECURITY cell shows the Agg-Book-style descriptor + the id,
    // and the TYPE chip reads ESP.
    const bondRow = screen.getByText("Millennium Capital").closest("tr") as HTMLElement;
    expect(within(bondRow).getByText("UST 10Y 4.375%")).toBeInTheDocument();
    expect(within(bondRow).getByText("91282CJP7")).toBeInTheDocument();
    expect(within(bondRow).getByText("ESP")).toBeInTheDocument();

    // The OIS RFQ row: no security identity (a plain em dash), and the RFQ chip.
    const oisRow = screen.getByText("Point72").closest("tr") as HTMLElement;
    expect(within(oisRow).queryByText("UST 10Y 4.375%")).toBeNull();
    expect(within(oisRow).getByText("RFQ")).toBeInTheDocument();
  });
});
