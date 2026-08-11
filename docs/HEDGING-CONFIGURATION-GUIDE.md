# Auto-Hedging — Trader Configuration Guide

> A practical, step-by-step guide to setting up auto-hedging on Celnet. It mirrors what
> actually ships in the **Hedging** workspace (three tabs: **Exit Policy**, **Thresholds**,
> **Monitor**) and the `celnet-hedge-routing` / `celnet-auto-hedge` engine behind it. Where
> the built feature is advisory-only or a deliberate follow-up, this guide says so plainly.
> Design rationale and the academic basis live in
> `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`; this is the how-to.
>
> **To understand what the system does with your configuration** — how risk is measured,
> how the bands and the exit policy combine, and the exact mechanism by which risk leaves a
> book — read [`HEDGING-AND-RISK-EXIT.md`](HEDGING-AND-RISK-EXIT.md). That is the as-built
> explanation; this is the control-by-control walkthrough.

---

## The idea in one line

> **Internalise the risk you capture up to a threshold, then hedge the overflow above it.**

You warehouse client flow while it is comfortably inside a risk budget (capturing the
spread and letting opposing flow net it off). As the net risk approaches the budget you
**skew** your quotes to attract the offsetting side for free. When it crosses the budget
you **hedge the overflow** — first by crossing internally, then by paying the street for
whatever is left. You author *how* to exit with the **same drag-and-drop decision-graph
building blocks** as risk routing — only the leaves are **exit actions** instead of book
targets.

### Where to find it

Open the **Hedging Rules** workspace. It has six tabs — together they answer *when* to
hedge, *what kind* of exit, *with what*, and *whether it fires by itself*:

| Tab | What you do there |
|---|---|
| **Exit Policy** | Build the exit-policy graph (the rules) + a live "what would fire" trace. |
| **Thresholds** | Set the per-scope warehouse budgets (the "100") and bands. |
| **Vehicles** | Register **what each class of risk is hedged with**, and its DV01 per unit (§3b). |
| **Exit mode** | Bind scopes to **Auto** (fire on breach) or **Suggest** (raise a standing row) (§6). |
| **LP Panels** | Maintain the standing include/exclude LP selection every external hedge inherits. |
| **Execution mode** | The kill-switch, Advisory-vs-live execution mode, and the rate/size guards. |

**Watching it run is a different surface:** the live monitor — per-book RAG, the fired-hedge
audit trail, and the **standing suggestions** — lives under **Risk → Hedge flows**, so
authoring the rules and watching them fire are deliberately separated.

**Who can edit:** authoring gates on the **`hedge` capability × Fixed Income**. Without it,
every tab is visible but **read-only** (see §8). This guide assumes you hold that capability.

---

## Step 1 — Pick the scope and metric (Thresholds tab)

A hedge policy governs a *scope* measured by a *risk metric*. Start on the **Thresholds**
tab and decide, for each book/portfolio you want managed:

1. **Scope** — one of **Desk**, **Book**, or **Instrument** (the `Scope` selector), plus a
   **Scope id** (e.g. `fi-rates-emea`, the desk/book/instrument id). Thresholds resolve
   **most-specific-wins**: an instrument threshold overrides its book's, which overrides its
   desk's.
2. **Metric** — the exposure the budget caps (`Metric` selector):
   - **Net DV01** (`dv01`) — the natural first-order rate risk for FI/rates books.
   - **Net notional** (`net_notional`) — the simplest cash form of "100".
   - **Net delta** (`net_delta`) — base-currency directional risk for FX.
   - **Net vega** (`net_vega`) — vol risk for an options book.

One desk can warehouse to a DV01 budget while another uses net notional — the metric is set
per threshold row.

> The risk state the engine actually branches on (net DV01, net notional, vega, gamma,
> inventory sign, toxicity, inventory age, internal offset available, hedge cost) is
> snapshotted per `(book × instrument)` on every risk update — you will reference these same
> fields when you author the policy in Step 3.

---

## Step 2 — Set the warehouse threshold and bands (Thresholds tab)

