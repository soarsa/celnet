/**
 * LatencyOpsWorkspace — the per-stage pipeline-latency / ops-observability table
 * (docs/ANALYTICS-REQUIREMENTS.md §11 latency). Reads the server digest via
 * `listLatencyMetrics()` and renders one row per instrumented stage of the
 * tick→quote→book pipeline, in the server's emit order:
 *
 *   Price (pinned core) → Surface rebuild → Spread/tiering → Aggregation →
 *   Quote publish → RFQ respond → Quote accept → Ack→fill→book
 *
 * Each row is a nanosecond HdrHistogram digest: count, p50 / p99 / p99.9 / p99.99,
 * max and mean, formatted in **adaptive units** (ns / µs / ms) so a sub-µs pinned
 * core and a multi-ms book round-trip are both readable. A zero/absent field renders
 * "—", never a fabricated value. A log-scale "latency spectrum" strip places each
 * stage's p50●—p99—p99.9 on a shared axis so the whole pipeline's tail is one glance.
 *
 * A telemetry-health strip reports the bounded offload queue (guardrail 11: the pinned
 * hot core stays alloc/log/lock-free and offloads samples over a bounded queue):
 * drained / dropped / observed gaps / tick frequency.
 *
 * Read-only and gated on `view_analytics` — the tab, its rail entry and the query are
 * all hidden/denied without it (docs/operations/PERMISSIONS-GRANULAR-REVIEW.md).
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import { TableSearch } from "../../components/TableSearch";
import type { LatencyHealth, LatencyMetrics, LatencyStage } from "../../data/contract";
import styles from "./LatencyOpsWorkspace.module.css";

// --- formatting (adaptive ns/µs/ms; — for absent/zero) -----------------------

const NS_PER_US = 1_000;
const NS_PER_MS = 1_000_000;

/**
 * A nanosecond latency in adaptive units — ns below 1µs, µs below 1ms, ms above.
 * A zero or absent value renders "—" (an unsampled stage), never "0 ns".
 */
function fmtLatency(ns: number | undefined): string {
  if (ns === undefined || ns <= 0) return "—";
  if (ns < NS_PER_US) return `${Math.round(ns)} ns`;
  if (ns < NS_PER_MS) {
    const us = ns / NS_PER_US;
    return `${us.toFixed(us < 10 ? 2 : 1)} µs`;
  }
  const ms = ns / NS_PER_MS;
  return `${ms.toFixed(ms < 10 ? 2 : 1)} ms`;
}

// --- p99 traffic-light tint (latency-threshold row colouring) ----------------

/**
 * Row-tint thresholds in **milliseconds**, applied to each stage's p99 (the
 * standard tail-SLA metric). Boundaries (kept identical in the legend):
 *   • p99 < 1 ms          → green  (healthy)
 *   • 1 ms ≤ p99 ≤ 2 ms   → amber  (watch)
 *   • p99 > 2 ms          → red    (over-SLA)
 * The edges are inclusive on the amber band: exactly 1 ms and exactly 2 ms are
 * amber, never green/red.
 */
const LATENCY_GREEN_MS = 1;
const LATENCY_RED_MS = 2;

/** The traffic-light band a stage falls in, or undefined when p99 is unsampled. */
export type LatencyTone = "green" | "amber" | "red";

/**
 * Classify a stage by its p99 tail latency (raw nanoseconds → ms) into a
 * traffic-light band. An unsampled p99 (≤ 0) returns undefined — an absent tail
 * must not read as a healthy green. Boundaries per `LATENCY_GREEN_MS`/`_RED_MS`.
 */
export function latencyTone(p99Ns: number): LatencyTone | undefined {
  if (p99Ns <= 0) return undefined;
  const ms = p99Ns / NS_PER_MS;
  if (ms < LATENCY_GREEN_MS) return "green";
  if (ms <= LATENCY_RED_MS) return "amber";
  return "red";
}

/** Maps a traffic-light band to its module tint class (sets `--lat-tint`). */
const TONE_CLASS: Record<LatencyTone, string> = {
  green: styles.latGreen ?? "",
  amber: styles.latAmber ?? "",
  red: styles.latRed ?? "",
};

const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });

