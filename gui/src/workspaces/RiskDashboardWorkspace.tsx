/**
 * RiskDashboardWorkspace — the consolidated fixed-income RISK surface: ONE rail entry,
 * "Risk", whose tabbed shell spans NINE sibling views that were previously separate
 * rail destinations (mirroring the Pricing and Transfers→Risk Transfer merges):
 *   • **Dashboard** (default) — the per-portfolio rolled-up risk view ({@link
 *     DashboardPanel}); the routed-risk roll-up (docs/FI-RISK-ROUTING-REQUIREMENTS.md
 *     §6.3, §8.6).
 *   • **Portfolios** — the create / enable / edit / limits / hierarchy editor
 *     ({@link RiskBooksWorkspace}, composed VERBATIM), the target of the Dashboard's
 *     empty-state link and the retired "Risk Portfolios" (`riskbooks`) deep-link.
 *   • **Routing** — the fill → portfolio rule builder ({@link RiskRoutingWorkspace}),
 *     the retired "Risk Routing" (`riskrouting`) rail entry, composed wholesale.
 *   • **Acceptance** — the incoming-lift accept/reject rule builder ({@link
 *     AcceptanceWorkspace}), the retired "Acceptance" (`acceptance`) rail entry.
 *   • **Positions** — the rates position ledger + booking form ({@link
 *     RatesBookWorkspace}, composed VERBATIM), folded in from the old FI "Book".
 *   • **Quotes** — the shown-quotes blotter ({@link QuotesBlotterWorkspace}); what was
 *     SHOWN (quoted), the non-redundant sibling of Deals.
 *   • **Client blotter** — the executed CLIENT-deals blotter ({@link
 *     DealsBlotterWorkspace}, forced to its client lens) incl. the routed
 *     Risk-Portfolio column + a BUY/SELL indicator per row.
 *   • **Hedge blotter** — the executed-HEDGE ledger ({@link HedgeDealsView}): who we
 *     hedged with, at what price, for how much — the sibling of the client blotter.
 *   • **Hedge flows** — the live hedge-engine monitor ({@link HedgeMonitor},
 *     self-fetching): engine status, per-book RAG, live hedges, needs-attention.
 *     Gated on `hedge·FI` (the hedge-engine view); moved here from the Hedging Rules
 *     surface's removed Monitor tab.
 * The Positions/Quotes/Client-blotter ledger views were previously nested one level deeper
 * inside a "Scenario" tab (which composed `RiskWorkspace` at its FI rates lens); that
 * tab AND its netted rates scenario-risk surface are REMOVED, and the three ledger
 * views are promoted to top-level siblings here — the existing table components are
 * composed directly, unchanged. The cross-asset `risk` scenario grid STAYS a
 * standalone FX-rail row (DOMAIN_RAIL_EXCLUDED withdraws it from the FI rail only).
 * Each tab keeps its ORIGINAL capability gate independently (see the shell function),
 * hiding a tab the identity cannot view and clamping the active tab to the first
 * visible one. The standalone "Risk Portfolios" / "Risk Routing" / "Acceptance" rail
 * entries are removed (`lib/commands.ts`); their ids deep-link straight to the matching
 * tab (`app/Shell.tsx` + CONSOLIDATED_WORKSPACE_ALIAS).
 *
 * The DASHBOARD tab ({@link DashboardPanel}) reads each enabled risk portfolio's
 * rolled-up risk from
 * `listRiskBookRisk()` (net/gross base-currency notional, position count, the
 * additive greeks Δ/Γ/Vega/Θ, and a per-cap limit-utilization strip with RAG bands)
 * plus the roster for names / tree order. A heat OVERVIEW ranks every portfolio by
 * its worst limit utilization; selecting a portfolio (or a row) shows its breakdown.
 *
 * `dv01` / `pnl` arrive as `null` when NOT yet evaluable at this seam (rates DV01 /
 * mark PnL) — rendered as "—", never as a fabricated 0. The RPC is admin-gated
 * server-side; this pane is read-only for everyone (a risk-management view).
 */

import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { useAcceptanceSeed } from "../app/AcceptanceSeedContext";
import { TableSkeleton } from "../components/TableSkeleton";
import {
  primeCachedResource,
  useCachedResource,
} from "../hooks/useCachedResource";
import { useTableUiState } from "../hooks/useTableUiState";
import type {
  CapabilityAction,
  Deal,
  RiskLimitUtilization,
  RagBand,
  RiskBook,
  RiskBookRisk,
} from "../data/contract";
import { fmtCompact } from "../lib/format";
import { bucketDealsByBook, dealsForBook } from "../data/riskBreakdown";
import { RiskBreakdownGrid } from "./RiskBreakdownGrid";
import { RiskBooksWorkspace } from "./RiskBooksWorkspace";
import { RiskRoutingWorkspace } from "./riskrouting/RiskRoutingWorkspace";
import { AcceptanceWorkspace } from "./acceptance/AcceptanceWorkspace";
import { RatesBookWorkspace } from "./RatesBookWorkspace";
import { QuotesBlotterWorkspace } from "./QuotesBlotterWorkspace";
import { DealsBlotterWorkspace } from "./DealsBlotterWorkspace";
import { HedgeDealsView } from "./HedgeDealsView";
import { HedgeMonitor } from "./hedging/HedgeMonitor";
import { RiskSetupWizard } from "./risksetup/RiskSetupWizard";
import styles from "./RiskDashboardWorkspace.module.css";

