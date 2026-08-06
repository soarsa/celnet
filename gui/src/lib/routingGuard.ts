/**
 * routingGuard — the pure, UI-free predicate behind the startup "no default risk
 * portfolio" guard. A firm's risk-routing graph is a first-match-wins decision tree
 * whose FALSE spine terminates at a catch-all (Otherwise) leaf: the risk book every
 * fill lands in when NO condition matches. If that catch-all leaf is missing, points
 * at nothing, or points at a DISABLED book, unmatched fills route into the void —
 * their risk is never booked into a portfolio. This module decides that condition.
 *
 * It reuses the shipped routing decompile ({@link decompileGraphToRules}) rather than
 * re-walking the raw node graph: the decompiler already models the catch-all as the
 * trailing rule with an empty condition set (walking the `on_false` spine from the
 * entry to the terminal book leaf), so the "default routed book" is exactly that
 * rule's destination. Keeping this here (not in a component) makes it unit-testable
 * without the UI and reusable by any client surface.
 *
 * Layering: depends ONLY on the wire contract + `lib/riskRules`; imports no
 * component, hook, or transport.
 */
import type { RiskBook, RiskRoutingGraph } from "../data/contract";
import { decompileGraphToRules } from "./riskRules";

/**
 * Why a firm has (or lacks) a valid default routed portfolio. `ok` is the single
 * success reason; the rest are the distinct failure modes the popup copy can adapt
 * its guidance to.
 */
export type DefaultRouteReason =
  | "ok"
  | "no-graph"
  | "no-default"
  | "empty-book"
  | "book-not-found"
  | "book-disabled";

/** The resolved default-routing state (the pure predicate's whole output). */
export interface DefaultRouteResolution {
  /** True iff the catch-all default leaf targets an ENABLED risk book. */
  ok: boolean;
  /** The specific state (drives both the guard decision and the popup copy). */
  reason: DefaultRouteReason;
  /** The default leaf's target book id, when the graph names one (else `null`). */
  bookId: string | null;
  /** The resolved book's display name, when it exists in the roster (else `null`). */
  bookName: string | null;
}

/** A resolution helper — keeps the returned object shape consistent at each exit. */
function resolution(
  ok: boolean,
  reason: DefaultRouteReason,
  bookId: string | null,
  bookName: string | null,
): DefaultRouteResolution {
  return { ok, reason, bookId, bookName };
}

/**
 * Resolve whether the installed routing graph has a valid catch-all destination — a
 * default/Otherwise leaf that targets an ENABLED risk book. Returns `ok: true` ONLY
 * in that case; every other state (no graph, no catch-all rule, an unset/empty target
 * id, a target that is not in the roster, or a target that is DISABLED) returns
 * `ok: false` with the reason, so the guard warns.
 *
 *   - `graph` is the firm-wide {@link RiskRoutingGraph}, or `null` when none is
 *     installed (⇒ `no-graph`).
 *   - `books` is the risk-portfolio roster (`listRiskBooks`); a book is a valid
 *     target only when present AND `enabled`.
 *
 * The catch-all is found by decompiling the graph to the ordered rule table and
 * taking the trailing rule with no conditions — the exact model the routing builder
 * and {@link decompileGraphToRules} use for the default rule, so a nested `on_false`
 * chain resolves to the true terminal leaf rather than an intermediate one.
 */
export function resolveDefaultRoutedBook(
  graph: RiskRoutingGraph | null,
  books: readonly RiskBook[],
): DefaultRouteResolution {
  // No graph installed at all — every fill is unrouted.
  if (graph === null || graph.nodes.length === 0) {
    return resolution(false, "no-graph", null, null);
  }

  // Decompile to the ordered rule table; the catch-all is the (at most one) rule
  // with an empty condition set — the terminal of the false spine. decompile pushes
  // it last and stops there, so `find` recovers exactly that terminal leaf.
  const rules = decompileGraphToRules(graph);
  const catchAll = rules.find((r) => r.conditions.length === 0);
  if (catchAll === undefined) {
    // The false spine never reached an unconditional leaf (dangling/cyclic wiring):
    // there is no catch-all, so unmatched fills fall off the end.
    return resolution(false, "no-default", null, null);
  }

  const bookId = catchAll.bookId;
  if (bookId === null || bookId.length === 0) {
    // A catch-all with no destination chosen.
    return resolution(false, "empty-book", null, null);
  }

  const book = books.find((b) => b.id === bookId);
  if (book === undefined) {
    // The catch-all names a book id that is not in the roster (deleted/renamed).
    return resolution(false, "book-not-found", bookId, null);
  }
  if (!book.enabled) {
    // The catch-all points at a DISABLED book — not a valid routing target, so the
    // server would reject or strand the fill.
    return resolution(false, "book-disabled", bookId, book.name);
  }

  return resolution(true, "ok", bookId, book.name);
}

/**
 * The window event the guard listens for to RE-EVALUATE live: any surface that
 * mutates the routing graph or the risk-portfolio roster (the guided-setup wizard's
 * Apply, the routing-graph editor's Save, the portfolio editor) dispatches this so a
 * newly-set valid default retires the warning immediately, without a reload.
 */
export const RISK_ROUTING_CHANGED_EVENT = "celnet:risk-routing-changed";

/** Dispatch {@link RISK_ROUTING_CHANGED_EVENT} (no-op outside a browser context). */
export function notifyRiskRoutingChanged(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(RISK_ROUTING_CHANGED_EVENT));
}
