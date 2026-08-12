/**
 * Shorthand magnitude entry for trader-facing numeric fields.
 *
 * A risk limit of one hundred billion is eleven zeros. Counting them by hand is
 * exactly how an order-of-magnitude error reaches production, so every quantity
 * field accepts the shorthand traders already speak: `1b`, `10k`, `5.5m`.
 *
 * ## The contract
 *
 * | suffix        | factor |
 * | ------------- | ------ |
 * | `k` / `K`     | 10^3   |
 * | `m` / `M`     | 10^6   |
 * | `b` / `B`     | 10^9   |
 * | `bn` / `BN`   | 10^9   |
 *
 * This is a strict SUPERSET of plain numeric entry: anything a native
 * `<input type="number">` accepts today — including exponent form (`1e9`) and a
 * leading sign — parses to exactly the same value it does today. Adding the
 * shorthand never changes what an existing keystroke means.
 *
 * ## Why a three-way result instead of `number`
 *
 * These fields carry risk limits, and `0` is a real, extremely restrictive
 * value while blank means "uncapped". A parser that folds bad input into either
 * one silently arms a live trading control with a number the trader never
 * typed. {@link parseMagnitude} therefore returns a discriminated union with no
 * numeric fallback: the caller cannot accidentally consume a rejected input,
 * because `kind: "error"` carries no `value` to read.
 *
 * ## Exactness
 *
 * Scaling is a decimal-point SHIFT over the digit string, not a float multiply.
 * `5.5m` re-points `55` to `5500000` and only then calls `Number`, so the result
 * is the exact integer 5500000 rather than a `5.5 * 1e6` rounding artefact.
 */

/** Suffix (lower-cased) → power-of-ten exponent it contributes. */
const SUFFIX_EXPONENT: ReadonlyMap<string, number> = new Map([
  ["k", 3],
  ["m", 6],
  ["b", 9],
  ["bn", 9],
]);

/** The suffixes offered to the user, in help text and error messages. */
export const MAGNITUDE_SUFFIX_HINT = "k = thousand, m = million, b (or bn) = billion";

/**
 * The outcome of parsing a shorthand numeric entry.
 *
 * `blank` is kept distinct from `ok` because a blank quantity field means
 * "unset" (in the pre-trade limits panel, "uncapped") — never zero.
 */
export type MagnitudeResult =
  | { readonly kind: "blank" }
  | { readonly kind: "ok"; readonly value: number }
  | { readonly kind: "error"; readonly message: string };

const ok = (value: number): MagnitudeResult => ({ kind: "ok", value });
const error = (message: string): MagnitudeResult => ({ kind: "error", message });

/** Well-formed en-US thousands grouping: `1`, `12`, `1,234`, `12,345,678`. */
const GROUPED_INTEGER = /^\d{1,3}(?:,\d{3})+$/;
/** A bare run of digits, ungrouped. */
const PLAIN_DIGITS = /^\d*$/;
/** Trailing exponent marker on the numeric body, e.g. the `e-3` of `1.5e-3`. */
const EXPONENT_TAIL = /[eE]([+-]?\d+)$/;
/** Splits a trimmed entry into sign, numeric body, and alphabetic suffix. */
const ENTRY = /^([+-]?)\s*(.*?)\s*([a-zA-Z]*)$/;

/**
 * Shift the decimal point of a digit string by `exponent` places and read the
 * result as a number. Pure string surgery, so no intermediate float rounding:
 * `("55", pointPos 1, exponent 6)` yields exactly `5500000`.
 *
 * @param digits    concatenated integer and fraction digits, no separators
 * @param pointPos  index in `digits` the decimal point currently sits before
 * @param exponent  places to move the point right (negative moves it left)
 */
function shiftDecimal(digits: string, pointPos: number, exponent: number): number {
  const target = pointPos + exponent;
  if (target >= digits.length) return Number(digits + "0".repeat(target - digits.length));
  if (target <= 0) return Number(`0.${"0".repeat(-target)}${digits}`);
  return Number(`${digits.slice(0, target)}.${digits.slice(target)}`);
}

