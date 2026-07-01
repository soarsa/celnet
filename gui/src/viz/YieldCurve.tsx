/**
 * YieldCurve — the FI term-structure workbench chart (mockup 14, curve workbench).
 * Overlays three mutually-consistent curves derived from ONE bootstrap of the
 * discount-factor nodes:
 *
 *   • ZERO rate      z(t) = −ln DF(t) / t            (left axis, %)
 *   • instantaneous  f(t) = −d/dt ln DF(t)           (left axis, %)
 *     FORWARD        — under log-linear-in-ln(DF) interpolation the forward is
 *                      CONSTANT within each pillar interval, so it is drawn as the
 *                      honest piecewise-flat staircase that interpolation actually
 *                      produces (not a smoothed line); under monotone-convex it is
 *                      the piecewise-quadratic the Hermite basis produces.
 *   • DISCOUNT       DF(t) = exp(ln DF(t))           (RIGHT axis, 0–1)
 *     FACTOR
 *
 * against a LOG-TIME tenor axis spanning the first pillar (ON) to the last (30Y),
 * because rate structure is read on log-time. Built on @visx (scales / LinePath /
 * axes / grid / ParentSize); colour comes exclusively from the dataviz sequential
 * palette tokens (--seq-*), never brand hues (dataviz colour contract). SVG fills
 * reference the CSS custom properties directly so every appearance/contrast theme
 * recolours the chart with no JS.
 *
 * Honesty discipline: with fewer than two dated pillars (or non-finite / non-positive
 * tenors) there is nothing to interpolate, so an explicit empty state renders rather
 * than a fabricated curve.
 *
 * Interaction: a clickable legend toggles each overlay independently; a hover
 * crosshair reads z / f / DF at any tenor off the same bootstrap; pillar dots mark
 * the calibrating nodes on the zero curve.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AxisBottom, AxisLeft, AxisRight } from "@visx/axis";
import { curveLinear } from "@visx/curve";
import { GridColumns, GridRows } from "@visx/grid";
import { Group } from "@visx/group";
import { ParentSize } from "@visx/responsive";
import { scaleLinear, scaleLog } from "@visx/scale";
import { LinePath } from "@visx/shape";

/** A single calibrating pillar of the curve. */
export interface CurveNode {
  /** Short pillar label, e.g. "ON", "3M", "5Y". */
  readonly label: string;
  /** Tenor in years (must be finite and > 0). */
  readonly tenorYears: number;
  /**
   * Continuously-compounded zero rate as a FRACTION (0.0531 = 5.31%), so
   * DF(t_i) = exp(−zeroRate · tenorYears).
   */
  readonly zeroRate: number;
}

/**
 * How ln(DF) is interpolated between pillars.
 *  • `log-linear`      — linear in ln(DF); forward is piecewise-flat.
 *  • `monotone-convex` — Fritsch–Carlson monotone Hermite in ln(DF); forward is
 *                        piecewise-quadratic (smooth) and stays arbitrage-monotone.
 */
export type InterpolationMode = "log-linear" | "monotone-convex";

export interface YieldCurveProps {
  /** The calibrating pillar nodes (any order; sorted internally by tenor). */
  readonly nodes: readonly CurveNode[];
  /** ln(DF) interpolation scheme. Defaults to log-linear. */
  readonly interpolation?: InterpolationMode;
  /** Chart height in CSS px (width fills the container). */
  readonly height?: number;
}

type LineKey = "zero" | "fwd" | "df";
type Visibility = Record<LineKey, boolean>;
interface Pt {
  readonly t: number;
  readonly v: number;
}

/** The three overlays' identity: axis side, token colour and human label. */
const LINES: readonly { key: LineKey; label: string; color: string; axis: string }[] = [
  { key: "zero", label: "Zero", color: "var(--seq-4)", axis: "L · %" },
  { key: "fwd", label: "Fwd", color: "var(--seq-6)", axis: "L · %" },
  { key: "df", label: "DF", color: "var(--seq-2)", axis: "R · 0–1" },
];

