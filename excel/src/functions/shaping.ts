/**
 * Pure request-shaping + dynamic-array formatting for the CELNET.* functions.
 *
 * These functions are deliberately side-effect-free: they translate the
 * Excel-cell argument strings a trader types (`"EURUSD"`, `"1Y"`, `"25dP"`,
 * `"C"`) into the ONE current contract's typed shapes (Instrument / Conventions),
 * and format contract results into the dynamic-array geometries Excel spills.
 * No transport, no Office globals — so they are exhaustively unit-testable under
 * node with no server and no mocks of our own functionality (CLAUDE.md: verify
 * against the real contract shapes, not fakes).
 *
 * Convention transparency (docs/EXCEL-INTEGRATION.md §3.4): every spill carries a
 * footer row with the resolved convention + surface_version + timestamp, so a
 * number never appears in the grid without the convention that produced it — the
 * #1 cure for FX-options mismarks.
 */

import type {
  CcyPair,
  Conventions,
  Greeks,
  Instrument,
  OptionType,
  Side,
  StrikeOrDelta,
  Tenor,
  TenorUnit,
} from "../contract/contract";

/** The canonical desk default convention (spot-unadjusted Δ / ATM-forward / …),
 * matching the server fixture (`crates/celnet-server/tests/common` wire_conventions).
 * A trader rarely overrides it; an explicit `conv` range can replace any field. */
export const DEFAULT_CONVENTIONS: Conventions = {
  deltaConvention: "SPOT_UNADJUSTED",
  atmConvention: "ATM_FORWARD",
  premiumStyle: "DOMESTIC_PIPS",
  cut: "NEW_YORK_1000",
  dayCount: "ACT_365_FIXED",
  settlement: "DELIVERABLE",
};

/** A shaping error: a malformed argument the trader typed. Carried to the cell. */
export class ShapingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ShapingError";
  }
}

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

/** Parse a `"EURUSD"` or `"EUR/USD"` pair string into a `CcyPair`. */
export function parsePair(raw: string): CcyPair {
  const s = raw.trim().toUpperCase().replace("/", "");
  if (s.length !== 6 || !/^[A-Z]{6}$/.test(s)) {
    throw new ShapingError(`invalid pair \`${raw}\` (expected e.g. EURUSD or EUR/USD)`);
  }
  return { base: s.slice(0, 3), quote: s.slice(3, 6) };
}

/** Years-per-unit for converting a parsed tenor to the contract `expiryYears`. */
const YEARS_PER_UNIT: Record<TenorUnit, number> = {
  OVERNIGHT: 1 / 365,
  WEEKS: 7 / 365,
  MONTHS: 1 / 12,
  YEARS: 1,
};

/** A parsed tenor: the structured `Tenor` plus its expiry year fraction. */
export interface ParsedTenor {
  readonly tenor: Tenor;
  readonly expiryYears: number;
}

/**
 * Parse a tenor string into the contract `Tenor` + expiry year fraction.
 * Accepts `ON`/`O/N` (overnight), `<n>W`, `<n>M`, `<n>Y` (case-insensitive).
 */
export function parseTenor(raw: string): ParsedTenor {
  const s = raw.trim().toUpperCase();
  if (s === "ON" || s === "O/N") {
    return { tenor: { unit: "OVERNIGHT", count: 1 }, expiryYears: YEARS_PER_UNIT.OVERNIGHT };
  }
  const m = /^(\d+)\s*([WMY])$/.exec(s);
  if (!m) {
    throw new ShapingError(`invalid tenor \`${raw}\` (expected ON, 1W, 3M, 1Y, …)`);
  }
  const count = Number(m[1]);
  const unit: TenorUnit = m[2] === "W" ? "WEEKS" : m[2] === "M" ? "MONTHS" : "YEARS";
  if (count <= 0) throw new ShapingError(`tenor count must be positive in \`${raw}\``);
  return { tenor: { unit, count }, expiryYears: count * YEARS_PER_UNIT[unit] };
}

/** Parse a `"C"`/`"P"` (or CALL/PUT) string into the contract `OptionType`. */
export function parseOptionType(raw: string): OptionType {
  const s = raw.trim().toUpperCase();
  if (s === "C" || s === "CALL") return "CALL";
  if (s === "P" || s === "PUT") return "PUT";
  throw new ShapingError(`invalid call/put \`${raw}\` (expected C or P)`);
}

/**
 * Parse a strike-or-delta argument. Accepts:
 *  - a number (absolute strike), e.g. `1.12`;
 *  - a delta string `"<n>dC"`/`"<n>dP"` (e.g. `"25dP"` ⇒ -0.25, `"25dC"` ⇒ +0.25);
 *  - `"ATM"` / `"DNS"` (delta-neutral) ⇒ the convention's ATM strike (delta 0).
 * The sign convention matches the contract `SmilePoint.delta` (call +, put −).
 */
