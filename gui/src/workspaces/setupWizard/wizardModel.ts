/**
 * The SHARED, framework-free model for the guided-setup wizards. Two wizards ride on
 * this scaffolding — the **Hedging** guided setup and the **Risk** guided setup — and
 * they differ ONLY in their final configuration step(s). Everything a guided setup has
 * in common lives here:
 *
 *   • the staged risk-portfolio model ({@link WizardBook}) + its step-1 validation
 *     ({@link portfolioErrors});
 *   • the routing step's validation ({@link routingErrors}) — the enabled-target guard
 *     the whole wizard exists to prevent;
 *   • the dependency-ordered Apply engine ({@link applyWizardCore}) that ALWAYS creates
 *     the risk books FIRST (minting the real ids the later graphs reference), THEN
 *     persists routing re-pointed at those ids, THEN runs each wizard-specific
 *     {@link ExtraApplyStep} in order — stopping on the first failure, never silently
 *     half-applying, and reporting exactly which step failed.
 *
 * Kept free of any React import so the sequencing + validation stay unit-testable in
 * isolation. The two wizards' own `wizardModel`/`riskWizardModel` build their final
 * steps as {@link ExtraApplyStep}s and delegate the books+routing spine to
 * {@link applyWizardCore}, so that spine is written — and tested — exactly once.
 */
import type { RiskBook, RiskLimits, RiskRoutingGraph } from "../../data/contract";
import { compileRulesToGraph, detectRuleConflicts, type RiskRule } from "../../lib/riskRules";

/**
 * A risk portfolio staged in the wizard. Referenced by its stable local {@link key}
 * (routing rules + a book-scoped config point at THIS, not the not-yet-minted server
 * id); {@link parentKey} nests it under an earlier staged book.
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

/** The generic status of one Apply step, surfaced in the progress list. */
export type ApplyStatus = "pending" | "running" | "done" | "error" | "skipped";

/** The live state of one persistence step, surfaced in the Apply progress list. */
export interface ApplyStepState {
  /** The step id — `books` / `routing` for the shared spine, else the wizard-specific id. */
  id: string;
  label: string;
  status: ApplyStatus;
  /** A short human detail (what succeeded / why it was skipped / the failure reason). */
  detail: string;
}

/** The outcome of an {@link applyWizardCore} run. */
export interface ApplyResult {
  ok: boolean;
  steps: ApplyStepState[];
  /** The id of the step that failed (stops the sequence), or `null` on success. */
  failedStep: string | null;
  /** The real server ids of the portfolios that were created before any failure. */
  createdBookIds: string[];
}

/** The books + routing transport surface the shared spine needs (the existing RPCs). */
export interface WizardApplyCoreTransport {
  createRiskBook(spec: RiskBook): Promise<RiskBook>;
  updateRiskRoutingGraph(graph: RiskRoutingGraph): Promise<RiskRoutingGraph>;
}

/**
 * One wizard-specific persistence step that runs AFTER the shared books+routing spine.
 * The Hedging wizard supplies two ({@link ExtraApplyStep}: threshold, policy); the Risk
 * wizard supplies one (acceptance). Each declares whether it will run + whether the
 * caller is entitled, and a `run` that receives the `key → real-id` map so a
 * book-scoped config can be re-pointed at the just-minted id.
 */
export interface ExtraApplyStep {
  /** The stable step id (distinct from `books`/`routing`). */
  id: string;
  /** The progress-row label. */
  label: string;
  /** Whether this step has anything to persist (else it renders skipped-with-reason). */
  willRun: boolean;
  /** Whether the caller holds the capability this step needs. */
  entitled: boolean;
  /** Persist the step; return a short success detail, or throw to fail the sequence. */
  run: (idByKey: ReadonlyMap<string, string>) => Promise<string>;
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
 * guard the whole wizard exists to prevent: every enabled rule (including the default)
 * must target an ENABLED step-1 portfolio, never a disabled/unknown one.
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

/** Narrow an unknown thrown value to a display string. */
export function messageOf(e: unknown): string {
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
function initStep(id: string, label: string, willRun: boolean, entitled: boolean): ApplyStepState {
  if (willRun) return { id, label, status: "pending", detail: "" };
  return {
    id,
    label,
    status: "skipped",
    detail: entitled ? "nothing to apply" : "you lack the required capability",
  };
}

/**
 * The SHARED, dependency-ordered Apply engine. Runs the books+routing spine common to
 * every guided setup, then each wizard-specific {@link ExtraApplyStep} in order. Books
 * are created FIRST so their real ids exist; the routing graph is then re-pointed at
 * those ids before it is persisted; each extra step receives the `key → real-id` map so
 * a book-scoped config re-points too. On the FIRST failure the sequence STOPS — it never
 * half-applies silently: the returned {@link ApplyResult} reports which step failed and
 * (via the step details) exactly what already succeeded. `onProgress` is invoked after
 * every state transition for live UI.
 */
export async function applyWizardCore(
  tx: WizardApplyCoreTransport,
  books: readonly WizardBook[],
  routingRules: readonly RiskRule[],
  risk: boolean,
  extra: readonly ExtraApplyStep[],
  onProgress: (steps: ApplyStepState[]) => void,
): Promise<ApplyResult> {
  const runBooks = risk && books.length > 0;
  const runRouting = risk && routingRules.some((r) => r.enabled);

  const steps: ApplyStepState[] = [
    initStep("books", "Create risk portfolios", runBooks, risk),
    initStep("routing", "Save routing rules", runRouting, risk),
    ...extra.map((s) => initStep(s.id, s.label, s.willRun, s.entitled)),
  ];
  const snapshot = (): ApplyStepState[] => steps.map((s) => ({ ...s }));
  const emit = (): void => onProgress(snapshot());
  const set = (id: string, status: ApplyStatus, detail?: string): void => {
    const s = steps.find((x) => x.id === id);
    if (!s) return;
    s.status = status;
    if (detail !== undefined) s.detail = detail;
    emit();
  };
  const idByKey = new Map<string, string>();
  const createdBookIds: string[] = [];
  const fail = (id: string, reason: string): ApplyResult => {
    set(id, "error", reason);
    return { ok: false, steps: snapshot(), failedStep: id, createdBookIds };
  };

  emit();

  // 1 — create the portfolios (mints the ids the later graphs reference).
  if (runBooks) {
    set("books", "running");
    for (const b of books) {
      try {
        const saved = await tx.createRiskBook(bookSpec(b, idByKey));
        idByKey.set(b.key, saved.id);
        createdBookIds.push(saved.id);
      } catch (e: unknown) {
        return fail(
          "books",
          `created ${createdBookIds.length} of ${books.length}; “${b.name.trim() || b.key}” failed: ${messageOf(e)}`,
        );
      }
    }
    set(
      "books",
      "done",
      `created ${createdBookIds.length} portfolio${createdBookIds.length === 1 ? "" : "s"}`,
    );
  }

  // 2 — persist routing, re-pointed at the real ids.
  if (runRouting) {
    set("routing", "running");
    try {
      const remapped = routingRules
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

  // 3 — each wizard-specific step, in order (threshold+policy for hedging; acceptance
  //     for risk). Re-points a book-scoped config at the real id via `idByKey`.
  for (const step of extra) {
    if (!step.willRun) continue;
    set(step.id, "running");
    try {
      const detail = await step.run(idByKey);
      set(step.id, "done", detail);
    } catch (e: unknown) {
      return fail(step.id, messageOf(e));
    }
  }

  return { ok: true, steps: snapshot(), failedStep: null, createdBookIds };
}
