/**
 * TourOverlay — the lightweight, self-contained coach-mark / spotlight engine
 * (NO tour-library dependency, CSP-safe). Given a scripted {@link Tour} and the
 * current step, it locates the step's target element, spotlights it (a dimmed
 * cut-out + ring), and floats an accessible tooltip with Back / Next / Skip.
 *
 * Design choices for a trading GUI:
 *  - The dim layer is `pointer-events: none` so the app stays INTERACTIVE during a
 *    tour — the "Build a pricing group" tour needs the trader to actually click
 *    "+ New", tick the toggle, and drag features. Only the tooltip captures events.
 *  - A target not on the current screen degrades gracefully: the tooltip centres
 *    and shows the step's `offScreenHint` so the trader can perform the action,
 *    then advance — no dead spotlight.
 *  - Fully keyboard-operable + screen-reader friendly: focus moves to the tooltip
 *    on each step, ←/→ page Back/Next, Esc skips, and a live region announces the
 *    step ("Step 2 of 4: …").
 *
 * Presentational + controlled — the {@link ../app/TourProvider} owns the step index.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { isLastStep, type Tour, type TourPlacement } from "../lib/tours";
import styles from "./TourOverlay.module.css";

export interface TourOverlayProps {
  tour: Tour;
  stepIndex: number;
  onNext: () => void;
  onBack: () => void;
  onSkip: () => void;
}

/** A viewport-space box for the spotlight. */
interface Box {
  top: number;
  left: number;
  width: number;
  height: number;
}

/** Find the FIRST on-screen element matching `selector` (skips hidden/inert panes). */
function findVisibleTarget(selector: string): HTMLElement | null {
  const els = Array.from(document.querySelectorAll<HTMLElement>(selector));
  for (const el of els) {
    const r = el.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) return el;
  }
  return null;
}

/**
 * The tooltip's top-left in viewport space from the target box, the preferred
 * placement, and the tooltip's MEASURED size — clamped so it is always fully on
 * screen (a target at the far edge won't push Next off the viewport).
 */
function tooltipPosition(
  box: Box | null,
  placement: TourPlacement,
  tipW: number,
  tipH: number,
): { top: number; left: number } {
  const vw = typeof window !== "undefined" ? window.innerWidth : 1440;
  const vh = typeof window !== "undefined" ? window.innerHeight : 900;
  const margin = 8;
  const gap = 14;
  const clamp = (v: number, max: number): number => Math.max(margin, Math.min(max - margin, v));

  if (!box || placement === "center") {
    return { top: (vh - tipH) / 2, left: (vw - tipW) / 2 };
  }
  const cxLeft = box.left + box.width / 2 - tipW / 2;
  const cyTop = box.top + box.height / 2 - tipH / 2;
  let top: number;
  let left: number;
  switch (placement) {
    case "top":
      top = box.top - tipH - gap;
      left = cxLeft;
      break;
    case "bottom":
      top = box.top + box.height + gap;
      left = cxLeft;
      break;
    case "left":
      top = cyTop;
      left = box.left - tipW - gap;
      break;
    case "right":
    default:
      top = cyTop;
      left = box.left + box.width + gap;
      break;
  }
  return { top: clamp(top, vh - tipH), left: clamp(left, vw - tipW) };
}

export function TourOverlay({
  tour,
  stepIndex,
  onNext,
  onBack,
  onSkip,
}: TourOverlayProps): React.ReactElement {
  const step = tour.steps[stepIndex];
  const [box, setBox] = useState<Box | null>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);

  // Locate the target — polling briefly so a target that appears after navigation
  // (a workspace mount, an async book load) is picked up; then track scroll/resize.
  useEffect(() => {
    if (!step?.targetSelector) {
      setBox(null);
      return;
    }
    let raf = 0;
    let tries = 0;
    let cancelled = false;
    const measure = (el: HTMLElement): void => {
      const r = el.getBoundingClientRect();
      setBox({ top: r.top, left: r.left, width: r.width, height: r.height });
    };
    const locate = (): void => {
      if (cancelled) return;
      const el = findVisibleTarget(step.targetSelector as string);
      if (el) {
        el.scrollIntoView({ block: "center", inline: "center" });
        measure(el);
        return;
      }
      setBox(null);
      tries += 1;
      if (tries < 150) raf = requestAnimationFrame(locate); // ~2.5s at 60fps
    };
    locate();
    const onMove = (): void => {
      const el = findVisibleTarget(step.targetSelector as string);
      if (el) measure(el);
    };
    window.addEventListener("resize", onMove);
    window.addEventListener("scroll", onMove, true);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", onMove);
      window.removeEventListener("scroll", onMove, true);
    };
  }, [step]);

  // Position the tooltip from the MEASURED size + the target box, clamped on screen.
  useLayoutEffect(() => {
    const el = tipRef.current;
    const w = el?.offsetWidth || 340;
    const h = el?.offsetHeight || 200;
    setPos(tooltipPosition(box, step?.placement ?? "center", w, h));
  }, [box, step, stepIndex]);

  // Move focus into the tooltip on each step so the keyboard user lands here.
  useEffect(() => {
    const t = requestAnimationFrame(() => tipRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, [stepIndex]);

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      onSkip();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      onNext();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      onBack();
    }
  };

  const last = isLastStep(tour, stepIndex);
  const tipStyle: React.CSSProperties = pos
    ? { top: pos.top, left: pos.left }
    : { top: "50%", left: "50%", transform: "translate(-50%, -50%)" };
  const showHint = !box && step?.targetSelector !== undefined && step.offScreenHint !== undefined;

  return createPortal(
    <div className={styles.layer} data-tour-active={tour.id}>
      {box && (
        <div
          className={styles.spot}
          style={{ top: box.top, left: box.left, width: box.width, height: box.height }}
          aria-hidden="true"
        />
      )}

      <div
        ref={tipRef}
        className={styles.tip}
        style={tipStyle}
        role="dialog"
        aria-modal="false"
        aria-labelledby="tour-step-title"
        tabIndex={-1}
        onKeyDown={onKeyDown}
        data-testid="tour-tooltip"
      >
        <div className={styles.tipHead}>
          <span className={styles.tourName}>{tour.title}</span>
          <span className={styles.counter}>
            {stepIndex + 1} / {tour.steps.length}
          </span>
        </div>
        <h3 id="tour-step-title" className={styles.tipTitle}>
          {step?.title}
        </h3>
        <p className={styles.tipBody}>{step?.body}</p>
        {showHint && <p className={styles.hint}>{step?.offScreenHint}</p>}
        <div className={styles.tipFoot}>
          <button type="button" className={styles.skip} onClick={onSkip}>
            Skip
          </button>
          <div className={styles.nav}>
            <button
              type="button"
              className={styles.back}
              onClick={onBack}
              disabled={stepIndex === 0}
            >
              Back
            </button>
            <button type="button" className={styles.nextBtn} onClick={onNext}>
              {last ? "Done" : "Next"}
            </button>
          </div>
        </div>
      </div>

      <div className={styles.srOnly} role="status" aria-live="assertive">
        Step {stepIndex + 1} of {tour.steps.length}: {step?.title}
      </div>
    </div>,
    document.body,
  );
}
