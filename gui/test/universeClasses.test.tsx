/**
 * Asset-class-aware universe navigation (the gui-universe lane).
 *
 * Renders the REAL Shell inside the REAL AppProvider (offline `?mock`) and proves:
 *   • the scope drill's terminal leaf is book → ASSET CLASS → underlier: the
 *     class rail (FX · Metals · Equity · Commodity · Crypto) with FX as the
 *     default — and the FX leaf is UNCHANGED (the regression guard);
 *   • selecting a non-FX underlier re-targets the terminal scope crumb to that
 *     underlier (metal pair projection / ticker / crypto pair);
 *   • the selection PRE-TARGETS the Ticket workspace's cross-asset vanilla spec
 *     with the exact `Underlying` (+ linear vs inverse settlement for crypto),
 *     while an FX selection leaves the ticket untouched;
 *   • the Surface workspace's family switch is asset-class-aware: a non-FX scope
 *     renders the typed, honest "no marked surface for this class" state (never
 *     a fabricated surface), and re-selecting FX restores the real workspace;
 *   • keyboard (type-to-filter + ↑↓/Enter + ⌘D favourites) works across classes,
 *     and the listbox/combobox structure stays a11y-correct (GW2-style
 *     structural assertions: options carry no nested focusable controls).
 */

import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { Shell } from "../src/app/Shell";

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

async function renderShell(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <Shell />
      </AppProvider>,
    );
  });
  await act(async () => {
    await Promise.resolve();
  });
}

/** Drill firm → desk → book through the org leaf, then open the universe leaf. */
async function drillToUniverse(): Promise<HTMLElement> {
  fireEvent.click(screen.getByRole("button", { name: /drill into a desk/i }));
  const desks = await screen.findByRole("dialog", { name: "Desks" });
  await act(async () => {
    fireEvent.click(within(desks).getAllByRole("option")[0]!);
  });
  fireEvent.click(screen.getByRole("button", { name: /drill into a book/i }));
  const books = await screen.findByRole("dialog", { name: "Books" });
  await act(async () => {
    fireEvent.click(within(books).getAllByRole("option")[0]!);
  });
  fireEvent.click(screen.getByRole("button", { name: /drill into a pair/i }));
  return await screen.findByRole("dialog", { name: "Pairs" });
}

/** Switch the open universe leaf to an asset class; returns the re-titled dialog. */
async function switchClass(dialog: HTMLElement, label: string): Promise<HTMLElement> {
  const rail = within(dialog).getByRole("group", { name: "asset class" });
  await act(async () => {
    fireEvent.click(within(rail).getByRole("button", { name: new RegExp(`^${label}$`) }));
  });
  return await screen.findByRole("dialog", { name: label === "FX" ? "Pairs" : label });
}

/** Jump to a workspace via its rail button. */
async function goTo(label: RegExp): Promise<void> {
  const rail = screen.getByRole("complementary", { name: "workspaces" });
  await act(async () => {
    fireEvent.click(within(rail).getByRole("button", { name: label }));
  });
}

/** The single scope nav. */
function scopeNav(): HTMLElement {
  return screen.getByRole("navigation", { name: "scope" });
}

describe("the universe leaf: book → asset class → underlier", () => {
  it("carries the class rail with FX as the unchanged default leaf", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const rail = within(dialog).getByRole("group", { name: "asset class" });
    for (const c of ["FX", "Metals", "Equity", "Commodity", "Crypto"]) {
      expect(within(rail).getByRole("button", { name: new RegExp(`^${c}$`) })).toBeInTheDocument();
    }
    // FX is the resting class…
    expect(
      within(rail).getByRole("button", { name: /^FX$/ }).getAttribute("aria-pressed"),
    ).toBe("true");
    // …and its leaf is the original pair universe (seeded pairs, honesty footer).
    expect(within(dialog).getByRole("option", { name: /EUR\/USD/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("listbox", { name: "currency pairs" })).toBeInTheDocument();
    expect(dialog.textContent).toContain("seeded set");
  });

  it("Metals lists the four metals vs majors + the metal crosses; selection re-targets the crumb", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const metals = await switchClass(dialog, "Metals");
    expect(within(metals).getByRole("listbox", { name: "metals underliers" })).toBeInTheDocument();
    for (const label of ["XAU/USD", "XAG/USD", "XPT/USD", "XPD/USD", "XAU/EUR", "XPT/EUR"]) {
      expect(within(metals).getByRole("option", { name: new RegExp(label) })).toBeInTheDocument();
    }
    // The honesty footer labels the seeded universe.
    expect(metals.textContent).toContain("seeded set");

    await act(async () => {
      fireEvent.click(within(metals).getByRole("option", { name: /XAU\/USD/ }));
    });
    // The terminal scope crumb now points at the metal underlier.
    expect(within(scopeNav()).getByRole("button", { name: "XAU/USD" })).toBeInTheDocument();
  });

  it("keyboard: type-to-filter + Enter selects across classes; ⌘D favourites a non-FX row", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const equity = await switchClass(dialog, "Equity");
    const combo = within(equity).getByRole("combobox");
    await act(async () => {
      fireEvent.change(combo, { target: { value: "nvda" } });
    });
    await act(async () => {
      fireEvent.keyDown(combo, { key: "Enter" });
    });
    expect(within(scopeNav()).getByRole("button", { name: "NVDA" })).toBeInTheDocument();

    // Reopen via the terminal crumb — the leaf lands on the ACTIVE class (Equity).
    fireEvent.click(within(scopeNav()).getByRole("button", { name: "NVDA" }));
    const reopened = await screen.findByRole("dialog", { name: "Equity" });
    // ⌘D favourites the highlighted row (the shared cross-class affordance)…
    await act(async () => {
      fireEvent.keyDown(within(reopened).getByRole("combobox"), { key: "d", metaKey: true });
    });
    // …so the Favourites section appears with the starred row.
    expect(within(reopened).getByText("Favourites")).toBeInTheDocument();
    expect(
      within(reopened).getAllByRole("option", { name: /\(favourite\)/ }).length,
    ).toBeGreaterThan(0);
  });

  it("a11y structure: options carry no nested focusable controls; the combobox tracks an option", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const crypto = await switchClass(dialog, "Crypto");
    const listbox = within(crypto).getByRole("listbox");
    // No real <button> inside the listbox (stars are inert spans; the class rail
    // chips live OUTSIDE it) — the nested-interactive guard, GW2-style.
    expect(within(listbox).queryAllByRole("button")).toHaveLength(0);
    const combo = within(crypto).getByRole("combobox");
    const adId = combo.getAttribute("aria-activedescendant");
    expect(adId).toBeTruthy();
    expect(document.getElementById(adId!)?.getAttribute("role")).toBe("option");
  });
});

