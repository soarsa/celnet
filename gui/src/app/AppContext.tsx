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
  // The transport is selected once at the app root: the deterministic in-app mock
  // by default (standalone), or the live WebSocket mirror when configured via the
  // build-time env flag (src/data/transportConfig.ts). One contract, two transports.
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

  useEffect(() => {
    void remarkSurface();
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
