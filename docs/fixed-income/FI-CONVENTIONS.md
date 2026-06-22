# Celnet — Fixed-Income Convention Spec (`RatesConvention` config schema)

**Status:** P0 spec (first synthesis) · 2026-06-22 · branch `feature/fixedincome`
**Scope:** the rates convention config schema for `celnet-rates` / `celnet-conventions` — day-count,
business-day conventions, RFR observation methods, fixing sources, per-currency calendars, spot lag —
expressed as a per-`(currency, index, tenor)` config **schema**, never a global default.
**Synthesised from:** [`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md)
§3 (every claim carries its pass-1 source). Mirrors [`../CONVENTIONS.md`](../CONVENTIONS.md)'s
per-`(pair, tenor)` record shape and table style; cross-references it rather than duplicating it.

> **Naming guardrail.** Enum variant names below are **purpose-named and vendor/method-neutral**
> (CLAUDE.md §8): `DayCount::Act360`, not a standards-body acronym in the identifier. The
> ISDA/ICMA clause provenance lives in this prose and in future doc-comments only.
>
> **Why per-`(currency, index, tenor)`, never global.** As in the FX core, *convention errors dwarf
> model error* ([`../CONVENTIONS.md`](../CONVENTIONS.md)). A rates convention is keyed on the
> **triple** — USD-SOFR-3M and GBP-SONIA-3M differ on day-count, calendar, fixing source, and accrual
> basis. There is no global default.

---

## 1. Day-count fractions (ISDA / ICMA)

The ISDA day-count fractions are the authoritative DCF set; each has a precise 2006-ISDA / ICMA
clause (findings §3.1). The `celnet-rates` `DayCount` enum (purpose-named; clause is provenance only):

| `DayCount` variant | Provenance clause | Typical use |
|---|---|---|
| `Act360` | ISDA 4.16(e) / ICMA 251.1(i) | USD/EUR money-market & SOFR/€STR legs |
| `Act365Fixed` | ISDA 4.16(d) | GBP/SONIA legs |
| `Thirty360` | ISDA 30/360 note | bond / US-corp fixed legs |
| `ActActIsda` | ISDA ACT/ACT | govt bonds (ISDA flavour) |
| `ActActIcma` | ICMA ACT/ACT | govt bonds (ICMA: divides by period-days × frequency) |

(findings §3.1 — [2006 ISDA Definitions](https://www.sc.com/en/uploads/sites/66/content/docs/2006-ISDA-Definitions.pdf),
[ISDA 30/360 note](https://www.isda.org/2008/12/22/30-360-day-count-conventions/),
[Wikipedia day-count, clause-cited](https://en.wikipedia.org/wiki/Day_count_convention),
[OpenGamma Strata DayCounts](https://strata.opengamma.io/apidocs/com/opengamma/strata/basics/date/DayCounts.html)).
`celnet-types::DayCount` already carries `Act365Fixed`/`Act360` for the FX vol-time/MM basis
([`../CONVENTIONS.md`](../CONVENTIONS.md)); the rates set **extends** that enum on the shared
`celnet-types` seam (additive, single contract — guardrail #9) rather than forking a parallel enum.

---

## 2. Business-day conventions + the adjusted/unadjusted accrual axis

Date-rolling conventions, and the **separate** accrual-date axis (findings §3.2):

| `BdConvention` variant | Behaviour |
|---|---|
| `Following` | roll to next business day |
| `ModifiedFollowing` | next business day **unless** it crosses month-end, then roll back — the **market default for swaps** |
| `Preceding` | roll to previous business day |
| `ModifiedPreceding` | previous business day unless it crosses month-start, then roll forward |

**Accrual dates are a separate config axis from payment-date rolling**: accrual may be computed on
**adjusted** or **unadjusted** dates independently of how payment dates roll (findings §3.2 —
[Wikipedia day-count](https://en.wikipedia.org/wiki/Day_count_convention),
[OpenGamma IR conventions guide](https://quant.opengamma.io/Interest-Rate-Instruments-and-Market-Conventions.pdf)).
The schema carries this as `accrual_dates: Adjusted | Unadjusted`, orthogonal to `business_day_conv`.
This mirrors how the FX schedule keeps the modified-following roll distinct from the vol-time anchor
([`../CONVENTIONS.md`](../CONVENTIONS.md) "Vol-time anchor").

---

## 3. RFR observation methods (the new post-LIBOR axis)

The RFR observation method is the genuinely new convention axis vs LIBOR — standardised by ISDA
Supplement 75 / the 2021 Definitions (effective 13 May 2021) (findings §3.3). The
`RfrObservation.method` enum:

| `RfrObservation` method | Behaviour |
|---|---|
| `Lookback` | use the rate from N business days **before** each observation day; rate **weighting unchanged** |
| `ObservationShift` | shift the whole observation **period** back N days; **weights shift too** |
| `Lockout` | freeze the rate for the last N days of the period |
| `PaymentDelay` | pay N days after period end |

(findings §3.3 — [ISDA RFR Conventions & IBOR Fallbacks Product Table, Oct 2021](https://www.isda.org/a/bdigE/RFR-Conventions-and-IBOR-Fallbacks-Product-Table-October-2021.pdf),
[ISDA Key Changes 2021 Definitions](https://www.isda.org/a/BNEgE/Key-Changes-in-the-2021-ISDA-Interest-Rate-Derivatives-Definitions-June-2021.pdf),
[ARRC Users Guide to SOFR](https://www.newyorkfed.org/medialibrary/Microsites/arrc/files/2021/users-guide-to-sofr2021-update.pdf)).

The floating coupon itself is the **geometric compound** of daily overnight fixings over the accrual
period, in arrears — `R_comp = (∏_i (1 + r_i·d_i/360) − 1)·(360/D)` over business days `i` — **not** a
single fixing (findings §2.2 —
[ARRC Users Guide to SOFR](https://www.newyorkfed.org/medialibrary/Microsites/arrc/files/2021/users-guide-to-sofr2021-update.pdf)).
Day-basis is ACT/360 for USD/EUR, ACT/365F for GBP-SONIA. The schema therefore also carries
`compounding: Compounded | Averaged`. (The pricing-side accrual lives in `celnet-rates::rfr`; see
[`FI-CURVES-SPEC.md`](./FI-CURVES-SPEC.md) §2 and [`FI-ARCHITECTURE.md`](./FI-ARCHITECTURE.md).)

---

## 4. Fixing sources, calendars, and spot lag (all per-currency)

Fixing sources and calendars are **per-currency** and must be **data, not code** (findings §3.4):

| Currency | RFR index / fixing source | Publisher | Calendar(s) |
|---|---|---|---|
| USD | SOFR | NY Fed | US (FED / NYSE / SIFMA) |
| EUR | €STR | ECB | EUR (TARGET2) |
| GBP | SONIA | BoE | GBP (London) |

RFR fixings are published by the central bank the morning **after** the rate day; calendars drive
both observation-day enumeration and payment rolling (findings §3.4 —
[ARRC How to Use SOFR](https://www.newyorkfed.org/medialibrary/microsites/arrc/files/2019/How_to_Use_SOFR.pdf)).
These extend `celnet-calendar`, which already carries Gregorian-computable settlement calendars on
the FX side ([`../CONVENTIONS.md`](../CONVENTIONS.md) "Calendar coverage").

**Spot/settlement lag is per-instrument**: USD/GBP swaps typically T+2, money-market deposits T+0/T+2
by currency, bonds T+1 (UST) / T+2 (many) — driving the effective/start date from trade date
(findings §3.5 —
[OpenGamma IR conventions guide](https://quant.opengamma.io/Interest-Rate-Instruments-and-Market-Conventions.pdf)).
The schema carries `spot_lag: BusinessDays` per `(instrument, currency)`.

> **Pending Q12.** Calendar/fixing data must come from **open/published central-bank/exchange
> sources** (NY-Fed/ECB/BoE published fixings; FED/TARGET2/London calendars) plus operator-supplied
> static, under the no-commercial-feed guardrail (findings §B Q12). **No live feed value is claimed
> in-repo** — only the convention/calendar *identity* code, exactly as the FX NDF fixing sources
> carry identity-only (`FixingSource`, [`../CONVENTIONS.md`](../CONVENTIONS.md)). **Operator must
> confirm Q12** (curate static vs pull an OSS calendar lib).

---

## 5. The `RatesConvention` config schema

The schema mirrors [`../CONVENTIONS.md`](../CONVENTIONS.md)'s per-`(pair, tenor)` record shape, keyed
on the **`(currency, index, tenor)` triple** (findings §3 config sketch). This is a **schema shape**,
not Rust:

```
RatesConvention {                       // keyed per (currency, index, tenor) — NEVER a global default
  index:            { name, fixing_source, currency }   // SOFR / €STR / SONIA … (identity only)
  day_count:        DayCount            // Act360 | Act365Fixed | Thirty360 | ActActIsda | ActActIcma
  business_day_conv: BdConvention       // Following | ModifiedFollowing | Preceding | ModifiedPreceding
  accrual_dates:    Adjusted | Unadjusted    // SEPARATE axis from payment rolling (§2)
  calendars:        [CalendarId]        // e.g. [US_FED, NYSE] | [TARGET2] | [GB_LON]
  spot_lag:         BusinessDays        // T+0 / T+1 / T+2, per (instrument, currency)
  payment_freq:     Frequency
  rfr_observation:  RfrObservation {    // null for term-rate / IBOR-style legs
     method:        Lookback | ObservationShift | Lockout | PaymentDelay
     offset_days:   BusinessDays
     lockout_days:  BusinessDays
     payment_delay: BusinessDays
     compounding:   Compounded | Averaged
  }
  roll_convention:  RollConvention      // EndOfMonth | Imm | DayOfMonth
  rounding:         { dp, mode }
}
```

### 5.1 Worked per-`(currency, index, tenor)` rows (illustrative)

Mirroring the FX "pair universe" table ([`../CONVENTIONS.md`](../CONVENTIONS.md) "Pair universe"):

| (Ccy, Index, Tenor) | day_count | bd_conv | accrual | calendars | spot_lag | rfr method / compounding |
|---|---|---|---|---|---|---|
| USD · SOFR · 3M | `Act360` | `ModifiedFollowing` | per-CSA | `[US_FED]` | T+2 | `Lookback` / `Compounded` |
| EUR · €STR · 3M | `Act360` | `ModifiedFollowing` | per-CSA | `[TARGET2]` | T+2 | `Lookback` / `Compounded` |
| GBP · SONIA · 3M | `Act365Fixed` | `ModifiedFollowing` | per-CSA | `[GB_LON]` | T+2 | `Lookback` / `Compounded` |

> The exact lookback/lockout/payment-delay offsets per market are an **operator-confirmed static**
> table (pending Q1 currency scope + Q12 data sourcing); the schema carries the axis, the values are
> data. The illustrative rows above are not a frozen pin.

---

## 6. P0 scope & open-question dependencies

- **In P0:** the `RatesConvention` schema, the `DayCount`/`BdConvention`/`RfrObservation` enums,
  per-currency fixing-source identity + calendars, per-instrument spot lag — for the Q1 currency set.
- **Pending Q1** (currency scope: USD-SOFR + EUR-€STR + GBP-SONIA, or USD-only) — sizes the calendar
  + fixing static.
- **Pending Q12** (calendar/fixing data sourcing) — see §4.
- **Deferred:** term-rate / IBOR-fallback legs beyond identity, inflation indexation lag, and
  bond-market-specific conventions (ex-div, settlement per DMO) — to the cash-bond pass (findings §C).

**Verification:** convention resolution is validated structurally (resolved convention matches the
published ISDA/ICMA/central-bank standard) and against QuantLib's day-count/calendar implementations
— see [`FI-VERIFICATION-CONTRACT.md`](./FI-VERIFICATION-CONTRACT.md). This mirrors the FX
`pair_universe.rs` parity proof ([`../CONVENTIONS.md`](../CONVENTIONS.md)).

---

### Sources

All sources are the pass-1 citations in
[`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md) §3 and its
consolidated source list; the load-bearing ones are inlined above.
