/**
 * TicketWorkspace product-selector coverage for the wave-1 volatility products.
 * Renders the REAL `TicketWorkspace` inside the REAL `AppProvider` (forced to the
 * offline in-app mock via `?mock`, so no server is dialled) and drives it through
 * the DOM exactly as a trader would: it asserts the new products appear in the
 * structure gallery and that selecting each surfaces the right inputs (the swap
 * strike-vol field; the Asian option-type / averaging / fixings / method
 * controls). This exercises the genuine build/encode wiring, not a mock of it.
 *
 * GW2: structure selection is the {@link StructureGallery} (a `role="listbox"` of
 * `role="option"` cards labelled by the spec label) — NOT the former flat
 * `<select>`. The input-filling steps (aria-labels) are unchanged: the per-family
 * InputBlocks were extracted verbatim into the product registry, so the controls
 * the trader fills are identical.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { TicketWorkspace } from "../src/workspaces/TicketWorkspace";
import { specById } from "../src/products";

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
  await screen.findByRole("listbox", { name: "Structure catalogue" });
}

/** The structure gallery listbox the ticket builds a product from. */
function structureGallery(): HTMLElement {
  return screen.getByRole("listbox", { name: "Structure catalogue" });
}

/** The structure ids the gallery currently offers (mapped back from card labels). */
function structureOptionIds(): string[] {
  return within(structureGallery())
    .getAllByRole("option")
    .map((card) => {
      // Each card's primary text is the spec label; recover its id via the registry.
      const label = (card.textContent ?? "").trim();
      const spec = SPEC_BY_LABEL.get(stripLabel(label));
      return spec?.id ?? label;
    });
}

/** The card-label prefix (the spec label precedes the asset-class chip + summary). */
function stripLabel(text: string): string {
  // Card text is "<label><chip><summary>"; match the longest known label prefix.
  for (const [lbl] of SPEC_BY_LABEL) {
    if (text.startsWith(lbl)) return lbl;
  }
  return text;
}

/** Select a structure by its id (click the gallery card with the spec's label). */
function selectStructure(id: string): void {
  const spec = specById(id)!;
  const card = within(structureGallery())
    .getAllByRole("option")
    .find((c) => (c.textContent ?? "").startsWith(spec.label));
  if (!card) throw new Error(`no gallery card for structure ${id} (label "${spec.label}")`);
  act(() => {
    fireEvent.click(card);
  });
}

/**
 * The active InputBlock's product NOTE paragraph matching `re`. The gallery card
 * summaries (`<span>`) can share wording with a family's note (`<p>`), so we scope
 * to the paragraph element — asserting the SAME thing the old flat-select test did
 * (the note renders) without the now-ambiguous whole-screen text match.
 */
function productNote(re: RegExp): HTMLElement {
  const note = screen.getAllByText(re).find((el) => el.tagName === "P");
  if (!note) throw new Error(`no product-note paragraph matching ${re}`);
  return note;
}

/** Registry label → spec, for recovering an id from a gallery card's text. */
const SPEC_BY_LABEL = new Map<string, ReturnType<typeof specById>>(
  [
    "VANILLA",
    "RISK_REVERSAL",
    "STRANGLE",
    "STRADDLE",
    "SEAGULL",
    "SINGLE_BARRIER",
    "DOUBLE_BARRIER",
    "DIGITAL",
    "TOUCH",
    "VARIANCE_SWAP",
    "VOLATILITY_SWAP",
    "ASIAN",
    "FORWARD_START",
    "CLIQUET",
    "QUANTO",
    "TARF",
    "ACCUMULATOR",
    "LOOKBACK",
    "WINDOW_BARRIER",
    "AMERICAN",
    "BASKET",
  ]
    .map((id) => specById(id))
    .filter((s): s is NonNullable<ReturnType<typeof specById>> => s !== undefined)
    .sort((a, b) => b.label.length - a.label.length) // longest-label-first prefix match
    .map((s) => [s.label, s] as const),
);

