/**
 * FI "Book" → "Risk" consolidation (docs/FI-BOOK-CONCEPTS.md), post-flatten.
 *
 * The redundant Fixed-Income "Book" rail entry is removed; its position-ledger
 * surfaces (Positions · Quotes · Client blotter) are now TOP-LEVEL tabs of the
 * consolidated "Risk" panel ({@link RiskDashboardWorkspace}) — previously they were
 * nested a level deeper inside a "Scenario" tab that composed the cross-asset
 * `RiskWorkspace` at its FI rates lens. That "Scenario" tab AND its netted rates
 * scenario-risk surface are dropped; the ledger views are promoted to siblings of
 * Dashboard / Portfolios / Routing / Acceptance (the executed-hedge ledger split into
 * its own "Hedge blotter" sibling) and composed verbatim.
 *
 * These render the REAL workspaces inside the REAL `AppProvider` driving the REAL
 * offline `MockTransport`, seeded via the genuine submit→quote→accept desk flow (no
 * stubs). The moved lens bodies (Client-deals blotter, Positions ledger, Quotes blotter) are
 * exercised through their real top-level tab buttons on the Risk panel — proving
 * nothing was lost by dropping the Book rail entry and flattening the nesting.
 *
 * `RiskWorkspace` itself is now purely the cross-asset SCENARIO grid: under FX Options
 * the spot×vol what-if grid, and under Fixed Income (reachable only via a direct
 * `?view=risk` deep-link) just the netted rates-risk panel — with NO folded-in ledger
 * tab bar.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { RiskDashboardWorkspace } from "../src/workspaces/RiskDashboardWorkspace";
import { RiskWorkspace } from "../src/workspaces/RiskWorkspace";
import { createMockTransport } from "../src/data/mockSource";

type MockTransport = ReturnType<typeof createMockTransport>;

/** Flush the mount + refresh effects so the async lens loads settle. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

/**
 * A seeded desk transport driven through the GENUINE lifecycle: Citadel's 5y RFQ is
 * quoted (→ a shown QUOTED quote) and Jane Street's 10y IOI is quoted-then-accepted
 * (→ a booked deal + a booked 10y rates position). Every row is OIS (fixed_income).
 */
async function seededDeskTransport(): Promise<MockTransport> {
  const t = createMockTransport();
  const reqs = (await t.listDeskRequests({})).requests;
  const citadel = reqs.find((r) => r.counterparty === "Citadel" && r.state === "PENDING");
  const jane = reqs.find((r) => r.counterparty === "Jane Street" && r.state === "PENDING");
  if (!citadel || !jane) throw new Error("expected the two seeded PENDING desk requests");
  await t.respondDeskRequest({
    requestId: citadel.requestId,
    response: {
      kind: "quote",
      quote: { price: 0.0415, notional: 50_000_000, validForMs: 30_000, trader: "Robin" },
    },
  });
  await t.respondDeskRequest({
    requestId: jane.requestId,
    response: {
      kind: "quote",
      quote: { price: 0.042, notional: 40_000_000, validForMs: 30_000, trader: "Sam" },
    },
  });
  await t.acceptDeskQuote({ requestId: jane.requestId });
  return t;
}

/**
 * Render the Fixed-Income LEDGER panel ("Book"), backed by the transport. These are the
 * read-side blotters — they host under Fixed Income, NOT under the top-level Risk tab
 * (which keeps only the management views: dashboard / portfolios / routing / acceptance).
 */
async function renderRiskPanel(transport: MockTransport): Promise<void> {
  window.history.replaceState(null, "", "/?mock&dom=fixed_income");
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <RiskDashboardWorkspace variant="ledgers" />
      </AppProvider>,
    );
  });
  await settle();
}

/** Render the standalone cross-asset Risk (scenario) workspace under a given domain. */
async function renderRisk(
  transport: MockTransport,
  dom: "fx_options" | "fixed_income",
): Promise<void> {
  window.history.replaceState(null, "", `/?mock&dom=${dom}&view=risk`);
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <RiskWorkspace />
      </AppProvider>,
    );
  });
  await settle();
}

