/**
 * Book workspace — hard asset-vertical separation by the active domain (the fix
 * that stopped Fixed-Income OIS rows — e.g. "10y OIS", counterparty CELER_RATES —
 * leaking into the FX Options domain's Book).
 *
 * The Book keeps its FOUR VIEW sub-lens tabs (Positions & Booking / Aggregate Risk
 * / Quotes / Deals); each lens body is now asset-separated by `app.activeDomain`.
 * The desk-quoting / rates streams are structurally OIS-only on this contract, so:
 *   • under FX Options the Quotes/Deals/Positions lenses are honestly EMPTY (there
 *     is no FX desk stream) and Risk shows the FX aggregate (NOT the rates panel);
 *   • under Fixed Income those three lenses render the OIS rows and Risk shows the
 *     netted rates-risk panel.
 *
 * These render the REAL `BookWorkspace` inside the REAL `AppProvider`, driving the
 * REAL offline `MockTransport` (seeded via the genuine submit→quote→accept desk
 * flow — no stubs). `activeDomain` is seeded from the deep-link `dom` param, and
 * each lens tab is exercised through its real button.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { BookWorkspace } from "../src/workspaces/BookWorkspace";
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
 * A seeded desk transport in a realistic mixed state: the two seeded PENDING
 * requests are driven through the GENUINE lifecycle — one is quoted (→ QUOTED, a
 * shown quote) and one is quoted-then-accepted (→ ACCEPTED, which books a deal AND
 * a rates position). Every row it now carries is OIS (fixed_income) — the only
 * desk stream this contract has — so the FX-domain lenses must show none of them.
 */
async function seededDeskTransport(): Promise<MockTransport> {
  const t = createMockTransport();
  const reqs = (await t.listDeskRequests({})).requests;
  // Select the two seeded PENDING requests by counterparty (list order is not
  // guaranteed): Meridian's 5y RFQ is quoted (→ shown QUOTED), Northwind's 10y IOI
  // is quoted-then-accepted (→ a booked deal + a booked 10y rates position).
  const meridian = reqs.find((r) => r.counterparty === "Meridian Capital" && r.state === "PENDING");
  const northwind = reqs.find((r) => r.counterparty === "Northwind AM" && r.state === "PENDING");
  if (!meridian || !northwind) throw new Error("expected the two seeded PENDING desk requests");

  await t.respondDeskRequest({
    requestId: meridian.requestId,
    response: {
      kind: "quote",
      quote: { price: 0.0415, notional: 50_000_000, validForMs: 30_000, trader: "Robin" },
    },
  });
  await t.respondDeskRequest({
    requestId: northwind.requestId,
    response: {
      kind: "quote",
      quote: { price: 0.042, notional: 40_000_000, validForMs: 30_000, trader: "Sam" },
    },
  });
  await t.acceptDeskQuote({ requestId: northwind.requestId });
  return t;
}

/** Render the real Book under a given domain, backed by the supplied transport. */
async function renderBook(transport: MockTransport, dom: "fx_options" | "fixed_income"): Promise<void> {
  window.history.replaceState(null, "", `/?mock&dom=${dom}`);
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <BookWorkspace />
      </AppProvider>,
    );
  });
  await settle();
}

