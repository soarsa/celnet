/**
 * PayoffDiagram — a multi-leg option-strategy payoff-at-expiry diagram (visx).
 *
 * Renders the combined terminal P&L of a signed set of vanilla legs against the
 * underlier at expiry, in the same honest idiom as {@link ../products/PayoffChart}:
 *
 *   • a BOLD net-payoff line (the signed leg sum) — the primary series,
 *   • a dotted, smoothed "today / MTM" curve (an illustrative time-value shape —
 *     the risk-neutral value is the terminal payoff convolved with the transition
 *     density, so a Gaussian-in-spot smoothing of the net payoff is a faithful
 *     first-order *shape*, NOT a priced mark; it is labelled illustrative),
 *   • light dashed per-leg lines (each leg's own premium-adjusted payoff; they sum
 *     to the net line),
 *   • profit / loss regions shaded with the diverging dataviz palette
 *     (--div-pos1 warm = profit, --div-neg2 cool = loss),
 *   • strike (kink), live-spot and break-even markers,
 *   • an auto-derived strategy name (straddle / strangle / risk-reversal / call- or
 *     put-spread / seagull / collar / butterfly / iron-condor / …, else "Custom").
 *
 * Honesty discipline: the diagram only draws structures whose expiry value is a
 * clean function of the single terminal spot. If ANY leg is path-dependent (Asian,
 * lookback, barrier, cliquet, accumulator, TARF, variance) the component renders an
 * honest empty state with a per-family reason instead of fabricating a curve.
 *
 * All quantitative colour comes from the dataviz tokens (--seq-* sequential,
 * --div-* diverging); chrome (chip/spot marker) uses the semantic brand/accent
 * tokens — never raw hex. Colours are consumed directly as `fill`/`stroke` CSS
 * vars because visx renders SVG. The mount transition honours prefers-reduced-motion.
 */

import { useEffect, useId, useMemo, useState } from "react";
import { AxisBottom, AxisLeft } from "@visx/axis";
import { curveLinear, curveMonotoneX } from "@visx/curve";
import { GridRows } from "@visx/grid";
import { Group } from "@visx/group";
import { ParentSize } from "@visx/responsive";
import { scaleLinear } from "@visx/scale";
import { Area, Line, LinePath } from "@visx/shape";

/* ------------------------------------------------------------------ domain */

/** Direction of a leg: `long` (+ sign) or `short` (− sign). */
export type LegSide = "long" | "short";

/** Families whose expiry payoff is a clean function of the single terminal spot. */
export type TerminalKind = "call" | "put" | "forward";

/** Path-dependent / non-terminal families — the diagram cannot honestly draw them. */
export type PathDependentKind =
  | "asian"
  | "lookback"
  | "barrier"
  | "cliquet"
  | "accumulator"
  | "tarf"
  | "variance-swap";

export type LegKind = TerminalKind | PathDependentKind;

/** One leg of a structure. Premium is a per-unit price (≥ 0); its sign comes from `side`. */
export interface PayoffLeg {
  readonly kind: LegKind;
  readonly side: LegSide;
  /** Strike (options) or entry / forward level (forward leg). */
  readonly strike: number;
  /** Leg ratio / multiplicity (e.g. the 2 in a 1-2-1 butterfly). Defaults to 1. */
  readonly quantity?: number;
  /** Per-unit option premium in spot terms (paid if long, received if short). Defaults to 0. */
  readonly premium?: number;
}

export interface PayoffDiagramProps {
  /** The signed leg set. */
  readonly legs: readonly PayoffLeg[];
  /** Current underlier spot — anchors the live marker and the % notional scaling. */
  readonly spot: number;
  /** Fixed pixel width; when omitted the diagram fills its container (visx ParentSize). */
  readonly width?: number;
  /** Pixel height. Defaults to 300. */
  readonly height?: number;
  /** Draw the light per-leg payoff lines. Defaults to true. */
  readonly showLegs?: boolean;
  /** Draw the smoothed illustrative today / MTM curve. Defaults to true. */
  readonly showToday?: boolean;
  /** ATM vol × √T used ONLY to shape the illustrative today curve. Defaults to 0.03. */
  readonly timeValueVol?: number;
  /** Human title override; otherwise the strategy is auto-derived. */
  readonly title?: string;
  /** Format a spot value for the x-axis + markers. */
  readonly formatSpot?: (v: number) => string;
  /** Format a P&L percentage for the y-axis + summary. */
  readonly formatPct?: (v: number) => string;
}

