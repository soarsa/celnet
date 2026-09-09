# Auto-Hedging & Risk Internalisation — Requirements / Design

> Status: **requirements + design for review** (2026-07-30). No code yet. Grounds a NEW
> Celnet capability on the shipped **risk-routing** decision-graph engine, the shipped
> **aggregation** (Agg Book) internal venue, the **RFQ / FIX LP** external order path, the
> position stores, and the **limits** gate — every seam cited `file:line` (current as of this
> doc). Follows the shipped **risk-routing** and **risk-transfer** patterns (registry +
> resolver + proto/WS CRUD + provenance waterfall + visual building-block surface) as the
> template. Vendor-neutral naming throughout (guardrail 8). OSS-only, academically-grounded
> methodology (guardrail 7). Off the pinned zero-alloc pricing hot core (guardrail 11). No
> mocks / placeholders / `todo!()` (guardrail 2).

---

## 1. Concept & goal — internalise up to a threshold, then hedge the overflow

A market-maker who captures client flow holds **inventory**. The desk faces one continuous
decision on every unit of that risk: **warehouse it** (hold it internally, hoping to earn the
spread and let opposing flow or mean-reversion flatten it) or **externalise it** (pay the
street to shed it now). Holding earns the bid-offer and the mean-reversion but bears
**adverse selection** (the flow may be informed/toxic) and **warehousing risk** (the market
moves against the inventory before it nets out). Shedding removes the risk but **pays away**
half-spread + market impact. That is the central market-making dilemma (§3).

The user's concept — the spine of this document:

> *"Good risk price — internalise up to 100, then back-to-back."*

- **Below the threshold** (the configurable "100"): **internalise** the captured risk —
  warehouse it and net it against opposing internal flow / other desks. Capture the spread,
  skew the two-way to attract the offsetting side, and let the position mean-revert.
- **At / over the threshold**: go **back-to-back** — hedge the **overflow** (the amount above
  the threshold) out to the external market, immediately, so the warehoused book never
  exceeds the risk budget.
- **The "100" is a configurable threshold**, not a magic number: a **risk budget** expressed
  in DV01, net/gross notional, or a greek, set **per instrument / per book / per desk** (§4).
- **Traders compose the exit policy themselves** using the **same decision-graph building
  blocks** as the shipped risk-routing feature — an `IF <risk-field> <op> <value> THEN
  <exit-action>` graph they build in the same drag-and-drop editor (§5). The policy states
  **how** to exit: into the **internal aggregator** (cross against opposing flow / other
  desks) and/or **out to the external market** (LPs / venues), and in what order. Then the
  engine **offsets the captured risk**: net internally first, hedge the residual (§6).

### Where this sits in the risk-lifecycle triad

Celnet already ships two of the three risk-management operations; auto-hedging is the third:

| Operation | What it does | Trigger | Status |
|---|---|---|---|
| **Risk routing** (`celnet-risk-routing`) | **Assigns NEW fills** to a risk book via a decision graph at book time | automatic, per fill | **shipped / live** |
| **Risk transfer** (`celnet-risk-transfer`) | **Moves EXISTING risk** between books/desks manually (re-attribution or economic cross) | manual, on demand | **shipped** (`docs/RISK-TRANSFER-REQUIREMENTS.md`) |
| **Auto-hedging** (this doc) | **Manages WAREHOUSED risk** against a threshold — internalise below, hedge the overflow above — via a trader-composed decision graph | automatic, continuous | **NEW** |

Routing decides *which book* owns a fill; auto-hedging decides *what to do* once a book's net
risk crosses the trader's threshold. It closes the loop: fills route in (routing), accumulate
as inventory (position stores), and are continuously netted/hedged against a budget
(auto-hedging), with any residual manually re-parked if needed (transfer).

---

## 2. Architecture fit — the real seams (build ON these, don't reinvent)

Every seam below already exists and is cited. Auto-hedging **reuses the decision-graph engine,
the internal aggregation venue, the external RFQ/FIX order path, the position stores, and the
limits gate**; it adds an **event-driven control loop** and a **new action vocabulary**, not a
new pricing stack or a new position store.

- **The decision-graph engine — the building-block template (reused verbatim in shape).**
  `celnet-risk-routing` is a validated, acyclic decision graph of `Condition` nodes → terminal
  `Book` leaves, walked one edge per node to the first leaf:
  - `RoutingNode` (`crates/celnet-risk-routing/src/graph.rs:21` — `Condition { field, op, value,
    on_true, on_false }` / `Book { risk_book_id }`).
  - `RiskRoutingGraph::validate` (`graph.rs:225`) collects **every** defect at once (dangling
    edge, cycle via 3-colour DFS, unknown target, op-invalid-for-field, value/op mismatch) so a
    GUI surfaces them all — the exact discipline the hedge-policy validator needs.
  - The typed vocabulary: `RouteField` / `RouteOp` / `RouteValue` / `FieldKind`
    (`field.rs:34/80/185/17`), the `RouteOp::valid_for(kind)` matrix (`field.rs:109`), and the
    **total, never-panicking** `RouteOp::eval` (`field.rs:133`). Auto-hedging **reuses `RouteOp`
    and `RouteValue` unchanged** and adds a hedge-specific field/action set (§5).
  - The resolver `RiskRouter::route` (`router.rs:27`) — a pure `O(depth)` bounded walk with a
    `nodes.len()+1` step cap. The hedge resolver is the same walk with **action leaves**.
  - `RoutingContext` (`context.rs:21`) — the flat per-fill snapshot the graph reads via
    `get(field)` (`context.rs:53`). The hedge analogue is a **risk-state** snapshot (§5.2).
- **The book-time route call site (the pattern the hedge loop mirrors for its own trigger).**
  `RiskRouter::route` is invoked at booking in `PositionStore::book`
  (`crates/celnet-server/src/services/risk/store.rs:1198`) and in the rates ledger
  (`crates/celnet-server/src/services/rates_book.rs:660`), each building a `RoutingContext`
  from the booked position (`store.rs:1469`, `rates_book.rs:1097`). Auto-hedging does **not**
  bolt onto this synchronous booking path (guardrail 11); it observes the **risk-version bump**
  the same booking makes (below) and runs its graph **off-core**.
- **The graph registry + CRUD (the config surface a hedge policy reuses).** The firm-wide
  routing graph lives on the identity store: `IdentityStore::risk_routing_graph`
  (`crates/celnet-server/src/config/identity.rs:907`, accessor `:1779`), validated on write by
  `set_risk_routing_graph` → `check_risk_routing_graph` (`identity.rs:1896`). The hedge policy
  is a **sibling field** validated identically against the live book/venue registries.
