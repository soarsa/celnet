/**
 * portfolioRisk — the desk-wide risk aggregator behind the Book workspace.
 *
 * This closes the product owner's gap "how can a user see aggregated risk?". It
 * takes the desk's REAL open positions (`app.positions`), the per-pair base
 * market each is priced against, the trade conventions, and the transport, and
 * folds the genuine per-position risk into one book-level structure:
 *
 *   - net mark / net delta (per pair AND total) / net vega / gamma / theta;
 *   - an aggregate vega ladder bucketed by (tenor, delta), summed across positions;
 *   - aggregate cross-gamma per factor pair, summed across positions;
 *   - aggregate theta-roll P&L over the standard roll horizons, summed.
 *
 * Aggregation is honest, not fabricated. Every number is computed by repricing
 * each position via `SurfaceService.Scenario` (the same contract RPC the Risk
 * workspace uses) at its own pair's base market — once with a zero-shock axis for
 * the base Greeks, and once with a `RiskBucketRequest` for the book-shaped
 * decomposition — then scaled by the position's *signed* notional and summed.
 *
 * Sign convention (long/short): the contract's `Instrument.quantity.notional` is
 * a magnitude; direction lives in `Instrument.side`. A held position that is BUY
 * is long (+), SELL is short (−); a TWO_WAY booked structure is treated as the
 * long the desk is carrying (+). The signed notional is the single multiplier
 * applied to every Greek, so a short position correctly *reduces* the book's net
 * exposure rather than inflating it.
 *
 * A position whose pair has no known base market is skipped honestly rather than
 * priced against an invented market — it is reported in `skipped` so the caller
 * can disclose it instead of silently dropping risk.
 */

import type {
  CrossGamma,
  Greeks,
  Instrument,
  MarketContext,
  RiskBucketRequest,
  Side,
} from "./contract";
import type { CelnetTransport } from "./transport";
import type { Conventions, ShockAxis } from "./contract";

/** A single zero-shock axis: one node at the base market → the base Greeks. */
const BASE_AXES: ShockAxis[] = [{ factor: "SPOT", relative: true, steps: [0] }];

/** The delta pillars the aggregate vega ladder buckets on (signed deltas). */
export const VEGA_DELTA_PILLARS = [0.5, 0.25, -0.25, 0.1, -0.1] as const;

/**
 * The factor pairs the aggregate cross-gamma covers — the desk's coupled
 * second-orders (spot×vol = vanna-flavoured, rate×spot, spot×time = charm-flavoured).
 */
export const CROSS_GAMMA_PAIRS = [
  { factorA: "SPOT", factorB: "VOL" },
  { factorA: "RATE_DOM", factorB: "SPOT" },
  { factorA: "SPOT", factorB: "TIME" },
] as const;

/** The theta-roll horizons (years rolled forward): overnight, a week, a month. */
export const ROLL_HORIZONS = [1 / 365, 7 / 365, 30 / 365] as const;

const ZERO_GREEKS: Greeks = {
  price: 0,
  deltaSpot: 0,
  deltaForward: 0,
  gamma: 0,
  vega: 0,
  theta: 0,
  rhoDom: 0,
  rhoFor: 0,
  vanna: 0,
  volga: 0,
  charm: 0,
  speed: 0,
  zomma: 0,
  color: 0,
};

/** The signed notional of a held position: + for long (BUY/TWO_WAY), − for SELL. */
export function signedNotional(instrument: Instrument): number {
  return positionSign(instrument.side) * instrument.quantity.notional;
}

function positionSign(side: Side): number {
  return side === "SELL" ? -1 : 1;
}

function scaleGreeks(g: Greeks, mult: number): Greeks {
  return {
    price: g.price * mult,
    deltaSpot: g.deltaSpot * mult,
    deltaForward: g.deltaForward * mult,
    gamma: g.gamma * mult,
    vega: g.vega * mult,
    theta: g.theta * mult,
    rhoDom: g.rhoDom * mult,
    rhoFor: g.rhoFor * mult,
    vanna: g.vanna * mult,
    volga: g.volga * mult,
    charm: g.charm * mult,
    speed: g.speed * mult,
    zomma: g.zomma * mult,
    color: g.color * mult,
  };
}

function addGreeks(a: Greeks, b: Greeks): Greeks {
  return {
    price: a.price + b.price,
    deltaSpot: a.deltaSpot + b.deltaSpot,
    deltaForward: a.deltaForward + b.deltaForward,
    gamma: a.gamma + b.gamma,
    vega: a.vega + b.vega,
    theta: a.theta + b.theta,
    rhoDom: a.rhoDom + b.rhoDom,
    rhoFor: a.rhoFor + b.rhoFor,
    vanna: a.vanna + b.vanna,
    volga: a.volga + b.volga,
    charm: a.charm + b.charm,
    speed: a.speed + b.speed,
    zomma: a.zomma + b.zomma,
    color: a.color + b.color,
  };
}

