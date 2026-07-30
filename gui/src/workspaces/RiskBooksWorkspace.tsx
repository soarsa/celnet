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
 * Gating: every risk-routing RPC is admin-gated server-side. Edit affordances gate
 * on `auth.isAdmin` (mirrors {@link PricingGroupsWorkspace}); a non-admin who
 * reaches the pane sees the tree read-only. The firm-wide routing GRAPH that maps
 * fills to leaf books is edited on the pass-6b flow-canvas surface, not here.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import type { DeskDesc, RiskBook, RiskLimits } from "../data/contract";
import styles from "./RiskBooksWorkspace.module.css";

/** A fresh blank book draft for the Create flow (id blank ⇒ server mints from name). */
function blankBook(parentId: string | null): RiskBook {
  return {
    id: "",
    name: "",
    parentId,
    deskId: null,
    description: "",
    limits: null,
    enabled: true,
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

/** Parse a limits number field: blank ⇒ null (uncapped); otherwise a finite number. */
function parseCap(raw: string): number | null {
  const t = raw.trim();
  if (t.length === 0) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

/** Render a cap for an input value: null ⇒ blank. */
function capValue(n: number | null): string {
  return n === null ? "" : String(n);
}

const compact = (n: number): string =>
  new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(n);

export function RiskBooksWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  const isAdmin = auth.isAdmin;
  const readOnly = !isAdmin;

  const [books, setBooks] = useState<RiskBook[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<RiskBook | null>(null);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

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

  const startCreate = useCallback((parentId: string | null): void => {
    setCreating(true);
    setSelectedId(null);
    setDraft(blankBook(parentId));
    setSaveState({ kind: "idle" });
  }, []);

  const patch = useCallback((p: Partial<RiskBook>): void => {
    setDraft((d) => (d ? { ...d, ...p } : d));
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
    setSaveState({ kind: "saving" });
    try {
      const saved = creating
        ? await app.transport.createRiskBook(draft)
        : await app.transport.updateRiskBook(draft.id, draft);
      await reload();
      setCreating(false);
      setSelectedId(saved.id);
      setSaveState({ kind: "ok", message: `saved “${saved.name}”` });
    } catch (e: unknown) {
      setSaveState({
        kind: "error",
        message: e instanceof Error ? e.message : "failed to save the risk portfolio",
      });
    }
  }, [draft, creating, app.transport, reload]);

  const remove = useCallback(async (): Promise<void> => {
    if (!selectedBook) return;
    setSaveState({ kind: "saving" });
    try {
      await app.transport.deleteRiskBook(selectedBook.id);
      const list = await reload();
      setSelectedId(list[0]?.id ?? null);
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
    return books.filter((b) => !excluded.has(b.id));
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
            Trader-defined risk portfolios (desk → portfolio → sub-portfolio) that routing rules drop
            each fill&apos;s risk into, each with its own limits. {readOnly ? "Read-only" : "Admin"}{" "}
            view — greeks, notional and PnL roll up the tree.
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
                  disabled={readOnly}
                  onChange={(e) => patch({ name: e.target.value })}
                  placeholder="e.g. FX EMEA Vanilla"
                />
              </label>

              <label className={styles.field}>
                <span className={styles.label}>Description</span>
                <input
                  className={styles.input}
                  value={draft.description}
                  disabled={readOnly}
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
                    disabled={readOnly}
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
                    disabled={readOnly}
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
              </div>

              <label className={styles.checkField}>
                <input
                  type="checkbox"
                  checked={draft.enabled}
                  disabled={readOnly}
                  onChange={(e) => patch({ enabled: e.target.checked })}
                />
                <span>Enabled (only enabled portfolios are valid routing targets)</span>
              </label>

              <fieldset className={styles.limits} disabled={readOnly}>
                <legend className={styles.label}>Pre-trade limits (blank ⇒ uncapped)</legend>
                <div className={styles.row}>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max net notional</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      value={capValue(draft.limits?.maxNetNotional ?? null)}
                      onChange={(e) => patchLimits({ maxNetNotional: parseCap(e.target.value) })}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max gross notional</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      value={capValue(draft.limits?.maxGrossNotional ?? null)}
                      onChange={(e) => patchLimits({ maxGrossNotional: parseCap(e.target.value) })}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.subLabel}>Max DV01</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      value={capValue(draft.limits?.maxDv01 ?? null)}
                      onChange={(e) => patchLimits({ maxDv01: parseCap(e.target.value) })}
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

              {!readOnly && (
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
