/**
 * RiskDashboardWorkspace — the per-book RISK dashboard (docs/FI-RISK-ROUTING-
 * REQUIREMENTS.md §6.3, §8.6). Reads each enabled risk book's rolled-up risk from
 * `listRiskBookRisk()` (net/gross base-currency notional, position count, the
 * additive greeks Δ/Γ/Vega/Θ, and a per-cap limit-utilization strip with RAG bands)
 * plus the book roster for names / tree order. A heat OVERVIEW ranks every book by
 * its worst limit utilization; selecting a book (or a row) shows its full breakdown.
 *
 * `dv01` / `pnl` arrive as `null` when NOT yet evaluable at this seam (rates DV01 /
 * mark PnL) — rendered as "—", never as a fabricated 0. The RPC is admin-gated
 * server-side; this pane is read-only for everyone (a risk-management view).
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import type { RiskLimitUtilization, RagBand, RiskBook, RiskBookRisk } from "../data/contract";
import styles from "./RiskDashboardWorkspace.module.css";

const notional = (n: number): string =>
  new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(n);

const greek = (n: number): string =>
  new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 3 }).format(n);

/** Render an optional metric: `null` ⇒ the honest "—" (not yet evaluated), never 0. */
const optMetric = (n: number | null): string => (n === null ? "—" : notional(n));

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

export function RiskDashboardWorkspace(): React.ReactElement {
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
            Per-book rolled-up risk — each book aggregates its own routed positions plus every
            descendant's. DV01 and PnL show “—” until the rates-book and mark passes are wired.
          </p>
        </div>
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {/* --- heat overview across all books --- */}
      <section className={styles.overview} aria-label="Risk heat overview">
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">Book</th>
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
                Δ
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
                <td colSpan={7} className={styles.empty}>
                  No enabled risk books to report.
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
                  <td className={styles.num}>{greek(r.delta)}</td>
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

          <div className={styles.stats}>
            <Stat label="Net notional" value={notional(selected.netNotional)} mono />
            <Stat label="Gross notional" value={notional(selected.grossNotional)} mono />
            <Stat label="Positions" value={String(selected.positionCount)} mono />
            <Stat label="Δ Delta" value={greek(selected.delta)} mono />
            <Stat label="Γ Gamma" value={greek(selected.gamma)} mono />
            <Stat label="Vega" value={greek(selected.vega)} mono />
            <Stat label="Θ Theta" value={greek(selected.theta)} mono />
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