- **The internal offset venue — the shipped Agg Book (`celnet-aggregation`).** The consolidated
  cross-venue book and the trader's risk-pricer already exist and already carry the exact hooks
  auto-hedging needs:
  - `ConsolidatedBook` (`crates/celnet-aggregation/src/consolidate.rs`, re-exported
    `lib.rs:61`) — continuous BBO across venues, the price a cross/offset is struck at.
  - `RiskPricer` / `RiskPriceParams` (`crates/celnet-aggregation/src/risk.rs:150/79`) — the
    two-way with **inventory/axe directional skew** and a **warehousing risk charge**
    (`RiskPriceParams::risk_charge_bp`, `risk.rs:91`). Skewing to attract the offsetting side
    is the **passive internalisation lever** (§6.3).
  - `AutoSkewSource` (`risk.rs:48`) — the **already-designed, non-stub seam** to lean the quote
    off **live net inventory** read from the position store. Auto-hedging is precisely the
    consumer that seam was built for: it implements `auto_skew_bp` from the book's net position
    to *pull in* offsetting flow before paying to hedge externally.
- **The external hedge order path — the shipped multi-dealer RFQ + real FIX LP adapter
  (`celnet-rfq`).** `MultiDealerEngine` (`crates/celnet-rfq/src/panel.rs`) fans one request to N
  `QuoteSource`s concurrently and ranks a `RankedPanel` (best-bid = max, best-offer = min,
  deterministic `(price, epoch_nanos, lp_id)` tie-break, timeout/last-look). `FixLpAdapter`
  (`crates/celnet-rfq/src/lp_fix.rs:1`) drives a **real FIX 4.4** `Logon → QuoteRequest(R) →
  Quote(S)` cycle against an LP acceptor over a socket; `InternalPricerSource`
  (`crates/celnet-rfq/src/internal.rs:24`) guarantees the panel always has ≥1 dealer. A hedge
  `SUBMIT_MARKET_ORDER` / `RFQ_OUT` action is a request onto this **existing** panel — no new
  LP connectivity stack. (Live WAN LP sessions remain ENV-gated per the honest boundary in
  `lp_fix.rs:25`.)
- **The risk being hedged — the position stores + DV01 / greeks.**
  - FX: `PositionStore` net position / greeks, read via `risk_book_of` (`store.rs:508`),
    `positions_in_risk_book` (`store.rs:519`); per-book roll-up `aggregate_risk_book`
    (`crates/celnet-server/src/services/risk/book_risk.rs:275`) surfacing net/gross notional and
    greeks.
  - Rates/FI: `RatesPosition` on the `rates_book` ledger (`services/rates_book.rs`), with
    **DV01** surfaced per book (`book_risk.rs:157/237`) and the signed key-rate DV01 ladder +
    scenario VaR/ES in `celnet-rates-risk` (`crates/celnet-rates-risk/src/{ladder,var}.rs`).
    DV01 / key-rate DV01 is the natural threshold metric for FI (§4).
- **The threshold / limit machinery — `celnet-limits` (reused for the warehouse budget).**
  `LimitMetric` (`crates/celnet-limits/src/limit.rs:39`), `LimitSpec::hard` / `::soft`
  (`limit.rs:160/172`) `.with_bands(amber, red)` (`limit.rs:186`), `Enforcement`
  (`limit.rs:125`), `RagStatus` (`limit.rs:240`), `Utilization::headroom` (`limit.rs:283`).
  The hard-limit gate `project_risk_book_breach` (`store.rs:1274`) walking `risk_book_chain`
  (`store.rs:1348`) already rejects a fill that would blow a book's cap. **The warehouse
  threshold is a *soft* limit with bands** — amber = "start skewing to attract offset", red =
  "hedge the overflow" — reusing this vocabulary rather than inventing a parallel one (§4).
- **The async off-core control-loop precedent (the pattern the engine copies).** The shipped
  `RiskTransferBroker` (`crates/celnet-server/src/services/risk_transfer/broker.rs:57`) and the
  desk `NotificationBroker` (`services/desk/notify.rs`) are **bounded-queue, `try_send`,
  never-block, never-await** fan-outs held *entirely off the pinned pricing core*
  (`broker.rs:6` doc; `INBOX_QUEUE_DEPTH = 128`, `broker.rs:32`). The live per-book risk stream
  is **version-gated**: `combined_risk_version` (`services/stream.rs:1210`) + the tick loop
  (`stream.rs:1053`) re-publish only when `risk_version` changed. The auto-hedge engine is a
  sibling bounded async loop on the **same version signal**.
- **The provenance/audit template.** `PricingProvenance`
  (`crates/celnet-proto/proto/celnet.proto`) and the shipped `RiskTransferProvenance` are the
  immutable, structured "why it happened" records carried on the deal. `HedgeProvenance`
  mirrors this discipline: *why the hedge fired, the threshold, the action taken, the fill*
  (§8).

**Net:** auto-hedging = **(reused decision-graph engine)** with **(new action leaves)**, driven
by a **(bounded async loop on the existing `risk_version` signal)**, emitting **(hedge intents)**
to the **(existing aggregation venue and existing RFQ/FIX panel)**, gated by the **(existing
limits machinery)**, audited by a **(provenance record mirroring risk-transfer)**. Nothing
touches the pinned pricing thread.

---

## 3. External research — internalise vs externalise, and how to auto-hedge (cited)

All methods below are open, academically published, and free (guardrail 7). Celnet implements
the **heuristics** the practitioner literature actually uses (clamped-linear skews, band-to-edge
hedging, impact-aware schedules), citing the paper/method in code comments and here — it does
**not** run a live HJB/stochastic-control solver on any hot path.

### 3.1 The central dilemma — internalisation vs externalisation

- **Internalisation ratio & the dealer's continuous choice.** A spot-FX (or FI) dealer that
  receives two-way client flow can **internalise** — hold the net and let subsequent opposing
  client flow offset it — or **externalise** — hedge the net into the interdealer market. The
  **internalisation ratio** (fraction of flow offset internally before hedging; operationally
  `1 − externalised/total`, and 80–95%+ at large dealers in major pairs) is the single most
  important control: a higher ratio saves market impact and information leakage but raises
  warehousing risk and inventory holding time (the **half-life of inventory**). Butz & Oomen
  (2019), *Internalisation by electronic FX spot dealers* (Quantitative Finance 19(1), 35–56;
  SSRN 3076575) formalise this — modelling the warehoused-risk process and deriving the
  internalisation ratio, the inventory half-life, and the market-impact-vs-warehousing-risk
  trade-off as a function of flow density and skewing intensity. The **direct academic basis for
  a configurable threshold** governing *how much* to internalise before externalising, and for
  exposing the internalisation ratio + inventory half-life as first-class monitored metrics (§8).
- **Optimal internalise/externalise as a control problem — the strongest anchor.** Barzykin,
  Bergault & Guéant (2023), *Algorithmic market making in dealer markets with hedging and market
  impact* (Mathematical Finance 33(1), 41–79; arXiv **2106.06974**) derive exactly this policy
  from the control problem: **within an inventory band the dealer internalises by skewing quotes,
  and only outside the band does it externalise (hedge) the excess** — an endogenous *no-hedge
  inventory band* whose width falls out of risk aversion, volatility, and external hedging
  impact/cost. This proves the "warehouse up to a threshold, then hedge the overflow" shape is
  **optimal**, not merely convenient, and names the exact knobs a trader should configure (§4).
  Cartea et al., *Competition in Dealer Markets with Internalisation and Externalisation* (arXiv
  **2606.06413**) and Cartea/Boyce, *unwinding toxic flow* (arXiv **2407.04510**) — both already
  cited in `docs/ANALYTICS-REQUIREMENTS.md §11.1` — cast the same choice as a stochastic-control
  trade of spread capture against adverse selection. Celnet implements the band (§4), not a live
  solve.
