/**
 * FiStreamingWorkspace (W2) — FI streaming desk + RFS request panel.
 *
 * (a) Domain membership: the workspace is Fixed-Income-only — `RAIL` carries it
 *     with `assets: ["fixed_income"]`, so `workspaceDomains("fistreaming")` derives
 *     to `["fixed_income"]` and it NEVER appears under the FX tab.
 * (b) RFS request path: filling the form + clicking "Request price" calls
 *     `app.stream.subscribeRates` with a correctly-built `RatesInstrument` (we spy
 *     the underlying StreamSession) and the streamed indicative line renders.
 * (c) Execute: routes the risk trade through the REAL existing FI trade path —
 *     `app.transport.submitDeskRequest` (the desk RFQ) — with the OIS instrument,
 *     side and notional built from the form. There is NO fabricated stream execute.
 * (d) Honest gap: the desk RFQ wire is OIS-only, so Execute is present-but-disabled
 *     for the IRS/bond arms (never a faked booking).
 */

import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { FiStreamingWorkspace } from "../src/workspaces/FiStreamingWorkspace";
import { RAIL, railForDomain, workspaceDomains } from "../src/lib/commands";
import { MockTransport } from "../src/data/mockSource";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import type { CelnetTransport, StreamSession } from "../src/data/transport";
import type { RatesInstrument, SubmitDeskRequestRequest } from "../src/data/contract";

/** The USD-SOFR par rate at a whole-year pillar (the fixedRate the form strikes). */
function par(years: number): number {
  return DEFAULT_USD_SOFR_CURVE.pillars.find(
    (p) => p.tenor.kind === "years" && p.tenor.years === years,
  )!.parRate;
}

/**
 * A MockTransport instrumented so the test can observe the two real seams the
 * workspace drives: the StreamSession's `subscribeRates` (the RFS open) and the
 * transport's `submitDeskRequest` (the desk-RFQ execute). Both delegate to the real
 * mock so the streamed line still materializes and the desk call still resolves.
 */
function instrumentedTransport(): {
  transport: CelnetTransport;
  subscribeRatesSpy: ReturnType<typeof vi.fn>;
  submitDeskRequestSpy: ReturnType<typeof vi.fn>;
} {
  // A slow tick keeps the synchronous test window free of streamed updates (the
  // baseline snapshot is still emitted synchronously on subscribe, independent of it).
  const base = new MockTransport({ tickMs: 60_000 });
  const subscribeRatesSpy = vi.fn();
  const submitDeskRequestSpy = vi.fn().mockResolvedValue({
    request: { requestId: "req-test", state: "PENDING" },
  });

  const realOpen = base.openStreamSession.bind(base);
  base.openStreamSession = (): StreamSession => {
    const session = realOpen();
    const realSub = session.subscribeRates.bind(session);
    session.subscribeRates = (instrument, curveSet, label) => {
      subscribeRatesSpy(instrument, curveSet, label);
      return realSub(instrument, curveSet, label);
    };
    return session;
  };
  (base as unknown as { submitDeskRequest: unknown }).submitDeskRequest = submitDeskRequestSpy;

  return { transport: base, subscribeRatesSpy, submitDeskRequestSpy };
}

async function renderFi(transport: CelnetTransport): Promise<void> {
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <FiStreamingWorkspace />
      </AppProvider>,
    );
  });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("FI Streaming — domain membership (Fixed Income ONLY)", () => {
  it("derives to the fixed_income domain and NOT fx_options", () => {
    expect(workspaceDomains("fistreaming")).toEqual(["fixed_income"]);
    expect(workspaceDomains("fistreaming")).not.toContain("fx_options");
  });

  it("is a RAIL row served by fixed_income only", () => {
    const row = RAIL.find((r) => r.id === "fistreaming");
    expect(row).toBeDefined();
    expect(row!.assets).toEqual(["fixed_income"]);
    expect(row!.label).toBe("Streaming");
  });

  it("LEADS the Fixed Income rail (the primary FI surface) — above the shared widgets", () => {
    const fi = railForDomain("fixed_income").map((r) => r.id);
    expect(fi[0]).toBe("fistreaming");
    // Above every other FI widget (Market Data / Risk / Quoting). "book" is no
    // longer on the FI rail — the FI ledger is folded into FI "Risk" as tabs.
    for (const other of ["surface", "risk", "quoting"] as const) {
      expect(fi.indexOf("fistreaming")).toBeLessThan(fi.indexOf(other));
    }
  });

  it("Ticket is NOT in the Fixed Income rail (re-scoped to FX only)", () => {
    const fi = railForDomain("fixed_income").map((r) => r.id);
    expect(fi).not.toContain("ticket");
    expect(workspaceDomains("ticket")).toEqual(["fx_options"]);
    // Ticket still leads the FX rail (unchanged there).
    expect(railForDomain("fx_options").map((r) => r.id)[0]).toBe("ticket");
  });
});

