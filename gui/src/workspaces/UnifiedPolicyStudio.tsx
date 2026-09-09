/**
 * UnifiedPolicyStudio — Studio 06: Operations & Policy.
 *
 * Consolidates the institutional rule and operations governance:
 * 1. Visual Policy Engines (Inbound Acceptance Rules, Fill-to-Book Risk Routing Rules, Pricing Groups)
 * 2. High-Frequency Telemetry (Latency histograms p50/p99/p99.9, Event Trace lifecycles)
 * 3. Infrastructure & Session Management (FIX Sessions, Liquidity Feeds, Access Entitlements)
 */

import { useState } from "react";
import { RiskDashboardWorkspace } from "./RiskDashboardWorkspace";
import { PricingGroupsWorkspace } from "./PricingGroupsWorkspace";
import { LatencyOpsWorkspace } from "./analytics/LatencyOpsWorkspace";
import { EventTraceWorkspace } from "./analytics/EventTraceWorkspace";
import { ConnectionsWorkspace } from "./ConnectionsWorkspace";
import { AdminWorkspace } from "./AdminWorkspace";

export type PolicyStudioTab = "routing" | "acceptance" | "pricing_groups" | "latency" | "event_trace" | "fix_sessions" | "admin";

export function UnifiedPolicyStudio({
  initialTab = "routing",
}: {
  initialTab?: PolicyStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<PolicyStudioTab>(initialTab);

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
            Desk 06
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Operations &amp; Connectivity</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("routing")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "routing" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "routing" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "routing" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Risk Routing Rules
          </button>
          <button
            type="button"
            onClick={() => setTab("acceptance")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "acceptance" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "acceptance" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "acceptance" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Acceptance Rules
          </button>
          <button
            type="button"
            onClick={() => setTab("pricing_groups")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "pricing_groups" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "pricing_groups" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "pricing_groups" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Pricing Pipelines
          </button>
          <button
            type="button"
            onClick={() => setTab("latency")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "latency" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "latency" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "latency" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Latency (p99)
          </button>
          <button
            type="button"
            onClick={() => setTab("event_trace")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "event_trace" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "event_trace" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "event_trace" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Event Trace
          </button>
          <button
            type="button"
            onClick={() => setTab("fix_sessions")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "fix_sessions" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "fix_sessions" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "fix_sessions" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FIX Sessions
          </button>
          <button
            type="button"
            onClick={() => setTab("admin")}
            style={{
              padding: "4px 10px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "admin" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "admin" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "admin" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Admin &amp; Roles
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "routing" && <RiskDashboardWorkspace initialTab="routing" />}
        {tab === "acceptance" && <RiskDashboardWorkspace initialTab="acceptance" />}
        {tab === "pricing_groups" && <PricingGroupsWorkspace />}
        {tab === "latency" && <LatencyOpsWorkspace />}
        {tab === "event_trace" && <EventTraceWorkspace />}
        {tab === "fix_sessions" && <ConnectionsWorkspace />}
        {tab === "admin" && <AdminWorkspace />}
      </div>
    </div>
  );
}