- **Soft hedging *rate*, not a binary switch.** Barzykin, Bergault & Guéant (2021), *Market
  making by an FX dealer: tiers, pricing ladders and hedging rates for optimal risk control*
  (arXiv **2112.02269**, already cited in `ANALYTICS §11.2/§11.4`) express externalisation as a
  continuous **hedging rate that ramps with how far inventory exceeds the band** — not "hedge
  everything above X" but "hedge faster the further past X you are." Celnet supports this as a
  band-and-overflow default (§4.3) with an optional **ramped hedge fraction** the policy can
  select (§5.3), so a small breach externalises gently and a large one aggressively.
- **"Back-to-back" vs "warehousing" — operational definitions (dealer parlance).**
  - **Back-to-back**: the desk immediately places an **offsetting external trade** for (some or
    all of) the risk it just took — near-zero net inventory, near-zero warehousing risk, the
    spread captured is only the difference between the client price and the hedge price. This is
    the **at/over-threshold overflow behaviour**.
  - **Warehousing**: the desk **holds** the risk (nets it against future opposing flow / lets it
    mean-revert), bearing market risk for the chance to capture more of the spread and avoid
    paying impact. This is the **below-threshold behaviour**.
  - Auto-hedging makes the switch between the two a **trader-configured, threshold-driven
    policy**, not a manual per-trade decision.

### 3.2 Hedging bands / no-trade regions under transaction costs

The foundational result for "hold within a band, hedge to the edge": because hedging is
**costly**, the optimal policy is **not** to re-hedge to flat continuously but to let the risk
drift inside a **no-trade band** and trade only enough to return to the band edge when it is
breached.

- **Davis & Norman (1990)**, *Portfolio selection with transaction costs* (Mathematics of
  Operations Research 15(4)) — the canonical **no-trade region**: under proportional costs the
  optimal control keeps the state inside a wedge and acts only at its boundary. The theoretical
  parent of "warehouse inside the band, hedge at the edge."
- **Whalley & Wilmott (1997)**, *An asymptotic analysis of an optimal hedging model for option
  pricing with transaction costs* (Mathematical Finance 7(3)) — the celebrated **hedging
  bandwidth**: the optimal band half-width around the target hedge scales like
  **`H ∝ (cost · |Γ| / risk-aversion)^{1/3}`**. The concrete formula behind a
  **transaction-cost-aware threshold** — a wider band (higher "100") when hedging is expensive
  or gamma is low; hedge back to the **band edge**, not to zero.
- **Leland (1985)**, *Option pricing and replication with transactions costs* (Journal of
  Finance 40(5)) — discrete re-hedging under costs; motivates **periodic / threshold-triggered**
  hedging over continuous.
