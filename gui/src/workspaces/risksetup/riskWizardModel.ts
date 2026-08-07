/**
 * The Risk guided-setup model — the RISK-MANAGEMENT slice of the guided wizard, layered
 * on the shared guided-setup scaffolding in `workspaces/setupWizard/wizardModel.ts`.
 *
 * Where the Hedging wizard's final steps are the warehouse threshold + exit policy, the
 * Risk wizard's single final step is the incoming-lift ACCEPTANCE policy. The shared
 * engine ({@link applyWizardCore}) owns the books+routing spine and the
 * dependency-ordered, stop-on-first-failure Apply; this module supplies only the
 * acceptance step + its validation, and wraps it into {@link applyRiskWizard}. The
 * wizard commits, in strict dependency order:
 *
 *   1. createRiskBook  (per book) — mints the real slug ids routing references
 *   2. updateRiskRoutingGraph      — the routing graph, re-pointed at those real ids
 *   3. updateAcceptanceGraph       — the compiled accept/reject/hold decision graph
 *
 * Acceptance gates AT ACCEPTANCE — after last-look, before booking — on each incoming
 * lift: Accept books it, Reject declines with a reason, Hold routes it to the desk inbox.
 */
import type { AcceptanceGraph } from "../../data/contract";
import { defaultAcceptanceAction } from "../../lib/acceptanceAction";
import {
  compileRulesToAcceptanceGraph,
  detectAcceptanceRuleConflicts,
  newAcceptanceRuleId,
  newDefaultAcceptanceRule,
  type AcceptanceRule,
} from "../../lib/acceptanceRules";
import { validateAcceptanceGraph } from "../../lib/acceptanceTrace";
import type { RiskRule } from "../../lib/riskRules";
import {
  applyWizardCore,
  type ApplyResult,
  type ApplyStepState,
  type ExtraApplyStep,
  type WizardApplyCoreTransport,
  type WizardBook,
} from "../setupWizard/wizardModel";

/** The default edge floor (bps) the seeded reject rule uses — a sensible starting gate. */
export const DEFAULT_EDGE_FLOOR_BPS = 0.5;

/** The whole staged configuration the Risk wizard will apply. */
export interface RiskWizardDraft {
  /** Step 1 — the portfolios to create, in creation (parents-before-children) order. */
  books: WizardBook[];
  /** Step 2 — the ordered routing rules; each `bookId` is a {@link WizardBook.key}. */
  routingRules: RiskRule[];
  /** Step 3 — the ordered acceptance rules (first-match; last is the catch-all default). */
  acceptanceRules: AcceptanceRule[];
}

/** The exact (minimal) transport surface {@link applyRiskWizard} needs — the existing RPCs. */
export interface RiskWizardApplyTransport extends WizardApplyCoreTransport {
  updateAcceptanceGraph(graph: AcceptanceGraph): Promise<AcceptanceGraph>;
}

/** Which capability-gated parts of the Risk wizard the caller is entitled to apply. */
export interface RiskWizardEntitlements {
  /** `risk_manage·FI` — books + routing. */
  risk: boolean;
  /** `manage_acceptance·FI` — the acceptance policy. */
  acceptance: boolean;
}

/**
 * The sensible starter acceptance policy the wizard pre-fills: REJECT a lift whose
 * dealer edge is below a floor (thin/toxic pricing), else ACCEPT everything. A trader
 * who deletes the specific rule is left with the safe accept-all catch-all — exactly the
 * "default to accept-all if they skip" behaviour.
 */
export function defaultAcceptanceRules(): AcceptanceRule[] {
  return [
    {
      id: newAcceptanceRuleId(),
      conditions: [{ field: "edge_bps", op: "lt", value: { kind: "num", num: DEFAULT_EDGE_FLOOR_BPS } }],
      action: { kind: "reject", reason: "below edge floor" },
      enabled: true,
    },
    newDefaultAcceptanceRule(),
  ];
}

/** A fresh accept-all catch-all — the safe default when no policy is composed. */
export { newDefaultAcceptanceRule } from "../../lib/acceptanceRules";

/**
 * Blocking errors for the acceptance step — the structural rule-table conflicts (from
 * {@link detectAcceptanceRuleConflicts}) plus any compiled-graph validity issues. The
 * SAME checks the standalone Acceptance workspace blocks Save on, so the wizard never
 * ships a policy the workspace would reject.
 */
export function acceptanceErrors(rules: readonly AcceptanceRule[]): string[] {
  const errs: string[] = [];
  for (const c of detectAcceptanceRuleConflicts(rules)) {
    if (c.severity === "error") errs.push(c.message);
  }
  const graph = compileRulesToAcceptanceGraph(rules.filter((r) => r.enabled));
  for (const g of validateAcceptanceGraph(graph)) errs.push(g.message);
  return errs;
}

/** Fallback default from {@link defaultAcceptanceAction} for a re-picked decision. */
export { defaultAcceptanceAction };

/**
 * Commit the staged Risk wizard through the existing RPCs in dependency order,
 * delegating the books+routing spine to {@link applyWizardCore} and supplying the single
 * acceptance final step: books → routing → acceptance, stopping on the first failure and
 * never half-applying silently. The acceptance graph is compiled from the ENABLED rules
 * (an accept-all catch-all when the trader composed nothing specific).
 */
export async function applyRiskWizard(
  tx: RiskWizardApplyTransport,
  draft: RiskWizardDraft,
  entitle: RiskWizardEntitlements,
  onProgress: (steps: ApplyStepState[]) => void,
): Promise<ApplyResult> {
  const extra: ExtraApplyStep[] = [
    {
      id: "acceptance",
      label: "Save acceptance policy",
      // Always persist a policy when entitled — the compiled graph is at minimum the
      // safe accept-all catch-all, so "skipping" acceptance still writes accept-all.
      willRun: entitle.acceptance && draft.acceptanceRules.some((r) => r.enabled),
      entitled: entitle.acceptance,
      run: async () => {
        const enabled = draft.acceptanceRules.filter((r) => r.enabled);
        await tx.updateAcceptanceGraph(compileRulesToAcceptanceGraph(enabled));
        return `saved ${enabled.length} rule${enabled.length === 1 ? "" : "s"}`;
      },
    },
  ];

  return applyWizardCore(tx, draft.books, draft.routingRules, entitle.risk, extra, onProgress);
}
