/**
 * DealsBlotterWorkspace — the Deals LENS of the unified, one-per-book
 * `BookWorkspace` (`fe-fi-migration` #4); the `deals` rail row opens the Book on
 * this lens. Behaviour is unchanged — it is still the received-deals blotter
 * (fixed-income). Every deal booked by an accepted desk quote, newest first:
 * counterparty, instrument, notional, dealt price, side, booking trader and
 * execution time.
 *
 * One contract, two transports (GUI-DESIGN §6.2): the blotter talks ONLY to the
 * `CelnetTransport.listDeals` seam, so the SAME deals render through the
 * deterministic in-app source and the live `RfqDeskService.ListDeals` edge. It
 * refreshes on every push `Notification` (a `QUOTE_ACCEPTED` mints a deal), so a
 * fill appears live without polling churn.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { useAcceptanceSeed } from "../app/AcceptanceSeedContext";
import { useHedgeSeed } from "../app/HedgeSeedContext";
import { hedgeSeedFromDeal } from "../lib/hedgeSeed";
import { Panel } from "../components/Panel";
import { FlowRowContextMenu, type FlowRowMenuTarget } from "../components/FlowRowContextMenu";
import { TableSearch } from "../components/TableSearch";
import { useTableFilter } from "../hooks/useTableFilter";
import { principalForScope } from "../data/riskView";
import { fmtRate, fmtClock, fmtCompact } from "../lib/format";
import type { Deal, Internalise, Side } from "../data/contract";
import { capabilityAssetForDomain, dealAsset } from "../data/assetClass";
import { fmtEdgeBps, hedgeBandLabel, internaliseLabel } from "../lib/internalise";
import { DealTicket } from "./DealTicket";
import { HedgeDealsView } from "./HedgeDealsView";
import styles from "./DealsBlotterWorkspace.module.css";

/** Which lens of the Deals blotter is shown: the client fills, or the executed hedges. */
type DealsLens = "client" | "hedge";

/**
 * The rates pay/receive-fixed reading of a `Side` — the SECONDARY detail kept
 * alongside the primary BUY/SELL badge. The wire `Side` already IS the buy/sell
 * axis (contract: `SIDE_BUY` pays fixed / is long the swap, `SIDE_SELL` receives
 * fixed), so this only spells out the rates convention: BUY → "Pay fixed",
 * SELL → "Receive fixed".
 */
function sideLabel(side: Side): string {
  if (side === "BUY") return "Pay fixed";
  if (side === "SELL") return "Receive fixed";
  return "Two-way";
}

/** The primary BUY / SELL / 2-WAY read of a deal's `Side` (native to the wire). */
function sideBuySell(side: Side): string {
  if (side === "BUY") return "BUY";
  if (side === "SELL") return "SELL";
  return "2-WAY";
}

/**
 * The BUY / SELL indicator per deal row — the PRIMARY read of `Side`, tinted green
 * (bid) for a buy and red (offer) for a sell via the shared trading semantic
 * tokens, with the rates pay/receive-fixed detail kept as secondary text (and the
 * accessible label). Rates deals map long-the-swap = BUY (pay fixed); FX / other
 * deals use their native `Side` directly. A two-way quote stays neutral.
 */
function SideBadge({ side }: { readonly side: Side }): React.ReactElement {
  const tag = sideBuySell(side);
  const detail = sideLabel(side);
  const label = `${tag} · ${detail}`;
  return (
    <span className={styles.side} data-side={side} aria-label={label} title={label}>
      <span className={styles.sideTag}>{tag}</span>
      <span className={styles.sideDetail}>{detail}</span>
    </span>
  );
}

/**
 * The product family a booked deal carries — its decoded `RatesInstrument` oneof arm
 * ({@link Deal.productKind}): `OIS` / `IRS` / `FRA` / `BOND`. Surfaced honestly off the
 * discriminant the codec threads through, so a mixed book classifies each fill by its
 * real family rather than assuming OIS.
 */
