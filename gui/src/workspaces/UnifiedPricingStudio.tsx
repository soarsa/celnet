/**
 * UnifiedPricingStudio — Studio 02: Pricing & Structuring.
 *
 * The definitive sell-side structuring workbench uniting Options & Exotics
 * (TicketWorkspace with 23 exotic structures, 13 Greeks, payoff diagrams),
 * Valuation Adjustments (XVA exposure profiles & netting sets), and Bond
 * Corporate Actions / Schedules into a single, high-density studio.
 */

import { useState } from "react";
import { TicketWorkspace } from "./TicketWorkspace";
import { XvaWorkspace } from "./XvaWorkspace";
import { CorporateActionsWorkspace } from "./CorporateActionsWorkspace";

export type PricingStudioTab = "ticket" | "xva" | "corpactions";

export function UnifiedPricingStudio({
  initialTab = "ticket",
}: {
  initialTab?: PricingStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<PricingStudioTab>(initialTab);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", width: "100%" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "8px 16px",
          background: "var(--bg-surface, #0d111a)",
          borderBottom: "1px solid var(--border-subtle, rgba(255,255,255,0.08))",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            style={{
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 11,
              textTransform: "uppercase",
              color: "var(--brand-coral, #ff7357)",
              fontWeight: 700,
            }}
          >
            Desk 02
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Pricing &amp; Structuring</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("ticket")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "ticket" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "ticket" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "ticket" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Cross-Asset Ticket &amp; Greeks
          </button>
          <button
            type="button"
            onClick={() => setTab("xva")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "xva" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "xva" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "xva" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            XVA &amp; Exposure Fans
          </button>
          <button
            type="button"
            onClick={() => setTab("corpactions")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "corpactions" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "corpactions" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "corpactions" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Corporate Actions
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "ticket" && <TicketWorkspace />}
        {tab === "xva" && <XvaWorkspace />}
        {tab === "corpactions" && <CorporateActionsWorkspace />}
      </div>
    </div>
  );
}