describe("FI Streaming — RFS request path (subscribeRates)", () => {
  it("Request price calls subscribeRates with the correctly-built OIS instrument and renders the line", async () => {
    const { transport, subscribeRatesSpy } = instrumentedTransport();
    await renderFi(transport);

    // Default form: Swap · OIS, 5Y, 50M notional, Pay.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /request price/i }));
    });

    expect(subscribeRatesSpy).toHaveBeenCalledTimes(1);
    const [instrument, curveSet, label] = subscribeRatesSpy.mock.calls[0]!;
    expect(instrument).toEqual({
      kind: "ois",
      ois: { tenorYears: 5, fixedRate: par(5), notional: 50_000_000, direction: "PAY_FIXED" },
    });
    expect(curveSet).toBe(DEFAULT_USD_SOFR_CURVE);
    expect(label).toMatch(/OIS 5Y pay/);

    // The streamed indicative line renders in the live grid (its baseline snapshot
    // is emitted synchronously by the mock, so the row is on frame 0).
    const table = screen.getByRole("table", { name: /fixed-income streaming lines/i });
    expect(within(table).getByText(/OIS 5Y pay/)).toBeInTheDocument();
  });

  it("builds an IRS RatesInstrument when the IRS arm is selected", async () => {
    const { transport, subscribeRatesSpy } = instrumentedTransport();
    await renderFi(transport);

    // The sidebar RFS form's short arm badges (OIS/IRS/FRA/BOND) refine the request.
    const rfsForm = screen.getByRole("group", { name: "instrument type" });
    fireEvent.click(within(rfsForm).getByRole("button", { name: "IRS" }));
    fireEvent.click(screen.getByRole("button", { name: "Receive" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /request price/i }));
    });

    const [instrument] = subscribeRatesSpy.mock.calls[0]!;
    const irs = instrument as Extract<RatesInstrument, { kind: "irs" }>;
    expect(irs.kind).toBe("irs");
    expect(irs.irs.tenorYears).toBe(5);
    expect(irs.irs.direction).toBe("RECEIVE_FIXED");
    expect(irs.irs.notional).toBe(50_000_000);
  });
});

describe("FI Streaming — instrument selector (the four registry families)", () => {
  it("lists the four FI families with their product-registry labels", async () => {
    const { transport } = instrumentedTransport();
    await renderFi(transport);
    const selector = screen.getByRole("group", { name: "stream instrument family" });
    for (const label of [
      "OIS (SOFR swap)",
      "IRS (fixed vs float)",
      "FRA (forward rate)",
      "Bond (cash)",
    ]) {
      expect(within(selector).getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("selecting a family drives the stream subscription for that family (FRA)", async () => {
    const { transport, subscribeRatesSpy } = instrumentedTransport();
    await renderFi(transport);
    const selector = screen.getByRole("group", { name: "stream instrument family" });
    await act(async () => {
      fireEvent.click(within(selector).getByRole("button", { name: "FRA (forward rate)" }));
    });
    // The selector alone opened the live line (streaming-first primary gesture).
    expect(subscribeRatesSpy).toHaveBeenCalledTimes(1);
    const [instrument, curveSet] = subscribeRatesSpy.mock.calls[0]!;
    const fra = instrument as Extract<RatesInstrument, { kind: "fra" }>;
    expect(fra.kind).toBe("fra");
    // Default tenor 5Y → the standard 3-month forward window starting there (60×63M).
    expect(fra.fra.startMonths).toBe(60);
    expect(fra.fra.endMonths).toBe(63);
    expect(curveSet).toBe(DEFAULT_USD_SOFR_CURVE);
    const table = screen.getByRole("table", { name: /fixed-income streaming lines/i });
    expect(within(table).getByText(/FRA 5Y×3M/)).toBeInTheDocument();
  });

  it("renders the dealer-style two-way rates blotter columns", async () => {
    const { transport } = instrumentedTransport();
    await renderFi(transport);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /request price/i }));
    });
    const table = screen.getByRole("table", { name: /fixed-income streaming lines/i });
    // Each dealer column maps to a REAL streamed field; Mid is the honest indicative
    // mid (the FI stream carries no two-way bid/offer or size — a flagged GAP).
    for (const col of [
      "Instrument",
      "Tenor",
      "Mid",
      "PV",
      "PV01",
      "DV01",
      "Δbp",
      "PV trend",
      "Updated",
    ]) {
      expect(within(table).getByRole("columnheader", { name: col })).toBeInTheDocument();
    }
  });
});

describe("FI Streaming — Execute routes through the real desk RFQ path", () => {
  it("Execute submits the OIS risk trade via transport.submitDeskRequest", async () => {
    const { transport, submitDeskRequestSpy } = instrumentedTransport();
    await renderFi(transport);

    // Request an OIS price (Pay fixed) so the desk-RFQ Execute is enabled.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /request price/i }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /execute \(desk rfq\)/i }));
    });

    expect(submitDeskRequestSpy).toHaveBeenCalledTimes(1);
    const req = submitDeskRequestSpy.mock.calls[0]![0] as SubmitDeskRequestRequest;
    expect(req.kind).toBe("RFQ");
    expect(req.instrument).toEqual({
      tenorYears: 5,
      fixedRate: par(5),
      notional: 50_000_000,
      direction: "PAY_FIXED",
    });
    expect(req.side).toBe("BUY"); // BUY = pay fixed (per the wire Side)
    expect(req.notional).toBe(50_000_000);
    expect(req.curveSet).toBe(DEFAULT_USD_SOFR_CURVE);
  });

  it("Execute is present-but-disabled for the IRS/bond arms (desk RFQ is OIS-only)", async () => {
    const { transport, submitDeskRequestSpy } = instrumentedTransport();
    await renderFi(transport);

    const rfsForm = screen.getByRole("group", { name: "instrument type" });
    fireEvent.click(within(rfsForm).getByRole("button", { name: "BOND" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /request price/i }));
    });

    const exec = screen.getByRole("button", { name: /execute \(desk rfq\)/i });
    expect(exec).toBeDisabled();
    expect(exec.getAttribute("title")).toMatch(/OIS swaps/i);
    // Attempting to click a disabled Execute never reaches the desk path.
    fireEvent.click(exec);
    expect(submitDeskRequestSpy).not.toHaveBeenCalled();
  });
});
