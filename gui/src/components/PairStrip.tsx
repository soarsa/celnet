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
 * statically and truthfully at its pip precision. The movement cue is ONE honest,
 * REPRESENTATIVE series per pair (P0-4): we pick that pair's single most-active
 * streamed line — the row with the longest real premium-mid history, preferring
 * an ATM/representative structure on ties — and plot its untransformed
 * `midHistory` directly (no rebasing, no cross-structure averaging, no synthetic
 * "activity index"). The tick glyph uses the SAME direction rule as the sparkline
 * (net first→last over the window), so the two never disagree. A pair with no
 * live line shows an honest empty trend ("—"), never a fabricated tick.
 */

import { useCallback, useMemo, useRef } from "react";
import { useApp } from "../app/AppContext";
import type { StreamRow } from "../hooks/useStreamSession";
import type { CcyPair } from "../data/contract";
import type { PairContext } from "../data/seed";
import { Sparkline, sparklineDirection, type SparklineDir } from "./Sparkline";
import styles from "./PairStrip.module.css";

type Direction = SparklineDir | "none";

/** A pair's real, live activity trace — its single most-active streamed line. */
interface PairActivity {
  /** The representative line's real premium-mid history (empty ⇒ no live stream). */
  trace: number[];
  /** Net direction over the window (same rule as the sparkline; `none` ⇒ no data). */
  direction: Direction;
  /** How many streamed lines back this pair (for an honest tooltip / a11y). */
  liveLines: number;
  /** The chosen line's structure label, for the tooltip (honest provenance). */
  lineLabel: string;
}

const EMPTY_ACTIVITY: PairActivity = {
  trace: [],
  direction: "none",
  liveLines: 0,
  lineLabel: "",
};

function samePair(a: CcyPair, b: CcyPair): boolean {
  return a.base === b.base && a.quote === b.quote;
}

function pairKey(p: CcyPair): string {
  return `${p.base}${p.quote}`;
}

/** Prefer an ATM / vanilla representative line when several are equally active. */
function representativeScore(label: string): number {
  const l = label.toUpperCase();
  if (l.includes("ATM")) return 2;
  if (l.includes("CALL") || l.includes("PUT")) return 1;
  return 0;
}

/**
 * Reduce the live stream rows to ONE representative series per pair: the most-
 * active streamed line (longest real `midHistory`), preferring an ATM/vanilla
 * structure on ties. We plot that line's history verbatim — no rebasing, no
 * averaging across structures — so the strip shows a real, honest series and its
 * direction follows the SAME net-over-window rule as the sparkline tint.
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
    // The representative line is the one with the most real ticks; ties break to
    // an ATM/vanilla structure, then to the first seen (stable order).
    let best: StreamRow | null = null;
    for (const r of group) {
      if (r.midHistory.length < 2) continue;
      if (
        !best ||
        r.midHistory.length > best.midHistory.length ||
        (r.midHistory.length === best.midHistory.length &&
          representativeScore(r.label) > representativeScore(best.label))
      ) {
        best = r;
      }
    }
    if (!best) {
      out.set(key, { trace: [], direction: "none", liveLines: group.length, lineLabel: "" });
      continue;
    }
    const trace = best.midHistory.slice();
    out.set(key, {
      trace,
      direction: sparklineDirection(trace),
      liveLines: group.length,
      lineLabel: best.label,
    });
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
  // Tint direction for the sparkline must match the glyph: both follow the
  // net-over-window rule. `none` (no live data) renders no line at all.
  const sparkDir: SparklineDir = dir === "none" ? "flat" : dir;

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
    ? `${label} — set active pair · trend: premium mid of ${activity.lineLabel || "primary line"}` +
      ` · ${activity.liveLines} live line${activity.liveLines === 1 ? "" : "s"}`
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
          <>
            <span className={styles.sparkTag} aria-hidden="true">
              premium
            </span>
            <Sparkline
              values={activity.trace}
              direction={sparkDir}
              width={68}
              height={18}
              ariaLabel={`premium-mid trend, ${dir}`}
            />
          </>
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
