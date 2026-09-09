/**
 * UnifiedMarketStudio — Studio 01: Markets & Feeds.
 *
 * Consolidates Volatility Surfaces (3D WebGL mesh + smile/term structure),
 * Multi-Curve Yield & Basis discounting (SOFR/EURIBOR dual curve + bootstrap),
 * and Inbound Feed Health / LP Liquidity monitoring into one seamless studio.
 */

import { useState } from "react";
import { SurfaceWorkspace } from "./SurfaceWorkspace";
import { CurveWorkspace } from "./CurveWorkspace";
import { LiquidityWorkspace } from "./LiquidityWorkspace";

export type MarketStudioTab = "surface" | "curves" | "feeds";

export function UnifiedMarketStudio({
  initialTab = "surface",
}: {
  initialTab?: MarketStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<MarketStudioTab>(initialTab);

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
            Desk 01
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Markets &amp; Volatility</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("surface")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "surface" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "surface" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "surface" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Vol Surfaces (3D)
          </button>
          <button
            type="button"
            onClick={() => setTab("curves")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "curves" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "curves" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "curves" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Yield Curves &amp; Basis
          </button>
          <button
            type="button"
            onClick={() => setTab("feeds")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "feeds" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "feeds" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "feeds" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Feed Health &amp; Liquidity
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "surface" && <SurfaceWorkspace />}
        {tab === "curves" && <CurveWorkspace />}
        {tab === "feeds" && <LiquidityWorkspace />}
      </div>
    </div>
  );
}
