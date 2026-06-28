/**
 * DealerPanel (the ranked multi-dealer LP panel) — component + ticket-flow tests.
 *
 * The component suite renders the REAL `DealerPanel` against contract-shaped
 * `MultiDealerQuote` values and asserts the lane's behavioural contract:
 *   - render order == FRAME order (the server's deterministic audit order; the
 *     panel never re-sorts — ranking is surfaced by the best-bid/offer badges),
 *   - a book click emits exactly `(quoteId, lpId, side)`,
 *   - countdown/expiry honesty: a live row shows its own last-look ring; an
 *     expired row is disabled with a reason, never tradable.
 *
 * The workspace suite drives the REAL `TicketWorkspace` through the offline
 * mock transport (`?mock` — a labeled deterministic synthetic panel mirroring
 * the server's `LpPanelConfig` demo law, NOT live LP connectivity): flip the
 * RFQ mode to "LP panel", request, book the highlighted best offer, and assert
 * a fill renders carrying that dealer's `lpId`.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { DealerPanel } from "../src/components/DealerPanel";
import { TicketWorkspace } from "../src/workspaces/TicketWorkspace";
import type { DealerQuote, MultiDealerQuote } from "../src/data/contract";
import { DEFAULT_CONVENTIONS } from "../src/data/seed";
import { nowNanos } from "../src/hooks/useClock";

const NS_PER_S = 1_000_000_000n;

/** A contract-shaped dealer line (no greeks — a non-native LP's honest shape). */
function dealer(
  lpId: string,
  bid: number,
  offer: number,
  validUntilNanos: bigint,
): DealerQuote {
  return { lpId, price: { bid, offer }, resolvedStrike: 1.0921, validUntilNanos };
}

/** A contract-shaped panel over the given rows (frame order preserved as-is). */
function panelWith(
  dealers: DealerQuote[],
  winners: { bestBidLpId: string; bestOfferLpId: string },
): MultiDealerQuote {
  return {
    quoteId: 7n,
    idempotencyKey: "tkt-test",
    dealers,
    bestBidLpId: winners.bestBidLpId,
    bestOfferLpId: winners.bestOfferLpId,
    conventions: DEFAULT_CONVENTIONS,
    epochNanos: nowNanos(),
  };
}

