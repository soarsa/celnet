/**
 * PayoffChart — a live payoff-at-expiry mini-chart (GW2). Inline SVG, no chart
 * library (rule 7): a pure terminal-payoff function over a spot grid, drawn with
 * axes, a zero line, the strike marker and the current-spot marker.
 *
 * Honesty discipline (GW0): the chart only draws structures whose payoff is a
 * clean *terminal* function of the expiry spot (vanilla, the strategy templates,
 * the knock/step/touch variants priced off the terminal level). For structures
 * whose expiry value is path-dependent or otherwise not a function of the single
 * terminal spot (Asian, lookback, cliquet, TARF, accumulator, variance/vol swap,
 * forward-start, quanto, American), {@link payoffAtExpiry} returns `null` and the
 * chart renders an em-dash empty state with an honest reason — it never fabricates
 * a curve for a path-dependent product.
 *
 * The strategy / barrier strikes a real ticket resolves from the marked surface
 * (delta-defined wings, spot-offset barriers) are not available to this preview,
 * so the shape is parameterised off the supplied `strike` / `spot`: a faithful
 * *shape* of the structure (the kink/step locations and the sign of each region),
 * NOT a priced P&L. The accessible label says so.
 */
import { fmtRate } from "../lib/format";
import styles from "./PayoffChart.module.css";

/** The terminal-payoff parameters a {@link payoffAtExpiry} structure reads. */
export interface PayoffParams {
  /** The (primary) strike level — the kink/step location. */
  strike: number;
  /** The current spot, used to scale the representative wing/barrier offsets. */
  spot: number;
  /** Trade direction; `SELL` mirrors the payoff through zero. Defaults to BUY. */
  side?: "BUY" | "SELL";
}

/**
 * The set of structure ids whose expiry payoff is a clean function of the single
 * terminal spot, so a deterministic curve can be drawn. Anything not in this set
 * is honestly path-dependent (or otherwise not a terminal function) and renders
 * the empty state.
 */
const TERMINAL_STRUCTURES = new Set<string>([
  "VANILLA",
  "RISK_REVERSAL",
  "STRANGLE",
  "STRADDLE",
  "SEAGULL",
  "SINGLE_BARRIER",
  "DOUBLE_BARRIER",
  "WINDOW_BARRIER",
  "DIGITAL",
  "TOUCH",
]);

/** Human reasons for the honest empty state, per path-dependent family. */
const EMPTY_REASON: Readonly<Record<string, string>> = {
  ASIAN: "payoff depends on the average fixing — priced server-side",
  LOOKBACK: "payoff depends on the path extremum — priced server-side",
  CLIQUET: "payoff depends on per-period returns — priced server-side",
  TARF: "payoff depends on the accumulated target — priced server-side",
  ACCUMULATOR: "payoff depends on the accumulation schedule — priced server-side",
  VARIANCE_SWAP: "payoff depends on realised variance — priced server-side",
  VOLATILITY_SWAP: "payoff depends on realised volatility — priced server-side",
  FORWARD_START: "strike sets at a future fixing — priced server-side",
  QUANTO: "payoff settles in a third currency — priced server-side",
  AMERICAN: "payoff depends on the early-exercise path — priced server-side",
  BASKET: "payoff depends on the basket path — priced server-side",
  PERPETUAL: "no expiry exists — exercisable at any time; valued by the perpetual closed form",
  LISTED_FUTURE_OPTION: "payoff is on the listed future's level, not the spot axis",
};

/** A vanilla call/put intrinsic at expiry. */
function vanillaPayoff(optionType: "CALL" | "PUT", s: number, k: number): number {
  return optionType === "CALL" ? Math.max(s - k, 0) : Math.max(k - s, 0);
}

/**
 * The terminal payoff of a structure at a single expiry spot `s`, or `null` if
 * the structure is path-dependent / not a clean terminal function. Strategy and
 * barrier wing/barrier levels are parameterised off `strike` / `spot` (a faithful
 * *shape*, not a priced value). `SELL` mirrors the whole profile through zero.
 */
