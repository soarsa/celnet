/**
 * XvaExposureFan — the counterparty exposure-profile FAN (XVA workspace, GUI
 * mockup 07). A @visx SVG chart plotting, across a netting set's time buckets:
 *   • an outer PFE 5–95% quantile band (a filled area between the 5% and 95%
 *     curves),
 *   • an inner 25–75% quantile band,
 *   • the expected-exposure (EE) line with node dots,
 *   • the PFE(95%) upper envelope drawn dashed,
 *   • the time-average EPE, a horizontal dashed reference,
 *   • a mirrored expected-negative-exposure (ENE) band + line below zero, with the
 *     time-average EEN dashed reference.
 *
 * Convention match (SmileChart / CurveChart / PayoffChart): a typed props
 * interface, ALL colour via design-token CSS vars (never raw hex — SVG fills read
 * `var(--seq-*)` / `var(--div-*)` directly), the honest-empty-state discipline (an
 * explicit empty state rather than a fabricated curve for data we cannot draw),
 * role/aria-label a11y, and prefers-reduced-motion honoured on the mount
 * transition. Quantitative colour uses the Viridis sequential ramp (--seq-*) for
 * positive exposure and the diverging cool pole (--div-neg*) for the mirrored
 * negative exposure — never brand coral/indigo.
 *
 * LIVE-WIRE STATUS — SEEDED SAMPLE DATA ONLY. The live exposure profile is blocked
 * on the deferred D-xva activation: `celnet-xva::compute_xva` exists but has ZERO
 * non-test callers (there is no `PriceXva` proto message / WS-mirror codec yet), so
 * nothing streams a real netting-set exposure vector into this component. Until
 * that lands, every consumer (see the accompanying story) feeds a deterministic
 * SEEDED sample profile and the chart labels itself as illustrative. These figures
 * must NEVER be presented as a live valuation.
 */

import { AxisBottom, AxisLeft } from "@visx/axis";
import { curveMonotoneX } from "@visx/curve";
import { GridColumns, GridRows } from "@visx/grid";
import { Group } from "@visx/group";
import { ParentSize } from "@visx/responsive";
import { scaleLinear } from "@visx/scale";
import { Area, LinePath } from "@visx/shape";
import { useEffect, useState } from "react";

/**
 * One time bucket of the exposure profile. All positive-exposure quantiles are
 * `>= 0`; all negative-exposure (ENE) quantiles are `<= 0`. Within a bucket the
 * caller is expected to keep the natural ordering
 * `pfeLo <= q25 <= ee <= q75 <= pfe` and `eneBandLo <= ene <= eneBandHi <= 0`.
 */
export interface FanBucket {
  /** Time from the as-of date to this bucket, in years (the x position). */
  readonly t: number;
  /** Short tenor label for the x axis (e.g. "6M", "1Y", "2Y·H"). */
  readonly label: string;
  /** Expected (mean) positive exposure — the EE line. */
  readonly ee: number;
  /** Inner-band lower quantile (25%). */
  readonly q25: number;
  /** Inner-band upper quantile (75%). */
  readonly q75: number;
  /** Outer-band lower quantile (5%). */
  readonly pfeLo: number;
  /** Outer-band upper quantile — the PFE(95%) envelope. */
  readonly pfe: number;
  /** Expected (mean) negative exposure — the mirrored ENE line (`<= 0`). */
  readonly ene: number;
  /** ENE band bound nearer zero (`<= 0`). */
  readonly eneBandHi: number;
  /** ENE band bound further from zero — the negative 5% tail (`<= 0`). */
  readonly eneBandLo: number;
}

/** Which measure the chart emphasises (mirrors the mockup's segment control). */
export type ExposureMeasure = "ee" | "epe" | "pfe";

export interface XvaExposureFanProps {
  /** The exposure profile, one entry per time bucket (min 2 to draw a fan). */
  readonly buckets: readonly FanBucket[];
  /** SVG height in CSS px; width fills the container. Defaults to 300. */
  readonly height?: number;
  /** The emphasised measure — thickens the corresponding line. Defaults to "ee". */
  readonly measure?: ExposureMeasure;
  /** Exposure unit suffix shown after the `$` (e.g. "mm", "k"). Defaults to "mm". */
  readonly unit?: string;
  /** Panel heading. Defaults to "Exposure profile". */
  readonly title?: string;
  /**
   * Override the auto-computed time-average EPE reference (the trapezoidal
   * time-average of the EE profile). Omit to derive it from `buckets`.
   */
  readonly epe?: number;
  /**
   * Override the auto-computed time-average EEN reference (the trapezoidal
   * time-average of the ENE profile; `<= 0`). Omit to derive it from `buckets`.
   */
  readonly een?: number;
}

