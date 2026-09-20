# CelNet GUI Architecture Critique & Rearchitecture Blueprint
## Transitioning from Monolithic Shell to Sovereign, Multi-Instance Micro-Apps on DeskModal

**Document Version:** 1.0.0  
**Date:** September 20, 2026  
**Status:** Canonical Design & Architecture Specification  
**Target Platform:** CelNet Sovereign Institutional FX/Rates Engine + DeskModal Desktop Agent (Rust + Tauri + FDC3 2.2)

---

## Executive Summary

CelNet's trading interface originated as a single-window institutional workbench containing 6 unified trading desks (Markets, Pricing, Distribution/RFQ, Blotter, Risk Cube, and Operations/Policy). While comprehensive, embedding this entire single-window shell into DeskModal—an operating-system-grade financial desktop container—creates an acute architectural mismatch: **the "OS-inside-an-OS" anti-pattern**.

In DeskModal, the **DeskModal Launcher / Dock / App Directory is the primary navigator**. Forcing traders to navigate through an internal CelNet rail, secondary domain tabs, and desk switcher buttons inside a single window violates desktop container doctrine, wastes 180+ pixels of vertical screen real estate, prevents multi-monitor layout optimization, and cripples multi-instance workflows.

This document presents:
1. A rigorous critique of the current CelNet GUI architecture.
2. The multi-instance institutional trading requirements (e.g. concurrent multi-tenor pricer tickets, multi-currency depth ladders, multi-book risk cubes).
3. The sovereign rearchitecture transforming CelNet's 6 desks into discrete, valuable, FDC3-interoperable micro-apps.
4. The FDC3 v2.1/2.2 channel linking and intent routing topology.
5. The dual-mode execution model preserving standalone browser fallback while enabling native DeskModal integration.

---

## 1. Deep Architectural Critique of Current GUI Tier

### 1.1 The Navigation Paradox: Monolithic Shell vs DeskModal as Navigator

| Aspect | Monolithic Single-Window Model | DeskModal Desktop Container Model | Critique & Resolution |
|---|---|---|---|
| **Primary Navigator** | Left-hand navigation rail (`Shell.tsx:rail`) + Desk Switcher (`StudioSwitcher.tsx`) | DeskModal OS Launcher, Dock, App Catalog, Cmd+K Palette | **Redundant & Confusing**: When running inside DeskModal, CelNet's internal rail duplicates DeskModal's Dock. CelNet must cede navigation to DeskModal. |
| **Window Topography** | Single monolithic window holding all 6 desks in memory, toggling visibility via CSS | Multi-window, multi-monitor grid of floating tearouts and tiled workspaces | **Screen Real Estate Waste**: The internal shell rail, domain tab bar, title bar, and status ribbon consume ~200px of vertical height. In micro-app mode, these must collapse into an info-dense 34px App Toolbar. |
| **App Identity** | One umbrella application ("CelNet") | 6 distinct, first-class financial apps registered in DeskModal's AppDirectory | **Marketplace Invisibility**: Traders look in DeskModal for "Options Pricer" or "Market Depth", not an undifferentiated "CelNet" monolith. Each desk must be an independently launchable app. |

### 1.2 The Multi-Instance Blind Spot

In institutional trading, desks are never singletons:
- **Options Structuring**: A trader running a complex hedge needs 3 Pricing Tickets open simultaneously (e.g. Window 1: `EUR/USD 1M ATM Call`, Window 2: `USD/JPY 3M 25D Risk Reversal`, Window 3: `XAU/USD 6M Barrier Option`).
- **Spot & Forward Market Making**: A dealer monitors 4 depth ladders concurrently across multiple displays (e.g. `EUR/USD`, `GBP/USD`, `USD/JPY`, `USD/CHF`).
- **Risk Management**: Risk managers monitor distinct portfolios in separate windows (e.g. `FXO_G10_FLOW` vs `FXO_EXOTICS_LATAM`).

