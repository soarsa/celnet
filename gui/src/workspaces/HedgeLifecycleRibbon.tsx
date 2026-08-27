/**
 * HedgeLifecycleRibbon — the missing middle of the hedge story.
 *
 * The desk could already see what it CARRIED (the bucket board) and what had FIRED
 * (the hedge ledger). What no screen showed was the journey between them, and above
 * all the **drop-off**: risk sized for exit that never actually left. This ribbon puts
 * the stages on one line so a gap is visible without opening a blotter:
 *
 *     exposure  →  decided  →  street  →  residual
 *
 * The street stage is the point of the whole component. Its counts come from the
 * street orders stamped with the hedge decision that raised them, so "9/12 filled ·
 * 3 shed nothing · NOT_A_WHOLE_LOT ×3" reads at a glance. That sentence, on this
 * screen, is what was missing while every futures hedge bounced and the board above it
 * cheerfully reported hedges firing.
 *
 * UNITS: the risk stages are the budget metric (DV01); the street stage is ORDER
 * COUNTS and never summed quantities — see the unit rule in `lib/hedgeLifecycle.ts`.
 * Mixing a future's contract face into a DV01 total is the original sin this screen
 * exists to catch.
 *
 * Presentational and prop-driven: it computes nothing, so what it shows and what the
 * ribbon logic says can never diverge.
 */

import { HelpButton } from "../components/HelpButton";
import type { HedgeLifecycle } from "../lib/hedgeLifecycle";
import { streetIsStalled } from "../lib/hedgeLifecycle";
import { formatDv01 } from "../lib/hedgeVehicle";
import styles from "./HedgeLifecycleRibbon.module.css";

/** The stages a click can drill the ledger into (mirrors the ledger's own lenses). */
export type LifecycleStageKey = "exposure" | "decided" | "street" | "residual";

interface HedgeLifecycleRibbonProps {
  /** The computed ribbon. */
  lifecycle: HedgeLifecycle;
  /** The stage currently drilled into, or `null` for none. */
  selected?: LifecycleStageKey | null;
  /** Select (or clear) a stage. Omit to render the ribbon read-only. */
  onSelect?: (stage: LifecycleStageKey | null) => void;
}

function StageBody({
  label,
  value,
  unit,
  detail,
}: {
  label: string;
  value: string;
  unit: string;
  detail: React.ReactNode;
}): React.ReactElement {
  return (
    <>
      <span className={styles.stageLabel}>{label}</span>
      <span className={styles.stageValue}>
        {value}
        <span className={styles.stageUnit}>{unit}</span>
      </span>
      <span className={styles.stageDetail}>{detail}</span>
    </>
  );
}

/** One stage cell: a headline figure, its unit, and the breakdown beneath. */
function Stage({
  stageKey,
  label,
  value,
  unit,
  detail,
  tone,
  selected,
  onSelect,
}: {
  stageKey: LifecycleStageKey;
  label: string;
  value: string;
  unit: string;
  detail: React.ReactNode;
  tone?: "warn" | undefined;
  selected: boolean;
  onSelect?: ((stage: LifecycleStageKey | null) => void) | undefined;
}): React.ReactElement {
  const className = [
    styles.stage,
    selected ? styles.stageActive : "",
    tone === "warn" ? styles.stageWarn : "",
  ]
    .filter(Boolean)
    .join(" ");
  // A stage with no handler is a readout, not a control — rendering it as a disabled
  // button would still put it in the tab order and promise an interaction it lacks.
  if (onSelect === undefined) {
    return (
      <div className={className} data-testid={`hedge-stage-${stageKey}`}>
        <StageBody label={label} value={value} unit={unit} detail={detail} />
      </div>
    );
  }
  return (
    <button
      type="button"
      aria-pressed={selected}
      className={className}
      data-testid={`hedge-stage-${stageKey}`}
      onClick={() => onSelect(selected ? null : stageKey)}
    >
      <StageBody label={label} value={value} unit={unit} detail={detail} />
    </button>
  );
}