- **Zakamouline (2006)** — utility-based **asymptotic hedging bands** with both fixed and
  proportional costs; extends Whalley–Wilmott to a fixed-cost component, which maps to a
  **minimum-clip / minimum-ticket** term on the hedge (don't fire a tiny back-to-back trade).

**Mapping to Celnet:** the threshold "100" is the **band edge**; the hedge action returns the
book to the edge (or to a configurable target inside it), not to flat — sized so a fixed
cost/minimum clip is respected (§4, §5).

### 3.3 Inventory-based optimal market making (skew before you pay to hedge)

- **Ho & Stoll (1981)**, *Optimal dealer pricing under transactions and return uncertainty*
  (Journal of Financial Economics 9(1)) — the original inventory dealer model: quotes skew with
  inventory to induce mean-reverting flow. The ancestor of the skew-to-attract lever.
- **Avellaneda & Stoikov (2008)**, *High-frequency trading in a limit order book* (Quantitative
  Finance 8(3)) — the **reservation price** `r = s − q·γ·σ²·(T−t)` and the **inventory skew**:
  lean the two-way against signed inventory `q` so the market pulls the position back toward
  neutral **for free**, before paying to hedge. This is exactly Celnet's `InventorySkew`
  (`celnet-tiering`, §3-ground) and the aggregation `AutoSkewSource` — the **passive**
  internalisation the policy tries **before** an external hedge.
- **Guéant, Lehalle & Fernandez-Tapia (2013)**, *Dealing with the inventory risk* (Mathematics
  and Financial Economics 7(4)) — closed-form approximations to Avellaneda–Stoikov **with an
  explicit inventory limit `q_max`**: the desk simply refuses to accumulate past a cap. `q_max`
  **is the warehouse threshold** in academic form — the direct justification for a hard cap
  above the soft band.
- **Guéant (2017)**, *Optimal market making* (Applied Mathematical Finance 24(2)) — multi-asset
  market making including the hedging of accumulated risk; supports **per-instrument** and
  **portfolio-level** thresholds (net DV01 across a book, not just per-line).
- **Cartea, Jaimungal & Penalva (2015)**, *Algorithmic and High-Frequency Trading* (Cambridge
  University Press) — the standard text: inventory penalties, running/terminal risk penalties,
  and the skew/hedge trade-off; the reference frame for the whole design.
- **Bergault et al.** (arXiv **1810.04383**, already cited in
  `crates/celnet-tiering/src/strategy.rs:13`) — closed-form multi-asset market-making skews;
  the provenance already in Celnet's tiering layer.

### 3.4 Optimal execution of the hedge (once you decide to externalise)

Deciding to hedge the overflow is one thing; **executing** it without moving the market is
another. When the overflow is large relative to liquidity, the hedge should be **worked**, not
dumped.

- **Almgren & Chriss (2000)**, *Optimal execution of portfolio transactions* (Journal of Risk
  3(2)) — the **efficient frontier** of execution: a schedule trading **temporary + permanent
  market impact** against **timing risk**, giving the optimal trade-off between hedging fast
  (certain but costly) and slow (cheaper but riskier). A `WORK_ORDER` / sliced hedge action
  uses this schedule; a `SUBMIT_MARKET_ORDER` is the immediate (back-to-back) special case.
- **Almgren (2003)**, *Optimal execution with nonlinear impact functions and trading-enhanced
  risk* (Applied Mathematical Finance 10) — nonlinear (concave) impact, relevant to sizing the
  hedge clip.
- **Obizhaeva & Wang (2013)**, *Optimal trading strategy and supply/demand dynamics* (Journal of
  Financial Markets) — impact with limit-order-book resilience; informs the minimum spacing
  between successive hedge clips.

**Mapping to Celnet:** the external hedge action carries an **execution style** — `IMMEDIATE`
(back-to-back, one clip onto the RFQ panel) or `WORKED` (an Almgren–Chriss-scheduled series of
clips) — chosen in the policy (§5).

### 3.5 Adverse selection / flow toxicity (why the threshold must adapt)

The right threshold depends on **who** you are trading against. Toxic (informed) flow should be
hedged sooner (a **lower** threshold); benign flow can be warehoused longer (a **higher**
threshold).

- **Glosten & Milgrom (1985)**, *Bid, ask and transaction prices in a specialist market with
  heterogeneously informed traders* (Journal of Financial Economics 14(1)) — the origin of
  **adverse selection** in dealer spreads: the price must protect against informed
  counterparties.
- **Easley, López de Prado & O'Hara (2012)**, *Flow toxicity and liquidity in a high-frequency
  world* (Review of Financial Studies 25(5)) — **VPIN** (volume-synchronised probability of
  informed trading): a real-time toxicity gauge. A high-VPIN / bad-markout counterparty warrants
  a **lower internalisation threshold** for its flow.
- **Per-trade, signal-driven internalise/externalise.** Cartea, Duran-Martin &
  Sánchez-Betancourt (2023), *Detecting Toxic Flow* (arXiv **2312.05827**), and Cartea &
  Sánchez-Betancourt (2024), *Brokers and Informed Traders: Dealing with Toxic Flow and
  Extracting Trading Signals* (SIAM J. Financial Mathematics, DOI 10.1137/24M1660243) treat the
  decision **per ticket**: predict each fill's toxicity and **internalise benign flow,
  externalise toxic flow** — externalising the toxic subset maximises P&L even when aggregate
  inventory is inside the band. This makes the threshold **flow-conditional**, not just
  size-conditional (a toxic ticket hedges out early; benign flow warehouses longer).
- **Markout / residual-toxicity** per client — already specified in
  `docs/ANALYTICS-REQUIREMENTS.md §11.1` (arXiv **2312.05827** / Quantitative Finance 2026), and
  the practitioner standard in electronic FX (mid-drift over post-fill horizons; the horizon at
  which drift turns adverse sets the **max hold time before a forced hedge**). The **toxicity
  signal is a routable field** in the hedge graph (§5.2): `IF counterparty_toxicity > X THEN
  hedge sooner` — even inside the size band.

---

## 4. The threshold & warehouse-limit model — the configurable "100"

The threshold is a **soft, banded risk budget** per (book × instrument-or-metric), reusing the
`celnet-limits` vocabulary (`limit.rs`) rather than a parallel one.

### 4.1 What the threshold measures (`LimitMetric`, `limit.rs:39`)

- **FI / rates:** **net DV01** (per book) or **key-rate DV01** per tenor bucket (the natural
  first-order rate risk; already surfaced `book_risk.rs:157/237`, laddered in
  `celnet-rates-risk`). *("internalise up to 100 [k DV01], then back-to-back.")*
- **FX / cash:** **net notional** or **net delta** (base-currency), and per-greek caps
  (delta/vega) for options.
- **Notional:** net or gross notional per instrument / per book — the simplest form of "100".

The metric is chosen per rule, so one desk warehouses to a DV01 budget while another uses net
notional — both expressed in the same `LimitMetric` enum.

### 4.2 Bands, not a single number (`LimitSpec::with_bands`, `limit.rs:186`)

A warehouse threshold is a **soft** `LimitSpec` (`limit.rs:172`) with two bands:

| Band | Condition | Behaviour |
|---|---|---|
| **Green** (`&#124;risk&#124; < amber`) | comfortably inside budget | **warehouse** — hold, capture spread, let it mean-revert |
| **Amber** (`amber ≤ &#124;risk&#124; < red`) | approaching the budget | **skew to attract offset** — lean the two-way (aggregation `AutoSkewSource` / tiering `InventorySkew`) to pull in the internalising side **for free** (§3.3) |
| **Red** (`&#124;risk&#124; ≥ red = the "100"`) | at/over the budget | **hedge the overflow** — the exit-policy graph fires (§5); net internal first, externalise the residual (§6) |

The **hard** cap (`LimitSpec::hard`, `limit.rs:160`, enforced by `project_risk_book_breach`,
`store.rs:1274`) remains the absolute ceiling that *rejects a fill* — auto-hedging keeps the
book comfortably below it. The soft banded threshold is the **auto-hedge trigger**; the hard cap
is the **backstop**.

### 4.3 Overflow, not flatten — hedge to the band edge (Whalley–Wilmott, §3.2)

When the red band fires, the default hedge size is the **overflow** — the amount by which
`|risk|` exceeds a configurable **target** (the band edge, or a `target_fraction` of it), **not**
the whole position:

```
overflow = |net_risk| − target          (target defaults to the amber edge)
hedge_size = clamp(overflow, min_clip, max_clip)
```

`min_clip` respects the fixed-cost / minimum-ticket term (Zakamouline, §3.2) — don't fire a
trivially small back-to-back. `max_clip` bounds a single hedge; a larger overflow is worked
(Almgren–Chriss, §3.4). Hedging to the edge (not to flat) is the transaction-cost-optimal band
policy — it avoids over-trading the position back and forth across the boundary.

**Optional ramped fraction (soft externalisation, Barzykin et al. 2021, §3.1).** Instead of
hedging the whole overflow at once, a policy may hedge a **fraction that ramps with utilisation**
— `hedge_fraction = clamp(k · (utilization − 1), 0, 1)` — so a small red breach externalises
gently and a large one aggressively, approximating the continuous hedging-rate control. This is
a per-rule option on the external action (§5.3), defaulting off (hedge the full overflow).

### 4.4 Per-scope resolution

Thresholds resolve **most-specific-wins**: `instrument` overrides `book` overrides `desk`
overrides a firm default — the same precedence style the pricing-group / routing configs use.
An instrument with no explicit threshold inherits its book's; a book inherits its desk's.

---

## 5. The exit policy as decision-graph building blocks (the key UX ask)

A trader states **how to exit risk** by building a graph of the **same shape** as a routing
graph — in the **same editor** (`gui/src/workspaces/riskrouting/`) — but the **leaves are exit
actions** instead of book targets. This is the explicit ask: *"traders configure the exit policy
using risk-routing-style building blocks."*

### 5.1 `celnet-hedge-routing` (NEW leaf crate) — mirrors `celnet-risk-routing`

Same five-file shape (`graph / field / router / context / lib`), same `RouteOp` / `RouteValue`
reused verbatim, same `validate`-collects-all-defects discipline, same bounded acyclic walk.

```rust
// The one structural difference from risk-routing: leaves are ACTIONS, not book ids.
pub enum HedgeNode {
    Condition { field: HedgeField, op: RouteOp, value: RouteValue,
                on_true: NodeId, on_false: NodeId },   // reuse RouteOp/RouteValue unchanged
    Action { exit: ExitAction },                        // terminal leaf (§5.3)
}
```

`HedgeGraph::validate(&known_books, &known_venues, &known_lps)` reuses the routing validator
almost verbatim, plus: an `Action` leaf's venue/LP targets must exist and be enabled; a
`SUBMIT_MARKET_ORDER` must name a reachable panel/LP; a `CROSS_INTERNAL` must name an
aggregation instrument that exists. All defects returned at once (mirrors `graph.rs:225`).

### 5.2 `HedgeContext` — the risk-state snapshot the graph reads

Where `RoutingContext` (`context.rs:21`) is a **per-fill** snapshot, `HedgeContext` is a
**per-book/per-instrument risk-state** snapshot, built off-core from the position store +
aggregation + analytics on each `risk_version` bump:

```rust
pub struct HedgeContext {
    // identity
    instrument_id: String, ccy: String, product: String, book: String, desk: String,
    counterparty: String,     // originating party-id of the flow (HedgeField tag 18) — hedge by counterparty
    // risk state (the numbers the policy branches on)
    net_dv01: f64,            // signed net DV01 (FI)
    net_notional: f64,        // signed net notional / delta (FX)
    net_vega: f64, net_gamma: f64,
    inventory_sign: f64,      // +1 long / −1 short
    // budget state
    threshold: f64,           // the resolved "100" for this scope (§4)
    utilization: f64,         // |risk| / threshold  (RagStatus green/amber/red)
    overflow: f64,            // max(0, |risk| − target)
    breached: bool,           // utilization ≥ red band
    // flow-quality state (from analytics §11.1)
    counterparty_toxicity: f64,   // markout / residual toxicity of the flow that built this
    inventory_age_secs: f64,      // how long this risk has sat (§11.2 aging)
    // market state
    internal_offset_available: f64, // opposing internal flow the aggregator could cross now
    hedge_cost_bp: f64,             // current external hedge cost estimate (spread+impact)
}
```

Each field is one `HedgeField`; `HedgeContext::get(field)` projects it to a `CtxValue`
exactly as `RoutingContext::get` does (`context.rs:53`). The string identity fields
(`instrument_id`, `counterparty`) support `= ≠ contains in`; the enum identity fields
(`ccy`/`product`/`book`/`desk`) support `= ≠ in`. The **field palette** the trader drags
from (`gui/src/workspaces/hedging/HedgeFieldPalette.tsx`) simply lists these fields instead of the
routing ones.

### 5.3 `ExitAction` — the new action vocabulary (the leaves)

The heart of the design. A terminal leaf is one exit action:

| Action | Meaning | Venue | Notes |
|---|---|---|---|
| **`WAREHOUSE`** | Hold — do nothing, keep the risk | — | the default/green-band leaf; explicit so the graph is total |
| **`CROSS_INTERNAL { instrument, max_size }`** | Offset against **opposing internal flow / other desks** in the Agg Book | internal aggregator (`celnet-aggregation`) | netting first (§6.1); books an internal cross like a risk-transfer economic leg |
| **`SKEW { bp &#124; to_edge }`** | Lean the two-way to **attract** the offsetting side (passive internalisation) | pricing (aggregation `AutoSkewSource` / tiering `InventorySkew`) | the amber-band lever; no trade, just a quote lean |
| **`SUBMIT_MARKET_ORDER { size, style }`** | **Back-to-back**: place an offsetting external order | RFQ/FIX panel (`celnet-rfq`) | `style = IMMEDIATE` (one clip) or `WORKED` (Almgren–Chriss slices, §3.4) |
| **`RFQ_OUT { lps, size }`** | Request a two-way from named external LPs and lift the best to hedge | RFQ panel (`MultiDealerEngine`) | ranked panel, last-look; the FI/large-clip externalisation path |
| **`SPLIT { internal_first, then_external }`** | Net internally up to `internal_offset_available`, externalise the residual | both | the composite "internalise then hedge overflow" primitive (§6) |
| **`ESCALATE { reason }`** | Fire an alert / route to a human desk instead of auto-acting | notification broker | for toxic/large/illiquid overflow the desk wants to hand-manage |

`SUBMIT_MARKET_ORDER` and `RFQ_OUT` carry a `style` (execution schedule) and inherit the
`min_clip`/`max_clip` sizing from §4.3. A `size` of `Overflow` (the default) hedges to the band
edge; `Full` flattens.

### 5.4 First-terminal-leaf semantics (identical to routing)

The engine walks the graph from `entry` against the `HedgeContext`, one edge per `Condition`
node, until it hits an `Action` leaf — the **first (and only) action** for that risk state
(the routing engine's exact `RiskRouter::route` shape, `router.rs:27`, with the same step-cap
cycle guard). A validated graph is total: every risk state reaches exactly one action. Example a
trader builds by drag-and-drop:

```
IF breached == false            → WAREHOUSE
ELSE IF counterparty_toxicity > 0.6   → SUBMIT_MARKET_ORDER { size: Overflow, style: IMMEDIATE }   // toxic → back-to-back now
ELSE IF internal_offset_available > overflow → CROSS_INTERNAL { max_size: Overflow }               // benign & offset exists → net internally
ELSE IF overflow > large_clip   → RFQ_OUT { lps: [LP-1, LP-2, LP-3], size: Overflow }              // big residual → work an RFQ
ELSE                            → SPLIT { internal_first, then_external }                            // net what we can, hedge the rest
```

### 5.5 GUI — reuse the shipped routing editor

The drag-and-drop builder already exists for routing: `RiskRoutingWorkspace.tsx`,
`FieldPalette.tsx`, `RuleEditor.tsx`, `RiskRulesTable.tsx`, `ValueEditor.tsx`, `graphOps.ts`
(`gui/src/workspaces/riskrouting/`). The hedge editor is the **same components** parameterised
with the `HedgeField` palette and the `ExitAction` leaf editor (an action picker + its
venue/size/style fields) in place of the book-target leaf. A **live trace** panel (as risk
routing has) shows, for the current book's risk state, which path fires and which action would
result — before the trader arms it.

---

## 6. Exit venues & offsetting mechanics — net internal first, hedge the residual

The engine always **internalises before it externalises**, per the internalisation-ratio
literature (§3.1): paying the street is the last resort.

### 6.1 Internal offset — the Agg Book as a cross venue

`CROSS_INTERNAL` books an **internal cross** against opposing flow the aggregator holds
(another desk long where we are short, or opposing client flow queued): two offsetting
bookings through the **existing** booking sinks (the risk-transfer economic-leg mechanic,
`store.rs:690` / `book_rates_position`), struck at the `ConsolidatedBook` mid
(`celnet-aggregation`, `consolidate.rs`). The source book flattens the crossed quantity; the
counterparty book opens it. This reuses the risk-transfer offsetting-booking machinery — it is
a **desk-to-desk economic transfer fired automatically** by the hedge policy instead of manually.

### 6.2 External hedge — the RFQ/FIX panel

`SUBMIT_MARKET_ORDER` / `RFQ_OUT` place the residual onto the **existing** `MultiDealerEngine`
panel (`celnet-rfq/src/panel.rs`): fan to the configured LP `QuoteSource`s (`FixLpAdapter` real
FIX 4.4, `lp_fix.rs`), rank, last-look, lift the best. The native `InternalPricerSource`
(`internal.rs`) guarantees a market always exists. `WORKED` style slices the clip on an
Almgren–Chriss schedule (§3.4); `IMMEDIATE` lifts one clip (true back-to-back). The resulting
external fill **books back into the same book**, flattening the overflow.

### 6.3 The netting waterfall (the "internalise-then-hedge" primitive)

```
1. compute net_risk for (book, instrument)            ← position store / book_risk.rs
2. resolve threshold + bands for the scope             ← §4 (celnet-limits)
3. if green      → WAREHOUSE (hold)
4. if amber      → SKEW to attract offset (passive internalisation; AutoSkewSource)   ← §3.3
5. if red (breached):
   a. internal_offset = opposing internal flow available now
   b. cross min(overflow, internal_offset) INTERNALLY  ← §6.1 (saves impact + leakage)
   c. residual = overflow − crossed
   d. hedge residual EXTERNALLY per the policy leaf     ← §6.2 (IMMEDIATE or WORKED)
   e. book both legs, bump risk_version, stamp HedgeProvenance
```

Step 4's skew is the **cheapest** internalisation (no trade at all — just lean the price so the
market brings the offset). Step 5b is the next cheapest (internal cross, mid, no external
impact). Step 5d is the last resort (pay the street). The trader's graph (§5) chooses *which*
of these the policy uses and in what order — the waterfall above is the default the graph can
override.