/** The tab the consolidated Risk surface shows. `dashboard` is the default; the other
 * management tabs are the deep-link targets for the retired standalone rail entries —
 * `portfolios` (old "Risk Portfolios"), `routing` (old "Risk Routing") and `acceptance`
 * (old "Acceptance"). The FI position-ledger views folded in from the old "Book" —
 * `positions`, `quotes` and `clientblotter` (the client-deals blotter) — are TOP-LEVEL
 * siblings (previously nested a level deeper inside a removed "Scenario" tab). The
 * `hedgeblotter` (executed-hedge ledger) and `hedgeflows` (live hedge-engine monitor,
 * moved from the Hedging Rules surface) round out the hedge side of the surface. */
export type RiskDashboardTab =
  | "dashboard"
  | "portfolios"
  | "routing"
  | "acceptance"
  | "positions"
  | "quotes"
  | "clientblotter"
  | "hedgeblotter"
  | "hedgeflows";

/**
 * One row per tab the consolidated Risk surface spans: its id, toggle label, and the
 * capability ACTION that gates it × fixed_income (each tab keeps its ORIGINAL gate —
 * Dashboard/Portfolios/Routing on `risk_manage`, Acceptance on `manage_acceptance`,
 * the folded-in ledger views (Positions/Quotes/Client blotter/Hedge blotter) on the
 * `view` floor, and Hedge flows on `hedge` (the hedge-engine monitor), so a
 * booking-only FI trader still reaches the ledgers but the live hedge monitor stays
 * hedge-gated). A tab the identity cannot view is hidden and the active tab clamps to
 * the first visible one (never shown empty).
 */
const RISK_TABS: readonly { tab: RiskDashboardTab; label: string; cap: CapabilityAction }[] = [
  { tab: "dashboard", label: "Dashboard", cap: "risk_manage" },
  { tab: "portfolios", label: "Portfolios", cap: "risk_manage" },
  { tab: "routing", label: "Routing", cap: "risk_manage" },
  { tab: "acceptance", label: "Acceptance", cap: "manage_acceptance" },
];

/**
 * The **ledger** tab set — the read-side blotters, hosted under **Fixed Income**
 * (rail row `filedgers`) rather than under the Risk tab.
 *
 * The split is by QUESTION ASKED, not by implementation. Risk answers "how is the desk
 * CONFIGURED, and what is it carrying" — portfolios, routing, acceptance, the roll-up
 * dashboard: firm-shape management surfaces, `risk_manage`-class, set up rarely. These
 * five answer "what actually HAPPENED" — the per-deal record a trader reads all day.
 * Together they made a nine-tab strip mixing two audiences.
 *
 * Each tab KEEPS its original gate (the `view` floor for the four ledgers, `hedge` for
 * the live hedge-flow monitor), so the move changes only WHERE a surface is reached —
 * never WHO can reach it.
 */
const LEDGER_TABS: readonly { tab: RiskDashboardTab; label: string; cap: CapabilityAction }[] = [
  { tab: "positions", label: "Positions", cap: "view" },
  { tab: "quotes", label: "Quotes", cap: "view" },
  { tab: "clientblotter", label: "Client blotter", cap: "view" },
  { tab: "hedgeblotter", label: "Hedge blotter", cap: "view" },
  { tab: "hedgeflows", label: "Hedge flows", cap: "hedge" },
];

/** Which tab set a mounted instance presents (see {@link LEDGER_TABS}). */
export type RiskWorkspaceVariant = "risk" | "ledgers";

/**
 * Compact magnitude formatting — the SHARED app formatter (`lib/format.fmtCompact`),
 * so the dashboard reads identically to the Deals / Positions blotters (`1.25m`, not
 * a divergent `1.25M` from a bespoke `Intl` compact) and its digits line up under the
 * tabular-figure numeric columns.
 */
const notional = fmtCompact;

/** Render an optional metric: `null` ⇒ the honest "—" (not yet evaluated), never 0. */
const optMetric = (n: number | null): string => (n === null ? "—" : notional(n));

/** RAG bands from a utilization fraction (mirrors the server `RagBand::from_fraction`). */
function bandOfFraction(fraction: number): RagBand {
  if (fraction >= 1) return "red";
  if (fraction >= 0.8) return "amber";
  return "green";
}

/**
 * The firm-wide roll-up across EVERY enabled risk portfolio — a pure client-side fold
 * over the rows already streamed, so the desk reads total exposure without summing by
 * eye. FI-relevant measures only (net / gross notional, positions, DV01, PnL) plus a
 * per-cap aggregate limit utilization (sum used / sum limit across portfolios). DV01
 * and PnL stay ABSENT (never a fabricated 0) until at least one portfolio reports one.
 */