export function parseStrikeOrDelta(raw: string | number): StrikeOrDelta {
  if (typeof raw === "number") {
    if (!Number.isFinite(raw) || raw <= 0) {
      throw new ShapingError(`invalid absolute strike \`${raw}\``);
    }
    return { kind: "strike", strike: raw };
  }
  const s = raw.trim().toUpperCase();
  if (s === "ATM" || s === "DNS") return { kind: "delta", delta: 0 };
  const numeric = Number(s);
  if (Number.isFinite(numeric) && numeric > 0) return { kind: "strike", strike: numeric };
  const m = /^(\d+(?:\.\d+)?)\s*D\s*([CP])$/.exec(s);
  if (!m) {
    throw new ShapingError(`invalid strike/delta \`${raw}\` (expected 1.12, 25dP, 10dC, ATM, DNS)`);
  }
  const pct = Number(m[1]);
  if (pct <= 0 || pct >= 100) throw new ShapingError(`delta percent out of range in \`${raw}\``);
  const magnitude = pct / 100;
  const signed = m[2] === "C" ? magnitude : -magnitude;
  return { kind: "delta", delta: signed };
}

// ---------------------------------------------------------------------------
// instrument shaping (one current contract)
// ---------------------------------------------------------------------------

/** The fully-parsed inputs a vanilla CELNET.* function shapes into a request. */
export interface VanillaArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
}

/**
 * Shape a vanilla instrument from the cell arguments. `side` is TWO_WAY (the cell
 * reads a market, not a directional ticket); a directional booking goes through the
 * task-pane Trade button. The notional is in the base/foreign currency (CCY1).
 */
export function shapeVanillaInstrument(args: VanillaArgs): Instrument {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const { tenor, expiryYears } = parseTenor(args.tenor);
  return {
    pair: parsePair(args.pair),
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "vanilla",
      vanilla: {
        optionType: parseOptionType(args.callPut),
        strike: parseStrikeOrDelta(args.strikeOrDelta),
      },
    },
  };
}

// ---------------------------------------------------------------------------
// dynamic-array formatting (spill geometries)
// ---------------------------------------------------------------------------

/** A cell matrix Excel spills: rows of (string | number) cells. */
export type SpillMatrix = (string | number)[][];

/**
 * The 13-Greek vector in the canonical contract order (the celnet-vanilla set,
 * `Greeks` field order in `celnet.proto` / contract.ts). `price` is the premium
 * and is surfaced separately by CELNET.PRICE; CELNET.GREEKS spills the 13 risk
 * Greeks (delta_spot … color) so the spill is exactly 13 rows, as the design says.
 */
export const GREEK_ROWS: readonly { readonly label: string; readonly key: keyof Greeks }[] = [
  { label: "delta_spot", key: "deltaSpot" },
  { label: "delta_forward", key: "deltaForward" },
  { label: "gamma", key: "gamma" },
  { label: "vega", key: "vega" },
  { label: "theta", key: "theta" },
  { label: "rho_dom", key: "rhoDom" },
  { label: "rho_for", key: "rhoFor" },
  { label: "vanna", key: "vanna" },
  { label: "volga", key: "volga" },
  { label: "charm", key: "charm" },
  { label: "speed", key: "speed" },
  { label: "zomma", key: "zomma" },
  { label: "color", key: "color" },
];

/**
 * Format the convention footer string stamped on every spill — the resolved
 * convention + surface_version + capture time (docs §3.4). Vendor-neutral, terse.
 */
export function conventionFooter(
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): string {
  const ver = surfaceVersion === undefined ? "live" : `v${surfaceVersion}`;
  const ms = Number(epochNanos / 1_000_000n);
  const iso = epochNanos === 0n ? "—" : new Date(ms).toISOString();
  return (
    `conv: Δ ${conv.deltaConvention} | ATM ${conv.atmConvention} | ` +
    `${conv.premiumStyle} | ${conv.dayCount} | ${conv.settlement} | ` +
    `surface ${ver} | t=${iso}`
  );
}

/**
 * Format CELNET.GREEKS as a vertical 13×2 spill `[name, value]`, plus a footer
 * row carrying the convention transparency. Returns a `(rows)×2` matrix.
 */
export function formatGreeksSpill(
  greeks: Greeks,
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const rows: SpillMatrix = GREEK_ROWS.map((r) => [r.label, greeks[r.key]]);
  rows.push([conventionFooter(conv, surfaceVersion, epochNanos), ""]);
  return rows;
}

/** A decoded smile point for surface formatting (delta, vol). */
export interface SurfacePoint {
  readonly delta: number;
  readonly vol: number;
}

/**
 * Format a single-tenor smile row as a spill: a header row of delta pillars and a
 * value row of vols, plus an arb-status + convention footer. Points are sorted by
 * signed delta (puts negative … calls positive) for a stable left-to-right grid.
 */