### 6.4 Interaction with the existing inventory skew

`celnet-tiering::InventorySkew` (`strategy.rs:91`, `s = clamp(κ·q, ±s_max)`) and the aggregation
`AutoSkewSource` (`risk.rs:48`) **already** lean quotes off inventory. Auto-hedging's `SKEW`
action and its amber-band behaviour **drive those existing knobs** (set `skew_bp` / implement
`auto_skew_bp` from the live net position) rather than adding a second skew path — one skew,
one source of truth. The auto-hedge engine is the long-awaited concrete consumer of the
`AutoSkewSource` seam (`risk.rs:38` doc: *"a later lane implements AutoSkewSource over the rates
position store to lean the axe automatically off live net inventory"*).

---

## 7. Architecture — the auto-hedge engine (off the hot core)

A single `AutoHedgeEngine` (a server service, not a pricing-crate primitive) runs a **bounded
async control loop**, mirroring the `RiskTransferBroker` / `NotificationBroker` pattern
(`risk_transfer/broker.rs`, `desk/notify.rs`): version-gated, `try_send`-style non-blocking,
held **entirely off the pinned pricing core** (guardrail 11).

### 7.1 The control loop

```
loop (async, one task; NOT the pricing thread):
  await risk_version change    ← the same signal the live risk stream watches
                                  (combined_risk_version, stream.rs:1210; bumped by
                                   PositionStore::book, store.rs, on every fill/transfer)
  for each (book, instrument) whose risk moved:
      ctx  = build HedgeContext (position store + aggregation + analytics)   ← §5.2, off-core
      band = classify(|risk|, threshold)                                     ← §4 (RagStatus)
      if band == green: continue
      action = HedgeRouter::resolve(policy_graph, ctx)                       ← §5.4 pure walk
      intent = HedgeIntent { book, instrument, action, ctx_snapshot }
      enqueue intent on the bounded hedge-intent queue (try_send, never block)
  # a separate executor task drains intents and applies them:
  drain intent:
      run the netting waterfall (§6.3): internal cross + external RFQ/FIX
      book the legs through the EXISTING sinks; re-check the target book's hard cap
      bump risk_version; stamp HedgeProvenance (§8)
```

Splitting **decision** (loop) from **execution** (drain) keeps the loop cheap and bounds
concurrency on the external order path. The engine is **event-driven** (reacts to
`risk_version`), not a busy timer — no allocation/log/lock on the pricing path.

### 7.2 Where it sits

```
   fills ──routing (celnet-risk-routing)──▶ PositionStore / rates_book  ──bump risk_version──┐
                                                    │  (net DV01/greeks, book_risk.rs)         │
                                                    ▼                                          │
                                          ┌───────────────────────┐   watches risk_version ◀──┘
                                          │   AutoHedgeEngine      │
                                          │  (bounded async loop)  │
                                          │  HedgeContext + policy │
                                          └───────┬───────┬───────┘
                                     CROSS_INTERNAL│       │SUBMIT_MARKET_ORDER / RFQ_OUT
                                                   ▼       ▼
                            internal venue: Agg Book     external venue: RFQ/FIX panel
                            (celnet-aggregation)          (celnet-rfq MultiDealerEngine,
                            offsetting booking @ mid       FixLpAdapter real FIX 4.4)
                                                   │       │
                                                   ▼       ▼
                                            legs booked → risk_version bump → HedgeProvenance
```

It is a **peer** of `celnet-risk-routing` (which feeds it inventory) and `celnet-risk-transfer`
(which shares its offsetting-booking machinery), consuming `celnet-aggregation` (internal venue)
and `celnet-rfq` (external venue), gated by `celnet-limits`.

### 7.3 Off-core guarantees (guardrail 11)

- The pricing hot core is untouched: the engine reads the **already-published** `risk_version`
  and the position snapshots the risk stream already builds — it adds no work to any fill's
  synchronous booking path.
- Bounded queues + `try_send` (mirror `INBOX_QUEUE_DEPTH = 128`, `broker.rs:32`): a slow external
  hedge never back-pressures the desk or the pricing loop.
- The external order path (FIX round-trips) runs on the async I/O tier the RFQ panel already
  uses (`join_all` over the panel, `panel.rs`), never inline with pricing.

---

## 8. Provenance, controls & safety

Every auto-hedge is a **P&L-moving, position-changing automated action** — it must be as
auditable and as gated as a manual transfer.

### 8.1 `HedgeProvenance` (immutable audit record — mirrors `RiskTransferProvenance`)

Stamped on every fired hedge and carried on the resulting deal legs:

```
HedgeProvenance {
  hedge_id, book, instrument, fired_at,
  trigger { metric, threshold, net_risk, utilization, band },     // WHY it fired (§4)
  policy_path: Vec<NodeId>,          // the exact graph path walked (§5.4) — the "why this action"
  action: ExitAction,                // WHAT it did (§5.3)
  netting { internal_crossed, external_hedged, residual },        // the waterfall outcome (§6.3)
  fills: Vec<FillRef>,               // the internal cross legs + external LP fills
  cost { hedge_price, mid_at_fire, slippage_bp, lp_won },         // realised hedge cost
}
```

Same discipline as `PricingProvenance` / `RiskTransferProvenance`: structured, additive,
immutable, surfaced on the blotter and the risk dashboard — so a desk head can answer *"why did
the system hedge EURUSD at 14:32, at what price, on whose policy?"* exactly.

### 8.2 Capability gating

An auto-hedge is a booking-class write, so it gates on `Action::Book`
(`crates/celnet-entitlements/src/capability.rs`) × the position's `AssetClass` — the same
authority manual booking and transfer require. **Arming or editing a hedge policy** gates on a
narrow new sub-capability (a `Hedge` action, or `Administer` on the desk) so that *running*
inside a policy (booking) is separable from *authoring* the policy — decided at review (§11).

### 8.3 Limits are never bypassed

Every internal-cross / external-hedge leg re-runs the **target book's** hard-limit gate
(`project_risk_book_breach`, `store.rs:1274`) exactly like a routed fill — an auto-hedge can
never itself breach a cap. The soft banded threshold triggers the hedge; the hard cap remains
the backstop (§4.2).

### 8.4 Kill-switch, advisory mode & rate limits

- **Advisory / dry-run mode.** A policy can be armed **advisory-only**: the engine computes and
  **logs the `HedgeIntent` + `HedgeProvenance` it *would* have fired** but does not trade —
  the mandatory shadow-run before a desk lets it act for real. (Mirrors the analytics §11.1
  stance: surface the signal, human-approved actuation.)
- **Global + per-desk kill-switch.** A single flag halts all auto-hedging instantly (positions
  simply warehouse); per-desk and per-book toggles.
- **Rate / size guards.** Max hedges per interval, max clip, min spacing between clips
  (Obizhaeva–Wang, §3.4), and a daily externalised-notional cap — so a mis-configured policy
  or a market gap can't machine-gun the LP panel.
- **Toxic/large escalation.** The `ESCALATE` action (and a configurable "overflow larger than X
  ⇒ escalate not auto-hedge" guard) hands unusual risk to a human via the notification broker
  rather than auto-acting.

---

## 9. OSS / licensing verdict (guardrail 7)

- **All methods are open and academically published** (§3): the warehouse-band-then-hedge policy
  itself (Barzykin–Bergault–Guéant 2023, arXiv 2106.06974 — the optimality proof; 2021 arXiv
  2112.02269 — the soft hedging-rate form), inventory market-making (Ho–Stoll,
  Avellaneda–Stoikov, Guéant–Lehalle–Fernandez-Tapia, Guéant, Cartea–Jaimungal–Penalva),
  transaction-cost hedging bands (Davis–Norman, Whalley–Wilmott, Leland, Zakamouline), optimal
  execution (Almgren–Chriss, Almgren, Obizhaeva–Wang), toxicity (Glosten–Milgrom, Easley–López
  de Prado–O'Hara VPIN, markout), and internalise/externalise (Butz–Oomen; Cartea et al. arXiv
  2606.06413 / 2407.04510 / 2312.05827). Each is cited in code comments and here; **no method
  identifier carries a person/paper name** (guardrail 8) — provenance is doc-only.
- **No commercial solver, no paid data, no proprietary SDK** anywhere on the path. Celnet
  implements the **practitioner heuristics** (clamped-linear inventory skew already in
  `celnet-tiering`; band-to-edge threshold hedging; an Almgren–Chriss *schedule*, not a live
  stochastic-control solve) — no HJB solver runs on any hot path, matching the tiering crate's
  existing stance (`strategy.rs:13`: *"real desks use clamped linear heuristics rather than
  solving the HJB"*).
- **Reuses only in-repo, permissively-licensed crates** (`celnet-risk-routing`,
  `celnet-aggregation`, `celnet-rfq`, `celnet-fix`, `celnet-limits`, `celnet-rates-risk`) — no
  new third-party dependency is required. `cargo-deny` license policy continues to hold.

---

## 10. Crate / workstream breakdown (parallel-safe, disjoint files)

1. **`celnet-hedge-routing`** (NEW leaf crate, models `celnet-risk-routing` file-for-file):
   `HedgeGraph` / `HedgeNode` / `HedgeField` / `HedgeContext` / `ExitAction` / `HedgeRouter`,
   reusing `RouteOp` + `RouteValue` from `celnet-risk-routing` (or a shared `celnet-decision`
   primitive — §11). Pure, no server deps → independent oracle = a hand-written truth table of
   `(risk-state, policy) → action`. `validate` collects all defects (mirror `graph.rs:225`).
2. **`celnet-auto-hedge`** (NEW leaf crate): the pure **netting waterfall** (§6.3) and **overflow
   sizing** (§4.3) — `(net_risk, threshold, bands, internal_offset, policy-action) → (cross_size,
   external_size, style)`. No I/O; oracle = a truth table of scenarios → hedge decomposition.
3. **`services/auto_hedge/`** (server): the `AutoHedgeEngine` control loop (§7) — watches
   `risk_version`, builds `HedgeContext`, resolves the policy, drains intents, applies legs
   through the existing sinks (`book_from_attribution` / `book_rates_position`), re-checks caps,
   bumps `risk_version`, stamps `HedgeProvenance`. Bounded queues + kill-switch + advisory mode.
4. **`celnet-aggregation` wiring**: implement `AutoSkewSource` (`risk.rs:48`) off the live net
   position (the `SKEW`/amber lever); expose the internal-cross booking.
5. **`celnet-rfq` wiring**: the `SUBMIT_MARKET_ORDER` / `RFQ_OUT` submission path onto the
   `MultiDealerEngine` panel + the `WORKED` (Almgren–Chriss) scheduler.
6. **`config/identity.rs`**: a `hedge_policy_graph` sibling field to `risk_routing_graph`
   (`identity.rs:907`) + `set_hedge_policy_graph` → `check_hedge_policy_graph` validation.
7. **`celnet-entitlements`**: the `Hedge` sub-capability (author-policy vs run-policy split).
8. **proto + `ws/` codecs + services**: `HedgeGraph` CRUD, `HedgeProvenance`, the advisory-intent
   stream, kill-switch RPCs (mirror the risk-routing / risk-transfer proto+WS precedent).
9. **GUI**: the hedge-policy builder (reuse `gui/src/workspaces/riskrouting/` components with the
   `HedgeField` palette + `ExitAction` leaf editor), a live trace/what-would-fire panel, the
   advisory-run viewer, the per-book threshold config, and the audit trail of fired hedges.

Each phase gated (`just t1` per crate; `just t2` at land). Numerical claims (overflow sizing,
netting decomposition, band classification) validated against the independent oracle, never
merely asserted (guardrail 5). GUI verified live end-to-end.

### Suggested phasing

- **P0 — engine skeleton + threshold model.** `celnet-hedge-routing` + `celnet-auto-hedge` pure
  crates (oracle-gated) + the banded threshold config on `celnet-limits`. No trading yet.
- **P1 — advisory mode.** The `AutoHedgeEngine` control loop in **dry-run**: build `HedgeContext`
  on `risk_version`, resolve the policy, emit `HedgeIntent` + `HedgeProvenance` to a stream, book
  nothing. Ship the GUI builder + trace + advisory viewer. Desks watch it shadow real flow.
- **P2 — internal offset (`CROSS_INTERNAL` + `SKEW`).** The cheapest exits first: wire the
  `AutoSkewSource` amber lever and the internal-cross booking (reusing risk-transfer legs). Still
  no external trading.
- **P3 — external hedge (`SUBMIT_MARKET_ORDER` `IMMEDIATE`).** True back-to-back onto the RFQ/FIX
  panel, behind the kill-switch + rate guards. Live LP connectivity stays ENV-gated (`lp_fix.rs:25`).
- **P4 — worked execution + `RFQ_OUT` + toxicity-adaptive thresholds.** Almgren–Chriss slicing,
  multi-LP RFQ externalisation, and the analytics toxicity signal feeding the threshold.

---

## 11. Open items for the next review

- **Share the decision-graph engine or fork it?** `celnet-hedge-routing` reuses `RouteOp` /
  `RouteValue` / the walk / the validator almost verbatim. Cleanest is a shared
  **`celnet-decision`** primitive crate that both `celnet-risk-routing` and `celnet-hedge-routing`
  depend on (leaf = generic `L`), vs. copying the ~200 lines. Leaning: extract `celnet-decision`
  now, before a third consumer appears. (DRY vs. an extra crate boundary.)
- **Threshold metric per asset.** Confirm the canonical budget metric per desk: net DV01 /
  key-rate DV01 (FI) vs net delta / vega / notional (FX-options) — and whether a **portfolio**
  (cross-instrument) DV01 budget is a first-class scope or only per-instrument/per-book.
- **Hedge-to-edge target vs flatten.** Default `target = amber edge` (hedge the overflow) vs a
  configurable `target_fraction`; and whether some desks want **full flatten** on a red breach.
- **Skew vs hedge ordering as policy vs default.** The §6.3 waterfall (skew → internal cross →
  external) is the default; how much of that ordering is fixed vs. expressed in the trader's
  graph (leaning: graph chooses, waterfall is the fallback).
- **Capability granularity.** New `Hedge` action (author-policy) vs `Book` (run) vs gating
  policy-authoring on `Administer` — and whether advisory-mode arming needs a lower bar than
  live-mode arming.
- **Toxicity signal source & latency.** Whether the markout/residual-toxicity from
  `ANALYTICS-REQUIREMENTS §11.1` is available to the `HedgeContext` in time to adapt the
  threshold intra-day, or is a slower human-tuned input first.
- **Internal-offset discovery.** How the engine sees "opposing internal flow available now"
  (`internal_offset_available`) — a live query across other desks' books via
  `positions_in_risk_book` (`store.rs:519`), vs a maintained cross-desk net cache. Interaction
  with the risk-transfer four-eyes rule when a `CROSS_INTERNAL` moves risk to *another desk's*
  book (does an auto-cross need the target desk's standing consent?).
- **Worked-hedge state across restarts.** A `WORKED` (sliced) external hedge is multi-step; where
  its in-flight state lives so a server restart/hot-upgrade resumes or cleanly cancels it.
- **Netting vs booking atomicity.** The internal-cross + external-residual legs of one red-band
  event should commit as a linked set (mirror the risk-transfer staged-then-apply-atomically
  rule, `RISK-TRANSFER §6.2`) — confirm the rollback boundary when the external leg partially
  fills.

---

## 12. References (verified against primary sources)

Provenance is doc-only; **no method identifier in code carries a person/paper name** (guardrail
8). All are open/published; none is a commercial product (guardrail 7).

**Internalise vs externalise (the core policy):**
1. Barzykin, A., Bergault, P. & Guéant, O. (2023). *Algorithmic market making in dealer markets
   with hedging and market impact.* Mathematical Finance 33(1), 41–79. arXiv:2106.06974. — the
   optimality proof for the warehouse-band-then-hedge-overflow policy.
2. Barzykin, A., Bergault, P. & Guéant, O. (2021). *Market making by an FX dealer: tiers,
   pricing ladders and hedging rates for optimal risk control.* arXiv:2112.02269 (multi-currency
   companion arXiv:2207.04100). — externalisation as a continuous hedging *rate*.
3. Butz, M. & Oomen, R. C. A. (2019). *Internalisation by electronic FX spot dealers.*
   Quantitative Finance 19(1), 35–56. SSRN 3076575. — internalisation ratio, inventory half-life.
4. Cartea, Á. et al. *Competition in Dealer Markets with Internalisation and Externalisation.*
   arXiv:2606.06413; and *unwinding toxic flow* arXiv:2407.04510 (both per `ANALYTICS §11.1`).

**Hedging bands / no-trade region under transaction costs (the threshold):**
5. Davis, M. H. A. & Norman, A. R. (1990). *Portfolio selection with transaction costs.* Math.
   of Operations Research 15(4), 676–713. — the no-trade region.
6. Whalley, A. E. & Wilmott, P. (1997). *An asymptotic analysis of an optimal hedging model …
   with transaction costs.* Mathematical Finance 7(3), 307–324. — band width ∝ (cost/(γ·Γ²))^{1/3}.
7. Zakamouline, V. I. (2006). *Optimal hedging of option portfolios with transaction costs.*
   SSRN 938934. — implementable utility-based band; rehedge to the edge.
8. Leland, H. E. (1985). *Option pricing and replication with transactions costs.* Journal of
   Finance 40(5), 1283–1301. — discrete cost-aware rehedging.

**Inventory-based market making (pricing inside the band):**
9. Ho, T. & Stoll, H. (1981). *Optimal dealer pricing under transactions and return
   uncertainty.* Journal of Financial Economics 9(1), 47–73.
10. Avellaneda, M. & Stoikov, S. (2008). *High-frequency trading in a limit order book.*
    Quantitative Finance 8(3), 217–224. — reservation-price inventory skew.
11. Guéant, O., Lehalle, C.-A. & Fernandez-Tapia, J. (2013). *Dealing with the inventory risk.*
    Mathematics and Financial Economics 7(4), 477–507. arXiv:1105.3115. — hard inventory cap ±Q.
12. Guéant, O. (2017). *Optimal market making.* Applied Mathematical Finance 24(2), 112–154.
    arXiv:1605.01862. — multi-asset / portfolio-level bands.
13. Cartea, Á., Jaimungal, S. & Penalva, J. (2015). *Algorithmic and High-Frequency Trading.*
    Cambridge University Press. — inventory-penalty formalism.
14. Bergault, P. et al. (2018). arXiv:1810.04383 (already in `celnet-tiering/src/strategy.rs`).

**Optimal execution of the hedge (the overflow):**
15. Almgren, R. & Chriss, N. (2000). *Optimal execution of portfolio transactions.* Journal of
    Risk 3(2), 5–39. — the impact-vs-timing-risk efficient frontier.
16. Almgren, R. F. (2003). *Optimal execution with nonlinear impact functions …* Applied
    Mathematical Finance 10(1), 1–18.
17. Obizhaeva, A. A. & Wang, J. (2013). *Optimal trading strategy and supply/demand dynamics.*
    Journal of Financial Markets 16(1), 1–32. — pace against LOB resilience.

**Adverse selection / flow toxicity (why the threshold adapts):**
18. Glosten, L. R. & Milgrom, P. R. (1985). *Bid, ask and transaction prices …* Journal of
    Financial Economics 14(1), 71–100. — adverse-selection spread.
19. Easley, D., López de Prado, M. & O'Hara, M. (2012). *Flow toxicity and liquidity in a
    high-frequency world.* Review of Financial Studies 25(5), 1457–1493. — VPIN.
20. Cartea, Á., Duran-Martin, G. & Sánchez-Betancourt, L. (2023). *Detecting Toxic Flow.*
    arXiv:2312.05827; and Cartea & Sánchez-Betancourt (2024), *Brokers and Informed Traders*,
    SIAM J. Financial Mathematics, DOI 10.1137/24M1660243. — per-trade toxicity-driven exit.
21. Markout (practitioner): mid-drift over post-fill horizons for FX flow toxicity / max hold
    time (Databento microstructure guide; Risk.net; FX Markets).
