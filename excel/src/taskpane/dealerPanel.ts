/**
 * The multi-dealer (RFQ-to-many) ranked-panel task-pane MODEL — pure,
 * framework-free, unit-testable. A headless port of the GUI's interactive
 * `DealerPanel` + `LastLookRing` (gui/src/components/DealerPanel.tsx,
 * LastLookRing.tsx) onto the add-in's `Connection`, so the Excel task pane reaches
 * multi-dealer parity (today the pane shows a single hardcoded dealer).
 *
 * It takes a `Connection.requestMultiDealerQuote` result and projects it to a flat
 * list of panel lines in the SERVER aggregator's ranking order — VERBATIM, never
 * re-sorted (the order is the deterministic, best-first audit order; a client that
 * re-ranks would diverge from the server's price-tape). Each line is stamped with
 * its touch-winner role (`bestBid` / `bestOffer`) from the frame's
 * `bestBidLpId` / `bestOfferLpId`, and depletes its OWN last-look ring against its
 * `validUntilNanos` — an expired line is no longer tradable, so the trader can
 * never book on an expired window (the same honest deadline the contract stamps).
 * Booking a chosen line delegates to `Connection.acceptQuote` keyed on the
 * aggregate `(quoteId, lpId)` plus the originating idempotency key, emitting the
 * exact frame the server's request-matched accept expects.
 *
 * `taskpane.ts` binds this model to the DOM and the live transport; keeping the
 * projection + the last-look math + the accept pure lets the node unit suite drive
 * every line, ranking, and depletion path with NO Office host and NO server.
 */

import type { Connection } from "../transport/connection";
import type { Execution, MultiDealerQuote, TwoWayPrice } from "../contract/contract";

/** Nanoseconds per second — the contract stamps last-look deadlines in nanos. */
const NANOS_PER_SECOND = 1_000_000_000n;

/** The full last-look window length (seconds) the ring depletes from, by default. */
export const DEFAULT_WINDOW_SECONDS = 8;

/**
 * One projected dealer line in the ranked panel — a flattened, presentation-ready
 * view of a `DealerQuote` with its touch-winner role resolved against the panel's
 * `bestBidLpId` / `bestOfferLpId`. The `rank` is the line's 0-based position in the
 * SERVER's ranking order (best-first), preserved verbatim from the frame.
 */
export interface DealerLine {
  /** The liquidity provider's stable identifier (the dealer/LP key). */
  readonly lpId: string;
  /** This dealer's two-way premium in the request's premium-style units. */
  readonly price: TwoWayPrice;
  /** The strike this dealer resolved (if the request used a delta / solve). */
  readonly resolvedStrike: number;
  /** This dealer's last-look deadline, nanoseconds since the Unix epoch (UTC). */
  readonly validUntilNanos: bigint;
  /** 0-based position in the server's ranking order (best-first), verbatim. */
  readonly rank: number;
  /** True iff this line is the touch best bid (`panel.bestBidLpId === lpId`). */
  readonly bestBid: boolean;
  /** True iff this line is the touch best offer (`panel.bestOfferLpId === lpId`). */
  readonly bestOffer: boolean;
}

/**
 * The full ranked-panel model: the aggregate `quoteId` + the originating
 * idempotency key that every booked line echoes, and the projected dealer lines in
 * the server's ranking order. A panel is an immutable snapshot of one
 * `MultiDealerQuote` frame; a fresh request yields a fresh panel (the client never
 * mutates ranking in place).
 */
export interface DealerPanelState {
  /** The aggregate quote id keying the request (echoed on every accept). */
  readonly quoteId: bigint;
  /**
   * The idempotency key the panel was REQUESTED under. The server's accept is
   * request-matched (an accept must echo the originating key, so a learned
   * `quoteId` alone is never an authority token); the panel carries the key from
   * request to accept — exactly as the RFQ ticket does.
   */
  readonly idempotencyKey: string;
  /** The projected dealer lines, in the server's ranking order (best-first). */
  readonly lines: readonly DealerLine[];
}