const MARGIN = { top: 16, right: 18, bottom: 30, left: 46 } as const;

/** `$X.Xmm`-style money with the mockup's real minus glyph (U+2212). */
function money(v: number, unit: string): string {
  return `${v < 0 ? "−" : ""}$${Math.abs(v).toFixed(1)}${unit}`;
}

/**
 * Trapezoidal time-average of a per-bucket field over the profile's horizon.
 * This is exactly EPE (over EE) / EEN (over ENE): the time-weighted mean the
 * horizontal reference lines represent.
 */
function timeAverage(buckets: readonly FanBucket[], key: "ee" | "ene"): number {
  if (buckets.length === 0) return 0;
  if (buckets.length === 1) return buckets[0]![key];
  let area = 0;
  for (let i = 1; i < buckets.length; i += 1) {
    const dt = buckets[i]!.t - buckets[i - 1]!.t;
    area += (dt * (buckets[i]![key] + buckets[i - 1]![key])) / 2;
  }
  const span = buckets[buckets.length - 1]!.t - buckets[0]!.t;
  return span > 0 ? area / span : buckets[0]![key];
}

/** Honour prefers-reduced-motion for the one-shot mount transition. */
function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false,
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = (): void => setReduced(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

const LEGEND: ReadonlyArray<{ label: string; color: string; dashed?: boolean }> = [
  { label: "EE", color: "var(--seq-5)" },
  { label: "PFE 95%", color: "var(--seq-3)", dashed: true },
  { label: "EPE (time-avg)", color: "var(--seq-6)", dashed: true },
  { label: "ENE (mirrored)", color: "var(--div-neg1)" },
];

/**
 * The exposure-profile fan. Renders a @visx SVG fan for a valid profile, or an
 * honest empty state (never a fabricated curve) when there is too little data.
 */
export function XvaExposureFan({
  buckets,
  height = 300,
  measure = "ee",
  unit = "mm",
  title = "Exposure profile",
  epe,
  een,
}: XvaExposureFanProps): React.ReactElement {
  const reduce = usePrefersReducedMotion();
  const [entered, setEntered] = useState(false);
  useEffect(() => {
    setEntered(true);
  }, []);

  const sorted = [...buckets].sort((a, b) => a.t - b.t);
  const valid =
    sorted.length >= 2 &&
    sorted.every((b) =>
      [b.t, b.ee, b.q25, b.q75, b.pfeLo, b.pfe, b.ene, b.eneBandHi, b.eneBandLo].every(
        Number.isFinite,
      ),
    );

  return (
    <figure
      style={{
        margin: 0,
        background: "var(--bg-inset)",
        border: "var(--hairline)",
        borderRadius: "var(--r-md)",
        padding: "var(--space-4)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-2)",
      }}
    >
      <figcaption
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--space-3)",
          flexWrap: "wrap",
        }}
      >
        <span
          style={{
            fontFamily: "var(--font-display)",
            fontSize: "var(--type-headline)",
            fontWeight: 600,
            color: "var(--text-primary)",
          }}
        >
          {title}
        </span>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)" }}>
          EPE · EE · PFE fan &amp; mirrored ENE
        </span>
        <span style={{ flex: 1 }} />
        <span style={{ display: "inline-flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {LEGEND.map((l) => (
            <span
              key={l.label}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 5,
                fontSize: "var(--type-micro)",
                color: "var(--text-secondary)",
              }}
            >
              <span
                aria-hidden
                style={{
                  width: 14,
                  height: 0,
                  borderTop: `2px ${l.dashed ? "dashed" : "solid"} ${l.color}`,
                  display: "inline-block",
                }}
              />
              {l.label}
            </span>
          ))}
        </span>
      </figcaption>

      {valid ? (
        <div style={{ width: "100%", height }}>
          <ParentSize>
            {({ width }) =>
              width < 60 ? null : (
                <FanSvg
                  width={width}
                  height={height}
                  buckets={sorted}
                  measure={measure}
                  unit={unit}
                  epe={epe ?? timeAverage(sorted, "ee")}
                  een={een ?? timeAverage(sorted, "ene")}
                  entered={entered}
                  reduce={reduce}
                />
              )
            }
          </ParentSize>
        </div>
      ) : (
        <div
          role="img"
          aria-label="Exposure-profile fan unavailable — fewer than two valid time buckets to draw a fan"
          style={{
            height,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: "var(--space-3)",
            color: "var(--text-tertiary)",
          }}
        >
          <span
            aria-hidden
            style={{ fontFamily: "var(--font-mono)", fontSize: "var(--type-title)" }}
          >
            —
          </span>
          <span style={{ fontSize: "var(--type-caption)" }}>
            no exposure profile — need at least two finite time buckets
          </span>
        </div>
      )}

      <figcaption
        style={{
          fontSize: "var(--type-micro)",
          color: "var(--text-tertiary)",
          textAlign: "right",
        }}
      >
        band = PFE 5–95% quantiles · inner = 25–75% · line = expected exposure · dashed =
        time-average EPE · <b>seeded sample</b> — live wire pending D-xva activation
      </figcaption>
    </figure>
  );
}