/** A position paired with the base market it is priced against. */
export interface BookPosition {
  instrument: Instrument;
  market: MarketContext;
  /** "BASE/QUOTE" key, e.g. "EUR/USD". */
  pairKey: string;
}

/** Per-pair aggregated net risk (signed, notional-scaled). */
export interface PairBreakdown {
  pairKey: string;
  base: string;
  quote: string;
  /** Number of open positions in this pair. */
  count: number;
  /** Signed sum of |notional| across the pair's positions (book weight). */
  netNotional: number;
  netDelta: number;
  netVega: number;
  netGamma: number;
  netTheta: number;
}

/** One aggregated vega-ladder rung: net vega at a (tenor, delta) pillar. */
export interface AggVegaBucket {
  tenorYears: number;
  delta: number;
  /** Net vega per 1-vol-point move, in book P&L units (notional-scaled). */
  vegaPerPoint: number;
}

/** One aggregated cross-gamma cell, summed across the book (notional-scaled). */
export interface AggCrossGamma {
  factorA: CrossGamma["factorA"];
  factorB: CrossGamma["factorB"];
  value: number;
}

/** One aggregated theta-roll rung: the book's decay P&L at a roll horizon. */
export interface AggThetaRoll {
  horizonYears: number;
  /** Net P&L of rolling every position forward to this horizon (notional-scaled). */
  pnl: number;
}

/** The complete desk-wide aggregated-risk picture. */
export interface BookRisk {
  /** Number of positions actually aggregated (excludes skipped). */
  positionCount: number;
  /** Distinct pairs with at least one aggregated position. */
  pairCount: number;
  /** Book totals: every position's signed-notional-scaled Greeks, summed. */
  total: Greeks;
  /** Sum of |notional| across all aggregated positions (book gross size). */
  grossNotional: number;
  /** Per-pair breakdown, sorted by descending |net vega|. */
  byPair: PairBreakdown[];
  /** Aggregate vega ladder, sorted by tenor then descending |vega|. */
  vegaLadder: AggVegaBucket[];
  /** Aggregate cross-gamma per factor pair. */
  crossGammas: AggCrossGamma[];
  /** Aggregate theta-roll P&L per horizon. */
  thetaRoll: AggThetaRoll[];
  /** Positions skipped because their pair had no known base market (honest). */
  skipped: Instrument[];
}

function pairKeyOf(instrument: Instrument): string {
  return `${instrument.pair.base}/${instrument.pair.quote}`;
}

/**
 * Build the `RiskBucketRequest` for one position. Vega is bucketed at the
 * position's own expiry tenor across the standard delta pillars (a single-expiry
 * structure carries vega only at its own tenor; summing many positions across
 * many tenors is what fills the ladder). Cross-gamma and the theta roll use the
 * desk-wide standard factor pairs and horizons so they sum coherently.
 */
function riskRequestFor(instrument: Instrument): RiskBucketRequest {
  return {
    vegaPillars: VEGA_DELTA_PILLARS.map((delta) => ({
      tenorYears: instrument.expiryYears,
      delta,
    })),
    crossGammaPairs: CROSS_GAMMA_PAIRS.map((p) => ({ ...p })),
    rollHorizonsYears: [...ROLL_HORIZONS],
  };
}

/**
 * Aggregate REAL desk-wide risk across every open position across every pair.
 *
 * For each position this issues ONE `scenario` call carrying both a zero-shock
 * base axis (for the base Greeks) and a `RiskBucketRequest` (for the book-shaped
 * decomposition), then scales by the signed notional and folds into the running
 * book totals. All positions are priced concurrently; the result is a single
 * `BookRisk`. An empty `positions` (or one with no resolvable markets) yields a
 * zero-position `BookRisk` so the caller renders an honest empty-state — never
 * fabricated numbers.
 */
