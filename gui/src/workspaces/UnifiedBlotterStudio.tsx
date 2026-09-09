/**
 * UnifiedBlotterStudio — Studio 04: Ledger & Blotter.
 *
 * The single firm-wide ledger consolidating Positions, Executed Trade Blotters,
 * Inbound Quotes, Risk Transfers, and Street Hedge Fills into one high-performance
 * virtualized grid.
 */

import { useState } from "react";
import { BookWorkspace } from "./BookWorkspace";
import { RiskDashboardWorkspace } from "./RiskDashboardWorkspace";
import { DealsBlotterWorkspace } from "./DealsBlotterWorkspace";
import { RiskTransferWorkspace } from "./risktransfer/RiskTransferWorkspace";
import { HedgeFlowWorkspace } from "./HedgeFlowWorkspace";

export type BlotterStudioTab = "positions_fx" | "ledgers_fi" | "deals" | "transfer" | "hedge_fills";

export function UnifiedBlotterStudio({
  initialTab = "positions_fx",
}: {
  initialTab?: BlotterStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<BlotterStudioTab>(initialTab);

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
            Desk 04
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Blotters &amp; Position Ledger</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("positions_fx")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "positions_fx" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "positions_fx" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "positions_fx" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FX Positions &amp; Book
          </button>
          <button
            type="button"
            onClick={() => setTab("ledgers_fi")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "ledgers_fi" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "ledgers_fi" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "ledgers_fi" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FI Ledgers &amp; Blotters
          </button>
          <button
            type="button"
            onClick={() => setTab("deals")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "deals" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "deals" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "deals" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Executed Deals Blotter
          </button>
          <button
            type="button"
            onClick={() => setTab("transfer")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "transfer" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "transfer" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "transfer" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Risk Transfers &amp; Audit
          </button>
          <button
            type="button"
            onClick={() => setTab("hedge_fills")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "hedge_fills" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "hedge_fills" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "hedge_fills" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Hedge Fills &amp; Street Orders
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "positions_fx" && <BookWorkspace />}
        {tab === "ledgers_fi" && <RiskDashboardWorkspace variant="ledgers" />}
        {tab === "deals" && <DealsBlotterWorkspace />}
        {tab === "transfer" && <RiskTransferWorkspace />}
        {tab === "hedge_fills" && <HedgeFlowWorkspace />}
      </div>
    </div>
  );
}
