/**
 * TieringWorkspace — the trader-facing FI Tiering surface (server commit 8404bc9).
 *
 * Until now a book's OUTBOUND tiering (widen / skew the consolidated composite
 * before it is published to clients) could only be retuned behind the ADMIN-only
 * "Manage" toggle on the Aggregated Book workspace. This first-class workspace
 * closes that gap: an ordinary TRADER (holding `quote_respond·fixed_income`, NOT
 * admin) can discover every aggregated book, see each book's current tiering
 * state, pick one, retune it with the shared {@link TieringEditor} (all three
 * strategies — Flat markup / Inventory skew / Scaled-smoothed spread — with the
 * per-strategy "?" doc links), and APPLY it through the trader-accessible
 * `AuthService.UpdateBookTiering` RPC. Book STRUCTURE (members / scope /
 * consolidation params) stays admin-only and is NOT editable here — this surface
 * edits ONLY tiering.
 *
 * Gating: registered in the rail under Fixed Income on `view·fixed_income` (the
 * same asset gate every FI trading workspace uses — reachable by any FI trader,
 * NOT admin-gated). Applying additionally requires `quote_respond·fixed_income`
 * (the exact capability the server enforces): a signed-in FI user who lacks it
 * still sees the books and their current tiering READ-ONLY, with a clear
 * permission note instead of a broken screen. `listAggregatedBooks` is
 * authenticated-only, so a non-admin can already list books + their `tiering`.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { TieringEditor } from "../components/TieringEditor";
import type { AggregatedBookDesc, TieringConfig } from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import {
  hasTieringErrors,
  TIERING_SPREAD_UNIT_LABEL,
  TIERING_STRATEGY_KIND_LABEL,
  validateTiering,
  type TieringErrors,
} from "../lib/tiering";
import styles from "./TieringWorkspace.module.css";

/** An all-clear error set for a disabled (null) tiering config. */
const NO_TIERING_ERRORS: TieringErrors = { strategies: {}, guardrails: {} };

/** Deep-clone a tiering config for the editor form (or pass through `null`). */
function cloneTiering(t: TieringConfig | null): TieringConfig | null {
  if (t === null) return null;
  return {
    unit: t.unit,
    strategies: t.strategies.map((s) => ({ ...s })),
    guardrails: t.guardrails ? { ...t.guardrails } : null,
    stalePolicy: t.stalePolicy,
  };
}

/** A compact human summary of a book's current tiering state (for the roster). */
function tieringSummary(t: TieringConfig | null): { on: boolean; label: string } {
  if (t === null) return { on: false, label: "No tiering" };
  const kinds = t.strategies.map((s) => TIERING_STRATEGY_KIND_LABEL[s.kind]);
  const label = kinds.length > 0 ? kinds.join(" · ") : "Enabled";
  return { on: true, label };
}

