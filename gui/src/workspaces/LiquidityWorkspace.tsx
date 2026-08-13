/**
 * LiquidityWorkspace — the **LP panel**: what inbound liquidity is actually
 * reaching the platform, provider by provider.
 *
 * The Connections workspace answers "which acceptors are defined and bound".
 * This one answers the operational question that follows: of those venues, who
 * is genuinely feeding us prices right now, on how many instruments, how fast,
 * how fresh, and how much of the composite are they actually setting. A venue
 * can be bound and silent, or quoting hard and excluded from every composite as
 * stale — both look identical on the Connections table and obvious here.
 *
 * Layout is a summary strip (the firm-wide facts an operator checks first,
 * starting with the inbound kill-switch) over a sortable provider table, with a
 * per-provider drill-down of live quotes and the consolidation verdict on each.
 */

import { useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type { LiquidityProvider } from "../data/contract";
import { useLiquidityProviders } from "../hooks/useLiquidityProviders";
import styles from "./LiquidityWorkspace.module.css";

/** Nanoseconds per millisecond — the wire carries epoch nanoseconds. */
const NANOS_PER_MS = 1e6;
/** Above this quote age the provider's freshness reads as a warning. */
const STALE_WARN_SECS = 10;

/** The sortable provider columns. */
type SortKey =
  | "connectionId"
  | "rate"
  | "instrumentsQuoted"
  | "freshQuotes"
  | "topOfBook"
  | "meanWeight"
  | "lastQuote";

/** A provider row joined with the client-derived tick rate. */
interface Row extends LiquidityProvider {
  /** Quote-updates per second, or `null` before a rate can be established. */
  rate: number | null;
}

/** Seconds since `nanos`, or `null` when the provider has never quoted. */
function ageSecs(nanos: number, nowMs: number): number | null {
  if (nanos <= 0) return null;
  return Math.max(0, nowMs - nanos / NANOS_PER_MS) / 1000;
}

/** A compact age, e.g. `1.2s` / `45s` / `12m`. */
function formatAge(secs: number | null): string {
  if (secs === null) return "never";
  if (secs < 10) return `${secs.toFixed(1)}s`;
  if (secs < 90) return `${Math.round(secs)}s`;
  return `${Math.round(secs / 60)}m`;
}

/** A tick rate, blank until two samples establish one. */
function formatRate(rate: number | null): string {
  if (rate === null) return "—";
  return rate >= 10 ? rate.toFixed(0) : rate.toFixed(1);
}

/** A price, or an em dash when the provider quoted a non-finite value. */
function formatPrice(price: number): string {
  return Number.isFinite(price) ? price.toFixed(4) : "—";
}

/**
 * The single sentence describing a provider's state, used as the status pill's
 * label AND its accessible name.
 *
 * Precedence is deliberately EVIDENCE-FIRST rather than registry-first: recent
 * ticks prove a provider is working, so nothing a registry says can override
 * them. Only once a provider is not demonstrably live do the configuration
 * faults rank — dropped at ingest (in no book), then the acceptor-backed states,
 * then quiet-vs-stale.
 */
function providerState(
  p: LiquidityProvider,
  secs: number | null,
): { label: string; tone: "bad" | "warn" | "good" | "idle" } {
  // Evidence of quoting outranks everything: a provider whose ticks are arriving
  // IS live, whatever the registries say. This matters because not every provider
  // is a FIX acceptor — an LP streaming over the gRPC LpFeed ingest (lp-sim, and
  // any API-adapter venue) has no FIX connection row at all, so treating a missing
  // one as a fault would flag every healthy gRPC feed as broken.
  if (secs !== null && secs <= STALE_WARN_SECS) return { label: "live", tone: "good" };
  // In no enabled book ⇒ its pushes are dropped at ingest, whatever else is true.
  if (p.bookIds.length === 0) return { label: "in no book", tone: "bad" };
  // The acceptor-backed states only mean anything when an acceptor exists.
  if (p.connectionDefined && !p.enabled) return { label: "disabled", tone: "idle" };
  if (p.connectionDefined && !p.running) return { label: "not bound", tone: "bad" };
  if (secs !== null) return { label: "stale", tone: "warn" };
  // Never quoted. Silent-with-an-acceptor is a warning; silent with no acceptor
  // and no ticks is the genuine misconfiguration — an id matching nothing.
  return p.connectionDefined
    ? { label: "silent", tone: "warn" }
    : { label: "unresolved id", tone: "bad" };
}

export function LiquidityWorkspace(): React.ReactElement {
  const app = useApp();
  const [selected, setSelected] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("rate");
  const [ascending, setAscending] = useState(false);
  const { panel, rates, isLoading, error, refetch } = useLiquidityProviders(
    app.transport,
    selected,
  );

  // The panel's own clock: ages are measured against the instant the SERVER
  // folded the panel, not the browser's wall clock, so a clock skew between the
  // two never shows up as a fake staleness.
  const nowMs = panel.asOfNanos > 0 ? panel.asOfNanos / NANOS_PER_MS : Date.now();

  const rows: Row[] = useMemo(() => {
    const joined: Row[] = panel.providers.map((p) => ({
      ...p,
      rate: rates[p.connectionId] ?? null,
    }));
    const direction = ascending ? 1 : -1;
    const value = (r: Row): number | string => {
      switch (sortKey) {
        case "connectionId":
          return r.connectionId;
        case "rate":
          return r.rate ?? -1;
        case "instrumentsQuoted":
          return r.instrumentsQuoted;
        case "freshQuotes":
          return r.freshQuotes;
        case "topOfBook":
          return r.bestBidCount + r.bestOfferCount;
        case "meanWeight":
          return r.meanWeight;
        case "lastQuote":
          return r.lastQuoteNanos;
      }
    };
    return [...joined].sort((a, b) => {
      const x = value(a);
      const y = value(b);
      if (typeof x === "string" || typeof y === "string") {
        return String(x).localeCompare(String(y)) * direction;
      }
      return (x - y) * direction;
    });
  }, [panel.providers, rates, sortKey, ascending]);

  const totals = useMemo(() => {
    const tones = rows.map((r) => providerState(r, ageSecs(r.lastQuoteNanos, nowMs)).tone);
    return {
      live: tones.filter((t) => t === "good").length,
      totalRate: rows.reduce((sum, r) => sum + (r.rate ?? 0), 0),
      stale: rows.reduce((sum, r) => sum + r.staleQuotes, 0),
      unhealthy: tones.filter((t) => t === "bad" || t === "warn").length,
    };
  }, [rows, nowMs]);

  const sortBy = (key: SortKey): void => {
    if (key === sortKey) {
      setAscending((a) => !a);
      return;
    }
    setSortKey(key);
    // Names read naturally A→Z; every metric reads best biggest-first.
    setAscending(key === "connectionId");
  };

  // Numeric columns right-align their DATA (`.num`), so the header must too —
  // otherwise every figure sits visibly right of the label naming it.
  const header = (
    key: SortKey,
    label: string,
    hint?: string,
    numeric = true,
  ): React.ReactElement => (
    <th className={numeric ? styles.num : undefined}>
      <button
        type="button"
        className={styles.sortHead}
        onClick={() => sortBy(key)}
        aria-sort={sortKey === key ? (ascending ? "ascending" : "descending") : "none"}
        title={hint}
      >
        {label}
        <span className={styles.sortMark} aria-hidden="true">
          {sortKey === key ? (ascending ? "▲" : "▼") : ""}
        </span>
      </button>
    </th>
  );

  const actions = (
    <Button variant="ghost" onClick={() => void refetch()} disabled={isLoading}>
      Refresh
    </Button>
  );

  const selectedRow = rows.find((r) => r.connectionId === selected) ?? null;

  return (
    <div className={styles.root}>
      <Panel title="Inbound liquidity" glyph="⇊" actions={actions}>
        {error && <p className={styles.banner}>{error}</p>}
        {!panel.inboundEnabled && panel.asOfNanos > 0 && (
          <p className={styles.killSwitch}>
            ⏻ Inbound ingest is <strong>disabled</strong> firm-wide — every pushed
            quote is being dropped, so every provider below will go stale.
          </p>
        )}

        <div className={styles.summary}>
          <div className={styles.stat}>
            <span className={styles.statValue}>
              {totals.live}
              <span className={styles.statOf}>/{rows.length}</span>
            </span>
            <span className={styles.statLabel}>providers live</span>
          </div>
          <div className={styles.stat}>
            <span className={styles.statValue}>{totals.totalRate.toFixed(0)}</span>
            <span className={styles.statLabel}>updates / sec</span>
          </div>
          <div className={styles.stat}>
            <span className={styles.statValue}>{totals.stale}</span>
            <span className={styles.statLabel}>quotes excluded</span>
          </div>
          <div
            className={[styles.stat, totals.unhealthy > 0 ? styles.statAlert : ""].join(" ")}
          >
            <span className={styles.statValue}>{totals.unhealthy}</span>
            <span className={styles.statLabel}>need attention</span>
          </div>
        </div>

        {rows.length === 0 && !isLoading ? (
          <div className={styles.empty}>
            <p className={styles.emptyTitle}>No inbound liquidity</p>
            <p className={styles.emptyHint}>
              No provider is a member of any enabled aggregated book. Add members
              to a book in Aggregation, and define the matching FIX connections in
              Connections — a book member is matched to a provider by connection
              id.
            </p>
          </div>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Status</th>
                {header("connectionId", "Provider", undefined, false)}
                <th className={styles.num}>Books</th>
                {header("rate", "Rate /s", "Quote updates per second, measured across polls")}
                {header("instrumentsQuoted", "Instruments")}
                {header("freshQuotes", "Fresh", "Quotes currently contributing to a composite")}
                <th
                  className={styles.num}
                  title="Quotes excluded from the composite as stale or divergent"
                >
                  Excluded
                </th>
                {header(
                  "topOfBook",
                  "Top of book",
                  "Instruments where this provider sets the best bid / offer",
                )}
                {header(
                  "meanWeight",
                  "Weight",
                  "Mean share of the composite mid this provider drives",
                )}
                {header("lastQuote", "Last quote")}
                <th className={styles.actionsCol} />
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const secs = ageSecs(r.lastQuoteNanos, nowMs);
                const state = providerState(r, secs);
                const isOpen = r.connectionId === selected;
                return (
                  <tr key={r.connectionId} className={isOpen ? styles.rowOpen : undefined}>
                    <td>
                      <span
                        className={[styles.pill, styles[`pill_${state.tone}`]].join(" ")}
                        role="img"
                        aria-label={state.label}
                      >
                        {state.label}
                      </span>
                    </td>
                    <td className={styles.nameCell}>
                      <span className={styles.providerName}>{r.name || r.connectionId}</span>
                      <span className={styles.providerId}>{r.connectionId}</span>
                    </td>
                    <td className={styles.num}>
                      {r.bookIds.length === 0 ? (
                        <span
                          className={styles.warnText}
                          title="Not a member of any enabled aggregated book — its pushes are dropped at ingest."
                        >
                          none
                        </span>
                      ) : (
                        <span title={r.bookIds.join(", ")}>{r.bookIds.length}</span>
                      )}
                    </td>
                    <td className={styles.num}>{formatRate(r.rate)}</td>
                    <td className={styles.num}>{r.instrumentsQuoted}</td>
                    <td className={styles.num}>{r.freshQuotes}</td>
                    <td
                      className={[styles.num, r.staleQuotes > 0 ? styles.warnText : ""].join(" ")}
                    >
                      {r.staleQuotes}
                    </td>
                    <td className={styles.num}>
                      {r.bestBidCount} / {r.bestOfferCount}
                    </td>
                    <td className={styles.num}>{(r.meanWeight * 100).toFixed(0)}%</td>
                    <td className={styles.num}>{formatAge(secs)}</td>
                    <td className={styles.actionsCol}>
                      <Button
                        variant={isOpen ? "primary" : "secondary"}
                        onClick={() =>
                          setSelected((id) => (id === r.connectionId ? null : r.connectionId))
                        }
                        aria-pressed={isOpen}
                      >
                        Quotes
                      </Button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Panel>

      {selectedRow && (
        <Panel
          title={`Live quotes — ${selectedRow.name || selectedRow.connectionId}`}
          glyph="⋮⋮"
          actions={
            <Button variant="ghost" onClick={() => setSelected(null)}>
              Close
            </Button>
          }
        >
          {panel.quotes.length === 0 ? (
            <p className={styles.emptyHint}>
              This provider has no live quote in any enabled book.
            </p>
          ) : (
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>Book</th>
                  <th>Instrument</th>
                  <th className={styles.num}>Bid</th>
                  <th className={styles.num}>Offer</th>
                  <th className={styles.num}>Bid size</th>
                  <th className={styles.num}>Offer size</th>
                  <th className={styles.num}>Age</th>
                  <th className={styles.num}>Weight</th>
                  <th>Verdict</th>
                </tr>
              </thead>
              <tbody>
                {panel.quotes.map((q) => (
                  <tr key={`${q.bookId}/${q.instrumentId}`}>
                    <td>{q.bookId}</td>
                    <td className={styles.nameCell}>
                      <span className={styles.providerName}>
                        {q.displayName || q.instrumentId}
                      </span>
                      {q.displayName && (
                        <span className={styles.providerId}>{q.instrumentId}</span>
                      )}
                    </td>
                    <td className={[styles.num, q.bestBid ? styles.best : ""].join(" ")}>
                      {formatPrice(q.bid)}
                    </td>
                    <td className={[styles.num, q.bestOffer ? styles.best : ""].join(" ")}>
                      {formatPrice(q.offer)}
                    </td>
                    <td className={styles.num}>{q.bidSize.toLocaleString()}</td>
                    <td className={styles.num}>{q.offerSize.toLocaleString()}</td>
                    <td className={styles.num}>{formatAge(q.ageSecs)}</td>
                    <td className={styles.num}>{(q.weight * 100).toFixed(0)}%</td>
                    <td>
                      {q.excluded ? (
                        <span className={[styles.pill, styles.pill_warn].join(" ")}>
                          {q.excluded.replace("_", " ")}
                        </span>
                      ) : (
                        <span className={[styles.pill, styles.pill_good].join(" ")}>
                          contributing
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      )}
    </div>
  );
}
