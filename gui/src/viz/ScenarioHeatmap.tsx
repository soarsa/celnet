/**
 * ScenarioHeatmap — the spot × vol P&L scenario grid (Risk & Scenario, mockup 06).
 * An ECharts heatmap: rows are vol shocks, columns are spot shocks, each cell the
 * revalued Δ P&L of the book under that joint shock. The colour is a DIVERGING
 * blue↔orange ramp CENTRED AT ZERO with a SYMMETRIC ±|max| domain, so a loss and
 * the equal-magnitude gain read as mirror-image intensities and the sign is never
 * ambiguous. Click a cell to drill (emits {@link ScenarioCell} to `onCellClick`).
 *
 * Palette discipline (dataviz contract): quantitative colour comes ONLY from the
 * `--div-*` tokens — never a brand hue. Because ECharts paints to a <canvas>, CSS
 * custom properties don't cascade into it the way they do for inline SVG; we resolve
 * the tokens once per render via getComputedStyle and re-resolve on an appearance /
 * contrast / density change (a MutationObserver on <html>) so a theme switch repaints
 * with the right ramp. The colour mapping is a *piecewise* visualMap (five discrete
 * bands cut at ±¼|max| and ±½|max|) rather than a continuous interpolation: the ramp
 * stops are oklch() strings, which the browser's canvas fills directly but a gradient
 * interpolator would have to numerically parse — discrete bands sidestep that and also
 * match the design's banded legend exactly.
 *
 * Honesty discipline: with no grid, a ragged grid, or a non-finite cell there is
 * nothing truthful to draw, so the component renders an em-dash empty state instead
 * of fabricating a surface.
 */

import { useEffect, useRef, useState } from "react";
// The heavy ECharts runtime is code-split OUT of the main bundle: only its TYPES
// are imported statically (fully erased at build — `verbatimModuleSyntax`), while
// the runtime is `await import("echarts")`-ed inside the mount effect (fixes the
// >500KB main-chunk warning). A lightweight loading state shows until it resolves;
// the public Props and rendered behaviour are unchanged.
import type { EChartsOption, EChartsType, ECElementEvent } from "echarts";

/** A single scenario cell, emitted on click for the drill panel. */
export interface ScenarioCell {
  /** Column index into `spotLabels`. */
  readonly spotIndex: number;
  /** Row index into `volLabels`. */
  readonly volIndex: number;
  /** The spot-shock tick label (e.g. "+2%"). */
  readonly spotLabel: string;
  /** The vol-shock tick label (e.g. "+4.0v"). */
  readonly volLabel: string;
  /** The cell's P&L, in the chart's `unit`. */
  readonly pnl: number;
}

export interface ScenarioHeatmapProps {
  /**
   * Row-major P&L grid: `pnl[volIndex][spotIndex]`. Row 0 renders at the BOTTOM of
   * the y-axis (ECharts category convention), so order the vol rows bottom → top.
   */
  readonly pnl: ReadonlyArray<ReadonlyArray<number>>;
  /** Column (x) tick labels — one per spot shock, left → right. */
  readonly spotLabels: readonly string[];
  /** Row (y) tick labels — one per vol shock, bottom → top. */
  readonly volLabels: readonly string[];
  /** Value unit shown in the tooltip / legend (e.g. "$k"). Defaults to "$k". */
  readonly unit?: string;
  /** Overall pixel height (width fills the container). Defaults to 300. */
  readonly height?: number;
  /** Draw the P&L number inside each cell (auto-suppressed for very large grids). */
  readonly showValues?: boolean;
  /** Format a P&L value for the cell labels + tooltip + legend. */
  readonly formatValue?: (v: number) => string;
  /** Fired when a cell is clicked — the drill hook. */
  readonly onCellClick?: (cell: ScenarioCell) => void;
  /** Accessible summary; a sensible one is derived when omitted. */
  readonly ariaLabel?: string;
}

/** The five diverging ramp tokens (loss → mid → gain), consumed by the legend + canvas. */
const DIV_RAMP = ["--div-neg2", "--div-neg1", "--div-mid", "--div-pos1", "--div-pos2"] as const;

/** Signed, compact default formatter using a real minus sign (U+2212). */
function defaultFormat(v: number): string {
  const rounded = Math.abs(v) >= 100 ? Math.round(v) : Math.round(v * 10) / 10;
  const mag = Number.isInteger(rounded) ? String(Math.abs(rounded)) : Math.abs(rounded).toFixed(1);
  const sign = rounded > 0 ? "+" : rounded < 0 ? "−" : "";
  return `${sign}${mag}`;
}

