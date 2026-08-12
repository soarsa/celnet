/**
 * Global select-on-focus for numeric fields, installed once at the document
 * level via event delegation so it covers every `type="number"` input in the
 * app — including inputs mounted later — without touching each of the ~180
 * call sites.
 *
 * The first keystroke then overwrites rather than appends to an existing value.
 * This no longer has anything to do with the leading-zero trap: an UNSET field
 * is now rendered EMPTY (with a `0` placeholder) by the `NumberField` drop-in,
 * so there is nothing there to select and the caret already starts at the left.
 * What remains is the case this still earns its keep for — replacing a REAL
 * value, e.g. clicking a field holding `5000000` and typing straight over it.
 *
 * ## Magnitude shorthand lives on the field, not here
 *
 * This module used to also intercept a `k`/`m`/`b` keystroke on ANY number
 * input and multiply the field in place. That was indiscriminate: it fired just
 * as readily on a volatility, a 0–1 band, or a basis-point spread, where
 * scaling by a billion is a footgun rather than a shortcut — and on the
 * `notionalMm` fields, which are already denominated in millions, `5m` silently
 * meant five trillion.
 *
 * Shorthand is now an explicit, opt-in property of the fields that should have
 * it ({@link ../components/MagnitudeField.MagnitudeField}), which parses on
 * commit, shows the resolved value before it is saved, and refuses input it
 * cannot parse instead of guessing.
 */

function isNumberInput(el: EventTarget | null): el is HTMLInputElement {
  return (
    el instanceof HTMLInputElement &&
    el.type === "number" &&
    !el.readOnly &&
    !el.disabled
  );
}

/**
 * Attach the select-on-focus affordance to `doc`.
 *
 * @returns a teardown function that removes the listeners.
 */
export function installNumberInputAutoSelect(doc: Document = document): () => void {
  const onFocusIn = (event: FocusEvent): void => {
    const target = event.target;
    if (!isNumberInput(target)) return;
    // Defer to the next frame so the browser's own caret placement (from a
    // mouse click) has already run; we then override it with a full selection.
    requestAnimationFrame(() => {
      if (doc.activeElement === target) target.select();
    });
  };

  doc.addEventListener("focusin", onFocusIn);
  return () => {
    doc.removeEventListener("focusin", onFocusIn);
  };
}
