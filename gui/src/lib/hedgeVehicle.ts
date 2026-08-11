/**
 * The hedge-VEHICLE vocabulary (docs/HEDGING-AND-RISK-EXIT §6, §8.2): what a size-bearing
 * exit action actually trades to shed risk, and how a target DV01 becomes a tradeable
 * number of units.
 *
 * The motivating asymmetry: selling back the SAME security is exact — leg and fill are the
 * identical instrument so their DV01 ratio is identically 1 under any duration measure —
 * but it is not how a desk hedges a corporate bond. A corp is hedged with a BENCHMARK at
 * matching maturity, in practice a Treasury future, and the moment the hedge instrument
 * differs from the position the ratio stops being 1 and a DV01-per-unit is REQUIRED. That
 * number can only come from the firm's vehicle registry, which is why a named vehicle must
 * be a registry row and why the registry validation below is strict about it.
 *
 * Kept in `lib/` (no component imports, no transport) so the model stays unit-testable and
 * shared by the leaf editor, the registry table and the suggestion surface alike.
 */
import type {
  ExitActionKind,
  HedgeExitMode,
  HedgeExitModeBinding,
  HedgeScopeKind,
  HedgeVehicleKind,
  HedgeVehiclePlan,
  HedgeVehicleRule,
} from "../data/contract";

/** Every vehicle kind, in wire-ordinal / picker order. */
export const HEDGE_VEHICLE_KINDS: readonly HedgeVehicleKind[] = [
  "self",
  "benchmark",
  "instrument",
  "future",
];

/** A short human label for a vehicle kind. */
export function vehicleKindLabel(kind: HedgeVehicleKind): string {
  switch (kind) {
    case "self":
      return "Same security (self)";
    case "benchmark":
      return "Benchmark (by maturity)";
    case "instrument":
      return "Named instrument";
    case "future":
      return "Named future";
  }
}

/** A one-line explanation of what a vehicle kind does — the picker's inline help. */
export function vehicleKindHint(kind: HedgeVehicleKind): string {
  switch (kind) {
    case "self":
      return "Sell back the same security. The DV01 ratio is exactly 1 — no registry, no duration model, no residual.";
    case "benchmark":
      return "Resolve the hedge instrument from the vehicle registry by instrument + maturity bucket — a 9y corp picks up the 7–10y benchmark row.";
    case "instrument":
      return "Hedge with one explicitly named cash instrument. It must be in the vehicle registry — that row carries its DV01 per unit.";
    case "future":
      return "Hedge with one explicitly named future, sized in WHOLE contracts. It must be in the vehicle registry.";
  }
}

/** Whether a vehicle kind NAMES an instrument (⇒ the instrument field applies). */
export function vehicleNamesInstrument(kind: HedgeVehicleKind): boolean {
  return kind === "instrument" || kind === "future";
}

/**
 * Whether an exit-action kind places an order and therefore CARRIES a vehicle.
 * `warehouse` / `skew` / `escalate` place no order, so asking what they hedge with is
 * meaningless and the picker is not shown for them.
 */
export function actionUsesVehicle(kind: ExitActionKind): boolean {
  return (
    kind === "submit_market_order" ||
    kind === "rfq_out" ||
    kind === "split" ||
    kind === "clear_risk" ||
    kind === "cross_internal"
  );
}

/** A fresh registry row with the sensible defaults (a cash 1mm-face vehicle). */
export function newHedgeVehicleRule(id: string): HedgeVehicleRule {
  return {
    id,
    instrumentId: "",
    product: "",
    ccy: "",
    minMaturityYears: 0,
    maxMaturityYears: 0,
    hedgeInstrumentId: "",
    isFuture: false,
    dv01PerUnit: 0,
    unitLabel: "1mm face",
  };
}

/** The unit label implied by the future flag, used when the trader flips it. */
export function defaultUnitLabel(isFuture: boolean): string {
  return isFuture ? "contract" : "1mm face";
}

/**
 * Validate ONE registry row against the server's write boundary, returning EVERY problem
 * rather than the first — a half-corrected row that fails again on save is the worst of
 * both worlds.
 *
 * `otherIds` is the id set of the rest of the roster, so a duplicate is caught client-side
 * instead of coming back as an opaque server rejection.
 */