**Current Limitation in CelNet**:
- `AppContext.tsx` persisted active scope (`pair`, `underlier`, `groupBy`) to un-namespaced `localStorage` keys (`celnet:scope`, `celnet:activeDomain`).
- Opening a second window or tearout resulted in state cross-talk: changing the currency pair in Window 1 immediately stomped Window 2 upon refresh or storage events.
- No mechanism existed to pass initial deep-link parameters (`?sym=...`, `?strike=...`, `?expiry=...`, `?book=...`, `?instanceId=...`) to seed and isolate distinct instances.

### 1.3 Inter-App Communication & FDC3 Channel Gaps

- **Lack of Inbound Context Synchronization**: While `AppContext.tsx` broadcasted `fdc3.instrument` upon user selection, it lacked an inbound context listener. If a user clicked a stock or currency in DeskModal's Watchlist, Chart, or Screener, CelNet windows remained deaf to the event.
- **Missing Channel Selector (Color Dot)**: Professional financial terminals (Bloomberg, Eikon, OpenFin, DeskModal) feature a prominent channel selector (Red, Green, Blue, Orange, Purple, Yellow, Global). CelNet had no in-app channel indicator, preventing traders from grouping Window A (Market Depth) with Window B (Pricer) on the "Red" channel while keeping Window C on "Blue".
- **Unregistered Intent Handlers**: DeskModal supports FDC3 intent resolution (`ViewAnalysis`, `ViewInstrument`, `Trade`, `ViewOrders`). CelNet's desks lacked registered listeners to handle these intents dynamically.

---

## 2. The Target Architecture: Discrete, High-Value Micro-Apps

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                           DESKMODAL DESKTOP CONTAINER                                   │
│  [Launcher / Dock]  →  Cmd+K Palette  →  App Catalog  →  FDC3 v2.2 Bus  →  Window Mgr    │
└────────┬─────────────────────────┬──────────────────────────┬───────────────────────────┘
         │                         │                          │
         ▼                         ▼                          ▼
┌──────────────────┐      ┌──────────────────┐      ┌───────────────────┐
│  Options Pricing │      │  Market Depth    │      │  Deals Blotter    │
│  Studio [App 02] │      │  Studio [App 01] │      │  Studio [App 04]  │
├──────────────────┤      ├──────────────────┤      ├───────────────────┤
│ [🔴 Red] EUR/USD │      │ [🔴 Red] EUR/USD │      │ [🔵 Blue] ALL      │
├──────────────────┤      ├──────────────────┤      ├───────────────────┤
│ • 23 Structures  │      │ • L1/L2 Depth    │      │ • ISDA CDM Audit  │
│ • 13 Live Greeks │      │ • Book Ladder    │      │ • Executed Fills  │
│ • Payoff Surface │      │ • VWAP Skews     │      │ • Position Ledger │
└──────────────────┘      └──────────────────┘      └───────────────────┘
         ▲                         ▲                          ▲
         └──────── FDC3 Bus ───────┴────── (Context: EUR/USD) ┘
