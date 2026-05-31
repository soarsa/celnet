# Celnet — FX-Options Convention Spec

The authoritative, detailed convention/analytics treatment lives in
[`ANALYTICS-SPEC.md`](./ANALYTICS-SPEC.md). This file is the quick map from those market
conventions to the **`celnet-types` enums** that encode them as first-class per-`(pair, tenor)`
configuration (never global defaults — convention errors dwarf model error).

| Convention | Type (`celnet-types`) | Variants | Notes |
|---|---|---|---|
| Delta | `DeltaConvention` | `SpotUnadjusted`, `ForwardUnadjusted`, `SpotPremiumAdjusted`, `ForwardPremiumAdjusted` | Premium-adjusted when premium paid in FOR/base ccy. Short tenors → spot; long (≳1–2Y) → forward. |
| ATM | `AtmConvention` | `AtmForward`, `DeltaNeutralStraddle` | DNS dominant interbank. DNS strike: `F·e^{+½σ²T}` (unadj) vs `F·e^{−½σ²T}` (prem-adj). |
| Premium | `PremiumStyle` | `DomesticPips`, `PercentForeign`, `PercentDomestic`, `ForeignPips` | `is_premium_adjusted()` ⇒ FOR-ccy premium carries FX risk. |
| Cut | `Cut` | `NewYork1000`, `Tokyo1500` | NY 10:00 standard; Tokyo 15:00 for JPY/Asia. |
| Day count | `DayCount` | `Act365Fixed`, `Act360` | Vol-time ACT/365 kept distinct from MM accrual basis. |
| Settlement | `Settlement` | `Deliverable`, `NonDeliverable` | NDO cash-settles at a published fixing (EMTA/WMR). |
| Tenor | `Tenor` | `Overnight`, `TomNext`, `SpotNext`, `Weeks`, `Months`, `Years`, `Imm(u8)`, `BrokenDate(BrokenDate)` | See **Tenor resolution** below — the pre-spot short end (ON/TN/SN) is anchored on **today (horizon)**, not spot. `BrokenDate{year,month,day}` is a POD triple (keeps `celnet-types` free of `time`). |
| Smile model | `SmileModel` | `MarketHedge`, `StochasticVol`, `Parametric`, `ParametricSurface` (`Default=MarketHedge`) | Calibration family selected per `MarkSurface`; vendor/method-neutral names mirror `celnet_surface::SmileModel` (vanna-volga / SABR / SVI / SSVI provenance lives in `ANALYTICS-SPEC.md` §3, never in identifiers). |

**Tenor resolution (`celnet-calendar::fx`, fixes ON-resolves-as-SN).** Resolution takes both the
`horizon` (today/trade date) and the `spot` date so the short end is anchored correctly:

- **ON (`Overnight`)** = next good business day after **horizon** (~T+1). *Previously this
  resolved relative to `spot` (≈T+3, the SN region) — that bug is closed.*
- **TN (`TomNext`)** = the good day after ON.
- **SN (`SpotNext`)** = next good day after **spot**.
- **Standard ladder (`Weeks`/`Months`/`Years`)** = added to **spot**, then **modified-following**
  + **end-of-month** rule.
- **IMM (`Imm(n)`)** = the `n`-th (1-based) 3rd-Wednesday of the Mar/Jun/Sep/Dec cycle strictly
  after horizon, then modified-following (CME-style; validated incl. the Juneteenth roll).
- **Broken date (`BrokenDate`)** = the explicit civil date, modified-following onto a good day.

**Vol-time anchor.** `FxSchedule.vol_anchor` is the date vol-time accrues **from** — `horizon` for
ON/TN, `spot` otherwise — so `vol_year_fraction` (`celnet-conventions`, now `Result<_, TenorError>`)
never yields a non-positive vol-time at the short end. `TenorError { ImmOrdinalZero,
InvalidBrokenDate }` are the only input-bearing failure cases; the standard ladder / short end /
positive-IMM always resolve.

**Quoting:** smile is given as ATM vol + `RR_25/RR_10` (skew) + `BF_25/BF_10` (convexity); the
**broker (market) strangle → smile strangle** calibration is mandatory (never arithmetic
average). Strike↔delta is a guarded root-find in the configured delta convention
(premium-adjusted call delta is non-monotone). These land in `celnet-conventions` (WS-A),
`celnet-vanilla` (WS-B), and `celnet-surface` (WS-C).