/** Integer count with compact suffix (4.21M). Zero renders "—". */
function fmtCount(n: number): string {
  return n <= 0 ? "—" : compact.format(n);
}

/** A plain integer with thousands separators (drained/dropped/gaps). */
function fmtInt(n: number): string {
  return new Intl.NumberFormat("en-US").format(n);
}

/** The histogram tick frequency in MHz (24 MHz), or Hz/GHz as the magnitude warrants. */
function fmtHz(hz: number): string {
  if (hz <= 0) return "—";
  if (hz >= 1_000_000_000) return `${(hz / 1_000_000_000).toFixed(2)} GHz`;
  if (hz >= 1_000_000) return `${(hz / 1_000_000).toFixed(hz % 1_000_000 === 0 ? 0 : 1)} MHz`;
  if (hz >= 1_000) return `${(hz / 1_000).toFixed(1)} kHz`;
  return `${hz} Hz`;
}

// --- latency spectrum (shared log axis) --------------------------------------

/** log10 with a floor so a 0/negative sample can't produce -Infinity. */
function log10(x: number): number {
  return Math.log10(Math.max(x, 1));
}

/**
 * The shared [min, max] nanosecond domain across all stages, as log10 endpoints — the
 * spectrum strip maps every stage's percentiles onto this one axis so the fast pinned
 * core and the slow book round-trip read on the same scale.
 */
function spectrumDomain(stages: readonly LatencyStage[]): { lo: number; hi: number } {
  let min = Number.POSITIVE_INFINITY;
  let max = 0;
  for (const s of stages) {
    if (s.minNs > 0) min = Math.min(min, s.minNs);
    if (s.maxNs > 0) max = Math.max(max, s.maxNs);
    if (s.p50Ns > 0) min = Math.min(min, s.p50Ns);
  }
  if (!Number.isFinite(min) || max <= 0) return { lo: 0, hi: 1 };
  return { lo: log10(min), hi: log10(Math.max(max, min * 10)) };
}

/** Position (0–100%) of a nanosecond value on the shared log axis. */
function spectrumPos(ns: number, dom: { lo: number; hi: number }): number {
  if (ns <= 0) return 0;
  const span = dom.hi - dom.lo;
  if (span <= 0) return 0;
  const p = ((log10(ns) - dom.lo) / span) * 100;
  return p < 0 ? 0 : p > 100 ? 100 : p;
}

// --- live-refresh cadence ----------------------------------------------------

/**
 * Trailing-debounce for the notification-driven revalidate: a fill storm from the
 * sub-second FIX sim coalesces into at most ~1 refetch per this window.
 */
const REVALIDATE_DEBOUNCE_MS = 750;

/**
 * The idle poll: latency is a continuously-sampled histogram, so the digest keeps
 * moving even with no deals booking — refresh on this low cadence so the table stays
 * fresh independent of notification traffic.
 */
const IDLE_REFRESH_MS = 5_000;

// --- column model ------------------------------------------------------------

type SortDir = "asc" | "desc";

interface ColumnDef {
  key: string;
  label: string;
  title: string;
  sortValue: (s: LatencyStage) => number | string;
}

/** The numeric metric columns (the Stage label is the row header, sorted separately). */
const COLUMNS: readonly ColumnDef[] = [
  { key: "count", label: "Count", title: "Samples in the digest", sortValue: (s) => s.count },
  { key: "p50Ns", label: "p50", title: "Median latency", sortValue: (s) => s.p50Ns },
  { key: "p99Ns", label: "p99", title: "99th-percentile latency", sortValue: (s) => s.p99Ns },
  { key: "p999Ns", label: "p99.9", title: "99.9th-percentile latency", sortValue: (s) => s.p999Ns },
  { key: "maxNs", label: "max", title: "Maximum observed latency", sortValue: (s) => s.maxNs },
  { key: "meanNs", label: "mean", title: "Arithmetic mean latency", sortValue: (s) => s.meanNs },
];

/** Compare two sort values; strings via locale, numbers numerically. */
function compareSort(a: number | string, b: number | string, dir: SortDir): number {
  let cmp: number;
  if (typeof a === "string" || typeof b === "string") cmp = String(a).localeCompare(String(b));
  else cmp = a - b;
  return dir === "asc" ? cmp : -cmp;
}

