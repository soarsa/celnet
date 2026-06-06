/**
 * TicketWorkspace product-selector coverage for the wave-1 volatility products.
 * Renders the REAL `TicketWorkspace` inside the REAL `AppProvider` (forced to the
 * offline in-app mock via `?mock`, so no server is dialled) and drives it through
 * the DOM exactly as a trader would: it asserts the new products appear in the
 * structure selector and that selecting each surfaces the right inputs (the swap
 * strike-vol field; the Asian option-type / averaging / fixings / method
 * controls). This exercises the genuine build/encode wiring, not a mock of it.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { TicketWorkspace } from "../src/workspaces/TicketWorkspace";

/**
 * Force the offline mock transport (no WebSocket dialled in jsdom). The mock
 * stream session ticks a price tape on a real `setInterval`; we stop it (and the
 * provider's bounded surface-mark retry) on teardown so no timer outlives a test.
 */
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
  // Settle the provider's async mount effects (the surface auto-mark resolves via
  // the mock transport's microtask chain, then schedules `setSurface`; the stream
  // session emits its baseline snapshot). Awaiting a findBy* drains those state
  // updates inside React Testing Library's own act(...) wrapper, so the render is
  // fully settled before we assert and no update escapes an act boundary.
  await screen.findByLabelText("structure");
}

/** The structure <select> the ticket builds a product from. */
function structureSelect(): HTMLSelectElement {
  return screen.getByLabelText("structure") as HTMLSelectElement;
}

describe("TicketWorkspace — wave-1 products in the selector", () => {
  it("offers variance swap / volatility swap / Asian alongside vanilla", async () => {
    await renderTicket();
    const options = within(structureSelect())
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(options).toContain("VARIANCE_SWAP");
    expect(options).toContain("VOLATILITY_SWAP");
    expect(options).toContain("ASIAN");
    // The pre-existing products are untouched (additive, zero-legacy).
    expect(options).toContain("VANILLA");
    expect(options).toContain("RISK_REVERSAL");
  });

  it("shows the strike-vol input when a variance swap is selected", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "VARIANCE_SWAP" } });
    });
    expect(screen.getByLabelText("strike vol")).toBeInTheDocument();
    // No option legs are rendered for a swap.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/Fair variance strike K_var/)).toBeInTheDocument();
  });

  it("shows the strike-vol input when a volatility swap is selected", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "VOLATILITY_SWAP" } });
    });
    expect(screen.getByLabelText("strike vol")).toBeInTheDocument();
    expect(screen.getByText(/Fair volatility strike K_vol/)).toBeInTheDocument();
  });

  it("shows option type, averaging, fixings and method for an Asian", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "ASIAN" } });
    });
    // Option-type toggle (Call/Put), averaging toggle, method toggle, fixings field.
    expect(screen.getByRole("tablist", { name: "option type" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "averaging" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "method" })).toBeInTheDocument();
    expect(screen.getByLabelText("observations")).toBeInTheDocument();
    // Curran and Turnbull-Wakeman are both selectable.
    const method = screen.getByRole("tablist", { name: "method" });
    expect(within(method).getByText("Curran")).toBeInTheDocument();
    expect(within(method).getByText("Turnbull-Wakeman")).toBeInTheDocument();
  });

  it("hides the discrete fixings field when continuous averaging is chosen", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "ASIAN" } });
    });
    expect(screen.getByLabelText("observations")).toBeInTheDocument();
    const averaging = screen.getByRole("tablist", { name: "averaging" });
    act(() => {
      fireEvent.click(within(averaging).getByText("Continuous"));
    });
    // Continuous has no fixing count.
    expect(screen.queryByLabelText("observations")).not.toBeInTheDocument();
  });
});