export function validateHedgeVehicleRule(
  rule: HedgeVehicleRule,
  otherIds: readonly string[],
): string[] {
  const errors: string[] = [];
  const id = rule.id.trim();
  if (id.length === 0) errors.push("Id is required — it is how a rule names this vehicle.");
  else if (otherIds.includes(id)) errors.push(`Id “${id}” is already used by another vehicle.`);

  if (rule.hedgeInstrumentId.trim().length === 0) {
    errors.push("Hedge instrument is required — it is the security this vehicle actually trades.");
  }
  if (!(rule.dv01PerUnit > 0)) {
    errors.push(
      "DV01 per unit must be greater than 0 — a zero or negative DV01 cannot size a hedge.",
    );
  }
  // The bucket is half-open [min, max). Either bound left at 0 means "unbounded on that
  // side", so only a row that sets at least one of them is checked for ordering.
  const bounded = rule.minMaturityYears !== 0 || rule.maxMaturityYears !== 0;
  if (bounded && !(rule.maxMaturityYears > rule.minMaturityYears)) {
    errors.push("Max maturity must be greater than min maturity (the bucket is [min, max)).");
  }
  if (rule.minMaturityYears < 0 || rule.maxMaturityYears < 0) {
    errors.push("Maturity bounds cannot be negative.");
  }
  if (rule.unitLabel.trim().length === 0) {
    errors.push("Unit label is required — a size is meaningless without its unit.");
  }
  return errors;
}

/** Validate the WHOLE roster (per-row problems, prefixed with the offending row). */
export function validateHedgeVehicleRegistry(rules: readonly HedgeVehicleRule[]): string[] {
  const out: string[] = [];
  rules.forEach((rule, i) => {
    const others = rules.filter((_, j) => j !== i).map((r) => r.id.trim());
    for (const e of validateHedgeVehicleRule(rule, others)) {
      out.push(`${rule.id.trim().length > 0 ? rule.id : `row ${i + 1}`}: ${e}`);
    }
  });
  return out;
}

/** The registry's hedge-instrument ids — the ONLY legal values for a named vehicle. */
export function registeredHedgeInstruments(rules: readonly HedgeVehicleRule[]): string[] {
  const seen = new Set<string>();
  for (const r of rules) {
    const id = r.hedgeInstrumentId.trim();
    if (id.length > 0) seen.add(id);
  }
  return [...seen].sort((a, b) => a.localeCompare(b));
}

/**
 * Is this action's named vehicle one the registry knows? A `self` / `benchmark` vehicle
 * names nothing, so it is always fine; a named one that the registry cannot price has no
 * DV01-per-unit and the server rejects it.
 */
export function namedVehicleIsRegistered(
  kind: HedgeVehicleKind,
  instrument: string,
  rules: readonly HedgeVehicleRule[],
): boolean {
  if (!vehicleNamesInstrument(kind)) return true;
  const wanted = instrument.trim();
  if (wanted.length === 0) return false;
  return registeredHedgeInstruments(rules).includes(wanted);
}

/** Render a maturity bucket for display; a bucket with neither bound set reads as "any". */
export function maturityBucketLabel(rule: HedgeVehicleRule): string {
  if (rule.minMaturityYears === 0 && rule.maxMaturityYears === 0) return "any";
  return `${rule.minMaturityYears}–${rule.maxMaturityYears}y`;
}

/** Render a row's match axes ("any" for each unset axis) as one compact string. */
export function matchLabel(rule: HedgeVehicleRule): string {
  const parts = [
    rule.instrumentId.trim().length > 0 ? rule.instrumentId : "any instrument",
    rule.product.trim().length > 0 ? rule.product : "any product",
    rule.ccy.trim().length > 0 ? rule.ccy : "any ccy",
  ];
  return parts.join(" · ");
}

// --- the suggestion surface's honesty helpers -------------------------------

