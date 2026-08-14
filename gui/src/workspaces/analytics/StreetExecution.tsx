/**
 * StreetExecution — the **order-level** half of the Street Liquidity workspace.
 *
 * The league table above it (`StreetLiquidityWorkspace`) grades each LP's
 * *behaviour*. It deliberately carries no order economics, so it cannot answer what
 * the desk actually asks about the street: **what went out, to whom, on what
 * product, and what came back**. These two panels answer that, off the same
 * `listStreetOrders` query:
 *
 *   • BREAKDOWN — one aggregated row per group on the chosen axis (LP, product
 *     family, instrument, tenor bucket, or hour): orders, fill ratio, win rate,
 *     mean slippage, last-look rate, mean cover, mean competition and traded
 *     notional. Folded server-side over the WHOLE matching window, not the page.
 *   • BLOTTER — one row per outbound order, newest first: the LP, the product, the
 *     side, requested-vs-filled, the price and slippage, the outcome and its reason,
 *     the competing panel, and the parent hedge it belongs to.
 *
 * ## Absence is rendered as absence
 *
 * Every optional metric renders {@link ABSENT} ("—") when the server sent `null`,
 * which it does whenever the datum was genuinely not observed: a composite backstop
 * has no LP, an unfilled order has no fill price or slippage, and order type /
 * time-in-force / response latency are absent while the routing seam lifts a
 * standing firm price in-process (no typed order is issued and there is no venue
 * round trip to time). None of those ever render as `0`.
 *
 * A **composite backstop** is shown as its own explicit bucket rather than being
 * folded into an LP or hidden: "we tried to hedge and the street showed us no firm
 * price" is a finding, not missing data.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  StreetBreakdownRow,
  StreetDimension,
  StreetOrder,
  StreetOrderFilter,
  StreetOutcome,
  StreetOrdersView,
} from "../../data/contract";
import styles from "./StreetLiquidityWorkspace.module.css";

/** The marker for a genuinely absent metric. Never substituted for a real zero. */
export const ABSENT = "—";

const compact = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});
const grouped = new Intl.NumberFormat("en-US");

/** An integer count. A count IS observed, so `0` here is a real zero. */
function fmtCount(n: number): string {
  return grouped.format(n);
}

/** A compact magnitude (quantities are in the order's native metric units). */
function fmtQty(n: number): string {
  return compact.format(n);
}

/** A ratio as a percent to 1dp; absent ⇒ "—". */
function fmtPct(n: number | undefined): string {
  return n === undefined ? ABSENT : `${(n * 100).toFixed(1)}%`;
}

/** Basis points to 2dp with an explicit sign; absent ⇒ "—". */
function fmtBp(n: number | undefined): string {
  if (n === undefined) return ABSENT;
  const sign = n > 0 ? "+" : "";
  return `${sign}${n.toFixed(2)}`;
}

/** A price to 6sf; absent ⇒ "—". */
function fmtPrice(n: number | undefined): string {
  return n === undefined ? ABSENT : n.toPrecision(6);
}

/** A nanosecond latency in the largest sensible unit; absent ⇒ "—". */
function fmtLatency(ns: number | undefined): string {
  if (ns === undefined) return ABSENT;
  if (ns < 1_000) return `${ns.toFixed(0)} ns`;
  if (ns < 1_000_000) return `${(ns / 1_000).toFixed(1)} µs`;
  return `${(ns / 1_000_000).toFixed(2)} ms`;
}

/** A tenor in years to 1dp; absent ⇒ "—". */
function fmtTenor(y: number | undefined): string {
  return y === undefined ? ABSENT : `${y.toFixed(1)}y`;
}

/** An epoch-nanos instant as a UTC wall-clock time. */
function fmtTime(tsNanos: bigint): string {
  const ms = Number(tsNanos / 1_000_000n);
  if (!Number.isFinite(ms) || ms === 0) return ABSENT;
  return new Date(ms).toISOString().replace("T", " ").slice(0, 19);
}

/** A mean count to 1dp; absent ⇒ "—". */
function fmtMean(n: number | undefined): string {
  return n === undefined ? ABSENT : n.toFixed(1);
}

