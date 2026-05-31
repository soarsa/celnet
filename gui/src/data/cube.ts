/**
 * cube.ts — the vol-cube data hook for the cube-pivot lane (TRADING-UNIVERSE-
 * SCALE §5: pivot a vol cube across pair × tenor × delta). It assembles a real
 * (pair × tenor × delta) volatility grid from the SERVER's calibrated smiles via
 * the existing transport seam — `markSurface` (to publish a fresh version) then
 * the smiles it returns, falling back to per-tenor `getSmile` for tenors the
 * marked surface didn't carry. No client-side vol construction (API-first parity,
 * CLAUDE.md rule 11): every vol number is the server's calibrated smile point.
 *
 * HONESTY (rule 2 — no fakes): a cell is filled ONLY when the server's smile for
 * that (pair, tenor) actually carries a point at that delta pillar (within a tight
 * tolerance). Where it doesn't — an un-marked pair, a tenor with no smile, a wing
 * the calibration omitted — the cell is honestly EMPTY (`null`), never
 * interpolated or fabricated client-side. The standard FX delta pillars are used
 * for the columns; the standard tenor ladder for the rows.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import type { CcyPair, Conventions, Smile } from "./contract";
import { TENOR_LADDER } from "./seed";
import { pairId, pairLabel } from "../lib/universe";

/**
 * The standard signed delta pillars across the smile (puts negative, calls
 * positive, ATM at 0). These are the cube's columns. 10Δ wings are included
 * because the broker ladder carries them; a cell stays empty if a given smile
 * has no point there.
 */
export const DELTA_PILLARS: readonly number[] = [-0.1, -0.25, -0.5, 0.25, 0.1] as const;

/** Display label for a signed delta pillar (e.g. -0.25 → "25ΔP", 0.25 → "25ΔC"). */
export function deltaPillarLabel(delta: number): string {
  if (delta === -0.5 || delta === 0.5) return "ATM";
  const pct = Math.round(Math.abs(delta) * 100);
  return `${pct}Δ${delta < 0 ? "P" : "C"}`;
}

/** One cell of the cube: a calibrated vol, or null where the server has none. */
export interface CubeCell {
  /** Signed delta pillar (column key). */
  delta: number;
  /** Absolute vol (0.10 = 10 vol), or null for an honest empty cell. */
  vol: number | null;
}

/** One row of the cube: a tenor and its cells across the delta pillars. */
export interface CubeRow {
  tenorLabel: string;
  tenorYears: number;
  cells: CubeCell[];
}

/** The assembled cube grid for one pair. */
export interface CubeGrid {
  pair: CcyPair;
  pairLabel: string;
  pairId: string;
  /** The delta pillars, in column order. */
  deltas: readonly number[];
  /** Rows, one per tenor in the ladder, in ladder order. */
  rows: CubeRow[];
  /** The surface version these vols came from (0n if not yet marked). */
  surfaceVersion: bigint;
}

/** Loading/error/ready state of the cube hook (honest, surfaced to the view). */
export interface CubeState {
  grid: CubeGrid | null;
  loading: boolean;
  /** A human error string when the assembly failed (honest empty-state copy). */
  error: string | null;
  /** Re-assemble the cube (re-marks + re-reads from the server). */
  refresh: () => void;
}

const DELTA_TOL = 1e-6;

/** Pick the calibrated vol at a delta pillar from a smile, or null if absent. */
function volAtPillar(smile: Smile, delta: number): number | null {
  let best: number | null = null;
  let bestErr = DELTA_TOL;
  for (const p of smile.points) {
    const err = Math.abs(p.delta - delta);
    if (err <= bestErr) {
      bestErr = err;
      best = p.vol;
    }
  }
  return best;
}

/** Find the smile in a list whose tenor matches `years` (within a small tol). */
function smileForTenor(smiles: Smile[], years: number): Smile | null {
  let best: Smile | null = null;
  let bestErr = Number.POSITIVE_INFINITY;
  for (const s of smiles) {
    const err = Math.abs(s.tenorYears - years);
    if (err < bestErr) {
      bestErr = err;
      best = s;
    }
  }
  // Accept only a genuinely close tenor (≤ ~1 day); otherwise no smile here.
  return best && bestErr <= 2 / 365 ? best : null;
}

/**
 * Build a cube grid from a marked surface's smiles for the chosen pair. Pure +
 * deterministic. Tenors with no matching smile, and pillars with no calibrated
 * point, are left as honest empty cells.
 */