const MARGIN = { top: 18, right: 54, bottom: 34, left: 50 };
const SAMPLES = 168; // log-time samples for the smooth zero / DF / forward curves

/** A tenor in years → compact human label (1d / 7d / 3m / 5y). */
function fmtTenor(t: number): string {
  if (t < 0.077) return `${Math.round(t * 365)}d`;
  if (t < 0.92) return `${Math.round(t * 12)}m`;
  return `${t < 9.95 ? t.toFixed(1) : Math.round(t)}y`;
}

/**
 * The bootstrapped curve model: ln(DF) interpolation nodes (with the t=0 origin),
 * the per-segment slopes, the Fritsch–Carlson monotone tangents, and the three
 * closed-form readouts z(t), f(t), DF(t). Pure; depends only on nodes + mode.
 */
function buildCurve(nodes: readonly CurveNode[], mode: InterpolationMode) {
  // ln(DF) nodes anchored at the origin (t=0, ln DF=0). ln DF(t_i) = −z_i·t_i.
  const nx: number[] = [0];
  const ny: number[] = [0];
  for (const nd of nodes) {
    nx.push(nd.tenorYears);
    ny.push(-nd.zeroRate * nd.tenorYears);
  }
  const n = nx.length;

  // Per-segment slope of ln(DF) — the forward is −slope (constant per segment).
  const delta: number[] = [];
  for (let k = 0; k < n - 1; k += 1) {
    delta.push((ny[k + 1]! - ny[k]!) / (nx[k + 1]! - nx[k]!));
  }

  // Fritsch–Carlson monotone tangents (used only by monotone-convex).
  const mt = new Array<number>(n);
  mt[0] = delta[0]!;
  mt[n - 1] = delta[n - 2]!;
  for (let j = 1; j < n - 1; j += 1) mt[j] = (delta[j - 1]! + delta[j]!) / 2;
  for (let m = 0; m < n - 1; m += 1) {
    if (delta[m] === 0) {
      mt[m] = 0;
      mt[m + 1] = 0;
    } else {
      const a = mt[m]! / delta[m]!;
      const b = mt[m + 1]! / delta[m]!;
      if (a * a + b * b > 9) {
        const tau = 3 / Math.sqrt(a * a + b * b);
        mt[m] = tau * a * delta[m]!;
        mt[m + 1] = tau * b * delta[m]!;
      }
    }
  }

  const seg = (t: number): number => {
    let k = 0;
    while (k < n - 2 && t > nx[k + 1]!) k += 1;
    return k;
  };
  const lnDF = (t: number): number => {
    const k = seg(t);
    const h = nx[k + 1]! - nx[k]!;
    const s = (t - nx[k]!) / h;
    if (mode === "log-linear") return ny[k]! + (ny[k + 1]! - ny[k]!) * s;
    const s2 = s * s;
    const s3 = s2 * s;
    return (
      (2 * s3 - 3 * s2 + 1) * ny[k]! +
      (s3 - 2 * s2 + s) * h * mt[k]! +
      (-2 * s3 + 3 * s2) * ny[k + 1]! +
      (s3 - s2) * h * mt[k + 1]!
    );
  };
  const dLnDF = (t: number): number => {
    const k = seg(t);
    const h = nx[k + 1]! - nx[k]!;
    const s = (t - nx[k]!) / h;
    if (mode === "log-linear") return delta[k]!;
    const s2 = s * s;
    return (
      ((6 * s2 - 6 * s) * ny[k]! + (-6 * s2 + 6 * s) * ny[k + 1]!) / h +
      (3 * s2 - 4 * s + 1) * mt[k]! +
      (3 * s2 - 2 * s) * mt[k + 1]!
    );
  };

  const zeroAt = (t: number): number => -lnDF(t) / t; // fraction
  const fwdAt = (t: number): number => -dLnDF(t); // fraction
  const dfAt = (t: number): number => Math.exp(lnDF(t));

  return { nx, delta, n, zeroAt, fwdAt, dfAt };
}

