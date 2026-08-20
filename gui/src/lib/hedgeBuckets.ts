/**
 * hedgeBuckets — the pure logic behind the Hedge Flow dashboard.
 *
 * Two questions a rates desk asks all morning, and nothing on the screen answered
 * before this existed:
 *
 *   1. **Which buckets need hedging?** Each risk book carries a signed `netRisk`
 *      against a resolved `threshold`, and the server bands the ratio. A bucket is
 *      literally a vessel filling toward its brim — {@link bucketsFromIntents} turns
 *      the live intent stream into that view, worst-first.
 *
 *   2. **Where did the risk actually go?** Every fired hedge splits into three
 *      disjoint parts — crossed internally, shed externally, or warehoused —
 *      summed by {@link flowTotals}.
 *
 * The band is ALWAYS the server's (`HedgeIntent.band` / `HedgeProvenance.band`),
 * never re-derived from the ratio here: the policy engine decides what is red, and a
 * screen that computed its own edges could disagree with the engine that trades.
 * This module only ORDERS and SIZES what the server already decided.
 *
 * React-free and I/O-free, so every rule below is directly testable.
 */

import type {
  HedgeIntent,
  HedgeProvenance,
  RiskBookRisk,
} from "../data/contract";

/** One risk bucket, ready to render as a vessel filling toward its brim. */
export interface HedgeBucket {
  /** The risk book this bucket represents (the id the engine bands). */
  book: string;
  /** The server's RAG band verbatim: `green` / `amber` / `red` / `breach`. */
  band: string;
  /** `|netRisk| / threshold` as the server computed it. */
  utilization: number;
  /** The signed net risk — sign carries the direction (long/short). */
  netRisk: number;
  /** The resolved threshold: the brim of the vessel. */
  threshold: number;
  /** How full to draw it, clamped to [0, 1]. 1 = at the brim. */
  fill: number;
  /** Over-brim fraction (0 when within threshold) — the part that spills. */
  overflow: number;
  /** True once the bucket is at or past its threshold: this one needs hedging. */
  needsHedge: boolean;
}

/** The three disjoint places risk lands once a hedge fires. */
export interface HedgeFlow {
  /** Netted against opposing internal flow (the aggregated book). */
  crossed: number;
  /** Shed externally onto the RFQ/FIX panel. */
  hedged: number;
  /** Unshed — still the firm's risk. */
  warehoused: number;
  /** How many fires contributed. */
  fires: number;
}

/**
 * Draw-height cap. A bucket at 250% would render indistinguishably from one at
 * 130% if the fill scaled without bound, so the VESSEL saturates at its brim and
 * the excess is reported separately as {@link HedgeBucket.overflow}. The number is
 * never truncated — only the drawing is.
 */
const FULL = 1;

/** Worst-first ordering. Unknown band labels sort after the known ones, never first. */
const BAND_SEVERITY: readonly string[] = ["breach", "red", "amber", "green"];

/** Severity rank of a band label; lower = more urgent. Unknown ⇒ after the known set. */
export function bandRank(band: string): number {
  const i = BAND_SEVERITY.indexOf(band.toLowerCase());
  return i >= 0 ? i : BAND_SEVERITY.length;
}

/**
 * Collapse a running intent stream to the LATEST state per book.
 *
 * The stream is append-only and a book ticks repeatedly, so rendering it raw would
 * show the same book many times with stale rows among them. Later entries win —
 * the caller appends in arrival order.
 */
export function latestIntentByBook(
  intents: readonly HedgeIntent[],
): HedgeIntent[] {
  const latest = new Map<string, HedgeIntent>();
  for (const intent of intents) latest.set(intent.book, intent);
  return [...latest.values()];
}

/**
 * The bucket board: one vessel per book, worst-first.
 *
 * Ordered by band severity, then by utilisation descending, then by book name so
 * the board is STABLE — two books in the same band at the same utilisation must not
 * swap places on every tick, or the eye cannot track a row.
 *
 * A non-finite or non-positive utilisation yields `fill = 0` and `needsHedge = false`
 * rather than `Infinity`/`NaN`: an unconfigured budget is not a breach, and a
 * dashboard that screamed red at un-tuned config would train the desk to ignore it.
 */
export function bucketsFromIntents(
  intents: readonly HedgeIntent[],
): HedgeBucket[] {
  return latestIntentByBook(intents)
    .map((intent): HedgeBucket => {
      const usable =
        Number.isFinite(intent.utilization) && intent.utilization > 0
          ? intent.utilization
          : 0;
      const fill = Math.min(FULL, usable);
      const overflow = usable > FULL ? usable - FULL : 0;
      return {
        book: intent.book,
        band: intent.band,
        utilization: usable,
        netRisk: intent.netRisk,
        threshold: intent.threshold,
        fill,
        overflow,
        needsHedge: usable >= FULL,
      };
    })
    .sort(
      (a, b) =>
        bandRank(a.band) - bandRank(b.band) ||
        b.utilization - a.utilization ||
        a.book.localeCompare(b.book),
    );
}