/** Render an absent value with the muted/italic treatment that marks it as absent. */
function Absent(): React.ReactElement {
  return (
    <span className={styles.absent} title="Genuinely absent — not observed, not zero">
      {ABSENT}
    </span>
  );
}

/** Wrap a formatted value, applying the absent treatment when it IS the marker. */
function Val({ children }: { children: string }): React.ReactElement {
  return children === ABSENT ? <Absent /> : <>{children}</>;
}

/** The explicit composite-backstop group key the server emits. */
const COMPOSITE_KEY = "COMPOSITE";

/** Human labels for the grouping axes. */
const DIMENSIONS: readonly { value: StreetDimension; label: string }[] = [
  { value: "lp", label: "Liquidity provider" },
  { value: "family", label: "Product family" },
  { value: "instrument", label: "Instrument" },
  { value: "tenor_bucket", label: "Tenor bucket" },
  { value: "hour", label: "Hour (UTC)" },
];

/** Human labels for the outcome filter. */
const OUTCOMES: readonly { value: StreetOutcome | ""; label: string }[] = [
  { value: "", label: "Any outcome" },
  { value: "filled", label: "Filled" },
  { value: "partially_filled", label: "Partially filled" },
  { value: "rejected", label: "Rejected" },
  { value: "cancelled", label: "Cancelled" },
  { value: "expired", label: "Expired" },
  { value: "last_look_pulled", label: "Last-looked away" },
  { value: "no_liquidity", label: "No liquidity" },
];

/** The outcome chip label. */
function outcomeLabel(o: StreetOutcome): string {
  return OUTCOMES.find((x) => x.value === o)?.label ?? o;
}

/**
 * The reason/qualifier tooltip.
 *
 * Two vocabularies meet in this field and they mean different things, so the ones
 * that describe OUR configuration rather than a counterparty's answer are spelled
 * out: a member we never sent an order to did not refuse us, and reading it as a
 * refusal would blame a counterparty for our own missing setup.
 */
const ROUTING_REASONS: Readonly<Record<string, string>> = {
  no_order_endpoint:
    "No order route — this liquidity provider is quoting, but no order-acceptor address is configured for it, so no order could be sent. Nobody refused us; nobody was asked. Set the order route on the connection in Administration → FIX Connections.",
  no_order_router_configured:
    "No order router — this edge has no outbound order-routing seam wired, so no order could be sent to any provider.",
  session_unavailable:
    "Session unavailable — the configured order route could not be connected to, or the FIX logon did not complete.",
  member_inbox_saturated:
    "Provider saturated — this provider already has the maximum number of orders outstanding, so this one was not queued behind them.",
  venue_no_response:
    "No response — the order was sent and the venue did not answer inside the routing deadline. The measured wait is an upper bound on its latency.",
  street_declined:
    "The street declined — every provider showing a firm price was sent an order and none traded, so the shed backstopped to the composite. The individual refusals are the rows above.",
  no_firm_lp_price:
    "No firm street price — no provider was showing an executable price on the side we needed, so no order was sent to anyone.",
  composite_venue_configured:
    "Composite by configuration — this desk's execution mode is composite-only, so the street was never asked.",
};

function reasonTitle(reason: string | undefined): string | undefined {
  if (reason === undefined) return undefined;
  const routing = ROUTING_REASONS[reason];
  if (routing !== undefined) return routing;
  // Anything else is the VENUE's own Text(58) code, verbatim.
  return `Venue reason: ${reason}`;
}

/** The group key as displayed: the explicit buckets get spelled out. */
function keyLabel(row: StreetBreakdownRow): string {
  if (row.key === COMPOSITE_KEY) return "Composite backstop";
  if (row.key.length === 0) return "Unattributed";
  return row.key;
}

type SortDir = "asc" | "desc";

interface BreakdownColumn {
  key: string;
  label: string;
  title: string;
  sortValue: (r: StreetBreakdownRow) => number | undefined;
  render: (r: StreetBreakdownRow) => React.ReactNode;
}

