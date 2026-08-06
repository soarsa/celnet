/**
 * The pure model + apply orchestration for the Hedging guided-setup wizard. Kept
 * free of any React/component import so the dependency-ordered save sequence and the
 * per-step validation stay unit-testable in isolation.
 *
 * The wizard stages three things a trader would otherwise configure across three
 * separate surfaces — risk portfolios (books), routing rules, and the
 * internalise/hedge policy — then, on Apply, commits them through the EXISTING
 * server RPCs in strict dependency order:
 *
 *   1. createRiskBook  (per book) — mints the real slug ids the later graphs reference
 *   2. updateRiskRoutingGraph      — the routing graph, re-pointed at those real ids
 *   3. updateHedgeThreshold        — the warehouse "100" (scope re-pointed if book-scoped)
 *   4. updateHedgePolicyGraph      — the exit-policy graph
 *
 * Books are referenced DURING editing by a stable local {@link WizardBook.key} (the
 * server has not minted the real id yet); {@link applyWizard} threads the create
 * responses' ids into the routing graph + threshold scope before persisting, so a
 * routing leaf never dangles at a not-yet-created / disabled portfolio.
 */
import type {
  HedgeGraph,
  RiskBook,
  RiskLimits,
  RiskRoutingGraph,
  WarehouseThreshold,
} from "../../../data/contract";
import {
  compileRulesToGraph,
  detectRuleConflicts,
  type RiskRule,
} from "../../../lib/riskRules";
import {
  compileRulesToHedgeGraph,
  detectHedgeRuleConflicts,
  type HedgeRule,
} from "../../../lib/hedgeRules";

/**
 * A risk portfolio staged in the wizard. Referenced by its stable local {@link key}
 * (routing rules + a book-scoped threshold point at THIS, not the not-yet-minted
 * server id); {@link parentKey} nests it under an earlier staged book.
 */
export interface WizardBook {
  /** Stable local id — the wizard's reference handle until the server mints the real id. */
  key: string;
  /** Human-friendly name (the server mints the slug id from it). */
  name: string;
  /** Parent by local key, or `null` for a top-level portfolio. */
  parentKey: string | null;
  /** Owning desk id, or `null` (unowned). */
  deskId: string | null;
  /** Only enabled portfolios are valid routing targets. */
  enabled: boolean;
  /** Optional per-portfolio pre-trade limits. */
  limits: RiskLimits | null;
}

/** The whole staged configuration the wizard will apply. */
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

/** The four dependency-ordered persistence steps of an Apply. */
export type ApplyStepId = "books" | "routing" | "threshold" | "policy";
export type ApplyStatus = "pending" | "running" | "done" | "error" | "skipped";

/** The live state of one persistence step, surfaced in the Apply progress list. */
export interface ApplyStepState {
  id: ApplyStepId;
  label: string;
  status: ApplyStatus;
  /** A short human detail (what succeeded / why it was skipped / the failure reason). */
  detail: string;
}

/** The outcome of an {@link applyWizard} run. */
export interface ApplyResult {
  ok: boolean;
  steps: ApplyStepState[];
  /** The step that failed (stops the sequence), or `null` on success. */
  failedStep: ApplyStepId | null;
  /** The real server ids of the portfolios that were created before any failure. */
  createdBookIds: string[];
}