/**
 * Parse a trader's numeric entry, with optional magnitude shorthand.
 *
 * Accepts: plain numbers (`100000000000`), decimals (`0.5`), exponent form
 * (`1e9`), a leading sign, well-formed thousands grouping (`1,000`), and the
 * `k`/`m`/`b`/`bn` suffixes in either case with optional space (`1 m`, `1M`).
 *
 * Rejects — never coerces — everything else: `abc`, `1x`, `1mm`, `--5`,
 * `1.2.3`, and malformed grouping like `1,00`.
 *
 * @param raw the field's literal text; surrounding whitespace is ignored
 */
export function parseMagnitude(raw: string): MagnitudeResult {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return { kind: "blank" };

  const entry = ENTRY.exec(trimmed);
  // `ENTRY` is total over any string (every group is optional), so a null match
  // is unreachable; the guard exists to keep the types honest rather than to
  // handle a real case. Groups are read with an explicit empty-string default
  // for the same reason — under `noUncheckedIndexedAccess` they are optional.
  if (entry === null) return error(`"${trimmed}" is not a number.`);
  const sign = entry[1] ?? "";
  const body = entry[2] ?? "";
  const suffixRaw = entry[3] ?? "";

  let exponent = 0;
  if (suffixRaw.length > 0) {
    const found = SUFFIX_EXPONENT.get(suffixRaw.toLowerCase());
    if (found === undefined) {
      return error(`"${suffixRaw}" is not a magnitude suffix — ${MAGNITUDE_SUFFIX_HINT}.`);
    }
    exponent = found;
  }

  if (body.length === 0) {
    return suffixRaw.length > 0
      ? error(`"${trimmed}" has no number before the "${suffixRaw}".`)
      : error(`"${trimmed}" is not a number.`);
  }

  // Exponent form (`1e9`) is passthrough compatibility with the native number
  // input. Combining it with a suffix (`1e9k`) is far more likely a typo than an
  // intent, so it is rejected rather than guessed at.
  let mantissa = body;
  const exponentTail = EXPONENT_TAIL.exec(body);
  if (exponentTail !== null) {
    if (suffixRaw.length > 0) {
      return error(`"${trimmed}" mixes exponent form with a "${suffixRaw}" suffix — use one or the other.`);
    }
    exponent = Number(exponentTail[1] ?? "0");
    mantissa = body.slice(0, exponentTail.index);
  }

  const parts = mantissa.split(".");
  if (parts.length > 2) return error(`"${trimmed}" has more than one decimal point.`);
  const intRaw = parts[0] ?? "";
  const fracRaw = parts[1] ?? "";

  if (!PLAIN_DIGITS.test(fracRaw)) return error(`"${trimmed}" is not a number.`);

  let intDigits = intRaw;
  if (intRaw.includes(",")) {
    if (!GROUPED_INTEGER.test(intRaw)) {
      return error(`"${trimmed}" has misplaced thousands separators.`);
    }
    intDigits = intRaw.replaceAll(",", "");
  } else if (!PLAIN_DIGITS.test(intRaw)) {
    return error(`"${trimmed}" is not a number.`);
  }

  if (intDigits.length + fracRaw.length === 0) {
    return error(`"${trimmed}" is not a number.`);
  }

  const magnitude = shiftDecimal(intDigits + fracRaw, intDigits.length, exponent);
  if (!Number.isFinite(magnitude)) return error(`"${trimmed}" is too large.`);

  // `-0` is normalised away: a limit of negative zero is a nonsense the rest of
  // the app should never have to think about.
  const signed = sign === "-" ? -magnitude : magnitude;
  return ok(signed === 0 ? 0 : signed);
}

/**
 * Render a committed value back into field text.
 *
 * Deliberately plain digits — no grouping — because this string is what the
 * field is re-seeded with, and it must parse back to the identical number.
 */
export function formatMagnitudeInput(value: number | null): string {
  return value === null ? "" : String(value);
}

/**
 * Render a resolved value for the confirmation the trader reads before saving,
 * e.g. `5.5m` → `5,500,000`. Grouped, never abbreviated and never in exponent
 * form: the whole point is that the digits are countable at a glance.
 */
export function formatMagnitudeEcho(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 20 }).format(value);
}
