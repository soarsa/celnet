/**
 * RiskDashboardWorkspace — the consolidated fixed-income RISK surface: ONE rail entry,
 * "Risk", whose tabbed shell spans SEVEN sibling views that were previously separate
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
 *   • **Deals** — the executed-deals blotter ({@link DealsBlotterWorkspace}) incl. the
 *     routed Risk-Portfolio column + a BUY/SELL indicator per row.
 * The Positions/Quotes/Deals ledger views were previously nested one level deeper
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

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { useAcceptanceSeed } from "../app/AcceptanceSeedContext";
import type {
  CapabilityAction,
  RiskLimitUtilization,
  RagBand,
  RiskBook,
  RiskBookRisk,
} from "../data/contract";
import { RiskBooksWorkspace } from "./RiskBooksWorkspace";
import { RiskRoutingWorkspace } from "./riskrouting/RiskRoutingWorkspace";
import { AcceptanceWorkspace } from "./acceptance/AcceptanceWorkspace";
import { RatesBookWorkspace } from "./RatesBookWorkspace";
import { QuotesBlotterWorkspace } from "./QuotesBlotterWorkspace";
import { DealsBlotterWorkspace } from "./DealsBlotterWorkspace";
import { RiskSetupWizard } from "./risksetup/RiskSetupWizard";
import styles from "./RiskDashboardWorkspace.module.css";

/** The tab the consolidated Risk surface shows. `dashboard` is the default; the other
 * management tabs are the deep-link targets for the retired standalone rail entries —
 * `portfolios` (old "Risk Portfolios"), `routing` (old "Risk Routing") and `acceptance`
 * (old "Acceptance"). The FI position-ledger views folded in from the old "Book" —
 * `positions`, `quotes` and `deals` — are now TOP-LEVEL siblings (previously nested a
 * level deeper inside a "Scenario" tab, which is removed along with the FI
 * scenario-risk surface). */
export type RiskDashboardTab =
  | "dashboard"
  | "portfolios"
  | "routing"
  | "acceptance"
  | "positions"
  | "quotes"
  | "deals";

/**
 * One row per tab the consolidated Risk surface spans: its id, toggle label, and the
 * capability ACTION that gates it × fixed_income (each tab keeps its ORIGINAL gate —
 * Dashboard/Portfolios/Routing on `risk_manage`, Acceptance on `manage_acceptance`,
 * and the folded-in ledger views (Positions/Quotes/Deals) on the `view` floor, so a
 * booking-only FI trader still reaches them exactly as under the former Scenario
 * fold). A tab the identity cannot view is hidden and the active tab clamps to the
 * first visible one (never shown empty).
 */
const RISK_TABS: readonly { tab: RiskDashboardTab; label: string; cap: CapabilityAction }[] = [
  { tab: "dashboard", label: "Dashboard", cap: "risk_manage" },
  { tab: "portfolios", label: "Portfolios", cap: "risk_manage" },
  { tab: "routing", label: "Routing", cap: "risk_manage" },
  { tab: "acceptance", label: "Acceptance", cap: "manage_acceptance" },
  { tab: "positions", label: "Positions", cap: "view" },
  { tab: "quotes", label: "Quotes", cap: "view" },
  { tab: "deals", label: "Deals", cap: "view" },
];

const notional = (n: number): string =>
  new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(n);

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
 * DashboardPanel — the per-portfolio rolled-up RISK view (this workspace's original
 * body, unchanged). The empty-state's "create a portfolio" affordance now switches
 * to the sibling Portfolios tab via {@link onGoToPortfolios} instead of pointing at a
 * separate rail entry.
 */
