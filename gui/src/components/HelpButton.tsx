/**
 * HelpButton — a discoverable "?" affordance that opens the rich {@link HelpPanel}
 * for one registry topic. Drops in next to a pricing feature card or a tiering
 * strategy row (replacing the old doc-link out to GitHub) so every feature's
 * explanation, worked example, and "Walk me through it" tour is IN the app.
 *
 * Self-contained: it owns the open/closed state and reads the tour launcher from
 * the {@link useTour} context (a no-op default when rendered outside a
 * TourProvider, so isolated component tests never crash).
 */

import { useState } from "react";

import { useTour } from "../app/TourProvider";
import { getHelp } from "../lib/help";
import { HelpPanel } from "./HelpPanel";
import styles from "./HelpButton.module.css";

export interface HelpButtonProps {
  /** The help-entry id to open (e.g. "feature.tiering", "strategy.flat-markup"). */
  helpId: string;
  /** A short subject for the accessible label ("the Flat markup strategy"). */
  subject: string;
  /** Optional extra class (to match a host's "?" pill styling). */
  className?: string;
}

export function HelpButton({ helpId, subject, className }: HelpButtonProps): React.ReactElement | null {
  const [open, setOpen] = useState(false);
  const { startTour } = useTour();
  const entry = getHelp(helpId);

  // A missing entry is a programming error, not a runtime surface — render nothing
  // rather than a dead "?" (the vitest registry test guards every referenced id).
  if (!entry) return null;

  return (
    <>
      <button
        type="button"
        className={`${styles.help} ${className ?? ""}`}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(true);
        }}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={`Help: ${entry.title} — ${entry.purpose}`}
        aria-label={`Help for ${subject}`}
      >
        <span aria-hidden="true">?</span>
      </button>
      {open && <HelpPanel entry={entry} onClose={() => setOpen(false)} onStartTour={startTour} />}
    </>
  );
}
