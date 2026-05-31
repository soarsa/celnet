/**
 * PairStrip — the persistent, always-visible PAIR NAVIGATOR (closes the product
 * owner's gap "it's not clear how users would view different currency pairs").
 *
 * A horizontal watchlist pinned below the TitleBar on every workspace: one tile
 * per watched pair (from `app.pairs`). Each tile shows the pair label (Anaheim
 * brand face), the live spot (mono, at the pair's pip precision), a tick-direction
 * colour cue, and an inline Sparkline of the pair's recent activity. The ACTIVE
 * pair (`app.pairCtx`) is marked with the Celer brand coral (left bar + tint +
 * glow + a subtle live pulse) and raised onto a brighter material; the rest stay
 * quiet. Clicking — or arrow-keying across the strip and pressing Enter/Space —
 * re-targets the global pair (`app.setPair`). The command palette (⌘P/⌘K) still
 * works; this is an additive, visible affordance.
 *
 * HONESTY (CLAUDE.md rule 2 — no fake data): the spot is the real seeded market
 * mark; there is no synthetic live spot feed in this transport, so it is shown
 * statically and truthfully at its pip precision. The movement cues — sparkline
 * and tick direction — are derived ONLY from real data: the live RFS stream rows
 * carry genuine streamed premium-mid histories (`midHistory`), each tagged with
 * its pair. We aggregate the histories of the rows belonging to a pair into a
 * per-pair activity trace and read its direction from the last two real points.
 * A pair with no streamed line shows an honest empty trend ("—"), never a
 * fabricated tick.
 */

import { useCallback, useMemo, useRef } from "react";
import { useApp } from "../app/AppContext";
import type { StreamRow } from "../hooks/useStreamSession";
import type { CcyPair } from "../data/contract";
import type { PairContext } from "../data/seed";
import { Sparkline } from "./Sparkline";
import styles from "./PairStrip.module.css";

type Direction = "up" | "down" | "flat" | "none";

/** A pair's real, live activity trace assembled from the streamed RFS rows. */
interface PairActivity {
  /** Recent activity mids for the sparkline (empty ⇒ no live stream for the pair). */
  trace: number[];
  /** Tick direction from the last two real points (`none` ⇒ no live data). */
  direction: Direction;
  /** How many streamed lines back this pair (for an honest tooltip / a11y). */
  liveLines: number;
}

const EMPTY_ACTIVITY: PairActivity = { trace: [], direction: "none", liveLines: 0 };

function samePair(a: CcyPair, b: CcyPair): boolean {
  return a.base === b.base && a.quote === b.quote;
}

function pairKey(p: CcyPair): string {
  return `${p.base}${p.quote}`;
}

/**
 * Aggregate the live stream rows into a per-pair activity trace. For each pair we
 * rebase every contributing row's real `midHistory` to its own first point (so
 * differently-scaled premiums combine into one comparable activity series) and
 * average across rows tick-for-tick over their common length. Direction is read
 * from the last two points of the resulting REAL series — no synthesis.
 */
function aggregateActivity(rows: StreamRow[]): Map<string, PairActivity> {
  const byPair = new Map<string, StreamRow[]>();
  for (const row of rows) {
    const key = pairKey(row.instrument.pair);
    const list = byPair.get(key);
    if (list) list.push(row);
    else byPair.set(key, [row]);
  }

  const out = new Map<string, PairActivity>();
  for (const [key, group] of byPair) {
    // Only rows with a usable (≥2-point) real history contribute movement.
    const usable = group.filter((r) => r.midHistory.length >= 2);
    if (usable.length === 0) {
      out.set(key, { trace: [], direction: "none", liveLines: group.length });
      continue;
    }
    // Align the rebased series on the most recent `len` ticks they all share, so
    // the latest tick of every contributing row lines up (right-aligned).
    const len = Math.min(...usable.map((r) => r.midHistory.length));
    const trace: number[] = new Array(len).fill(0) as number[];
    for (const r of usable) {
      const h = r.midHistory;
      const base = h[h.length - len] ?? h[0] ?? 0;
      const denom = base !== 0 ? base : 1;
      const start = h.length - len;
      for (let i = 0; i < len; i += 1) {
        // Rebased to a 1.0 baseline: a unitless, comparable activity index.
        trace[i]! += (h[start + i] ?? base) / denom;
      }
    }
    for (let i = 0; i < len; i += 1) trace[i]! /= usable.length;

    const last = trace[len - 1] ?? 0;
    const prev = trace[len - 2] ?? last;
    const direction: Direction = last > prev ? "up" : last < prev ? "down" : "flat";
    out.set(key, { trace, direction, liveLines: group.length });
  }
  return out;
}

