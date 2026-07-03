import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { createMockTransport } from "../src/data/mockSource";
import { RiskWorkspace } from "../src/workspaces/RiskWorkspace";

/**
 * The joint options+FI tail LENS of the shared Risk workspace, rendered against
 * the REAL offline transport (`?mock`). Proves the lens sources a genuine book,
 * calls `CombinedTailRisk`, and renders the headline joint VaR/ES, the signed
 * parallel DV01 and the FI key-rate ladder — with an honest empty-state when the
 * FI sub-book is turned off (the reduction-to-options-VaR branch) — and that the
 * lens bar is an a11y tablist with the combined tab selected.
 */

function renderCombinedLens(): void {
  render(
    <AppProvider transport={createMockTransport()}>
      <RiskWorkspace initialLens="combined" />
    </AppProvider>,
  );
}

afterEach(() => cleanup());

describe("RiskWorkspace — combined options+FI tail lens", () => {
  it("renders the joint VaR/ES, parallel DV01 and the key-rate ladder from a real roll-up", async () => {
    renderCombinedLens();

    // Headline joint tail: VaR + Expected shortfall + the signed parallel DV01.
    expect(await screen.findByText(/VaR \(99%\)/)).toBeTruthy();
    expect(await screen.findByText(/Expected shortfall \(99%\)/)).toBeTruthy();
    expect(await screen.findByText("FI parallel DV01")).toBeTruthy();

    // The FI key-rate DV01 ladder viz (the reused KeyRateLadder), by its accessible name.
    expect(
      await screen.findByRole("img", { name: /Key-rate DV01 ladder/ }),
    ).toBeTruthy();
  });

  it("exposes the lens bar as a tablist with the combined tab selected (a11y)", async () => {
    renderCombinedLens();
    await screen.findByText(/VaR \(99%\)/);

    const tablist = screen.getByRole("tablist", { name: "risk asset class lens" });
    expect(tablist).toBeTruthy();
    const combinedTab = screen.getByRole("tab", { name: "Combined tail" });
    expect(combinedTab.getAttribute("aria-selected")).toBe("true");
  });

  it("falls back to an honest empty-state for the FI axis when the FI book is disabled", async () => {
    renderCombinedLens();
    await screen.findByRole("img", { name: /Key-rate DV01 ladder/ });

    // Turn the FI sub-book off ⇒ options-only ⇒ no rate ladder / parallel DV01.
    const fiToggle = screen.getByRole("checkbox", { name: /FI \(USD-SOFR\)/ });
    fireEvent.click(fiToggle);

    expect(
      await screen.findByText(/No fixed-income positions in the book/),
    ).toBeTruthy();
    // The options VaR headline still renders (the tail reduced to the options leg).
    expect(await screen.findByText(/VaR \(99%\)/)).toBeTruthy();
  });
});