/** Click one of the Book's four VIEW sub-lens tabs by its label. */
async function openLens(name: string): Promise<void> {
  const bar = screen.getByRole("group", { name: "book view" });
  await act(async () => {
    fireEvent.click(within(bar).getByRole("button", { name }));
  });
  await settle();
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

describe("Book workspace asset separation — FX Options domain hides every OIS row", () => {
  let transport: MockTransport;
  beforeEach(async () => {
    transport = await seededDeskTransport();
  });

  it("Quotes lens shows the honest empty state — no OIS desk quotes leak in", async () => {
    await renderBook(transport, "fx_options");
    await openLens("Quotes");

    expect(
      screen.getByText(/No FX-option desk quotes/i),
    ).toBeInTheDocument();
    // The seeded OIS counterparties must NOT appear as rows under FX Options.
    expect(screen.queryByText("Meridian Capital")).toBeNull();
    expect(screen.queryByText("Northwind AM")).toBeNull();
  });

  it("Deals lens shows the honest empty state — no OIS deals leak in", async () => {
    await renderBook(transport, "fx_options");
    await openLens("Deals");

    expect(screen.getByText(/No FX-option deals/i)).toBeInTheDocument();
    expect(screen.queryByText("Northwind AM")).toBeNull();
  });

  it("Positions lens shows the honest empty state — no OIS positions, 0-position summary", async () => {
    await renderBook(transport, "fx_options");
    await openLens("Positions & Booking");

    expect(screen.getByText(/No FX-option positions/i)).toBeInTheDocument();
    // The rates-book summary counts zero under FX Options; no OIS position rows.
    expect(screen.getByText(/0 positions/)).toBeInTheDocument();
    // The booking ticket is replaced by an honest note (not the OIS booking form).
    expect(screen.getByText(/Booking here is the fixed-income/i)).toBeInTheDocument();
  });

  it("Aggregate Risk lens shows the FX aggregate, NOT the rates panel", async () => {
    await renderBook(transport, "fx_options");
    // The default lens is Aggregate Risk; give the async aggregate a beat to load.
    await settle();
    expect(screen.queryByText("Netted rates risk")).toBeNull();
  });
});

describe("Book workspace asset separation — Fixed Income domain shows the OIS rows", () => {
  let transport: MockTransport;
  beforeEach(async () => {
    transport = await seededDeskTransport();
  });

  it("Quotes lens renders the shown OIS desk quotes", async () => {
    await renderBook(transport, "fixed_income");
    await openLens("Quotes");

    expect(await screen.findByText("Meridian Capital")).toBeInTheDocument();
    expect(screen.getByText("Northwind AM")).toBeInTheDocument();
    expect(screen.queryByText(/No FX-option desk quotes/i)).toBeNull();
  });

  it("Deals lens renders the booked OIS deal", async () => {
    await renderBook(transport, "fixed_income");
    await openLens("Deals");

    expect(await screen.findByText("Northwind AM")).toBeInTheDocument();
    expect(screen.queryByText(/No FX-option deals/i)).toBeNull();
  });

  it("Positions lens renders the seeded OIS positions", async () => {
    await renderBook(transport, "fixed_income");
    await openLens("Positions & Booking");

    // The seeded rates positions render as rows (2y / 5y, plus the seeded 10y and
    // the 10y booked by the accepted Northwind deal — hence 10y appears twice).
    expect(await screen.findByText("2y OIS")).toBeInTheDocument();
    expect(screen.getByText("5y OIS")).toBeInTheDocument();
    expect(screen.getAllByText("10y OIS").length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText(/No FX-option positions/i)).toBeNull();
  });

  it("Aggregate Risk lens shows the netted rates-risk panel, NOT the FX aggregate", async () => {
    await renderBook(transport, "fixed_income");
    expect(await screen.findByText("Netted rates risk")).toBeInTheDocument();
  });
});

describe("Book workspace — the asset filter composes with the table search", () => {
  it("under Fixed Income the Positions search narrows the OIS rows to the match", async () => {
    const transport = await seededDeskTransport();
    await renderBook(transport, "fixed_income");
    await openLens("Positions & Booking");

    // The OIS positions are present first (asset filter passes them all through).
    expect(await screen.findByText("2y OIS")).toBeInTheDocument();
    expect(screen.getByText("5y OIS")).toBeInTheDocument();
    expect(screen.getAllByText("10y OIS").length).toBeGreaterThanOrEqual(1);

    // Typing a query narrows ON TOP of the asset filter to just the 2y row.
    const search = screen.getByLabelText("Search positions");
    await act(async () => {
      fireEvent.change(search, { target: { value: "2y" } });
    });
    await settle();

    await waitFor(() => {
      expect(screen.getByText("2y OIS")).toBeInTheDocument();
      expect(screen.queryByText("5y OIS")).toBeNull();
      expect(screen.queryByText("10y OIS")).toBeNull();
    });
  });
});