export function payoffAt(structureId: string, params: PayoffParams, s: number): number | null {
  if (!TERMINAL_STRUCTURES.has(structureId)) return null;
  const { strike: k, spot } = params;
  const sell = params.side === "SELL";
  const w = spot * 0.05; // representative 5% wing / barrier offset
  let v: number;
  switch (structureId) {
    case "VANILLA":
      v = vanillaPayoff("CALL", s, k);
      break;
    case "RISK_REVERSAL":
      // Long 25Δ call, short 25Δ put (wings either side of the strike).
      v = vanillaPayoff("CALL", s, k + w) - vanillaPayoff("PUT", s, k - w);
      break;
    case "STRANGLE":
      // Long OTM call + long OTM put.
      v = vanillaPayoff("CALL", s, k + w) + vanillaPayoff("PUT", s, k - w);
      break;
    case "STRADDLE":
      // Long ATM call + long ATM put.
      v = vanillaPayoff("CALL", s, k) + vanillaPayoff("PUT", s, k);
      break;
    case "SEAGULL":
      // Long 25Δ call, short 10Δ call, short 25Δ put (call-spread financed by a put).
      v =
        vanillaPayoff("CALL", s, k + w) -
        vanillaPayoff("CALL", s, k + 2 * w) -
        vanillaPayoff("PUT", s, k - w);
      break;
    case "SINGLE_BARRIER": {
      // Representative up-and-out call: the vanilla, knocked to 0 above the barrier.
      const barrier = k + w;
      v = s >= barrier ? 0 : vanillaPayoff("CALL", s, k);
      break;
    }
    case "DOUBLE_BARRIER":
    case "WINDOW_BARRIER": {
      // Representative knock-out corridor: the vanilla, alive only inside the band.
      const lower = k - w;
      const upper = k + w;
      v = s <= lower || s >= upper ? 0 : vanillaPayoff("CALL", s, k);
      break;
    }
    case "DIGITAL":
      // Cash-or-nothing call: a unit step at the strike.
      v = s > k ? 1 : 0;
      break;
    case "TOUCH":
      // One-touch (terminal view): pays the unit rebate when finishing past the barrier.
      v = s >= k + w ? 1 : 0;
      break;
    default:
      return null;
  }
  return sell ? -v : v;
}

/**
 * Sample the terminal payoff of a structure over a spot grid. Returns `null` when
 * the structure is path-dependent (the caller renders the honest empty state) so
 * a curve is never fabricated.
 */
export function payoffAtExpiry(
  structureId: string,
  params: PayoffParams,
  spotGrid: readonly number[],
): number[] | null {
  if (!TERMINAL_STRUCTURES.has(structureId)) return null;
  const out: number[] = [];
  for (const s of spotGrid) {
    const p = payoffAt(structureId, params, s);
    if (p === null) return null;
    out.push(p);
  }
  return out;
}

/** A monotone spot grid of `n` points centred on `spot` spanning ±`span` fractional. */
function buildSpotGrid(spot: number, n: number, span: number): number[] {
  const lo = spot * (1 - span);
  const hi = spot * (1 + span);
  const grid: number[] = [];
  for (let i = 0; i < n; i += 1) grid.push(lo + ((hi - lo) * i) / (n - 1));
  return grid;
}

/** A short, screen-reader description of the payoff shape for `aria-label`. */
function describeShape(structureId: string, params: PayoffParams, payoff: number[]): string {
  const dir = params.side === "SELL" ? "short" : "long";
  let min = Infinity;
  let max = -Infinity;
  for (const v of payoff) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  const shape =
    max <= 0 && min < 0
      ? "a capped-downside"
      : min >= 0 && max > 0
        ? "a limited-downside"
        : "a two-sided";
  return `${dir} ${structureId.toLowerCase().replace(/_/g, " ")} payoff at expiry — ${shape} profile around the ${fmtRate(params.strike)} strike`;
}