function assembleGrid(
  pair: CcyPair,
  smiles: Smile[],
  surfaceVersion: bigint,
): CubeGrid {
  const rows: CubeRow[] = TENOR_LADDER.map((t) => {
    const smile = smileForTenor(smiles, t.years);
    const cells: CubeCell[] = DELTA_PILLARS.map((delta) => ({
      delta,
      vol: smile ? volAtPillar(smile, delta) : null,
    }));
    return { tenorLabel: t.label, tenorYears: t.years, cells };
  });
  return {
    pair,
    pairLabel: pairLabel(pair),
    pairId: pairId(pair),
    deltas: DELTA_PILLARS,
    rows,
    surfaceVersion,
  };
}

/**
 * Assemble a vol cube for a pair from the server's calibrated smiles. When
 * `pair` is omitted the active pair is used. The cube re-assembles when the pair
 * or the app's current `surfaceVersion` changes (e.g. after a re-mark), so the
 * pivot stays in lockstep with the marked surface.
 *
 * Implementation: we prefer the app's already-marked `surface` when it is for the
 * requested pair (no redundant round-trip); otherwise we `markSurface` the pair's
 * broker ladder to obtain a fresh, versioned smile set. Any tenor the surface
 * omitted is backfilled with a direct `getSmile` so the grid is as complete as
 * the server can make it — and only as complete (empty cells stay empty).
 */
export function useCube(pair?: CcyPair): CubeState {
  const app = useApp();
  const targetPair = pair ?? app.pairCtx.pair;
  const conventions: Conventions = app.conventions;
  const transport = app.transport;

  const [grid, setGrid] = useState<CubeGrid | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  // The active marked surface, only usable when it is for the requested pair.
  const surface = app.surface;
  const surfaceForPair =
    surface &&
    surface.pair.base === targetPair.base &&
    surface.pair.quote === targetPair.quote
      ? surface
      : null;

  const refresh = useCallback(() => setNonce((n) => n + 1), []);

  // Re-assemble on pair change, surface-version change, or an explicit refresh.
  const surfaceVersionKey = surfaceForPair ? surfaceForPair.surfaceVersion.toString() : "none";
  const pairKey = pairId(targetPair);

  // Hold the latest broker ladder for the pair without re-subscribing the effect
  // to `app` churn: read it through a ref so the effect deps stay minimal.
  const appRef = useRef(app);
  appRef.current = app;

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);

    const run = async (): Promise<void> => {
      try {
        // 1. Obtain a versioned smile set for the pair: reuse the active marked
        //    surface when it matches, else mark the pair's broker ladder.
        let smiles: Smile[];
        let version: bigint;
        if (surfaceForPair) {
          smiles = surfaceForPair.smiles.slice();
          version = surfaceForPair.surfaceVersion;
        } else {
          const { brokerLadder } = await import("./seed");
          const ladderCtx = appRef.current.pairs.find(
            (p) => p.pair.base === targetPair.base && p.pair.quote === targetPair.quote,
          );
          if (!ladderCtx) {
            if (live) {
              setGrid(assembleGrid(targetPair, [], 0n));
              setLoading(false);
            }
            return;
          }
          const marked = await transport.markSurface(
            targetPair,
            brokerLadder(ladderCtx),
            conventions,
            appRef.current.surfaceModel,
          );
          smiles = marked.smiles.slice();
          version = marked.surfaceVersion;
        }

        // 2. Backfill any ladder tenor the surface did not carry via getSmile.
        const haveYears = new Set(smiles.map((s) => Math.round(s.tenorYears * 365)));
        const missing = TENOR_LADDER.filter((t) => !haveYears.has(Math.round(t.years * 365)));
        if (missing.length > 0) {
          const fetched = await Promise.all(
            missing.map((t) =>
              transport
                .getSmile(targetPair, t.years, conventions)
                .then((s) => s)
                .catch(() => null),
            ),
          );
          for (const s of fetched) if (s) smiles.push(s);
        }

        if (!live) return;
        setGrid(assembleGrid(targetPair, smiles, version));
        setLoading(false);
      } catch (e) {
        if (!live) return;
        // Honest failure: surface the message + an empty grid (empty cells), so
        // the pivot shows its empty-state rather than a fabricated cube.
        setError(e instanceof Error ? e.message : "Could not assemble the vol cube");
        setGrid(assembleGrid(targetPair, [], 0n));
        setLoading(false);
      }
    };

    void run();
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pairKey, surfaceVersionKey, nonce, transport, conventions]);

  return useMemo(
    () => ({ grid, loading, error, refresh }),
    [grid, loading, error, refresh],
  );
}
