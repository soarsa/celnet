import { useEffect, useId, useRef, useState } from "react";
import type { FocusEvent, KeyboardEvent } from "react";

import {
  MAGNITUDE_SUFFIX_HINT,
  formatMagnitudeEcho,
  formatMagnitudeInput,
  parseMagnitude,
} from "../lib/magnitude";
import styles from "./MagnitudeField.module.css";

/**
 * A quantity input that accepts trader magnitude shorthand — `1b`, `10k`,
 * `5.5m` — on top of ordinary numeric entry.
 *
 * ## Why this is not `NumberField`
 *
 * A native `<input type="number">` cannot hold the text `1b`: the browser
 * discards non-numeric keystrokes before any value change is observable, so the
 * shorthand can never be shown, checked, or confirmed. This field is therefore
 * `type="text"` with `inputMode="decimal"`, which keeps the numeric keypad on
 * touch devices while letting the suffix survive long enough to be parsed.
 *
 * It is deliberately opt-in per call site rather than a global upgrade of every
 * numeric input. `k`/`m`/`b` are meaningless on a percentage, a basis-point
 * spread, or a ratio, and silently scaling one of those by a billion is a far
 * worse outcome than not offering the shortcut. Only quantity fields — cash
 * notionals, DV01 caps, clip sizes — use this component.
 *
 * ## Commit, not keystroke
 *
 * Parsing happens on COMMIT (blur or Enter), never per keystroke, and the
 * resolved number is written back into the field so the trader reads the digits
 * they are actually saving. While the entry is still shorthand a live preview
 * sits beside the field, so `5.5m` is visibly `5,500,000` *before* commit. An
 * order-of-magnitude limit change must never happen invisibly.
 *
 * Invalid entries are refused, not coerced: nothing is committed, the parser's
 * specific complaint is shown, and `onValidityChange` lets the surrounding form
 * disable its save action. A field that quietly reads `0` — a real and severely
 * restrictive limit — or `null` (uncapped!) from a typo is a live risk bug.
 */
export interface MagnitudeFieldProps {
  /** Committed value; `null` renders blank. */
  readonly value: number | null;
  /**
   * Called only with a VALID committed value. Never called for an entry that
   * failed to parse, so the bound state always holds a number the trader saw.
   */
  readonly onCommit: (value: number | null) => void;
  /**
   * Whether a blank entry is meaningful (e.g. "uncapped"). When `false`, a
   * blank commit is reported as invalid rather than committed as `null`.
   * @default true
   */
  readonly allowBlank?: boolean | undefined;
  /** Rejected below this bound. Use `0` to keep a field non-negative. */
  readonly min?: number | undefined;
  /** Rejected above this bound. */
  readonly max?: number | undefined;
  /**
   * Notified whenever the field's entry becomes invalid or valid again, so the
   * enclosing form can block saving while a typo is on screen.
   */
  readonly onValidityChange?: ((valid: boolean) => void) | undefined;
  /**
   * Increment applied by the Up/Down arrow keys, mirroring the native number
   * spinner this field replaces. A `type="text"` input has no built-in stepper,
   * so the affordance is reimplemented rather than silently dropped.
   */
  readonly step?: number | undefined;
  readonly id?: string | undefined;
  readonly className?: string | undefined;
  readonly disabled?: boolean | undefined;
  readonly placeholder?: string | undefined;
  readonly "aria-label"?: string | undefined;
  readonly "aria-labelledby"?: string | undefined;
  readonly "data-testid"?: string | undefined;
}

/** Clamp check shared by the commit path; returns a message when out of range. */
function rangeError(value: number, min?: number, max?: number): string | null {
  if (min !== undefined && value < min) {
    return `Must be at least ${formatMagnitudeEcho(min)}.`;
  }
  if (max !== undefined && value > max) {
    return `Must be at most ${formatMagnitudeEcho(max)}.`;
  }
  return null;
}

