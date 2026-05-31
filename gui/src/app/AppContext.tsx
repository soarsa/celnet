/**
 * App-wide context: the active workspace, the active pair, the shared transport,
 * the streaming store, and the marked surface. One spatial model, not a sea of
 * MDI windows (GUI-DESIGN §2). All workspaces read this; the pair switcher and
 * command palette re-target it.
 */

import {
  createContext,
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
import { useStreamSession, type StreamApi } from "../hooks/useStreamSession";

export type WorkspaceId = "ticket" | "stream" | "surface" | "risk" | "book";

export interface TicketSeed {
  pair: CcyPair;
  /** A structure name to preload into the ticket (from "Stream this"/"Add"). */
  label: string;
  expiryYears: number;
}

// --- scope (P0-6: entitlement-ready "what slice of the firm" seam) ----------

/** The organizational level a scope crumb sits at, firm-down. */
export type ScopeLevel = "firm" | "desk" | "book" | "pair";

/** One node on the scope path (a breadcrumb crumb). */
export interface ScopeNode {
  level: ScopeLevel;
  label: string;
}

/** A secondary grouping dimension for scoped views (book/blotter aggregation). */
export type ScopeGroupBy = "none" | "desk" | "book" | "pair";

/**
 * The active scope: WHO is looking (principal) and WHAT slice (path + groupBy).
 * Today `principal` is the literal `"grant-all"` — the scope filters NOTHING, but
 * every data path flows through it so a real entitlement predicate slots in later
 * with zero rework. The root path is always `[{level:"firm"}]` meaning "all desks
 * · all books · all pairs"; drilling appends desk/book/pair crumbs.
 */
export interface ScopeContext {
  /** Entitlement principal. `"grant-all"` today (no filtering); a real predicate later. */
  principal: "grant-all";
  /** The drill path, firm root first. The tail crumb is the current scope. */
  path: ScopeNode[];
  /** Secondary grouping for aggregated views. */
  groupBy: ScopeGroupBy;
}

/** The firm root — "all desks · all books · all pairs". */
export const FIRM_SCOPE_ROOT: ScopeNode = { level: "firm", label: "Firm" };

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
   * The desk's OPEN POSITIONS for book-wide risk aggregation. These are the real
   * instruments behind the seeded streaming book (the same `seedSubscriptions()`
   * structures that populate the RFS blotter, spanning all pairs) — not invented
   * notionals. The BookWorkspace reprices each one via `transport.scenario` and
   * sums the result to a desk-wide view.
   */
  positions: Instrument[];
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
   * The active scope (P0-6). Entitlement-ready: `grant-all` today, so it filters
   * nothing, but data flows through it. Default path = `[Firm]`.
   */
  scope: ScopeContext;
  /** Set the scope drill path (e.g. truncate to an ancestor crumb). */
  setScopePath: (path: ScopeNode[]) => void;
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

export function AppProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  // The transport is selected once at the app root: the LIVE WebSocket mirror
  // against celnet-server by default (every number is the server's), or the
  // explicit offline in-app mock when `?mock` is set (src/data/transportConfig.ts).
  // One contract, two transports.
  const transport = useMemo(() => resolveTransport().transport, []);
  const conventions = DEFAULT_CONVENTIONS;

  const [workspace, setWorkspace] = useState<WorkspaceId>("stream");
  const [pairIndex, setPairIndex] = useState(0);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [surface, setSurface] = useState<MarkedSurface | null>(null);
  // The smile-calibration model the surface is marked under (default = the desk's
  // market-hedge construction; the server's default when the field is absent).
  const [surfaceModel, setSurfaceModelState] = useState<SmileModel>("MARKET_HEDGE");
  // Scope (P0-6): default to the firm root — "all desks · all books · all pairs".
  const [scopePath, setScopePath] = useState<ScopeNode[]>([FIRM_SCOPE_ROOT]);
  // Shared selection (P0-5): null until a lane selects/drills; Risk falls back to
  // its seeded default so first load still renders a structure.
  const [selected, setSelected] = useState<Selection | null>(null);

  const pairCtx = PAIRS[pairIndex]!;

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

  // The desk's open positions for book-wide risk: the exact instruments behind
  // the seeded streaming book (one per seed subscription), so the BookWorkspace
  // aggregates real exposures across all pairs rather than fabricated notionals.
  const positions = useMemo(() => seed.map((s) => s.instrument), [seed]);

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
    if (idx >= 0) setPairIndex(idx);
  };

  const scope: ScopeContext = useMemo(
    () => ({ principal: "grant-all", path: scopePath, groupBy: "none" }),
    [scopePath],
  );

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
    positions,
    stream,
    surface,
    remarkSurface,
    surfaceModel,
    setSurfaceModel,
    paletteOpen,
    setPaletteOpen,
    scope,
    setScopePath,
    selected,
    setSelected,
    drillToRisk,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
