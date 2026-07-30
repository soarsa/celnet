/**
 * Simulator (counterparty injector) — capability vocabulary, gating, and the
 * LIVE-injection guarantee.
 *
 * Proves:
 *  • `simulate` is in the canonical capability vocabulary and round-trips;
 *  • `can(...)` admits/denies `simulate·fixed_income` and the denial tooltip reads;
 *  • the SimulatorPanel injects a generated request into the live desk via the
 *    `submitDeskRequest` transport seam, with the mapped economics;
 *  • INJECTION guard (inverted): the panel source DOES call `submitDeskRequest`
 *    and no longer reaches for the offline `priceRatesOffline` sandbox pricer.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { CAPABILITY_ACTIONS } from "../src/data/contract";
import type { SubmitDeskRequestRequest } from "../src/data/contract";
import { can, capabilityDenialTitle } from "../src/lib/capabilityMatrix";
import { SimulatorPanel } from "../src/components/SimulatorPanel";

// A SPY desk-submit seam passed in as the panel's `transport` prop (the same shape
// Shell hands the live `app.transport`): we observe it to confirm the generated
// economics reach the canonical injection RPC.
const submitDeskRequest = vi.fn();
const transport = { submitDeskRequest };

afterEach(() => {
  document.body.innerHTML = "";
  submitDeskRequest.mockReset();
});

describe("simulate capability vocabulary", () => {
  it("includes `simulate` in the canonical action list (kernel parity)", () => {
    expect(CAPABILITY_ACTIONS).toContain("simulate");
    // Canonical order: …book, risk_transfer, simulate, administer (mirroring
    // `Action::ALL`, which inserts `RiskTransfer` between `Book` and `Simulate`).
    expect(CAPABILITY_ACTIONS.indexOf("risk_transfer")).toBe(
      CAPABILITY_ACTIONS.indexOf("book") + 1,
    );
    expect(CAPABILITY_ACTIONS.indexOf("simulate")).toBe(
      CAPABILITY_ACTIONS.indexOf("risk_transfer") + 1,
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

describe("SimulatorPanel — live desk injection", () => {
  it("injects a generated request into the desk with the mapped economics", async () => {
    submitDeskRequest.mockResolvedValue({
      request: {
        requestId: "rfq-101",
        kind: "RFQ",
        counterparty: "Acme Capital",
        instrument: { tenorYears: 5, fixedRate: 0.04, notional: 50_000_000, direction: "PAY_FIXED" },
        notional: 50_000_000,
        side: "BUY",
        state: "PENDING",
      },
    });

    await act(async () => {
      render(<SimulatorPanel transport={transport} onClose={() => {}} />);
    });
    const panel = screen.getByRole("region", { name: "Simulator" });
    // The note states the live-desk behavior — NOT a sandbox disclaimer.
    expect(within(panel).getByRole("note").textContent).toMatch(/injects into the live desk/i);
    expect(within(panel).getByRole("note").textContent).not.toMatch(/not sent to the desk/i);
    // No injected rows before generating.
    expect(within(panel).queryAllByRole("listitem")).toHaveLength(0);

    await act(async () => {
      fireEvent.click(within(panel).getByRole("button", { name: "Generate" }));
    });

    // The injection seam was called once with the mapped economics.
    expect(submitDeskRequest).toHaveBeenCalledTimes(1);
    const arg = submitDeskRequest.mock.calls[0]![0] as SubmitDeskRequestRequest;
    expect(arg.kind).toBe("RFQ");
    expect(arg.desk).toBe("g10-rates");
    expect(arg.side).toBe("BUY");
    expect(arg.notional).toBe(50_000_000);
    expect(arg.instrument.tenorYears).toBe(5);
    expect(arg.instrument.direction).toBe("PAY_FIXED");
    expect(arg.ttlMs).toBeGreaterThan(0);

    // The injected request renders with its server-minted id.
    const items = within(panel).queryAllByRole("listitem");
    expect(items).toHaveLength(1);
    expect(within(items[0]!).getByText("rfq-101")).toBeTruthy();
    expect(within(items[0]!).getByText("RFQ")).toBeTruthy();
  });

  it("surfaces an injection failure without inventing a row", async () => {
    submitDeskRequest.mockRejectedValue(new Error("desk unavailable"));
    await act(async () => {
      render(<SimulatorPanel transport={transport} onClose={() => {}} />);
    });
    const panel = screen.getByRole("region", { name: "Simulator" });
    await act(async () => {
      fireEvent.click(within(panel).getByRole("button", { name: "Generate" }));
    });
    expect(within(panel).getByRole("alert").textContent).toMatch(/desk unavailable/i);
    expect(within(panel).queryAllByRole("listitem")).toHaveLength(0);
  });
});

describe("SimulatorPanel — injection source guard (inverted)", () => {
  it("the panel source injects into the desk and drops the offline sandbox pricer", () => {
    const src = readFileSync(
      join(__dirname, "../src/components/SimulatorPanel.tsx"),
      "utf8",
    );
    // It MUST inject into the live desk via the canonical submit seam.
    expect(src).toMatch(/submitDeskRequest/);
    // The sandbox-only offline pricer is gone — generated items are no longer
    // priced locally; the live desk prices them.
    expect(src.includes("priceRatesOffline")).toBe(false);
    // And no "not sent to the desk" sandbox disclaimer remains.
    expect(src.toLowerCase().includes("not sent to the desk")).toBe(false);
  });
});
