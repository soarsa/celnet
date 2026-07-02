/**
 * Stories — BookWorkspace (GUI-DESIGN §4.5 / BOOK-RISK).
 *
 * The firm-scale HIERARCHICAL risk view, computed SERVER-SIDE. Aggregation is owned
 * by the server: this view issues ONE `aggregate_risk` call for the rolled-up node
 * tree over the org dimension the active Scope selects, plus a `drill_risk` for the
 * Book→Risk drill — never a client-side position-sum loop.
 *
 * Every measure is collapsed into a single common reporting numeraire (USD) via
 * `celnet-risk-normalize`, so cross-pair totals are directly comparable. The Scope
 * toolbar drives the group-by dimension (CCY_PAIR / TRADER / DESK / …); the Limits
 * panel surfaces `celnet-limits` utilization/RAG for the scope. Clicking a node row
 * drills into the largest contributing position in Risk (Book and Risk are the same
 * cube at two zooms).
 *
 * Token contract: positive/negative values use var(--bid)/var(--offer) via the
 * `.pos`/`.neg` semantic classes; RAG dots use var(--rag-green)/--amber/--red/--breach.
 * The limit bar fill uses OKLCH tinting from the same variable. No inline hex.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { BookWorkspace } from "./BookWorkspace";

const meta = {
  title: "Workspaces/BookWorkspace",
  component: BookWorkspace,
  // Inlined (untyped) so the meta infers the component's optional-arg type — a
  // top-level `Decorator` const clashes with StrictArgs under
  // exactOptionalPropertyTypes now that BookWorkspace takes `{ initialLens? }`
  // (same fix the #1 RiskWorkspace fold used).
  decorators: [
    (Story) => (
      <AppProvider transport={createMockTransport()}>
        <Story />
      </AppProvider>
    ),
  ],
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The firm-scale hierarchical risk view (BOOK-RISK). Aggregation is server-side: " +
          "one aggregate_risk call over the org dimension the Scope selects. Every measure " +
          "is in a common reporting numeraire (USD). The Limits panel shows celnet-limits " +
          "utilization/RAG. Clicking a node row drills to its largest contributing position " +
          "in RiskWorkspace (Book and Risk are the same cube at two zooms).",
      },
    },
  },
} satisfies Meta<typeof BookWorkspace>;

export default meta;

type Story = StoryObj<typeof BookWorkspace>;

/**
 * Book at firm scope — the default view. The mock transport seeds a small book
 * (a few positions across EUR/USD, GBP/USD, USD/JPY), aggregates them server-side,
 * and the view renders the rolled-up node tree with summary cards (Net P&L, Net Δ,
 * Net Vega, Net Theta) all in USD. The aggregate vega ladder and limits panel appear
 * in the right column. Click a node row to drill to its largest position in Risk.
 */
export const Default: Story = {};

/**
 * Firm-level summary cards — the four additive risk measures (Net P&L / Net Δ /
 * Net Vega / Net Theta in reporting numeraire USD) rendered at the top of the view.
 * Positive values receive the var(--bid) green tint; negative receive var(--offer)
 * red — semantic color, never coral (brand only).
 *
 * The aggregate vega ladder in the right column shows the firm's bucketed vega
 * exposure by (tenor, delta) pillar; bar widths are proportional to |vega| / max.
 */
export const FirmSummaryCards: Story = {
  name: "Firm summary cards + aggregate vega ladder",
  parameters: {
    docs: {
      description: {
        story:
          "The four additive-risk summary cards and the aggregate vega ladder panel. " +
          "Color is semantic (bid/offer green/red via CSS variables); the ladder bar " +
          "fills use OKLCH tinting from var(--bid)/var(--offer) at 0.4 alpha.",
      },
    },
  },
};

/**
 * Node breakdown table — the rolled-up CCY_PAIR dimension (the default scope
 * grouping). Each row is one currency pair's net exposure; clicking a row fires
 * `drill_risk` to find the largest contributing flat position and opens it in
 * RiskWorkspace. The `›` glyph cues drill-ability.
 *
 * The table footer row shows the firm-level total (sum of all node additive
 * measures — additive roll-up is valid because the server already collapsed every
 * leg into the common reporting numeraire).
 */
export const NodeBreakdownTable: Story = {
  name: "Node breakdown table (CCY_PAIR grouping)",
  parameters: {
    docs: {
      description: {
        story:
          "The breakdown table groups positions by CCY_PAIR (the Scope's default). " +
          "Each row shows the server-aggregated net Δ / Vega / Gamma / Theta for that " +
          "pair's book. Clicking drills via drill_risk to the largest contributing " +
          "position; the footer is the firm-level additive total.",
      },
    },
  },
};

/**
 * Limits RAG panel — the celnet-limits utilization view for the active scope.
 * Each metric (vega bucket, tenor vega, delta concentration, stop-loss, ES) is
 * shown with a utilization bar (fill width = ratio × 100%) and a RAG dot
 * (GREEN / AMBER / RED / BREACH). A hard breach triggers the HARD BREACH banner.
 *
 * The mock transport returns a populated limits set for the demo book; the honest
 * empty-state ("No limits configured for this scope") renders when the set is empty.
 */
export const LimitsRagPanel: Story = {
  name: "Limits RAG panel (celnet-limits)",
  parameters: {
    docs: {
      description: {
        story:
          "The Limits panel surfaces celnet-limits utilization for the active scope. " +
          "Bar fill = ratio × 100%; dot color = GREEN/AMBER/RED/BREACH from " +
          "var(--rag-green)/var(--rag-amber)/var(--rag-red)/var(--rag-breach). " +
          "A HARD BREACH badge appears when limits.hardBreach is true.",
      },
    },
  },
};