/** Everything derived from the model that does NOT depend on pixel width. */
function useCurveModel(nodes: readonly CurveNode[], interpolation: InterpolationMode) {
  return useMemo(() => {
    const model = buildCurve(nodes, interpolation);
    const tmin = model.nx[1]!; // first real pillar (log axis cannot show t=0)
    const tmax = model.nx[model.n - 1]!;
    const lnMin = Math.log(tmin);
    const lnMax = Math.log(tmax);

    const zeroPts: Pt[] = [];
    const dfPts: Pt[] = [];
    for (let s = 0; s <= SAMPLES; s += 1) {
      const t = Math.exp(lnMin + (lnMax - lnMin) * (s / SAMPLES));
      zeroPts.push({ t, v: model.zeroAt(t) });
      dfPts.push({ t, v: model.dfAt(t) });
    }

    // Forward: the piecewise-flat staircase (log-linear) or a dense smooth sample.
    const fwdPts: Pt[] = [];
    if (interpolation === "log-linear") {
      for (let k = 1; k < model.n - 1; k += 1) {
        const f = -model.delta[k]!;
        fwdPts.push({ t: model.nx[k]!, v: f });
        fwdPts.push({ t: model.nx[k + 1]!, v: f });
      }
    } else {
      for (let s = 0; s <= SAMPLES; s += 1) {
        const t = Math.exp(lnMin + (lnMax - lnMin) * (s / SAMPLES));
        fwdPts.push({ t, v: model.fwdAt(t) });
      }
    }

    const pillars = nodes.map((nd) => ({
      t: nd.tenorYears,
      v: nd.zeroRate,
      label: nd.label,
    }));

    // Stable left-axis (rate) band over BOTH zero and forward so toggling a line
    // never rescales the axis. Padded 8%; not clamped to 0 (rates may be negative).
    let rMin = Infinity;
    let rMax = -Infinity;
    for (const p of zeroPts) {
      if (p.v < rMin) rMin = p.v;
      if (p.v > rMax) rMax = p.v;
    }
    for (const p of fwdPts) {
      if (p.v < rMin) rMin = p.v;
      if (p.v > rMax) rMax = p.v;
    }
    const pad = (rMax - rMin || Math.abs(rMax) || 0.01) * 0.08;

    return {
      model,
      tmin,
      tmax,
      zeroPts,
      fwdPts,
      dfPts,
      pillars,
      rDomain: [rMin - pad, rMax + pad] as const,
      tenorLabels: new Map(nodes.map((nd) => [nd.tenorYears, nd.label] as const)),
    };
  }, [nodes, interpolation]);
}

