/**
 * The ONE trade-detail view model, and the adapters that build it.
 *
 * A desk asks the same question of a client fill and of a hedge — *what exactly was
 * this trade?* — so both must answer it on the same surface, in the same shape, with the
 * same affordance. Before this, the client blotter's row action deep-linked AWAY to the
 * Event Trace workspace and the hedge ledger offered nothing at all, so "view the trade"
 * meant two different things depending on which table you were looking at, and on the
 * hedge desk it meant nothing.
 *
 * The fork is avoided the way the rest of the platform avoids one: normalise at the seam
 * rather than branching at the renderer. {@link TradeDetails} is the single shape;
 * {@link clientTradeDetails} and {@link hedgeTradeDetails} are the only places that know
 * which record they came from. A third trade kind is a third adapter, never a third
 * modal.
 *
 * Values are pre-formatted STRINGS. The renderer stays a dumb list, which is what keeps
 * the two kinds visually identical — a component that formatted numbers itself would
 * drift the moment one caller wanted a different precision.
 */

import type { Deal, HedgeProvenance, Side, StreetOrder } from "../data/contract";

/**
 * The trader-facing name of a side.
 *
 * Spelled out here rather than looked up: a rates desk reads BUY as "pay fixed", and a
 * bare "BUY" on a settlement view is the kind of ambiguity that gets a trade booked the
 * wrong way round.
 */
function sideLabel(side: Side): string {
  if (side === "BUY") return "Buy · pay fixed";
  if (side === "SELL") return "Sell · receive fixed";
  return "Two-way";
}

/** One labelled fact. */
export interface DetailRow {
  readonly label: string;
  readonly value: string;
  /** Render in the mono face — ids, symbols, prices. */
  readonly mono?: boolean;
  /** Long-form explanation for a term that is genuinely ambiguous on its own. */
  readonly hint?: string;
}

/** A titled group of facts. */
export interface DetailSection {
  readonly title: string;
  readonly rows: readonly DetailRow[];
}

/** Everything the detail modal renders, for either kind of trade. */
export interface TradeDetails {
  /** Which adapter produced this — drives only the modal's kind chip. */
  readonly kind: "client" | "hedge";
  /** The record's own id (deal id / hedge id) — the modal's stable test hook. */
  readonly id: string;
  readonly title: string;
  readonly subtitle: string;
  readonly sections: readonly DetailSection[];
  /**
   * The street orders a HEDGE put on the wire. Empty for a client trade, which has no
   * street leg by construction — not missing data.
   */
  readonly orders: readonly StreetOrder[];
}

/** Epoch nanos → an ISO-ish UTC stamp a desk can paste into a log search. */
export function fmtNanos(nanos: bigint | number): string {
  const ms = Number(BigInt(Math.trunc(Number(nanos))) / 1_000_000n);
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  return new Date(ms).toISOString().replace("T", " ").replace("Z", " UTC");
}

/** A number for display: grouped, and compact only once it is genuinely large. */
export function fmtAmount(n: number | undefined): string {
  if (n === undefined || !Number.isFinite(n)) return "—";
  return new Intl.NumberFormat("en-US", {
    notation: Math.abs(n) >= 1_000_000 ? "compact" : "standard",
    maximumFractionDigits: 2,
  }).format(n);
}

/** A price: full precision, because a basis point of a bond price lives in the 4th dp. */
function fmtPrice(n: number | undefined): string {
  return n === undefined || !Number.isFinite(n) ? "—" : n.toFixed(6);
}

/** A wire enum token (`no_firm_lp_price`) as prose. */
export function humanToken(token: string | undefined): string {
  return token === undefined || token === "" ? "—" : token.replace(/_/g, " ");
}