Still on **Thresholds**, add a row with **Add / edit a threshold**. Each field:

| Field | What it does | How to choose it |
|---|---|---|
| **Cap (the "100")** | The budget magnitude in the metric's native units. | Your warehousing appetite for that scope — e.g. `300000` DV01. Must be `> 0`. |
| **Amber (0–1)** | Utilisation fraction where you **start skewing** to attract offset. | Conventionally `0.7–0.8`. Below amber = warehouse. |
| **Red (0–1)** | Utilisation fraction where you **start hedging the overflow**. | Conventionally `0.9`. Must satisfy `0 ≤ amber ≤ red ≤ 1`. |
| **Target fraction** | The band edge you hedge **back to** (as a fraction of cap). | Defaults to the amber fraction — hedge the overflow *to the amber edge*, not to flat. Set `0` to flatten fully on a breach. |
| **Min clip** | Minimum hedge ticket (fixed-cost floor). | Don't fire trivially small back-to-backs; a breach whose overflow is below this still hedges exactly the min clip. |
| **Max clip** | Largest single hedge; a bigger overflow is worked in slices. | Bounds market impact per clip. |
| **Ramp the hedged fraction** | Optional — hedge a *fraction that grows with utilisation* rather than the whole overflow at once. | Leave off for "hedge the full overflow". On, with gain `k`, hedges `clamp(k·(utilisation−1), 0, 1)` of the overflow — gentle for a small breach, aggressive for a large one. |

### What the three bands mean

| Band | Condition (on `|net risk|`) | Behaviour |
|---|---|---|
| 🟢 **Green** | below the amber edge | **Warehouse** — hold, capture the spread, let it mean-revert. |
| 🟠 **Amber** | amber ≤ utilisation < red | **Skew** — lean the two-way to pull in the offsetting side for free. |
| 🔴 **Red / Breach** | utilisation ≥ red | **Hedge the overflow** — the exit policy fires. |

Band classification is **sign-agnostic**: a −295k DV01 short and a +295k long against a 300k
cap both read the same band.

**Sizing when red fires (default `Overflow` size):**

```
target      = target_fraction × cap          (default: the amber edge)
overflow    = max(0, |net_risk| − target)
hedge_size  = clamp(overflow, min_clip, max_clip)
```

You hedge *to the band edge*, not to flat — the transaction-cost-optimal policy (don't
over-trade the position back and forth across the boundary). Delete a threshold with the
row's **Delete** button (sends cap `0` server-side).

---

## Step 3 — Author the exit policy (Exit Policy tab)

This is the heart of it, and it uses the **same building blocks as risk routing**. The
policy is an **ordered list of rules**, each `IF <conditions> THEN <exit action>`, evaluated
**first-match-wins**. A rule with **no conditions** is the **catch-all** and sits at the
bottom (a fresh install seeds a single `WAREHOUSE` catch-all so the table is never empty or
invalid).

### Build a rule

1. Click **+ Create hedge rule**.
2. **Drag risk-state field chips** from the palette into the **Conditions** area. Each
   condition is `field op value`; multiple conditions are **ANDed** ("all must hold"). Edit
   each leg's operator and value with the shared typed value editor.
3. Pick the **exit action** (the leaf) and its parameters (below).
4. Read the plain-English **Preview**, then **Save rule**. New *specific* rules slot **above**
   the trailing catch-all automatically.
5. Back on the table, **reorder** rules (first-match-wins — order matters), toggle them
   on/off, and finally **Save policy**. The editor blocks save while any conflict or graph
   issue is unresolved (it lists them inline).

### The condition fields

| Group | Fields |
|---|---|
| **Identity** | `instrument_id`, `ccy`, `product`, `book`, `desk`, `counterparty` |
| **Risk state** | `net_dv01`, `net_notional`, `net_vega`, `net_gamma`, `inventory_sign` (+1 long / −1 short) |
| **Budget state** | `threshold`, `utilization` (`|risk|/threshold`), `overflow`, `breached` (`true`/`false` — the red-band trigger) |
| **Flow quality** | `counterparty_toxicity` (markout; high ⇒ hedge sooner), `inventory_age_secs` |
| **Market state** | `internal_offset_available` (opposing internal flow you could cross now), `hedge_cost_bp` |

