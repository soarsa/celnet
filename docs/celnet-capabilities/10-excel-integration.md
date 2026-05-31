<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Excel Integration</sub>

# 10. Excel Integration

The spreadsheet is where structuring, marking, and ad-hoc risk actually happen on most desks — so Celnet meets the desk there, natively. An Office.js add-in exposes the platform's full pricing, surface, quoting, and streaming capability as worksheet functions under the **`CELNET.*`** namespace, speaking the *same* single, unversioned contract as the GUI, the Rust client SDK, and the CLI. Excel becomes a first-class Celnet client: every number a cell shows is the server's value, computed in the engine and bit-identical to what the trader sees in the GUI. There is no second pricing path hiding in a macro.

![Branded Excel hero — a CELNET.RFQ formula in the bar, a two-way RFQ spill, the full Greek vector spilling from CELNET.GREEKS, a smile grid from CELNET.SURFACE, live CELNET.SERIES observable cells, and the connected Celnet task pane.](../assets/celnet-capabilities/shot-10-excel-grid-branded.png)
*The branded Excel surface: live two-way prices, spilled Greeks, a smile grid, and streaming observables — all served from the engine over the one contract.*

### 10.1 Design principles

Three rules make the add-in safe to trust on a live desk.

- **No pricing in the cell.** The add-in never approximates, never falls back to a local model, never reimplements a Greek. Each function marshals inputs to the engine and renders what comes back. The cell is a view, not a calculator — so a worksheet can never silently disagree with the firm's system-of-record.
- **Read is zero-click; write is two-phase.** Read-and-spill functions (`PRICE`, `GREEKS`, `SURFACE`, `RFQ`, `SUBSCRIBE`, `SERIES`) evaluate the moment the cell does, with no confirmation friction. Anything that *mutates* shared state — contributing a mark — is two-phase: the cell stages the intent, and the action is only transmitted after explicit confirmation in the task pane. Recalculating a sheet can never accidentally publish a price.
- **Convention and version transparency, per cell.** Every priced cell carries, on its face or in a spilled footer, the exact convention basis (delta convention, premium adjustment, ATM rule) and the **surface version** it was marked against. A trader auditing a workbook can see precisely which canonical inputs and which pinned surface produced any figure — no hidden state.

Failures are typed, not silent: the add-in surfaces structured **`#CELNET_*`** spreadsheet errors (for example a bad-argument error versus a stale-data error versus a transport error), so a mistyped tenor, an unknown surface version, and a dropped connection are visibly distinct rather than all collapsing to a generic `#VALUE!`.

### 10.2 The `CELNET.*` worksheet functions

| Function | What it does | Shape |
|----------|--------------|-------|
| `CELNET.PRICE` | Prices a vanilla off the engine — outright forward, separate domestic/foreign discounting | Single value (bid / mid / offer) |
| `CELNET.GREEKS` | The full FX desk Greek set in one pass | Spills the Greek vector with a convention footer |
| `CELNET.SURFACE` | Pulls the marked smile/surface | Spills a smile grid (ATM, risk-reversals, butterflies, by tenor) |
| `CELNET.MARKSURFACE` | Calibrates and marks a surface under a chosen smile model (VV / SABR / SVI / SSVI) | Two-phase, task-pane-confirmed |
| `CELNET.RFQ` | Requests a two-way quote | Spills two-way price, quote id, validity |
| `CELNET.SUBSCRIBE` | Subscribes a cell to a live streaming two-way | Self-updating streaming cell |
| `CELNET.SERIES` | Subscribes to a live market observable | Streaming series (ATM vol / spot / RR / BF / forward) |
| `CELNET.MARK` | Contributes a mark to the official surface | Two-phase, task-pane-confirmed |

The reads spill structured output natively — `GREEKS` lays the whole sensitivity vector down a column with its convention basis attached; `SURFACE` lays a smile grid across the sheet — so a single formula populates an entire analytic block that stays live as inputs change. The streaming functions (`SUBSCRIBE`, `SERIES`) turn ordinary cells into pinned subscriptions on the engine's multiplex stream, refreshing in place as the market moves, with no polling logic for the user to maintain.

![The Celnet Excel task pane, connected — RFQ entry, the Contribute-Mark two-phase confirm, and the VV / SABR / SVI / SSVI smile-model selector.](../assets/celnet-capabilities/shot-09-excel-taskpane.png)
*The task pane is the control surface for connection, RFQ, and the confirmed contribute-mark — including the smile-model selector that drives `MARKSURFACE` and `MARK`.*

### 10.3 Desk workflows in the spreadsheet

Because the add-in rides the same contract as every other client, real desk work runs end-to-end inside Excel — and every result reconciles, to the last bit, with the GUI.

- **Live model sheet.** A structuring sheet wires `CELNET.PRICE` and `CELNET.GREEKS` into a trader's own layout. Price and the entire Greek vector spill live, each cell stamped with its convention basis, so a hand-built term sheet prices off the firm engine rather than a parallel model — and stays correct as spot, vol, and dates move.
- **RFQ to trade ticket.** `CELNET.RFQ` returns a two-way with a quote id and a validity window straight into the grid. The trader works the quote in the sheet and carries it to a ticket, with the quote's identity and last-look validity intact — the spreadsheet becomes a quoting front-end, not a detached calculator.
- **Two-phase contribute-mark.** A marking sheet stages an updated smile and re-marks it under any of VV, SABR, SVI, or SSVI via the smile-model selector; `MARKSURFACE` calibrates and `MARK` contributes. The write only leaves the cell on explicit task-pane confirmation, and arbitrage gates apply server-side, so a contributed surface is always arbitrage-checked and never published by an accidental recalculation.
- **Streaming cells.** `CELNET.SUBSCRIBE` pins a live two-way to a cell and `CELNET.SERIES` pins a live observable (ATM vol, spot, risk-reversal, butterfly, forward), so a monitoring sheet or a trend block updates itself off the multiplex stream — the same feed that drives the GUI's trend modes.
- **Bespoke risk and scenario sheets.** With prices and the full Greek set available per cell, a trader composes private scenario and risk layouts — shock grids, vega ladders, custom roll-ups — in familiar spreadsheet form, every figure traceable to its convention basis and pinned surface version.

Because there is exactly one contract and one pricing path, the spreadsheet is never a fork. A value computed in a cell, shown in the GUI, returned to the Rust SDK, or carried over FIX is the same value — Excel is simply another window onto the one engine.

---
<sub>[← API & Wire Contract + API-First Client Parity](09-api-contract-parity.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [The Trader GUI →](11-trader-gui.md)</sub>
