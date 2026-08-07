/**
 * WizardShell — the SHARED chrome of a guided-setup wizard: the modal scrim + panel,
 * the 🪄 header, the capability banner slot, the keyboard-navigable stepper, the
 * footer (Back / Next / Apply, per-step issues, and the failed-apply recovery), and the
 * live Apply-progress panel. Both the Hedging and Risk guided setups render their own
 * per-step bodies as {@link WizardShellProps.children} inside this one shell, so the two
 * wizards are pixel-identical everywhere except their step CONTENT.
 *
 * The shell is purely presentational + a11y plumbing: the HOST owns all draft state,
 * the step index, the per-step errors and the Apply sequence. The shell only reflects
 * them and calls back (`onStep`, `onBack`, `onNext`, `onApply`, `onEditAgain`, `onClose`).
 *
 * Accessibility: `role="dialog"` + `aria-modal`, a labelled title, a stepper of
 * `aria-current="step"` buttons, Esc to close (unless mid-apply), and focus moved to the
 * active step body on each step change.
 */
import { useEffect, useId, useRef } from "react";

import type { ApplyStepState } from "./wizardModel";
import styles from "./setupWizard.module.css";

/** One stepper entry — its long title (stepper + step-body aria-label) and short chip. */
export interface StepMeta {
  title: string;
  short: string;
}

/** The apply lifecycle the host drives; the shell renders the progress panel from it. */
export type ApplyPhase =
  | { kind: "idle" }
  | { kind: "applying"; steps: ApplyStepState[] }
  | { kind: "failed"; steps: ApplyStepState[] };

export interface WizardShellProps {
  /** Dialog title (e.g. "Guided setup"). */
  title: string;
  /** One-line subtitle under the title. */
  subtitle: string;
  /** The stepper entries, in order. */
  steps: readonly StepMeta[];
  /** The active step index. */
  step: number;
  /** Jump to a step (stepper click). */
  onStep: (i: number) => void;
  /** Whether step `i` is read-only (uncapped) — shown as a "read-only" chip. */
  stepLocked: (i: number) => boolean;
  /** Whether step `i` is completed (drawn with a ✓). */
  stepComplete: (i: number) => boolean;
  /** The blocking issues for the ACTIVE step (footer list; disables Next). */
  currentErrors: string[];
  /** Optional capability banner content, rendered under the header when present. */
  capBanner: React.ReactNode | null;
  /** The apply lifecycle. */
  phase: ApplyPhase;
  /** Close / discard. */
  onClose: () => void;
  /** Step back. */
  onBack: () => void;
  /** Step forward. */
  onNext: () => void;
  /** Commit (only shown on the last step). */
  onApply: () => void;
  /** Return from a failed apply to editing. */
  onEditAgain: () => void;
  /** The primary-button label on the last step (e.g. "Apply setup"). */
  applyLabel: string;
  /** The active step body. */
  children: React.ReactNode;
}

