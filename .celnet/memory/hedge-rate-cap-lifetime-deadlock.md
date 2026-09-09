---
name: hedge-rate-cap-lifetime-deadlock
description: "ROOT CAUSE of \"hedge panel works but portfolios don't offset\" — max_hedges_per_interval never rolled, so it was a LIFETIME cap that wedged the whole desk. Fixed ce38b59e."
metadata: 
  node_type: memory
  type: project
  originSessionId: 87dda9d9-850c-48da-9447-a8082df24186
  modified: 2026-08-19T22:42:00.972Z
---

**2026-08-19. The "portfolios are not offsetting" symptom was a rate-limiter bug, and it
wedged the desk completely — not just hedging.**

`AutoHedgeEngine` guards a live external fire with `max_hedges_per_interval`, but
`roll_rate_interval()` / `roll_daily_window()` — documented as "the control loop calls this
each interval boundary" — had **no production caller** (only a unit test).
`hedges_in_interval` only ever incremented, so the *rate* limit was a **lifetime** cap.

**The deadlock chain** (all four links verified live on UAT):

1. Past the Nth hedge every external fire is silently downgraded to `advisory`. UAT had
   `max_hedges_per_interval = 30` and `advisory` flips at exactly **HDG-31**, forever.
2. Advisory hedges externalise nothing ⇒ `external_hedged: 0` on every red-band record.
3. `rates-usd` DV01 climbs to its 5000 cap and pins at `utilization: 1.000`.
4. At the cap the **pre-trade gate rejects every incoming client lift**
   (`"rates lift rejected: pre-trade / risk-book limit breach"`) ⇒ no fill ⇒ no
   `risk_version` bump ⇒ **hedge evaluation is fill-driven, so nothing ever re-evaluates**
   and the book can never drain.

**Fixed (`ce38b59e`)**: both windows now roll inside the guard off the same *injected*
`now_nanos` the record is stamped with — no background task, deterministic in tests.
Interval width is a documented 60s constant (the trader's tunable is the COUNT).

**Still open, deliberately not fixed there:** hedge evaluation is driven ONLY by position
changes, so any future non-hedging breach re-creates the same deadlock. A periodic risk
sweep would break it.

**Live unblock without a deploy:** set `max_hedges_per_interval = 0` (unbounded) via
`set_hedge_config`. ⚠ That RPC is a **FULL REPLACE** — read the whole config and echo every
field back, or you wipe the vehicle registry / exit_modes / hedging_models (this is how the
registry was lost before). Reply type is `hedge_config_updated`, NOT `hedge_config`.

**Three prior-session diagnoses that were WRONG — do not trust them:**

- "Benchmark resolution is failing / another scope says Self" — no. Benchmark resolves
  correctly to `ZTU26` with an analytic DV01 basis. The 67 `no_firm_lp_price` orders were
  stale history predating the policy repoint.
- "The LP-sims Administration panel was never built" — no. `LiquidityWorkspace`
  (`gui/src/workspaces/LiquidityWorkspace.tsx`, nav row `liquidity`, label **"LP Panel"**,
  `section: "admin"`) exists, polls `FixAdminService.ListLiquidityProviders`, and returns
  all five sims live (CME_SIM + 4 bond LPs) with freshness, weights, best-bid/offer counts.
- "UAT is running week-old code" — it was on `a065777f`, one commit behind.

A hedge whose target rounds below one whole contract correctly trades nothing
(`HedgeRatioPlan::is_tradeable`); that is honest, not a bug. UAT's vehicle registry has
`unit_label: "contracts"` (already plural), so summaries read "contractss" — cosmetic, fix
the registry row not the pluraliser.

Related: [[bond-hedge-books-no-offsetting-leg]], [[uat-reset-dv01-hedging-acceptance]],
[[hedging-execution-and-buckets-shipped]], [[cs01-z-spread-from-live-price]].
