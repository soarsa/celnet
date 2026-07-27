/**
 * PricingGroupsWorkspace — the admin drag-and-drop pricing-pipeline builder
 * (docs/FI-PRICING-GROUPS-DESIGN.md §6 + §8.5, server commit 07fc99f).
 *
 * A pricing group maps many FIX connections / users / desks onto ONE pricing
 * definition: two feature pipelines (ESP streaming + RFQ / RFS quoting), each a
 * RAW → ordered-features → OUTBOUND waterfall the trader composes by dragging
 * features from a palette (MID SHIFT · TIERING · AXE · POSITION · PANIC/SKEW) onto
 * a canvas, reordering + configuring them inline. A `sharePipeline` switch makes
 * RFQ mirror ESP. Membership is assigned from the live FIX-connection / user / desk
 * rosters. A client-side {@link previewPipeline} waterfall shows the indicative
 * two-way AFTER each feature as the pipeline is built.
 *
 * Gating: registered under Administration and admin-gated by the rail. An admin
 * creates / updates the whole group (structure + both pipelines); a non-admin who
 * still reaches the pane but holds `quote_respond·fixed_income` may retune ONLY the
 * pipeline block (structure read-only) via `UpdatePricingGroupPipeline` — the exact
 * capability the server enforces.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type {
  DeskDesc,
  FeatureKind,
  FeaturePipeline,
  FeatureSpec,
  FixConnection,
  PricingGroup,
  PricingMode,
  UserDesc,
} from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import {
  defaultFeatureSpec,
  defaultPipeline,
  FEATURE_KIND_HINT,
  FEATURE_KIND_LABEL,
  FEATURE_KINDS,
  hasPipelineErrors,
  hasPricingGroupErrors,
  insertFeatureAt,
  moveFeature,
  previewPipeline,
  removeFeatureAt,
  SAMPLE_RAW,
  updateFeatureAt,
  validatePipeline,
  validatePricingGroup,
  type PipelineErrors,
  type TwoWay,
} from "../lib/pricingGroups";
import { PricingFeatureCard } from "./PricingFeatureCard";
import styles from "./PricingGroupsWorkspace.module.css";

const PRICING_MODES: readonly PricingMode[] = ["ESP", "RFQ"];
const PRICING_MODE_LABEL: Record<PricingMode, string> = { ESP: "ESP (streaming)", RFQ: "RFQ / RFS" };
const EMPTY_PIPELINE_ERRORS: PipelineErrors = { features: {}, guardrails: {} };

/** A fresh blank group draft for the Create flow (id blank ⇒ server mints from name). */
function blankGroup(): PricingGroup {
  return {
    id: "",
    name: "",
    description: "",
    memberConnectionIds: [],
    memberUserIds: [],
    memberDesks: [],
    espPipeline: null,
    rfqPipeline: null,
    sharePipeline: false,
    enabled: true,
  };
}

/** Deep-clone a group draft so the editor never aliases the loaded roster object. */
function cloneGroup(g: PricingGroup): PricingGroup {
  return {
    ...g,
    memberConnectionIds: [...g.memberConnectionIds],
    memberUserIds: [...g.memberUserIds],
    memberDesks: [...g.memberDesks],
    espPipeline: clonePipeline(g.espPipeline),
    rfqPipeline: clonePipeline(g.rfqPipeline),
  };
}

function clonePipeline(p: FeaturePipeline | null): FeaturePipeline | null {
  if (p === null) return null;
  return {
    features: p.features.map((f) => ({
      ...f,
      tiering: f.tiering
        ? {
            ...f.tiering,
            strategies: f.tiering.strategies.map((s) => ({ ...s })),
            guardrails: f.tiering.guardrails ? { ...f.tiering.guardrails } : null,
          }
        : null,
    })),
    guardrails: p.guardrails ? { ...p.guardrails } : null,
  };
}

/** The result of the last Save attempt (inline success / failure feedback). */
type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; name: string }
  | { kind: "error"; message: string };

const fmt = (n: number): string => n.toFixed(4);

