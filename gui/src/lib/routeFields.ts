/**
 * The routable-field registry for the risk-routing decision graph
 * (docs/FI-RISK-ROUTING-REQUIREMENTS.md §8.6). Every {@link RouteField} the flow
 * canvas can test is described here ONCE: its human label, the palette group it
 * lives in, its value KIND (enum / numeric / string), the operators legal for that
 * kind (mirrors `celnet_risk_routing::RouteOp::valid_for`, same matrix), and — for
 * enum fields — which live roster supplies the dropdown of valid values.
 *
 * The kind ↔ field mapping is a byte-faithful mirror of the Rust
 * `RouteField::kind` (`crates/celnet-risk-routing/src/field.rs`): the six
 * enum-typed fields (side/product/ccy/counterparty/user/desk) produce a text
 * context; the four numeric fields (notional/tenor/strike/price) a number; and
 * `instrument_id` is a free string. Keeping the matrices here (not re-derived per
 * component) is what lets {@link ../workspaces/riskrouting/NodeEditor} render a
 * TYPED value editor and the validator reject a graph the server would reject.
 */
import type { RouteField, RouteOp } from "../data/contract";

/** The value shape of a field — pins which operators and value editor apply. */
export type FieldKind = "enum" | "numeric" | "string";

/** Which live roster fills an enum field's value dropdown (or a static list). */
export type EnumSource = "side" | "product" | "ccy" | "desk" | "counterparty";

/** The palette section a field is grouped under. */
export type FieldGroup = "Instrument" | "Trade" | "Counterparty";

/** One routable field's full descriptor (the palette + editor read this). */
export interface FieldSpec {
  /** The wire field selector. */
  field: RouteField;
  /** Human-friendly chip / editor label. */
  label: string;
  /** The palette section it belongs to. */
  group: FieldGroup;
  /** Its value kind — drives the operator set + value editor. */
  kind: FieldKind;
  /** The operators legal for this field's kind (mirrors `RouteOp::valid_for`). */
  validOps: RouteOp[];
  /** For an enum field, the roster that supplies its value dropdown (else free-typed). */
  enumSource?: EnumSource;
  /** A one-line description shown on the palette chip + editor. */
  hint: string;
}

/** Numeric fields accept ordering + equality + range (no substring / membership-by-list). */
const NUMERIC_OPS: RouteOp[] = ["eq", "ne", "gt", "ge", "lt", "le", "between"];
/** Free strings accept equality + substring + list membership (no ordering). */
const STRING_OPS: RouteOp[] = ["eq", "ne", "contains", "in"];
/** Enums accept equality + list membership only (no substring — an enum has no infix). */
const ENUM_OPS: RouteOp[] = ["eq", "ne", "in"];

/**
 * The full field registry, in palette-render order. Grouped Instrument → Trade →
 * Counterparty; within a group the order here is the chip order.
 */
