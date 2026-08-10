/**
 * MobileRiskBoard — the glanceable per-portfolio RISK view of the mobile status
 * board. A 2-second summary strip (net notional / net DV01 / # breaching / worst
 * utilisation) sits above per-portfolio cards sorted WORST-FIRST, each showing its
 * RAG band, net DV01, net notional and a utilisation bar with breach highlighting.
 *
 * Pure presentation over `listRiskBookRisk` rows already split by asset + folded by
 * {@link ../../lib/mobileStatus}. Read-only; no mutating affordance.
 */

import type { CapabilityAsset, RiskBookRisk, RiskLimitUtilization } from "../../data/contract";
import { fmtCompact } from "../../lib/format";
import { riskCards, summariseRisk, type MobileRiskCard } from "../../lib/mobileStatus";
import { MobileEmpty, MobileError, MobileSkeleton } from "./MobileStates";
import styles from "./MobileStatusApp.module.css";

/** `null` DV01/PnL renders as the honest em dash, never a fabricated 0. */
function optCompact(value: number | null): string {
  return value === null ? "—" : fmtCompact(value);
}

/** The worst-utilisation percentage label (or "—" when no caps are evaluable). */
function worstPctLabel(fraction: number): string {
  if (fraction <= 0) return "—";
  return `${Math.round(fraction * 100)}%`;
}

function UtilBar({ util }: { util: RiskLimitUtilization }): React.ReactElement {
  const pct = Number.isFinite(util.fraction) ? Math.min(100, Math.max(0, util.fraction * 100)) : 100;
  const label = util.metric.replace(/_/g, " ");
  const pctText = Number.isFinite(util.fraction) ? `${Math.round(util.fraction * 100)}%` : "breach";
  return (
    <div className={styles.util}>
      <div className={styles.utilHead}>
        <span className={styles.utilMetric}>{label}</span>
        <span className={styles.utilRatio}>
          {fmtCompact(util.used)} / {fmtCompact(util.limit)}{" "}
          <span className={styles.utilPct}>({pctText})</span>
        </span>
      </div>
      <div className={styles.bar}>
        <div
          className={styles.barFill}
          data-band={util.band}
          style={{ width: `${pct}%` }}
          role="meter"
          aria-valuenow={Math.round(pct)}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label={`${label} utilisation`}
        />
      </div>
    </div>
  );
}

function RiskCard({ card }: { card: MobileRiskCard }): React.ReactElement {
  const { row, band } = card;
  return (
    <article className={styles.card} data-band={band} data-breach={card.breaching || undefined}>
      <div className={styles.cardTop}>
        <span className={styles.ragDot} data-band={band} aria-hidden />
        <span className={styles.cardTitle}>{row.name}</span>
        <span className={styles.cardBadge} data-band={band}>
          {band === "red" ? "Breach" : band === "amber" ? "Watch" : "OK"}
        </span>
      </div>
      <dl className={styles.metricRow}>
        <div className={styles.metric}>
          <dt>Net notional</dt>
          <dd className={styles.num}>{fmtCompact(row.netNotional)}</dd>
        </div>
        <div className={styles.metric}>
          <dt>Net DV01</dt>
          <dd className={styles.num}>{optCompact(row.dv01)}</dd>
        </div>
        <div className={styles.metric}>
          <dt>Positions</dt>
          <dd className={styles.num}>{row.positionCount}</dd>
        </div>
      </dl>
      {row.limits.length > 0 ? (
        <div className={styles.utils}>
          {row.limits.map((u) => (
            <UtilBar key={u.metric} util={u} />
          ))}
        </div>
      ) : (
        <p className={styles.noLimits}>No limits configured</p>
      )}
    </article>
  );
}

export interface MobileRiskBoardProps {
  rows: readonly RiskBookRisk[];
  isLoading: boolean;
  error: unknown;
  asset: CapabilityAsset;
}

export function MobileRiskBoard({
  rows,
  isLoading,
  error,
  asset,
}: MobileRiskBoardProps): React.ReactElement {
  if (isLoading && rows.length === 0) return <MobileSkeleton rows={4} />;
  if (error && rows.length === 0) return <MobileError message="Couldn't load risk." />;
  if (rows.length === 0) {
    return (
      <MobileEmpty
        message={
          asset === "fx_options"
            ? "No FX Options risk portfolios."
            : "No risk portfolios yet."
        }
      />
    );
  }

  const summary = summariseRisk(rows);
  const cards = riskCards(rows);

  return (
    <div className={styles.board}>
      <section className={styles.summary} aria-label="risk summary">
        <div className={styles.statTile} data-tone="accent">
          <span className={styles.statLabel}>Net notional</span>
          <span className={styles.statValue}>{fmtCompact(summary.netNotional)}</span>
        </div>
        <div className={styles.statTile}>
          <span className={styles.statLabel}>Net DV01</span>
          <span className={styles.statValue}>{optCompact(summary.netDv01)}</span>
        </div>
        <div className={styles.statTile} data-tone={summary.breaching > 0 ? "danger" : undefined}>
          <span className={styles.statLabel}>Breaching</span>
          <span className={styles.statValue}>
            {summary.breaching}
            <span className={styles.statSub}>/{summary.bookCount}</span>
          </span>
        </div>
        <div className={styles.statTile} data-tone={summary.worstBand}>
          <span className={styles.statLabel}>Worst util</span>
          <span className={styles.statValue}>{worstPctLabel(summary.worstFraction)}</span>
        </div>
      </section>

      <ul className={styles.cardList}>
        {cards.map((card) => (
          <li key={card.row.bookId}>
            <RiskCard card={card} />
          </li>
        ))}
      </ul>
    </div>
  );
}
