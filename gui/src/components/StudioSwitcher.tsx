/**
 * StudioSwitcher — Fast switcher for Celnet's 6 Unified Trading Desks.
 *
 * Institutional trading workflow switcher representing the full sell-side
 * market-making lifecycle as per the September 2026 commercial benchmark.
 */

import { useState, useRef, useEffect } from "react";
import { useApp } from "../app/AppContext";
import { type WorkspaceId } from "../lib/commands";

export const CELNET_DESKS: readonly {
  id: WorkspaceId;
  code: string;
  name: string;
  desc: string;
}[] = [
  {
    id: "studio_markets",
    code: "01",
    name: "Markets & Volatility",
    desc: "Live 3D Vol Surfaces, Multi-Curve Yield & Basis, Feed Health",
  },
  {
    id: "studio_pricing",
    code: "02",
    name: "Pricing & Structuring",
    desc: "Universal Ticket, 13 Greeks, Payoff Profiler, XVA Exposure Fans",
  },
  {
    id: "studio_distribution",
    code: "03",
    name: "Distribution & Quoting",
    desc: "RFS Streaming Engine, Client Tiering Matrix, RFQ Auto-Quoter",
  },
  {
    id: "studio_blotter",
    code: "04",
    name: "Blotters & Position Ledger",
    desc: "Real-Time Position Book, Executed Deals, Quote History, Transfers",
  },
  {
    id: "studio_risk",
    code: "05",
    name: "Risk & Hedging Cockpit",
    desc: "Hierarchical Risk Cube, Joint C2c VaR, Scenario Grids, Auto-Hedge",
  },
  {
    id: "studio_policy",
    code: "06",
    name: "Operations & Connectivity",
    desc: "Pre-Trade Acceptance Rules, Risk Routing, Latency Ops, FIX Sessions",
  },
];

// Alias for backwards-compatibility
export const PARAMOUNT_STUDIOS = CELNET_DESKS;

export function StudioSwitcher(): React.ReactElement {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const activeDesk = CELNET_DESKS.find((s) => s.id === app.workspace);

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [open]);

  return (
    <div ref={containerRef} style={{ position: "relative", display: "inline-block" }}>
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          padding: "4px 10px",
          borderRadius: 4,
          fontSize: 12,
          fontWeight: 600,
          cursor: "pointer",
          background: activeDesk ? "rgba(255, 115, 87, 0.15)" : "var(--bg-elevated, #1a2233)",
          color: activeDesk ? "var(--brand-coral, #ff7357)" : "var(--text-primary, #f3f4f6)",
          border: activeDesk
            ? "1px solid var(--brand-coral, #ff7357)"
            : "1px solid var(--border-subtle, rgba(255,255,255,0.12))",
          transition: "all 0.15s ease",
        }}
        title="Switch to one of the 6 Celnet Unified Trading Desks"
      >
        <span
          style={{
            display: "inline-block",
            width: 6,
            height: 6,
            borderRadius: "50%",
            background: activeDesk ? "var(--brand-coral, #ff7357)" : "var(--text-muted, #9ca3af)",
          }}
        />
        <span>{activeDesk ? `${activeDesk.code} ${activeDesk.name}` : "Trading Desks"}</span>
        <span style={{ fontSize: 9, opacity: 0.7 }}>▾</span>
      </button>

      {open && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            left: 0,
            width: 290,
            background: "var(--bg-elevated, #161c28)",
            border: "1px solid var(--border-subtle, rgba(255,255,255,0.15))",
            borderRadius: 6,
            boxShadow: "0 10px 25px -5px rgba(0,0,0,0.5), 0 8px 10px -6px rgba(0,0,0,0.4)",
            zIndex: 9999,
            padding: "6px 0",
            backdropFilter: "blur(8px)",
          }}
        >
          <div
            style={{
              padding: "4px 12px 6px",
              fontSize: 10,
              fontWeight: 700,
              textTransform: "uppercase",
              letterSpacing: "0.05em",
              color: "var(--text-muted, #9ca3af)",
              borderBottom: "1px solid var(--border-subtle, rgba(255,255,255,0.08))",
            }}
          >
            Celnet Trading Desks
          </div>
          {CELNET_DESKS.map((s) => {
            const isSelected = app.workspace === s.id;
            return (
              <button
                key={s.id}
                type="button"
                onClick={() => {
                  app.setWorkspace(s.id);
                  setOpen(false);
                }}
                style={{
                  width: "100%",
                  textAlign: "left",
                  padding: "8px 12px",
                  background: isSelected ? "rgba(255, 115, 87, 0.12)" : "transparent",
                  border: "none",
                  cursor: "pointer",
                  display: "flex",
                  flexDirection: "column",
                  gap: 2,
                  transition: "background 0.1s ease",
                }}
                onMouseEnter={(e) => {
                  if (!isSelected) e.currentTarget.style.background = "rgba(255,255,255,0.05)";
                }}
                onMouseLeave={(e) => {
                  if (!isSelected) e.currentTarget.style.background = "transparent";
                }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <span
                    style={{
                      fontFamily: "var(--font-mono, monospace)",
                      fontSize: 11,
                      fontWeight: 700,
                      color: isSelected ? "var(--brand-coral, #ff7357)" : "var(--brand-indigo, #6b6bf5)",
                    }}
                  >
                    {s.code}
                  </span>
                  <span
                    style={{
                      fontSize: 12,
                      fontWeight: 600,
                      color: isSelected ? "var(--brand-coral, #ff7357)" : "var(--text-primary, #f3f4f6)",
                    }}
                  >
                    {s.name}
                  </span>
                </div>
                <div
                  style={{
                    fontSize: 10,
                    color: "var(--text-secondary, #9ca3af)",
                    lineHeight: 1.3,
                  }}
                >
                  {s.desc}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
