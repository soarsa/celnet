# Celnet — FX-Options Convention Spec

The authoritative, detailed convention/analytics treatment lives in
[`ANALYTICS-SPEC.md`](ANALYTICS-SPEC.md). This file is the quick map from those market
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
| Smile model | `SmileModel` | `MarketHedge`, `StochasticVol`, `Parametric`, `ParametricSurface`, `ExtendedSurface` (`Default=MarketHedge`) | Calibration family selected per `MarkSurface`; vendor/method-neutral names mirror `celnet_surface::SmileModel` (vanna-volga / SABR / SVI / SSVI / eSSVI provenance lives in `ANALYTICS-SPEC.md` §3, never in identifiers). |

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

## Pair universe (Wave 4d)

`celnet-conventions::pair_meta` resolves the full **pair-universe** view of a covered pair —
spot lag, premium currency, ATM/delta convention, cut, settlement style, NDF fixing source +
cash-settlement currency, and the metal-base flag — in the queried orientation. It consults the
same `PairProfile` the wire `resolve` uses, so the universe view and the per-`(pair, tenor)` wire
record can never disagree. New `celnet-types` enum `FixingSource` names the published NDF/NDO
reference fixing (EMTA/ISDA identity only — never live values). The **EXACT covered set**:

| Class | Pairs | Spot lag | Premium ccy | ATM | Cut | Settlement | Fixing |
|---|---|---|---|---|---|---|---|
| G10 majors | EURUSD, USDJPY, GBPUSD, AUDUSD, USDCHF, **USDCAD**, NZDUSD | T+2 (**USDCAD T+1**) | per-pair (FOR or DOM) | DNS | NY (JPY→Tokyo) | Deliverable | — |
| EM deliverable | USDMXN, USDZAR, USDNOK, USDSEK | T+2 | USD (FOR, prem-adj) | DNS | NY | Deliverable | — |
| EM non-deliverable (NDF/NDO) | USDKRW, USDTWD, USDINR, USDBRL, USDCLP, USDCOP | T+2 | USD (FOR, prem-adj) | DNS | Tokyo (Asia) / NY (LatAm) | **Non-deliverable, cash-settled USD** | KFTC18 / Taipei / RBIB / PTAX / Dólar-Obs / TRM |
| Precious metals | XAUUSD, XAGUSD | T+2 (loco-London) | USD (DOM, unadj) | DNS | NY | Deliverable | — |

**Calendar coverage (honest boundary).** Spot dates resolve **algorithmically** over the joined
leg calendars. `celnet-calendar` adds fully **Gregorian-computable** settlement calendars for the
EM deliverable currencies — **MXN** (Banxico), **ZAR** (SA Public Holidays Act), **NOK** (Oslo),
**SEK** (Stockholm) — and metals settle on the **London ∩ US** loco-London calendar. The NDF
currencies' *onshore* calendars (KRW/TWD/INR/BRL/CLP/COP) are driven by **lunisolar/religious**
holidays and are deliberately **not** modelled (approximating a lunar holiday would be a
placeholder); those pairs carry their full *convention* identity but `has_calendar_support`
returns `false` until a real lunisolar ephemeris is wired. The `celnet-parity` row
`tests/pair_universe.rs` proves (i) every resolved convention matches the published EMTA/ISDA +
interbank standard, (ii) the algorithmic spot date equals an **independent** rata-die +
holiday-predicate walk over ~8.7k trade-date/pair combinations, and (iii) the structural
invariants (NDF⇒USD-cash-settled+flagged, metal-base, self-consistency, orientation-invariant
fixing). **No live EM/NDF feed data is claimed** — only the convention/calendar code.
