/**
 * RiskBooksWorkspace — the admin hierarchical RISK-PORTFOLIO tree editor
 * (docs/FI-RISK-ROUTING-REQUIREMENTS.md §6.2). USER-FACING name "Risk Portfolios";
 * the wire type stays {@link RiskBook}/`RiskBookDef` (the rename is UI-only — see
 * docs/FI-BOOK-CONCEPTS.md). Risk portfolios form a TREE (desk → portfolio →
 * sub-portfolio); a filled order/RFQ routes its risk into a leaf portfolio so
 * limits / greeks / PnL are managed per portfolio. This pane lets an admin list /
 * create / rename / nest / enable-disable portfolios, set per-portfolio
 * {@link RiskLimits}, tag an owning desk, and re-parent (a client-side acyclic
 * guard forbids parenting a portfolio under itself or a descendant).
 *
 * Each portfolio declares the ASSET CLASS whose risk it holds ({@link RiskBook.assetClass}):
 * vega and DV01 are not commensurable, so a tree holds ONE franchise and a sub-portfolio
 * inherits its parent's (server-validated). That tag is what lets this be a single
 * firm-wide surface rather than one screen per asset.
 *
 * Gating: the risk-portfolio RPCs gate on the granular `risk_manage` capability
 * server-side (docs/PERMISSIONS-GRANULAR-REVIEW.md §4 — a firm risk-control authority
 * distinct from super-admin, so a desk/risk lead manages portfolios WITHOUT full
 * Administer). Edit affordances mirror it in TWO tiers: reaching the pane needs
 * `risk_manage` on EITHER class (`readOnly`), while editing a particular portfolio needs
 * it on THAT portfolio's class (`draftReadOnly`) — so a single-franchise risk manager
 * sees the whole firm tree but can only edit their own side. The firm-wide routing GRAPH
 * that maps fills to leaf books is edited on the flow-canvas surface, not here.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { MAGNITUDE_HELP, MagnitudeField } from "../components/MagnitudeField";
import type {
  CapabilityAsset,
  DeskDesc,
  HedgeConfig,
  HedgingModel,
  RiskBook,
  RiskLimits,
} from "../data/contract";
import {
  explainRiskModel,
  HEDGING_MODEL_HINT,
  HEDGING_MODEL_LABEL,
  HEDGING_MODELS,
  modelUsesBudget,
  resolveRiskModel,
} from "../lib/riskModel";
import { notifyRiskRoutingChanged } from "../lib/routingGuard";
import styles from "./RiskBooksWorkspace.module.css";

/**
 * A fresh blank book draft for the Create flow (id blank ⇒ server mints from name).
 * `assetClass` is explicit rather than defaulted: a sub-book MUST hold its parent's
 * franchise (the server rejects a tree that changes class mid-branch), and a top-level
 * book takes the franchise the trader is currently looking at.
 */
function blankBook(parentId: string | null, assetClass: CapabilityAsset): RiskBook {
  return {
    id: "",
    name: "",
    parentId,
    deskId: null,
    description: "",
    limits: null,
    enabled: true,
    assetClass,
  };
}

/** Deep-clone a book draft so the editor never aliases the loaded roster object. */
function cloneBook(b: RiskBook): RiskBook {
  return { ...b, limits: b.limits === null ? null : { ...b.limits } };
}

/** The result of the last Save/Delete attempt (inline feedback). */
type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

/** A row in the flattened, depth-annotated render order of the book tree. */
interface TreeRow {
  book: RiskBook;
  depth: number;
}

/** Flatten the book forest into render order (roots first, children under parents). */
function flattenTree(books: readonly RiskBook[]): TreeRow[] {
  const childrenOf = new Map<string | null, RiskBook[]>();
  for (const b of books) {
    const key = b.parentId;
    const bucket = childrenOf.get(key);
    if (bucket) bucket.push(b);
    else childrenOf.set(key, [b]);
  }
  // Any book whose parent id is not itself a known book is treated as a root.
  const ids = new Set(books.map((b) => b.id));
  const roots = books.filter((b) => b.parentId === null || !ids.has(b.parentId));
  const out: TreeRow[] = [];
  const visit = (book: RiskBook, depth: number): void => {
    out.push({ book, depth });
    for (const child of childrenOf.get(book.id) ?? []) visit(child, depth + 1);
  };
  for (const r of roots) visit(r, 0);
  return out;
}

