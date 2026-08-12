/**
 * The firm's RISK MODEL resolution — which posture actually governs a given risk
 * portfolio, and why.
 *
 * A {@link HedgingModel} is bound per scope ({@link HedgingModelBinding}) and resolved
 * **most-specific-wins: instrument > book > desk**, the same precedence the hedging LP
 * panels and exit modes use. That precedence is deliberately invisible in any single
 * control, which is precisely why it needs to be shown: a trader looking at a portfolio
 * cannot otherwise tell whether its posture is its own, inherited from the desk, or
 * overridden at the instrument.
 *
 * This module is the single source of that answer for the client. It mirrors the
 * server's rules exactly — including the two that are easy to get wrong:
 *
 *  - An UNBOUND scope is `CUSTOM` (0), meaning "the desk's own authored exit-policy
 *    graph governs". That is the historical behaviour, not "no hedging".
 *  - A NON-POSITIVE `dv01Budget` means **inherit the configured warehouse threshold**,
 *    never a cap of zero. A zero cap would mean "warehouse nothing", which is the
 *    opposite of what a blank field means.
 */

import type { HedgeScopeKind, HedgingModel, HedgingModelBinding } from "../data/contract";

/** Trader-facing label for each risk model (the selector option text). */
export const HEDGING_MODEL_LABEL: Record<HedgingModel, string> = {
  0: "Custom — the desk's authored exit policy",
  1: "Back-to-back — hedge every fill, warehouse nothing",
  2: "Internalise to a DV01 budget",
};

/** A one-line explanation of each model (helper text under the selector). */
export const HEDGING_MODEL_HINT: Record<HedgingModel, string> = {
  0: "The exit-policy graph authored for this scope governs, unchanged. This is the default and the escape hatch — binding no model means this.",
  1: "Every fill is hedged straight out on the street; nothing is warehoused. Spread capture only, with no directional carry between the client trade and the hedge.",
  2: "Client flow is warehoused against a DV01 budget so opposing flow can net off internally; only the overflow above the band edge is shed to the street.",
};

/** The models in wire (enum) order — the selector's option order. */
export const HEDGING_MODELS: readonly HedgingModel[] = [0, 1, 2];

/** Whether a model reads {@link HedgingModelBinding.dv01Budget} at all. */
export function modelUsesBudget(model: HedgingModel): boolean {
  return model === 2;
}

/** The scope identifiers a portfolio resolves against, least → most specific. */
export interface RiskModelScopes {
  /** The portfolio's owning desk id, or `null` when unowned. */
  readonly deskId: string | null;
  /** The risk portfolio's own id. */
  readonly bookId: string;
  /** An instrument id, when resolving for one specific instrument. */
  readonly instrumentId?: string | null;
}

/** One step of the resolution, in precedence order — the trace a trader reads. */
export interface RiskModelStep {
  readonly scopeKind: HedgeScopeKind;
  readonly scopeId: string;
  /** The binding found at this scope, or `null` when this scope binds nothing. */
  readonly binding: HedgingModelBinding | null;
  /** Whether this step is the one that WON (the most specific bound scope). */
  readonly effective: boolean;
}

/** The resolved posture for a portfolio, plus the trace that explains it. */
export interface ResolvedRiskModel {
  readonly model: HedgingModel;
  /**
   * The DV01 budget this binding actually imposes, or `null` when it imposes none —
   * a non-budget model, or a blank/non-positive budget meaning "inherit the configured
   * warehouse threshold". `null` is NOT a cap of zero.
   */
  readonly budget: number | null;
  /** Which scope supplied the model, or `null` when nothing is bound (⇒ `CUSTOM`). */
  readonly source: RiskModelStep | null;
  /** Every scope considered, least → most specific. */
  readonly trace: readonly RiskModelStep[];
}

/**
 * Resolve the risk model governing `scopes`, most-specific-wins.
 *
 * Matching is case-insensitive on the scope id, mirroring the server's own
 * `eq_ignore_ascii_case` lookups so the client never disagrees with the engine about
 * which binding applies.
 */
export function resolveRiskModel(
  bindings: readonly HedgingModelBinding[],
  scopes: RiskModelScopes,
): ResolvedRiskModel {
  // Least → most specific. A desk-less portfolio simply has no desk step.
  const candidates: { kind: HedgeScopeKind; id: string }[] = [];
  if (scopes.deskId !== null && scopes.deskId !== "") {
    candidates.push({ kind: "desk", id: scopes.deskId });
  }
  candidates.push({ kind: "book", id: scopes.bookId });
  if (
    scopes.instrumentId !== undefined &&
    scopes.instrumentId !== null &&
    scopes.instrumentId !== ""
  ) {
    candidates.push({ kind: "instrument", id: scopes.instrumentId });
  }

  const found = candidates.map((c) => ({
    scopeKind: c.kind,
    scopeId: c.id,
    binding:
      bindings.find(
        (b) => b.scopeKind === c.kind && b.scopeId.toLowerCase() === c.id.toLowerCase(),
      ) ?? null,
  }));

  // The LAST bound step wins — the list is ordered least → most specific.
  let winnerIndex = -1;
  for (let i = 0; i < found.length; i += 1) {
    if (found[i]!.binding !== null) winnerIndex = i;
  }

  const trace: RiskModelStep[] = found.map((f, i) => ({
    ...f,
    effective: i === winnerIndex,
  }));

  if (winnerIndex === -1) {
    // Nothing bound anywhere ⇒ CUSTOM, the authored graph, with no imposed budget.
    return { model: 0, budget: null, source: null, trace };
  }

  const winner = trace[winnerIndex]!;
  const binding = winner.binding!;
  const budget =
    modelUsesBudget(binding.model) && Number.isFinite(binding.dv01Budget) && binding.dv01Budget > 0
      ? binding.dv01Budget
      : null;
  return { model: binding.model, budget, source: winner, trace };
}

/**
 * A one-sentence plain-English account of why this portfolio has the posture it has —
 * rendered under the picker so the precedence is never something a trader has to infer.
 */
export function explainRiskModel(r: ResolvedRiskModel, bookId: string): string {
  if (r.source === null) {
    return "Nothing is bound at this portfolio, its desk, or any instrument — so the exit policy authored for this scope governs (Custom).";
  }
  const where =
    r.source.scopeKind === "book" && r.source.scopeId.toLowerCase() === bookId.toLowerCase()
      ? "set on this portfolio"
      : `inherited from ${r.source.scopeKind} “${r.source.scopeId}”`;
  const budget =
    r.budget === null
      ? modelUsesBudget(r.model)
        ? " Its budget is blank, so the scope's configured warehouse threshold applies."
        : ""
      : ` It warehouses up to a DV01 of ${r.budget.toLocaleString("en-US")} before shedding.`;
  return `${HEDGING_MODEL_LABEL[r.model]} — ${where}.${budget}`;
}