export function LatencyOpsWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [metrics, setMetrics] = useState<LatencyMetrics | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // Default: no sort key ⇒ preserve the server's pipeline emit order (top-to-bottom
  // is the tick→quote→book flow, the natural way an ops reader scans the pipeline).
  const [sortKey, setSortKey] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  // Search matches the stage's display label AND its raw op tag — the label is what the
  // eye reads, the op is what a trace or a log line carries, and an operator arriving
  // from either should find the row.
  const [query, setQuery] = useState("");
  // The one filter this table genuinely wants: "show me only what is slow". A stage's
  // band is derived from p99, so it is not a column you could filter as text.
  const [toneFilter, setToneFilter] = useState<LatencyTone | "">("");

  const load = useCallback((): void => {
    if (!signedIn) {
      setMetrics(null);
      setLoadError(null);
      return;
    }
    setLoading(true);
    void app.transport
      .listLatencyMetrics()
      .then((m) => {
        setMetrics(m);
        setLoadError(null);
      })
      .catch((e: unknown) => {
        setMetrics(null);
        setLoadError(e instanceof Error ? e.message : "failed to load latency metrics");
      })
      .finally(() => setLoading(false));
  }, [app.transport, signedIn]);

  useEffect(() => {
    load();
  }, [load]);

  // Live refresh: latency is a continuously-sampled histogram, so give it BOTH a
  // notification-driven revalidate (trailing-debounced, like the Risk Dashboard drill-
  // down, so the FIX sim's fill storm coalesces into ≤1 refetch/REVALIDATE_DEBOUNCE_MS)
  // AND a low-frequency idle poll so the digest stays fresh even when nothing is booking.
  // The mount effect above owns the FIRST fetch; both timers + the disposer are torn
  // down on unmount. Best-effort: a transport without the push seam still idle-polls.
  useEffect(() => {
    if (!signedIn) return;
    let debounce: ReturnType<typeof setTimeout> | undefined;
    const revalidate = (): void => {
      if (debounce !== undefined) clearTimeout(debounce);
      debounce = setTimeout(() => {
        debounce = undefined;
        load();
      }, REVALIDATE_DEBOUNCE_MS);
    };
    const interval = setInterval(load, IDLE_REFRESH_MS);
    const stream = app.transport.streamNotifications;
    const dispose =
      typeof stream === "function"
        ? stream.call(app.transport, undefined, revalidate)
        : undefined;
    return () => {
      if (debounce !== undefined) clearTimeout(debounce);
      clearInterval(interval);
      dispose?.();
    };
  }, [app.transport, signedIn, load]);

  // Sort a header: same key toggles direction; a new key starts descending (ops reads
  // the worst tail first). The initial (unsorted) state preserves pipeline emit order.
  const onSort = useCallback((key: string): void => {
    setSortKey((prev) => {
      if (prev === key) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"));
        return prev;
      }
      setSortDir("desc");
      return key;
    });
  }, []);

  const stages = metrics?.stages ?? [];
  const dom = useMemo(() => spectrumDomain(stages), [stages]);

  /** Stages surviving the search + band filter, before sorting. */
  const matchedRows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return stages.filter((s) => {
      if (toneFilter !== "" && latencyTone(s.p99Ns) !== toneFilter) return false;
      if (needle === "") return true;
      return `${s.stageLabel} ${s.op}`.toLowerCase().includes(needle);
    });
  }, [stages, query, toneFilter]);

  const visibleRows = useMemo(() => {
    if (sortKey === null) return matchedRows;
    const rows = [...matchedRows];
    if (sortKey === "stage") {
      rows.sort((a, b) => compareSort(a.stageLabel, b.stageLabel, sortDir));
      return rows;
    }
    const col = COLUMNS.find((c) => c.key === sortKey);
    if (!col) return rows;
    rows.sort((a, b) => compareSort(col.sortValue(a), col.sortValue(b), sortDir));
    return rows;
  }, [matchedRows, sortKey, sortDir]);

  const ariaSort = (key: string): "ascending" | "descending" | "none" =>
    sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";

  return (
    <section className={styles.wrap} aria-labelledby="latencyops-title">
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 id="latencyops-title" className={styles.title}>
            Latency / Ops
          </h1>
          <p className={styles.subtitle}>
            Per-stage latency of the tick→quote→book pipeline — p50 / p99 / p99.9 and the
            tail, in adaptive units, plus telemetry-queue health.
          </p>
        </div>
      </header>

      {signedIn && metrics && <HealthStrip health={metrics.health} />}

      <p className={styles.explainer}>
        Each row is one instrumented pipeline stage&apos;s nanosecond histogram. The
        pinned pricing core is <strong>sub-µs</strong>; the venue round-trips (RFQ, accept,
        book) run into the <strong>ms</strong>. The <strong>spectrum</strong> plots every
        stage&apos;s p50&nbsp;●&nbsp;p99&nbsp;p99.9 on one shared log axis — the pipeline&apos;s
        whole latency profile at a glance.
      </p>

      {loadError !== null && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {!signedIn ? (
        <p className={styles.empty}>Sign in to view latency / ops analytics.</p>
      ) : loading && stages.length === 0 ? (
        <p className={styles.empty}>Loading latency metrics…</p>
      ) : stages.length === 0 ? (
        <p className={styles.empty}>No latency metrics reported.</p>
      ) : (
        <>
          <ToneLegend />
          <div className={styles.tableTools}>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={visibleRows.length}
              total={stages.length}
              label="Search stages"
              placeholder="Filter by stage or op…"
            />
            <label className={styles.toneFilter}>
              <span className={styles.toneFilterLabel}>Band</span>
              <select
                className={styles.toneFilterSelect}
                value={toneFilter}
                aria-label="Filter by latency band"
                data-testid="latency-tone-filter"
                onChange={(e) => setToneFilter(e.target.value as LatencyTone | "")}
              >
                <option value="">Any band</option>
                <option value="green">Green — within budget</option>
                <option value="amber">Amber — approaching budget</option>
                <option value="red">Red — over budget</option>
              </select>
            </label>
          </div>
          <div className={styles.tableScroll}>
          <table className={styles.table}>
            <caption className={styles.caption}>
              Per-stage pipeline latency (nanosecond histograms, adaptive units) — spectrum
              on a shared log axis
            </caption>
            <thead>
              <tr>
                <th scope="col" className={styles.thLabel} aria-sort={ariaSort("stage")}>
                  <button type="button" className={styles.sortBtn} onClick={() => onSort("stage")}>
                    Stage
                    <SortGlyph active={sortKey === "stage"} dir={sortDir} />
                  </button>
                </th>
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    scope="col"
                    className={styles.thNum}
                    aria-sort={ariaSort(c.key)}
                    title={c.title}
                  >
                    <button type="button" className={styles.sortBtnNum} onClick={() => onSort(c.key)}>
                      <SortGlyph active={sortKey === c.key} dir={sortDir} />
                      {c.label}
                    </button>
                  </th>
                ))}
                <th scope="col" className={styles.thSpectrum}>
                  Latency spectrum <span className={styles.axisNote}>(log · ns→ms)</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.length === 0 && (
                <tr>
                  <td className={styles.emptyRow} colSpan={COLUMNS.length + 2}>
                    No stage matches this search or band.
                  </td>
                </tr>
              )}
              {visibleRows.map((s) => {
                const tone = latencyTone(s.p99Ns);
                const rowClass = tone ? `${styles.latRow} ${TONE_CLASS[tone]}` : undefined;
                return (
                <tr key={s.op} className={rowClass} data-latency-tone={tone}>
                  <th scope="row" className={styles.rowLabel}>
                    <span className={styles.stageName}>{s.stageLabel}</span>
                    <span className={styles.opTag}>{s.op}</span>
                  </th>
                  <td className={styles.num}>{fmtCount(s.count)}</td>
                  <td className={styles.num}>{fmtLatency(s.p50Ns)}</td>
                  <td className={styles.num}>{fmtLatency(s.p99Ns)}</td>
                  <td className={styles.num}>{fmtLatency(s.p999Ns)}</td>
                  <td className={styles.num}>{fmtLatency(s.maxNs)}</td>
                  <td className={styles.num}>{fmtLatency(s.meanNs)}</td>
                  <td className={styles.spectrumCell}>
                    <SpectrumBar stage={s} dom={dom} />
                  </td>
                </tr>
                );
              })}
            </tbody>
          </table>
          </div>
        </>
      )}
    </section>
  );
}

