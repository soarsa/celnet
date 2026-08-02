/**
 * PricingControlMenu — the firm-wide pricing kill-switch, a compact top-toolbar
 * control. It reflects the LIVE {@link usePricingControl} state (never merely
 * optimistic local state) and lets an entitled operator halt or resume pricing.
 *
 * Gating: the WHOLE control is hidden unless the signed-in user holds
 * `manage_liquidity·fixed_income` (the banner still shows to everyone — see
 * {@link PricingHaltBanner}). The two halt actions are DESTRUCTIVE, so each
 * requires an inline confirm before it is sent; Resume (the safe recovery) fires
 * immediately and is only offered while something is halted.
 *
 * Semantics (frozen server contract):
 *   • Stop all pricing → {outbound:false, inbound:true}  (still aggregate LP
 *     pricing into the books, but stop ALL outbound quoting to FIX clients).
 *   • Stop all         → {outbound:false, inbound:false} (also stop inbound
 *     aggregation into the books).
 *   • Resume           → {outbound:true,  inbound:true}.
 *
 * Accessibility: a labelled trigger with `aria-haspopup`/`aria-expanded`; the
 * popover is a `role="dialog"`, focus moves into it on open and back to the trigger
 * on close, and Escape / outside-click close it.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { usePricingControl, type HaltLevel } from "../app/PricingControlProvider";
import { Button } from "./Button";
import styles from "./PricingControlMenu.module.css";

/** A pending destructive action awaiting inline confirmation. */
type Confirming = "outbound" | "all" | null;

/** The trigger's short status label for each halt level. */
function statusLabel(level: HaltLevel): string {
  switch (level) {
    case "all":
      return "All pricing halted";
    case "outbound":
      return "Outbound halted";
    case "inbound":
      return "Inbound halted";
    case "none":
      return "Pricing live";
  }
}

export function PricingControlMenu(): React.ReactElement | null {
  const app = useApp();
  const { level, halted, pending, error, setPricingControl } = usePricingControl();

  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState<Confirming>(null);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const titleId = useId();

  const close = useCallback(() => {
    setOpen(false);
    setConfirming(null);
  }, []);

  // Focus into the popover on open; return focus to the trigger on close.
  const wasOpenRef = useRef(false);
  useEffect(() => {
    if (open) {
      popRef.current?.focus();
      wasOpenRef.current = true;
    } else if (wasOpenRef.current) {
      triggerRef.current?.focus();
      wasOpenRef.current = false;
    }
  }, [open]);

  // Escape + outside-click close while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onDown = (e: MouseEvent): void => {
      const t = e.target as Node | null;
      if (t && !popRef.current?.contains(t) && !triggerRef.current?.contains(t)) close();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open, close]);

  // Only entitled operators see the control at all (UX gate; the server enforces).
  if (!app.auth.can("manage_liquidity", "fixed_income")) return null;

  const send = (outbound: boolean, inbound: boolean): void => {
    void setPricingControl(outbound, inbound)
      .then(() => close())
      .catch(() => {
        // Keep the popover open so the captured `error` is visible; reset confirm.
        setConfirming(null);
      });
  };

  const levelClass =
    level === "all" ? styles.stateDanger : level === "none" ? styles.stateLive : styles.stateWarn;

  return (
    <div className={styles.root}>
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${levelClass}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Firm-wide pricing controls — ${statusLabel(level)}`}
        title="Firm-wide pricing controls"
        onClick={() => setOpen((o) => !o)}
      >
        <span className={styles.dot} aria-hidden />
        <span className={styles.triggerLabel}>{statusLabel(level)}</span>
      </button>

      {open && (
        <div
          ref={popRef}
          className={styles.popover}
          role="dialog"
          aria-modal="false"
          aria-labelledby={titleId}
          tabIndex={-1}
        >
          <div className={styles.head}>
            <h2 id={titleId} className={styles.title}>
              Pricing controls
            </h2>
            <span className={`${styles.badge} ${levelClass}`}>{statusLabel(level)}</span>
          </div>
          <p className={styles.note}>
            Firm-wide kill-switch. Halting stops pricing for ALL FIX-connected clients.
          </p>

          {confirming === null ? (
            <div className={styles.actions}>
              <button
                type="button"
                className={styles.actionWarn}
                disabled={pending}
                onClick={() => setConfirming("outbound")}
              >
                <span className={styles.actionTitle}>Stop all pricing</span>
                <span className={styles.actionSub}>
                  Stop outbound quoting; keep aggregating LP pricing into the books
                </span>
              </button>
              <button
                type="button"
                className={styles.actionDanger}
                disabled={pending}
                onClick={() => setConfirming("all")}
              >
                <span className={styles.actionTitle}>Stop all</span>
                <span className={styles.actionSub}>
                  Stop outbound quoting AND inbound aggregation into the books
                </span>
              </button>
              {halted && (
                <button
                  type="button"
                  className={styles.actionResume}
                  disabled={pending}
                  onClick={() => send(true, true)}
                >
                  <span className={styles.actionTitle}>Resume pricing</span>
                  <span className={styles.actionSub}>Re-enable outbound quoting + inbound aggregation</span>
                </button>
              )}
            </div>
          ) : (
            <div className={styles.confirm} role="group" aria-label="confirm halt">
              <p className={styles.confirmText}>
                {confirming === "all"
                  ? "Stop ALL pricing — outbound quoting and inbound aggregation? Clients will not be quoted and the books will stop updating."
                  : "Stop ALL outbound pricing? FIX-connected clients will not be quoted."}
              </p>
              <div className={styles.confirmButtons}>
                <Button variant="ghost" onClick={() => setConfirming(null)} disabled={pending}>
                  Cancel
                </Button>
                <button
                  type="button"
                  className={styles.confirmGo}
                  disabled={pending}
                  onClick={() => send(false, confirming === "all" ? false : true)}
                >
                  {pending ? "Halting…" : confirming === "all" ? "Confirm — stop all" : "Confirm — stop pricing"}
                </button>
              </div>
            </div>
          )}

          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
