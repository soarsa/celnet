/**
 * DefaultRoutePrompt — the startup guard popup. On app mount after sign-in, once the
 * routing graph + risk-portfolio roster have loaded, if the firm has NO valid default
 * routed portfolio (the routing graph's catch-all leaf is missing, unset, or points at
 * a disabled book — see {@link useDefaultRoutedBook}), it flashes a prominent, focus-
 * trapped `alertdialog` warning so risk from unmatched fills is never dropped into the
 * void. The panel pulses briefly to draw the eye, then settles (the global
 * `prefers-reduced-motion` rule zeroes the pulse — it just appears).
 *
 * Discipline:
 *   - Shows at most ONCE per browser session (a `sessionStorage` flag set on any
 *     dismissal), and NEVER while a valid default exists.
 *   - Re-evaluates LIVE: if the user sets a valid catch-all (the guided wizard Apply,
 *     the routing-editor Save), the hook flips to `ok` and the popup closes — and,
 *     being dismissed, it does not reappear.
 *   - Gated on the cap needed to FIX it (`risk_manage·fixed_income`): a user without it
 *     still SEES the firm-state warning, but the primary CTA is disabled with a note to
 *     ask an admin rather than leading to a forbidden surface.
 *
 * Accessibility: `role="alertdialog"`, labelled + described, focus moved in on open and
 * restored on close, Tab focus-trapped within the panel, Esc closes. Theme-aware via
 * design tokens (light / dark / high-contrast).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { useDefaultRoutedBook } from "../hooks/useDefaultRoutedBook";
import type { DefaultRouteResolution } from "../lib/routingGuard";
import { SetupWizard } from "../workspaces/hedging/SetupWizard/SetupWizard";
import styles from "./DefaultRoutePrompt.module.css";

/** Session flag: the guard has been shown+dismissed once this session (don't nag). */
const DISMISSED_KEY = "celnet:default-route-warn-dismissed";

function readDismissed(): boolean {
  try {
    return sessionStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}
function markDismissed(): void {
  try {
    sessionStorage.setItem(DISMISSED_KEY, "1");
  } catch {
    /* private-mode / storage-disabled — the in-memory flag still suppresses re-show */
  }
}

/** A reason-specific detail line under the shared lead copy (null ⇒ no extra line). */
function reasonDetail(res: DefaultRouteResolution | null): string | null {
  if (res === null) return null;
  switch (res.reason) {
    case "no-graph":
      return "No routing is configured yet, so no fill has a home portfolio.";
    case "no-default":
      return "The routing rules have no catch-all (Otherwise) rule, so unmatched fills fall off the end.";
    case "empty-book":
      return "The catch-all rule has no destination portfolio set.";
    case "book-not-found":
      return "The catch-all rule points at a portfolio that no longer exists.";
    case "book-disabled":
      return res.bookName !== null
        ? `The catch-all rule points at a disabled portfolio (“${res.bookName}”).`
        : "The catch-all rule points at a disabled portfolio.";
    default:
      return null;
  }
}

/** The focusable descendants of `root`, in DOM order (for the Tab focus-trap). */
function focusable(root: HTMLElement): HTMLElement[] {
  const sel =
    'a[href],button:not([disabled]),textarea,input,select,[tabindex]:not([tabindex="-1"])';
  return Array.from(root.querySelectorAll<HTMLElement>(sel)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}

export function DefaultRoutePrompt(): React.ReactElement | null {
  const app = useApp();
  const { status, resolution } = useDefaultRoutedBook();
  const canManageRisk = app.auth.can("risk_manage", "fixed_income");

  const [dismissed, setDismissed] = useState(readDismissed);
  const [wizardOpen, setWizardOpen] = useState(false);

  const panelRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // The warning is visible only when: warranted, not yet dismissed this session, and
  // the guided wizard is not itself open (it supersedes the prompt). `status` is
  // "warn" ONLY when the firm genuinely has no valid default.
  const open = status === "warn" && !dismissed && !wizardOpen;

  const close = useCallback((): void => {
    markDismissed();
    setDismissed(true);
  }, []);

  const openWizard = useCallback((): void => {
    // Opening the guided flow consumes this session's single show; on Apply the hook
    // flips to "ok" and the prompt stays closed.
    markDismissed();
    setDismissed(true);
    setWizardOpen(true);
  }, []);

  const goToRouting = useCallback((): void => {
    markDismissed();
    setDismissed(true);
    app.setWorkspace("riskrouting");
  }, [app]);

  // Esc closes; Tab is trapped within the panel while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        return;
      }
      if (e.key !== "Tab") return;
      const panel = panelRef.current;
      if (panel === null) return;
      const items = focusable(panel);
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0] as HTMLElement;
      const last = items[items.length - 1] as HTMLElement;
      const active = document.activeElement;
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, close]);

  // Move focus into the panel on open (to the primary action) and restore it on close.
  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const target = panel?.querySelector<HTMLElement>("[data-autofocus]") ?? panel;
    target?.focus();
    return () => {
      restoreFocusRef.current?.focus?.();
    };
  }, [open]);

  const detail = useMemo(() => reasonDetail(resolution), [resolution]);
  const titleId = "default-route-prompt-title";
  const descId = "default-route-prompt-desc";

  return (
    <>
      {open && (
        <div className={styles.scrim} data-testid="default-route-prompt">
          <div
            ref={panelRef}
            className={styles.panel}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby={titleId}
            aria-describedby={descId}
            tabIndex={-1}
          >
            <button
              type="button"
              className={styles.closeBtn}
              aria-label="Dismiss warning"
              data-testid="default-route-close"
              onClick={close}
            >
              ✕
            </button>

            <div className={styles.head}>
              <span className={styles.badge} aria-hidden="true">
                ⚠
              </span>
              <div className={styles.headText}>
                <h2 id={titleId} className={styles.title}>
                  No default risk portfolio
                </h2>
                <p id={descId} className={styles.desc}>
                  Some fills may not be booked into a risk book. Set a catch-all portfolio so all
                  risk is routed.
                </p>
                {detail !== null && <p className={styles.detail}>{detail}</p>}
              </div>
            </div>

            {!canManageRisk && (
              <p className={styles.capNote} role="note" data-testid="default-route-cap-note">
                You don&apos;t hold Manage-Risk, so you can&apos;t fix this yourself — ask an
                administrator to set a default routed portfolio.
              </p>
            )}

            <div className={styles.foot}>
              <button
                type="button"
                className={styles.linkBtn}
                data-testid="default-route-routing"
                onClick={goToRouting}
              >
                Risk → Routing
              </button>
              <div className={styles.footActions}>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  data-testid="default-route-later"
                  data-autofocus={canManageRisk ? undefined : ""}
                  onClick={close}
                >
                  Later
                </button>
                <button
                  type="button"
                  className={styles.primaryBtn}
                  data-testid="default-route-guided"
                  data-autofocus={canManageRisk ? "" : undefined}
                  disabled={!canManageRisk}
                  title={
                    canManageRisk
                      ? undefined
                      : "Requires the Manage-Risk capability — ask an administrator."
                  }
                  onClick={openWizard}
                >
                  Guided setup
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
      {wizardOpen && <SetupWizard onClose={() => setWizardOpen(false)} />}
    </>
  );
}
