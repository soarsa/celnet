/**
 * VolSmile — the 2D implied-volatility smile slice (mockup 01, the "Smile · 1M"
 * linked panel). A @visx SVG chart plotting implied vol against the delta / wing
 * axis (puts left · ATM centre · calls right): market pillar marks with a real
 * bid/ask BAND, the fitted model curve, ATM & 25Δ/10Δ reference markers with the
 * RR/BF read-out, and an optional multi-tenor family overlay.
 *
 * Colour discipline (design tokens): ALL quantitative colour comes from the
 * Viridis sequential dataviz palette (--seq-1..6) — tenors map onto the ramp in
 * order (short → long), and market marks / fitted curve / bid-ask band of a tenor
 * all share that tenor's ramp colour, distinguished by GLYPH (● marks · — fit ·
 * translucent band) rather than by hue, so the ordered ramp keeps its meaning.
 * The signed RR read-out is tinted by the diverging palette (--div-neg2/-pos2).
 * Brand coral/indigo is NEVER used for data — only chrome uses --text/--grid.
 *
 * Honesty discipline: `x` is the plotting coordinate the caller supplies (signed
 * delta, log-moneyness, or an evenly-spaced wing index) and `label` carries the
 * delta semantics; RR/BF and the wing markers are derived ONLY from recognizable
 * pillar labels (10Δ/25Δ/ATM), so they are shown only when actually computable and
 * never fabricated. A tenor with no drawable geometry renders an honest empty state
 * instead of an invented curve.
 *
 * SVG throughout, so CSS custom properties are consumed directly as `var(--seq-3)`
 * (no getComputedStyle needed — that is only for canvas/WebGL). prefers-reduced-
 * motion is honoured (the mount fade-in is dropped). Width fills the container via
 * @visx ParentSize; height is a prop.
 */

import { useEffect, useId, useMemo, useState } from "react";
import { AxisBottom, AxisLeft } from "@visx/axis";
import { curveNatural } from "@visx/curve";
import { LinearGradient } from "@visx/gradient";
import { GridRows } from "@visx/grid";
import { Group } from "@visx/group";
import { ParentSize } from "@visx/responsive";
import { scaleLinear } from "@visx/scale";
import { Area, LinePath } from "@visx/shape";
import { TooltipWithBounds, defaultStyles, useTooltip } from "@visx/tooltip";

/* ─────────────────────────── public data model ─────────────────────────── */

/** A continuous fitted-smile model: implied vol (vol points) as a fn of the wing `x`. */
export type SmileFit = (x: number) => number;

/** A discrete curve/pillar point in (wing coordinate, vol points). */
export interface CurvePoint {
  readonly x: number;
  readonly y: number;
}

/** One market pillar mark on a smile. */
export interface SmilePillar {
  /** Plotting coordinate on the wing axis (signed delta / log-moneyness / index). */
  readonly x: number;
  /** Mid implied vol in vol points (e.g. 7.85 for 7.85%). */
  readonly mid: number;
  /** Two-way market quote `[bid, ask]` in the same vol units; drives the band + error bar. */
  readonly bidask?: readonly [number, number];
  /** Wing label (e.g. "10ΔP", "25ΔP", "ATM", "25ΔC", "10ΔC"); parsed for RR/BF + markers. */
  readonly label?: string;
}

/** One tenor's smile: its market pillars and (optionally) a fitted curve. */
export interface SmileTenor {
  /** Tenor label (e.g. "1M"). */
  readonly tenor: string;
  /** Market pillar marks. */
  readonly pillars: readonly SmilePillar[];
  /** The fitted model — a continuous fn over `x`, or an explicit point set. */
  readonly fit?: SmileFit | readonly CurvePoint[];
  /** When true, this tenor is the focus (gets marks + band + markers + RR/BF read-out). */
  readonly focus?: boolean;
}