function DashboardPanel({
  onGoToPortfolios,
}: {
  onGoToPortfolios: () => void;
}): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [risk, setRisk] = useState<RiskBookRisk[]>([]);
  const [books, setBooks] = useState<RiskBook[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [live, setLive] = useState(false);

  // Apply a fresh risk-book set (from a pushed frame or the fallback poll): keep the
  // current selection if it still exists, else fall to the first book.
  const applyRisk = useCallback((rows: RiskBookRisk[]): void => {
    setRisk(rows);
    setLoadError(null);
    setSelectedId((prev) =>
      prev && rows.some((x) => x.bookId === prev) ? prev : (rows[0]?.bookId ?? null),
    );
  }, []);

  useEffect(() => {
    if (!signedIn) {
      setRisk([]);
      setBooks([]);
      setSelectedId(null);
      setLive(false);
      return;
    }
    let cancelled = false;

    // The book roster (names / desk / tree order) is one-shot; only the risk rows
    // stream. Load it alongside the subscription.
    void app.transport
      .listRiskBooks()
      .then((b) => {
        if (!cancelled) setBooks(b);
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load books");
      });

    // Prefer the LIVE push: subscribe to `RiskBookRisk` frames over the multiplexed
    // RFS session, applying only a frame whose `version` is not older than the last.
    const subscribe = app.transport.subscribeRiskBookRisk;
    if (typeof subscribe === "function") {
      try {
        let lastVersion = -1;
        const teardown = subscribe.call(app.transport, (rows, version) => {
          if (cancelled || version < lastVersion) return;
          lastVersion = version;
          applyRisk(rows);
        });
        setLive(true);
        return () => {
          cancelled = true;
          teardown();
        };
      } catch {
        // Fall through to the one-shot poll on a transport that errors on subscribe.
      }
    }

    // Graceful fallback: a transport without the push (or one that threw) polls once.
    setLive(false);
    void app.transport
      .listRiskBookRisk()
      .then((r) => {
        if (!cancelled) applyRisk(r);
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load risk");
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn, applyRisk]);

  const deskOf = useCallback(
    (bookId: string): string | null => books.find((b) => b.id === bookId)?.deskId ?? null,
    [books],
  );

  const selected = useMemo(
    () => risk.find((r) => r.bookId === selectedId) ?? null,
    [risk, selectedId],
  );

  // Firm-wide exposure across ALL enabled portfolios — a pure fold over the streamed
  // rows, so the desk reads total exposure without summing rows by eye.
  const global = useMemo(() => globalExposureOf(risk), [risk]);

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

      {/* --- heat overview across all books --- */}
      {/* tabIndex makes the horizontally-scrollable region keyboard-reachable (axe
          scrollable-region-focusable) so a keyboard user can scroll the wide table. */}
      <section className={styles.overview} aria-label="Risk heat overview" tabIndex={0}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">Portfolio</th>
              <th scope="col" className={styles.numCol}>
                Net
              </th>
              <th scope="col" className={styles.numCol}>
                Gross
              </th>
              <th scope="col" className={styles.numCol}>
                Positions
              </th>
              <th scope="col" className={styles.numCol}>
                DV01
              </th>
              <th scope="col">Limits</th>
            </tr>
          </thead>
          <tbody>
            {risk.length === 0 && (
              <tr>
                <td colSpan={6} className={styles.empty}>
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
            {risk.map((r) => {
              const band = worstBand(r.limits);
              return (
                <tr
                  key={r.bookId}
                  className={r.bookId === selectedId ? styles.rowActive : undefined}
                  onClick={() => setSelectedId(r.bookId)}
                  aria-current={r.bookId === selectedId}
                >
                  <td>
                    <span className={`${styles.dot} ${styles[`band_${band}`]}`} aria-hidden />
                    {r.name}
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
              );
            })}
          </tbody>
        </table>
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
        </section>
      )}
    </div>
  );
}

/**
 * RiskDashboardWorkspace — the tabbed shell composing the SEVEN consolidated FI-risk
 * views as sibling tabs (see the file header): the rolled-up {@link DashboardPanel},
 * the {@link RiskBooksWorkspace} portfolio editor, the {@link RiskRoutingWorkspace}
 * fill-routing builder, the {@link AcceptanceWorkspace} accept/reject builder, and the
 * three folded-in FI position-ledger views — {@link RatesBookWorkspace} (Positions),
 * {@link QuotesBlotterWorkspace} (Quotes) and {@link DealsBlotterWorkspace} (Deals),
 * composed VERBATIM. Mirrors the Risk Transfer / Pricing tab primitive VERBATIM: a
 * slim segmented bar above the active panel, which fills the remaining pane height and
 * scrolls its OWN content (the Shell pane is overflow:hidden with a definite height).
 * Only the active tab's body mounts, so each panel's effects fire only while it is on
 * screen.
 *
 * Each tab keeps its ORIGINAL capability gate: Dashboard/Portfolios/Routing on
 * `risk_manage·FI`, Acceptance on `manage_acceptance·FI`, and Positions/Quotes/Deals
 * on the `view·FI` floor (the same floor they had under the removed Scenario fold). A
 * tab the identity cannot view is HIDDEN and the active tab clamps to the first
 * visible one, so a hidden tab is never shown empty (`can` is permissive signed-out,
 * so pre-login every tab renders). Reaching the host ROW itself follows the rail's
 * `risk_manage·FI` viewCap; the cross-asset `risk` scenario grid stays a standalone
 * row on the FX rail, only WITHDRAWN from the FI rail (DOMAIN_RAIL_EXCLUDED).
 */
export function RiskDashboardWorkspace({
  initialTab = "dashboard",
}: {
  /** The initial tab — the `riskbooks` deep-link opens on `portfolios`, `riskrouting`
   * on `routing`, `acceptance` on `acceptance`; the rail's "Risk" entry (and
   * stories/tests) default to `dashboard`. */
  initialTab?: RiskDashboardTab;
} = {}): React.ReactElement {
  const { auth } = useApp();
  const { pending: acceptanceSeed } = useAcceptanceSeed();

  const [tab, setTab] = useState<RiskDashboardTab>(initialTab);
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

  const visibleTabs = RISK_TABS.filter(
    (t) =>
      auth.can(t.cap, "fixed_income") || (t.tab === "acceptance" && seedRevealAcceptance),
  );
  // Clamp to a VISIBLE tab so a deep-link (or default) landing on a tab this identity
  // cannot view falls to the first tab it can, never an empty pane.
  const activeTab: RiskDashboardTab = visibleTabs.some((t) => t.tab === tab)
    ? tab
    : (visibleTabs[0]?.tab ?? "dashboard");

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
        ) : (
          // The executed-deals blotter incl. the routed Risk-Portfolio column and the
          // per-row BUY/SELL indicator.
          <DealsBlotterWorkspace />
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
