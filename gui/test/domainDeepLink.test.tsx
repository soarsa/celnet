/**
 * Domain deep-link + hard vertical asset separation on the shared screens
 * (regression for the 3-tab domain bar, commit 333c458; updated for W1).
 *
 * The saved-view / URL codec encodes the active product DOMAIN as `dom`. A
 * deep-link / reload must seed `activeDomain` on FIRST paint so:
 *   • the domain TAB BAR renders the URL's domain as active (not the fx_options
 *     default), and
 *   • each SHARED screen (Risk / Market Data) renders ONLY the active domain's
 *     asset on the initial mount — the lens is DERIVED STRICTLY from the domain,
 *     with NO in-screen cross-asset toggle (hard vertical separation).
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

/** The removed cross-asset lens toggles must never appear on a shared screen. */
function noAssetLensToggle(): void {
  expect(screen.queryByRole("tablist", { name: "risk asset class lens" })).toBeNull();
  expect(screen.queryByRole("group", { name: "market data asset class" })).toBeNull();
}

describe("domain deep-link seeds activeDomain + the shared screen's single asset on first paint", () => {
  it("?dom=fixed_income&view=risk activates the FI tab AND renders ONLY the FI risk lens (no FX tab)", async () => {
    await renderAt("/?mock&dom=fixed_income&view=risk");

    // The domain tab bar reflects the URL's domain (not the fx_options default).
    expect(
      within(domainTabs()).getByRole("tab", { name: "Fixed Income", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      within(domainTabs()).getByRole("tab", { name: "FX Options", exact: true }),
    ).toHaveAttribute("aria-selected", "false");

    // The shared Risk screen renders ONLY the FI rates panel — no cross-asset
    // toggle and no FX scenario grid. Scope to the VISIBLE heading via role: the
    // always-mounted Book pane's Aggregate-Risk lens also renders the rates panel
    // under FI, but it is `aria-hidden`/`inert`, so the role query (hidden:false)
    // resolves to the one active Risk screen.
    expect(
      await screen.findByRole("heading", { name: "Netted rates risk" }),
    ).toBeInTheDocument();
    noAssetLensToggle();
    expect(screen.queryByLabelText(/scenario heatmap/i)).toBeNull();
  });

  it("?dom=fixed_income ALONE (no other view param) still seeds the FI domain", async () => {
    await renderAt("/?mock&dom=fixed_income");

    expect(
      within(domainTabs()).getByRole("tab", { name: "Fixed Income", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  it("an old link with NO dom (view=risk only) decodes to the fx_options default and renders ONLY the FX lens", async () => {
    await renderAt("/?mock&view=risk");

    expect(
      within(domainTabs()).getByRole("tab", { name: "FX Options", exact: true }),
    ).toHaveAttribute("aria-selected", "true");

    // The shared Risk screen renders ONLY the FX scenario grid — no cross-asset
    // toggle and no FI rates panel.
    expect(await screen.findByLabelText(/scenario heatmap/i)).toBeInTheDocument();
    noAssetLensToggle();
    expect(screen.queryByText("Netted rates risk")).toBeNull();
  });

  it("Market Data renders ONLY the FI (curve) lens on ?dom=fixed_income — no toggle, no FX surface", async () => {
    await renderAt("/?mock&dom=fixed_income&view=surface");

    expect(
      await screen.findByRole("tablist", { name: "curves manager lens" }),
    ).toBeInTheDocument();
    noAssetLensToggle();
    expect(screen.queryByRole("group", { name: "surface view" })).toBeNull();
  });

  it("Market Data renders ONLY the FX (vol-surface) lens on ?dom=fx_options — no toggle, no FI curve", async () => {
    await renderAt("/?mock&dom=fx_options&view=surface");

    expect(await screen.findByRole("group", { name: "surface view" })).toBeInTheDocument();
    noAssetLensToggle();
    expect(screen.queryByRole("tablist", { name: "curves manager lens" })).toBeNull();
  });
});