function globalExposureOf(rows: readonly RiskBookRisk[]): {
  net: number;
  gross: number;
  positions: number;
  dv01: number | null;
  pnl: number | null;
  limits: RiskLimitUtilization[];
} {
  let net = 0;
  let gross = 0;
  let positions = 0;
  let dv01: number | null = null;
  let pnl: number | null = null;
  const used = new Map<string, number>();
  const cap = new Map<string, number>();
  for (const r of rows) {
    net += r.netNotional;
    gross += r.grossNotional;
    positions += r.positionCount;
    if (r.dv01 !== null) dv01 = (dv01 ?? 0) + r.dv01;
    if (r.pnl !== null) pnl = (pnl ?? 0) + r.pnl;
    for (const l of r.limits) {
      used.set(l.metric, (used.get(l.metric) ?? 0) + l.used);
      cap.set(l.metric, (cap.get(l.metric) ?? 0) + l.limit);
    }
  }
  const limits: RiskLimitUtilization[] = [...cap.keys()].map((metric) => {
    const u = used.get(metric) ?? 0;
    const limit = cap.get(metric) ?? 0;
    const fraction = limit > 0 ? u / limit : u > 0 ? Number.POSITIVE_INFINITY : 0;
    return { metric, used: u, limit, fraction, band: bandOfFraction(fraction) };
  });
  return { net, gross, positions, dv01, pnl, limits };
}

/** The worst (highest-fraction) utilization band across a book's caps, or green. */
function worstBand(limits: readonly RiskLimitUtilization[]): RagBand {
  let band: RagBand = "green";
  for (const l of limits) {
    if (l.band === "red") return "red";
    if (l.band === "amber") band = "amber";
  }
  return band;
}

/** The RAG bar for one utilization: a clamped fill width + a band colour class. */
function UtilizationBar({ util }: { util: RiskLimitUtilization }): React.ReactElement {
  const pct = Number.isFinite(util.fraction)
    ? Math.min(100, Math.max(0, util.fraction * 100))
    : 100;
  const label = util.metric.replace(/_/g, " ");
  return (
    <div className={styles.util}>
      <div className={styles.utilHead}>
        <span className={styles.utilMetric}>{label}</span>
        <span className={styles.utilRatio}>
          {notional(util.used)} / {notional(util.limit)}{" "}
          <span className={styles.utilPct}>
            ({Number.isFinite(util.fraction) ? `${(util.fraction * 100).toFixed(0)}%` : "breach"})
          </span>
        </span>
      </div>
      <div className={styles.bar}>
        <div
          className={`${styles.barFill} ${styles[`band_${util.band}`]}`}
          style={{ width: `${pct}%` }}
          role="meter"
          aria-valuenow={Math.round(pct)}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label={`${label} utilization`}
        />
      </div>
    </div>
  );
}

/**
 * The per-portfolio roster column the overview is sorted on. Gross notional is the
 * standard EXPOSURE measure, so it is the default sort key (descending — highest
 * exposure first). Every key maps to a numeric roster field; DV01 is null-aware.
 */
type RiskSortKey = "net" | "gross" | "positions" | "dv01";
type SortDir = "asc" | "desc";

/** Human column labels for the active-sort sub-note. */
const SORT_LABELS: Record<RiskSortKey, string> = {
  net: "Net",
  gross: "Gross",
  positions: "Positions",
  dv01: "DV01",
};

/** The numeric roster value a sort key reads (null only for the not-yet-wired DV01). */
function sortValue(r: RiskBookRisk, key: RiskSortKey): number | null {
  switch (key) {
    case "net":
      return r.netNotional;
    case "gross":
      return r.grossNotional;
    case "positions":
      return r.positionCount;
    case "dv01":
      return r.dv01;
  }
}

/**
 * Order the roster by the active sort — a pure, stable sort over a COPY (never
 * mutating the streamed rows). A null metric (an unevaluated DV01) always sorts to
 * the BOTTOM regardless of direction, so the honest "—" cells never crowd the top.
 */
function sortRisk(
  rows: readonly RiskBookRisk[],
  key: RiskSortKey,
  dir: SortDir,
): RiskBookRisk[] {
  const sign = dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const av = sortValue(a, key);
    const bv = sortValue(b, key);
    if (av === null && bv === null) return 0;
    if (av === null) return 1; // nulls last, both directions
    if (bv === null) return -1;
    return sign * (av - bv);
  });
}

/** The `aria-sort` token for a column header given the active sort. */
function ariaSortFor(active: boolean, dir: SortDir): "ascending" | "descending" | "none" {
  if (!active) return "none";
  return dir === "asc" ? "ascending" : "descending";
}

/**
 * A sortable numeric column header — a real `<button>` (keyboard-operable) inside a
 * `<th className="numCol">` carrying `aria-sort`. The active column shows a directional
 * caret (▲ asc / ▼ desc); an inactive one shows a dim neutral caret as the affordance.
 * Clicking the active column toggles direction; clicking another selects it.
 */