export function HedgeLifecycleRibbon({
  lifecycle,
  selected = null,
  onSelect,
}: HedgeLifecycleRibbonProps): React.ReactElement {
  const { street } = lifecycle;
  const stalled = streetIsStalled(street);
  const shed = street.filled + street.partial;

  return (
    <section className={styles.root} data-testid="hedge-lifecycle-ribbon">
      <header className={styles.head}>
        <h3 className={styles.title}>
          The flow
          <HelpButton helpId="concept.hedge-lifecycle" subject="how to read the hedge flow" />
        </h3>
        <p className={styles.caption}>
          What the desk carries, what the engine decided, and what the street actually
          took. Advisory fires excluded.
        </p>
      </header>

      {/*
       * The stall banner leads the ribbon rather than sitting in a corner, because it
       * is the one state where every OTHER number on this page looks healthy: hedges
       * fire, decisions size correctly, and not one unit of risk leaves the book.
       */}
      {stalled && (
        <p className={styles.stalled} role="alert" data-testid="hedge-street-stalled">
          <strong>Nothing is filling.</strong> All {street.sent} order
          {street.sent === 1 ? "" : "s"} sent to the street shed no risk
          {street.reasons.length > 0 ? ` — ${street.reasons[0]!.reason}` : ""}. The books
          will not drain until this clears.
        </p>
      )}

      <ol className={styles.ribbon}>
        <li className={styles.step}>
          <Stage
            stageKey="exposure"
            label="Exposure"
            value={formatDv01(lifecycle.exposure)}
            unit="DV01"
            detail="carried across capped books"
            selected={selected === "exposure"}
            onSelect={onSelect}
          />
        </li>
        <li className={styles.step} aria-hidden="true">
          <span className={styles.arrow}>→</span>
        </li>
        <li className={styles.step}>
          <Stage
            stageKey="decided"
            label="Decided"
            value={formatDv01(lifecycle.decided)}
            unit="DV01"
            detail={
              <>
                {formatDv01(lifecycle.crossed)} crossed · {formatDv01(lifecycle.external)}{" "}
                external · {formatDv01(lifecycle.warehoused)} warehoused
              </>
            }
            selected={selected === "decided"}
            onSelect={onSelect}
          />
        </li>
        <li className={styles.step} aria-hidden="true">
          <span className={styles.arrow}>→</span>
        </li>
        <li className={styles.step}>
          <Stage
            stageKey="street"
            label="Street"
            value={`${shed}/${street.sent}`}
            unit="filled"
            // Counts, never quantities — a future's face and a swap's notional do not
            // add up to anything a desk can act on.
            detail={
              street.sent === 0 ? (
                "no orders sent"
              ) : (
                <>
                  {street.unfilled > 0 ? (
                    <strong className={styles.unfilled}>
                      {street.unfilled} shed nothing
                    </strong>
                  ) : (
                    "all filled"
                  )}
                  {street.partial > 0 ? ` · ${street.partial} partial` : ""}
                  {street.reasons.length > 0
                    ? ` · ${street.reasons
                        .slice(0, 2)
                        .map((r) => `${r.reason} ×${r.count}`)
                        .join(" · ")}`
                    : ""}
                </>
              )
            }
            tone={street.unfilled > 0 ? "warn" : undefined}
            selected={selected === "street"}
            onSelect={onSelect}
          />
        </li>
        <li className={styles.step} aria-hidden="true">
          <span className={styles.arrow}>→</span>
        </li>
        <li className={styles.step}>
          <Stage
            stageKey="residual"
            label="Residual"
            value={formatDv01(lifecycle.warehoused)}
            unit="DV01"
            detail="sized but never shed — still ours"
            selected={selected === "residual"}
            onSelect={onSelect}
          />
        </li>
      </ol>

      <p className={styles.foot}>
        {lifecycle.fires === 0
          ? "No live hedge has fired yet."
          : `From ${lifecycle.fires} live fire${lifecycle.fires === 1 ? "" : "s"}.`}
        {street.unlinked > 0 && (
          // Never silently dropped: an unstamped order is a gap in the audit walk, not
          // an absence of activity, and the two look identical if we say nothing.
          <span className={styles.unlinked} data-testid="hedge-street-unlinked">
            {" "}
            {street.unlinked} street order{street.unlinked === 1 ? "" : "s"} carry no hedge
            id and are not counted above.
          </span>
        )}
      </p>
    </section>
  );
}
