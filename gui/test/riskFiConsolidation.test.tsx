/**
 * FI "Book" → "Risk" consolidation (docs/FI-BOOK-CONCEPTS.md).
 *
 * The redundant Fixed-Income "Book" rail entry is removed; its position-ledger
 * surfaces are folded INTO the Fixed-Income "Risk" workspace as tabs, so under
 * Fixed Income `RiskWorkspace` is the SINGLE risk + positions + deals surface
 * (tab bar: Scenario Risk · Positions · Quotes · Deals). FX is UNCHANGED — under
 * FX Options the Risk workspace still renders ONLY the scenario grid (no tab bar).
 *
 * These render the REAL `RiskWorkspace` inside the REAL `AppProvider` driving the
 * REAL offline `MockTransport`, seeded via the genuine submit→quote→accept desk
 * flow (no stubs). `activeDomain` is seeded from the deep-link `dom` param, and the
 * moved lens bodies (Deals blotter, Positions ledger) are exercised through their
 * real tab buttons — proving nothing was lost by dropping the Book rail entry.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
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

/** Render the real Risk workspace under a given domain, backed by the transport. */
async function renderRisk(
  transport: MockTransport,
  dom: "fx_options" | "fixed_income",
): Promise<void> {
  window.history.replaceState(null, "", `/?mock&dom=${dom}`);
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <RiskWorkspace />
      </AppProvider>,
    );
  });
  await settle();
}

/** Click one of the FI Risk surface's tabs by its label. */
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

describe("FI Risk consolidation — the Fixed-Income Risk surface carries the folded-in tabs", () => {
  let transport: MockTransport;
  beforeEach(async () => {
    transport = await seededDeskTransport();
  });

  it("presents the tab bar Scenario Risk · Positions · Quotes · Deals under Fixed Income", async () => {
    await renderRisk(transport, "fixed_income");
    const bar = screen.getByRole("group", { name: "risk view" });
    const labels = within(bar)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(labels).toEqual(["Scenario Risk", "Positions", "Quotes", "Deals"]);
  });

  it("the Deals tab renders the deals blotter incl. the routed Risk Portfolio column + the booked deal", async () => {
    await renderRisk(transport, "fixed_income");
    await openTab("Deals");
    // The blotter (former Book "Deals" lens) renders verbatim — its panel, the routed
    // Risk-Portfolio column header, and the booked Jane Street OIS deal.
    expect(await screen.findByText("Received deals")).toBeInTheDocument();
    // The routed Risk-Portfolio column header lives inside the deals table region
    // (the subtitle also names "Risk Portfolio", so scope the assertion to the table).
    const dealsTable = screen.getByRole("region", { name: "Deals table" });
    expect(within(dealsTable).getByText("Risk Portfolio")).toBeInTheDocument();
    expect(await screen.findByText("Jane Street")).toBeInTheDocument();
  });

  it("the Positions tab renders the rates position ledger (the seeded OIS positions)", async () => {
    await renderRisk(transport, "fixed_income");
    await openTab("Positions");
    expect(await screen.findByText("2y OIS")).toBeInTheDocument();
    expect(screen.getByText("5y OIS")).toBeInTheDocument();
    expect(screen.getAllByText("10y OIS").length).toBeGreaterThanOrEqual(1);
  });

  it("the Quotes tab renders the shown-quotes blotter (kept — non-redundant with Deals)", async () => {
    await renderRisk(transport, "fixed_income");
    await openTab("Quotes");
    // Both quoted desk requests appear (Citadel shown-only, Jane Street shown+booked).
    expect(await screen.findByText("Citadel")).toBeInTheDocument();
    expect(screen.getByText("Jane Street")).toBeInTheDocument();
  });

  it("the default tab is Scenario Risk (the netted rates panel), NOT a ledger view", async () => {
    await renderRisk(transport, "fixed_income");
    // The FI Risk workspace's own content — the netted rates-risk panel — is default.
    expect(await screen.findByText("Netted rates risk")).toBeInTheDocument();
  });
});

describe("FI Risk consolidation — FX Options is unchanged (no tab bar; scenario grid only)", () => {
  it("under FX Options the Risk workspace renders NO folded-in tab bar", async () => {
    const transport = await seededDeskTransport();
    await renderRisk(transport, "fx_options");
    // The consolidation is FI-only: the FX Risk surface has no "risk view" tab bar
    // (Book stays a separate rail row on FX). The scenario what-if grid is what shows.
    expect(screen.queryByRole("group", { name: "risk view" })).toBeNull();
    expect(screen.queryByText("2y OIS")).toBeNull();
  });
});