/* --------------------------------------------------------------- constants */

const GRID_N = 241;
const DEFAULT_TIME_VOL = 0.03;
/** Sequential dataviz tokens cycled across the per-leg lines (never brand hues). */
const LEG_PALETTE = ["var(--seq-1)", "var(--seq-2)", "var(--seq-4)", "var(--seq-6)"] as const;

const TERMINAL_KINDS: ReadonlySet<LegKind> = new Set<LegKind>(["call", "put", "forward"]);

/** Honest reasons for the empty state, per path-dependent family. */
const PATH_REASON: Readonly<Record<PathDependentKind, string>> = {
  asian: "an averaging (Asian) leg settles on the average fixing, not the terminal spot",
  lookback: "a lookback leg settles on the path extremum, not the terminal spot",
  barrier: "a barrier leg's survival is path-dependent, not a function of the terminal spot",
  cliquet: "a cliquet leg settles on per-period returns, not the terminal spot",
  accumulator: "an accumulator leg settles on its accumulation schedule, not the terminal spot",
  tarf: "a target-redemption leg settles on the accumulated target, not the terminal spot",
  "variance-swap": "a variance leg settles on realised variance, not the terminal spot",
};

const KEYFRAMES = `
@keyframes pdEnter { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
.pd-enter { animation: pdEnter 560ms cubic-bezier(0.22, 1, 0.36, 1) both; }
@media (prefers-reduced-motion: reduce) { .pd-enter { animation: none; } }
`;

const defaultFormatSpot = (v: number): string => (Math.abs(v) >= 100 ? v.toFixed(2) : v.toFixed(4));
const defaultFormatPct = (v: number): string =>
  `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v).toFixed(2)}%`;

/* ------------------------------------------------------------- payoff math */

interface PayoffPoint {
  s: number;
  netPct: number;
  mtmPct: number;
}
interface LegPoint {
  s: number;
  pct: number;
}
interface LegLine {
  tag: string;
  color: string;
  pts: LegPoint[];
}
interface StrikeMark {
  s: number;
  tag: string;
}
interface StrategyMeta {
  name: string;
  kind: string;
  custom: boolean;
}
interface PayoffModel {
  spot: number;
  domainX: [number, number];
  domainY: [number, number];
  points: PayoffPoint[];
  legLines: LegLine[];
  strikes: StrikeMark[];
  breakevens: number[];
  strategy: StrategyMeta;
  netPremiumPct: number;
  maxProfit: { value: number; unbounded: boolean };
  maxLoss: { value: number; unbounded: boolean };
}
type BuildResult = { ok: true; model: PayoffModel } | { ok: false; reason: string };

const legSign = (side: LegSide): 1 | -1 => (side === "long" ? 1 : -1);

/** A vanilla/forward intrinsic at a single expiry spot. */
function terminalIntrinsic(kind: TerminalKind, s: number, k: number): number {
  switch (kind) {
    case "call":
      return Math.max(s - k, 0);
    case "put":
      return Math.max(k - s, 0);
    case "forward":
      return s - k;
  }
}

/** A single leg's premium-adjusted P&L (price terms) at expiry spot `s`. */
function legPnlPrice(leg: PayoffLeg, s: number): number {
  const q = leg.quantity ?? 1;
  const prem = leg.premium ?? 0;
  return legSign(leg.side) * q * (terminalIntrinsic(leg.kind as TerminalKind, s, leg.strike) - prem);
}

/** Short marker tag for a leg, e.g. `+C`, `−P`, `−C×2`, `+F`. */
function legTag(leg: PayoffLeg): string {
  const sign = leg.side === "long" ? "+" : "−";
  const right = leg.kind === "call" ? "C" : leg.kind === "put" ? "P" : "F";
  const q = leg.quantity && leg.quantity !== 1 ? `×${leg.quantity}` : "";
  return `${sign}${right}${q}`;
}