function productLabel(d: Deal): string {
  return d.productKind;
}

/**
 * The CSS chip class for a deal's request kind: RFQ (accent), IOI (warn), or ESP —
 * an executable streaming-price lift, tinted with the bid/streaming token so it reads
 * distinctly from the request-driven flows.
 */
function kindClass(kind: Deal["kind"]): string {
  if (kind === "IOI") return styles.kindIoi ?? "";
  if (kind === "ESP") return styles.kindEsp ?? "";
  return styles.kindRfq ?? "";
}

/**
 * The security descriptor a BOND deal renders in the SECURITY cell — the human
 * `display_name` when the bond is in the curated refdata (matching the Agg Book tile),
 * else the raw `instrument_id`, else empty. OIS/IRS/FRA carry no security identity.
 */
function securityDescriptor(d: Deal): string {
  return d.bondDisplayName ?? d.bondSecurityId ?? "";
}

/**
 * The SECURITY cell — for a BOND fill, the human descriptor over the stable security
 * id (matching the Agg Book tile); the descriptor falls back to the raw id when the
 * bond is outside the curated refdata (never blank/fabricated). OIS/IRS/FRA rows have
 * no security identity, so they render a plain em dash.
 */
function SecurityCell({ deal }: { readonly deal: Deal }): React.ReactElement {
  const descriptor = securityDescriptor(deal);
  if (descriptor.length === 0) {
    return <span className={styles.securityNone}>—</span>;
  }
  const id = deal.bondSecurityId ?? "";
  // Show the id sub-line only when it exists AND is not already the descriptor (a
  // refdata-less bond shows the id as the descriptor, so avoid repeating it).
  const showId = id.length > 0 && id !== descriptor;
  const label = showId ? `${descriptor} · ${id}` : descriptor;
  return (
    <span className={styles.security} aria-label={label} title={label}>
      <span className={styles.securityName}>{descriptor}</span>
      {showId && <span className={styles.securityId}>{id}</span>}
    </span>
  );
}

/** The tenor a rates deal carries, as a compact `Ny` label (e.g. `10y`). */
function tenorLabel(d: Deal): string {
  return `${d.instrument.tenorYears}y`;
}

/** The booked position id the fill landed in the ledger Book, or `—` when none. */
function positionLabel(d: Deal): string {
  return d.positionId !== undefined ? `#${d.positionId.toString()}` : "—";
}

/**
 * The Risk Portfolio (risk book) the fill's risk routed into, resolved id → the
 * portfolio's human NAME via the `listRiskBooks` roster. Falls back to the raw id
 * when the roster does not carry it (a routed-but-since-renamed book), and `—` when
 * the fill routed to no portfolio (unrouted — no graph / routing fall-back).
 */
function riskPortfolioLabel(d: Deal, names: ReadonlyMap<string, string>): string {
  if (d.riskBookId === undefined) return "—";
  return names.get(d.riskBookId) ?? d.riskBookId;
}

/**
 * A compact inline badge surfacing an FI lift's internalise decision: whether the
 * fill was warehoused internally ("Internalised") or shed external ("B2B"), tinted
 * by the DV01-utilisation `hedgeBand` (green→bid, amber→warn, red→offer,
 * breach→danger), with a "losing" dot when the captured edge is off-tolerance.
 * Rendered ONLY for deals that carry `internalise` (FI lifts).
 */
function InternaliseBadge({ inl }: { readonly inl: Internalise }): React.ReactElement {
  const toleranceNote = inl.withinTolerance ? "" : " · below tolerance (losing)";
  const label = `${internaliseLabel(inl)} · edge ${fmtEdgeBps(inl.edgeBps)} · ${hedgeBandLabel(inl.hedgeBand)} band${toleranceNote}`;
  return (
    <span
      className={styles.inl}
      data-band={inl.hedgeBand}
      data-losing={inl.withinTolerance ? undefined : "true"}
      aria-label={label}
      title={label}
    >
      {internaliseLabel(inl)}
      {!inl.withinTolerance && <span className={styles.inlLosing} aria-hidden="true" />}
    </span>
  );
}

