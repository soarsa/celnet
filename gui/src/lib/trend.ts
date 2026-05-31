/**
 * Shared label + mode helpers for the Stream lane (and any workspace that names a
 * tenor or offers a trend series). LABEL-ONLY: `tenorLabel` decides how a year
 * fraction READS (ON/TN/SN at the very short end, then W/M/Y) — it never resolves
 * a true settlement date (calendar-faithful date resolution is Phase 1 contract
 * work). The `TrendMode` table is the honest catalogue of what a trend strip
 * COULD plot; only PREMIUM is `available` today because every other series gates
 * on a market-history feed the current contract does not expose. The rest are
 * shown as disabled options, never as fabricated lines.
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

/**
 * The trend series a strip can plot. PREMIUM is the live mid/bid-offer the stream
 * already carries; every other mode needs a time-series feed (ATM term history,
 * RR/BF history, spot/forward ticks, vega/P&L attribution over time) that the
 * single current contract does not yet expose — so they are catalogued but gated.
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

export interface TrendModeSpec {
  id: TrendMode;
  /** Short label for a segmented control / menu. */
  label: string;
  /**
   * `true` once the contract can stream this series. Only PREMIUM is live today;
   * the rest gate on a market-history feed that does not exist yet, so they render
   * as disabled options (honest, never a fabricated line).
   */
  available: boolean;
}

/** The honest catalogue, in display order. Only PREMIUM is `available` today. */
export const TREND_MODES: readonly TrendModeSpec[] = [
  { id: "PREMIUM", label: "Premium", available: true },
  { id: "ATM_VOL", label: "ATM vol", available: false },
  { id: "RR", label: "Risk reversal", available: false },
  { id: "BF", label: "Butterfly", available: false },
  { id: "SPOT", label: "Spot", available: false },
  { id: "FORWARD", label: "Forward", available: false },
  { id: "VEGA", label: "Vega", available: false },
  { id: "PNL", label: "P&L", available: false },
] as const;

/** The default trend mode — the one series that is live today. */
export const DEFAULT_TREND_MODE: TrendMode = "PREMIUM";
