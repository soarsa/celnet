/**
 * CubeWorkspace — the vol-cube pivot/heatmap (TRADING-UNIVERSE-SCALE §5: the SOTA
 * scale-UX for scanning a whole vol universe at a glance). It is the read/scan
 * counterpart to the editable marking grid in Surface: where Surface marks one
 * pair's smiles, the cube *pivots* the calibrated vol across pair × tenor × delta
 * and tints each cell by magnitude so the eye finds the kinks, the rich/cheap
 * wings, and the term-structure shape without reading numbers.
 *
 * Two pivots, one cube (the standard vol-trader scan axes):
 *   • Smile pivot   — rows = tenor, cols = delta pillar, heat = that smile point's
 *                     vol (one pair). The classic "read a surface as a heatmap".
 *   • Universe pivot — rows = pair, cols = tenor, heat = the chosen metric (ATM /
 *                     25Δ RR / 25Δ BF). Scans the whole pair universe at once.
 *
 * HONESTY (CLAUDE.md rule 2 — no fakes): every vol shown is the SERVER's
 * calibrated smile point, read through the `useCube` hook (which never does
 * client-side vol math). A cell the server has no point for is an honest EMPTY
 * ("—", neutral), never interpolated. Multi-pair scope is registry-ready but
 * honest: it pivots the CURRENT seeded pair set (the hundreds-of-pairs registry
 * is Phase-1 backlog P1-10) — when the registry lands, `app.universe` grows and
 * the cube grows with it, no code change.
 *
 * API-FIRST PARITY (rule 11): no server-ownable capability is computed here. The
 * cube only *arranges and tints* numbers the server already calibrated and the
 * cube hook fetched over the real transport.
 *
 * Drill: clicking any cell calls `onDrill(pair, tenorYears, delta)` so the host
 * (Surface) re-targets that pair and selects that (tenor, delta) on the marking
 * grid + smile — cube → smile, the standard scan-then-inspect flow.
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { CcyPair } from "../data/contract";
import { Panel } from "../components/Panel";
import { rampColor, rampGradient } from "../viz/ramp";
import { fmtVol, fmtVolPoint } from "../lib/format";
import {
  deltaPillarLabel,
  useCube,
  type CubeCell,
  type CubeGrid,
} from "../data/cube";
import { pairLabel, type UniversePair } from "../lib/universe";
import { TENOR_LADDER } from "../data/seed";
import styles from "./CubeWorkspace.module.css";

/** How the trader drills from a cube cell back into the marking/smile view. */
export interface CubeDrill {
  pair: CcyPair;
  tenorYears: number;
  /** Signed delta pillar of the cell, or null for a tenor-only (ATM) cell. */
  delta: number | null;
}

export interface CubeWorkspaceProps {
  /** Drill from a cell to its smile in the host (Surface) view. */
  onDrill: (d: CubeDrill) => void;
}

/** The two pivots the cube can show. */
type Pivot = "smile" | "universe";

/**
 * The universe-pivot metric: which calibrated quantity to tint a (pair × tenor)
 * cell with. ATM is a level (one-sided ramp); RR/BF are signed skew/convexity
 * measures (diverging ramp centred at zero). Purpose-named, vendor-neutral.
 */
type UnivMetric = "atm" | "rr25" | "bf25";
const UNIV_METRICS: { id: UnivMetric; label: string; hint: string; signed: boolean }[] = [
  { id: "atm", label: "ATM", hint: "At-the-money vol level", signed: false },
  { id: "rr25", label: "25Δ RR", hint: "25-delta risk-reversal (skew)", signed: true },
  { id: "bf25", label: "25Δ BF", hint: "25-delta butterfly (smile convexity)", signed: true },
];

/** Format a cube value for the cell label, by whether it is a level or a spread. */
function fmtCell(value: number | null, signed: boolean): string {
  if (value === null) return "—";
  return signed ? fmtVolPoint(value) : fmtVol(value);
}

/**
 * Map a value to a ramp position in [0,1]. For an unsigned level we normalise
 * across the visible value band; for a signed spread we centre zero at 0.5 and
 * scale by the band's max magnitude (so positive=warm, negative=cool, zero=neutral).
 */