export function formatSmileSpill(
  points: readonly SurfacePoint[],
  arbFree: boolean,
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const sorted = [...points].sort((a, b) => a.delta - b.delta);
  const header: (string | number)[] = ["delta", ...sorted.map((p) => p.delta)];
  const vols: (string | number)[] = ["vol", ...sorted.map((p) => p.vol)];
  const footer: (string | number)[] = [
    `${arbFree ? "arb-free" : "ARB!"} | ${conventionFooter(conv, surfaceVersion, epochNanos)}`,
  ];
  return [header, vols, footer];
}

/**
 * Format a multi-tenor surface cube as a spill grid: a header row of delta
 * pillars, one row per tenor `[tenorYears, vol@pillar…]`, plus a footer. The
 * pillar set is the union of deltas across tenors, sorted ascending; a missing
 * (tenor, pillar) cell is left blank.
 */
export function formatSurfaceCubeSpill(
  tenors: readonly { readonly tenorYears: number; readonly points: readonly SurfacePoint[] }[],
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const pillarSet = new Set<number>();
  for (const t of tenors) for (const p of t.points) pillarSet.add(p.delta);
  const pillars = [...pillarSet].sort((a, b) => a - b);
  const header: (string | number)[] = ["tenor\\delta", ...pillars];
  const rows: SpillMatrix = [header];
  for (const t of tenors) {
    const byDelta = new Map(t.points.map((p) => [p.delta, p.vol]));
    const row: (string | number)[] = [t.tenorYears];
    for (const d of pillars) {
      const vol = byDelta.get(d);
      row.push(vol === undefined ? "" : vol);
    }
    rows.push(row);
  }
  rows.push([conventionFooter(conv, surfaceVersion, epochNanos)]);
  return rows;
}

/** The decoded fields a CELNET.RFQ spill renders. */
export interface RfqResult {
  readonly bid: number;
  readonly offer: number;
  readonly quoteId: bigint;
  readonly validUntilNanos: bigint;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.RFQ as a 1×4 horizontal spill `[bid, offer, quoteId, validUntil]`
 * followed by a convention footer row. `quoteId`/`validUntil` are rendered as
 * strings to avoid JS number precision loss on the 64-bit wire ids.
 */
export function formatRfqSpill(r: RfqResult): SpillMatrix {
  const validIso = r.validUntilNanos === 0n
    ? "—"
    : new Date(Number(r.validUntilNanos / 1_000_000n)).toISOString();
  return [
    [r.bid, r.offer, r.quoteId.toString(), validIso],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ];
}

/** The decoded fields a CELNET.MARK status spill renders (two-phase staging). */
export interface MarkStatus {
  readonly status: "PENDING" | "COMMITTED" | "REJECTED";
  readonly stagingId: string;
  readonly surfaceVersionAfter: bigint | undefined;
  readonly detail: string;
}

/** Format CELNET.MARK as a 1×3 status spill `[status, surfaceVersionAfter, detail]`. */
export function formatMarkStatusSpill(m: MarkStatus): SpillMatrix {
  return [[m.status, m.surfaceVersionAfter === undefined ? m.stagingId : `v${m.surfaceVersionAfter}`, m.detail]];
}

// ---------------------------------------------------------------------------
// streaming dedup / coalescing — the multiplex-key for shared subscriptions
// ---------------------------------------------------------------------------

/**
 * The coalescing key for a streamed cell: identical-argument cells share ONE
 * subscription on the single multiplexed session (the add-in reference-counts
 * subscribers, docs §3.2). The key is the canonical, order-stable serialization
 * of (instrument, conventions) so two cells with equal arguments — regardless of
 * whitespace/case in what the trader typed — map to the same key.
 */
export function subscriptionKey(instrument: Instrument, conventions: Conventions): string {
  return JSON.stringify([canonicalInstrument(instrument), canonicalConventions(conventions)]);
}

function canonicalInstrument(i: Instrument): unknown {
  const product =
    i.product.kind === "vanilla"
      ? {
          k: "v",
          ot: i.product.vanilla.optionType,
          s: canonicalStrike(i.product.vanilla.strike),
        }
      : { k: "s", kind: i.product.strategy.kind };
  return {
    base: i.pair.base,
    quote: i.pair.quote,
    tu: i.tenor.unit,
    tc: i.tenor.count,
    ey: i.expiryYears,
    n: i.quantity.notional,
    bc: i.quantity.baseCcy,
    side: i.side,
    product,
  };
}

function canonicalStrike(s: StrikeOrDelta): unknown {
  return s.kind === "strike" ? { k: "strike", v: s.strike } : { k: "delta", v: s.delta };
}

function canonicalConventions(c: Conventions): unknown {
  return [c.deltaConvention, c.atmConvention, c.premiumStyle, c.cut, c.dayCount, c.settlement];
}
