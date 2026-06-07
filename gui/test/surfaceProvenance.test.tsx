/**
 * SurfaceWorkspace typed-provenance migration — proves the "marked as" provenance
 * line reads the TYPED `arbitrage.model` field, NOT the retired `model=<family>`
 * note regex. Renders the REAL SurfaceWorkspace inside the REAL AppProvider
 * (offline mock via `?mock`, no server), selects eSSVI, and asserts the displayed
 * family is the typed family the surface was marked under.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { SurfaceWorkspace } from "../src/workspaces/SurfaceWorkspace";

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
});

async function renderSurface(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <SurfaceWorkspace />
      </AppProvider>,
    );
  });
  await screen.findByRole("group", { name: "smile calibration model" });
}

describe("SurfaceWorkspace — typed model provenance display", () => {
  it("shows the default family from the typed field on first mark", async () => {
    await renderSurface();
    // The provenance line renders the typed family of the marked surface.
    expect(screen.getByText("market-hedge")).toBeInTheDocument();
  });

  it("selecting eSSVI shows extended-surface from the TYPED arbitrage.model", async () => {
    await renderSurface();
    const group = screen.getByRole("group", { name: "smile calibration model" });
    const essvi = within(group).getByRole("button", { name: "eSSVI" });

    await act(async () => {
      fireEvent.click(essvi);
    });

    // The re-marked surface's typed provenance is extended-surface — read from
    // `arbitrage.model`, not scraped from a note. (The previous regex would have
    // shown the same string, but it is now sourced from the authoritative field.)
    expect(await screen.findByText("extended-surface")).toBeInTheDocument();
    // The stale market-hedge provenance is gone (the surface re-marked).
    expect(screen.queryByText("market-hedge")).toBeNull();
  });
});
