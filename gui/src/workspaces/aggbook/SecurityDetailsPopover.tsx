/**
 * SecurityDetailsPopover — the per-tile "Details" affordance for the Aggregated
 * Book. The tile face now carries only the trading essentials; the full static
 * terms (ISIN, CUSIP, issuer, coupon, frequency, day-count, maturity) move behind
 * this button.
 *
 * Behaviour: a keyboard-focusable trigger that OPENS the terms on click (pinned)
 * and PREVIEWS them on hover/focus. The panel is rendered through a portal so it
 * escapes the tile's `overflow: hidden`, is positioned to stay on-screen, and
 * dismisses on Escape / click-away / blur (see {@link useAnchoredPopover}). It is a
 * labelled `role="dialog"`. Terms are sourced from the reference-data join the
 * workspace already does ({@link resolveBondDef}) — no refetch.
 */

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import type { AggregatedInstrument, BondDef } from "../../data/contract";
import { bondTermRows } from "../../lib/bondTerms";
import { useAnchoredPopover } from "../../lib/useAnchoredPopover";
import styles from "./SecurityDetailsPopover.module.css";

/** One label/value detail row, tagged for monospace when it is a code/number. */
interface DetailRow {
  key: string;
  label: string;
  value: string;
  mono: boolean;
}

/** Assemble the identifier + bond-term rows for the popover body. */
function detailRows(instrument: AggregatedInstrument, bond: BondDef | null): DetailRow[] {
  const rows: DetailRow[] = [];
  if (instrument.isin) rows.push({ key: "isin", label: "ISIN", value: instrument.isin, mono: true });
  if (instrument.cusip) rows.push({ key: "cusip", label: "CUSIP", value: instrument.cusip, mono: true });
  if (bond) {
    for (const r of bondTermRows(bond)) {
      rows.push({ key: r.key, label: r.label, value: r.value, mono: r.numeric });
    }
  }
  return rows;
}

export function SecurityDetailsPopover({
  instrument,
  bond,
}: {
  instrument: AggregatedInstrument;
  /** The joined reference-data bond terms, or `null` (non-bond / unseeded). */
  bond: BondDef | null;
}): React.ReactElement | null {
  const { open, setOpen, anchorRef, floatingRef, floatingStyle, reposition } =
    useAnchoredPopover<HTMLButtonElement>("bottom-end");
  // A click PINS the popover open; hover/focus only PREVIEWS. Kept in a ref so the
  // enter/leave handlers read the latest value without re-subscribing. Any close
  // (Escape / outside / leave) clears the pin so a later hover previews again.
  const pinned = useRef(false);
  // When the popover is dismissed by Escape, the hook returns focus to the trigger.
  // That refocus (and any lingering hover) must NOT immediately re-preview it open,
  // so a short window after any close suppresses the hover/focus preview path.
  const lastClosedAt = useRef(0);
  useEffect(() => {
    if (!open) {
      pinned.current = false;
      lastClosedAt.current = Date.now();
    }
  }, [open]);
  const RECENT_CLOSE_MS = 250;

  const rows = detailRows(instrument, bond);
  const name = instrument.displayName || instrument.instrumentId;
  const titleId = `secdetails-${instrument.instrumentId}`;

  // Nothing to reveal (no identifiers, no bond terms): omit the affordance.
  if (rows.length === 0) return null;

  const preview = (): void => {
    // Do not re-open on the refocus/hover that immediately follows a dismissal.
    if (Date.now() - lastClosedAt.current < RECENT_CLOSE_MS) return;
    setOpen(true);
  };
  const endPreview = (): void => {
    if (!pinned.current) setOpen(false);
  };
  const togglePin = (): void => {
    pinned.current = !pinned.current;
    setOpen(pinned.current);
  };
  // Blur that moves focus INTO the portalled panel must not close it.
  const onBlur = (e: React.FocusEvent): void => {
    if (floatingRef.current?.contains(e.relatedTarget as Node | null)) return;
    endPreview();
  };

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        className={styles.trigger}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Security details for ${name}`}
        onClick={togglePin}
        onPointerEnter={preview}
        onPointerLeave={endPreview}
        onFocus={preview}
        onBlur={onBlur}
      >
        <span aria-hidden="true" className={styles.triggerGlyph}>
          ⓘ
        </span>
        Details
      </button>

      {open &&
        createPortal(
          <div
            ref={floatingRef}
            role="dialog"
            aria-labelledby={titleId}
            className={styles.pop}
            style={floatingStyle}
            onPointerEnter={preview}
            onPointerLeave={endPreview}
          >
            <div className={styles.popHead}>
              <span id={titleId} className={styles.popTitle}>
                {name}
              </span>
              <span className={styles.popKicker}>Security details</span>
            </div>
            <dl className={styles.rows}>
              {rows.map((r) => {
                // The layout effect measures once mounted; nudge a reposition on the
                // first row so a tall panel re-clamps after content lands.
                return (
                  <div key={r.key} className={styles.row}>
                    <dt className={styles.rowLabel}>{r.label}</dt>
                    <dd className={`${styles.rowValue} ${r.mono ? styles.mono : ""}`}>{r.value}</dd>
                  </div>
                );
              })}
            </dl>
            <MeasureOnMount onReady={reposition} />
          </div>,
          document.body,
        )}
    </>
  );
}

/** Fire a one-shot callback after the panel's content has laid out (re-clamp). */
function MeasureOnMount({ onReady }: { onReady: () => void }): null {
  useEffect(() => {
    onReady();
  }, [onReady]);
  return null;
}
