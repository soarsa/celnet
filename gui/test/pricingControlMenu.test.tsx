/**
 * PricingControlMenu + PricingHaltBanner — the firm-wide kill-switch UI. Driven
 * through the REAL {@link PricingControlProvider} over a fake transport (so the
 * push + set round-trip and the version-gated banner are exercised), with `useApp`
 * mocked to control the capability gate and hand in the fake transport.
 *
 * Proves: the control is HIDDEN without `manage_liquidity·fixed_income` (but the
 * banner still shows to everyone); each destructive action requires an inline
 * confirm; the confirmed action sends the frozen-contract combination; and the
 * banner reflects the LIVE pushed state (warn on outbound-only, danger on all).
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { PricingControl } from "../src/data/contract";

// A minimal fake transport implementing only the pricing-control seam: it replays
// the current state on subscribe (connect-time push) and broadcasts on set.
function makeTransport(initial: PricingControl) {
  let control = initial;
  const subs = new Set<(c: PricingControl) => void>();
  const setPricingControl = vi.fn(async (outboundEnabled: boolean, inboundEnabled: boolean) => {
    control = { outboundEnabled, inboundEnabled, version: control.version + 1 };
    for (const cb of subs) cb(control);
    return control;
  });
  return {
    setPricingControl,
    subscribePricingControl(cb: (c: PricingControl) => void) {
      subs.add(cb);
      cb(control);
      return () => subs.delete(cb);
    },
  };
}

// The `useApp` stub the provider + menu read. `can` and `transport` are swapped
// per test via the mutable holder.
const holder: { can: boolean; transport: ReturnType<typeof makeTransport> } = {
  can: true,
  transport: makeTransport({ outboundEnabled: true, inboundEnabled: true, version: 1 }),
};
vi.mock("../src/app/AppContext", () => ({
  useApp: () => ({
    transport: holder.transport,
    auth: { can: () => holder.can },
  }),
}));

import { PricingControlProvider } from "../src/app/PricingControlProvider";
import { PricingControlMenu } from "../src/components/PricingControlMenu";
import { PricingHaltBanner } from "../src/components/PricingHaltBanner";

function mount(opts: { can: boolean; initial?: PricingControl }) {
  holder.can = opts.can;
  holder.transport = makeTransport(
    opts.initial ?? { outboundEnabled: true, inboundEnabled: true, version: 1 },
  );
  render(
    <PricingControlProvider>
      <PricingHaltBanner />
      <PricingControlMenu />
    </PricingControlProvider>,
  );
  return holder.transport;
}

afterEach(() => cleanup());

describe("PricingControlMenu — gating", () => {
  it("is hidden without manage_liquidity, but the banner still shows the halt to everyone", () => {
    mount({ can: false, initial: { outboundEnabled: false, inboundEnabled: true, version: 2 } });
    // No operator control for an unentitled user.
    expect(screen.queryByRole("button", { name: /Firm-wide pricing controls/i })).toBeNull();
    // …but the halt is visible to them (shown to all clients).
    expect(screen.getByText("OUTBOUND PRICING HALTED")).toBeTruthy();
  });

  it("renders the live-state trigger for an entitled operator", () => {
    mount({ can: true });
    expect(
      screen.getByRole("button", { name: /Firm-wide pricing controls — Pricing live/i }),
    ).toBeTruthy();
    // Nothing halted ⇒ no banner.
    expect(screen.queryByText(/HALTED/)).toBeNull();
  });
});

describe("PricingControlMenu — halt / resume round-trip", () => {
  it("Stop all pricing requires a confirm, then sends {outbound:false, inbound:true} and shows the warn banner", async () => {
    const transport = mount({ can: true });
    fireEvent.click(screen.getByRole("button", { name: /Firm-wide pricing controls/i }));
    // The confirm gate: the send must not fire on the first click.
    fireEvent.click(screen.getByRole("button", { name: /Stop all pricing/i }));
    expect(transport.setPricingControl).not.toHaveBeenCalled();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Confirm — stop pricing/i }));
    });
    expect(transport.setPricingControl).toHaveBeenCalledWith(false, true);
    await waitFor(() => expect(screen.getByText("OUTBOUND PRICING HALTED")).toBeTruthy());
    // The warn banner is a polite status (not an assertive alert).
    expect(screen.getByRole("status").textContent).toMatch(/clients are not being quoted/i);
    // The trigger reflects the live state.
    expect(screen.getByRole("button", { name: /Outbound halted/i })).toBeTruthy();
  });

  it("Stop all sends {outbound:false, inbound:false} and raises the danger alert banner", async () => {
    const transport = mount({ can: true });
    fireEvent.click(screen.getByRole("button", { name: /Firm-wide pricing controls/i }));
    fireEvent.click(screen.getByRole("button", { name: /AND inbound aggregation into the books/i }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Confirm — stop all/i }));
    });
    expect(transport.setPricingControl).toHaveBeenCalledWith(false, false);
    await waitFor(() => expect(screen.getByRole("alert").textContent).toMatch(/ALL PRICING HALTED/));
  });

  it("Resume is offered only when halted and clears the banner", async () => {
    const transport = mount({
      can: true,
      initial: { outboundEnabled: false, inboundEnabled: false, version: 4 },
    });
    // Banner is up for the halted initial state.
    expect(screen.getByRole("alert").textContent).toMatch(/ALL PRICING HALTED/);
    fireEvent.click(screen.getByRole("button", { name: /Firm-wide pricing controls/i }));
    await act(async () => {
      fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: /Resume pricing/i }));
    });
    expect(transport.setPricingControl).toHaveBeenCalledWith(true, true);
    await waitFor(() => expect(screen.queryByText(/HALTED/)).toBeNull());
  });

  it("does not offer Resume when pricing is live", () => {
    mount({ can: true });
    fireEvent.click(screen.getByRole("button", { name: /Firm-wide pricing controls/i }));
    expect(screen.queryByRole("button", { name: /Resume pricing/i })).toBeNull();
  });
});
