/**
 * Guided-tutorial engine: the pure step reducer helpers, the TourOverlay's
 * Next/Back/Skip advancement (a controlled harness drives the step index), and the
 * authored tours' referential sanity (workspaces + target selectors).
 */
import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { TourOverlay } from "../src/components/TourOverlay";
import {
  clampStep,
  isLastStep,
  nextStepIndex,
  prevStepIndex,
  TOUR_INDEX,
  type Tour,
} from "../src/lib/tours";

const STUB: Tour = {
  id: "bid-offer-tiering",
  title: "Stub tour",
  summary: "three centred steps",
  steps: [
    { title: "Step one", body: "The first step body.", placement: "center" },
    { title: "Step two", body: "The second step body.", placement: "center" },
    { title: "Step three", body: "The third step body.", placement: "center" },
  ],
};

/** A controlled harness that owns the step index (as the TourProvider does). */
function Harness({ onFinish }: { onFinish: () => void }): React.ReactElement {
  const [step, setStep] = useState(0);
  return (
    <TourOverlay
      tour={STUB}
      stepIndex={step}
      onNext={() => {
        const n = nextStepIndex(STUB, step);
        if (n === null) onFinish();
        else setStep(n);
      }}
      onBack={() => setStep(prevStepIndex(STUB, step))}
      onSkip={onFinish}
    />
  );
}

describe("pure step helpers", () => {
  test("clampStep bounds the index into range", () => {
    expect(clampStep(STUB, -5)).toBe(0);
    expect(clampStep(STUB, 99)).toBe(2);
    expect(clampStep(STUB, 1)).toBe(1);
  });

  test("nextStepIndex returns null on the last step (⇒ finish)", () => {
    expect(nextStepIndex(STUB, 0)).toBe(1);
    expect(nextStepIndex(STUB, 2)).toBeNull();
  });

  test("prevStepIndex clamps Back on step 0", () => {
    expect(prevStepIndex(STUB, 0)).toBe(0);
    expect(prevStepIndex(STUB, 2)).toBe(1);
  });

  test("isLastStep flags the final step", () => {
    expect(isLastStep(STUB, 1)).toBe(false);
    expect(isLastStep(STUB, 2)).toBe(true);
  });
});

describe("TourOverlay advancement", () => {
  test("Next / Back page through the steps and the counter tracks", () => {
    render(<Harness onFinish={() => {}} />);
    expect(screen.getByText("Step one")).toBeInTheDocument();
    expect(screen.getByText("1 / 3")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("Step two")).toBeInTheDocument();
    expect(screen.getByText("2 / 3")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByText("Step one")).toBeInTheDocument();
  });

  test("Back is disabled on the first step", () => {
    render(<Harness onFinish={() => {}} />);
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  });

  test("the last step shows Done and finishes the tour", () => {
    const onFinish = vi.fn();
    render(<Harness onFinish={onFinish} />);
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    const done = screen.getByRole("button", { name: "Done" });
    expect(done).toBeInTheDocument();
    fireEvent.click(done);
    expect(onFinish).toHaveBeenCalledOnce();
  });

  test("Skip finishes the tour immediately", () => {
    const onFinish = vi.fn();
    render(<Harness onFinish={onFinish} />);
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(onFinish).toHaveBeenCalledOnce();
  });

  test("Escape skips the tour", () => {
    const onFinish = vi.fn();
    render(<Harness onFinish={onFinish} />);
    fireEvent.keyDown(screen.getByTestId("tour-tooltip"), { key: "Escape" });
    expect(onFinish).toHaveBeenCalledOnce();
  });

  test("announces the step through a live region", () => {
    render(<Harness onFinish={() => {}} />);
    const live = screen.getByText(/Step 1 of 3:/);
    expect(live).toHaveAttribute("aria-live", "assertive");
  });
});

describe("authored tours", () => {
  test("every tour has steps and a home workspace", () => {
    for (const t of TOUR_INDEX) {
      expect(t.steps.length, `${t.id} has no steps`).toBeGreaterThan(0);
      expect(t.workspace, `${t.id} has no workspace`).toBeDefined();
    }
  });

  test("the bid/offer tiering tour targets the strategy, bps and preview anchors", () => {
    const t = TOUR_INDEX.find((x) => x.id === "bid-offer-tiering");
    const selectors = t?.steps.map((s) => s.targetSelector).filter(Boolean) ?? [];
    expect(selectors).toContain('[data-tour-id="tiering-strategy"]');
    expect(selectors).toContain('[data-tour-id="tiering-bps"]');
    expect(selectors).toContain('[data-tour-id="tiering-preview"]');
  });

  test("the configure-hedging tour lives in the hedging workspace and anchors to real controls", () => {
    const t = TOUR_INDEX.find((x) => x.id === "configure-hedging");
    expect(t, "configure-hedging tour is registered").toBeDefined();
    expect(t!.workspace).toBe("hedging");
    // Five guide-mirroring anchored steps + a centred intro + a centred pointer to
    // Risk → Hedge flows (the live monitor moved out of the Hedging Rules surface).
    expect(t!.steps.length).toBe(7);
    const selectors = t!.steps.map((s) => s.targetSelector).filter(Boolean);
    // Each anchor is a real, present data-testid on the HedgingWorkspace (Hedging Rules)
    // surface (the tab buttons + threshold/policy/trace/execution-mode controls). The
    // live monitor is no longer here — it lives under Risk → Hedge flows, so the final
    // step is a centred pointer with no anchor.
    for (const sel of [
      '[data-testid="tab-thresholds"]',
      '[data-testid="threshold-cap"]',
      '[data-testid="hedge-create-rule"]',
      '[data-testid="hedge-trace"]',
      '[data-testid="tab-execution"]',
    ]) {
      expect(selectors, `missing anchor ${sel}`).toContain(sel);
    }
    // The monitor anchor was removed with the tab — assert it's gone so the tour can't
    // regress to targeting a control that no longer exists on this surface.
    expect(selectors).not.toContain('[data-testid="tab-monitor"]');
    expect(selectors).not.toContain('[data-testid="hedge-monitor"]');
    // Every non-intro (targeted) step off the default Policy tab carries an
    // off-screen hint so it degrades gracefully when its tab isn't active.
    for (const s of t!.steps) {
      if (s.placement !== "center" && s.targetSelector && !s.targetSelector.startsWith('[data-testid="tab-')) {
        expect(s.offScreenHint, `${s.title} needs an offScreenHint`).toBeTruthy();
      }
    }
  });
});