/**
 * The utilisation percentage AS DISPLAYED, in whole percent.
 *
 * `toFixed(0)` alone rounds 99.94% up to "100%", which put two buckets on the board
 * reading "100% of limit" directly under a header saying "no bucket at limit" — the badge
 * tests `utilization >= 1` exactly, so the two disagreed by construction. Worse, "100%"
 * on a book that is merely *near* its cap sends a desk hunting for a breach that has not
 * happened.
 *
 * So a bucket that is under its limit is capped at 99: **"100%" means AT the limit**, and
 * the number now agrees with the badge, the band, and `needsHedge`. Over the limit still
 * reports the true figure (a 150% bucket says 150%).
 */
export function displayPercent(utilization: number): number {
  if (!Number.isFinite(utilization) || utilization <= 0) return 0;
  if (utilization >= FULL) return Math.round(utilization * 100);
  return Math.min(99, Math.round(utilization * 100));
}

/** Shape one already-banded (utilisation, band) pair into a drawable bucket. */
function shapeBucket(
  book: string,
  band: string,
  utilization: number,
  netRisk: number,
  threshold: number,
): HedgeBucket {
  const usable =
    Number.isFinite(utilization) && utilization > 0 ? utilization : 0;
  return {
    book,
    band,
    utilization: usable,
    netRisk,
    threshold,
    fill: Math.min(FULL, usable),
    overflow: usable > FULL ? usable - FULL : 0,
    needsHedge: usable >= FULL,
  };
}

/**
 * The bucket board from the CURRENT risk poll — the source that actually works on a
 * quiet desk.
 *
 * The hedge-intent stream only publishes when the engine EVALUATES, so a board
 * subscribed to it alone renders empty until something moves, however much risk is
 * already on the book. `listRiskBookRisk()` returns the state right now, with each
 * cap already banded server-side, so the board is populated on first paint and the
 * intent stream becomes what it should be: the signal to re-poll.
 *
 * One vessel per book, taking the book's WORST cap — a book is only as safe as its
 * tightest constraint, and showing an average would hide the one that breaches. A
 * book with no computable cap is omitted rather than drawn empty: absent is honest,
 * a full-looking empty vessel is not.
 */
export function bucketsFromRiskBooks(
  books: readonly RiskBookRisk[],
): HedgeBucket[] {
  const buckets: HedgeBucket[] = [];
  for (const book of books) {
    let worst: (typeof book.limits)[number] | undefined;
    for (const limit of book.limits) {
      if (!Number.isFinite(limit.fraction)) continue;
      if (worst === undefined || limit.fraction > worst.fraction) worst = limit;
    }
    if (worst === undefined) continue;
    buckets.push(
      shapeBucket(
        book.name.length > 0 ? book.name : book.bookId,
        worst.band,
        worst.fraction,
        worst.used,
        worst.limit,
      ),
    );
  }
  return buckets.sort(
    (a, b) =>
      bandRank(a.band) - bandRank(b.band) ||
      b.utilization - a.utilization ||
      a.book.localeCompare(b.book),
  );
}

/**
 * Sum the disposition of every fired hedge.
 *
 * ADVISORY fires are EXCLUDED: they are dry-run computations that never traded, so
 * counting them would overstate how much risk the desk actually shed — the single
 * most misleading thing this panel could do.
 */
export function flowTotals(provenance: readonly HedgeProvenance[]): HedgeFlow {
  const live = provenance.filter((p) => !p.advisory);
  const add = (acc: number, v: number): number =>
    acc + (Number.isFinite(v) ? v : 0);
  return {
    crossed: live.reduce((acc, p) => add(acc, p.internalCrossed), 0),
    hedged: live.reduce((acc, p) => add(acc, p.externalHedged), 0),
    warehoused: live.reduce((acc, p) => add(acc, p.residual), 0),
    fires: live.length,
  };
}

/**
 * Share of the flow each leg represents, for proportional bars. Returns zeros when
 * nothing has fired, so the bars collapse rather than dividing by zero.
 */
export function flowShares(flow: HedgeFlow): {
  crossed: number;
  hedged: number;
  warehoused: number;
} {
  const total = flow.crossed + flow.hedged + flow.warehoused;
  if (!(total > 0)) return { crossed: 0, hedged: 0, warehoused: 0 };
  return {
    crossed: flow.crossed / total,
    hedged: flow.hedged / total,
    warehoused: flow.warehoused / total,
  };
}

/**
 * The hedges that offset a given booked position — the trade→hedge lineage.
 *
 * The id is a `bigint`: a position id is a 64-bit wire integer, and comparing it as a
 * `number` would silently mis-match past 2^53 — the same class of bug that made the LP
 * panel read every timestamp as zero.
 *
 * `parentPositionId` is stamped on per-fill execution records that were classified
 * B2B or shed externally. Book-level advisory-intent records are keyed by BOOK, not
 * by a fill, and legitimately carry no parent. Those are returned by
 * {@link bookLevelFires} instead of being forced into a lineage they do not have —
 * a screen implying every hedge traces to one client trade would be inventing a
 * link the data never claimed.
 */
export function hedgesForPosition(
  provenance: readonly HedgeProvenance[],
  positionId: bigint,
): HedgeProvenance[] {
  return provenance.filter((p) => p.parentPositionId === positionId);
}

/** Fires with no parent position — book-level breaches, shown in their own lane. */
export function bookLevelFires(
  provenance: readonly HedgeProvenance[],
): HedgeProvenance[] {
  return provenance.filter(
    (p) => p.parentPositionId === undefined || p.parentPositionId === null,
  );
}
