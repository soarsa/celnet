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

import { TradeDetailsModal } from "../components/TradeDetailsModal";
import type { Deal } from "../data/contract";
import { clientTradeDetails } from "../lib/tradeDetails";
import { fmtClock, fmtCompact } from "../lib/format";
import { fmtEdgeBps, hedgeBandLabel, internaliseLabel } from "../lib/internalise";
import styles from "./DealTicket.module.css";


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

  return (
    <TradeDetailsModal
      details={clientTradeDetails(deal)}
      onClose={onClose}
      extra={
        <>
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

        </>
      }
    />
  );
}