Numeric fields support `> ≥ < ≤ = ≠ between in`; enum/string fields (`ccy`, `product`,
`book`, `desk`, `breached`, `instrument_id`, `counterparty`) support `= ≠` (and
`contains`/`in` for the string fields `instrument_id`/`counterparty`). A malformed rule
such as `net_dv01 contains "x"` is rejected by validation and is unrepresentable in the UI.

Hedging **by counterparty** and **by book** are the two identity dimensions a desk reaches
for most: `book` scopes the policy to one risk book, while `counterparty` (the originating
party-id, matched as the blotter shows it) scopes it to one client's flow — e.g.
`Counterparty = CITADEL → SUBMIT_MARKET_ORDER` back-to-backs **all** of Citadel's flow the
moment it lands. Right-clicking a deal on the Deals blotter → **Change hedging strategy**
pre-fills exactly this counterparty condition (alongside currency/product/desk).

### The exit-action leaves

| Action | What it does | When to use it |
|---|---|---|
| **`WAREHOUSE`** | Hold — do nothing, keep the risk. | The green-band default; the explicit catch-all so the graph is total. |
| **`CROSS_INTERNAL` { instrument, size }** | Offset against opposing internal flow / other desks in the **Agg Book**, at the consolidated mid. | Benign flow when an internal offset exists — the cheapest *trade* (no street impact or leakage). |
| **`SKEW` { bp \| to-edge }** | Lean the two-way to **attract** the offsetting side — no trade, just a quote lean. | The amber-band lever; try this *before* paying to hedge. |
| **`SUBMIT_MARKET_ORDER` { size, style }** | **Back-to-back**: place an offsetting external order. | The classic overflow hedge. `style = Immediate` (one clip) or `Worked` (sliced). |
| **`RFQ_OUT` { LPs, size }** | Request a two-way from **named LPs** and lift the best. | Larger / FI clips where you want competition across a chosen LP panel. |
| **`SPLIT` { internal-first, size, style }** | Net internally up to the available offset, then externalise the **residual**. | The composite "internalise then hedge overflow" primitive — the workhorse for mixed flow. |
| **`ESCALATE` { reason }** | Fire an alert / hand to a human desk instead of auto-acting. | Toxic, illiquid, or unusually large overflow the desk wants to hand-manage. |

**Size** on any trading leaf is one of: **Overflow** (default — hedge to the band edge),
**Full** (flatten the whole net position), or **Fixed** (an explicit magnitude, never more
than you hold). **Style** is **Immediate** (one clip, true back-to-back) or **Worked** (an
Almgren–Chriss-scheduled series of slices for large clips).

**Vehicle** — every *size-bearing* leaf (`SUBMIT_MARKET_ORDER`, `RFQ_OUT`, `SPLIT`,
`CLEAR_RISK`, `CROSS_INTERNAL`) also carries a **Hedge with** picker: *what* the order
actually trades. It defaults to **Same security (self)**, which is exactly what every rule
did before the picker existed. See §3b. `WAREHOUSE`, `SKEW` and `ESCALATE` place no order,
so they carry no vehicle.

### Try it before you arm it — "What would fire?"

Below the rules table, the **What would fire?** trace panel lets you dial a sample risk state
(book, instrument, breached, utilisation, overflow, threshold, net DV01, counterparty
toxicity, internal offset available, hedge cost) and walks your **currently-compiled** policy
live — showing the **RAG band**, the **exit action that fires**, and the **node path**. Use it
to sanity-check ordering before saving.

### Worked example rule set (build top-to-bottom; first match wins)