function linspace(lo: number, hi: number, n: number): number[] {
  const out = new Array<number>(n);
  const step = (hi - lo) / (n - 1);
  for (let i = 0; i < n; i += 1) out[i] = lo + step * i;
  return out;
}

/**
 * Gaussian-in-spot smoothing of the terminal payoff — the illustrative today curve.
 * O(n²) over the fixed grid (~58k ops); cheap and deterministic.
 */
function gaussianSmooth(xs: number[], ys: number[], sigma: number): number[] {
  const n = xs.length;
  if (sigma <= 0) return ys.slice();
  const inv = 1 / (2 * sigma * sigma);
  const out = new Array<number>(n);
  for (let i = 0; i < n; i += 1) {
    let wsum = 0;
    let vsum = 0;
    const xi = xs[i]!;
    for (let j = 0; j < n; j += 1) {
      const dx = xi - xs[j]!;
      const w = Math.exp(-dx * dx * inv);
      wsum += w;
      vsum += w * ys[j]!;
    }
    out[i] = wsum > 0 ? vsum / wsum : ys[i]!;
  }
  return out;
}

/** Zero-crossings of the net payoff (linearly interpolated), deduped. */
function findBreakevens(points: PayoffPoint[]): number[] {
  const raw: number[] = [];
  for (let i = 0; i < points.length - 1; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (a.netPct === 0) {
      raw.push(a.s);
      continue;
    }
    if (a.netPct < 0 !== b.netPct < 0) {
      const t = a.netPct / (a.netPct - b.netPct);
      raw.push(a.s + t * (b.s - a.s));
    }
  }
  const last = points[points.length - 1];
  if (last && last.netPct === 0) raw.push(last.s);
  const uniq: number[] = [];
  for (const s of raw) {
    const tol = (Math.abs(s) || 1) * 1e-4;
    if (!uniq.some((u) => Math.abs(u - s) < tol)) uniq.push(s);
  }
  return uniq;
}

/** Unique strike markers within the plot window, tag-labelled by their legs. */
function buildStrikeMarks(legs: PayoffLeg[], lo: number, hi: number): StrikeMark[] {
  const groups = new Map<number, PayoffLeg[]>();
  for (const leg of legs) {
    if (leg.strike < lo || leg.strike > hi) continue;
    const arr = groups.get(leg.strike) ?? [];
    arr.push(leg);
    groups.set(leg.strike, arr);
  }
  return Array.from(groups.entries())
    .sort((a, b) => a[0] - b[0])
    .map(([s, ls]) => ({ s, tag: ls.map(legTag).join(" ") }));
}

/**
 * Auto-derive the strategy name from the (terminal-only) leg set. A pragmatic
 * recognizer covering the standard FX-options families; anything unrecognized is an
 * honest "Custom N-leg" signed leg sum.
 */
function deriveStrategy(legs: PayoffLeg[]): StrategyMeta {
  const n = legs.length;
  const calls = legs.filter((l) => l.kind === "call").sort((a, b) => a.strike - b.strike);
  const puts = legs.filter((l) => l.kind === "put").sort((a, b) => a.strike - b.strike);
  const fwds = legs.filter((l) => l.kind === "forward");
  const auto = (name: string, kind: string): StrategyMeta => ({ name, kind, custom: false });
  const custom = (name: string, kind: string): StrategyMeta => ({ name, kind, custom: true });

  if (n === 1) {
    const l = legs[0]!;
    const dir = l.side === "long" ? "Long" : "Short";
    if (l.kind === "forward") return auto(`${dir} Forward`, "FORWARD");
    const right = l.kind === "call" ? "Call" : "Put";
    return auto(`${dir} ${right}`, `VANILLA_${right.toUpperCase()}`);
  }

  if (n === 2 && fwds.length === 0) {
    if (calls.length === 1 && puts.length === 1) {
      const c = calls[0]!;
      const p = puts[0]!;
      if (c.side === "long" && p.side === "long") {
        return c.strike === p.strike ? auto("Straddle", "STRADDLE") : auto("Strangle", "STRANGLE");
      }
      if (c.side === "short" && p.side === "short") {
        return c.strike === p.strike
          ? auto("Short Straddle", "STRADDLE")
          : auto("Short Strangle", "STRANGLE");
      }
      return auto("Risk Reversal", "RISK_REVERSAL");
    }
    if (calls.length === 2) return custom("Call Spread", "CALL_SPREAD");
    if (puts.length === 2) return custom("Put Spread", "PUT_SPREAD");
  }

  if (n === 3 && fwds.length === 1 && calls.length === 1 && puts.length === 1) {
    return custom("Collar", "COLLAR");
  }
  if (n === 3 && fwds.length === 0) {
    if ((calls.length === 2 && puts.length === 1) || (puts.length === 2 && calls.length === 1)) {
      return auto("Seagull", "SEAGULL");
    }
    if (calls.length === 3 || puts.length === 3) return custom("Butterfly", "BUTTERFLY");
  }

  if (n === 4 && calls.length === 2 && puts.length === 2) return custom("Iron Condor", "IRON_CONDOR");
  if (n === 4 && (calls.length === 4 || puts.length === 4)) return custom("Condor", "CONDOR");

  return custom(`Custom ${n}-leg`, "CUSTOM");
}

