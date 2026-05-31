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
  MarketObservable,
  OptionType,
  Side,
  SmileModel,
  StrikeOrDelta,
  Tenor,
  TenorUnit,
} from "../contract/contract";
import type {
  AdditiveRisk,
  LimitUtilization,
  NumeraireRate,
  ReportingNumeraire,
  RiskDimension,
  RiskNode,
  RiskPosition,
  RiskScope,
} from "../contract/riskCodec";

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

/**
 * Parse a smile-model selector string into the contract `SmileModel`. Accepts the
 * trader-facing short names (`VV`, `SABR`, `SVI`, `SSVI`) and the canonical
 * contract names (`MARKET_HEDGE`, `STOCHASTIC_VOL`, `PARAMETRIC`,
 * `PARAMETRIC_SURFACE`), case-insensitive. Empty/absent ⇒ `MARKET_HEDGE` (the
 * server default Vanna-Volga construction), so an omitted argument is the
 * unchanged current behaviour.
 */
export function parseSmileModel(raw: string | undefined): SmileModel {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "VV":
    case "VANNA_VOLGA":
    case "VANNA-VOLGA":
    case "MARKET_HEDGE":
    case "MARKET-HEDGE":
      return "MARKET_HEDGE";
    case "SABR":
    case "STOCHASTIC_VOL":
    case "STOCHASTIC-VOL":
    case "STOCHVOL":
      return "STOCHASTIC_VOL";
    case "SVI":
    case "PARAMETRIC":
      return "PARAMETRIC";
    case "SSVI":
    case "PARAMETRIC_SURFACE":
    case "PARAMETRIC-SURFACE":
      return "PARAMETRIC_SURFACE";
    default:
      throw new ShapingError(
        `invalid smile model \`${raw}\` (expected VV, SABR, SVI or SSVI)`,
      );
  }
}

/**
 * Parse a market-observable selector string into the contract `MarketObservable`.
 * Accepts the trader-facing names (`ATM`/`ATMVOL`, `SPOT`, `RR`/`RISK_REVERSAL`,
 * `BF`/`FLY`/`BUTTERFLY`, `FWD`/`FORWARD`), case-insensitive.
 */
export function parseObservable(raw: string): MarketObservable {
  const s = raw.trim().toUpperCase();
  switch (s) {
    case "ATM":
    case "ATMVOL":
    case "ATM_VOL":
    case "VOL":
      return "ATM_VOL";
    case "SPOT":
      return "SPOT";
    case "RR":
    case "RISK_REVERSAL":
    case "RISKREVERSAL":
      return "RISK_REVERSAL";
    case "BF":
    case "FLY":
    case "BUTTERFLY":
      return "BUTTERFLY";
    case "FWD":
    case "FORWARD":
      return "FORWARD";
    default:
      throw new ShapingError(
        `invalid observable \`${raw}\` (expected ATM, SPOT, RR, BF or FWD)`,
      );
  }
}

/**
 * Parse a delta-wing argument for the wing observables (RR/BF) and surface marks.
 * Accepts a signed/unsigned fraction (`0.25`, `0.10`) or a percent-delta string
 * (`25`, `25d`, `10`). Always returns the positive wing magnitude (the server
 * reads both signed wings for RR/BF). Throws on a wing outside (0, 0.5).
 */
export function parseDeltaWing(raw: string | number): number {
  const numeric = typeof raw === "number" ? raw : Number(String(raw).trim().replace(/D$/i, ""));
  if (!Number.isFinite(numeric) || numeric <= 0) {
    throw new ShapingError(`invalid delta wing \`${raw}\` (expected 0.25 or 25)`);
  }
  const wing = numeric >= 1 ? numeric / 100 : numeric;
  if (wing <= 0 || wing >= 0.5) {
    throw new ShapingError(`delta wing \`${raw}\` out of range (expected 0 < δ < 0.5)`);
  }
  return wing;
}

// ---------------------------------------------------------------------------
// risk shaping (RiskService — server-side hierarchical risk)
// ---------------------------------------------------------------------------

/**
 * Parse an org-dimension selector string into the contract `RiskDimension` (the
 * `aggregate_risk` group-by axis / scope dimension). Accepts the trader-facing
 * short names (`FIRM`, `TRADER`, `BOOK`, `DESK`, `PAIR`/`CCYPAIR`, `LOCATION`/
 * `LOC`, `ENTITY`/`LE`), case-insensitive. Empty/absent ⇒ `FIRM` (the apex, the
 * proto3 default), so an omitted argument rolls the whole entitled book to one node.
 */
