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
import type { CcyPair, Conventions, MarkedSurface } from "../data/contract";
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

export type WorkspaceId = "ticket" | "stream" | "surface" | "risk";

export interface TicketSeed {
  pair: CcyPair;
  /** A structure name to preload into the ticket (from "Stream this"/"Add"). */
  label: string;
  expiryYears: number;
}

interface AppState {
  transport: CelnetTransport;
  conventions: Conventions;
  workspace: WorkspaceId;
  setWorkspace: (w: WorkspaceId) => void;
  pairCtx: PairContext;
  setPair: (pair: CcyPair) => void;
  pairs: PairContext[];
  stream: StreamApi;
  surface: MarkedSurface | null;
  remarkSurface: () => Promise<void>;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
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

  const remarkSurface = useMemo(
    () => async () => {
      const marked = await transport.markSurface(
        pairCtx.pair,
        brokerLadder(pairCtx),
        conventions,
      );
      setSurface(marked);
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

  const value: AppState = {
    transport,
    conventions,
    workspace,
    setWorkspace,
    pairCtx,
    setPair,
    pairs: PAIRS,
    stream,
    surface,
    remarkSurface,
    paletteOpen,
    setPaletteOpen,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
