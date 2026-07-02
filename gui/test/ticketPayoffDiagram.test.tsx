/**
 * TicketWorkspace payoff-preview wiring (GW2 → viz): the ticket renders the
 * multi-leg {@link PayoffDiagram} for a strategy ladder whose EVERY leg carries
 * a typed ABSOLUTE strike, and keeps the compact {@link PayoffChart} shape
 * preview everywhere else. Renders the REAL `TicketWorkspace` inside the REAL
 * `AppProvider` (offline mock transport via `?mock` — no server dialled) and
 * drives it through the DOM as a trader would:
 *   - the default risk reversal is delta-keyed (25dC / 25dP) — delta legs solve
 *     to a level server-side, so no honest client-side kink exists and the
 *     compact shape preview stays;
 *   - typing an absolute K on every leg of a straddle upgrades the preview to
 *     the true multi-leg diagram (auto-derived strategy name + kind chip);
 *   - re-keying a leg back to a delta entry honestly falls back to the compact
 *     shape preview again.
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

/** Set a leg's strike text (change commits a parsed entry immediately). */
function setStrike(legNo: number, raw: string): void {
  act(() => {
    fireEvent.change(screen.getByLabelText(`leg ${legNo} strike`), { target: { value: raw } });
  });
}

/** The multi-leg diagram's figure (aria-label `<name> payoff at expiry — net …`). */
function queryDiagram(name: string): HTMLElement | null {
  return screen.queryByRole("figure", {
    name: new RegExp(`^${name} payoff at expiry — net `),
  });
}

/** The compact shape preview's svg/empty-state (`<dir> <structure> payoff at expiry — a …`). */
function queryShapePreview(structure: string): HTMLElement | null {
  return screen.queryByRole("img", {
    name: new RegExp(`^(long|short) ${structure} payoff at expiry — a `),
  });
}

describe("TicketWorkspace — the multi-leg payoff diagram wiring", () => {
  it("keeps the compact shape preview for the delta-keyed default risk reversal", async () => {
    await renderTicket();
    // Delta legs (25dC / 25dP) resolve server-side — no honest client-side kink,
    // so the compact PayoffChart shape preview renders, never the diagram.
    expect(queryShapePreview("risk reversal")).toBeInTheDocument();
    expect(queryDiagram("Risk Reversal")).not.toBeInTheDocument();
  });

  it("upgrades to the multi-leg diagram once every straddle leg carries a typed absolute K", async () => {
    await renderTicket();
    selectStructure("STRADDLE");
    // The template defaults are ATM (delta-keyed): still the compact preview.
    expect(queryShapePreview("straddle")).toBeInTheDocument();
    setStrike(1, "1.0850");
    // One typed leg is not enough — the OTHER leg is still delta-keyed.
    expect(queryDiagram("Straddle")).not.toBeInTheDocument();
    setStrike(2, "1.0850");
    // Every leg now carries an absolute level: the true multi-leg diagram, with
    // the strategy auto-derived off the actual ladder (name + kind chip).
    const figure = queryDiagram("Straddle");
    expect(figure).toBeInTheDocument();
    expect(within(figure!).getByText("kind · STRADDLE")).toBeInTheDocument();
    expect(queryShapePreview("straddle")).not.toBeInTheDocument();
  });

  it("falls back to the compact shape preview when a leg re-keys to a delta entry", async () => {
    await renderTicket();
    selectStructure("STRANGLE");
    setStrike(1, "1.1000");
    setStrike(2, "1.0600");
    expect(queryDiagram("Strangle")).toBeInTheDocument();
    // Re-keying leg 1 to a convention delta drops the honest client-side kink.
    setStrike(1, "25dC");
    expect(queryDiagram("Strangle")).not.toBeInTheDocument();
    expect(queryShapePreview("strangle")).toBeInTheDocument();
  });
});