/** The exact (minimal) transport surface {@link applyWizard} needs — the existing RPCs. */
export interface WizardApplyTransport {
  createRiskBook(spec: RiskBook): Promise<RiskBook>;
  updateRiskRoutingGraph(graph: RiskRoutingGraph): Promise<RiskRoutingGraph>;
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

let BOOK_SEQ = 0;
/** Mint a fresh, unique local book key (a new step-1 row). */
export function newBookKey(): string {
  BOOK_SEQ += 1;
  return `wb-${Date.now().toString(36)}-${BOOK_SEQ}`;
}

/** A conservative slug suggestion from a name (the server still mints the authoritative id). */
export function suggestSlug(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
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

/** The set of local keys of the ENABLED staged portfolios (the only valid routing targets). */
export function enabledBookKeys(books: readonly WizardBook[]): Set<string> {
  return new Set(books.filter((b) => b.enabled && b.name.trim().length > 0).map((b) => b.key));
}

/** Blocking errors for step 1 — at least one enabled, named portfolio is required. */
export function portfolioErrors(books: readonly WizardBook[]): string[] {
  const errs: string[] = [];
  if (books.length === 0) {
    errs.push("Add at least one risk portfolio — routing needs somewhere to book risk.");
    return errs;
  }
  if (books.some((b) => b.name.trim().length === 0)) {
    errs.push("Every portfolio needs a name.");
  }
  if (enabledBookKeys(books).size === 0) {
    errs.push("Enable at least one portfolio — only enabled portfolios are valid routing targets.");
  }
  return errs;
}

/**
 * Blocking errors for step 2 — the logical rule-table conflicts PLUS the explicit
 * guard the whole wizard exists to prevent: every enabled rule (including the
 * default) must target an ENABLED step-1 portfolio, never a disabled/unknown one.
 */
export function routingErrors(rules: readonly RiskRule[], enabledKeys: ReadonlySet<string>): string[] {
  const errs: string[] = [];
  for (const c of detectRuleConflicts(rules, enabledKeys)) {
    if (c.severity === "error") errs.push(c.message);
  }
  for (let i = 0; i < rules.length; i += 1) {
    const r = rules[i] as RiskRule;
    if (!r.enabled) continue;
    if (r.bookId === null || !enabledKeys.has(r.bookId)) {
      errs.push(
        `Rule ${i + 1} routes to a portfolio that is not one of your enabled portfolios — pick an enabled step-1 portfolio.`,
      );
    }
  }
  return errs;
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

/** Narrow an unknown thrown value to a display string. */
function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : "the server rejected the change";
}

/** Build the create spec for one staged book, resolving its parent to the real minted id. */
function bookSpec(b: WizardBook, idByKey: ReadonlyMap<string, string>): RiskBook {
  return {
    id: suggestSlug(b.name),
    name: b.name.trim(),
    parentId: b.parentKey !== null ? (idByKey.get(b.parentKey) ?? null) : null,
    deskId: b.deskId,
    description: "",
    limits: b.limits,
    enabled: b.enabled,
  };
}

/** Compose one persistence step's initial state: pending if it will run, else skipped-with-reason. */
function initStep(id: ApplyStepId, label: string, willRun: boolean, entitled: boolean): ApplyStepState {
  if (willRun) return { id, label, status: "pending", detail: "" };
  return {
    id,
    label,
    status: "skipped",
    detail: entitled ? "nothing to apply" : "you lack the required capability",
  };
}

/**
 * Commit the staged wizard through the existing RPCs in dependency order. Books are
 * created FIRST so their real ids exist; the routing graph + a book-scoped threshold
 * are then re-pointed at those ids before they are persisted. On the first failure the
 * sequence STOPS — it never half-applies silently: the returned {@link ApplyResult}
 * reports which step failed and (via the step details) exactly what already succeeded.
 * `onProgress` is invoked after every state transition for live UI.
 */
export async function applyWizard(
  tx: WizardApplyTransport,
  draft: WizardDraft,
  entitle: WizardEntitlements,
  onProgress: (steps: ApplyStepState[]) => void,
): Promise<ApplyResult> {
  const runBooks = entitle.risk && draft.books.length > 0;
  const runRouting = entitle.risk && draft.routingRules.some((r) => r.enabled);
  const runThreshold = entitle.hedge && draft.includeThreshold && draft.threshold.cap > 0;
  const runPolicy = entitle.hedge && draft.hedgeRules.some((r) => r.enabled);

  const steps: ApplyStepState[] = [
    initStep("books", "Create risk portfolios", runBooks, entitle.risk),
    initStep("routing", "Save routing rules", runRouting, entitle.risk),
    initStep("threshold", "Set warehouse threshold", runThreshold, entitle.hedge),
    initStep("policy", "Save hedge policy", runPolicy, entitle.hedge),
  ];
  const snapshot = (): ApplyStepState[] => steps.map((s) => ({ ...s }));
  const emit = (): void => onProgress(snapshot());
  const set = (id: ApplyStepId, status: ApplyStatus, detail?: string): void => {
    const s = steps.find((x) => x.id === id);
    if (!s) return;
    s.status = status;
    if (detail !== undefined) s.detail = detail;
    emit();
  };
  const fail = (id: ApplyStepId, reason: string): ApplyResult => {
    set(id, "error", reason);
    return { ok: false, steps: snapshot(), failedStep: id, createdBookIds };
  };

  emit();

  const idByKey = new Map<string, string>();
  const createdBookIds: string[] = [];

  // 1 — create the portfolios (mints the ids the later graphs reference).
  if (runBooks) {
    set("books", "running");
    for (const b of draft.books) {
      try {
        const saved = await tx.createRiskBook(bookSpec(b, idByKey));
        idByKey.set(b.key, saved.id);
        createdBookIds.push(saved.id);
      } catch (e: unknown) {
        return fail(
          "books",
          `created ${createdBookIds.length} of ${draft.books.length}; “${b.name.trim() || b.key}” failed: ${messageOf(e)}`,
        );
      }
    }
    set("books", "done", `created ${createdBookIds.length} portfolio${createdBookIds.length === 1 ? "" : "s"}`);
  }

  // 2 — persist routing, re-pointed at the real ids.
  if (runRouting) {
    set("routing", "running");
    try {
      const remapped = draft.routingRules
        .filter((r) => r.enabled)
        .map((r) => ({
          ...r,
          bookId: r.bookId !== null ? (idByKey.get(r.bookId) ?? r.bookId) : null,
        }));
      await tx.updateRiskRoutingGraph(compileRulesToGraph(remapped));
      set("routing", "done", `saved ${remapped.length} rule${remapped.length === 1 ? "" : "s"}`);
    } catch (e: unknown) {
      return fail("routing", messageOf(e));
    }
  }

  // 3 — persist the warehouse threshold, re-pointing a book scope at the real id.
  if (runThreshold) {
    set("threshold", "running");
    try {
      const scopeId =
        draft.threshold.scopeKind === "book" && idByKey.has(draft.threshold.scopeId)
          ? (idByKey.get(draft.threshold.scopeId) as string)
          : draft.threshold.scopeId;
      await tx.updateHedgeThreshold({ ...draft.threshold, scopeId });
      set("threshold", "done", `${draft.threshold.metric} cap ${draft.threshold.cap.toLocaleString()}`);
    } catch (e: unknown) {
      return fail("threshold", messageOf(e));
    }
  }

  // 4 — persist the exit policy.
  if (runPolicy) {
    set("policy", "running");
    try {
      const enabled = draft.hedgeRules.filter((r) => r.enabled);
      await tx.updateHedgePolicyGraph(compileRulesToHedgeGraph(enabled));
      set("policy", "done", `saved ${enabled.length} rule${enabled.length === 1 ? "" : "s"}`);
    } catch (e: unknown) {
      return fail("policy", messageOf(e));
    }
  }

  return { ok: true, steps: snapshot(), failedStep: null, createdBookIds };
}
