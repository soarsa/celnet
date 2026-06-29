/**
 * ReferenceData family-aware form — the REAL `InstrumentForm` (no server).
 * Proves that picking a family from the family `<select>` swaps in exactly that
 * family's fields, and that submitting assembles the correct `InstrumentInput`
 * (blank `instrumentId` on create ⇒ server mints it; the chosen family's bag).
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { InstrumentInput } from "../src/data/contract";
import { InstrumentForm } from "../src/workspaces/ReferenceDataForms";

afterEach(() => {
  cleanup();
});

const noopRun = async (action: () => Promise<unknown>): Promise<void> => {
  await action();
};

describe("InstrumentForm", () => {
  it("renders only the selected family's fields and swaps on family change", () => {
    render(
      <InstrumentForm
        editing={null}
        onCreate={vi.fn(async () => undefined)}
        onUpdate={vi.fn(async () => undefined)}
        onDone={vi.fn()}
        run={noopRun}
      />,
    );

    // Defaults to OIS — its fields are present, the bond's are not.
    expect(screen.getByLabelText("Index")).toBeInTheDocument();
    expect(screen.queryByLabelText("Issuer")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Family"), { target: { value: "bond" } });

    // Now the bond fields show and the OIS-only field is gone.
    expect(screen.getByLabelText("Issuer")).toBeInTheDocument();
    expect(screen.queryByLabelText("Index")).not.toBeInTheDocument();
  });

  it("submits a correct InstrumentInput for the chosen family", async () => {
    const onCreate = vi.fn(async (_input: InstrumentInput) => undefined);
    render(
      <InstrumentForm
        editing={null}
        onCreate={onCreate}
        onUpdate={vi.fn(async () => undefined)}
        onDone={vi.fn()}
        run={noopRun}
      />,
    );

    fireEvent.change(screen.getByLabelText("Family"), { target: { value: "bond" } });
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "US Treasury 4% 2034" },
    });
    fireEvent.change(screen.getByLabelText("Issuer"), { target: { value: "US Treasury" } });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /create instrument/i }));
    });

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    const input = onCreate.mock.calls[0]![0];
    expect(input.family).toBe("bond");
    expect(input.instrumentId).toBe(""); // blank on create — server mints it
    expect(input.name).toBe("US Treasury 4% 2034");
    if (input.family === "bond") {
      expect(input.bond.issuer).toBe("US Treasury");
      expect(input.bond.calendars.length).toBeGreaterThan(0);
    }
  });
});
