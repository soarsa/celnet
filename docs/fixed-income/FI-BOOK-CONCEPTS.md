# FI "Book" concepts — Agg Book vs Book vs Risk Portfolios

The Fixed Income surfaces expose three different things that historically all carried
the word "book", which confused traders. They are distinct concepts. This is the
canonical reference for what each one is, how a fill relates to all three, and the
UI-name ↔ wire-name mapping.

## The three concepts

| Surface (UI label) | What it is | Workspace file | Rail id |
| --- | --- | --- | --- |
| **Agg Book** | The **liquidity composite** — consolidated best bid/offer aggregated across a book's inbound LP connections (FIX/API). A *price* view, not positions. | `gui/src/workspaces/AggregatedBookWorkspace.tsx` | `aggbook` |
| **Book** | The **position ledger** — your booked positions, booking, and deals (the rates warehouse; what you actually hold). **FX-tab only:** under Fixed Income this rail entry is removed and its ledger surfaces (Positions · Quotes · Deals) are folded into the FI **Risk** workspace as tabs — see the note below. | `gui/src/workspaces/BookWorkspace.tsx` (+ `RatesBookWorkspace.tsx`, `DealsBlotterWorkspace.tsx`) | `book` (FX rail only) |
| **Risk Portfolios** | The **risk-management overlay** — the trader-defined hierarchy (desk → portfolio → sub-portfolio) that routing rules drop each fill's risk into, each with its own limits, feeding the Risk Dashboard roll-up. | `gui/src/workspaces/RiskBooksWorkspace.tsx`, `gui/src/workspaces/RiskDashboardWorkspace.tsx`, `gui/src/workspaces/riskrouting/` | `riskbooks`, `riskdashboard`, `riskrouting` |

One-line distinguishers (also shown as subtitles in each workspace):

- **Agg Book** — "Live LP-aggregated PRICES (liquidity) — not your positions."
- **Book** — "Your booked POSITIONS + booking + deals (the ledger)."
- **Risk Portfolios** — "How your risk is BUCKETED for management — routing drops fills here; the ledger 'Book' is where they're actually booked."

## How a single fill relates to all three

A fill touches all three surfaces, in different roles:

1. It is **priced off the Agg Book** — the outbound price a counterparty trades on is
   the LP-aggregated composite (optionally tiered/skewed per pricing group).
2. It is **booked into the Book** — the resulting position lands in the position
   ledger (the warehouse of what you hold), where P&L and booking live.
3. Its risk is **routed into a Risk Portfolio** — the risk-routing rules
   (first-match-wins) drop the fill's *risk* into a leaf risk portfolio so
   per-portfolio limits, greeks, notional and PnL roll up the tree in the Risk
   Dashboard.

So: **prices come from the Agg Book, the position is booked in the Book, and the risk
is bucketed into a Risk Portfolio.** The Book (ledger) and a Risk Portfolio are not
the same thing — the ledger is where the position is actually held; the risk
portfolio is a management overlay for limits and risk roll-up.

## Fixed-Income "Book" → "Risk" consolidation (2026-07-31)

Traders found the FI **Book** and FI **Risk** rail entries redundant: the FI Book's
"Aggregate Risk" lens rendered the SAME `RatesRiskPanel` the Risk workspace already
shows. So under **Fixed Income** the **Book** rail entry is REMOVED and its
position-ledger surfaces are folded INTO the **Risk** workspace as tabs — FI Risk is
now the single **risk + positions + deals** surface:

| FI Risk tab | Renders | Was |
| --- | --- | --- |
| **Scenario Risk** | `RatesRiskPanel` (netted rates what-if) | the Risk workspace's own content |
| **Positions** | `RatesBookWorkspace` (ledger + booking) | Book lens "Positions & Booking" |
| **Quotes** | `QuotesBlotterWorkspace` (shown-quotes) | Book lens "Quotes" (kept — non-redundant with Deals) |
| **Deals** | `DealsBlotterWorkspace` (incl. routed Risk-Portfolio column) | Book lens "Deals" |

The Book lens **"Aggregate Risk"** is DROPPED as the redundancy this removes (it was
identical to Scenario Risk). **FX Options is unchanged:** the FX rail keeps BOTH a
**Risk** row (scenario grid only, no tabs) and a separate **Book** row — the
consolidation is FI-only.

Mechanism (GUI): the `book` workspace and its `assets` are untouched (still serves
both classes; reachability / license / `⌘N` unchanged) — only its Fixed-Income rail
membership is withdrawn via `DOMAIN_RAIL_EXCLUDED` in `gui/src/lib/commands.ts`. The
FI tab bar lives in `RiskWorkspace.tsx` (`FiRiskSurface`).

## UI name ↔ wire name mapping (Risk Portfolios)

"Risk Portfolios" is a **UI-only rename** (2026-07-30). The internal wire contract is
unchanged for compatibility — only the trader-facing text changed:

| User-facing (UI) | Internal / wire name (unchanged) |
| --- | --- |
| Risk Portfolio | `RiskBookDef` / `RiskBook` (TS `gui/src/data/contract.ts`); proto `RiskBookDesc` |
| (the RPCs) | `ListRiskBooks` / `CreateRiskBook` / `UpdateRiskBook` / `DeleteRiskBook`, `ListRiskBookRisk` |
| Risk Portfolios rail | rail id `riskbooks` (`gui/src/lib/commands.ts`) |
| the workspace | `RiskBooksWorkspace` (file name unchanged) |
| a rule's destination portfolio | `RiskRule.bookId`, `RiskBook` graph leaf (`gui/src/lib/riskRules.ts`) |

The `RiskBook`/`RiskBookDef` types, the `riskbooks` rail id, the `*RiskBook*` RPCs,
the `RiskBooksWorkspace` file, and the `bookId`/`parentId` fields all stay. Do not
rename them — the rename lives entirely in the visible copy.

## Pointers

- Risk routing + risk portfolio requirements → [`docs/FI-RISK-ROUTING-REQUIREMENTS.md`](FI-RISK-ROUTING-REQUIREMENTS.md) (§6.1 routing, §6.2 the portfolio tree, §6.3 the dashboard).
- Aggregated book (liquidity composite) requirements → [`docs/FI-AGGREGATED-BOOK-REQUIREMENTS.md`](FI-AGGREGATED-BOOK-REQUIREMENTS.md).
- Rail labels + glyphs → `gui/src/lib/commands.ts`.