export interface VolSmileProps {
  /** One or more tenor smiles. The focused one (`focus:true`, else the last) is detailed. */
  readonly tenors: readonly SmileTenor[];
  /** Chart height in CSS px; width fills the container. */
  readonly height?: number;
  /** Caption for the wing axis (e.g. "delta (put ◂ ▸ call)" or "log-moneyness"). */
  readonly xUnitLabel?: string;
  /** Draw the focused tenor's market bid/ask band. Default true. */
  readonly showBand?: boolean;
  /** Enable hover tooltips on the focused tenor's marks. Default true. */
  readonly interactive?: boolean;
  /** Accessible label override (a summary is derived when omitted). */
  readonly ariaLabel?: string;
}

/* ─────────────────────────── label / RR-BF parsing ─────────────────────── */

type ParsedLabel = { kind: "atm" } | { kind: "wing"; delta: number; side: "P" | "C" } | null;

/**
 * Parse a wing label into its delta semantics. Accepts "ATM", "25ΔC", "10ΔP",
 * "25C", "10P", "0.25C" (a leading integer is read as a delta percentage). Returns
 * null for anything not recognisable, so downstream RR/BF is computed only when the
 * convention is actually present.
 */
function parseLabel(label?: string): ParsedLabel {
  if (!label) return null;
  const s = label.trim().toUpperCase();
  if (s === "ATM" || s === "ATMF" || s === "0" || s === "0D") return { kind: "atm" };
  const m = /^(\d+(?:\.\d+)?)\s*(?:Δ|D)?\s*([PC])$/.exec(s);
  if (!m) return null;
  const num = Number.parseFloat(m[1]!);
  if (!Number.isFinite(num)) return null;
  // "25" ⇒ 0.25, "0.25" ⇒ 0.25 — normalise to a fractional delta.
  const delta = num > 1 ? num / 100 : num;
  return { kind: "wing", delta, side: m[2] as "P" | "C" };
}

/** A canonical RR/BF row for a delta pair. RR = σ_call − σ_put; BF = ½(σ_call+σ_put) − σ_atm. */
interface RrBfRow {
  readonly delta: number;
  readonly rr: number;
  readonly bf: number;
}

interface RrBf {
  readonly atm: number | undefined;
  readonly rows: readonly RrBfRow[];
}

/** Derive ATM/RR/BF from a tenor's labelled pillars (only for pairs actually present). */
function computeRrBf(pillars: readonly SmilePillar[]): RrBf {
  const atm = pillars.find((p) => parseLabel(p.label)?.kind === "atm")?.mid;
  const byKey = new Map<string, number>();
  for (const p of pillars) {
    const pl = parseLabel(p.label);
    if (pl && pl.kind === "wing") byKey.set(`${pl.delta}:${pl.side}`, p.mid);
  }
  const rows: RrBfRow[] = [];
  for (const delta of [0.25, 0.1]) {
    const call = byKey.get(`${delta}:C`);
    const put = byKey.get(`${delta}:P`);
    if (call !== undefined && put !== undefined && atm !== undefined) {
      rows.push({ delta, rr: call - put, bf: (call + put) / 2 - atm });
    }
  }
  return { atm, rows };
}

/* ─────────────────────────── prepared (width-independent) model ─────────── */

/** A canonical wing marker (ATM emphasis line, or a 25Δ/10Δ wing reference line). */
interface WingMarker {
  readonly x: number;
  readonly label: string;
  readonly atm: boolean;
}

interface PreparedTenor {
  readonly tenor: string;
  readonly color: string;
  readonly focus: boolean;
  readonly pillars: readonly SmilePillar[];
  readonly fit: readonly CurvePoint[] | null;
  readonly band: readonly BandPoint[] | null;
}

interface BandPoint {
  readonly x: number;
  readonly lo: number;
  readonly hi: number;
}

interface PreparedModel {
  readonly tenors: readonly PreparedTenor[];
  readonly focusIdx: number;
  readonly xDomain: readonly [number, number];
  readonly yDomain: readonly [number, number];
  readonly rrbf: RrBf;
  readonly markers: readonly WingMarker[];
  readonly multi: boolean;
}

const FIT_SAMPLES = 72;

/** Map a tenor index onto the Viridis sequential ramp (short → long), 1..6. */
function rampVar(i: number, n: number): string {
  if (n <= 1) return "var(--seq-4)";
  const stop = 1 + Math.round((i / (n - 1)) * 5);
  return `var(--seq-${Math.min(6, Math.max(1, stop))})`;
}

