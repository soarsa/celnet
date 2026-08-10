/**
 * MobileHelpSheet — a bottom-sheet overlay for the mobile status board. Two variants:
 *   • `hint` — the first-open 3-step mini tour (auto-shown once, then dismissed).
 *   • `help` — the full explainer (opened from the header "?").
 *
 * The board is read-only, so the "tour" is explanatory rather than interactive; the
 * same content is registered in `lib/help.ts` (`concept.mobile-status-board`) so it is
 * searchable from the desktop Help center too. Self-contained content keeps the sheet
 * independent of the desktop `TourOverlay`, which targets desktop DOM anchors.
 */

import { useState } from "react";

import styles from "./MobileStatusApp.module.css";

interface HintStep {
  title: string;
  body: string;
}

const HINT_STEPS: readonly HintStep[] = [
  {
    title: "Your risk, at a glance",
    body: "This is the mobile status board — a read-only glance at risk and flow. The summary strip reads in two seconds; portfolio cards below are sorted worst-first, with a green / amber / red band each.",
  },
  {
    title: "Three views, per asset",
    body: "Switch between Risk, Hedges and Client flow. If you're entitled to both Fixed Income and FX Options, use the asset tabs at the top. Tap any blotter row to expand its detail.",
  },
  {
    title: "Need the full desk?",
    body: "Everything here is view-only. Tap “Full app ↗” in the header to switch to the complete desktop trading app on this device — your choice is remembered until you switch back.",
  },
];

const HELP_SECTIONS: readonly HintStep[] = [
  {
    title: "What this is",
    body: "A purpose-built, touch-friendly, read-only status board for phones. It shows the same live risk and flow the desktop Risk surface does — never any create / edit / route / hedge-fire control.",
  },
  {
    title: "Risk",
    body: "A summary strip (net notional, net DV01, breaching count, worst utilisation) over per-portfolio cards sorted worst-first. Each card shows its RAG band, net DV01, net notional, positions, and a utilisation bar per limit. DV01 shows “—” when not yet evaluated — never a fabricated zero.",
  },
  {
    title: "Hedges",
    body: "The fired-hedge audit trail: time, portfolio, venue (internal cross vs external LP), hedged size and whether it was advisory. Tap a row for slippage, band, residual and volumes.",
  },
  {
    title: "Client flow",
    body: "Received client deals: time, counterparty, product · tenor, side, notional, routed portfolio and the internalised / B2B disposition. Tap a row for the level and trader. Long blotters are capped with “Show more”.",
  },
  {
    title: "Full app",
    body: "Tap “Full app ↗” to use the complete desktop app on this device; a “Mobile view” button returns you. Sign-in, sign-out and reconnect behave exactly as the desktop app.",
  },
];

export interface MobileHelpSheetProps {
  variant: "hint" | "help";
  onClose: () => void;
}

export function MobileHelpSheet({ variant, onClose }: MobileHelpSheetProps): React.ReactElement {
  const [step, setStep] = useState(0);
  const isHint = variant === "hint";
  const steps = isHint ? HINT_STEPS : HELP_SECTIONS;
  const last = step >= steps.length - 1;

  return (
    <div className={styles.sheetScrim} role="dialog" aria-modal="true" aria-label="help">
      <button
        type="button"
        className={styles.sheetBackdrop}
        aria-label="close help"
        onClick={onClose}
      />
      <div className={styles.sheet}>
        <div className={styles.sheetHandle} aria-hidden />
        {isHint ? (
          <>
            <h2 className={styles.sheetTitle}>{steps[step]!.title}</h2>
            <p className={styles.sheetBody}>{steps[step]!.body}</p>
            <div className={styles.dots} aria-hidden>
              {steps.map((_, i) => (
                <span key={i} className={styles.dot} data-on={i === step || undefined} />
              ))}
            </div>
            <div className={styles.sheetActions}>
              <button type="button" className={styles.textBtn} onClick={onClose}>
                Skip
              </button>
              <button
                type="button"
                className={styles.primaryBtn}
                onClick={() => (last ? onClose() : setStep((s) => s + 1))}
              >
                {last ? "Got it" : "Next"}
              </button>
            </div>
          </>
        ) : (
          <>
            <h2 className={styles.sheetTitle}>Mobile status board</h2>
            <div className={styles.helpScroll}>
              {HELP_SECTIONS.map((s) => (
                <section key={s.title} className={styles.helpSection}>
                  <h3 className={styles.helpHeading}>{s.title}</h3>
                  <p className={styles.sheetBody}>{s.body}</p>
                </section>
              ))}
            </div>
            <div className={styles.sheetActions}>
              <button type="button" className={styles.primaryBtn} onClick={onClose}>
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
