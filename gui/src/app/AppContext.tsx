/**
 * App-wide context: the active workspace, the active pair, the shared transport,
 * the streaming store, and the marked surface. One spatial model, not a sea of
 * MDI windows (GUI-DESIGN §2). All workspaces read this; the pair switcher and
 * command palette re-target it.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import type {
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  Instrument,
  MarkedSurface,
  SmileModel,
} from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { resolveTransport } from "../data/transportConfig";
import {
  brokerLadder,
  DEFAULT_CONVENTIONS,
  PAIRS,
  seedSubscriptions,
  type PairContext,
} from "../data/seed";
import { buildUniverse, pairId, type Universe } from "../lib/universe";
import { useStreamSession, type StreamApi } from "../hooks/useStreamSession";
import {
  currentLevel,
  FIRM_SCOPE_ROOT,
  INITIAL_SCOPE,
  scopeReducer,
  type ScopeGroupBy,
  type ScopeLevel,
  type ScopeNode,
  type ScopeState,
} from "../lib/scope";
import {
  decodeView,
  encodeView,
  loadSavedViews,
  storeSavedViews,
  type AnalyticsSelection,
  type SavedView,
  type ViewState,
} from "../lib/savedViews";
import type { Density } from "../design/density";

export type WorkspaceId = "ticket" | "stream" | "surface" | "risk" | "book";

// Re-export the scope vocabulary from its owning module so existing consumers
// (riskView, riskScope tests) import it from AppContext unchanged — the types now
// have ONE definition in `lib/scope.ts` (zero legacy: no parallel scope type).
export type { ScopeLevel, ScopeNode, ScopeGroupBy };
export { FIRM_SCOPE_ROOT };

export interface TicketSeed {
  pair: CcyPair;
  /** A structure name to preload into the ticket (from "Stream this"/"Add"). */
  label: string;
  expiryYears: number;
}

// --- scope (P0-6: entitlement-ready "what slice of the firm" seam) ----------

/**
 * The active scope: WHO is looking (principal) and WHAT slice (path + groupBy).
 * Today `principal` is the literal `"grant-all"` — the scope filters NOTHING, but
 * every data path flows through it so a real entitlement predicate slots in later
 * with zero rework. The root path is always `[{level:"firm"}]` meaning "all desks
 * · all books · all pairs"; drilling appends desk/book/pair crumbs. The path +
 * groupBy algebra lives in `lib/scope.ts` (the pure reducer); this wraps it with
 * the entitlement principal so downstream readers see one `ScopeContext`.
 */
export interface ScopeContext {
  /** Entitlement principal. `"grant-all"` today (no filtering); a real predicate later. */
  principal: "grant-all";
  /** The drill path, firm root first. The tail crumb is the current scope. */
  path: ScopeNode[];
  /** Secondary grouping for aggregated views. */
  groupBy: ScopeGroupBy;
}

/** The smile-model ids a saved view may carry (guards the URL-recall path). */
const SMILE_MODELS: readonly SmileModel[] = [
  "MARKET_HEDGE",
  "STOCHASTIC_VOL",
  "PARAMETRIC",
  "PARAMETRIC_SURFACE",
  "EXTENDED_SURFACE",
];

/** Type guard: is `s` a known smile model (so a recalled snapshot is honoured)? */
function isSmileModel(s: string): s is SmileModel {
  return (SMILE_MODELS as readonly string[]).includes(s);
}

/** A shared selection: the instrument under focus and a human label for it. */
export interface Selection {
  instrument: Instrument;
  label: string;
}