export const FIELD_REGISTRY: readonly FieldSpec[] = [
  // --- Instrument ---------------------------------------------------------
  {
    field: "instrument_id",
    label: "Instrument",
    group: "Instrument",
    kind: "string",
    validOps: STRING_OPS,
    hint: "Symbol / identifier of the filled instrument (e.g. EURUSD-1Y).",
  },
  {
    field: "ccy",
    label: "Currency",
    group: "Instrument",
    kind: "enum",
    validOps: ENUM_OPS,
    enumSource: "ccy",
    hint: "Currency or pair of the fill.",
  },
  {
    field: "product",
    label: "Product",
    group: "Instrument",
    kind: "enum",
    validOps: ENUM_OPS,
    enumSource: "product",
    hint: "Product family — vanilla, swap, bond, forward…",
  },
  {
    field: "strike",
    label: "Strike",
    group: "Instrument",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Strike / resolved level, in price space.",
  },
  {
    field: "price",
    label: "Price",
    group: "Instrument",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Fill price / premium.",
  },
  // --- Trade --------------------------------------------------------------
  {
    field: "side",
    label: "Side",
    group: "Trade",
    kind: "enum",
    validOps: ENUM_OPS,
    enumSource: "side",
    hint: "Trade direction — Buy or Sell.",
  },
  {
    field: "notional",
    label: "Notional",
    group: "Trade",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Absolute base notional of the fill (side-agnostic).",
  },
  {
    field: "tenor",
    label: "Tenor",
    group: "Trade",
    kind: "numeric",
    validOps: NUMERIC_OPS,
    hint: "Tenor / expiry, in years.",
  },
  // --- Counterparty -------------------------------------------------------
  {
    field: "counterparty",
    label: "Counterparty",
    group: "Counterparty",
    kind: "enum",
    validOps: ENUM_OPS,
    enumSource: "counterparty",
    hint: "Originating FIX session / counterparty.",
  },
  {
    field: "user",
    label: "User",
    group: "Counterparty",
    kind: "enum",
    validOps: ENUM_OPS,
    hint: "Booking user of the fill.",
  },
  {
    field: "desk",
    label: "Desk",
    group: "Counterparty",
    kind: "enum",
    validOps: ENUM_OPS,
    enumSource: "desk",
    hint: "Owning desk of the fill.",
  },
];

/** Palette section order. */
export const FIELD_GROUPS: readonly FieldGroup[] = ["Instrument", "Trade", "Counterparty"];

const BY_FIELD: ReadonlyMap<RouteField, FieldSpec> = new Map(
  FIELD_REGISTRY.map((s) => [s.field, s]),
);

/** The descriptor for a field (throws on an unknown selector — a programming error). */
export function fieldSpec(field: RouteField): FieldSpec {
  const s = BY_FIELD.get(field);
  if (!s) throw new Error(`routeFields: unknown field \`${field}\``);
  return s;
}

/** The value kind of a field — the single mirror of `RouteField::kind`. */
export function fieldKind(field: RouteField): FieldKind {
  return fieldSpec(field).kind;
}

/** Whether `op` is legal for `field`'s kind (mirrors `RouteOp::valid_for`). */
export function opValidForField(field: RouteField, op: RouteOp): boolean {
  return fieldSpec(field).validOps.includes(op);
}

/** Static value suggestions for the `side` enum. */
export const SIDE_VALUES: readonly string[] = ["Buy", "Sell"];

/** Static value suggestions for the `product` enum (extend freely — advisory only). */
export const PRODUCT_VALUES: readonly string[] = [
  "vanilla",
  "swap",
  "bond",
  "forward",
  "ndf",
  "fra",
  "irs",
  "option",
  "digital",
  "barrier",
];

/** Static value suggestions for the `ccy` enum (common ccys + majors). */
export const CCY_VALUES: readonly string[] = [
  "EUR",
  "USD",
  "GBP",
  "JPY",
  "CHF",
  "AUD",
  "CAD",
  "EURUSD",
  "GBPUSD",
  "USDJPY",
];

/** Human label for an operator (the editor dropdown + edge affordances share it). */
export function opLabel(op: RouteOp): string {
  switch (op) {
    case "eq":
      return "equals";
    case "ne":
      return "not equal";
    case "gt":
      return "greater than";
    case "ge":
      return "≥";
    case "lt":
      return "less than";
    case "le":
      return "≤";
    case "contains":
      return "contains";
    case "in":
      return "in list";
    case "between":
      return "between";
  }
}

/** A compact operator glyph for the node card (kept legible at small sizes). */
export function opGlyph(op: RouteOp): string {
  switch (op) {
    case "eq":
      return "=";
    case "ne":
      return "≠";
    case "gt":
      return ">";
    case "ge":
      return "≥";
    case "lt":
      return "<";
    case "le":
      return "≤";
    case "contains":
      return "⊃";
    case "in":
      return "∈";
    case "between":
      return "⇔";
  }
}