/** Resolve the token colours ECharts needs baked into the (canvas) chart option. */
function resolveTokens(): {
  ramp: string[];
  gridLine: string;
  textTertiary: string;
  textPrimary: string;
  bgBase: string;
  bgInset: string;
  fontMono: string;
  fontUi: string;
} {
  const root = getComputedStyle(document.documentElement);
  const get = (name: string, fallback: string): string => root.getPropertyValue(name).trim() || fallback;
  return {
    ramp: [
      get("--div-neg2", "oklch(0.58 0.13 248)"),
      get("--div-neg1", "oklch(0.70 0.07 240)"),
      get("--div-mid", "oklch(0.5 0.006 264)"),
      get("--div-pos1", "oklch(0.72 0.11 62)"),
      get("--div-pos2", "oklch(0.68 0.16 45)"),
    ],
    gridLine: get("--grid-line", "oklch(1 0 0 / 0.06)"),
    textTertiary: get("--text-tertiary", "#888"),
    textPrimary: get("--text-primary", "#fff"),
    bgBase: get("--bg-base", "oklch(0.255 0.018 264)"),
    bgInset: get("--bg-inset", "oklch(0.21 0.016 264)"),
    fontMono: get("--font-mono", "ui-monospace, monospace"),
    fontUi: get("--font-ui", "system-ui, sans-serif"),
  };
}

