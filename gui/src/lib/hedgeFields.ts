/**
 * The risk-state field registry for the AUTO-HEDGE exit-policy decision graph
 * (docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md §5.2). Every
 * {@link HedgeField} a hedge condition can test is described here ONCE: its human
 * label, the palette group it lives in, its value KIND (enum / numeric / string),
 * and the operators legal for that kind. It is the exact analogue of
 * `lib/routeFields.ts` for the hedge graph — the leaves differ (exit actions, not
 * book targets) but the CONDITION vocabulary is the same shape.
 *
 * The kind ↔ field mapping is a byte-faithful mirror of the Rust
 * `celnet_hedge_routing::HedgeField::kind` (`crates/celnet-hedge-routing/src/field.rs`):
 * `breached` + the identity enum fields (ccy/product/book/desk) are ENUM
 * (compared by equality / membership); `instrument_id` and `counterparty` are free
 * STRINGs; every other risk-state number (net_dv01 … hedge_cost_bp) is NUMERIC. Keeping
 * the matrices here
 * (not re-derived per component) lets the editor render a TYPED value editor and the
 * client-side validator reject a graph the server would reject.
 */
import type { HedgeField, RouteOp } from "../data/contract";
import { type FieldKind, ENUM_LIKE_OPS, NUMERIC_OPS, STRING_OPS } from "./routeOps";

/** The palette section a hedge field is grouped under. */
export type HedgeFieldGroup = "Identity" | "Risk state" | "Budget" | "Flow & market";

/**
 * Whether anything in the server's production `HedgeContext` builder actually POPULATES this
 * field — the client mirror of `celnet_hedge_routing::FieldProvider`
 * (`crates/celnet-hedge-routing/src/field.rs`).
 *
 * A rule branching on an `unprovided` field can never fire, so `HedgeGraph::validate` REJECTS
 * it server-side (`HedgeError::UnprovidedField`). Mirroring the declaration here is what lets
 * the trader see the refusal — and its cause — while building the rule, instead of only on
 * save. `test/hedgeFieldProviderParity.test.ts` parses the Rust match arms and fails if this
 * mirror drifts from them, so the two cannot silently disagree.
 */
export type HedgeFieldProvider =
  | {
      readonly state: "computed";
      /** Where the value comes from, in the server builder's own terms. */
      readonly basis: string;
    }
  | {
      readonly state: "unprovided";
      /** Why nothing populates it — shown verbatim to the rule author, as the server does. */
      readonly reason: string;
    };

/** One hedge risk-state field's full descriptor (the palette + editor read this). */
export interface HedgeFieldSpec {
  /** The wire field selector. */
  field: HedgeField;
  /** Human-friendly chip / editor label. */
  label: string;
  /** The palette section it belongs to. */
  group: HedgeFieldGroup;
  /** Its value kind — drives the operator set + value editor. */
  kind: FieldKind;
  /** The operators legal for this field's kind (mirrors `RouteOp::valid_for`). */
  validOps: RouteOp[];
  /** A one-line description shown on the palette chip + editor. */
  hint: string;
  /** Whether the server populates this field — mirrors `HedgeField::provider`. */
  provider: HedgeFieldProvider;
}

/**
 * The full hedge-field registry, in ordinal order 0..18 — the SAME ordinal order as
 * the Rust `HedgeField` enum and the proto `HedgeFieldEnum` (so the array index equals
 * the wire tag). The palette clusters chips by their {@link HedgeFieldSpec.group}, so a
 * field's ordinal position and its render section are independent — `counterparty` sits
 * last (wire tag 18) yet renders under the `Identity` section.
 */