```
IF breached == false                          → WAREHOUSE
IF counterparty_toxicity > 0.6                → SUBMIT_MARKET_ORDER { size: Overflow, style: Immediate }   // toxic → back-to-back now
IF internal_offset_available > overflow       → CROSS_INTERNAL { instrument: AGG-OIS, size: Overflow }     // benign & offset exists → net internally
IF overflow > 50000000                        → RFQ_OUT { LPs: [LP-1, LP-2, LP-3], size: Overflow }        // big residual → work an RFQ panel
(catch-all, no conditions)                    → SPLIT { internal-first, size: Overflow, style: Worked }    // net what we can, work the rest
```

---

## Step 3b — Pick what you hedge *with* (Vehicles tab + the leaf's "Hedge with" picker)

Every exit action above says **what kind** of exit to make. It now also says **with what**.

### Why this exists

Selling the **same security back** is exact: the offsetting leg is the identical instrument
— same coupon, maturity, day-count, security id — so the DV01 ratio between leg and fill is
**identically 1** under any duration measure. Sell `factor` of the face you are long and you
shed exactly `factor` of the risk. No curve, no duration input, nothing to get wrong.

That is the right answer for a swap or a government bond. **It is not how a corporate bond is
hedged.** A corp is hedged with a *benchmark* at matching maturity — in practice a Treasury
future. And the moment the hedge instrument differs from the position, that ratio stops being
1: the size now needs a **DV01 per unit**, which has to come from somewhere.

### The four vehicles

| Vehicle | What it trades | Sizing |
|---|---|---|
| **Same security (self)** | The position itself, sold back. **The default** — unchanged behaviour. | Ratio 1. Exact, no residual. |
| **Benchmark (by maturity)** | Resolved from the **Vehicles** registry by instrument + maturity bucket. | The matched row's DV01 per unit. |
| **Named instrument** | One explicitly named cash instrument. | That registry row's DV01 per unit. |
| **Named future** | One explicitly named future. | DV01 per unit, rounded to **whole contracts**. |

A **named** vehicle must be a row in the registry — that row is where its DV01 per unit comes
from — so the picker offers exactly the registered hedge instruments, and **the server rejects
a rule naming one it cannot price**.

### Building the registry (Vehicles tab)

Each row matches on three ANDed axes — **instrument**, **product**, **currency**, each
optional (empty ⇒ matches anything) — plus a **half-open maturity bucket `[min, max)`** in
years, and names the instrument a match hedges with, its **DV01 per unit** and its **unit
label** ("contract" for a future, "1mm face" for cash).

| Id | Match | Bucket | Hedges with | Future? | DV01/unit | Unit |
|---|---|---|---|---|---|---|
| `us-corp-7-10y` | any · BOND · USD | 7–10y | `TY-DEC26` | yes | 78 | contract |
| `us-corp-2-5y` | any · BOND · USD | 2–5y | `FV-DEC26` | yes | 42 | contract |
| `uk-gilt-3-7y` | any · BOND · GBP | 3–7y | `G-MAR27` | yes | 64 | contract |
| `usd-ois-any` | any · OIS · USD | any | `USD-SOFR-OIS-5Y` | no | 480 | 1mm face |

Validation is client-side and shows **every** problem at once: the id must be unique and
non-empty, the hedge instrument must be named, **DV01 per unit must be > 0** (a zero cannot
size anything), and `max > min` whenever either bound is set. Leave both bounds at `0` for
"any maturity". Ticking **Future** also re-defaults the unit label, because a contract and a
face amount are not interchangeable units.

The roster saves with the rest of the engine config — there is no separate CRUD step.

> **⚠ The DV01 per unit is entered, not derived.** It drifts as the benchmark rolls. Review it
> when contracts roll or the curve moves materially, and keep maturity buckets **disjoint** —
> overlapping buckets make it ambiguous which row a benchmark resolves through.

---

## Step 4 — Where the hedge goes, and which LPs it uses

Every action is either **internal** (never leaves the firm) or **external** (places a street
order):