describe("TicketWorkspace — wave-1 products in the selector", () => {
  it("offers variance swap / volatility swap / Asian alongside vanilla", async () => {
    await renderTicket();
    const options = structureOptionIds();
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
      selectStructure("VARIANCE_SWAP");
    });
    expect(screen.getByLabelText("strike vol")).toBeInTheDocument();
    // No option legs are rendered for a swap.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/Fair variance strike K_var/)).toBeInTheDocument();
  });

  it("shows the strike-vol input when a volatility swap is selected", async () => {
    await renderTicket();
    act(() => {
      selectStructure("VOLATILITY_SWAP");
    });
    expect(screen.getByLabelText("strike vol")).toBeInTheDocument();
    expect(screen.getByText(/Fair volatility strike K_vol/)).toBeInTheDocument();
  });

  it("shows option type, averaging, fixings and method for an Asian", async () => {
    await renderTicket();
    act(() => {
      selectStructure("ASIAN");
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
      selectStructure("ASIAN");
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
    const options = structureOptionIds();
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
      selectStructure("SINGLE_BARRIER");
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
      selectStructure("DOUBLE_BARRIER");
    });
    expect(screen.getByLabelText("lower barrier")).toBeInTheDocument();
    expect(screen.getByLabelText("upper barrier")).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "barrier kind" })).toBeInTheDocument();
    expect(screen.getByText(/Double-barrier/)).toBeInTheDocument();
  });

  it("shows the settlement-style toggle and payout for a digital", async () => {
    await renderTicket();
    act(() => {
      selectStructure("DIGITAL");
    });
    const style = screen.getByRole("tablist", { name: "digital style" });
    expect(within(style).getByText("Cash")).toBeInTheDocument();
    expect(within(style).getByText("Asset")).toBeInTheDocument();
    expect(screen.getByLabelText("payout")).toBeInTheDocument();
    expect(productNote(/digital/)).toBeInTheDocument();
  });

  it("shows the touch-kind toggle and reveals the upper barrier only for double kinds", async () => {
    await renderTicket();
    act(() => {
      selectStructure("TOUCH");
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
    const options = structureOptionIds();
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
      selectStructure("FORWARD_START");
    });
    expect(screen.getByLabelText("moneyness")).toBeInTheDocument();
    expect(screen.getByLabelText("reset")).toBeInTheDocument();
    // No option legs are rendered for a forward start.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(productNote(/Forward-start vanilla/)).toBeInTheDocument();
  });

  it("shows the cliquet schedule + clamp toggles; MC-pairs appears only when clamped", async () => {
    await renderTicket();
    act(() => {
      selectStructure("CLIQUET");
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
      selectStructure("QUANTO");
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
    const options = structureOptionIds();
    expect(options).toContain("WINDOW_BARRIER");
    // The pre-existing products are untouched.
    expect(options).toContain("VANILLA");
    expect(options).toContain("SINGLE_BARRIER");
  });

  it("shows the booking-model selector with Default + Local-Stoch-Vol for a vanilla", async () => {
    await renderTicket();
    act(() => {
      selectStructure("VANILLA");
    });
    const model = screen.getByRole("tablist", { name: "booking model" });
    expect(within(model).getByText("Default")).toBeInTheDocument();
    expect(within(model).getByText("Local-Stoch-Vol")).toBeInTheDocument();
  });

  it("hides the booking-model selector for an LSV-unsupported product (digital)", async () => {
    await renderTicket();
    act(() => {
      selectStructure("DIGITAL");
    });
    // A digital has no LSV route ⇒ no booking-model choice is offered.
    expect(screen.queryByRole("tablist", { name: "booking model" })).not.toBeInTheDocument();
  });

  it("locks the window barrier to Local-Stoch-Vol (the only model) and shows its inputs", async () => {
    await renderTicket();
    act(() => {
      selectStructure("WINDOW_BARRIER");
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
      selectStructure("WINDOW_BARRIER");
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
      selectStructure("VANILLA");
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
    const options = structureOptionIds();
    expect(options).toContain("AMERICAN");
    // The pre-existing products are untouched.
    expect(options).toContain("VANILLA");
    expect(options).toContain("WINDOW_BARRIER");
  });

  it("shows option / strike / exercise-style for an American (no Bermudan dates field)", async () => {
    await renderTicket();
    act(() => {
      selectStructure("AMERICAN");
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
      selectStructure("AMERICAN");
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
      selectStructure("AMERICAN");
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
      selectStructure("AMERICAN");
    });
    // The offline binomial engine prices it (no LSV gate) ⇒ Request is enabled.
    const request = screen.getByRole("button", { name: /Request quote/ });
    expect((request as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("TicketWorkspace — correlated multi-asset basket / best-of / worst-of", () => {
  it("offers the basket builder in the structure selector (additive, zero-legacy)", async () => {
    await renderTicket();
    const options = structureOptionIds();
    expect(options).toContain("BASKET");
    // The pre-existing products are untouched.
    expect(options).toContain("VANILLA");
    expect(options).toContain("AMERICAN");
  });

  it("renders the leg builder (aggregation / two legs / correlation / strike / MC)", async () => {
    await renderTicket();
    act(() => {
      selectStructure("BASKET");
    });
    expect(screen.getByRole("tablist", { name: "basket kind" })).toBeInTheDocument();
    // The default basket has two legs, each with its own pair + market data.
    expect(screen.getByLabelText("leg 1 pair")).toBeInTheDocument();
    expect(screen.getByLabelText("leg 1 weight")).toBeInTheDocument();
    expect(screen.getByLabelText("leg 1 vol")).toBeInTheDocument();
    expect(screen.getByLabelText("leg 2 pair")).toBeInTheDocument();
    expect(screen.getByLabelText("correlation")).toBeInTheDocument();
    expect(screen.getByLabelText("mc paths")).toBeInTheDocument();
    // No option-leg ladder is rendered for a multi-asset product.
    expect(screen.queryByText(/^LEG 1$/)).not.toBeInTheDocument();
    expect(screen.getByText(/correlated currency-pair legs/)).toBeInTheDocument();
  });

  it("adds and removes a third leg within the 2–3 leg bounds", async () => {
    await renderTicket();
    act(() => {
      selectStructure("BASKET");
    });
    // Two legs by default; Remove is disabled at the 2-leg floor.
    expect(screen.queryByLabelText("leg 3 pair")).not.toBeInTheDocument();
    expect((screen.getByLabelText("remove leg 1") as HTMLButtonElement).disabled).toBe(true);
    // Add a third leg.
    act(() => {
      fireEvent.click(screen.getByLabelText("add leg"));
    });
    expect(screen.getByLabelText("leg 3 pair")).toBeInTheDocument();
    // At the 3-leg ceiling Add is disabled and Remove is enabled.
    expect((screen.getByLabelText("add leg") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByLabelText("remove leg 1") as HTMLButtonElement).disabled).toBe(false);
    // Remove it again.
    act(() => {
      fireEvent.click(screen.getByLabelText("remove leg 3"));
    });
    expect(screen.queryByLabelText("leg 3 pair")).not.toBeInTheDocument();
  });

  it("warns when the correlation makes the matrix non positive-definite", async () => {
    await renderTicket();
    act(() => {
      selectStructure("BASKET");
    });
    // A correlation at the +1 boundary is singular (not strictly positive-definite).
    act(() => {
      fireEvent.change(screen.getByLabelText("correlation"), { target: { value: "1" } });
    });
    expect(screen.getByText(/not positive-definite/)).toBeInTheDocument();
  });

  it("switches aggregation kind across basket / best-of / worst-of", async () => {
    await renderTicket();
    act(() => {
      selectStructure("BASKET");
    });
    const kind = screen.getByRole("tablist", { name: "basket kind" });
    act(() => {
      fireEvent.click(within(kind).getByText("Best-of"));
    });
    expect(screen.getByText(/Best-of \(rainbow max\)/)).toBeInTheDocument();
    act(() => {
      fireEvent.click(within(kind).getByText("Worst-of"));
    });
    expect(screen.getByText(/Worst-of \(rainbow min\)/)).toBeInTheDocument();
  });

  it("prices a basket offline through the real transport, surfacing a std-error", async () => {
    await renderTicket();
    act(() => {
      selectStructure("BASKET");
    });
    // The offline correlated MC prices it (no LSV gate) ⇒ Request is enabled.
    const request = screen.getByRole("button", { name: /Request quote/ });
    expect((request as HTMLButtonElement).disabled).toBe(false);
    await act(async () => {
      fireEvent.click(request);
    });
    // The basket is always Monte-Carlo ⇒ the honest std-error is shown.
    expect(await screen.findByLabelText("price std error")).toBeInTheDocument();
    expect(screen.getByText(/Monte-Carlo · std error/)).toBeInTheDocument();
  });
});