describe("DealerPanel — render order, winners, booking, expiry", () => {
  it("renders one row per LP in FRAME order (never re-sorted client-side)", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    // A deliberately non-alphabetical frame order: the panel must keep it.
    const panel = panelWith(
      [
        dealer("SYNTH-LP-3", 0.119, 0.131, live),
        dealer("celnet-auto-pricer", 0.12, 0.13, live),
        dealer("SYNTH-LP-1", 0.1206, 0.1296, live),
      ],
      { bestBidLpId: "SYNTH-LP-1", bestOfferLpId: "SYNTH-LP-1" },
    );
    render(<DealerPanel panel={panel} onBook={vi.fn()} />);
    const table = screen.getByRole("table", { name: "multi-dealer quote panel" });
    const rows = within(table)
      .getAllByRole("rowheader")
      .map((el) => el.textContent);
    expect(rows).toEqual(["SYNTH-LP-3", "celnet-auto-pricer", "SYNTH-LP-1"]);
  });

  it("highlights the best bid and best offer rows with a textual badge", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    const panel = panelWith(
      [
        dealer("celnet-auto-pricer", 0.12, 0.13, live),
        dealer("SYNTH-LP-1", 0.1206, 0.1296, live),
        dealer("SYNTH-LP-2", 0.1182, 0.1292, live),
      ],
      { bestBidLpId: "SYNTH-LP-1", bestOfferLpId: "SYNTH-LP-2" },
    );
    render(<DealerPanel panel={panel} onBook={vi.fn()} />);
    // The winning sides carry "— best bid"/"— best offer" in the accessible name
    // and a visible "best" badge (text, never colour alone) — exactly one each.
    const bestBid = screen.getByRole("button", { name: / — best bid$/ });
    expect(bestBid).toHaveAccessibleName(/sell to SYNTH-LP-1/);
    const bestOffer = screen.getByRole("button", { name: / — best offer$/ });
    expect(bestOffer).toHaveAccessibleName(/buy from SYNTH-LP-2/);
    expect(screen.getAllByText("best")).toHaveLength(2);
  });

  it("booking a row emits exactly (quoteId, lpId, side)", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    const panel = panelWith(
      [
        dealer("celnet-auto-pricer", 0.12, 0.13, live),
        dealer("SYNTH-LP-2", 0.1182, 0.1292, live),
      ],
      { bestBidLpId: "celnet-auto-pricer", bestOfferLpId: "SYNTH-LP-2" },
    );
    const onBook = vi.fn();
    render(<DealerPanel panel={panel} onBook={onBook} />);
    // Lift SYNTH-LP-2's offer (BUY) …
    fireEvent.click(screen.getByRole("button", { name: /^buy from SYNTH-LP-2/ }));
    expect(onBook).toHaveBeenCalledWith(7n, "SYNTH-LP-2", "BUY");
    // … and hit the native maker's bid (SELL).
    fireEvent.click(screen.getByRole("button", { name: /^sell to celnet-auto-pricer/ }));
    expect(onBook).toHaveBeenCalledWith(7n, "celnet-auto-pricer", "SELL");
    expect(onBook).toHaveBeenCalledTimes(2);
  });

  it("disables an expired row with its reason; live rows keep their countdown ring", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    const expired = nowNanos() - 1n * NS_PER_S;
    const panel = panelWith(
      [
        dealer("celnet-auto-pricer", 0.12, 0.13, live),
        dealer("SYNTH-LP-1", 0.1206, 0.1296, expired),
      ],
      { bestBidLpId: "celnet-auto-pricer", bestOfferLpId: "celnet-auto-pricer" },
    );
    const onBook = vi.fn();
    render(<DealerPanel panel={panel} onBook={onBook} />);
    // The expired row: both sides disabled, the honest reason shown, no ring.
    const expiredBuy = screen.getByRole("button", {
      name: /^buy from SYNTH-LP-1/,
    }) as HTMLButtonElement;
    const expiredSell = screen.getByRole("button", {
      name: /^sell to SYNTH-LP-1/,
    }) as HTMLButtonElement;
    expect(expiredBuy.disabled).toBe(true);
    expect(expiredSell.disabled).toBe(true);
    expect(screen.getByText("expired — re-request")).toBeInTheDocument();
    expect(screen.queryByLabelText(/^SYNTH-LP-1 last-look/)).not.toBeInTheDocument();
    fireEvent.click(expiredBuy);
    expect(onBook).not.toHaveBeenCalled();
    // The live row: bookable, with its own depleting last-look countdown.
    const liveBuy = screen.getByRole("button", {
      name: /^buy from celnet-auto-pricer/,
    }) as HTMLButtonElement;
    expect(liveBuy.disabled).toBe(false);
    expect(screen.getByLabelText(/^celnet-auto-pricer last-look/)).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// TicketWorkspace integration — the multi-dealer RFQ flow over the offline mock
// ---------------------------------------------------------------------------

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
});

async function renderTicket(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <TicketWorkspace />
      </AppProvider>,
    );
  });
  // Settle the provider's async mount effects (see ticketProducts.test.tsx).
  await screen.findByRole("listbox", { name: "Structure catalogue" });
}