- **Internal exits** — `CROSS_INTERNAL` (net against opposing flow in the Agg Book at the
  consolidated mid) and `SKEW` (a quote lean, no trade at all). `CROSS_INTERNAL` names the
  **aggregation instrument** to cross against (e.g. `AGG-OIS`, `AGG-US10Y`, `AGG-EURUSD`,
  `AGG-UK5Y`).
- **External exits** — `SUBMIT_MARKET_ORDER`, `RFQ_OUT`, and the external leg of `SPLIT` place
  the residual onto the multi-dealer RFQ / FIX panel.

The engine always **internalises before it externalises**: for a `SPLIT`, it crosses
`min(want, internal_offset_available)` internally first and externalises only the residual.

### LP selection (the honest state today)

- **`RFQ_OUT` carries a per-rule LP *include* list** — the **"LPs to fan to"** control in the
  action editor. Add/remove named LPs (from the panel `LP-1 … LP-4`) and the request fans to
  exactly those. Validation rejects an `RFQ_OUT` with an empty panel or an unknown LP. This is
  the LP-selection mechanism that shipped: **per-rule, include-only, for `RFQ_OUT`.**
- **`SUBMIT_MARKET_ORDER` has no LP picker** — it routes to the default panel.
- **There is no LP *exclude* list, and no global / per-desk "hedging LP panel" config.** You
  cannot yet say "hedge on all LPs except X" or maintain a standing hedge-LP whitelist that
  every external action inherits.

> **⚠ Follow-up (small config gap):** a global/per-desk **LP include/exclude panel** for
> hedging — inherited by `SUBMIT_MARKET_ORDER` and defaulting `RFQ_OUT` — is **not built
> yet**. Today, choose LPs per `RFQ_OUT` rule; there is no exclude semantics. Track this as
> the LP-panel config follow-up.

