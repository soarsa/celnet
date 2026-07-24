/**
 * InstrumentDialog — the create / edit instrument-definition modal for the
 * Reference Data (instrument registry) workspace.
 *
 * The `InstrumentForm` is long (deposit / FRA / STIR / IRS / OIS / bond families
 * each swap in their own field set), so it MUST NOT be rendered inline beneath the
 * registry table: when the workspace mounts inside a height-constrained container
 * (the FI shell) an inline panel is pushed below the fold and clipped, leaving the
 * form unreachable. Rendering it as a portalled scrim+blur modal — above the list,
 * centred, with a scrollable body — makes it fully usable in every context the
 * workspace mounts (FX / rates / FI admin).
 *
 * Mirrors the established dialog pattern (`FixSpecModal` / `UserDialog`):
 * `createPortal` to `document.body` (escapes any `overflow`/`transform` ancestor),
 * `role="dialog"` + `aria-modal`, a labelled title, backdrop-click + Esc to close,
 * initial focus moved into the dialog and returned to the opener on close.
 */

import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";

import type { InstrumentDef, InstrumentInput } from "../data/contract";
import { InstrumentForm } from "../workspaces/ReferenceDataForms";
import { Button } from "./Button";
import styles from "./InstrumentDialog.module.css";

export interface InstrumentDialogProps {
  /** Whether the dialog is mounted/visible. */
  open: boolean;
  /** The def to edit, or `null` for a fresh create form. */
  editing: InstrumentDef | null;
  /** The shared action-error banner (a server rejection), surfaced in-context. */
  error?: string | null;
  /** Close without submitting (backdrop / Esc / Close / after a successful submit). */
  onClose: () => void;
  onCreate: (input: InstrumentInput) => Promise<unknown>;
  onUpdate: (input: InstrumentInput) => Promise<unknown>;
  /** The shared run/clear-after-await wrapper (surfaces errors). */
  run: (action: () => Promise<unknown>) => Promise<void>;
}

/** Interactive elements eligible for the dialog's initial focus. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function InstrumentDialog({
  open,
  editing,
  error,
  onClose,
  onCreate,
  onUpdate,
  run,
}: InstrumentDialogProps): React.ReactElement | null {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  // Remember the opener, move focus into the dialog on open, and return focus to
  // the opener when it closes (or unmounts).
  useEffect(() => {
    if (!open) return;
    openerRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = panelRef.current;
    const target =
      panel?.querySelector<HTMLElement>("select, input, textarea") ??
      panel?.querySelector<HTMLElement>(FOCUSABLE) ??
      null;
    target?.focus();
    return () => {
      openerRef.current?.focus();
    };
  }, [open]);

  // Esc closes while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div ref={panelRef} className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            {editing ? `Edit ${editing.name}` : "New instrument"}
          </h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
        <div className={styles.body}>
          {error && (
            <p className={styles.banner} role="alert">
              {error}
            </p>
          )}
          <InstrumentForm
            key={editing ? editing.instrumentId : "new"}
            editing={editing}
            onCreate={onCreate}
            onUpdate={onUpdate}
            onDone={onClose}
            run={run}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}
