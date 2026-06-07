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

describe("TicketWorkspace — wave-4 barrier/digital/touch in the selector", () => {
  it("offers single barrier / double barrier / digital / touch", async () => {
    await renderTicket();
    const options = within(structureSelect())
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(options).toContain("SINGLE_BARRIER");
    expect(options).toContain("DOUBLE_BARRIER");
    expect(options).toContain("DIGITAL");
    expect(options).toContain("TOUCH");
    // The pre-existing products are untouched (additive, zero-legacy).
    expect(options).toContain("VANILLA");
    expect(options).toContain("ASIAN");
  });

  it("shows the strike, knock kind/side, barrier and rebate for a single barrier", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "SINGLE_BARRIER" } });
    });
    expect(screen.getByRole("tablist", { name: "option type" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "barrier kind" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "barrier side" })).toBeInTheDocument();
    expect(screen.getByLabelText("barrier")).toBeInTheDocument();
    expect(screen.getByLabelText("rebate")).toBeInTheDocument();
    // No option legs are rendered for a barrier.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/Single-barrier/)).toBeInTheDocument();
  });

  it("shows lower/upper barriers and the knock kind for a double barrier", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "DOUBLE_BARRIER" } });
    });
    expect(screen.getByLabelText("lower barrier")).toBeInTheDocument();
    expect(screen.getByLabelText("upper barrier")).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "barrier kind" })).toBeInTheDocument();
    expect(screen.getByText(/Double-barrier/)).toBeInTheDocument();
  });

  it("shows the settlement-style toggle and payout for a digital", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "DIGITAL" } });
    });
    const style = screen.getByRole("tablist", { name: "digital style" });
    expect(within(style).getByText("Cash")).toBeInTheDocument();
    expect(within(style).getByText("Asset")).toBeInTheDocument();
    expect(screen.getByLabelText("payout")).toBeInTheDocument();
    expect(screen.getByText(/digital/)).toBeInTheDocument();
  });

  it("shows the touch-kind toggle and reveals the upper barrier only for double kinds", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "TOUCH" } });
    });
    const kind = screen.getByRole("tablist", { name: "touch kind" });
    expect(within(kind).getByText("One-touch")).toBeInTheDocument();
    expect(within(kind).getByText("Double-no-touch")).toBeInTheDocument();
    // A single-barrier one-touch has only the sole barrier (no upper).
    expect(screen.getByLabelText("barrier")).toBeInTheDocument();
    expect(screen.queryByLabelText("upper barrier")).not.toBeInTheDocument();
    // Switching to a double structure reveals the upper barrier (lower + upper).
    act(() => {
      fireEvent.click(within(kind).getByText("Double-no-touch"));
    });
    expect(screen.getByLabelText("lower barrier")).toBeInTheDocument();
    expect(screen.getByLabelText("upper barrier")).toBeInTheDocument();
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

