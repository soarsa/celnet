/**
 * Domain deep-link + shared-screen lens pre-select (regression for the 3-tab
 * domain bar, commit 333c458).
 *
 * The saved-view / URL codec encodes the active product DOMAIN as `dom`. A
 * deep-link / reload must seed `activeDomain` on FIRST paint so:
 *   • the domain TAB BAR renders the URL's domain as active (not the fx_options
 *     default), and
 *   • each SHARED screen (Risk / Market Data) pre-selects its FX↔rates lens to the
 *     active domain on the initial mount — not only after a user tab-click.
 *
 * These render the REAL Shell inside the REAL AppProvider (offline `?mock`) and
 * assert the first-paint state for both domains, plus that an old (no-`dom`) link
 * still decodes to the fx_options default (forward-compat).
 */
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { Shell } from "../src/app/Shell";

/** Render the Shell at a given URL search string and settle the initial effects. */
async function renderAt(search: string): Promise<void> {
  window.history.replaceState(null, "", search);
  await act(async () => {
    render(
      <AppProvider>
        <Shell />
      </AppProvider>,
    );
  });
  // Flush the mount effects (auth gating, live URL mirror) so any post-mount
  // re-home would be visible — the point being that NONE is needed for the seed.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

/** The product-domain tab bar. */
function domainTabs() {
  return screen.getByRole("tablist", { name: "product domains" });
}
/** The Risk workspace's FX↔rates lens tablist. */
function riskLens() {
  return screen.getByRole("tablist", { name: "risk asset class lens" });
}
/** The Market Data workspace's FX↔rates lens group. */
function marketDataLens() {
  return screen.getByRole("group", { name: "market data asset class" });
}

describe("domain deep-link seeds activeDomain + shared-screen lens on first paint", () => {
  it("?dom=fixed_income&view=risk activates the FI tab AND the FI risk lens", async () => {
    await renderAt("/?mock&dom=fixed_income&view=risk");

    // The domain tab bar reflects the URL's domain (not the fx_options default).
    expect(
      within(domainTabs()).getByRole("tab", { name: "Fixed Income", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      within(domainTabs()).getByRole("tab", { name: "FX Options", exact: true }),
    ).toHaveAttribute("aria-selected", "false");

    // The shared Risk screen pre-selects the rates lens for the FI domain.
    expect(
      within(riskLens()).getByRole("tab", { name: "Fixed Income", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  it("?dom=fixed_income ALONE (no other view param) still seeds the FI domain", async () => {
    await renderAt("/?mock&dom=fixed_income");

    expect(
      within(domainTabs()).getByRole("tab", { name: "Fixed Income", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  it("an old link with NO dom (view=risk only) decodes to the fx_options default", async () => {
    await renderAt("/?mock&view=risk");

    expect(
      within(domainTabs()).getByRole("tab", { name: "FX Options", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      within(riskLens()).getByRole("tab", { name: "FX Options", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  it("Market Data initial lens follows activeDomain — FI on ?dom=fixed_income", async () => {
    await renderAt("/?mock&dom=fixed_income&view=surface");

    expect(
      within(marketDataLens()).getByRole("button", { name: "Fixed Income" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("Market Data initial lens follows activeDomain — FX on ?dom=fx_options", async () => {
    await renderAt("/?mock&dom=fx_options&view=surface");

    expect(
      within(marketDataLens()).getByRole("button", { name: "FX Options" }),
    ).toHaveAttribute("aria-pressed", "true");
  });
});