export function PricingGroupsWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  const isAdmin = auth.isAdmin;
  // The exact capability the server gates the pipeline-only RPC on; an admin holds
  // it too (grant-all). Drives whether a non-admin who reaches the pane may retune.
  const canRetune = auth.can("quote_respond", "fixed_income");
  const readOnlyStructure = !isAdmin;
  const readOnlyPipeline = !(isAdmin || canRetune);

  const [groups, setGroups] = useState<PricingGroup[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<PricingGroup | null>(null);
  const [mode, setMode] = useState<PricingMode>("ESP");
  const [expandedFeature, setExpandedFeature] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  const [dragCardIndex, setDragCardIndex] = useState<number | null>(null);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [users, setUsers] = useState<UserDesc[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);

  // --- roster loading -------------------------------------------------------
  const reloadGroups = useCallback(async (): Promise<PricingGroup[]> => {
    const list = await app.transport.listPricingGroups();
    setGroups(list);
    setLoadError(null);
    return list;
  }, [app.transport]);

  useEffect(() => {
    if (!signedIn) {
      setGroups([]);
      setSelectedId(null);
      setDraft(null);
      return;
    }
    let cancelled = false;
    void reloadGroups()
      .then((list) => {
        if (cancelled) return;
        setSelectedId((prev) => {
          if (prev && list.some((g) => g.id === prev)) return prev;
          return list.length > 0 ? (list[0] as PricingGroup).id : null;
        });
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(e instanceof Error ? e.message : "failed to load pricing groups");
      });
    return () => {
      cancelled = true;
    };
  }, [reloadGroups, signedIn]);

  // Membership candidate rosters — admin-gated RPCs, so fetched only for admins.
  useEffect(() => {
    if (!isAdmin) {
      setConnections([]);
      setUsers([]);
      setDesks([]);
      return;
    }
    let cancelled = false;
    void Promise.allSettled([
      app.transport.listFixConnections(),
      app.transport.listUsers(),
      app.transport.listDesks(),
    ]).then((results) => {
      if (cancelled) return;
      const [c, u, d] = results;
      if (c.status === "fulfilled") setConnections(c.value);
      if (u.status === "fulfilled") setUsers(u.value);
      if (d.status === "fulfilled") setDesks(d.value);
    });
    return () => {
      cancelled = true;
    };
  }, [app.transport, isAdmin]);

  const selectedGroup = useMemo(
    () => groups.find((g) => g.id === selectedId) ?? null,
    [groups, selectedId],
  );

  // Reseed the editor draft when the SELECTED group changes (keyed on id via a ref so
  // a save-driven roster replacement of the same id does not wipe the applied draft).
  const seededRef = useRef<string | null>(null);
  useEffect(() => {
    if (creating) return;
    const key = selectedId ?? "";
    if (key === seededRef.current) return;
    seededRef.current = key;
    const g = groups.find((x) => x.id === selectedId) ?? null;
    setDraft(g ? cloneGroup(g) : null);
    setMode("ESP");
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
  }, [selectedId, groups, creating]);

  // --- selection / create / clone / delete ---------------------------------
  const selectGroup = useCallback((id: string): void => {
    setCreating(false);
    seededRef.current = null; // force a reseed even if id matches a stale ref
    setSelectedId(id);
  }, []);

  const startCreate = useCallback((): void => {
    setCreating(true);
    setSelectedId(null);
    setDraft(blankGroup());
    setMode("ESP");
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
  }, []);

  const startClone = useCallback((): void => {
    if (!selectedGroup) return;
    setCreating(true);
    setSelectedId(null);
    setDraft({ ...cloneGroup(selectedGroup), id: "", name: `${selectedGroup.name}-COPY` });
    setMode("ESP");
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
  }, [selectedGroup]);

  const deleteSelected = useCallback(async (): Promise<void> => {
    if (!selectedGroup) return;
    setSaveState({ kind: "saving" });
    try {
      await app.transport.deletePricingGroup(selectedGroup.id);
      const list = await reloadGroups();
      setCreating(false);
      seededRef.current = null;
      setSelectedId(list.length > 0 ? (list[0] as PricingGroup).id : null);
      setSaveState({ kind: "idle" });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to delete group" });
    }
  }, [app.transport, reloadGroups, selectedGroup]);

  // --- draft mutation -------------------------------------------------------
  const patchDraft = useCallback((next: Partial<PricingGroup>): void => {
    setDraft((d) => (d ? { ...d, ...next } : d));
    setSaveState((s) => (s.kind === "ok" ? { kind: "idle" } : s));
  }, []);

  const effectiveMode: PricingMode = draft?.sharePipeline ? "ESP" : mode;
  const activePipeline: FeaturePipeline | null = draft
    ? effectiveMode === "ESP"
      ? draft.espPipeline
      : draft.rfqPipeline
    : null;

  // Update the pipeline of the effective mode (ESP when sharing) immutably.
  const updateActivePipeline = useCallback(
    (fn: (p: FeaturePipeline | null) => FeaturePipeline | null): void => {
      setDraft((d) => {
        if (!d) return d;
        const em: PricingMode = d.sharePipeline ? "ESP" : mode;
        const cur = em === "ESP" ? d.espPipeline : d.rfqPipeline;
        const next = fn(cur);
        return em === "ESP" ? { ...d, espPipeline: next } : { ...d, rfqPipeline: next };
      });
      setSaveState((s) => (s.kind === "ok" ? { kind: "idle" } : s));
    },
    [mode],
  );

  const toggleCustomPipeline = useCallback(
    (on: boolean): void => {
      updateActivePipeline(() => (on ? defaultPipeline() : null));
      setExpandedFeature(null);
    },
    [updateActivePipeline],
  );

  const insertFeature = useCallback(
    (feature: FeatureSpec, at: number): void => {
      updateActivePipeline((p) => {
        const base = p ?? defaultPipeline();
        return { ...base, features: insertFeatureAt(base.features, feature, at) };
      });
    },
    [updateActivePipeline],
  );

  const removeFeature = useCallback(
    (index: number): void => {
      updateActivePipeline((p) => (p ? { ...p, features: removeFeatureAt(p.features, index) } : p));
      setExpandedFeature(null);
    },
    [updateActivePipeline],
  );

  const moveFeatureBy = useCallback(
    (from: number, to: number): void => {
      updateActivePipeline((p) => (p ? { ...p, features: moveFeature(p.features, from, to) } : p));
      setExpandedFeature(null);
    },
    [updateActivePipeline],
  );

  const patchFeature = useCallback(
    (index: number, next: Partial<FeatureSpec>): void => {
      updateActivePipeline((p) => (p ? { ...p, features: updateFeatureAt(p.features, index, next) } : p));
    },
    [updateActivePipeline],
  );

  const patchGuardrails = useCallback(
    (next: Partial<NonNullable<FeaturePipeline["guardrails"]>>): void => {
      updateActivePipeline((p) => {
        if (!p || p.guardrails === null) return p;
        return { ...p, guardrails: { ...p.guardrails, ...next } };
      });
    },
    [updateActivePipeline],
  );

  const toggleMember = useCallback(
    (kind: "conn" | "user" | "desk", id: string): void => {
      setDraft((d) => {
        if (!d) return d;
        const key =
          kind === "conn" ? "memberConnectionIds" : kind === "user" ? "memberUserIds" : "memberDesks";
        const cur = d[key];
        const has = cur.includes(id);
        return { ...d, [key]: has ? cur.filter((x) => x !== id) : [...cur, id] };
      });
      setSaveState((s) => (s.kind === "ok" ? { kind: "idle" } : s));
    },
    [],
  );

  // --- drag & drop (native HTML5) -------------------------------------------
  const onPaletteDragStart = (kindOfFeature: FeatureKind) => (e: React.DragEvent): void => {
    e.dataTransfer.setData("text/plain", `palette:${kindOfFeature}`);
    e.dataTransfer.effectAllowed = "copy";
  };
  const onCardDragStart = (index: number) => (e: React.DragEvent): void => {
    e.dataTransfer.setData("text/plain", `card:${index}`);
    e.dataTransfer.effectAllowed = "move";
    setDragCardIndex(index);
  };
  const onZoneDragOver = (targetIndex: number) => (e: React.DragEvent): void => {
    if (readOnlyPipeline) return;
    e.preventDefault();
    setDragOverIndex(targetIndex);
  };
  const clearDrag = (): void => {
    setDragOverIndex(null);
    setDragCardIndex(null);
  };
  // Precise child zones (`dropzone-N`, cards) stop propagation so a drop that lands
  // on them does NOT also reach the canvas-level append handler below.
  const onZoneDrop = (targetIndex: number) => (e: React.DragEvent): void => {
    if (readOnlyPipeline) return;
    e.preventDefault();
    e.stopPropagation();
    const payload = e.dataTransfer.getData("text/plain");
    clearDrag();
    if (payload.startsWith("palette:")) {
      const kindOfFeature = payload.slice("palette:".length) as FeatureKind;
      insertFeature(defaultFeatureSpec(kindOfFeature), targetIndex);
      setExpandedFeature(targetIndex);
    } else if (payload.startsWith("card:")) {
      const from = Number(payload.slice("card:".length));
      if (!Number.isNaN(from)) {
        const dest = from < targetIndex ? targetIndex - 1 : targetIndex;
        moveFeatureBy(from, dest);
      }
    }
  };

  // --- validation + save ----------------------------------------------------
  const nameErrors = useMemo(
    () => (draft ? validatePricingGroup(draft) : {}),
    [draft],
  );
  const espErrors = useMemo(
    () => (draft?.espPipeline ? validatePipeline(draft.espPipeline) : null),
    [draft],
  );
  const rfqErrors = useMemo(
    () => (draft && !draft.sharePipeline && draft.rfqPipeline ? validatePipeline(draft.rfqPipeline) : null),
    [draft],
  );
  const activeErrors: PipelineErrors =
    (effectiveMode === "ESP" ? espErrors : rfqErrors) ?? EMPTY_PIPELINE_ERRORS;

  const valid =
    draft !== null &&
    !hasPricingGroupErrors(nameErrors) &&
    !(espErrors !== null && hasPipelineErrors(espErrors)) &&
    !(rfqErrors !== null && hasPipelineErrors(rfqErrors));

  const dirty = useMemo(() => {
    if (draft === null) return false;
    if (creating) return true;
    return JSON.stringify(draft) !== JSON.stringify(selectedGroup);
  }, [draft, creating, selectedGroup]);

  const saving = saveState.kind === "saving";
  const canSave = signedIn && draft !== null && valid && dirty && !saving && (isAdmin || (canRetune && !creating));

  const save = useCallback(async (): Promise<void> => {
    if (draft === null || !valid) return;
    setSaveState({ kind: "saving" });
    try {
      if (creating) {
        const created = await app.transport.createPricingGroup(draft);
        await reloadGroups();
        setCreating(false);
        seededRef.current = null;
        setSelectedId(created.id);
        setDraft(cloneGroup(created));
        seededRef.current = created.id;
        setSaveState({ kind: "ok", name: created.name });
      } else if (isAdmin && selectedGroup) {
        const updated = await app.transport.updatePricingGroup(selectedGroup.id, draft);
        setGroups((prev) => prev.map((g) => (g.id === updated.id ? updated : g)));
        setDraft(cloneGroup(updated));
        setSaveState({ kind: "ok", name: updated.name });
      } else if (selectedGroup) {
        // Non-admin, pipeline-only: retune the effective mode's pipeline block.
        const updated = await app.transport.updatePricingGroupPipeline(
          selectedGroup.id,
          effectiveMode,
          activePipeline,
          draft.sharePipeline,
        );
        setGroups((prev) => prev.map((g) => (g.id === updated.id ? updated : g)));
        setDraft(cloneGroup(updated));
        setSaveState({ kind: "ok", name: updated.name });
      }
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save group" });
    }
  }, [
    app.transport,
    activePipeline,
    creating,
    draft,
    effectiveMode,
    isAdmin,
    reloadGroups,
    selectedGroup,
    valid,
  ]);

  const resetDraft = useCallback((): void => {
    if (creating) {
      setDraft(blankGroup());
    } else {
      setDraft(selectedGroup ? cloneGroup(selectedGroup) : null);
    }
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
  }, [creating, selectedGroup]);

  // --- preview waterfall ----------------------------------------------------
  const previewSteps: TwoWay[] = useMemo(
    () => previewPipeline(SAMPLE_RAW, activePipeline?.features ?? [], activePipeline?.guardrails ?? null),
    [activePipeline],
  );

  // --- sign-in gate ---------------------------------------------------------
  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Pricing groups</h2>
          <p className={styles.gateHint}>
            Sign in to build and assign pricing-group pipelines.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  const features = activePipeline?.features ?? [];

  return (
    <div className={styles.wrap}>
      <div className={styles.head}>
        <div className={styles.headMain}>
          <span className={styles.title}>Pricing Groups</span>
          <span className={styles.note}>
            Compose a pricing pipeline — RAW ▸ features ▸ OUTBOUND — for a group of FIX
            connections, users, and desks. Drag features from the palette onto the canvas,
            reorder and configure them, and watch the indicative two-way update after each step.
          </span>
        </div>
      </div>

      {readOnlyStructure && (
        <p className={styles.permBanner} role="note">
          <span className={styles.permGlyph} aria-hidden="true">
            🔒︎
          </span>
          You can retune pipelines but not change group structure. Structural edits require the{" "}
          <strong>Administer</strong> capability
          {canRetune ? (
            <span className={styles.permHint}> — pipeline retune uses {capabilityDenialTitle("quote_respond", "fixed_income")}.</span>
          ) : (
            <span className={styles.permHint}> ({capabilityDenialTitle("quote_respond", "fixed_income")}).</span>
          )}
        </p>
      )}

      {loadError && <p className={styles.banner}>{loadError}</p>}

      <div className={styles.body}>
        {/* LEFT — the group roster + create. */}
        <section className={styles.roster} aria-label="pricing groups">
          <div className={styles.rosterHead}>
            <h3 className={styles.rosterTitle}>Groups</h3>
            {isAdmin && (
              <Button variant="ghost" onClick={startCreate} data-tour-id="pg-new">
                + New
              </Button>
            )}
          </div>
          <ul className={styles.groupList}>
            {groups.map((g) => {
              const active = !creating && selectedId === g.id;
              const members =
                g.memberConnectionIds.length + g.memberUserIds.length + g.memberDesks.length;
              return (
                <li key={g.id}>
                  <button
                    type="button"
                    className={`${styles.groupBtn} ${active ? styles.groupBtnActive : ""}`}
                    aria-pressed={active}
                    onClick={() => selectGroup(g.id)}
                  >
                    <span className={styles.groupRow}>
                      <span className={styles.groupName}>{g.name}</span>
                      {!g.enabled && <span className={styles.groupOff}>off</span>}
                    </span>
                    <span className={styles.groupMeta}>
                      <span className={styles.metaPill}>
                        {members} member{members === 1 ? "" : "s"}
                      </span>
                      {g.espPipeline && <span className={styles.metaPill}>ESP</span>}
                      {(g.sharePipeline || g.rfqPipeline) && <span className={styles.metaPill}>RFQ</span>}
                    </span>
                  </button>
                </li>
              );
            })}
            {groups.length === 0 && (
              <li className={styles.empty}>No pricing groups yet. Create one to get started.</li>
            )}
          </ul>
        </section>

        {/* RIGHT — the group + pipeline editor. */}
        <section className={styles.editorPane} aria-label="pricing group editor">
          {draft === null ? (
            <Panel title="Pricing group" glyph="⚙">
              <div className={styles.empty}>Select a group, or create a new one.</div>
            </Panel>
          ) : (
            <Panel
              title={creating ? "New pricing group" : draft.name || draft.id}
              glyph="⚙"
              actions={
                saveState.kind === "ok" ? (
                  <span className={styles.okBadge} role="status" aria-live="polite">
                    ✓ Saved
                  </span>
                ) : !creating ? (
                  <span className={styles.editorId}>{draft.id}</span>
                ) : undefined
              }
            >
              <div className={styles.section}>
                {/* Structural fields. */}
                <div className={styles.formGrid}>
                  <label className={styles.field} htmlFor="pg-name">
                    <span className={styles.fieldLabel}>Name</span>
                    <input
                      id="pg-name"
                      className={`${styles.input} ${nameErrors.name ? styles.inputError : ""}`}
                      value={draft.name}
                      disabled={readOnlyStructure}
                      aria-invalid={nameErrors.name ? true : undefined}
                      onChange={(e) => patchDraft({ name: e.target.value })}
                    />
                    {nameErrors.name && <span className={styles.error}>{nameErrors.name}</span>}
                  </label>
                  <label className={styles.field} htmlFor="pg-id">
                    <span className={styles.fieldLabel}>Id {creating ? "(optional slug)" : ""}</span>
                    <input
                      id="pg-id"
                      className={`${styles.input} ${styles.numInput}`}
                      value={draft.id}
                      placeholder={creating ? "minted from name if blank" : ""}
                      disabled={readOnlyStructure || !creating}
                      onChange={(e) => patchDraft({ id: e.target.value })}
                    />
                  </label>
                  <label className={`${styles.field} ${styles.fieldWide}`} htmlFor="pg-desc">
                    <span className={styles.fieldLabel}>Description</span>
                    <textarea
                      id="pg-desc"
                      className={styles.textarea}
                      value={draft.description}
                      disabled={readOnlyStructure}
                      onChange={(e) => patchDraft({ description: e.target.value })}
                    />
                  </label>
                  <label className={styles.checkboxRow} htmlFor="pg-enabled">
                    <input
                      id="pg-enabled"
                      type="checkbox"
                      checked={draft.enabled}
                      disabled={readOnlyStructure}
                      onChange={(e) => patchDraft({ enabled: e.target.checked })}
                    />
                    <span>Enabled — a disabled group prices nobody</span>
                  </label>
                </div>
              </div>

              {/* Pipeline mode bar + share switch. */}
              <div className={styles.section}>
                <div className={styles.modeBar}>
                  <div className={styles.modeTabs} role="tablist" aria-label="pricing mode">
                    {PRICING_MODES.map((m) => {
                      const disabled = m === "RFQ" && draft.sharePipeline;
                      return (
                        <button
                          key={m}
                          type="button"
                          role="tab"
                          aria-selected={mode === m}
                          className={`${styles.modeTab} ${mode === m ? styles.modeTabActive : ""}`}
                          disabled={disabled}
                          title={disabled ? "RFQ mirrors ESP while sharing is on" : undefined}
                          onClick={() => {
                            setMode(m);
                            setExpandedFeature(null);
                          }}
                        >
                          {PRICING_MODE_LABEL[m]}
                        </button>
                      );
                    })}
                  </div>
                  <label className={styles.shareRow} htmlFor="pg-share">
                    <input
                      id="pg-share"
                      type="checkbox"
                      checked={draft.sharePipeline}
                      disabled={readOnlyPipeline}
                      onChange={(e) => {
                        patchDraft({ sharePipeline: e.target.checked });
                        if (e.target.checked) setMode("ESP");
                        setExpandedFeature(null);
                      }}
                    />
                    <span>Share ESP pipeline with RFQ</span>
                  </label>
                </div>
                {draft.sharePipeline && mode === "RFQ" && (
                  <p className={styles.modeNote}>
                    RFQ mirrors the ESP pipeline while sharing is on — editing the ESP pipeline below.
                  </p>
                )}

                {/* Per-mode custom-pipeline toggle. */}
                <label className={styles.checkboxRow} htmlFor="pg-custom">
                  <input
                    id="pg-custom"
                    type="checkbox"
                    checked={activePipeline !== null}
                    disabled={readOnlyPipeline}
                    onChange={(e) => toggleCustomPipeline(e.target.checked)}
                  />
                  <span>
                    Custom {effectiveMode} pipeline{" "}
                    <span className={styles.permHint}>(off ⇒ this mode uses the book-default pricing)</span>
                  </span>
                </label>
              </div>

              {activePipeline !== null && (
                <>
                  {/* Feature palette. */}
                  <div className={styles.section} data-tour-id="pg-palette">
                    <h4 className={styles.sectionHead}>Feature palette</h4>
                    <div className={styles.palette}>
                      <div className={styles.paletteChips} role="list" aria-label="feature palette">
                        {FEATURE_KINDS.map((k) => (
                          <div
                            key={k}
                            role="listitem"
                            className={styles.chip}
                            draggable={!readOnlyPipeline}
                            onDragStart={onPaletteDragStart(k)}
                            onDragEnd={clearDrag}
                            title={FEATURE_KIND_HINT[k]}
                            data-testid={`palette-${k}`}
                          >
                            <span className={styles.chipGlyph} aria-hidden="true">
                              ⠿
                            </span>
                            <span>{FEATURE_KIND_LABEL[k]}</span>
                            <button
                              type="button"
                              className={styles.chipAdd}
                              disabled={readOnlyPipeline}
                              aria-label={`Add ${FEATURE_KIND_LABEL[k]} to the end of the pipeline`}
                              onClick={() => {
                                insertFeature(defaultFeatureSpec(k), features.length);
                                setExpandedFeature(features.length);
                              }}
                            >
                              +
                            </button>
                          </div>
                        ))}
                      </div>
                    </div>
                  </div>

                  {/* Pipeline canvas. */}
                  <div className={styles.section}>
                    <h4 className={styles.sectionHead}>Pipeline</h4>
                    <div
                      className={styles.canvas}
                      aria-label="pricing pipeline canvas"
                      data-testid="pipeline-canvas"
                      onDragOver={onZoneDragOver(features.length)}
                      onDrop={onZoneDrop(features.length)}
                    >
                      <span className={styles.terminal}>RAW</span>
                      <span className={styles.spine} aria-hidden="true" />
                      <div
                        className={`${styles.dropZone} ${dragOverIndex === 0 ? styles.dropZoneActive : ""}`}
                        onDragOver={onZoneDragOver(0)}
                        onDrop={onZoneDrop(0)}
                        data-testid="dropzone-0"
                      />
                      {features.length === 0 && (
                        <div className={styles.canvasEmpty}>
                          Drag a feature here (or use a palette + button) to start the pipeline.
                        </div>
                      )}
                      {features.map((f, i) => (
                        <div key={`${f.kind}-${i}`}>
                          <PricingFeatureCard
                            feature={f}
                            index={i}
                            count={features.length}
                            expanded={expandedFeature === i}
                            readOnly={readOnlyPipeline}
                            errors={activeErrors.features[i]}
                            idPrefix={`pg-f${i}`}
                            dragging={dragCardIndex === i}
                            dragOver={dragOverIndex === i}
                            onToggleExpand={() => setExpandedFeature((cur) => (cur === i ? null : i))}
                            onPatch={(next) => patchFeature(i, next)}
                            onRemove={() => removeFeature(i)}
                            onMoveUp={() => moveFeatureBy(i, i - 1)}
                            onMoveDown={() => moveFeatureBy(i, i + 1)}
                            onDragStart={onCardDragStart(i)}
                            onDragEnd={clearDrag}
                            onDragOver={onZoneDragOver(i)}
                            onDrop={onZoneDrop(i)}
                          />
                          <div
                            className={`${styles.dropZone} ${dragOverIndex === i + 1 ? styles.dropZoneActive : ""}`}
                            onDragOver={onZoneDragOver(i + 1)}
                            onDrop={onZoneDrop(i + 1)}
                            data-testid={`dropzone-${i + 1}`}
                          />
                        </div>
                      ))}
                      <span className={styles.spine} aria-hidden="true" />
                      <span className={`${styles.terminal} ${styles.terminalOut}`}>OUTBOUND</span>
                    </div>

                    {/* Pipeline guardrails. */}
                    {activePipeline.guardrails && (
                      <div className={styles.grid}>
                        <label className={styles.param} htmlFor="pg-g-hmin">
                          <span className={styles.paramLabel}>Guardrail h_min</span>
                          <input
                            id="pg-g-hmin"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.hMin ? styles.inputError : ""}`}
                            type="number"
                            step="any"
                            value={activePipeline.guardrails.hMin}
                            disabled={readOnlyPipeline}
                            onChange={(e) => patchGuardrails({ hMin: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.hMin && (
                            <span className={styles.error}>{activeErrors.guardrails.hMin}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-hmax">
                          <span className={styles.paramLabel}>Guardrail h_max</span>
                          <input
                            id="pg-g-hmax"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.hMax ? styles.inputError : ""}`}
                            type="number"
                            step="any"
                            value={activePipeline.guardrails.hMax}
                            disabled={readOnlyPipeline}
                            onChange={(e) => patchGuardrails({ hMax: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.hMax && (
                            <span className={styles.error}>{activeErrors.guardrails.hMax}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-smax">
                          <span className={styles.paramLabel}>Guardrail s_max</span>
                          <input
                            id="pg-g-smax"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.sMax ? styles.inputError : ""}`}
                            type="number"
                            step="any"
                            value={activePipeline.guardrails.sMax}
                            disabled={readOnlyPipeline}
                            onChange={(e) => patchGuardrails({ sMax: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.sMax && (
                            <span className={styles.error}>{activeErrors.guardrails.sMax}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-floor">
                          <span className={styles.paramLabel}>Guardrail spread_floor</span>
                          <input
                            id="pg-g-floor"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.spreadFloor ? styles.inputError : ""}`}
                            type="number"
                            step="any"
                            value={activePipeline.guardrails.spreadFloor}
                            disabled={readOnlyPipeline}
                            onChange={(e) => patchGuardrails({ spreadFloor: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.spreadFloor && (
                            <span className={styles.error}>{activeErrors.guardrails.spreadFloor}</span>
                          )}
                        </label>
                      </div>
                    )}
                  </div>

                  {/* Live preview waterfall. */}
                  <div className={styles.section}>
                    <div className={styles.preview} aria-label="pipeline price preview">
                      <div className={styles.previewHead}>
                        <span className={styles.previewTitle}>Live preview — {effectiveMode}</span>
                        <span className={styles.previewNote}>indicative · sample raw 99.50 / 99.60</span>
                      </div>
                      <div className={styles.previewTable} role="table" aria-label="pipeline price waterfall" data-testid="preview-waterfall">
                        <div className={`${styles.previewRow} ${styles.previewColHead}`} role="row">
                          <span role="columnheader">Stage</span>
                          <span className={styles.previewNum} role="columnheader">Bid</span>
                          <span className={styles.previewNum} role="columnheader">Offer</span>
                          <span className={styles.previewSpread} role="columnheader">Spread</span>
                        </div>
                        {previewSteps.map((tw, i) => {
                          const isRaw = i === 0;
                          const isOut = i === previewSteps.length - 1 && features.length > 0;
                          const label = isRaw
                            ? "RAW"
                            : `${i}. ${FEATURE_KIND_LABEL[features[i - 1]!.kind]}`;
                          const rowClass = [
                            styles.previewRow,
                            isRaw ? styles.previewRowRaw : "",
                            isOut ? styles.previewRowOut : "",
                          ]
                            .filter(Boolean)
                            .join(" ");
                          return (
                            <div key={i} className={rowClass} role="row" data-testid={`preview-row-${i}`}>
                              <span className={styles.previewLabel} role="cell">{label}</span>
                              <span className={styles.previewNum} role="cell">{fmt(tw.bid)}</span>
                              <span className={styles.previewNum} role="cell">{fmt(tw.offer)}</span>
                              <span className={styles.previewSpread} role="cell">{fmt(tw.offer - tw.bid)}</span>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                </>
              )}

              {/* Membership. */}
              <div className={styles.section}>
                <h4 className={styles.sectionHead}>Membership</h4>
                <div className={styles.memberGrid}>
                  <MemberList
                    heading="FIX connections"
                    items={connections.map((c) => ({ id: c.id, label: c.name }))}
                    selected={draft.memberConnectionIds}
                    readOnly={readOnlyStructure}
                    onToggle={(id) => toggleMember("conn", id)}
                  />
                  <MemberList
                    heading="Users"
                    items={users.map((u) => ({ id: u.id, label: u.displayName || u.email }))}
                    selected={draft.memberUserIds}
                    readOnly={readOnlyStructure}
                    onToggle={(id) => toggleMember("user", id)}
                  />
                  <MemberList
                    heading="Desks"
                    items={desks.map((d) => ({ id: d.id, label: d.name }))}
                    selected={draft.memberDesks}
                    readOnly={readOnlyStructure}
                    onToggle={(id) => toggleMember("desk", id)}
                  />
                </div>
              </div>

              {saveState.kind === "error" && (
                <p className={styles.banner} role="alert">
                  {saveState.message}
                </p>
              )}

              {/* Actions. */}
              <div className={styles.actions}>
                <Button
                  variant="primary"
                  onClick={() => void save()}
                  disabled={!canSave}
                  title={
                    !valid
                      ? "Fix the highlighted errors first"
                      : !dirty
                        ? "No changes to save"
                        : creating
                          ? "Create this pricing group"
                          : "Save this pricing group"
                  }
                >
                  {saving ? "Saving…" : creating ? "Create group" : "Save group"}
                </Button>
                <Button variant="ghost" onClick={resetDraft} disabled={!dirty || saving}>
                  Reset
                </Button>
                {isAdmin && !creating && selectedGroup && (
                  <>
                    <Button variant="ghost" onClick={startClone} disabled={saving}>
                      Clone
                    </Button>
                    <Button variant="ghost" onClick={() => void deleteSelected()} disabled={saving}>
                      Delete
                    </Button>
                  </>
                )}
                <span className={styles.dirtyHint} aria-live="polite">
                  {dirty ? "Unsaved changes" : "In sync"}
                </span>
              </div>
            </Panel>
          )}
        </section>
      </div>
    </div>
  );
}

interface MemberListProps {
  heading: string;
  items: readonly { id: string; label: string }[];
  selected: readonly string[];
  readOnly: boolean;
  onToggle: (id: string) => void;
}

function MemberList({ heading, items, selected, readOnly, onToggle }: MemberListProps): React.ReactElement {
  return (
    <div className={styles.memberGroup}>
      <span className={styles.memberGroupHead}>
        {heading} ({selected.length})
      </span>
      {items.length === 0 ? (
        <span className={styles.memberEmpty}>None available.</span>
      ) : (
        <ul className={styles.memberList}>
          {items.map((it) => (
            <li key={it.id}>
              <label className={styles.memberItem}>
                <input
                  type="checkbox"
                  checked={selected.includes(it.id)}
                  disabled={readOnly}
                  onChange={() => onToggle(it.id)}
                />
                <span>{it.label}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