function SortHeader({
  label,
  sortKey,
  activeKey,
  dir,
  onSort,
}: {
  label: string;
  sortKey: RiskSortKey;
  activeKey: RiskSortKey;
  dir: SortDir;
  onSort: (key: RiskSortKey) => void;
}): React.ReactElement {
  const active = activeKey === sortKey;
  return (
    <th scope="col" className={styles.numCol} aria-sort={ariaSortFor(active, dir)}>
      <button
        type="button"
        className={`${styles.sortBtn} ${active ? styles.sortBtnActive : ""}`}
        data-testid={`risk-sort-${sortKey}`}
        onClick={() => onSort(sortKey)}
      >
        {label}
        <span className={styles.sortCaret} aria-hidden="true">
          {active ? (dir === "asc" ? "▲" : "▼") : "▾"}
        </span>
      </button>
    </th>
  );
}

/**
 * DashboardPanel — the per-portfolio rolled-up RISK view (this workspace's original
 * body, unchanged). The empty-state's "create a portfolio" affordance now switches
 * to the sibling Portfolios tab via {@link onGoToPortfolios} instead of pointing at a
 * separate rail entry.
 */
export function DashboardPanel({
  onGoToPortfolios,
}: {
  onGoToPortfolios: () => void;
}): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [live, setLive] = useState(false);

  // Persisted table UI state: the column sort, the selected portfolio, and which rows
  // are expanded all SURVIVE a tab switch (the workspace unmounting) and are restored
  // on return. Default sort: GROSS notional DESCENDING (highest exposure first).
  const [ui, setUi] = useTableUiState<{
    sort: { key: RiskSortKey; dir: SortDir };
    selectedId: string | null;
    expanded: string[];
  }>("fi-risk-dashboard", {
    sort: { key: "gross", dir: "desc" },
    selectedId: null,
    expanded: [],
  });
  const sort = ui.sort;
  const selectedId = ui.selectedId;
  const setSelectedId = useCallback((id: string | null) => setUi({ selectedId: id }), [setUi]);

  // Toggle direction on the active column; select another column fresh at descending
  // (largest-first, the exposure-ranked default).
  const onSort = useCallback(
    (key: RiskSortKey): void => {
      setUi({
        sort:
          ui.sort.key === key
            ? { key, dir: ui.sort.dir === "asc" ? "desc" : "asc" }
            : { key, dir: "desc" },
      });
    },
    [setUi, ui.sort],
  );

  // Stale-while-revalidate cache for the per-portfolio risk roll-up: the rows SURVIVE
  // the workspace unmounting on a tab switch, so returning shows them instantly (no
  // blank flash). The poll fetcher backs a cache MISS; the LIVE push (below) primes the
  // SAME cache line on every frame, so while mounted the rows stay live without a poll.
  const {
    data: riskData,
    isLoading: riskLoading,
    error: riskError,
  } = useCachedResource<RiskBookRisk[]>(
    "riskBookRisk",
    () => app.transport.listRiskBookRisk(),
    { enabled: signedIn },
  );
  const risk = useMemo(() => riskData ?? [], [riskData]);
  const loadError =
    riskError === undefined || riskError === null
      ? null
      : riskError instanceof Error
        ? riskError.message
        : "failed to load risk";

  // Prefer the LIVE push: subscribe to `RiskBookRisk` frames over the multiplexed RFS
  // session and PRIME the cache with each not-older frame (the payload is delivered by
  // the push itself — no refetch). A transport without the seam simply relies on the
  // poll fetcher above.
  useEffect(() => {
    if (!signedIn) {
      setLive(false);
      return;
    }
    const subscribe = app.transport.subscribeRiskBookRisk;
    if (typeof subscribe !== "function") {
      setLive(false);
      return;
    }
    try {
      let lastVersion = -1;
      const teardown = subscribe.call(app.transport, (rows, version) => {
        if (version < lastVersion) return;
        lastVersion = version;
        primeCachedResource<RiskBookRisk[]>("riskBookRisk", rows);
      });
      setLive(true);
      return () => {
        setLive(false);
        teardown();
      };
    } catch {
      // A transport that errors on subscribe relies on the poll fetcher above.
      setLive(false);
    }
    return undefined;
  }, [app.transport, signedIn]);

  // The book roster (names / desk / tree order) — cached + shared with other surfaces
  // that list risk books, so it does not blank on a tab switch either.
  const { data: booksData } = useCachedResource<RiskBook[]>(
    "riskBooks",
    () => app.transport.listRiskBooks(),
    { enabled: signedIn },
  );
  const books = useMemo(() => booksData ?? [], [booksData]);

  // Reconcile the selected portfolio against the current rows: keep a still-present
  // selection, else fall to the first book (or none).
  useEffect(() => {
    if (risk.length === 0) {
      if (selectedId !== null) setSelectedId(null);
      return;
    }
    if (selectedId && risk.some((x) => x.bookId === selectedId)) return;
    setSelectedId(risk[0]?.bookId ?? null);
  }, [risk, selectedId, setSelectedId]);

  // The routed deals for the drill-down: cached (survives unmount) and refreshed on
  // every push `Notification` (a fill mints a deal), exactly as the Deals blotter does.
  // Best-effort: a transport without the seam simply leaves the breakdown empty.
  const { data: dealsData, refresh: refreshDeals } = useCachedResource<Deal[]>(
    "riskDeals",
    () => {
      const listDeals = app.transport.listDeals;
      return typeof listDeals === "function"
        ? listDeals.call(app.transport, {}).then((res) => res.deals)
        : Promise.resolve([]);
    },
    { enabled: signedIn },
  );
  const deals = useMemo(() => dealsData ?? [], [dealsData]);
  const refreshDealsRef = useRef(refreshDeals);
  refreshDealsRef.current = refreshDeals;
  useEffect(() => {
    if (!signedIn) return;
    const stream = app.transport.streamNotifications;
    const dispose =
      typeof stream === "function"
        ? stream.call(app.transport, undefined, () => refreshDealsRef.current())
        : undefined;
    return () => dispose?.();
  }, [app.transport, signedIn]);

  const deskOf = useCallback(
    (bookId: string): string | null => books.find((b) => b.id === bookId)?.deskId ?? null,
    [books],
  );

  // Routed deals bucketed by the portfolio their risk routed into (`riskBookId`),
  // so the drill-down for a book is an O(1) lookup rather than a per-row filter. The
  // bucket keys AND the per-book lookup normalize the id (see
  // `data/riskBreakdown.normalizeBookKey`) so a routed `Deal.riskBookId` still joins
  // its `RiskBookRisk.bookId` even under casing/format drift between the two seams —
  // the failure mode where the breakdown came back blank on live.
  const dealsByBook = useMemo(() => bucketDealsByBook(deals), [deals]);

  // Expanded rows are persisted (as an id list) so open drill-downs survive a tab
  // switch. A Set view keeps the per-row lookup O(1) in render.
  const expandedSet = useMemo(() => new Set(ui.expanded), [ui.expanded]);
  const toggleExpanded = useCallback(
    (bookId: string): void => {
      setUi({
        expanded: ui.expanded.includes(bookId)
          ? ui.expanded.filter((x) => x !== bookId)
          : [...ui.expanded, bookId],
      });
    },
    [setUi, ui.expanded],
  );

  const selected = useMemo(
    () => risk.find((r) => r.bookId === selectedId) ?? null,
    [risk, selectedId],
  );

  // Firm-wide exposure across ALL enabled portfolios — a pure fold over the streamed
  // rows, so the desk reads total exposure without summing rows by eye.
  const global = useMemo(() => globalExposureOf(risk), [risk]);

  // The roster in its active sort order (default: gross desc = highest exposure first).
  const sortedRisk = useMemo(() => sortRisk(risk, sort.key, sort.dir), [risk, sort]);

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.empty}>Sign in to view the risk dashboard.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>
            Risk Dashboard
            {live && (
              <span className={styles.liveTag} role="status" aria-label="Live risk stream">
                <span className={styles.liveDot} aria-hidden />
                live
              </span>
            )}
          </h1>
          <p className={styles.note}>
            Per-portfolio rolled-up risk — each risk portfolio aggregates its own routed positions
            plus every descendant&apos;s. DV01 and PnL show “—” until the rates-book and mark passes
            are wired. This is how routed risk is bucketed for management — not the ledger
            &ldquo;Book&rdquo; where fills are booked, nor the &ldquo;Agg Book&rdquo; of LP prices.
          </p>
        </div>
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {/* --- firm-wide global exposure across ALL portfolios --- */}
      {risk.length > 0 && (
        <section className={styles.global} aria-label="Global exposure across all risk portfolios">
          <div className={styles.globalHead}>
            <h2 className={styles.globalTitle}>Global exposure</h2>
            <span className={styles.globalSub}>
              all {risk.length} portfolio{risk.length === 1 ? "" : "s"}
            </span>
          </div>
          <div className={styles.stats}>
            <Stat label="Net notional" value={notional(global.net)} mono />
            <Stat label="Gross notional" value={notional(global.gross)} mono />
            <Stat label="Positions" value={String(global.positions)} mono />
            <Stat label="DV01" value={optMetric(global.dv01)} mono muted={global.dv01 === null} />
            <Stat label="PnL" value={optMetric(global.pnl)} mono muted={global.pnl === null} />
          </div>
          {global.limits.length > 0 && (
            <div className={styles.utils}>
              {global.limits.map((u) => (
                <UtilizationBar key={u.metric} util={u} />
              ))}
            </div>
          )}
        </section>
      )}

      {/* --- per-portfolio roll-up — the FOCAL panel: bigger rows/type, default-sorted
          by gross notional (exposure) desc, with sortable numeric column headers. --- */}
      <section className={styles.overview} aria-label="Risk portfolios by exposure">
        <div className={styles.overviewHead}>
          <h2 className={styles.overviewTitle}>Portfolios</h2>
          <span className={styles.overviewSub}>
            {risk.length} portfolio{risk.length === 1 ? "" : "s"} · sorted by{" "}
            {SORT_LABELS[sort.key]} {sort.dir === "desc" ? "▼" : "▲"}
            {sort.key === "gross" ? " (exposure)" : ""} · click a column to re-sort
          </span>
        </div>
        {/* tabIndex makes the horizontally-scrollable region keyboard-reachable (axe
            scrollable-region-focusable) so a keyboard user can scroll the wide table. */}
        <div
          className={styles.tableScroll}
          tabIndex={0}
          role="group"
          aria-label="Risk heat overview table"
        >
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col" className={styles.expandCol}>
                <span className={styles.visuallyHidden}>Expand breakdown</span>
              </th>
              <th scope="col">Portfolio</th>
              <SortHeader label="Net" sortKey="net" activeKey={sort.key} dir={sort.dir} onSort={onSort} />
              <SortHeader label="Gross" sortKey="gross" activeKey={sort.key} dir={sort.dir} onSort={onSort} />
              <SortHeader
                label="Positions"
                sortKey="positions"
                activeKey={sort.key}
                dir={sort.dir}
                onSort={onSort}
              />
              <SortHeader label="DV01" sortKey="dv01" activeKey={sort.key} dir={sort.dir} onSort={onSort} />
              <th scope="col">Limits</th>
            </tr>
          </thead>
          <tbody>
            {riskLoading && risk.length === 0 && (
              <tr>
                <td colSpan={7}>
                  <TableSkeleton rows={4} label="Loading risk portfolios…" />
                </td>
              </tr>
            )}
            {!riskLoading && risk.length === 0 && (
              <tr>
                <td colSpan={7} className={styles.empty}>
                  No enabled risk portfolios to report. Routing <em>rules</em> only pick a
                  destination — they do not create the portfolio. Create one on the{" "}
                  <button
                    type="button"
                    className={styles.emptyLink}
                    onClick={onGoToPortfolios}
                    data-testid="empty-goto-portfolios"
                  >
                    Portfolios
                  </button>{" "}
                  tab and mark it <strong>enabled</strong>; routed fills then roll up here.
                </td>
              </tr>
            )}
            {sortedRisk.map((r) => {
              const band = worstBand(r.limits);
              const isOpen = expandedSet.has(r.bookId);
              const panelId = `risk-breakdown-${r.bookId}`;
              return (
                <Fragment key={r.bookId}>
                  <tr
                    className={r.bookId === selectedId ? styles.rowActive : undefined}
                    onClick={() => setSelectedId(r.bookId)}
                    aria-current={r.bookId === selectedId}
                  >
                    <td className={styles.expandCell}>
                      {/* A real disclosure button — keyboard-focusable, aria-expanded/
                          controls the breakdown row; stopPropagation so it toggles the
                          drill-down WITHOUT also selecting the row. */}
                      <button
                        type="button"
                        className={styles.expandBtn}
                        aria-expanded={isOpen}
                        aria-controls={panelId}
                        aria-label={`${isOpen ? "Collapse" : "Expand"} risk breakdown for ${r.name}`}
                        data-testid={`risk-expand-${r.bookId}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleExpanded(r.bookId);
                        }}
                      >
                        <span className={styles.chevron} data-open={isOpen} aria-hidden>
                          ▸
                        </span>
                      </button>
                    </td>
                    <td className={styles.portfolioCell}>
                      <span className={`${styles.dot} ${styles[`band_${band}`]}`} aria-hidden />
                      <span className={styles.portfolioName}>{r.name}</span>
                    </td>
                    <td className={styles.num}>{notional(r.netNotional)}</td>
                    <td className={styles.num}>{notional(r.grossNotional)}</td>
                    <td className={styles.num}>{r.positionCount}</td>
                    <td className={styles.num}>{optMetric(r.dv01)}</td>
                    <td>
                      {r.limits.length === 0 ? (
                        <span className={styles.muted}>none</span>
                      ) : (
                        <span className={`${styles.pill} ${styles[`band_${band}`]}`}>
                          {band}
                        </span>
                      )}
                    </td>
                  </tr>
                  {isOpen && (
                    <tr className={styles.breakdownRow}>
                      <td colSpan={7} id={panelId} className={styles.breakdownCell}>
                        <RiskBreakdownGrid
                          deals={dealsForBook(dealsByBook, r.bookId)}
                          bookName={r.name}
                        />
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
        </div>
      </section>

      {/* --- selected book detail --- */}
      {selected && (
        <section className={styles.detail} aria-label={`Risk detail for ${selected.name}`}>
          <div className={styles.detailHead}>
            <h2 className={styles.detailTitle}>{selected.name}</h2>
            {deskOf(selected.bookId) && (
              <span className={styles.deskTag}>desk · {deskOf(selected.bookId)}</span>
            )}
          </div>

          {/* FI portfolios carry rates risk, not FX-option greeks — so this view shows the
              DV01 family + notional/positions/PnL, never Δ/Γ/Vega/Θ (meaningless for
              rates/bonds). The Risk Dashboard is a fixed-income-only surface. */}
          <div className={styles.stats}>
            <Stat label="Net notional" value={notional(selected.netNotional)} mono />
            <Stat label="Gross notional" value={notional(selected.grossNotional)} mono />
            <Stat label="Positions" value={String(selected.positionCount)} mono />
            <Stat label="DV01" value={optMetric(selected.dv01)} mono muted={selected.dv01 === null} />
            <Stat label="PnL" value={optMetric(selected.pnl)} mono muted={selected.pnl === null} />
          </div>

          <div className={styles.utils}>
            <h3 className={styles.utilsTitle}>Limit utilization</h3>
            {selected.limits.length === 0 ? (
              <p className={styles.muted}>No computable caps configured on this book.</p>
            ) : (
              selected.limits.map((u) => <UtilizationBar key={u.metric} util={u} />)
            )}
          </div>

          {/* The per-product / per-tenor / per-instrument deal-level breakdown for the
              SELECTED book — the same fold the row's expand-caret shows, surfaced here
              so a trader who selects a portfolio reads its composition without also
              having to expand the row. Fed by the SAME normalized deal→book join, so
              it populates on live; the grid renders its own honest empty note when the
              book has no mapped deal-level detail (never a silently omitted section). */}
          <div className={styles.breakdown}>
            <h3 className={styles.utilsTitle}>Risk breakdown</h3>
            <RiskBreakdownGrid
              deals={dealsForBook(dealsByBook, selected.bookId)}
              bookName={selected.name}
            />
          </div>
        </section>
      )}
    </div>
  );
}

/**
 * RiskDashboardWorkspace — the tabbed shell composing the NINE consolidated FI-risk
 * views as sibling tabs (see the file header): the rolled-up {@link DashboardPanel},
 * the {@link RiskBooksWorkspace} portfolio editor, the {@link RiskRoutingWorkspace}
 * fill-routing builder, the {@link AcceptanceWorkspace} accept/reject builder, the
 * folded-in FI position-ledger views — {@link RatesBookWorkspace} (Positions),
 * {@link QuotesBlotterWorkspace} (Quotes) and {@link DealsBlotterWorkspace} (Client
 * blotter, forced to its client lens) — and the hedge side: {@link HedgeDealsView}
 * (Hedge blotter) + the self-fetching {@link HedgeMonitor} (Hedge flows), composed
 * VERBATIM. Mirrors the Risk Transfer / Pricing tab primitive VERBATIM: a
 * slim segmented bar above the active panel, which fills the remaining pane height and
 * scrolls its OWN content (the Shell pane is overflow:hidden with a definite height).
 * Only the active tab's body mounts, so each panel's effects fire only while it is on
 * screen.
 *
 * Each tab keeps its ORIGINAL capability gate: Dashboard/Portfolios/Routing on
 * `risk_manage·FI`, Acceptance on `manage_acceptance·FI`, Positions/Quotes/Client
 * blotter/Hedge blotter on the `view·FI` floor, and Hedge flows on `hedge·FI` (the
 * hedge-engine monitor). A
 * tab the identity cannot view is HIDDEN and the active tab clamps to the first
 * visible one, so a hidden tab is never shown empty (`can` is permissive signed-out,
 * so pre-login every tab renders). Reaching the host ROW itself follows the rail's
 * `risk_manage·FI` viewCap; the cross-asset `risk` scenario grid stays a standalone
 * row on the FX rail, only WITHDRAWN from the FI rail (DOMAIN_RAIL_EXCLUDED).
 */
export function RiskDashboardWorkspace({
  initialTab,
  variant = "risk",
}: {
  /** The initial tab — the `riskbooks` deep-link opens on `portfolios`, `riskrouting`
   * on `routing`, `acceptance` on `acceptance`; the rail's "Risk" entry (and
   * stories/tests) default to `dashboard`. Omitted ⇒ the variant's first tab. */
  initialTab?: RiskDashboardTab;
  /** Which tab set to present: the Risk-tab management views (default) or the
   * Fixed-Income ledger blotters ({@link LEDGER_TABS}). */
  variant?: RiskWorkspaceVariant;
} = {}): React.ReactElement {
  const tabSet = variant === "ledgers" ? LEDGER_TABS : RISK_TABS;
  // Default to the variant's OWN first tab, so the ledgers host opens on Positions
  // rather than a `dashboard` tab it does not present (which would clamp anyway, but
  // via the fallback path rather than by intent).
  const firstTab = tabSet[0]?.tab ?? "dashboard";
  const { auth } = useApp();
  const { pending: acceptanceSeed } = useAcceptanceSeed();

  const [tab, setTab] = useState<RiskDashboardTab>(initialTab ?? firstTab);
  const [wizardOpen, setWizardOpen] = useState(false);
  // A pending acceptance-seed (from a Deals/Quotes row "Create acceptance rule") reveals
  // AND switches to the Acceptance tab. The reveal is sticky so a non-`manage_acceptance`
  // holder — who normally can't see the tab — still LANDS on it (read-only) rather than
  // being clamped away; it drops again once they leave the tab.
  //
  // ONLY the Acceptance-host instance reacts. The Shell keeps every workspace pane mounted
  // (P0-11) and mounts a hidden `acceptance` alias pane (this component with
  // `initialTab="acceptance"`) alongside the visible `riskdashboard` one — so several
  // RiskDashboardWorkspace instances share this app-level seed. The blotter navigates to
  // the `acceptance` alias, so gating on `initialTab === "acceptance"` makes exactly that
  // (now-visible) instance the SINGLE reactor + seed consumer — no hidden pane races the
  // one-shot, and the base dashboard instance never spuriously flips to Acceptance.
  const isAcceptanceHost = initialTab === "acceptance";
  const [seedRevealAcceptance, setSeedRevealAcceptance] = useState(false);
  const seenSeedNonce = useRef(0);
  useEffect(() => {
    if (!isAcceptanceHost) return;
    if (acceptanceSeed && acceptanceSeed.nonce !== seenSeedNonce.current) {
      seenSeedNonce.current = acceptanceSeed.nonce;
      setSeedRevealAcceptance(true);
      setTab("acceptance");
    }
  }, [acceptanceSeed, isAcceptanceHost]);

  const visibleTabs = tabSet.filter(
    (t) =>
      auth.can(t.cap, "fixed_income") || (t.tab === "acceptance" && seedRevealAcceptance),
  );
  // Clamp to a VISIBLE tab so a deep-link (or default) landing on a tab this identity
  // cannot view falls to the first tab it can, never an empty pane.
  const activeTab: RiskDashboardTab = visibleTabs.some((t) => t.tab === tab)
    ? tab
    : (visibleTabs[0]?.tab ?? firstTab);

  // The guided-setup launcher shows for anyone who can actually run any part of the
  // wizard — the risk books/routing half (`risk_manage`) or the acceptance half
  // (`manage_acceptance`). Power users keep the individual tabs.
  const canGuided =
    auth.can("risk_manage", "fixed_income") || auth.can("manage_acceptance", "fixed_income");

  return (
    <div className={styles.shell}>
      <div className={styles.topBar}>
        {canGuided && (
          <button
            type="button"
            className={styles.guidedSetupBtn}
            data-testid="open-risk-guided-setup"
            onClick={() => setWizardOpen(true)}
          >
            <span aria-hidden="true">🪄</span> Guided setup
            <span className={styles.guidedSetupSub}>portfolios · routing · acceptance in one flow</span>
          </button>
        )}
        <div className={styles.tabBar} role="group" aria-label="risk view">
          {visibleTabs.map((t) => (
            <button
              key={t.tab}
              type="button"
              className={`${styles.tabBtn} ${activeTab === t.tab ? styles.tabBtnActive : ""}`}
              aria-pressed={activeTab === t.tab}
              data-testid={`risk-tab-${t.tab}`}
              onClick={() => {
                setTab(t.tab);
                // Leaving the seed-revealed Acceptance tab drops the temporary reveal (a
                // non-holder returns to their normal tab set); staying keeps it.
                if (t.tab !== "acceptance") setSeedRevealAcceptance(false);
              }}
            >
              {t.label}
            </button>
          ))}
        </div>
      </div>
      {/* On apply the wizard navigates to the Risk → Acceptance tab (re-mounting this
          host); on discard it simply closes. Either way onClose clears the overlay. */}
      {wizardOpen && <RiskSetupWizard onClose={() => setWizardOpen(false)} />}
      <div className={styles.tabPanel}>
        {activeTab === "dashboard" ? (
          <DashboardPanel onGoToPortfolios={() => setTab("portfolios")} />
        ) : activeTab === "portfolios" ? (
          <RiskBooksWorkspace />
        ) : activeTab === "routing" ? (
          <RiskRoutingWorkspace />
        ) : activeTab === "acceptance" ? (
          <AcceptanceWorkspace />
        ) : activeTab === "positions" ? (
          // The FI position ledger + booking form (folded in from the old "Book"),
          // composed verbatim — the same table the Book rail row used to render.
          <RatesBookWorkspace />
        ) : activeTab === "quotes" ? (
          // The shown-quotes blotter — what was quoted (non-redundant with Deals).
          <QuotesBlotterWorkspace />
        ) : activeTab === "clientblotter" ? (
          // The executed CLIENT-deals blotter incl. the routed Risk-Portfolio column and
          // the per-row BUY/SELL indicator. Forced to the client lens (its own dedicated
          // tab), so the redundant client/hedge toggle is hidden — the hedge ledger is
          // the sibling "Hedge blotter" tab.
          <DealsBlotterWorkspace lens="client" />
        ) : activeTab === "hedgeblotter" ? (
          // The executed-HEDGE ledger (who we hedged with, at what price, for how much) —
          // its own Risk tab, distinct from the client blotter.
          <HedgeDealsView />
        ) : (
          // The live hedge-engine flow monitor (engine status, per-book RAG, live hedges,
          // needs-attention) — self-fetching, moved here from the Hedging Rules surface.
          <HedgeMonitor />
        )}
      </div>
    </div>
  );
}

/** One labelled stat tile in the selected-book breakdown. */
function Stat({
  label,
  value,
  mono,
  muted,
}: {
  label: string;
  value: string;
  mono?: boolean;
  muted?: boolean;
}): React.ReactElement {
  return (
    <div className={styles.stat}>
      <span className={styles.statLabel}>{label}</span>
      <span
        className={[styles.statValue, mono ? styles.mono : "", muted ? styles.muted : ""]
          .filter(Boolean)
          .join(" ")}
      >
        {value}
      </span>
    </div>
  );
}