**Also honest:** external execution is currently **advisory** (see §5). External actions emit
a real intent + provenance (including the simulated winning LP in the audit's `LP` column) but
do **not** yet place a live street order — that is the deliberate P3 wiring seam. The internal
cross path, by contrast, has a real booking ledger with the hard-cap gate applied.

---

## Step 5 — Advisory vs live, and the safety controls (Monitor tab)

Open the **Monitor** tab → **Engine controls**. Auto-hedging is **advisory (dry-run) by
default** — the mandatory shadow-run before any desk lets it trade for real.

| Control | Effect |
|---|---|
| **Advisory only** (default ON) | Dry-run: the engine computes and **emits every intent + stamps real provenance** but **trades nothing**. This is the shadow-run posture. Turn OFF to let hedges act. |
| **Kill switch** | HALTS all auto-hedging instantly — every position simply warehouses. The big red stop. |
| **Per-desk enable** | Toggle auto-hedging per desk independently. |
| **Max clip** | Hard ceiling on any single hedge size. |
| **Max hedges / interval** | Rate limit — caps how many live hedges fire per interval so a misconfigured policy or a market gap can't machine-gun the LP panel. |
| **Daily external cap** | Ceiling on total externalised notional per day. |

### How to arm live (recommended path)

1. Save your thresholds (Step 2) and policy (Step 3).
2. Leave **Advisory only** ON. Watch the Monitor for a session or more — confirm the intents
   match your intent and the RAG bands behave.
3. When satisfied, turn **Advisory only** OFF. Internal crosses will book (through the
   cap-gated ledger); external actions remain advisory until the P3 street-order wiring is
   enabled.
4. Keep **Max clip / Max hedges per interval / Daily external cap** set as your guardrails,
   and keep the **Kill switch** within reach.

**Limits are never bypassed:** every internal-cross leg re-runs the **target book's hard-cap
gate** before it commits — an auto-hedge can never itself breach a cap. The soft banded
threshold is the *trigger*; the hard cap is the *backstop*.

---

## Step 6 — Fire it, or suggest it (Exit mode tab)

Everything above decides *what* the hedge is. This decides **whether it fires by itself**.

| Mode | What happens on a breach |
|---|---|
| **Auto** | The policy resolves and the hedge **trades**. This is the existing behaviour and the default. |
| **Suggest** | The engine still measures, still resolves the policy and still **sizes** the hedge in its vehicle — then trades nothing and publishes a **standing row** on **Risk → Hedge flows**. |

> **Suggest is deliberately *not* a confirmation dialog.** Nothing pops up and nothing
> interrupts you. A modal gets dismissed reflexively and takes the decision with it; a
> standing row survives being ignored and waits on the risk panel until someone acts on it.

Bind scopes on the **Exit mode** tab. The scoping axis and precedence are the *same* as the
warehouse thresholds and the LP panels — **instrument > book > desk**, most-specific-wins —
and an unbound scope is **Auto**.

```
Bind:  book fi-credit-emea → Suggest      // benchmark-future hedges get a human eye
       (everything else unbound)          → Auto
```

### Reading a standing suggestion (Risk → Hedge flows → Suggestions)

A suggestion row leads with the **instruction**, because that is the thing you would do:

```
Sell 318 contracts of TY-DEC26
BREACH · fi-credit-emea · XS2034-ACME-4H · 138% of budget · 09:41:22
Corporate 9y is 138% of its DV01 budget. A corp is not hedged with itself —
the 7–10y benchmark row resolves to the 10Y future.

⚠ Approximate size — computed off a duration-blind exposure proxy, not a curve DV01.

  TRADES 318 contracts   EXACT 318.46   TARGET DV01 24,840   HEDGED DV01 24,804
  RESIDUAL rounded down, 36 DV01 still on the book        [ Hedge now ] [ Dismiss ]
```

Read it in three parts:

1. **The instruction** — the headline is exactly the order that would go out.
2. **The arithmetic** — `TARGET ÷ DV01-per-unit = EXACT units`, rounded to whole lots for a
   future. **Read the residual's sign**, because it is easy to read backwards: it is
   *target − hedged*, so *"rounded down, 36 DV01 still on the book"* means you stay
   **under**-hedged by 36, while *"rounded up, over-hedged by 31 DV01"* means the rounding
   took off **more** than the target.
3. **The caveat** — when the size came off the **duration-blind exposure proxy** (the coarse
   measure that treats every bond as duration 1 — see `HEDGING-AND-RISK-EXIT.md` §10.1), the
   row carries a prominent **approximate-size warning**. A 10-year bond's true DV01 is roughly
   8× what the proxy reports, so verify such a size against your own duration before acting.

**Hedge now** fires the sized hedge; **Dismiss** drops the row and trades nothing. **Neither
asks for confirmation** — that is the entire point of the mode. A rejected hedge **restores**
the row and shows the reason inline, so a suggestion is never silently lost. Acting requires
the **`hedge` capability on Fixed Income**; without it the rows are still visible (they are
risk information) but carry no buttons.

> **⚠ A suggestion does not hedge.** Risk keeps running until someone acts on the row, so a
> Suggest-scoped book needs someone watching it. The numbers are a snapshot at raise time —
> the mid and the exposure move afterwards. And Suggest is orthogonal to the **execution
> mode**: an Advisory engine still trades nothing even when you press *Hedge now*.

---

## Monitoring (Risk → Hedge flows)

The **Hedge monitor** shows:

- **Suggestions** — the standing `Suggest`-mode rows described above, first on the panel
  because they are the only section that asks you to act.
- **Per-book RAG strip** — the latest band and utilisation % for each book the engine has
  ticked.
- **Advisory intents (live)** — a rolling stream of what the engine resolved, each row showing
  band, `book · instrument`, the exit action, utilisation %, overflow, and an **ADVISORY**
  badge when the policy is armed dry-run.
- **Fired provenance (audit)** — the immutable audit trail of fired hedges: **When,
  Book · instrument, Band, Action, Internal** (crossed), **External** (hedged), **LP** (won),
  and **Mode** (`ADVISORY` / `LIVE`). This answers "*why did the system hedge this book at
  this time, at what price, on whose policy?*" — a green-band hold changes nothing and is not
  stamped.

---

## Permissions

- **The Hedging workspace is capability-gated.** Authoring (editing thresholds, the policy, or
  the engine controls) requires the **`hedge` capability on Fixed Income**. Holders see full
  edit affordances; everyone else sees the same surface **read-only**.
- **Running a policy is separable from authoring it.** The narrow `hedge` capability governs
  *composing and arming* the policy; the *booking* an auto-hedge performs is a booking-class
  write gated exactly like a manual booking/transfer. This lets a desk grant "can shape the
  hedge policy" without granting "can book", and vice-versa.

---

## End-to-end example — a rates book, 300k DV01

You run **`fi-rates-emea`** and want to warehouse rate risk up to **300k DV01**, skew as you
approach it, and split the overflow above the red band.

**1. Threshold (Thresholds tab):**

| Field | Value |
|---|---|
| Scope / Scope id | **Book** / `fi-rates-emea` |
| Metric | **Net DV01** |
| Cap | **300000** |
| Amber / Red | **0.7 / 0.9** |
| Target fraction | **0.7** (hedge back to the amber edge = 210k) |
| Min / Max clip | **1000 / 50000** |
| Ramp | off |

This means: 🟢 warehouse below 210k, 🟠 skew from 210k to 270k, 🔴 hedge the overflow beyond
270k — sizing the hedge back down to the 210k edge, clipped to `[1k, 50k]`.

**2. Policy (Exit Policy tab), top-to-bottom:**

```
IF breached == false                     → WAREHOUSE                                   // under red → hold
IF utilization >= 0.7 AND breached == false → SKEW { to-edge }                          // amber → lean to attract offset
IF internal_offset_available > overflow  → CROSS_INTERNAL { AGG-OIS, size: Overflow }   // offset exists → net internally
(catch-all)                              → SPLIT { internal-first, Overflow, Worked }   // net what we can, work the residual
```

*(Order matters — the `breached == false` rule must be first so the whole green band holds;
the amber `SKEW` rule then catches the not-yet-breached-but-approaching states.)*

**3. Trace it.** In **What would fire?**, dial `book = fi-rates-emea`, `utilization = 1.05`,
`overflow = 75000`, `internal_offset_available = 60000`, `breached = true`. Confirm it lands on
`SPLIT` (offset 60k < overflow 75k, so it crosses 60k internally and works the 15k residual).

**4. Shadow, then arm.** Leave **Advisory only** ON; watch the Monitor confirm the bands and
intents. When happy, turn Advisory off (internal crosses book live and cap-gated; external
legs remain advisory pending the P3 street wiring). Keep the daily cap and kill switch set.

---

## Gaps and honest boundaries (read before you rely on it)

1. **External hedges are advisory today.** `SUBMIT_MARKET_ORDER` / `RFQ_OUT` / the external leg
   of `SPLIT` emit real intents + provenance but do **not** place a live street order yet
   (the P3 `celnet-rfq` panel wiring seam). Internal `CROSS_INTERNAL` has a real, cap-gated
   booking ledger. The live `PositionStore` booking sink is the P2 seam.
2. **LP include/exclude is partial.** You get a **per-rule LP *include* list on `RFQ_OUT`**
   only. There is **no exclude list**, **no LP picker on `SUBMIT_MARKET_ORDER`** (default
   panel), and **no global/per-desk hedge-LP panel**. → follow-up config item (§4).
3. **Aggregation instruments and LP ids are a fixed advisory list** in the UI
   (`AGG-OIS/…`, `LP-1…LP-4`) — wiring them to the live aggregation registry / real LP
   sessions is a follow-up.
4. **Skew is expressed as a policy leaf**, but driving the live aggregation `AutoSkewSource`
   off net inventory is the P2 wiring; the amber `SKEW` action currently resolves as an
   advisory intent.
5. **Threshold defaults differ slightly between layers** — the engine's built-in band default
   is 80%/90%, while the Thresholds form pre-fills 70%/90%. Set the bands explicitly per row
   and don't rely on the pre-fill.