/** The pixel-resolved chart, rendered inside a measured container. */
function YieldCurveInner({
  nodes,
  interpolation,
  visible,
  width,
  height,
}: {
  readonly nodes: readonly CurveNode[];
  readonly interpolation: InterpolationMode;
  readonly visible: Visibility;
  readonly width: number;
  readonly height: number;
}): React.ReactElement | null {
  const c = useCurveModel(nodes, interpolation);
  const overlayRef = useRef<SVGRectElement>(null);
  const [hoverT, setHoverT] = useState<number | null>(null);

  // prefers-reduced-motion: the crosshair fades in unless motion is reduced.
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = (): void => setReduced(mq.matches);
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, []);

  const innerW = Math.max(1, width - MARGIN.left - MARGIN.right);
  const innerH = Math.max(1, height - MARGIN.top - MARGIN.bottom);

  const xScale = useMemo(
    () => scaleLog({ domain: [c.tmin, c.tmax], range: [0, innerW] }),
    [c.tmin, c.tmax, innerW],
  );
  const yLeft = useMemo(
    () => scaleLinear({ domain: [c.rDomain[0], c.rDomain[1]], range: [innerH, 0] }),
    [c.rDomain, innerH],
  );
  const yRight = useMemo(
    () => scaleLinear({ domain: [0, 1], range: [innerH, 0] }),
    [innerH],
  );

  const onMove = useCallback(
    (e: React.PointerEvent<SVGRectElement>): void => {
      const rect = overlayRef.current?.getBoundingClientRect();
      if (!rect) return;
      const px = Math.max(0, Math.min(innerW, e.clientX - rect.left));
      const t = Math.max(c.tmin, Math.min(c.tmax, xScale.invert(px)));
      setHoverT(t);
    },
    [c.tmin, c.tmax, xScale, innerW],
  );
  const onLeave = useCallback((): void => setHoverT(null), []);

  if (innerW < 40 || innerH < 40) return null;

  const pillarTenors = c.pillars.map((p) => p.t);
  const labelFor = (t: number): string => c.tenorLabels.get(t) ?? fmtTenor(t);

  // Hover readout, computed off the same bootstrap the curves use.
  const hz = hoverT === null ? 0 : c.model.zeroAt(hoverT);
  const hf = hoverT === null ? 0 : c.model.fwdAt(hoverT);
  const hdf = hoverT === null ? 0 : c.model.dfAt(hoverT);
  const hx = hoverT === null ? 0 : xScale(hoverT);
  const boxW = 128;
  const boxH = 62;
  const boxX = hx + 12 + boxW > innerW ? hx - 12 - boxW : hx + 12;
  const boxY = Math.max(0, Math.min(innerH - boxH, yLeft(hz) - boxH - 6));

  const axisLabelProps = {
    fill: "var(--text-tertiary)",
    fontSize: 9,
    fontFamily: "var(--font-mono)",
  };

  return (
    <svg
      width={width}
      height={height}
      role="img"
      aria-label={`Zero rate, instantaneous forward and discount factor term structure from ${labelFor(c.tmin)} to ${labelFor(c.tmax)} on a log-time tenor axis (${interpolation} interpolation)`}
    >
      <Group left={MARGIN.left} top={MARGIN.top}>
        <GridRows
          scale={yLeft}
          width={innerW}
          numTicks={5}
          stroke="var(--grid-line)"
          strokeWidth={0.5}
        />
        <GridColumns
          scale={xScale}
          height={innerH}
          tickValues={pillarTenors}
          stroke="var(--grid-line)"
          strokeWidth={0.5}
          strokeOpacity={0.6}
        />

        {/* Discount factor — right 0–1 axis, drawn first so the rate lines sit on top. */}
        {visible.df && (
          <LinePath<Pt>
            data={c.dfPts}
            x={(d) => xScale(d.t)}
            y={(d) => yRight(d.v)}
            curve={curveLinear}
            stroke="var(--seq-2)"
            strokeWidth={2}
            strokeLinejoin="round"
            fill="none"
          />
        )}
        {/* Instantaneous forward — the honest piecewise-flat staircase (log-linear). */}
        {visible.fwd && (
          <LinePath<Pt>
            data={c.fwdPts}
            x={(d) => xScale(d.t)}
            y={(d) => yLeft(d.v)}
            curve={curveLinear}
            stroke="var(--seq-6)"
            strokeWidth={2}
            strokeLinejoin="round"
            fill="none"
          />
        )}
        {/* Zero rate — left % axis. */}
        {visible.zero && (
          <LinePath<Pt>
            data={c.zeroPts}
            x={(d) => xScale(d.t)}
            y={(d) => yLeft(d.v)}
            curve={curveLinear}
            stroke="var(--seq-4)"
            strokeWidth={2.4}
            strokeLinejoin="round"
            fill="none"
          />
        )}

        {/* Calibrating pillar dots on the zero curve. */}
        {visible.zero &&
          c.pillars.map((p) => (
            <circle
              key={p.label}
              cx={xScale(p.t)}
              cy={yLeft(p.v)}
              r={3}
              fill="var(--seq-4)"
              stroke="var(--bg-raised)"
              strokeWidth={1}
            />
          ))}

        <AxisBottom
          top={innerH}
          scale={xScale}
          tickValues={pillarTenors}
          tickFormat={(v) => labelFor(Number(v))}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickLabelProps={() => ({ ...axisLabelProps, textAnchor: "middle" as const, dy: "0.25em" })}
        />
        <AxisLeft
          scale={yLeft}
          numTicks={5}
          tickFormat={(v) => `${(Number(v) * 100).toFixed(1)}`}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickLabelProps={() => ({ ...axisLabelProps, textAnchor: "end" as const, dx: "-0.25em", dy: "0.25em" })}
        />
        <AxisRight
          left={innerW}
          scale={yRight}
          numTicks={5}
          tickFormat={(v) => Number(v).toFixed(2)}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickLabelProps={() => ({ ...axisLabelProps, textAnchor: "start" as const, dx: "0.25em", dy: "0.25em" })}
        />

        {/* axis titles */}
        <text
          x={0}
          y={-6}
          textAnchor="start"
          fill="var(--text-secondary)"
          fontSize={9}
          fontFamily="var(--font-display)"
          letterSpacing="0.06em"
          style={{ textTransform: "uppercase" }}
        >
          zero / fwd %
        </text>
        <text
          x={innerW}
          y={-6}
          textAnchor="end"
          fill="var(--text-secondary)"
          fontSize={9}
          fontFamily="var(--font-display)"
          letterSpacing="0.06em"
          style={{ textTransform: "uppercase" }}
        >
          discount factor
        </text>
        <text
          x={innerW / 2}
          y={innerH + 30}
          textAnchor="middle"
          fill="var(--text-tertiary)"
          fontSize={9}
          fontFamily="var(--font-display)"
          letterSpacing="0.06em"
          style={{ textTransform: "uppercase" }}
        >
          tenor · log-time · {labelFor(c.tmin)} → {labelFor(c.tmax)}
        </text>

        {/* hover crosshair + readout */}
        {hoverT !== null && (
          <g style={{ transition: reduced ? "none" : "opacity 120ms ease-out" }}>
            <line
              x1={hx}
              y1={0}
              x2={hx}
              y2={innerH}
              stroke="var(--accent)"
              strokeWidth={1}
              strokeDasharray="3 3"
              opacity={0.85}
            />
            {visible.zero && <circle cx={hx} cy={yLeft(hz)} r={3.2} fill="var(--seq-4)" />}
            {visible.fwd && <circle cx={hx} cy={yLeft(hf)} r={3.2} fill="var(--seq-6)" />}
            {visible.df && <circle cx={hx} cy={yRight(hdf)} r={3.2} fill="var(--seq-2)" />}
            <rect
              x={boxX}
              y={boxY}
              width={boxW}
              height={boxH}
              rx={5}
              fill="var(--bg-overlay-solid)"
              stroke="var(--grid-line)"
              strokeWidth={1}
            />
            <text
              x={boxX + 9}
              y={boxY + 14}
              fill="var(--text-secondary)"
              fontSize={9}
              fontFamily="var(--font-display)"
              fontWeight={600}
            >
              {fmtTenor(hoverT)} · {hoverT.toFixed(2)}y
            </text>
            {(
              [
                ["ZERO", `${(hz * 100).toFixed(3)}%`],
                ["FWD", `${(hf * 100).toFixed(3)}%`],
                ["DF", hdf.toFixed(5)],
              ] as const
            ).map(([k, val], i) => (
              <g key={k}>
                <text
                  x={boxX + 9}
                  y={boxY + 27 + i * 12}
                  fill="var(--text-tertiary)"
                  fontSize={8}
                  fontFamily="var(--font-display)"
                  letterSpacing="0.04em"
                >
                  {k}
                </text>
                <text
                  x={boxX + boxW - 9}
                  y={boxY + 27 + i * 12}
                  textAnchor="end"
                  fill="var(--text-primary)"
                  fontSize={9}
                  fontFamily="var(--font-mono)"
                >
                  {val}
                </text>
              </g>
            ))}
          </g>
        )}

        {/* transparent hit area for the crosshair (declared last = topmost) */}
        <rect
          ref={overlayRef}
          x={0}
          y={0}
          width={innerW}
          height={innerH}
          fill="transparent"
          onPointerMove={onMove}
          onPointerLeave={onLeave}
        />
      </Group>
    </svg>
  );
}

