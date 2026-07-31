/**
 * The risk-state field registry for the AUTO-HEDGE exit-policy decision graph
 * (docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md §5.2). Every
 * {@link HedgeField} a hedge condition can test is described here ONCE: its human
 * label, the palette group it lives in, its value KIND (enum / numeric / string),
 * and the operators legal for that kind. It is the exact analogue of
 * `lib/routeFields.ts` for the hedge graph — the leaves differ (exit actions, not
 * book targets) but the CONDITION vocabulary is the same shape.
 *
 * The kind ↔ field mapping is a byte-faithful mirror of the Rust
 * `celnet_hedge_routing::HedgeField::kind` (`crates/celnet-hedge-routing/src/field.rs`):
 * `breached` + the five identity fields (ccy/product/book/desk/breached) are ENUM
 * (compared by equality / membership); `instrument_id` is free STRING; every other
 * risk-state number (net_dv01 … hedge_cost_bp) is NUMERIC. Keeping the matrices here
 * (not re-derived per component) lets the editor render a TYPED value editor and the
 * client-side validator reject a graph the server would reject.
 */
import type { HedgeField, RouteOp } from "../data/contract";
import { type FieldKind, ENUM_LIKE_OPS, NUMERIC_OPS, STRING_OPS } from "./routeOps";

/** The palette section a hedge field is grouped under. */
export type HedgeFieldGroup = "Identity" | "Risk state" | "Budget" | "Flow & market";

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
}

/**
 * The full hedge-field registry, in palette-render order 0..17 — the SAME ordinal
 * order as the Rust `HedgeField` enum and the proto `HedgeFieldEnum`.
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
  },
  {
    field: "ccy",
    label: "Currency",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Currency / pair of the risk.",
  },
  {
    field: "product",
    label: "Product",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Product family — vanilla, swap, bond, forward…",
  },
  {
    field: "book",
    label: "Book",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The risk book / portfolio holding the inventory.",
  },
  {
    field: "desk",
    label: "Desk",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The owning desk.",
  },
  // --- Risk state ---------------------------------------------------------
  {
    field: "net_dv01",
    label: "Net DV01",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net DV01 (PV per +1bp) — the FI first-order rate risk.",
  },
  {
    field: "net_notional",
    label: "Net notional",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net base-currency notional / delta (FX).",
  },
  {
    field: "net_vega",
    label: "Net vega",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net vega.",
  },
  {
    field: "net_gamma",
    label: "Net gamma",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed net gamma.",
  },
  {
    field: "inventory_sign",
    label: "Inventory sign",
    group: "Risk state",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "+1 long / −1 short.",
  },
  // --- Budget -------------------------------------------------------------
  {
    field: "threshold",
    label: "Threshold",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "The resolved warehouse threshold (the “100”) for this scope.",
  },
  {
    field: "utilization",
    label: "Utilization",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "|risk| / threshold — the RAG driver (≥1 is a breach).",
  },
  {
    field: "overflow",
    label: "Overflow",
    group: "Budget",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "max(0, |risk| − band edge) — the amount to hedge.",
  },
  {
    field: "breached",
    label: "Breached",
    group: "Budget",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Whether the red band fired — compare with true / false.",
  },
  // --- Flow & market ------------------------------------------------------
  {
    field: "counterparty_toxicity",
    label: "Counterparty toxicity",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Markout / residual toxicity of the flow that built this risk.",
  },
  {
    field: "inventory_age_secs",
    label: "Inventory age (s)",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "How long the risk has sat (aging → forced hedge).",
  },
  {
    field: "internal_offset_available",
    label: "Internal offset avail.",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Opposing internal flow the aggregator could cross now.",
  },
  {
    field: "hedge_cost_bp",
    label: "Hedge cost (bp)",
    group: "Flow & market",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Current external hedge-cost estimate (spread + impact).",
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