/**
 * Project a `Connection.requestMultiDealerQuote` result into the ranked-panel
 * model. The dealer lines are taken in the frame's order VERBATIM — the server's
 * aggregator already ranked them best-first and that order is the audit order; the
 * client never re-sorts. Each line's touch role is resolved against the frame's
 * `bestBidLpId` / `bestOfferLpId` (an empty winner id ⇒ no line is marked on that
 * side, the contract's "no dealer quoted that side" sentinel).
 */
export function fromMultiDealer(result: MultiDealerQuote): DealerPanelState {
  return {
    quoteId: result.quoteId,
    idempotencyKey: result.idempotencyKey,
    lines: result.dealers.map((d, rank) => ({
      lpId: d.lpId,
      price: d.price,
      resolvedStrike: d.resolvedStrike,
      validUntilNanos: d.validUntilNanos,
      rank,
      // A winner id is only a marker when non-empty (the contract uses "" for a
      // side no dealer quoted), so an empty id never spuriously badges a line.
      bestBid: result.bestBidLpId !== "" && result.bestBidLpId === d.lpId,
      bestOffer: result.bestOfferLpId !== "" && result.bestOfferLpId === d.lpId,
    })),
  };
}

/**
 * The fraction (0..1) of a line's last-look window still remaining at `nowNanos`,
 * clamped to `[0, 1]` — the ring depletion base the GUI's `LastLookRing` renders.
 * `1` at the start of the window, `0` once the deadline passes (or the window is
 * degenerate). Reads the SAME nanosecond deadline the contract stamps on the line,
 * so the depletion is honest — a trader never sees remaining time on an expired
 * line. The window length defaults to {@link DEFAULT_WINDOW_SECONDS}.
 */
export function lastLookRemaining(
  line: DealerLine,
  nowNanos: bigint,
  windowSeconds: number = DEFAULT_WINDOW_SECONDS,
): number {
  if (windowSeconds <= 0) return 0;
  const remainingNanos = line.validUntilNanos - nowNanos;
  if (remainingNanos <= 0n) return 0;
  // Integer-nanos / window math kept in bigint until the final ratio so a 64-bit
  // deadline beyond the JS safe range never loses precision before the clamp.
  const windowNanos = BigInt(Math.round(windowSeconds * 1000)) * (NANOS_PER_SECOND / 1000n);
  if (remainingNanos >= windowNanos) return 1;
  return Number(remainingNanos) / Number(windowNanos);
}

/** True iff the line's last-look window has elapsed at `nowNanos` (not tradable). */
export function isExpired(line: DealerLine, nowNanos: bigint): boolean {
  return line.validUntilNanos <= nowNanos;
}

/** Locate a panel line by its LP id, or `undefined` if no such line exists. */
export function lineFor(panel: DealerPanelState, lpId: string): DealerLine | undefined {
  return panel.lines.find((l) => l.lpId === lpId);
}

/**
 * Book a chosen panel line: BUY lifts that dealer's offer, SELL hits its bid. The
 * accept is keyed on the aggregate `(quoteId, lpId)` plus the originating
 * idempotency key the panel carries — exactly the request-matched frame the
 * server's `accept_quote` expects, with `lpId` selecting the pinned dealer line.
 * Rejects if the named line is absent from this panel (a stale / re-requested
 * panel), so a forged or expired-panel lpId can never be booked.
 */
export async function accept(
  panel: DealerPanelState,
  lpId: string,
  side: "BUY" | "SELL",
  conn: Connection,
): Promise<Omit<Execution, "instrument">> {
  const line = lineFor(panel, lpId);
  if (!line) {
    throw new Error(`no panel line for lp \`${lpId}\` — re-request the panel`);
  }
  return conn.acceptQuote({
    quoteId: panel.quoteId,
    side,
    idempotencyKey: panel.idempotencyKey,
    lpId: line.lpId,
  });
}