/**
 * The Hedge-column legend — a concise key for the internalise badge, since traders
 * ask what the tints mean. Documents the REAL badge semantics read off the code:
 *   • the LABEL is the routing decision — "Internalised" (warehoused from risk) vs
 *     "B2B" (shed external back-to-back) — independent of colour;
 *   • the COLOUR is the DV01 warehouse-utilisation band (NOT the edge): green =
 *     comfortable, amber = approaching the cap, red = at/over the cap, breach = the
 *     hard limit exceeded — the same bid/warn/offer/danger tokens as the badge;
 *   • the DOT (+ ring) marks a LOSING fill — captured edge below the min-edge
 *     tolerance (`withinTolerance === false`), shown regardless of band.
 * The band swatches reuse the badge classes (`.inl` + `data-band`) so the legend
 * colours are byte-identical to the rows and theme automatically.
 */
function InternaliseLegend(): React.ReactElement {
  const bands: readonly { band: Internalise["hedgeBand"]; note: string }[] = [
    { band: "green", note: "comfortable" },
    { band: "amber", note: "approaching cap" },
    { band: "red", note: "at / over cap" },
    { band: "breach", note: "limit breached" },
  ];
  return (
    <div className={styles.legend} role="note" aria-label="Hedge badge legend">
      <span className={styles.legendTitle}>Hedge</span>
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="green" aria-hidden="true">
          Internalised
        </span>
        warehoused
      </span>
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="green" aria-hidden="true">
          B2B
        </span>
        external back-to-back
      </span>
      <span className={styles.legendSep} aria-hidden="true" />
      <span className={styles.legendGroupLabel}>DV01 band:</span>
      {bands.map(({ band, note }) => (
        <span key={band} className={styles.legendItem}>
          <span className={styles.legendSwatch} data-band={band} aria-hidden="true" />
          {hedgeBandLabel(band)} — {note}
        </span>
      ))}
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="red" data-losing="true" aria-hidden="true">
          <span className={styles.inlLosing} />
        </span>
        dot = losing (edge below tolerance)
      </span>
    </div>
  );
}

/** The searchable text of a deal's internalise decision (empty when it has none). */
function internaliseSearchText(d: Deal): string {
  if (d.internalise === undefined) return "";
  return [
    internaliseLabel(d.internalise),
    hedgeBandLabel(d.internalise.hedgeBand),
    fmtEdgeBps(d.internalise.edgeBps),
    d.internalise.withinTolerance ? "within tolerance" : "below tolerance losing",
  ].join(" ");
}

/** All of a deal's user-visible textual fields, concatenated for substring search. */
function dealSearchText(d: Deal, names: ReadonlyMap<string, string>): string {
  return [
    fmtClock(d.executedAtNanos),
    d.counterparty,
    d.desk,
    d.kind,
    productLabel(d),
    securityDescriptor(d),
    d.bondSecurityId ?? "",
    tenorLabel(d),
    d.curveSet.currency,
    fmtCompact(d.notional),
    fmtRate(d.price),
    sideBuySell(d.side),
    sideLabel(d.side),
    d.trader,
    positionLabel(d),
    riskPortfolioLabel(d, names),
    internaliseSearchText(d),
    d.dealId,
  ].join(" ");
}