/** Assemble the full render model, or an honest empty-state reason. */
function buildModel(
  legs: readonly PayoffLeg[],
  spot: number,
  timeValueVol: number,
): BuildResult {
  if (!Number.isFinite(spot) || spot <= 0) {
    return { ok: false, reason: "a positive spot is required to place the payoff axes" };
  }
  if (legs.length === 0) {
    return { ok: false, reason: "add one or more legs to draw a payoff" };
  }
  const pathLegs = legs.filter((l) => !TERMINAL_KINDS.has(l.kind));
  if (pathLegs.length > 0) {
    const kinds = Array.from(new Set(pathLegs.map((l) => l.kind))) as PathDependentKind[];
    const reason = kinds.map((k) => PATH_REASON[k]).join("; ");
    return { ok: false, reason: `payoff-at-expiry is undrawn — ${reason}; priced server-side` };
  }
  const terminal = legs as readonly PayoffLeg[]; // all terminal at this point
  if (!terminal.every((l) => Number.isFinite(l.strike))) {
    return { ok: false, reason: "every leg needs a finite strike" };
  }

  const strikes = terminal.map((l) => l.strike);
  const kMin = Math.min(spot, ...strikes);
  const kMax = Math.max(spot, ...strikes);
  const pad = Math.max((kMax - kMin) * 0.35, spot * 0.06);
  const lo = Math.max(kMin - pad, 0);
  const hi = kMax + pad;

  const sVals = linspace(lo, hi, GRID_N);
  const netPrice = sVals.map((s) => terminal.reduce((acc, l) => acc + legPnlPrice(l, s), 0));
  const sigma = Math.max(spot * timeValueVol, (hi - lo) / GRID_N);
  const mtmPrice = gaussianSmooth(sVals, netPrice, sigma);
  const toPct = (p: number): number => (p / spot) * 100;

  const points: PayoffPoint[] = sVals.map((s, i) => ({
    s,
    netPct: toPct(netPrice[i]!),
    mtmPct: toPct(mtmPrice[i]!),
  }));

  const legLines: LegLine[] = terminal.map((l, idx) => ({
    tag: legTag(l),
    color: LEG_PALETTE[idx % LEG_PALETTE.length]!,
    pts: sVals.map((s) => ({ s, pct: toPct(legPnlPrice(l, s)) })),
  }));

  let yMin = 0;
  let yMax = 0;
  for (const p of points) {
    yMin = Math.min(yMin, p.netPct, p.mtmPct);
    yMax = Math.max(yMax, p.netPct, p.mtmPct);
  }
  const yPad = (yMax - yMin || 1) * 0.15;
  const domainY: [number, number] = [yMin - yPad, yMax + yPad];

  const breakevens = findBreakevens(points);
  const strikeMarks = buildStrikeMarks([...terminal], lo, hi);
  const strategy = deriveStrategy([...terminal]);

  const netPremiumPrice = terminal.reduce(
    (acc, l) => acc + -legSign(l.side) * (l.quantity ?? 1) * (l.premium ?? 0),
    0,
  );

  const netPct = points.map((p) => p.netPct);
  const count = netPct.length;
  let maxV = -Infinity;
  let minV = Infinity;
  for (const v of netPct) {
    if (v > maxV) maxV = v;
    if (v < minV) minV = v;
  }
  const eps = 1e-6;
  const dRight = count >= 2 ? netPct[count - 1]! - netPct[count - 2]! : 0;
  const dLeft = count >= 2 ? netPct[0]! - netPct[1]! : 0;
  const maxProfit = { value: maxV, unbounded: dRight > eps || dLeft > eps };
  const maxLoss = { value: minV, unbounded: dRight < -eps || dLeft < -eps };

  return {
    ok: true,
    model: {
      spot,
      domainX: [lo, hi],
      domainY,
      points,
      legLines,
      strikes: strikeMarks,
      breakevens,
      strategy,
      netPremiumPct: toPct(netPremiumPrice),
      maxProfit,
      maxLoss,
    },
  };
}

