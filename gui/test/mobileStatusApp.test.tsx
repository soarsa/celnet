/**
 * MobileStatusApp — the touch-friendly READ-ONLY status board rendered in place of the
 * dense desktop `Shell` when a trader opens the app on a phone.
 *
 * These mount the REAL board inside the REAL `AppProvider`, driving the REAL offline
 * `MockTransport` — no stubs of our own functionality. They pin the behaviours the
 * board exists for:
 *   • asset tabs (Fixed Income / FX Options) and the three read-only sub-views;
 *   • hard asset separation — the FX lenses are honestly EMPTY (the desk/risk/hedge
 *     streams are fixed-income on this contract), never fabricated rows;
 *   • READ-ONLY: no mutating affordance (no book/submit/accept/quote control) anywhere;
 *   • the "Full app ↗" escape hatch calls back to the app root, which owns the choice;
 *   • the first-open hint shows once and its dismissal persists.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { MobileClientBlotter } from "../src/app/mobile/MobileClientBlotter";
import { MobileStatusApp } from "../src/app/mobile/MobileStatusApp";
import { createMockTransport } from "../src/data/mockSource";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import type { Deal } from "../src/data/contract";

type MockTransport = ReturnType<typeof createMockTransport>;

/** localStorage key the board writes when the first-open hint is dismissed. */
const HINT_SEEN_KEY = "celnet.mobile.hintSeen";

/** Flush the mount + resource-load effects so the async sub-views settle. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

/**
 * Render the real board. `hintSeen` defaults to true so the one-time onboarding sheet
 * does not sit over the board in the tests that are not about the hint itself.
 */
async function renderBoard(
  onUseFullApp: () => void = () => {},
  { hintSeen = true }: { hintSeen?: boolean } = {},
): Promise<MockTransport> {
  if (hintSeen) window.localStorage.setItem(HINT_SEEN_KEY, "1");
  const transport = createMockTransport();
  await act(async () => {
    render(
      <AppProvider transport={transport}>
        <MobileStatusApp onUseFullApp={onUseFullApp} />
      </AppProvider>,
    );
  });
  await settle();
  return transport;
}

/** Select an asset tab by its label ("Fixed Income" / "FX Options"). */
async function openAsset(name: string): Promise<void> {
  const tabs = screen.getByRole("tablist", { name: "asset class" });
  await act(async () => {
    fireEvent.click(within(tabs).getByRole("tab", { name }));
  });
  await settle();
}

/** Select a sub-view tab by its label ("Risk" / "Hedges" / "Client flow"). */
async function openSubView(name: string): Promise<void> {
  const tabs = screen.getByRole("tablist", { name: "status view" });
  await act(async () => {
    fireEvent.click(within(tabs).getByRole("tab", { name }));
  });
  await settle();
}

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  window.localStorage.clear();
  document.body.innerHTML = "";
});