describe("TicketWorkspace — multi-dealer RFQ (LP panel) flow", () => {
  it("defaults to the single-dealer mode (the pre-panel flow is untouched)", async () => {
    await renderTicket();
    const mode = screen.getByRole("tablist", { name: "rfq mode" });
    expect(within(mode).getByText("Single-dealer")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("button", { name: /Request quote/ })).toBeInTheDocument();
    expect(
      screen.queryByRole("table", { name: "multi-dealer quote panel" }),
    ).not.toBeInTheDocument();
  });

  it("requests a ranked panel and books the best offer by (quote_id, lp_id)", async () => {
    await renderTicket();
    // Flip the RFQ mode to the LP panel; the Request button relabels honestly.
    const mode = screen.getByRole("tablist", { name: "rfq mode" });
    act(() => {
      fireEvent.click(within(mode).getByText("LP panel"));
    });
    const request = screen.getByRole("button", { name: /Request panel/ });
    await act(async () => {
      fireEvent.click(request);
    });

    // The ranked panel renders: the native maker plus the deterministic
    // synthetic demo LPs, in the engine's audit order (sorted by lpId — ASCII
    // "S" < "c", so the synthetic rows precede the native maker).
    const table = await screen.findByRole("table", {
      name: "multi-dealer quote panel",
    });
    const rows = within(table)
      .getAllByRole("rowheader")
      .map((el) => el.textContent);
    expect(rows).toEqual(["SYNTH-LP-1", "SYNTH-LP-2", "SYNTH-LP-3", "celnet-auto-pricer"]);

    // The deterministic winners of the synthetic law: SYNTH-LP-1's bid and
    // SYNTH-LP-2's offer (the server's `synthetic_lp_two_way` algebra).
    expect(
      within(table).getByRole("button", { name: / — best bid$/ }),
    ).toHaveAccessibleName(/sell to SYNTH-LP-1/);
    const bestOffer = within(table).getByRole("button", { name: / — best offer$/ });
    expect(bestOffer).toHaveAccessibleName(/buy from SYNTH-LP-2/);

    // Book the best offer: the accept carries (quote_id, lp_id) and the fill
    // renders that dealer's identity — then the consumed panel is dropped.
    await act(async () => {
      fireEvent.click(bestOffer);
    });
    expect(screen.getByText(/Filled Buy SYNTH-LP-2 @/)).toBeInTheDocument();
    expect(
      screen.queryByRole("table", { name: "multi-dealer quote panel" }),
    ).not.toBeInTheDocument();
  });
});

describe("DealerPanel — capability gating (slice 5: disable + tooltip, never hide)", () => {
  it("disables every book button and explains why when bookDisabled is set", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    const denyTitle = "Your permissions don't allow executing FX-options trades.";
    const panel = panelWith(
      [
        dealer("celnet-auto-pricer", 0.12, 0.13, live),
        dealer("SYNTH-LP-1", 0.1206, 0.1296, live),
      ],
      { bestBidLpId: "SYNTH-LP-1", bestOfferLpId: "SYNTH-LP-1" },
    );
    const onBook = vi.fn();
    render(
      <DealerPanel
        panel={panel}
        onBook={onBook}
        bookDisabled
        bookDisabledTitle={denyTitle}
      />,
    );
    // The rows are NEVER hidden — the panel table is still present.
    expect(screen.getByRole("table", { name: "multi-dealer quote panel" })).toBeInTheDocument();
    // Every book button is disabled and carries the explanatory denial tooltip.
    const bookButtons = screen
      .getAllByRole("button")
      .filter((b) => /sell to|buy from/i.test(b.getAttribute("aria-label") ?? ""));
    expect(bookButtons.length).toBe(4); // 2 LPs × (bid + offer)
    for (const b of bookButtons) {
      expect(b).toBeDisabled();
      expect(b.getAttribute("title")).toBe(denyTitle);
    }
    // A click on a disabled control never fires the booking callback.
    fireEvent.click(bookButtons[0]!);
    expect(onBook).not.toHaveBeenCalled();
  });

  it("leaves the book buttons live (no denial tooltip) when not capability-gated", () => {
    const live = nowNanos() + 8n * NS_PER_S;
    const panel = panelWith([dealer("SYNTH-LP-1", 0.1206, 0.1296, live)], {
      bestBidLpId: "SYNTH-LP-1",
      bestOfferLpId: "SYNTH-LP-1",
    });
    render(<DealerPanel panel={panel} onBook={vi.fn()} />);
    const sell = screen.getByRole("button", { name: /^sell to SYNTH-LP-1/ });
    expect(sell).not.toBeDisabled();
    expect(sell.getAttribute("title")).toBe("Hit this bid (SELL)");
  });
});
