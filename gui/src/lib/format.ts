/**
 * Locale-aware, convention-faithful formatting helpers. Numbers are always
 * rendered with tabular figures at the component layer (see design tokens); the
 * formatters here only decide digit counts and units. Canonical values are never
 * localized away — only labels are (GUI-DESIGN §7).
 */

import type {
  AtmConvention,
  Conventions,
  Cut,
  DeltaConvention,
  PremiumStyle,
  Side,
} from "../data/contract";

const PCT = new Intl.NumberFormat("en-US", {
  minimumFractionDigits: 3,
  maximumFractionDigits: 3,
});

const VOL = new Intl.NumberFormat("en-US", {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

const PIPS = new Intl.NumberFormat("en-US", {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

/** Descending magnitude thresholds for compact suffixing. */
const COMPACT_UNITS: ReadonlyArray<readonly [number, string]> = [
  [1e12, "t"],
  [1e9, "b"],
  [1e6, "m"],
  [1e3, "k"],
];

/**
 * Compact magnitude formatting for large *counting* quantities — notionals,
 * sizes, order/deal quantities, and whole-currency amounts. Renders a k/m/b/t
 * suffix with up to `maxFrac` fractional digits, trailing zeros trimmed:
 *   1_000_000 → "1m", 50_000_000 → "50m", 100_000 → "100k",
 *   1_250_000_000 → "1.25b", 1_234 → "1.23k", 750 → "750", 0 → "0".
 * Negatives keep their sign; non-finite values render as an em dash.
 *
 * NEVER use this for rates, prices, vols, deltas, or percentages — those keep
 * full precision (see {@link fmtPct} / {@link fmtVol} / {@link fmtPips}). It is
 * for magnitudes where "10m" reads better than "10,000,000".
 */
export function fmtCompact(value: number, maxFrac = 2): string {
  if (!Number.isFinite(value)) return "—";
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const trim = (s: string): string => s.replace(/\.?0+$/, "");
  for (const [scale, suffix] of COMPACT_UNITS) {
    if (abs >= scale) {
      return `${sign}${trim((abs / scale).toFixed(maxFrac))}${suffix}`;
    }
  }
  return `${sign}${Number.isInteger(abs) ? String(abs) : trim(abs.toFixed(maxFrac))}`;
}

/**
 * A premium quoted in a *percent* style (PERCENT_FOREIGN / PERCENT_DOMESTIC),
 * rendered as a true percent. The wire premium is a fraction of notional
 * (e.g. a 25Δ call at 0.0035 = 0.350% of foreign notional), so the value is
 * scaled ×100 to read in percent under a "%" unit label: 0.0035 → "0.350".
 * (Previously this printed the bare fraction "0.004" under a "%" column — a
 * 100× unit mismatch; the number and its unit now agree.)
 */
export function fmtPremiumPct(value: number): string {
  return PCT.format(value * 100);
}

/** A volatility in vol points, e.g. 0.0755 → "7.55". */
export function fmtVol(absVol: number): string {
  return VOL.format(absVol * 100);
}

/** A signed vol point (RR), e.g. -0.003 → "-0.30". */
export function fmtVolPoint(absVol: number): string {
  const v = absVol * 100;
  const s = VOL.format(Math.abs(v));
  return v < 0 ? `−${s}` : v > 0 ? `+${s}` : s;
}

/** A spot/strike level rendered to 4 decimals (5 for JPY-style is handled by pip). */
export function fmtRate(rate: number, decimals = 4): string {
  return rate.toFixed(decimals);
}

/** A signed Greek, fixed precision, with a true minus glyph. */
export function fmtSigned(value: number, decimals = 3): string {
  const s = Math.abs(value).toFixed(decimals);
  return value < 0 ? `−${s}` : value > 0 ? `+${s}` : ` ${s}`;
}

/** A delta as a percent for axis labels, e.g. 0.25 → "25". */
export function fmtDeltaPillar(delta: number): string {
  return Math.round(Math.abs(delta) * 100).toString();
}

/**
 * Magnitude-adaptive P&L so a *real* but small mark-to-market value stays visible
 * rather than rounding to a misleading "0". Large books read in `k`/`m`; sub-`k`
 * P&L (common for near-zero-cost structures like a risk-reversal, where a 2% spot
 * shock is genuinely ~hundreds of units on 10mm) reads in whole units. The sign is
 * an explicit diverging glyph; exact zero is the only value that shows "0".
 */
export function fmtPnlAdaptive(value: number): string {
  const sign = value < 0 ? "−" : value > 0 ? "+" : "";
  const a = Math.abs(value);
  if (a === 0) return "0";
  let body: string;
  if (a >= 1_000_000) body = `${(a / 1_000_000).toFixed(a >= 10_000_000 ? 0 : 1)}m`;
  else if (a >= 1_000) body = `${(a / 1_000).toFixed(a >= 10_000 ? 0 : 1)}k`;
  else if (a >= 1) body = `${Math.round(a)}`;
  else body = a.toFixed(2);
  return `${sign}${body}`;
}

/** Pips (domestic-pips premium style). */
export function fmtPips(value: number): string {
  return PIPS.format(value);
}

/** A nanosecond epoch to a 24h HH:MM:SS clock in the local zone. */
export function fmtClock(epochNanos: bigint): string {
  const ms = Number(epochNanos / 1_000_000n);
  const d = new Date(ms);
  return d.toLocaleTimeString("en-GB", { hour12: false });
}

/**
 * A latency in nanoseconds rendered to a human, scale-adaptive unit (ns / µs / ms).
 * Sub-µs stays in ns; sub-ms in µs (1 dp); otherwise ms (2 dp). Used for the
 * server-reported price-compute percentiles on the status ribbon. Pure integer
 * `bigint` in, so no precision is lost above the JS safe-integer range.
 */
export function fmtLatencyNanos(nanos: bigint): string {
  if (nanos < 1_000n) return `${nanos.toString()}ns`;
  if (nanos < 1_000_000n) return `${(Number(nanos) / 1_000).toFixed(1)}µs`;
  return `${(Number(nanos) / 1_000_000).toFixed(2)}ms`;
}

/** Seconds remaining until a nanosecond deadline, clamped at 0. */
export function secondsUntil(deadlineNanos: bigint, nowNanos: bigint): number {
  const remNs = deadlineNanos - nowNanos;
  if (remNs <= 0n) return 0;
  return Number(remNs / 1_000_000n) / 1000;
}

// --- convention chip labels (short, on-the-face) ----------------------------

export function deltaConvChip(c: DeltaConvention): string {
  switch (c) {
    case "SPOT_UNADJUSTED":
      return "spot Δ";
    case "FORWARD_UNADJUSTED":
      return "fwd Δ";
    case "SPOT_PREMIUM_ADJUSTED":
      return "spot Δ pa";
    case "FORWARD_PREMIUM_ADJUSTED":
      return "fwd Δ pa";
  }
}

export function atmConvChip(c: AtmConvention): string {
  return c === "ATM_FORWARD" ? "ATMF" : "DNS";
}

export function premiumChip(c: PremiumStyle): string {
  switch (c) {
    case "DOMESTIC_PIPS":
      return "dom pips";
    case "PERCENT_FOREIGN":
      return "% for";
    case "PERCENT_DOMESTIC":
      return "% dom";
    case "FOREIGN_PIPS":
      return "for pips";
  }
}

export function cutChip(c: Cut): string {
  return c === "NEW_YORK_1000" ? "NY 1000" : "TYO 1500";
}

/** The premium-style unit suffix shown next to a price (e.g. "EUR prem", "% for"). */
export function premiumUnit(c: Conventions): string {
  switch (c.premiumStyle) {
    case "PERCENT_FOREIGN":
      return "%";
    case "PERCENT_DOMESTIC":
      return "% dom";
    case "DOMESTIC_PIPS":
      return "pips";
    case "FOREIGN_PIPS":
      return "for pips";
  }
}

/** The verb a trader performs on a given side of a two-way. */
export function sideVerb(side: Side): string {
  switch (side) {
    case "BUY":
      return "Buy";
    case "SELL":
      return "Sell";
    case "TWO_WAY":
      return "Two-way";
  }
}
