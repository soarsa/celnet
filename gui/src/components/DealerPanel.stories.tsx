/**
 * Stories — DealerPanel (the ranked multi-dealer / RFQ-to-many quote panel).
 *
 * One row per responding LP from a `MultiDealerQuote` frame, rendered IN FRAME ORDER
 * (the server's deterministic audit order — the panel never re-sorts), with the touch
 * winners (`bestBidLpId` / `bestOfferLpId`) badged on their winning side. Clicking a
 * row's bid/offer books exactly that pinned dealer line by (quoteId, lpId); each row
 * depletes its OWN last-look ring against the line's `validUntilNanos`, and an expired
 * row is disabled with an honest reason. Pure and context-free — it takes a panel + an
 * onBook callback.
 *
 * The panels are built by a factory at render so each line's last-look deadline is
 * fresh (the ring depletes from `windowSeconds`); an intentionally past deadline
 * demonstrates the expired/disabled row.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { DealerPanel } from "./DealerPanel";
import { DEFAULT_CONVENTIONS } from "../data/seed";
import type { DealerQuote, MultiDealerQuote } from "../data/contract";

/** Nanoseconds since the Unix epoch, `secondsAhead` in the future (may be negative). */
function deadlineNanos(secondsAhead: number): bigint {
  const nowNanos = BigInt(Date.now()) * 1_000_000n;
  return nowNanos + BigInt(Math.round(secondsAhead * 1e9));
}

/** One dealer line at a given price with `secondsLeft` on its last-look window. */
function line(lpId: string, bid: number, offer: number, secondsLeft: number): DealerQuote {
  return {
    lpId,
    price: { bid, offer },
    resolvedStrike: 1.08,
    validUntilNanos: deadlineNanos(secondsLeft),
  };
}

/** A three-LP panel; `charlieSeconds` sets LP-CHARLIE's remaining last-look window. */
function makePanel(charlieSeconds: number): MultiDealerQuote {
  return {
    quoteId: 90210n,
    idempotencyKey: "story-rfq-1",
    // Frame order == render order (the server's deterministic audit order).
    dealers: [
      line("LP-ALPHA", 0.00842, 0.00891, 7.5),
      line("LP-BRAVO", 0.00838, 0.00885, 6.0), // best offer (lowest)
      line("LP-CHARLIE", 0.00845, 0.00898, charlieSeconds), // best bid (highest)
    ],
    bestBidLpId: "LP-CHARLIE",
    bestOfferLpId: "LP-BRAVO",
    conventions: DEFAULT_CONVENTIONS,
    epochNanos: BigInt(Date.now()) * 1_000_000n,
  };
}

const meta = {
  title: "Components/DealerPanel",
  component: DealerPanel,
  tags: ["autodocs"],
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "The ranked multi-dealer quote panel. One row per responding LP in the " +
          "server's audit order (never re-sorted); the touch winners are badged " +
          "('best') on their winning side. Each row depletes its own last-look ring " +
          "and disables on expiry with an honest reason; clicking a live bid/offer " +
          "books exactly that pinned dealer line.",
      },
    },
  },
} satisfies Meta<typeof DealerPanel>;

export default meta;

type Story = StoryObj<typeof DealerPanel>;

/**
 * Three live LP lines. Every window is in the future, so every row is tradable; the
 * best-bid (LP-CHARLIE) and best-offer (LP-BRAVO) lines carry their "best" badges on
 * the winning side. Clicking a bid or offer would book that line via onBook — here a
 * no-op. The last-look rings deplete toward each line's deadline.
 */
export const Default: Story = {
  render: () => <DealerPanel panel={makePanel(6.5)} onBook={() => {}} />,
};

/**
 * An expired line. LP-CHARLIE's last-look window is already in the past, so its book
 * buttons are disabled with the honest "window expired" reason while its ring reads
 * empty — the trader can never trade an expired window. The other two lines stay live.
 */
export const ExpiredLine: Story = {
  name: "Expired dealer line",
  render: () => <DealerPanel panel={makePanel(-1)} onBook={() => {}} />,
  parameters: {
    docs: {
      description: {
        story:
          "LP-CHARLIE's window is expired (validUntil in the past): its book buttons " +
          "are disabled with an explanatory reason and its ring is empty. Frame order " +
          "is preserved — an expired row is disabled, never removed or re-sorted.",
      },
    },
  },
};

/**
 * Capability-gated booking. `bookDisabled` disables every row's book button
 * regardless of expiry (the signed-in user lacks execute on this surface); the rows
 * stay VISIBLE with `bookDisabledTitle` explaining why — the panel discloses the
 * price without offering an action the identity can't take.
 */
export const BookDisabled: Story = {
  name: "Execute capability withheld",
  render: () => (
    <DealerPanel
      panel={makePanel(6.5)}
      onBook={() => {}}
      bookDisabled
      bookDisabledTitle="You lack the execute capability for this surface."
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          "With bookDisabled set, all book buttons are disabled irrespective of the " +
          "live windows; every row stays visible and carries the bookDisabledTitle " +
          "tooltip. Gating narrows the action, never hides the disclosed price.",
      },
    },
  },
};
