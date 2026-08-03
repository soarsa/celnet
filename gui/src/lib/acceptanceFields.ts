/**
 * The lift-field registry for the INCOMING-QUOTE-ACCEPTANCE decision graph
 * (`celnet-acceptance`). Every {@link AcceptanceField} an acceptance condition can
 * test is described here ONCE: its human label, the palette group it lives in, its
 * value KIND (enum / numeric / string), and the operators legal for that kind. It is
 * the exact analogue of `lib/hedgeFields.ts` for the acceptance graph — the leaves
 * differ (accept/reject/hold decisions, not exit actions) but the CONDITION
 * vocabulary is the same shape and reuses the shared {@link RouteOp} matrix.
 *
 * The kind ↔ field mapping is a byte-faithful mirror of the Rust
 * `celnet_acceptance::AcceptanceField::kind` (`crates/celnet-acceptance/src/field.rs`):
 * `counterparty` / `side` / `asset_class` / `desk` are ENUM (compared by equality /
 * membership); `instrument_symbol` is a free STRING (supports substring); every other
 * lift attribute (notional_usd / tenor_years / edge_bps / quote_age_ms) is NUMERIC.
 */
import type { AcceptanceField, RouteOp } from "../data/contract";
import { type FieldKind, ENUM_LIKE_OPS, NUMERIC_OPS, STRING_OPS } from "./routeOps";

/** The palette section an acceptance field is grouped under. */
export type AcceptanceFieldGroup = "Identity" | "Economics" | "Timing";

/** One acceptance lift-field's full descriptor (the palette + editor read this). */
export interface AcceptanceFieldSpec {
  /** The wire field selector. */
  field: AcceptanceField;
  /** Human-friendly chip / editor label. */
  label: string;
  /** The palette section it belongs to. */
  group: AcceptanceFieldGroup;
  /** Its value kind — drives the operator set + value editor. */
  kind: FieldKind;
  /** The operators legal for this field's kind (mirrors `RouteOp::valid_for`). */
  validOps: RouteOp[];
  /** A one-line description shown on the palette chip + editor. */
  hint: string;
}

/**
 * The full acceptance-field registry, in the SAME ordinal order 0..8 as the Rust
 * `AcceptanceField` enum and the wire `AcceptanceFieldEnum`.
 */
export const ACCEPTANCE_FIELD_REGISTRY: readonly AcceptanceFieldSpec[] = [
  {
    field: "counterparty",
    label: "Counterparty",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The originating client / party id of the lift.",
  },
  {
    field: "notional_usd",
    label: "Notional (USD)",
    group: "Economics",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Absolute USD notional of the lifted amount.",
  },
  {
    field: "tenor_years",
    label: "Tenor (years)",
    group: "Economics",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Years to maturity of the lifted instrument.",
  },
  {
    field: "instrument_symbol",
    label: "Instrument",
    group: "Identity",
    kind: "string",
    validOps: STRING_OPS,
    hint: "Instrument / curve / security symbol (substring matches a family prefix).",
  },
  {
    field: "side",
    label: "Side",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Lift side — pay / receive / buy / sell.",
  },
  {
    field: "edge_bps",
    label: "Edge (bps)",
    group: "Economics",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Signed dealer edge vs the engine mid in bp (positive ⇒ captured spread).",
  },
  {
    field: "quote_age_ms",
    label: "Quote age (ms)",
    group: "Timing",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Age of the quote at lift time (now − mint time), in milliseconds.",
  },
  {
    field: "asset_class",
    label: "Asset class",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "Owning asset class — fx_options / fixed_income.",
  },
  {
    field: "desk",
    label: "Desk",
    group: "Identity",
    kind: "enum",
    validOps: ENUM_LIKE_OPS,
    hint: "The owning / target desk.",
  },
];

/** Palette section order. */
export const ACCEPTANCE_FIELD_GROUPS: readonly AcceptanceFieldGroup[] = [
  "Identity",
  "Economics",
  "Timing",
];

const BY_FIELD: ReadonlyMap<AcceptanceField, AcceptanceFieldSpec> = new Map(
  ACCEPTANCE_FIELD_REGISTRY.map((s) => [s.field, s]),
);

/** The descriptor for an acceptance field (throws on an unknown selector — a bug). */
export function acceptanceFieldSpec(field: AcceptanceField): AcceptanceFieldSpec {
  const s = BY_FIELD.get(field);
  if (!s) throw new Error(`acceptanceFields: unknown field \`${field}\``);
  return s;
}

/** The value kind of an acceptance field — the single mirror of `AcceptanceField::kind`. */
export function acceptanceFieldKind(field: AcceptanceField): FieldKind {
  return acceptanceFieldSpec(field).kind;
}

/** Whether `op` is legal for `field`'s kind (mirrors `RouteOp::valid_for`). */
export function opValidForAcceptanceField(field: AcceptanceField, op: RouteOp): boolean {
  return acceptanceFieldSpec(field).validOps.includes(op);
}

/** Static value suggestions for the `side` enum (advisory). */
export const ACCEPTANCE_SIDE_VALUES: readonly string[] = ["buy", "sell", "pay", "receive"];

/** Static value suggestions for the `asset_class` enum (the two wire asset classes). */
export const ACCEPTANCE_ASSET_CLASS_VALUES: readonly string[] = ["fixed_income", "fx_options"];