interface FanSvgProps {
  readonly width: number;
  readonly height: number;
  readonly buckets: FanBucket[];
  readonly measure: ExposureMeasure;
  readonly unit: string;
  readonly epe: number;
  readonly een: number;
  readonly entered: boolean;
  readonly reduce: boolean;
}

function FanSvg({
  width,
  height,
  buckets,
  measure,
  unit,
  epe,
  een,
  entered,
  reduce,
}: FanSvgProps): React.ReactElement {
  const innerW = Math.max(10, width - MARGIN.left - MARGIN.right);
  const innerH = Math.max(10, height - MARGIN.top - MARGIN.bottom);

  const tMin = buckets[0]!.t;
  const tMax = buckets[buckets.length - 1]!.t;

  let posMax = 0;
  let negMin = 0;
  for (const b of buckets) {
    if (b.pfe > posMax) posMax = b.pfe;
    if (b.eneBandLo < negMin) negMin = b.eneBandLo;
  }
  const yTop = posMax > 0 ? posMax * 1.08 : 1;
  const yBottom = negMin < 0 ? negMin * 1.08 : 0;

  const xScale = scaleLinear<number>({ domain: [tMin, tMax], range: [0, innerW] });
  const yScale = scaleLinear<number>({ domain: [yBottom, yTop], range: [innerH, 0] });

  const labelByT = new Map(buckets.map((b) => [b.t, b.label]));
  const tenorTs = buckets.map((b) => b.t);
  const zeroY = yScale(0);

  // Peak PFE bucket (the profile hump) for the marker + accessible summary.
  const peakPfe = buckets.reduce((a, b) => (b.pfe > a.pfe ? b : a), buckets[0]!);
  const peakEe = buckets.reduce((m, b) => Math.max(m, b.ee), 0);

  // Measure-driven emphasis (mirrors the mockup's #fan[data-m=…] rules).
  const outerFillOpacity = measure === "pfe" ? 0.3 : 0.16;
  const eeWidth = measure === "ee" ? 3.2 : 2.2;
  const pfeWidth = measure === "pfe" ? 2.6 : 1.5;
  const pfeDash = measure === "pfe" ? undefined : "5 3";
  const epeWidth = measure === "epe" ? 2.6 : 1.2;

  const groupStyle: React.CSSProperties = {
    opacity: reduce ? 1 : entered ? 1 : 0,
    transition: reduce ? "none" : "opacity 280ms cubic-bezier(0.22, 1, 0.36, 1)",
  };

  const axisTickLabel = {
    fill: "var(--text-tertiary)",
    fontSize: 9,
    fontFamily: "var(--font-mono)",
  };

  const desc =
    `XVA counterparty exposure-profile fan (seeded sample data): expected exposure peaks near ` +
    `${money(peakEe, unit)}, PFE(95%) peaks ${money(peakPfe.pfe, unit)} at ${peakPfe.label}, with a ` +
    `mirrored expected-negative-exposure band below zero (time-average EPE ${money(epe, unit)}, ` +
    `EEN ${money(een, unit)}). Illustrative figures — live wire pending D-xva activation.`;

  return (
    <svg width={width} height={height} role="img" aria-label={desc}>
      <Group left={MARGIN.left} top={MARGIN.top}>
        <GridRows
          scale={yScale}
          width={innerW}
          stroke="var(--grid-line)"
          strokeWidth={0.5}
          pointerEvents="none"
        />
        <GridColumns
          scale={xScale}
          height={innerH}
          tickValues={tenorTs}
          stroke="var(--grid-line)"
          strokeWidth={0.5}
          pointerEvents="none"
        />

        <g style={groupStyle}>
          {/* outer PFE 5–95% band (Viridis --seq-2) */}
          <Area
            data={buckets}
            x={(d) => xScale(d.t)}
            y0={(d) => yScale(d.pfeLo)}
            y1={(d) => yScale(d.pfe)}
            curve={curveMonotoneX}
            fill="var(--seq-2)"
            fillOpacity={outerFillOpacity}
            stroke="var(--seq-2)"
            strokeOpacity={0.55}
            strokeWidth={0.75}
          />
          {/* inner 25–75% band (Viridis --seq-4) */}
          <Area
            data={buckets}
            x={(d) => xScale(d.t)}
            y0={(d) => yScale(d.q25)}
            y1={(d) => yScale(d.q75)}
            curve={curveMonotoneX}
            fill="var(--seq-4)"
            fillOpacity={0.34}
            stroke="var(--seq-4)"
            strokeOpacity={0.72}
            strokeWidth={0.75}
          />
          {/* mirrored ENE band (diverging cool pole --div-neg2) */}
          <Area
            data={buckets}
            x={(d) => xScale(d.t)}
            y0={(d) => yScale(d.eneBandLo)}
            y1={(d) => yScale(d.eneBandHi)}
            curve={curveMonotoneX}
            fill="var(--div-neg2)"
            fillOpacity={0.2}
            stroke="var(--div-neg1)"
            strokeOpacity={0.42}
            strokeWidth={0.6}
          />

          {/* zero-exposure baseline */}
          <line
            x1={0}
            x2={innerW}
            y1={zeroY}
            y2={zeroY}
            stroke="var(--text-secondary)"
            strokeWidth={1}
            strokeOpacity={0.55}
          />

          {/* PFE(95%) upper envelope, dashed */}
          <LinePath
            data={buckets}
            x={(d) => xScale(d.t)}
            y={(d) => yScale(d.pfe)}
            curve={curveMonotoneX}
            fill="none"
            stroke="var(--seq-3)"
            strokeWidth={pfeWidth}
            strokeDasharray={pfeDash}
            strokeLinejoin="round"
          />
          {/* expected-exposure line */}
          <LinePath
            data={buckets}
            x={(d) => xScale(d.t)}
            y={(d) => yScale(d.ee)}
            curve={curveMonotoneX}
            fill="none"
            stroke="var(--seq-5)"
            strokeWidth={eeWidth}
            strokeLinejoin="round"
          />
          {/* mirrored ENE line */}
          <LinePath
            data={buckets}
            x={(d) => xScale(d.t)}
            y={(d) => yScale(d.ene)}
            curve={curveMonotoneX}
            fill="none"
            stroke="var(--div-neg1)"
            strokeWidth={2}
            strokeLinejoin="round"
          />

          {/* time-average EPE reference (horizontal dashed) */}
          <line
            x1={0}
            x2={innerW}
            y1={yScale(epe)}
            y2={yScale(epe)}
            stroke="var(--seq-6)"
            strokeWidth={epeWidth}
            strokeDasharray="6 4"
          />
          <text x={4} y={yScale(epe) - 4} fill="var(--seq-6)" fontSize={9} fontFamily="var(--font-mono)">
            EPE {money(epe, unit)}
          </text>
          {/* time-average EEN reference (horizontal dashed, negative side) */}
          <line
            x1={0}
            x2={innerW}
            y1={yScale(een)}
            y2={yScale(een)}
            stroke="var(--div-neg1)"
            strokeWidth={1}
            strokeDasharray="4 4"
            strokeOpacity={0.8}
          />
          <text
            x={4}
            y={yScale(een) + 11}
            fill="var(--div-neg1)"
            fontSize={9}
            fontFamily="var(--font-mono)"
          >
            EEN {money(Math.abs(een), unit)}
          </text>

          {/* EE node dots */}
          {buckets.map((d) => (
            <circle key={`ee-${d.t}`} cx={xScale(d.t)} cy={yScale(d.ee)} r={2.4} fill="var(--seq-5)" />
          ))}
          {/* peak PFE marker */}
          <circle cx={xScale(peakPfe.t)} cy={yScale(peakPfe.pfe)} r={3.2} fill="var(--seq-3)" />
          <text
            x={xScale(peakPfe.t) + 6}
            y={yScale(peakPfe.pfe) - 2}
            fill="var(--seq-3)"
            fontSize={9}
            fontFamily="var(--font-mono)"
          >
            peak PFE95 {money(peakPfe.pfe, unit)} · {peakPfe.label}
          </text>
        </g>

        <AxisLeft
          scale={yScale}
          numTicks={6}
          tickFormat={(v) => `${Number(v)}`}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickLabelProps={() => ({ ...axisTickLabel, textAnchor: "end", dx: "-0.25em", dy: "0.25em" })}
        />
        <AxisBottom
          top={innerH}
          scale={xScale}
          tickValues={tenorTs}
          tickFormat={(v) => labelByT.get(Number(v)) ?? ""}
          stroke="var(--grid-line)"
          tickStroke="var(--grid-line)"
          tickLabelProps={() => ({ ...axisTickLabel, textAnchor: "middle", dy: "0.25em" })}
        />
      </Group>

      {/* y-axis caption ($mm), rotated */}
      <text
        transform={`translate(11, ${MARGIN.top + innerH / 2}) rotate(-90)`}
        textAnchor="middle"
        fill="var(--text-tertiary)"
        fontSize={9}
        fontFamily="var(--font-mono)"
      >
        exposure (${unit})
      </text>
    </svg>
  );
}