/**
 * Plain-English wording for a vehicle plan's RESIDUAL, with the SIGN spelled out.
 *
 * The sign is the whole point and is easy to read backwards: `residualDv01` is
 * `target − hedged`, so POSITIVE means the rounding left risk ON the book (under-hedged)
 * and NEGATIVE means it took off more than the target (over-hedged). A bare signed number
 * next to a size invites exactly the wrong reading, so this returns the sentence.
 */
export function residualWording(plan: HedgeVehiclePlan): string {
  const magnitude = Math.abs(plan.residualDv01);
  const dv01 = formatDv01(magnitude);
  if (magnitude < 1e-9) return "exact — no residual";
  if (plan.residualDv01 > 0) return `rounded down, ${dv01} DV01 still on the book`;
  return `rounded up, over-hedged by ${dv01} DV01`;
}

/** A compact DV01 magnitude (no currency symbol — the metric's own units). */
export function formatDv01(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1000) {
    return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value);
  }
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: abs < 10 ? 2 : 0 }).format(value);
}

/**
 * Whether a plan's size came off a DURATION-BLIND proxy and must be flagged.
 *
 * `dv01_basis === "exposure-proxy"` is the coarse `redemption × 1bp` exposure measure
 * (docs/HEDGING-AND-RISK-EXIT §10.1), which treats every bond as though it had a duration
 * of 1 — a 10-year bond's true DV01 is roughly 8× what it reports. Sizing a DIFFERENT
 * instrument off that number is approximate by construction, so the surface must say so:
 * a proxy-based size is never presented as exact.
 */
export function isDurationProxy(plan: HedgeVehiclePlan): boolean {
  return !plan.durationCorrect;
}

/** The warning text shown when {@link isDurationProxy} holds. */
export const DURATION_PROXY_WARNING =
  "Approximate size — computed off a duration-blind exposure proxy, not a curve DV01. Check it before trading.";

// --- exit-mode bindings -----------------------------------------------------

/** Both exit modes, in wire-ordinal order. */
export const HEDGE_EXIT_MODES: readonly HedgeExitMode[] = ["auto", "suggest"];

/** A short human label for an exit mode. */
export function exitModeLabel(mode: HedgeExitMode): string {
  return mode === "auto" ? "Auto — fire on breach" : "Suggest — raise a standing row";
}

/**
 * A one-line explanation of an exit mode. The `suggest` copy is deliberately explicit
 * that it is NOT a popup: the point of the mode is a row that survives being ignored.
 */
export function exitModeHint(mode: HedgeExitMode): string {
  return mode === "auto"
    ? "A breach resolves the policy and TRADES immediately. This is the existing behaviour."
    : "The engine measures, resolves the policy and sizes the hedge, then trades nothing and puts a STANDING row on the risk panel with a “Hedge now” button. It is never a confirmation dialog — the row waits until you act on it.";
}

/**
 * Resolve the exit mode governing a (desk, book, instrument) triple, most-specific-wins
 * exactly like the LP panels: instrument > book > desk. An unbound scope is `auto`, which
 * is the pre-feature behaviour.
 */
export function resolveExitMode(
  bindings: readonly HedgeExitModeBinding[],
  scope: { instrument?: string; book?: string; desk?: string },
): HedgeExitMode {
  const order: readonly { kind: HedgeScopeKind; id: string | undefined }[] = [
    { kind: "instrument", id: scope.instrument },
    { kind: "book", id: scope.book },
    { kind: "desk", id: scope.desk },
  ];
  for (const { kind, id } of order) {
    if (id === undefined || id.length === 0) continue;
    const hit = bindings.find((b) => b.scopeKind === kind && b.scopeId === id);
    if (hit !== undefined) return hit.mode;
  }
  return "auto";
}

/** Validate one exit-mode binding (the scope id is the only free text). */
export function validateExitModeBinding(
  binding: HedgeExitModeBinding,
  others: readonly HedgeExitModeBinding[],
): string[] {
  const errors: string[] = [];
  const id = binding.scopeId.trim();
  if (id.length === 0) errors.push("Scope id is required.");
  else if (others.some((b) => b.scopeKind === binding.scopeKind && b.scopeId.trim() === id)) {
    errors.push(`A ${binding.scopeKind} binding for “${id}” already exists.`);
  }
  return errors;
}
