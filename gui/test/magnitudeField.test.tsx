import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { MagnitudeField } from "../src/components/MagnitudeField";

/** A minimal controlled host, mirroring how the real panels bind the field. */
function Host({
  initial = null,
  allowBlank = true,
  min,
  onValidityChange,
}: {
  initial?: number | null;
  allowBlank?: boolean;
  min?: number;
  onValidityChange?: (valid: boolean) => void;
}) {
  const [value, setValue] = useState<number | null>(initial);
  return (
    <>
      <MagnitudeField
        aria-label="Max DV01"
        value={value}
        onCommit={setValue}
        allowBlank={allowBlank}
        min={min}
        onValidityChange={onValidityChange}
      />
      {/* The committed value, observed the way a save payload would read it. */}
      <output data-testid="committed">{value === null ? "BLANK" : String(value)}</output>
    </>
  );
}

const field = (): HTMLInputElement => screen.getByLabelText("Max DV01") as HTMLInputElement;
const committed = (): string => screen.getByTestId("committed").textContent ?? "";

/** Type `text` into the field and commit it the way a trader would (blur). */
function enterAndCommit(text: string): void {
  const input = field();
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: text } });
  fireEvent.blur(input);
}

describe("MagnitudeField — the pre-trade limits entry path", () => {
  it("turns `1b` into 1000000000 and shows the trader the resolved digits", () => {
    render(<Host />);
    enterAndCommit("1b");

    expect(committed()).toBe("1000000000");
    // The echo: the field itself now reads the number that will be saved.
    expect(field().value).toBe("1000000000");
  });

  it("previews the resolved value BEFORE the trader commits", () => {
    render(<Host />);
    const input = field();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "5.5m" } });

    // Still uncommitted...
    expect(committed()).toBe("BLANK");
    // ...but the expansion is already on screen.
    expect(screen.getByText("= 5,500,000")).toBeTruthy();
  });

  it("commits on Enter as well as blur", () => {
    render(<Host />);
    const input = field();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "10k" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(committed()).toBe("10000");
  });

  it("handles the 100 billion limit from the screenshot", () => {
    render(<Host />);
    enterAndCommit("100b");
    expect(committed()).toBe("100000000000");
  });

  it("keeps plain numeric entry working exactly as before", () => {
    render(<Host />);
    enterAndCommit("100000000000");
    expect(committed()).toBe("100000000000");
    expect(field().value).toBe("100000000000");
  });
});

describe("MagnitudeField — invalid input is refused, never coerced", () => {
  it.each(["abc", "1x", "1mm", "--5", "1.2.3"])("refuses %j", (raw) => {
    render(<Host initial={250} />);
    enterAndCommit(raw);

    // The previously committed value is untouched — no 0, no null, no NaN.
    expect(committed()).toBe("250");
  });

  it("shows a specific, visible complaint", () => {
    render(<Host />);
    enterAndCommit("1x");

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("k = thousand");
    expect(field().getAttribute("aria-invalid")).toBe("true");
  });

  it("never silently reads 0 from nonsense", () => {
    render(<Host />);
    enterAndCommit("abc");
    expect(committed()).toBe("BLANK");
    expect(committed()).not.toBe("0");
  });

  it("reports invalidity so the form can block saving, then recovers", () => {
    const onValidityChange = vi.fn();
    render(<Host onValidityChange={onValidityChange} />);

    enterAndCommit("1x");
    expect(onValidityChange).toHaveBeenLastCalledWith(false);

    enterAndCommit("1b");
    expect(onValidityChange).toHaveBeenLastCalledWith(true);
    expect(committed()).toBe("1000000000");
  });

  it("clears the complaint as soon as the entry is edited", () => {
    render(<Host />);
    enterAndCommit("1x");
    expect(screen.queryByRole("alert")).not.toBeNull();

    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "1" } });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("MagnitudeField — blank keeps meaning 'unset'", () => {
  it("commits blank as null, not 0", () => {
    render(<Host initial={5_000} />);
    enterAndCommit("");

    expect(committed()).toBe("BLANK");
    expect(committed()).not.toBe("0");
  });

  it("renders an unset value as an empty field", () => {
    render(<Host initial={null} />);
    expect(field().value).toBe("");
  });

  it("refuses blank when the field is required", () => {
    render(<Host initial={5_000} allowBlank={false} />);
    enterAndCommit("");

    expect(committed()).toBe("5000");
    expect(screen.getByRole("alert").textContent).toContain("Enter a value");
  });
});

describe("MagnitudeField — bounds", () => {
  it("refuses a negative where the field forbids it", () => {
    render(<Host initial={100} min={0} />);
    enterAndCommit("-1m");

    expect(committed()).toBe("100");
    expect(screen.getByRole("alert").textContent).toContain("at least");
  });

  it("accepts a negative where no lower bound is set", () => {
    render(<Host />);
    enterAndCommit("-2.5b");
    expect(committed()).toBe("-2500000000");
  });

  it("accepts zero as a real, restrictive value", () => {
    render(<Host initial={1_000} min={0} />);
    enterAndCommit("0");
    expect(committed()).toBe("0");
  });
});

describe("MagnitudeField — editing affordances", () => {
  it("reverts the draft on Escape without committing", () => {
    render(<Host initial={1_000} />);
    const input = field();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "9b" } });
    fireEvent.keyDown(input, { key: "Escape" });

    expect(input.value).toBe("1000");
    expect(committed()).toBe("1000");
  });

  it("uses a text input so the suffix survives to be parsed", () => {
    render(<Host />);
    // A native number input would discard the "b" keystroke outright.
    expect(field().getAttribute("type")).toBe("text");
    expect(field().getAttribute("inputmode")).toBe("decimal");
  });
});