/* --------------------------------------------------------------- a11y hook */

function usePrefersReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState<boolean>(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia(query).matches
      : false,
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mql = window.matchMedia(query);
    const onChange = (): void => setReduced(mql.matches);
    onChange();
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

/* ------------------------------------------------------------- subcomponents */

function Stat({
  label,
  value,
  color,
}: {
  label: string;
  value: string;
  color?: string;
}): React.ReactElement {
  return (
    <span style={{ display: "inline-flex", gap: "var(--space-2)", alignItems: "baseline" }}>
      <span
        style={{
          fontSize: "var(--type-micro)",
          textTransform: "uppercase",
          letterSpacing: "0.05em",
          color: "var(--text-tertiary)",
          fontWeight: 700,
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: "var(--type-callout)",
          fontVariantNumeric: "tabular-nums",
          color: color ?? "var(--text-secondary)",
        }}
      >
        {value}
      </span>
    </span>
  );
}

function LegendSwatch({
  color,
  label,
  filled,
  dashed,
  dot,
}: {
  color: string;
  label: string;
  filled?: boolean;
  dashed?: boolean;
  dot?: boolean;
}): React.ReactElement {
  const mark = dot ? (
    <span
      style={{ display: "inline-block", width: 8, height: 8, borderRadius: "50%", background: color }}
    />
  ) : filled ? (
    <span
      style={{ display: "inline-block", width: 11, height: 9, background: color, opacity: 0.6 }}
    />
  ) : (
    <span
      style={{
        display: "inline-block",
        width: 15,
        height: 0,
        borderTop: `2px ${dashed ? "dashed" : "solid"} ${color}`,
      }}
    />
  );
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-2)" }}>
      {mark}
      {label}
    </span>
  );
}

/* ---------------------------------------------------------------- SVG body */

interface PayoffCanvasProps {
  width: number;
  height: number;
  model: PayoffModel;
  clipId: string;
  animate: boolean;
  showLegs: boolean;
  showToday: boolean;
  formatSpot: (v: number) => string;
  formatPct: (v: number) => string;
  ariaLabel: string;
}

