/**
 * Simulator (counterparty sandbox) — capability vocabulary, gating, and the
 * pure-client-sandbox guarantee.
 *
 * Proves:
 *  • `simulate` is in the canonical capability vocabulary and round-trips;
 *  • `can(...)` admits/denies `simulate·fixed_income` and the denial tooltip reads;
 *  • the SimulatorPanel generates sandbox items with a deterministic sample quote;
 *  • CRITICAL no-injection guard: the panel source touches NO transport/desk/
 *    pricing RPC — generated items can never enter the live priced desk flow.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { CAPABILITY_ACTIONS } from "../src/data/contract";
import { can, capabilityDenialTitle } from "../src/lib/capabilityMatrix";
import { SimulatorPanel } from "../src/components/SimulatorPanel";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("simulate capability vocabulary", () => {
  it("includes `simulate` in the canonical action list (kernel parity)", () => {
    expect(CAPABILITY_ACTIONS).toContain("simulate");
    // Canonical order: between `book` and `administer`, mirroring `Action::ALL`.
    expect(CAPABILITY_ACTIONS.indexOf("simulate")).toBe(
      CAPABILITY_ACTIONS.indexOf("book") + 1,
    );
    expect(CAPABILITY_ACTIONS.indexOf("administer")).toBe(
      CAPABILITY_ACTIONS.indexOf("simulate") + 1,
    );
  });

  it("`can` admits a held simulate cap and denies an empty set", () => {
    const held = [{ action: "simulate", asset: "fixed_income" } as const];
    expect(can(held, "simulate", "fixed_income")).toBe(true);
    expect(can(held, "simulate", "fx_options")).toBe(false);
    expect(can([], "simulate", "fixed_income")).toBe(false);
  });

  it("the denial tooltip explains the simulator (never a silent grey-out)", () => {
    const title = capabilityDenialTitle("simulate", "fixed_income");
    expect(title).toMatch(/permissions don't allow/i);
    expect(title).toMatch(/simulator/i);
  });
});

describe("SimulatorPanel — pure client sandbox", () => {
  it("generates a sandbox item with a deterministic sample quote", async () => {
    await act(async () => {
      render(<SimulatorPanel open onClose={() => {}} />);
    });
    const dialog = screen.getByRole("dialog", { name: "Simulator" });
    // The not-live banner is present and explicit.
    expect(within(dialog).getByRole("note").textContent).toMatch(/not sent to the desk/i);
    // No items before generating.
    expect(within(dialog).queryAllByRole("listitem")).toHaveLength(0);

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Generate" }));
    });

    const items = within(dialog).queryAllByRole("listitem");
    expect(items).toHaveLength(1);
    const item = items[0]!;
    expect(within(item).getByText("RFQ")).toBeTruthy();
    // A sample quote (the offline par rate) is shown for the generated item.
    expect(within(item).getByText(/sample quote/i)).toBeTruthy();
  });

  it("returns null when closed (ephemeral, mounts only when opened)", () => {
    const { container } = render(<SimulatorPanel open={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("NO-INJECTION: the panel source calls no transport / desk / pricing RPC", () => {
    const src = readFileSync(
      join(__dirname, "../src/components/SimulatorPanel.tsx"),
      "utf8",
    );
    // The sandbox must never touch the desk submit/respond/accept seam, the
    // generic transport, or any priced-flow RPC — generated items stay local.
    for (const forbidden of [
      "submitDeskRequest",
      "respondDeskRequest",
      "acceptDeskQuote",
      "listDeskRequests",
      "app.transport",
      "useApp",
      "priceRates(", // the transport pricing seam (the offline pricer is priceRatesOffline)
    ]) {
      expect(src.includes(forbidden)).toBe(false);
    }
    // It DOES use the deterministic in-browser pricer (a local sample quote).
    expect(src).toMatch(/priceRatesOffline/);
  });
});