/**
 * The p99-tail traffic-light key: green < 1 ms, amber 1–2 ms, red > 2 ms. Sits by
 * the table header so the row tint is self-explanatory. The colour labels are real
 * text (screen-reader legible); the swatches are decorative reinforcement.
 */
function ToneLegend(): React.ReactElement {
  return (
    <div className={styles.legend}>
      <span className={styles.legendLabel}>Row tint — p99 tail:</span>
      <span className={styles.legendItem}>
        <span className={`${styles.legendDot} ${styles.latGreen}`} aria-hidden />
        &lt;1&nbsp;ms
      </span>
      <span className={styles.legendItem}>
        <span className={`${styles.legendDot} ${styles.latAmber}`} aria-hidden />
        1–2&nbsp;ms
      </span>
      <span className={styles.legendItem}>
        <span className={`${styles.legendDot} ${styles.latRed}`} aria-hidden />
        &gt;2&nbsp;ms
      </span>
    </div>
  );
}

/** The telemetry offload-queue health strip — four stat tiles. */
function HealthStrip({ health }: { health: LatencyHealth }): React.ReactElement {
  const dropped = health.droppedTotal;
  const gaps = health.observedGaps;
  return (
    <div className={styles.healthStrip} role="group" aria-label="Telemetry queue health">
      <HealthTile label="Drained" value={fmtInt(health.drainedTotal)} hint="Samples drained from the offload queue into the digests" />
      <HealthTile
        label="Dropped"
        value={fmtInt(dropped)}
        tone={dropped > 0 ? "warn" : "ok"}
        hint="Samples dropped when the bounded queue was full (the hot core never blocks)"
      />
      <HealthTile
        label="Gaps"
        value={fmtInt(gaps)}
        tone={gaps > 0 ? "danger" : "ok"}
        hint="Observed gaps in the sample sequence (a monotonic-counter skip)"
      />
      <HealthTile label="Tick freq" value={fmtHz(health.tickHz)} hint="Histogram tick frequency the nanosecond figures derive from" />
    </div>
  );
}

