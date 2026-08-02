/**
 * DealTicket — the full detail view of one booked deal, opened from a row in the
 * received-deals blotter. The blotter is a trader-scannable summary; this ticket
 * carries the settlement-grade economics a detailer (middle/back office) needs to
 * confirm and settle the trade — schedule dates, day-count, currency, the
 * originating request, and the booked position id.
 *
 * Pure presentation over the existing `Deal` contract (no extra fetch): the OIS
 * schedule is spot-starting off the curve reference date, so the effective date is
 * that anchor and maturity is anchor + tenor whole years (SOFR OIS, ACT/360).
 */

import type { Deal, Side } from "../data/contract";
import { fmtRate, fmtClock, fmtCompact } from "../lib/format";
import { fmtEdgeBps, hedgeBandLabel, internaliseLabel } from "../lib/internalise";
import styles from "./DealTicket.module.css";

function directionLabel(side: Side): string {
  if (side === "BUY") return "Pay fixed (payer)";
  if (side === "SELL") return "Receive fixed (receiver)";
  return "Two-way";
}

/** `{year,month,day}` → ISO civil date `2026-06-29`. */
function fmtCivil(d: { year: number; month: number; day: number }): string {
  const mm = String(d.month).padStart(2, "0");
  const dd = String(d.day).padStart(2, "0");
  return `${d.year}-${mm}-${dd}`;
}

interface RowProps {
  readonly label: string;
  readonly value: React.ReactNode;
  readonly mono?: boolean;
}

function Row({ label, value, mono }: RowProps): React.ReactElement {
  return (
    <div className={styles.row}>
      <dt className={styles.dt}>{label}</dt>
      <dd className={`${styles.dd} ${mono ? styles.mono : ""}`}>{value}</dd>
    </div>
  );
}

export interface DealTicketProps {
  readonly deal: Deal;
  readonly onClose: () => void;
}

export function DealTicket({ deal, onClose }: DealTicketProps): React.ReactElement {
  const ccy = deal.curveSet.currency;
  const ref = deal.curveSet.referenceDate;
  // OIS is spot-starting off the curve anchor; maturity is anchor + tenor years.
  const maturity = { year: ref.year + deal.instrument.tenorYears, month: ref.month, day: ref.day };
  const notionalText = fmtCompact(deal.notional);

  return (
    <aside
      className={styles.ticket}
      role="dialog"
      aria-modal="false"
      aria-label={`Deal ${deal.dealId}`}
    >
      <header className={styles.header}>
        <div>
          <span className={`${styles.kindTag} ${deal.kind === "IOI" ? styles.ioi : styles.rfq}`}>
            {deal.kind}
          </span>
          <h2 className={styles.title}>
            {deal.instrument.tenorYears}Y {ccy} OIS
          </h2>
          <p className={styles.subtitle}>{deal.counterparty}</p>
        </div>
        <button type="button" className={styles.close} onClick={onClose} aria-label="Close ticket">
          ×
        </button>
      </header>

      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>Economics</h3>
        <dl className={styles.dl}>
          <Row label="Product" value={`${ccy} SOFR OIS`} />
          <Row label="Tenor" value={`${deal.instrument.tenorYears}Y`} />
          <Row label="Direction (desk)" value={directionLabel(deal.side)} />
          <Row label="Notional" value={`${notionalText} ${ccy}`} mono />
          <Row label="Dealt rate" value={fmtRate(deal.price)} mono />
          <Row label="Fixed coupon" value={fmtRate(deal.instrument.fixedRate)} mono />
        </dl>
      </section>

      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>Schedule &amp; settlement</h3>
        <dl className={styles.dl}>
          <Row label="Trade time" value={fmtClock(deal.executedAtNanos)} mono />
          <Row label="Effective (spot)" value={fmtCivil(ref)} mono />
          <Row label="Maturity" value={fmtCivil(maturity)} mono />
          <Row label="Day-count" value="ACT/360" />
          <Row label="Fixed frequency" value="Annual" />
          <Row label="Currency" value={ccy} />
        </dl>
      </section>

      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>Counterparty &amp; booking</h3>
        <dl className={styles.dl}>
          <Row label="Counterparty" value={deal.counterparty} />
          <Row label="Request type" value={deal.kind === "IOI" ? "Indication of interest" : "Request for quote"} />
          <Row label="Desk" value={deal.desk} />
          <Row label="Trader" value={deal.trader} />
        </dl>
      </section>

      {deal.internalise !== undefined && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>Internalise &amp; auto-hedge</h3>
          <dl className={styles.dl}>
            <Row
              label="Decision"
              value={
                <span
                  className={styles.decision}
                  data-band={deal.internalise.hedgeBand}
                  data-losing={deal.internalise.withinTolerance ? undefined : "true"}
                >
                  {internaliseLabel(deal.internalise)}
                </span>
              }
            />
            <Row
              label="Captured edge"
              value={
                <span
                  className={deal.internalise.edgeBps < 0 ? styles.neg : styles.pos}
                >
                  {fmtEdgeBps(deal.internalise.edgeBps)}
                </span>
              }
              mono
            />
            <Row
              label="Within tolerance"
              value={
                deal.internalise.withinTolerance ? (
                  "Yes"
                ) : (
                  <span className={styles.neg}>No — losing</span>
                )
              }
            />
            <Row
              label="Hedge band"
              value={
                <span className={styles.band} data-band={deal.internalise.hedgeBand}>
                  {hedgeBandLabel(deal.internalise.hedgeBand)}
                </span>
              }
            />
            <Row label="Internal DV01" value={fmtCompact(deal.internalise.internalDv01)} mono />
            <Row label="External DV01" value={fmtCompact(deal.internalise.externalDv01)} mono />
          </dl>
        </section>
      )}

      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>References</h3>
        <dl className={styles.dl}>
          <Row label="Deal ID" value={deal.dealId} mono />
          <Row label="Request ID" value={deal.requestId} mono />
          {deal.positionId !== undefined && (
            <Row label="Position ID" value={String(deal.positionId)} mono />
          )}
          {deal.correlationId !== undefined && (
            <Row label="Correlation ID" value={deal.correlationId} mono />
          )}
        </dl>
      </section>
    </aside>
  );
}