const BREAKDOWN_COLUMNS: readonly BreakdownColumn[] = [
  {
    key: "orders",
    label: "Orders",
    title: "Outbound street orders in this group",
    sortValue: (r) => r.orders,
    render: (r) => fmtCount(r.orders),
  },
  {
    key: "filled",
    label: "Filled",
    title: "Orders that fully filled",
    sortValue: (r) => r.filled,
    render: (r) => fmtCount(r.filled),
  },
  {
    key: "partiallyFilled",
    label: "Partial",
    title: "Orders that partially filled (an IOC-style partial)",
    sortValue: (r) => r.partiallyFilled,
    render: (r) => fmtCount(r.partiallyFilled),
  },
  {
    key: "rejected",
    label: "Rejected",
    title:
      "Orders the venue REFUSED (FIX OrdStatus=8) — it declined to trade. Distinct from Cancelled: this is “would not”",
    sortValue: (r) => r.rejected,
    render: (r) => fmtCount(r.rejected),
  },
  {
    key: "cancelled",
    label: "Cancelled",
    title:
      "Orders the venue ACCEPTED but could not satisfy (FIX OrdStatus=4) — e.g. an all-or-nothing clip larger than the available size. This is “could not”, not “would not”",
    sortValue: (r) => r.cancelled,
    render: (r) => fmtCount(r.cancelled),
  },
  {
    key: "lastLookPulled",
    label: "Last-look",
    title: "Orders pulled at last look after we tried to deal",
    sortValue: (r) => r.lastLookPulled,
    render: (r) => fmtCount(r.lastLookPulled),
  },
  {
    key: "compositeBackstop",
    label: "Backstopped",
    title:
      "Orders that reached no named LP and backstopped to the internal composite mid — the street showed no firm price",
    sortValue: (r) => r.compositeBackstop,
    render: (r) => fmtCount(r.compositeBackstop),
  },
  {
    key: "winRate",
    label: "Win rate",
    title: "Orders that traded ÷ orders. Absent when the group is empty",
    sortValue: (r) => r.winRate,
    render: (r) => <Val>{fmtPct(r.winRate)}</Val>,
  },
  {
    key: "fillRatio",
    label: "Fill ratio",
    title: "Filled ÷ requested quantity. Absent when nothing was requested",
    sortValue: (r) => r.fillRatio,
    render: (r) => <Val>{fmtPct(r.fillRatio)}</Val>,
  },
  {
    key: "filledQty",
    label: "Traded",
    title: "Total quantity filled across the group (native metric units)",
    sortValue: (r) => r.filledQty,
    render: (r) => fmtQty(r.filledQty),
  },
  {
    key: "meanSlippageBp",
    label: "Slippage bp",
    title:
      "Mean signed slippage over the orders that actually filled. Absent when none did — never a slippage of zero for a group that never traded",
    sortValue: (r) => r.meanSlippageBp,
    render: (r) => <Val>{fmtBp(r.meanSlippageBp)}</Val>,
  },
  {
    key: "lastLookRate",
    label: "Pull rate",
    title: "Last-look pulls ÷ orders. Absent when the group is empty",
    sortValue: (r) => r.lastLookRate,
    render: (r) => <Val>{fmtPct(r.lastLookRate)}</Val>,
  },
  {
    key: "meanCover",
    label: "Mean cover",
    title:
      "Mean distance from the price we took to the runner-up. Absent when no order in the group had a cover",
    sortValue: (r) => r.meanCover,
    render: (r) => <Val>{fmtPrice(r.meanCover)}</Val>,
  },
  {
    key: "meanCompetitors",
    label: "Competition",
    title:
      "Mean number of LPs showing a firm price when these orders were worked — how contested our flow was",
    sortValue: (r) => r.meanCompetitors,
    render: (r) => <Val>{fmtMean(r.meanCompetitors)}</Val>,
  },
  {
    key: "meanResponseLatencyNanos",
    label: "Response",
    title:
      "Mean measured venue round-trip latency. Absent when no order carried a measured latency (an in-process panel lift has no round trip)",
    sortValue: (r) => r.meanResponseLatencyNanos,
    render: (r) => <Val>{fmtLatency(r.meanResponseLatencyNanos)}</Val>,
  },
];

