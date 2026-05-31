/**
 * Shared label + mode helpers for the Stream lane (and any workspace that names a
 * tenor or offers a trend series). LABEL-ONLY: `tenorLabel` decides how a year
 * fraction READS (ON/TN/SN at the very short end, then W/M/Y) — it never resolves
 * a true settlement date (calendar-faithful date resolution is Phase 1 contract
 * work). The `TrendMode` table is the catalogue of what a trend strip can plot;
 * PREMIUM plots the row's OWN streamed premium mid; ATM_VOL/SPOT/RR/BF/FORWARD
 * are now LIVE because the contract exposes the market-series feed
 * (`MarketSeriesSubscribe`, served by celnet-server) — each maps to a
 * `MarketObservable` and is streamed as a real observed series, never fabricated.
 * VEGA/PNL remain gated (they need the position-fact store, a later phase).
 */

/** One business day as a year fraction on ACT/365 — the ON horizon boundary. */
const ONE_BIZ_DAY_YEARS = 1 / 365;
/** Tom-next / spot-next live in the ~2–3 calendar-day band off the short end. */
const TN_YEARS = 2 / 365;
const SN_YEARS = 3 / 365;
/** Tolerance so floating tenor arithmetic (e.g. 7/365) lands on its label. */
const EPS = 1e-9;

/**
 * Label a tenor by its year fraction (ACT/365), purpose-named and date-free:
 *
 * - `≤ 1 business day` horizon  → "ON" (overnight)
 * - the very short end where representable → "TN" (tom-next), "SN" (spot-next)
 * - `< 1 month`  → whole weeks "1W".."3W"
 * - `< 1 year`   → whole months "1M".."11M"
 * - otherwise    → years "1Y", "1.5Y" (one decimal only when not whole)
 *
 * This is a DISPLAY label only — it does not assert a settlement/expiry date.
 */
export function tenorLabel(years: number): string {
  if (!Number.isFinite(years) || years <= 0) return "—";
  if (years <= ONE_BIZ_DAY_YEARS + EPS) return "ON";
  if (years <= TN_YEARS + EPS) return "TN";
  if (years <= SN_YEARS + EPS) return "SN";

  const days = years * 365;
  // Short end: whole weeks up to (but not including) ~1 month.
  if (days < 28 - EPS) {
    const weeks = Math.round(days / 7);
    if (weeks >= 1) return `${weeks}W`;
  }
  // Months up to (but not including) a year.
  if (years < 1 - EPS) {
    const months = Math.round(years * 12);
    if (months >= 1) return `${Math.min(11, months)}M`;
  }
  // Years: whole when it rounds clean, else one decimal.
  const wholeYears = Math.round(years);
  if (Math.abs(years - wholeYears) < 0.02) return `${wholeYears}Y`;
  return `${years.toFixed(1)}Y`;
}

import type { MarketObservable, Tenor } from "../data/contract";

/**
 * The trend series a strip can plot. PREMIUM is the row's own streamed premium
 * mid; ATM_VOL/RR/BF/SPOT/FORWARD are streamed market observables (the
 * `MarketSeriesSubscribe` feed the contract now exposes); VEGA/PNL are gated on
 * the position-fact store (a later phase) and shown disabled.
 */
export type TrendMode =
  | "PREMIUM"
  | "ATM_VOL"
  | "RR"
  | "BF"
  | "SPOT"
  | "FORWARD"
  | "VEGA"
  | "PNL";

/** The natural unit a trend series is plotted in (for the tile's label). */
export type TrendUnit = "premium" | "vol" | "rate" | "vega" | "pnl";

export interface TrendModeSpec {
  id: TrendMode;
  /** Short label for a segmented control / menu. */
  label: string;
  /**
   * `true` once the contract can stream this series. PREMIUM and every
   * `MarketObservable`-backed mode are live; VEGA/PNL gate on the position-fact
   * store (a later phase) and render disabled (honest, never a fabricated line).
   */
  available: boolean;
  /** The natural unit the series is plotted in. */
  unit: TrendUnit;
  /**
   * The market observable this mode streams via `MarketSeriesSubscribe`, or
   * `null` for PREMIUM (the row's own mid, already streamed) and the gated modes.
   */
  observable: MarketObservable | null;
  /** True if the observable needs a signed delta wing (RR/BF). */
  needsDelta?: boolean;
}

/** The catalogue, in display order. PREMIUM + market observables are live. */
export const TREND_MODES: readonly TrendModeSpec[] = [
  { id: "PREMIUM", label: "Premium", available: true, unit: "premium", observable: null },
  { id: "ATM_VOL", label: "ATM vol", available: true, unit: "vol", observable: "ATM_VOL" },
  {
    id: "RR",
    label: "Risk reversal",
    available: true,
    unit: "vol",
    observable: "RISK_REVERSAL",
    needsDelta: true,
  },
  {
    id: "BF",
    label: "Butterfly",
    available: true,
    unit: "vol",
    observable: "BUTTERFLY",
    needsDelta: true,
  },
  { id: "SPOT", label: "Spot", available: true, unit: "rate", observable: "SPOT" },
  { id: "FORWARD", label: "Forward", available: true, unit: "rate", observable: "FORWARD" },
  { id: "VEGA", label: "Vega", available: false, unit: "vega", observable: null },
  { id: "PNL", label: "P&L", available: false, unit: "pnl", observable: null },
] as const;

/** Look up a trend-mode spec by id. */
export function trendModeSpec(id: TrendMode): TrendModeSpec {
  return TREND_MODES.find((m) => m.id === id) ?? TREND_MODES[0]!;
}

/** The default trend mode — the row's own premium, live with no extra feed. */
export const DEFAULT_TREND_MODE: TrendMode = "PREMIUM";

/** The standard delta wing for the RR/BF observable series (25Δ). */
export const TREND_WING_DELTA = 0.25;

/**
 * A representative `Tenor` for a market-series subscription from a year fraction —
 * the nearest standard label (ON/W/M/Y). Used so a trend series for a tenor-
 * dependent observable (ATM_VOL/RR/BF/FORWARD) pins a real pillar tenor; SPOT is
 * tenor-independent and passes no tenor.
 */
export function tenorForYears(years: number): Tenor {
  if (!Number.isFinite(years) || years <= ONE_BIZ_DAY_YEARS + EPS) {
    return { unit: "OVERNIGHT", count: 1 };
  }
  const days = years * 365;
  if (days < 28 - EPS) return { unit: "WEEKS", count: Math.max(1, Math.round(days / 7)) };
  if (years < 1 - EPS) return { unit: "MONTHS", count: Math.max(1, Math.round(years * 12)) };
  return { unit: "YEARS", count: Math.max(1, Math.round(years)) };
}

/** A year fraction for a `Tenor` (the short-end units map to their day band). */
export function tenorYearsOf(t: Tenor): number {
  switch (t.unit) {
    case "OVERNIGHT":
      return ONE_BIZ_DAY_YEARS;
    case "TOM_NEXT":
      return TN_YEARS;
    case "SPOT_NEXT":
      return SN_YEARS;
    case "WEEKS":
      return (t.count * 7) / 365;
    case "MONTHS":
      return t.count / 12;
    case "YEARS":
      return t.count;
    case "IMM":
      // The n-th quarterly IMM ≈ n quarters out (display approximation; the server
      // resolves the true 3rd-Wednesday date).
      return Math.max(1, t.count) * 0.25;
    case "BROKEN_DATE":
      // No clock here to resolve a date to a year fraction; default to ~1M so a
      // series still pins a sensible pillar. The server resolves the real date.
      return 1 / 12;
  }
}
