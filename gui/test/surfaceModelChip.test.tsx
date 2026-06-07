/**
 * SurfaceWorkspace smile-model chip coverage — W5 eSSVI client-parity.
 *
 * Renders the REAL `SurfaceWorkspace` inside the REAL `AppProvider` (forced to the
 * offline in-app mock via `?mock`, so no server is dialled) and drives the model
 * chip row exactly as a trader would. It proves the eSSVI (EXTENDED_SURFACE) chip
 * is present and that clicking it pressing-toggles the active model — the genuine
 * `MarkSurfaceRequest.smile_model` selection path, not a mock of it.
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
  // Drain the provider's async mount (auto-mark resolves via the mock transport's
  // microtask chain) inside RTL's act wrapper before asserting.
  await screen.findByRole("group", { name: "smile calibration model" });
}

/** The model-chip row (the `MarkSurfaceRequest.smile_model` selector). */
function modelGroup(): HTMLElement {
  return screen.getByRole("group", { name: "smile calibration model" });
}

describe("SurfaceWorkspace — eSSVI (EXTENDED_SURFACE) model chip", () => {
  it("offers the eSSVI chip alongside the other calibration families", async () => {
    await renderSurface();
    const chip = within(modelGroup()).getByRole("button", { name: "eSSVI" });
    expect(chip).toBeInTheDocument();
    // It starts un-pressed (default model is the desk market-hedge).
    expect(chip.getAttribute("aria-pressed")).toBe("false");
  });

  it("selecting eSSVI marks the live surface under EXTENDED_SURFACE (chip becomes active)", async () => {
    await renderSurface();
    const group = modelGroup();
    const essvi = within(group).getByRole("button", { name: "eSSVI" });
    const marketHedge = within(group).getByRole("button", { name: "Market hedge" });

    expect(marketHedge.getAttribute("aria-pressed")).toBe("true");
    expect(essvi.getAttribute("aria-pressed")).toBe("false");

    await act(async () => {
      fireEvent.click(essvi);
    });

    // The selection routed to `setSurfaceModel("EXTENDED_SURFACE")`, re-marking the
    // live surface under it: eSSVI is now the single active (pressed) chip.
    expect(essvi.getAttribute("aria-pressed")).toBe("true");
    expect(marketHedge.getAttribute("aria-pressed")).toBe("false");
  });
});