export function MagnitudeField({
  value,
  onCommit,
  allowBlank = true,
  min,
  max,
  step,
  onValidityChange,
  id,
  className,
  disabled,
  placeholder,
  ...aria
}: MagnitudeFieldProps) {
  // `null` = not editing; the display is derived from the committed value, so
  // an external change (reset, load, another control) is picked up immediately.
  const [draft, setDraft] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const errorId = `${useId()}-error`;
  const hintId = `${useId()}-hint`;

  // Report validity transitions only, and never during render.
  const lastReported = useRef(true);
  const notify = useRef(onValidityChange);
  notify.current = onValidityChange;

  useEffect(() => {
    const valid = message === null;
    if (lastReported.current !== valid) {
      lastReported.current = valid;
      notify.current?.(valid);
    }
  }, [message]);

  // A field that unmounts while invalid must not leave the enclosing form's
  // save action disabled forever — the offending entry is gone with it.
  useEffect(
    () => () => {
      if (!lastReported.current) notify.current?.(true);
    },
    [],
  );

  const display = draft ?? formatMagnitudeInput(value);

  // The live preview: shown only while the draft is shorthand that resolves to
  // something the raw text does not already say, so a plain `1000` stays quiet.
  const parsed = draft === null ? null : parseMagnitude(draft);
  const preview =
    parsed !== null && parsed.kind === "ok" && String(parsed.value) !== draft?.trim()
      ? formatMagnitudeEcho(parsed.value)
      : null;

  const commit = (): void => {
    if (draft === null) return;
    const result = parseMagnitude(draft);

    if (result.kind === "blank") {
      if (!allowBlank) {
        setMessage("Enter a value.");
        return;
      }
      setMessage(null);
      setDraft(null);
      onCommit(null);
      return;
    }

    if (result.kind === "error") {
      setMessage(result.message);
      return;
    }

    const outOfRange = rangeError(result.value, min, max);
    if (outOfRange !== null) {
      setMessage(outOfRange);
      return;
    }

    setMessage(null);
    // Drop the draft so the field re-derives from the committed number: this is
    // the echo that turns `5.5m` into a visible `5500000`.
    setDraft(null);
    onCommit(result.value);
  };

  /**
   * Reimplement the native spinner: nudge by `step` and commit at once, the way
   * an `<input type="number">` arrow key does. Nudging an unparseable entry is
   * refused rather than silently restarting from zero.
   */
  const nudge = (direction: 1 | -1): void => {
    if (step === undefined) return;
    const source = draft ?? formatMagnitudeInput(value);
    const parsedSource = parseMagnitude(source);
    const base =
      parsedSource.kind === "ok" ? parsedSource.value : parsedSource.kind === "blank" ? 0 : null;
    if (base === null) return;

    // Step in the same decimal-exact way the parser scales, so repeated nudges
    // on a fractional step cannot accumulate float error.
    const next = parseMagnitude(String(base + direction * step));
    if (next.kind !== "ok") return;
    if (rangeError(next.value, min, max) !== null) return;

    setMessage(null);
    setDraft(null);
    onCommit(next.value);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setDraft(null);
      setMessage(null);
      return;
    }
    if (step !== undefined && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
      event.preventDefault();
      nudge(event.key === "ArrowUp" ? 1 : -1);
    }
  };

  const handleFocus = (event: FocusEvent<HTMLInputElement>): void => {
    setDraft(display);
    // Select the existing value so typing straight over it replaces rather than
    // appends — matching the app-wide behaviour for numeric inputs. An unset
    // field is empty, so the caret simply starts at the left, preserving the
    // left-cursor fix. Deferred a frame because a mouse click places its own
    // caret after focus fires, which would collapse an immediate selection.
    const input = event.target;
    requestAnimationFrame(() => {
      if (input.ownerDocument.activeElement === input) input.select();
    });
  };

  const describedBy = [message !== null ? errorId : null, preview !== null ? hintId : null]
    .filter((v): v is string => v !== null)
    .join(" ");

  return (
    <span className={styles.wrap}>
      <input
        {...aria}
        id={id}
        type="text"
        inputMode="decimal"
        autoComplete="off"
        spellCheck={false}
        className={[styles.input, className, message !== null ? styles.invalid : null]
          .filter((c): c is string => typeof c === "string" && c.length > 0)
          .join(" ")}
        value={display}
        disabled={disabled}
        placeholder={placeholder ?? "0"}
        aria-invalid={message !== null}
        aria-describedby={describedBy.length > 0 ? describedBy : undefined}
        onChange={(e) => {
          setDraft(e.target.value);
          // Clear a stale complaint as soon as the trader edits; the entry is
          // re-judged on the next commit, not mid-keystroke.
          if (message !== null) setMessage(null);
        }}
        onKeyDown={handleKeyDown}
        onFocus={handleFocus}
        onBlur={commit}
      />
      {preview !== null && (
        <span className={styles.preview} id={hintId}>
          = {preview}
        </span>
      )}
      {message !== null && (
        <span className={styles.error} id={errorId} role="alert">
          {message}
        </span>
      )}
    </span>
  );
}

/** The shorthand legend, for form-level help text beside a group of fields. */
export const MAGNITUDE_HELP = `Shorthand: ${MAGNITUDE_SUFFIX_HINT}. e.g. 1b = 1,000,000,000`;