/** Resolve a fit (fn sampled over the wing extent, or explicit points) to ≥2 clean points. */
function resolveFit(
  fit: SmileFit | readonly CurvePoint[] | undefined,
  xMin: number,
  xMax: number,
): readonly CurvePoint[] | null {
  if (!fit) return null;
  if (typeof fit === "function") {
    if (!(xMax > xMin)) return null;
    const pts: CurvePoint[] = [];
    for (let i = 0; i < FIT_SAMPLES; i += 1) {
      const x = xMin + ((xMax - xMin) * i) / (FIT_SAMPLES - 1);
      const y = fit(x);
      if (Number.isFinite(y)) pts.push({ x, y });
    }
    return pts.length >= 2 ? pts : null;
  }
  const arr = fit
    .filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y))
    .slice()
    .sort((a, b) => a.x - b.x);
  return arr.length >= 2 ? arr : null;
}

/**
 * Build the width-independent drawing model from the tenors, or `null` when there
 * is nothing honest to draw (so the caller renders the empty state). Domains are
 * SHARED across every tenor so an overlaid family stays visually comparable.
 */
function prepareModel(tenors: readonly SmileTenor[]): PreparedModel | null {
  if (tenors.length === 0) return null;

  // Wing (x) extent from every pillar and every explicit fit point.
  const xs: number[] = [];
  for (const t of tenors) {
    for (const p of t.pillars) if (Number.isFinite(p.x)) xs.push(p.x);
    if (t.fit && typeof t.fit !== "function") {
      for (const p of t.fit) if (Number.isFinite(p.x)) xs.push(p.x);
    }
  }
  if (xs.length === 0) return null;
  const xMinRaw = Math.min(...xs);
  const xMaxRaw = Math.max(...xs);
  if (!(xMaxRaw > xMinRaw)) return null;

  const n = tenors.length;
  const prepared: PreparedTenor[] = [];
  const ys: number[] = [];

  tenors.forEach((t, i) => {
    const fit = resolveFit(t.fit, xMinRaw, xMaxRaw);
    const band: BandPoint[] = t.pillars
      .filter((p) => p.bidask && Number.isFinite(p.bidask[0]) && Number.isFinite(p.bidask[1]))
      .map((p) => ({
        x: p.x,
        lo: Math.min(p.bidask![0], p.bidask![1]),
        hi: Math.max(p.bidask![0], p.bidask![1]),
      }))
      .sort((a, b) => a.x - b.x);

    for (const p of t.pillars) {
      if (Number.isFinite(p.mid)) ys.push(p.mid);
      if (p.bidask) {
        if (Number.isFinite(p.bidask[0])) ys.push(p.bidask[0]);
        if (Number.isFinite(p.bidask[1])) ys.push(p.bidask[1]);
      }
    }
    if (fit) for (const p of fit) ys.push(p.y);

    prepared.push({
      tenor: t.tenor,
      color: rampVar(i, n),
      focus: t.focus === true,
      pillars: t.pillars,
      fit,
      band: band.length >= 2 ? band : null,
    });
  });

  if (ys.length === 0) return null;

  let focusIdx = prepared.findIndex((t) => t.focus);
  if (focusIdx < 0) focusIdx = prepared.length - 1;
  // A tenor is only usefully "focused" if it has geometry; fall back otherwise.
  if (!prepared[focusIdx]!.fit && prepared[focusIdx]!.pillars.length < 2) {
    const alt = prepared.findIndex((t) => t.fit || t.pillars.length >= 2);
    if (alt >= 0) focusIdx = alt;
  }

  const focus = prepared[focusIdx]!;
  const rrbf = computeRrBf(focus.pillars);

  // Wing markers from the focused tenor's labelled pillars (ATM + 25Δ/10Δ wings).
  const markers: WingMarker[] = [];
  for (const p of focus.pillars) {
    const pl = parseLabel(p.label);
    if (!pl) continue;
    if (pl.kind === "atm") markers.push({ x: p.x, label: "ATM", atm: true });
    else if (pl.delta === 0.25 || pl.delta === 0.1)
      markers.push({ x: p.x, label: p.label!, atm: false });
  }

  // Padded, shared domains (a small symmetric pad keeps extremes off the frame).
  const xPad = (xMaxRaw - xMinRaw) * 0.06;
  const yMinRaw = Math.min(...ys);
  const yMaxRaw = Math.max(...ys);
  const yPad = (yMaxRaw - yMinRaw || Math.abs(yMaxRaw) || 1) * 0.12;

  return {
    tenors: prepared,
    focusIdx,
    xDomain: [xMinRaw - xPad, xMaxRaw + xPad],
    yDomain: [yMinRaw - yPad, yMaxRaw + yPad],
    rrbf,
    markers,
    multi: n > 1,
  };
}