const DIR_GLYPH: Record<Direction, string> = {
  up: "▲",
  down: "▼",
  flat: "▪",
  none: "–",
};

function PairTile({
  ctx,
  active,
  activity,
  index,
  registerRef,
  onSelect,
  onNavigate,
}: {
  ctx: PairContext;
  active: boolean;
  activity: PairActivity;
  index: number;
  registerRef: (index: number, el: HTMLButtonElement | null) => void;
  onSelect: (pair: CcyPair) => void;
  onNavigate: (from: number, delta: number) => void;
}): React.ReactElement {
  const label = `${ctx.pair.base}/${ctx.pair.quote}`;
  const live = activity.trace.length >= 2;
  const dir = activity.direction;

  const handleKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>): void => {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      onNavigate(index, 1);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      onNavigate(index, -1);
    } else if (e.key === "Home") {
      e.preventDefault();
      onNavigate(index, -index);
    }
  };

  const tip = live
    ? `${label} — set active pair · ${activity.liveLines} live line${activity.liveLines === 1 ? "" : "s"}`
    : `${label} — set active pair · no live stream`;

  return (
    <button
      ref={(el) => registerRef(index, el)}
      type="button"
      role="tab"
      className={[styles.tile, active ? styles.active : ""].join(" ")}
      onClick={() => onSelect(ctx.pair)}
      onKeyDown={handleKeyDown}
      aria-current={active ? "true" : undefined}
      aria-label={tip}
      title={tip}
      tabIndex={active ? 0 : -1}
    >
      <span className={styles.head}>
        <span className={styles.pair}>{label}</span>
        {active && <span className={styles.livePulse} aria-hidden="true" />}
      </span>
      <span className={styles.line}>
        <span className={`num ${styles.spot}`}>{ctx.market.spot.toFixed(ctx.pipDecimals)}</span>
        <span
          className={`num ${styles.tick} ${styles[`tick_${dir}`] ?? ""}`}
          aria-hidden="true"
        >
          {DIR_GLYPH[dir]}
        </span>
      </span>
      <span className={styles.spark}>
        {live ? (
          <Sparkline values={activity.trace} width={84} height={18} />
        ) : (
          <span className={styles.noSpark} aria-hidden="true">
            no live stream
          </span>
        )}
      </span>
    </button>
  );
}

export function PairStrip(): React.ReactElement {
  const app = useApp();
  const activePair = app.pairCtx.pair;
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  // Real per-pair activity derived from the live RFS stream rows (no fake ticks).
  const activity = useMemo(() => aggregateActivity(app.stream.rows), [app.stream.rows]);

  const registerRef = useCallback((index: number, el: HTMLButtonElement | null): void => {
    refs.current[index] = el;
  }, []);

  // Roving-tabindex arrow navigation across the watchlist (keyboard accessible).
  const navigate = useCallback(
    (from: number, delta: number): void => {
      const n = app.pairs.length;
      if (n === 0) return;
      const next = Math.min(n - 1, Math.max(0, from + delta));
      refs.current[next]?.focus();
    },
    [app.pairs.length],
  );

  return (
    <nav className={styles.strip} aria-label="currency pair watchlist">
      <span className={styles.railLabel} aria-hidden="true">
        Pairs
      </span>
      <div className={styles.tiles} role="tablist" aria-label="currency pairs">
        {app.pairs.map((ctx, i) => (
          <PairTile
            key={pairKey(ctx.pair)}
            ctx={ctx}
            index={i}
            active={samePair(ctx.pair, activePair)}
            activity={activity.get(pairKey(ctx.pair)) ?? EMPTY_ACTIVITY}
            registerRef={registerRef}
            onSelect={app.setPair}
            onNavigate={navigate}
          />
        ))}
      </div>
    </nav>
  );
}