/** Props for the {@link PayoffChart}. */
export interface PayoffChartProps {
  /** The structure id (matches the `Structure` union / registry ids). */
  structureId: string;
  /** The (primary) strike level. */
  strike: number;
  /** The current spot. */
  spot: number;
  /** Trade direction; mirrors the payoff. Defaults to BUY. */
  side?: "BUY" | "SELL";
  /** Pixel width (defaults to a compact ticket preview). */
  width?: number;
  /** Pixel height. */
  height?: number;
}

const GRID_POINTS = 121;
const SPAN = 0.18;
const PAD = { top: 8, right: 8, bottom: 8, left: 8 };

/**
 * The payoff-at-expiry mini-chart. Renders an inline SVG curve for terminal-payoff
 * structures, or an honest em-dash empty state (with a per-family reason) for the
 * path-dependent families.
 */
export function PayoffChart({
  structureId,
  strike,
  spot,
  side,
  width = 240,
  height = 120,
}: PayoffChartProps): React.ReactElement {
  const params: PayoffParams = { strike, spot, ...(side !== undefined ? { side } : {}) };
  const grid = buildSpotGrid(spot, GRID_POINTS, SPAN);
  const payoff = payoffAtExpiry(structureId, params, grid);

  if (payoff === null || !Number.isFinite(spot) || spot <= 0 || !Number.isFinite(strike)) {
    const reason =
      EMPTY_REASON[structureId] ?? "payoff depends on the path — priced server-side";
    return (
      <div
        className={styles.empty}
        role="img"
        aria-label={`payoff chart unavailable — ${reason}`}
        title={reason}
      >
        <span className={`num ${styles.dash}`} aria-hidden="true">
          —
        </span>
        <span className={styles.reason}>{reason}</span>
      </div>
    );
  }

  // Plot extent: the spot grid on x, the payoff (padded to include 0) on y.
  const xMin = grid[0]!;
  const xMax = grid[grid.length - 1]!;
  let yMin = 0;
  let yMax = 0;
  for (const v of payoff) {
    if (v < yMin) yMin = v;
    if (v > yMax) yMax = v;
  }
  if (yMin === yMax) {
    // A flat profile (e.g. an all-zero region) — give the axis a unit of room.
    yMin -= 1;
    yMax += 1;
  }

  const innerW = width - PAD.left - PAD.right;
  const innerH = height - PAD.top - PAD.bottom;
  const xPix = (x: number): number => PAD.left + ((x - xMin) / (xMax - xMin)) * innerW;
  const yPix = (y: number): number => PAD.top + innerH - ((y - yMin) / (yMax - yMin)) * innerH;

  const path = grid
    .map((x, i) => `${i === 0 ? "M" : "L"}${xPix(x).toFixed(2)},${yPix(payoff[i]!).toFixed(2)}`)
    .join(" ");

  const zeroY = yPix(0);
  const strikeIn = strike >= xMin && strike <= xMax;
  const spotIn = spot >= xMin && spot <= xMax;
  const label = describeShape(structureId, params, payoff);

  return (
    <svg
      className={styles.chart}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={label}
    >
      {/* plot frame */}
      <rect
        x={PAD.left}
        y={PAD.top}
        width={innerW}
        height={innerH}
        className={styles.frame}
      />
      {/* zero P&L line */}
      <line
        x1={PAD.left}
        x2={PAD.left + innerW}
        y1={zeroY}
        y2={zeroY}
        className={styles.zeroLine}
      />
      {/* strike marker */}
      {strikeIn && (
        <line
          x1={xPix(strike)}
          x2={xPix(strike)}
          y1={PAD.top}
          y2={PAD.top + innerH}
          className={styles.strikeLine}
        />
      )}
      {/* current-spot marker */}
      {spotIn && (
        <line
          x1={xPix(spot)}
          x2={xPix(spot)}
          y1={PAD.top}
          y2={PAD.top + innerH}
          className={styles.spotLine}
        />
      )}
      {/* payoff curve */}
      <path d={path} className={styles.curve} />
    </svg>
  );
}