/* ─────────────────────────── formatting + a11y ─────────────────────────── */

const fmtVol = (v: number): string => `${v.toFixed(2)}v`;
const fmtSigned = (v: number): string => `${v >= 0 ? "+" : "−"}${Math.abs(v).toFixed(2)}v`;

/** Diverging tint for a signed risk-reversal (put-skew ↔ call-skew), centred at 0. */
function rrTint(rr: number): string {
  if (rr <= -0.4) return "var(--div-neg2)";
  if (rr < -0.05) return "var(--div-neg1)";
  if (rr <= 0.05) return "var(--div-mid)";
  if (rr < 0.4) return "var(--div-pos1)";
  return "var(--div-pos2)";
}

function describeModel(model: PreparedModel): string {
  const focus = model.tenors[model.focusIdx]!;
  const rr = model.rrbf.rows
    .map((r) => `RR${Math.round(r.delta * 100)} ${fmtSigned(r.rr)}`)
    .join(", ");
  const family = model.multi ? ` family (${model.tenors.length} tenors)` : "";
  const atm = model.rrbf.atm !== undefined ? `, ATM ${fmtVol(model.rrbf.atm)}` : "";
  return (
    `Implied-volatility smile${family} for ${focus.tenor}: implied vol versus the delta / wing axis, ` +
    `market marks with a bid/ask band and a fitted curve${atm}${rr ? `, ${rr}` : ""}.`
  );
}

/* ─────────────────────────── prefers-reduced-motion ────────────────────── */

function usePrefersReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduce, setReduce] = useState<boolean>(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia(query).matches
      : false,
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia(query);
    const onChange = (): void => setReduce(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return reduce;
}

/* ─────────────────────────── the inner (sized) chart ───────────────────── */

interface HoverDatum {
  readonly tenor: string;
  readonly label: string | undefined;
  readonly mid: number;
  readonly bidask: readonly [number, number] | undefined;
}

interface ChartProps {
  readonly model: PreparedModel;
  readonly width: number;
  readonly height: number;
  readonly xUnitLabel: string;
  readonly showBand: boolean;
  readonly interactive: boolean;
  readonly gradId: string;
}

const MARGIN = { top: 14, right: 20, bottom: 34, left: 46 } as const;

function VolSmileChart({
  model,
  width,
  height,
  xUnitLabel,
  showBand,
  interactive,
  gradId,
}: ChartProps): React.ReactElement | null {
  const reduce = usePrefersReducedMotion();
  const [drawn, setDrawn] = useState(reduce);
  useEffect(() => {
    if (reduce) {
      setDrawn(true);
      return;
    }
    const id = requestAnimationFrame(() => setDrawn(true));
    return () => cancelAnimationFrame(id);
  }, [reduce]);

  const {
    showTooltip,
    hideTooltip,
    tooltipData,
    tooltipLeft,
    tooltipTop,
    tooltipOpen,
  } = useTooltip<HoverDatum>();

  const innerW = width - MARGIN.left - MARGIN.right;
  const innerH = height - MARGIN.top - MARGIN.bottom;

  const { xScale, yScale } = useMemo(() => {
    return {
      xScale: scaleLinear<number>({
        domain: [model.xDomain[0], model.xDomain[1]],
        range: [0, Math.max(0, innerW)],
      }),
      yScale: scaleLinear<number>({
        domain: [model.yDomain[0], model.yDomain[1]],
        range: [Math.max(0, innerH), 0],
      }),
    };
  }, [model.xDomain, model.yDomain, innerW, innerH]);

  if (innerW <= 0 || innerH <= 0) return null;

  const focus = model.tenors[model.focusIdx]!;

  // x tick values + label map from the focused tenor's pillars (falls back to numeric).
  const labelByX = new Map<string, string>();
  for (const p of focus.pillars) if (p.label) labelByX.set(p.x.toFixed(4), p.label);
  const tickValues = focus.pillars.length >= 2 ? focus.pillars.map((p) => p.x) : undefined;
  const formatXTick = (v: number): string => labelByX.get(v.toFixed(4)) ?? v.toFixed(2);

  return (
    <div style={{ position: "relative" }}>
      <svg width={width} height={height} role="img" aria-hidden="true" style={{ display: "block" }}>
        <LinearGradient
          id={gradId}
          from={focus.color}
          to={focus.color}
          fromOpacity={0.26}
          toOpacity={0.06}
          vertical
        />
        <Group left={MARGIN.left} top={MARGIN.top}>
          {/* horizontal vol gridlines (chrome) */}
          <GridRows
            scale={yScale}
            width={innerW}
            numTicks={5}
            stroke="var(--grid-line)"
            strokeWidth={0.5}
          />

          <Group
            style={{
              opacity: drawn ? 1 : 0,
              transition: reduce ? undefined : "opacity 320ms cubic-bezier(0.22,1,0.36,1)",
            }}
          >
            {/* focused tenor market bid/ask band */}
            {showBand && focus.band && (
              <Area<BandPoint>
                data={[...focus.band]}
                x={(d) => xScale(d.x)}
                y0={(d) => yScale(d.lo)}
                y1={(d) => yScale(d.hi)}
                curve={curveNatural}
                fill={`url(#${gradId})`}
                stroke="none"
              />
            )}

            {/* wing reference markers (ATM emphasis + 25Δ/10Δ wings) */}
            {model.markers.map((mk) => (
              <line
                key={`mk-${mk.label}-${mk.x}`}
                x1={xScale(mk.x)}
                x2={xScale(mk.x)}
                y1={0}
                y2={innerH}
                stroke="var(--text-tertiary)"
                strokeWidth={mk.atm ? 1 : 0.75}
                strokeOpacity={mk.atm ? 0.5 : 0.24}
                strokeDasharray={mk.atm ? "4 3" : "2 3"}
              />
            ))}

            {/* context (non-focused) tenor fits — thin, ramp-ordered */}
            {model.tenors.map((t, i) =>
              i === model.focusIdx || !t.fit ? null : (
                <LinePath<CurvePoint>
                  key={`ctx-${t.tenor}-${i}`}
                  data={[...t.fit]}
                  x={(d) => xScale(d.x)}
                  y={(d) => yScale(d.y)}
                  curve={curveNatural}
                  stroke={t.color}
                  strokeWidth={1.25}
                  strokeOpacity={0.5}
                  fill="none"
                  strokeLinejoin="round"
                />
              ),
            )}

            {/* focused tenor fitted curve */}
            {focus.fit && (
              <LinePath<CurvePoint>
                data={[...focus.fit]}
                x={(d) => xScale(d.x)}
                y={(d) => yScale(d.y)}
                curve={curveNatural}
                stroke={focus.color}
                strokeWidth={2.25}
                fill="none"
                strokeLinejoin="round"
              />
            )}

            {/* focused tenor error bars (bid→ask) + market marks */}
            {focus.pillars.map((p, i) => {
              const cx = xScale(p.x);
              const cy = yScale(p.mid);
              return (
                <g key={`mark-${p.label ?? i}-${p.x}`}>
                  {p.bidask && (
                    <line
                      x1={cx}
                      x2={cx}
                      y1={yScale(Math.min(p.bidask[0], p.bidask[1]))}
                      y2={yScale(Math.max(p.bidask[0], p.bidask[1]))}
                      stroke={focus.color}
                      strokeWidth={1}
                      strokeOpacity={0.55}
                    />
                  )}
                  <circle
                    cx={cx}
                    cy={cy}
                    r={3.2}
                    fill={focus.color}
                    stroke="var(--bg-base)"
                    strokeWidth={1}
                    style={interactive ? { cursor: "crosshair" } : undefined}
                    onMouseMove={
                      interactive
                        ? () =>
                            showTooltip({
                              tooltipData: {
                                tenor: focus.tenor,
                                label: p.label,
                                mid: p.mid,
                                bidask: p.bidask,
                              },
                              tooltipLeft: MARGIN.left + cx,
                              tooltipTop: MARGIN.top + cy,
                            })
                        : undefined
                    }
                    onMouseLeave={interactive ? hideTooltip : undefined}
                  />
                </g>
              );
            })}
          </Group>

          {/* axes (chrome) */}
          <AxisLeft
            scale={yScale}
            numTicks={5}
            hideAxisLine
            hideTicks
            tickFormat={(v) => `${Number(v).toFixed(1)}`}
            tickLabelProps={{
              fill: "var(--text-tertiary)",
              fontFamily: "var(--font-mono)",
              fontSize: 9,
              textAnchor: "end",
              dx: "-0.25em",
              dy: "0.25em",
            }}
            label="implied vol (%)"
            labelProps={{
              fill: "var(--text-secondary)",
              fontFamily: "var(--font-display)",
              fontSize: 10,
              textAnchor: "middle",
            }}
            labelOffset={30}
          />
          <AxisBottom
            top={innerH}
            scale={xScale}
            {...(tickValues ? { tickValues } : { numTicks: 5 })}
            tickFormat={(v) => formatXTick(Number(v))}
            stroke="var(--grid-line)"
            tickStroke="var(--grid-line)"
            tickLabelProps={{
              fill: "var(--text-tertiary)",
              fontFamily: "var(--font-mono)",
              fontSize: 9,
              textAnchor: "middle",
              dy: "0.2em",
            }}
            label={xUnitLabel}
            labelProps={{
              fill: "var(--text-secondary)",
              fontFamily: "var(--font-display)",
              fontSize: 10,
              textAnchor: "middle",
            }}
            labelOffset={14}
          />
        </Group>
      </svg>

      {interactive && tooltipOpen && tooltipData && (
        <TooltipWithBounds
          top={tooltipTop ?? 0}
          left={tooltipLeft ?? 0}
          style={{
            ...defaultStyles,
            background: "var(--bg-overlay-solid)",
            border: "var(--hairline)",
            color: "var(--text-primary)",
            padding: "6px 8px",
            borderRadius: "8px",
            boxShadow: "var(--shadow-float)",
            fontFamily: "var(--font-mono)",
            fontSize: 11,
            lineHeight: 1.4,
          }}
        >
          <div style={{ color: "var(--text-secondary)", marginBottom: 2 }}>
            {tooltipData.tenor}
            {tooltipData.label ? ` · ${tooltipData.label}` : ""}
          </div>
          <div>mid {fmtVol(tooltipData.mid)}</div>
          {tooltipData.bidask && (
            <div style={{ color: "var(--text-tertiary)" }}>
              {fmtVol(Math.min(...tooltipData.bidask))} / {fmtVol(Math.max(...tooltipData.bidask))}
            </div>
          )}
        </TooltipWithBounds>
      )}
    </div>
  );
}

/* ─────────────────────────── the exported component ────────────────────── */

const wrapStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--space-2)",
  fontFamily: "var(--font-display)",
  color: "var(--text-primary)",
};

