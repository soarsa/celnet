/**
 * Guided-tutorial (coach-mark) definitions + pure step helpers. A tour is an
 * ordered list of steps; each step spotlights a target DOM element (by CSS
 * selector) and shows a tooltip. The engine ({@link ../components/TourOverlay})
 * renders them; this module is pure data + navigation so the vitest suite can
 * exercise step advancement without a DOM.
 *
 * Targets are addressed by STABLE anchors — `data-tour-id` attributes we add to
 * the key UI elements, plus the existing `data-testid` anchors on the pricing-
 * group builder — so a tour never binds to an incidental class name. A step whose
 * target is not on the current screen degrades gracefully: the overlay centres the
 * tooltip and shows `offScreenHint` so the trader can perform the action, then
 * advance.
 */

import type { WorkspaceId } from "./commands";

/** The authored tours. */
export type TourId = "bid-offer-tiering" | "build-pricing-group" | "configure-tiering-feature";

/** Where the tooltip sits relative to its target. */
export type TourPlacement = "top" | "bottom" | "left" | "right" | "center";

/** One coach-mark step. */
export interface TourStep {
  /** CSS selector for the element to spotlight. Omitted ⇒ a centred, target-less step. */
  targetSelector?: string;
  /** The step heading. */
  title: string;
  /** The explanatory body. */
  body: string;
  /** Preferred tooltip placement relative to the target. */
  placement: TourPlacement;
  /** Shown (centred) when the target is not currently on screen. */
  offScreenHint?: string;
}

/** A scripted tour. */
export interface Tour {
  id: TourId;
  /** The tour's display title (announced on launch). */
  title: string;
  /** One line describing what it walks. */
  summary: string;
  /** The workspace the tour lives in — the engine navigates here on launch. */
  workspace?: WorkspaceId;
  steps: readonly TourStep[];
}

