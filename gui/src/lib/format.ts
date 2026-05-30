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

/** A premium in percent-of-notional, e.g. 0.155 → "0.155". */
export function fmtPremiumPct(value: number): string {
  return PCT.format(value);
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

/** P&L in thousands with a k suffix and diverging sign, e.g. -41200 → "−41k". */
export function fmtPnlK(value: number): string {
  const k = value / 1000;
  const rounded = Math.round(k);
  const s = `${Math.abs(rounded)}k`;
  return rounded < 0 ? `−${s}` : rounded > 0 ? `+${s}` : "0";
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
