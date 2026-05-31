<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › API & Wire Contract + API-First Client Parity</sub>

# 9. API & Wire Contract + API-First Client Parity

Celnet exposes **one clean, unversioned contract**. There is no schema-version negotiation, no N/N-1 compatibility window, no privileged internal path — a single current contract that every surface speaks. Each capability the platform offers is reachable through that one contract, and every client — the trader GUI, the typed Rust SDK, the admin CLI, and the Excel `CELNET.*` add-in — consumes exactly the same service families. The front-end is just another client. This is the governing **API-first** rule: capability lives in the contract, never in a client, so a number you read in the GUI is bit-identical to the one the SDK returns, the CLI prints, and a spreadsheet cell spills.

![API-first client parity: one contract, every surface](../assets/celnet-capabilities/fig-02-api-first-parity.png)
*Figure 9.1 — One contract, every surface. The GUI, Rust SDK, CLI, and Excel add-in are peers over the same Pricing / Quote / StreamSession / Surface families; a value is identical wherever it is read.*

### 9.1 The service families

The contract is organised into four service families that map directly to how a desk works: get a price, get a two-way quote, stream live markets, and mark surfaces.

| Family | What a desk does with it |
|--------|--------------------------|
| **Pricing** | Request a price and the full FX desk Greek set for any instrument, against live or a pinned marked surface; the response carries a correlation id and the surface version it priced on. |
| **Quote** | Request a firm two-way (RFQ): bid/mid/offer with a quote id and a validity window, ready to lift. |
| **StreamSession** | One bidirectional, multiplexed channel carrying many concurrent subscriptions — streaming prices, two-ways, and observable series — with click-to-trade. |
| **Surface** | Mark, recalibrate, and publish smiles under a chosen smile model; pin a surface version through a `Pin` resolver so every downstream price is reproducible. |

A **unified `Instrument` type** runs through all four — the same instrument vocabulary describes a vanilla, a digital, a barrier, or a structured leg, so the contract does not fork by product. The contract also carries, as first-class fields, a **smile-model selector** (mark or recalibrate under Vanna-Volga, SABR, raw-SVI, or SSVI), a **market-series feed** of live observables (ATM, spot, risk-reversal, butterfly, forward) that drives the GUI trend modes, and an **attribution identity** (book / owner) dimension that ties every price and position to its place in the risk hierarchy.

### 9.2 Surface-version pinning

Pricing and risk are only reproducible if everyone agrees on the surface. Every marked surface is deposited under a fresh **surface version**, and any pricing, RFQ, or stream request can **pin** to a specific version through the `Pin` resolver. An unknown version is **rejected** — never silently resolved to the live surface — so a quote, a risk report, and a re-priced ticket all reference the identical calibrated smile, and "what surface did this price come from?" always has a single, exact answer.

### 9.3 The multiplex StreamSession and click-to-trade

The StreamSession is a single bidirectional channel that fans out to many subscriptions. Each subscription follows a clean lifecycle — **Subscribe → Snapshot → Update(seq) → Heartbeat** — with monotonic sequence numbers so a client can detect a gap and issue **Resync** to re-baseline, and an in-place **Modify** that re-bases a subscription (new strike, notional, or tenor) without tearing it down. Heartbeats keep liveness explicit even on a quiet instrument.

Click-to-trade is built into the same channel. A streamed line carries an **unguessable, keyed-MAC tradable token** — one to sell at the bid, one to buy at the offer — cryptographically bound to that exact line. To deal, the client returns the token through **Execute**:

- **Last-look validity** — the token carries a validity window; the maker checks it on receipt, so a stale token is declined rather than filled at a moved market.
- **Forgery-proof** — the token is a keyed message authentication code over the line-binding tuple under a per-session secret; a forged or tampered token is rejected.
- **Idempotent** — a token is single-use; a duplicate or already-consumed Execute is rejected, so a retransmit can never double-deal.

![StreamSession lifecycle and click-to-trade token flow](../assets/celnet-capabilities/fig-11-streamsession-clicktrade.png)
*Figure 9.2 — The multiplex StreamSession: many subscriptions over one channel, sequence-gap Resync, in-place Modify, and the keyed-MAC click-to-trade token with last-look and idempotent Execute.*

The trader sees this as a single confident gesture: click a streamed price, get a last-look response, and have the deal confirmed — or cleanly declined — with no ambiguity about which market was dealt.

![Click-to-trade last-look response in the live stream blotter](../assets/celnet-capabilities/shot-08-clicktrade-lastlook.png)
*Figure 9.3 — Click-to-trade in the live blotter: a lifted line returns a last-look response bound to the exact streamed price.*

### 9.4 The byte-identical WebSocket mirror

The contract is served over a high-performance binary transport for native clients and over a **byte-identical WebSocket JSON mirror** for browser and lightweight clients. The mirror is not a parallel API — it is the same contract, field-for-field, so a value crossing the WebSocket is identical to the one crossing the binary edge. This is what lets the React GUI and a native SDK client share one mental model and one set of guarantees, and it is verified continuously: the JSON projection is checked to mirror the wire contract exactly.

### 9.5 Typed SDK and admin CLI

The **typed Rust client SDK** turns the contract into ergonomic, statically-checked calls: typed requests and responses, **typed errors** (so a rejected stale token or an unknown surface version surfaces as a specific, matchable error rather than a string), and built-in **reconnect-liveness** — across a blue-green cutover or a dropped link, the SDK drains its pending click-to-trade waiters and resolves every outstanding deal with a typed outcome, so a client never hangs waiting on a lost connection. The **admin CLI** drives the same contract for operations and scripting — subscribe, price, quote, inspect surfaces — with no separate control API to learn.

### 9.6 API-first client parity in practice

Because there is exactly one contract and every client is a peer over it, parity is structural rather than aspirational:

- The **GUI** (React + WebGPU, live-WebSocket-by-default) drives Pricing, Quote, StreamSession, and Surface — the same families, no shortcuts.
- The **Rust SDK** and **CLI** call the identical service families with typed semantics.
- The **Excel `CELNET.*` add-in** runs no maths in the cell — `CELNET.PRICE`, `CELNET.GREEKS`, `CELNET.SURFACE`, `CELNET.MARKSURFACE`, `CELNET.RFQ`, `CELNET.SUBSCRIBE`, `CELNET.SERIES`, and `CELNET.MARK` are thin calls into the same contract, so every spreadsheet number is the server's value, bit-identical to the GUI, with per-cell convention and surface-version transparency and typed `#CELNET_*` errors.

A capability ships once, in the contract, and every surface gains it in lockstep — including the documentation, which tracks the single current contract with no stale or duplicate references. The result is a platform where the trading desk, the quant in a spreadsheet, and an automated client all see the same prices, the same Greeks, the same surfaces, and the same deals — proven identical, not merely intended to be.

---
<sub>[← Scalability & Scale-Out](08-scalability-scaleout.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Excel Integration →](10-excel-integration.md)</sub>