export function parseRiskDimension(raw: string | undefined): RiskDimension {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "FIRM":
      return "FIRM";
    case "TRADER":
      return "TRADER";
    case "BOOK":
      return "BOOK";
    case "DESK":
      return "DESK";
    case "PAIR":
    case "CCYPAIR":
    case "CCY_PAIR":
      return "CCY_PAIR";
    case "LOCATION":
    case "LOC":
      return "LOCATION";
    case "ENTITY":
    case "LE":
      return "ENTITY";
    default:
      throw new ShapingError(
        `invalid dimension \`${raw}\` (expected FIRM, TRADER, BOOK, DESK, PAIR, LOCATION or ENTITY)`,
      );
  }
}

/** Parse a numeraire ccy code (3 letters), upper-cased. */
export function parseNumeraireCcy(raw: string | undefined): string {
  const s = (raw ?? "").trim().toUpperCase();
  if (s === "") return "USD"; // a sensible reporting default; overridable per call
  if (!/^[A-Z]{3}$/.test(s)) {
    throw new ShapingError(`invalid numeraire \`${raw}\` (expected a 3-letter code, e.g. USD)`);
  }
  return s;
}

/**
 * Shape a `ReportingNumeraire` from a numeraire code and an optional `[ccy, rate]`
 * spill range. The numeraire's own rate is implicit 1.0; every other currency in
 * the book needs a rate (units of numeraire per 1 unit of ccy at spot). The rates
 * arrive as the 2-D cell range Excel passes for the optional argument — rows of
 * `[ccyString, rateNumber]`; a blank/short row is skipped. A row naming the
 * numeraire itself is dropped (it is implicitly 1.0).
 */
export function shapeReportingNumeraire(
  numeraireCcy: string,
  rateRange: unknown,
): ReportingNumeraire {
  const numeraire = parseNumeraireCcy(numeraireCcy);
  const rates: NumeraireRate[] = [];
  if (Array.isArray(rateRange)) {
    for (const row of rateRange) {
      if (!Array.isArray(row) || row.length < 2) continue;
      const ccyCell = row[0];
      const rateCell = row[1];
      if (ccyCell === "" || ccyCell === null || ccyCell === undefined) continue;
      const ccy = String(ccyCell).trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(ccy)) {
        throw new ShapingError(`invalid rate ccy \`${ccyCell}\` (expected a 3-letter code)`);
      }
      if (ccy === numeraire) continue; // implicitly 1.0
      const rate = typeof rateCell === "number" ? rateCell : Number(rateCell);
      if (!Number.isFinite(rate) || rate <= 0) {
        throw new ShapingError(`invalid numeraire rate for ${ccy}: \`${rateCell}\` (finite, > 0)`);
      }
      rates.push({ ccy, rate });
    }
  }
  return { numeraire, rates };
}

/**
 * Parse a scope argument `<DIM>:<value>` (e.g. `DESK:99`) into a `RiskScope`, or
 * undefined for an empty/`ALL`/`FIRM` scope (the whole entitled book). The value is
 * the cube's interned group handle (a non-negative integer).
 */
export function parseRiskScope(raw: string | undefined): RiskScope | undefined {
  const s = (raw ?? "").trim();
  if (s === "" || s.toUpperCase() === "ALL" || s.toUpperCase() === "FIRM") return undefined;
  const m = /^([A-Za-z_]+)\s*[:=]\s*(\d+)$/.exec(s);
  if (!m || m[1] === undefined || m[2] === undefined) {
    throw new ShapingError(`invalid scope \`${raw}\` (expected DIM:value, e.g. DESK:99, or ALL)`);
  }
  return { dimension: parseRiskDimension(m[1]), value: BigInt(m[2]) };
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

/** The fully-parsed inputs a model-selected surface calibration shapes. */
export interface CalibrateArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly model: string | undefined;
  readonly atmVol: number;
  readonly rr25: number;
  readonly bf25: number;
  /** Optional 10Δ wings; both present ⇒ a five-point smile is calibrated. */
  readonly rr10?: number | undefined;
  readonly bf10?: number | undefined;
}

/** The validated, contract-shaped pieces of a calibration request. */
export interface ShapedCalibration {
  readonly pair: CcyPair;
  readonly tenorYears: number;
  readonly model: SmileModel;
  readonly atmVol: number;
  readonly rr25: number;
  readonly bf25: number;
  readonly rr10: number;
  readonly bf10: number;
  readonly hasTenDelta: boolean;
}

/**
 * Shape a model-selected surface calibration from cell arguments: parse the pair,
 * tenor and model, and validate the broker marks. No transport — the function
 * layer turns this into the `mark_surface` body (broker_quotes + smile_model).
 */