export const HEDGE_FIELD_REGISTRY: readonly HedgeFieldSpec[] = [
  // --- Identity -----------------------------------------------------------
  {
    field: "instrument_id",
    label: "Instrument",
    group: "Identity",
    kind: "string",
    validOps: STRING_OPS,
    hint: "Symbol of the warehoused instrument (e.g. EURUSD, US10Y).",
    provider: { state: "computed", basis: "the fill's product-family label" },
  },
  {
    field: "ccy",
    label: "Currency",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Currency / pair of the risk.",
    provider: { state: "computed", basis: "the settlement currency on the fill's risk-routing attribution" },
  },
  {
    field: "product",
    label: "Product",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Product family — vanilla, swap, bond, forward…",
    provider: { state: "computed", basis: "the fill's product-family label" },
  },
  {
    field: "book",
    label: "Book",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The risk book / portfolio holding the inventory.",
    provider: { state: "computed", basis: "the risk book the fill routed into" },
  },
  {
    field: "desk",
    label: "Desk",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The owning desk.",
    provider: { state: "unprovided", reason: "a booked rates fill carries no desk — the desk belongs to the FIX/RFQ session that priced it, not to the resulting position; scope the rule by `book` instead" },
  },
  // --- Risk state ---------------------------------------------------------
  {
    field: "net_dv01",
    label: "Net DV01",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net DV01 — the FI first-order rate RISK, in $/bp (PV per +1bp). NOT a face amount.",
    provider: { state: "computed", basis: "the signed DV01 of the fill's book, or of the bucket subtree under a bucket-scoped policy" },
  },
  {
    field: "net_notional",
    label: "Net notional",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net base-currency FACE notional, in $ (the delta for FX). NOT a $/bp risk.",
    provider: { state: "computed", basis: "the signed face notional of the same scope" },
  },
  {
    field: "net_vega",
    label: "Net vega",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net vega.",
    provider: { state: "unprovided", reason: "only linear-rates cells are ever evaluated and they carry no volatility risk, so this field is a constant zero and any rule reading it can never fire" },
  },
  {
    field: "net_gamma",
    label: "Net gamma",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net gamma.",
    provider: { state: "unprovided", reason: "only linear-rates cells are ever evaluated and they carry no convexity risk, so this field is a constant zero and any rule reading it can never fire" },
  },
  {
    field: "inventory_sign",
    label: "Inventory sign",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "+1 long / −1 short.",
    provider: { state: "computed", basis: "the three-way sign of the book's net risk, in the threshold's own budget metric" },
  },
  // --- Budget -------------------------------------------------------------
  {
    field: "threshold",
    label: "Threshold",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "The resolved warehouse threshold (the “100”) for this scope.",
    provider: { state: "computed", basis: "the warehouse cap resolved for the fill's most-specific scope" },
  },
  {
    field: "utilization",
    label: "Utilization",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "|risk| / threshold — the RAG driver (≥1 is a breach).",
    provider: { state: "computed", basis: "|net risk| / cap, in the threshold's metric" },
  },
  {
    field: "overflow",
    label: "Overflow",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "max(0, |risk| − band edge) — the amount to hedge.",
    provider: { state: "computed", basis: "max(0, |net risk| − target), in the threshold's metric" },
  },
  {
    field: "breached",
    label: "Breached",
    group: "Budget",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Whether the red band fired — compare with true / false.",
    provider: { state: "computed", basis: "the warehouse band classification, in the threshold's metric" },
  },
  // --- Flow & market ------------------------------------------------------
  {
    field: "counterparty_toxicity",
    label: "Counterparty toxicity",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Markout / residual toxicity of the flow that built this risk.",
    provider: { state: "unprovided", reason: "no post-fill mark trajectory is retained, so markout cannot be derived and nothing computes a per-counterparty toxicity score" },
  },
  {
    field: "inventory_age_secs",
    label: "Inventory age (s)",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "How long the risk has sat (aging → forced hedge).",
    provider: { state: "unprovided", reason: "a stored rates position carries no acquisition timestamp, so how long the risk has sat cannot be derived" },
  },
  {
    field: "internal_offset_available",
    label: "Internal offset avail.",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Opposing internal flow the aggregator could cross now.",
    provider: { state: "computed", basis: "the netted opposing risk held by sibling books under the same parent, within the same product family" },
  },
  {
    field: "hedge_cost_bp",
    label: "Hedge cost (bp)",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Current external hedge-cost estimate (spread + impact).",
    provider: { state: "computed", basis: "the live LP top-of-book crossing half-spread, falling back to the desk's configured composite spread" },
  },
  // --- Identity (wire tag 18, appended after the numeric fields) -----------
  {
    field: "counterparty",
    label: "Counterparty",
    group: "Identity",
    kind: "string",
    validOps: STRING_OPS,
    hint: "Originating party-id/name of the flow that built this risk (e.g. CITADEL).",
    provider: { state: "computed", basis: "the originating party id on the fill's risk-routing attribution" },
  },
];

/** Palette section order. */
export const HEDGE_FIELD_GROUPS: readonly HedgeFieldGroup[] = [
  "Identity",
  "Risk state",
  "Budget",
  "Flow & market",
];

const BY_FIELD: ReadonlyMap<HedgeField, HedgeFieldSpec> = new Map(
  HEDGE_FIELD_REGISTRY.map((s) => [s.field, s]),
);

/** The descriptor for a hedge field (throws on an unknown selector — a programming error). */
export function hedgeFieldSpec(field: HedgeField): HedgeFieldSpec {
  const s = BY_FIELD.get(field);
  if (!s) throw new Error(`hedgeFields: unknown field \`${field}\``);
  return s;
}

/** The value kind of a hedge field — the single mirror of `HedgeField::kind`. */
export function hedgeFieldKind(field: HedgeField): FieldKind {
  return hedgeFieldSpec(field).kind;
}

/**
 * Why this field has no production source, or `null` when the server computes it — the
 * mirror of `HedgeField::unprovided_reason`. A non-null result means a rule branching on
 * this field will be REFUSED by `HedgeGraph::validate` on save, so the editors disable the
 * field and the client validator raises `unprovided_field` before the trader ever gets there.
 */
export function hedgeFieldUnprovidedReason(field: HedgeField): string | null {
  const p = hedgeFieldSpec(field).provider;
  return p.state === "unprovided" ? p.reason : null;
}

/** Whether `op` is legal for `field`'s kind (mirrors `RouteOp::valid_for`). */
export function opValidForHedgeField(field: HedgeField, op: RouteOp): boolean {
  return hedgeFieldSpec(field).validOps.includes(op);
}

/** Static value suggestions for the `breached` enum. */
export const BREACHED_VALUES: readonly string[] = ["true", "false"];

/** Static value suggestions for the `product` enum (advisory). */
export const HEDGE_PRODUCT_VALUES: readonly string[] = [
  "vanilla",
  "swap",
  "bond",
  "forward",
  "ndf",
  "option",
];

/** Static value suggestions for the `ccy` enum (advisory). */
export const HEDGE_CCY_VALUES: readonly string[] = [
  "EUR",
  "USD",
  "GBP",
  "JPY",
  "EURUSD",
  "GBPUSD",
  "USDJPY",
];