const TOUR_LIST: readonly Tour[] = [
  {
    id: "bid-offer-tiering",
    title: "How bid / offer tiering works",
    summary: "Walk the Tiering workspace: the strategy, the bps input, and the live preview.",
    workspace: "tiering",
    steps: [
      {
        title: "Bid / offer tiering",
        body: "Tiering constructs the two-way you stream to clients by widening around the LP mid. We'll walk a book's tiering: the strategy, its bps input, and the live preview turning 99.50 / 99.60 into 99.30 / 99.80.",
        placement: "center",
      },
      {
        targetSelector: '[data-tour-id="tiering-strategy"]',
        title: "The strategy",
        body: "Each tiering strategy is a rule for the spread. A Flat markup applies a constant half-spread H around the mid — the simplest, always-on margin.",
        placement: "right",
        offScreenHint:
          "Pick an aggregated book in the roster on the left so its tiering editor (and its strategy) appears, then continue.",
      },
      {
        targetSelector: '[data-tour-id="tiering-bps"]',
        title: "The half-spread input",
        body: "This is the margin. With the unit set to Price bps, 25 here means ±0.25 in price around the mid — your quoted half-spread.",
        placement: "right",
        offScreenHint: "Add a Flat markup strategy to reveal its half-spread input, then continue.",
      },
      {
        targetSelector: '[data-tour-id="tiering-preview"]',
        title: "The live preview",
        body: "The sample shows the outbound two-way: LP 99.50 / 99.60 (mid 99.55) becomes 99.30 / 99.80 with the ±25 bp margin. Change the half-spread and watch it move.",
        placement: "top",
        offScreenHint: "The sample preview appears once tiering is enabled with a strategy.",
      },
    ],
  },
  {
    id: "configure-tiering-feature",
    title: "Configure a tiering feature",
    summary: "The per-feature config: unit, strategy, half-spread, and guardrails.",
    workspace: "tiering",
    steps: [
      {
        title: "Configuring tiering",
        body: "Tiering has a spread unit, one or more strategies, and guardrails. We'll set each in turn.",
        placement: "center",
      },
      {
        targetSelector: '[data-tour-id="tiering-unit"]',
        title: "Spread unit",
        body: "Choose how the half-spread is expressed. Price bps is a fixed price offset (25 bp = 0.25); Yield bps is duration-consistent across the curve via the bond's DV01.",
        placement: "bottom",
        offScreenHint: "Select a book and enable tiering to reach the spread-unit control.",
      },
      {
        targetSelector: '[data-tour-id="tiering-strategy"]',
        title: "Add a strategy",
        body: "Flat markup for a constant margin; Scaled Smoothed Spread instead when the raw spread is jumpy; layer Inventory skew to lean the book against your position.",
        placement: "right",
        offScreenHint: "Add a strategy from the strategy buttons to configure it.",
      },
      {
        targetSelector: '[data-tour-id="tiering-bps"]',
        title: "Set the half-spread",
        body: "Enter the margin H in the chosen unit. The preview reflects it immediately.",
        placement: "right",
        offScreenHint: "A Flat markup or Inventory skew strategy exposes the half-spread input.",
      },
      {
        targetSelector: '[data-tour-id="tiering-guardrails"]',
        title: "Guardrails",
        body: "h_min / h_max clamp the half-spread, s_max caps the skew, and spread_floor keeps the book from crossing. Clamp skew last so extreme inventory can't invert the book.",
        placement: "top",
        offScreenHint: "The guardrails block appears below the strategies when tiering is enabled.",
      },
    ],
  },
  {
    id: "build-pricing-group",
    title: "Build a pricing group",
    summary: "Administration → Pricing Groups: create, drag a feature, configure, assign, preview.",
    workspace: "pricinggroups",
    steps: [
      {
        title: "Build a pricing group",
        body: "A pricing group binds clients to their own feature pipeline (RAW → features → OUTBOUND). We'll create one, drop a feature, and watch the live preview.",
        placement: "center",
      },
      {
        targetSelector: '[data-tour-id="pg-new"]',
        title: "Create a group",
        body: "Click “+ New” to start a group. Give it a code name (e.g. GROUP-A) in the name field.",
        placement: "right",
        offScreenHint: "Open Administration → Pricing Groups to see the group roster.",
      },
      {
        targetSelector: "#pg-custom",
        title: "Enable a custom pipeline",
        body: "Tick “Custom pipeline” so the feature palette and canvas appear. Off ⇒ the mode uses the book-default pricing.",
        placement: "bottom",
        offScreenHint: "Create a group first (+ New), then tick the custom-pipeline toggle.",
      },
      {
        targetSelector: '[data-tour-id="pg-palette"]',
        title: "The feature palette",
        body: "Drag a feature — MID SHIFT, TIERING, AXE, POSITION, or PANIC/SKEW — from here onto the canvas. New feature modules appear here automatically.",
        placement: "bottom",
        offScreenHint: "Enable the custom pipeline to reveal the palette.",
      },
      {
        targetSelector: '[data-testid="pipeline-canvas"]',
        title: "The pipeline canvas",
        body: "Drop features here and reorder them (drag, or the ↑ / ↓ buttons). The order IS the pipeline: each feature transforms the running two-way in turn. Click a card to configure it.",
        placement: "top",
        offScreenHint: "The canvas appears once a custom pipeline is enabled.",
      },
      {
        targetSelector: '[data-testid="preview-waterfall"]',
        title: "The live preview",
        body: "The waterfall shows the two-way after each feature — the provenance a client's fill will carry. Assign FIX connections / users as members and save.",
        placement: "top",
        offScreenHint: "Add at least one feature to populate the preview waterfall.",
      },
    ],
  },
];

/** The tours keyed by id. */
export const TOURS: Readonly<Record<TourId, Tour>> = Object.freeze(
  Object.fromEntries(TOUR_LIST.map((t) => [t.id, t])) as Record<TourId, Tour>,
);

/** All tours in authored order (the Help center's tour index). */
export const TOUR_INDEX: readonly Tour[] = TOUR_LIST;

/** Look up a tour by id. */
export function getTour(id: TourId): Tour | undefined {
  return TOURS[id];
}

// --- pure step helpers (the overlay + tests share these) ---------------------

/** Clamp a step index into a tour's valid range `[0, len − 1]`. */
export function clampStep(tour: Tour, index: number): number {
  const last = Math.max(0, tour.steps.length - 1);
  return Math.max(0, Math.min(last, index));
}

/** The next step index, or `null` when already on the last step (⇒ finish). */
export function nextStepIndex(tour: Tour, index: number): number | null {
  return index >= tour.steps.length - 1 ? null : index + 1;
}

/** The previous step index (clamped into range — Back on step 0 stays put). */
export function prevStepIndex(tour: Tour, index: number): number {
  return clampStep(tour, index - 1);
}

/** Whether `index` is the final step of `tour`. */
export function isLastStep(tour: Tour, index: number): boolean {
  return index >= tour.steps.length - 1;
}
