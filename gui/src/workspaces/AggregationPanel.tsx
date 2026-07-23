/**
 * AggregationPanel — the FI Aggregated Book admin surface (ADR-0022 tab B). The
 * admin defines composite books that consolidate N inbound liquidity members
 * (FIX acceptors + LP-feed members such as `LP-SIM-01`) into ONE best bid/offer
 * per instrument. A two-column workspace: the roster of existing books (left) and
 * the definition editor (right — create, or edit a selected book). Mutations go
 * through the parent's `run` wrapper (which surfaces failures) and refetch on
 * success (no optimistic state — the server owns the roster), mirroring the
 * Entity/Book registry panels.
 *
 * Members are transport-agnostic connection ids: the editor offers the managed
 * FIX-connection registry as toggle candidates, quick-adds for the LP-SIM fleet,
 * and a free-text field to name any member id (the wire is a plain string set).
 * Instrument scope is either ALL_MEMBERS_QUOTE (the union of the members' live
 * streams) or an EXPLICIT instrument-id set. The consolidation tuning (staleness
 * τ, hard max age, divergence gating, min contributors, depth) is edited inline.
 */

import { useMemo, useState } from "react";

import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type {
  AggregatedBookDesc,
  AggregatedBookSpec,
  AggregationParams,
  AggregationScopeMode,
  FixConnection,
} from "../data/contract";
import styles from "./AggregationPanel.module.css";

/** The LP-SIM fleet members the bundled sim streams (ADR-0022 D3 / celnet-lp-sim). */
const LP_SIM_MEMBERS: readonly string[] = [
  "LP-SIM-01",
  "LP-SIM-02",
  "LP-SIM-03",
  "LP-SIM-04",
];

/** Sane default consolidation tuning for a fresh book (the admin can override). */
const DEFAULT_PARAMS: AggregationParams = {
  stalenessTauMs: 2000,
  maxQuoteAgeMs: 5000,
  divergenceGating: true,
  minContributors: 2,
  depthLevels: 1,
};

/** The editable form model; `editingId === null` ⇒ create mode. */
interface AggForm {
  editingId: string | null;
  name: string;
  members: string[];
  scopeMode: AggregationScopeMode;
  instrumentIds: string[];
  params: AggregationParams;
  enabled: boolean;
}

const EMPTY_FORM: AggForm = {
  editingId: null,
  name: "",
  members: [],
  scopeMode: "ALL_MEMBERS_QUOTE",
  instrumentIds: [],
  params: DEFAULT_PARAMS,
  enabled: true,
};

interface AggregationPanelProps {
  books: AggregatedBookDesc[];
  /** Managed FIX connections offered as member candidates (by id). */
  connections: FixConnection[];
  onCreate: (spec: AggregatedBookSpec) => Promise<unknown>;
  onUpdate: (id: string, spec: AggregatedBookSpec) => Promise<unknown>;
  onDelete: (id: string) => Promise<unknown>;
  run: (action: () => Promise<unknown>) => Promise<void>;
}

/** Load an existing book into the editor form (edit mode). */
function formFromBook(book: AggregatedBookDesc): AggForm {
  return {
    editingId: book.id,
    name: book.name,
    members: [...book.memberConnectionIds],
    scopeMode: book.scopeMode,
    instrumentIds: [...book.instrumentIds],
    params: { ...book.params },
    enabled: book.enabled,
  };
}

