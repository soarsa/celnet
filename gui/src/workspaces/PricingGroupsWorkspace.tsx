/**
 * PricingGroupsWorkspace — the consolidated fixed-income CLIENT-PRICING surface: a
 * tabbed shell over TWO sibling views that were previously two separate rail
 * destinations (mirroring the "Risk Portfolios → Risk Dashboard" tab-merge in {@link
 * RiskDashboardWorkspace}):
 *   • **Pricing Groups** (default) — the drag-and-drop feature-pipeline builder
 *     ({@link PricingGroupsPanel}, this file's original body, extracted VERBATIM).
 *   • **Tiering** — the session→pricing-group roster ({@link TieringWorkspace},
 *     composed WHOLESALE as a tab panel), formerly the separate "Tiering" rail row,
 *     now folded in one tab away.
 * Both tabs share the SAME rail gate (`manage_pricing·fixed_income`), so anyone who
 * can reach this workspace can view/edit both — the merge changes no authorization.
 * The standalone "Tiering" rail entry is removed (`lib/commands.ts`); the `tiering`
 * workspace id now deep-links straight to the Tiering tab (`app/Shell.tsx`), gating
 * identically via the `tiering → pricinggroups` consolidated alias.
 *
 * ---
 *
 * The **Pricing Groups** tab ({@link PricingGroupsPanel}) is the admin drag-and-drop
 * pricing-pipeline builder (docs/FI-PRICING-GROUPS-DESIGN.md §6 + §8.5, server commit
 * 07fc99f).
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
 * Gating: a Fixed-Income CLIENT-PRICING surface (moved OFF Administration onto the FI
 * tab). Rail visibility AND all edits gate on the granular `manage_pricing·fixed_income`
 * capability (docs/PERMISSIONS-GRANULAR-REVIEW.md §4 — the FI pricing-desk authority,
 * distinct from super-admin): a pricing manager creates / updates the whole group
 * (structure + both pipelines) WITHOUT full Administer, and a user lacking the cap never
 * reaches the pane. (The prior split — an `Administer` structure gate plus a
 * `quote_respond` pipeline-only retune path — is retired; the pane's single gate is
 * `manage_pricing`. The server still exposes `UpdatePricingGroupPipeline` on
 * `quote_respond` for non-GUI clients, but the GUI edits via the full update.)
 */

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { useApp } from "../app/AppContext";
import { useHedgeSeed } from "../app/HedgeSeedContext";
import { useTour } from "../app/TourProvider";
import { Button } from "../components/Button";
import { HelpButton } from "../components/HelpButton";
import {
  PricingGroupRowMenu,
  type PricingGroupMenuTarget,
} from "../components/PricingGroupRowMenu";
import { hedgeSeedFromPricingGroup } from "../lib/hedgeSeed";
import type {
  DeskDesc,
  FeatureKind,
  FeaturePipeline,
  FeatureSpec,
  FixConnection,
  LastLookMode,
  PricingGroup,
  PricingMode,
  PricingSourceMode,
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
  PRICING_SOURCE_MODE_HINT,
  PRICING_SOURCE_MODE_LABEL,
  PRICING_SOURCE_MODES,
  DEFAULT_BOOK_SKEW_WEIGHT,
  CURVE_ANCHORED_BOOK_SKEW_MODE,
  LAST_LOOK_MODE_HINT,
  LAST_LOOK_MODE_LABEL,
  LAST_LOOK_MODES,
  DEFAULT_LAST_LOOK_TOLERANCE_BPS,
  DEFAULT_ASYNC_GIVEBACK_PCT,
  ASYNC_LAST_LOOK_MODE,
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
import { TieringWorkspace } from "./TieringWorkspace";

import { NumberField } from "../components/NumberField";

const PRICING_MODES: readonly PricingMode[] = ["ESP", "RFQ"];
const PRICING_MODE_LABEL: Record<PricingMode, string> = { ESP: "ESP (streaming)", RFQ: "RFQ / RFS" };
const EMPTY_PIPELINE_ERRORS: PipelineErrors = { features: {}, guardrails: {} };

/** Interactive elements eligible for the editor modal's focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

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
    pricingSourceMode: 0,
    bookSkewWeight: null,
    lastLookMode: 0,
    lastLookToleranceBps: null,
    asyncGivebackPct: null,
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

/** The tab the consolidated Pricing surface shows: the pipeline builder or the
 * session-tiering roster. `groups` is the default; `tiering` is the deep-link target
 * for the retired "Tiering" rail entry. */
export type PricingTab = "groups" | "tiering";

/** One row per tab the consolidated Pricing surface spans: its id + toggle label. */
const PRICING_TABS: readonly { tab: PricingTab; label: string }[] = [
  { tab: "groups", label: "Pricing Groups" },
  { tab: "tiering", label: "Tiering" },
];

/**
 * PricingGroupsWorkspace — the tabbed shell composing the {@link PricingGroupsPanel}
 * builder and the {@link TieringWorkspace} roster as sibling tabs (see the file
 * header). Reuses the Risk Dashboard "Book → Risk" tab primitive VERBATIM: a slim
 * segmented bar above the active panel, which fills the remaining pane height and
 * scrolls its own content (the Shell pane is overflow:hidden with a definite height).
 * Only the active tab's body mounts, so each panel's effects (the builder's roster
 * loads, the Tiering session/group loads) fire only while it is on screen. The
 * Tiering tab's "Edit in Pricing Groups" affordance switches to the Groups tab in
 * place via {@link onEditInPricingGroups}.
 */
export function PricingGroupsWorkspace({
  initialTab = "groups",
}: {
  /** The initial tab — the retired `tiering` deep-link opens on `tiering`; the rail's
   * Pricing entry (and stories/tests) default to `groups`. */
  initialTab?: PricingTab;
} = {}): React.ReactElement {
  const [tab, setTab] = useState<PricingTab>(initialTab);
  return (
    <div className={styles.shell}>
      <div className={styles.tabBar} role="group" aria-label="pricing view">
        {PRICING_TABS.map((t) => (
          <button
            key={t.tab}
            type="button"
            className={`${styles.tabBtn} ${tab === t.tab ? styles.tabBtnActive : ""}`}
            aria-pressed={tab === t.tab}
            data-testid={`pricing-tab-${t.tab}`}
            onClick={() => setTab(t.tab)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className={styles.tabPanel}>
        {tab === "groups" ? (
          <PricingGroupsPanel />
        ) : (
          <TieringWorkspace onEditInPricingGroups={() => setTab("groups")} />
        )}
      </div>
    </div>
  );
}

/**
 * PricingGroupsPanel — the admin drag-and-drop pricing-pipeline builder (this
 * workspace's original body, extracted VERBATIM as the default "Pricing Groups"
 * tab). See the file header for the full builder description.
 */
function PricingGroupsPanel(): React.ReactElement {
  const app = useApp();
  const { activeTourId } = useTour();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // The single gate: the FI pricing-desk capability the server enforces on every
  // pricing-group mutation (admin holds it via grant-all). Rail visibility uses the
  // same cap, so anyone reaching the pane may edit; `can` is permissive signed-out.
  const canManagePricing = auth.can("manage_pricing", "fixed_income");
  const readOnly = !canManagePricing;
  // Seeding a hedge rule from a group is a HEDGE authority (`hedge` × FI — the SAME cap
  // the Deals-blotter "Change hedging strategy" and the Hedging surface gate on), NOT a
  // pricing authority: a hedge trader may create a rule from a group they can only view.
  const hedge = useHedgeSeed();
  const canHedge = auth.can("hedge", "fixed_income");
  // The roster right-click menu ("Create hedging rule"): the group + anchor point, or null.
  const [rowMenu, setRowMenu] = useState<PricingGroupMenuTarget | null>(null);

  // Seed a hedge exit-policy rule scoped from `g` (a `desk =` condition when the group has
  // exactly one member desk, else a no-condition draft), then deep-link to the Hedging
  // workspace — its Exit Policy tab (the default) consumes the seed and opens the draft.
  const createHedgeRuleFromGroup = useCallback(
    (g: PricingGroup): void => {
      hedge.requestHedgeSeedFromPricingGroup(hedgeSeedFromPricingGroup(g));
      app.setWorkspace("hedging");
    },
    [hedge, app],
  );

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

  // The editor is a dismissible portal modal over the group list (opened by "New"
  // or by selecting a group; closed by X / Esc / backdrop / Cancel / a successful save).
  const [editorOpen, setEditorOpen] = useState(false);
  const modalRef = useRef<HTMLDivElement | null>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  const titleId = useId();

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

  // Membership candidate rosters — fetched for a pricing manager (admin holds the cap
  // via grant-all). Denials on any individual roster RPC are swallowed by allSettled,
  // so a manager without the desk/connection admin cap just gets an empty picker.
  useEffect(() => {
    if (!canManagePricing) {
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
  }, [app.transport, canManagePricing]);

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
    setSaveState({ kind: "idle" });
    setEditorOpen(true);
  }, []);

  const startCreate = useCallback((): void => {
    setCreating(true);
    setSelectedId(null);
    setDraft(blankGroup());
    setMode("ESP");
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
    setEditorOpen(true);
  }, []);

  // Close the editor modal, discarding any unsaved edits (a create is dropped; an
  // edit reseeds from the stored group) — the app's "simple close" dialog behaviour.
  const closeEditor = useCallback((): void => {
    setEditorOpen(false);
    setExpandedFeature(null);
    setSaveState({ kind: "idle" });
    if (creating) {
      setCreating(false);
      setSelectedId(null);
      setDraft(null);
      seededRef.current = null;
    } else {
      setDraft(selectedGroup ? cloneGroup(selectedGroup) : null);
    }
  }, [creating, selectedGroup]);

  // Move focus into the modal on open (first editable field, else the close button)
  // and return it to the opener on close. Mirrors the app's dialog pattern.
  useEffect(() => {
    if (!editorOpen) return;
    openerRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const raf = requestAnimationFrame(() => {
      const modal = modalRef.current;
      const target =
        modal?.querySelector<HTMLElement>(
          "input:not([disabled]), select:not([disabled]), textarea:not([disabled])",
        ) ??
        closeBtnRef.current ??
        null;
      target?.focus();
    });
    return () => {
      cancelAnimationFrame(raf);
      openerRef.current?.focus();
    };
  }, [editorOpen]);

  // Esc closes the modal — but a running guided tour owns Esc (Skip) while it
  // spotlights the in-modal targets, so yield to it.
  useEffect(() => {
    if (!editorOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && activeTourId === null) {
        e.preventDefault();
        closeEditor();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editorOpen, activeTourId, closeEditor]);

  // Keep Tab focus cycling within the open modal (a lightweight focus trap).
  const onModalKeyDown = useCallback((e: React.KeyboardEvent): void => {
    if (e.key !== "Tab") return;
    const modal = modalRef.current;
    if (!modal) return;
    const focusables = Array.from(modal.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => el.offsetParent !== null || el === document.activeElement,
    );
    if (focusables.length === 0) return;
    const first = focusables[0] as HTMLElement;
    const last = focusables[focusables.length - 1] as HTMLElement;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
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
      setEditorOpen(false);
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
    if (readOnly) return;
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
    if (readOnly) return;
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
  const canSave = signedIn && draft !== null && valid && dirty && !saving && canManagePricing;

  const save = useCallback(async (): Promise<void> => {
    if (draft === null || !valid || !canManagePricing) return;
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
      } else if (selectedGroup) {
        // A Manage-Pricing edit is a FULL update (structure + both pipelines) via the
        // manage_pricing-gated RPC — the retired pipeline-only path is gone (§4).
        const updated = await app.transport.updatePricingGroup(selectedGroup.id, draft);
        setGroups((prev) => prev.map((g) => (g.id === updated.id ? updated : g)));
        setDraft(cloneGroup(updated));
        setSaveState({ kind: "ok", name: updated.name });
      }
      setEditorOpen(false); // close the modal on a successful create / save
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save group" });
    }
  }, [
    app.transport,
    canManagePricing,
    creating,
    draft,
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

      {readOnly && (
        <p className={styles.permBanner} role="note">
          <span className={styles.permGlyph} aria-hidden="true">
            🔒︎
          </span>
          Read-only — {capabilityDenialTitle("manage_pricing", "fixed_income")}
        </p>
      )}

      {loadError && <p className={styles.banner}>{loadError}</p>}

      <div className={styles.body}>
        {/* LEFT — the group roster + create. */}
        <section className={styles.roster} aria-label="pricing groups">
          <div className={styles.rosterHead}>
            <h3 className={styles.rosterTitle}>Groups</h3>
            {canManagePricing && (
              <Button variant="primary" onClick={startCreate} data-tour-id="pg-new">
                + New pricing group
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
                    aria-label={
                      canHedge
                        ? `${g.name} — click to edit; right-click or press the menu key to create a hedging rule`
                        : undefined
                    }
                    onClick={() => selectGroup(g.id)}
                    onContextMenu={(e) => {
                      // Right-click offers "Create hedging rule" — only when the viewer can
                      // author hedge policy; otherwise let the native menu stand.
                      if (!canHedge) return;
                      e.preventDefault();
                      setRowMenu({ groupId: g.id, name: g.name, x: e.clientX, y: e.clientY });
                    }}
                    onKeyDown={(e) => {
                      // Keyboard parity for the right-click: the context-menu key / Shift+F10.
                      if (!canHedge) return;
                      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
                        e.preventDefault();
                        const r = e.currentTarget.getBoundingClientRect();
                        setRowMenu({ groupId: g.id, name: g.name, x: r.left + 12, y: r.bottom - 8 });
                      }
                    }}
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

      </div>

      {/* Roster right-click menu — "Create hedging rule" from the picked group. */}
      <PricingGroupRowMenu
        target={rowMenu}
        onClose={() => setRowMenu(null)}
        onCreateHedgingRule={(groupId) => {
          const g = groups.find((x) => x.id === groupId);
          if (g) createHedgeRuleFromGroup(g);
        }}
      />

      {/* The group + pipeline editor — a dismissible portal modal over the list. */}
      {editorOpen &&
        draft !== null &&
        createPortal(
          <div
            className={styles.scrim}
            role="presentation"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) closeEditor();
            }}
          >
            <div
              ref={modalRef}
              className={styles.modal}
              role="dialog"
              aria-modal="true"
              aria-labelledby={titleId}
              data-testid="pricing-group-editor"
              onKeyDown={onModalKeyDown}
            >
              <div className={styles.modalHead}>
                <div className={styles.modalTitleWrap}>
                  <span className={styles.modalGlyph} aria-hidden="true">
                    ⚙
                  </span>
                  <h2 id={titleId} className={styles.modalTitle}>
                    {creating ? "New pricing group" : draft.name || draft.id}
                  </h2>
                  {saveState.kind === "ok" ? (
                    <span className={styles.okBadge} role="status" aria-live="polite">
                      ✓ Saved
                    </span>
                  ) : !creating ? (
                    <span className={styles.editorId}>{draft.id}</span>
                  ) : null}
                </div>
                <button
                  ref={closeBtnRef}
                  type="button"
                  className={styles.modalClose}
                  onClick={closeEditor}
                  aria-label="Close pricing group editor"
                >
                  <span aria-hidden="true">✕</span>
                </button>
              </div>

              <div className={styles.modalBody}>
                <div className={styles.section}>
                {/* Structural fields. */}
                <div className={styles.formGrid}>
                  <label className={styles.field} htmlFor="pg-name">
                    <span className={styles.fieldLabel}>Name</span>
                    <input
                      id="pg-name"
                      className={`${styles.input} ${nameErrors.name ? styles.inputError : ""}`}
                      value={draft.name}
                      disabled={readOnly}
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
                      disabled={readOnly || !creating}
                      onChange={(e) => patchDraft({ id: e.target.value })}
                    />
                  </label>
                  <label className={`${styles.field} ${styles.fieldWide}`} htmlFor="pg-desc">
                    <span className={styles.fieldLabel}>Description</span>
                    <textarea
                      id="pg-desc"
                      className={styles.textarea}
                      value={draft.description}
                      disabled={readOnly}
                      onChange={(e) => patchDraft({ description: e.target.value })}
                    />
                  </label>
                  <label className={styles.checkboxRow} htmlFor="pg-enabled">
                    <input
                      id="pg-enabled"
                      type="checkbox"
                      checked={draft.enabled}
                      disabled={readOnly}
                      onChange={(e) => patchDraft({ enabled: e.target.checked })}
                    />
                    <span>Enabled — a disabled group prices nobody</span>
                  </label>

                  {/*
                    Pricing-source policy: how rates/bond FIX auto-quotes source their
                    RAW price (persisted on the group spec; orthogonal to the feature
                    pipeline below). Switching away from the book-skew mode drops the
                    weight so it is OMITTED on write; switching TO it keeps `null` so the
                    server default (0.5) applies until the trader moves the slider.
                  */}
                  <label className={`${styles.field} ${styles.fieldWide}`} htmlFor="pg-source-mode">
                    <span className={styles.fieldLabel}>Pricing source</span>
                    <select
                      id="pg-source-mode"
                      className={styles.select}
                      value={draft.pricingSourceMode}
                      disabled={readOnly}
                      onChange={(e) => {
                        const next = Number(e.target.value) as PricingSourceMode;
                        patchDraft({
                          pricingSourceMode: next,
                          bookSkewWeight:
                            next === CURVE_ANCHORED_BOOK_SKEW_MODE ? draft.bookSkewWeight : null,
                        });
                      }}
                    >
                      {PRICING_SOURCE_MODES.map((m) => (
                        <option key={m} value={m}>
                          {PRICING_SOURCE_MODE_LABEL[m]}
                        </option>
                      ))}
                    </select>
                    <span className={styles.modeNote}>
                      {PRICING_SOURCE_MODE_HINT[draft.pricingSourceMode]}
                    </span>
                  </label>

                  {draft.pricingSourceMode === CURVE_ANCHORED_BOOK_SKEW_MODE && (
                    <label
                      className={`${styles.field} ${styles.fieldWide}`}
                      htmlFor="pg-skew-weight"
                    >
                      <span className={styles.fieldLabel}>
                        Book skew weight — {(draft.bookSkewWeight ?? DEFAULT_BOOK_SKEW_WEIGHT).toFixed(2)}
                      </span>
                      <div className={styles.skewRow}>
                        <input
                          id="pg-skew-weight"
                          className={styles.skewRange}
                          type="range"
                          min={0}
                          max={1}
                          step={0.05}
                          value={draft.bookSkewWeight ?? DEFAULT_BOOK_SKEW_WEIGHT}
                          disabled={readOnly}
                          aria-describedby="pg-skew-help"
                          onChange={(e) => patchDraft({ bookSkewWeight: Number(e.target.value) })}
                        />
                        <NumberField
                          className={`${styles.input} ${styles.numInput} ${styles.skewNum}`}
                          min={0}
                          max={1}
                          step={0.05}
                          value={draft.bookSkewWeight ?? DEFAULT_BOOK_SKEW_WEIGHT}
                          disabled={readOnly}
                          aria-label="Book skew weight"
                          onChange={(e) => patchDraft({ bookSkewWeight: Number(e.target.value) })}
                        />
                      </div>
                      <span className={styles.modeNote} id="pg-skew-help">
                        0 = pure curve, 1 = pure book, 0.5 = halfway. Left unset, the server
                        applies its 0.5 default.
                      </span>
                    </label>
                  )}

                  {/*
                    Last-look policy: governs a streamed-quote lift when the market
                    moved between quote and order (persisted on the group spec). The
                    TOLERANCE protects the desk (an adverse move beyond it is rejected);
                    the MODE decides who keeps a FAVORABLE move — Sync ⇒ the desk keeps
                    it all; Async ⇒ a configurable % is passed back to the client. The
                    giveback control is only meaningful (and only written) for Async;
                    both scalars stay `null` until the trader moves them, so the server
                    applies its defaults (1.0 bps / 50%).
                  */}
                  <label className={`${styles.field} ${styles.fieldWide}`} htmlFor="pg-last-look-mode">
                    <span className={styles.fieldLabel}>Last-look</span>
                    <select
                      id="pg-last-look-mode"
                      className={styles.select}
                      value={draft.lastLookMode}
                      disabled={readOnly}
                      onChange={(e) => {
                        const next = Number(e.target.value) as LastLookMode;
                        patchDraft({
                          lastLookMode: next,
                          asyncGivebackPct:
                            next === ASYNC_LAST_LOOK_MODE ? draft.asyncGivebackPct : null,
                        });
                      }}
                    >
                      {LAST_LOOK_MODES.map((m) => (
                        <option key={m} value={m}>
                          {LAST_LOOK_MODE_LABEL[m]}
                        </option>
                      ))}
                    </select>
                    <span className={styles.modeNote}>{LAST_LOOK_MODE_HINT[draft.lastLookMode]}</span>
                  </label>

                  <label className={`${styles.field} ${styles.fieldWide}`} htmlFor="pg-last-look-tol">
                    <span className={styles.fieldLabel}>Tolerance (bps)</span>
                    <NumberField
                      id="pg-last-look-tol"
                      className={`${styles.input} ${styles.numInput}`}
                      min={0}
                      step={0.1}
                      value={draft.lastLookToleranceBps ?? DEFAULT_LAST_LOOK_TOLERANCE_BPS}
                      disabled={readOnly}
                      aria-describedby="pg-last-look-tol-help"
                      onChange={(e) => patchDraft({ lastLookToleranceBps: Number(e.target.value) })}
                    />
                    <span className={styles.modeNote} id="pg-last-look-tol-help">
                      Reject a lift when the market moved against the desk by more than this
                      between quote and order. Left unset, the server applies its 1.0 default.
                    </span>
                  </label>

                  {draft.lastLookMode === ASYNC_LAST_LOOK_MODE && (
                    <label
                      className={`${styles.field} ${styles.fieldWide}`}
                      htmlFor="pg-async-giveback"
                    >
                      <span className={styles.fieldLabel}>
                        Async giveback % — {(draft.asyncGivebackPct ?? DEFAULT_ASYNC_GIVEBACK_PCT).toFixed(0)}
                      </span>
                      <div className={styles.skewRow}>
                        <input
                          id="pg-async-giveback"
                          className={styles.skewRange}
                          type="range"
                          min={0}
                          max={100}
                          step={5}
                          value={draft.asyncGivebackPct ?? DEFAULT_ASYNC_GIVEBACK_PCT}
                          disabled={readOnly}
                          aria-describedby="pg-async-giveback-help"
                          onChange={(e) => patchDraft({ asyncGivebackPct: Number(e.target.value) })}
                        />
                        <NumberField
                          className={`${styles.input} ${styles.numInput} ${styles.skewNum}`}
                          min={0}
                          max={100}
                          step={5}
                          value={draft.asyncGivebackPct ?? DEFAULT_ASYNC_GIVEBACK_PCT}
                          disabled={readOnly}
                          aria-label="Async giveback %"
                          onChange={(e) => patchDraft({ asyncGivebackPct: Number(e.target.value) })}
                        />
                      </div>
                      <span className={styles.modeNote} id="pg-async-giveback-help">
                        The share of a favorable move handed back to the client as price
                        improvement. Left unset, the server applies its 50% default.
                      </span>
                    </label>
                  )}
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
                      disabled={readOnly}
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
                    disabled={readOnly}
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
                    <h3 className={styles.sectionHead}>Feature palette</h3>
                    <div className={styles.palette}>
                      <div className={styles.paletteChips} role="list" aria-label="feature palette">
                        {FEATURE_KINDS.map((k) => (
                          <div
                            key={k}
                            role="listitem"
                            className={styles.chip}
                            draggable={!readOnly}
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
                              disabled={readOnly}
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
                    <h3 className={styles.sectionHead}>Pipeline</h3>
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
                            readOnly={readOnly}
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
                          <NumberField
                            id="pg-g-hmin"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.hMin ? styles.inputError : ""}`}
                            step="any"
                            value={activePipeline.guardrails.hMin}
                            disabled={readOnly}
                            onChange={(e) => patchGuardrails({ hMin: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.hMin && (
                            <span className={styles.error}>{activeErrors.guardrails.hMin}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-hmax">
                          <span className={styles.paramLabel}>Guardrail h_max</span>
                          <NumberField
                            id="pg-g-hmax"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.hMax ? styles.inputError : ""}`}
                            step="any"
                            value={activePipeline.guardrails.hMax}
                            disabled={readOnly}
                            onChange={(e) => patchGuardrails({ hMax: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.hMax && (
                            <span className={styles.error}>{activeErrors.guardrails.hMax}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-smax">
                          <span className={styles.paramLabel}>Guardrail s_max</span>
                          <NumberField
                            id="pg-g-smax"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.sMax ? styles.inputError : ""}`}
                            step="any"
                            value={activePipeline.guardrails.sMax}
                            disabled={readOnly}
                            onChange={(e) => patchGuardrails({ sMax: Number(e.target.value) })}
                          />
                          {activeErrors.guardrails.sMax && (
                            <span className={styles.error}>{activeErrors.guardrails.sMax}</span>
                          )}
                        </label>
                        <label className={styles.param} htmlFor="pg-g-floor">
                          <span className={styles.paramLabel}>Guardrail spread_floor</span>
                          <NumberField
                            id="pg-g-floor"
                            className={`${styles.input} ${styles.numInput} ${activeErrors.guardrails.spreadFloor ? styles.inputError : ""}`}
                            step="any"
                            value={activePipeline.guardrails.spreadFloor}
                            disabled={readOnly}
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
                <h3 className={styles.sectionHead}>Membership</h3>
                <div className={styles.memberGrid}>
                  <MemberList
                    heading="FIX connections"
                    items={connections.map((c) => ({ id: c.id, label: c.name }))}
                    selected={draft.memberConnectionIds}
                    readOnly={readOnly}
                    onToggle={(id) => toggleMember("conn", id)}
                  />
                  <MemberList
                    heading="Users"
                    items={users.map((u) => ({ id: u.id, label: u.displayName || u.email }))}
                    selected={draft.memberUserIds}
                    readOnly={readOnly}
                    onToggle={(id) => toggleMember("user", id)}
                  />
                  <MemberList
                    heading="Desks"
                    items={desks.map((d) => ({ id: d.id, label: d.name }))}
                    selected={draft.memberDesks}
                    readOnly={readOnly}
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
                <Button variant="ghost" onClick={closeEditor} disabled={saving}>
                  {creating ? "Cancel" : "Close"}
                </Button>
                {canManagePricing && !creating && selectedGroup && (
                  <>
                    <Button variant="ghost" onClick={startClone} disabled={saving}>
                      Clone
                    </Button>
                    <Button variant="ghost" onClick={() => void deleteSelected()} disabled={saving}>
                      Delete
                    </Button>
                  </>
                )}
                {/* HEDGE authority (not pricing): seed a hedge exit-policy rule from this
                    group and jump to the Hedging builder. Shown to a hedge trader even
                    when they can only VIEW the group (read-only pricing). */}
                {canHedge && !creating && selectedGroup && (
                  <span className={styles.hedgeAction}>
                    <Button
                      variant="ghost"
                      onClick={() => createHedgeRuleFromGroup(selectedGroup)}
                      disabled={saving}
                      title="Seed a hedge exit-policy rule from this pricing group and open the Hedging builder"
                    >
                      Create hedging rule
                    </Button>
                    <HelpButton
                      helpId="concept.hedge-rule-from-pricing-group"
                      subject="creating a hedging rule from this pricing group"
                    />
                  </span>
                )}
                <span className={styles.dirtyHint} aria-live="polite">
                  {dirty ? "Unsaved changes" : "In sync"}
                </span>
              </div>
            </div>
          </div>
        </div>,
        document.body,
      )}
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