/**
 * VolSmile — implied-vol smile (visx). Renders the header (title + market/fit key
 * + tenor family legend), the responsive chart, and the ATM/RR/BF read-out strip.
 * Falls back to an honest empty state when there is no drawable geometry.
 */
export function VolSmile({
  tenors,
  height = 280,
  xUnitLabel = "delta (put ◂ · ▸ call)",
  showBand = true,
  interactive = true,
  ariaLabel,
}: VolSmileProps): React.ReactElement {
  const rawId = useId();
  const gradId = `volsmile-band-${rawId.replace(/[^a-zA-Z0-9]/g, "")}`;
  const model = useMemo(() => prepareModel(tenors), [tenors]);

  if (!model) {
    return (
      <div
        role="img"
        aria-label="Volatility smile unavailable — no drawable market pillars or fitted curve"
        style={{
          display: "grid",
          placeItems: "center",
          height,
          gap: "var(--space-2)",
          background: "var(--bg-inset)",
          border: "var(--hairline)",
          borderRadius: "var(--r-md)",
          color: "var(--text-tertiary)",
          fontFamily: "var(--font-display)",
        }}
      >
        <span
          aria-hidden="true"
          style={{ fontFamily: "var(--font-mono)", fontSize: "var(--type-title)" }}
        >
          —
        </span>
        <span style={{ fontSize: "var(--type-caption)" }}>
          no drawable smile — supply market pillars or a fitted curve
        </span>
      </div>
    );
  }

  const focus = model.tenors[model.focusIdx]!;
  const label = ariaLabel ?? describeModel(model);

  return (
    <figure style={wrapStyle} aria-label={label} role="group">
      {/* header: title + key + (multi-tenor) legend */}
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--space-4)",
          flexWrap: "wrap",
        }}
      >
        <span
          style={{
            fontSize: "var(--type-headline)",
            fontWeight: "var(--weight-header)" as unknown as number,
            letterSpacing: "0.01em",
          }}
        >
          Smile · {focus.tenor}
        </span>
        <span
          style={{
            fontSize: "var(--type-caption)",
            color: "var(--text-tertiary)",
            fontFamily: "var(--font-mono)",
          }}
        >
          IV vs Δ · market ● &nbsp; fit —
        </span>
        <span style={{ flex: 1 }} />
        {model.multi && (
          <span
            style={{ display: "inline-flex", gap: "var(--space-3)", flexWrap: "wrap" }}
            aria-label="tenor family legend"
          >
            {model.tenors.map((t, i) => (
              <span
                key={`leg-${t.tenor}-${i}`}
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  fontSize: "var(--type-caption)",
                  color:
                    i === model.focusIdx ? "var(--text-primary)" : "var(--text-tertiary)",
                  fontFamily: "var(--font-mono)",
                  fontWeight: i === model.focusIdx ? 600 : 400,
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    width: 14,
                    height: 0,
                    borderTop: `2px solid ${t.color}`,
                    opacity: i === model.focusIdx ? 1 : 0.6,
                    display: "inline-block",
                  }}
                />
                {t.tenor}
              </span>
            ))}
          </span>
        )}
      </header>

      {/* responsive chart */}
      <div style={{ width: "100%" }}>
        <ParentSize debounceTime={0}>
          {({ width }) =>
            width < 80 ? (
              <div style={{ height }} />
            ) : (
              <VolSmileChart
                model={model}
                width={width}
                height={height}
                xUnitLabel={xUnitLabel}
                showBand={showBand}
                interactive={interactive}
                gradId={gradId}
              />
            )
          }
        </ParentSize>
      </div>

      {/* ATM · RR/BF read-out strip (derived, shown only when computable) */}
      {(model.rrbf.atm !== undefined || model.rrbf.rows.length > 0) && (
        <figcaption
          style={{
            display: "flex",
            gap: "var(--space-5)",
            flexWrap: "wrap",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-caption)",
            color: "var(--text-tertiary)",
          }}
        >
          {model.rrbf.atm !== undefined && (
            <span>
              ATM{" "}
              <b style={{ color: "var(--text-primary)", fontWeight: 600 }}>
                {fmtVol(model.rrbf.atm)}
              </b>
            </span>
          )}
          {model.rrbf.rows.map((r) => (
            <span key={`rrbf-${r.delta}`} style={{ display: "inline-flex", gap: "var(--space-3)" }}>
              <span>
                RR{Math.round(r.delta * 100)}{" "}
                <b style={{ color: rrTint(r.rr), fontWeight: 600 }}>{fmtSigned(r.rr)}</b>
              </span>
              <span>
                BF{Math.round(r.delta * 100)}{" "}
                <b style={{ color: "var(--text-secondary)", fontWeight: 600 }}>{fmtVol(r.bf)}</b>
              </span>
            </span>
          ))}
        </figcaption>
      )}
    </figure>
  );
}
