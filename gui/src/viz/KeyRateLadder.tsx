/**
 * KeyRateLadder — a signed key-rate DV01 pillar ladder (visx bar chart).
 *
 * Fixed-income risk is NOT a Greek vector — it is a *pillar Jacobian*: the
 * sensitivity of PV to a 1bp bump of each calibrating curve pillar, taken one
 * pillar at a time (bump-and-re-bootstrap). This chart draws that Jacobian as a
 * ladder of signed bars, one per pillar (1Y/2Y/…/30Y), coloured by the diverging
 * `--div` dataviz ramp centred at 0 (cool = negative, warm = positive — NEVER the
 * brand hues, which must not encode a quantitative scale). A Σ annotation
 * reconciles the ladder total back to the parallel DV01 so the decomposition is
 * provably additive; if a caller supplies a parallel DV01 that the ladder does not
 * sum to, the residual is surfaced honestly in `--warn` rather than hidden.
 *
 * Hand-rolled from visx primitives (`scaleBand`/`scaleLinear`/`Group`/`Bar`) so
 * the scale + colour-encoding logic lives in one place. SVG only, so design tokens
 * are consumed directly as `fill="var(--div-…)"` — no canvas colour resolution.
 * The mount transition is a fade (suppressed under `prefers-reduced-motion`); the
 * bars themselves are drawn deterministically from props, never fabricated.
 */

import { useEffect, useState } from "react";
import { Group } from "@visx/group";
import { scaleBand, scaleLinear } from "@visx/scale";
import { Bar } from "@visx/shape";
import { fmtPnlAdaptive } from "../lib/format";

/** One rung of the ladder: a calibrating curve pillar and its signed key-rate DV01. */
export interface KeyRatePillar {
  /** The calibrating pillar tenor label, e.g. "1Y", "5Y", "30Y". */
  readonly pillar: string;
  /** Signed key-rate DV01 at this pillar, in the ladder's currency unit per bp. */
  readonly dv01: number;
}

export interface KeyRateLadderProps {
  /** The signed key-rate DV01 ladder — one entry per calibrating pillar. */
  readonly data: readonly KeyRatePillar[];
  /**
   * The independently-computed parallel DV01 (a single simultaneous 1bp bump of
   * every pillar) the ladder should reconcile to. When supplied, the Σ row proves
   * additivity: it flags any residual in `--warn` instead of claiming a clean
   * reconciliation. When omitted, Σ simply reports the ladder total.
   */
  readonly parallelDv01?: number;
  /** Currency-per-bp unit label for the Σ annotation and the accessible name. */
  readonly unit?: string;
  /** Pixel width (SVG scales to fit its container up to this). */
  readonly width?: number;
  /** Pixel height of the plot area. */
  readonly height?: number;
}

const MARGIN = { top: 22, right: 14, bottom: 30, left: 32 } as const;

/**
 * Map a magnitude-normalised value in [-1, 1] onto the diverging `--div` ramp,
 * centred at 0. Two buckets either side of the midpoint give the ladder its
 * cool→neutral→warm reading without ever touching a brand hue.
 */
function divToken(normalized: number): string {
  if (normalized <= -0.5) return "var(--div-neg2)";
  if (normalized < 0) return "var(--div-neg1)";
  if (normalized === 0) return "var(--div-mid)";
  if (normalized < 0.5) return "var(--div-pos1)";
  return "var(--div-pos2)";
}

/** Build the accessible sentence describing the whole ladder + its reconciliation. */
function describeLadder(
  data: readonly KeyRatePillar[],
  sum: number,
  unit: string,
  parallelDv01: number | undefined,
  reconciles: boolean,
  residual: number,
): string {
  const rungs = data.map((d) => `${d.pillar} ${fmtPnlAdaptive(d.dv01)}`).join(", ");
  const total = `Sum of key-rate DV01 ${fmtPnlAdaptive(sum)} ${unit}`;
  const recon =
    parallelDv01 === undefined
      ? "."
      : reconciles
        ? `, reconciling to the parallel DV01 ${fmtPnlAdaptive(parallelDv01)} ${unit}.`
        : `, against a parallel DV01 of ${fmtPnlAdaptive(parallelDv01)} ${unit} with an unreconciled residual of ${fmtPnlAdaptive(residual)} ${unit}.`;
  return `Key-rate DV01 ladder, ${unit}, across ${data.length} calibrating pillars: ${rungs}. ${total}${recon}`;
}

