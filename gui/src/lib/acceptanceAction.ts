/**
 * Helpers for the {@link AcceptanceAction} DECISION-leaf vocabulary of an acceptance
 * graph (`celnet-acceptance`): the human labels, a fresh default decision per kind,
 * whether a kind reads a `reason`, and a plain-English one-liner. Kept in `lib/` (no
 * component imports) so the decision model stays unit-testable and reusable by any
 * client surface. The acceptance analogue of `lib/hedgeExit.ts`.
 */
import type { AcceptanceAction, AcceptanceActionKind } from "../data/contract";

/** Every acceptance-decision kind, in picker order (mirrors `AcceptanceActionKind`). */
export const ACCEPTANCE_ACTION_KINDS: readonly AcceptanceActionKind[] = [
  "accept",
  "reject",
  "hold_for_review",
];

/** A short human label for a decision kind. */
export function acceptanceActionLabel(kind: AcceptanceActionKind): string {
  switch (kind) {
    case "accept":
      return "Accept";
    case "reject":
      return "Reject";
    case "hold_for_review":
      return "Hold for review";
  }
}

/** A one-line description of what a decision kind does (editor + palette hint). */
export function acceptanceActionHint(kind: AcceptanceActionKind): string {
  switch (kind) {
    case "accept":
      return "Book the lift exactly as today — the safe accept-all default.";
    case "reject":
      return "Decline the lift; the reason is surfaced to the counterparty (FIX Text 58).";
    case "hold_for_review":
      return "Route the lift to the desk inbox for a human to accept manually.";
  }
}

/** Whether a decision kind reads a `reason` (reject / hold — not accept). */
export function actionUsesReason(kind: AcceptanceActionKind): boolean {
  return kind === "reject" || kind === "hold_for_review";
}

/** A fresh, fully-formed {@link AcceptanceAction} of `kind` (empty reason). */
export function defaultAcceptanceAction(kind: AcceptanceActionKind): AcceptanceAction {
  return { kind, reason: "" };
}

/**
 * A plain-English one-liner for an acceptance decision, e.g. `Reject · "below edge
 * floor"` or `Accept`, used on rule rows + the trace panel.
 */
export function describeAcceptanceAction(action: AcceptanceAction | null): string {
  if (action === null) return "(no decision)";
  const label = acceptanceActionLabel(action.kind);
  if (!actionUsesReason(action.kind)) return label;
  return action.reason.length > 0 ? `${label} · "${action.reason}"` : `${label} · (no reason)`;
}
