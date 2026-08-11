/**
 * Global UX affordances for numeric fields, installed once at the document
 * level via event delegation so they cover every `type="number"` input in the
 * app — including inputs mounted later — without touching each of the ~180
 * call sites.
 *
 * 1. Select-on-focus, so the first keystroke overwrites rather than appends to an
 *    existing value. This no longer has anything to do with the leading-zero trap:
 *    an UNSET field is now rendered EMPTY (with a `0` placeholder) by the
 *    `NumberField` drop-in, so there is nothing there to select and the caret
 *    already starts at the left. What remains is the case this still earns its
 *    keep for — replacing a REAL value, e.g. clicking a field holding `5000000`
 *    and typing straight over it.
 *
 * 2. Magnitude shorthand. Traders enter large notionals with `k`/`m`/`b`
 *    suffixes: `5m` → 5,000,000, `1k` → 1,000, `2b` → 2,000,000,000. Native
 *    number inputs reject letter keys before any value change is visible, so we
 *    intercept the suffix keystroke, multiply the current value, and write it
 *    back through React's native value setter so the bound `onChange` fires.
 */

/** Suffix → multiplier. Case-insensitive; standard trading magnitudes. */
const MAGNITUDE: Readonly<Record<string, number>> = {
  k: 1_000,
  m: 1_000_000,
  b: 1_000_000_000,
};

function isNumberInput(el: EventTarget | null): el is HTMLInputElement {
  return (
    el instanceof HTMLInputElement &&
    el.type === "number" &&
    !el.readOnly &&
    !el.disabled
  );
}

/**
 * Write `value` into a React-controlled input so its `onChange` handler fires.
 * React installs its own value setter on the input node; calling the prototype
 * setter and dispatching a bubbling `input` event is the supported way to make
 * React observe a programmatic value change.
 */
function setControlledValue(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  if (setter) {
    setter.call(input, value);
  } else {
    input.value = value;
  }
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Attach the numeric-field affordances to `doc`.
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

  const onKeyDown = (event: KeyboardEvent): void => {
    const target = event.target;
    if (!isNumberInput(target)) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;

    const factor = MAGNITUDE[event.key.toLowerCase()];
    if (factor === undefined) return;

    const raw = target.value.trim();
    const base = raw === "" ? Number.NaN : Number(raw);
    if (!Number.isFinite(base)) {
      // Nothing sensible to scale — swallow the key so no stray letter lingers.
      event.preventDefault();
      return;
    }

    event.preventDefault();
    // `String()` renders plain decimals (no exponent) for any magnitude below
    // 1e21 — far above any real notional — and preserves fractional inputs like
    // `1.5m` → "1500000" without forced rounding.
    setControlledValue(target, String(base * factor));
  };

  doc.addEventListener("focusin", onFocusIn);
  doc.addEventListener("keydown", onKeyDown);
  return () => {
    doc.removeEventListener("focusin", onFocusIn);
    doc.removeEventListener("keydown", onKeyDown);
  };
}
