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
  SettlementStyle,
  SmileModel,
  Underlying,
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
import { buildUniverse, pairId, pairLabel, type Universe } from "../lib/universe";
import {
  buildAssetUniverse,
  underlierAssetClass,
  type AssetUniverse,
  type UnderlierRow,
} from "../lib/assetUniverse";
import { ASSET_UNDERLIERS } from "../data/assetUniverse";
import type { AssetClass } from "../products/types";
import { useStreamSession, type StreamApi } from "../hooks/useStreamSession";
import { useAuth, type AuthApi } from "../hooks/useAuth";
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
import {
  domainAccessible,
  DOMAINS,
  firstAccessibleWorkspace,
  workspaceAccessible,
  workspaceDomains,
  type Domain,
} from "../lib/commands";
import type { Density } from "../design/density";

// fe-fi-migration #6: the single class-parametric WorkspaceId set (the FX/FI
// duplicate rows collapsed into one capability row each). Mirrors
// `commands.WorkspaceId` exactly.
export type WorkspaceId =
  | "ticket"
  | "stream"
  | "surface"
  | "risk"
  | "book"
  | "quoting"
  | "fistreaming"
  | "aggbook"
  | "xva"
  | "excel"
  | "connections"
  | "admin"
  | "permissions"
  | "refdata";

// Re-export the scope vocabulary from its owning module so existing consumers
// (riskView, riskScope tests) import it from AppContext unchanged — the types now
// have ONE definition in `lib/scope.ts` (zero legacy: no parallel scope type).
export type { ScopeLevel, ScopeNode, ScopeGroupBy };
export { FIRM_SCOPE_ROOT };

// --- active underlier (the cross-class terminal scope dimension) ------------

/**
 * The ACTIVE underlier the terminal scope crumb points at — the cross-class
 * generalisation of "the active pair". FX is the resting/default class (derived
 * from `pairCtx`, so every FX flow is byte-identical to before); a non-FX
 * selection in the universe leaf overlays this with the selected `Underlying`.
 * The FX `pairCtx` (market context, surface keying, streams) is NOT disturbed by
 * a non-FX selection — FX-keyed data paths stay honest (they never pretend to
 * cover a class they don't), and re-selecting an FX pair restores the overlay.
 */
export interface ActiveUnderlier {
  /** The asset class the underlier belongs to (drives class-aware workspaces). */
  assetClass: AssetClass;
  /** The contract identity (what a ticket books / a pre-target seeds). */
  underlying: Underlying;
  /**
   * The underlier's contract settlement mechanics (`Instrument.settlementStyle`):
   * INVERSE_COIN is meaningful only for a crypto underlier, LINEAR for every other
   * class. Carried here so the active cross-asset identity is COMPLETE — the
   * asset-class-agnostic ticket arms (perpetual / listed-future-option) read it to
   * carry the right `settlement_style` onto the wire, the cross-class twin of the
   * one-shot `ticketTarget.settlementStyle`. FX rests at LINEAR.
   */
  settlementStyle: SettlementStyle;
  /** The scope-crumb / display label ("EUR/USD", "XAU/USD", "AAPL", "BTC/USDT"). */
  label: string;
  /** Stable id (the projected pair id — one id space across classes). */
  id: string;
}

/**
 * A one-shot ticket pre-target armed by a non-FX underlier selection: the
 * TicketWorkspace consumes it (re-pointing itself at the cross-asset vanilla
 * spec seeded with exactly this `Underlying` + settlement mechanics) and clears
 * it. FX selections never arm it — the FX ticket flow is untouched.
 */