function rampT(value: number, band: { min: number; max: number }, signed: boolean): number {
  if (signed) {
    const mag = Math.max(Math.abs(band.min), Math.abs(band.max), 1e-9);
    return 0.5 + value / (2 * mag);
  }
  const span = band.max - band.min || 1e-9;
  return (value - band.min) / span;
}

export function CubeWorkspace({ onDrill }: CubeWorkspaceProps): React.ReactElement {
  const app = useApp();
  const [pivot, setPivot] = useState<Pivot>("smile");
  const [metric, setMetric] = useState<UnivMetric>("atm");

  return (
    <Panel
      glyph="▦"
      title={`Vol cube · ${pivot === "smile" ? pairLabel(app.pairCtx.pair) : "universe"}`}
      className={styles.panel}
      noPadding
      actions={
        <div className={styles.pivotToggle} role="group" aria-label="cube pivot">
          <button
            type="button"
            className={`${styles.pivotChip} ${pivot === "smile" ? styles.pivotActive : ""}`}
            aria-pressed={pivot === "smile"}
            onClick={() => setPivot("smile")}
            title="Tenor × delta heatmap of the active pair's surface"
          >
            Smile
          </button>
          <button
            type="button"
            className={`${styles.pivotChip} ${pivot === "universe" ? styles.pivotActive : ""}`}
            aria-pressed={pivot === "universe"}
            onClick={() => setPivot("universe")}
            title="Pair × tenor heatmap across the whole universe"
          >
            Universe
          </button>
        </div>
      }
    >
      <div className={styles.body}>
        {pivot === "smile" ? (
          <SmilePivot onDrill={onDrill} />
        ) : (
          <UniversePivot metric={metric} setMetric={setMetric} onDrill={onDrill} />
        )}
      </div>
    </Panel>
  );
}

/* ── Smile pivot: rows = tenor, cols = delta pillar, heat = vol (one pair) ── */