/**
 * The YieldCurve chart. Sorts/validates the pillar nodes, renders the honest empty
 * state when there is nothing to interpolate, and otherwise draws the responsive
 * overlaid term structure with a per-line-visibility legend.
 */
export function YieldCurve({
  nodes,
  interpolation = "log-linear",
  height = 340,
}: YieldCurveProps): React.ReactElement {
  const [visible, setVisible] = useState<Visibility>({ zero: true, fwd: true, df: true });

  // Keep only finite, strictly-positive-tenor pillars, ascending by tenor, de-duped.
  const clean = useMemo(() => {
    const seen = new Set<number>();
    return [...nodes]
      .filter(
        (n) =>
          Number.isFinite(n.tenorYears) &&
          n.tenorYears > 0 &&
          Number.isFinite(n.zeroRate) &&
          !seen.has(n.tenorYears) &&
          seen.add(n.tenorYears),
      )
      .sort((a, b) => a.tenorYears - b.tenorYears);
  }, [nodes]);

  const toggle = useCallback(
    (key: LineKey): void => setVisible((v) => ({ ...v, [key]: !v[key] })),
    [],
  );

  if (clean.length < 2) {
    return (
      <div
        role="img"
        aria-label="Yield curve unavailable — at least two dated pillars are required to interpolate a curve"
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          gap: "var(--space-2)",
          height,
          border: "var(--hairline)",
          borderRadius: "var(--r-md)",
          background: "var(--bg-inset)",
        }}
      >
        <span style={{ fontFamily: "var(--font-mono)", fontSize: 24, color: "var(--text-tertiary)" }}>
          —
        </span>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)" }}>
          curve needs ≥ 2 dated pillars
        </span>
      </div>
    );
  }

  return (
    <div style={{ width: "100%" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-3)",
          marginBottom: "var(--space-2)",
          flexWrap: "wrap",
        }}
      >
        {LINES.map((ln) => {
          const on = visible[ln.key];
          return (
            <button
              key={ln.key}
              type="button"
              onClick={() => toggle(ln.key)}
              aria-pressed={on}
              title={`Toggle the ${ln.label} overlay`}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                padding: "3px 9px",
                border: "var(--hairline)",
                borderRadius: "var(--r-sm)",
                background: "oklch(1 0 0 / 0.04)",
                color: on ? "var(--text-secondary)" : "var(--text-tertiary)",
                fontFamily: "var(--font-display)",
                fontSize: "var(--type-caption)",
                fontWeight: 600,
                cursor: "pointer",
                opacity: on ? 1 : 0.4,
                textDecoration: on ? "none" : "line-through",
              }}
            >
              <span
                aria-hidden
                style={{
                  width: 14,
                  height: 0,
                  borderTop: `3px solid ${ln.color}`,
                  borderRadius: 2,
                  display: "inline-block",
                }}
              />
              {ln.label}
              <span
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: 8,
                  color: "var(--text-tertiary)",
                }}
              >
                {ln.axis}
              </span>
            </button>
          );
        })}
      </div>

      <div style={{ position: "relative", width: "100%", height }}>
        <ParentSize>
          {({ width }) =>
            width > 0 ? (
              <YieldCurveInner
                nodes={clean}
                interpolation={interpolation}
                visible={visible}
                width={width}
                height={height}
              />
            ) : null
          }
        </ParentSize>
      </div>
    </div>
  );
}
