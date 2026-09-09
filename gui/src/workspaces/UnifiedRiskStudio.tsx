/**
 * UnifiedRiskStudio — Studio 05: Risk & Auto-Hedging.
 *
 * Institutional Risk Cockpit combining:
 * 1. Joint Multi-Asset Risk Grid (FX Greeks, Spot x Vol Scenarios, DV01 Ladders, and C2c VaR/ES)
 * 2. Hierarchical Portfolio Rollup (Firm -> Desk -> Book)
 * 3. Auto-Hedging Exit-Policy Decision Engine and Warehouse-Band Monitor
 */

import { useState } from "react";
import { RiskWorkspace } from "./RiskWorkspace";
import { RatesRiskWorkspace } from "./RatesRiskWorkspace";
import { CubeWorkspace } from "./CubeWorkspace";
import { HedgingWorkspace } from "./hedging/HedgingWorkspace";
import { RiskBooksWorkspace } from "./RiskBooksWorkspace";

export type RiskStudioTab = "joint_cube" | "fx_risk" | "rates_risk" | "auto_hedge" | "portfolios";

export function UnifiedRiskStudio({
  initialTab = "joint_cube",
}: {
  initialTab?: RiskStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<RiskStudioTab>(initialTab);

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
            Desk 05
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Risk &amp; Hedging Cockpit</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("joint_cube")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "joint_cube" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "joint_cube" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "joint_cube" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Joint Risk Cube &amp; Tail (C2c)
          </button>
          <button
            type="button"
            onClick={() => setTab("fx_risk")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "fx_risk" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "fx_risk" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "fx_risk" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FX Greeks &amp; Scenarios
          </button>
          <button
            type="button"
            onClick={() => setTab("rates_risk")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "rates_risk" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "rates_risk" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "rates_risk" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Rates DV01 &amp; Curve Shocks
          </button>
          <button
            type="button"
            onClick={() => setTab("auto_hedge")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "auto_hedge" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "auto_hedge" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "auto_hedge" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Auto-Hedging &amp; Warehousing
          </button>
          <button
            type="button"
            onClick={() => setTab("portfolios")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "portfolios" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "portfolios" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "portfolios" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Portfolio Hierarchy
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "joint_cube" && <CubeWorkspace />}
        {tab === "fx_risk" && <RiskWorkspace />}
        {tab === "rates_risk" && <RatesRiskWorkspace />}
        {tab === "auto_hedge" && <HedgingWorkspace />}
        {tab === "portfolios" && <RiskBooksWorkspace />}
      </div>
    </div>
  );
}