/** Whether two tiering configs are value-equal (drives the dirty / Apply state). */
function tieringEqual(a: TieringConfig | null, b: TieringConfig | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The result of the last Apply attempt (inline success / failure feedback). */
type ApplyState =
  | { kind: "idle" }
  | { kind: "applying" }
  | { kind: "ok"; bookName: string }
  | { kind: "error"; message: string };

export function TieringWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // The exact capability the server gates `UpdateBookTiering` on. An FI trader
  // holds it; an admin holds it too (grant-all). A signed-in FI user WITHOUT it
  // sees the books read-only + a permission note (never a broken screen).
  const canRetune = auth.can("quote_respond", "fixed_income");

  const [books, setBooks] = useState<AggregatedBookDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The editor's working config — seeded from the selected book's stored tiering,
  // retuned locally until Apply pushes it to the server.
  const [draft, setDraft] = useState<TieringConfig | null>(null);
  const [applyState, setApplyState] = useState<ApplyState>({ kind: "idle" });

  // Load (and reload after an apply) the roster — any authenticated user may list
  // it. A deterministic default selection lands on the first ENABLED book.
  const reloadBooks = useCallback(async (): Promise<void> => {
    const list = await app.transport.listAggregatedBooks();
    setBooks(list);
    setLoadError(null);
    setSelectedId((prev) => {
      if (prev && list.some((b) => b.id === prev)) return prev;
      const firstEnabled = list.find((b) => b.enabled) ?? list[0];
      return firstEnabled ? firstEnabled.id : null;
    });
  }, [app.transport]);

  useEffect(() => {
    if (!signedIn) {
      setBooks([]);
      setSelectedId(null);
      return;
    }
    let cancelled = false;
    void reloadBooks().catch((e: unknown) => {
      if (cancelled) return;
      setLoadError(e instanceof Error ? e.message : "failed to load aggregated books");
    });
    return () => {
      cancelled = true;
    };
  }, [reloadBooks, signedIn]);

  const selectedBook = useMemo(
    () => books.find((b) => b.id === selectedId) ?? null,
    [books, selectedId],
  );

  // Reseed the editor draft (+ clear apply feedback) ONLY when the SELECTED BOOK
  // changes — keyed on the id via a ref, NOT on the book object's identity. A
  // successful Apply replaces the book object in `books` (same id); guarding on the
  // id means that re-render does NOT wipe the just-applied draft / success badge
  // (apply reseeds the draft itself). `books` is a dep so the seed reads the freshly
  // loaded roster on first arrival, but the ref guard makes it a no-op unless the id
  // actually changed.
  const seededIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (selectedId === seededIdRef.current) return;
    seededIdRef.current = selectedId;
    const b = books.find((x) => x.id === selectedId) ?? null;
    setDraft(cloneTiering(b ? b.tiering : null));
    setApplyState({ kind: "idle" });
  }, [selectedId, books]);

  const errors = useMemo(
    () => (draft === null ? NO_TIERING_ERRORS : validateTiering(draft)),
    [draft],
  );
  const valid = draft === null || !hasTieringErrors(errors);
  const dirty = selectedBook !== null && !tieringEqual(draft, selectedBook.tiering);
  const applying = applyState.kind === "applying";
  const canApply = canRetune && selectedBook !== null && valid && dirty && !applying;

  const onChangeDraft = useCallback((next: TieringConfig | null): void => {
    setDraft(next);
    // A fresh edit clears any stale apply feedback.
    setApplyState((s) => (s.kind === "idle" || s.kind === "applying" ? s : { kind: "idle" }));
  }, []);

  const resetDraft = useCallback((): void => {
    setDraft(cloneTiering(selectedBook ? selectedBook.tiering : null));
    setApplyState({ kind: "idle" });
  }, [selectedBook]);

  const apply = useCallback(async (): Promise<void> => {
    if (selectedBook === null || !canRetune || !valid) return;
    setApplyState({ kind: "applying" });
    try {
      const updated = await app.transport.updateBookTiering(selectedBook.id, draft);
      setBooks((prev) => prev.map((b) => (b.id === updated.id ? updated : b)));
      setDraft(cloneTiering(updated.tiering));
      setApplyState({ kind: "ok", bookName: updated.name });
    } catch (e: unknown) {
      setApplyState({
        kind: "error",
        message: e instanceof Error ? e.message : "failed to apply tiering",
      });
    }
  }, [app.transport, canRetune, draft, selectedBook, valid]);

  // --- the sign-in gate ----------------------------------------------------
  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Book tiering</h2>
          <p className={styles.gateHint}>
            Sign in to discover and retune an aggregated book&apos;s outbound tiering.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  const selectedSummary = selectedBook ? tieringSummary(selectedBook.tiering) : null;

  return (
    <div className={styles.wrap}>
      <div className={styles.head}>
        <div className={styles.headMain}>
          <span className={styles.title}>Tiering</span>
          <span className={styles.note}>
            Widen and/or skew a book&apos;s consolidated composite before it is published to
            clients. Pick a book, retune its outbound tiering, and apply — book structure
            (members, scope, consolidation) stays with the desk administrator.
          </span>
        </div>
      </div>

      {!canRetune && (
        <p className={styles.permBanner} role="note">
          <span className={styles.permGlyph} aria-hidden="true">
            🔒︎
          </span>
          You don&apos;t have permission to retune tiering. This requires the{" "}
          <strong>Quote · Fixed Income</strong> capability
          {" "}
          <span className={styles.permHint}>
            ({capabilityDenialTitle("quote_respond", "fixed_income")}).
          </span>{" "}
          Books and their current tiering are shown below read-only.
        </p>
      )}

      {loadError && <p className={styles.banner}>{loadError}</p>}

      {books.length === 0 ? (
        <div className={styles.empty}>
          No aggregated books are defined. An administrator can define one in{" "}
          <strong>Administration → Aggregation</strong> or under{" "}
          <strong>Fixed Income → Agg Book → Manage</strong>.
        </div>
      ) : (
        <div className={styles.body}>
          {/* LEFT — the book roster with each book's tiering-state indicator. */}
          <section className={styles.roster} aria-label="select an aggregated book">
            <h3 className={styles.rosterHead}>Books</h3>
            <ul className={styles.bookList}>
              {books.map((b) => {
                const sum = tieringSummary(b.tiering);
                const active = selectedId === b.id;
                return (
                  <li key={b.id}>
                    <button
                      type="button"
                      className={`${styles.bookBtn} ${active ? styles.bookBtnActive : ""}`}
                      aria-pressed={active}
                      onClick={() => setSelectedId(b.id)}
                    >
                      <span className={styles.bookRow}>
                        <span className={styles.bookName}>{b.name}</span>
                        {!b.enabled && <span className={styles.bookOff}>off</span>}
                      </span>
                      <span className={styles.bookMeta}>
                        <span
                          className={`${styles.tierBadge} ${sum.on ? styles.tierOn : styles.tierOff}`}
                        >
                          <span className={styles.tierDot} aria-hidden="true" />
                          {sum.on ? "Tiering on" : "Tiering off"}
                        </span>
                        <span className={styles.tierLabel}>{sum.label}</span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>

          {/* RIGHT — the tiering editor (or a read-only summary without the cap),
              hosted in the shared Panel material (opaque `--bg-raised` surface) so
              the editor renders in the SAME context as the admin Manage editor. */}
          <section className={styles.editorPane} aria-label="book tiering">
            {selectedBook === null ? (
              <Panel title="Book tiering" glyph="⚖">
                <div className={styles.empty}>Select a book to view or retune its tiering.</div>
              </Panel>
            ) : (
              <Panel
                title={selectedBook.name}
                glyph="⚖"
                actions={
                  applyState.kind === "ok" ? (
                    <span className={styles.okBadge} role="status" aria-live="polite">
                      ✓ Applied
                    </span>
                  ) : (
                    <span className={styles.editorId}>{selectedBook.id}</span>
                  )
                }
              >
                {canRetune ? (
                  <>
                    <TieringEditor
                      value={draft}
                      onChange={onChangeDraft}
                      errors={errors}
                      idPrefix={`tiering-${selectedBook.id}`}
                    />

                    {applyState.kind === "error" && (
                      <p className={styles.banner} role="alert">
                        {applyState.message}
                      </p>
                    )}

                    <div className={styles.actions}>
                      <Button
                        variant="primary"
                        onClick={() => void apply()}
                        disabled={!canApply}
                        title={
                          !valid
                            ? "Fix the highlighted tiering errors first"
                            : !dirty
                              ? "No changes to apply"
                              : "Apply this tiering to the book"
                        }
                      >
                        {applying ? "Applying…" : "Apply tiering"}
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={resetDraft}
                        disabled={!dirty || applying}
                      >
                        Reset
                      </Button>
                      <span className={styles.dirtyHint} aria-live="polite">
                        {dirty ? "Unsaved changes" : "In sync with the book"}
                      </span>
                    </div>
                  </>
                ) : (
                  <div
                    className={styles.readonly}
                    aria-label="current tiering (read-only)"
                  >
                    <div className={styles.roRow}>
                      <span className={styles.roLabel}>State</span>
                      <span className={styles.roValue}>
                        {selectedSummary?.on ? "Enabled" : "Disabled"}
                      </span>
                    </div>
                    {selectedBook.tiering && (
                      <>
                        <div className={styles.roRow}>
                          <span className={styles.roLabel}>Spread unit</span>
                          <span className={styles.roValue}>
                            {TIERING_SPREAD_UNIT_LABEL[selectedBook.tiering.unit]}
                          </span>
                        </div>
                        <div className={styles.roRow}>
                          <span className={styles.roLabel}>Strategies</span>
                          <span className={styles.roValue}>
                            {selectedBook.tiering.strategies.length > 0
                              ? selectedBook.tiering.strategies
                                  .map((s) => TIERING_STRATEGY_KIND_LABEL[s.kind])
                                  .join(" · ")
                              : "—"}
                          </span>
                        </div>
                      </>
                    )}
                    <p className={styles.roNote}>
                      Read-only — you can view the book&apos;s tiering but not change it.
                    </p>
                  </div>
                )}
              </Panel>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
