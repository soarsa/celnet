/**
 * AggregatedBookWorkspace — the compacted price-tile face, the per-tile "Details"
 * popover, and the per-user security-selection preference.
 *
 * The live composite + reference-data hooks are mocked to a DETERMINISTIC baseline
 * (no stream timing / rAF), so these assertions are stable; the app context is a
 * light view-only fixture. The REAL SecurityDetailsPopover + SecuritySelectionControl
 * + SettingsProvider run, so the popover, the filter, and the localStorage
 * persistence are all exercised end-to-end. (The live Playwright + axe pass is the
 * separate real gate.)
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type {
  AggregatedInstrument,
  InstrumentDef,
} from "../src/data/contract";

const hoisted = vi.hoisted(() => ({
  compositeState: null as unknown,
  refInstruments: [] as unknown[],
}));

const appState: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => appState.app }));
vi.mock("../src/hooks/useAggregatedBook", () => ({
  useAggregatedBook: () => hoisted.compositeState,
}));
vi.mock("../src/hooks/useReferenceData", () => ({
  useReferenceData: () => ({
    instruments: hoisted.refInstruments,
    isLoading: false,
    error: null,
    refetch: async () => {},
    createInstrument: async () => ({}),
    updateInstrument: async () => ({}),
    deleteInstrument: async () => true,
  }),
}));

import { SettingsProvider } from "../src/settings/SettingsProvider";
import { AggregatedBookWorkspace } from "../src/workspaces/AggregatedBookWorkspace";

/** A composite line fixture. */
function line(
  instrumentId: string,
  displayName: string,
  isin: string,
  cusip: string,
): AggregatedInstrument {
  return {
    instrumentId,
    displayName,
    isin,
    cusip,
    bestBid: 99.5,
    bestOffer: 99.7,
    bidSize: 3_000_000,
    offerSize: 3_000_000,
    confidence: 0.88,
    contributions: [
      { lpName: "LP-1", bid: 99.5, offer: 99.72, stale: false },
      { lpName: "LP-2", bid: 99.48, offer: 99.7, stale: false },
    ],
  };
}

/** A bond reference-data definition matching a composite line. */
function bondDef(
  instrumentId: string,
  name: string,
  isin: string,
  cusip: string,
  couponRate: number,
): InstrumentDef {
  return {
    instrumentId,
    name,
    description: "",
    currency: "USD",
    externalIds: [
      { scheme: "isin", value: isin },
      { scheme: "cusip", value: cusip },
    ],
    subAssetType: "",
    region: "",
    family: "bond",
    bond: {
      issuer: "US Treasury",
      couponRate,
      couponType: "fixed",
      couponFrequency: "semi_annual",
      dayCount: "act_act",
      maturityDate: { year: 2035, month: 2, day: 15 },
      redemption: 100,
      calendars: ["united_states"],
    },
  };
}

const LINE_2Y = line("91282CJL6", "UST 2Y 4.25%", "US91282CJL63", "91282CJL6");
const LINE_10Y = line("91282CJP7", "UST 10Y 4.375%", "US91282CJP77", "91282CJP7");

function makeApp() {
  return {
    transport: {
      listAggregatedBooks: vi.fn(async () => [
        { id: "ust", name: "US Treasuries", enabled: true, memberConnectionIds: ["LP-1", "LP-2"] },
      ]),
      listFixConnections: vi.fn(async () => []),
    },
    auth: { user: { id: "u", email: "trader@celnet.com" }, isAdmin: false, can: () => false },
    setSignInOpen: vi.fn(),
  };
}

function renderWorkspace() {
  return render(
    <SettingsProvider>
      <AggregatedBookWorkspace />
    </SettingsProvider>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  vi.clearAllMocks();
  appState.app = makeApp();
  hoisted.compositeState = {
    bookId: "ust",
    baselined: true,
    sequence: 1n,
    instruments: [LINE_2Y, LINE_10Y],
    epochNanos: 1_700_000_000_000_000_000n,
    gaps: 0,
  };
  hoisted.refInstruments = [
    bondDef("91282CJL6", "UST 2Y 4.25% note", "US91282CJL63", "91282CJL6", 4.25),
    bondDef("91282CJP7", "UST 10Y 4.375% note", "US91282CJP77", "91282CJP7", 4.375),
  ];
});
afterEach(() => cleanup());

