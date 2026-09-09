# Risk Models & DV01 — the desk's risk strategy

**Status:** as-built. Every mechanism described here exists in code and is gate-covered;
the two things that are *not* built are called out explicitly in §7 rather than left to
be discovered.

This is the firm's statement of **how a risk portfolio manages the risk it takes on**.
It sits above `HEDGING-AND-RISK-EXIT.md` (which describes the exit mechanism in detail)
and below `ARCHITECTURE.md`.

---

## 1. The strategy in one paragraph

The desk is a market maker, not a directional book. Its income is the bid-ask spread,
earned by quoting two-way and recycling inventory quickly. Interest-rate exposure is
therefore something to be **neutralised or deliberately budgeted**, never accumulated by
accident. Every risk portfolio declares one of three postures, and that declaration —
not an operator's memory — is what governs the booking path.

## 2. The three postures

| Model | What it does | Earns | Carries |
|---|---|---|---|
| **Back-to-back** | Hedges every fill straight out; warehouses nothing | The spread between client price and street price | No directional carry |
| **Internalise to a DV01 budget** | Warehouses client flow up to a DV01 budget so opposing flow nets off internally; sheds only the overflow | More spread — the netted portion never pays a street leg | Directional risk up to the budget |
| **Custom** | The exit-policy graph authored for that scope governs, unchanged | — | Whatever that graph implies |

`Custom` is the default and the escape hatch. **An unbound portfolio is `Custom`, which
means "its authored graph governs" — it does not mean "no hedging".** This distinction
has bitten before and is asserted by test.

A model is a *control over the existing exit primitives*, not a second engine: each
derives an ordinary `HedgeGraph` from the same node/action vocabulary a trader could
author by hand, and the booking path hands it to the same evaluator.

## 3. Scope and precedence

A model binds to a **desk**, a **risk portfolio (book)**, or an **instrument**, and
resolves **most-specific-wins: instrument > book > desk**. This is the same precedence
the hedging LP panels and exit modes use.

The practical consequence is the one that matters: a firm can run **one toxic book
back-to-back while the rest of the estate warehouses**.

Because precedence is invisible in any single control, the portfolio editor renders a
**resolution line** stating which scope actually supplied the posture ("inherited from
desk EMEA", "set on this portfolio"). The client resolver (`gui/src/lib/riskModel.ts`)
mirrors the server's rules exactly, including case-insensitive scope matching, so the
two can never disagree about which binding applies.

## 4. How DV01 is computed and hedged

DV01 is the P&L change for a 1bp move in yield. The engine computes it **analytically
from the bond's cashflows** (`celnet-bond::risk::dv01`), validated two ways: against a
finite-difference reprice, and against `modified duration × price × bp`.

The hedge is sized as a **ratio of DV01s**, not of notionals:

```
units = position_DV01 / vehicle_DV01_per_unit
```

Worked example (the institutional case):

| | |
|---|---|
| Position | Long $50,000,000 face, 10y corporate, spread duration 7.0 |
| Position DV01 | $50,000,000 × 7.0 × 0.0001 = **$35,000 / bp** |
| Hedge vehicle | On-the-run 10y UST, DV01 **$800** per $1mm |
| Treasury to short | $35,000 ÷ $800 = **$43.75mm** |
| Result | Long $50mm corps, short $43.75mm USTs → net rate DV01 ≈ 0 |

$43.75mm offsets $50mm because the durations differ. That is the whole point of sizing
on DV01 rather than notional.

**The engine refuses to guess.** If the vehicle registry has no DV01 per unit for the
resolved vehicle, `plan_hedge_ratio` returns nothing and the shed falls back to the
self-hedge (the same security sold back, ratio ≡ 1) with a loud warning — rather than
trading a fabricated size. The DV01 basis (`Analytic` vs a duration-blind proxy) is
carried on the plan so a proxy is never silently presented as a genuine measure.

## 5. Budget vs limit — two controls, deliberately

| | Meaning | Behaviour |
|---|---|---|
| **DV01 budget** | Warehouse up to here | **Soft** — crossing it *triggers hedging* |
| **DV01 limit** | Never exceed | **Hard** — crossing it *blocks the trade* |

Budget ≤ limit. The gap between them is the working room in which the desk manages
inventory. Collapsing them into one number destroys the distinction between "start
working out of this" and "stop taking it on", which is where a market-making desk lives.

**A blank budget means _inherit_ the scope's configured warehouse threshold — never a cap
of zero.** A zero cap would mean "warehouse nothing", the opposite of what an empty field
implies. Non-positive is therefore treated as inherit, and binding a model never silently
invents a cap. Asserted by test on both sides of the wire.

## 6. Why portfolios carry an asset class

A portfolio buckets **one franchise's** risk. FX-options vega and rates DV01 are not
commensurable, so a tree mixing them would roll up a meaningless total at the join. Each
portfolio declares `asset_class`, an unrecognised value is rejected rather than coerced,
and a sub-portfolio must match its parent. This is what lets Risk be a single firm-wide
surface instead of one screen per asset.

## 7. What is NOT built

Stated explicitly so nobody infers coverage that does not exist.

- **CS01 / credit spread risk has no model anywhere in the codebase.** A corporate bond
  hedged to DV01-neutral will display as flat while the desk carries its full spread
  exposure. For the worked example above that is $35,000/bp of credit risk shown as zero.
  This is the single most important gap and it is structural, not additive.
- **The pre-trade DV01 *limit* is not enforced at the booking gate.** Net and gross
  notional caps are (`rates_book.rs`); DV01 is deliberately skipped there because
  book-level DV01 is not computed at that seam — `book_risk.rs` omits it rather than
  publish "a utilization with no numerator", which is the right call. Until that lands,
  the DV01 **limit** documents intent and the DV01 **budget** is the live control.
- **Inventory decay clocks** (holding limit → systematic markdown). `inventory_age_secs`
  exists as a hedge decision input, so this is a policy layer rather than an engine change.
- **Captured-spread P&L** as a first-class metric.
- **Margin (IM/VM, SIMM)** — a separate programme.

## 8. Where a trader sets this

**Risk → Portfolios → select a portfolio → Risk model.** The model saves immediately (it
lives on the firm hedge config as a `book`-scoped binding, not on the portfolio
definition), so it does not wait for "Save changes". In-app help: the `concept.risk-models`,
`concept.dv01-hedge`, and `concept.dv01-budget` entries carry the same worked example as
§4 above.