function PayoffCanvas({
  width,
  height,
  model,
  clipId,
  animate,
  showLegs,
  showToday,
  formatSpot,
  formatPct,
  ariaLabel,
}: PayoffCanvasProps): React.ReactElement | null {
  const margin = { top: 20, right: 22, bottom: 38, left: 58 };
  const innerW = width - margin.left - margin.right;
  const innerH = height - margin.top - margin.bottom;
  if (innerW <= 0 || innerH <= 0) return null;

  const xScale = scaleLinear<number>({ domain: model.domainX, range: [0, innerW] });
  const yScale = scaleLinear<number>({ domain: model.domainY, range: [innerH, 0] });
  const zeroY = yScale(0);
  const { points, legLines, strikes, breakevens, spot, domainX } = model;
  const spotIn = spot >= domainX[0] && spot <= domainX[1];

  const tickBase = { fill: "var(--text-tertiary)", fontFamily: "var(--font-mono)", fontSize: 10 };

  return (
    <svg
      width={width}
      height={height}
      role="img"
      aria-label={ariaLabel}
      style={{
        display: "block",
        width: "100%",
        borderRadius: "var(--r-sm)",
        background: "var(--bg-inset)",
      }}
    >
      <style>{KEYFRAMES}</style>
      <Group left={margin.left} top={margin.top}>
        <defs>
          <clipPath id={clipId}>
            <rect x={0} y={0} width={innerW} height={innerH} />
          </clipPath>
        </defs>

        <GridRows scale={yScale} width={innerW} numTicks={5} stroke="var(--grid-line)" strokeOpacity={0.5} />

        {/* profit / loss shaded regions (diverging dataviz palette) */}
        <Area
          data={points}
          curve={curveLinear}
          x={(d: PayoffPoint) => xScale(d.s)}
          y0={zeroY}
          y1={(d: PayoffPoint) => yScale(Math.max(d.netPct, 0))}
          fill="var(--div-pos1)"
          fillOpacity={0.18}
          stroke="none"
        />
        <Area
          data={points}
          curve={curveLinear}
          x={(d: PayoffPoint) => xScale(d.s)}
          y0={zeroY}
          y1={(d: PayoffPoint) => yScale(Math.min(d.netPct, 0))}
          fill="var(--div-neg2)"
          fillOpacity={0.18}
          stroke="none"
        />

        {/* zero P&L line */}
        <Line
          from={{ x: 0, y: zeroY }}
          to={{ x: innerW, y: zeroY }}
          stroke="var(--text-tertiary)"
          strokeOpacity={0.55}
          strokeWidth={1}
        />

        {/* strike kink markers */}
        {strikes.map((k) => (
          <g key={`k-${k.s}`}>
            <Line
              from={{ x: xScale(k.s), y: 0 }}
              to={{ x: xScale(k.s), y: innerH }}
              stroke="var(--text-tertiary)"
              strokeOpacity={0.5}
              strokeDasharray="2 3"
              strokeWidth={1}
            />
            <text
              x={xScale(k.s)}
              y={-7}
              textAnchor="middle"
              style={{ fill: "var(--text-secondary)", fontFamily: "var(--font-mono)", fontSize: 9 }}
            >
              {k.tag} {formatSpot(k.s)}
            </text>
          </g>
        ))}

        {/* animated data group (net / today / per-leg) */}
        <g className={animate ? "pd-enter" : undefined}>
          {showLegs && (
            <g clipPath={`url(#${clipId})`}>
              {legLines.map((ll, i) => (
                <LinePath
                  key={`leg-${i}`}
                  data={ll.pts}
                  curve={curveLinear}
                  x={(d: LegPoint) => xScale(d.s)}
                  y={(d: LegPoint) => yScale(d.pct)}
                  stroke={ll.color}
                  strokeWidth={1}
                  strokeDasharray="3 3"
                  strokeOpacity={0.5}
                  fill="none"
                />
              ))}
            </g>
          )}
          {showToday && (
            <LinePath
              data={points}
              curve={curveMonotoneX}
              x={(d: PayoffPoint) => xScale(d.s)}
              y={(d: PayoffPoint) => yScale(d.mtmPct)}
              stroke="var(--seq-3)"
              strokeWidth={1.5}
              strokeDasharray="1 4"
              strokeLinecap="round"
              fill="none"
            />
          )}
          <LinePath
            data={points}
            curve={curveLinear}
            x={(d: PayoffPoint) => xScale(d.s)}
            y={(d: PayoffPoint) => yScale(d.netPct)}
            stroke="var(--seq-5)"
            strokeWidth={2.5}
            strokeLinejoin="round"
            strokeLinecap="round"
            fill="none"
          />
        </g>

        {/* live-spot marker (brand accent — chrome, not a data encoding) */}
        {spotIn && (
          <g>
            <Line
              from={{ x: xScale(spot), y: 0 }}
              to={{ x: xScale(spot), y: innerH }}
              stroke="var(--brand)"
              strokeOpacity={0.7}
              strokeWidth={1}
              strokeDasharray="2 2"
            />
            <circle cx={xScale(spot)} cy={innerH} r={2.6} fill="var(--brand)" />
            <text
              x={xScale(spot) + 4}
              y={innerH - 5}
              style={{ fill: "var(--brand)", fontFamily: "var(--font-mono)", fontSize: 9 }}
            >
              spot {formatSpot(spot)}
            </text>
          </g>
        )}

        {/* break-even markers */}
        {breakevens.map((be) => (
          <g key={`be-${be}`}>
            <circle
              cx={xScale(be)}
              cy={zeroY}
              r={3}
              fill="var(--bg-raised)"
              stroke="var(--brand)"
              strokeWidth={1.5}
            />
            <text
              x={xScale(be)}
              y={zeroY - 8}
              textAnchor="middle"
              style={{ fill: "var(--brand)", fontFamily: "var(--font-mono)", fontSize: 9 }}
            >
              b/e {formatSpot(be)}
            </text>
          </g>
        ))}

        <AxisBottom
          top={innerH}
          scale={xScale}
          numTicks={6}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickFormat={(v) => formatSpot(Number(v))}
          tickLabelProps={() => ({ ...tickBase, textAnchor: "middle", dy: "0.25em" })}
        />
        <AxisLeft
          scale={yScale}
          numTicks={5}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickFormat={(v) => formatPct(Number(v))}
          tickLabelProps={() => ({ ...tickBase, textAnchor: "end", dx: "-0.25em", dy: "0.25em" })}
        />

        <text
          x={innerW}
          y={innerH + 32}
          textAnchor="end"
          style={{
            fill: "var(--text-tertiary)",
            fontFamily: "var(--font-display)",
            fontSize: 9,
            letterSpacing: "0.04em",
          }}
        >
          underlier at expiry
        </text>
        <text
          x={0}
          y={-8}
          textAnchor="start"
          style={{
            fill: "var(--text-tertiary)",
            fontFamily: "var(--font-display)",
            fontSize: 9,
            letterSpacing: "0.04em",
          }}
        >
          P&amp;L · % of spot notional
        </text>
      </Group>
    </svg>
  );
}