describe("AggregatedBookWorkspace — compact tile", () => {
  it("keeps the security NAME on the face but moves the static terms off it", async () => {
    renderWorkspace();
    // The name stays on the tile face.
    expect(await screen.findByText("UST 2Y 4.25%")).toBeInTheDocument();
    // The ISIN string and the term labels are NOT inline on the face anymore.
    expect(screen.queryByText(/US91282CJL63/)).toBeNull();
    expect(screen.queryByText("Issuer")).toBeNull();
    expect(screen.queryByText("Day count")).toBeNull();
    // Each tile offers a Details affordance instead.
    expect(
      screen.getByRole("button", { name: /Security details for UST 2Y 4\.25%/ }),
    ).toBeInTheDocument();
  });
});

describe("AggregatedBookWorkspace — Details popover", () => {
  it("opens a labelled dialog with the full terms, and closes on Escape", async () => {
    renderWorkspace();
    const trigger = await screen.findByRole("button", {
      name: /Security details for UST 2Y 4\.25%/,
    });

    fireEvent.click(trigger);
    const dialog = await screen.findByRole("dialog", { name: /UST 2Y 4\.25%/ });
    // The moved terms appear inside the popover.
    expect(within(dialog).getByText("US91282CJL63")).toBeInTheDocument(); // ISIN
    expect(within(dialog).getByText("91282CJL6")).toBeInTheDocument(); // CUSIP
    expect(within(dialog).getByText("Issuer")).toBeInTheDocument();
    expect(within(dialog).getByText("US Treasury")).toBeInTheDocument();
    expect(within(dialog).getByText("4.25%")).toBeInTheDocument(); // coupon
    expect(within(dialog).getByText("15 Feb 2035")).toBeInTheDocument(); // maturity

    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });
});

describe("AggregatedBookWorkspace — security-selection preference", () => {
  it("filters the grid to the ticked security and persists the choice", async () => {
    renderWorkspace();
    // Both securities show by default (empty selection ⇒ show all).
    expect(await screen.findByText("UST 2Y 4.25%")).toBeInTheDocument();
    expect(screen.getByText("UST 10Y 4.375%")).toBeInTheDocument();

    // Open the header "Securities" picker and tick only the 10Y.
    fireEvent.click(screen.getByRole("button", { name: /Securities/ }));
    const picker = await screen.findByRole("dialog", { name: /choose which securities/i });
    fireEvent.click(within(picker).getByRole("checkbox", { name: /UST 10Y/ }));

    // The grid now renders only the 10Y line; the 2Y is filtered out.
    await waitFor(() => {
      expect(screen.queryByText("UST 2Y 4.25%")).toBeNull();
    });
    expect(screen.getByText("UST 10Y 4.375%")).toBeInTheDocument();

    // The choice is persisted (survives a reload) under the settings key.
    const stored = JSON.parse(window.localStorage.getItem("celnet.settings.v2") ?? "{}");
    expect(stored.aggBookInstrumentSelection).toEqual(["91282CJP7"]);
  });

  it("an emptied selection shows all again (never an accidentally-blank book)", async () => {
    // Seed a persisted selection of just the 2Y, then clear it in the picker.
    window.localStorage.setItem(
      "celnet.settings.v2",
      JSON.stringify({ aggBookInstrumentSelection: ["91282CJL6"] }),
    );
    renderWorkspace();

    // Only the 2Y shows initially.
    expect(await screen.findByText("UST 2Y 4.25%")).toBeInTheDocument();
    expect(screen.queryByText("UST 10Y 4.375%")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Securities/ }));
    const picker = await screen.findByRole("dialog", { name: /choose which securities/i });
    fireEvent.click(within(picker).getByRole("button", { name: /clear \(show all\)/i }));

    await waitFor(() => {
      expect(screen.getByText("UST 10Y 4.375%")).toBeInTheDocument();
    });
    expect(screen.getByText("UST 2Y 4.25%")).toBeInTheDocument();
  });
});
