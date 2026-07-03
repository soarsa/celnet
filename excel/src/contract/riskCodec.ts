// ONE CONTRACT — the Excel-side JSON codec for the `RiskService` WS frames, the
// exact field-for-field mirror of the server's `crates/celnet-server/src/ws/codec.rs`
// risk arms (the `*_request_from_json` / `*_response_to_json` fns) and the documented
// §Phase-2 contract in `docs/INTERFACES.md`. Same single, current `celnet.wire`
// contract (CLAUDE.md rule 9), second encoding: snake_case proto field names, every
// enum by its canonical proto enum NUMBER, `optional` (presence-tracked) fields
// `null`/absent ⇒ `undefined`. There is no second contract and no client-side
// aggregation — the client sends scope/principal/numeraire and receives the rolled-up
// node tree the SERVER computed.
//
// Encoders here build the `aggregate_risk` / `list_positions` / `limit_status` /
// `drill_risk` request bodies; decoders read the `*_response` replies. The proto
// `RiskPosition` is encoded by the server only (built from the live book) — a client
// never sends positions — so there is only a `riskPositionFromWire` decoder, no
// encoder (mirroring "no dead decoder" on the server's `risk_position_from_json`
// absence). Enum numbers are the canonical proto tags verified against
// `crates/celnet-proto/proto/celnet.proto`.

import type { CcyPair, OisInstrument, RatesCurveSet } from "./contract";
import {
  ccyPairFromWire,
  ratesCurveSetToWire,
  ratesInstrumentToWire,
  type WireObject,
} from "./wsCodec";

// ---------------------------------------------------------------------------
// vocabulary — the typed string projection of the proto risk enums
// ---------------------------------------------------------------------------

/** `RiskService` org dimension (proto `RiskDimension`, ORTHOGONAL axes). */
export type RiskDimension =
  | "FIRM"
  | "TRADER"
  | "BOOK"
  | "DESK"
  | "CCY_PAIR"
  | "LOCATION"
  | "ENTITY";

/** `RiskDimension` members in proto wire order (index IS the enum number). */
const RISK_DIMENSIONS: readonly RiskDimension[] = [
  "FIRM",
  "TRADER",
  "BOOK",
  "DESK",
  "CCY_PAIR",
  "LOCATION",
  "ENTITY",
];

/** A limit's constrained exposure (proto `LimitMetricKind`). */
export type LimitMetricKind =
  | "DELTA"
  | "GAMMA"
  | "VEGA"
  | "VANNA"
  | "VOLGA"
  | "VEGA_BUCKET"
  | "TENOR_VEGA"
  | "CONCENTRATION_DELTA"
  | "CONCENTRATION_VEGA"
  | "VAR"
  | "EXPECTED_SHORTFALL"
  | "STOP_LOSS";

/** `LimitMetricKind` members in proto wire order (index IS the enum number). */
const LIMIT_METRIC_KINDS: readonly LimitMetricKind[] = [
  "DELTA",
  "GAMMA",
  "VEGA",
  "VANNA",
  "VOLGA",
  "VEGA_BUCKET",
  "TENOR_VEGA",
  "CONCENTRATION_DELTA",
  "CONCENTRATION_VEGA",
  "VAR",
  "EXPECTED_SHORTFALL",
  "STOP_LOSS",
];

/** Traffic-light status (proto `RagStatus`, severity-ordered, BREACH worst). */
export type RagStatus = "GREEN" | "AMBER" | "RED" | "BREACH";

const RAG_STATUSES: readonly RagStatus[] = ["GREEN", "AMBER", "RED", "BREACH"];

/** Whether a limit warns or blocks (proto `Enforcement`). */
export type Enforcement = "SOFT" | "HARD";

const ENFORCEMENTS: readonly Enforcement[] = ["SOFT", "HARD"];

/** Map a proto enum number to its typed member (unknown ⇒ proto3 zero value). */
function enumFrom<T extends string>(members: readonly T[], n: number): T {
  return members[n] ?? members[0]!;
}

/** Map a typed member to its proto enum number (the index in wire order). */
function enumTo<T extends string>(members: readonly T[], value: T): number {
  const i = members.indexOf(value);
  return i >= 0 ? i : 0;
}

export const riskDimensionToWire = (d: RiskDimension): number => enumTo(RISK_DIMENSIONS, d);
export const riskDimensionFromWire = (n: number): RiskDimension => enumFrom(RISK_DIMENSIONS, n);
export const limitMetricKindFromWire = (n: number): LimitMetricKind =>
  enumFrom(LIMIT_METRIC_KINDS, n);
export const ragStatusFromWire = (n: number): RagStatus => enumFrom(RAG_STATUSES, n);
export const enforcementFromWire = (n: number): Enforcement => enumFrom(ENFORCEMENTS, n);