interface AppState {
  transport: CelnetTransport;
  conventions: Conventions;
  workspace: WorkspaceId;
  setWorkspace: (w: WorkspaceId) => void;
  pairCtx: PairContext;
  setPair: (pair: CcyPair) => void;
  pairs: PairContext[];
  /**
   * The pair-universe model (registry-ready) over the CURRENT seeded pairs:
   * classified into majors/crosses/EM buckets, grouped, and indexed for fuzzy
   * search. Honest scope: today's pairs only — a future pair-universe registry
   * (P1-10) feeds a larger list here with zero downstream rework.
   */
  universe: Universe;
  /** The user's favourite pair ids (persisted in-memory for the session). */
  favourites: ReadonlySet<string>;
  /** Toggle a pair's favourite status (keyed by `pairId`). */
  toggleFavourite: (pair: CcyPair) => void;
  /**
   * Recently-activated pair ids, most-recent first (driven by `setPair`).
   * Bounded; the active pair is excluded from the head so "recents" means
   * "where I was", not "where I am".
   */
  recents: readonly string[];
  /**
   * The scope/underlier switcher open state. GW1-S3 absorbed the four redundant
   * pair affordances (PairMenu · the "Pairs" button · the ⌘K pair list · PairStrip
   * · the UniverseNavigator overlay) into ONE breadcrumb-scope control whose
   * TERMINAL case is underlier selection — opening this switcher is the leaf drill
   * (a re-homed UniverseNavigator, no longer a parallel toolbar overlay). Bound to
   * ⌘P (distinct from ⌘K's command index) and to clicking the pair crumb.
   */
  scopeSwitcherOpen: boolean;
  setScopeSwitcherOpen: (open: boolean) => void;
  stream: StreamApi;
  surface: MarkedSurface | null;
  /**
   * Mark/publish the surface. With no argument it (re-)marks from the pair's live
   * broker ladder; given an explicit `ladder` it publishes the trader's EDITED
   * marks. `model` selects the calibration family the server marks under (defaults
   * to the active `surfaceModel`). Either way the edited quotes flow through the
   * same `MarkSurface` API the SDK and Excel use — the GUI never side-channels a
   * mark. Bumps `surfaceVersion`.
   */
  remarkSurface: (ladder?: BrokerQuoteSet[], model?: SmileModel) => Promise<void>;
  /** The smile-calibration model the surface is currently marked under. */
  surfaceModel: SmileModel;
  /** Select the smile model AND re-mark the live surface under it. */
  setSurfaceModel: (model: SmileModel) => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  /**
   * The active density (GW0 axis). Read-only VALUE provided by the boot wiring
   * (`App.tsx` owns the density hook) so components consume the value through
   * context WITHOUT importing the hook — the cascade-disjointness contract: only
   * the attribute + tokens move in CSS, and the single JS touch-point is the boot.
   */
  density: Density;
  /** Toggle the density axis (the boot-owned setter, threaded through context). */
  toggleDensity: () => void;
  /**
   * The active scope (P0-6). Entitlement-ready: `grant-all` today, so it filters
   * nothing, but data flows through it. Default path = `[Firm]`. The `groupBy` is
   * now REAL (driven by the scope reducer), no longer hardwired `"none"`.
   */
  scope: ScopeContext;
  /** Drill DOWN one ladder level, appending `label` as the new tail crumb. */
  drillScopeDown: (label: string) => void;
  /** Drill UP to the ancestor at `depth` crumbs (1 = firm root). */
  drillScopeUp: (depth: number) => void;
  /** Reset the scope to the firm root (and clear group-by). */
  resetScope: () => void;
  /** Pin the secondary group-by axis (order-independent; never touches the path). */
  setScopeGroupBy: (groupBy: ScopeGroupBy) => void;
  // --- saved views (GW1-S4) -------------------------------------------------
  /** The persisted named views (localStorage-mirrored). */
  savedViews: readonly SavedView[];
  /** Capture the current (workspace, scope, analytics) triple under a name. */
  saveView: (name: string) => void;
  /** Recall a saved view by id (restores workspace + scope + analytics). */
  recallView: (id: string) => void;
  /** Delete a saved view by id. */
  deleteView: (id: string) => void;
  /** The current reproducible view state (the triple a save/URL captures). */
  viewState: ViewState;
  /** Apply a decoded view state (used by the URL-recall path). */
  applyViewState: (state: ViewState) => void;
  /** The analytics selection the inspector strips capture into a saved view. */
  analytics: AnalyticsSelection;
  /** Merge a partial analytics selection (a lane setting its own axis). */
  setAnalytics: (patch: AnalyticsSelection) => void;
  /**
   * The shared selection (P0-5): the instrument the Risk workspace analyses,
   * driven by the Book/Ticket lanes. `null` until something is selected — Risk
   * falls back to its seeded default so first load still shows a structure.
   */
  selected: Selection | null;
  /** Set the shared selection without changing the active workspace. */
  setSelected: (sel: Selection | null) => void;
  /** Select an instrument AND jump to the Risk workspace (one-click drill-to-risk). */
  drillToRisk: (instrument: Instrument, label: string) => void;
}

