/**
 * hedgeLifecycle — the pure logic behind the Hedge Flow **lifecycle ribbon**.
 *
 * The hedge story has five stages and, until this existed, each one lived on a
 * different screen:
 *
 *   exposure → decided → sent to the street → filled → residual
 *
 * A desk could see what it was carrying (the bucket board), and separately see what
 * had fired (the hedge ledger), and separately again see the street orders — but
 * nothing showed the **drop-off between stages**. That gap is not cosmetic: it is
 * exactly how a desk ran with every futures hedge refused `NOT_A_WHOLE_LOT` while the
 * board cheerfully reported hedges firing. Orders were sent, none filled, and no
 * screen subtracted the two.
 *
 * ## The unit rule (do not break this)
 *
 * Stage totals in the RISK metric come only from {@link HedgeProvenance}, whose
 * `internalCrossed` / `externalHedged` / `residual` are all denominated in the budget
 * metric (DV01). The street stage is reported in **order COUNTS only** — never summed
 * quantities — because `StreetOrder.requestedQty` and `filledQty` are in the order's
 * *venue-native* units: contract face for a future, notional for a swap. Adding a
 * bond future's face to a swap's notional and calling the total "risk" is the precise
 * unit conflation that produced the DV01→contracts→face bug; a ribbon that did it
 * would re-introduce the very defect it exists to expose.
 *
 * React-free and I/O-free, so every rule below is directly testable.
 */

import type { HedgeBucket } from "./hedgeBuckets";
import type { HedgeProvenance, StreetOrder, StreetOutcome } from "../data/contract";

/** Outcomes that put at least some risk away. Everything else shed nothing. */
const FILLED_OUTCOMES: readonly StreetOutcome[] = ["filled", "partially_filled"];

/** One machine-stable unfilled reason and how often it occurred. */
export interface ReasonCount {
  /** The venue's own code (`NOT_A_WHOLE_LOT`) or ours (`no_firm_lp_price`). */
  reason: string;
  /** How many unfilled orders carried it. */
  count: number;
}

/** What actually happened on the street, in ORDER COUNTS (never summed quantities). */
export interface StreetTally {
  /** Orders belonging to a hedge decision. */
  sent: number;
  /** Orders that filled completely. */
  filled: number;
  /** Orders that filled in part — risk moved, but not all of it. */
  partial: number;
  /** Orders that shed NOTHING: rejected, cancelled, expired, pulled, no liquidity. */
  unfilled: number;
  /**
   * Why the unfilled ones failed, worst-first.
   *
   * Surfaced at OVERVIEW level deliberately. These reasons already existed on the
   * street blotter, but a desk does not open a blotter to discover that hedging has
   * silently stopped working — it has to be on the board that says hedging is fine.
   */
  reasons: ReasonCount[];
  /**
   * Street orders carrying NO `parentHedgeId`, so they cannot be attributed to a
   * hedge decision.
   *
   * Reported rather than dropped: a ribbon that silently discarded them would read
   * "0 sent" on a desk whose orders simply are not stamped, which is indistinguishable
   * from a desk that sent nothing — and those are opposite problems.
   */
  unlinked: number;
}

/** The five-stage ribbon. Risk-metric stages are DV01; the street stage is counts. */
export interface HedgeLifecycle {
  /** Risk currently carried: `|netRisk|` summed over capped books. */
  exposure: number;
  /** What the engine sized for exit = {@link crossed} + {@link external} + {@link warehoused}. */
  decided: number;
  /** Netted against opposing internal flow (the aggregated book). */
  crossed: number;
  /** Externalised onto the RFQ/FIX panel. */
  external: number;
  /** Sized but never shed — still the firm's risk. */
  warehoused: number;
  /** How many LIVE fires contributed (advisory dry-runs excluded). */
  fires: number;
  /** The street stage. */
  street: StreetTally;
}