// ---------------------------------------------------------------------------
// scalar accessors (decode side) — defensive against a malformed frame
// ---------------------------------------------------------------------------

function num(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

function str(o: WireObject, key: string): string {
  const v = o[key];
  return typeof v === "string" ? v : "";
}

function enumNum(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

function child(o: WireObject, key: string): WireObject {
  const v = o[key];
  return v && typeof v === "object" ? (v as WireObject) : {};
}

function array(o: WireObject, key: string): WireObject[] {
  const v = o[key];
  return Array.isArray(v) ? (v as WireObject[]) : [];
}

/** A 64-bit wire integer (JSON number) recovered as a `bigint`. */
function numToBigInt(o: WireObject, key: string): bigint {
  const v = o[key];
  if (typeof v === "number" && Number.isFinite(v)) return BigInt(Math.trunc(v));
  if (typeof v === "bigint") return v;
  if (typeof v === "string" && v.length > 0) {
    try {
      return BigInt(v);
    } catch {
      return 0n;
    }
  }
  return 0n;
}

/** An optional presence-tracked `double` (`null`/absent ⇒ undefined). */
function optNum(o: WireObject, key: string): number | undefined {
  const v = o[key];
  if (v === null || v === undefined) return undefined;
  return typeof v === "number" ? v : undefined;
}

// ---------------------------------------------------------------------------
// shared value shapes
// ---------------------------------------------------------------------------

/** A `(dimension, value)` scope key — the cube's group-by + entitlement key space. */
export interface RiskScope {
  readonly dimension: RiskDimension;
  /** The group value on that dimension (ignored for FIRM). */
  readonly value: bigint;
}

export function riskScopeToWire(s: RiskScope): WireObject {
  return { dimension: riskDimensionToWire(s.dimension), value: Number(s.value) };
}

export function riskScopeFromWire(o: WireObject): RiskScope {
  return {
    dimension: riskDimensionFromWire(enumNum(o, "dimension")),
    value: numToBigInt(o, "value"),
  };
}

/** One entitlement rule: a conjunction of pinned scopes (empty ⇒ firm root). */
export interface EntitlementRule {
  readonly scopes: readonly RiskScope[];
}

/**
 * The entitlement principal. Default = grant-all, sent as an **explicit** grant-all
 * when a request carries none (see `applyCommon`) so the workflow clears the
 * server's production deny-by-default edge; deny wins over any grant.
 */
export interface EntitlementPrincipal {
  readonly grantAll: boolean;
  readonly grants: readonly EntitlementRule[];
  readonly denies: readonly EntitlementRule[];
}

function entitlementRuleToWire(r: EntitlementRule): WireObject {
  return { scopes: r.scopes.map(riskScopeToWire) };
}

export function entitlementPrincipalToWire(p: EntitlementPrincipal): WireObject {
  return {
    grant_all: p.grantAll,
    grants: p.grants.map(entitlementRuleToWire),
    denies: p.denies.map(entitlementRuleToWire),
  };
}

/** One spot conversion rate (units of numeraire per 1 unit of `ccy`). */
export interface NumeraireRate {
  readonly ccy: string;
  readonly rate: number;
}

/** The reporting-numeraire conversion table (numeraire's own rate is implicitly 1.0). */
export interface ReportingNumeraire {
  readonly numeraire: string;
  readonly rates: readonly NumeraireRate[];
}

export function reportingNumeraireToWire(n: ReportingNumeraire): WireObject {
  return {
    numeraire: n.numeraire,
    rates: n.rates.map((r) => ({ ccy: r.ccy, rate: r.rate })),
  };
}

/** A `(tenor x delta)` vega pillar coordinate (0.25Δ → 2500 delta_bp). */
export interface VegaPillar {
  readonly tenorDays: number;
  readonly deltaBp: number;
}

export function vegaPillarToWire(p: VegaPillar): WireObject {
  return { tenor_days: p.tenorDays, delta_bp: p.deltaBp };
}

export function vegaPillarFromWire(o: WireObject): VegaPillar {
  return { tenorDays: num(o, "tenor_days"), deltaBp: num(o, "delta_bp") };
}

// ---------------------------------------------------------------------------
// node measures — decode (server → Excel)
// ---------------------------------------------------------------------------

/** One signed currency leg of a node's netted delta exposure vector. */
export interface CcyExposureLeg {
  readonly ccy: string;
  readonly amount: number;
}

/** One `(tenor x delta)` bucket of the additive vega ladder (reporting numeraire). */
export interface VegaLadderBucket {
  readonly pillar: VegaPillar;
  readonly vega: number;
}

/** The additive measures of a node, netted and in the reporting numeraire. */
export interface AdditiveRisk {
  readonly deltaNumeraire: number;
  readonly deltaVector: readonly CcyExposureLeg[];
  readonly gamma: number;
  readonly vegaNumeraire: number;
  readonly theta: number;
  readonly vanna: number;
  readonly volga: number;
  readonly charm: number;
  readonly speed: number;
  readonly zomma: number;
  readonly color: number;
  readonly premiumNumeraire: number;
  readonly vegaLadder: readonly VegaLadderBucket[];
}

/** The non-additive measures of a node (re-derived; presence-tracked ⇒ optional). */
export interface NonAdditiveRisk {
  readonly var?: number;
  readonly es?: number;
  readonly varAlpha?: number;
  readonly curvatureSpot?: number;
}

/** One node of the rolled-up risk tree (the SERVER's aggregation result). */
export interface RiskNode {
  readonly dimension: RiskDimension;
  readonly group: bigint;
  readonly additive: AdditiveRisk;
  readonly nonadditive: NonAdditiveRisk;
  readonly positionCount: number;
}

function ccyExposureLegFromWire(o: WireObject): CcyExposureLeg {
  return { ccy: str(o, "ccy"), amount: num(o, "amount") };
}

function vegaLadderBucketFromWire(o: WireObject): VegaLadderBucket {
  return { pillar: vegaPillarFromWire(child(o, "pillar")), vega: num(o, "vega") };
}

export function additiveRiskFromWire(o: WireObject): AdditiveRisk {
  return {
    deltaNumeraire: num(o, "delta_numeraire"),
    deltaVector: array(o, "delta_vector").map(ccyExposureLegFromWire),
    gamma: num(o, "gamma"),
    vegaNumeraire: num(o, "vega_numeraire"),
    theta: num(o, "theta"),
    vanna: num(o, "vanna"),
    volga: num(o, "volga"),
    charm: num(o, "charm"),
    speed: num(o, "speed"),
    zomma: num(o, "zomma"),
    color: num(o, "color"),
    premiumNumeraire: num(o, "premium_numeraire"),
    vegaLadder: array(o, "vega_ladder").map(vegaLadderBucketFromWire),
  };
}

export function nonAdditiveRiskFromWire(o: WireObject): NonAdditiveRisk {
  const r: { -readonly [K in keyof NonAdditiveRisk]?: NonAdditiveRisk[K] } = {};
  const varLoss = optNum(o, "var");
  if (varLoss !== undefined) r.var = varLoss;
  const es = optNum(o, "es");
  if (es !== undefined) r.es = es;
  const alpha = optNum(o, "var_alpha");
  if (alpha !== undefined) r.varAlpha = alpha;
  const curv = optNum(o, "curvature_spot");
  if (curv !== undefined) r.curvatureSpot = curv;
  return r as NonAdditiveRisk;
}

export function riskNodeFromWire(o: WireObject): RiskNode {
  return {
    dimension: riskDimensionFromWire(enumNum(o, "dimension")),
    group: numToBigInt(o, "group"),
    additive: additiveRiskFromWire(child(o, "additive")),
    nonadditive: nonAdditiveRiskFromWire(child(o, "nonadditive")),
    positionCount: num(o, "position_count"),
  };
}

// ---------------------------------------------------------------------------
// positions — decode (server → Excel; never encoded by a client)
// ---------------------------------------------------------------------------

/** The org placement of a position across the independent dimensions. */
export interface OrgKey {
  readonly trader: number;
  readonly book: number;
  readonly desk: number;
  readonly ccyPair: CcyPair;
  readonly location: number;
  readonly entity: number;
}

/** The pricing inputs a position was marked under (proto `VanillaInputs`). */
export interface VanillaInputs {
  readonly spot: number;
  readonly strike: number;
  readonly vol: number;
  readonly t: number;
  readonly rDom: number;
  readonly rFor: number;
}

/** One open position the cube aggregates (decoded leaf). */
export interface RiskPosition {
  readonly positionId: bigint;
  readonly org: OrgKey;
  readonly optionType: "CALL" | "PUT";
  readonly notionalBase: number;
  readonly inputs: VanillaInputs;
  readonly surfaceVersion: bigint;
  /** A readable holder/quoter line from the attribution chain (absent ⇒ undefined). */
  readonly attribution?: string;
}

function orgKeyFromWire(o: WireObject): OrgKey {
  return {
    trader: num(o, "trader"),
    book: num(o, "book"),
    desk: num(o, "desk"),
    ccyPair: ccyPairFromWire(child(o, "ccy_pair")),
    location: num(o, "location"),
    entity: num(o, "entity"),
  };
}

function vanillaInputsFromWire(o: WireObject): VanillaInputs {
  return {
    spot: num(o, "spot"),
    strike: num(o, "strike"),
    vol: num(o, "vol"),
    t: num(o, "t"),
    rDom: num(o, "r_dom"),
    rFor: num(o, "r_for"),
  };
}

/**
 * Render the optional `AttributionRecord` (the existing camelCase WS chain —
 * `quotedBy`/`heldBy` `BookId` `{book, owner:{trader|autoPricer}}`, `won`/`lpCount`)
 * into a terse holder/quoter display string. Absent ⇒ undefined. The Excel surface
 * shows attribution as one readable cell, not a nested struct.
 */
function attributionDisplayFromWire(o: WireObject): string | undefined {
  const a = o["attribution"];
  if (!a || typeof a !== "object") return undefined;
  const rec = a as WireObject;
  const seat = (bookKey: string): string | undefined => {
    const b = rec[bookKey];
    if (!b || typeof b !== "object") return undefined;
    const book = b as WireObject;
    const owner = book["owner"];
    let who = "";
    if (owner && typeof owner === "object") {
      const ow = owner as WireObject;
      who =
        typeof ow["trader"] === "string"
          ? ow["trader"]
          : typeof ow["autoPricer"] === "string"
            ? `auto:${ow["autoPricer"]}`
            : "";
    }
    const bookName = typeof book["book"] === "string" ? book["book"] : "";
    return [bookName, who].filter((x) => x.length > 0).join("/") || undefined;
  };
  const held = seat("heldBy");
  const quoted = seat("quotedBy");
  const parts: string[] = [];
  if (held !== undefined) parts.push(`held ${held}`);
  if (quoted !== undefined) parts.push(`quoted ${quoted}`);
  if (parts.length === 0) return undefined;
  return parts.join(" | ");
}

export function riskPositionFromWire(o: WireObject): RiskPosition {
  const p: { -readonly [K in keyof RiskPosition]?: RiskPosition[K] } = {
    positionId: numToBigInt(o, "position_id"),
    org: orgKeyFromWire(child(o, "org")),
    optionType: enumNum(o, "option_type") === 1 ? "PUT" : "CALL",
    notionalBase: num(o, "notional_base"),
    inputs: vanillaInputsFromWire(child(o, "inputs")),
    surfaceVersion: numToBigInt(o, "surface_version"),
  };
  const attribution = attributionDisplayFromWire(o);
  if (attribution !== undefined) p.attribution = attribution;
  return p as RiskPosition;
}

// ---------------------------------------------------------------------------
// limits — decode (server → Excel)
// ---------------------------------------------------------------------------

/** One limit's evaluated utilization at a scope node (computed server-side). */
export interface LimitUtilization {
  readonly metric: LimitMetricKind;
  readonly vegaPillar: VegaPillar;
  readonly tenorDays: number;
  readonly cap: number;
  readonly exposure: number;
  readonly ratio: number;
  readonly status: RagStatus;
  readonly enforcement: Enforcement;
  readonly headroom: number;
}

export function limitUtilizationFromWire(o: WireObject): LimitUtilization {
  return {
    metric: limitMetricKindFromWire(enumNum(o, "metric")),
    vegaPillar: vegaPillarFromWire(child(o, "vega_pillar")),
    tenorDays: num(o, "tenor_days"),
    cap: num(o, "cap"),
    exposure: num(o, "exposure"),
    ratio: num(o, "ratio"),
    status: ragStatusFromWire(enumNum(o, "status")),
    enforcement: enforcementFromWire(enumNum(o, "enforcement")),
    headroom: num(o, "headroom"),
  };
}

// ---------------------------------------------------------------------------
// requests — encode (Excel → server)
// ---------------------------------------------------------------------------

/** Common optional request pieces every risk RPC shares. */
export interface RiskCommon {
  readonly principal?: EntitlementPrincipal;
  readonly scope?: RiskScope;
}

/**
 * Apply the principal/scope onto a request body. A risk request always carries a
 * principal: the caller's, or — when absent — an **explicit** grant-all (the
 * audited show-all-now default every client shares), so the headline risk function
 * clears the server's production deny-by-default boundary (`AccessMode::Enforce`),
 * which denies a *genuinely* absent principal. A deployment gateway injects the
 * real principal in production. Scope stays optional (absent ⇒ the firm root).
 */
function applyCommon(body: WireObject, common: RiskCommon): WireObject {
  body["principal"] = entitlementPrincipalToWire(
    common.principal ?? { grantAll: true, grants: [], denies: [] },
  );
  if (common.scope !== undefined) body["scope"] = riskScopeToWire(common.scope);
  return body;
}

/** Build the `list_positions` request body. */
export function listPositionsRequest(common: RiskCommon): WireObject {
  return applyCommon({}, common);
}

/** The fully-shaped pieces of an `aggregate_risk` request. */
export interface AggregateRiskInput extends RiskCommon {
  readonly dimension: RiskDimension;
  readonly numeraire: ReportingNumeraire;
  readonly vegaPillars?: readonly VegaPillar[];
  readonly varSpotShocks?: readonly number[];
  readonly varAlpha?: number;
  readonly curvatureRiskWeight?: number;
}

/** Build the `aggregate_risk` request body. */
export function aggregateRiskRequest(input: AggregateRiskInput): WireObject {
  const body: WireObject = {
    dimension: riskDimensionToWire(input.dimension),
    numeraire: reportingNumeraireToWire(input.numeraire),
    vega_pillars: (input.vegaPillars ?? []).map(vegaPillarToWire),
    var_spot_shocks: [...(input.varSpotShocks ?? [])],
    var_alpha: input.varAlpha ?? 0,
    curvature_risk_weight: input.curvatureRiskWeight ?? 0,
  };
  return applyCommon(body, input);
}

/** The fully-shaped pieces of a `drill_risk` request. */
export interface DrillRiskInput {
  readonly node: RiskScope;
  readonly childDimension: RiskDimension;
  readonly numeraire: ReportingNumeraire;
  readonly principal?: EntitlementPrincipal;
  readonly vegaPillars?: readonly VegaPillar[];
  readonly includeChildren: boolean;
  readonly includePositions: boolean;
}

/** Build the `drill_risk` request body. */
export function drillRiskRequest(input: DrillRiskInput): WireObject {
  const body: WireObject = {
    node: riskScopeToWire(input.node),
    child_dimension: riskDimensionToWire(input.childDimension),
    numeraire: reportingNumeraireToWire(input.numeraire),
    vega_pillars: (input.vegaPillars ?? []).map(vegaPillarToWire),
    include_children: input.includeChildren,
    include_positions: input.includePositions,
  };
  if (input.principal !== undefined) {
    body["principal"] = entitlementPrincipalToWire(input.principal);
  }
  return body;
}

/** The fully-shaped pieces of a `limit_status` request. */
export interface LimitStatusInput {
  readonly scope: RiskScope;
  readonly numeraire: ReportingNumeraire;
  readonly principal?: EntitlementPrincipal;
  readonly vegaPillars?: readonly VegaPillar[];
  readonly varSpotShocks?: readonly number[];
  readonly varAlpha?: number;
}

/** Build the `limit_status` request body. */
export function limitStatusRequest(input: LimitStatusInput): WireObject {
  const body: WireObject = {
    scope: riskScopeToWire(input.scope),
    numeraire: reportingNumeraireToWire(input.numeraire),
    vega_pillars: (input.vegaPillars ?? []).map(vegaPillarToWire),
    var_spot_shocks: [...(input.varSpotShocks ?? [])],
    var_alpha: input.varAlpha ?? 0,
  };
  if (input.principal !== undefined) {
    body["principal"] = entitlementPrincipalToWire(input.principal);
  }
  return body;
}

// ---------------------------------------------------------------------------
// responses — decode (server → Excel)
// ---------------------------------------------------------------------------

/** Decode a `list_positions_response`. */
export interface ListPositionsResult {
  readonly positions: readonly RiskPosition[];
}

export function listPositionsResponseFromWire(o: WireObject): ListPositionsResult {
  return { positions: array(o, "positions").map(riskPositionFromWire) };
}

/** Decode an `aggregate_risk_response`. */
export interface AggregateRiskResult {
  readonly dimension: RiskDimension;
  readonly numeraire: string;
  readonly nodes: readonly RiskNode[];
}

export function aggregateRiskResponseFromWire(o: WireObject): AggregateRiskResult {
  return {
    dimension: riskDimensionFromWire(enumNum(o, "dimension")),
    numeraire: str(o, "numeraire"),
    nodes: array(o, "nodes").map(riskNodeFromWire),
  };
}

/** Decode a `drill_risk_response`. */
export interface DrillRiskResult {
  readonly node: RiskScope;
  readonly children: readonly RiskNode[];
  readonly positions: readonly RiskPosition[];
}

export function drillRiskResponseFromWire(o: WireObject): DrillRiskResult {
  return {
    node: riskScopeFromWire(child(o, "node")),
    children: array(o, "children").map(riskNodeFromWire),
    positions: array(o, "positions").map(riskPositionFromWire),
  };
}

/** Decode a `limit_status_response`. */
export interface LimitStatusResult {
  readonly scope: RiskScope;
  readonly limits: readonly LimitUtilization[];
  readonly worst: RagStatus;
  readonly hardBreach: boolean;
}

export function limitStatusResponseFromWire(o: WireObject): LimitStatusResult {
  return {
    scope: riskScopeFromWire(child(o, "scope")),
    limits: array(o, "limits").map(limitUtilizationFromWire),
    worst: ragStatusFromWire(enumNum(o, "worst")),
    hardBreach: Boolean(o["hard_breach"]),
  };
}

// ---------------------------------------------------------------------------
// linear-rates portfolio risk (RiskService.AggregateRatesRisk) — the WS mirror
// of the rates-risk edge (`crates/celnet-server/src/ws/codec.rs`
// `aggregate_rates_risk_*`). Every signed `RatesPosition` prices against the ONE
// request-supplied `curveSet` and rolls up ADDITIVELY into one `RatesRiskNode`
// per settlement currency — netted PV / PV01 / DV01 + a tenor-bucketed key-rate
// DV01 ladder. Purely additive, per-ccy partitioned, deterministic; the exact
// rates analogue of the options `aggregate_risk` path. The `curve_set` /
// `instrument` sub-shapes reuse the shared `price_rates` encoders (`wsCodec.ts`)
// so the risk path speaks the IDENTICAL market the pricing edge does — the same
// single server `curve_set_from_json` decodes both. Aggregation is SERVER-owned:
// the client sends the market + positions and receives the netted node tree.
// ---------------------------------------------------------------------------

/**
 * One open linear-rates position the rollup nets (`celnet.wire.RatesPosition`):
 * the `(entity, book)` cell it books into plus the `OisInstrument` to price. The
 * instrument carries its own signed direction (PAY_FIXED / RECEIVE_FIXED), so the
 * priced PV / PV01 / DV01 already net by sign across long and short books.
 */
export interface RatesPosition {
  /** Stable position identity (the pricer's `request_id` echo); informational. */
  readonly positionId: bigint;
  /** The legal-entity id the position books into (a scope filter dimension). */
  readonly entity: number;
  /** The trading-book id the position books into (a scope filter dimension). */
  readonly book: number;
  /** The OIS to price against the request `curveSet` (the only P0 arm). */
  readonly instrument: OisInstrument;
}

/**
 * The optional `(entity, book, ccy)` filter applied BEFORE the rollup
 * (`celnet.wire.RatesRiskScope`): each present field narrows the contributing
 * positions; an absent field does not constrain. `ccy` matches case-insensitively
 * server-side.
 */
export interface RatesRiskScope {
  /** Keep only positions in this legal entity, when set. */
  readonly entity?: number;
  /** Keep only positions in this trading book, when set. */
  readonly book?: number;
  /** Keep only positions whose settlement currency matches, when set. */
  readonly ccy?: string;
}

/**
 * `RiskService.AggregateRatesRisk` request — price every `RatesPosition` against
 * the shared `curveSet`, narrow by the optional `scope`, then sum additively into
 * one `RatesRiskNode` per settlement currency. The market is the request-supplied
 * `curveSet`, so the rollup is a pure, deterministic calculation.
 */
export interface AggregateRatesRiskRequest {
  /** The calibrated curve set every position prices against (the shared market). */
  readonly curveSet: RatesCurveSet;
  /** The positions to net; empty ⇒ an empty rollup. */
  readonly positions: readonly RatesPosition[];
  /** The optional pre-rollup `(entity, book, ccy)` filter. */
  readonly scope?: RatesRiskScope;
  /** Entitlement principal; omitted ⇒ the audited explicit grant-all default. */
  readonly principal?: EntitlementPrincipal;
}

/**
 * One tenor bucket of a node's key-rate DV01 ladder (`celnet.wire.KeyRateDv01`):
 * the netted PV change for a +1bp bump of the curve pillar at `tenorYears` alone.
 */
export interface KeyRateDv01 {
  /** The curve pillar tenor (whole years) this bucket bumps. */
  readonly tenorYears: number;
  /** The netted DV01 contribution at this pillar (curve currency). */
  readonly dv01: number;
}

/**
 * The netted risk of one settlement currency (`celnet.wire.RatesRiskNode`): the
 * additively summed PV / PV01 / DV01 across every contributing position, plus the
 * per-pillar key-rate DV01 ladder (which sums to `netDv01` to first order).
 */
export interface RatesRiskNode {
  /** ISO-4217 settlement currency of this node (the rollup partition key). */
  readonly ccy: string;
  /** Summed present value across the node's positions (curve currency). */
  readonly netPv: number;
  /** Summed analytic PV01 across the node's positions. */
  readonly netPv01: number;
  /** Summed parallel DV01 across the node's positions. */
  readonly netDv01: number;
  /** The tenor-bucketed key-rate DV01 ladder, in ascending pillar order. */
  readonly keyRateLadder: readonly KeyRateDv01[];
}

/**
 * `RiskService.AggregateRatesRisk` response — one `RatesRiskNode` per settlement
 * currency, in ascending-currency order (the server-computed rollup).
 */
export interface AggregateRatesRiskResponse {
  readonly nodes: readonly RatesRiskNode[];
}

/** Encode one `RatesPosition` to its wire object (the OIS oneof + booking cell). */
function ratesPositionToWire(p: RatesPosition): WireObject {
  return {
    // `position_id` is a wire `uint64`; the connection's other ids ride as JSON
    // numbers, so narrow the bigint exactly as the correlation id is narrowed.
    position_id: Number(p.positionId),
    entity: p.entity,
    book: p.book,
    instrument: ratesInstrumentToWire(p.instrument),
  };
}

/** Encode the optional `(entity, book, ccy)` scope; absent fields are omitted. */
function ratesRiskScopeToWire(s: RatesRiskScope): WireObject {
  const w: WireObject = {};
  if (s.entity !== undefined) w["entity"] = s.entity;
  if (s.book !== undefined) w["book"] = s.book;
  if (s.ccy !== undefined) w["ccy"] = s.ccy;
  return w;
}

/**
 * Build the `aggregate_rates_risk` request body. Always carries an EXPLICIT
 * principal — the caller's, or (when absent) the audited show-all-now grant-all —
 * so the request clears the server's production deny-by-default edge
 * (`AccessMode::Enforce`), exactly as the options `aggregate_risk` request does
 * (the rates-risk edge requires only `ReadAny` + a present principal; a valid
 * `session_token` is optional and is injected by the transport when held). Scope
 * stays optional (absent ⇒ the whole book).
 */
export function aggregateRatesRiskRequest(request: AggregateRatesRiskRequest): WireObject {
  const body: WireObject = {
    curve_set: ratesCurveSetToWire(request.curveSet),
    positions: request.positions.map(ratesPositionToWire),
  };
  body["principal"] = entitlementPrincipalToWire(
    request.principal ?? { grantAll: true, grants: [], denies: [] },
  );
  if (request.scope !== undefined) body["scope"] = ratesRiskScopeToWire(request.scope);
  return body;
}

/** Decode one key-rate DV01 ladder bucket (`{ tenor_years, dv01 }`). */
function keyRateDv01FromWire(o: WireObject): KeyRateDv01 {
  return { tenorYears: num(o, "tenor_years"), dv01: num(o, "dv01") };
}

/** Decode one per-currency `RatesRiskNode` (netted scalars + tenor ladder). */
function ratesRiskNodeFromWire(o: WireObject): RatesRiskNode {
  return {
    ccy: str(o, "ccy"),
    netPv: num(o, "net_pv"),
    netPv01: num(o, "net_pv01"),
    netDv01: num(o, "net_dv01"),
    keyRateLadder: array(o, "key_rate_ladder").map(keyRateDv01FromWire),
  };
}

/** Decode an `aggregate_rates_risk_response` into the per-currency node tree. */
export function aggregateRatesRiskResponseFromWire(o: WireObject): AggregateRatesRiskResponse {
  return { nodes: array(o, "nodes").map(ratesRiskNodeFromWire) };
}

// ---------------------------------------------------------------------------
// linear-rates BOOK ledger (RiskService.ListRatesPositions) + the named
// entity/book registry (AuthService.List{Entities,Books}) — the WS mirror of the
// rates-Book List edge (`crates/celnet-server/src/ws/codec.rs`
// `list_rates_positions_*`, oracle `list_rates_positions_round_trip`) and the
// entity/book registry (`entity_desc_to_json` / `book_desc_to_json`). The
// CELNET.RATESBOOK add-in path lists the desk's standing OIS positions — which
// carry NUMERIC `(entity, book)` partition keys on the wire — and resolves each
// key to its registry NAME, exactly like the GUI RatesBookWorkspace (an unknown
// key falls back to `#<key>`). A position is SERVER-owned — a client never sends
// one on this read path — so there is only a decoder here, mirroring the server's
// `rates_position_to_json` encoding (the OIS `side` code 0/1 ⇔ PAY/RECEIVE fixed).
// ---------------------------------------------------------------------------

/** The wire OIS `side` code → the typed `OisDirection` (PAY_FIXED=0, RECEIVE_FIXED=1). */
function oisDirectionFromSide(side: number): OisInstrument["direction"] {
  return side === 1 ? "RECEIVE_FIXED" : "PAY_FIXED";
}

/**
 * Decode a `RatesInstrument` `{ ois: { tenor_years, fixed_rate, notional, side } }`
 * into the typed `OisInstrument` — the exact inverse of `ratesInstrumentToWire`
 * (`wsCodec.ts`), so the ledger reads back the market the book was booked under.
 */
function ratesInstrumentFromWire(o: WireObject): OisInstrument {
  const ois = child(o, "ois");
  return {
    tenorYears: num(ois, "tenor_years"),
    fixedRate: num(ois, "fixed_rate"),
    notional: num(ois, "notional"),
    direction: oisDirectionFromSide(num(ois, "side")),
  };
}

/** Decode one `RatesPosition` `{ position_id, entity, book, instrument }` (the server-owned leaf). */
export function ratesPositionFromWire(o: WireObject): RatesPosition {
  return {
    positionId: numToBigInt(o, "position_id"),
    entity: num(o, "entity"),
    book: num(o, "book"),
    instrument: ratesInstrumentFromWire(child(o, "instrument")),
  };
}

/** The fully-shaped pieces of a `list_rates_positions` request. */
export interface ListRatesPositionsInput {
  /** The optional `(entity, book, ccy)` pre-list filter (absent ⇒ the whole book). */
  readonly scope?: RatesRiskScope;
  /** Entitlement principal; omitted ⇒ the audited explicit grant-all default. */
  readonly principal?: EntitlementPrincipal;
}

/**
 * Build the `list_rates_positions` request body. Carries an EXPLICIT principal —
 * the caller's, or (when absent) the audited show-all-now grant-all — so the read
 * clears the server's production deny-by-default edge (`RiskService/
 * ListRatesPositions`, `ReadAny`), exactly as `aggregate_rates_risk` does; the
 * edge needs no session token (parity with the options risk RPCs). The optional
 * `(entity, book, ccy)` scope narrows the listing (the server filters on
 * entity/book; a stored position has no currency of its own, so `ccy` — accepted
 * for parity with the risk scope — does not constrain).
 */
export function listRatesPositionsRequest(input: ListRatesPositionsInput): WireObject {
  const body: WireObject = {};
  body["principal"] = entitlementPrincipalToWire(
    input.principal ?? { grantAll: true, grants: [], denies: [] },
  );
  if (input.scope !== undefined) body["scope"] = ratesRiskScopeToWire(input.scope);
  return body;
}

/** Decode a `list_rates_positions_response` into the server-owned position ledger. */
export interface ListRatesPositionsResponse {
  readonly positions: readonly RatesPosition[];
}

export function listRatesPositionsResponseFromWire(o: WireObject): ListRatesPositionsResponse {
  return { positions: array(o, "positions").map(ratesPositionFromWire) };
}

// --- named entity/book registry (the display-name ↔ partition-key map) -------
//
// The WS mirror of `AuthService.List{Entities,Books}`. A position carries opaque
// `uint32` keys on the wire; this registry is the name ↔ key map the Book views
// resolve a key back to a name with (an unknown key ⇒ `#<key>`). Listing is open
// to any AUTHENTICATED user; the wire carries snake_case `entity_key`, mapped to
// the camelCase `entityKey` (the only rename), exactly as the GUI codec does.

/** A named legal entity a position books into (`celnet.wire.EntityDesc`). */
export interface EntityDesc {
  /** The `uint32` partition key carried on `RatesPosition.entity` (immutable identity). */
  readonly key: number;
  /** Human-friendly legal-entity name, e.g. "Celnet Global Markets". */
  readonly name: string;
  /** Short code, e.g. "CGM" (unique). */
  readonly code: string;
}

/** A named netting book under an entity (`celnet.wire.BookDesc`). */
export interface BookDesc {
  /** The `uint32` partition key carried on `RatesPosition.book` (immutable identity). */
  readonly key: number;
  /** Human-friendly book name, e.g. "Rates Trading". */
  readonly name: string;
  /** The owning entity's `EntityDesc.key`. */
  readonly entityKey: number;
}

/** Decode one `EntityDesc` `{ key, name, code }`. */
export function entityDescFromWire(o: WireObject): EntityDesc {
  return { key: num(o, "key"), name: str(o, "name"), code: str(o, "code") };
}

/** Decode one `BookDesc` `{ key, name, entity_key }` (maps `entity_key` → `entityKey`). */
export function bookDescFromWire(o: WireObject): BookDesc {
  return { key: num(o, "key"), name: str(o, "name"), entityKey: num(o, "entity_key") };
}

/** Decode a `list_entities` response (`{ entities: [...] }`, server-framed `type:"entities"`). */
export function listEntitiesResponseFromWire(o: WireObject): EntityDesc[] {
  return array(o, "entities").map(entityDescFromWire);
}

/** Decode a `list_books` response (`{ books: [...] }`, server-framed `type:"books"`). */
export function listBooksResponseFromWire(o: WireObject): BookDesc[] {
  return array(o, "books").map(bookDescFromWire);
}
