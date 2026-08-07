/**
 * The Hedging guided-setup model — the HEDGE-SPECIFIC slice of the wizard, layered on
 * the shared guided-setup scaffolding in `workspaces/setupWizard/wizardModel.ts`.
 *
 * The shared engine ({@link applyWizardCore}) owns the books+routing spine and the
 * dependency-ordered, stop-on-first-failure Apply semantics; this module supplies only
 * the two hedge-specific final steps — the warehouse THRESHOLD and the exit-POLICY
 * graph — and wraps them into {@link applyWizard} with the wizard's exact legacy
 * signature. The wizard commits, in strict dependency order:
 *
 *   1. createRiskBook  (per book) — mints the real slug ids the later graphs reference
 *   2. updateRiskRoutingGraph      — the routing graph, re-pointed at those real ids
 *   3. updateHedgeThreshold        — the warehouse "100" (scope re-pointed if book-scoped)
 *   4. updateHedgePolicyGraph      — the exit-policy graph
 *
 * The portfolio + routing model ({@link WizardBook}, {@link portfolioErrors},
 * {@link routingErrors}, {@link enabledBookKeys}, {@link newBookKey}, {@link suggestSlug})
 * is re-exported from the shared module so the step surfaces + tests keep importing it
 * from here unchanged.
 */
import type { HedgeGraph, WarehouseThreshold } from "../../../data/contract";
import { compileRulesToHedgeGraph, detectHedgeRuleConflicts, type HedgeRule } from "../../../lib/hedgeRules";
import type { RiskRule } from "../../../lib/riskRules";
import {
  applyWizardCore,
  type ApplyResult,
  type ApplyStepState,
  type ExtraApplyStep,
  type WizardApplyCoreTransport,
  type WizardBook,
} from "../../setupWizard/wizardModel";

// Re-export the shared portfolio/routing model so the step surfaces + tests keep
// importing these from the hedging wizardModel unchanged.
export {
  enabledBookKeys,
  newBookKey,
  portfolioErrors,
  routingErrors,
  suggestSlug,
} from "../../setupWizard/wizardModel";
export type {
  ApplyResult,
  ApplyStatus,
  ApplyStepState,
  WizardBook,
} from "../../setupWizard/wizardModel";

/** The whole staged configuration the Hedging wizard will apply. */
export interface WizardDraft {
  /** Step 1 — the portfolios to create, in creation (parents-before-children) order. */
  books: WizardBook[];
  /** Step 2 — the ordered routing rules; each `bookId` is a {@link WizardBook.key}. */
  routingRules: RiskRule[];
  /** Step 3 — whether to set a warehouse threshold at all. */
  includeThreshold: boolean;
  /** Step 3 — the warehouse threshold ("the 100"); `scopeId` may be a book key when book-scoped. */
  threshold: WarehouseThreshold;
  /** Step 3 — the ordered exit-policy rules (always at least the warehouse catch-all). */
  hedgeRules: HedgeRule[];
}

/** The exact (minimal) transport surface {@link applyWizard} needs — the existing RPCs. */
export interface WizardApplyTransport extends WizardApplyCoreTransport {
  updateHedgeThreshold(threshold: WarehouseThreshold): Promise<WarehouseThreshold[]>;
  updateHedgePolicyGraph(graph: HedgeGraph): Promise<HedgeGraph>;
}

/** Which capability-gated halves of the wizard the caller is entitled to apply. */
export interface WizardEntitlements {
  /** `risk_manage·FI` — books + routing. */
  risk: boolean;
  /** `hedge·FI` — threshold + policy. */
  hedge: boolean;
}

/**
 * The default warehouse threshold — mirrors the engine defaults the standalone
 * Thresholds editor pre-fills (amber 0.80 / red 0.90, target at the amber edge).
 */
export function defaultWizardThreshold(scopeId: string): WarehouseThreshold {
  return {
    scopeKind: "book",
    scopeId,
    metric: "dv01",
    cap: 100_000,
    amber: 0.8,
    red: 0.9,
    targetFraction: 0.8,
    minClip: 1_000,
    maxClip: 50_000,
    ramped: false,
    rampK: 0,
  };
}

/** Blocking errors for the step-3 threshold form (only when a threshold is included). */
export function thresholdErrors(t: WarehouseThreshold): string[] {
  const errs: string[] = [];
  if (!(t.cap > 0)) errs.push('The cap (the "100") must be greater than zero.');
  if (!(t.amber >= 0 && t.amber <= t.red && t.red <= 1)) {
    errs.push("Bands must satisfy 0 ≤ amber ≤ red ≤ 1.");
  }
  return errs;
}

/** Blocking errors for the step-3 exit policy (structural rule-table conflicts). */
export function hedgingErrors(rules: readonly HedgeRule[]): string[] {
  return detectHedgeRuleConflicts(rules)
    .filter((c) => c.severity === "error")
    .map((c) => c.message);
}

/**
 * Commit the staged Hedging wizard through the existing RPCs in dependency order,
 * delegating the books+routing spine to {@link applyWizardCore} and supplying the two
 * hedge-specific final steps (threshold, policy). Preserves the wizard's legacy
 * signature + behaviour exactly: books → routing → threshold → policy, stopping on the
 * first failure and never half-applying silently.
 */
export async function applyWizard(
  tx: WizardApplyTransport,
  draft: WizardDraft,
  entitle: WizardEntitlements,
  onProgress: (steps: ApplyStepState[]) => void,
): Promise<ApplyResult> {
  const extra: ExtraApplyStep[] = [
    {
      id: "threshold",
      label: "Set warehouse threshold",
      willRun: entitle.hedge && draft.includeThreshold && draft.threshold.cap > 0,
      entitled: entitle.hedge,
      run: async (idByKey) => {
        const scopeId =
          draft.threshold.scopeKind === "book" && idByKey.has(draft.threshold.scopeId)
            ? (idByKey.get(draft.threshold.scopeId) as string)
            : draft.threshold.scopeId;
        await tx.updateHedgeThreshold({ ...draft.threshold, scopeId });
        return `${draft.threshold.metric} cap ${draft.threshold.cap.toLocaleString()}`;
      },
    },
    {
      id: "policy",
      label: "Save hedge policy",
      willRun: entitle.hedge && draft.hedgeRules.some((r) => r.enabled),
      entitled: entitle.hedge,
      run: async () => {
        const enabled = draft.hedgeRules.filter((r) => r.enabled);
        await tx.updateHedgePolicyGraph(compileRulesToHedgeGraph(enabled));
        return `saved ${enabled.length} rule${enabled.length === 1 ? "" : "s"}`;
      },
    },
  ];

  return applyWizardCore(tx, draft.books, draft.routingRules, entitle.risk, extra, onProgress);
}