/** The detail view of a booked CLIENT trade. */
export function clientTradeDetails(deal: Deal): TradeDetails {
  // A bond names itself; a swap has no symbol at all, so it is named the way a rates desk
  // says it out loud — "5Y USD OIS".
  const security =
    deal.bondDisplayName ??
    deal.bondSecurityId ??
    `${deal.instrument.tenorYears}Y ${deal.curveSet.currency} OIS`;
  return {
    kind: "client",
    id: deal.dealId,
    title: security,
    subtitle: `${deal.counterparty} · ${sideLabel(deal.side)}`,
    orders: [],
    sections: [
      {
        title: "Trade",
        rows: [
          { label: "Deal id", value: deal.dealId, mono: true },
          { label: "Executed", value: fmtNanos(deal.executedAtNanos) },
          { label: "Counterparty", value: deal.counterparty },
          { label: "Desk", value: deal.desk },
          { label: "Trader", value: deal.trader },
        ],
      },
      {
        title: "Economics",
        rows: [
          { label: "Security", value: security, mono: true },
          { label: "Side", value: sideLabel(deal.side) },
          { label: "Notional", value: fmtAmount(deal.notional), mono: true },
          { label: "Price", value: fmtPrice(deal.price), mono: true },
        ],
      },
      {
        title: "Booking",
        rows: [
          {
            label: "Position",
            value: deal.positionId === undefined ? "—" : `#${deal.positionId}`,
            mono: true,
            hint: "The durable position this fill booked into — the join key to its event trace.",
          },
          { label: "Risk book", value: deal.riskBookId ?? "—", mono: true },
          { label: "Request", value: deal.requestId, mono: true },
          {
            label: "Correlation",
            value: deal.correlationId ?? "—",
            mono: true,
            hint: "The id that ties this deal to its request across every service log.",
          },
        ],
      },
    ],
  };
}

/** The detail view of a fired HEDGE, including the orders it sent to market. */
export function hedgeTradeDetails(
  p: HedgeProvenance,
  orders: readonly StreetOrder[],
): TradeDetails {
  const plan = p.vehiclePlan;
  const sections: DetailSection[] = [
    {
      title: "Decision",
      rows: [
        { label: "Hedge id", value: p.hedgeId, mono: true },
        { label: "Fired", value: fmtNanos(p.firedAt) },
        { label: "Book", value: p.book, mono: true },
        { label: "Instrument", value: p.instrument, mono: true },
        {
          label: "Band",
          value: p.band,
          hint: "The RAG band the book was in when this fired — the trigger, not the outcome.",
        },
        {
          label: "Mode",
          value: p.advisory ? "Advisory (shadow run)" : "Live",
          hint: p.advisory
            ? "A shadow run: the decision was computed and recorded, but nothing was ever traded."
            : "A live fire: orders were put on the wire.",
        },
      ],
    },
    {
      title: "Risk",
      rows: [
        { label: "Net risk", value: fmtAmount(p.netRisk), mono: true },
        { label: "Threshold", value: fmtAmount(p.threshold), mono: true },
        {
          label: "Utilisation",
          value: Number.isFinite(p.utilization) ? `${(p.utilization * 100).toFixed(2)}%` : "—",
          mono: true,
        },
        { label: "Crossed internally", value: fmtAmount(p.internalCrossed), mono: true },
        { label: "Hedged externally", value: fmtAmount(p.externalHedged), mono: true },
        {
          label: "Residual",
          value: fmtAmount(p.residual),
          mono: true,
          hint: "What this hedge did NOT remove — risk the desk still carries.",
        },
      ],
    },
    {
      title: "Execution",
      rows: [
        { label: "Won by", value: p.lpWon ?? "—" },
        { label: "Hedge price", value: fmtPrice(p.hedgePrice), mono: true },
        { label: "Mid at fire", value: fmtPrice(p.midAtFire), mono: true },
        {
          label: "Slippage",
          value: Number.isFinite(p.slippageBp) ? `${p.slippageBp.toFixed(1)}bp` : "—",
          mono: true,
        },
        {
          label: "Panel (eligible)",
          value: p.lps.length === 0 ? "—" : p.lps.join(", "),
          hint: "The providers the exit policy made ELIGIBLE — not the ones that quoted or filled.",
        },
      ],
    },
  ];

  if (plan !== undefined && plan !== null) {
    sections.push({
      title: "Vehicle",
      rows: [
        { label: "Hedged with", value: plan.hedgeInstrumentId, mono: true },
        {
          label: "Units",
          value: `${fmtAmount(plan.units)} ${plan.unitLabel}`,
          mono: true,
          hint: "What the plan sized. A listed venue is asked in face, so the wire quantity is units × contract face.",
        },
        { label: "Exact units", value: plan.exactUnits.toFixed(4), mono: true },
        { label: "DV01 per unit", value: fmtAmount(plan.dv01PerUnit), mono: true },
        {
          label: "Basis",
          value: humanToken(plan.dv01Basis),
          hint: "Which measure the size was computed on. An exposure-proxy basis is duration-blind and therefore approximate.",
        },
        { label: "Residual DV01", value: fmtAmount(plan.residualDv01), mono: true },
      ],
    });
  }

  return {
    kind: "hedge",
    id: p.hedgeId,
    title: plan?.hedgeInstrumentId ?? p.instrument,
    subtitle: `${p.book} · ${p.band} · ${p.advisory ? "advisory" : "live"}`,
    sections,
    orders,
  };
}