/** Click one of the Risk panel's top-level tabs by its label. */
async function openTab(name: string): Promise<void> {
  const bar = screen.getByRole("group", { name: "risk view" });
  await act(async () => {
    fireEvent.click(within(bar).getByRole("button", { name }));
  });
  await settle();
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

describe("FI Risk consolidation — the Risk panel carries the folded-in ledger tabs (top-level)", () => {
  let transport: MockTransport;
  beforeEach(async () => {
    transport = await seededDeskTransport();
  });

  it("promotes Positions · Quotes · Client blotter to top-level tabs (no 'Scenario Risk' sub-view)", async () => {
    await renderRiskPanel(transport);
    const bar = screen.getByRole("group", { name: "risk view" });
    const labels = within(bar)
      .getAllByRole("button")
      .map((b) => b.textContent);
    // The ledger views are now siblings of the management tabs — the client-deals
    // blotter surfaces as "Client blotter", with the executed-hedge ledger split into
    // its own "Hedge blotter" sibling …
    expect(labels).toEqual(
      expect.arrayContaining(["Positions", "Quotes", "Client blotter", "Hedge blotter"]),
    );
    // … the old single "Deals" tab is gone (split into Client/Hedge blotters) …
    expect(labels).not.toContain("Deals");
    // … and the old nested "Scenario Risk" sub-view is gone.
    expect(labels).not.toContain("Scenario Risk");
    expect(within(bar).queryByRole("button", { name: "Scenario" })).toBeNull();
  });

  it("the Client blotter tab renders the deals blotter incl. the routed Risk Portfolio column, the booked deal, and a BUY/SELL badge", async () => {
    await renderRiskPanel(transport);
    await openTab("Client blotter");
    // The blotter (former Book "Deals" lens, forced to the client lens) renders verbatim
    // — its panel, the routed
    // Risk-Portfolio column header, and the booked Jane Street OIS deal.
    expect(await screen.findByText("Received deals")).toBeInTheDocument();
    // Anchored on the TABLE itself, not on the scrollport's region landmark: the
    // scrollport is only exposed as a focusable `region` while it GENUINELY
    // overflows (useScrollableRegion), and jsdom computes no layout so it never
    // does here. The old hardcoded `role="region"` was present unconditionally,
    // which was itself the bug — a non-overflowing table became a dead tab stop.
    const dealsTable = screen.getByRole("table", { name: "Deals" });
    expect(within(dealsTable).getByText("Risk Portfolio")).toBeInTheDocument();
    expect(await screen.findByText("Jane Street")).toBeInTheDocument();
    // Every row carries a primary BUY / SELL indicator (mapped from pay/receive fixed).
    expect(within(dealsTable).getAllByText(/^(BUY|SELL)$/).length).toBeGreaterThanOrEqual(1);
  });

  it("the Positions tab renders the rates position ledger (the seeded OIS positions)", async () => {
    await renderRiskPanel(transport);
    await openTab("Positions");
    expect(await screen.findByText("2y OIS")).toBeInTheDocument();
    expect(screen.getByText("5y OIS")).toBeInTheDocument();
    expect(screen.getAllByText("10y OIS").length).toBeGreaterThanOrEqual(1);
  });

  it("the Quotes tab renders the shown-quotes blotter (kept — non-redundant with Deals)", async () => {
    await renderRiskPanel(transport);
    await openTab("Quotes");
    // Both quoted desk requests appear (Citadel shown-only, Jane Street shown+booked).
    expect(await screen.findByText("Citadel")).toBeInTheDocument();
    expect(screen.getByText("Jane Street")).toBeInTheDocument();
  });
});

describe("FI Risk consolidation — RiskWorkspace is now purely the scenario grid", () => {
  it("under Fixed Income the Risk workspace renders ONLY the netted rates risk (no ledger tab bar, no scenario heatmap)", async () => {
    const transport = await seededDeskTransport();
    await renderRisk(transport, "fixed_income");
    // The rates lens shows just the netted rates-risk panel — the FI scenario-risk
    // surface the standalone `risk` row retains — with no folded-in ledger tab bar.
    expect(await screen.findByRole("heading", { name: "Netted rates risk" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "risk view" })).toBeNull();
    expect(screen.queryByText("2y OIS")).toBeNull();
    expect(screen.queryByLabelText(/scenario heatmap/i)).toBeNull();
  });

  it("under FX Options the Risk workspace renders the scenario grid (no tab bar, no OIS)", async () => {
    const transport = await seededDeskTransport();
    await renderRisk(transport, "fx_options");
    expect(screen.queryByRole("group", { name: "risk view" })).toBeNull();
    expect(screen.queryByText("2y OIS")).toBeNull();
    expect(await screen.findByLabelText(/scenario heatmap/i)).toBeInTheDocument();
  });
});
