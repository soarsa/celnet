import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FixConnectionWizard } from "../src/components/FixConnectionWizard";
import type {
  CapabilityAction,
  CapabilityAsset,
  DeskDesc,
  FixConnection,
  FixConnectionSpec,
} from "../src/data/contract";

/**
 * The Dialect step must offer the two fixed-income dialect cards as ENABLED when
 * the caller holds the matching FI capability, and as DISABLED (with a tooltip,
 * never hidden) when they do not — the affordance-discipline the server enforces
 * on create. Driven against the presentational wizard with a stubbed `can`.
 */

const DESKS: readonly DeskDesc[] = [{ id: "g10", name: "G10 Rates" }];

afterEach(() => {
  document.body.innerHTML = "";
});

function renderWizard(can: (a: CapabilityAction, s: CapabilityAsset) => boolean) {
  const onCreate = vi.fn(
    async (spec: FixConnectionSpec): Promise<FixConnection> => ({
      id: "fi-1",
      name: spec.name,
      kind: spec.kind,
      bindAddr: spec.bindAddr,
      senderCompId: spec.senderCompId,
      targetCompId: spec.targetCompId,
      enabled: spec.enabled,
      running: false,
      boundAddr: "",
      desk: spec.desk ?? "",
    }),
  );
  render(
    <FixConnectionWizard
      open
      onClose={() => {}}
      onCreate={onCreate}
      existing={[]}
      desks={DESKS}
      can={can}
    />,
  );
  return { onCreate };
}

describe("FixConnectionWizard — fixed-income dialect cards", () => {
  it("enables both FI cards when the caller holds the FI capabilities", () => {
    renderWizard(() => true);
    expect(
      screen.getByRole("button", { name: /Fixed Income — Quote \(RFQ\)/ }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: /Fixed Income — Streaming \(RFS\)/ }),
    ).toBeEnabled();
  });

  it("disables the FI-streaming card (with a tooltip) when the caller lacks stream·fixed_income", () => {
    renderWizard((action, asset) => !(action === "stream" && asset === "fixed_income"));
    const streamCard = screen.getByRole("button", {
      name: /Fixed Income — Streaming \(RFS\)/,
    });
    expect(streamCard).toBeDisabled();
    expect(streamCard).toHaveAttribute("title", expect.stringContaining("stream"));
    // The quote card stays enabled — the gate is per-dialect.
    expect(
      screen.getByRole("button", { name: /Fixed Income — Quote \(RFQ\)/ }),
    ).toBeEnabled();
  });

  it("creates an FI-quote connection when the caller selects that dialect", async () => {
    const { onCreate } = renderWizard(() => true);

    fireEvent.click(screen.getByRole("button", { name: /Fixed Income — Quote \(RFQ\)/ }));
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → identity

    fireEvent.change(screen.getByPlaceholderText(/Bank A/), {
      target: { value: "Rates Venue" },
    });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "g10" } });
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → compids
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → review

    // The review step labels the chosen dialect (only the kind name, not the card).
    expect(screen.getByText("Fixed Income — Quote (RFQ)")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Create connection/ }));
    await waitFor(() =>
      expect(onCreate).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "FIXED_INCOME_QUOTE", desk: "g10" }),
      ),
    );
  });
});

describe("FixConnectionWizard — routing desk (optional)", () => {
  /** Advance from the (default OPTIONS) dialect step into the identity step. */
  function toIdentity(name: string): void {
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → identity
    fireEvent.change(screen.getByPlaceholderText(/Bank A/), { target: { value: name } });
  }

  it("shows each desk by NAME but submits its id", async () => {
    const { onCreate } = renderWizard(() => true);
    toIdentity("Named Venue");

    const select = screen.getByRole("combobox", { name: "Routing desk" });
    // The option label is the human name, not the id.
    expect(screen.getByRole("option", { name: "G10 Rates" })).toBeInTheDocument();
    fireEvent.change(select, { target: { value: "g10" } });

    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → compids
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → review
    fireEvent.click(screen.getByRole("button", { name: /Create connection/ }));
    await waitFor(() =>
      expect(onCreate).toHaveBeenCalledWith(expect.objectContaining({ desk: "g10" })),
    );
  });

  it("creates an UNROUTED connection (blank desk) and warns about it", async () => {
    const { onCreate } = renderWizard(() => true);
    toIdentity("Unrouted Venue");

    // Default selection is "— unrouted —" (blank), and the step surfaces a warning
    // without blocking progress (the routing desk is optional).
    expect(screen.getByRole("combobox", { name: "Routing desk" })).toHaveValue("");
    expect(screen.getByText(/Unrouted —/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → compids
    fireEvent.click(screen.getByRole("button", { name: /^Next$/ })); // → review
    fireEvent.click(screen.getByRole("button", { name: /Create connection/ }));
    await waitFor(() =>
      expect(onCreate).toHaveBeenCalledWith(expect.objectContaining({ desk: "" })),
    );
  });
});