function HealthTile({
  label,
  value,
  tone = "ok",
  hint,
}: {
  label: string;
  value: string;
  tone?: "ok" | "warn" | "danger";
  hint: string;
}): React.ReactElement {
  return (
    <div className={`${styles.healthTile} ${styles[`tone-${tone}`]}`} title={hint}>
      <span className={styles.healthLabel}>{label}</span>
      <span className={styles.healthValue}>{value}</span>
    </div>
  );
}

/**
 * The per-stage spectrum: a track on the shared log axis with a fill from p50→p99.9
 * (the tail spread), a p50 marker, and a p99 marker. Purely decorative (the numbers
 * are the source of truth) so it is `aria-hidden`; the numeric columns carry the data.
 */
function SpectrumBar({
  stage,
  dom,
}: {
  stage: LatencyStage;
  dom: { lo: number; hi: number };
}): React.ReactElement {
  const p50 = spectrumPos(stage.p50Ns, dom);
  const p99 = spectrumPos(stage.p99Ns, dom);
  const p999 = spectrumPos(stage.p999Ns, dom);
  const left = Math.min(p50, p999);
  const width = Math.max(Math.abs(p999 - p50), 1.5);
  return (
    <span
      className={styles.spectrum}
      aria-hidden
      title={`p50 ${fmtLatency(stage.p50Ns)} · p99 ${fmtLatency(stage.p99Ns)} · p99.9 ${fmtLatency(stage.p999Ns)}`}
    >
      <span className={styles.spectrumTrack} />
      <span className={styles.spectrumFill} style={{ left: `${left}%`, width: `${width}%` }} />
      <span className={styles.spectrumP99} style={{ left: `${p99}%` }} />
      <span className={styles.spectrumP50} style={{ left: `${p50}%` }} />
    </span>
  );
}

/** A tiny sort-direction caret shown on the active sort column. */
function SortGlyph({ active, dir }: { active: boolean; dir: SortDir }): React.ReactElement {
  return (
    <span className={styles.sortGlyph} aria-hidden>
      {active ? (dir === "asc" ? "▲" : "▼") : "↕"}
    </span>
  );
}
