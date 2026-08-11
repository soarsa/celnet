/**
 * `NumberField` — the drop-in that renders an unset numeric field as EMPTY with a
 * grey `0` placeholder, so the caret sits at the left instead of behind a literal
 * `0` the user has to delete first.
 *
 * The behaviours pinned here are the ones that make it safe to swap in at ~180
 * call sites without touching their handlers: an emptied field must still report
 * `0` through the unchanged `Number(e.target.value)` contract, and a literal `0`
 * (and anything beginning with one, like `0.5`) must remain typeable despite zero
 * being the value that renders as empty.
 */

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

import { NumberField } from "../src/components/NumberField";

/** A controlled host mirroring how every call site binds the field. */
function Host({ initial = 0, onValue }: { initial?: number; onValue?: (v: number) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <NumberField
      aria-label="amount"
      value={value}
      onChange={(e) => {
        const next = Number(e.target.value);
        setValue(next);
        onValue?.(next);
      }}
    />
  );
}

describe("NumberField", () => {
  it("renders a zero value as an empty field with a 0 placeholder", () => {
    render(<Host initial={0} />);
    const input = screen.getByLabelText("amount") as HTMLInputElement;
    // Empty — so the caret lands at the left, which is the whole point.
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("0");
    // Still a number input: the spinner, keypad and k/m/b shorthand all key off this.
    expect(input.type).toBe("number");
  });

  it("renders a non-zero value as its digits, not a placeholder", () => {
    render(<Host initial={250} />);
    expect((screen.getByLabelText("amount") as HTMLInputElement).value).toBe("250");
  });

  it("reports the typed number through the unchanged onChange contract", () => {
    const onValue = vi.fn();
    render(<Host onValue={onValue} />);
    const input = screen.getByLabelText("amount");
    fireEvent.change(input, { target: { value: "5" } });
    // The first keystroke is the first digit — no leading zero to delete.
    expect(onValue).toHaveBeenLastCalledWith(5);
    expect((input as HTMLInputElement).value).toBe("5");
  });

  it("keeps a literal 0 typeable even though 0 renders as empty", () => {
    const onValue = vi.fn();
    render(<Host onValue={onValue} />);
    const input = screen.getByLabelText("amount");
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "0" } });
    // Without the focused draft this would round-trip 0 -> "" and be untypeable.
    expect((input as HTMLInputElement).value).toBe("0");
    expect(onValue).toHaveBeenLastCalledWith(0);
  });

  it("keeps a leading-zero decimal typeable", () => {
    const onValue = vi.fn();
    render(<Host onValue={onValue} />);
    const input = screen.getByLabelText("amount");
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "0" } });
    fireEvent.change(input, { target: { value: "0." } });
    fireEvent.change(input, { target: { value: "0.5" } });
    expect((input as HTMLInputElement).value).toBe("0.5");
    expect(onValue).toHaveBeenLastCalledWith(0.5);
  });

  it("reports 0 when the field is cleared, and stays empty after blur", () => {
    const onValue = vi.fn();
    render(<Host initial={42} onValue={onValue} />);
    const input = screen.getByLabelText("amount") as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "" } });
    // `Number("")` is 0 — exactly what an emptied field should report.
    expect(onValue).toHaveBeenLastCalledWith(0);
    fireEvent.blur(input);
    expect(input.value).toBe("");
  });

  it("forwards the caller's own focus and blur handlers", () => {
    const onFocus = vi.fn();
    const onBlur = vi.fn();
    render(<NumberField aria-label="amount" value={0} readOnly onFocus={onFocus} onBlur={onBlur} />);
    const input = screen.getByLabelText("amount");
    fireEvent.focus(input);
    fireEvent.blur(input);
    expect(onFocus).toHaveBeenCalledTimes(1);
    expect(onBlur).toHaveBeenCalledTimes(1);
  });

  it("lets a caller override the placeholder", () => {
    render(<NumberField aria-label="amount" value={0} placeholder="auto" readOnly />);
    expect((screen.getByLabelText("amount") as HTMLInputElement).placeholder).toBe("auto");
  });
});
