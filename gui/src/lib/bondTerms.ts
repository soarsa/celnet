/**
 * bondTerms — the client-side join between an aggregated-book composite line and
 * the instrument reference-data registry, plus the formatting of a bond's static
 * terms for display.
 *
 * The composite wire message (`AggregatedInstrument`) carries only identity
 * (`displayName`/`isin`/`cusip`) + prices — NOT the security's terms. The full
 * definitions are already loadable client-side via the reference-data surface
 * (`transport.listInstruments()`), each carrying a `BondDef` with issuer, coupon,
 * frequency, day-count and maturity. This module indexes those definitions and
 * resolves the terms for a composite line so the price tile can surface them,
 * with NO change to the proto / server / wire codec.
 *
 * Everything here is pure (no React, no I/O) so the join + formatting is a
 * directly testable seam.
 */

import type {
  AggregatedInstrument,
  BondDef,
  BrokenDate,
  Frequency,
  InstrumentDef,
  RatesDayCount,
} from "../data/contract";

/** A displayable bond-term row: a short label and its formatted value. */
export interface BondTermRow {
  /** Stable key for the React list + the `<dt>`/`<dd>` pairing. */
  key: string;
  /** The short field label (e.g. "Coupon"). */
  label: string;
  /** The formatted field value (e.g. "4.125%"). */
  value: string;
  /** Whether the value is a numeric/code token that should render monospace. */
  numeric: boolean;
}

/**
 * Index the BOND-family instrument definitions for the aggregated-book join.
 * Keyed by BOTH the canonical `instrumentId` and each external identifier value
 * (ISIN/CUSIP), so a composite line resolves whether the server keys it by its
 * internal id or by a market identifier. Non-bond families are skipped (only a
 * bond carries the coupon/maturity terms the tile renders). The canonical
 * `instrumentId` always wins — an external alias never clobbers a primary entry.
 */
export function indexBondDefs(
  defs: readonly InstrumentDef[],
): ReadonlyMap<string, BondDef> {
  const index = new Map<string, BondDef>();
  // First pass: canonical instrument ids (authoritative keys).
  for (const def of defs) {
    if (def.family !== "bond") continue;
    if (def.instrumentId) index.set(def.instrumentId, def.bond);
  }
  // Second pass: external aliases, never overwriting a canonical id entry.
  for (const def of defs) {
    if (def.family !== "bond") continue;
    for (const ext of def.externalIds) {
      if (ext.scheme !== "isin" && ext.scheme !== "cusip") continue;
      if (ext.value && !index.has(ext.value)) index.set(ext.value, def.bond);
    }
  }
  return index;
}

/**
 * Resolve the bond terms for a composite line: match on the canonical
 * `instrumentId` first, then fall back to the ISIN and CUSIP the composite
 * carries. Returns `null` when no bond definition matches (a non-bond line or an
 * unseeded instrument) — the caller then renders no extra terms.
 */
export function resolveBondDef(
  index: ReadonlyMap<string, BondDef>,
  instrument: Pick<AggregatedInstrument, "instrumentId" | "isin" | "cusip">,
): BondDef | null {
  return (
    (instrument.instrumentId ? index.get(instrument.instrumentId) : undefined) ??
    (instrument.isin ? index.get(instrument.isin) : undefined) ??
    (instrument.cusip ? index.get(instrument.cusip) : undefined) ??
    null
  );
}

const MONTHS: readonly string[] = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/** A `BrokenDate` as a compact human date, e.g. `{2035,2,15}` → "15 Feb 2035". */
export function formatMaturity(d: BrokenDate): string {
  const month = MONTHS[d.month - 1];
  const day = String(d.day).padStart(2, "0");
  if (month === undefined) {
    // Defensive: an out-of-range month degrades to an ISO date rather than NaN.
    return `${d.year}-${String(d.month).padStart(2, "0")}-${day}`;
  }
  return `${day} ${month} ${d.year}`;
}

/**
 * A registry coupon rate (stored as a PERCENT, e.g. 4.125 ⇒ 4.125%) as a trimmed
 * percent string: up to 3 decimals with trailing zeros removed (4.25 → "4.25%",
 * 4 → "4%"). A non-finite rate degrades to a dash.
 */
export function formatCouponPct(rate: number): string {
  if (!Number.isFinite(rate)) return "—";
  return `${Number.parseFloat(rate.toFixed(3))}%`;
}

const DAY_COUNT_LABELS: Readonly<Record<RatesDayCount, string>> = {
  act_360: "ACT/360",
  act_365_fixed: "ACT/365F",
  thirty_360_bond_basis: "30/360",
  act_act: "ACT/ACT",
};

const FREQUENCY_LABELS: Readonly<Record<Frequency, string>> = {
  annual: "Annual",
  semi_annual: "Semi-annual",
  quarterly: "Quarterly",
};

/** The coupon descriptor: a percent for a fixed coupon, else the coupon nature. */
function formatCoupon(bond: BondDef): string {
  switch (bond.couponType) {
    case "zero":
      return "Zero coupon";
    case "frn":
      return "Floating (FRN)";
    case "fixed":
      return formatCouponPct(bond.couponRate);
  }
}

/**
 * The ordered, displayable term rows for a bond: issuer, coupon, coupon
 * frequency, day-count and maturity. The issuer row is omitted when blank and
 * the frequency row when the bond carries none (a zero-coupon bond). Every row
 * is a plain label/value pair the tile renders as an accessible `<dl>`.
 */
export function bondTermRows(bond: BondDef): BondTermRow[] {
  const rows: BondTermRow[] = [];
  if (bond.issuer) {
    rows.push({ key: "issuer", label: "Issuer", value: bond.issuer, numeric: false });
  }
  rows.push({ key: "coupon", label: "Coupon", value: formatCoupon(bond), numeric: true });
  if (bond.couponFrequency) {
    rows.push({
      key: "frequency",
      label: "Frequency",
      value: FREQUENCY_LABELS[bond.couponFrequency],
      numeric: false,
    });
  }
  rows.push({
    key: "dayCount",
    label: "Day count",
    value: DAY_COUNT_LABELS[bond.dayCount],
    numeric: true,
  });
  rows.push({
    key: "maturity",
    label: "Maturity",
    value: formatMaturity(bond.maturityDate),
    numeric: true,
  });
  return rows;
}
