/**
 * Helpers for the {@link ExitAction} leaf vocabulary of a hedge policy graph
 * (docs/AUTO-HEDGING §5.3): the human labels, a fresh default action per kind, the
 * per-kind field relevance (which of the flat union's fields an action actually
 * reads), and a plain-English one-liner. Kept in `lib/` (no component imports) so
 * the action model stays unit-testable and reusable by any client surface.
 */
import type { ExecStyle, ExitAction, ExitActionKind, HedgeSize, HedgeSizeKind } from "../data/contract";

/** Every exit-action kind, in palette / picker order (mirrors `ExitActionKind`). */
export const EXIT_ACTION_KINDS: readonly ExitActionKind[] = [
  "warehouse",
  "cross_internal",
  "skew",
  "submit_market_order",
  "rfq_out",
  "split",
  "escalate",
  "clear_risk",
];

/**
 * Whether an exit-action kind trades AWAY (leaves the firm) — the EXTERNAL set
 * (mirrors the server `ExitAction::is_external`): submit_market_order / rfq_out /
 * split / clear_risk. The internal set (warehouse / cross_internal / skew / escalate)
 * never externalises. Used to show the hedge desk hedging-only by default.
 */
export function isExternalExitAction(kind: ExitActionKind): boolean {
  return (
    kind === "submit_market_order" ||
    kind === "rfq_out" ||
    kind === "split" ||
    kind === "clear_risk"
  );
}

/** A short human label for an exit-action kind. */
export function exitActionLabel(kind: ExitActionKind): string {
  switch (kind) {
    case "warehouse":
      return "Warehouse (hold)";
    case "cross_internal":
      return "Cross internal";
    case "skew":
      return "Skew to attract";
    case "submit_market_order":
      return "Submit market order";
    case "rfq_out":
      return "RFQ out";
    case "split":
      return "Split (net then hedge)";
    case "escalate":
      return "Escalate to desk";
    case "clear_risk":
      return "Clear risk";
  }
}

/** A one-line description of what an exit-action kind does (editor + palette hint). */
export function exitActionHint(kind: ExitActionKind): string {
  switch (kind) {
    case "warehouse":
      return "Hold the risk — the green-band default.";
    case "cross_internal":
      return "Offset against opposing internal flow in the Agg Book (@ mid).";
    case "skew":
      return "Lean the two-way to pull in the offsetting side — no trade.";
    case "submit_market_order":
      return "Back-to-back: place an offsetting external order onto the RFQ/FIX panel.";
    case "rfq_out":
      return "Request a two-way from named LPs and lift the best.";
    case "split":
      return "Net internally up to the offset, externalise the residual.";
    case "escalate":
      return "Fire an alert / hand to a human desk instead of auto-acting.";
    case "clear_risk":
      return "Flatten the book's entire net to zero via the live composite.";
  }
}

/** The default sizing choice (hedge the overflow to the band edge). */
export function defaultHedgeSize(): HedgeSize {
  return { kind: "overflow", fixed: 0 };
}

/** A short human label for a sizing kind. */
export function hedgeSizeLabel(kind: HedgeSizeKind): string {
  switch (kind) {
    case "overflow":
      return "Overflow (to band edge)";
    case "full":
      return "Full (flatten)";
    case "fixed":
      return "Fixed amount";
  }
}

/** A short human label for an execution style. */
export function execStyleLabel(style: ExecStyle): string {
  switch (style) {
    case "immediate":
      return "Immediate (one clip)";
    case "worked":
      return "Worked (sliced)";
  }
}

/**
 * A fresh, fully-formed {@link ExitAction} of `kind` with the sensible defaults. The
 * vehicle defaults to `self` — the same security sold back — which is the behaviour every
 * exit action had before the vehicle choice existed, so a newly authored leaf is never
 * silently given a different hedge instrument than the trader asked for.
 */
export function defaultExitAction(kind: ExitActionKind): ExitAction {
  return {
    kind,
    instrument: "",
    size: defaultHedgeSize(),
    skewBp: null,
    toEdge: kind === "skew",
    style: "immediate",
    lps: [],
    internalFirst: kind === "split",
    reason: "",
    vehicleKind: "self",
    vehicleInstrument: "",
  };
}

/** Whether an action kind reads a `size` (cross/market-order/rfq/split). */
export function actionUsesSize(kind: ExitActionKind): boolean {
  return (
    kind === "cross_internal" ||
    kind === "submit_market_order" ||
    kind === "rfq_out" ||
    kind === "split"
  );
}

/** Whether an action kind reads an execution `style` (market-order/split). */
export function actionUsesStyle(kind: ExitActionKind): boolean {
  return kind === "submit_market_order" || kind === "split";
}

/** Render a sizing choice as compact text. */
function sizeText(size: HedgeSize): string {
  switch (size.kind) {
    case "overflow":
      return "overflow";
    case "full":
      return "flatten";
    case "fixed":
      return `${size.fixed}`;
  }
}

/**
 * The vehicle suffix appended to a size-bearing action's description, e.g.
 * ` · via TY-DEC26`. A `self` vehicle adds NOTHING — it is the default and the behaviour
 * every pre-vehicle rule already had, so spelling it out on every row would be noise.
 */
export function vehicleSuffix(action: ExitAction): string {
  switch (action.vehicleKind) {
    case "self":
      return "";
    case "benchmark":
      return " · via benchmark";
    case "instrument":
    case "future":
      return action.vehicleInstrument.length > 0
        ? ` · via ${action.vehicleInstrument}`
        : " · via (vehicle?)";
  }
}

/**
 * A plain-English one-liner for an exit action, e.g.
 * `Submit market order · overflow · immediate`, used on rule rows + the trace panel.
 */
export function describeExitAction(action: ExitAction | null): string {
  if (action === null) return "(no action)";
  return describeExitActionBody(action) + vehicleSuffix(action);
}

/** The action description WITHOUT its vehicle suffix (see {@link describeExitAction}). */
function describeExitActionBody(action: ExitAction): string {
  const label = exitActionLabel(action.kind);
  switch (action.kind) {
    case "warehouse":
      return label;
    case "cross_internal":
      return `${label} · ${action.instrument.length > 0 ? action.instrument : "(instrument?)"} · ${sizeText(action.size)}`;
    case "skew":
      return action.toEdge
        ? `${label} · to edge`
        : `${label} · ${action.skewBp ?? 0} bp`;
    case "submit_market_order":
      return `${label} · ${sizeText(action.size)} · ${action.style}`;
    case "rfq_out":
      return `${label} · [${action.lps.join(", ")}] · ${sizeText(action.size)}`;
    case "split":
      return `${label} · ${action.internalFirst ? "internal first" : "external first"} · ${action.style}`;
    case "escalate":
      return `${label}${action.reason.length > 0 ? ` · ${action.reason}` : ""}`;
    case "clear_risk":
      return `${label} · flatten to zero`;
  }
}
