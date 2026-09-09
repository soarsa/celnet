/**
 * UnifiedDistributionStudio — Studio 03: Distribution & RFQ.
 *
 * Consolidates Outbound Price Streaming (options & FI), Inbound LP Book Aggregation,
 * Client-tier margin/spread skews, and Sales-Trader RFQ triage into a single
 * market-making distribution studio.
 */

import { useState } from "react";
import { StreamWorkspace } from "./StreamWorkspace";
import { FiStreamingWorkspace } from "./FiStreamingWorkspace";
import { AggregatedBookWorkspace } from "./AggregatedBookWorkspace";
import { QuotingWorkspace } from "./QuotingWorkspace";
import { PricingGroupsWorkspace } from "./PricingGroupsWorkspace";

export type DistributionStudioTab = "rfq" | "stream_fx" | "stream_fi" | "aggbook" | "tiering";

export function UnifiedDistributionStudio({
  initialTab = "rfq",
}: {
  initialTab?: DistributionStudioTab;
}): React.ReactElement {
  const [tab, setTab] = useState<DistributionStudioTab>(initialTab);

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
            Desk 03
          </span>
          <span style={{ color: "var(--text-muted, #6b7280)" }}>&bull;</span>
          <span style={{ fontWeight: 600, fontSize: 13 }}>Distribution &amp; Quoting</span>
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            type="button"
            onClick={() => setTab("rfq")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "rfq" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "rfq" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "rfq" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            RFQ Desk &amp; Auto-Quote
          </button>
          <button
            type="button"
            onClick={() => setTab("stream_fx")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "stream_fx" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "stream_fx" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "stream_fx" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FX Outbound Streams
          </button>
          <button
            type="button"
            onClick={() => setTab("stream_fi")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "stream_fi" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "stream_fi" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "stream_fi" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            FI Outbound Streams
          </button>
          <button
            type="button"
            onClick={() => setTab("aggbook")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "aggbook" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "aggbook" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "aggbook" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Aggregated LP Book
          </button>
          <button
            type="button"
            onClick={() => setTab("tiering")}
            style={{
              padding: "4px 12px",
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              border: "1px solid",
              background: tab === "tiering" ? "var(--bg-elevated, #161c28)" : "transparent",
              borderColor: tab === "tiering" ? "var(--border-medium, rgba(255,255,255,0.2))" : "transparent",
              color: tab === "tiering" ? "var(--text-primary, #f3f4f6)" : "var(--text-secondary, #9ca3af)",
            }}
          >
            Client Tiering &amp; Skew
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "rfq" && <QuotingWorkspace />}
        {tab === "stream_fx" && <StreamWorkspace />}
        {tab === "stream_fi" && <FiStreamingWorkspace />}
        {tab === "aggbook" && <AggregatedBookWorkspace />}
        {tab === "tiering" && <PricingGroupsWorkspace initialTab="tiering" />}
      </div>
    </div>
  );
}
