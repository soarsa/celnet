import { describe, expect, it } from "vitest";

import type {
  OisInstrument,
  RatesCurveSet,
  RiskBook,
  RiskBookRisk,
  RiskRoutingGraph,
  SubmitDeskRequestRequest,
} from "../src/data/contract";
import { createMockTransport } from "../src/data/mockSource";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";

/**
 * REGRESSION — the "No enabled risk portfolios to report" feature-blocking bug.
 *
 * A risk portfolio created LIVE (via the CRUD store) must be visible to the Risk
 * Dashboard's risk source ({@link CelnetTransport.listRiskBookRisk} — the offline
 * mirror of the server's `ListRiskBookRisk` / `aggregate_enabled_risk_books`), and a
 * booked deal routed into it must SUM into that same portfolio's roll-up. The bug was
 * a disconnect between the CRUD store the user edits and the risk source the dashboard
 * reads; this pins that they are the SAME store and that routed positions land in the
 * created portfolio — so a created-and-enabled portfolio with routed fills shows its
 * net / gross / positions, never the empty state.
 *
 * The server analogue is the Rust test
 * `created_risk_portfolio_is_visible_to_the_streamed_dashboard_aggregate`
 * (`crates/celnet-server/src/services/auth.rs`).
 */

const CURVE: RatesCurveSet = DEFAULT_USD_SOFR_CURVE;

function ois(overrides: Partial<OisInstrument> = {}): OisInstrument {
  return {
    tenorYears: 5,
    fixedRate: 0.04,
    notional: 50_000_000,
    direction: "PAY_FIXED",
    ...overrides,
  };
}

function submitReq(
  overrides: Partial<SubmitDeskRequestRequest> = {},
): SubmitDeskRequestRequest {
  return {
    kind: "RFQ",
    counterparty: "Acme Capital",
    desk: "g10-rates",
    instrument: ois(),
    curveSet: CURVE,
    side: "BUY",
    notional: 50_000_000,
    ttlMs: 60_000,
    ...overrides,
  };
}

function newPortfolio(overrides: Partial<RiskBook> = {}): RiskBook {
  return {
    id: "",
    name: "MAREX FI LDN",
    parentId: null,
    deskId: null,
    description: "",
    limits: null,
    enabled: true,
    ...overrides,
  };
}

function rowFor(rows: readonly RiskBookRisk[], bookId: string): RiskBookRisk | undefined {
  return rows.find((r) => r.bookId === bookId);
}

/** Drive submit → quote → accept, which books a routed OIS deal offline. */
async function bookRoutedDeal(
  t: ReturnType<typeof createMockTransport>,
  notional: number,
  side: "BUY" | "SELL",
): Promise<void> {
  const { request } = await t.submitDeskRequest(submitReq({ notional, side }));
  await t.respondDeskRequest({
    requestId: request.requestId,
    response: {
      kind: "quote",
      quote: { price: 0.041, notional, validForMs: 60_000, trader: "Robin" },
    },
  });
  await t.acceptDeskQuote({ requestId: request.requestId });
}

describe("Risk Dashboard — a created portfolio with routed fills rolls up (mock)", () => {
  it("a freshly created enabled portfolio appears in the dashboard risk source", async () => {
    const t = createMockTransport();
    const created = await t.createRiskBook(newPortfolio());
    expect(created.id).toBe("marex-fi-ldn");
    expect(created.enabled).toBe(true);

    // The dashboard's risk source (poll) must SEE the created enabled portfolio — this
    // is exactly the row whose absence produced the empty-state bug.
    const rows = await t.listRiskBookRisk();
    expect(rowFor(rows, "marex-fi-ldn")).toBeDefined();

    // …and so must the LIVE push the dashboard prefers (subscribeRiskBookRisk). The
    // baseline snapshot is emitted synchronously on subscribe, so capture it directly.
    let firstFrame: RiskBookRisk[] | null = null;
    const teardown = t.subscribeRiskBookRisk!((frame) => {
      if (firstFrame === null) firstFrame = frame;
    });
    teardown();
    expect(firstFrame).not.toBeNull();
    expect(rowFor(firstFrame!, "marex-fi-ldn")).toBeDefined();
  });

  it("a deal routed into the created portfolio sums into its net / gross / positions", async () => {
    const t = createMockTransport();
    await t.createRiskBook(newPortfolio());

    // Route EVERY fill into the created portfolio (single book leaf as the graph entry).
    const graph: RiskRoutingGraph = {
      entry: 0,
      nodes: [{ kind: "book", id: 0, bookId: "marex-fi-ldn" }],
    };
    await t.updateRiskRoutingGraph(graph);

    const before = rowFor(await t.listRiskBookRisk(), "marex-fi-ldn");
    expect(before).toBeDefined();

    // Book a BUY (pay-fixed ⇒ +notional) 50mm OIS routed into the portfolio.
    const notional = 50_000_000;
    await bookRoutedDeal(t, notional, "BUY");

    const after = rowFor(await t.listRiskBookRisk(), "marex-fi-ldn");
    expect(after).toBeDefined();
    // The routed fill adds exactly one position and +notional net / +notional gross.
    expect(after!.positionCount - before!.positionCount).toBe(1);
    expect(after!.netNotional - before!.netNotional).toBeCloseTo(notional, 3);
    expect(after!.grossNotional - before!.grossNotional).toBeCloseTo(notional, 3);
    // A routed rates fill surfaces a DV01 on the portfolio (linear PV01 proxy, signed +).
    expect((after!.dv01 ?? 0) - (before!.dv01 ?? 0)).toBeGreaterThan(0);
  });
});