export function KeyRateLadder({
  data,
  parallelDv01,
  unit = "USD/bp",
  width = 360,
  height = 200,
}: KeyRateLadderProps): React.ReactElement {
  // Mount fade — a single motion cue, disabled under prefers-reduced-motion.
  const [reduced, setReduced] = useState(false);
  const [entered, setEntered] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) {
      setEntered(true);
      return;
    }
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReduced(mq.matches);
    const onChange = (): void => setReduced(mq.matches);
    mq.addEventListener("change", onChange);
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => {
      mq.removeEventListener("change", onChange);
      cancelAnimationFrame(raf);
    };
  }, []);
  const shown = reduced || entered;

  const valid =
    data.length > 0 && data.every((d) => d.pillar.length > 0 && Number.isFinite(d.dv01));

  const sum = data.reduce((acc, d) => acc + d.dv01, 0);
  const target = parallelDv01 ?? sum;
  const residual = sum - target;
  // Reconciles within a small absolute floor + a 0.1% relative tolerance (bump-
  // and-reprice is not bit-exact); otherwise the residual is stated, not buried.
  const reconciles = Math.abs(residual) <= Math.max(1, Math.abs(target) * 1e-3);

  // Honest empty state — never draw an axis with no (or non-finite) risk on it.
  if (!valid) {
    return (
      <div
        role="img"
        aria-label="Key-rate DV01 ladder unavailable — the curve exposes no calibrating pillars"
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          gap: "var(--space-1)",
          minHeight: height,
          padding: "var(--space-3)",
          border: "var(--hairline)",
          borderRadius: "var(--r-sm)",
          background: "var(--bg-inset)",
          textAlign: "center",
          fontFamily: "var(--font-ui)",
        }}
      >
        <span
          aria-hidden="true"
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-title)",
            color: "var(--text-tertiary)",
            lineHeight: 1,
          }}
        >
          —
        </span>
        <span
          style={{
            fontSize: "var(--type-caption)",
            lineHeight: "var(--type-caption-lh)",
            color: "var(--text-tertiary)",
            maxWidth: "26ch",
          }}
        >
          no key-rate ladder — the curve exposes no calibrating pillars
        </span>
      </div>
    );
  }

  const innerW = width - MARGIN.left - MARGIN.right;
  const innerH = height - MARGIN.top - MARGIN.bottom;

  const values = data.map((d) => d.dv01);
  const maxAbs = Math.max(...values.map((v) => Math.abs(v)));
  // Keep 0 in the value domain so the baseline is always drawn; pad the loaded
  // side(s) by 14% so the extreme bar + its label sit inside the frame.
  const dataMin = Math.min(0, ...values);
  const dataMax = Math.max(0, ...values);
  const span = dataMax - dataMin || 1;
  const pad = span * 0.14;
  const yMin = dataMin - (dataMin < 0 ? pad : 0);
  const yMax = dataMax + (dataMax > 0 ? pad : 0);

  const xScale = scaleBand<string>({
    domain: data.map((d) => d.pillar),
    range: [0, innerW],
    padding: 0.34,
  });
  const yScale = scaleLinear<number>({ domain: [yMin, yMax], range: [innerH, 0] });
  const zeroY = yScale(0);
  const bw = xScale.bandwidth();

  const fadeStyle: React.CSSProperties = {
    opacity: shown ? 1 : 0,
    transform: shown ? "translateY(0)" : "translateY(6px)",
    transition: reduced
      ? "none"
      : "opacity var(--quote-in, 180ms) var(--ease-out), transform var(--quote-in, 180ms) var(--ease-out)",
  };

  const ariaLabel = describeLadder(data, sum, unit, parallelDv01, reconciles, residual);
  const reconTone = reconciles ? "var(--text-primary)" : "var(--warn)";

  return (
    <figure
      style={{
        margin: 0,
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-2)",
        fontFamily: "var(--font-ui)",
        maxWidth: width,
      }}
    >
      <figcaption style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <span
          style={{
            fontFamily: "var(--font-display)",
            fontSize: "var(--type-headline)",
            fontWeight: 600,
            color: "var(--text-primary)",
          }}
        >
          Key-rate DV01 ladder
        </span>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)" }}>
          FI risk is a pillar Jacobian, not a Greek vector · {unit} per calibrating pillar
        </span>
      </figcaption>

      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={ariaLabel}
        style={{ display: "block", maxWidth: "100%", height: "auto", overflow: "visible" }}
      >
        <Group left={MARGIN.left} top={MARGIN.top}>
          <g style={fadeStyle}>
            {/* zero baseline — the reference the whole diverging ramp is centred on */}
            <line
              x1={0}
              x2={innerW}
              y1={zeroY}
              y2={zeroY}
              stroke="var(--text-tertiary)"
              strokeOpacity={0.4}
              strokeWidth={1}
            />
            <text
              x={-6}
              y={zeroY + 3}
              textAnchor="end"
              fontFamily="var(--font-mono)"
              fontSize={9}
              fill="var(--text-tertiary)"
            >
              0
            </text>

            {data.map((d) => {
              const bx = xScale(d.pillar);
              if (bx === undefined) return null;
              const yVal = yScale(d.dv01);
              const barY = Math.min(zeroY, yVal);
              const barH = Math.max(1.5, Math.abs(yVal - zeroY));
              const positive = d.dv01 >= 0;
              const norm = maxAbs > 0 ? d.dv01 / maxAbs : 0;
              const cx = bx + bw / 2;
              // Value label sits just outside the bar end, away from zero, clamped
              // inside the plot so a dominant bar keeps its label on-frame.
              const labelY = positive
                ? Math.max(9, barY - 5)
                : Math.min(innerH - 4, barY + barH + 12);
              return (
                <g key={d.pillar}>
                  <Bar
                    x={bx}
                    y={barY}
                    width={bw}
                    height={barH}
                    rx={1.5}
                    fill={divToken(norm)}
                  />
                  <text
                    x={cx}
                    y={labelY}
                    textAnchor="middle"
                    fontFamily="var(--font-mono)"
                    fontSize={9}
                    fontWeight={500}
                    fill="var(--text-secondary)"
                  >
                    {fmtPnlAdaptive(d.dv01)}
                  </text>
                  <text
                    x={cx}
                    y={innerH + 20}
                    textAnchor="middle"
                    fontFamily="var(--font-mono)"
                    fontSize={10}
                    fontWeight={600}
                    fill="var(--text-secondary)"
                  >
                    {d.pillar}
                  </text>
                </g>
              );
            })}
          </g>
        </Group>
      </svg>

      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--space-3)",
          borderTop: "var(--hairline)",
          paddingTop: "var(--space-2)",
          fontSize: "var(--type-caption)",
          color: "var(--text-tertiary)",
        }}
      >
        <span style={{ textTransform: "uppercase", letterSpacing: "0.05em" }}>Σ key-rate</span>
        <span style={{ fontFamily: "var(--font-mono)", fontWeight: 600, color: reconTone }}>
          {fmtPnlAdaptive(sum)} {unit}
        </span>
        <span style={{ marginLeft: "auto", color: reconciles ? "var(--text-tertiary)" : "var(--warn)" }}>
          {parallelDv01 === undefined
            ? "= ladder total"
            : reconciles
              ? "= parallel DV01 · reconciles"
              : `vs parallel ${fmtPnlAdaptive(parallelDv01)} · residual ${fmtPnlAdaptive(residual)}`}
        </span>
      </div>
    </figure>
  );
}
