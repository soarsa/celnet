/**
 * InstrumentDialog — the create / edit modal that replaced the clipped inline
 * form in the Reference Data (instrument registry) workspace. These assert the
 * modal presentation contract: it renders the REAL `InstrumentForm` above the list
 * (as a labelled `role="dialog"`), a valid submit reaches `onCreate` and then
 * closes, and Esc / the Close button dismiss it without submitting — the behaviour
 * a height-constrained FI container previously made unreachable.
 */

import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { InstrumentDialog } from "../src/components/InstrumentDialog";

afterEach(() => {
  cleanup();
});

const noopRun = async (action: () => Promise<unknown>): Promise<void> => {
  await action();
};

function renderDialog(overrides: Partial<React.ComponentProps<typeof InstrumentDialog>> = {}) {
  const props = {
    open: true,
    editing: null,
    onClose: vi.fn(),
    onCreate: vi.fn(async () => undefined),
    onUpdate: vi.fn(async () => undefined),
    run: noopRun,
    ...overrides,
  } satisfies React.ComponentProps<typeof InstrumentDialog>;
  render(<InstrumentDialog {...props} />);
  return props;
}

describe("InstrumentDialog", () => {
  it("renders nothing when closed", () => {
    renderDialog({ open: false });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders the create form as a labelled modal dialog when open", () => {
    renderDialog();
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByRole("heading", { name: "New instrument" })).toBeInTheDocument();
    // The REAL InstrumentForm is mounted (its family select is present).
    expect(screen.getByLabelText("Family")).toBeInTheDocument();
  });

  it("titles the dialog for the edited instrument", () => {
    renderDialog({
      editing: {
        instrumentId: "OIS-USD-SOFR",
        name: "USD SOFR OIS",
        currency: "USD",
        family: "ois",
        externalIds: [],
        ois: { index: "SOFR", calendars: ["USNY"] },
      } as unknown as React.ComponentProps<typeof InstrumentDialog>["editing"],
    });
    expect(screen.getByRole("heading", { name: "Edit USD SOFR OIS" })).toBeInTheDocument();
  });

  it("submits a valid create through the form then closes", async () => {
    const onCreate = vi.fn(async () => undefined);
    const onClose = vi.fn();
    renderDialog({ onCreate, onClose });

    fireEvent.change(screen.getByLabelText("Family"), { target: { value: "bond" } });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "US Treasury 4% 2034" } });
    fireEvent.change(screen.getByLabelText("Issuer"), { target: { value: "US Treasury" } });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /create instrument/i }));
    });

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("surfaces the action-error banner in context", () => {
    renderDialog({ error: "instrument id already exists" });
    expect(screen.getByRole("alert").textContent ?? "").toMatch(/already exists/i);
  });

  it("closes on the Close button and on Escape", () => {
    const onClose = vi.fn();
    renderDialog({ onClose });

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