/* ------------------------------------------------------------------ shell */

const figureStyle: React.CSSProperties = {
  margin: 0,
  width: "100%",
  display: "flex",
  flexDirection: "column",
  gap: "var(--space-3)",
  fontFamily: "var(--font-ui)",
  color: "var(--text-primary)",
};
const capStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "baseline",
  flexWrap: "wrap",
  gap: "var(--space-3)",
  margin: 0,
};
const nameStyle: React.CSSProperties = {
  fontFamily: "var(--font-display)",
  fontWeight: 600,
  fontSize: 15,
  color: "var(--text-primary)",
};
const chipStyle: React.CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: "var(--type-micro)",
  textTransform: "uppercase",
  letterSpacing: "0.04em",
  padding: "2px 7px",
  borderRadius: "var(--r-sm)",
  background: "var(--accent-soft)",
  color: "var(--accent)",
  fontWeight: 700,
};
const tagAutoStyle: React.CSSProperties = {
  fontSize: 8,
  textTransform: "uppercase",
  letterSpacing: "0.04em",
  fontWeight: 700,
  padding: "2px 5px",
  borderRadius: "var(--r-sm)",
  background: "var(--brand-soft)",
  color: "var(--brand)",
};
const tagCustomStyle: React.CSSProperties = {
  ...tagAutoStyle,
  background: "oklch(1 0 0 / 0.06)",
  color: "var(--text-tertiary)",
};
const statsWrapStyle: React.CSSProperties = {
  display: "flex",
  gap: "var(--space-4)",
  marginLeft: "auto",
  flexWrap: "wrap",
};
const legendStyle: React.CSSProperties = {
  display: "flex",
  gap: "var(--space-5)",
  flexWrap: "wrap",
  margin: 0,
  fontSize: "var(--type-caption)",
  color: "var(--text-tertiary)",
};
const emptyStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  justifyContent: "center",
  gap: "var(--space-2)",
  minHeight: 180,
  padding: "var(--space-4)",
  border: "1px solid var(--grid-line)",
  borderRadius: "var(--r-sm)",
  background: "var(--bg-inset)",
  textAlign: "center",
};

/**
 * The multi-leg payoff-at-expiry diagram. Renders the strategy header + a visx SVG
 * body for terminal-payoff structures, or an honest empty state for path-dependent
 * legs. Width fills the container unless a fixed `width` is supplied.
 */