export async function aggregateBookRisk(
  positions: BookPosition[],
  conventions: Conventions,
  transport: CelnetTransport,
  skipped: Instrument[] = [],
): Promise<BookRisk> {
  // Running accumulators.
  let total = ZERO_GREEKS;
  let grossNotional = 0;
  const byPair = new Map<string, PairBreakdown>();
  // Vega ladder keyed by "tenor|delta" so identical pillars across positions sum.
  const vegaByKey = new Map<string, AggVegaBucket>();
  // Cross-gamma keyed by "A|B".
  const cgByKey = new Map<string, AggCrossGamma>();
  // Theta roll keyed by horizon (positions share the standard horizons).
  const thetaByHorizon = new Map<number, AggThetaRoll>();

  const perPosition = await Promise.all(
    positions.map(async ({ instrument, market, pairKey }) => {
      const riskReq = riskRequestFor(instrument);
      const [baseResult, riskResult] = await Promise.all([
        transport.scenario(instrument, market, conventions, BASE_AXES),
        transport.scenario(instrument, market, conventions, BASE_AXES, riskReq),
      ]);
      const basePoint =
        baseResult.points.find((p) => p.appliedShocks.every((s) => s === 0)) ??
        baseResult.points[0];
      const baseGreeks = basePoint?.greeks ?? ZERO_GREEKS;
      const basePrice = baseGreeks.price;
      return { instrument, market, pairKey, baseGreeks, basePrice, riskResult };
    }),
  );

  for (const p of perPosition) {
    const mult = signedNotional(p.instrument);
    const absNotional = p.instrument.quantity.notional;
    grossNotional += absNotional;

    // --- book totals + per-pair net Greeks ---
    const scaled = scaleGreeks(p.baseGreeks, mult);
    total = addGreeks(total, scaled);

    const [base, quote] = p.pairKey.split("/") as [string, string];
    const pb =
      byPair.get(p.pairKey) ??
      ({
        pairKey: p.pairKey,
        base,
        quote,
        count: 0,
        netNotional: 0,
        netDelta: 0,
        netVega: 0,
        netGamma: 0,
        netTheta: 0,
      } satisfies PairBreakdown);
    pb.count += 1;
    pb.netNotional += mult;
    pb.netDelta += scaled.deltaSpot;
    pb.netVega += scaled.vega;
    pb.netGamma += scaled.gamma;
    pb.netTheta += scaled.theta;
    byPair.set(p.pairKey, pb);

    // --- aggregate vega ladder (bucketed by tenor × delta) ---
    const br = p.riskResult.bucketedRisk;
    if (br) {
      for (const vb of br.vegaBuckets) {
        // The bucketed vega is the sensitivity to a 1-vol-point move expressed as
        // a premium fraction; convert to book P&L per vol point (×notional×0.01),
        // signed by direction, consistent with the Risk workspace's ladder units.
        const vegaPerPoint = vb.vega * mult * 0.01;
        const key = `${vb.tenorYears.toFixed(6)}|${vb.delta}`;
        const cur = vegaByKey.get(key);
        if (cur) {
          cur.vegaPerPoint += vegaPerPoint;
        } else {
          vegaByKey.set(key, { tenorYears: vb.tenorYears, delta: vb.delta, vegaPerPoint });
        }
      }

      // --- aggregate cross-gamma ---
      for (const cg of br.crossGammas) {
        const key = `${cg.factorA}|${cg.factorB}`;
        const value = cg.value * mult;
        const cur = cgByKey.get(key);
        if (cur) {
          cur.value += value;
        } else {
          cgByKey.set(key, { factorA: cg.factorA, factorB: cg.factorB, value });
        }
      }

      // --- aggregate theta roll ---
      // The contract returns absolute repriced values per rolled expiry; the decay
      // P&L is (rolled − base) × signed notional. Sum across positions per horizon.
      br.thetaRoll.forEach((rolledPrice, i) => {
        const horizon = br.rollHorizonsYears[i];
        if (horizon === undefined) return;
        const pnl = (rolledPrice - p.basePrice) * mult;
        const cur = thetaByHorizon.get(horizon);
        if (cur) {
          cur.pnl += pnl;
        } else {
          thetaByHorizon.set(horizon, { horizonYears: horizon, pnl });
        }
      });
    }
  }

  const pairBreakdown = [...byPair.values()].sort(
    (a, b) => Math.abs(b.netVega) - Math.abs(a.netVega),
  );

  const vegaLadder = [...vegaByKey.values()].sort(
    (a, b) =>
      a.tenorYears - b.tenorYears || Math.abs(b.vegaPerPoint) - Math.abs(a.vegaPerPoint),
  );

  // Preserve the canonical factor-pair order for the cross-gamma panel.
  const crossGammas = CROSS_GAMMA_PAIRS.map((p) => cgByKey.get(`${p.factorA}|${p.factorB}`)).filter(
    (x): x is AggCrossGamma => x !== undefined,
  );

  const thetaRoll = [...ROLL_HORIZONS]
    .map((h) => thetaByHorizon.get(h))
    .filter((x): x is AggThetaRoll => x !== undefined);

  return {
    positionCount: perPosition.length,
    pairCount: byPair.size,
    total,
    grossNotional,
    byPair: pairBreakdown,
    vegaLadder,
    crossGammas,
    thetaRoll,
    skipped,
  };
}

/**
 * Resolve each position to the base market of its pair, splitting out positions
 * whose pair has no known market so the caller can disclose them honestly rather
 * than price against a fabricated market.
 */
export function resolveBookPositions(
  positions: Instrument[],
  marketByPair: Map<string, MarketContext>,
): { resolved: BookPosition[]; skipped: Instrument[] } {
  const resolved: BookPosition[] = [];
  const skipped: Instrument[] = [];
  for (const instrument of positions) {
    const pairKey = pairKeyOf(instrument);
    const market = marketByPair.get(pairKey);
    if (market) {
      resolved.push({ instrument, market, pairKey });
    } else {
      skipped.push(instrument);
    }
  }
  return { resolved, skipped };
}