export function AggregationPanel({
  books,
  connections,
  onCreate,
  onUpdate,
  onDelete,
  run,
}: AggregationPanelProps): React.ReactElement {
  const [form, setForm] = useState<AggForm>(EMPTY_FORM);
  const [memberDraft, setMemberDraft] = useState("");
  const [instrumentDraft, setInstrumentDraft] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const isEditing = form.editingId !== null;
  const name = form.name.trim();
  // A member set must be non-empty; an EXPLICIT scope additionally needs ≥1 id.
  const scopeOk = form.scopeMode === "ALL_MEMBERS_QUOTE" || form.instrumentIds.length > 0;
  const paramsOk =
    form.params.stalenessTauMs > 0 &&
    form.params.maxQuoteAgeMs > 0 &&
    form.params.minContributors >= 1 &&
    form.params.depthLevels >= 1;
  const canSubmit = name.length > 0 && form.members.length > 0 && scopeOk && paramsOk;

  // The FIX-connection candidates NOT already selected (offered as toggles).
  const connectionCandidates = useMemo(
    () => connections.filter((c) => !form.members.includes(c.id)),
    [connections, form.members],
  );
  const lpSimCandidates = useMemo(
    () => LP_SIM_MEMBERS.filter((m) => !form.members.includes(m)),
    [form.members],
  );

  const resetForm = (): void => {
    setForm(EMPTY_FORM);
    setMemberDraft("");
    setInstrumentDraft("");
  };

  const addMember = (id: string): void => {
    const trimmed = id.trim();
    if (trimmed.length === 0) return;
    setForm((f) =>
      f.members.includes(trimmed) ? f : { ...f, members: [...f.members, trimmed] },
    );
    setMemberDraft("");
  };
  const removeMember = (id: string): void =>
    setForm((f) => ({ ...f, members: f.members.filter((m) => m !== id) }));

  const addInstrument = (id: string): void => {
    const trimmed = id.trim();
    if (trimmed.length === 0) return;
    setForm((f) =>
      f.instrumentIds.includes(trimmed)
        ? f
        : { ...f, instrumentIds: [...f.instrumentIds, trimmed] },
    );
    setInstrumentDraft("");
  };
  const removeInstrument = (id: string): void =>
    setForm((f) => ({ ...f, instrumentIds: f.instrumentIds.filter((x) => x !== id) }));

  const setParam = <K extends keyof AggregationParams>(
    key: K,
    value: AggregationParams[K],
  ): void => setForm((f) => ({ ...f, params: { ...f.params, [key]: value } }));

  const submit = (e: React.FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!canSubmit) return;
    const spec: AggregatedBookSpec = {
      id: form.editingId ?? "",
      name,
      memberConnectionIds: form.members,
      scopeMode: form.scopeMode,
      // Only an EXPLICIT book carries instrument ids (ignored server-side otherwise).
      instrumentIds: form.scopeMode === "EXPLICIT" ? form.instrumentIds : [],
      params: form.params,
      enabled: form.enabled,
    };
    void run(async () => {
      if (form.editingId === null) {
        await onCreate(spec);
      } else {
        await onUpdate(form.editingId, spec);
      }
      resetForm();
    });
  };

  const nameOfMember = (id: string): string => {
    const conn = connections.find((c) => c.id === id);
    return conn ? conn.name : id;
  };

  return (
    <div className={styles.layout}>
      {/* LEFT — the roster of defined books. */}
      <Panel title="Aggregated books" glyph="◫">
        {books.length === 0 ? (
          <p className={styles.empty}>
            No aggregated books yet. An aggregated book consolidates several inbound
            liquidity members into one best bid/offer per instrument. Define one on the
            right.
          </p>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Name</th>
                <th>Members</th>
                <th>Scope</th>
                <th>State</th>
                <th className={styles.actionsCol}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {books.map((book) => {
                const selected = form.editingId === book.id;
                return (
                  <tr key={book.id} className={selected ? styles.rowSelected : undefined}>
                    <td className={styles.nameCell}>
                      <span className={styles.bookName}>{book.name}</span>
                      <span className={styles.bookId}>{book.id}</span>
                    </td>
                    <td className={styles.mono}>{book.memberConnectionIds.length}</td>
                    <td>
                      {book.scopeMode === "ALL_MEMBERS_QUOTE" ? (
                        <span className={styles.scopeTag}>all quotes</span>
                      ) : (
                        <span className={styles.scopeTag}>
                          {book.instrumentIds.length} instrument
                          {book.instrumentIds.length === 1 ? "" : "s"}
                        </span>
                      )}
                    </td>
                    <td>
                      {book.enabled ? (
                        <span className={`${styles.stateBadge} ${styles.stateOn}`}>Enabled</span>
                      ) : (
                        <span className={`${styles.stateBadge} ${styles.stateOff}`}>Disabled</span>
                      )}
                    </td>
                    <td className={styles.actionsCol}>
                      <div className={styles.rowActions}>
                        <Button variant="secondary" onClick={() => setForm(formFromBook(book))}>
                          Edit
                        </Button>
                        {confirmDeleteId === book.id ? (
                          <>
                            <Button
                              variant="ghost"
                              onClick={() =>
                                void run(async () => {
                                  await onDelete(book.id);
                                  setConfirmDeleteId(null);
                                  if (form.editingId === book.id) resetForm();
                                })
                              }
                            >
                              Confirm
                            </Button>
                            <Button variant="ghost" onClick={() => setConfirmDeleteId(null)}>
                              Cancel
                            </Button>
                          </>
                        ) : (
                          <Button variant="ghost" onClick={() => setConfirmDeleteId(book.id)}>
                            Delete
                          </Button>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Panel>

      {/* RIGHT — the definition editor. */}
      <Panel title={isEditing ? "Edit book" : "Define a book"} glyph="✎">
        <form className={styles.form} onSubmit={submit}>
          <label className={styles.field} htmlFor="agg-name">
            <span className={styles.fieldLabel}>Name</span>
            <input
              id="agg-name"
              className={styles.input}
              type="text"
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              placeholder="e.g. US Treasuries"
              aria-label="aggregated book name"
            />
          </label>

          {/* Members — chips + FIX/LP-SIM candidate toggles + free-text add. */}
          <div className={styles.field}>
            <span className={styles.fieldLabel}>
              Members{" "}
              <span className={styles.fieldHint}>
                inbound liquidity connections whose quotes feed the composite
              </span>
            </span>
            {form.members.length > 0 ? (
              <ul className={styles.chips} aria-label="selected members">
                {form.members.map((m) => (
                  <li key={m} className={styles.chip}>
                    <span className={styles.chipLabel}>{nameOfMember(m)}</span>
                    <button
                      type="button"
                      className={styles.chipRemove}
                      onClick={() => removeMember(m)}
                      aria-label={`Remove member ${m}`}
                      title="Remove member"
                    >
                      ✕
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className={styles.chipsEmpty}>No members yet — add at least one below.</p>
            )}

            {(connectionCandidates.length > 0 || lpSimCandidates.length > 0) && (
              <div className={styles.candidates} role="group" aria-label="add a member">
                {connectionCandidates.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={styles.candidate}
                    onClick={() => addMember(c.id)}
                    title={`FIX connection · ${c.id}`}
                  >
                    + {c.name}
                  </button>
                ))}
                {lpSimCandidates.map((m) => (
                  <button
                    key={m}
                    type="button"
                    className={`${styles.candidate} ${styles.candidateSim}`}
                    onClick={() => addMember(m)}
                    title="LP-SIM fleet member"
                  >
                    + {m}
                  </button>
                ))}
              </div>
            )}

            <div className={styles.addRow}>
              <input
                className={styles.input}
                type="text"
                value={memberDraft}
                onChange={(e) => setMemberDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addMember(memberDraft);
                  }
                }}
                placeholder="…or type a member connection id"
                aria-label="add member connection id"
              />
              <Button
                type="button"
                variant="secondary"
                onClick={() => addMember(memberDraft)}
                disabled={memberDraft.trim().length === 0}
              >
                Add
              </Button>
            </div>
          </div>

          {/* Instrument scope — segmented control + explicit id chips. */}
          <div className={styles.field}>
            <span className={styles.fieldLabel}>Instrument scope</span>
            <div className={styles.seg} role="group" aria-label="instrument scope">
              <button
                type="button"
                className={`${styles.segBtn} ${form.scopeMode === "ALL_MEMBERS_QUOTE" ? styles.segBtnActive : ""}`}
                aria-pressed={form.scopeMode === "ALL_MEMBERS_QUOTE"}
                onClick={() => setForm((f) => ({ ...f, scopeMode: "ALL_MEMBERS_QUOTE" }))}
              >
                All members quote
              </button>
              <button
                type="button"
                className={`${styles.segBtn} ${form.scopeMode === "EXPLICIT" ? styles.segBtnActive : ""}`}
                aria-pressed={form.scopeMode === "EXPLICIT"}
                onClick={() => setForm((f) => ({ ...f, scopeMode: "EXPLICIT" }))}
              >
                Explicit instruments
              </button>
            </div>

            {form.scopeMode === "EXPLICIT" && (
              <div className={styles.explicitBox}>
                {form.instrumentIds.length > 0 ? (
                  <ul className={styles.chips} aria-label="explicit instruments">
                    {form.instrumentIds.map((id) => (
                      <li key={id} className={styles.chip}>
                        <span className={`${styles.chipLabel} ${styles.mono}`}>{id}</span>
                        <button
                          type="button"
                          className={styles.chipRemove}
                          onClick={() => removeInstrument(id)}
                          aria-label={`Remove instrument ${id}`}
                          title="Remove instrument"
                        >
                          ✕
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className={styles.chipsEmpty}>
                    Add at least one instrument id (e.g. a Treasury CUSIP).
                  </p>
                )}
                <div className={styles.addRow}>
                  <input
                    className={styles.input}
                    type="text"
                    value={instrumentDraft}
                    onChange={(e) => setInstrumentDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        addInstrument(instrumentDraft);
                      }
                    }}
                    placeholder="Instrument id, e.g. 91282CJL6"
                    aria-label="add instrument id"
                  />
                  <Button
                    type="button"
                    variant="secondary"
                    onClick={() => addInstrument(instrumentDraft)}
                    disabled={instrumentDraft.trim().length === 0}
                  >
                    Add
                  </Button>
                </div>
              </div>
            )}
          </div>

          {/* Consolidation tuning. */}
          <div className={styles.field}>
            <span className={styles.fieldLabel}>Consolidation tuning</span>
            <div className={styles.paramsGrid}>
              <label className={styles.param} htmlFor="agg-tau">
                <span className={styles.paramLabel}>Staleness τ (ms)</span>
                <input
                  id="agg-tau"
                  className={`${styles.input} ${styles.numInput}`}
                  type="number"
                  min={1}
                  step={100}
                  value={form.params.stalenessTauMs}
                  onChange={(e) => setParam("stalenessTauMs", Number(e.target.value))}
                />
              </label>
              <label className={styles.param} htmlFor="agg-maxage">
                <span className={styles.paramLabel}>Max quote age (ms)</span>
                <input
                  id="agg-maxage"
                  className={`${styles.input} ${styles.numInput}`}
                  type="number"
                  min={1}
                  step={100}
                  value={form.params.maxQuoteAgeMs}
                  onChange={(e) => setParam("maxQuoteAgeMs", Number(e.target.value))}
                />
              </label>
              <label className={styles.param} htmlFor="agg-mincontrib">
                <span className={styles.paramLabel}>Min contributors</span>
                <input
                  id="agg-mincontrib"
                  className={`${styles.input} ${styles.numInput}`}
                  type="number"
                  min={1}
                  step={1}
                  value={form.params.minContributors}
                  onChange={(e) => setParam("minContributors", Number(e.target.value))}
                />
              </label>
              <label className={styles.param} htmlFor="agg-depth">
                <span className={styles.paramLabel}>Depth levels</span>
                <input
                  id="agg-depth"
                  className={`${styles.input} ${styles.numInput}`}
                  type="number"
                  min={1}
                  step={1}
                  value={form.params.depthLevels}
                  onChange={(e) => setParam("depthLevels", Number(e.target.value))}
                />
              </label>
            </div>
            <label className={styles.toggleRow}>
              <input
                type="checkbox"
                checked={form.params.divergenceGating}
                onChange={(e) => setParam("divergenceGating", e.target.checked)}
              />
              <span>
                Divergence gating{" "}
                <span className={styles.fieldHint}>drop MAD-outlier members before consolidating</span>
              </span>
            </label>
          </div>

          <label className={styles.toggleRow}>
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
            />
            <span>
              Enabled{" "}
              <span className={styles.fieldHint}>a disabled book stands up no engine</span>
            </span>
          </label>

          <div className={styles.formActions}>
            <Button type="submit" variant="primary" disabled={!canSubmit}>
              {isEditing ? "Save book" : "Create book"}
            </Button>
            {isEditing && (
              <Button type="button" variant="ghost" onClick={resetForm}>
                Cancel
              </Button>
            )}
          </div>
        </form>
      </Panel>
    </div>
  );
}
