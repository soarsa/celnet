/**
 * HedgeDealsView — the executed-hedge ledger lens of the Deals blotter, driven with
 * `useApp` mocked (no server). Covers: a fired hedge renders the full picture — the LP
 * we hit (`lpWon`), the LP panel fanned to, the realised hedge price + mid + slippage,
 * and the internal/external/residual amounts; an empty trail shows the honest note;
 * and a non-`hedge` identity sees the capability note WITHOUT the read being attempted.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { HedgeProvenance } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgeDealsView } from "../src/workspaces/HedgeDealsView";

function provenance(over: Partial<HedgeProvenance> = {}): HedgeProvenance {
  return {
    hedgeId: "hedge-1",
    book: "EMEA Rates",
    instrument: "USD-OIS 10y",
    firedAt: Date.UTC(2026, 7, 7, 9, 30, 0),
    metric: "dv01",
    threshold: 100,
    netRisk: 145,
    utilization: 1.45,
    band: "red",
    policyPath: [0, 1],
    action: null,
    internalCrossed: 30_000_000,
    externalHedged: 70_000_000,
    residual: 5_000_000,
    hedgePrice: 100.262,
    midAtFire: 100.25,
    slippageBp: 1.2,
    lpWon: "JPM",
    advisory: false,
    lps: ["JPM", "Citi", "Barclays"],
    ...over,
  };
}

function makeApp(opts: { can?: boolean; rows?: HedgeProvenance[] } = {}) {
  const listHedgeProvenance = vi.fn(async () => opts.rows ?? []);
  return {
    app: {
      transport: {
        label: "in-app",
        listHedgeProvenance,
        streamHedgeIntents: vi.fn(() => () => {}),
      },
      auth: { can: () => opts.can ?? true },
    },
    listHedgeProvenance,
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("HedgeDealsView", () => {
  it("renders the full hedge picture — LP hit, price, mid, slippage, amounts", async () => {
    state.app = makeApp({ can: true, rows: [provenance()] }).app;
    render(<HedgeDealsView />);

    const row = await screen.findByTestId("hedge-deal-row-hedge-1");
    const scope = within(row);
    // Who we hedged with (the winner) + the panel we fanned to.
    expect(scope.getByTestId("hedge-lpwon-hedge-1")).toHaveTextContent("JPM");
    expect(scope.getByText("Citi")).toBeInTheDocument();
    expect(scope.getByText("Barclays")).toBeInTheDocument();
    // The realised price + mid + slippage.
    expect(scope.getByText("100.2620")).toBeInTheDocument();
    expect(scope.getByText("100.2500")).toBeInTheDocument();
    expect(scope.getByText("+1.2bp")).toBeInTheDocument();
    // The amounts (internal cross / external / residual), compact.
    expect(scope.getByText("30m")).toBeInTheDocument();
    expect(scope.getByText("70m")).toBeInTheDocument();
    expect(scope.getByText("5m")).toBeInTheDocument();
    // A live external fire.
    expect(scope.getByText("LIVE")).toBeInTheDocument();
  });

  it("reconciles a per-fill execution record to its parent deal via parent_position_id", async () => {
    state.app = makeApp({
      can: true,
      rows: [provenance({ hedgeId: "HDG-7", advisory: true, parentPositionId: 9001n })],
    }).app;
    render(<HedgeDealsView />);

    const row = await screen.findByTestId("hedge-deal-row-HDG-7");
    // The parent-deal link cell carries the `#<position_id>` reconciliation reference.
    expect(within(row).getByTestId("hedge-parent-HDG-7")).toHaveTextContent("#9001");
    // An advisory external-shed fill still renders (it is NOT hidden) — it reads ADVISORY.
    expect(within(row).getByText("ADVISORY")).toBeInTheDocument();
  });

  it("renders an em dash in the parent-deal cell for a book-level record (no parent)", async () => {
    state.app = makeApp({ can: true, rows: [provenance({ hedgeId: "HDG-8" })] }).app;
    render(<HedgeDealsView />);
    const cell = await screen.findByTestId("hedge-parent-HDG-8");
    expect(cell).toHaveTextContent("—");
  });

  it("shows the honest empty note when no hedges have fired", async () => {
    state.app = makeApp({ can: true, rows: [] }).app;
    render(<HedgeDealsView />);
    expect(await screen.findByText(/No fired hedges yet/i)).toBeInTheDocument();
  });

  it("gates a non-hedge identity WITHOUT attempting the read", async () => {
    const built = makeApp({ can: false });
    state.app = built.app;
    render(<HedgeDealsView />);
    expect(screen.getByText(/requires the/i)).toBeInTheDocument();
    // The gated view never calls the server-gated read.
    expect(built.listHedgeProvenance).not.toHaveBeenCalled();
  });
});