export function DealsBlotterWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // Hard asset separation: the Book's Deals lens shows ONLY the active domain's
  // asset class. Booked deals are structurally OIS (rates), so under the FX Options
  // domain this lens is correctly empty and under Fixed Income it shows them.
  const activeAsset = capabilityAssetForDomain(app.activeDomain);

  const seed = useAcceptanceSeed();
  const canManageAcceptance = app.auth.can("manage_acceptance", "fixed_income");
  const hedge = useHedgeSeed();
  // Authoring a hedge policy gates on `hedge` × FI (same as the Hedging surface); the
  // "Change hedging strategy" row action is hidden entirely without it.
  const canHedge = app.auth.can("hedge", "fixed_income");

  // Client fills vs executed hedges — the two separated lenses of the blotter.
  const [lens, setLens] = useState<DealsLens>("client");
  const [allDeals, setAllDeals] = useState<Deal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Deal | null>(null);
  // The row context menu (right-click / context-menu key): the counterparty + anchor
  // point of the row whose "Create acceptance rule" the trader is spawning.
  const [rowMenu, setRowMenu] = useState<FlowRowMenuTarget | null>(null);
  // The Risk Portfolio roster (id → human name) resolves each deal's routed
  // `riskBookId` to its portfolio name. Best-effort: an unavailable roster (e.g. no
  // routing configured) leaves the column falling back to the raw id / `—`.
  const [riskBookNames, setRiskBookNames] = useState<ReadonlyMap<string, string>>(
    () => new Map(),
  );

  useEffect(() => {
    let live = true;
    void app.transport
      .listRiskBooks()
      .then((books) => {
        if (live) setRiskBookNames(new Map(books.map((b) => [b.id, b.name])));
      })
      .catch(() => {
        /* no roster ⇒ the column falls back to the raw id / `—` (non-fatal). */
      });
    return () => {
      live = false;
    };
  }, [app.transport]);

  const refresh = useCallback(() => {
    void app.transport
      .listDeals({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setAllDeals(res.deals);
        setError(null);
      })
      .catch((err) => setError(err instanceof Error ? err.message : "failed to load deals"));
  }, [app.transport, principal]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    refreshRef.current();
    const dispose = app.transport.streamNotifications(undefined, () => refreshRef.current());
    return dispose;
  }, [app.transport]);

  // Asset-scope to the active domain BEFORE the search filter composes on top.
  const deals = useMemo(
    () => allDeals.filter((d) => dealAsset(d) === activeAsset),
    [allDeals, activeAsset],
  );

  const searchText = useCallback(
    (d: Deal) => dealSearchText(d, riskBookNames),
    [riskBookNames],
  );
  const { query, setQuery, filtered, shown, total } = useTableFilter(deals, searchText);

  const isOffline = !app.transport.label.startsWith("live");
  const totalNotional = deals.reduce((acc, d) => acc + d.notional, 0);

  return (
    <div className={styles.shell}>
      <div className={styles.lensBar} role="group" aria-label="deals lens">
        <button
          type="button"
          className={`${styles.lensBtn} ${lens === "client" ? styles.lensBtnActive : ""}`}
          aria-pressed={lens === "client"}
          data-testid="deals-lens-client"
          onClick={() => setLens("client")}
        >
          Client deals
        </button>
        <button
          type="button"
          className={`${styles.lensBtn} ${lens === "hedge" ? styles.lensBtnActive : ""}`}
          aria-pressed={lens === "hedge"}
          data-testid="deals-lens-hedge"
          onClick={() => setLens("hedge")}
        >
          Hedge deals
        </button>
      </div>
      {lens === "hedge" ? (
        <HedgeDealsView />
      ) : (
        <div className={styles.wrap}>
          <Panel className={styles.panel} title="Received deals">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app desk" : "live desk"}</span>
          <span className={styles.summary}>
            {deals.length} deal{deals.length === 1 ? "" : "s"} ·{" "}
            {fmtCompact(totalNotional)} notional
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {deals.length === 0 ? (
          <p className={styles.empty}>
            {activeAsset === "fixed_income"
              ? "No deals yet — accept a quote in the Quoting workspace to book one."
              : "No FX-option deals — the RFQ desk books rates (OIS). Switch to the Fixed Income domain to see booked deals."}
          </p>
        ) : (
          <>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={shown}
              total={total}
              label="Search deals"
              placeholder="Filter deals…"
            />
            {filtered.length === 0 ? (
              <p className={styles.empty}>No deals match “{query}”.</p>
            ) : (
              <>
              <InternaliseLegend />
              <div className={styles.tableWrap} tabIndex={0} role="region" aria-label="Deals table">
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th>Time</th>
                      <th>Counterparty</th>
                      <th>Desk</th>
                      <th>Type</th>
                      <th>Product</th>
                      <th>Security</th>
                      <th className={styles.num}>Tenor</th>
                      <th>Ccy</th>
                      <th className={styles.num}>Notional</th>
                      <th className={styles.num}>Rate</th>
                      <th>Side</th>
                      <th>Trader</th>
                      <th>Position</th>
                      <th>Risk Portfolio</th>
                      <th>Hedge</th>
                      <th>Deal</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((d) => (
                  <tr
                    key={d.dealId}
                    className={`${styles.row} ${selected?.dealId === d.dealId ? styles.rowSelected : ""}`}
                    onClick={() => setSelected(d)}
                    tabIndex={0}
                    role="button"
                    aria-label={`Open deal ${d.dealId}. Right-click or press the menu key for row actions`}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setRowMenu({
                        counterparty: d.counterparty,
                        x: e.clientX,
                        y: e.clientY,
                        hedgeSeed: hedgeSeedFromDeal(d),
                      });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        setSelected(d);
                        return;
                      }
                      // The standard context-menu key (or Shift+F10) opens the row menu —
                      // keyboard parity for the right-click, without a nested-interactive
                      // kebab inside this role="button" row (axe-clean).
                      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
                        e.preventDefault();
                        const r = e.currentTarget.getBoundingClientRect();
                        setRowMenu({
                          counterparty: d.counterparty,
                          x: r.left + 12,
                          y: r.bottom - 8,
                          hedgeSeed: hedgeSeedFromDeal(d),
                        });
                      }
                    }}
                  >
                    <td className={styles.mono}>{fmtClock(d.executedAtNanos)}</td>
                    <td className={styles.strong}>{d.counterparty}</td>
                    <td>{d.desk}</td>
                    <td>
                      <span className={`${styles.kind} ${kindClass(d.kind)}`}>
                        {d.kind}
                      </span>
                    </td>
                    <td>
                      <span className={styles.product}>{productLabel(d)}</span>
                    </td>
                    <td>
                      <SecurityCell deal={d} />
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>{tenorLabel(d)}</td>
                    <td className={styles.mono}>{d.curveSet.currency}</td>
                    <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(d.notional)}</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.price}`}>{fmtRate(d.price)}</td>
                    <td>
                      <SideBadge side={d.side} />
                    </td>
                    <td>{d.trader}</td>
                    <td className={styles.mono}>{positionLabel(d)}</td>
                    <td>{riskPortfolioLabel(d, riskBookNames)}</td>
                    <td>
                      {d.internalise ? (
                        <InternaliseBadge inl={d.internalise} />
                      ) : (
                        <span className={styles.inlNone}>—</span>
                      )}
                    </td>
                    <td className={styles.dealId}>{d.dealId}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              </>
            )}
          </>
        )}
      </Panel>
          <FlowRowContextMenu
            target={rowMenu}
            onClose={() => setRowMenu(null)}
            onCreateAcceptanceRule={(cp) => {
              // Seed the rule, then navigate to the Acceptance surface (the `acceptance`
              // alias → the consolidated Risk host's Acceptance tab), which consumes the seed.
              seed.requestAcceptanceSeed(cp);
              app.setWorkspace("acceptance");
            }}
            canManageAcceptance={canManageAcceptance}
            onChangeHedgingStrategy={(s) => {
              // Seed a hedge rule scoped to this deal's flow, then deep-link to the Hedging
              // workspace — its Exit Policy tab (the default) consumes the seed and opens a
              // pre-filled draft rule for the trader to tailor + save.
              hedge.requestHedgeSeed(s);
              app.setWorkspace("hedging");
            }}
            canHedge={canHedge}
          />
          {selected && <DealTicket deal={selected} onClose={() => setSelected(null)} />}
        </div>
      )}
    </div>
  );
}