export function PayoffDiagram(props: PayoffDiagramProps): React.ReactElement {
  const {
    legs,
    spot,
    width,
    height = 300,
    showLegs = true,
    showToday = true,
    timeValueVol = DEFAULT_TIME_VOL,
    title,
    formatSpot = defaultFormatSpot,
    formatPct = defaultFormatPct,
  } = props;

  const reduced = usePrefersReducedMotion();
  const clipId = `pd-clip-${useId().replace(/:/g, "")}`;
  const result = useMemo(() => buildModel(legs, spot, timeValueVol), [legs, spot, timeValueVol]);

  if (!result.ok) {
    const label = `payoff diagram unavailable — ${result.reason}`;
    return (
      <figure style={figureStyle} aria-label={label}>
        <div role="img" aria-label={label} title={result.reason} style={emptyStyle}>
          <span
            aria-hidden
            style={{
              color: "var(--text-tertiary)",
              fontFamily: "var(--font-mono)",
              fontSize: "var(--type-title)",
              lineHeight: 1,
            }}
          >
            —
          </span>
          <span
            style={{
              color: "var(--text-tertiary)",
              fontSize: "var(--type-caption)",
              lineHeight: "var(--type-caption-lh)",
              maxWidth: "44ch",
            }}
          >
            {result.reason}
          </span>
        </div>
      </figure>
    );
  }

  const m = result.model;
  const name = title ?? m.strategy.name;
  const netLabel = `${formatPct(m.netPremiumPct)} ${m.netPremiumPct >= 0 ? "credit" : "debit"}`;
  const beLabel = m.breakevens.length ? m.breakevens.map(formatSpot).join(" · ") : "—";
  const maxProfitLabel = m.maxProfit.unbounded ? "unbounded" : formatPct(m.maxProfit.value);
  const maxLossLabel = m.maxLoss.unbounded ? "unbounded" : formatPct(m.maxLoss.value);
  const ariaLabel = `${name} payoff at expiry — net ${netLabel}${
    m.breakevens.length ? `, breakeven ${beLabel}` : ""
  }; profit and loss regions shaded; sample data`;

  const canvasProps = {
    height,
    model: m,
    clipId,
    animate: !reduced,
    showLegs,
    showToday,
    formatSpot,
    formatPct,
    ariaLabel,
  };

  return (
    <figure style={figureStyle} aria-label={ariaLabel}>
      <style>{KEYFRAMES}</style>
      <figcaption style={capStyle}>
        <span style={nameStyle}>{name}</span>
        <span style={chipStyle}>kind · {m.strategy.kind}</span>
        <span style={m.strategy.custom ? tagCustomStyle : tagAutoStyle}>
          {m.strategy.custom ? "custom" : "auto"}
        </span>
        <span style={statsWrapStyle}>
          <Stat
            label="net"
            value={netLabel}
            color={m.netPremiumPct >= 0 ? "var(--div-pos1)" : "var(--div-neg2)"}
          />
          <Stat label="b/e" value={beLabel} />
          <Stat label="max +" value={maxProfitLabel} color="var(--div-pos1)" />
          <Stat label="max −" value={maxLossLabel} color="var(--div-neg2)" />
        </span>
      </figcaption>

      <div style={{ width: "100%", height }}>
        {width != null ? (
          <PayoffCanvas width={width} {...canvasProps} />
        ) : (
          <ParentSize>
            {({ width: w }) => (w > 0 ? <PayoffCanvas width={w} {...canvasProps} /> : null)}
          </ParentSize>
        )}
      </div>

      <div style={legendStyle} aria-hidden>
        <LegendSwatch color="var(--div-pos1)" filled label="profit" />
        <LegendSwatch color="var(--div-neg2)" filled label="loss" />
        <LegendSwatch color="var(--seq-5)" label="net payoff" />
        {showToday && <LegendSwatch color="var(--seq-3)" dashed label="today · illustrative" />}
        {showLegs && <LegendSwatch color="var(--text-tertiary)" dashed label="per-leg" />}
        <LegendSwatch color="var(--brand)" dot label="spot / break-even" />
      </div>
    </figure>
  );
}
