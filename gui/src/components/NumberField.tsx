import { useState } from "react";
import type { ChangeEvent, FocusEvent, InputHTMLAttributes } from "react";

/**
 * A drop-in replacement for `<input type="number">` that shows an EMPTY field
 * with a grey `0` placeholder instead of a literal `0`.
 *
 * Every numeric control in the app is bound to a number that defaults to `0`, so
 * the field rendered a real `0` and the caret landed *after* it — the user had to
 * delete the zero before typing, and a stray keystroke produced `05`. Rendering
 * the unset state as empty puts the caret at the left where it belongs and makes
 * the first keystroke the first digit.
 *
 * It is a drop-in on purpose: it accepts the full `<input>` prop surface and emits
 * the native change event unchanged, so existing `onChange={(e) =>
 * f(Number(e.target.value))}` handlers keep working verbatim — `Number("")` is `0`,
 * which is exactly the value an emptied field should report.
 *
 * ## Why a draft string
 *
 * Deriving the displayed text from the bound number alone cannot work: typing `0`
 * would set the value to `0`, which renders as empty, so a literal zero — and
 * anything starting with one, like `0.5` — would be untypeable. While the field is
 * focused it therefore owns a `draft` string and shows exactly what was typed,
 * including transient states (`""`, `"0"`, `"0."`, `"-"`) that are not yet a
 * number. On blur the draft is dropped and the display returns to the bound value.
 */
export type NumberFieldProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type">;

export function NumberField({
  value,
  placeholder = "0",
  onChange,
  onFocus,
  onBlur,
  ...rest
}: NumberFieldProps) {
  // `null` = not editing; the display is derived from the bound value.
  const [draft, setDraft] = useState<string | null>(null);

  // An unset field is one whose bound value is zero (or absent) — that is the
  // state we render as empty. A non-zero value always renders its digits.
  const bound = value === undefined || value === null ? "" : String(value);
  const display = draft ?? (Number(bound) === 0 ? "" : bound);

  const handleChange = (event: ChangeEvent<HTMLInputElement>): void => {
    setDraft(event.target.value);
    onChange?.(event);
  };

  const handleFocus = (event: FocusEvent<HTMLInputElement>): void => {
    // Seed the draft from what is currently shown, so an empty zero-valued field
    // stays empty on focus rather than snapping back to "0".
    setDraft(display);
    onFocus?.(event);
  };

  const handleBlur = (event: FocusEvent<HTMLInputElement>): void => {
    setDraft(null);
    onBlur?.(event);
  };

  return (
    <input
      {...rest}
      type="number"
      value={display}
      placeholder={placeholder}
      onChange={handleChange}
      onFocus={handleFocus}
      onBlur={handleBlur}
    />
  );
}