```

### 2.1 The 6 Autonomous Micro-Apps

| App ID | Display Title | Description & Institutional Value | Primary FDC3 Intents |
|---|---|---|---|
| `celnet.studio.pricing` | **Options Pricing Studio** | Sell-side structuring workbench: 23 option structures, 13 live Greeks, closed-form Garman-Kohlhagen, PDE local-vol, and payoff visualizer. | Listens: `ViewAnalysis`, `celnet.Price`<br>Raises: `Trade`, `CalculateValuation` |
| `celnet.studio.markets` | **Markets & Depth Studio** | Multi-venue L1/L2 orderbook aggregation, real-time streaming quotes, consolidated depth ladders, and microsecond tick-by-tick tape. | Listens: `ViewInstrument`<br>Raises: `Trade`, `ViewAnalysis` |
| `celnet.studio.rfq` | **RFQ & Distribution Cockpit** | Two-way dealer RFQ negotiation engine, client margin/spread tiering, auto-quote rules, and ISDA CDM 2026 execution states. | Listens: `Trade`<br>Raises: `CalculateValuation` |
| `celnet.studio.risk` | **Risk Cube & Stress Studio** | Real-time hierarchical risk cube, net delta/gamma/vega cash/percentile aggregation, historical VaR, CS01, and jump-diffusion shock matrices. | Listens: `celnet.ViewRisk`<br>Raises: `ViewAnalysis` |
| `celnet.studio.blotter` | **Deals & Position Blotter** | High-density trade execution blotter, real-time position ledger, clearing status, and full cryptographic ISDA CDM lifecycle audit log. | Listens: `deskmodal.ViewOrders`<br>Raises: `ViewInstrument`, `ViewAnalysis` |
| `celnet.studio.analytics` | **Curve & Swaption Analytics** | Multi-curve discount term structure, OIS/SOFR bootstrapping, basis swaps, and Cheyette 2F swaption calibration. | Listens: `ViewAnalysis`<br>Raises: `CalculateValuation` |

---

## 3. Implementation Blueprint

### 3.1 Dual-Mode Operation (`isAppMode`)
- When running embedded in DeskModal (`deskmodal-plugin://`, iframe, or `?mode=app`):
  - Strip outer left rail (`<aside className="rail">`).
  - Strip top product domain tabs (`<div className="tabBar">`).
  - Strip heavy shell title bar and bottom status ribbon.
  - Mount the **Micro-App Toolbar**:
    - App Icon & Title (`📈 Options Pricing Studio`)
    - **FDC3 Channel Selector** (Global, Red, Orange, Yellow, Green, Blue, Purple, Cyan)
    - Active Instrument Scope pill (`EUR/USD`)
    - Instance ID indicator (`Instance #1`)
    - Quick actions: "Duplicate / Popout", "Compact Density", "SHM Latency"
  - Allocate 100% of remaining screen real estate to the active studio canvas.
- When running standalone in an external browser (`http://localhost:4317`):
  - Retain the full multi-pane Shell for complete browser-based navigation.

### 3.2 Instance Isolation & Parameter Hydration
- Each window reads:
  - `instanceId`: Identifies the instance (e.g. `?instanceId=win-trader-1`).
  - `sym` / `pair`: Seeds the initial currency pair / underlier.
  - `channel`: Seeds the initial FDC3 user channel (e.g. `?channel=red`).
  - `strike` / `expiry` / `structure`: Seeds the initial option parameters.
- Storage keys for instance-specific UI state are namespaced by `instanceId` to prevent cross-window interference.
- Authentication tokens and credentials remain shared across all instances via `useAuth.ts` and host token pass-through.

### 3.3 FDC3 Inbound & Outbound Integration
- **Inbound Context Listener**: Subscribes to `fdc3.instrument` on the active user channel. Upon arrival, updates the active underlier and ticket target without page reload.
- **Inbound Intent Handlers**: Handles `ViewAnalysis`, `ViewInstrument`, `Trade`, `ViewOrders`, and `ViewRisk` to route contexts directly to the appropriate desk.
- **Outbound Broadcast**: Dispatches `fdc3.instrument` and `fdc3.valuation` whenever the trader modifies underliers or executes trades.

### 3.4 Build Pipeline & Sub-App Generation
- A dedicated post-build generator (`gui/scripts/generate-deskmodal-apps.mjs`) parses the production Vite bundle in `dist/` and automatically synthesizes the sub-app entrypoints (`dist/pricing/index.html`, `dist/markets/index.html`, etc.) with the exact asset hashes and pre-configured default views.
- Deploys directly to `/Users/adrian/deskmodal/plugins/celnet-studio/app/` ensuring zero stale asset references.
