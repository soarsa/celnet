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

import type { CcyPair } from "./contract";
import { ccyPairFromWire, type WireObject } from "./wsCodec";

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
