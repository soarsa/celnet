/**
 * TicketWorkspace round-2 leg-builder coverage: the EDITABLE vanilla/strategy
 * ladder, the structure-law gate, and the inline strike solve. Renders the REAL
 * `TicketWorkspace` inside the REAL `AppProvider` (offline mock transport via
 * `?mock` — no server dialled) and drives it through the DOM as a trader would:
 *   - the caption claims exactly what ships (the inline strike solve — delta /
 *     ATM legs price to a server-solved K echoed on the quote), with the old
 *     unqualified "Solve inline" claim gone;
 *   - a custom-strike vanilla (absolute level + put) is structurable and the
 *     priced quote echoes the resolved strike beside the two-way;
 *   - strategy legs are editable per leg (side / type / strike / ratio,
 *     add & remove) and the template structure laws gate Request quote with
 *     honest inline messages (typed in the editor module; display-ready here).
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { TicketWorkspace } from "../src/workspaces/TicketWorkspace";
import { specById } from "../src/products";

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
  await screen.findByRole("listbox", { name: "Structure catalogue" });
}

/** Select a structure by its id (click the gallery card with the spec's label). */
function selectStructure(id: string): void {
  const spec = specById(id)!;
  const card = within(screen.getByRole("listbox", { name: "Structure catalogue" }))
    .getAllByRole("option")
    .find((c) => (c.textContent ?? "").startsWith(spec.label));
  if (!card) throw new Error(`no gallery card for structure ${id} (label "${spec.label}")`);
  act(() => {
    fireEvent.click(card);
  });
}

/** The primary request button (its accessible name changes with the gate state). */
function requestButton(): HTMLButtonElement {
  const btn = screen
    .getAllByRole("button")
    .find((b) => /Request quote|Re-request|Fix structure|Pick a date/.test(b.textContent ?? ""));
  if (!btn) throw new Error("no primary request button");
  return btn as HTMLButtonElement;
}

/** Set a leg's strike text (change only — a draft stays live until blur). */
function setStrike(legNo: number, raw: string): void {
  act(() => {
    fireEvent.change(screen.getByLabelText(`leg ${legNo} strike`), { target: { value: raw } });
  });
}

/** Click a leg's call/put toggle. */
function toggleType(legNo: number, label: "Call" | "Put"): void {
  const tablist = screen.getByRole("tablist", { name: `leg ${legNo} option type` });
  act(() => {
    fireEvent.click(within(tablist).getByText(label));
  });
}

