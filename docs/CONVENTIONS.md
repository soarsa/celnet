# Celnet — FX-Options Convention Spec

The authoritative, detailed convention/analytics treatment lives in
[`ANALYTICS-SPEC.md`](./ANALYTICS-SPEC.md). This file is the quick map from those market
conventions to the **`celer-types` enums** that encode them as first-class per-`(pair, tenor)`
configuration (never global defaults — convention errors dwarf model error).

| Convention | Type (`celer-types`) | Variants | Notes |
|---|---|---|---|
| Delta | `DeltaConvention` | `SpotUnadjusted`, `ForwardUnadjusted`, `SpotPremiumAdjusted`, `ForwardPremiumAdjusted` | Premium-adjusted when premium paid in FOR/base ccy. Short tenors → spot; long (≳1–2Y) → forward. |
| ATM | `AtmConvention` | `AtmForward`, `DeltaNeutralStraddle` | DNS dominant interbank. DNS strike: `F·e^{+½σ²T}` (unadj) vs `F·e^{−½σ²T}` (prem-adj). |
| Premium | `PremiumStyle` | `DomesticPips`, `PercentForeign`, `PercentDomestic`, `ForeignPips` | `is_premium_adjusted()` ⇒ FOR-ccy premium carries FX risk. |
| Cut | `Cut` | `NewYork1000`, `Tokyo1500` | NY 10:00 standard; Tokyo 15:00 for JPY/Asia. |
| Day count | `DayCount` | `Act365Fixed`, `Act360` | Vol-time ACT/365 kept distinct from MM accrual basis. |
| Settlement | `Settlement` | `Deliverable`, `NonDeliverable` | NDO cash-settles at a published fixing (EMTA/WMR). |

**Quoting:** smile is given as ATM vol + `RR_25/RR_10` (skew) + `BF_25/BF_10` (convexity); the
**broker (market) strangle → smile strangle** calibration is mandatory (never arithmetic
average). Strike↔delta is a guarded root-find in the configured delta convention
(premium-adjusted call delta is non-monotone). These land in `celer-conventions` (WS-A),
`celer-vanilla` (WS-B), and `celer-surface` (WS-C).