describe("selection drives the workspaces", () => {
  it("a metal selection pre-targets the ticket's cross-asset spec with the exact Underlying", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const metals = await switchClass(dialog, "Metals");
    await act(async () => {
      fireEvent.click(within(metals).getByRole("option", { name: /XAU\/USD/ }));
    });
    await goTo(/Ticket/);
    // The cross-asset input block is active, seeded METAL · GOLD · USD.
    const metalSelect = screen.getByLabelText("metal") as HTMLSelectElement;
    expect(metalSelect.value).toBe("GOLD");
    expect((screen.getByLabelText("currency") as HTMLInputElement).value).toBe("USD");
    const classTabs = screen.getByRole("tablist", { name: "asset class" });
    expect(
      within(classTabs).getByRole("tab", { name: "Metal" }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("a crypto selection carries linear vs inverse settlement into the ticket", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const crypto = await switchClass(dialog, "Crypto");
    // The seeded universe notes the contract mechanics per row…
    expect(
      within(crypto).getByRole("option", { name: /BTC\/USD \(inverse · coin-margined\)/ }),
    ).toBeInTheDocument();
    expect(
      within(crypto).getByRole("option", { name: /BTC\/USDT \(linear · stable-margined\)/ }),
    ).toBeInTheDocument();
    // …and selecting the coin-margined row pre-targets INVERSE_COIN.
    await act(async () => {
      fireEvent.click(within(crypto).getByRole("option", { name: /BTC\/USD \(inverse/ }));
    });
    expect(within(scopeNav()).getByRole("button", { name: "BTC/USD" })).toBeInTheDocument();
    await goTo(/Ticket/);
    expect((screen.getByLabelText("symbol") as HTMLInputElement).value).toBe("BTC");
    const settlement = screen.getByRole("tablist", { name: "settlement style" });
    expect(
      within(settlement)
        .getByRole("tab", { name: /Inverse \(coin-margined\)/ })
        .getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("an FX selection does NOT re-point the ticket (FX flow regression)", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("option", { name: /GBP\/USD/ }));
    });
    expect(within(scopeNav()).getByRole("button", { name: "GBP/USD" })).toBeInTheDocument();
    await goTo(/Ticket/);
    // The default FX structure is untouched — no cross-asset block was forced in.
    expect(screen.queryByLabelText("metal")).toBeNull();
    expect(screen.queryByRole("tablist", { name: "settlement style" })).toBeNull();
  });
});

describe("the asset-class-aware surface-family switch", () => {
  it("a non-FX scope renders the typed honest empty state — never a fabricated surface", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const crypto = await switchClass(dialog, "Crypto");
    await act(async () => {
      fireEvent.click(within(crypto).getByRole("option", { name: /ETH\/USD \(inverse/ }));
    });
    await goTo(/Market Data/);
    const empty = screen.getByRole("status", { name: "surface unavailable" });
    expect(empty.textContent).toContain("No marked surface for Crypto");
    expect(empty.textContent).toContain("FX-only today");
    // No calibration-family chips and no marking grid are faked for the class.
    expect(screen.queryByRole("group", { name: "smile calibration model" })).toBeNull();
    expect(screen.queryByText(/Publish v/)).toBeNull();
  });

  it("re-selecting an FX underlier restores the real FX surface workspace", async () => {
    await renderShell();
    const dialog = await drillToUniverse();
    const metals = await switchClass(dialog, "Metals");
    await act(async () => {
      fireEvent.click(within(metals).getByRole("option", { name: /XAU\/USD/ }));
    });
    await goTo(/Market Data/);
    const empty = screen.getByRole("status", { name: "surface unavailable" });
    // The empty state offers the way back: the switch-underlier affordance.
    await act(async () => {
      fireEvent.click(within(empty).getByRole("button", { name: /Switch underlier/i }));
    });
    const reopened = await screen.findByRole("dialog", { name: "Metals" });
    const fx = await switchClass(reopened, "FX");
    await act(async () => {
      fireEvent.click(within(fx).getByRole("option", { name: /EUR\/USD/ }));
    });
    // Back on FX: the honest empty state is gone and the crumb is the FX pair.
    expect(screen.queryByRole("status", { name: "surface unavailable" })).toBeNull();
    expect(within(scopeNav()).getByRole("button", { name: "EUR/USD" })).toBeInTheDocument();
  });
});