export function WizardShell({
  title,
  subtitle,
  steps,
  step,
  onStep,
  stepLocked,
  stepComplete,
  currentErrors,
  capBanner,
  phase,
  onClose,
  onBack,
  onNext,
  onApply,
  onEditAgain,
  applyLabel,
  children,
}: WizardShellProps): React.ReactElement {
  const titleId = useId();
  const bodyRef = useRef<HTMLDivElement>(null);
  const applying = phase.kind === "applying";
  const isLast = step === steps.length - 1;

  // Esc closes (unless mid-apply).
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && !applying) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, applying]);

  // Move focus to the step body on each step change.
  useEffect(() => {
    bodyRef.current?.focus();
  }, [step]);

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !applying) onClose();
      }}
    >
      <div className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <div className={styles.headMain}>
            <span className={styles.wand} aria-hidden="true">
              🪄
            </span>
            <div>
              <h2 id={titleId} className={styles.title}>
                {title}
              </h2>
              <p className={styles.subtitle}>{subtitle}</p>
            </div>
          </div>
          <button
            type="button"
            className={styles.closeBtn}
            data-testid="wiz-close"
            aria-label={`Close ${title.toLowerCase()}`}
            disabled={applying}
            onClick={onClose}
          >
            ✕
          </button>
        </div>

        {capBanner !== null && (
          <p className={styles.capBanner} role="note" data-testid="wiz-cap-banner">
            {capBanner}
          </p>
        )}

        <div className={styles.layout}>
          <nav className={styles.stepper} aria-label="Setup steps">
            <ol className={styles.stepperList}>
              {steps.map((m, i) => {
                const state = i === step ? "active" : stepComplete(i) ? "complete" : "upcoming";
                return (
                  <li key={m.title} className={styles.stepperItem}>
                    <button
                      type="button"
                      className={`${styles.stepNode} ${styles[`stepNode_${state}` as const] ?? ""}`}
                      aria-current={i === step ? "step" : undefined}
                      data-testid={`wiz-step-${i}`}
                      onClick={() => !applying && onStep(i)}
                      disabled={applying}
                    >
                      <span className={styles.stepDot} aria-hidden="true">
                        {state === "complete" ? "✓" : i + 1}
                      </span>
                      <span className={styles.stepLabels}>
                        <span className={styles.stepTitle}>{m.title}</span>
                        {stepLocked(i) && <span className={styles.stepLocked}>read-only</span>}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ol>
          </nav>

          <div className={styles.content}>
            {phase.kind === "idle" ? (
              <div className={styles.stepScroll} ref={bodyRef} tabIndex={-1} aria-label={steps[step]?.title}>
                {children}
              </div>
            ) : (
              <ApplyPanel steps={phase.steps} failed={phase.kind === "failed"} />
            )}
          </div>
        </div>

        <div className={styles.foot}>
          {phase.kind === "failed" ? (
            <>
              <p className={styles.footError} role="alert" data-testid="wiz-apply-error">
                Apply stopped — nothing further was written. Fix the issue and try again.
              </p>
              <div className={styles.footActions}>
                <button type="button" className={styles.ghostBtn} onClick={onEditAgain}>
                  Back to edit
                </button>
                <button type="button" className={styles.ghostBtn} onClick={onClose}>
                  Close
                </button>
              </div>
            </>
          ) : (
            <>
              {currentErrors.length > 0 && phase.kind === "idle" && (
                <ul className={styles.footIssues} data-testid="wiz-step-errors">
                  {currentErrors.slice(0, 3).map((msg, i) => (
                    <li key={i}>⚠ {msg}</li>
                  ))}
                </ul>
              )}
              <div className={styles.footActions}>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  onClick={onBack}
                  disabled={step === 0 || applying}
                  data-testid="wiz-back"
                >
                  Back
                </button>
                {isLast ? (
                  <button
                    type="button"
                    className={styles.primaryBtn}
                    onClick={onApply}
                    disabled={applying}
                    data-testid="wiz-apply"
                  >
                    {applying ? "Applying…" : applyLabel}
                  </button>
                ) : (
                  <button
                    type="button"
                    className={styles.primaryBtn}
                    onClick={onNext}
                    disabled={currentErrors.length > 0}
                    data-testid="wiz-next"
                  >
                    Next
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** The Apply progress list — one row per dependency-ordered persistence step. */
function ApplyPanel({ steps, failed }: { steps: ApplyStepState[]; failed: boolean }): React.ReactElement {
  return (
    <div className={styles.stepScroll} aria-live="polite">
      <h3 className={styles.applyHeading}>{failed ? "Apply failed" : "Applying your setup…"}</h3>
      <ol className={styles.applyList} data-testid="wiz-apply-progress">
        {steps.map((s) => (
          <li
            key={s.id}
            className={`${styles.applyRow} ${styles[`apply_${s.status}` as const] ?? ""}`}
            data-testid={`wiz-apply-${s.id}`}
          >
            <span className={styles.applyIcon} aria-hidden="true">
              {s.status === "done"
                ? "✓"
                : s.status === "error"
                  ? "✕"
                  : s.status === "running"
                    ? "…"
                    : s.status === "skipped"
                      ? "–"
                      : "○"}
            </span>
            <span className={styles.applyLabel}>{s.label}</span>
            <span className={styles.applyDetail}>{s.detail}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