describe("MobileStatusApp — navigation surface", () => {
  it("renders both entitled asset tabs, Fixed Income first and selected", async () => {
    await renderBoard();
    const tabs = screen.getByRole("tablist", { name: "asset class" });
    const labels = within(tabs)
      .getAllByRole("tab")
      .map((t) => t.textContent);
    expect(labels).toEqual(["Fixed Income", "FX Options"]);
    expect(within(tabs).getByRole("tab", { name: "Fixed Income" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("renders the three read-only sub-views with Risk selected by default", async () => {
    await renderBoard();
    const tabs = screen.getByRole("tablist", { name: "status view" });
    const labels = within(tabs)
      .getAllByRole("tab")
      .map((t) => t.textContent);
    expect(labels).toEqual(["Risk", "Hedges", "Client flow"]);
    expect(within(tabs).getByRole("tab", { name: "Risk" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("moves the selection when another asset tab is tapped", async () => {
    await renderBoard();
    await openAsset("FX Options");
    const tabs = screen.getByRole("tablist", { name: "asset class" });
    expect(within(tabs).getByRole("tab", { name: "FX Options" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(tabs).getByRole("tab", { name: "Fixed Income" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("surfaces the connection status and the identity row", async () => {
    await renderBoard();
    expect(screen.getByRole("status", { name: /^Connection / })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
  });
});

describe("MobileStatusApp — hard asset separation (FX lenses are honestly empty)", () => {
  it("shows the honest empty risk state under FX Options — no fixed-income portfolios leak in", async () => {
    await renderBoard();
    await openAsset("FX Options");
    expect(screen.getByText("No FX Options risk portfolios.")).toBeInTheDocument();
  });

  it("shows the honest empty hedge state under FX Options — hedging runs on fixed income", async () => {
    await renderBoard();
    await openAsset("FX Options");
    await openSubView("Hedges");
    expect(
      screen.getByText(/No FX Options hedges \(hedging runs on fixed income\)\./),
    ).toBeInTheDocument();
  });

  it("shows the honest empty client-flow state under FX Options", async () => {
    await renderBoard();
    await openAsset("FX Options");
    await openSubView("Client flow");
    expect(screen.getByText("No FX Options client deals yet.")).toBeInTheDocument();
  });

  it("keeps each Fixed Income sub-view on its own honest state, never the FX copy", async () => {
    await renderBoard();
    expect(screen.queryByText(/No FX Options risk portfolios\./)).toBeNull();

    await openSubView("Hedges");
    expect(screen.queryByText(/No FX Options hedges/)).toBeNull();

    await openSubView("Client flow");
    expect(screen.queryByText(/No FX Options client deals/)).toBeNull();
  });
});

describe("MobileStatusApp — read-only guarantee", () => {
  it("exposes no mutating control in any asset × sub-view combination", async () => {
    await renderBoard();
    const mutating = /book|submit|accept|reject|quote|price|hedge now|execute|save|delete|new /i;
    for (const asset of ["Fixed Income", "FX Options"]) {
      await openAsset(asset);
      for (const view of ["Risk", "Hedges", "Client flow"]) {
        await openSubView(view);
        const offenders = screen
          .getAllByRole("button")
          .map((b) => b.getAttribute("aria-label") ?? b.textContent ?? "")
          .filter((label) => mutating.test(label));
        expect(offenders, `${asset} / ${view} exposed a mutating control`).toEqual([]);
      }
    }
  });

  it("never calls a mutating transport method while the board is driven", async () => {
    const transport = await renderBoard();
    const book = vi.spyOn(transport, "acceptDeskQuote");
    const respond = vi.spyOn(transport, "respondDeskRequest");
    await openAsset("FX Options");
    await openSubView("Hedges");
    await openSubView("Client flow");
    await openAsset("Fixed Income");
    await openSubView("Risk");
    expect(book).not.toHaveBeenCalled();
    expect(respond).not.toHaveBeenCalled();
  });
});

/**
 * A client deal that was warehoused, so its row carries the disposition badge — the
 * element that used to be absolutely positioned into the card's bottom-right corner
 * and collided with the notional stacked in the same corner.
 */
function warehousedDeal(): Deal {
  return {
    dealId: "d-overlap",
    requestId: "req-overlap",
    kind: "RFQ",
    counterparty: "Point72",
    desk: "g10-rates",
    productKind: "IRS",
    instrument: { tenorYears: 7, fixedRate: 0.04, notional: 90_000_000, direction: "PAY_FIXED" },
    curveSet: DEFAULT_USD_SOFR_CURVE,
    side: "SELL",
    notional: 90_000_000,
    price: 0.0412,
    executedAtNanos: 1n,
    trader: "T",
    internalise: {
      internalised: true,
      internalDv01: 1_200,
      externalDv01: 0,
      edgeBps: 0.4,
      withinTolerance: true,
      hedgeBand: "amber",
    },
  };
}

describe("MobileClientBlotter — the disposition badge lays out in flow (no overlap)", () => {
  it("renders the badge inside the row's meta line, beside the product descriptor", () => {
    render(
      <MobileClientBlotter
        deals={[warehousedDeal()]}
        bookNames={new Map()}
        isLoading={false}
        error={null}
        asset="fixed_income"
      />,
    );
    const badge = screen.getByText("Internalised");
    const product = screen.getByText("IRS · 7Y");
    // Same parent ⇒ the badge is a normal-flow sibling of the descriptor, NOT a
    // free-floating element overlaying the card's corner.
    expect(badge.parentElement).toBe(product.parentElement);
  });

  it("never absolutely positions the badge (the overlap regression)", () => {
    render(
      <MobileClientBlotter
        deals={[warehousedDeal()]}
        bookNames={new Map()}
        isLoading={false}
        error={null}
        asset="fixed_income"
      />,
    );
    const badge = screen.getByText("Internalised");
    expect(badge).not.toHaveAttribute("data-corner");
    // The badge and the notional must not share a positioning corner: the notional
    // lives in the right-hand column, the badge in the left-hand meta line.
    const notional = screen.getByText("90m");
    expect(notional.parentElement).not.toBe(badge.parentElement);
  });
});

describe("MobileStatusApp — the escape hatch", () => {
  it("calls back to the app root when “Full app ↗” is tapped (the root owns the choice)", async () => {
    const onUseFullApp = vi.fn();
    await renderBoard(onUseFullApp);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Full app/ }));
    });
    expect(onUseFullApp).toHaveBeenCalledTimes(1);
  });
});

describe("MobileStatusApp — the first-open hint", () => {
  it("opens the onboarding sheet on a first-ever open", async () => {
    await renderBoard(() => {}, { hintSeen: false });
    expect(screen.getByRole("dialog", { name: "help" })).toBeInTheDocument();
  });

  it("closes on Skip and persists the flag so it never nags again", async () => {
    await renderBoard(() => {}, { hintSeen: false });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    });
    expect(screen.queryByRole("dialog", { name: "help" })).toBeNull();
    expect(window.localStorage.getItem(HINT_SEEN_KEY)).toBe("1");
  });

  it("stays closed once the flag is set, and the help button still opens the full guide", async () => {
    await renderBoard();
    expect(screen.queryByRole("dialog", { name: "help" })).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "open help" }));
    });
    const sheet = screen.getByRole("dialog", { name: "help" });
    expect(within(sheet).getByText("Mobile status board")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(within(sheet).getByRole("button", { name: "Close" }));
    });
    expect(screen.queryByRole("dialog", { name: "help" })).toBeNull();
  });
});