export function shapeCalibration(args: CalibrateArgs): ShapedCalibration {
  const pair = parsePair(args.pair);
  const { expiryYears } = parseTenor(args.tenor);
  const model = parseSmileModel(args.model);
  if (!Number.isFinite(args.atmVol) || args.atmVol <= 0 || args.atmVol >= 5) {
    throw new ShapingError(`invalid ATM vol \`${args.atmVol}\` (absolute, e.g. 0.102)`);
  }
  if (!Number.isFinite(args.rr25) || !Number.isFinite(args.bf25)) {
    throw new ShapingError("rr25/bf25 must be finite vols (e.g. 0.01, 0.003)");
  }
  const hasTenDelta = args.rr10 !== undefined && args.bf10 !== undefined;
  if (hasTenDelta && (!Number.isFinite(args.rr10) || !Number.isFinite(args.bf10))) {
    throw new ShapingError("rr10/bf10 must be finite vols when supplied");
  }
  return {
    pair,
    tenorYears: expiryYears,
    model,
    atmVol: args.atmVol,
    rr25: args.rr25,
    bf25: args.bf25,
    rr10: hasTenDelta ? (args.rr10 as number) : 0,
    bf10: hasTenDelta ? (args.bf10 as number) : 0,
    hasTenDelta,
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

/**
 * Format a calibrated smile (the result of a model-selected `mark_surface`) as a
 * spill: a delta-pillar header row, a vol row, then a footer carrying the marked
 * `surface_version`, the model provenance (read from the server's
 * `arbitrage.note`, which the server stamps as `model=<family>`), the arb status,
 * and the convention transparency. The `model` argument is the model the cell
 * REQUESTED; the footer prefers the server-reported provenance when present so a
 * silent server default is visible, never assumed.
 */
export function formatCalibratedSmileSpill(args: {
  readonly points: readonly SurfacePoint[];
  readonly requestedModel: SmileModel;
  readonly providerNote: string;
  readonly arbFree: boolean;
  readonly conv: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}): SpillMatrix {
  const sorted = [...args.points].sort((a, b) => a.delta - b.delta);
  const header: (string | number)[] = ["delta", ...sorted.map((p) => p.delta)];
  const vols: (string | number)[] = ["vol", ...sorted.map((p) => p.vol)];
  // The server stamps the model used as `model=<family>` inside the arbitrage
  // note (the frozen contract has no echo field). Surface it verbatim when present
  // so the trader sees exactly what was calibrated, falling back to the requested
  // model name when the note carries no provenance token.
  const noteModel = /model=([A-Za-z0-9_-]+)/.exec(args.providerNote)?.[1];
  const modelLabel = noteModel ?? args.requestedModel;
  const footer: (string | number)[] = [
    `model ${modelLabel} | ${args.arbFree ? "arb-free" : "ARB!"} | ` +
      conventionFooter(args.conv, args.surfaceVersion, args.epochNanos),
  ];
  return [header, vols, footer];
}

/** A decoded market-series tick for a single trend cell render. */
export interface SeriesCellInput {
  readonly value: number;
  readonly observable: MarketObservable;
  readonly baselined: boolean;
  readonly epochNanos: bigint;
}

/**
 * Render a streamed market-series tick to a single cell string. Vols are shown in
 * vol points (×100, two-decimal); rates (SPOT/FORWARD) in full precision. An
 * un-baselined / non-finite value renders an explicit waiting marker rather than a
 * frozen number (docs §5: never show stale-as-live).
 */
export function formatSeriesCell(t: SeriesCellInput): string {
  if (!t.baselined || !Number.isFinite(t.value)) return "… (awaiting)";
  const isVol = t.observable === "ATM_VOL" || t.observable === "RISK_REVERSAL" || t.observable === "BUTTERFLY";
  return isVol ? `${(t.value * 100).toFixed(2)}v` : t.value.toFixed(5);
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
// risk spill formatting (RiskService — server-side hierarchical risk)
// ---------------------------------------------------------------------------

/**
 * The additive node columns surfaced by CELNET.RISK, in a stable order: the
 * numeraire-collapsed measures the desk reads. `delta_numeraire`/`vega_numeraire`/
 * `premium_numeraire` are in the reporting numeraire (the §2.2/§2.3 collapse), so a
 * cell is NEVER in native premium units — the old "native premium units" caveat is
 * resolved (aggregation goes through celnet-risk-normalize server-side).
 */
const RISK_MEASURE_COLUMNS: readonly { readonly label: string; readonly key: keyof AdditiveRisk }[] =
  [
    { label: "delta", key: "deltaNumeraire" },
    { label: "gamma", key: "gamma" },
    { label: "vega", key: "vegaNumeraire" },
    { label: "theta", key: "theta" },
    { label: "vanna", key: "vanna" },
    { label: "volga", key: "volga" },
    { label: "premium", key: "premiumNumeraire" },
  ];

/** A footer summarising the reporting numeraire + roll-up dimension (transparency). */
function riskFooter(numeraire: string, dimension: RiskDimension, nodeCount: number): string {
  return `roll-up by ${dimension} | reporting ${numeraire} | ${nodeCount} node${nodeCount === 1 ? "" : "s"} | server-aggregated (celnet-risk-normalize)`;
}

/**
 * Format CELNET.RISK as a node grid: a header row
 * `[group, count, delta, gamma, vega, theta, vanna, volga, premium, VaR, ES, curvature]`,
 * one row per rolled-up node (the value column for each additive measure plus the
 * presence-tracked non-additive measures rendered as a blank when absent — never a
 * spurious zero), then a numeraire/dimension footer. Aggregation is the server's;
 * this only lays the server-returned node tree out in cells.
 */
export function formatRiskSpill(
  dimension: RiskDimension,
  numeraire: string,
  nodes: readonly RiskNode[],
): SpillMatrix {
  const header: (string | number)[] = [
    "group",
    "count",
    ...RISK_MEASURE_COLUMNS.map((c) => c.label),
    "VaR",
    "ES",
    "curvature",
  ];
  const rows: SpillMatrix = [header];
  for (const node of nodes) {
    const row: (string | number)[] = [
      // FIRM rolls to the apex (group 0); show "FIRM" rather than a bare 0 so the
      // single-node firm view reads honestly.
      node.dimension === "FIRM" ? "FIRM" : node.group.toString(),
      node.positionCount,
      ...RISK_MEASURE_COLUMNS.map((c) => node.additive[c.key] as number),
      node.nonadditive.var ?? "",
      node.nonadditive.es ?? "",
      node.nonadditive.curvatureSpot ?? "",
    ];
    rows.push(row);
  }
  rows.push([riskFooter(numeraire, dimension, nodes.length)]);
  return rows;
}

/**
 * Format CELNET.POSITIONS as a leaf grid: a header row
 * `[position_id, book, trader, pair, type, notional, strike, vol, t, surface, attribution]`,
 * one row per entitled open position, then a count footer. The position id /
 * surface version are rendered as strings to avoid 64-bit precision loss.
 */
export function formatPositionsSpill(positions: readonly RiskPosition[]): SpillMatrix {
  const header: (string | number)[] = [
    "position_id",
    "book",
    "trader",
    "pair",
    "type",
    "notional",
    "strike",
    "vol",
    "t",
    "surface",
    "attribution",
  ];
  const rows: SpillMatrix = [header];
  for (const p of positions) {
    rows.push([
      p.positionId.toString(),
      p.org.book,
      p.org.trader,
      `${p.org.ccyPair.base}${p.org.ccyPair.quote}`,
      p.optionType,
      p.notionalBase,
      p.inputs.strike,
      p.inputs.vol,
      p.inputs.t,
      `v${p.surfaceVersion}`,
      p.attribution ?? "—",
    ]);
  }
  if (positions.length === 0) {
    // Honest empty-state: no entitled open positions (a real, not error, state).
    rows.push(["(no entitled open positions)"]);
  } else {
    rows.push([`${positions.length} position${positions.length === 1 ? "" : "s"}`]);
  }
  return rows;
}

/**
 * Format CELNET.LIMITS as a utilization grid: a header row
 * `[metric, cap, exposure, ratio, status, enforcement, headroom]`, one row per
 * limit at the scope, then a worst-RAG / hard-breach footer. A `VEGA_BUCKET` /
 * `TENOR_VEGA` metric annotates its pillar/tenor on the metric label so the row is
 * self-describing.
 */
export function formatLimitsSpill(
  scopeLabel: string,
  limits: readonly LimitUtilization[],
  worst: string,
  hardBreach: boolean,
): SpillMatrix {
  const header: (string | number)[] = [
    "metric",
    "cap",
    "exposure",
    "ratio",
    "status",
    "enforcement",
    "headroom",
  ];
  const rows: SpillMatrix = [header];
  for (const l of limits) {
    let metricLabel: string = l.metric;
    if (l.metric === "VEGA_BUCKET") {
      metricLabel = `VEGA_BUCKET(${l.vegaPillar.tenorDays}d,${l.vegaPillar.deltaBp}bp)`;
    } else if (l.metric === "TENOR_VEGA") {
      metricLabel = `TENOR_VEGA(${l.tenorDays}d)`;
    }
    rows.push([
      metricLabel,
      l.cap,
      l.exposure,
      l.ratio,
      l.status,
      l.enforcement,
      l.headroom,
    ]);
  }
  if (limits.length === 0) {
    rows.push([`scope ${scopeLabel}: no limits configured`]);
  } else {
    rows.push([
      `scope ${scopeLabel} | worst ${worst}${hardBreach ? " | HARD BREACH" : ""}`,
    ]);
  }
  return rows;
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