export interface TicketTarget {
  underlying: Underlying;
  settlementStyle: SettlementStyle;
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
  /**
   * Navigate to a workspace. Composes the raw setter with an activeDomain sync:
   * a jump to a SINGLE-domain workspace (e.g. ⌘-jump to `stream`/`quoting`, an
   * admin pane) re-homes {@link activeDomain} to that workspace's domain so the
   * tab stays honest; a jump to a SHARED workspace leaves activeDomain unchanged
   * (Model A — the tab keeps its lens). ALL callers (palette, ⌘N, drillToRisk)
   * route through this so navigation and the tab bar never disagree.
   */
  setWorkspace: (w: WorkspaceId) => void;
  /** The active top-level product domain (tab): FX Options / Fixed Income / admin. */
  activeDomain: Domain;
  /**
   * Select the active domain tab. For a SHARED screen this only flips the pane's
   * lens (Model A); the Shell decides whether to also navigate. Threaded into the
   * saved-view / URL codec so a deep-link restores the correct tab + lens.
   */
  setActiveDomain: (d: Domain) => void;
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
  /**
   * The NON-FX underlier universes (metals / equity / commodity / crypto) the
   * scope drill's asset-class rail navigates, built over the seeded
   * `ASSET_UNDERLIERS` (honest scope: today's seeded set — an estate feed drops
   * a larger list in with zero rework). The FX class stays `universe` above.
   */
  assetUniverse: AssetUniverse;
  /**
   * The active underlier (cross-class): FX = the active pair (default, derived
   * from `pairCtx`); non-FX = the last universe-leaf selection. Class-aware
   * workspaces (Surface family switch) read `assetClass` from here.
   */
  underlier: ActiveUnderlier;
  /**
   * Select a non-FX underlier from the universe leaf: overlays the active
   * underlier, keeps a terminal `pair` crumb in lock-step (the cross-class
   * twin of `setPair`'s invariant), and arms the one-shot ticket pre-target.
   */
  selectUnderlier: (row: UnderlierRow) => void;
  /** The armed ticket pre-target (non-FX selection), or `null`. */
  ticketTarget: TicketTarget | null;
  /** Consume the ticket pre-target (the TicketWorkspace clears it on apply). */
  clearTicketTarget: () => void;
  /** The user's favourite underlier ids (persisted in-memory for the session). */
  favourites: ReadonlySet<string>;
  /** Toggle a pair's favourite status (keyed by `pairId`). */
  toggleFavourite: (pair: CcyPair) => void;
  /** Toggle a favourite by its universe id (any class — pairs share the id space). */
  toggleFavouriteId: (id: string) => void;
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
  /**
   * The signed-in identity (server-enforced sessions). `auth.user` is `null` when
   * anonymous; signing in installs the bearer token on the transport so gated
   * RPCs authenticate and the FIX monitor narrows to the user's desk. Sign-in is
   * optional — the app runs anonymously until a user signs in.
   */
  auth: AuthApi;
  /** Whether the modal sign-in dialog is open (rendered once in the Shell). */
  signInOpen: boolean;
  setSignInOpen: (open: boolean) => void;
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

/**
 * The view-param keys the URL carries (the canonical saved-view wire form). Shared
 * by the boot-seed detector and the live-URL mirror so BOTH agree on which params
 * are "the view" — notably `dom` (the active product domain), so a domain-only
 * deep-link seeds on load AND a relaxed-to-default domain is dropped from the URL.
 */
const VIEW_PARAM_KEYS = ["view", "dom", "scope", "group", "model", "meas", "axes", "trend"] as const;

/**
 * Decode the BOOT view from the URL once (module read at first render): a
 * deep-link / saved-view link seeds the initial workspace + domain + scope +
 * analytics so the FIRST paint already matches the link — the domain tab bar shows
 * the URL's domain, and each shared screen (Risk / Market Data / Ticket) derives
 * its FX↔rates lens from that seeded domain on initial mount (not only after a
 * user tab-click). Returns `null` for a bare URL (boot the app defaults) and never
 * throws (the codec is forward-compatible). `dom` counts as a view param so a
 * domain-only link (`?dom=fixed_income`) seeds too.
 */
function bootViewState(): ViewState | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const hasViewParam = VIEW_PARAM_KEYS.some((k) => params.has(k));
  return hasViewParam ? decodeView(params) : null;
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
  transport: providedTransport,
}: {
  children: React.ReactNode;
  /** The active density, owned by `App.tsx`'s boot hook (the one JS touch-point). */
  density?: Density;
  /** Toggle the density axis (the boot-owned setter). */
  toggleDensity?: () => void;
  /**
   * The transport to run against. `App.tsx` resolves it ONCE and threads it in so
   * the same socket backs both the provider tree and the connection monitor that
   * drives the reconnect overlay. Defaults to `resolveTransport()` when omitted so
   * a test/embedding can still mount the provider standalone.
   */
  transport?: CelnetTransport;
}): React.ReactElement {
  // The transport is selected once at the app root: the LIVE WebSocket mirror
  // against celnet-server by default (every number is the server's), or the
  // explicit offline in-app mock when `?mock` is set (src/data/transportConfig.ts).
  // One contract, two transports. `App.tsx` threads in the resolved transport so it
  // is shared with the connection monitor; we fall back to resolving our own when
  // mounted standalone (tests/embeddings).
  const transport = useMemo(
    () => providedTransport ?? resolveTransport().transport,
    [providedTransport],
  );
  const conventions = DEFAULT_CONVENTIONS;

  // The boot view decoded from the URL ONCE (a deep-link / saved-view link). Seeds
  // the initial workspace + domain + scope + analytics below so the FIRST paint
  // already matches the link — the tab bar's active domain AND each shared screen's
  // domain-derived lens are correct on initial mount, with no post-mount re-home
  // flash. A bare URL yields `null` ⇒ the app boots its defaults.
  const bootView = useMemo(bootViewState, []);

  const [workspace, setWorkspaceRaw] = useState<WorkspaceId>(() => bootView?.workspace ?? "stream");
  // The active top-level domain tab. Seeded from the deep-link `dom` when present,
  // else `fx_options` to match the default `stream` workspace (an FX-only row).
  // Selected by the Shell tab bar; kept honest with the active workspace by
  // `navigate` below and re-homed by the gating effect if the identity lacks it.
  const [activeDomain, setActiveDomain] = useState<Domain>(() => bootView?.domain ?? "fx_options");
  const [pairIndex, setPairIndex] = useState(0);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [scopeSwitcherOpen, setScopeSwitcherOpen] = useState(false);
  const [signInOpen, setSignInOpen] = useState(false);
  // Favourites + recents are persisted in-memory for the session (no fake
  // backend store — an honest client-side preference until a server prefs
  // service exists). `recents` is most-recent-first, excluding the active pair.
  const [favourites, setFavourites] = useState<ReadonlySet<string>>(() => new Set());
  const [recents, setRecents] = useState<readonly string[]>([]);
  // The non-FX active-underlier OVERLAY: null = FX (the resting/default class,
  // derived from pairCtx below). Set by a universe-leaf non-FX selection; cleared
  // by any FX pair selection. The FX pairCtx is never disturbed.
  const [nonFxUnderlier, setNonFxUnderlier] = useState<ActiveUnderlier | null>(null);
  // The one-shot ticket pre-target a non-FX selection arms (consumed by the
  // TicketWorkspace, which seeds the cross-asset spec from it and clears it).
  const [ticketTarget, setTicketTarget] = useState<TicketTarget | null>(null);
  const [surface, setSurface] = useState<MarkedSurface | null>(null);
  // The smile-calibration model the surface is marked under (default = the desk's
  // market-hedge construction; the server's default when the field is absent). A
  // deep-link folds the model into `analytics.model`, so seed it from there.
  const [surfaceModel, setSurfaceModelState] = useState<SmileModel>(() => {
    const m = bootView?.analytics.model;
    return m !== undefined && isSmileModel(m) ? m : "MARKET_HEDGE";
  });
  // Scope (P0-6): the pure drill path + group-by state, owned by `lib/scope.ts`'s
  // reducer. Seeded from the deep-link scope, else the firm root, no grouping.
  const [scopeState, setScopeState] = useState<ScopeState>(() => bootView?.scope ?? INITIAL_SCOPE);
  // The analytics selection the inspector strips capture (per-lane axes). A flat
  // bag merged by `setAnalytics`; seeded from a deep-link's analytics (the model
  // lives in `surfaceModel` above, so it is stripped here — the two never diverge).
  const [analytics, setAnalyticsState] = useState<AnalyticsSelection>(() => {
    if (!bootView) return {};
    const { model: _model, ...rest } = bootView.analytics;
    return rest;
  });
  // The persisted named views (localStorage-mirrored via savedViews.ts).
  const [savedViews, setSavedViews] = useState<readonly SavedView[]>(() => loadSavedViews());
  // Shared selection (P0-5): null until a lane selects/drills; Risk falls back to
  // its seeded default so first load still renders a structure.
  const [selected, setSelected] = useState<Selection | null>(null);

  const pairCtx = PAIRS[pairIndex]!;

  // The registry-ready universe over the current seeded pairs (pure; memoized).
  const universe = useMemo(() => buildUniverse(PAIRS), []);
  // The non-FX underlier universes (metals/equity/commodity/crypto) the scope
  // drill's asset-class rail navigates (pure; memoized; seeded — honest scope).
  const assetUniverse = useMemo(() => buildAssetUniverse(ASSET_UNDERLIERS), []);

  // The active underlier: the non-FX overlay when set, else the active FX pair.
  const underlier: ActiveUnderlier = useMemo(
    () =>
      nonFxUnderlier ?? {
        assetClass: "FX",
        underlying: { kind: "fx", fx: pairCtx.pair, settlementCcy: pairCtx.pair.quote },
        settlementStyle: "LINEAR",
        label: pairLabel(pairCtx.pair),
        id: pairId(pairCtx.pair),
      },
    [nonFxUnderlier, pairCtx.pair],
  );

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

  // The signed-in identity (server-enforced sessions). One shared instance so the
  // title-bar identity menu and the Admin workspace agree; installing the bearer
  // token on the transport flows it onto every gated RPC.
  const auth = useAuth(transport);

  // Navigate to a workspace, keeping the active DOMAIN tab honest (Model A): a jump
  // to a single-domain workspace (⌘-jump to `stream`/`quoting`, an admin pane)
  // re-homes activeDomain to that workspace's domain; a jump to a SHARED workspace
  // leaves activeDomain unchanged (the tab keeps its lens). Uses the FUNCTIONAL
  // updater so it composes correctly when a caller (the Shell tab bar) already
  // queued a `setActiveDomain(d)` in the same event — the updater observes that
  // just-set domain, not a stale closure value.
  const navigate = useCallback((w: WorkspaceId) => {
    setWorkspaceRaw(w);
    setActiveDomain((cur) => {
      const doms = workspaceDomains(w);
      return doms.includes(cur) ? cur : doms[0]!;
    });
  }, []);

  // Enforce navigation gating: keep BOTH the active workspace AND the active domain
  // tab accessible to this identity. If either is inaccessible — an admin pane / tab
  // for a non-admin, or a trading workspace/tab in a class the user lacks `view` on
  // (e.g. an FX-only trader re-logging in while parked on the FX tab) — re-home:
  //   • the DOMAIN tab: keep activeDomain if its tab is accessible, else pick the
  //     first accessible domain (DOMAINS order) so the user never sits on a hidden
  //     tab (an FI-only trader lands on the Fixed Income tab, not a dead FX one);
  //   • the WORKSPACE: the first accessible workspace WITHIN that landing domain
  //     (falling back to the global first accessible when the domain has none).
  // Covers every entry path: a recalled/URL saved view, a ⌘-jump, the command
  // palette, or losing access while parked. Signed out, `can` is permissive ⇒ both
  // trading tabs + every trading workspace are accessible, so the anonymous default
  // is untouched. The degenerate all-inaccessible case leaves state as-is (no thrash).
  useEffect(() => {
    const domainOk = domainAccessible(activeDomain, auth);
    const wsOk = workspaceAccessible(workspace, auth);
    if (domainOk && wsOk) return;
    const landingDomain: Domain | undefined = domainOk
      ? activeDomain
      : DOMAINS.map((d) => d.id).find((d) => domainAccessible(d, auth));
    const target =
      landingDomain !== undefined
        ? firstAccessibleWorkspace(auth, landingDomain)
        : firstAccessibleWorkspace(auth);
    if (landingDomain !== undefined && landingDomain !== activeDomain) {
      setActiveDomain(landingDomain);
    }
    if (target && target !== workspace) setWorkspaceRaw(target);
  }, [auth.isAdmin, auth.can, workspace, activeDomain]);

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
    // Any FX pair selection re-targets the active underlier back to FX (clears a
    // non-FX overlay). Purely additive: a no-op in pure-FX flows.
    setNonFxUnderlier(null);
    if (idx === pairIndex) {
      // Same FX pair re-selected — possibly RETURNING from a non-FX underlier,
      // whose label sits on the terminal crumb: re-sync it to the pair label.
      // (Identical-label relabels return the same state — zero extra renders in
      // the pure-FX path, preserving the prior behavior exactly.)
      setScopeState((s) => {
        if (currentLevel(s) !== "pair") return s;
        const label = pairLabel(pair);
        if (s.path[s.path.length - 1]!.label === label) return s;
        const path = [...s.path];
        path[path.length - 1] = { level: "pair", label };
        return { path, groupBy: s.groupBy };
      });
      return;
    }
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

  // Favourites are keyed by the universe id — pairs and non-FX underliers share
  // one id space (the pair projection), so ONE favourites affordance covers every
  // class (extension, not a fork).
  const toggleFavouriteId = useCallback((id: string) => {
    setFavourites((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleFavourite = (pair: CcyPair) => toggleFavouriteId(pairId(pair));

  // Select a non-FX underlier (the universe leaf's cross-class twin of setPair):
  // overlay the active underlier, keep a terminal `pair` crumb in lock-step (the
  // same invariant setPair maintains for FX), and arm the one-shot ticket
  // pre-target with the exact contract identity + settlement mechanics.
  const selectUnderlier = useCallback((row: UnderlierRow) => {
    setNonFxUnderlier({
      assetClass: underlierAssetClass(row.underlying),
      underlying: row.underlying,
      settlementStyle: row.settlementStyle,
      label: row.label,
      id: row.id,
    });
    setTicketTarget({ underlying: row.underlying, settlementStyle: row.settlementStyle });
    setScopeState((s) => {
      if (currentLevel(s) !== "pair") return s;
      if (s.path[s.path.length - 1]!.label === row.label) return s;
      const path = [...s.path];
      path[path.length - 1] = { level: "pair", label: row.label };
      return { path, groupBy: s.groupBy };
    });
  }, []);

  const clearTicketTarget = useCallback(() => setTicketTarget(null), []);

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
      domain: activeDomain,
      scope: scopeState,
      analytics: { ...analytics, model: surfaceModel },
    }),
    [workspace, activeDomain, scopeState, analytics, surfaceModel],
  );

  // Apply a decoded view state (URL recall / saved-view recall). Restores the
  // workspace, the scope path+group-by, and the analytics selection; if the
  // snapshot carried a model it re-selects it (which re-marks the live surface).
  const applyViewState = useCallback(
    (state: ViewState) => {
      setWorkspaceRaw(state.workspace);
      setActiveDomain(state.domain);
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

  // URL recall (GW1-S4) is done at INIT, not in a mount effect: `bootView` above
  // seeds the workspace + domain + scope + analytics + smile model straight into
  // the useState initializers, so the FIRST render already matches a pasted link —
  // the domain tab and each shared screen's domain-derived lens are correct on the
  // initial paint with no post-mount re-home flash. A saved-view recall after mount
  // still routes through `applyViewState` (see `recallView`). The transport params
  // (mock/ws/transport) are untouched by both paths.

  // Live URL mirror (GW1-S4): keep the address bar in sync with the current view
  // so a bookmark/copy captures the live state. We MERGE the view params over the
  // existing query (preserving transport params) and `replaceState` (no history
  // spam). The canonical view query is computed by the shared codec.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const current = new URLSearchParams(window.location.search);
    // Drop the prior view params, then write the fresh ones — so a field that is
    // no longer present (e.g. group-by relaxed to none, or the domain relaxed back
    // to the fx_options default which emits no `dom`) is removed from the URL.
    for (const key of VIEW_PARAM_KEYS) {
      current.delete(key);
    }
    for (const [k, v] of encodeView(viewState)) current.set(k, v);
    const qs = current.toString();
    const next = `${window.location.pathname}${qs.length > 0 ? `?${qs}` : ""}${window.location.hash}`;
    window.history.replaceState(window.history.state, "", next);
  }, [viewState]);

  const drillToRisk = (instrument: Instrument, label: string) => {
    setSelected({ instrument, label });
    // Risk is a SHARED screen ⇒ navigate leaves activeDomain unchanged (the FX/FI
    // lens carries over); it only re-homes the tab if drilling in from admin.
    navigate("risk");
  };

  const value: AppState = {
    transport,
    conventions,
    workspace,
    setWorkspace: navigate,
    activeDomain,
    setActiveDomain,
    pairCtx,
    setPair,
    pairs: PAIRS,
    universe,
    assetUniverse,
    underlier,
    selectUnderlier,
    ticketTarget,
    clearTicketTarget,
    favourites,
    toggleFavourite,
    toggleFavouriteId,
    recents,
    scopeSwitcherOpen,
    setScopeSwitcherOpen,
    auth,
    signInOpen,
    setSignInOpen,
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