describe("TicketWorkspace — the caption claims exactly what ships", () => {
  it("advertises the inline strike solve (server-solved K echoed on the quote)", async () => {
    await renderTicket();
    expect(
      screen.getByText(/strike solve inline \(25dC \/ 25dP \/ ATM legs price to a server-solved K/),
    ).toBeInTheDocument();
    // The old unqualified claim is gone (no zero-cost premium solve ships).
    expect(screen.queryByText(/Solve inline/)).not.toBeInTheDocument();
  });
});

describe("TicketWorkspace — the editable strategy leg ladder", () => {
  it("renders the default risk reversal as editable legs (side / type / strike / ratio)", async () => {
    await renderTicket();
    // Default structure is RISK_REVERSAL: two editable legs.
    expect(screen.getByRole("tablist", { name: "leg 1 side" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "leg 2 side" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "leg 1 option type" })).toBeInTheDocument();
    expect(screen.getByLabelText("leg 1 strike")).toHaveValue("25dC");
    expect(screen.getByLabelText("leg 2 strike")).toHaveValue("25dP");
    expect(screen.getByLabelText("leg 1 ratio")).toHaveValue(1);
    expect(screen.getByLabelText("add leg")).toBeInTheDocument();
    expect(requestButton().textContent).toMatch(/Request quote/);
    expect(requestButton()).toBeEnabled();
  });

  it("gates Request quote on the template structure law with an honest message", async () => {
    await renderTicket();
    // Two calls is not a risk reversal: the law speaks and the request gates.
    toggleType(2, "Call");
    const violations = screen.getByLabelText("structure law violations");
    expect(violations.textContent).toMatch(/pairs a call against a put/);
    expect(requestButton().textContent).toMatch(/Fix structure/);
    expect(requestButton()).toBeDisabled();
    // Restoring the put restores the lawful template and the request.
    toggleType(2, "Put");
    expect(screen.queryByLabelText("structure law violations")).not.toBeInTheDocument();
    expect(requestButton()).toBeEnabled();
  });

  it("add/remove reshapes the ladder and the leg-count law gates until it is fixed", async () => {
    await renderTicket();
    act(() => {
      fireEvent.click(screen.getByLabelText("add leg"));
    });
    expect(screen.getByText("LEG 3")).toBeInTheDocument();
    expect(screen.getByLabelText("structure law violations").textContent).toMatch(
      /carries exactly 2 legs \(3 now\) — remove 1 leg/,
    );
    expect(requestButton()).toBeDisabled();
    act(() => {
      fireEvent.click(screen.getByLabelText("remove leg 3"));
    });
    expect(screen.queryByLabelText("structure law violations")).not.toBeInTheDocument();
    expect(requestButton()).toBeEnabled();
  });

  it("an unparseable strike entry shows its typed error inline and gates the request", async () => {
    await renderTicket();
    setStrike(1, "banana");
    const field = screen.getByLabelText("leg 1 strike");
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText(/`banana` is not a strike/)).toBeInTheDocument();
    expect(requestButton().textContent).toMatch(/Fix structure/);
    expect(requestButton()).toBeDisabled();
    // Blur abandons the draft: the display snaps back to the committed strike.
    act(() => {
      fireEvent.blur(field);
    });
    expect(screen.getByLabelText("leg 1 strike")).toHaveValue("25dC");
    expect(requestButton()).toBeEnabled();
  });

  it("a delta-letter mismatch is a typed, actionable error", async () => {
    await renderTicket();
    setStrike(1, "25dP"); // leg 1 is the bought call
    expect(screen.getByText(/keys the opposite type — this leg is a call/)).toBeInTheDocument();
    expect(requestButton()).toBeDisabled();
  });

  it("a straddle's legs must share one strike (both ATM by default; a wing breaks it)", async () => {
    await renderTicket();
    selectStructure("STRADDLE");
    expect(screen.getByLabelText("leg 1 strike")).toHaveValue("ATM");
    expect(screen.getByLabelText("leg 2 strike")).toHaveValue("ATM");
    setStrike(1, "25dC");
    expect(screen.getByLabelText("structure law violations").textContent).toMatch(
      /share one strike/,
    );
    expect(requestButton()).toBeDisabled();
  });

  it("prices an edited risk reversal and echoes the server-solved strike on the quote", async () => {
    await renderTicket();
    await act(async () => {
      fireEvent.click(requestButton());
    });
    // The delta-keyed legs were solved to a level; the headline (first-leg) K is
    // echoed beside the priced two-way — the inline strike solve, visible.
    const solved = await screen.findByLabelText("resolved strike");
    expect(solved.textContent).toMatch(/solved K \d+\.\d{4}/);
  });
});

describe("TicketWorkspace — the custom-strike vanilla", () => {
  it("structures a PUT at a custom absolute strike and prices it", async () => {
    await renderTicket();
    selectStructure("VANILLA");
    // Single leg: no side toggle, no ratio, no add/remove (the wire vanilla arm
    // carries exactly one payoff).
    expect(screen.queryByRole("tablist", { name: "leg 1 side" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("leg 1 ratio")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("add leg")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("remove leg 1")).not.toBeInTheDocument();
    // The default 25Δ call re-signs to the put pillar on toggle…
    toggleType(1, "Put");
    expect(screen.getByLabelText("leg 1 strike")).toHaveValue("25dP");
    // …and a typed absolute level books as-is.
    setStrike(1, "1.0850");
    await act(async () => {
      fireEvent.click(requestButton());
    });
    const solved = await screen.findByLabelText("resolved strike");
    expect(solved.textContent).toContain("1.0850");
  });

  it("ATM is a valid vanilla strike entry (the signed 50Δ pillar)", async () => {
    await renderTicket();
    selectStructure("VANILLA");
    setStrike(1, "ATM");
    expect(screen.queryByLabelText("structure law violations")).not.toBeInTheDocument();
    expect(requestButton()).toBeEnabled();
  });
});
