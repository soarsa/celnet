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

describe("TicketWorkspace — wave-2 products in the selector", () => {
  it("offers forward-start / cliquet / quanto alongside the wave-1 products", async () => {
    await renderTicket();
    const options = within(structureSelect())
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(options).toContain("FORWARD_START");
    expect(options).toContain("CLIQUET");
    expect(options).toContain("QUANTO");
    // The pre-existing products are untouched (additive, zero-legacy).
    expect(options).toContain("ASIAN");
    expect(options).toContain("VANILLA");
  });

  it("shows the reset-moneyness and reset-date inputs for a forward start", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "FORWARD_START" } });
    });
    expect(screen.getByLabelText("moneyness")).toBeInTheDocument();
    expect(screen.getByLabelText("reset")).toBeInTheDocument();
    // No option legs are rendered for a forward start.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/Forward-start vanilla/)).toBeInTheDocument();
  });

  it("shows the cliquet schedule + clamp toggles; MC-pairs appears only when clamped", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "CLIQUET" } });
    });
    expect(screen.getByLabelText("periods")).toBeInTheDocument();
    expect(screen.getByLabelText("use local cap")).toBeInTheDocument();
    expect(screen.getByLabelText("use local floor")).toBeInTheDocument();
    // A plain ratchet (no clamp) hides the MC controls and labels itself closed-form.
    expect(screen.queryByLabelText("mc pairs")).not.toBeInTheDocument();
    expect(screen.getByText(/Plain ratchet/)).toBeInTheDocument();
    // Enabling the local cap switches to the Monte-Carlo path (MC-pairs + note).
    act(() => {
      fireEvent.click(screen.getByLabelText("use local cap"));
    });
    expect(screen.getByLabelText("mc pairs")).toBeInTheDocument();
    expect(screen.getByText(/Clamped cliquet/)).toBeInTheDocument();
  });

  it("shows the payoff toggle, conversion vol and correlation for a quanto", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "QUANTO" } });
    });
    const payoff = screen.getByRole("tablist", { name: "payoff" });
    expect(within(payoff).getByText("Vanilla")).toBeInTheDocument();
    expect(within(payoff).getByText("Digital")).toBeInTheDocument();
    expect(screen.getByLabelText("conversion vol")).toBeInTheDocument();
    expect(screen.getByLabelText("correlation")).toBeInTheDocument();
    // The product note describes the quanto-drift adjustment (distinct from the
    // "Quanto" selector option label, which also contains the word).
    expect(screen.getByText(/quanto-drift adjustment/)).toBeInTheDocument();
  });
});