/** Compare two sort values; `undefined` always sorts LAST regardless of direction. */
function compareSort(a: number | undefined, b: number | undefined, dir: SortDir): number {
  if (a === undefined && b === undefined) return 0;
  if (a === undefined) return 1;
  if (b === undefined) return -1;
  return dir === "asc" ? a - b : b - a;
}

const EMPTY_VIEW: StreetOrdersView = { orders: [], breakdown: [], totalMatching: 0 };

/**
 * The street-side execution panels. Rendered beneath the LP league table inside the
 * same workspace, so the desk reads "who we trade with" and "what we actually sent
 * them" as one surface rather than two competing screens.
 */
export function StreetExecution(): React.ReactElement {
  const app = useApp();
  const signedIn = app.auth.user !== undefined && app.auth.user !== null;

  const [dimension, setDimension] = useState<StreetDimension>("lp");
  const [lpFilter, setLpFilter] = useState("");
  const [familyFilter, setFamilyFilter] = useState("");
  const [outcomeFilter, setOutcomeFilter] = useState<StreetOutcome | "">("");
  const [view, setView] = useState<StreetOrdersView>(EMPTY_VIEW);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sortKey, setSortKey] = useState("orders");
  const [sortDir, setSortDir] = useState<SortDir>("desc");

  const load = useCallback((): void => {
    if (!signedIn) {
      setView(EMPTY_VIEW);
      setLoadError(null);
      return;
    }
    const filter: StreetOrderFilter = {
      dimension,
      // Empty controls are omitted by the codec — a blank field does not constrain.
      lpId: lpFilter,
      family: familyFilter,
      outcome: outcomeFilter === "" ? undefined : outcomeFilter,
    };
    setLoading(true);
    void app.transport
      .listStreetOrders(undefined, filter)
      .then((v) => {
        setView(v);
        setLoadError(null);
      })
      .catch((e: unknown) => {
        setView(EMPTY_VIEW);
        setLoadError(
          e instanceof Error ? e.message : "failed to load street-side execution records",
        );
      })
      .finally(() => setLoading(false));
  }, [app.transport, signedIn, dimension, lpFilter, familyFilter, outcomeFilter]);

  useEffect(() => {
    load();
  }, [load]);

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

  const sortedBreakdown = useMemo(() => {
    const col = BREAKDOWN_COLUMNS.find((c) => c.key === sortKey);
    const rows = [...view.breakdown];
    rows.sort((a, b) => {
      if (sortKey === "key") {
        const cmp = a.key.localeCompare(b.key);
        return sortDir === "asc" ? cmp : -cmp;
      }
      if (!col) return 0;
      return compareSort(col.sortValue(a), col.sortValue(b), sortDir);
    });
    return rows;
  }, [view.breakdown, sortKey, sortDir]);

  const ariaSort = (key: string): "ascending" | "descending" | "none" =>
    sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";

  const dimensionLabel =
    DIMENSIONS.find((d) => d.value === dimension)?.label ?? dimension;

  if (!signedIn) return <></>;

  return (
    <>
      <section className={styles.section} aria-labelledby="street-exec-title">
        <h2 id="street-exec-title" className={styles.sectionTitle}>
          Street-side execution
        </h2>
        <p className={styles.explainer}>
          What we actually <strong>sent</strong> to the street, and what came back. A
          <strong> composite backstop</strong> is its own bucket — it means the street
          showed us no firm price at all, so no LP is credited or debited for it. Every
          metric is <strong>{ABSENT}</strong> when genuinely absent (a group that never
          traded has no slippage; an in-process panel lift has no measured round trip),
          never a fabricated zero.
        </p>

        <div className={styles.controls}>
          <label className={styles.control}>
            Group by
            <select
              value={dimension}
              onChange={(e) => setDimension(e.target.value as StreetDimension)}
            >
              {DIMENSIONS.map((d) => (
                <option key={d.value} value={d.value}>
                  {d.label}
                </option>
              ))}
            </select>
          </label>
          <label className={styles.control}>
            LP
            <input
              type="search"
              value={lpFilter}
              placeholder="any"
              onChange={(e) => setLpFilter(e.target.value)}
            />
          </label>
          <label className={styles.control}>
            Product family
            <input
              type="search"
              value={familyFilter}
              placeholder="any (ois, bond_future, …)"
              onChange={(e) => setFamilyFilter(e.target.value)}
            />
          </label>
          <label className={styles.control}>
            Outcome
            <select
              value={outcomeFilter}
              onChange={(e) => setOutcomeFilter(e.target.value as StreetOutcome | "")}
            >
              {OUTCOMES.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        {loadError !== null && (
          <p className={styles.error} role="alert">
            {loadError}
          </p>
        )}

        {loading && view.breakdown.length === 0 ? (
          <p className={styles.empty}>Loading street-side execution records…</p>
        ) : sortedBreakdown.length === 0 ? (
          <p className={styles.empty}>
            No street orders recorded in this window. Nothing has been sent to the street
            (or nothing matches these filters) — this is an honest empty state, not a
            failed load.
          </p>
        ) : (
          <div className={styles.tableScroll}>
            <table className={styles.table}>
              <caption className={styles.caption}>
                Street-side execution breakdown — one row per {dimensionLabel.toLowerCase()}
              </caption>
              <thead>
                <tr>
                  <th scope="col" className={styles.thLabel} aria-sort={ariaSort("key")}>
                    <button
                      type="button"
                      className={styles.sortBtn}
                      onClick={() => onSort("key")}
                    >
                      {dimensionLabel}
                      <SortGlyph active={sortKey === "key"} dir={sortDir} />
                    </button>
                  </th>
                  {BREAKDOWN_COLUMNS.map((c) => (
                    <th
                      key={c.key}
                      scope="col"
                      className={styles.thNum}
                      aria-sort={ariaSort(c.key)}
                      title={c.title}
                    >
                      <button
                        type="button"
                        className={styles.sortBtnNum}
                        onClick={() => onSort(c.key)}
                      >
                        <SortGlyph active={sortKey === c.key} dir={sortDir} />
                        {c.label}
                      </button>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {sortedBreakdown.map((r) => (
                  <tr key={`${r.dimension}:${r.key}`}>
                    <th
                      scope="row"
                      className={`${styles.rowLabel} ${r.key === COMPOSITE_KEY ? styles.backstop : ""}`}
                      title={
                        r.key === COMPOSITE_KEY
                          ? "These orders reached no named LP — the street showed no firm price and they backstopped to the internal composite mid"
                          : r.key.length === 0
                            ? "These orders carried no value on this axis — shown explicitly rather than merged into a real group"
                            : undefined
                      }
                    >
                      {keyLabel(r)}
                    </th>
                    {BREAKDOWN_COLUMNS.map((c) => (
                      <td key={c.key} className={styles.num}>
                        {c.render(r)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className={styles.section} aria-labelledby="street-blotter-title">
        <h2 id="street-blotter-title" className={styles.sectionTitle}>
          Street order blotter
        </h2>
        <p className={styles.count}>
          Showing {fmtCount(view.orders.length)} of {fmtCount(view.totalMatching)} matching
          orders, newest first.
        </p>
        {view.orders.length === 0 ? (
          <p className={styles.empty}>No street orders match these filters.</p>
        ) : (
          <div className={styles.tableScroll}>
            <table className={styles.table}>
              <caption className={styles.caption}>
                Outbound street orders — one row per order sent
              </caption>
              <thead>
                <tr>
                  <th scope="col" className={styles.thLabel}>
                    Time (UTC)
                  </th>
                  <th scope="col" className={styles.thLabel}>
                    LP
                  </th>
                  <th scope="col" className={styles.thLabel}>
                    Instrument
                  </th>
                  <th scope="col" className={styles.thLabel}>
                    Family
                  </th>
                  <th scope="col" className={styles.thNum} title="Risk tenor of the hedged exposure">
                    Tenor
                  </th>
                  <th scope="col" className={styles.thLabel}>
                    Side
                  </th>
                  <th scope="col" className={styles.thNum}>
                    Requested
                  </th>
                  <th scope="col" className={styles.thNum}>
                    Filled
                  </th>
                  <th scope="col" className={styles.thNum} title="The reference mid at fire">
                    Ref px
                  </th>
                  <th scope="col" className={styles.thNum}>
                    Fill px
                  </th>
                  <th scope="col" className={styles.thNum}>
                    Slip bp
                  </th>
                  <th scope="col" className={styles.thLabel}>
                    Outcome
                  </th>
                  <th
                    scope="col"
                    className={styles.thNum}
                    title="How many LPs showed a firm price on the side we needed"
                  >
                    Panel
                  </th>
                  <th
                    scope="col"
                    className={styles.thLabel}
                    title="Order type / time-in-force actually sent — absent while the routing seam lifts a standing firm price in-process"
                  >
                    Type / TIF
                  </th>
                  <th
                    scope="col"
                    className={styles.thLabel}
                    title="The hedge decision this order belongs to — walk from a breach to the street orders it produced"
                  >
                    Parent hedge
                  </th>
                </tr>
              </thead>
              <tbody>
                {view.orders.map((o) => (
                  <OrderRow key={o.orderId} order={o} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </>
  );
}

/** One blotter row. Split out so the (wide) row markup stays readable. */
function OrderRow({ order: o }: { order: StreetOrder }): React.ReactElement {
  const backstop = o.venue === "composite_backstop";
  return (
    <tr>
      <th scope="row" className={styles.rowLabel}>
        {fmtTime(o.tsNanos)}
      </th>
      <td className={`${styles.rowLabel} ${backstop ? styles.backstop : ""}`}>
        {o.lpId !== undefined ? (
          o.lpId
        ) : backstop ? (
          <span title="No named LP — the street showed no firm price, so this backstopped to the internal composite mid">
            Composite backstop
          </span>
        ) : (
          <Absent />
        )}
      </td>
      <td className={styles.rowLabel}>{o.instrument}</td>
      <td className={styles.rowLabel}>
        {o.family.length > 0 ? o.family : <Absent />}
      </td>
      <td className={styles.num}>
        <Val>{fmtTenor(o.tenorYears)}</Val>
      </td>
      <td className={styles.rowLabel}>{o.side === "buy" ? "Buy" : "Sell"}</td>
      <td className={styles.num}>{fmtQty(o.requestedQty)}</td>
      <td className={styles.num}>{fmtQty(o.filledQty)}</td>
      <td className={styles.num}>{fmtPrice(o.requestedPrice)}</td>
      <td className={styles.num}>
        <Val>{fmtPrice(o.filledPrice)}</Val>
      </td>
      <td className={styles.num}>
        <Val>{fmtBp(o.slippageBp)}</Val>
      </td>
      <td className={styles.rowLabel}>
        <span
          className={`${styles.chip} ${styles[`chip-${o.outcome}`] ?? ""}`}
          title={reasonTitle(o.reason)}
        >
          {outcomeLabel(o.outcome)}
        </span>
      </td>
      <td
        className={styles.num}
        title={
          o.competitors.length > 0
            ? o.competitors.map((c) => `${c.lpId} @ ${c.price.toPrecision(6)}`).join("\n")
            : "No LP showed a firm price on the side we needed"
        }
      >
        {fmtCount(o.competitors.length)}
      </td>
      <td
        className={styles.rowLabel}
        title="FIX OrdType(40) / TimeInForce(59) as actually sent"
      >
        {o.orderType !== undefined || o.timeInForce !== undefined ? (
          `${o.orderType ?? ABSENT} / ${o.timeInForce ?? ABSENT}`
        ) : (
          <Absent />
        )}
      </td>
      <td className={styles.rowLabel}>
        {o.parentHedgeId !== undefined ? o.parentHedgeId : <Absent />}
      </td>
    </tr>
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