/**
 * Did this order put any risk away?
 *
 * A `partially_filled` order counts as filled HERE and is also counted in
 * {@link StreetTally.partial}, because both facts matter: risk moved, and not all of
 * it did. Treating a partial as unfilled would overstate the failure; treating it as
 * a clean fill would hide a venue that never completes.
 */
function shedSomething(order: StreetOrder): boolean {
  return FILLED_OUTCOMES.includes(order.outcome) && order.filledQty > 0;
}

/**
 * Tally the street orders belonging to hedge decisions.
 *
 * Orders are attributed by `parentHedgeId` — the breach → street-order walk the wire
 * already stamps. `hedgeIds`, when given, narrows the tally to those fires (so the
 * ribbon can follow a filtered board); omit it to tally every hedge-linked order.
 */
export function streetTally(
  orders: readonly StreetOrder[],
  hedgeIds?: ReadonlySet<string>,
): StreetTally {
  const reasons = new Map<string, number>();
  let sent = 0;
  let filled = 0;
  let partial = 0;
  let unfilled = 0;
  let unlinked = 0;

  for (const order of orders) {
    const parent = order.parentHedgeId;
    if (parent === undefined || parent === "") {
      unlinked += 1;
      continue;
    }
    if (hedgeIds !== undefined && !hedgeIds.has(parent)) continue;
    sent += 1;
    if (shedSomething(order)) {
      if (order.outcome === "partially_filled") partial += 1;
      else filled += 1;
      continue;
    }
    unfilled += 1;
    // An absent reason on a non-filling order is itself the finding: the venue said
    // nothing we recorded. Naming the outcome beats dropping the row from the tally.
    const reason =
      order.reason !== undefined && order.reason !== "" ? order.reason : order.outcome;
    reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
  }

  return {
    sent,
    filled,
    partial,
    unfilled,
    unlinked,
    reasons: [...reasons.entries()]
      .map(([reason, count]): ReasonCount => ({ reason, count }))
      .sort((a, b) => b.count - a.count || a.reason.localeCompare(b.reason)),
  };
}

/** Sum a field over the rows, treating a non-finite value as 0 rather than poisoning the total. */
function sumOf(rows: readonly HedgeProvenance[], pick: (p: HedgeProvenance) => number): number {
  return rows.reduce((acc, p) => {
    const v = pick(p);
    return acc + (Number.isFinite(v) ? v : 0);
  }, 0);
}

/**
 * Build the whole ribbon.
 *
 * ADVISORY fires are excluded from every risk stage: they are dry-run computations
 * that never traded, so counting them would overstate how much the desk actually shed
 * — the single most misleading thing this board could do (the same rule
 * `flowTotals` follows, for the same reason).
 */
export function hedgeLifecycle(
  buckets: readonly HedgeBucket[],
  provenance: readonly HedgeProvenance[],
  orders: readonly StreetOrder[],
): HedgeLifecycle {
  const live = provenance.filter((p) => !p.advisory);
  const crossed = sumOf(live, (p) => p.internalCrossed);
  const external = sumOf(live, (p) => p.externalHedged);
  const warehoused = sumOf(live, (p) => p.residual);
  const exposure = buckets.reduce(
    (acc, b) => acc + (Number.isFinite(b.netRisk) ? Math.abs(b.netRisk) : 0),
    0,
  );
  return {
    exposure,
    decided: crossed + external + warehoused,
    crossed,
    external,
    warehoused,
    fires: live.length,
    street: streetTally(orders, new Set(live.map((p) => p.hedgeId))),
  };
}

/**
 * Is the street stage telling us hedging is BROKEN rather than merely quiet?
 *
 * True when orders were sent and none of them shed anything. A desk reading a board
 * full of firing hedges has no other signal that every one of them bounced — this is
 * the condition that went unnoticed while the books sat pinned at their limit.
 */
export function streetIsStalled(street: StreetTally): boolean {
  return street.sent > 0 && street.filled === 0 && street.partial === 0;
}