export function ScenarioHeatmap(props: ScenarioHeatmapProps): React.ReactElement {
  const {
    pnl,
    spotLabels,
    volLabels,
    unit = "$k",
    height = 300,
    showValues = true,
    formatValue = defaultFormat,
    onCellClick,
    ariaLabel,
  } = props;

  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<EChartsType | null>(null);
  // False until the code-split ECharts runtime resolves; drives the loading state.
  const [libLoaded, setLibLoaded] = useState(false);
  // Latest render closure, so the mount-bound observers repaint with current props.
  const renderRef = useRef<() => void>(() => {});
  // Latest props for the mount-bound click handler (avoids re-binding on every render).
  const clickRef = useRef<{
    spotLabels: readonly string[];
    volLabels: readonly string[];
    onCellClick: ((cell: ScenarioCell) => void) | undefined;
  }>({ spotLabels, volLabels, onCellClick });
  clickRef.current = { spotLabels, volLabels, onCellClick };

  const nCols = spotLabels.length;
  const nRows = volLabels.length;
  const valid =
    nCols > 0 &&
    nRows > 0 &&
    pnl.length === nRows &&
    pnl.every((row) => row.length === nCols && row.every((v) => Number.isFinite(v)));

  // Symmetric domain: the ramp is centred on 0 with a ±|max| extent, computed here
  // (so the legend can show it too) and fed to the effect as a dependency.
  let maxAbs = 0;
  if (valid) {
    for (const row of pnl) {
      for (const v of row) {
        const a = Math.abs(v);
        if (a > maxAbs) maxAbs = a;
      }
    }
  }
  const absMax = maxAbs > 0 ? maxAbs : 1;
  const withValues = showValues && nCols * nRows <= 240;

  const label =
    ariaLabel ??
    `Spot by vol P&L scenario heatmap, diverging colour centred at zero${
      valid ? `, ${nRows} vol by ${nCols} spot grid` : ""
    }`;

  // Mount: lazily init the chart, wire click + resize + appearance/reduced-motion
  // re-render, and dispose everything on unmount.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    // The heavy ECharts runtime is code-split out of the main bundle and loaded
    // on mount; the effect stays sync-return (React needs a cleanup fn, not a
    // Promise), so the async work runs in an IIFE and its teardown is captured
    // into `cleanup`, guarded by `disposed` against an unmount mid-import.
    let disposed = false;
    let cleanup: (() => void) | null = null;

    void (async () => {
      const echarts = await import("echarts").catch(() => null);
      if (disposed || !echarts) return;
      const chart = echarts.init(el, undefined, { renderer: "canvas" });
      chartRef.current = chart;
      setLibLoaded(true);

      const onClick = (params: ECElementEvent): void => {
        const cur = clickRef.current;
        if (!cur.onCellClick) return;
        const d = params.data as unknown as [number, number, number] | undefined;
        if (!d) return;
        const [x, y, v] = d;
        const spotLabel = cur.spotLabels[x];
        const volLabel = cur.volLabels[y];
        if (spotLabel === undefined || volLabel === undefined) return;
        cur.onCellClick({ spotIndex: x, volIndex: y, spotLabel, volLabel, pnl: v });
      };
      chart.on("click", onClick);

      const ro = new ResizeObserver(() => chart.resize());
      ro.observe(el);

      // Canvas can't consume CSS vars live like SVG — repaint with freshly resolved
      // tokens when the COLOUR axes flip on <html>. The density axis is sizing-only (it
      // doesn't move the --seq/--div palette) and is owned by its own module, so we
      // watch appearance + contrast only.
      const mo = new MutationObserver(() => renderRef.current());
      mo.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-appearance", "data-contrast"],
      });

      const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
      const onMq = (): void => renderRef.current();
      mq.addEventListener("change", onMq);

      renderRef.current();

      cleanup = () => {
        mq.removeEventListener("change", onMq);
        mo.disconnect();
        ro.disconnect();
        chart.off("click", onClick);
        chart.dispose();
        chartRef.current = null;
      };

      // Unmounted while the dynamic import was in flight — tear straight back down.
      if (disposed) cleanup();
    })();

    return () => {
      disposed = true;
      if (cleanup) cleanup();
    };
    // Mount-only: current props reach the handlers via refs / renderRef.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Build + apply the chart option whenever the inputs change (also re-run by the
  // appearance / motion observers via renderRef).
  useEffect(() => {
    renderRef.current = (): void => {
      const chart = chartRef.current;
      if (!chart) return;
      if (!valid) {
        chart.clear();
        return;
      }

      const t = resolveTokens();
      const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

      // [xIndex, yIndex, value] cells for the category × category heatmap.
      const cells: [number, number, number][] = [];
      for (let r = 0; r < nRows; r += 1) {
        const row = pnl[r];
        if (!row) continue;
        for (let c = 0; c < nCols; c += 1) {
          const v = row[c];
          if (v === undefined) continue;
          cells.push([c, r, v]);
        }
      }

      const half = absMax / 2;
      const quarter = absMax / 4;
      // Guaranteed-string ramp accessor (noUncheckedIndexedAccess widens t.ramp[i]).
      const band = (i: number): string => t.ramp[i] ?? "oklch(0.5 0.006 264)";

      const option: EChartsOption = {
        animation: !reducedMotion,
        animationDuration: 240,
        grid: { left: 8, right: 14, top: 24, bottom: 6, containLabel: true },
        tooltip: {
          trigger: "item",
          backgroundColor: t.bgInset,
          borderColor: t.gridLine,
          borderWidth: 1,
          padding: 10,
          textStyle: { color: t.textPrimary, fontFamily: t.fontUi, fontSize: 11 },
          extraCssText: "border-radius:var(--r-md); box-shadow:var(--shadow-float);",
          formatter: (params) => {
            const p = Array.isArray(params) ? params[0] : params;
            if (!p) return "";
            const d = p.data as unknown as [number, number, number] | undefined;
            if (!d) return "";
            const [x, y, v] = d;
            const sl = spotLabels[x] ?? "";
            const vl = volLabels[y] ?? "";
            return (
              `<div style="font-family:var(--font-ui);line-height:1.55">` +
              `<div style="font-size:9px;letter-spacing:0.06em;text-transform:uppercase;color:var(--text-tertiary)">scenario cell</div>` +
              `<div>spot shock&nbsp;<b style="font-family:var(--font-mono)">${sl}</b></div>` +
              `<div>vol shock&nbsp;<b style="font-family:var(--font-mono)">${vl}</b></div>` +
              `<div style="margin-top:3px">&Delta; P&amp;L&nbsp;<b style="font-family:var(--font-mono)">${formatValue(v)} ${unit}</b></div>` +
              `</div>`
            );
          },
        },
        xAxis: {
          type: "category",
          data: [...spotLabels],
          name: "spot shock →",
          nameLocation: "end",
          nameGap: 8,
          nameTextStyle: { color: t.textTertiary, fontFamily: t.fontUi, fontSize: 9, align: "right" },
          axisLine: { lineStyle: { color: t.gridLine } },
          axisTick: { show: false },
          axisLabel: { color: t.textTertiary, fontFamily: t.fontMono, fontSize: 10 },
          splitArea: { show: false },
        },
        yAxis: {
          type: "category",
          data: [...volLabels],
          name: "vol shock ↑ (ATM v)",
          nameLocation: "end",
          nameGap: 6,
          nameTextStyle: { color: t.textTertiary, fontFamily: t.fontUi, fontSize: 9, align: "left" },
          axisLine: { lineStyle: { color: t.gridLine } },
          axisTick: { show: false },
          axisLabel: { color: t.textTertiary, fontFamily: t.fontMono, fontSize: 10 },
          splitArea: { show: false },
        },
        // Piecewise (banded) diverging map, symmetric about 0: cuts at ±¼|max| and
        // ±½|max|. `show:false` — the mapping still applies; we draw our own
        // token-driven legend in the DOM below the canvas.
        visualMap: {
          type: "piecewise",
          show: false,
          dimension: 2,
          seriesIndex: 0,
          pieces: [
            { lt: -half, color: band(0) },
            { gte: -half, lt: -quarter, color: band(1) },
            { gte: -quarter, lte: quarter, color: band(2) },
            { gt: quarter, lte: half, color: band(3) },
            { gt: half, color: band(4) },
          ],
        },
        series: [
          {
            type: "heatmap",
            name: "Scenario P&L",
            data: cells,
            label: {
              show: withValues,
              formatter: (p) => formatValue((p.data as unknown as [number, number, number])[2]),
              color: t.textPrimary,
              // A halo so the value reads on any band, in any appearance.
              textBorderColor: t.bgBase,
              textBorderWidth: 2,
              fontFamily: t.fontMono,
              fontSize: 9,
              fontWeight: "bold",
            },
            itemStyle: { borderColor: t.gridLine, borderWidth: 1 },
            emphasis: {
              itemStyle: { borderColor: t.textPrimary, borderWidth: 1.5 },
              label: { show: withValues },
            },
          },
        ],
      };

      chart.setOption(option, { notMerge: true });
    };
    renderRef.current();
  }, [pnl, spotLabels, volLabels, unit, withValues, absMax, nRows, nCols, valid, formatValue]);

  return (
    <div style={{ width: "100%" }}>
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--space-3)",
          marginBottom: "var(--space-2)",
        }}
      >
        <span
          style={{
            fontFamily: "var(--font-display)",
            fontSize: "var(--type-caption)",
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: "0.06em",
            color: "var(--text-secondary)",
          }}
        >
          Scenario P&amp;L · spot × vol
        </span>
        <span
          style={{
            marginLeft: "auto",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-micro)",
            color: "var(--text-tertiary)",
          }}
        >
          P&amp;L in {unit} · click a cell to drill
        </span>
      </div>

      <div style={{ position: "relative", width: "100%" }}>
        <div
          ref={containerRef}
          style={{ width: "100%", height, cursor: valid ? "pointer" : "default" }}
          role="img"
          aria-label={label}
        />
        {valid && !libLoaded && (
          <div
            aria-hidden="true"
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              pointerEvents: "none",
              fontFamily: "var(--font-display)",
              fontSize: "var(--type-caption)",
              letterSpacing: "0.04em",
              color: "var(--text-tertiary)",
            }}
          >
            loading scenario grid…
          </div>
        )}
        {!valid && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              gap: "var(--space-2)",
              textAlign: "center",
              padding: "var(--space-4)",
            }}
          >
            <span
              aria-hidden="true"
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: "var(--type-display)",
                color: "var(--text-tertiary)",
              }}
            >
              —
            </span>
            <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)" }}>
              no scenario grid — supply a rectangular P&amp;L matrix aligned to the spot × vol axes
            </span>
          </div>
        )}
      </div>

      {valid && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-1)",
            marginTop: "var(--space-2)",
            fontSize: "var(--type-micro)",
            color: "var(--text-tertiary)",
          }}
        >
          <span style={{ fontFamily: "var(--font-mono)", paddingRight: "var(--space-2)" }}>loss</span>
          {DIV_RAMP.map((token) => (
            <span
              key={token}
              aria-hidden="true"
              style={{ width: 30, height: 10, background: `var(${token})` }}
            />
          ))}
          <span style={{ fontFamily: "var(--font-mono)", paddingLeft: "var(--space-2)" }}>gain</span>
          <span style={{ flex: 1 }} />
          <span style={{ fontFamily: "var(--font-mono)" }}>
            {formatValue(-absMax)} {unit} … 0 … {formatValue(absMax)} {unit} · symmetric · diverging, centred 0
          </span>
        </div>
      )}
    </div>
  );
}
