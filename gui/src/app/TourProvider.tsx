/**
 * TourProvider — the app-level launcher + host for the guided tutorials. It exposes
 * `startTour(id)` through context (consumed by the Help center and every in-editor
 * "?" help panel / "Show me" affordance), navigates to the tour's workspace, and
 * renders the {@link TourOverlay} on top of the whole shell while a tour runs.
 *
 * `useTour()` returns a SAFE no-op when there is no provider above it, so isolated
 * component tests (which render an editor without the shell) never crash on the
 * "?" help button's tour launcher.
 */

import { createContext, useCallback, useContext, useMemo, useState } from "react";

import { useApp } from "./AppContext";
import { TourOverlay } from "../components/TourOverlay";
import { workspaceDomains } from "../lib/commands";
import {
  getTour,
  isLastStep,
  nextStepIndex,
  prevStepIndex,
  type TourId,
} from "../lib/tours";

interface TourContextValue {
  /** Launch a tour by id (navigates to its workspace first). */
  startTour: (id: TourId) => void;
  /** The running tour's id, or null. */
  activeTourId: TourId | null;
}

const TourContext = createContext<TourContextValue>({
  startTour: () => {},
  activeTourId: null,
});

/** Access the tour launcher (safe no-op outside a TourProvider). */
export function useTour(): TourContextValue {
  return useContext(TourContext);
}

export function TourProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  const app = useApp();
  const [activeTourId, setActiveTourId] = useState<TourId | null>(null);
  const [stepIndex, setStepIndex] = useState(0);

  const tour = activeTourId ? getTour(activeTourId) : undefined;

  const startTour = useCallback(
    (id: TourId): void => {
      const t = getTour(id);
      if (!t) return;
      // Navigate to the tour's home workspace so its targets are on screen. The
      // overlay polls for each target, so the async workspace/data mount is fine.
      if (t.workspace) {
        const doms = workspaceDomains(t.workspace);
        if (doms[0]) app.setActiveDomain(doms[0]);
        app.setWorkspace(t.workspace);
      }
      setActiveTourId(id);
      setStepIndex(0);
    },
    [app],
  );

  const close = useCallback((): void => {
    setActiveTourId(null);
    setStepIndex(0);
  }, []);

  const next = useCallback((): void => {
    if (!tour) return;
    if (isLastStep(tour, stepIndex)) {
      close();
      return;
    }
    const n = nextStepIndex(tour, stepIndex);
    if (n !== null) setStepIndex(n);
  }, [tour, stepIndex, close]);

  const back = useCallback((): void => {
    if (!tour) return;
    setStepIndex((i) => prevStepIndex(tour, i));
  }, [tour]);

  const value = useMemo<TourContextValue>(() => ({ startTour, activeTourId }), [startTour, activeTourId]);

  return (
    <TourContext.Provider value={value}>
      {children}
      {tour && (
        <TourOverlay
          tour={tour}
          stepIndex={stepIndex}
          onNext={next}
          onBack={back}
          onSkip={close}
        />
      )}
    </TourContext.Provider>
  );
}