/** The set of book ids at or below `id` (self + descendants) — excluded as re-parent targets. */
function subtreeIds(books: readonly RiskBook[], id: string): Set<string> {
  const childrenOf = new Map<string, RiskBook[]>();
  for (const b of books) {
    if (b.parentId === null) continue;
    const bucket = childrenOf.get(b.parentId);
    if (bucket) bucket.push(b);
    else childrenOf.set(b.parentId, [b]);
  }
  const out = new Set<string>([id]);
  const walk = (parent: string): void => {
    for (const c of childrenOf.get(parent) ?? []) {
      if (!out.has(c.id)) {
        out.add(c.id);
        walk(c.id);
      }
    }
  };
  walk(id);
  return out;
}

const compact = (n: number): string =>
  new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(n);

export function RiskBooksWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  const isAdmin = auth.isAdmin;
  // Risk-portfolio editing is gated on the granular risk_manage·FI capability (not
  // super-admin) — see the header note. The owning-desk picker below still needs the
  // Administer-gated desk roster, so it stays isAdmin-fetched (optional metadata).
  // Portfolios are now per-franchise, so reaching the surface needs `risk_manage` on
  // EITHER class — gating the whole screen on FI would lock an FX risk manager out of
  // their own books. Editing a PARTICULAR book is gated on that book's own class below,
  // so a single-franchise manager sees the whole tree but can only edit their side.
  const canManageAnyRisk =
    auth.can("risk_manage", "fixed_income") || auth.can("risk_manage", "fx_options");
  const canManageClass = (cls: CapabilityAsset): boolean => auth.can("risk_manage", cls);
  const readOnly = !canManageAnyRisk;

  const [books, setBooks] = useState<RiskBook[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<RiskBook | null>(null);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  // The firm hedge config carries the per-scope RISK-MODEL bindings. It is loaded here
  // (not just on the Hedging surface) because the model is a property of how a PORTFOLIO
  // runs, and a trader editing the portfolio must see and set it in one place.
  const [hedgeConfig, setHedgeConfig] = useState<HedgeConfig | null>(null);
  const [modelState, setModelState] = useState<SaveState>({ kind: "idle" });
  // The EDITOR is gated on the open book's own franchise: a manager entitled to one
  // class can see the whole firm-wide tree but may only edit their side of it.
  const draftReadOnly = readOnly || (draft !== null && !canManageClass(draft.assetClass));

  // The posture actually governing the open portfolio, resolved most-specific-wins
  // (instrument > book > desk) so the trader sees the EFFECTIVE model, not just whatever
  // happens to be bound at this one scope.
  const resolvedModel = useMemo(
    () =>
      draft === null || hedgeConfig === null
        ? null
        : resolveRiskModel(hedgeConfig.hedgingModels, {
            deskId: draft.deskId,
            bookId: draft.id,
          }),
    [draft, hedgeConfig],
  );

  /**
   * Bind (or re-bind) this PORTFOLIO's risk model. Writes a `book`-scoped binding and
   * round-trips the whole hedge config through the existing set_hedge_config RPC — there
   * is deliberately no separate CRUD verb for bindings.
   */
  const applyModel = useCallback(
    async (model: HedgingModel, dv01Budget: number): Promise<void> => {
      if (hedgeConfig === null || draft === null || draft.id === "") return;
      setModelState({ kind: "saving" });
      const others = hedgeConfig.hedgingModels.filter(
        (b) => !(b.scopeKind === "book" && b.scopeId.toLowerCase() === draft.id.toLowerCase()),
      );
      const next: HedgeConfig = {
        ...hedgeConfig,
        hedgingModels: [
          ...others,
          { scopeKind: "book", scopeId: draft.id, model, dv01Budget },
        ],
      };
      try {
        await app.transport.setHedgeConfig(next);
        setHedgeConfig(next);
        setModelState({ kind: "ok", message: "Risk model saved." });
      } catch (e: unknown) {
        setModelState({
          kind: "error",
          message: e instanceof Error ? e.message : "the server rejected the risk model",
        });
      }
    },
    [app.transport, draft, hedgeConfig],
  );

  const reload = useCallback(async (): Promise<RiskBook[]> => {
    const list = await app.transport.listRiskBooks();
    setBooks(list);
    setLoadError(null);
    return list;
  }, [app.transport]);

  useEffect(() => {
    if (!signedIn) {
      setBooks([]);
      setSelectedId(null);
      setDraft(null);
      return;
    }
    let cancelled = false;
    void reload()
      .then((list) => {
        if (cancelled) return;
        setSelectedId((prev) =>
          prev && list.some((b) => b.id === prev) ? prev : (list[0]?.id ?? null),
        );
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(e instanceof Error ? e.message : "failed to load risk portfolios");
      });
    return () => {
      cancelled = true;
    };
  }, [reload, signedIn]);

  // The firm hedge config (for the risk-model bindings). Failure is non-fatal: the
  // portfolio editor still works, the model control just reports itself unavailable
  // rather than silently rendering "Custom" over an unknown posture.
  useEffect(() => {
    if (!signedIn) {
      setHedgeConfig(null);
      return;
    }
    let cancelled = false;
    void app.transport
      .getHedgeConfig()
      .then((c) => {
        if (!cancelled) setHedgeConfig(c);
      })
      .catch(() => {
        if (!cancelled) setHedgeConfig(null);
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  // Desk roster for the owning-desk picker — an admin-gated RPC, fetched for admins.
  useEffect(() => {
    if (!isAdmin) {
      setDesks([]);
      return;
    }
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => {
        if (!cancelled) setDesks(d);
      })
      .catch(() => {
        /* desks are optional metadata — a failure just leaves the picker empty */
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, isAdmin]);

  const rows = useMemo(() => flattenTree(books), [books]);
  const selectedBook = useMemo(
    () => books.find((b) => b.id === selectedId) ?? null,
    [books, selectedId],
  );

  // Reseed the editor draft when the selected book changes (unless mid-create).
  useEffect(() => {
    if (creating) return;
    setDraft(selectedBook ? cloneBook(selectedBook) : null);
    setSaveState({ kind: "idle" });
  }, [selectedBook, creating]);

  const selectBook = useCallback((id: string): void => {
    setCreating(false);
    setSelectedId(id);
    setSaveState({ kind: "idle" });
  }, []);

  const startCreate = useCallback(
    (parentId: string | null): void => {
      setCreating(true);
      setSelectedId(null);
      // A sub-book INHERITS its parent's franchise (the server rejects a tree that
      // changes class mid-branch, so offering a choice here would only manufacture a
      // rejection); a top-level book takes the franchise currently on screen.
      const parent = parentId === null ? undefined : books.find((b) => b.id === parentId);
      const cls: CapabilityAsset =
        parent?.assetClass ?? (app.activeDomain === "fx_options" ? "fx_options" : "fixed_income");
      setDraft(blankBook(parentId, cls));
      setSaveState({ kind: "idle" });
    },
    [books, app.activeDomain],
  );

  const patch = useCallback((p: Partial<RiskBook>): void => {
    setDraft((d) => (d ? { ...d, ...p } : d));
  }, []);

  // Which limit fields currently hold an entry that failed to parse. A limit
  // the trader mistyped must never be saved as "uncapped" — the permissive
  // direction — so Save is blocked until the entry is fixed or cleared.
  const [invalidLimits, setInvalidLimits] = useState<ReadonlySet<string>>(new Set());
  const setLimitValidity = useCallback((key: string, valid: boolean): void => {
    setInvalidLimits((prev) => {
      if (valid === !prev.has(key)) return prev;
      const next = new Set(prev);
      if (valid) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const patchLimits = useCallback((p: Partial<RiskLimits>): void => {
    setDraft((d) => {
      if (!d) return d;
      const base: RiskLimits = d.limits ?? {
        maxNetNotional: null,
        maxGrossNotional: null,
        maxDv01: null,
      };
      const next: RiskLimits = { ...base, ...p };
      const empty =
        next.maxNetNotional === null &&
        next.maxGrossNotional === null &&
        next.maxDv01 === null;
      return { ...d, limits: empty ? null : next };
    });
  }, []);

  const save = useCallback(async (): Promise<void> => {
    if (!draft) return;
    if (draft.name.trim().length === 0) {
      setSaveState({ kind: "error", message: "a portfolio name is required" });
      return;
    }
    if (invalidLimits.size > 0) {
      setSaveState({
        kind: "error",
        message: "fix the highlighted pre-trade limit before saving",
      });
      return;
    }
    setSaveState({ kind: "saving" });
    try {
      const saved = creating
        ? await app.transport.createRiskBook(draft)
        : await app.transport.updateRiskBook(draft.id, draft);
      await reload();
      setCreating(false);
      setSelectedId(saved.id);
      // Enabling/disabling or renaming a book can flip the default-route validity —
      // let the startup routing guard re-evaluate.
      notifyRiskRoutingChanged();
      setSaveState({ kind: "ok", message: `saved “${saved.name}”` });
    } catch (e: unknown) {
      setSaveState({
        kind: "error",
        message: e instanceof Error ? e.message : "failed to save the risk portfolio",
      });
    }
  }, [draft, creating, invalidLimits, app.transport, reload]);

  const remove = useCallback(async (): Promise<void> => {
    if (!selectedBook) return;
    setSaveState({ kind: "saving" });
    try {
      await app.transport.deleteRiskBook(selectedBook.id);
      const list = await reload();
      setSelectedId(list[0]?.id ?? null);
      notifyRiskRoutingChanged();
      setSaveState({ kind: "ok", message: `deleted “${selectedBook.name}”` });
    } catch (e: unknown) {
      setSaveState({
        kind: "error",
        message: e instanceof Error ? e.message : "failed to delete the risk portfolio",
      });
    }
  }, [selectedBook, app.transport, reload]);

  // Valid re-parent targets: any book NOT in the draft's own subtree (acyclic guard).
  const parentOptions = useMemo(() => {
    if (!draft) return books;
    const excluded = draft.id ? subtreeIds(books, draft.id) : new Set<string>();
    // Only SAME-FRANCHISE portfolios are offerable parents: the server rejects a tree
    // that changes asset class mid-branch, so listing the other franchise here would
    // only let the trader build a selection that cannot be saved.
    return books.filter((b) => !excluded.has(b.id) && b.assetClass === draft.assetClass);
  }, [books, draft]);

  const deskName = useCallback(
    (id: string | null): string => {
      if (id === null) return "—";
      return desks.find((d) => d.id === id)?.name ?? id;
    },
    [desks],
  );

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.empty}>Sign in to manage risk portfolios.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>Risk Portfolios</h1>
          <p className={styles.note}>
            Firm risk portfolios (desk → portfolio → sub-portfolio) that routing rules drop each
            fill&apos;s risk into, each with its own limits.{" "}
            {readOnly
              ? "Read-only — the Manage-Risk capability is required to edit."
              : "You hold Manage-Risk — editing enabled."}{" "}
            Greeks, notional and PnL roll up the tree.
          </p>
          <p className={styles.note}>
            How your risk is <strong>bucketed</strong> for management — this is not the ledger
            &ldquo;Book&rdquo; where fills are actually booked, nor the &ldquo;Agg Book&rdquo; of LP
            prices.
          </p>
        </div>
        {!readOnly && (
          <Button onClick={() => startCreate(null)} data-testid="new-risk-book">
            + New portfolio
          </Button>
        )}
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      <div className={styles.body}>
        <nav className={styles.tree} aria-label="Risk portfolio tree">
          {rows.length === 0 && <p className={styles.empty}>No risk portfolios defined yet.</p>}
          <ul className={styles.treeList}>
            {rows.map(({ book, depth }) => (
              <li key={book.id}>
                <button
                  type="button"
                  className={
                    book.id === selectedId && !creating
                      ? `${styles.treeRow} ${styles.treeRowActive}`
                      : styles.treeRow
                  }
                  style={{ paddingLeft: `calc(${depth} * var(--space-5) + var(--space-3))` }}
                  onClick={() => selectBook(book.id)}
                  aria-current={book.id === selectedId && !creating}
                >
                  <span className={styles.treeName}>{book.name}</span>
                  {!book.enabled && <span className={styles.disabledTag}>disabled</span>}
                  {book.limits && <span className={styles.limitTag}>limits</span>}
                </button>
              </li>
            ))}
          </ul>
        </nav>

        <section className={styles.editor} aria-label="Risk portfolio editor">
          {!draft && <p className={styles.empty}>Select a portfolio to view or edit it.</p>}
          {draft && (
            <div className={styles.form}>
              <h2 className={styles.editorTitle}>
                {creating ? "New risk portfolio" : `Edit “${selectedBook?.name ?? draft.name}”`}
              </h2>

              <label className={styles.field}>
                <span className={styles.label}>Name</span>
                <input
                  className={styles.input}
                  value={draft.name}
                  disabled={draftReadOnly}
                  onChange={(e) => patch({ name: e.target.value })}
                  placeholder="e.g. FX EMEA Vanilla"
                />
              </label>

              <label className={styles.field}>
                <span className={styles.label}>Description</span>
                <input
                  className={styles.input}
                  value={draft.description}
                  disabled={draftReadOnly}
                  onChange={(e) => patch({ description: e.target.value })}
                  placeholder="What this portfolio is for"
                />
              </label>

              <div className={styles.row}>
                <label className={styles.field}>
                  <span className={styles.label}>Parent portfolio</span>
                  <select
                    className={styles.input}
                    value={draft.parentId ?? ""}
                    disabled={draftReadOnly}
                    onChange={(e) => patch({ parentId: e.target.value === "" ? null : e.target.value })}
                  >
                    <option value="">(top-level)</option>
                    {parentOptions.map((b) => (
                      <option key={b.id} value={b.id}>
                        {b.name}
                      </option>
                    ))}
                  </select>
                </label>

                <label className={styles.field}>
                  <span className={styles.label}>Owning desk</span>
                  <select
                    className={styles.input}
                    value={draft.deskId ?? ""}
                    disabled={draftReadOnly}
                    onChange={(e) => patch({ deskId: e.target.value === "" ? null : e.target.value })}
                  >
                    <option value="">(unowned)</option>
                    {desks.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>

                <label className={styles.field}>
                  <span className={styles.label}>Asset class</span>
                  <select
                    className={styles.input}
                    value={draft.assetClass}
                    // A sub-portfolio INHERITS its parent's franchise — the server rejects
                    // a tree that changes class mid-branch, so the control is locked rather
                    // than offering a choice that could only be refused.
                    disabled={draftReadOnly || draft.parentId !== null}
                    onChange={(e) => patch({ assetClass: e.target.value as CapabilityAsset })}
                  >
                    <option value="fixed_income">Fixed Income</option>
                    <option value="fx_options">FX Options</option>
                  </select>
                  <span className={styles.hint}>
                    {draft.parentId !== null
                      ? "Inherited from the parent portfolio."
                      : "The franchise this portfolio buckets. Vega and DV01 do not net, so a portfolio tree holds one class."}
                  </span>
                </label>
              </div>

              {/*
                RISK MODEL — how this portfolio manages the risk it holds. Saved
                separately from the portfolio definition (it lives on the firm hedge
                config as a `book`-scoped binding), so it applies immediately on change
                rather than waiting for "Save changes".
              */}
              {draft.id !== "" && (
                <fieldset className={styles.limits} disabled={draftReadOnly}>
                  <legend>Risk model</legend>
                  {hedgeConfig === null ? (
                    <span className={styles.hint}>
                      The hedge configuration could not be loaded, so this portfolio&apos;s risk
                      model is unknown. It is not being reported as Custom — that would claim a
                      posture we cannot currently read.
                    </span>
                  ) : (
                    <>
                      <label className={`${styles.field} ${styles.fieldWide}`}>
                        <span className={styles.label}>How this portfolio manages risk</span>
                        <select
                          className={styles.input}
                          value={resolvedModel?.model ?? 0}
                          onChange={(e) => {
                            const m = Number(e.target.value) as HedgingModel;
                            void applyModel(m, modelUsesBudget(m) ? (resolvedModel?.budget ?? 0) : 0);
                          }}
                        >
                          {HEDGING_MODELS.map((m) => (
                            <option key={m} value={m}>
                              {HEDGING_MODEL_LABEL[m]}
                            </option>
                          ))}
                        </select>
                        <span className={styles.hint}>
                          {HEDGING_MODEL_HINT[resolvedModel?.model ?? 0]}
                        </span>
                      </label>

                      {modelUsesBudget(resolvedModel?.model ?? 0) && (
                        <label className={`${styles.field} ${styles.fieldWide}`}>
                          <span className={styles.label}>DV01 warehouse budget</span>
                          <MagnitudeField
                            value={resolvedModel?.budget ?? null}
                            disabled={draftReadOnly}
                            onCommit={(v) => {
                              void applyModel(2, v ?? 0);
                            }}
                          />
                          <span className={styles.hint}>
                            How much DV01 this portfolio warehouses before it shivers risk out to
                            the street. Leave blank to inherit the scope&apos;s configured warehouse
                            threshold — blank means inherit, NOT a budget of zero. {MAGNITUDE_HELP}
                          </span>
                        </label>
                      )}

                      {/*
                        The resolution trace. Most-specific-wins across desk / book /
                        instrument means the effective posture is not inferable from any
                        single control — so state plainly which scope won and why.
                      */}
                      {resolvedModel !== null && (
                        <p className={styles.hint} data-testid="risk-model-resolution">
                          {explainRiskModel(resolvedModel, draft.id)}
                        </p>
                      )}
                      {modelState.kind === "error" && (
                        <p className={styles.error} role="alert">
                          {modelState.message}
                        </p>
                      )}
                      {modelState.kind === "ok" && (
                        <p className={styles.hint}>{modelState.message}</p>
                      )}
                    </>
                  )}
                </fieldset>
              )}

              <label className={styles.checkField}>
                <input
                  type="checkbox"
                  checked={draft.enabled}
                  disabled={draftReadOnly}
                  onChange={(e) => patch({ enabled: e.target.checked })}
                />
                <span>Enabled (only enabled portfolios are valid routing targets)</span>
              </label>

              <fieldset className={styles.limits} disabled={draftReadOnly}>
                <legend className={styles.label}>Pre-trade limits (blank ⇒ uncapped)</legend>
                <p className={styles.hint}>{MAGNITUDE_HELP}</p>
                <div className={styles.row}>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max net notional</span>
                    <MagnitudeField
                      className={styles.input}
                      value={draft.limits?.maxNetNotional ?? null}
                      onCommit={(v) => patchLimits({ maxNetNotional: v })}
                      onValidityChange={(ok) => setLimitValidity("maxNetNotional", ok)}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max gross notional</span>
                    <MagnitudeField
                      className={styles.input}
                      value={draft.limits?.maxGrossNotional ?? null}
                      onCommit={(v) => patchLimits({ maxGrossNotional: v })}
                      onValidityChange={(ok) => setLimitValidity("maxGrossNotional", ok)}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max DV01</span>
                    <MagnitudeField
                      className={styles.input}
                      value={draft.limits?.maxDv01 ?? null}
                      onCommit={(v) => patchLimits({ maxDv01: v })}
                      onValidityChange={(ok) => setLimitValidity("maxDv01", ok)}
                    />
                  </label>
                </div>
              </fieldset>

              {!creating && selectedBook && (
                <dl className={styles.meta}>
                  <div>
                    <dt>id</dt>
                    <dd className={styles.mono}>{selectedBook.id}</dd>
                  </div>
                  <div>
                    <dt>desk</dt>
                    <dd>{deskName(selectedBook.deskId)}</dd>
                  </div>
                  {selectedBook.limits?.maxNetNotional != null && (
                    <div>
                      <dt>net cap</dt>
                      <dd className={styles.mono}>{compact(selectedBook.limits.maxNetNotional)}</dd>
                    </div>
                  )}
                </dl>
              )}

              {!draftReadOnly && (
                <div className={styles.actions}>
                  <Button onClick={() => void save()} disabled={saveState.kind === "saving"}>
                    {creating ? "Create portfolio" : "Save changes"}
                  </Button>
                  {!creating && selectedBook && (
                    <Button
                      variant="ghost"
                      onClick={() => void remove()}
                      disabled={saveState.kind === "saving"}
                    >
                      Delete
                    </Button>
                  )}
                </div>
              )}

              {saveState.kind === "ok" && (
                <p className={styles.ok} role="status">
                  {saveState.message}
                </p>
              )}
              {saveState.kind === "error" && (
                <p className={styles.error} role="alert">
                  {saveState.message}
                </p>
              )}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