function SmilePivot({ onDrill }: { onDrill: (d: CubeDrill) => void }): React.ReactElement {
  const { grid, loading, error } = useCube();

  // Visible vol band across all filled cells — the ramp normalises against it so
  // the heat reads relative within the surface (the eye finds the kinks/wings).
  const band = useMemo(() => volBand(grid), [grid]);

  if (loading && !grid) {
    return <div className={styles.empty}>Assembling cube…</div>;
  }
  if (error) {
    return (
      <div className={styles.empty}>
        <span className={styles.errorMark}>cube unavailable</span>
        <span className={styles.emptyHint}>{error}</span>
      </div>
    );
  }
  if (!grid) {
    return <div className={styles.empty}>No surface for this pair yet.</div>;
  }

  const filled = band !== null;

  return (
    <div className={styles.scanWrap}>
      <table className={styles.cube} aria-label="tenor by delta vol cube">
        <thead>
          <tr>
            <th className={styles.corner} scope="col">
              {fmtSurfaceVersion(grid)}
            </th>
            {grid.deltas.map((d) => (
              <th key={d} className={styles.colHead} scope="col">
                {deltaPillarLabel(d)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {grid.rows.map((row) => (
            <tr key={row.tenorLabel}>
              <th className={styles.rowHead} scope="row">
                {row.tenorLabel}
              </th>
              {row.cells.map((cell) => (
                <HeatCell
                  key={cell.delta}
                  value={cell.vol}
                  band={band}
                  signed={false}
                  label={fmtCell(cell.vol, false)}
                  title={`${grid.pairLabel} ${row.tenorLabel} ${deltaPillarLabel(cell.delta)}${
                    cell.vol === null ? " — unmarked" : ` · ${fmtVol(cell.vol)}`
                  }`}
                  onClick={
                    cell.vol === null
                      ? undefined
                      : () =>
                          onDrill({ pair: grid.pair, tenorYears: row.tenorYears, delta: cell.delta })
                  }
                />
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {filled ? (
        <HeatLegend band={band} signed={false} />
      ) : (
        <div className={styles.emptyHint}>No calibrated points on this surface yet.</div>
      )}
    </div>
  );
}

/** Compose a small surface-version corner badge for the smile pivot. */
function fmtSurfaceVersion(grid: CubeGrid): string {
  return grid.surfaceVersion > 0n ? `v${grid.surfaceVersion.toString()}` : "—";
}

/* ── Universe pivot: rows = pair, cols = tenor, heat = chosen metric ── */

function UniversePivot({
  metric,
  setMetric,
  onDrill,
}: {
  metric: UnivMetric;
  setMetric: (m: UnivMetric) => void;
  onDrill: (d: CubeDrill) => void;
}): React.ReactElement {
  const app = useApp();
  const pairs = app.universe.all;
  const signed = UNIV_METRICS.find((m) => m.id === metric)?.signed ?? false;

  return (
    <div className={styles.scanWrap}>
      <div className={styles.metricBar} role="group" aria-label="universe metric">
        {UNIV_METRICS.map((m) => (
          <button
            key={m.id}
            type="button"
            className={`${styles.metricChip} ${m.id === metric ? styles.metricActive : ""}`}
            aria-pressed={m.id === metric}
            onClick={() => setMetric(m.id)}
            title={m.hint}
          >
            {m.label}
          </button>
        ))}
        <span className={styles.scopeNote} title="The cube pivots the currently-loaded pair universe. The full pair registry (P1-10) is not yet built; when it lands, this grid grows with it.">
          {pairs.length} pair{pairs.length === 1 ? "" : "s"} · current universe
        </span>
      </div>
      <table className={styles.cube} aria-label="pair by tenor vol cube">
        <thead>
          <tr>
            <th className={styles.corner} scope="col" />
            {TENOR_LADDER.map((t) => (
              <th key={t.label} className={styles.colHead} scope="col">
                {t.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {pairs.map((p) => (
            <UniverseRow
              key={p.id}
              up={p}
              metric={metric}
              signed={signed}
              onDrill={onDrill}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * One universe-pivot row: a pair, one `useCube(pair)` worth of vols pivoted by
 * tenor. Each row fetches its own surface through the hook — registry-ready: the
 * grid scales to however many pairs `app.universe.all` carries. The row's band is
 * shared across the whole pivot via a two-pass render? No: to keep the band
 * comparable across pairs we compute a per-row band; cross-row comparison is by
 * absolute label + a shared ramp scale per metric class (signed vs level).
 */
function UniverseRow({
  up,
  metric,
  signed,
  onDrill,
}: {
  up: UniversePair;
  metric: UnivMetric;
  signed: boolean;
  onDrill: (d: CubeDrill) => void;
}): React.ReactElement {
  const { grid, loading, error } = useCube(up.pair);

  // The value at each tenor for the chosen metric, derived from the calibrated
  // cube cells (ATM = the -0.5 pillar; RR/BF from the wing pillars). Honest null
  // where the server carries no points there.
  const values = useMemo(() => metricByTenor(grid, metric), [grid, metric]);
  const band = useMemo(() => valueBand(values, signed), [values, signed]);

  return (
    <tr>
      <th className={styles.rowHead} scope="row" title={up.label}>
        {up.label}
      </th>
      {TENOR_LADDER.map((t, i) => {
        const value = values[i] ?? null;
        const titleBase = `${up.label} ${t.label} ${metric.toUpperCase()}`;
        const title =
          value === null
            ? `${titleBase} — ${loading ? "loading…" : error ? "unavailable" : "unmarked"}`
            : `${titleBase} · ${fmtCell(value, signed)}`;
        return (
          <HeatCell
            key={t.label}
            value={value}
            band={band}
            signed={signed}
            label={value === null && loading ? "·" : fmtCell(value, signed)}
            title={title}
            onClick={
              value === null
                ? undefined
                : () =>
                    onDrill({
                      pair: up.pair,
                      tenorYears: t.years,
                      // ATM metric drills to the ATM pillar; skew/convexity to the
                      // 25Δ put wing (the canonical inspection point for those).
                      delta: metric === "atm" ? -0.5 : -0.25,
                    })
            }
          />
        );
      })}
    </tr>
  );
}

/* ── Shared heat cell + legend ── */

function HeatCell({
  value,
  band,
  signed,
  label,
  title,
  onClick,
}: {
  value: number | null;
  band: { min: number; max: number } | null;
  signed: boolean;
  label: string;
  title: string;
  onClick?: (() => void) | undefined;
}): React.ReactElement {
  const empty = value === null;
  const bg =
    !empty && band !== null
      ? rampColor(Math.max(0, Math.min(1, rampT(value, band, signed))))
      : undefined;
  return (
    <td className={styles.cellWrap}>
      <button
        type="button"
        className={`num ${styles.cell} ${empty ? styles.cellEmpty : ""}`}
        style={bg ? { background: bg } : undefined}
        onClick={onClick}
        disabled={onClick === undefined}
        title={title}
        aria-label={title}
      >
        {label}
      </button>
    </td>
  );
}

function HeatLegend({
  band,
  signed,
}: {
  band: { min: number; max: number };
  signed: boolean;
}): React.ReactElement {
  return (
    <div className={styles.legend}>
      <span className={styles.legendLabel}>
        {signed ? fmtVolPoint(-Math.max(Math.abs(band.min), Math.abs(band.max))) : fmtVol(band.min)}
      </span>
      <span className={styles.legendBar} style={{ background: rampGradient() }} />
      <span className={styles.legendLabel}>
        {signed ? fmtVolPoint(Math.max(Math.abs(band.min), Math.abs(band.max))) : fmtVol(band.max)}
      </span>
    </div>
  );
}

/* ── Pure derivations (no vol math — only selecting & ranging server values) ── */

/** The min/max calibrated vol across every filled cell of a grid, or null if none. */
function volBand(grid: CubeGrid | null): { min: number; max: number } | null {
  if (!grid) return null;
  let min = Infinity;
  let max = -Infinity;
  for (const row of grid.rows) {
    for (const c of row.cells) {
      if (c.vol !== null) {
        if (c.vol < min) min = c.vol;
        if (c.vol > max) max = c.vol;
      }
    }
  }
  return Number.isFinite(min) && Number.isFinite(max) && max > min ? { min, max } : null;
}

/** Range a set of (possibly null) values into a band, or null if degenerate. */
function valueBand(
  values: (number | null)[],
  signed: boolean,
): { min: number; max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (const v of values) {
    if (v !== null) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  if (max === min) {
    // A single distinct value: give it a non-degenerate band so it tints mid.
    return signed ? { min: -Math.abs(max) || -1e-6, max: Math.abs(max) || 1e-6 } : { min, max: min + 1e-6 };
  }
  return { min, max };
}

/**
 * Pull the chosen metric per ladder tenor out of a calibrated cube grid. ATM is
 * the -0.5 pillar; the 25Δ RR/BF are derived ARITHMETICALLY from the calibrated
 * wing & ATM pillars the server already produced — this is a market-standard
 * RECOMBINATION of server outputs (RR = call − put; BF = ½(call+put) − ATM), NOT
 * a re-calibration, so API-first parity holds (we only rearrange the server's
 * own calibrated smile points). A tenor missing any needed pillar yields null.
 */
function metricByTenor(grid: CubeGrid | null, metric: UnivMetric): (number | null)[] {
  if (!grid) return TENOR_LADDER.map(() => null);
  return TENOR_LADDER.map((t) => {
    const row = grid.rows.find((r) => Math.abs(r.tenorYears - t.years) <= 2 / 365);
    if (!row) return null;
    const at = (delta: number): number | null =>
      row.cells.find((c: CubeCell) => Math.abs(c.delta - delta) <= 1e-6)?.vol ?? null;
    if (metric === "atm") return at(-0.5);
    const put = at(-0.25);
    const call = at(0.25);
    const atm = at(-0.5);
    if (put === null || call === null) return null;
    if (metric === "rr25") return call - put;
    // 25Δ butterfly: ½(call+put) − ATM.
    if (atm === null) return null;
    return 0.5 * (call + put) - atm;
  });
}