const Ctx = createContext<AppState | null>(null);

export function useApp(): AppState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApp outside AppProvider");
  return v;
}

const NOOP = (): void => {};

export function AppProvider({
  children,
  // The density value + setter are OWNED by `App.tsx`'s boot hook (the one JS
  // touch-point for the cascade axis) and threaded in. They default to the
  // comfortable resting state + a no-op so a test/embedding can mount the provider
  // without re-owning the density hook — the provider itself never reads density
  // in JS (the cascade-disjointness contract).
  density = "comfortable",
  toggleDensity = NOOP,
}: {
  children: React.ReactNode;
  /** The active density, owned by `App.tsx`'s boot hook (the one JS touch-point). */
  density?: Density;
  /** Toggle the density axis (the boot-owned setter). */
  toggleDensity?: () => void;
}): React.ReactElement {
  // The transport is selected once at the app root: the LIVE WebSocket mirror
  // against celnet-server by default (every number is the server's), or the
  // explicit offline in-app mock when `?mock` is set (src/data/transportConfig.ts).
  // One contract, two transports.
  const transport = useMemo(() => resolveTransport().transport, []);
  const conventions = DEFAULT_CONVENTIONS;

  const [workspace, setWorkspace] = useState<WorkspaceId>("stream");
  const [pairIndex, setPairIndex] = useState(0);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [scopeSwitcherOpen, setScopeSwitcherOpen] = useState(false);
  // Favourites + recents are persisted in-memory for the session (no fake
  // backend store — an honest client-side preference until a server prefs
  // service exists). `recents` is most-recent-first, excluding the active pair.
  const [favourites, setFavourites] = useState<ReadonlySet<string>>(() => new Set());
  const [recents, setRecents] = useState<readonly string[]>([]);
  const [surface, setSurface] = useState<MarkedSurface | null>(null);
  // The smile-calibration model the surface is marked under (default = the desk's
  // market-hedge construction; the server's default when the field is absent).
  const [surfaceModel, setSurfaceModelState] = useState<SmileModel>("MARKET_HEDGE");
  // Scope (P0-6): the pure drill path + group-by state, owned by `lib/scope.ts`'s
  // reducer. Default = the firm root, no secondary grouping.
  const [scopeState, setScopeState] = useState<ScopeState>(INITIAL_SCOPE);
  // The analytics selection the inspector strips capture (per-lane axes). A flat
  // bag merged by `setAnalytics`; restored verbatim by a recalled/URL view.
  const [analytics, setAnalyticsState] = useState<AnalyticsSelection>({});
  // The persisted named views (localStorage-mirrored via savedViews.ts).
  const [savedViews, setSavedViews] = useState<readonly SavedView[]>(() => loadSavedViews());
  // Shared selection (P0-5): null until a lane selects/drills; Risk falls back to
  // its seeded default so first load still renders a structure.
  const [selected, setSelected] = useState<Selection | null>(null);

  const pairCtx = PAIRS[pairIndex]!;

  // The registry-ready universe over the current seeded pairs (pure; memoized).
  const universe = useMemo(() => buildUniverse(PAIRS), []);

  const seed = useMemo(
    () =>
      seedSubscriptions().map((s) => ({
        instrument: s.instrument,
        conventions,
        label: s.label,
      })),
    [conventions],
  );
  const stream = useStreamSession(transport, seed);

  const remarkSurface = useMemo(
    () => async (ladder?: BrokerQuoteSet[], model?: SmileModel) => {
      const marked = await transport.markSurface(
        pairCtx.pair,
        ladder ?? brokerLadder(pairCtx),
        conventions,
        model ?? surfaceModel,
      );
      setSurface(marked);
    },
    [transport, pairCtx, conventions, surfaceModel],
  );

  // Select a smile model AND immediately re-mark the live broker ladder under it,
  // so the displayed surface + its provenance reflect the chosen model. Routes
  // through the SAME MarkSurface API the SDK/Excel use (no GUI side-channel).
  const setSurfaceModel = useMemo(
    () => (model: SmileModel) => {
      setSurfaceModelState(model);
      void transport
        .markSurface(pairCtx.pair, brokerLadder(pairCtx), conventions, model)
        .then(setSurface)
        .catch(() => {
          // A transient mark failure leaves the prior surface in place (honest);
          // the cold-start retry / a manual re-mark recovers it.
        });
    },
    [transport, pairCtx, conventions],
  );

  // Cold-start surface load with a BOUNDED retry. On a fresh edge the first mark
  // can fail transiently — the readiness gate replies `unavailable` while the
  // instance is still starting/draining, or the request times out before the
  // socket is up. Without a retry the Surface workspace would be stuck on its
  // "Marking surface…" state until the user manually re-marks. We retry with
  // capped exponential backoff for a bounded number of attempts; once a surface
  // lands (or the pair/transport changes, or we unmount) the loop stops. This is
  // recovery only — it never fabricates a surface; a genuine persistent failure
  // simply leaves the honest loading state and stops retrying.
  useEffect(() => {
    let live = true;
    let attempt = 0;
    const MAX_ATTEMPTS = 8;
    const BASE_MS = 300;
    const MAX_MS = 4_000;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tryMark = async (): Promise<void> => {
      if (!live) return;
      try {
        await remarkSurface();
        // Success: stop retrying (the surface state is set inside remarkSurface).
      } catch {
        attempt += 1;
        if (!live || attempt >= MAX_ATTEMPTS) return;
        const delay = Math.min(MAX_MS, BASE_MS * 2 ** (attempt - 1));
        timer = setTimeout(() => {
          void tryMark();
        }, delay);
      }
    };

    void tryMark();
    return () => {
      live = false;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [remarkSurface]);

  const setPair = (pair: CcyPair) => {
    const idx = PAIRS.findIndex((p) => p.pair.base === pair.base && p.pair.quote === pair.quote);
    if (idx < 0) return;
    if (idx === pairIndex) return;
    // Push the pair we are LEAVING onto recents (most-recent first, deduped,
    // bounded to 6) so "recents" reflects navigation history.
    const leaving = pairId(pairCtx.pair);
    setRecents((prev) => [leaving, ...prev.filter((id) => id !== leaving)].slice(0, 6));
    setPairIndex(idx);
    // FX terminal == active pair: when the scope is drilled to a pair crumb, keep
    // that crumb in lock-step with the active underlier (the round-trip invariant
    // the path algebra asserts). When the scope is above the pair level, leave the
    // path alone — selecting a pair from the watchlist doesn't force a drill.
    setScopeState((s) => {
      if (currentLevel(s) !== "pair") return s;
      const label = `${pair.base}/${pair.quote}`;
      const path = [...s.path];
      path[path.length - 1] = { level: "pair", label };
      return { path, groupBy: s.groupBy };
    });
  };

  const toggleFavourite = (pair: CcyPair) => {
    const id = pairId(pair);
    setFavourites((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // The active scope: the reducer's path + group-by, wrapped with the (grant-all)
  // entitlement principal. `groupBy` is now REAL — driven by the scope reducer.
  const scope: ScopeContext = useMemo(
    () => ({ principal: "grant-all", path: scopeState.path, groupBy: scopeState.groupBy }),
    [scopeState],
  );

  // Scope drill API — thin dispatchers over the pure `scopeReducer`.
  const drillScopeDown = useCallback(
    (label: string) => setScopeState((s) => scopeReducer(s, { type: "drillDown", label })),
    [],
  );
  const drillScopeUp = useCallback(
    (depth: number) => setScopeState((s) => scopeReducer(s, { type: "drillUp", depth })),
    [],
  );
  const resetScope = useCallback(() => setScopeState((s) => scopeReducer(s, { type: "reset" })), []);
  const setScopeGroupBy = useCallback(
    (groupBy: ScopeGroupBy) => setScopeState((s) => scopeReducer(s, { type: "setGroupBy", groupBy })),
    [],
  );

  const setAnalytics = useCallback(
    (patch: AnalyticsSelection) => setAnalyticsState((a) => ({ ...a, ...patch })),
    [],
  );

  // The reproducible view triple a save / URL captures: where + scope + analytics.
  // The active smile model is folded into the analytics snapshot so a saved
  // Surface/Cube view restores its calibration family too.
  const viewState: ViewState = useMemo(
    () => ({
      workspace,
      scope: scopeState,
      analytics: { ...analytics, model: surfaceModel },
    }),
    [workspace, scopeState, analytics, surfaceModel],
  );

  // Apply a decoded view state (URL recall / saved-view recall). Restores the
  // workspace, the scope path+group-by, and the analytics selection; if the
  // snapshot carried a model it re-selects it (which re-marks the live surface).
  const applyViewState = useCallback(
    (state: ViewState) => {
      setWorkspace(state.workspace);
      setScopeState(state.scope);
      const { model, ...rest } = state.analytics;
      setAnalyticsState(rest);
      if (model !== undefined && isSmileModel(model)) setSurfaceModelState(model);
    },
    [],
  );

  const persistViews = useCallback((views: SavedView[]) => {
    setSavedViews(views);
    storeSavedViews(views);
  }, []);

  const saveView = useCallback(
    (name: string) => {
      const trimmed = name.trim();
      if (trimmed.length === 0) return;
      const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
      const entry: SavedView = { id, name: trimmed, state: viewState };
      persistViews([...savedViews, entry]);
    },
    [viewState, savedViews, persistViews],
  );

  const recallView = useCallback(
    (id: string) => {
      const v = savedViews.find((s) => s.id === id);
      if (v) applyViewState(v.state);
    },
    [savedViews, applyViewState],
  );

  const deleteView = useCallback(
    (id: string) => persistViews(savedViews.filter((s) => s.id !== id)),
    [savedViews, persistViews],
  );

  // URL recall (GW1-S4): on first mount, if the URL carries any saved-view param,
  // restore that exact (workspace, scope, analytics) triple — a pasted link IS the
  // view. Runs once (a ref guards re-application on subsequent renders); the
  // transport params (mock/ws/transport) are untouched.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const hasViewParam =
      params.has("view") ||
      params.has("scope") ||
      params.has("group") ||
      params.has("model") ||
      params.has("meas") ||
      params.has("axes") ||
      params.has("trend");
    if (hasViewParam) applyViewState(decodeView(params));
    // Intentionally mount-only: later URL writes are driven by `viewState` below,
    // and re-decoding on every render would fight the user's live navigation.
    // `applyViewState` is a stable useCallback, so an empty dep list is correct.
  }, [applyViewState]);

  // Live URL mirror (GW1-S4): keep the address bar in sync with the current view
  // so a bookmark/copy captures the live state. We MERGE the view params over the
  // existing query (preserving transport params) and `replaceState` (no history
  // spam). The canonical view query is computed by the shared codec.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const current = new URLSearchParams(window.location.search);
    // Drop the prior view params, then write the fresh ones — so a field that is
    // no longer present (e.g. group-by relaxed to none) is removed from the URL.
    for (const key of ["view", "scope", "group", "model", "meas", "axes", "trend"]) {
      current.delete(key);
    }
    for (const [k, v] of encodeView(viewState)) current.set(k, v);
    const qs = current.toString();
    const next = `${window.location.pathname}${qs.length > 0 ? `?${qs}` : ""}${window.location.hash}`;
    window.history.replaceState(window.history.state, "", next);
  }, [viewState]);

  const drillToRisk = (instrument: Instrument, label: string) => {
    setSelected({ instrument, label });
    setWorkspace("risk");
  };

  const value: AppState = {
    transport,
    conventions,
    workspace,
    setWorkspace,
    pairCtx,
    setPair,
    pairs: PAIRS,
    universe,
    favourites,
    toggleFavourite,
    recents,
    scopeSwitcherOpen,
    setScopeSwitcherOpen,
    stream,
    surface,
    remarkSurface,
    surfaceModel,
    setSurfaceModel,
    paletteOpen,
    setPaletteOpen,
    density,
    toggleDensity,
    scope,
    drillScopeDown,
    drillScopeUp,
    resetScope,
    setScopeGroupBy,
    savedViews,
    saveView,
    recallView,
    deleteView,
    viewState,
    applyViewState,
    analytics,
    setAnalytics,
    selected,
    setSelected,
    drillToRisk,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