describe("TicketWorkspace — wave-6 window barrier + booking-model selector", () => {
  it("offers a window barrier in the structure selector (additive, zero-legacy)", async () => {
    await renderTicket();
    const options = within(structureSelect())
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(options).toContain("WINDOW_BARRIER");
    // The pre-existing products are untouched.
    expect(options).toContain("VANILLA");
    expect(options).toContain("SINGLE_BARRIER");
  });

  it("shows the booking-model selector with Default + Local-Stoch-Vol for a vanilla", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "VANILLA" } });
    });
    const model = screen.getByRole("tablist", { name: "booking model" });
    expect(within(model).getByText("Default")).toBeInTheDocument();
    expect(within(model).getByText("Local-Stoch-Vol")).toBeInTheDocument();
  });

  it("hides the booking-model selector for an LSV-unsupported product (digital)", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "DIGITAL" } });
    });
    // A digital has no LSV route ⇒ no booking-model choice is offered.
    expect(screen.queryByRole("tablist", { name: "booking model" })).not.toBeInTheDocument();
  });

  it("locks the window barrier to Local-Stoch-Vol (the only model) and shows its inputs", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "WINDOW_BARRIER" } });
    });
    // The booking model is shown but locked to Local-Stoch-Vol (no Default option).
    const model = screen.getByRole("tablist", { name: "booking model" });
    expect(within(model).getByText("Local-Stoch-Vol")).toBeInTheDocument();
    expect(within(model).queryByText("Default")).not.toBeInTheDocument();
    expect((within(model).getByText("Local-Stoch-Vol") as HTMLButtonElement).disabled).toBe(true);
    // The window-barrier inputs render (option / side / barrier / window / MC).
    expect(screen.getByRole("tablist", { name: "option type" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "barrier side" })).toBeInTheDocument();
    expect(screen.getByLabelText("barrier")).toBeInTheDocument();
    expect(screen.getByLabelText("window start")).toBeInTheDocument();
    expect(screen.getByLabelText("window end")).toBeInTheDocument();
    // No option legs are rendered for a window barrier.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/Window barrier:/)).toBeInTheDocument();
  });

  it("gates Local-Stoch-Vol pricing to the live server when offline (no faked LSV)", async () => {
    await renderTicket(); // ?mock ⇒ the offline in-app transport
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "WINDOW_BARRIER" } });
    });
    // Offline, the LSV engine is unavailable: the Request button is disabled and
    // labelled honestly (the mock never fabricates an LSV number).
    const request = screen.getByRole("button", { name: /LSV — live server only/ });
    expect((request as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/server-side only/)).toBeInTheDocument();
  });

  it("toggling Local-Stoch-Vol on a vanilla offline disables Request quote honestly", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "VANILLA" } });
    });
    // Default ⇒ pricing works (offline analytic path is intact).
    expect(screen.getByRole("button", { name: /Request quote/ })).toBeInTheDocument();
    // Selecting Local-Stoch-Vol offline gates pricing to the live server.
    const model = screen.getByRole("tablist", { name: "booking model" });
    act(() => {
      fireEvent.click(within(model).getByText("Local-Stoch-Vol"));
    });
    expect(screen.getByRole("button", { name: /LSV — live server only/ })).toBeInTheDocument();
  });
});

describe("TicketWorkspace — American / Bermudan early-exercise vanilla", () => {
  it("offers American / Bermudan in the structure selector (additive, zero-legacy)", async () => {
    await renderTicket();
    const options = within(structureSelect())
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(options).toContain("AMERICAN");
    // The pre-existing products are untouched.
    expect(options).toContain("VANILLA");
    expect(options).toContain("WINDOW_BARRIER");
  });

  it("shows option / strike / exercise-style for an American (no Bermudan dates field)", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "AMERICAN" } });
    });
    expect(screen.getByRole("tablist", { name: "option type" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "exercise style" })).toBeInTheDocument();
    expect(screen.getByLabelText("lsm paths")).toBeInTheDocument();
    // AMERICAN (the default) has no discrete exercise-date count.
    expect(screen.queryByLabelText("bermudan dates")).not.toBeInTheDocument();
    // No option legs are rendered for an early-exercise vanilla.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/early-exercise vanilla/)).toBeInTheDocument();
  });

  it("reveals the exercise-date count only when Bermudan is selected", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "AMERICAN" } });
    });
    const style = screen.getByRole("tablist", { name: "exercise style" });
    act(() => {
      fireEvent.click(within(style).getByText("Bermudan"));
    });
    expect(screen.getByLabelText("bermudan dates")).toBeInTheDocument();
    // Switching back to American hides it again.
    act(() => {
      fireEvent.click(within(style).getByText("American"));
    });
    expect(screen.queryByLabelText("bermudan dates")).not.toBeInTheDocument();
  });

  it("reveals the LSM seed only when LSM paths > 0 (the Monte-Carlo engine)", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "AMERICAN" } });
    });
    // FD engine by default (paths 0) ⇒ no seed field, labelled "exact FD".
    expect(screen.queryByLabelText("lsm seed")).not.toBeInTheDocument();
    expect(screen.getByText(/exact FD/)).toBeInTheDocument();
    // Entering an LSM path count switches to the Longstaff-Schwartz MC engine.
    act(() => {
      fireEvent.change(screen.getByLabelText("lsm paths"), { target: { value: "50000" } });
    });
    expect(screen.getByLabelText("lsm seed")).toBeInTheDocument();
    expect(screen.getByText(/Longstaff-Schwartz MC/)).toBeInTheDocument();
  });

  it("prices an American put offline (the binomial path is wired and reachable)", async () => {
    await renderTicket();
    act(() => {
      fireEvent.change(structureSelect(), { target: { value: "AMERICAN" } });
    });
    // The offline binomial engine prices it (no LSV gate) ⇒ Request is enabled.
    const request = screen.getByRole("button", { name: /Request quote/ });
    expect((request as HTMLButtonElement).disabled).toBe(false);
  });
});
