# Knowledge: github.com-soarsa-celnet

Deterministic projection of graph-anchored knowledge claims. Active claims only.

## github.com-soarsa-celnet.crates.celnet-bench.benches.aad.bump\_first\_order

- **claim** (`cl\_24391ade08e8ba53`): bump\_first\_order computes all five first-order Greeks (delta, vega, theta, rho\_dom, rho\_for) via symmetric finite difference using step sizes h\_s=1e-5×spot (relative), h\_v=1e-5, h\_t=1e-6, h\_r=1e-6. Theta is sign-flipped (desk convention: −∂V/∂T). This numerical oracle is used in the aad bench to cross-validate adjoint\_greeks against central differences — the benchmark both times and validates correctness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.benches.aad.bump\_first\_order` (hash `d4af2f7e21c6145b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-bench.benches.iai\_instructions.soft\_regression\_limits

- **claim** (`cl\_cc4a6555b5b5b1bd`): DELIVERABLE bench/iai-instruction-gate-no-limits = LANDED (backlog tracker still lists it OPEN as a Round-2 P1/S finding; reconciled against the live graph). \`soft\_regression\_limits\` is a pure (side-effect-free) builder returning the iai-callgrind regression config: it constructs Callgrind::default().soft\_limits(\[(EventKind::Ir, SOFT\_INSTRUCTION\_REGRESSION\_PCT), (EventKind::EstimatedCycles, SOFT\_ESTIMATED\_CYCLES\_REGRESSION\_PCT)\]) with no external writes. The Round-2 gap (the iai instruction-count regression gate was structurally unable to fail because NO RegressionConfig/soft\_limit/hard\_limit existed — the lane always exited 0) is CLOSED: the gate now carries explicit per-EventKind percentage soft limits on instructions (Ir) and estimated cycles, applied via instruction\_gate, so an instruction-count regression beyond the band now flags. SELF-INVALIDATING: removing or editing the limit construction shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.benches.iai\_instructions.soft\_regression\_limits` (hash `ceb88d1fe876aead`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-bench.src.gpu\_load.compare\_to\_baseline

- **claim** (`cl\_5cc9c689ac3f9daa`): compare\_to\_baseline detects GPU performance regressions using a slowdown-only policy: for every batch size present in both reports it checks (1) throughput: a breach is recorded when measured \< baseline / (1 + tolerance), i.e. only falls count; and (2) dispatch p99 latency: a breach is recorded when measured \> baseline \* (1 + tolerance), i.e. only rises count. Improvements in either direction are never flagged. The function is pure over its inputs and allocates a new Vec\<GpuBreach\> — one entry per violated metric — leaving the inputs unchanged.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.gpu\_load.compare\_to\_baseline` (hash `0f8b7c0504ce9e4e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-bench.src.lib.batch\_builder\_is\_sane

- **claim** (`cl\_273845000884665d`): representative\_batch() constructs a 64-strike surface-slice (BATCH\_STRIKES=64) as a linear moneyness ladder spanning ±35% around the forward (\[0.65, 1.35\] × forward), with a symmetric quadratic-in-log-moneyness vol smile: vol = base\_vol + 0.6 × (ln(K/F))². This ensures ATM, skew, and deep-wing strikes all exercise the full d1/d2 range. The batch builder is validated by batch\_builder\_is\_sane, which asserts strictly-increasing positive strikes bracketing the forward plus finite, non-negative prices and 13 finite Greeks for every fixture.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.lib.batch\_builder\_is\_sane` (hash `a0515b4169a1f65c`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-bench.src.lib.representative\_batch` (hash `9050efa2dcda1b3b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-bench.src.lib.representative\_inputs

- **claim** (`cl\_604839394c0a311f`): representative\_inputs() returns a single canonical at-the-money EUR/USD-style vanilla option fixture: VanillaInputs::new(spot=1.10, strike=1.10, vol=0.095, t=0.5, r\_dom=0.025, r\_for=0.015). This is the exact input the hot-path benchmarks price; it is consumed by 9 callers across benches and unit tests, ensuring published benchmark numbers and test-suite numbers are the same workload.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.lib.representative\_inputs` (hash `3bb091ad98ea5f71`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-bench.src.lib.sweep\_inputs

- **claim** (`cl\_e651535f182637ab`): sweep\_inputs() returns a SWEEP\_LEN-element sequence of VanillaInputs where spot drifts ±5% (base×\[0.95, 1.05\]), moneyness spans \[0.75, 1.25\] (strike = forward × moneyness), and vol ranges \[0.07, 0.13\] — all linear in the index fraction. This smooth, varied sweep is the working set the coordinated-omission-aware core\_load histogram runs over; its coverage of the realistic liquid parameter window is validated by sweep\_inputs\_is\_varied\_and\_smooth.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.lib.sweep\_inputs` (hash `b9d63b53509401a8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-bench.src.surface\_rebuild.model\_name

- **claim** (`cl\_04da72a12303eba1`): model\_name maps each SmileModel variant to its canonical display string (one of "MarketHedge", "StochasticVol", "Parametric", "ParametricSurface", "ExtendedSurface"). The match is exhaustive and returns a \`'static str\`. This display name is written into benchmark reports and used by the §1.2 gate output.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.surface\_rebuild.model\_name` (hash `8df1037f244d21f7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-bench.src.surface\_rebuild.model\_static\_name

- **claim** (`cl\_90aad13e342c6706`): model\_static\_name interns a model name string back to a \`'static\` string literal for the four known variants ("MarketHedge", "StochasticVol", "Parametric", "ParametricSurface"); any other input maps to "unknown". This provides a safe zero-allocation conversion when deserializing a report's model name field back to a static string.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-bench.src.surface\_rebuild.model\_static\_name` (hash `a718cc73d937b1a4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.actual\_days

- **claim** (`cl\_db03b116927ee333`): \`actual\_days\` is the signed day-count primitive underlying every \`DayCount\` accrual: it returns \`(end - start).whole\_days()\` as an \`i64\`, so it is signed (negative when end precedes start) and counts whole days only. This signedness is what makes \`year\_fraction\` anti-symmetric under interval reversal; it is the sole bridge from the \`time::Date\` calendar type into the ACT/365 and ACT/360 numerators. Pure: reads two dates, returns i64, no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.actual\_days` (hash `2a3d55d6c5e2c9c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.signed\_when\_reversed

- **claim** (`cl\_22f054e1fa8ba73f`): \`signed\_when\_reversed\` pins the anti-symmetry of the day-count year fraction: with \`DayCount::Act365Fixed\`, \`year\_fraction(basis, 2024-01-01, 2023-01-01)\` equals −1.0 (asserted via \`assert\_close!\`, the sanctioned float comparator, never \`==\`). It guards that a reversed accrual interval yields the exact negative year fraction — the property exotic/vol-time accrual relies on for signed time spans — and that the 2024→2023 span is exactly 365 days over the ACT/365 denominator. Pure test: builds dates and asserts via assert\_close!, mutating no external state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.signed\_when\_reversed` (hash `1c2552dc17bc4b25`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.year\_fraction

- **claim** (`cl\_64d404d9484ab7ca`): \`year\_fraction\` is the canonical realization of the \`DayCount\` convention: it divides the actual day count by the basis-selected denominator — 365.0 for \`DayCount::Act365Fixed\`, 360.0 for \`DayCount::Act360\` — via an exhaustive match with no wildcard, returning a \`Time\`. Because the day count is signed (\`actual\_days\` = end − start in whole days), an end strictly before start yields a negative year fraction (the reversed-interval property), so the function is anti-symmetric in (start,end) by construction. This is the one place the celnet-types \`DayCount\` enum becomes a numeric accrual factor; ACT/365-fixed (vol-time) and ACT/360 (money-market) are kept deliberately distinct (docs/CONVENTIONS.md). Pure: reads basis and the two dates, returns Time, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.year\_fraction` (hash `871ea4854fbd0b49`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.TenorError.fmt

- **claim** (`cl\_c210de1bba1af647`): Implements \`Display\` for \`TenorError\` with two variants: \`ImmOrdinalZero\` formats as the string 'IMM ordinal must be \>= 1 (Tenor::Imm(0) is invalid)'; \`InvalidBrokenDate(b)\` formats as 'broken date YYYY-MM-DD is not a valid Gregorian date' using zero-padded year/month/day fields. No side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.TenorError.fmt` (hash `f586a9aff894b138`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.centre\_for

- **claim** (`cl\_090c4a36774d69ef`): Maps a G10 currency code to its FX settlement centre: USD→UnitedStates, EUR→Target2, GBP→UnitedKingdom, JPY→Japan, CHF→Switzerland, AUD→Australia, CAD→Canada, NZD→NewZealand. For any other currency it delegates to \`centre\_for\_em\`; unknown currencies return \`None\`. The function is exhaustive over the G10 set and has no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.centre\_for` (hash `e5afdb989769fe02`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.centre\_for\_em

- **claim** (`cl\_739fe627a4bda94f`): Maps emerging-market and precious-metal currencies to settlement centres: MXN→Mexico, ZAR→SouthAfrica, NOK→Norway, SEK→Sweden, and all four precious metals (XAU/XAG/XPT/XPD)→UnitedKingdom (reflecting loco-London LBMA/LPPM settlement convention). Any unrecognised currency returns \`None\`. Pure; no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.centre\_for\_em` (hash `830c97d66105369b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.expiry\_for\_tenor

- **claim** (`cl\_0940574a1971b9b3`): \`expiry\_for\_tenor(pair, horizon, spot, tenor)\` maps the full \`Tenor\` enum to an expiry \`Date\` with per-variant anchor rules: (1) Overnight → next business day after horizon; (2) TomNext → business day after ON expiry; (3) SpotNext → business day after spot; (4) Weeks(n) → spot + n weeks, ModifiedFollowing-adjusted; (5) Months(n)/Years(n) → spot + period via \`roll\_period\` (end-of-month rule if spot is last business day of its month, otherwise ModifiedFollowing); (6) Imm(n) → nth third-Wednesday of the Mar/Jun/Sep/Dec cycle strictly after horizon, ModifiedFollowing-adjusted (ordinal zero returns \`Err(TenorError::ImmOrdinalZero)\`); (7) BrokenDate → civil date parsed from the broken-date tag, ModifiedFollowing-adjusted. All standard-ladder tenors (Weeks/Months/Years/Imm) are anchored on \`spot\`, not \`horizon\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.expiry\_for\_tenor` (hash `44483082a37b5097`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.imm\_date` (hash `50064e2cde4b8a93`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.roll\_period` (hash `39a80df0f377db3b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.first\_imm\_month\_on\_or\_after

- **claim** (`cl\_fce83ba87d9ef24c`): Returns the first IMM quarterly cycle month (Mar/Jun/Sep/Dec) that is at or after the supplied month. The mapping is: months 1–3 → March, 4–6 → June, 7–9 → September, 10–12 → December. Pure; output is fully determined by the numeric value of the input month.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.first\_imm\_month\_on\_or\_after` (hash `a66e253790f4c031`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.imm\_date

- **claim** (`cl\_17bdbf1151ed0cda`): \`imm\_date(horizon, n) -\> Date\` returns the nth IMM date (third Wednesday of a quarterly Mar/Jun/Sep/Dec cycle) strictly after \`horizon\` by scanning forward from \`first\_imm\_month\_on\_or\_after(horizon.month())\` in 3-month steps via \`next\_imm\_month\`, counting only candidates strictly greater than horizon. \`n\` is 1-based; \`n == 0\` is rejected upstream by \`expiry\_for\_tenor\` with \`TenorError::ImmOrdinalZero\`. No allocation; terminates in at most \`n\` quarter-cycle iterations.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.imm\_date` (hash `50064e2cde4b8a93`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.third\_wednesday` (hash `51da29937b32c598`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.is\_t\_plus\_one\_pair

- **claim** (`cl\_21862b87cc4dd89f`): \`spot\_date(pair, horizon) -\> Date\` adds exactly \`spot\_lag\_days(pair)\` business days (T+1 for USD/CAD, USD/TRY, USD/RUB, USD/PHP; T+2 for all other pairs) to \`horizon\` using the pair's composite \`BusinessCalendar\`. \`spot\_lag\_days\` returns \`1\` when \`is\_t\_plus\_one\_pair\` matches, otherwise \`2\`. No side effects; deterministic over inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.is\_t\_plus\_one\_pair` (hash `e60ecaaa4009c3c1`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.spot\_date` (hash `770cb27c3ab1c0b4`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.spot\_lag\_days` (hash `1a94d6c62506f05a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.fx.next\_imm\_month

- **claim** (`cl\_ef6e0eafb4e981db`): Advances the IMM cycle to the next quarterly month, rolling the year at December. Inputs restricted to the four IMM months: March→June (same year), June→September (same year), September→December (same year), December→March (year+1). Pure; no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.next\_imm\_month` (hash `0b5687d24cb7cba6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_australia\_holiday

- **claim** (`cl\_60b5d5f99e4aae4a`): Returns true if \`d\` is an Australian national public holiday: New Year (next-weekday-observed), Australia Day (next-weekday-observed Jan 26), Good Friday, Easter Monday, Anzac Day (Apr 25, observed on the day — no weekend shift), Sovereign's Birthday (2nd Mon Jun), or the Commonwealth Christmas substitute (Dec 25/26 with non-colliding substitute days). Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_australia\_holiday` (hash `aa768f6ce3f79028`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_canada\_holiday

- **claim** (`cl\_05450c7f3e7f36a2`): Returns true if \`d\` is a Canadian statutory holiday: New Year (next-weekday-observed), Good Friday, Victoria Day, Canada Day (next-weekday-observed Jul 1), Labour Day (1st Mon Sep), Truth & Reconciliation Day (next-weekday-observed Sep 30, from 2021), Thanksgiving (2nd Mon Oct), Remembrance Day (next-weekday-observed Nov 11), or the Commonwealth Christmas substitute. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_canada\_holiday` (hash `9e0f5357f12e212d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_japan\_holiday

- **claim** (`cl\_d14a63c5d5bf5828`): Returns true if \`d\` is a Japan bank holiday: Dec 31, Jan 2, Jan 3 (bank new-year closure), any Japan statutory public holiday (via \`is\_japan\_public\_holiday\`), any transfer holiday (via \`is\_japan\_transfer\_holiday\`), or a citizens' holiday (via \`is\_japan\_citizens\_holiday\`). Pure; no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_japan\_holiday` (hash `086bf82d7785df31`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_japan\_public\_holiday

- **claim** (`cl\_5ac837bbf4e50fa0`): Returns true if \`d\` is a Japan statutory public holiday: fixed dates (1 Jan, 11 Feb, 23 Feb from 2020, 29 Apr, 3/4/5 May, 3/23 Nov) plus four Happy-Monday movable holidays (Coming of Age 2nd Mon Jan, Marine Day 3rd Mon Jul, Respect for the Aged 3rd Mon Sep, Sports Day 2nd Mon Oct) plus the astronomical spring and autumn equinoxes (approximation valid 1980–2099). Does not include substitute or citizens' holidays. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_japan\_public\_holiday` (hash `e7435e520c146206`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_mexico\_holiday

- **claim** (`cl\_b29c86e82d30297f`): Returns true if \`d\` is a Mexican banking non-working day: New Year (Jan 1, exact), Constitution Day (1st Mon Feb), Benito Juárez birthday (3rd Mon Mar), Labour Day (May 1, exact), Independence Day (Sep 16, exact), Revolution Day (3rd Mon Nov), Christmas (Dec 25, exact). No weekend-shift substitution; movable holidays are already on Monday. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_mexico\_holiday` (hash `8a1184da56777d26`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_newzealand\_holiday

- **claim** (`cl\_4f7512ada9279b5b`): Returns true if \`d\` is a New Zealand public holiday: New Year's Day (Mondayised Jan 1) and the Day after New Year (Mondayised Jan 2, advanced one extra day if it collides with New Year's observed date), Waitangi Day (Mondayised Feb 6), Good Friday, Easter Monday, Anzac Day (Mondayised Apr 25), King's Birthday (1st Mon Jun), Labour Day (4th Mon Oct), or the Commonwealth Christmas substitute. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_newzealand\_holiday` (hash `390e4ec17ecb1829`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_norway\_holiday

- **claim** (`cl\_95e5c803824a64bd`): Returns true if \`d\` is a Norwegian public holiday: New Year (Jan 1), Maundy Thursday, Good Friday, Easter Monday, Labour Day (May 1), Constitution Day (May 17), Ascension, Whit Monday, Christmas Day (Dec 25), Second Day of Christmas (Dec 26). No weekend shifts — all observed on the exact calendar date. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_norway\_holiday` (hash `6934c0cef7b48a6f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_sweden\_holiday

- **claim** (`cl\_0457ec1dae785784`): Returns true if \`d\` is a Swedish banking holiday: New Year (Jan 1), Epiphany (Jan 6), Good Friday, Easter Monday, Ascension, Labour Day (May 1), National Day (Jun 6), Midsummer Eve (Friday between Jun 19–25 via \`swedish\_midsummer\_eve\`), Christmas Eve (Dec 24), Christmas Day (Dec 25), Boxing Day (Dec 26), New Year's Eve (Dec 31). Swedish banks also close Dec 24 and Dec 31 — the 'mellandagar' closure. No weekend shifts. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_sweden\_holiday` (hash `4476e044c62342a6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_switzerland\_holiday

- **claim** (`cl\_9dbf93bfafcb0554`): Returns true if \`d\` is a Swiss banking holiday: New Year (Jan 1), Berchtold's Day (Jan 2), Good Friday, Easter Monday, Labour Day (May 1), Ascension, Whit Monday, National Day (Aug 1), Christmas (Dec 25), St Stephen's Day (Dec 26). All dates observed on the calendar day itself — no weekend shift. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_switzerland\_holiday` (hash `ff1d5a105aaa2cb4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_target2\_holiday

- **claim** (`cl\_a0b4bc9701cb912b`): Returns true if \`d\` is a TARGET2 system holiday: 1 Jan, Good Friday, Easter Monday, 1 May (Labour Day), 25 Dec, or 26 Dec. No weekend-shift substitution — all dates are observed exactly on the calendar date. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_target2\_holiday` (hash `b42c1eb6ae1f43e1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_uk\_holiday

- **claim** (`cl\_51a0b1e3a8b5a747`): Returns true if \`d\` is a UK bank holiday: New Year's Day (next-weekday-observed), Good Friday, Easter Monday, first Monday in May (Early May), last Monday in May (Spring), last Monday in August (Summer), or the UK Christmas substitute (handled by \`is\_uk\_christmas\_substitute\`). No other dates qualify. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_uk\_holiday` (hash `c2c9cb445f84d6fe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_us\_holiday

- **claim** (`cl\_f0c45a05ae7e8eaf`): Returns true if \`d\` is a US federal holiday (observable date): New Year (us\_observed Jan 1), MLK Day (3rd Mon Jan), Washington's Birthday (3rd Mon Feb), Memorial Day (last Mon May), Juneteenth (us\_observed Jun 19, from 2021 onwards), Independence Day (us\_observed Jul 4), Labor Day (1st Mon Sep), Thanksgiving (4th Thu Nov), Christmas (us\_observed Dec 25). Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.is\_us\_holiday` (hash `cd9f36faa47bf3b2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.mondayise

- **claim** (`cl\_c01b29cee748a3d1`): New Zealand / Commonwealth Mondayisation rule: a date falling on Saturday is shifted to Monday (+2 days); Sunday is shifted to Monday (+1 day); any other weekday is returned unchanged. Distinguished from \`next\_weekday\_observed\` only by name/docstring context; the numeric shifts are identical. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.mondayise` (hash `9cac0a882ce6d96a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.next\_weekday\_observed

- **claim** (`cl\_28f2c0aaff7c5bb7`): Commonwealth next-business-day observation rule: a fixed-date holiday falling on Saturday or Sunday is observed on the following Monday (\`d + 2\` or \`d + 1\` day respectively); weekday dates are returned unchanged. Used for AU/CA/NZ/UK New Year and other fixed-date holidays. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.next\_weekday\_observed` (hash `aee8c8cc0790df1a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.sa\_observed

- **claim** (`cl\_9e22e6035bfd9814`): South-Africa observation rule: a fixed-date holiday falling on Sunday is observed the following Monday (+1 day); Saturday dates are NOT shifted (unlike the US rule). Weekday dates are returned unchanged. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.sa\_observed` (hash `c901090570ed5e59`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.us\_observed

- **claim** (`cl\_3e65243422021623`): Standard US federal-holiday weekend-shift rule: a fixed-date holiday falling on Saturday is observed the preceding Friday (\`d - 1 day\`); one falling on Sunday is observed the following Monday (\`d + 1 day\`); weekday dates are unchanged. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.us\_observed` (hash `0d397da7aa0825dd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.CliAsset.label

- **claim** (`cl\_ed3cfa8cd14d922c`): \`CliAsset::label\` is a \`const fn\` that maps the four asset class variants to their lowercase CLI tokens: \`Fx\` → \`"fx"\`, \`Equity\` → \`"equity"\`, \`Commodity\` → \`"commodity"\`, \`Crypto\` → \`"crypto"\`. These tokens are the canonical asset-class discriminators used by the CLI routing layer to select the correct pricing path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.CliAsset.label` (hash `25426e8f0a9ae74b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.CliBarrier.style

- **claim** (`cl\_b67546aa8537114e`): CliBarrier.style is a const fn that classifies barrier topology into knock-out/knock-in: DownAndOut and UpAndOut map to BarrierStyle::KnockOut; DownAndIn and UpAndIn map to BarrierStyle::KnockIn. This classification is used upstream to select the in-out parity formula.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.CliBarrier.style` (hash `f581605b13562524`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.DeltaConvention.from

- **claim** (`cl\_da4311542a1b0937`): \`DeltaConvention::from(CliDeltaConvention)\` maps the four CLI delta-convention tokens to the wire analytic enum: \`SpotUnadj\` → \`SpotUnadjusted\`, \`ForwardUnadj\` → \`ForwardUnadjusted\`, \`SpotPa\` → \`SpotPremiumAdjusted\`, \`ForwardPa\` → \`ForwardPremiumAdjusted\`. The abbreviated CLI names expand to full analytics-spec names without information loss.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.DeltaConvention.from` (hash `8d7b387c678810c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.DigitalKind.from

- **claim** (`cl\_7eab4afd16ac8ac6`): \`DigitalKind::from(CliDigital)\` always produces a \`DigitalKind\` with \`style: DigitalStyle::CashOrNothing\`; the CLI exposes only cash-or-nothing digitals (no asset-or-nothing variant). \`DigitalCall\` → \`{CashOrNothing, Call}\`, \`DigitalPut\` → \`{CashOrNothing, Put}\`. This constrains the CLI surface to the single most common digital payoff and prevents accidental asset-or-nothing dispatch.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.DigitalKind.from` (hash `7afe44ad0311b19a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.OptionType.from

- **claim** (`cl\_3df71f48e6feadd4`): OptionType::from(CliOptionType) is a total, bijective enum projection: CliOptionType::Call maps to OptionType::Call and CliOptionType::Put maps to OptionType::Put, covering both variants exhaustively. This preserves the call/put flag without ambiguity across the CLI-to-domain boundary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.OptionType.from` (hash `37fa980fef4a8534`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-cli.src.args.SettlementStyle.from

- **claim** (`cl\_c07143832a29b701`): SettlementStyle::from(CliSettlementStyle) is a total, bijective enum projection: CliSettlementStyle::Linear maps to SettlementStyle::Linear and CliSettlementStyle::InverseCoin maps to SettlementStyle::InverseCoin, covering both crypto settlement conventions exhaustively. This ensures the CLI surface faithfully routes the payoff-currency choice (base-asset linear vs. coin-inverse) to the crypto-vanilla pricer.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.args.SettlementStyle.from` (hash `5a056b95f0cc8df6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.src.basket.BasketKind.from

- **claim** (`cl\_4e14edd4efe8f9d2`): BasketKind::from(CliBasketKind) is a total, bijective enum projection across all three basket payoff types: CliBasketKind::Basket → BasketKind::Basket (arithmetic blend), CliBasketKind::BestOf → BasketKind::BestOf, CliBasketKind::WorstOf → BasketKind::WorstOf. Every variant is covered exhaustively with no default arm, ensuring the CLI parser cannot silently collapse distinct multi-asset payoff structures.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.basket.BasketKind.from` (hash `c1a435b73bec9b3e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-cli.src.basket.format\_report

- **claim** (`cl\_fcfbb111398c037a`): basket format\_report emits a header line with kind (basket/best-of/worst-of), option side, strike, expiry, and leg count; then one line per leg showing label/weight/spot/vol/r\_for; then a price + std\_error footer with the parenthetical note "Monte-Carlo; multi-asset Greeks deferred". Greeks are explicitly deferred because the MC estimator does not compute them.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.basket.format\_report` (hash `fd390c0f79f747e3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.src.basket.parse\_leg

- **claim** (`cl\_7a07b2813a5314dc`): parse\_leg parses a colon-separated basket leg spec "PAIR:WEIGHT:SPOT:VOL:R\_FOR" into a ParsedLeg, enforcing: exactly 5 colon-separated tokens; non-empty pair label; spot strictly positive and finite; vol non-negative and finite; weight and r\_for finite (any real sign allowed). Any violation returns Err with an actionable string naming the offending field.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.basket.parse\_leg` (hash `3a8a161fff5f88f0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-cli.src.cli.AtmConvention.from

- **claim** (`cl\_d666e4a21b915475`): \`AtmConvention::from(CliAtmConvention)\` maps the two CLI ATM convention tokens to their wire counterparts: \`AtmForward\` → \`AtmConvention::AtmForward\`; \`DeltaNeutral\` → \`AtmConvention::DeltaNeutralStraddle\`. The deliberate name asymmetry (\`DeltaNeutral\` → \`DeltaNeutralStraddle\`) is load-bearing: it ensures the CLI's terse user-facing label resolves to the full analytics-spec name on the wire.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.cli.AtmConvention.from` (hash `98ff4549a4895edb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-cli.src.cli.default\_underlying

- **claim** (`cl\_f6014b170e84b4da`): default\_underlying returns the canonical demo underlying symbol for each asset class when --underlying is omitted from the CLI: Fx→"EURUSD", Equity→"EQUITY", Commodity→"COMMODITY", Crypto→"BTCUSDT". This is a pure lookup with no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.cli.default\_underlying` (hash `8d51d70a8da199c2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-cli.src.cli.nominal\_tenor\_years

- **claim** (`cl\_f300b1f7d0610fe6`): nominal\_tenor\_years converts a Tenor to its ACT/365-fixed year fraction using exact rational constants: Overnight=1/365, TomNext=2/365, SpotNext=3/365, Weeks(n)=n\*7/365, Months(n)=n/12, Years(n)=n, Imm(n)=n\*0.25 (nominal quarters). BrokenDate returns None because only the calendar-resolved path yields its exact year fraction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.cli.nominal\_tenor\_years` (hash `fc4233081a26e76e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.src.cli.resolve\_priced\_expiry

- **claim** (`cl\_a19ea6b57c46e5a2`): DELIVERABLE cli-stream-rfq-tenor-expiry-drift = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "--tenor 3M silently streams a 1Y-priced quote labelled 3M"; reconciled against the live graph). \`resolve\_priced\_expiry(pair, tenor, explicit, horizon)\` is a pure, deterministic function (reads only its borrowed args, returns Result\<f64, DispatchError\>, mutates nothing) that DERIVES the priced expiry-year-fraction from the requested tenor via celnet\_conventions::vol\_year\_fraction over the pair's calendar, and — when an explicit --expiry-years is also supplied — rejects any value that drifts from the tenor-derived anchor beyond the abs/rel tolerance with DispatchError. The label and the priced expiry can no longer drift apart silently across the CLI stream/rfq seams (regression-pinned by stream\_and\_rfq\_reject\_a\_contradictory\_tenor\_expiry\_pair and priced\_expiry\_derives\_from\_tenor\_via\_the\_conventions\_calendar). SELF-INVALIDATES on any change to this resolver.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.cli.resolve\_priced\_expiry` (hash `a59e54bb4d1043bc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.src.exotic.format\_report

- **claim** (`cl\_fdab8748ecd4bf6b`): exotic format\_report routes the output format by ExoticSpec variant: VarianceSwap emits fair\_variance and its square root as fair\_vol; VolatilitySwap emits only fair\_vol; any MC-priced product (std\_error is Some) emits price + std\_error; all closed-form products emit only price. The label string uniquely identifies the sub-variant (e.g. "american-fd" vs "american-lsm", "lookback-continuous" vs "lookback-discrete").
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.exotic.format\_report` (hash `5669a8aa742ec886`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.CliMargining.label

- **claim** (`cl\_4db62b44ba13cd0b`): \`CliMargining::label\` is a \`const fn\` that maps the two futures-option margining variants to their canonical kebab-case CLI tokens: \`EquityStyle\` → \`"equity-style"\`, \`FuturesStyle\` → \`"futures-style"\`. Being \`const\`, it is usable in static contexts and compile-time assertions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.CliMargining.label` (hash `01cb10bf58574b65`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.Margining.from

- **claim** (`cl\_148ac41140825d16`): Margining::from(CliMargining) is a total, bijective enum projection: CliMargining::EquityStyle maps to Margining::EquityStyle and CliMargining::FuturesStyle maps to Margining::FuturesStyle, covering every variant exhaustively. This ensures the CLI layer faithfully conveys the option margining convention (premium-upfront equity-style vs. daily-settled futures-style) to the commodity/future-option pricer without truncation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.Margining.from` (hash `170a1b3228f5256a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.format\_report

- **claim** (`cl\_408746c9a41916fc`): future\_option format\_report emits the contract identity as "TICKER@MIC" when venue is non-empty or bare ticker when empty; routes rate sensitivities from the CarryGreeks to discount\_rho/carry\_rho (always the Carry variant for on-future options, with the Fx fallback being structurally unreachable); and emits the full Greek surface including delta (as delta\_forward, not delta\_spot) at standard precision.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.future\_option.format\_report` (hash `955bcad28b34b2ee`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-cli.src.linear.FixingSource.from

- **claim** (`cl\_977290cd0e4b5b22`): \`FixingSource::from(CliFixing)\` is a total, exhaustive bijection from the CLI fixing enum to the domain \`FixingSource\` type, covering all six exotic-NDF settlement fixings (KrwKftc18, TwdTaipei, InrRbiRef, BrlPtax, ClpDolarObs, CopTrm). No wildcard branch — adding a fixing source to one enum without the other is a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.linear.FixingSource.from` (hash `aa6403eea746ff8b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-cli.src.linear.LinearSide.from

- **claim** (`cl\_285336166673231e`): LinearSide::from(CliSide) is a total, bijective enum projection: CliSide::Buy maps to LinearSide::Buy and CliSide::Sell maps to LinearSide::Sell, covering every variant exhaustively with no default arm. This guarantees that the CLI boundary introduces no information loss or reinterpretation of trade direction when constructing linear instrument inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.linear.LinearSide.from` (hash `8e493ad05770f724`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-cli.src.perpetual.PerpetualMarket.carry

- **claim** (`cl\_772c2ba3850e8cba`): PerpetualMarket.carry maps the CLI asset class to the correct Carry variant: for Fx it produces Carry::FxRates{r\_dom, r\_for}; for Equity, Commodity, and Crypto it produces Carry::CostOfCarry{r: r\_dom, b: r\_dom - r\_for}, where b = cost-of-carry = domestic rate minus foreign/dividend yield.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.perpetual.PerpetualMarket.carry` (hash `90cd63fe016e572f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-cli.src.perpetual.format\_report

- **claim** (`cl\_0b6c11b31322b842`): perpetual format\_report always appends theta=0 with the comment "exact: the perpetual value is time-homogeneous". It also branches on RateSensitivities: for Fx it emits rho\_dom/rho\_for; for Carry it emits discount\_rho/carry\_rho. The perpetual theta is mathematically exact at zero because the value function has no explicit time dependency.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.perpetual.format\_report` (hash `604219b581f3de23`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-cli.src.price.atm\_forward\_strike\_equals\_forward

- **claim** (`cl\_d2ec11bd2b8ad34f`): The \`price\` module's ATM-Forward strike resolver sets the solved strike equal to the theoretical forward price \`spot \* exp((r\_dom - r\_for) \* t)\` to within 1e-12: \`StrikeSpec::Atm { atm: AtmConvention::AtmForward, .. }\` resolves to \`VanillaInputs::forward()\` on the same market parameters.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.price.atm\_forward\_strike\_equals\_forward` (hash `5441038d13ec11bb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.src.price.delta\_spec\_round\_trips\_to\_target\_delta

- **claim** (`cl\_bd48733f3bfb02c8`): The delta-spec solver in the \`price\` module is self-inverse with tolerance 1e-9: solving a strike from a 25Δ target via \`StrikeSpec::Delta { target: 0.25, convention }\` and then computing the convention delta of that result recovers exactly 0.25. The resolved strike additionally matches \`celnet\_vanilla::strike\_from\_delta\` called directly on the same inputs to 1e-14, confirming the CLI adds no solver indirection.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.price.delta\_spec\_round\_trips\_to\_target\_delta` (hash `9c2f5e8a08350c79`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-cli.src.price.format\_report\_for

- **claim** (`cl\_c3f666a577d7300a`): price format\_report\_for renders the full first- and second-order Greek surface for a vanilla option: price, delta\_spot, delta\_forward, conv\_delta (with delta\_convention label), gamma, vega, theta, rho\_dom, rho\_for, vanna, volga, charm, speed, zomma, color — all at 10 decimal places. The settlement style (linear vs inverse-coin) is included in the header.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.price.format\_report\_for` (hash `a8c573d36fdd840d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-cli.src.price.outright\_matches\_direct\_vanilla

- **claim** (`cl\_5988084b6b22d13c`): The \`price\` module's \`run\` function, exercised by \`outright\_matches\_direct\_vanilla\`, produces price/vega/gamma and a convention delta that are bit-for-bit identical (tolerance 1e-14) to the \`celnet\_vanilla::greeks\` and \`celnet\_vanilla::convention\_delta\` functions called directly on the same \`VanillaInputs\`. This cross-check proves the CLI's market-to-inputs pipeline introduces zero numerical drift for the outright-strike case.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.price.outright\_matches\_direct\_vanilla` (hash `c4c66251af91adfe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.src.rfq.Side.from

- **claim** (`cl\_f25e0f87989d103f`): Side::from(CliAcceptSide) is a total, bijective enum projection: CliAcceptSide::Buy maps to Side::Buy and CliAcceptSide::Sell maps to Side::Sell, covering both variants exhaustively. This guarantees that RFQ accept-side direction is conveyed without loss from the CLI parser to the RFQ engine's trade-side domain type.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.Side.from` (hash `5c6af6f5fb2f1c00`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-cli.src.rfq.countdown

- **claim** (`cl\_3fa218b63b41ff29`): countdown renders a DealerQuote's last-look window as either a three-decimal-place seconds string (e.g. "2.500s") when the window is still open, or the literal string "expired" when it has closed. It delegates entirely to DealerQuote::last\_look\_remaining(now\_nanos) and adds only the formatting layer.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.countdown` (hash `df0ce6fd217f36bf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-cli.src.rfq.format\_panel

- **claim** (`cl\_960e2e8f855047e5`): The \`format\_panel\` function in the \`rfq\` module renders RFQ panel rows using Rust's \`{}\` (shortest-round-trip) float formatter, guaranteeing that parsing the printed bid/offer recovers the exact \`f64\` bit pattern. This is verified by \`ladder\_prices\_round\_trip\_bit\_for\_bit\`: \`col(row, "bid").to\_bits() == (0.1\_f64 + 0.2).to\_bits()\` and \`col(row, "offer").to\_bits() == 0.32\_f64.to\_bits()\`, including the canonical \`0.1 + 0.2\` rounding case.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.format\_panel` (hash `4d6c9dc12e221ace`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.ladder\_prices\_round\_trip\_bit\_for\_bit` (hash `3752117909cf2fe9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.src.risk.OrgDimension.from

- **claim** (`cl\_2a1081aa386113fa`): \`OrgDimension::from(CliDimension)\` is a lossless, total bijection from the CLI dimension enum to the wire \`OrgDimension\` type: every CLI variant maps to its identically-named wire counterpart (Firm, Trader, Book, Desk, CcyPair, Location, Entity) with no default branch, guaranteeing that adding a CLI variant without a matching wire variant is a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.risk.OrgDimension.from` (hash `49d96663fb20f624`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-cli.src.risk.parse\_dimension

- **claim** (`cl\_2b837230bcf2e06d`): parse\_dimension maps lower-case dimension name strings to OrgDimension enum variants: "firm"→Firm, "trader"→Trader, "book"→Book, "desk"→Desk, "ccy-pair"/"ccypair"/"pair"→CcyPair, "location"→Location, "entity"→Entity. Any other value returns Err with a formatted message identifying the unknown token.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.risk.parse\_dimension` (hash `13929ac111d1e738`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-cli.src.surface.atm\_vol\_recovered\_at\_atm\_strike

- **claim** (`cl\_97923329422eb418`): The \`surface\` module's smile calibration pipeline guarantees arbitrage-freedom on a standard 25Δ broker slice: \`SurfaceResult::arbitrage.is\_arbitrage\_free(1e-6)\` must hold for any benign EUR/USD-style input (ATM=10.5%, RR25=-0.5%, BF25=0.2%). Additionally, the calibrated smile reprices the ATM vol at the ATM strike to within 1e-9 (\`r.smile.implied\_vol(r.atm\_strike, r.forward, 1.0).0 ≈ atm\_vol\`), verifying the pipeline's internal consistency.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.surface.atm\_vol\_recovered\_at\_atm\_strike` (hash `6d6a6cc94083af5b`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-cli.src.surface.calibrated\_slice\_is\_arbitrage\_free` (hash `2e04c0965d0798fe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-cli.src.tenor.format\_tenor

- **claim** (`cl\_a5b2aa6bb535f640`): format\_tenor is the exact left-inverse of parse\_tenor for the non-broken-date arms: Overnight→"ON", TomNext→"TN", SpotNext→"SN", Weeks(n)→"\<n\>W", Months(n)→"\<n\>M", Years(n)→"\<n\>Y", Imm(n)→"\<n\>IMM", BrokenDate({y,m,d})→"YYYY-MM-DD" (zero-padded). The round-trip parse\_tenor(format\_tenor(t)) == Ok(t) holds for every non-BrokenDate Tenor variant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.tenor.format\_tenor` (hash `25daa33ed62ced08`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-cli.src.tenor.parse\_tenor

- **claim** (`cl\_ffcf8900df4758f1`): parse\_tenor maps the FX market-standard tenor string grammar to the typed Tenor enum: special aliases "ON"/"TN"/"SN" map to Overnight/TomNext/SpotNext; "YYYY-MM-DD" maps to BrokenDate; "\<n\>IMM" (n≥1) maps to Imm(n); "\<count\>\<W\|M\|Y\>" (count≥1) maps to Weeks/Months/Years. Any non-positive count, unknown unit, or malformed string is rejected with a typed TenorParseError — never panics, never silently truncates.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.src.tenor.parse\_tenor` (hash `a94b18f751443859`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.argv\_for

- **claim** (`cl\_2cbf149287445414`): \`argv\_for\` is the canonical CLI argument generator for the conformance test harness: given a \`GoldenVector\` it produces the exact \`argv\` slice that, when passed to the \`celnet\` binary, exercises the same pricing path the vector was generated from. It covers every product family the CLI exposes (vanilla, digital, touch, single\_barrier, var-swap, vol-swap, asian, forward-start, cliquet, quanto, tarf, pivot, accumulator, lookback, american, basket, fx\_forward, fx\_swap, ndf, perpetual\_option, listed\_future\_option) and returns \`None\` for corpus shapes not yet reachable via the CLI (e.g. corridor touches, LSM american). Negative-valued flags (correlation, clamp bounds) are passed in \`--flag=value\` form to prevent clap from misidentifying them as new flags.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.argv\_for` (hash `d820d91979adb270`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.fixing\_token

- **claim** (`cl\_e417c1ddb0b2a050`): \`fixing\_token\` maps the corpus wire enum variant for a non-deliverable fixing source (e.g. \`"KrwKftc18"\`) to the exact kebab-case token the CLI value-enum accepts (\`"krw-kftc18"\`). The mapping is exhaustive over the six currently defined fixing sources; any unknown value panics at test time, preventing silent test-oracle mismatches.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.fixing\_token` (hash `ef482790e190a667`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.listed\_future\_underlying

- **claim** (`cl\_161365994d9c894f`): \`listed\_future\_underlying\` constructs the typed \`Underlying\` for listed-future-option conformance vectors: \`"WTI"\` → \`Underlying::Commodity(CommodityRef { symbol: ("WTI", "XNYM"), ccy: USD })\`, \`"ES"\` → \`Underlying::Equity(EquityRef { symbol: ("ES", "XCME"), ccy: USD })\`. Venue codes (XNYM for NYMEX crude, XCME for CME equity index) are hardcoded, matching the golden corpus symbol definitions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.listed\_future\_underlying` (hash `e8a079c9f829a566`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.margining\_of

- **claim** (`cl\_fd0488f12eebfcfd`): \`margining\_of\` translates the corpus margining term string to the domain \`Margining\` enum used by \`ListedFutureTerms\` construction in \`new\_payoff\_spec\_of\`. This is the typed bridge that ensures the conformance \`InstrumentSpec\` carries the same margining convention the vector was priced under.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.margining\_of` (hash `2822b85c85b8b930`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.margining\_token

- **claim** (`cl\_03e15a6485069e76`): \`margining\_token\` translates the corpus margining string (\`"EQUITY\_STYLE"\` / \`"FUTURES\_STYLE"\`) to its CLI kebab-case token (\`"equity-style"\` / \`"futures-style"\`). Used by \`argv\_for\` to build the \`--margining\` flag for listed-future-option vectors.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.margining\_token` (hash `41b10d11420c361c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.new\_payoff\_spec\_of

- **claim** (`cl\_4bf383a9791446d6`): \`new\_payoff\_spec\_of\` constructs the typed \`InstrumentSpec\` for the new-payoff-shape families (perpetual\_option and listed\_future\_option) in the conformance test, mirroring the same SDK path the client arms use. For perpetuals it parses the FX pair from \`v.underlying\`; for listed futures it assembles \`ListedFutureTerms\` with symbol, expiry, option type, strike, and margining from the vector's terms, then delegates to \`InstrumentSpec::listed\_future\_option\`. Any unrecognised family panics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.new\_payoff\_spec\_of` (hash `c3c9c7d49d18783b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.opt\_token

- **claim** (`cl\_8fdef47e6d53607b`): \`opt\_token\` translates the corpus wire option-type string (\`"CALL"\` / \`"PUT"\`) to the CLI's lowercase clap value-enum token (\`"call"\` / \`"put"\`). The wildcard panics on any unrecognised value, enforcing that the test corpus and the CLI enum remain in lock-step.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.opt\_token` (hash `5fb1f9633fdc9131`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.side\_token

- **claim** (`cl\_e3ce3078bce95b98`): \`side\_token\` translates the corpus wire side field (\`"BUY"\` / \`"SELL"\`) to the lowercase CLI token (\`"buy"\` / \`"sell"\`) for \`--side\` and \`--near-side\` flags on linear products. The wildcard panics, ensuring corpus and CLI enum stay aligned.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-cli.tests.conformance.side\_token` (hash `afefacc3a4b44aed`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-client.src.error.ClientError.source

- **claim** (`cl\_f42be7b0023f43dd`): \`ClientError::source\` exposes the underlying error cause for exactly three variants: \`Transport(e)\` → \`Some(e)\`, \`Status(e)\` → \`Some(e.as\_ref())\`, \`Wire(e)\` → \`Some(e)\`. All other variants (e.g. \`StreamClosed\`, \`Reconnected\`, \`MissingField\`, \`InvalidEndpoint\`) return \`None\`. This is the \`std::error::Error::source\` contract for the crate's public error type. Source: \`ClientError.source\` in \`error.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.error.ClientError.source` (hash `31a4a208097fbb4a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-client.src.idempotency.splitmix64

- **claim** (`cl\_da651cf27339bacd`): \`splitmix64(z: u64) -\> u64\` is a pure bijective mixer used to expand OS-seeded entropy into idempotency-key halves. It applies three fixed multiply-xorshift rounds: \`z += 0x9E3779B97F4A7C15; z = (z ^ z\>\>30) \* 0xBF58476D1CE4E5B9; z = (z ^ z\>\>27) \* 0x94D049BB133111EB; z ^ z\>\>31\`. All operations are wrapping. This is the standard SplitMix64 finaliser (Vigna 2015); its output is uniformly distributed over all 64-bit values for any non-repeating input sequence.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.idempotency.splitmix64` (hash `dcff583be223b68c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-client.src.rfs.RejectReason.from\_wire

- **claim** (`cl\_f39076a5547f276d`): RejectReason::from\_wire is an exhaustive bijection between stream\_reject::Reason {Expired, UnknownToken, AlreadyConsumed} and the client RejectReason enum. All three click-to-trade rejection causes are represented with no fallback.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.rfs.RejectReason.from\_wire` (hash `60bd92cbe455e9f2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.rfs.clone\_err

- **claim** (`cl\_47a5a75601a6b0ed`): \`clone\_err\` produces a lossless clone for the three zero-allocation \`ClientError\` variants (\`StreamClosed\`, \`Reconnected\`, \`MissingField\`) and a direct \`clone()\` for \`InvalidEndpoint\`. For all other variants it downgrades to \`ClientError::Status(unavailable(other.to\_string()))\`, preserving the human-readable message at the cost of losing the original type. This is the documented partial-clone contract for broadcasting an error to multiple waiters. Source: \`clone\_err\` in \`rfs.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.rfs.clone\_err` (hash `3dcf2a9c04dc4d8b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-client.src.rfs.decode\_side

- **claim** (`cl\_c1296264d1b0b238`): decode\_side converts a raw i32 protobuf tag to a client-side Side enum: on a valid tag it delegates to \`Side::from\_wire(celnet\_proto::Side::try\_from(tag))\`; on an unrecognized tag it returns Side::Buy as a safe default. This default is safe because the server only ever stamps BUY or SELL on an executed trade token — TWO\_WAY is never a valid executed side.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.rfs.decode\_side` (hash `d626d8bc5d1f1af4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-client.src.rfs.is\_gap

- **claim** (`cl\_96261e5fc7c10c37`): \`is\_gap(last\_good, observed) -\> bool\` is a pure sequence-integrity predicate: returns \`true\` iff a baseline is established (\`last\_good != 0\`) AND \`observed \> last\_good + 1\`, meaning at least one sequence number was skipped. Returns \`false\` before a baseline is established (no gap can be declared on the very first frame). The complementary \`is\_stale\` returns \`true\` iff baseline is established AND \`observed \<= last\_good\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.rfs.is\_gap` (hash `3c4f360872555292`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.rfs.is\_stale` (hash `dec541a676a81e3c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-client.src.risk.Enforcement.from\_tag

- **claim** (`cl\_e3b0f63fa83ed593`): Enforcement::from\_tag decodes the two limit enforcement modes — Soft (warn only) and Hard (block execution) — from a wire i32 tag. An unknown tag is a protocol error. The two-arm exhaustive match ensures any new enforcement mode added to the proto is a compile-time gap in the client.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.risk.Enforcement.from\_tag` (hash `3f81ddae7a671a2c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-client.src.risk.LimitMetric.from\_tag

- **claim** (`cl\_789fe7158a605c34`): LimitMetric::from\_tag decodes the full set of twelve limit metric kinds from a wire i32 tag: Delta, Gamma, Vega, Vanna, Volga, VegaBucket, TenorVega, ConcentrationDelta, ConcentrationVega, Var, ExpectedShortfall, StopLoss. An unrecognized tag is an error (WireError::UnknownEnum{kind:"LimitMetricKind"}) — there is no silent default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.risk.LimitMetric.from\_tag` (hash `6e803d58d9b885bb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-client.src.risk.OrgDimension.from\_tag

- **claim** (`cl\_57d587d73b912777`): OrgDimension::from\_tag and OrgDimension::to\_wire form a bijection for the seven organizational risk dimensions: Firm, Trader, Book, Desk, CcyPair (wire: Underlying), Location, Entity. The noteworthy alias is CcyPair↔Underlying — the client uses the purpose-name CcyPair while the wire proto uses Underlying. An unknown tag returns WireError::UnknownEnum.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.risk.OrgDimension.from\_tag` (hash `5912f81876c13068`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.risk.OrgDimension.to\_wire` (hash `3330d187123a5c66`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-client.src.risk.Rag.from\_tag

- **claim** (`cl\_4afe5a043085e029`): Rag::from\_tag decodes the four risk traffic-light states from a wire RagStatus tag: Green, Amber, Red, Breach. An unrecognized tag is a protocol error (WireError::UnknownEnum{kind:"RagStatus"}). Breach is a distinct fourth state above Red, used for limit violations that exceed hard-breach thresholds.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.risk.Rag.from\_tag` (hash `ff827a66f823f374`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.from\_wire\_tag

- **claim** (`cl\_c86a5591dbbf07c7`): Observable::from\_wire\_tag decodes a market series observable from an i32 wire tag, returning an error (WireError::UnknownEnum{kind:"MarketObservable"}) on an unrecognized tag rather than using a silent default. Wing observables (RiskReversal, Butterfly) are initialized with delta=0.0 at decode time; the true delta is patched in separately via wing\_delta.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.from\_wire\_tag` (hash `3e295205767ac552`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.wing\_delta

- **claim** (`cl\_77b56750d85e5cc5`): Observable::wing\_delta extracts the delta parameter exclusively from wing observables: it returns Some(delta) for RiskReversal{delta} and Butterfly{delta}, and None for AtmVol, Spot, and Forward. This is the sole accessor for the wing-observable delta scalar and is used to patch decoded snapshots.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.wing\_delta` (hash `e906f68e34a0792c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.wire\_tag

- **claim** (`cl\_d7d00c7d608af0d9`): Observable::wire\_tag converts a client Observable to its i32 protobuf tag by matching to WireObservable and casting to i32. Wing observables (RiskReversal, Butterfly) are collapsed to their base WireObservable variant (losing the delta) because the wire protocol carries delta as a separate field. AtmVol, Spot, and Forward map 1-to-1.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.series.Observable.wire\_tag` (hash `edd37d851f9384cc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.ShockFactor.from\_wire

- **claim** (`cl\_dcdfa164db0eabf2`): ShockFactor::to\_wire and ShockFactor::from\_wire form an exhaustive lossless bijection over the five PnL scenario shock axes: Spot, Vol, RateDom, RateFor, Time — mapping to/from celnet\_proto::shock\_axis::Factor. from\_wire returns WireError::UnknownEnum{kind:"ShockAxis.Factor"} for unrecognized tags; to\_wire has no fallback arm.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.ShockFactor.from\_wire` (hash `00b60edbb5acc154`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.ShockFactor.to\_wire` (hash `690c2bbe1b063013`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.Smile.atm\_vol

- **claim** (`cl\_4a098612a5b19d16`): \`Smile::atm\_vol(&self) -\> Option\<f64\>\` is a pure accessor that returns the ATM volatility by querying \`vol\_at\_delta(0.50)\`. \`vol\_at\_delta\` performs exact pillar lookup with an absolute tolerance \`DELTA\_MATCH\_ABS\`: queries within that tolerance band of a pillar resolve to that pillar's vol; queries between pillars return \`None\` with no aliasing or fallthrough to adjacent pillars. \`atm\_vol\` is therefore \`None\` when the 0.50-delta pillar is absent from the smile.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.Smile.atm\_vol` (hash `60040258267cabca`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.surface\_vocab.vol\_at\_delta\_matches\_within\_tolerance\_without\_pillar\_aliasing` (hash `ff00f7df79c29b20`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.BasketKind.to\_wire

- **claim** (`cl\_9b9f905df4ae5a59`): BasketKind::to\_wire, PricingModel::to\_wire, and Margining::to\_wire are each exhaustive bijections with no fallback: BasketKind{Basket,BestOf,WorstOf}↔celnet\_proto::BasketKind; PricingModel{Default,LocalStochVol}↔celnet\_proto::PricingModel; Margining{EquityStyle,FuturesStyle}↔celnet\_proto::Margining. Exhaustiveness is enforced by the Rust compiler via non-wildcard match arms.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.BasketKind.to\_wire` (hash `186c1b8ea17928c7`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Margining.to\_wire` (hash `1a4c190f007a0602`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.PricingModel.to\_wire` (hash `fb8a52ffcf7e5e0c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.ForwardSide.instrument\_side

- **claim** (`cl\_6fdb8340d50489df`): ForwardSide::to\_wire maps ForwardSide{Buy,Sell} to celnet\_proto::Side{Buy,Sell}, and ForwardSide::instrument\_side maps ForwardSide{Buy,Sell} to the client Side{Buy,Sell}. TwoWay is not in the ForwardSide vocabulary — FX forwards and swaps are always directional.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.ForwardSide.instrument\_side` (hash `14c756749c4af215`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.ForwardSide.to\_wire` (hash `949ffd6b0b0018ca`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.Product.to\_wire

- **claim** (`cl\_1955822c56f4450e`): Product::to\_wire is the canonical, exhaustive serialization of every client-side Product variant to its protobuf instrument::Product oneof. It is self-recursive for Strategy legs. FxSwap serialization constructs both near and far FxForward legs: far carries the opposite ForwardSide of near at the same contract\_rate and notional, making the wire instrument self-describing. Barrier and Touch products always emit MonitoringStyle::Continuous.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Product.to\_wire` (hash `2f058807b42bb3f1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.Seat.from\_wire

- **claim** (`cl\_7eface47184a62d0`): Seat::from\_wire deserializes a celnet\_proto::Owner to a client Seat: Trader(id) and AutoPricer(id) map to the corresponding Seat variants; a missing \`seat\` oneof field returns Err(ClientError::MissingField("Owner.seat")) — the seat field is required and its absence is a protocol error, not a default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Seat.from\_wire` (hash `59445824233aaa5d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.Seat.to\_wire

- **claim** (`cl\_222902a63d6151c6`): Seat::to\_wire serializes a client Seat to celnet\_proto::Owner: Trader(id) and AutoPricer(id) each produce their respective owner::Seat variant wrapped in Owner{seat:Some(...)}. The Some wrapper is always present.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Seat.to\_wire` (hash `fe5537f9729bee87`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.Side.from\_wire

- **claim** (`cl\_e0a11b4ae9ba9a61`): Side::to\_wire and Side::from\_wire form an exhaustive, lossless bijection between the client Side enum {Buy, Sell, TwoWay} and the protobuf celnet\_proto::Side enum. Every variant maps 1-to-1 with no fallback or lossy path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Side.from\_wire` (hash `94cad465b4901648`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.Side.to\_wire` (hash `1751c858231a1181`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.StrategyKind.to\_wire

- **claim** (`cl\_73cb98ce4d4a329c`): StrategyKind::to\_wire is an exhaustive bijection mapping {RiskReversal, Strangle, Straddle, Seagull} to the corresponding celnet\_proto::StrategyKind variants. The four-arm match has no wildcard, ensuring any new strategy kind added to the client type causes a compile error until the wire arm is provided.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.StrategyKind.to\_wire` (hash `b0bc89feb0bd47d6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.StrikeSpec.to\_wire

- **claim** (`cl\_311b4b6ed108fd2b`): StrikeSpec::to\_wire serializes the client strike/delta union to celnet\_proto::StrikeOrDelta: Absolute(k) maps to strike\_or\_delta::Spec::Strike(k) and Delta(d) maps to Spec::Delta(d), wrapped in StrikeOrDelta{spec:Some(...)}. The Some wrapper is always present — a None spec is never emitted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.StrikeSpec.to\_wire` (hash `0a52c4de68322f48`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.american\_terms\_encode\_to\_the\_wire\_arm

- **claim** (`cl\_61fb5a4ff9e598fe`): American \`InstrumentSpec\` wire encoding: the American/FD path maps to \`instrument::Product::American\` with \`exercise\_style = ExerciseStyle::American\`, \`bermudan\_dates\` empty, and \`lsm\_paths = 0\`. The Bermudan/LSM path maps to \`exercise\_style = ExerciseStyle::Bermudan\`, \`bermudan\_dates.len() == number\_of\_exercise\_dates\`, \`lsm\_paths = mc\_paths\`, \`lsm\_exercise\_dates = mc\_exercise\_dates\`, \`lsm\_seed = seed\`. Source: \`american\_terms\_encode\_to\_the\_wire\_arm\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.american\_terms\_encode\_to\_the\_wire\_arm` (hash `298eb4ffd9f92c14`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.basket\_terms\_encode\_to\_the\_wire\_arm

- **claim** (`cl\_97edfe07fe3b5e22`): Basket \`InstrumentSpec\` wire encoding: \`InstrumentSpec::basket(…, BasketTerms)\` serializes to \`instrument::Product::Basket\` with all leg \`spot\` and \`vol\` fields bit-for-bit identical to the input (using \`.to\_bits()\` comparison), the full correlation flat-array preserved, \`kind\` mapping to \`BasketKind\`, and MC simulation parameters (\`mc\_paths\`, \`mc\_replications\`, \`mc\_seed\`) carried without loss. Source: \`basket\_terms\_encode\_to\_the\_wire\_arm\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.basket\_terms\_encode\_to\_the\_wire\_arm` (hash `bdd7e5ab849e9475`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.calibration\_from\_wire

- **claim** (`cl\_77a0ea871efac883`): calibration\_from\_wire decodes a SmileModel wire tag with a safe fallback: if the i32 tag is a recognized SmileModel proto variant it attempts to convert to the client Calibration type; any conversion failure (unknown or unimplemented variant) returns SmileModel::MarketHedge. This is a provenance annotation, not a correctness gate — the caller already holds a valid calibrated smile.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.calibration\_from\_wire` (hash `f9168ae9e1f6aa89`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.fixing\_source\_to\_wire

- **claim** (`cl\_82a3f730c4811f9f`): fixing\_source\_to\_wire is an exhaustive, lossless bijection for the six NDF fixing sources: KrwKftc18, TwdTaipei, InrRbiRef, BrlPtax, ClpDolarObs, CopTrm each map exactly to the corresponding celnet\_proto::FixingSource variant with no wildcard or default arm, preventing silent mis-mapping.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.fixing\_source\_to\_wire` (hash `7db9876aab4db639`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.listed\_future\_terms\_encode\_to\_the\_wire\_arm

- **claim** (`cl\_d67c5f915ed4a8e5`): Listed-future-option wire encoding: \`InstrumentSpec::listed\_future\_option\` serializes \`expiry\_years\` at the outer instrument level (not inside the product arm), carries \`future\_symbol.ticker\` and \`future\_symbol.venue\` as non-null strings, preserves \`future\_expiry\_years\`, \`option\_type\`, \`strike\`, \`notional\`, and \`margining\` bit-for-bit. Source: \`listed\_future\_terms\_encode\_to\_the\_wire\_arm\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.listed\_future\_terms\_encode\_to\_the\_wire\_arm` (hash `973009cfc84d8389`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-client.src.vocab.perpetual\_terms\_encode\_to\_the\_wire\_arm

- **claim** (`cl\_8db2f28ee3f27251`): Perpetual option wire encoding: \`InstrumentSpec::perpetual\` encodes to \`instrument::Product::PerpetualOption\` with \`expiry\_years == 0.0\` (bit-exact) and \`tenor == None\` on the wire — the contract has no tenor or expiry. Strike and notional are bit-identical to the builder inputs. Source: \`perpetual\_terms\_encode\_to\_the\_wire\_arm\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.src.vocab.perpetual\_terms\_encode\_to\_the\_wire\_arm` (hash `27cc65d0ba2c4f30`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-client.tests.conformance.cp

- **claim** (`cl\_0e7594d3d866d272`): \`cp\` (conformance helper) is the canonical bijection from the corpus option-type string token to \`OptionType\`: \`"CALL"\` → \`OptionType::Call\`, \`"PUT"\` → \`OptionType::Put\`. Any other token is a test-corpus defect and panics. Used uniformly by \`instrument\_of\` across all product families. Source: \`cp\` in \`conformance.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.conformance.cp` (hash `1e35ed4523006bae`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-client.tests.conformance.fixing\_source

- **claim** (`cl\_cb202f55cf4972a4`): \`fixing\_source\` is the canonical token→\`FixingSource\` map for all NDF fixing sources covered by the conformance corpus: \`"KrwKftc18"\`, \`"TwdTaipei"\`, \`"InrRbiRef"\`, \`"BrlPtax"\`, \`"ClpDolarObs"\`, \`"CopTrm"\`. Any unrecognised token panics — no silent default. Source: \`fixing\_source\` in \`conformance.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.conformance.fixing\_source` (hash `d3ea8b798c613eed`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-client.tests.conformance.forward\_side

- **claim** (`cl\_d594764f460ab6fc`): \`forward\_side\` is the canonical bijection for linear-product directional sides in the conformance corpus: \`"BUY"\` → \`ForwardSide::Buy\`, \`"SELL"\` → \`ForwardSide::Sell\`. Any other token panics. Used for FX-forward, FX-swap, and NDF vector terms. Source: \`forward\_side\` in \`conformance.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.conformance.forward\_side` (hash `ea56763542bf178f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-client.tests.conformance.listed\_future\_underlying

- **claim** (`cl\_96f5ee5be6d3f48a`): \`listed\_future\_underlying\` maps the two conformance-corpus listed-future underlying tokens to typed \`Underlying\` variants: \`"WTI"\` → \`Underlying::Commodity(CommodityRef::new(Symbol{"WTI","XNYM"}, USD))\`, \`"ES"\` → \`Underlying::Equity(EquityRef::new(Symbol{"ES","XCME"}, USD))\`. Source: \`listed\_future\_underlying\` in \`conformance.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.conformance.listed\_future\_underlying` (hash `7dea7a093607fff0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-client.tests.conformance.margining

- **claim** (`cl\_68b094d7cc458965`): \`margining\` maps the listed-future premium-margining token to \`Margining\`: \`"EQUITY\_STYLE"\` → \`Margining::EquityStyle\`, \`"FUTURES\_STYLE"\` → \`Margining::FuturesStyle\`. No other values are accepted; any unknown token panics. Source: \`margining\` in \`conformance.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.conformance.margining` (hash `c74878846fe5237e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-client.tests.multi\_dealer\_workflow.law\_winner

- **claim** (`cl\_562ceac5f1cc3dfe`): The local reference implementation of multi-dealer best-bid/offer law: among all rows with \`valid\_until\_nanos \>= now\_nanos\`, the winner on the offer side is the row with the lowest offer price; ties are broken by lexicographic minimum \`lp\_id\`. On the bid side the winner has the highest bid, same tie-break. Returns \`None\` when every row is expired. Formula: \`argmin\_{d valid} d.price.offer\` (offer), \`argmax\_{d valid} d.price.bid\` (bid), tie → \`min lp\_id\`. Source: \`law\_winner\` in \`multi\_dealer\_workflow.rs\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-client.tests.multi\_dealer\_workflow.law\_winner` (hash `1f3ab251173f7a66`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.CommodityInputs.on\_future

- **claim** (`cl\_58c40b2a069d3f5b`): \`CommodityInputs::on\_future\` is the canonical smart constructor for an option priced directly on a futures price: it sets spot := future (the futures price), carry b := 0.0 (no drift on a futures price under risk-neutral measure), and r to the discount rate. The test \`future\_equals\_spot\_reparameterization\` pins the equivalence: on\_future(F, K, σ, t, r) produces the same price as on\_spot(S, K, σ, t, r, convenience) when F = S·e^{b·t} — confirming the two constructors are equivalent reparameterizations of the same model, not two distinct models. const fn: evaluates to a struct literal at compile time, no writes/allocation/IO.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.CommodityInputs.on\_future` (hash `03fc950447dc229c`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.future\_equals\_spot\_reparameterization` (hash `b32f7c2192ce2a4c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.aux

- **claim** (`cl\_f0df6d0c6e470f7e`): \`aux\` is the shared Black-76 precomputation kernel: given CommodityInputs it computes σ√t, the carry-adjusted forward F = S·e^{b·t} (via CommodityInputs::forward), the discount factor df = e^{−r·t} (via CommodityInputs::discount\_df), and the canonical log-moneyness d1 = \[ln(F/K) + ½σ²t\] / (σ√t) and d2 = d1 − σ√t using libm-routed ln/sqrt for cross-platform determinism. All five pricing/Greeks functions (price, greeks, futures\_style\_price, futures\_style\_greeks, forward\_delta) read exclusively from this Aux struct so the critical-path arithmetic is computed once. Pure: reads &CommodityInputs, returns Aux, no writes/allocation/IO.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.aux` (hash `e46c29c83d24d03d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.forward\_delta

- **claim** (`cl\_ab448dcb40f53201`): \`forward\_delta\` returns the driftless (forward) delta ∂V/∂F = df·Φ(d1) for a Call and df·(Φ(d1)−1) for a Put — the Black-76 forward-space sensitivity used as the standardised delta quote for commodity options (where hedging is via the futures contract, not the spot). It is distinct from the spot delta in \`greeks\` which carries the additional e^{bt} factor. The test \`forward\_delta\_helper\_matches\_strip\` asserts it is bitwise identical to the delta\_forward field extracted from \`greeks\` for both EquityStyle variants. Pure: reads (OptionType, &CommodityInputs), returns f64, no writes/allocation/IO.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.forward\_delta` (hash `b3df56cd2a0c2c2f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.forward\_delta\_helper\_matches\_strip` (hash `a299fd4295f6edb7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures\_style\_greeks

- **claim** (`cl\_59e5803a544d365f`): futures\_style\_greeks computes the full 14-field CarryGreeks for a commodity option under futures-style (daily-margined) settlement using the undiscounted Black-76 formula: price = F·N(d1) − K·N(d2) for calls (K·N(−d2) − F·N(−d1) for puts), where F = S·e^{bt}. Every sensitivity follows by differentiating that undiscounted value: delta\_spot = e^{bt}·N(±d1), delta\_forward = N(±d1), gamma = e^{2bt}·φ(d1)/(F·σ√t), vega = F·√t·φ(d1). Crucially, discount\_rho is identically 0.0 (the financing leg present in equity-style pricing is eliminated by daily margining), while carry\_rho = ±t·F·N(±d1). The RateSensitivities variant is always Carry{discount\_rho: 0.0, carry\_rho}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures\_style\_greeks` (hash `3092ff1345d3f103`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures\_style\_is\_undiscounted\_and\_rate\_invariant

- **claim** (`cl\_f996d2758fe9411e`): \`futures\_style\_price\` is the undiscounted Black-76 closed form for futures-style (CME daily-margined) commodity options: Call = F·Φ(d1) − K·Φ(d2), Put = K·Φ(−d2) − F·Φ(−d1), with no discount factor. The test \`futures\_style\_is\_undiscounted\_and\_rate\_invariant\` pins two invariants: (1) futures\_style\_price × e^{−r·t} == price bitwise (the discounted form is exactly df×undiscounted), and (2) futures\_style\_price is bitwise invariant to changes in r at fixed b — the discount rate is entirely absent from the formula, so the futures-style price has zero discount-rho. Pure: reads (OptionType, &CommodityInputs), returns f64, no writes/allocation/IO.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures\_style\_is\_undiscounted\_and\_rate\_invariant` (hash `896e897edc1d361f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures\_style\_price` (hash `7f37b1be00fcc4db`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.greeks

- **claim** (`cl\_b66592e8843360d2`): \`greeks\` computes the full 13-field CarryGreeks for equity-style (discounted) commodity options under the cost-of-carry parameterization. Key formulas: price = df·\[F·Φ(±d1) − K·Φ(±d2)\]; delta\_spot = e^{(b−r)t}·Φ(±d1) (chain rule ∂V/∂S = e^{bt}·∂V/∂F with ∂V/∂F = df·Φ(±d1)); gamma = e^{2bt}·df·φ(d1)/(F·σ√t); discount\_rho = −t·price (∂V/∂r at fixed b, since V = e^{−rt}·\[…\] independent of r in forward space); carry\_rho = ±t·F·df·Φ(±d1) (only F = S·e^{bt} depends on b); theta includes the pdf term df·F·φ(d1)·σ/(2√t) plus carry/financing legs. All sensitivities are cross-validated against central finite differences in \`greeks\_vs\_finite\_difference\`. Pure: no writes, no allocation, no IO.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.greeks` (hash `ae6e1887b83f7744`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price

- **claim** (`cl\_8029c1cf5e969d09`): CAPABILITY (commodity cross-asset leaf): celnet-commodity-vanilla::price is the Black-76 commodity/future-option pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &CommodityInputs) that discounts the forward directly (no spot carry), reconciled to Haug's published Black-76 reference and an independent QuantLib-pinned oracle. It is the commodity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price` (hash `6d52632f9740df7d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price\_with\_margining

- **claim** (`cl\_fe4d57e95a404783`): price\_with\_margining is a pure dispatch: for Margining::EquityStyle it delegates to price(opt, i) (discounted Black-Scholes-Merton with cost-of-carry b), and for Margining::FuturesStyle it delegates to futures\_style\_price(opt, i) (undiscounted Black-76). No arithmetic is performed; the function is a zero-cost match arm selector over the Margining enum.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price\_with\_margining` (hash `60e13f31ccd7563c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.strip\_fields

- **claim** (`cl\_f14b987dd66f772b`): strip\_fields flattens a CarryGreeks struct into a fixed-length array of 14 named (label, f64) pairs in a canonical order: \[price, delta\_spot, delta\_forward, gamma, vega, theta, discount\_rho, carry\_rho, vanna, volga, charm, speed, zomma, color\]. It panics if rates is the Fx variant, enforcing that only Carry-tagged greeks (i.e. commodity and equity paths) flow through this helper. The fixed array size makes field-by-field bitwise comparison across margining modes possible without heap allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.strip\_fields` (hash `145249d6f55c54ad`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.lib.expect\_forward

- **claim** (`cl\_133692c0a38411fa`): expect\_forward classifies a Tenor as requiring a forward rate (rather than spot) when its equivalent month count exceeds 12: Overnight/TomNext/SpotNext → 0; Weeks(w) → w/5; Months(m) → m; Years(y) → y\*12; Imm(n) → n\*3; BrokenDate → 0. Returns true iff months \> 12. This is the single authoritative FX tenor classification gate that determines spot vs forward axis routing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.lib.expect\_forward` (hash `62530629964b139a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.lib.schedule

- **claim** (`cl\_57e9da92cf466742`): \`vol\_year\_fraction(pair, horizon, tenor) -\> Result\<f64, TenorError\>\` computes the ACT/365-fixed year fraction for volatility time: it calls \`resolve(pair, tenor)\` to get the convention record (which always carries \`day\_count\_vol = DayCount::Act365Fixed\` per \`record\_at\`), builds an \`FxSchedule\` via \`schedule\`, and returns \`year\_fraction(Act365Fixed, sch.vol\_anchor, sch.expiry)\`. The result is the standard FX vol-time measure used as input to all volatility surface and pricing functions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.lib.schedule` (hash `cb864b36683343b2`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.lib.vol\_year\_fraction` (hash `78c78da60f56cdca`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_consistent

- **claim** (`cl\_d88c68456380f339`): Convention cross-field consistency invariant (docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention coupling): ConventionRecord::is\_consistent is a pure total predicate asserting self.premium\_style.is\_premium\_adjusted() == self.is\_delta\_premium\_adjusted() — i.e. a record is consistent exactly when its premium style and its delta convention agree on premium-adjustment. const fn, no writes/allocation/IO; deterministic. Self-invalidates if either underlying mapping or this coupling changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_consistent` (hash `184f50c1b2ee2d45`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_delta\_forward

- **claim** (`cl\_706143a176eeafaf`): Convention forward-delta axis (docs/CONVENTIONS.md DeltaConvention; short tenors quote spot delta, long tenors switch to forward/driftless delta). \`ConventionRecord::is\_delta\_forward\` is a pure total function of self.delta — returns true exactly for the two forward variants DeltaConvention::ForwardUnadjusted and DeltaConvention::ForwardPremiumAdjusted, false for the two spot variants. It is the orthogonal counterpart to is\_delta\_premium\_adjusted: together the (is\_delta\_forward, is\_delta\_premium\_adjusted) pair decomposes DeltaConvention into its two independent boolean axes (the axes premium\_adjusted\_of recomposes). const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_delta\_forward` (hash `5b0b3b6b11cd177d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_delta\_premium\_adjusted

- **claim** (`cl\_5c39f185816e3403`): Convention premium-adjusted mapping (docs/CONVENTIONS.md DeltaConvention→premium-style): ConventionRecord::is\_delta\_premium\_adjusted is a pure total function of self.delta — returns true exactly for the two premium-adjusted variants DeltaConvention::SpotPremiumAdjusted and DeltaConvention::ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_delta\_premium\_adjusted` (hash `d1325d856aaad9dc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_non\_deliverable

- **claim** (`cl\_e62150cb9d2ae08b`): Settlement→deliverability mapping (docs/CONVENTIONS.md Settlement enum → celnet-types): ConventionRecord::is\_non\_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, false otherwise. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the Settlement variant set or this match arm changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is\_non\_deliverable` (hash `4e2342fb0523c95c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_non\_deliverable

- **claim** (`cl\_9aa246ca1097d849`): Pair-universe settlement classification (docs/CONVENTIONS.md Settlement → registry PairMeta): PairMeta::is\_non\_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, mirroring ConventionRecord::is\_non\_deliverable so the registry-level and resolved-record classifications agree. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or Settlement variants change (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_non\_deliverable` (hash `6664f16ca66812f6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_precious\_metal

- **claim** (`cl\_244f1fc3d033857b`): Pair-universe asset-class classification (docs/CONVENTIONS.md InstrumentClass → registry PairMeta): PairMeta::is\_precious\_metal is a pure total function of self.instrument — returns true exactly for InstrumentClass::PreciousMetal (the loco-London metal-leg pairs), false for fiat. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or InstrumentClass variants change (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_precious\_metal` (hash `b5ebd496f4edf371`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_self\_consistent

- **claim** (`cl\_b651fe794638697b`): Pair-universe structural self-consistency (docs/CONVENTIONS.md PairMeta entry invariants): PairMeta::is\_self\_consistent is a pure total predicate over its own fields returning false on any contradiction — NDF terms present iff non-deliverable; the cash-settlement currency (when present) is one of the pair's two legs; the premium currency is one of the pair's legs and agrees with the premium-adjusted flag; and the spot lag is the canonical T+1 or T+2. No writes/allocation/IO; deterministic, reads only self. Self-invalidates if the invariant set or any field-coupling changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is\_self\_consistent` (hash `ab8f472f9659c96f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.inverted

- **claim** (`cl\_4ef3390583c9d4a5`): \`PairProfile.inverted()\` flips a profile for an inverted quote orientation: it swaps \`day\_count\_accrual\_for\` and \`day\_count\_accrual\_dom\`, calls \`premium\_style.flip\_orientation()\`, and crucially leaves \`ndf\` and \`metal\_leg\` unchanged — enforcing that the NDF fixing reference + physical settlement currency and the precious-metal lease leg are intrinsic pair properties invariant under quote direction (e.g. USDKRW and KRWUSD share the same KFTC18 fixing and USD settlement).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.inverted` (hash `6e82cb875d3d4a81`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.record\_at

- **claim** (`cl\_0287b152f4ce7448`): \`PairProfile.record\_at(tenor)\` produces a \`ConventionRecord\` with the vol day-count unconditionally set to \`DayCount::Act365Fixed\` (§1.5 of the convention spec), regardless of pair or tenor. The delta convention is derived as \`premium\_adjusted\_of(is\_long\_tenor(tenor), premium\_adjusted)\`, so short tenors and long tenors receive different delta conventions while all other fields come directly from the profile.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.record\_at` (hash `93ad8ee3cf0709a3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is\_long\_tenor` (hash `2cad661506df38d5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.accrual\_basis

- **claim** (`cl\_400f7cd41ed4e1ad`): Accrual day-count is a CURRENCY property, not a pair property (docs/CONVENTIONS.md DayCount → celnet-types::DayCount). \`accrual\_basis(ccy)\` is a pure total function mapping each Ccy to its money-market money accrual basis: GBP, AUD and NZD accrue ACT/365-fixed (DayCount::Act365Fixed); every other currency accrues ACT/360 (DayCount::Act360). Because it keys on the single currency leg (not the pair), AUD as the foreign leg accrues ACT/365 whether the pair is a covered major (AUDUSD), a covered G10 cross (AUDJPY), or a region-default-derived uncovered cross (AUDPLN) — the single source of truth the registry's accrual\_basis\_is\_single\_source\_of\_truth test pins. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the currency→basis mapping changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.accrual\_basis` (hash `c89d69e0175ec94d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize

- **claim** (`cl\_4033e3948a40d970`): \`canonicalize(pair) -\> Option\<Canonical\>\` performs orientation-agnostic lookup: it first tries the direct key; if not in \`COVERED\_PAIRS\` it tries the flipped pair (base and quote swapped) and sets \`Canonical.flipped = true\`. Returns \`None\` if neither orientation is covered. This means \`pair\_profile\` and all downstream callers are indifferent to quote orientation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize` (hash `70cdd0989f3c922d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize

- **claim** (`cl\_6baeaa6e292d9301`): \`resolve(pair, tenor)\` is the primary two-path resolution function: it calls \`pair\_profile(pair)\` which attempts a direct-key then flipped-key lookup in the static \`COVERED\_PAIRS\` table via \`canonicalize\`; on a hit it calls \`profile.record\_at(tenor)\` with \`ResolutionSource::PairProfile\`; on a miss it falls back to \`region\_default(pair, tenor)\` with \`ResolutionSource::RegionDefault\`. The function is pure: no I/O, no global mutation, deterministic over its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize` (hash `70cdd0989f3c922d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair\_profile` (hash `3971e3c09380cf64`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.resolve` (hash `c2b4d1566ed99703`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is\_long\_tenor

- **claim** (`cl\_98ba6e4d39ebef16`): The spot-vs-forward delta switch is a one-year tenor threshold (docs/CONVENTIONS.md DeltaConvention: short tenors quote spot delta, long tenors switch to forward/driftless delta). \`is\_long\_tenor(tenor)\` is a pure total predicate returning tenor\_days(tenor) \> 365 — STRICTLY greater, so exactly-one-year tenors (Tenor::Years(1) and Tenor::Months(12), both 365 days) classify as SHORT (spot delta) and 18M / 2Y classify as LONG (forward delta), exactly as long\_tenor\_threshold\_is\_one\_year pins. This boolean is the \`forward\` axis fed to premium\_adjusted\_of in region\_default. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the threshold or tenor\_days mapping changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is\_long\_tenor` (hash `2cad661506df38d5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair\_meta

- **claim** (`cl\_909827ec7426ce12`): \`pair\_meta(pair) -\> Option\<PairMeta\>\` aggregates the full convention bundle for a covered pair: it calls \`pair\_profile\` then \`premium\_ccy\_of\` to derive the premium currency (base ccy when premium-adjusted, quote ccy otherwise), and collects \`spot\_lag\_days\`, \`atm\`, \`premium\_style\`, \`premium\_ccy\`, \`premium\_adjusted\`, \`cut\`, \`settlement\`, \`ndf\`, \`instrument\`, and \`metal\_leg\` into a single \`PairMeta\` struct. Returns \`None\` for uncovered pairs. With 11 callers it is the primary rich-metadata entry point.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair\_meta` (hash `2151a19e8da9c2bb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.premium\_adjusted\_of

- **claim** (`cl\_bda6b8f8a3a5b3d5`): DeltaConvention is the cartesian product of two independent booleans (docs/CONVENTIONS.md DeltaConvention; docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention). \`premium\_adjusted\_of(forward, premium\_adjusted)\` is the const-fn total constructor that composes the (forward?, premium-adjusted?) flags back into the four-variant enum: (false,false)→SpotUnadjusted, (false,true)→SpotPremiumAdjusted, (true,false)→ForwardUnadjusted, (true,true)→ForwardPremiumAdjusted. The match is exhaustive over both booleans, so no combination is defaulted — it is the exact inverse of the record predicates is\_delta\_forward (the \`forward\` axis) and is\_delta\_premium\_adjusted (the \`premium\_adjusted\` axis). Pure: no writes/allocation/IO; deterministic. Self-invalidates if the DeltaConvention variant set or the flag→variant mapping changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.premium\_adjusted\_of` (hash `63e6fdee6fc5f846`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.profile\_for\_canonical

- **claim** (`cl\_d8a28cc570b2c909`): \`profile\_for\_canonical(k)\` builds a \`PairProfile\` from the 6-byte canonical key by (1) looking up the \`PairSpec\` in \`COVERED\_PAIRS\`; (2) mapping \`SpecKind::NonDeliverable(fixing)\` to \`NdfTerms { fixing, settlement\_ccy: Ccy::USD }\` (NDF pairs always settle in USD); (3) for precious metals, overriding \`day\_count\_accrual\_for\` to \`DayCount::Act360\` (loco-London bullion basis) and attaching a \`MetalLeg { metal, lease\_day\_count: Act360, loco\_london: true }\`; (4) hard-coding \`AtmConvention::DeltaNeutralStraddle\` for all covered pairs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.profile\_for\_canonical` (hash `4ae7db47a5f612f8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region\_default

- **claim** (`cl\_9eba92a368fc0b2d`): The fall-through (uncovered-pair) convention record is assembled deterministically from per-currency and per-region rules (docs/CONVENTIONS.md house-default conventions; ResolutionSource::RegionDefault). \`region\_default(pair, tenor)\` is a pure total function building a ConventionRecord with: cut = Tokyo1500 for a Tokyo-region pair else NewYork1000 (via region\_of); premium\_style = PercentForeign; delta = premium\_adjusted\_of(is\_long\_tenor(tenor), premium\_style.is\_premium\_adjusted()) — so the spot/forward axis follows the tenor and the premium-adjusted axis follows the premium style; atm = DeltaNeutralStraddle; day\_count\_vol = Act365Fixed; the foreign and domestic accrual day-counts = accrual\_basis(pair.base) and accrual\_basis(pair.quote) respectively (per-currency, not per-pair); settlement = Deliverable. No combination is defaulted ad hoc — every field is a documented function of (pair, tenor). const-style assembly, no writes/allocation/IO; deterministic. Self-invalidates if any of the composed mapping helpers or the default field set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region\_default` (hash `075e3c245dd95cb3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region\_of

- **claim** (`cl\_4358f59fc3711c2a`): The expiry-cut region is decided by the QUOTE currency (docs/CONVENTIONS.md Cut → New York 10:00 vs Tokyo 15:00). \`region\_of(pair)\` is a pure total function returning Region::Tokyo exactly when pair.quote == Ccy::JPY, else Region::NewYork — the JPY-region/Asian business books the Tokyo 15:00 cut, every other pair the New York 10:00 cut. It keys on the quote leg only (the JPY pairs are quoted XXXJPY), so the region/cut is a deterministic function of the pair, never of spot or tenor. const-foldable, no writes/allocation/IO. Self-invalidates if the region-selection rule or Region/Ccy variant set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region\_of` (hash `a91c9ba8b4a18c92`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-conventions.src.registry.tenor\_days

- **claim** (`cl\_946bb30c7bf8d8e2`): tenor\_days is the exhaustive nominal-horizon primitive that drives the short/long delta-convention classification (docs/CONVENTIONS.md tenor axis). It is a pure total function over the whole Tenor enum returning u32 days: the pre-spot short end Overnight \| TomNext \| SpotNext → 1 (and BrokenDate → 1, the conservative short default since the exact pricing axis is set later by the pricer from the resolved expiry, not here); Weeks(w) → 7·w; Months(m) → (365·m + 6)/12 (the 365/12 ≈ 30.4167 days-per-month rounded to nearest day, so Months(12) = 365 and Months(6) = 183); Years(y) → 365·y; Imm(n) → (3·n·365 + 6)/12 (≈ 3 months per IMM step). The match is exhaustive over Tenor, so no variant is defaulted, and it is consumed by is\_long\_tenor (\>365 ⇒ forward delta). const-foldable integer arithmetic, no writes/allocation/IO; deterministic. Self-invalidates if the Tenor variant set or any per-variant day formula changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.tenor\_days` (hash `956e137174ad658d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryInputs.forward

- **claim** (`cl\_2bfbd934139b2493`): \`CarryInputs::forward(&self) -\> f64\` computes the outright forward price as \`spot \* carry.forward\_factor(t)\`, where \`Carry::forward\_factor(t)\` returns \`e^{b\*t}\` via \`libm::exp(carry\_rate() \* t)\`. For FX this is \`spot \* e^{(r\_dom - r\_for)\*t}\` — the two-rate FX forward. For cost-of-carry underlyings it is \`spot \* e^{b\*t}\`. Pure: reads &self, no side effects, no WRITES. This is byte-identical to \`VanillaInputs::forward\` on the FX path (proved by \`fx\_carry\_inputs\_byte\_identical\`), making \`CarryInputs\` a drop-in generalization of \`VanillaInputs\` for cross-asset consumers.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryInputs.forward` (hash `600b21cd729cb70e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryPriceError.fmt

- **claim** (`cl\_e1527e3ca58498dd`): CarryPriceError::fmt distinguishes exactly two rejection reasons at the carry-pricing boundary: 'pricer does not support this underlying asset class' (UnsupportedUnderlying) and 'pricer does not support this cost-of-carry model' (UnsupportedCarry). Both messages are static strings — zero allocation on format.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryPriceError.fmt` (hash `fbf2b8a6180f2c27`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_greeks

- **claim** (`cl\_cb1a94b0a746b8f6`): \`fx\_carry\_greeks(g: &Greeks) -\> CarryGreeks\` is the pure field-copy lifting function from the FX leaf's \`Greeks\` struct to the generalized \`CarryGreeks\` on the carry seam. It copies all 14 fields verbatim: price, delta\_spot, delta\_forward, gamma, vega, theta, vanna, volga, charm, speed, zomma, color, and packages the FX rate sensitivities as \`RateSensitivities::Fx { rho\_dom, rho\_for }\`. No arithmetic, no branch, no allocation — a structural repackaging. The byte-identity of this lift is verified by \`fx\_carry\_greeks\_lifts\_byte\_identically\` (assert\_eq! on to\_bits() for every field). Pure: reads only &Greeks, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_greeks` (hash `f37d2dc482a6085f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_greeks\_lifts\_byte\_identically

- **claim** (`cl\_4261707d9d199c48`): fx\_carry\_greeks\_lifts\_byte\_identically asserts that fx\_carry\_greeks() is a zero-loss lift: every scalar field (price, delta\_spot, delta\_forward, gamma, vega, theta, vanna, volga, charm, speed, zomma, color) and both rate fields (rho\_dom, rho\_for) are preserved bit-for-bit (via to\_bits() equality). The rates field must tag as RateSensitivities::Fx{rho\_dom, rho\_for}; tagging as Carry panics. This proves no precision is lost in the carry → FX greek conversion.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_greeks\_lifts\_byte\_identically` (hash `8a098e925efc3464`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_inputs\_byte\_identical

- **claim** (`cl\_8ef50af6ab97368e`): The carry-seam FX byte-identity is enforced by fx\_carry\_inputs\_byte\_identical: the FX arm of Carry (FxRates) lowers to VanillaInputs with forward/df\_dom/df\_for bit-identical (to\_bits) to the native FX leaf, and the CostOfCarry arm is rejected (UnsupportedCarry). This is a pure byte-identity gate over the carry seam (ADR-0008). Supersedes a withdrawn spec:satisfies probe whose design-target sentinel did not resolve in this build.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx\_carry\_inputs\_byte\_identical` (hash `0776104362fd01e2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-core.src.compare.is\_close

- **claim** (`cl\_6ae9c2aae1cde9e2`): Determinism rule — \`is\_close\` is the canonical float comparator and never uses \`==\` semantics that would misbehave on the special cases: it returns false whenever either operand is NaN (NaN is never close to anything, including itself); it short-circuits true on bitwise \`a == b\`, which deliberately also makes +0.0 and -0.0 close even at zero tolerance and makes equal infinities close; it returns false for unequal infinities; otherwise it accepts when the absolute difference is within \`abs\` OR within \`rel \* max(\|a\|,\|b\|)\` (a combined absolute-or-relative band). Tolerances are debug-asserted finite and non-negative. This is the single comparator behind \`assert\_close!\`; the interface determinism rule (docs/INTERFACES.md) is that all float comparison flows through it — never a bare \`==\`, never an assert on a NaN payload. Pure: reads only its four f64 args, returns a bool.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.compare.is\_close` (hash `7eae72f39302350d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf

- **claim** (`cl\_aa8300ca191b5f15`): Determinism rule — \`norm\_cdf\` computes the standard-normal CDF as \`0.5 \* libm::erfc(-x \* INV\_SQRT\_2)\`, routing the transcendental through \`rust-lang/libm\` (correctly-rounded) rather than the platform libm, so the result is bit-identical across targets (the cross-platform determinism guarantee of docs/INTERFACES.md). Using the complementary error function \`erfc\` on \`-x·1/√2\` keeps the deep left tail stable (no catastrophic cancellation), which is why the tail tests pass. f64 is the CPU-canonical scalar. Pure: maps one f64 to one f64 via libm, no side effects, no state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf` (hash `a88bce70bc6e0800`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf\_deep\_tail\_matches\_reference

- **claim** (`cl\_6504e2fac763ca24`): Determinism rule — \`norm\_cdf\_deep\_tail\_matches\_reference\` pins \`norm\_cdf\` against high-precision reference values deep in the left tail: Φ(−1)=0.15865525393145705, Φ(−5)=2.866515718791939e-7, Φ(−10)=7.619853024160525e-24, each via \`assert\_close!\` with explicit rel/abs tolerances (never \`==\`). Because \`norm\_cdf\` routes through \`libm::erfc\` (correctly-rounded), these exact-digit references encode the bit-stable, cross-platform tail behaviour; a regression that dropped the erfc routing (reintroducing catastrophic cancellation) would fail here. Pure test: evaluates norm\_cdf and asserts, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf\_deep\_tail\_matches\_reference` (hash `27f3073a8221240d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf\_tail\_symmetry\_and\_no\_underflow

- **claim** (`cl\_cbb9f0087855719b`): Determinism rule — \`norm\_cdf\_tail\_symmetry\_and\_no\_underflow\` guards the tail-stability that the \`libm::erfc\` routing in \`norm\_cdf\` buys: it checks the reflection identity Φ(−x)=1−Φ(x) at x∈{3,4,5} (the largest x where the RHS is still representable before it underflows to 0), pins Φ(−15)=3.670966199312858e-51 and Φ(−20)=2.753624118606331e-89 against high-precision references, and asserts Φ(−37)\>0 (≈5.7e-300, never flushed to zero). All comparisons go through \`assert\_close!\`, never \`==\`. A regression that reintroduced the cancellation-prone 1−Φ(x) form on the direct path would be caught. Pure test: evaluates norm\_cdf and asserts, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_cdf\_tail\_symmetry\_and\_no\_underflow` (hash `465a95da16e43597`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_pdf

- **claim** (`cl\_548a02f48c75ae0a`): Determinism rule — \`norm\_pdf\` computes the standard-normal density as \`INV\_SQRT\_2PI \* exp(-0.5 \* x \* x)\` where \`exp\` is the crate's \`libm\`-backed wrapper, so the transcendental is correctly-rounded and bit-identical across platforms (docs/INTERFACES.md cross-platform determinism). The argument is symmetric in x (x·x), so norm\_pdf is exactly even. f64 is the CPU-canonical type. Pure: maps one f64 to one f64, no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-core.src.math.norm\_pdf` (hash `2eb0294bb04e515e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.funding.funding\_carry

- **claim** (`cl\_fbd5ea1a8905e500`): \`funding\_carry(r, funding) -\> Carry\` is the canonical Carry constructor for crypto: it builds \`Carry::CostOfCarry { r, b: r - funding }\` so the net cost-of-carry b equals the difference between the risk-free rate and the perpetual funding rate. When funding == r the carry rate is zero and the forward equals spot (zero-drift, Black-76 limit). The function is called by 14 callers — it is the standard entry point for both inverse and linear crypto inputs. Pure: returns a new \`Carry\` from two \`f64\` scalars with no WRITES, no allocation, no I/O.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.funding.funding\_carry` (hash `afd3b2bbaf429b93`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.aux

- **claim** (`cl\_10d0cd5befa6ba0b`): \`inverse::aux(i)\` precomputes the five shared scalars for the coin-margined closed form: \`sqt=√t\`, \`vsqt=σ√t\`, \`f=S·e^{b·t}\` (forward), \`df=e^{-r·t}\` (discount factor), \`s2t=σ²·t\`, and the three log-moneyness distances \`d1=(ln(F/K)+½σ²t)/(σ√t)\`, \`d2=d1-σ√t\`, \`d3=d1-2σ√t\`, plus \`es2t=e^{σ²t}\`. The inverse formula uses d2 and d3 (not d1) as the CDF arguments — d3 = d1 - 2σ√t is specific to the coin-margined payoff, absent from both the FX/linear and the standard Black-Scholes aux. Pure: reads \`&InverseInputs\`, returns \`Aux\`, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.aux` (hash `3078450c94e8b6d7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.check\_greeks

- **claim** (`cl\_6d7ebbd18390f364`): celnet-crypto-vanilla inverse check\_greeks validates Greeks for inverse (coin-settled) options via the \`.coin\` field of the returned struct; all FD checks use 1e-9 absolute tolerance (tighter than linear/equity). Rate sensitivities must tag as RateSensitivities::Carry or panics with 'crypto inverse greeks must tag as Carry'. The forward delta is validated as spot-FD / carry.forward\_factor(t) on the coin Greek, not the USD Greek.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.check\_greeks` (hash `7bcf2feaf4fc6756`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.closed\_form\_matches\_deterministic\_quadrature

- **claim** (`cl\_dbdbff77e04d5f97`): The inverse-vanilla closed-form price is verified against a code-disjoint midpoint-quadrature integration of the literal payoff E^Q\[df·max(φ·(S\_T−K),0)/S\_T\] over the risk-neutral lognormal distribution. The quadrature uses 400 000 uniform panels spanning z ∈ \[−12, 12\] of the standard normal, evaluating the integrand (phi·(S\_T−K)).max(0)/S\_T at each midpoint z = lo+(j+0.5)·dz, where S\_T = F·exp(−½σ²t + σ√t·z). Three (S,K,σ,t,r,funding) vectors spanning OTM/ITM/ATM at different crypto vol regimes (0.55–0.80) are tested for both Call and Put. Agreement is required to 1e-6 relative with a 1e-7 absolute floor. The method shares no analytic structure with the production CDF-based \`price\` function, providing a structurally independent oracle.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.closed\_form\_matches\_deterministic\_quadrature` (hash `02da446bf061659b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.closed\_form\_within\_monte\_carlo\_standard\_error

- **claim** (`cl\_05501a526dd3bb8b`): The inverse-vanilla closed-form price agrees with a fully deterministic Monte Carlo oracle (4 000 000 paths, Box-Muller normals from a seeded SplitMix64 PRNG defined inline — no dependency on any production sampler) to within 4 standard errors of the MC estimate plus 1e-12. Each path computes the coin-denominated payoff df·max(φ·(S\_T−K),0)/S\_T and the MC standard error is derived from the empirical variance. The same three (S,K,σ,t,r,funding) market scenarios tested in the quadrature verification are covered for both Call and Put, gating the closed form from below by an independent stochastic oracle at 4σ.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.closed\_form\_within\_monte\_carlo\_standard\_error` (hash `301f04823a80c518`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.convexity\_sandwich\_vs\_linear\_is\_signed

- **claim** (`cl\_b63a697d71de8da5`): The convexity sandwich test pins a signed directional inequality between the coin-margined and linear prices: for all tested parameters, \`inverse::price(Call)\*S \< linear::price(Call)\` strictly, and \`inverse::price(Put)\*S \> linear::price(Put)\` strictly (tolerance 1e-6). This is the anti-circular guard — a naive V\_lin/S₀ rescale of the linear price would produce equality at both legs, making both differences zero and failing both assertions. The test thus verifies that the inverse formula is not a trivial rescale of the linear one. Pure validator: reads from value arguments, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.convexity\_sandwich\_vs\_linear\_is\_signed` (hash `b35d3cd040adc258`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.greeks

- **claim** (`cl\_524da74cbea58f08`): \`inverse::greeks(opt, i)\` is the complete analytic Greek strip for the coin-margined vanilla. It introduces the convexity amplitude \`amp = A = (K/F)·e^{σ²t}\` and \`ψ₃ = A·Φ(φ·d3)\`, exploiting the exact lognormal pdf identity \`A·φ(d3) = φ(d2)\` (because d2-d3=σ√t) so all pdf cross-terms cancel into clean closed forms. Key sensitivities: \`price = φ·df·(Φ(φd2) - ψ₃)\`, \`delta\_spot = φ·df·ψ₃/S\`, \`delta\_forward = φ·df·ψ₃/F\`, \`discount\_rho = -t·price\`, \`carry\_rho = φ·df·t·ψ₃\`, \`vega = df·φ(d2)·√t - φ·df·2σt·ψ₃\`, \`gamma = df·φ(d2)·u/S - φ·df·2·ψ₃/S²\` (u=1/(S·σ√t)). Returns \`InverseGreeks { coin: CarryGreeks, usd\_equivalent: price\*S }\`. Pure: reads \`(OptionType, &InverseInputs)\`, no WRITES, no allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.greeks` (hash `13dbc415b2f6fdb9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.price

- **claim** (`cl\_85a5b8710c396738`): CAPABILITY (crypto inverse/coin-margined leaf): celnet-crypto-vanilla::inverse::price is the inverse (coin-margined, 1/S\_T payoff) crypto vanilla pricing entry — a pure, side-effect-free closed form taking (OptionType, &InverseInputs) whose value is expressed in the coin numeraire via the k/F and e^{sigma^2 t} convexity terms (norm\_cdf of d2/d3), reconciled to an independent oracle with a signed convexity sandwich. It is the crypto inverse capability's projection target on the carry seam through the one contract; the sibling linear (USDT-margined) crypto path collapses to the Black-76 forward limit at zero carry. No I/O, allocation, logging, or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.price` (hash `8d2e0c32fcae388d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.aux

- **claim** (`cl\_141dec0fa916643e`): \`linear::price(opt, i)\` is the USDT/coin-quoted (linear-margined) crypto vanilla closed form: it computes \`s\_disc = spot·e^{-r\_for·t}\` and \`k\_disc = strike·e^{-r\_dom·t}\` where \`(r\_dom, r\_for) = (r, r-b)\` extracted from the Carry via \`fx\_equiv\_rates\`, then prices as Call = s\_disc·Φ(d1) - k\_disc·Φ(d2) and Put = k\_disc·Φ(-d2) - s\_disc·Φ(-d1). The \`d1/d2\` use the spot-space log-moneyness formula \`d1 = (ln(S/K) + (r\_dom - r\_for + ½σ²)·t) / (σ√t)\` (comment: same operation order as the FX GK leaf, so price is bit-identical to the FX leaf under matching rates). Pure: reads \`(OptionType, &LinearInputs)\`, returns \`f64\`, no WRITES, allocation, or I/O.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.aux` (hash `be9d7c8f3a572b57`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.price` (hash `36fff6de5a952236`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.check\_greeks

- **claim** (`cl\_026ee0070023bcc5`): celnet-crypto-vanilla linear check\_greeks validates all greeks for crypto linear options (coin-margined notional in fiat) against FD with Carry tagging required: delta\_forward = spot-FD / carry.forward\_factor(t); carry\_rho = ∂V/∂b (positive carry direction, not negated); rate sensitivities must tag as RateSensitivities::Carry or the test panics with 'crypto linear greeks must tag as Carry'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.check\_greeks` (hash `ba81d47456452870`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.fx\_equiv\_rates

- **claim** (`cl\_ed390c7a6d0c04ef`): \`linear::fx\_equiv\_rates(i) -\> (r\_dom, r\_for)\` recovers the GK-equivalent domestic/foreign rate pair from the unified Carry: \`r\_dom = carry.discount\_rate()\` and \`r\_for = r\_dom - carry.carry\_rate()\` (= r - b = funding rate). This is the seam that maps the crypto funding-rate carry convention onto the same two-rate spot-discounting formula as the FX GK leaf, making \`linear::price\` bit-identical to \`celnet-vanilla::price\` under matching rates (as the comment in \`aux\` documents). Pure: reads \`&LinearInputs\`, returns \`(f64, f64)\`, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.fx\_equiv\_rates` (hash `e6da5db2267b2f76`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.greeks

- **claim** (`cl\_0895fadbd4b19b4d`): \`linear::greeks(opt, i)\` is the complete analytic Greek strip for the USDT-margined (GK-equivalent) crypto vanilla. It computes price in the same spot-space order as \`linear::price\` so \`greeks().price\` is \`to\_bits\`-identical to the standalone price call. Spot delta = e^{bt}·df·Φ(±d1); forward delta = df·Φ(±d1); gamma = e^{2bt}·df·φ(d1)/(F·σ√t); vega = df·F·√t·φ(d1); discount\_rho = -t·price; carry\_rho (∂V/∂b) = ±t·F·df·Φ(±d1). The theta algebra uses the identity F·φ(d1)=K·φ(d2) to collapse to a single positive pdf term \`df·F·φ(d1)·σ/(2√t)\`. Returns \`CarryGreeks\` (the same struct as FX/commodity). Pure: reads \`(OptionType, &LinearInputs)\`, no WRITES, no allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.greeks` (hash `a5c588bc6802790e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.settlement.route\_price

- **claim** (`cl\_a92f768b29ed11a4`): CAPABILITY (carry-seam deliverable, crypto leaf reach): route\_price is the pure crypto settlement-style dispatcher that reaches both crypto vanilla leaves on the shared Carry seam — SettlementStyle::Linear → linear::price (GK-funding, USDT/coin-quoted) and SettlementStyle::InverseCoin → inverse::price (inverse/coin-margined 1/S\_T payoff) — selecting the leaf by settlement style and forwarding the same (spot, strike, vol, t, Carry). Pure: returns the leaf price from value/ref args with no WRITES; self-invalidates if either leaf arm gains a side effect.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.settlement.route\_price` (hash `5fb097e58e2b8bc2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.HandoffError.fmt

- **claim** (`cl\_7a2dce2cdcb64085`): HandoffError::fmt maps all four binary-handoff deserialization failure modes to fixed, allocation-free static strings via a single f.write\_str call. The variant-to-message mapping is: Truncated → "handoff buffer truncated"; BadMagic → "handoff buffer has wrong content tag"; BadDiscriminant → "handoff buffer has invalid enum discriminant"; TrailingBytes → "handoff buffer has trailing bytes". No heap allocation occurs (static &str, single write\_str). The match is exhaustive.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.HandoffError.fmt` (hash `470c364e090dd0a5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_atm

- **claim** (`cl\_317230338faf545b`): enc\_atm and dec\_atm form a stable, exhaustive bijection over the two-variant AtmConvention enum: AtmForward→0, DeltaNeutralStraddle→1. This byte assignment is the permanent on-disk journal encoding for ATM convention. Any other byte yields HandoffError::BadDiscriminant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_atm` (hash `ffce76027cadb4a3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_atm` (hash `25362c7e138a5922`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_cut

- **claim** (`cl\_5009f087efd747bb`): Engine-handoff Cut codec (docs/CONVENTIONS.md Cut enum; engine state serialization): dec\_cut is the pure total inverse of enc\_cut over the Cut discriminant — decodes byte 0→Cut::NewYork1000, 1→Cut::Tokyo1500, and any other byte to Err(HandoffError::BadDiscriminant), never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the discriminant assignment or Cut variant set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_cut` (hash `f5a47e5817e0404c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_daycount

- **claim** (`cl\_d25b9494d6a2e279`): enc\_daycount and dec\_daycount form a stable, exhaustive bijection over the two-variant DayCount enum: Act365Fixed→0, Act360→1. This byte assignment is the permanent on-disk journal encoding. Any byte outside {0,1} is rejected with HandoffError::BadDiscriminant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_daycount` (hash `6266d2081be6d90a`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_daycount` (hash `6b55744f805ae649`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_delta

- **claim** (`cl\_58d4a9aa9174c53e`): enc\_delta and dec\_delta form a stable, exhaustive bijection over the four-variant DeltaConvention enum: SpotUnadjusted→0, ForwardUnadjusted→1, SpotPremiumAdjusted→2, ForwardPremiumAdjusted→3. This byte ordering is the permanent on-disk journal encoding. Any byte outside {0,1,2,3} is rejected with HandoffError::BadDiscriminant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_delta` (hash `97008c4c37e51c44`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_delta` (hash `b467135723fe9e54`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_option\_type

- **claim** (`cl\_b1703933b9fbdc1a`): enc\_option\_type and dec\_option\_type form a stable, exhaustive bijection over the two-variant OptionType enum: Call→0, Put→1. This byte assignment is the permanent on-disk encoding in the durable journal — changing it would corrupt existing journal files. dec\_option\_type rejects any byte outside {0,1} with HandoffError::BadDiscriminant, guaranteeing the codec is total and safe.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_option\_type` (hash `fd86c64adddf9665`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_option\_type` (hash `d1e77a2c69d90d93`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_premium

- **claim** (`cl\_c12b1d619d8eddcb`): enc\_premium and dec\_premium form a stable, exhaustive bijection over the four-variant PremiumStyle enum: DomesticPips→0, PercentForeign→1, PercentDomestic→2, ForeignPips→3. This byte ordering is the permanent on-disk journal encoding. Bytes outside {0,1,2,3} are rejected with HandoffError::BadDiscriminant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_premium` (hash `f505b1a7fe67b072`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_premium` (hash `837e75557b88eeed`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_settlement

- **claim** (`cl\_c7534229a88d175b`): enc\_settlement and dec\_settlement form a stable, exhaustive bijection over the two-variant Settlement enum: Deliverable→0, NonDeliverable→1. This byte assignment is the permanent on-disk journal encoding. Any byte outside {0,1} is rejected with HandoffError::BadDiscriminant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec\_settlement` (hash `4b1c6f4948a6ff22`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc\_settlement` (hash `331d36c8e65b1fa7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-engine.src.handoff.serialize\_state

- **claim** (`cl\_b2adcf055a19e657`): ADR-0007 (one unversioned contract — engine hot-upgrade handoff). \`serialize\_state\` is the single, deterministic encoder of the engine's live state (MarketState + BookState) into the handoff byte image: it is a pure function of its two borrowed inputs (it reads no global/external state and mutates none — the only allocation is the returned Vec\<u8\>), so the same (market, book) always yields byte-identical output. The image is a fixed self-describing layout — MAGIC header, market scalars, conventions, the three smile benchmark pillars + reference forward/time (exactly the state from which MarketHedgeSmile::new reconstructs an identical smile), then the length-prefixed book — and \`restore\_state\` is its exact inverse (round-trip proven by roundtrip\_restores\_identical\_state / restored\_state\_reprices\_identically). DECISION/RATIONALE: hot-upgrade carries state across a code swap through this ONE current handoff format with a MAGIC sentinel and NO schema\_version field and NO N/N-1 negotiation — consistent with the platform-wide single-unversioned-contract decision (ADR-0007). An upgrade deploys a single uniform engine version: the old build serializes, the new build restores; there is no mixed-version window to negotiate, so the format evolves in place rather than versioning. (Guardrail: no versioned APIs; hot-upgradable single-version estate.)
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.serialize\_state` (hash `b895f29603af787e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-engine.src.journal.EventTag.from\_byte

- **claim** (`cl\_3cbcddf42fd5ff66`): EventTag::from\_byte is the sole entry point for deserializing journal event type tags: byte 0 maps to EventTag::BookLine, byte 1 maps to EventTag::MarkState, and every other byte yields RecoveryError::UnknownEventTag(b). This two-tag space is exhaustive over the current journal format — any addition of a new event kind requires a new byte assignment here.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.journal.EventTag.from\_byte` (hash `b0ac16f36d1bab5e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-engine.src.journal.RecoveryError.fmt

- **claim** (`cl\_a3e845c7c0d90b30`): RecoveryError::fmt produces a fully-qualified diagnostic string for every variant of the journal replay error taxonomy. The five variants map to these exact human-readable messages: Journal(e) → "journal error: {e}"; Decode(e) → "durable event decode error: {e}"; UnknownEventTag(b) → "unknown durable event tag byte: {b}"; EmptyEvent → "durable event payload was empty"; NoMarkedState → "recovered journal has no accepted market state". The match is exhaustive — the compiler enforces that no variant is silently omitted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.journal.RecoveryError.fmt` (hash `e23fba22c7a43810`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-engine.src.journal.RecoveryError.source

- **claim** (`cl\_13286e7e74a407d5`): RecoveryError::source exposes a causal error chain only for the two variants that wrap foreign errors: Journal(e) and Decode(e) each return Some(e as &dyn Error). The remaining three leaf variants — UnknownEventTag, EmptyEvent, and NoMarkedState — return None because they carry no underlying cause. This ensures std::error::Error chain traversal terminates correctly at structural leaf errors.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.journal.RecoveryError.source` (hash `7204226a6ba0cd0d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-engine.src.journal.decode\_event

- **claim** (`cl\_668813803b056657`): decode\_event dispatches on the first byte of a journal payload (via EventTag::from\_byte) to reconstruct one of two DurableEvent variants: BookLine (deserialised via handoff::read\_book\_entry with a Reader finish guard that rejects trailing bytes) or MarkState (via handoff::restore\_state, discarding the empty embedded book section). An empty payload returns RecoveryError::EmptyEvent. No other branches exist — the tag byte fully determines the decode path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-engine.src.journal.decode\_event` (hash `baaaae24b60100b6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessDecision.label

- **claim** (`cl\_75f96440e8ef4d62`): AccessDecision::label is a pure const &'static str bijection: Allow =\> "allow", Deny =\> "deny". Exhaustive with no default arm; the two values form the complete access decision vocabulary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessDecision.label` (hash `142ca5a74edf6814`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessMode.is\_permissive

- **claim** (`cl\_149f41bf878704ad`): \`AccessMode\` defaults to \`Enforce\` (\`AccessMode::default() == AccessMode::Enforce\`), verified by the \`default\_mode\_is\_enforce\` test. In \`Enforce\` mode \`is\_permissive()\` returns \`false\`. This is the production posture — the system never silently opens access due to an absent or misconfigured principal.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessMode.is\_permissive` (hash `1746563e5be23f67`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.default\_mode\_is\_enforce` (hash `ae5250fedec5191b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessMode.label

- **claim** (`cl\_3ef8b3641f42f6a1`): AccessMode::label is a pure const &'static str bijection: Enforce =\> "enforce", Permissive =\> "permissive". This is the canonical wire/log label for the access mode; all telemetry and entitlement log lines use this label.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessMode.label` (hash `9b6223247438fefe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.decision

- **claim** (`cl\_ac1f4fb8f1d617b7`): \`AccessReason::decision(self)\` is the pure, total, const reason→decision mapping making every access outcome first-class and auditable: PrincipalAsserted / PermissiveAbsent / SessionAuthenticated → Allow, and PrincipalAbsent / MalformedPrincipal / SessionInsufficientRole / SessionMissingCapability → Deny. A genuinely-absent or malformed principal, an insufficient role, or a missing action-capability all resolve to Deny (deny-by-default at the boundary), and each AccessReason is the per-decision audit datum emitted via the celnet-observability AuditSink. Pinned by reason\_determines\_decision + the server entitlements\_boundary::decision\_records test. SELF-INVALIDATES on any change to this decision mapping.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.decision` (hash `7450251204e3401e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.label

- **claim** (`cl\_54d9b121b68e8496`): AccessReason::label maps each of seven access-decision reasons to a distinct stable ASCII string token used in structured logs and wire output: PrincipalAsserted =\> "principal\_asserted"; PermissiveAbsent =\> "permissive\_dev\_mode\_absent\_principal"; PrincipalAbsent =\> "principal\_absent"; MalformedPrincipal =\> "malformed\_principal"; SessionAuthenticated =\> "session\_authenticated"; SessionInsufficientRole =\> "session\_insufficient\_role"; SessionMissingCapability =\> "session\_missing\_capability". The PermissiveAbsent label embeds 'dev\_mode' to make permissive-mode grants visible in audit logs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.label` (hash `9e46f7d5e410c464`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.admits

- **claim** (`cl\_77982ebac8af0271`): SAFETY — entitlement cube-pruning admission is deny-by-default at the fact level (deliverable: entitlements-deny-by-default). EntitlementFilter::admits(fact) is a pure, deterministic predicate that delegates to the deny-first Principal::admits(hierarchy, &fact.key): it returns false on any matching deny rule (deny wins over grant) and otherwise requires an explicit grant whose every scope covers the fact — so an un-granted fact is never admitted. It reads only &self (borrowed principal + hierarchy) and the fact key; no mutation, I/O, or allocation. This is the exact per-fact gate that entitled\_cube / prune apply when projecting a risk cube to a principal, so an information-barrier breach cannot leak a fact the principal was not explicitly granted. Self-invalidates if admits stops delegating to the deny-first principal evaluation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.admits` (hash `4a91d2934d146bfc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.entitled\_cube

- **claim** (`cl\_fc86b1e88b0ed5bf`): \`EntitlementFilter::entitled\_cube\` builds an entitlement-gated \`Cube\` in a single pass: it constructs a \`Cube::with\_hierarchy(self.hierarchy.clone())\` then upserts each fact that passes \`self.admits(&fact)\`. Facts that do not pass the principal are silently dropped; no placeholder rows are inserted. The result is the minimum-information cube visible to the principal — guaranteed to contain no data from denied scopes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.entitled\_cube` (hash `707d5edb09bb1e82`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.prune

- **claim** (`cl\_1a0d0b0edda4c1f2`): \`EntitlementFilter::prune\` is a pure filter over a \`RiskFact\` slice: it returns \`facts.iter().filter(\|f\| self.admits(f)).cloned().collect()\`. The output length is always \<= input length and the relative order of surviving facts is preserved (stable filter, no sort). This is the canonical synchronous entitlement gate used in 6 call-sites.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter\<'a\>.prune` (hash `8a9a5a24fb12981a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.admits

- **claim** (`cl\_437d565dd47b06c4`): Entitlement visibility is deny-by-default and information-barrier-first: \`Principal::admits\` evaluates the deny rules before anything else and returns false on any deny match (deny wins over grant), then admits unconditionally only for the grant-all principal (\`all == true\`), and otherwise requires at least one grant rule to cover the fact. A non-grant-all principal with no matching grant admits nothing. The decision is a pure read over \`denies\`/\`grants\`/\`all\` and the supplied hierarchy+key; it mutates no state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.admits` (hash `a33ed66214c676a1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Rule.covers

- **claim** (`cl\_d62ae32d492d23ae`): A single entitlement \`Rule\` covers a fact only if EVERY one of its scopes covers it (\`scopes.iter().all\`) — scope conjunction, so adding a scope narrows a rule, never widens it. The match is a pure read over the rule's scopes against the hierarchy and fact key; no state is mutated.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Rule.covers` (hash `a107b8c050d75245`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Scope.covers

- **claim** (`cl\_f06e02b618490323`): A single \`Scope\` covers a fact iff the fact's resolved group value on the scope's dimension equals the scope's value (\`resolved\_group\_value(hierarchy, key, dimension) == value\`) — an exact equality on one hierarchy dimension, the atom from which \`Rule::covers\` (scope-conjunction) and \`Principal::admits\` are built. Pure: it only reads the hierarchy and fact key.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Scope.covers` (hash `dc74cc2f895ae7a1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.resolved\_group\_value

- **claim** (`cl\_1d99b23bd76845a3`): Entitlements deny-by-default scope-resolution safety (deliverable: entitlements-deny-by-default). resolved\_group\_value is a pure, deterministic resolver: given (&Hierarchy, &FactKey, DimensionId) it maps Desk/Entity through the hierarchy (falling back to the key's own group value when no parent edge exists) and passes every other dimension straight through key.group\_value, with no I/O and no mutation of its borrowed inputs. Determinism is the safety property — a grant's scope.covers test resolves a fact to exactly the same governing node on every evaluation, so a principal can never be admitted to a scope the deny-by-default rule did not actually grant. Pure (no WRITES edges); self-invalidates on change.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.resolved\_group\_value` (hash `245132bda8435c27`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.carry

- **claim** (`cl\_786ca1fced0f46f6`): EquityInputs::carry() is the pure, side-effect-free accessor for the net cost of carry b = r − q − repo: it reads the three stored fields and returns their scalar difference. This scalar b is the single carry coordinate threading through d1, the forward (S·e^{b·t}), and all carry-tagged Greeks. By convention repo=0.0 for an unencumbered name, which collapses b to r−q (continuous dividend yield form).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.carry` (hash `658e0204ffa59063`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.discount\_df

- **claim** (`cl\_5ccb58f70fe105cf`): EquityInputs::discount\_df() is the pure numeraire discount factor e^{−r·t}: it delegates to celnet\_core::math::exp (libm, correctly-rounded) with no mutation or I/O. Called by 39 downstream consumers and by price() as k\_disc = strike·discount\_df(). Because r is the independent discount axis (separate from carry b), this method's pure-function semantics are load-bearing for the clean partial derivatives in greeks(): the discount\_rho formula holds analytically only when r enters the price solely through e^{-r·t} and e^{(b-r)·t}, which requires b to be independently varied — enforced by with\_r\_fixed\_b.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.discount\_df` (hash `748934905793db60`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.forward

- **claim** (`cl\_14b9e438bf9869c0`): EquityInputs::forward() is the pure equity forward pricer: F = S·e^{b·t} where b = carry() = r−q−repo and the exponential routes through celnet\_core::math::exp (libm-backed, correctly-rounded). No side effects or mutation — a read-only scalar computation. It is called by 45+ downstream consumers (surface rebuilds, risk ladder nodes, carry-seam nodes) making it the canonical equity forward reference.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.forward` (hash `5bcf0268933122a9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.aux

- **claim** (`cl\_99264f337486c073`): aux(i) is the pure precomputation kernel for the generalized-BSM equity pricer: given EquityInputs it computes d1 = \[ln(S/K) + (b + ½σ²)·t\] / (σ√t) where b = carry() = r − q − repo is the net cost of carry, d2 = d1 − σ√t, and returns the Aux struct {d1, d2, sqt=√t, vsqt=σ√t}. No side effects, no allocation, no I/O — a pure closed-form function of its argument. Both price() and greeks() call this exactly once and re-use the cached (d1, d2, sqt, vsqt), so transcendental cost is paid once per pricing call.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.aux` (hash `8ab6c8a47633bc41`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.check\_greeks

- **claim** (`cl\_756aaf79c236bfed`): celnet-equity-vanilla check\_greeks validates all first- and second-order Greeks against finite-difference oracles with tight tolerances: delta\_spot and vega are correct to 1e-4 relative / 1e-7 absolute; delta\_forward = (∂V\_fwd/∂S) / e^{bT} where V\_fwd = V·e^{rT}; theta = −∂V/∂T; discount\_rho = ∂V/∂r at fixed b; carry\_rho = −∂V/∂q; gamma/vanna/volga/charm/speed/zomma/color are validated via first-order FD of lower-order Greeks. Rate sensitivities must tag as RateSensitivities::Carry (not Fx) or the test panics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.check\_greeks` (hash `1cc85f60ba24a6ae`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.greeks

- **claim** (`cl\_57d0b3e8dd13aeb6`): greeks(opt, i) is the complete, pure generalized-BSM Greeks engine for equity vanilla options: it returns EquityGreeks containing price, delta\_spot (∂V/∂S = e^{(b-r)t}·Φ(±d1)), delta\_forward (∂V/∂F = Φ(±d1)), gamma (e^{(b-r)t}·φ(d1)/(S·σ√t)), vega (S·e^{(b-r)t}·√t·φ(d1)), theta (generalized-BSM carry-tagged decay), discount\_rho (∂V/∂r holding b fixed), carry\_rho (∂V/∂b holding r — the dividend-rho), vanna (−e^{(b-r)t}·φ(d1)·d2/σ), volga (vega·d1·d2/σ), charm (∂Δ/∂T), speed (∂γ/∂S), zomma (∂γ/∂σ), and color (∂γ/∂T). The two rate sensitivities are INDEPENDENT partials in the (r, b) basis: since d1/d2 depend on b but not r, the φ-terms cancel in carry\_rho, yielding clean closed forms. No I/O, no mutation, no allocation beyond the returned struct.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.greeks` (hash `27e69108a36b4d7d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.hull\_index\_option\_reference

- **claim** (`cl\_e07ceca0ef5bbf00`): hull\_index\_option\_reference() is the externally-pinned numerical oracle for the generalized-BSM equity pricer: for S=930, K=900, r=0.08, q=0.03, σ=0.20, T=1/6 yr, repo=0 it asserts call=51.832956796490860 and put=14.550996773772400 (both to 1e-9 relative and absolute). This is an independently-computed full-precision result — not round-tripped from the crate under test — providing a ground-truth anchor that is source-stable (the specific reference values appear in the test source verbatim). Validated by assert\_close! to 9 decimal places.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.hull\_index\_option\_reference` (hash `be855c4e137b83ad`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.price

- **claim** (`cl\_7f08e3859f7fdb56`): CAPABILITY (equity cross-asset leaf): celnet-equity-vanilla::price is the generalized-BSM equity vanilla pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &EquityInputs) where the dividend yield enters as the carry b = r - q, so the no-dividend limit collapses to standard Black-Scholes (proven by no\_dividend\_limit\_is\_standard\_bsm) and the leaf reconciles to an independent QuantLib-pinned BSM oracle. Heavily re-used (in\_degree 147) as the equity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.price` (hash `a6f92a2e94bed6b0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.with\_b\_via\_q

- **claim** (`cl\_37245da41f9104a3`): with\_r\_fixed\_b(i, r) is the pure finite-difference perturbation helper for discount\_rho (∂V/∂r at fixed b): to hold b = r−q−repo constant while shifting r, it adjusts q by the same delta (q' = q + (r\_new − r\_old)) so that carry() is unchanged. with\_b\_via\_q(i, q) is the complementary helper for carry\_rho (∂V/∂b at fixed r): it directly replaces q in the struct, since ∂b/∂q = −1 so ∂V/∂b = −∂V/∂q. Both are pure (no side effects, no I/O, return a new EquityInputs by struct-update). These helpers are used by the FD oracle in the test suite to verify the closed-form discount\_rho and carry\_rho against central differences.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.with\_b\_via\_q` (hash `14730663a9fb9608`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.with\_r\_fixed\_b` (hash `2eb0535342584b40`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.accumulator.accumulator\_price

- **claim** (`cl\_9fe5eeeb19134264`): accumulator\_price prices a target-accrual accumulator via antithetic-pair quasi-Monte Carlo: it draws counter-based normal variates with CounterRng, walks two antithetic paths (sign +1 and −1) via walk\_path, averages each pair's (bank\_pv, settled\_fixings) with coefficient 0.5, and accumulates both statistics in Welford online estimators. The carry drift per step is \`(carry\_rate() − ½σ²)·dt\` and the discount factors are precomputed once as \`discount\_df\_at(t\_k)\` for k=1..n, matching the FX two-rate GK form byte-for-bit (ADR-0008).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.accumulator.accumulator\_price` (hash `b93836d37a8bac76`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.adi.Lsv2d\<'\_\>.is\_dead

- **claim** (`cl\_f7ce9f2adc097763`): Lsv2d::is\_dead returns true for a log-spot grid index i that lies on the absorbed (knocked-out) side of the barrier, but only when the wall is active. For an up-barrier at index idx, indices i ≥ idx are dead; for a down-barrier, indices i ≤ idx are dead. When the wall is inactive or absent the method always returns false.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.adi.Lsv2d\<'\_\>.is\_dead` (hash `dd3b65fafee19271`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.adi.Lsv2d\<'a\>.new

- **claim** (`cl\_8ffcfc4524f3e4de`): Lsv2d::new constructs the 2-D ADI grid layout by aligning both axes so that the initial spot and initial variance are exactly on nodes. For the log-spot axis it either uses axis\_with\_barrier (when a knock-out level is present, aligning both spot and barrier on nodes) or computes a uniform grid spanning the ATM ±width\_in\_std standard-deviation region with the spot pinned on a node. For the variance axis it computes a uniform grid over \[0, v\_max\_mult·max(v0, long\_var)\] with v0 on a node. wall\_active is set to true if and only if a knock-out level was provided.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.adi.Lsv2d\<'a\>.new` (hash `958f2dd81a87e246`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.adi.apply\_wall

- **claim** (`cl\_9eb41451e6b6442c`): apply\_wall enforces the absorbing Dirichlet barrier condition on the 2-D option value grid after an ADI step: when the barrier is active (\`wall\_active=true\`), it zeroes every grid cell on the knocked-out side. For an up-barrier at index \`idx\`, all cells with spot-index i ≥ idx are zeroed across all variance nodes j; for a down-barrier, all cells with i ≤ idx are zeroed. When \`wall\_active\` is false the function returns immediately, adding zero overhead.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.adi.apply\_wall` (hash `a7d891f45a73ca06`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.adi.implicit\_v

- **claim** (`cl\_c9600718a074effc`): implicit\_v solves one implicit half-step of the ADI scheme in the variance (v) dimension for the 2-D LSV PDE. For each interior log-spot node i that is not dead (barrier-absorbed), it assembles a tridiagonal system over all nv variance nodes: the v=0 boundary uses upwind discretisation of the pure-convection term κθ·∂U/∂v; interior nodes use the v-stencil (which includes mean-reversion, vol-of-var diffusion, and convection); the top boundary applies the linear extrapolation condition U\_vv=0 (reducing to first-order finite difference). The system is solved with the Thomas algorithm, and the result overwrites the output buffer for that x-slice.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.adi.implicit\_v` (hash `7932a1e7684cd8e1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.adi.implicit\_x

- **claim** (`cl\_92cf3f4a4130b496`): implicit\_x solves one implicit half-step of the ADI scheme in the log-spot (x) dimension for the 2-D LSV PDE. For each interior variance node j, it assembles a tridiagonal system using the x-stencil coefficients derived from the local-volatility–scaled drift-diffusion operator \`l²·v\` (where l is the leverage function at the current calendar time and spot, and v is the variance level), folds the fixed Dirichlet boundary values at the live-range endpoints into the right-hand side, and solves via the Thomas algorithm (\`thomas\_variable\`). The implicit weight is \`HV\_THETA·dtau\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.adi.implicit\_x` (hash `930269b67bf6e6ac`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.american.ExerciseSchedule.new

- **claim** (`cl\_690fadc3f0e53ab5`): ExerciseSchedule::new encodes the exercise schedule into either \`every:true\` (American — exercise allowed at every backward step) or a sorted, deduped set of interior backward-step indices for Bermudan dates. Bermudan dates are filtered to strictly-interior calendar times (0 \< d \< t), mapped to backward step index k = round(d/dtau)−1 clamped to \[1, time\_steps\], and stored in ascending order. Expiry is always available via the initial terminal-boundary seeding and is explicitly excluded from the interior set.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.american.ExerciseSchedule.new` (hash `17c28038420024e5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.american.exercise\_dates

- **claim** (`cl\_f03ccf3c2b1e2cb6`): exercise\_dates enumerates all exercise dates for an AmericanOption. For American style it returns n equally-spaced dates {t·k/n \| k=1..=n}. For Bermudan style it takes the caller-supplied dates, filters to (0, t\], clamps each to t, appends the expiry t unconditionally, sorts, and deduplicates to within 1e-12. This guarantees expiry is always included and dates are strictly ordered.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.american.exercise\_dates` (hash `d78679d53c686264`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.american.fd\_greeks

- **claim** (`cl\_87d8654e5f17560f`): fd\_greeks computes the full set of American-option finite-difference Greeks by repeated calls to a pricing closure. Spot Greeks use central differences with h\_S = spot × 1e-4: delta = (p\_up − p\_dn)/(2h), gamma = (p\_up − 2·base + p\_dn)/h², speed = (p\_up2 − 2p\_up + 2p\_dn − p\_dn2)/(2h³). Vol Greeks use h\_V = 1e-4 bp. Cross Greeks (vanna, zomma) use 4-point cross-difference stencils. Rate bumps follow the carry arm (FxRates or CostOfCarry) so FX byte-identity is preserved. Theta and charm use a relative time bump h\_T = t × 1e-4 with t\_dn clamped to f64::MIN\_POSITIVE. Forward-delta is scaled by exp(yield\_rate × t) using the stored yield (never a discount−carry reconstruction).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.american.fd\_greeks` (hash `f9372eb810bff2c9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.american.ridge\_regress

- **claim** (`cl\_86a420f0da16a67a`): ridge\_regress solves the 4-feature Longstaff-Schwartz regression with ridge regularisation for the American-option LSM pricer. It accumulates XᵀX (4×4) and Xᵀy (4-vector) in a single pass, then adds diagonal ridge \`ε = 1e-12·(trace(XᵀX) + 1)\` for numerical stability when the ITM spot range is narrow and the design matrix is near-singular. The system is solved by Thomas-like elimination via \`solve4\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.american.ridge\_regress` (hash `d74c76237bf7abc8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.black\_on\_average

- **claim** (`cl\_d516337860d4e4a3`): black\_on\_average applies a lognormal Black formula to the conditional arithmetic-average option, matching the log-variance from the first two moments: \`var = ln(E\[A²\]/E\[A\]²)\`. It degenerates gracefully: (1) if \`var ≤ 0\` or \`ex ≤ 0\` it returns discounted intrinsic \`df·(φ(ex − k\_eff))⁺\`; (2) if \`k\_eff ≤ 0\` a call returns \`df·(ex − k\_eff)\` and a put returns 0. Otherwise \`d1 = (ln(ex/k\_eff) + ½var)/√var\`, \`d2 = d1 − √var\`, result = \`df·φ·(ex·Φ(φ·d1) − k\_eff·Φ(φ·d2))\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.black\_on\_average` (hash `15150e758a0324c3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.continuous\_moments

- **claim** (`cl\_8772178d75354521`): continuous\_moments computes the first and second moment of a continuously-averaged arithmetic Asian (E\[A\] and E\[A²\]) in closed form. M1 handles the \`b → 0\` degenerate case by the limit \`S₀·exp(b·t\_start)\` (rather than 0/0). M2 is the double integral \`(2/τ²)·S₀²·exp(2b·t\_start + σ²·t\_start)·∫∫ exp(b(r+s)+σ²·min(r,s)) dr ds\` evaluated analytically via the \`expm1\_over\` helper (which switches to the limit \`x\` when the rate is near zero to avoid cancellation), covering all degenerate sub-cases \`p≈0\`, \`q≈0\`, and the general case.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.continuous\_moments` (hash `8748cf94fc4bf35d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.curran\_price

- **claim** (`cl\_99269430bb6ca263`): curran\_price prices an arithmetic Asian option by Curran's conditioning approximation: it conditions on \`ln G\` (the log of the geometric average, which is Gaussian), finds the single exercise boundary \`z\*\` where the conditional average equals the strike by 80-iteration bisection on a ±8σ window, splits the Gauss-Legendre-64 outer integral at \`z\*\` for smoothness, and applies a conditional Black formula for the residual within-G dispersion at each quadrature node. The \`k\_eff = strike − fixed\` effective strike handles already-accrued fixings (the seasoned weight \`w\` and \`elapsed\_avg\`). The continuous averaging case is handled as a dense 256-point discrete approximation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.curran\_price` (hash `94ac42b0fac1f38e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.discrete\_moments

- **claim** (`cl\_c22ba71ffb9e33e4`): discrete\_moments computes the first and second moment of a discretely-averaged arithmetic Asian from the observation schedule. M1 = \`(1/n) Σ S₀·exp(b·t\_k)\`. M2 = \`(S₀²/n²) Σ\_i Σ\_j exp(b(t\_i+t\_j) + σ²·min(t\_i,t\_j))\`, exploiting symmetry by iterating the upper triangle and doubling off-diagonal terms so the full n×n sum is formed with n(n+1)/2 exponential evaluations.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.discrete\_moments` (hash `e604a5d1f4a9e4f2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.future\_moments

- **claim** (`cl\_f532edbbca962d86`): future\_moments dispatches Asian moment computation to the correct sub-routine based on the averaging schedule: for a discrete schedule with at least 1 future observation it calls discrete\_moments; for a continuous schedule it calls continuous\_moments. The assertion \`future\_obs \>= 1\` is an enforced precondition.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.future\_moments` (hash `aa8c471bdd016837`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.geometric\_average\_price

- **claim** (`cl\_981065dc4cd081be`): geometric\_average\_price prices a geometric-average Asian option analytically. For discrete schedules with n future observations it computes the effective volatility σ\_G²= σ²(n+1)(2n+1)/(6n²) and effective cost-of-carry b\_G = ½(b − ½σ²)(n+1)/n + ½σ\_G², constructs a synthetic ExoticInputs with these parameters, and delegates to carry\_vanilla\_price. For continuous averaging it delegates to continuous\_geometric\_price which uses σ\_G²=σ²/3 and the corresponding carry adjustment. Both paths are asset-class-agnostic via the carry-seam ExoticInputs representation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.geometric\_average\_price` (hash `8633f7d1b7e061f3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.asian.turnbull\_wakeman\_price

- **claim** (`cl\_bf20fc897be0e73a`): turnbull\_wakeman\_price is the Turnbull-Wakeman moment-matching Asian pricer: it computes future\_moments(i, spec) to get the first and second moments (ex, ex2) of the remaining average, applies seasoned\_match to account for any already-settled fixings (adjusting the effective strike k\_eff = spec.strike − fixed), then prices via black\_on\_average(spec.option, ex, ex2, k\_eff, df) — a Black-76 formula on the lognormal approximation of the arithmetic average. Pure: reads &ExoticInputs + AnalyticAsian, returns f64, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.turnbull\_wakeman\_price` (hash `03370830947a2313`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.double\_knock\_out\_price

- **claim** (`cl\_124eab5cda19697b`): double\_knock\_out\_price prices a double-knock-out option using the Andersen-Brotherton-Ratcliffe reflection-image series (summed over n = −DKO\_TERMS..=DKO\_TERMS). For each image term it evaluates two scaled CDF differences — one for the direct image (weight \`(U/L)^{n·μ₁}\`) and one for the lower-wall mirror (weight base \`L^{n+1}/(Uⁿ·S)\`) — via \`scaled\_cdf\_diff\` which evaluates the product \`exp(ln\_scale + ln\|ΔΦ\|)\` in log-space to prevent Inf×0 NaN. The payoff is oriented by \`phi = option.sign()\` applied inside each CDF argument, so the raw \`sum\` is the put/call value directly and only the payoff floor \`.max(0.0)\` is applied on return.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.double\_knock\_out\_price` (hash `d15aa37a05dc9761`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.scaled\_cdf\_diff

- **claim** (`cl\_3d03e8855ebc32f9`): scaled\_cdf\_diff evaluates \`exp(ln\_scale) · (Φ(a) − Φ(b))\` in a numerically stable form that prevents Inf×0 NaN when the Girsanov weight overflows while the CDF difference underflows. When both arguments are non-negative it uses the equivalent tail form \`Φ(−b) − Φ(−a)\` (exploiting symmetry for precision). The product is evaluated as \`signum(diff) · exp(ln\_scale + ln\|diff\|)\` and returned as 0.0 if non-finite.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.scaled\_cdf\_diff` (hash `5d29c0f093882476`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single\_barrier\_no\_rebate

- **claim** (`cl\_22bec881757fe676`): single\_barrier\_no\_rebate prices a FX single-barrier option analytically via the Reiner-Rubinstein image construction. It computes the knock-in value directly from the four RrBlocks (a, b, c, d) according to the {up/down} × {call/put} × {K≥H / K\<H} case table, then derives knock-out as \`vanilla − knock\_in\`, guaranteeing exact in/out parity. A pre-breached barrier is resolved immediately: knock-out returns 0, knock-in returns the plain vanilla. The function hard-rejects non-FX carry via \`as\_fx\_vanilla(strike).expect(...)\` — non-FX barriers must route to PDE/MC.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single\_barrier\_no\_rebate` (hash `2b492c5a8cb6171c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single\_barrier\_price

- **claim** (`cl\_b6e5a9c4bf173005`): single\_barrier\_price decomposes a single-barrier option with optional rebate into two independent legs: (1) bare = single\_barrier\_no\_rebate(i, kind, strike, barrier) — the core Reiner-Rubinstein reflection formula; (2) rebate leg — for KnockOut, a one-touch paying rebate at hit (one\_touch\_price(i, barrier, rebate, RebateTiming::AtHit)); for KnockIn, a no-touch paying rebate at expiry if the barrier is never touched (no\_touch\_price). Returns bare + reb. When rebate == 0.0, returns bare immediately without pricing the touch. Pure: reads &ExoticInputs + SingleBarrier, returns f64, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single\_barrier\_price` (hash `6e2df6f61e5e1226`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.digital.digital\_greeks

- **claim** (`cl\_e56bb71ac780bc63`): digital\_greeks computes closed-form delta, gamma, and vega for both cash-or-nothing and asset-or-nothing digitals. For cash-or-nothing: delta = \`ω·df\_dom·φ(d₂)/(S·σ√T)\`, gamma = \`−ω·df\_dom·φ(d₂)·d₁/(S²·σ²T)\`, vega = \`−ω·df\_dom·φ(d₂)·d₁/σ\`. For asset-or-nothing: delta = \`df\_for·\[Φ(ω·d₁) + ω·φ(d₁)/σ√T\]\`, gamma = \`ω·df\_for·\[φ(d₁)/(S·σ√T) − d₁·φ(d₁)/(S·(σ√T)²)\]\`, vega = \`−ω·S·df\_for·φ(d₁)·d₂/σ\`. Both use \`d12(i)\` for the standard GK arguments.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.digital.digital\_greeks` (hash `70b9d4cb55045295`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.digital.digital\_price

- **claim** (`cl\_e0c7d1e7b5718500`): digital\_price is the closed-form dual-style digital kernel: given (kind, i) it reads (d1,d2) from d12(i), df\_dom = i.discount\_df(), df\_for = i.carry\_df(), and returns — CashOrNothing Call: df\_dom·Φ(d2); CashOrNothing Put: df\_dom·Φ(−d2); AssetOrNothing Call: S·df\_for·Φ(d1); AssetOrNothing Put: S·df\_for·Φ(−d1). The match is exhaustive over (DigitalStyle, OptionType) with no wildcard, enforcing that cash digitals discount with the numeraire factor and asset digitals with the yield factor. Pure: reads &ExoticInputs, returns f64, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.digital.digital\_price` (hash `4edebc5f14b4d33d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.carry\_vanilla\_price\_at

- **claim** (`cl\_467a150c645b2316`): CAPABILITY (cross-asset carry-seam reach): carry\_vanilla\_price\_at is the generalized closed-form pricing kernel of the cross-asset carry seam — a pure, side-effect-free generalized-Black-Scholes-Merton evaluation parameterized by a generalized Carry (cost-of-carry b = r - q via Carry::discount\_rate/yield\_rate). Because every asset family lowers onto this one Carry-parameterized kernel (FX as r\_dom/r\_for, equity as r/dividend-yield, commodity as Black-76 r/b, crypto-linear as r/funding), the SAME pure kernel reaches vanilla/exotics/surface/risk across the FX, equity, commodity, and crypto/digital-asset and linear leaves. It performs no I/O, allocation, logging, or mutation: d1/d2 and discounted spot/strike are computed and one branch on OptionType returns the price.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.carry\_vanilla\_price\_at` (hash `0c52adc0ff99c0b2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.leverage.LocalVolSurface\<'a, S\>.local\_var

- **claim** (`cl\_c9ad5a2d82194f01`): local\_var implements Dupire's local-variance formula in the Gatheral total-variance parameterisation. Given spot S and time t, it computes log-moneyness y = ln(S/F(t)) against the carry-consistent forward F(t), evaluates total variance w(y,t) and its spatial derivatives wy, wyy via central finite differences with step dy, and its time derivative wt via central differences in t (one-sided at the boundary). It then evaluates the Gatheral denominator g(y) = (1 − y·wy/w) + ¼·(−¼ − 1/w + y²/w²)·wy² + ½·wyy and returns wt/g(y), floored at floor\_var. When w ≤ 0 it immediately returns floor\_var to prevent division by zero.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.leverage.LocalVolSurface\<'a, S\>.local\_var` (hash `efc0482ef50faea2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.leverage.bracket

- **claim** (`cl\_d4df4e0c0e6a4f25`): bracket performs a binary search on a sorted f64 grid to find the bracketing interval \[lo, hi\] and the linear interpolation fraction for a query value x. It clamps to the first element when x ≤ grid\[0\] (returning fraction 0.0) and to the last element when x ≥ grid\[n-1\]. For interior values, the loop invariant grid\[lo\] ≤ x \< grid\[hi\] is maintained until hi−lo=1, after which fraction = (x−grid\[lo\])/(grid\[hi\]−grid\[lo\]) ∈ \[0,1).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.leverage.bracket` (hash `d8ea0814178ce8ea`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.fixed\_lookback\_price

- **claim** (`cl\_2d94a89503286659`): floating\_lookback\_price and fixed\_lookback\_price are pure Conze-Viswanathan closed-form lookback pricers over the carry seam: both read b = carry\_rate(), df\_dom = discount\_df(), df\_for = carry\_df() — byte-identical for FX. floating\_lookback\_price prices the option on the running extremum ξ=S at inception: Call = S·df\_for·Φ(a1) − S·df\_dom·Φ(a2) + S·df\_dom·(σ²/2b)·\[Φ(−a1+2b√T/σ) − e^{bT}·Φ(−a1)\]. fixed\_lookback\_price branches on K≷S to select between the standard Conze-Viswanathan form (K≥S) and the intrinsic-lock form (K\<S), each with the corresponding reflection term σ²/(2b)·\[±(S/K)^{−2b/σ²}·Φ(d1−2b√T/σ·…) ∓ e^{bT}·Φ(d1)\]. Both are pure (no WRITES, no allocation).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.fixed\_lookback\_price` (hash `35d6965174cd38ac`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.floating\_lookback\_price` (hash `8429a808e1194a12`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.lookback\_payoff

- **claim** (`cl\_0896453993df563d`): lookback\_payoff simulates one antithetic-signed lookback path and computes the path payoff. At each step it updates the running minimum and maximum using the reflected-Brownian-bridge interval extrema: \`bridge\_min = ½\[(x\_prev+x\_next) − √((x\_next−x\_prev)² − 2·var\_step·ln(u\[k\]))\]\` and \`bridge\_max\` symmetrically (u\[k\] uniform variate for the bridge correction). The four payoff cases are: floating-strike call \`S\_T − min\`, floating-strike put \`max − S\_T\`, fixed-strike call \`(max − K)⁺\`, fixed-strike put \`(K − min)⁺\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.lookback\_payoff` (hash `e7a5bb0e7908d3ab`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.lsv.LsvModel.calibrate

- **claim** (`cl\_2c5fad6df3a1ee14`): DELIVERABLE lsv/market-calibration-frontend = PARTIAL / still OPEN (backlog tracker docs/WORLD-CLASS-BACKLOG.md Round-2 P2/L: "LSV booking model has no market-calibration front end: no Heston-backbone NLS calibration and no mixing-weight (eta) tuning to touch/DNT quotes"; reconciled against the live graph). What EXISTS today: \`LsvModel::calibrate(inputs, var, iv, spot\_grid, cfg)\` is a pure, side-effect-free constructor that builds an LsvModel by delegating to particle::calibrate\_leverage(iv, &var, spot\_grid, spot, t, cfg) — a PARTICLE leverage-function calibration to an ImpliedVolSurface — and stores {inputs, var, leverage}, no I/O/writes/allocation-in-loop in the constructor itself. The OPEN gap the finding names is NOT closed here: this calibrates the local-vol LEVERAGE to a given IV surface; it does NOT do a Heston-backbone nonlinear-least-squares calibration of the variance params, and the mixing weight (eta) is taken from VarianceParams rather than tuned to touch/DNT market quotes. SELF-INVALIDATING: when a Heston-NLS + mixing-eta market-calibration front end lands, this method's signature/body changes (it would take market touch/DNT quotes and tune var/eta), flipping or unresolving this claim — the signal that the deliverable's remaining half closed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.lsv.LsvModel.calibrate` (hash `66c306c10a5fc97a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.market\_hedge\_overlay.hedge\_smile\_cost

- **claim** (`cl\_c143b7449828ede7`): DELIVERABLE exotics/vanna-volga-overlay-magnitude-unvalidated = LANDED (backlog tracker still lists it OPEN as a Round-2 P2/M finding; reconciled against the live graph). \`hedge\_smile\_cost\` is the pure (side-effect-free) vanna-volga market-hedge smile-overlay cost: it reads ExoticSensitivities + the broker RR/BF marks and returns the overlay cost with no external writes. The Round-2 gap (only flat-smile/sign/scaling tests; the spec-mandated VV-vs-replication magnitude cross-validation unimplemented) is CLOSED: celnet-parity::vv\_magnitude::engine\_overlay\_matches\_replicating\_portfolio\_oracle\_in\_magnitude now pins the engine overlay against a CODE-DISJOINT replicating-portfolio oracle (oracle\_cost) within a derived 20% magnitude band, with \<=1% relative agreement on the cross-Greeks (vanna/volga) and a materiality floor + sign-agreement guard across the product set, backed by the golden oracle hedge\_smile\_overlay\_cost. SELF-INVALIDATING: any edit to the overlay arithmetic shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.market\_hedge\_overlay.hedge\_smile\_cost` (hash `29524a2874bb4b1d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.market\_hedge\_overlay.hedge\_smile\_cost

- **claim** (`cl\_e04fb533388dd3ed`): hedge\_smile\_overlay is the Vanna-Volga smile-cost overlay: it computes raw = vanna \* market.vanna\_price + volga \* market.volga\_price (hedge\_smile\_cost), multiplies by survival.probability (the barrier-survival weight, clamped to \[0,1\] to prevent over-hedging near the barrier), and returns OverlayResult{flat\_vol\_price, hedge\_smile\_cost: survival.probability\*raw, smile\_price: flat\_vol\_price + cost}. The survival weighting accounts for the reduced probability that an exotic product survives to expiry — reducing the smile correction proportionally. Pure: reads inputs and returns OverlayResult, no WRITES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.market\_hedge\_overlay.hedge\_smile\_cost` (hash `29524a2874bb4b1d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.market\_hedge\_overlay.hedge\_smile\_overlay` (hash `fab6408771222d73`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.mc.price\_asian

- **claim** (`cl\_a707a4cad50f97e3`): price\_asian prices an arithmetic Asian option by Monte Carlo with a geometric-average control variate. It runs two passes: the first collects antithetic-pair (arithmetic, geometric) samples and accumulates online covariance via Welford to estimate the regression coefficient β = Cov(arith, geo)/Var(geo). The second pass applies the control correction Y = arith\_sample − β·(geo\_sample − E\[geo\]) where E\[geo\] is \`geometric\_asian\_price / df\` (the undiscounted analytic geometric mean). The final price is \`df·E\[Y\]\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.mc.price\_asian` (hash `f9332c4946b55771`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.CorrelationError.fmt

- **claim** (`cl\_801973b6de4f7f13`): CorrelationError::fmt maps each variant to a fixed, human-readable diagnostic string with no allocation or formatting beyond a static str write: Shape → 'correlation matrix shape does not match the leg count'; NotSymmetric → 'correlation matrix is not symmetric'; NonUnitDiagonal → 'correlation matrix diagonal is not unit'; NotPositiveDefinite → 'correlation matrix is not positive-definite (not a valid correlation matrix)'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.CorrelationError.fmt` (hash `fdbf4366e3f836cb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.discounted\_payoff

- **claim** (`cl\_6fb091c89754f907`): discounted\_payoff aggregates basket legs according to the basket kind and applies the option payoff. For BasketKind::Basket the aggregate is \`Σ weight\_i · S\_i(T)\` (weighted sum). For BestOf it is \`max\_i(weight\_i · S\_i(T))\`. For WorstOf it is \`min\_i(weight\_i · S\_i(T))\`. The intrinsic \`(agg − K)⁺\` or \`(K − agg)⁺\` is then discounted by \`df\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.discounted\_payoff` (hash `b901b0075bddecc8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.pde.GridLayout.apply\_dirichlet

- **claim** (`cl\_ea5a692a2cdeb2ad`): apply\_dirichlet enforces the 1-D absorbing barrier condition on the 1-D PDE value slice: when a wall is set at index \`idx\`, it zeroes all elements at or beyond that index (for an up-barrier, elements from idx onward; for a down-barrier, elements up to and including idx). This is the 1-D analogue of \`apply\_wall\` used by the scalar PDE solver.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.pde.GridLayout.apply\_dirichlet` (hash `9046a32a6e8efddd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.pde.GridLayout.new

- **claim** (`cl\_32e15ed3d0f0d8e7`): GridLayout::new constructs the PDE log-spot grid, centering the domain at \`ln\_S + (b−½σ²)T\` (the expected log-spot at expiry) with half-width \`width\_in\_std · σ√T\`. When a knock-out barrier is present the grid is aligned to pass a node exactly through \`ln(level)\` (barrier alignment via \`aligned\_to\_barrier\`) to eliminate the grid-non-alignment error in the barrier price.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.pde.GridLayout.new` (hash `98e1dbbc0c07ddc5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.analytic\_greeks\_match\_central\_finite\_difference

- **claim** (`cl\_659ca2a412eefa71`): analytic\_greeks\_match\_central\_finite\_difference verifies that perpetual\_greeks returns Greeks consistent with perpetual\_price via symmetric finite-difference checks at h=1e-4·S for delta/gamma, h=1e-6 for vega, and h=1e-7 for rate sensitivities. Specifically: delta = ∂V/∂S, gamma = ∂²V/∂S² (via delta FD), vega = ∂V/∂σ, and rhos are computed through the cost-of-carry chain (discount\_rho = ∂V/∂r, carry\_rho = ∂V/∂b) or the FX chain (rho\_dom = ∂V/∂r\_dom, rho\_for = ∂V/∂r\_for). All agree to 1e-6 relative / 1e-9 absolute (spot Greeks) or 1e-5/1e-7 (rate Greeks). The price field of PerpetualGreeks must be bit-identical to perpetual\_price output.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.analytic\_greeks\_match\_central\_finite\_difference` (hash `21968505c96079a3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.characteristic\_roots

- **claim** (`cl\_eb534af67ea52c9d`): characteristic\_roots computes the two real roots of the perpetual-option characteristic equation \`½σ²·y² + (b−½σ²)·y − r = 0\` using a cancellation-free pairing: the half-sum \`half = −½(lin ± √disc)\` always adds same-signed terms, and the second root is recovered as \`con/half\` (product-of-roots formula \`−r/(½σ²)/first\`) without ever subtracting nearly-equal magnitudes. This matters in the σ→0 regime where \`\|b\|/σ² → ∞\`. The degenerate double-root case (\`half == 0\`) returns \`(0, 0)\` by exact structural comparison (r == 0 and b == ½σ²).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.characteristic\_roots` (hash `144114fc26be515a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.closed\_form\_matches\_independent\_bisection\_rederivation

- **claim** (`cl\_47dfd34e73bf7c14`): closed\_form\_matches\_independent\_bisection\_rederivation verifies that perpetual\_price and perpetual\_exercise\_boundary implement the exact closed-form solution to the characteristic equation ψ(y) = ½σ²y(y−1) + by − r = 0. For calls, the root y₁ \> 1 is isolated by bisection (200 halvings, reaching machine precision) with initial bracket expanded from hi=2 doubling until ψ(hi)\>0; the exercise boundary is S\* = K·y₁/(y₁−1) and the price is (S\*−K)·(S/S\*)^y₁ for S\<S\*, else S−K. For puts, the root y₂\<0 is found symmetrically by bracketing on the negative axis. Both price and boundary must agree with the bisection rederivation to 1e-10 relative / 1e-12 absolute tolerance across six market points including the σ→0 regime (vol=1e-3).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.closed\_form\_matches\_independent\_bisection\_rederivation` (hash `2bfdefde1af04c63`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_exercise\_boundary

- **claim** (`cl\_1985f1ea23157af6`): perpetual\_exercise\_boundary exposes the early-exercise boundary S\* as a public function. For calls the boundary is \`K·y₁/(y₁−1)\` where y₁ \> 1; \`b ≥ r\` cases return \`INFINITY\` (no finite boundary). For puts the boundary is \`K·y₂/(y₂−1) ∈ (0,K)\`; \`y₂ == 0\` (r == 0) returns \`0.0\`. The sub-ulp rounding-collapse \`y₁ == 1.0\` also returns \`INFINITY\`, mirroring \`valuation\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_exercise\_boundary` (hash `f3bf87073e347c84`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_greeks

- **claim** (`cl\_356f8ea1552c01a8`): perpetual\_greeks computes delta, gamma, vega, and rate sensitivities for a perpetual option by differentiating the closed-form valuation via smooth-pasting identities. In the continuation region, delta = y·V/S and gamma = y·(y−1)·V/S². Vega uses the implicit-function derivative of the characteristic root y with respect to σ: dy/dσ = −σ·y·(y−1)/ψ'(y), where ψ'(y) = dpsi\_dy. Rate sensitivities are tagged by carry type: for FX inputs, rho\_dom = discount\_rho + carry\_rho and rho\_for = −carry\_rho (chain rule through r=r\_dom, b=r\_dom−r\_for); for cost-of-carry inputs, the two are reported separately.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_greeks` (hash `aaf2e67d714b089d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_price

- **claim** (`cl\_f2d3a9e45035253c`): DELIVERABLE proto/new-payoff-shapes (PerpetualOption arm) = DONE (RC cut a0817d6; arm 30). \`perpetual\_price(opt, i)\` is a pure, total closed-form valuation of an American perpetual option: it dispatches on the \`valuation(opt, i)\` result and returns spot for a never-exercised call, strike for a never-exercised put, the intrinsic \`opt.sign()\*(spot-strike)\` when immediately exercised, and the continuation \`value\` otherwise — every \`Valuation\` arm enumerated, no wildcard, so the match is exhaustive and a new regime is a compile error rather than a silent fall-through. Pure: it borrows \`&PerpetualInputs\`, performs no I/O/allocation/mutation, and returns \`Result\<f64, PerpetualError\>\`. Verified non-circularly against an independent root-bracketing re-derivation (closed\_form\_matches\_independent\_bisection\_rederivation) in the same module. This is one of the two genuinely-new payoff shapes the backlog tracked as OPEN; it is now built + golden/parity-gated + surfaced across all 5 clients.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual\_price` (hash `a6fd1abce7dcc68b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.valuation

- **claim** (`cl\_073832c06c898074`): valuation determines the perpetual-option continuation value and optimal exercise boundary from the characteristic roots. For a call it uses \`y₁ = y\_high \> 1\` (guaranteed when b \< r) with boundary \`K·y₁/(y₁−1)\` and value \`(L\*−K)·(S/L\*)^{y₁}\`; the sub-ulp rounding-collapse case \`y₁ == 1.0\` and the structural \`b == r\` case both map to \`NeverExercisedCall\` (the y₁ → 1⁺ limit V = S). For a put it uses \`y₂ = y\_low \< 0\` with boundary \`K·y₂/(y₂−1) ∈ (0,K)\`; the \`y₂ == 0\` case (r == 0, exact-zero structural comparison) maps to \`NeverExercisedPut\`. The error \`CallCarryExceedsDiscount\` is returned for b \> r (divergent call value).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.valuation` (hash `fec97f0db07a1b52`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.pivot.pivot\_tra\_price\_cv

- **claim** (`cl\_d90298a951bff578`): pivot\_tra\_price\_cv prices a Pivot TRA via antithetic-pair MC with a linear control variate. The control is the sum of discounted forward legs \`Σ\_k g·(S₀·carry\_df\_at(t\_k) − K·df\_k)\` — computed from the STORED yield via \`carry\_df\_at\` (never \`discount\_rate() − carry\_rate()\` reconstruction, preserving FX byte-identity per spec §8). The regression coefficient β = online Cov(pivot\_pv, X) / Var(X) is estimated in the first pass; the second pass applies \`Y = pivot\_pv − β·(X − E\[X\])\` and reports \`E\[Y\]\` with its standard error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.pivot.pivot\_tra\_price\_cv` (hash `234bea16e1b009c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.pivot.walk\_path

- **claim** (`cl\_1fa0e2293c305680`): walk\_path simulates one antithetic-signed Pivot-TRA path and accumulates the bank PV. At each fixing, the pivot determines which leg applies: on the favourable side (g·(S−P) ≥ 0) the per-unit cash flow c = g·(S−K); on the adverse side c = leverage·g·(S−K), preserving the signed intrinsic. Positive cash flows accrue toward the target; on breach the settled amount is either the full gain (FullGain) or capped at the remaining target (CappedGain), and a PathOutcome with the overshoot and redemption index is returned immediately. The control strip (un-knocked-out forward strip) continues to accumulate over all remaining fixings even after breach to enable control-variate variance reduction. Bank PV sign convention: bank pays favourable legs (bank\_pv decremented) and receives adverse legs (bank\_pv incremented). At P=K the function is bit-identical to the TARF pricer.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.pivot.walk\_path` (hash `a22971ccbab42c99`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.quanto.quanto\_digital\_mc

- **claim** (`cl\_e76a17b83f10d671`): quanto\_digital\_mc prices a quanto digital option via antithetic Monte Carlo, applying the quanto drift adjustment through quanto\_adjusted\_inputs before simulation. Each path draws a single standard-normal variate z; antithetic pairing evaluates pays(+1) and pays(−1) and averages them, which zeroes the first-order MC bias. The discounted mean over cfg.pairs paths is the price estimate; the standard error is computed from the Welford online variance.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.quanto.quanto\_digital\_mc` (hash `243f3ac275c43974`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.quanto.quanto\_digital\_price

- **claim** (`cl\_e32bb0d3c6156aaf`): quanto\_digital\_price prices a cash-or-nothing digital under quanto adjustment. It calls \`quanto\_adjusted\_inputs\` to shift the drift by the quanto correlation adjustment (−ρ·σ\_S·σ\_X·T absorbed into an effective carry), then applies the standard GK cash-digital formula: \`d2 = (ln(S/K) + (b\_adj + ½σ²)T) / (σ√T) − σ√T\`; price = \`df·Φ(ω·d2)\` where ω = +1 for call, −1 for put.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.quanto.quanto\_digital\_price` (hash `152a15a152484889`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.VarianceParams.feller\_ratio

- **claim** (`cl\_72de29836162f7cf`): VarianceParams::satisfies\_feller returns true iff 2κθ ≥ ξ², the Feller condition that ensures the CIR/Heston variance process never reaches zero. VarianceParams::feller\_ratio computes 2κθ/ξ² (returning f64::INFINITY when ξ=0, i.e. the deterministic limit), so satisfies\_feller iff feller\_ratio ≥ 1. Both methods are pure: they read only &self, perform no I/O, no allocation, and no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.VarianceParams.feller\_ratio` (hash `d9dec553c9e618b3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.VarianceParams.satisfies\_feller` (hash `0b109b9ba23f21a5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.log\_spot\_increment

- **claim** (`cl\_ebb60aac10cb48da`): log\_spot\_increment(p, v0, v1, dt, lev, z\_perp) returns the log-spot increment dlnS over \[t, t+dt\] under the stochastic-volatility model using Andersen's (2008) broadband discretisation (§3.2, γ1=γ2=½). Full-truncation clips v0,v1 to max(·,0). The exact stochastic-integral substitution ∫√v dW^v = (v1−v0−κθΔ+κ·int\_v)/ξ (int\_v=(½v0+½v1)Δ) is used when ξ\>0, else 0. Result = −½L²·int\_v  +  ρ·L·stoch\_int  +  √(1−ρ²)·L·√int\_v·Z⊥. The function is pure: reads only its six arguments, no I/O, no allocation, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.log\_spot\_increment` (hash `9b53904842cd0208`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.qe\_variance\_step

- **claim** (`cl\_89ed54b9232ff6b3`): qe\_variance\_step(p, v, dt, u) advances the Heston CIR variance v by one time step dt using Andersen's (2008) Quadratic-Exponential (QE) scheme. It computes the exact conditional mean m = θ + (v−θ)e^{−κΔ} and conditional variance s² = v·ξ²·e^{−κΔ}·(1−e^{−κΔ})/κ + θ·ξ²·(1−e^{−κΔ})²/(2κ) (Andersen 2008 eqn 17). The dispersion ratio ψ = s²/m² selects the branch: (a) ψ ≤ QE\_SWITCH: squared-Gaussian branch v' = a(b+Z)², b=√(2/ψ−1+√(2/ψ)·√(2/ψ−1)), a=m/(1+b²), Z=Φ⁻¹(u); (b) ψ \> QE\_SWITCH: exponential-with-atom branch, p\*=(ψ−1)/(ψ+1), β=(1−p\*)/m, v'=0 if u≤p\* else ln((1−p\*)/(1−u))/β. A degenerate guard returns m when ψ≤1e-12 (deterministic limit). The function is pure: reads p/v/dt/u, performs no I/O, no allocation, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.qe\_variance\_step` (hash `e23b555cdc5780d6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.step\_uniforms

- **claim** (`cl\_ddfca5f4935f1a85`): step\_uniforms(seed, stream, path, step) is a pure convenience that constructs a CounterRng with the given coordinates and draws exactly two next\_u01() values, returning them as (u0, u1). These two f64 uniforms in (0,1) are the canonical per-step random inputs for the QE + log-spot increment pair: u0 drives qe\_variance\_step (variance draw) and u1 drives the orthogonal normal via inverse\_cdf. The function owns no persistent state and has no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.stochvol.step\_uniforms` (hash `172f1da5cfdebfdc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.tarf\_price

- **claim** (`cl\_04c6c859e09ad7a6`): DELIVERABLE exotics/qmc-pathwise-wiring = OPEN (Round-2 P2/M finding; reconciled against the live graph — still genuinely open at this round). \`tarf\_price\` is a pure (side-effect-free) Monte-Carlo TARF valuation: it reads ExoticInputs/Tarf/TarfMcConfig, builds local per-fixing buffers, and returns a TarfResult with no external writes. The OPEN gap: the path generator is STILL the plain \`CounterRng\` antithetic Philox stream (\`CounterRng::new(cfg.seed, 0, pair, 0)\` + inverse\_cdf), NOT the scrambled-Sobol / Brownian-bridge QMC stack in celnet-qmc that already feeds american.rs/multiasset.rs. The path-dependent pricers (tarf/accumulator/lookback/quanto/pivot) therefore forgo the low-discrepancy variance reduction the QMC crate provides. SELF-INVALIDATING: when this pricer is rewired onto celnet-qmc (Sobol/bridge) the function body changes and this claim flips stale, signalling the deliverable has closed; a write-introducing regression also flips it.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.tarf\_price` (hash `b20a416008596f75`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.walk\_path

- **claim** (`cl\_7f7f95d608da3048`): tarf walk\_path simulates one antithetic-signed TARF path. At each fixing k the log-spot evolves as \`ln\_s += drift\_step + vol\_sqrt\_dt \* sign \* z\[k\]\`. When the per-unit gain \`\|S\_k−K\|·gain\_sign \> 0\` (favourable): if \`raw\_gain ≥ remaining\_target\` the structure redeems, settling either \`raw\_gain\` (FullGain) or \`remaining\` (CappedGain) per unit. Adverse fixings contribute \`leverage·\|S\_k−K\|·notional·df\_k\` to the bank (received). At-the-money fixings generate no cash flow and no target accrual.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.walk\_path` (hash `d32b0095665afedb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.touch.dnt\_survival

- **claim** (`cl\_9c00fc989974be0e`): dnt\_survival computes the double-no-touch survival probability (probability spot stays inside (lower, upper) until T) using a Kunitomo-Ikeda / Feller strip image series. Each term is evaluated in log-space as \`exp(log\_w + ln\|ΔΦ\|)\` (the \`weighted\` closure) to prevent Inf×0 NaN when the Girsanov weight is large and the CDF difference is tiny. The series is clamped to \[0,1\] and then capped by \`min(1−P(upper\_hit), 1−P(lower\_hit))\` — the single-wall hit probability upper bound — to keep the result consistent with the single-touch pricers in numerically stiff extreme-drift corners.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.touch.dnt\_survival` (hash `dc826363b8af8f69`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one\_touch\_price

- **claim** (`cl\_7d9883a303f7f2dc`): DELIVERABLE exotics/one-touch-at-hit-pairing-flip = LANDED (backlog tracker still lists it OPEN as the Round-2 P0; reconciled against the live graph). \`one\_touch\_price\` (-\> one\_touch\_with\_side) is a pure (side-effect-free) closed-form one-touch valuation: it reads ExoticInputs/barrier/rebate/timing and returns a price with no external writes. The Round-2 P0 (~28% high / 2x-on-far-barriers at-hit pairing flip + circular golden oracle) is FIXED-AT-ROOT: the at-hit branch is now pinned to an INDEPENDENT first-passage quadrature reference (at\_hit\_matches\_independent\_first\_passage\_quadrature, 1e-12) and guarded by a non-circular family — discounted-hit-probability sandwich (at\_hit\_sandwiched\_by\_discounted\_hit\_probability), t-\>inf perpetual-discounted-hit limit (at\_hit\_t\_infinity\_is\_perpetual\_discounted\_hit\_factor), barrier continuity (at\_hit\_continuous\_at\_the\_barrier), zero-rate collapse to deferred (zero\_discount\_rate\_collapses\_at\_hit\_to\_deferred), and barrier monotonicity. SELF-INVALIDATING: a regression that re-introduces a WRITES side effect, or any re-pairing edit that shifts these anchors, flips this claim stale, re-opening the reconciliation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one\_touch\_price` (hash `ef6ab9375633b54c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one\_touch\_with\_side

- **claim** (`cl\_6be92b08fec4cbd7`): one\_touch\_with\_side prices a one-touch (single-barrier) option under GK dynamics using the first-passage closed form. For at-hit timing the formula is \`R·\[(H/S)^{μ+λ}·Φ(a₁) + (H/S)^{μ−λ}·Φ(a₂)\]\` where \`λ = √(μ² + 2r\_d/σ²)\`, \`a₁ = base + η·λ·vsqt\` pairs with the \`(μ+λ)\` power (same side-sign η), and \`a₂ = base − η·λ·vsqt\` pairs with \`(μ−λ)\`. The PAIRING is load-bearing: the historic P0 defect (≈+28% error, T→∞ explosion) arose from flipping the pairing; the correct form has no clamp and the T→∞ limit converges monotonically. For at-expiry timing the formula is \`R·df·\[Φ(a₁) + (H/S)^{2μ}·Φ(a₂)\]\` with drift-sign arguments (probability under the spot measure).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one\_touch\_with\_side` (hash `45fd9ad8d27c0108`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-exotics.src.var\_swap.simpson\_block

- **claim** (`cl\_1a90088109d365c9`): simpson\_block integrates a single block of the var-swap log-contract integrand using composite Simpson's rule in log-moneyness u ∈ \[u\_lo, u\_hi\]. The integrand g(u) = OTM\_forward\_value(F·eᵘ) / (F·eᵘ) corresponds to the ∫ O(K)/K² dK integrand after the K=F·eᵘ change of variables. The panel count n is rounded to the nearest even integer ≥ 2, giving step size hh = span/n. Simpson weights are 1 at the endpoints and alternate 4/2 at interior nodes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.src.var\_swap.simpson\_block` (hash `b2a2c15d59b3c392`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-exotics.tests.pivot\_oracle.oracle\_price

- **claim** (`cl\_59b87a71eaad18a7`): oracle\_price is the independent Monte-Carlo reference pricer for the pivot TARF test oracle. It simulates n\_paths GBM paths (SplitMix64 PRNG, seed-reproducible) with per-fixing log-normal steps drift=(r\_dom−r\_for−½σ²)·dt, diff=σ·√dt. At each fixing k, the two-level piecewise-linear cash flow is: c = g·(S−K) on the favourable side (g·(S−pivot)≥0), else c = leverage·g·(S−K). Positive cash flow accrues until the target is hit (early redemption: FullGain settles the full fixing amount, CappedGain caps at the remaining target). Each cash flow is discounted at the per-fixing rate e^(−r\_dom·(k+1)·dt). Returns (mean bank PV, standard error, E\[redemption index\], E\[overshoot\]) across all paths.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-exotics.tests.pivot\_oracle.oracle\_price` (hash `59177da3f8fdf5fb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.LegSide.as\_byte

- **claim** (`cl\_6acb90be2cfd2d87`): LegSide::as\_byte and LegSide::from\_byte form a closed bijection over FIX tag-54 values: Buy↔SIDE\_BUY, Sell↔SIDE\_SELL. from\_byte returns None for any unrecognised byte, preventing silent misrouting of leg direction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.LegSide.as\_byte` (hash `c538ad8bb8e87fb3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.LegSide.from\_byte` (hash `dfad38269e157845`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.LegSide.sign

- **claim** (`cl\_7e9230c131f9d785`): LegSide::sign is a pure sign convention: Buy maps to +1.0 and Sell maps to -1.0. With 16 call-sites this is the authoritative multiplier used throughout the multileg pricing path to directionalise per-leg notional and Greeks.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.LegSide.sign` (hash `a88db40abda64d5d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.inputs\_for

- **claim** (`cl\_4821235706c86e5e`): inputs\_for(desc, snap) is the pure bridge from FIX-decoded option descriptor + live market snapshot to VanillaInputs: \`VanillaInputs::new(snap.spot, desc.strike, snap.vol, snap.t, snap.r\_dom, snap.r\_for)\`. It reads only its two arguments and allocates nothing. price\_leg(desc, snap, pricer) composes it with a VanillaPricer fn-pointer: \`pricer(desc.option\_type, &inputs\_for(desc, snap))\`, returning the single-leg option price as f64. These two functions are the seam between the FIX wire representation and the celnet-pricer analytics kernel.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.inputs\_for` (hash `e08ad3afe976d084`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.price\_leg` (hash `54e8936f89ef3882`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.parse\_put\_or\_call

- **claim** (`cl\_bbf15454dcbe6e80`): parse\_put\_or\_call maps FIX tag-201 byte values to OptionType: b"0"→Put, b"1"→Call, anything else→None. This is the single decode point for put/call on both single-leg (tag 201) and multileg (tag 1358) paths.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.parse\_put\_or\_call` (hash `a5dc4d1336d1870c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.put\_or\_call\_value

- **claim** (`cl\_8e27c154a4866595`): put\_or\_call\_value is the inverse encoding of parse\_put\_or\_call: OptionType::Call→CALL constant, OptionType::Put→PUT constant, returning a u32 suitable for embedding in a FIX tag-201 push.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dialect\_fx.put\_or\_call\_value` (hash `107f19901decc4ff`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.MsgType.as\_bytes

- **claim** (`cl\_20ab82edd56f68de`): MsgType::as\_bytes returns the exact FIX 4.4 wire byte sequence for each message-type discriminant: Heartbeat→b"0", TestRequest→b"1", ResendRequest→b"2", Reject→b"3", SequenceReset→b"4", Logout→b"5", Logon→b"A", QuoteRequest→b"R", Quote→b"S", MassQuote→b"i", QuoteCancel→b"Z", NewOrderSingle→b"D", NewOrderMultileg→b"AB", ExecutionReport→b"8", QuoteRequestReject→b"AG". The mapping is a closed bijection with MsgType::from\_bytes and is the sole source of tag-35 values written to the wire.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.MsgType.as\_bytes` (hash `da6754b029405474`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.MsgType.from\_bytes

- **claim** (`cl\_50c810a9ad0f59c5`): MsgType::from\_bytes is the exact inverse of MsgType::as\_bytes: it maps the same 15 byte-string literals (the FIX 4.4 tag-35 set, including b"AG"→QuoteRequestReject) to the corresponding MsgType variant and returns None for any unrecognised value. The round-trip from\_bytes(b).map(MsgType::as\_bytes) == Some(b) holds for every recognised literal.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.MsgType.from\_bytes` (hash `33f6ad51ff2f2afb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.float\_ok

- **claim** (`cl\_e319eb0c003b608a`): float\_ok validates that a byte slice encodes a valid FIX float: optional leading '-', at least one decimal digit, and at most one '.'. It returns false for empty input, non-digit/non-dot bytes, or a string with no digits. It is the sole byte-level float gate used before parse\_float in the dictionary validator.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.float\_ok` (hash `fe78209033413c6c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.required\_tags

- **claim** (`cl\_2df149c47930567a`): required\_tags returns the minimal set of FIX tag numbers that must be present for a well-formed message of each MsgType. Tag 35 (MsgType) is always included. The sets are: Logon=\[35,98,108\], ResendRequest=\[35,7,16\], SequenceReset=\[35,36\], QuoteRequest=\[35,131\], NewOrderSingle=\[35,11,54,38\], NewOrderMultileg=\[35,11,555\], ExecutionReport=\[35,37,17,150,39\]. This function is the authoritative tag-presence gate for inbound FIX validation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.required\_tags` (hash `c958238e0ab2dd6f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.type\_ok

- **claim** (`cl\_f7c484d8cea12332`): type\_ok dispatches FIX field-type validation: Int requires all bytes ASCII-digit; Float delegates to float\_ok; Char requires exactly 1 byte; Currency requires exactly 3 ASCII-alphabetic bytes; LocalMktDate requires exactly 8 ASCII-digit bytes; String and UtcTimestamp accept any non-empty slice. An empty slice always returns false regardless of type.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.type\_ok` (hash `d00354c07f0f4360`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameCursor\<'a\>.parse

- **claim** (`cl\_2fb5b46c07d90651`): FrameCursor::parse is the FIX 4.4 frame validator: given a raw &\[u8\] it (1) rejects frames shorter than 21 bytes; (2) asserts tag 8 = "FIX.4.4"; (3) reads tag 9 (BodyLength) as the declared byte-count; (4) locates the tag-10 checksum field via find\_checksum\_field, computes actual\_body\_len = cs\_field\_start − body\_start and rejects if actual ≠ declared; (5) recomputes checksum(&raw\[..cs\_field\_start\]) mod-256 and rejects on mismatch; (6) scans the body for tag 35 (MsgType) and rejects if absent. All errors are typed FrameError variants (TooShort, MissingBeginString, UnsupportedBeginString, MissingBodyLength, BadBodyLength, BodyLengthMismatch{declared,actual}, MissingCheckSum, BadCheckSum, CheckSumMismatch{declared,computed}, MissingMsgType). The function reads only its argument and allocates nothing (zero-copy cursor over the input slice).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameCursor\<'a\>.parse` (hash `834b2790e374819a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameEncoder.finish

- **claim** (`cl\_4e6ebae43c9b6cc8`): FrameEncoder::finish serialises a complete FIX 4.4 frame from the accumulated body: it prepends \`8=FIX.4.4\<SOH\>9=\<bodylen\>\<SOH\>\`, appends the body, then computes checksum(&out) over all preceding bytes and appends \`10=\<3-digit-padded checksum\>\<SOH\>\`. The result is a freshly allocated Vec\<u8\> with pre-sized capacity (body.len() + 24). finish() only reads self and calls no I/O — its output is the unique well-formed wire frame corresponding to the encoder's accumulated tags.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameEncoder.finish` (hash `f840a83230017da8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-fix.src.framing.checksum

- **claim** (`cl\_bc41c6016ec8b6b6`): SAFETY/WIRE — FIX session-layer frame integrity is the standard FIX BodyLength/CheckSum (tag 10) modulo-256 sum: checksum folds every byte of the message-up-to-and-including the SOH before tag 10 with wrapping u32 addition and returns (acc & 0xFF) as u8 — the canonical FIX checksum that is always rendered as a 3-digit field and validated on inbound frames (rejects\_corrupted\_checksum, checksum\_is\_mod\_256). A counterparty frame whose recomputed mod-256 checksum does not match the transmitted tag-10 value is rejected at framing, so a corrupted/truncated FIX message never reaches order/quote handling. Pure: it reads only the input byte slice and returns the u8 checksum, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.framing.checksum` (hash `8eb7bdacfe56c638`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-fix.src.framing.parse\_uint

- **claim** (`cl\_8d94a86c92b5474f`): parse\_uint converts a non-empty ASCII decimal byte slice to u64, returning None on an empty slice, any non-digit byte, or integer overflow. Overflow is detected via checked\_mul(10)?.checked\_add(digit)? — no panic, no silent truncation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-fix.src.framing.parse\_uint` (hash `a4b18c217efe3a24`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.basket\_vector

- **claim** (`cl\_700dd2a50e0015f3`): basket\_vector is a pure builder that assembles a GoldenVector for a basket option family from its ingredients. It serialises legs as a JSON array with pair/spot/vol/r\_for/weight per leg, flattens the correlation matrix to a 1-D array, sets the family to 'basket', price\_std\_error to Some(est.std\_error) marking it as an MC vector, and attaches oracle label 'code-disjoint splitmix64 Cholesky-correlated GBM basket Monte-Carlo'. The mc\_paths/replications/steps/seed fields in terms are set to 0 as placeholders; the actual MC budget is in the oracle, not the stored terms.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.basket\_vector` (hash `3ec903376af4ef8e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.cp\_token

- **claim** (`cl\_8cf31e5b4e742059`): cp\_token is a pure exhaustive bijection from Cp to a static ASCII string token: Cp::Call =\> "CALL", Cp::Put =\> "PUT". It is const-evaluable and allocation-free (returns &'static str). No default branch exists; adding a new Cp variant without updating this function is a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.cp\_token` (hash `2bf3e1194b02c9b8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.gen\_vanilla

- **claim** (`cl\_ee4ac5cb22cb72a7`): gen\_vanilla selects 8 representative rows from the QuantLib vanilla\_gk.csv golden table at fixed indices \[120, 480, 905, 1330, 1755, 2180, 2605, 3050\] (using idx % len to guard against length changes), maps each to a GoldenVector in the \`vanilla\` family with all six Greeks (delta\_spot, gamma, vega, theta, rho\_dom, rho\_for), oracle tag 'quantlib-1.42.1 vanilla\_gk.csv row {idx}', and tightens the tolerance to rel=1e-7/abs=1e-9 (reflecting sub-ULP agreement with the production GK path).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.gen\_vanilla` (hash `7a11c1f0216449c8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.side\_sign

- **claim** (`cl\_de19661c2fb2336e`): side\_sign (both in gen\_vectors and vectors\_selfcheck) maps the BUY/SELL trade-side token to the signed PV multiplier: BUY → +1.0, SELL → −1.0. This is the convention used by strategy, fx\_forward, and ndf oracle computations where the linear PV is sign × \|payoff\|.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.bin.gen\_vectors.side\_sign` (hash `cc699b4e75fe67c7`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.side\_sign` (hash `59d3bbf5febe64c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.src.csv.CsvError.fmt

- **claim** (`cl\_fefc4a8c025b5bc8`): CsvError::fmt provides structured, position-aware messages for all five CSV parsing failure modes: Io wraps the inner error ('I/O error reading CSV: {e}'); MissingHeader is the static string 'CSV has no header row'; RaggedRow embeds row index, expected, and found column counts ('CSV row {row} has {found} columns, expected {expected}'); UnknownColumn names the offending column ('unknown CSV column: {c}'); Parse embeds row, column, and the raw value debug-formatted ('CSV row {row} column {column}: cannot parse {value:?}'). Exhaustive over the five variants; allocation/IO-free formatting.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.csv.CsvError.fmt` (hash `c194d04f1ca1dbd4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.Cp.parse

- **claim** (`cl\_cab9cc09ad05f7ae`): Cp::parse deserialises the canonical all-caps string tokens 'CALL'/'PUT' into the oracle's Cp enum. Any other input panics, enforcing a strict closed vocabulary at the oracle boundary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.Cp.parse` (hash `2a55e7884f979f9b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.Cp.sign

- **claim** (`cl\_c13f7990d33fd70d`): Cp::sign returns the put-call sign multiplier used throughout the oracle layer: Call → +1.0, Put → −1.0. This is the φ factor in the Garman-Kohlhagen / Black-76 formula d·φ·(F·Φ(φ·d₁) − K·Φ(φ·d₂)).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.Cp.sign` (hash `e5bfc5d4a3dfd7cc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76\_price

- **claim** (`cl\_c85aec296a96836c`): SAFETY/ORACLE — black76\_price is the independent golden reference for futures-style (forward-measure) options (the Black-76 closed form), NOT the production engine's own pricer: for t\<=0 it returns the discounted intrinsic exp(-r·t)·max(sign·(F-K),0); otherwise the standard Black-76 with vsqt=vol·√t, d1=(ln(F/K)+½σ²t)/vsqt, d2=d1-vsqt, discount df=exp(-r·t), Call=df·(F·N(d1)-K·N(d2)) and the Put put-call complement. It is pure and deterministic over its 6 scalar inputs (libm transcendentals only, no I/O/mutation/allocation), so it is a trustworthy can-disagree oracle gating commodity / listed-future-option parity against the engine. Self-invalidates if the closed form drifts.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76\_price` (hash `fbf56720552eda3d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76\_undiscounted\_price

- **claim** (`cl\_3452f577e4510dbf`): Golden-oracle gate (Black-76 commodity/forward reference): black76\_undiscounted\_price is a pure closed-form function of (cp, forward, strike, vol, t) using only libm math and the pure xerf\_norm\_cdf, with the t\<=0 intrinsic-payoff branch. No writes, no I/O, deterministic — the QuantLib-pinned reference price the parity suite gates production engines against must be a pure function of its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76\_undiscounted\_price` (hash `c6f4e1af999c70ba`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cholesky

- **claim** (`cl\_121d1d222e5f12cc`): SAFETY/ORACLE — cholesky is the correlation-decomposition primitive underpinning the independent basket / multi-asset Monte-Carlo golden references: it computes the lower-triangular factor L of a symmetric positive-definite correlation matrix via the classic doubly-indexed recurrence (sum a\[i\]\[j\] - Σ\_k l\[i\]\[k\]·l\[j\]\[k\], diagonal = √sum, off-diagonal = sum/l\[j\]\[j\]). It is pure and deterministic over its input matrix (no I/O, no mutation of the argument, no RNG), and it is fail-closed: a non-positive-definite matrix trips assert!(sum \> 0.0) and panics rather than silently emitting a NaN/garbage factor that would corrupt every correlated-path draw. This makes the oracle's correlated scenarios reproducible and trustworthy as a can-disagree reference. Self-invalidates if the PD guard is removed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cholesky` (hash `3bc7710e06080a48`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto\_inverse\_price

- **claim** (`cl\_996daa922b717523`): Golden-oracle gate (inverse/coin-margined crypto reference): crypto\_inverse\_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — carry b=r-funding, forward, the (K/F)e^{sigma^2 t} amplitude correction for the 1/S\_T inverse payoff, libm math and pure xerf\_norm\_cdf only. No writes, no I/O, deterministic; the reference price gating the inverse crypto engine must depend solely on its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto\_inverse\_price` (hash `166c41af6b52707f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto\_linear\_price

- **claim** (`cl\_430ace89e9606b33`): Golden-oracle gate (linear/GK-funding crypto reference): crypto\_linear\_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — applies carry b=r-funding to the forward then delegates to the pure black76\_price. No writes, no I/O, deterministic; the linear crypto reference price the parity suite uses is a pure function of its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto\_linear\_price` (hash `79275a81bcbb455e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.det3

- **claim** (`cl\_c94c527dd04a7e66`): Golden-oracle numerical kernel: det3 computes the 3x3 determinant by cofactor expansion over a borrowed \[\[f64;3\];3\] with no writes, no I/O — a pure function of the matrix. Determinism of this kernel underpins the correctness of the oracle routines (e.g. correlation/quanto projections) that the parity gate trusts as ground truth.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.det3` (hash `c1f23e6a1933be60`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.equity\_bsm\_price

- **claim** (`cl\_7e5a2b865ed8dfb7`): SAFETY/ORACLE — equity\_bsm\_price is the independent golden reference for generalized Black-Scholes-Merton equity vanillas (carry b = r - q - repo), NOT the engine's own pricer: for t\<=0 it returns intrinsic max(sign·(S-K),0); otherwise d1=(ln(S/K)+(b+½σ²)t)/(σ√t), d2=d1-σ√t, with cost-of-carry-discounted spot s\_disc=S·exp((b-r)t) and rate-discounted strike k\_disc=K·exp(-r·t), Call=s\_disc·N(d1)-k\_disc·N(d2) and the Put complement. It is pure and deterministic over its 8 scalar inputs (libm only, no I/O/mutation/allocation), serving as a can-disagree oracle gating equity-vanilla parity (dividend yield + repo carry) against the engine. Self-invalidates if the carry decomposition drifts.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.equity\_bsm\_price` (hash `624d37274e134fe3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.floating\_lookback\_price

- **claim** (`cl\_a745d175dfce8d67`): \`floating\_lookback\_price\` computes the closed-form price of a floating-strike lookback option (call: S\_T − S\_min; put: S\_max − S\_T) using the Goldman–Sosin–Gatto formula. With b = r\_dom − r\_for and two\_b\_over\_sig2 = 2b/σ², the call is \`S·df\_for·N(a1) − S·df\_dom·N(a2) + S·df\_dom·(σ²/2b)·\[N(−a1+two\_b\_over\_sig2·σ√T) − e^{bT}·N(−a1)\]\` and the put is the symmetric complement.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.floating\_lookback\_price` (hash `a2702326ffc025e2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.forward\_start\_price

- **claim** (`cl\_17d04c8f4d88aa7b`): \`forward\_start\_price\` prices a forward-start option as \`e^{−r\_for·reset} · spot · V\_unit\`, where V\_unit is a unit-spot vanilla (gk\_price with spot=1, strike=moneyness, maturity=expiry−reset). If expiry ≤ reset the option has already started and the payoff collapses to the intrinsic max(cp·(1−moneyness), 0). The formula is exact for a GBM model with flat vol.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.forward\_start\_price` (hash `486bddeb39ab97df`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_forward\_pv

- **claim** (`cl\_8e5ebe3e13c95663`): Golden-oracle gate (FX forward PV reference): fx\_forward\_pv is a pure closed-form function of (side, spot, strike, notional, t, r\_dom, r\_for) — side\*notional\*(spot\*e^{-r\_for t} - strike\*e^{-r\_dom t}), dual-discounted, no writes, no I/O, deterministic. The FX-forward reference PV the parity suite pins must depend solely on its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_forward\_pv` (hash `187a2834cf590418`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_swap\_points

- **claim** (`cl\_cd2462c3aff76c69`): Golden-oracle gate (FX swap points reference): fx\_swap\_points is a pure closed-form function of (spot, near\_t, far\_t, r\_dom, r\_for) — spot\*(e^{b\*far\_t}-e^{b\*near\_t}) with carry b=r\_dom-r\_for, no writes, no I/O, deterministic. The FX-swap points reference the parity suite pins must depend solely on its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_swap\_points` (hash `23f9c25cf4df3229`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_swap\_pv

- **claim** (`cl\_db7487c29f25af6c`): \`fx\_swap\_pv\` prices an FX swap as the algebraic sum of two offsetting forward legs: a near leg with side \`near\_side\` at \`near\_t\` and a far leg with side \`−near\_side\` at \`far\_t\`, both evaluated by \`fx\_forward\_pv\`. This makes the swap PV the difference of two discounted FX-forward residuals, consistent with the convention that the near and far legs carry opposite sign on the domestic-currency notional.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx\_swap\_pv` (hash `c7e2f68692e8ee94`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.gk\_price

- **claim** (`cl\_d2298180027fca5b`): SAFETY/ORACLE — gk\_price is the independent golden reference (the Garman-Kohlhagen two-rate FX vanilla closed form) against which the production engine is validated, NOT the engine's own pricer: for t\>0 it computes d1=(ln(S/K)+(r\_dom-r\_for+0.5\*vol^2)\*t)/(vol\*sqrt(t)), d2=d1-vol\*sqrt(t), df\_dom=e^{-r\_dom\*t}, df\_for=e^{-r\_for\*t}, and returns S\*df\_for\*N(d1)-K\*df\_dom\*N(d2) for a Call (put by symmetry); for t\<=0 it returns the discounted intrinsic max(sign\*(S-K),0). It deliberately re-derives the price from first principles with its own norm\_cdf so a parity test (e.g. vanilla\_price\_and\_greeks\_match\_quantlib, also pinned to published QuantLib numbers) can disagree with the engine — the anti-circular-oracle property: numerical correctness is checked against this reference, never merely asserted plausible. Pure: it reads only its scalar args and returns the f64 price, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.gk\_price` (hash `05d9a81c58c49f99`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.hedge\_smile\_overlay\_cost

- **claim** (`cl\_359b07923050b64b`): \`hedge\_smile\_overlay\_cost\` computes the smile-overlay hedge cost for a target vega/vanna/volga exposure vector using Cramer's rule on the 3×3 pillar Greeks matrix. It assembles a matrix A where A\[i\]\[j\] is the j-th Greek (vega, vanna, volga) of the i-th pillar vanilla, inverts it via \`det3\`, solves Aw = g for weights w, then computes cost = Σ w\_i · (pillar\_price\_at\_pillar\_vol − pillar\_price\_at\_flat\_vol). Returns \`HedgeOverlayOracleError::SingularHedgeMatrix\` if \|det\| \< 1e-12.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.hedge\_smile\_overlay\_cost` (hash `5742ca5755680881`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.ndf\_pv

- **claim** (`cl\_c29a3098c89319ac`): Golden-oracle gate (NDF PV reference): ndf\_pv is a pure closed-form function of (side, spot, strike, notional, t, r\_dom, r\_for) — a non-deliverable forward prices as the deliverable forward, delegating to the pure fx\_forward\_pv. No writes, no I/O, deterministic; the NDF reference PV the parity suite pins is a pure function of its inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.ndf\_pv` (hash `9e2764e739302994`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto\_digital\_price

- **claim** (`cl\_a3116c1499229390`): \`quanto\_digital\_price\` prices a FX-quanto cash-or-nothing digital using the same quanto drift correction as \`quanto\_vanilla\_price\`: it applies r\_for\_adj = r\_for + ρ·σ·σ\_conv, computes d2 directly, and returns df·N(±d2). The formula is the exact BSM limit for a cash digital with quanto-adjusted drift.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto\_digital\_price` (hash `2b069ffcefb73e36`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto\_vanilla\_price

- **claim** (`cl\_875abc909c645c40`): \`quanto\_vanilla\_price\` prices a FX-quanto vanilla by replacing r\_for with r\_for − (−ρ·σ\_asset·σ\_fx), i.e. the quanto carry adjustment is −ρ·σ·σ\_conv, then delegating to \`gk\_price\`. This is the standard quanto drift correction: the asset grows at r\_dom − (r\_for − ρσσ\_conv) under the domestic risk-neutral measure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto\_vanilla\_price` (hash `e2f3e1259f692ad4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-golden.src.oracle.tarf\_bank\_pv\_mc

- **claim** (`cl\_b298973a86df579f`): tarf\_bank\_pv\_mc prices a Target-Accrual Redemption Forward (TARF) from the bank's perspective using antithetic-variate SplitMix64 Monte-Carlo under GBM: drift = (r\_dom − r\_for − ½σ²)dt, diffusion = σ√dt per fixing step. On each favourable fixing the function accumulates the intrinsic gain against a running total; when the total reaches \`target\` the redemption leg settles either the full raw intrinsic (FullGain) or only the remaining cap (CappedGain) and the path terminates immediately. Each unfavourable fixing contributes \`leverage × \|intrinsic\|\` to the bank PV with sign reversed. The antithetic pair (walk(z,+1), walk(z,−1)) is averaged before pushing to the Welford accumulator, so variance is halved relative to a plain estimator. Returns McEstimate{price: mean, std\_error}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.tarf\_bank\_pv\_mc` (hash `99c33d1efb9ce07b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_barrier\_type

- **claim** (`cl\_a8be64e4df4408b0`): parse\_barrier\_type maps CSV barrier-type tokens ('DOWN\_OUT', 'DOWN\_IN', 'UP\_OUT', 'UP\_IN') to the BarrierType enum, returning CsvError::Parse{row:0, column:"barrier\_type"} on any unknown token. Row 0 is a sentinel; callers replace it with the actual row index after the parse call.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_barrier\_type` (hash `0d9dff89915f5500`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_digital\_style

- **claim** (`cl\_c5620e62d35dbd37`): parse\_digital\_style maps the CSV settlement column ('CASH' → CashOrNothing, 'ASSET' → AssetOrNothing). The CSV uses abbreviated tokens that are distinct from the wire tokens ('CASH\_OR\_NOTHING', 'ASSET\_OR\_NOTHING'); the table module owns the CSV vocabulary boundary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_digital\_style` (hash `8a48b85d614006af`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_double\_barrier\_kind

- **claim** (`cl\_f3a8c341e0d079b2`): parse\_double\_barrier\_kind maps CSV double-barrier kind tokens ('KO' → KnockOut, 'KI' → KnockIn). The abbreviated CSV tokens differ from the API tokens ('KNOCK\_OUT', 'KNOCK\_IN'); the table module owns the CSV vocabulary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_double\_barrier\_kind` (hash `a94ead058cde7b8f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_flag

- **claim** (`cl\_d81ff15dd9147192`): parse\_flag parses a boolean flag column from its CSV string representation: "0" maps to Ok(false), "1" maps to Ok(true), and any other value returns Err(CsvError::Parse) with the column name and offending value preserved. The row field in the error is always 0 (callers are expected to annotate the row externally).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_flag` (hash `4c32566e828c8da4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_option\_type

- **claim** (`cl\_fb25c7555e56a28a`): parse\_option\_type parses the canonical option-type CSV token: "CALL" maps to Ok(OptionType::Call), "PUT" maps to Ok(OptionType::Put), and any other string yields Err(CsvError::Parse) with column fixed to "option\_type". This is the single authoritative token decoder used by all five table-loading paths in celnet-golden.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_option\_type` (hash `c6d7a4e9f6234ee7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_touch\_kind

- **claim** (`cl\_df8f8914a80fd23e`): parse\_touch\_kind maps CSV touch-kind tokens ('ONE\_TOUCH', 'NO\_TOUCH', 'DNT', 'DOUBLE\_TOUCH') to the TouchKind enum. Note the CSV uses 'DNT' (not 'DOUBLE\_NO\_TOUCH') and 'DOUBLE\_TOUCH' (not 'DOUBLE\_ONE\_TOUCH') — the mismatch with the wire protocol tokens is intentional; the table module owns the CSV vocabulary, not the API vocabulary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.table.parse\_touch\_kind` (hash `bf1348f8cc81a5db`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.src.vectors.GoldenVector.term\_opt\_f64

- **claim** (`cl\_8b0f17092f2b881e`): GoldenVector::term\_opt\_f64 extracts an optional numeric term from the JSON terms map: a missing key or an explicit JSON null returns None; a present non-null value is coerced to f64 (panicking with the vector id and key name on type mismatch). This prevents silent numeric defaults when an MC parameter is absent from a closed-form vector's terms bag.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.vectors.GoldenVector.term\_opt\_f64` (hash `e4fa5833f0d47e10`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-golden.src.vectors.VectorError.fmt

- **claim** (`cl\_681c4b17cbf9e412`): VectorError::fmt always includes the file path (via path.display()) in its message for both variants — Io prefixes 'reading {path}: {source}' and Parse prefixes 'parsing {path}: {source}'. This guarantees that every golden-vector load failure can be located by path without inspecting the inner source error alone.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.src.vectors.VectorError.fmt` (hash `f962de7d1cd324bf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.barrier\_grid.kind\_of

- **claim** (`cl\_9edc890498ccee62`): kind\_of (barrier\_grid) is a pure, exhaustive bijection from the four BarrierType enum variants (DownOut, DownIn, UpOut, UpIn) to the corresponding BarrierKind struct, setting the \`up\` flag (false for Down, true for Up) and \`style\` (KnockOut/KnockIn), plus forwarding the option type. Every variant is covered; the function cannot return without a match.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.barrier\_grid.kind\_of` (hash `1ee9e5ead4d83cd6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.digital\_grid.kind\_of

- **claim** (`cl\_8197a1339d434794`): kind\_of (digital\_grid) maps DigitalSettlement to DigitalKind by dispatching CashOrNothing to DigitalKind::cash(option\_type) and AssetOrNothing to DigitalKind::asset(option\_type). The mapping is exhaustive and pure; it is the sole bridge between the two-variant oracle settlement enum and the exotics crate's DigitalKind type.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.digital\_grid.kind\_of` (hash `972f7ebd18bd798a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.double\_barrier\_grid.celnet\_double\_barrier

- **claim** (`cl\_e406f8181e0aca5f`): celnet\_double\_barrier prices a double-barrier option by computing the knock-out value via double\_knock\_out\_price and deriving knock-in via vanilla−KO put-call parity: KI = vanilla\_price(option\_type, inputs) − KO. This ensures KI+KO == vanilla for every parameter set, grounding the two complementary double-barrier branches in a single analytical identity.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.double\_barrier\_grid.celnet\_double\_barrier` (hash `4ab99e6faee32581`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.touch\_grid.celnet\_touch

- **claim** (`cl\_96818b6148bc1909`): celnet\_touch is the golden-vector pricing dispatcher for all four touch/no-touch flavours. OneTouch and NoTouch each require exactly one barrier (panics with a descriptive message if absent); Dnt and DoubleTouch each require lower and upper corridor bounds with no single barrier. The function constructs inputs with spot==strike (forward-at-the-money convention for touch instruments) before delegating to the matching entry-point.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.touch\_grid.celnet\_touch` (hash `0b2fb5309b1dea80`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_basket\_mc

- **claim** (`cl\_0393eb822435ad59`): redrive\_basket\_mc re-runs the basket Monte-Carlo pricer from a GoldenVector's JSON term bag: it deserialises each leg's spot/vol/r\_for/weight from the \`legs\` array, extracts the flat correlation matrix from the \`correlations\` array (row-major, n×n), maps the \`kind\` token to BasketKind, and invokes oracle::basket\_mc with the fixed seed 0x5E1F\_0007 and SELFCHECK\_MC\_PATHS paths. The deterministic seed guarantees byte-identical results across re-drives.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_basket\_mc` (hash `016ce32e9cae201f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_digital

- **claim** (`cl\_823e5aede8044212`): redrive\_digital resolves the expected price for a digital golden vector by locating the matching row in the frozen QuantLib digital oracle table using exact field equality (via eqf). The lookup is restricted to CashOrNothing settlement; it panics with the vector id if no matching oracle row is found, making a missing oracle row a hard test failure rather than a silent default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_digital` (hash `abd1b46d0e0722ea`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_double\_barrier

- **claim** (`cl\_f83d8e373a1afb11`): redrive\_double\_barrier resolves the expected price for a double-barrier golden vector by finding its matching row in the frozen QuantLib oracle restricted to DoubleBarrierKind::KnockOut (KnockIn vectors derive their expected price via the KO+KI=vanilla identity tested separately). The lookup matches on kind, option\_type, spot, strike, lower, upper, vol, t, r\_dom, r\_for using eqf; absence panics with the vector id.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_double\_barrier` (hash `7b1ff29f96799f5b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_mc

- **claim** (`cl\_3ef2b4b6cd35ef2e`): redrive\_mc dispatches a GoldenVector to the correct independent MC oracle for its family: asian\_option → asian\_arithmetic\_mc, tarf → tarf\_bank\_pv\_mc, pivot → pivot\_tra\_bank\_pv\_mc (with a mandatory cross-check against tarf\_bank\_pv\_mc when pivot == strike, verified within 6 combined standard errors), accumulator → accumulator\_client\_pv\_mc, lookback → lookback\_discrete\_mc, cliquet → cliquet\_clamped\_mc, window\_barrier → window\_barrier\_mc, basket → redrive\_basket\_mc. Each call uses a SELFCHECK\_MC\_PAIRS antithetic budget and a family-unique seed. An unhandled family panics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_mc` (hash `44e62deeb58b40c7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_single\_barrier

- **claim** (`cl\_4c2dce218b0aa6a1`): redrive\_single\_barrier looks up the QuantLib barrier row that exactly matches a GoldenVector's market and contract terms (barrier\_type, option\_type, spot, strike, barrier, vol, t, r\_dom, r\_for) using field-level float equality via \`eqf\`, and returns its CSV price as the oracle value. This ensures the selfcheck oracle is the verbatim QuantLib analytic value — not a re-computation — avoiding double-error cancellation in the regression.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-golden.tests.vectors\_selfcheck.redrive\_single\_barrier` (hash `62f1eb1014b2df2b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.as\_normal.as\_inverse\_brackets\_libm\_inverse

- **claim** (`cl\_a4c6e79fafd4b633`): inv\_norm\_cdf(p: f32) -\> f32 (WGSL, path.wgsl) is the inverse normal CDF used in the Sobol QMC path generator. It implements the Acklam rational-approximation with three regions (tail p \< 0.02425, central, upper tail), followed by one Halley-step refinement: \`x = x - (norm\_cdf(x)-p)/norm\_pdf(x) / (1 + 0.5\*x\*(norm\_cdf(x)-p)/norm\_pdf(x))\`. The function is pure (no writes, no allocation). It is the GPU-side counterpart of the CPU Acklam implementation in celnet-qmc, and the two are kept bit-comparable within f32 precision by the test \`as\_inverse\_brackets\_libm\_inverse\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.as\_normal.as\_inverse\_brackets\_libm\_inverse` (hash `693a644adb6e992f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.path.inv\_norm\_cdf` (hash `da3f8c37ce18a8fa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as\_erf\_oracle\_brackets\_golden

- **claim** (`cl\_c9c1c238f31b68fc`): erf\_as(x: f32) -\> f32 (WGSL, batch.wgsl) is the GPU f32 error-function, implementing Abramowitz & Stegun formula 7.1.26: \`t = 1/(1+p\*\|x\|)\`, Horner-evaluated 5-term polynomial, \`y = 1 - poly\*exp(-x^2)\`, reflected for x\<0 via \`s = sign(x)\`. The coefficients are bit-identical to the CPU oracle \`erf\_as\_oracle\` in batch.rs (verified by \`as\_erf\_oracle\_brackets\_golden\`). norm\_cdf\_f32 wraps it as \`0.5\*(1 + erf\_as(x \* INV\_SQRT\_2))\`. Both functions are pure (no writes).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as\_erf\_oracle\_brackets\_golden` (hash `a47f2dd3feef0bed`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.erf\_as` (hash `1fdd259c335fff02`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.norm\_cdf\_f32` (hash `07843579b5c8c768`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as\_erf\_price\_bound

- **claim** (`cl\_a97acfd71f8e5756`): PILLAR (GUIDE.md guardrails 5 + 6 + 7 — numerical code is VALIDATED against a derived bound, never merely asserted plausible; the GPU scale path uses open methods and Metal lacks f64 so the f32 path is rigorously bounded). \`as\_erf\_price\_bound(b)\` is the pure, closed-form per-instrument absolute-error envelope for the f32/\`as\`-erf batch GPU kernel: it returns \`(s\_disc + k\_disc) \* 0.5 \* AS\_ERF\_MAX\_ABS\_ERR\` with \`AS\_ERF\_MAX\_ABS\_ERR = 1.5e-7\`, where the discounted-spot and discounted-strike legs scale the worst-case erf approximation error into a price tolerance. The many-instrument GPU batch path (GUIDE.md guardrail 6 — IB-sized portfolios / high-throughput scale-out) is reconciled three-way against the exact f64 oracle WITHIN this analytic bound, so the precision claim is proven rather than assumed. The function is side-effect-free: it reads only the borrowed BatchInstrument and computes a scalar via libm-backed exp, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as\_erf\_price\_bound` (hash `0f6462b0bc7d793c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.batch.batch\_reconciles\_three\_way

- **claim** (`cl\_4a0a30aea6ff05fd`): gk\_price\_as\_oracle(b: &BatchInstrument) -\> f64 is the CPU closed-form GBM (Garman-Kohlhagen) pricer used as the validation oracle for the GPU batch path: d1 = (ln(spot/strike) + (r\_dom − r\_for + 0.5σ²)T) / (σ√T), d2 = d1 − σ√T, price = spot\*exp(−r\_for\*T)\*N(sign\*d1) − strike\*exp(−r\_dom\*T)\*N(sign\*d2) for call (sign=+1) or put (sign=−1), clamped to zero. All transcendentals route through celnet\_core::math (libm-backed, deterministic). cpu\_batch\_with\_as\_erf maps this oracle over a &\[BatchInstrument\] slice. The test \`batch\_reconciles\_three\_way\` asserts GPU batch, CPU path Monte Carlo, and this oracle agree within \`as\_erf\_price\_bound\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.batch\_reconciles\_three\_way` (hash `62a9827b9fb3314f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.cpu\_batch\_with\_as\_erf` (hash `4be3259a0fe967a0`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.gk\_price\_as\_oracle` (hash `1ccad39e7b3d46c9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.batch.realistic\_batch

- **claim** (`cl\_08a13ed5a6d9ff9a`): realistic\_batch constructs a deterministic, fully reproducible cross-product test fixture of 4 spots × 7 moneyness ratios × 4 vols × 4 expiries × 2 rate pairs × 2 sides (call+put) = 1,792 BatchInstrument entries. Strike is derived as K = S \* moneyness. The fixture covers a wide range of practical FX option parameters (2-week to 2.5-year expiry, 6%-28% vol, near and far from the money) and is used by batch pricer tests to verify GPU/CPU parity and statistical accuracy without any randomness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.realistic\_batch` (hash `521be0f193643b3a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.CpuBackend.label

- **claim** (`cl\_9c66829ec2823894`): ADR (GPU abstraction = wgpu baseline + CPU-SIMD fallback, CUDA optional). \`CpuBackend::label\` returns the constant identity "cpu-f64": a pure accessor (reads nothing, mutates nothing) that names the f64 CPU oracle within the shared \`PricingBackend\` trait. That trait is the portability seam — \`CpuBackend\` and the wgpu \`GpuBackend\` both implement the same simulate\_paths/reduce\_payoff/price\_vanilla contract, with Philox path index i fixed across backends so results reconcile (modulo the f32/f64 element type) — which is exactly what makes the GPU backend swappable behind one interface. DECISION/RATIONALE: the GPU strategy is wgpu (Metal/Vulkan/DX12) as the open, permissively-licensed baseline, with the CPU-SIMD path as the always-available f64 oracle and reconciliation reference, and an optional CUDA backend behind the same trait — chosen over a CubeCL/CUDA-first design because wgpu keeps the runtime dependency-set fully open-source and portable across the M4/Metal dev box and Linux/Vulkan CI. The label encodes the key portability caveat the design must respect: the CPU oracle is f64 while the wgpu/Metal path is f32 (Metal lacks f64), so cross-backend agreement is asserted to the f32 tolerance, never bit-identity. (Guardrail: no commercial products; open GPU stack with wgpu first-class.)
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.CpuBackend.label` (hash `d1b8e23ba16c2b4c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.pairwise\_sum

- **claim** (`cl\_8b4f203cb6f72b51`): pairwise\_sum(xs: &\[f64\]) -\> f64 is the numerically-stable recursive summation used for all Monte Carlo reduction readbacks. For slices of length ≤ 64 it uses a sequential accumulator (cache-friendly base case); for longer slices it recursively splits at the midpoint. This avoids catastrophic cancellation in large path counts compared to a naive left-fold. It is pure (reads only xs, no allocation of its own, self-recursive). The test \`pairwise\_sum\_is\_order\_stable\` confirms the result is independent of call-site ordering.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.pairwise\_sum` (hash `6b4e609b46d2654d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.pairwise\_sum\_is\_order\_stable` (hash `17bcf91c91020bf5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.greeks.sobol\_coord

- **claim** (`cl\_b36068fb5241adbf`): sobol\_coord (greeks.wgsl) computes the i-th point of a 1-dimensional Sobol sequence using Gray-code enumeration. The algorithm: (1) compute Gray code g = i XOR (i \>\> 1); (2) for each set bit k in g, XOR the k-th direction number from the dir\_nums uniform buffer into the accumulator. The result is the raw u32 Sobol sample for index i in dimension 0. This is the standard Gray-code Sobol recurrence — O(32) XOR operations, no multiplication, fully branchless except for the early-exit when bits is exhausted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.greeks.sobol\_coord` (hash `f7d1c0e52f99a679`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.path.shared\_qmc\_tables

- **claim** (`cl\_abf04a2fd59a1f2d`): shared\_qmc\_tables builds the two GPU-upload tables needed for multi-step QMC path generation: (1) a flat array of Sobol direction numbers for all m dimensions, concatenated in dimension order (m \* 32 u32 values); (2) the Brownian Bridge weight matrix A flattened row-major as f32 (m\*m values). Both tables are derived exclusively from the celnet\_qmc public API (SobolSequence::direction\_numbers, BrownianBridge::weight\_matrix) so the GPU shader and the CPU oracle use identical mathematical tables by construction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.path.shared\_qmc\_tables` (hash `2bd5fcb59ac5a925`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.path.sobol\_coord

- **claim** (`cl\_6c650d3c95bafacd`): sobol\_coord(i: u32, j: u32) -\> u32 (WGSL, path.wgsl) computes the j-th coordinate of the i-th Sobol' quasi-random point via Gray-code enumeration: g = i ^ (i \>\> 1), then XORs the direction numbers dir\_nums\[j\*32 + k\] for every set bit k of g. The docstring states it is bit-identical to \`celnet\_qmc::SobolSequence::point\_u32\`, making it the GPU mirror of the CPU Sobol engine. It is pure (no writes).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.path.sobol\_coord` (hash `4ec248a177554f85`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.counter\_raw

- **claim** (`cl\_6306a2c63b1dbfaf`): counter\_raw (WGSL, both shader.wgsl and scenario.wgsl) implements a single output lane of the Philox counter-based PRNG. It constructs a 4-wide counter vector \`(path, step, dim \>\> 2, PHILOX\_DOMAIN\_TAG)\` and a 2-wide key \`(params.seed\_lo, params.seed\_hi)\`, computes a full Philox block via \`counter\_block\`, then selects the lane \`dim & 3\` by explicit comparison (no dynamic indexing) to return the raw u32. The identity is: \`counter\_raw(path, step, dim) == philox4x32(counter=(path,step,dim\>\>2,TAG), key=(seed\_lo,seed\_hi))\[dim & 3\]\`. The domain tag ensures PRNG outputs for different Monte-Carlo use-sites are independent even when (path,step,dim) indices collide.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.counter\_raw` (hash `fd6bb177fc1ae06f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.shader.counter\_raw` (hash `590c118d3a7811fa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-gpu.src.shader.mc\_vanilla

- **claim** (`cl\_edfd050307f506ad`): mc\_vanilla is a WGSL compute shader (@workgroup\_size(256)) implementing Black-Scholes Monte Carlo for a vanilla option. Each thread independently evaluates one path: it draws a standard normal via counter\_normal(path, 0, 0) (a counter-based quasi-random draw), computes log-spot as ln\_spot + mu\_term + vol\_sqrt\_t \* z, exponentiates to get S\_T, then evaluates the payoff as max(sign \* (S\_T - strike), 0). A tree reduction over shared memory (scratch\_sum / scratch\_sum\_sq) accumulates per-workgroup sums and sums-of-squares into group\_sums\[2\*wid\] and group\_sums\[2\*wid+1\] respectively, which the host reads to compute mean and variance. The guard \`if (path \< params.paths)\` zeroes out-of-bounds lanes before reduction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-gpu.src.shader.mc\_vanilla` (hash `03d78bf924dd0900`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr\_madan

- **claim** (`cl\_7f135423cb6357b0`): carr\_madan(opt, m, p) prices a European FX option via the Carr–Madan damped-integrand method, working in log-moneyness κ = ln(K/S₀) to avoid forming the large ln(S₀) phase. It evaluates ψ(v) = φ\_ret(v−(α+1)i) / (α²+α−v²+i(2α+1)v) with damping constant α = CM\_ALPHA, then integrates Re\[e^{−ivκ}·ψ(v)\] from 0 to an adaptively chosen upper limit via Gauss–Legendre quadrature. The call price is df\_d·S₀·e^{−ακ}/π·∫…dv; puts are obtained via exact put–call parity. The upper integration limit and panel count are both chosen adaptively (carr\_madan\_upper + oscillation count) to keep truncation and quadrature error below double precision.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr\_madan` (hash `56fd7bb70b6fbfe3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr\_madan\_upper` (hash `582b96c856df7992`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr\_madan\_upper

- **claim** (`cl\_be6cbe558b0d16f4`): carr\_madan\_upper(m, p) determines the Carr–Madan integration upper bound by taking the larger of two regime-specific decay envelopes — a Gaussian estimate sqrt(2·ln(10¹⁶)/E\[variance\]) covering small-σ regimes, and an exponential-tail estimate ln(10¹⁶)/rate (rate = v₀/(σ√(1−ρ²)) + κθT√(1−ρ²)/σ) covering slow-decaying high-σ regimes — applying a 1.5× safety margin and clamping to \[50, 20 000\]. The threshold ln(10¹⁶)≈36.84 ensures the integrand envelope is ≤1e−16 at the upper limit in either regime.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr\_madan\_upper` (hash `582b96c856df7992`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.effective\_log\_variance` (hash `13f2fac7fde9089f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.char\_exponent

- **claim** (`cl\_08b582fc42524fc4`): heston\_c4(p, t) computes the 4th cumulant of the Heston log-return distribution via a 5-point central-difference approximation of the 4th derivative of the real log-CF ln φ\_ret at u=0, using h=1e-2: d4 = (ψ(2h) − 4ψ(h) + 6ψ(0) − 4ψ(−h) + ψ(−2h)) / h⁴. By evaluating char\_exponent at real frequencies it is independent of the COS series, so it cannot mask a COS bug. Market inputs are neutralised (spot=strike=1, rates=0) since spot/strike/rates only shift c₁ and do not affect centred cumulants.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.char\_exponent` (hash `07cbde360d5f7ec3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.heston\_c4` (hash `8bb293ada7026de8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.char\_exponent

- **claim** (`cl\_857fc9b6e021c651`): char\_exponent(u: Complex, m: &MarketInputs, p: &HestonParams) -\> Complex computes ln φ\_ret(u), the log of the Heston return-space characteristic function, using an overflow-stable factoring: it forms w = exp(−d·t) (bounded, \|w\|≤1 since Re(d)≥0) and then computes A = (u²+iu)(1−w) / \[d(1+w) + ξ(1−w)\] and B via e^{−dt/2} kept inside the log, so the large e^{Re(d)·t/2} factor that would overflow cosh/sinh at high \|u\| or long t cancels before any floating-point operation. Result: φ\_ret = exp(iuμt − (κθρt/σ)·iu − v₀·A + (2κθ/σ²)·ln B). No I/O, no allocation, no mutation — the function reads only its three arguments.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.char\_exponent` (hash `07cbde360d5f7ec3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.chi

- **claim** (`cl\_8112889bc490bea4`): chi(w,c,d,lo) and psi\_coeff(w,c,d,lo) are the two closed-form basis-coefficient integrals of the Fang–Oosterlee (2008) COS method used by \`cos\`. chi = ∫\_c^d e^y cos(w(y−lo)) dy evaluated analytically as (cos(w(d−lo))e^d − cos(w(c−lo))e^c + w·sin(w(d−lo))e^d − w·sin(w(c−lo))e^c)/(1+w²); psi\_coeff = ∫\_c^d cos(w(y−lo)) dy = (sin(w(d−lo)) − sin(w(c−lo)))/w with the exact w=0 limit (d−c) handled as a special case to avoid 0/0. Both are pure: they read only their scalar f64 arguments, allocate nothing, and have no WRITES — total functions of (w,c,d,lo).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.chi` (hash `ff5cf0a1ccd1d81e`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.psi\_coeff` (hash `fb3cfd5d604d6625`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.cos

- **claim** (`cl\_638cafae94c63a3d`): cos(opt, m, p) prices a European FX option via the Fang–Oosterlee (2008) COS series. It computes Heston cumulants c₁ (mean log-return) and c₂ (variance) per Fang–Oosterlee Table 11, then the 4th cumulant c₄ via numerical 4th-difference of char\_exponent (see heston\_c4). The truncation range is \[c₁ ± L·√(\|c₂\|+√\|c₄\|)\] — including c₄ is essential for fat-tailed regimes (high σ, long T). The put leg is always priced directly (call coefficients evaluate e^{hi} at the wide right edge and lose precision); the call is recovered by exact put–call parity C = P + S·e^{−r\_f T} − K·e^{−r\_d T}. N=128 cosine terms are summed with the n=0 half-weight Fourier convention.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.cos` (hash `2f8e2710d9e960de`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.heston\_c4` (hash `8bb293ada7026de8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-heston.src.lib.gauss\_legendre

- **claim** (`cl\_4918333ab2b03b5c`): gauss\_legendre(f, lo, hi, n\_panels) integrates a real-valued function using a composite 16-point Gauss–Legendre rule. It divides \[lo,hi\] into n\_panels equal sub-intervals and applies the 8-node symmetric GL rule (GL16\_X/GL16\_W, exploiting symmetry: one loop over k=0..8 evaluates f(mid+dx)+f(mid−dx)) to each panel, accumulating acc += GL16\_W\[k\]\*(f(mid+dx)+f(mid−dx)) then returning acc\*half. No heap allocation; no branching; a purely functional numeric kernel.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-heston.src.lib.gauss\_legendre` (hash `1a4c524cc2e06df6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-integration.src.aggregate.BlendError.fmt

- **claim** (`cl\_4923d513fc92621c`): BlendError::fmt encodes the four invariants the blending/aggregation stage requires of its inputs: at least one source must be present (\`NoSources\`); all sources must describe the same (pair, tenor) slice (\`MixedSlices\`) — mixing slices would produce a meaningless mid; at least one source must survive the divergence gate (\`AllExcluded\`) — a blend where all sources are outliers is rejected rather than producing a stale or poisoned mid; and the blend configuration (e.g. half-life) must be valid (\`BadConfig\`). The Display messages are the canonical human-readable form of these four preconditions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.aggregate.BlendError.fmt` (hash `3fb77b5b075262f0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-integration.src.divergence.SmileVols.from\_quotes

- **claim** (`cl\_3bade1528987a09e`): SmileVols::from\_quotes decodes raw market-quote conventions (ATM vol, 25Δ risk-reversal, 25Δ butterfly, optional 10Δ wing quotes) into five individual implied-vol pillars using the standard FX smile decomposition. The exact formulas applied are: call\_25 = atm + 0.5\*RR\_25 + BF\_25; put\_25 = atm − 0.5\*RR\_25 + BF\_25; call\_10 = atm + 0.5\*RR\_10 + BF\_10; put\_10 = atm − 0.5\*RR\_10 + BF\_10. When outer (10Δ) quotes are absent, call\_10 and put\_10 are None. This decomposition upholds the FX market quoting convention where RR = call\_vol − put\_vol and BF = (call\_vol + put\_vol)/2 − atm\_vol.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.divergence.SmileVols.from\_quotes` (hash `18dc1447eba4a66e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-integration.src.egress.EgressError.fmt

- **claim** (`cl\_6c53180021655795`): EgressError::fmt encodes the three construction invariants for the egress rate-limiting ring: (1) capacity must be non-zero (\`ZeroCapacity\`), (2) drain\_rate\_per\_sec must be positive and finite (\`BadDrainRate\`), (3) burst must be ≥ 1 and finite (\`BadBurst\`). These are the exact conditions rejected at ring construction time; the Display messages mirror the field-level constraints verbatim, making them the canonical human-readable record of what the egress builder validates before accepting configuration.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.egress.EgressError.fmt` (hash `cae226ad9b2088f3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-integration.src.lib.PipelineError.fmt

- **claim** (`cl\_f3cea30f9f9d2fea`): PipelineError::fmt decomposes the three-stage integration pipeline error hierarchy into prefixed messages — 'feed decode error: {e}', 'normalize error: {e}', 'blend error: {e}' — each delegating to the inner error's Display. This guarantees that the stage of failure is always identifiable from the top-level error string without unwrapping.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.lib.PipelineError.fmt` (hash `3326811f5fca35b5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-integration.src.lib.PipelineError.source

- **claim** (`cl\_01ee358747d7549f`): PipelineError::source exhaustively exposes inner error sources for all three pipeline stage variants (Decode, Normalize, Blend) as Some(&dyn Error), enabling full error-chain traversal via std::error::Error::source(). No variant swallows its cause.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.lib.PipelineError.source` (hash `6e61e921fc4f63b3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-integration.src.normalize.NormalizeError.fmt

- **claim** (`cl\_37005ea64e6cee58`): NormalizeError::fmt encodes the complete set of convention-agreement invariants the normalization layer enforces on incoming vendor surface messages. A message is rejected with a structured error if: the pair token is not a valid 6-letter ISO pair (\`BadPair\`); the tenor token is unrecognised (\`BadTenor\`); a required numeric field is non-finite or non-positive (\`BadValue\`); the vol-time year fraction is non-positive (\`BadMaturity\`); the feed's declared delta convention disagrees with Celnet's canonically resolved convention for that (pair, tenor) (\`DeltaConventionMismatch\`) — such a mismatch mis-signs every hedge delta and so is rejected rather than silently accepted; the feed's ATM convention disagrees with the canonical ATM convention (\`AtmConventionMismatch\`) — this mis-places the ATM pillar strike for the entire surface; the feed's premium-currency flag is internally inconsistent with its delta convention (\`PremiumFlagInconsistent\`); or the feed's premium-currency flag disagrees with the canonical premium style resolved for that (pair, tenor) (\`PremiumStyleMismatch\`) — a wrong-currency premium mis-signs every hedge delta. All eight rejection reasons are surfaced with enough context (declared vs resolved values) for diagnosis.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.NormalizeError.fmt` (hash `d0533e5ea3f5e380`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-integration.src.normalize.declared\_is\_premium\_adjusted

- **claim** (`cl\_c2904d7b12d577d7`): Vendor-feed normalization re-derives the premium-adjusted axis at the integration seam (docs/CONVENTIONS.md DeltaConvention→premium-style; docs/CELNET-INTEGRATION.md vendor normalization). \`declared\_is\_premium\_adjusted(d)\` is a pure const-fn total predicate over DeltaConvention — true exactly for SpotPremiumAdjusted and ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants — identical in meaning to ConventionRecord::is\_delta\_premium\_adjusted but defined on the integration normalize path where an incoming vendor delta convention is checked against the resolved house record. The two definitions must agree variant-for-variant; drift here would mis-normalize a vendor feed's premium-adjusted flag. Pure: no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.declared\_is\_premium\_adjusted` (hash `9f1572abbc9326f1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-integration.src.normalize.nominal\_year\_fraction

- **claim** (`cl\_24ce58e07bd2acd4`): nominal\_year\_fraction converts a Tenor to a nominal year fraction using fixed calendar approximations: Overnight=1/365, TomNext=2/365, SpotNext=3/365, Weeks(w)=w\*7/365, Months(m)=m\*30/365, Years(y)=y\*365/365 (=y), Imm(n)=n\*3\*30/365, BrokenDate(\_)=1/365 (floor to avoid divide-by-zero downstream). The function is used only as a fallback/seed; the precise vol-time on real paths is computed separately by a calendar-exact method. The BrokenDate branch returns 1/365 by design so that the downstream positivity check always passes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.nominal\_year\_fraction` (hash `e95fabf13473acee`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-integration.src.normalize.parse\_tenor

- **claim** (`cl\_f96bedc043b91a67`): parse\_tenor converts a vendor tenor label string to a canonical Tenor. The sole case-insensitive keyword is 'ON' → Tenor::Overnight. All other tokens are parsed as a decimal integer prefix followed by a single case-insensitive unit suffix: W/w → Tenor::Weeks(n), M/m → Tenor::Months(n), Y/y → Tenor::Years(n). Any string that does not match one of these forms returns NormalizeError::BadTenor. Whitespace is trimmed before matching. The function does not recognise TN, SN, or IMM labels from a string (those variants exist in the Tenor enum but are not produced by this parser).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.parse\_tenor` (hash `f12460947d5945c0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-integration.src.normalize.vol\_time

- **claim** (`cl\_573f53a204ff7f76`): vol\_time() computes ACT/365-fixed calendar vol-time for a pair/tenor anchored at the feed's observation timestamp (via vol\_year\_fraction over the real spot→expiry schedule), falling back to nominal\_year\_fraction only when no horizon date can be derived from the nanosecond timestamp. This means the canonical surface input is always built on a well-defined, calendar-exact vol-time — never a nominal months×30 approximation when the observation date is available.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.vol\_time` (hash `c88d1a5a443bdd95`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireAtmConvention.canonical

- **claim** (`cl\_cee20cf21eeae3b7`): WireAtmConvention::canonical is a total, exhaustive bijection from the two vendor ATM-convention tags to celnet\_types::AtmConvention: AtmForward→A::AtmForward, DeltaNeutralStraddle→A::DeltaNeutralStraddle. No default arm exists, so the compiler enforces completeness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireAtmConvention.canonical` (hash `0342e29e1b0e97e0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireAtmConvention.from\_canonical

- **claim** (`cl\_9c0a56447fad88f8`): WireAtmConvention::from\_canonical is the exact inverse of WireAtmConvention::canonical, completing a lossless round-trip isomorphism for ATM conventions between the wire and canonical domains.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireAtmConvention.from\_canonical` (hash `b42401e997f59679`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireDeltaConvention.canonical

- **claim** (`cl\_c0533ff4a965b381`): WireDeltaConvention::canonical is a total, exhaustive bijection from the four vendor wire delta-convention tags to the four canonical celnet\_types::DeltaConvention variants. Every arm is mapped 1-to-1 with no default fallback: SpotUnadjusted→D::SpotUnadjusted, ForwardUnadjusted→D::ForwardUnadjusted, SpotPremiumAdjusted→D::SpotPremiumAdjusted, ForwardPremiumAdjusted→D::ForwardPremiumAdjusted. Because the match is exhaustive and const, it can never silently misclassify a convention.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireDeltaConvention.canonical` (hash `33ae5415fe84bfe2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireDeltaConvention.from\_canonical

- **claim** (`cl\_0eaf652db8c479fe`): WireDeltaConvention::from\_canonical is the exact inverse of WireDeltaConvention::canonical: it maps each celnet\_types::DeltaConvention variant back to the corresponding wire tag, forming a round-trip identity (from\_canonical(x.canonical()) == x for all x). The pair is therefore a lossless isomorphism between the two delta-convention domains.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireDeltaConvention.from\_canonical` (hash `eddc56e0be171348`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireForward.outright

- **claim** (`cl\_750df1d75fbfeb84`): WireForward::outright resolves a vendor forward quote to an absolute outright rate regardless of the wire representation. When the feed delivers the forward as pip-points it computes: outright = spot + points / pip\_factor. When the feed delivers it directly as an outright rate the spot argument is ignored and the rate is returned verbatim. This normalization guarantees that downstream code always receives a single consistent outright f64 irrespective of vendor encoding.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-integration.src.vendor.WireForward.outright` (hash `f5db9e9f9d2ceb92`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-journal.src.crc32.build\_table

- **claim** (`cl\_cc424451a3c5d4e8`): Journal CRC-32 table safety (deliverable: journal-durability). build\_table is a const fn computing the reflected IEEE CRC-32 (polynomial 0xEDB88320) lookup table purely from compile-time constants — referentially transparent by construction, no I/O, no mutable global state escaping the function, identical on every build and every platform. This is the deterministic root the entire journal torn-tail / corruption-rejection guarantee rests on: a stable table means a stable CRC. Pure (no WRITES edges); self-invalidates if the table derivation changes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.build\_table` (hash `70b5253b9d7711fa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-journal.src.crc32.crc32

- **claim** (`cl\_8b65afe5dcc76a06`): SAFETY — journal record integrity is a standard CRC-32 (IEEE 802.3 reflected polynomial): crc32 seeds 0xFFFF\_FFFF, folds each byte through the 256-entry reflected lookup TABLE (crc = (crc \>\> 8) ^ TABLE\[(crc ^ b) & 0xFF\]), and finalizes with the XOR-out 0xFFFF\_FFFF — the bit-exact reflected CRC-32 whose known-answer vectors (e.g. "123456789" =\> 0xCBF43926) are pinned by the crate's own vectors test. Every framed journal record carries this checksum over sync-word+header+payload (frame\_record appends crc32(frame).to\_le\_bytes()), so any single-bit flip in a persisted record changes the CRC and the record is rejected on replay rather than silently mis-applied to recovered book/market state. Pure: it reads only the input byte slice and returns the u32 checksum, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.crc32` (hash `7a5214530d659c2c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame\_record

- **claim** (`cl\_aeefb71d1bc2502d`): Journal CRC/torn-tail safety (deliverable: journal-durability). frame\_record is a pure, deterministic framing function: given (sequence, payload\_len, payload) it builds the sync-word + little-endian header + payload + trailing CRC-32 byte-for-byte with no I/O, no shared mutation, and no observable side effect. Determinism is the load-bearing invariant — the recovery reader recomputes the same CRC over the same framed prefix, so any torn or corrupted tail fails the checksum identically on every replay. Pure (no WRITES edges); self-invalidates if framing gains state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame\_record` (hash `a5aa1e6a4b7fb0b6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame\_snapshot

- **claim** (`cl\_a918de8c8dfa5a56`): Journal snapshot framing safety (deliverable: journal-durability). frame\_snapshot is a pure, deterministic framing function: given (watermark, snap\_len, snapshot) it emits sync-word + snapshot-marker + little-endian watermark/len + snapshot bytes + trailing CRC-32 with no I/O and no side effect, sharing the identical crc32 trailer discipline as the record path so the two framings cannot drift. Determinism guarantees the recovery reader's recomputed CRC matches bit-for-bit, rejecting any torn snapshot tail on replay. Pure (no WRITES edges); self-invalidates on code change.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame\_snapshot` (hash `fb097f3260884b35`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-limits.src.check.LimitCheck.is\_hard\_breach

- **claim** (`cl\_d63453c699025fe5`): SAFETY — pre-trade hard-breach gating (deliverable: limits-breach-detection). LimitCheck::is\_hard\_breach is a pure, deterministic predicate: it returns true iff BOTH the limit's enforcement == Enforcement::Hard AND its RagStatus utilization status is\_breach() — the conjunction that distinguishes a rejectable hard breach from a soft (advisory) warning. It reads only &self (the limit spec + the precomputed utilization), performs no I/O, no mutation, no allocation; identical inputs always yield the identical verdict. This is the exact gate pre\_trade\_check / post\_trade\_check funnel through to decide PreTradeDecision::Reject, so a hard limit is enforced (a soft one only warns). Self-invalidates if the enforcement/status conjunction is ever weakened.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.check.LimitCheck.is\_hard\_breach` (hash `3fdc49f25975528a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-limits.src.check.exposure\_of

- **claim** (`cl\_8c335dc6a146e070`): Pre-trade limit-breach exposure safety (deliverable: limits-breach-detection). exposure\_of is a pure, deterministic, total function over LimitMetric: it reads the aggregated net Greeks, vega ladder, gross concentration, and non-additive VaR/ES/StopLoss out of borrowed &NodeAggregate / &NonAdditiveExposure and returns the scalar exposure with no I/O and no mutation of any input. Determinism is the load-bearing safety property — pre-trade and post-trade checks (its four callers) measure the same metric against the same limit cap identically, so a breach can never be hidden by a non-reproducible reading. Pure (no WRITES edges); self-invalidates on change.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.check.exposure\_of` (hash `a8ae45d0cf0ac814`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-limits.src.check.gross\_concentration

- **claim** (`cl\_3e5cbcf594bcb1ce`): Pre-trade/post-trade gross concentration exposure is a pure read-only reduction: gross\_concentration folds the abs of each leaf greek (delta\_base or vega per ConcentrationMetric) over node.leaves with no writes. The limit-breach concentration metric is therefore a deterministic function of the aggregate snapshot alone — the safety property a pre-trade limit check relies on.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.check.gross\_concentration` (hash `75e2cd194db3890d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.classify

- **claim** (`cl\_60882d3165269880`): SAFETY — pre-trade limit breach is hard-classified at the cap, not approximated: LimitSpec::classify maps a projected exposure to a RagStatus by exact threshold order — ratio \> 1.0 (exposure strictly over cap) is RagStatus::Breach; ratio \>= red is Red; ratio \>= amber is Amber; else Green. Breach is the ONLY band above the cap, so a hard limit can never be silently under-classified as a mere Red warning. classify is the single pure breach-decision atom (it reads &self spec + the f64 exposure and returns a Utilization, mutating nothing); pre\_trade\_check builds on it and turns any is\_hard\_breach into PreTradeDecision::Reject. The strict-greater-than at the cap means an exposure exactly AT the cap (ratio == 1.0) is Red, not Breach — utilisation up to and including the cap is permitted, beyond it is rejected.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.classify` (hash `d245c1622e25ccab`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.utilization

- **claim** (`cl\_a150b49150567b2a`): SAFETY — limit utilization is a total, fail-closed ratio (deliverable: limits-breach-detection). LimitSpec::utilization(exposure) is a pure deterministic function: it returns \|exposure\|/cap when cap \> 0, f64::INFINITY when cap == 0 (or negative) and \|exposure\| \> 0, and 0.0 only when both are zero. The zero-cap → INFINITY branch is the fail-closed safety property: a zero cap is treated as an instant breach, never as an unbounded/divide-by-zero allowance, so a misconfigured zero limit can never silently admit risk. It reads only &self and the f64 argument; no I/O, mutation, or allocation. Self-invalidates if the zero-cap branch stops returning INFINITY.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.utilization` (hash `a9d3e93301353494`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitScope.dimension

- **claim** (`cl\_737773dc97b5481f`): LimitScope::dimension is a pure surjective map from scope variant to Option\<DimensionId\>: Firm =\> None; Trader =\> Some(Trader); Book =\> Some(Book); Desk =\> Some(Desk); CcyPair =\> Some(Underlying); Location =\> Some(Location); Entity =\> Some(Entity). None iff the scope is firm-wide (no group key).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitScope.dimension` (hash `3b16dee5be81a6a8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitScope.group\_value

- **claim** (`cl\_188a33a2a218b8ec`): LimitScope::group\_value returns None only for Firm; for all other variants it returns Some(u64) derived from the variant's inner raw ID. For CcyPair, the value is computed by constructing a zeroed FactKey with the underlying set to Underlying::Fx(p) and then delegating to FactKey::group\_value(DimensionId::Underlying), preserving byte-identical backward compatibility with the pre-generalization CcyPair encoding.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitScope.group\_value` (hash `394f3ccdc37c2f2a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-limits.src.tree.ScopePath.resolve

- **claim** (`cl\_35dfff87467762e7`): ScopePath::resolve deterministically maps a \`FactKey\` + \`Hierarchy\` to a fixed 7-element ordered scope list: \[Trader(key.trader), Book(key.book), Desk(hierarchy.desk\_of(key.book).unwrap\_or(key.desk)), CcyPair(scope\_pair\_of(&key.underlying)), Location(key.location), Entity(hierarchy.entity\_of(key.location).unwrap\_or(key.entity)), Firm\]. The Hierarchy overrides desk and entity via parent-pointer lookup (\`desk\_of\`/\`entity\_of\`), falling back to the fact's own ids when no parent is registered. This is a pure function with no I/O or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.tree.ScopePath.resolve` (hash `c43620bdc26f03f4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-limits.src.tree.scope\_pair\_of

- **claim** (`cl\_fcd398e771867bae`): scope\_pair\_of derives a CcyPair from any Underlying using a defined priority fallback: (1) direct as\_ccy\_pair() if present; (2) equity currency → self-pair (n, n); (3) commodity currency → self-pair; (4) digital asset quote currency parsed via Ccy::parse, defaulting to USD on failure; (5) USD/USD for any other variant. This ensures every Underlying maps to a valid non-panicking CcyPair for limit-tree bucketing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-limits.src.tree.scope\_pair\_of` (hash `ddbd97de15422e12`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-linear.src.forward.fair\_forward

- **claim** (`cl\_06dbe46d9109159c`): fair\_forward(inputs) returns the at-market forward rate F = spot \* e^{b \* near\_settle\_t} (carry.forward\_factor delegates to libm::exp). It is the zero-PV strike: a forward struck at this rate has PV = 0, verified bit-for-bit by fair\_forward\_has\_zero\_pv\_to\_bits (assert\_eq!(pv(...at\_fair...).to\_bits(), 0\_f64.to\_bits())). The function reads only &LinearInputs and writes nothing — pure and side-effect-free.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.fair\_forward` (hash `4b4493a8019e6442`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-linear.src.forward.fd\_central

- **claim** (`cl\_846de935cbd16189`): greeks(inputs) computes the complete analytic Greek vector {pv, delta, rho\_dom, rho\_for, theta} for an FX outright forward in closed form, routing all transcendentals through celnet\_core::math::exp (libm-backed, bit-identical to the Carry accessors) for cross-platform reproducibility. The closed forms are: delta = sign\*N\*e^{-r\_for\*t}; rho\_dom = sign\*N\*K\*t\*e^{-r\_dom\*t}; rho\_for = -sign\*N\*S\*t\*e^{-r\_for\*t}; theta = sign\*N\*(-r\_for\*S\*e^{-r\_for\*t} + r\_dom\*K\*e^{-r\_dom\*t}). All five outputs are independently cross-checked against central finite differences (fd\_central: (f(x+h)-f(x-h))/(2h)) in greeks\_match\_central\_finite\_difference. The function reads only its &LinearInputs argument and writes nothing — pure and side-effect-free.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.fd\_central` (hash `dd5e7bfbcae0835c`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.greeks` (hash `a15380cab2973d35`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-linear.src.forward.oracle\_pv

- **claim** (`cl\_96907b4546b8fe15`): pv\_at(inputs, t) is the single closed-form present-value kernel for an FX outright forward at time t: PV = sign(side) \* N \* df\_dom(t) \* (F(t) - K), where F(t) = spot \* e^{b\*t} (forward via carry.forward\_factor), df\_dom(t) = e^{-r\_dom\*t} (via carry.discount\_df), K = contract\_rate, N = notional, sign = +1 Buy / -1 Sell. It reads only its two arguments, has no side effects, no allocation, no I/O — pure by inspection. Both forward::pv and swap::pv delegate entirely to this kernel. The formula is cross-validated against oracle\_pv (an independent flat expansion: sign\*N\*(spot\*e^{-r\_for\*t} - K\*e^{-r\_dom\*t})) byte-for-byte in pv\_matches\_independent\_discount\_bond\_route.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.oracle\_pv` (hash `98c8cc36056c9c1e`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.pv` (hash `89e9a976bdfbfd58`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.forward.pv\_at` (hash `0459cf4629f916ac`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputError.fmt

- **claim** (`cl\_dda489868b3cd135`): LinearInputError::fmt enforces two domain-level pre-condition messages: NonPositiveNotional → 'linear notional must be strictly positive' (notional ≤ 0 rejected) and NegativeSettleTime → 'linear settlement time must be non-negative' (t \< 0 rejected). Both are static strings — the message set is the machine-readable contract for valid linear instrument inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputError.fmt` (hash `347fd218a5f33b15`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.discount\_df

- **claim** (`cl\_555af803ef905080`): LinearInputs::forward(t) and LinearInputs::discount\_df(t) are the two primitive carry-seam accessors on the inputs struct: forward(t) = self.spot \* self.carry.forward\_factor(t) (i.e. S\*e^{b\*t}); discount\_df(t) = self.carry.discount\_df(t) (i.e. e^{-r\_dom\*t}). Both delegate entirely to celnet\_core::carry::Carry, so they are byte-identical to CarryInputs::forward / CarryInputs::discount\_df — proved by forward\_and\_df\_match\_core\_carry\_inputs\_byte\_for\_byte. This identity is the structural seam ensuring the linear and options leaves share one pricing substrate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.discount\_df` (hash `686907ab62301e9d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.forward` (hash `3375ce35f2c16367`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-linear.src.inputs.Side.opposite

- **claim** (`cl\_3dec056185137e63`): Side::opposite is a pure const involution: opposite(Buy) = Sell, opposite(Sell) = Buy. Applying it twice returns the original value (self-inverse). It is const-evaluable and has no heap allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.Side.opposite` (hash `bdaa7848b8479241`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-linear.src.inputs.Side.sign

- **claim** (`cl\_8ccb8e551443ff47`): Side::sign is a pure const bijection: Side::Buy =\> 1.0, Side::Sell =\> -1.0. Used throughout linear pricing to convert directional side to a multiplicative sign factor with no branching overhead at call sites.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.Side.sign` (hash `f86171fbbc29dadd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-linear.src.ndf.Ndf.pv

- **claim** (`cl\_d9cc02d9b52b69e3`): CAPABILITY (linear leaf — FX forward/swap/NDF): celnet-linear::ndf::Ndf::pv is the non-deliverable-forward present-value entry of the linear-products leaf — a pure, side-effect-free closed form (delegating to pv\_at over LinearInputs at the near settle tenor) with no I/O, allocation-mutation, or logging. The NDF cash-settled PV equals the deliverable forward PV (proven by ndf\_pv\_equals\_deliverable\_forward\_pv), and LinearInputs::forward/discount are byte-identical to the core CarryInputs (forward\_and\_df\_match\_core\_carry\_inputs\_byte\_for\_byte), so the linear capability (FX forward/swap/NDF) sits on the same carry seam as the option leaves and reaches the one contract.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.ndf.Ndf.pv` (hash `85b4b1360da6ac92`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-linear.src.swap.leg\_rates

- **claim** (`cl\_a827d20b84f0ed0f`): swap\_points(inputs) returns the FX swap-point differential: far\_forward - near\_forward, where both legs use the same carry curve (F(t) = spot \* e^{b\*t}). Swap points are positive when the far rate exceeds the near rate (base currency at a forward premium, r\_dom \> r\_for), verified by swap\_points\_sign\_matches\_carry. Returns SwapError::MissingFarLeg when far\_settle\_t is absent. Pure: reads only &LinearInputs, no writes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.swap.leg\_rates` (hash `87b51576d4e81784`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.swap.swap\_points` (hash `58e56a22361de98b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-linear.src.swap.pv

- **claim** (`cl\_2c43fce9e934a008`): swap::pv(inputs) prices a two-legged FX swap as the algebraic sum of two outright forward PVs evaluated at different settle times: near\_pv = pv\_at(inputs, near\_settle\_t) on the stated side, far\_pv = pv\_at(far\_leg, far\_t) on the opposite side (far\_leg clones the inputs with side.opposite()). It returns SwapError::MissingFarLeg if far\_settle\_t is absent. The two-leg decomposition is independently verified by pv\_equals\_independent\_two\_leg\_sum and the opposite-leg netting property by equal\_dates\_opposite\_legs\_net\_to\_zero.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-linear.src.swap.pv` (hash `7b7c901b864dacea`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-linear.src.swap.swap\_inp` (hash `85a746f5802fba8f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-observability.src.channel.telemetry\_channel

- **claim** (`cl\_fd0ef5022a75650e`): PILLAR (GUIDE.md guardrail 11 — zero-cost observability, telemetry offloads over a BOUNDED queue so the pinned hot core stays alloc/lock/log-free). \`telemetry\_channel(capacity)\` is the single constructor of the hot-path-to-drain seam: it builds an rtrb single-producer/single-consumer RingBuffer of FIXED \`capacity.max(1)\` (the bounded queue) and returns the (HotProbe, TelemetryDrain) pair sharing one Arc\<Shared\>. The hot side (HotProbe) only pushes HotSamples into the pre-sized ring and never blocks or allocates per sample; backpressure is absorbed by dropping/counting gaps, never by stalling the pricing core. The function itself is a pure constructor — its output depends only on \`capacity\`, it mutates no shared/global state and has no observable side effect beyond returning the owned channel ends.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.channel.telemetry\_channel` (hash `91c12ae6fd592725`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.p99\_ns

- **claim** (`cl\_99794794dd6ebe9f`): PILLAR (GUIDE.md guardrail 11 — mission-critical ops instrumentation with HdrHistogram p50/p99/p99.9, without diminishing hot-path performance). \`LatencyRecorder::p99\_ns\` is the canonical p99 tail-latency readout: a pure read-only accessor returning \`self.percentile\_ns(99.0)\` over the recorded latency histogram in nanoseconds. It reads the histogram, mutates nothing, and has no side effects — the p99/tail telemetry is computed off the bounded drain, never on the zero-alloc pricing core. This is the latency-budget observability surface referenced by docs/ARCHITECTURE.md §1.2 / docs/SCALE-OUT.md.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.p99\_ns` (hash `2affb89bd55eb475`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.percentile\_ns

- **claim** (`cl\_352ec0140098e6eb`): PILLAR (GUIDE.md guardrail 11 — instrument for mission-critical ops with HdrHistogram p50/p99/p99.9, without diminishing performance). \`LatencyRecorder::percentile\_ns(q)\` is the single quantile-readout primitive that the named p50\_ns/p99\_ns/p999\_ns accessors all delegate to: it returns \`self.hist.value\_at\_quantile(q / 100.0)\` — the HdrHistogram value at the q-th percentile, in nanoseconds (q is a percentage, converted to a \[0,1\] quantile). It is a pure read-only accessor: it reads the recorded latency histogram and mutates nothing, so quantile readout never touches the zero-alloc pricing hot path (recording is offloaded; this only reads the already-merged histogram). This is the tail-latency budget surface in docs/ARCHITECTURE.md §1.2.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.percentile\_ns` (hash `50ca549cd6a6b82f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.with\_expected\_interval

- **claim** (`cl\_9000e6ae7c88f509`): LatencyRecorder.with\_expected\_interval constructs an HdrHistogram with 3 significant figures over the range \[1 ns, MAX\_NS\] (60 s), enabling coordinated-omission-corrected recording. It gracefully degrades to an unconstrained 3-sig-fig histogram on the theoretically-impossible constructor failure, keeping mission-critical code panic-free.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.with\_expected\_interval` (hash `dc493aef9a2fbb07`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-observability.src.logging.AuditStage.label

- **claim** (`cl\_ae409d0c7ababdee`): \`AuditStage::label\` is a const total mapping from audit lifecycle stage to the static structured-log event name used in compliance audit records: \`QuoteRequested\`→\`"quote\_requested"\`, \`QuoteIssued\`→\`"quote\_issued"\`, \`QuoteAccepted\`→\`"quote\_accepted"\`, \`QuoteRejected\`→\`"quote\_rejected"\`, \`TradeBooked\`→\`"trade\_booked"\`, \`TradeAmended\`→\`"trade\_amended"\`, \`TradeCancelled\`→\`"trade\_cancelled"\`. These are the seven canonical stages of the trade lifecycle captured in the lossless audit drain.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.logging.AuditStage.label` (hash `bc9c8457d9ad386a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.from\_error\_class

- **claim** (`cl\_0c260bf1b665fd8e`): \`LogClass::from\_error\_class\` is a total, exhaustive const mapping from \`ErrorClass\` to \`LogClass\` that encodes the severity contract: \`Ok\` → \`Lifecycle\`; \`InvalidInput\` → \`ClientError\`; \`StaleState\|Arbitrage\` → \`MarketData\`; \`NoConvergence\` → \`Fault\`; \`TelemetryDropped\` → \`Degraded\`; \`Internal\` → \`Fault\`. This mapping determines the log level and structured label that appear in every operational log line derived from an \`ErrorClass\`-classified result.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.from\_error\_class` (hash `4b32ea93b5250a9c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.label

- **claim** (`cl\_b3446dee6163ef84`): \`LogClass::label\` is a const total mapping from log class to the structured-log category string: \`Lifecycle\`→\`"lifecycle"\`, \`ClientError\`→\`"client\_error"\`, \`MarketData\`→\`"market\_data"\`, \`Degraded\`→\`"degraded"\`, \`Fault\`→\`"fault"\`, \`Security\`→\`"security"\`. This label is the primary dimension used to route log output to operations dashboards and alerting rules.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.label` (hash `1adddaefdd40f61b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.level

- **claim** (`cl\_aae37393ec61d5b4`): \`LogClass::level\` is a const total mapping from log class to the \`tracing::Level\` used when emitting log records: \`Lifecycle\`→\`INFO\`, \`ClientError\`→\`WARN\`, \`MarketData\`→\`WARN\`, \`Degraded\`→\`WARN\`, \`Fault\`→\`ERROR\`, \`Security\`→\`INFO\`. Notably \`Fault\` (which covers \`NoConvergence\` and \`Internal\` error classes) emits at \`ERROR\`, while client-visible errors (\`ClientError\`, \`MarketData\`, \`Degraded\`) emit at \`WARN\`. This determines alerting and on-call trigger thresholds.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogClass.level` (hash `ed55ce48bc95f95b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogConfig.env\_filter

- **claim** (`cl\_174a1cc44cd17d38`): LogConfig::env\_filter resolves the tracing EnvFilter with a two-level fallback: if a filter directive is explicitly configured, it is parsed via EnvFilter::try\_new; if that parse fails, it falls back to EnvFilter::new(max\_level). If no directive is configured, it tries EnvFilter::try\_from\_default\_env() (reads RUST\_LOG); on failure falls back to max\_level. The function never panics and always returns a valid EnvFilter.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.logging.LogConfig.env\_filter` (hash `6ac1c5e8bcc5e2df`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.ErrorClass.from\_u16

- **claim** (`cl\_7705d67195c4dabd`): \`ErrorClass::from\_u16\` is the partial inverse of the wire encoding for error classes, forming a bijection on \`\[0, 6\]\`: 0↔\`Ok\`, 1↔\`InvalidInput\`, 2↔\`StaleState\`, 3↔\`NoConvergence\`, 4↔\`Arbitrage\`, 5↔\`TelemetryDropped\`, 6↔\`Internal\`; any value outside returns \`None\`. Together with \`ErrorClass::label\` this pair pins the wire-serialisation contract for audit and telemetry records.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.ErrorClass.from\_u16` (hash `079bdf168b0e3808`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.ErrorClass.label

- **claim** (`cl\_7451109f07a37c2d`): \`ErrorClass::label\` is a const total mapping from error class to the static structured-log tag string: \`Ok\`→\`"ok"\`, \`InvalidInput\`→\`"invalid\_input"\`, \`StaleState\`→\`"stale\_state"\`, \`NoConvergence\`→\`"no\_convergence"\`, \`Arbitrage\`→\`"arbitrage"\`, \`TelemetryDropped\`→\`"telemetry\_dropped"\`, \`Internal\`→\`"internal"\`. These strings appear in every log record and metric label produced by the observability crate; they are part of the public operational contract.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.ErrorClass.label` (hash `a4822aa02ab5463a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.HotSample.new

- **claim** (`cl\_9c1a96f8fd12cae4`): HotSample.new is a const fn that initializes a HotSample with seq=0 (stamped by HotProbe.publish at enqueue time), class=ErrorClass::Ok (overridable via with\_class), and \_pad=0 for POD alignment. The struct is 32 bytes and Copy, satisfying the zero-alloc constraint on the hot path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.HotSample.new` (hash `4970dc2818aebfaf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.OpKind.from\_u16

- **claim** (`cl\_f4fe87cc3e811619`): \`OpKind::from\_u16\` is the partial inverse of the wire encoding for operation kinds, forming a bijection on the domain \`\[0, 5\]\`: 0↔\`VanillaPrice\`, 1↔\`SurfaceVol\`, 2↔\`ExoticPrice\`, 3↔\`StreamQuote\`, 4↔\`RfqQuote\`, 5↔\`StatePublish\`; any value outside this range returns \`None\`. The wire integer identity \`from\_u16(x as u16) == Some(x)\` holds for all valid variants and \`from\_u16(v).map(\|k\| k as u16) == Some(v)\` for \`v\` in \`\[0,5\]\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.OpKind.from\_u16` (hash `ea1bf8f7f85915c7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.OpKind.label

- **claim** (`cl\_4f300f87ec92daa4`): \`OpKind::label\` is a const total mapping from operation-kind to the static metric/log tag string used in all telemetry output: \`VanillaPrice\`→\`"vanilla\_price"\`, \`SurfaceVol\`→\`"surface\_vol"\`, \`ExoticPrice\`→\`"exotic\_price"\`, \`StreamQuote\`→\`"stream\_quote"\`, \`RfqQuote\`→\`"rfq\_quote"\`, \`StatePublish\`→\`"state\_publish"\`. These strings form the stable metric-dimension values for Prometheus/HdrHistogram label sets; changing them is a breaking observability schema change.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.OpKind.label` (hash `cea78bcf54042c25`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-observability.src.record.TickRate.ticks\_to\_nanos

- **claim** (`cl\_29fa0d60d621fc3b`): TickRate.ticks\_to\_nanos converts a raw hardware-tick count to nanoseconds using round-to-nearest arithmetic in u128 to avoid overflow and sub-tick truncation bias: \`(ticks \* 1\_000\_000\_000 + ticks\_per\_sec/2) / ticks\_per\_sec\`. The rounding comment explicitly cites p99.9 fairness as the motivation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-observability.src.record.TickRate.ticks\_to\_nanos` (hash `0ebaa0d63729979a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.american.gk\_european

- **claim** (`cl\_688bd875c0406c1d`): The parity oracle \`gk\_european\` implements the Garman-Kohlhagen closed form independently of \`celnet\_vanilla\`: d1 = (ln(S/K) + (r\_d - r\_f + ½σ²)·T) / (σ√T), d2 = d1 − σ√T; Call = S·e^{-r\_f T}·N(d1) − K·e^{-r\_d T}·N(d2); Put = K·e^{-r\_d T}·N(-d2) − S·e^{-r\_f T}·N(-d1). Its only callers are parity tests — it never wraps production code, guaranteeing the cross-check is code-disjoint.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.american.gk\_european` (hash `24b6a36b2ea52fa1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.broker\_smile.high\_rr\_em\_case\_reprices

- **claim** (`cl\_7edd0b9283791bd6`): Rows 10–11 (parity) — \`smile\_reprices\_broker\_strangle\` and \`high\_rr\_em\_case\_reprices\` prove the broker→smile calibration reprices the market strangle (a call+put priced at a single vol σ\_ATM+BF at the broker wing strikes) to 1e-8 relative / 1e-10 absolute, AND that the naive arithmetic-butterfly smile (the documented '\#1 production bug' in docs/CAPABILITIES-VS-COMPETITION.md) \*misprices\* the same strangle by a demonstrably larger error. Asserting the naive misprice exceeds a threshold while the calibrated one is within tolerance proves the calibration is a real correction, not a tautology — on both a G10 benign slice and a high-risk-reversal EM case.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.broker\_smile.high\_rr\_em\_case\_reprices` (hash `03f57290a8b1a816`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.broker\_smile.smile\_reprices\_broker\_strangle` (hash `70729153226c2e71`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.atm\_dns\_strike\_is\_delta\_neutral

- **claim** (`cl\_a6da14885129cc70`): Row 2 (parity) — \`strike\_delta\_roundtrip\_all\_conventions\` proves the convention-aware strike↔delta solver round-trips in all four FX delta conventions (SpotUnadjusted, ForwardUnadjusted, SpotPremiumAdjusted, ForwardPremiumAdjusted) for both calls and puts at the 25Δ and 10Δ wings: solve \`strike\_from\_delta(conv, opt, target, &inputs)\` then re-read \`convention\_delta(conv, opt, &solved\_inputs)\` and require agreement to 1e-9 relative / 1e-10 absolute, for ≥50 rows. \`atm\_dns\_strike\_is\_delta\_neutral\` additionally proves the delta-neutral-straddle ATM strike satisfies call\_delta + put\_delta = 0 to 1e-9 in unadjusted conventions, and the ATMF strike equals the outright forward F=S·e^{(r\_d−r\_f)T} to 1e-12.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.atm\_dns\_strike\_is\_delta\_neutral` (hash `73de77f36df21a34`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.strike\_delta\_roundtrip\_all\_conventions` (hash `e09a675fd159bfee`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.closed\_form\_gk

- **claim** (`cl\_7e3bfd7b459c91b2`): closed\_form\_gk is an independent reference implementation of the Garman-Kohlhagen closed-form FX option formula used as the parity oracle in the conventions test suite. It computes d₁ = (ln(S/K) + (r\_dom − r\_for + ½σ²)T) / (σ√T), d₂ = d₁ − σ√T, and prices: Call = S·e^{-r\_for·T}·N(d₁) − K·e^{-r\_dom·T}·N(d₂); Put = K·e^{-r\_dom·T}·N(−d₂) − S·e^{-r\_for·T}·N(−d₁). Uses only math:: primitives and no production pricer code, making it a genuine cross-implementation oracle.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.closed\_form\_gk` (hash `91022a5addb6f7a7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.premium\_adjusted\_call\_delta\_is\_guarded

- **claim** (`cl\_9d5dd5d3c8219f10`): Row 3 (parity) — \`premium\_adjusted\_call\_delta\_is\_guarded\` proves the premium-adjusted call delta non-monotone guard: (a) a target above the attainable ceiling (delta at strike K\_max where ∂Δ/∂K=0, computed by \`premium\_adjusted\_call\_delta\_max\`) returns \`Err(Unreachable)\` — not a silently wrong strike — and (b) a target at 50% of the ceiling is reachable and round-trips to 1e-9. This is the non-monotone primitive Bloomberg/Fenics bury; Celnet exposes and gates it.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.premium\_adjusted\_call\_delta\_is\_guarded` (hash `b9386e1dfd675dca`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.vanilla\_price\_matches\_closed\_form

- **claim** (`cl\_7ade83252f6cee45`): Row 1 (parity) — \`vanilla\_price\_matches\_closed\_form\` proves the production pricer \`celnet\_vanilla::price\` reproduces the independent closed-form Garman-Kohlhagen formula C = S·e^{-r\_f·T}·N(d1) − K·e^{-r\_d·T}·N(d2), P = K·e^{-r\_d·T}·N(-d2) − S·e^{-r\_f·T}·N(-d1), d1=(ln(S/K)+(r\_d−r\_f+½σ²)T)/(σ√T), d2=d1−σ√T, computed in the test via a separate expression grouping (not the same code path), to tolerance 1e-12 relative / 1e-14 absolute across ≥14 reference-market × option-side rows. This is the pricing floor every incumbent meets behind closed doors; Celnet meets it in the open.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.vanilla\_price\_matches\_closed\_form` (hash `f05b1b8013557276`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.crossasset.cp

- **claim** (`cl\_f05990413ea4a320`): Each of the four parity test modules (pivot\_wire, listed\_future, crossasset, perpetual) defines a local \`cp\` helper that is a total, exhaustive bijection from \`OptionType\` to \`Cp\`: \`OptionType::Call\` maps to \`Cp::Call\` and \`OptionType::Put\` maps to \`Cp::Put\`. The function performs no allocation or I/O and acts purely as a type-bridge between the wire \`OptionType\` enum (used in parity test inputs) and the pricer-layer \`Cp\` enum (consumed by pricer constructors). Because Rust's exhaustive match guarantees both arms are covered, no \`OptionType\` variant is ever silently dropped or mismapped.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.crossasset.cp` (hash `60cc219e38a73464`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.listed\_future.cp` (hash `ffa1ba93d892b48a`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.perpetual.cp` (hash `5cc960beda42ba40`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pivot\_wire.cp` (hash `2ffaa4e359dd696c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.price\_and\_greeks\_are\_bit\_identical

- **claim** (`cl\_8a8478da665b66b0`): Row 15 (parity) — \`price\_and\_greeks\_are\_bit\_identical\` proves bit-for-bit reproducibility (IEEE-754 \`to\_bits()\` equality) of the price and full 13-Greek set via three non-vacuous checks: (a) reconstruct inputs from a shortest-round-trip decimal snapshot string (no shared provenance) and recompute — proves value-determinism not identity-dependence; (b) cross-thread: 8 spawned threads each rebuild inputs from the snapshot and compute independently, all must agree to the bit — catches hidden global/thread-local state; (c) committed golden-bit table for two textbook regimes (e.g. call price bits \`0x4024\_e6b2\_e3d5\_4dc0\` for S=K=100, σ=20%, T=1, r\_d=5%) — catches cross-run/cross-build ULP regressions. \`smile\_and\_exotics\_are\_bit\_identical\` extends the same three-mode check to the broker smile and analytic exotic pricers.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.price\_and\_greeks\_are\_bit\_identical` (hash `0fe16ae3b42af347`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.smile\_and\_exotics\_are\_bit\_identical` (hash `aefe5d5596012bf1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.essvi.ssvi\_byte\_recovered\_at\_constant\_rho

- **claim** (`cl\_02d6020d983e660a`): \`ssvi\_byte\_recovered\_at\_constant\_rho\` establishes that \`ExtendedSlice::from\_curvature(θ, ρ, φ(θ))\` is bit-for-bit identical to \`ParametricSurface::total\_variance(k, θ)\` across a full 5×4×3×4×7 parameter sweep (420 checked points). This proves the eSSVI single-tenor slice is a lossless, zero-approximation projection of the full SSVI surface when ρ is held constant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.essvi.ssvi\_byte\_recovered\_at\_constant\_rho` (hash `cc7a668f244fbb8a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.barrier\_kind

- **claim** (`cl\_3c6a1388dabc7cdb`): barrier\_kind maps the four BarrierType values from the golden reference corpus (DownOut, DownIn, UpOut, UpIn) onto the canonical celnet BarrierKind struct. The mapping is: DownOut → (up=false, KnockOut); DownIn → (up=false, KnockIn); UpOut → (up=true, KnockOut); UpIn → (up=true, KnockIn). The option\_type field is forwarded unchanged.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.barrier\_kind` (hash `279b3911efb65ad9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.barriers\_match\_quantlib

- **claim** (`cl\_b83c149bfcba39e2`): Rows 12–14 (parity) — \`digitals\_match\_quantlib\`, \`touches\_and\_dnt\_match\_quantlib\`, and \`barriers\_match\_quantlib\` prove the first-generation exotic pricers reproduce an independent QuantLib 1.42.1 reference to 1e-9 relative / 1e-10 absolute (and 1e-6 / 1e-8 for the double-barrier reflection series) across: European digitals (cash-or-nothing and asset-or-nothing, both directions), one-touch / no-touch / double-no-touch / double-touch, all eight single-barrier flavours (up/down × in/out × call/put), and the double knock-out/knock-in. The QuantLib-sourced CSV tables live in celnet-golden; this crate re-runs the checks through the public celnet-exotics API.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.barriers\_match\_quantlib` (hash `0052a4181fd7be9d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.digitals\_match\_quantlib` (hash `c9894d6be061dd40`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.touches\_and\_dnt\_match\_quantlib` (hash `4a3ffc3274ace677`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.celnet\_double\_barrier

- **claim** (`cl\_2414a7bcd4a6a95f`): celnet\_double\_barrier converts a DoubleBarrierRecord to a Celnet double-barrier price using the put-call in-out parity identity: KnockOut price is computed directly via double\_knock\_out\_price; KnockIn price is derived as vanilla − KO (in-out parity). This ensures both variants are priced consistently from the same underlying KO formula without a separate KI model.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.celnet\_double\_barrier` (hash `305234423927b985`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.celnet\_touch

- **claim** (`cl\_aecee9bbda9ee047`): celnet\_touch dispatches a TouchRecord to the matching Celnet pricer: OneTouch → one\_touch\_price with RebateTiming::AtExpiry; NoTouch → no\_touch\_price; Dnt → double\_no\_touch\_price with a DoubleNoTouch struct; DoubleTouch → double\_touch\_price with the same struct. Each branch expects its required barrier field(s) to be present and panics with a descriptive message otherwise.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.celnet\_touch` (hash `5a12279df46b27f1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.forward\_start.forward\_start\_mc

- **claim** (`cl\_2d303270802f75de`): forward\_start\_mc is the antithetic Monte Carlo reference pricer for forward-start options. For each of \`pairs\` antithetic pairs it draws two independent standard normals (z₁, z₂) via CounterRng and prices both the +z and −z paths: S\_reset = S·exp(drift₁ + σ√t₁·z₁), S\_term = S\_reset·exp(drift₂ + σ√dt₂·z₂), payoff = max(φ·(S\_term − m·S\_reset), 0), antithetic average = (payoff(+z) + payoff(−z))/2. Returns (df·mean, df·std\_error) where df = e^{-r\_dom·T}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.forward\_start.forward\_start\_mc` (hash `4a749ffd8d9654d3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.frtb.longhand\_k\_b

- **claim** (`cl\_8ca46881c4d69e71`): \`longhand\_k\_b\` computes the FRTB SA-CVR bucket aggregation formula K\_b = sqrt(max(0, Σ\_k w\_k² + Σ\_{k≠l} ρ·w\_k·w\_l)) as a literal double loop with uniform intra-bucket correlation ρ. It serves as the code-disjoint oracle against the vectorised \`celnet-risk\` implementation; agreement proves the production formula is correct.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.frtb.longhand\_k\_b` (hash `b1d771d89aa64fe7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.frtb.longhand\_scale

- **claim** (`cl\_1ff72ba0c1aab19a`): \`longhand\_scale\` encodes the FRTB SA correlation scenario scaling rules exactly as published: High → min(1.25ρ, 1.0); Medium → ρ (identity); Low → max(2ρ−1, 0.75ρ). The Low floor of 0.75ρ is critical — the docstring explicitly warns it is NOT a plain max(2ρ−1, 0), a distinction that was a historical oracle bug in this codebase.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.frtb.longhand\_scale` (hash `15a1935199a65016`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.full\_greek\_set\_matches\_finite\_difference

- **claim** (`cl\_f6fd37324d5c6a02`): Rows 4–5 (parity) — \`full\_greek\_set\_matches\_finite\_difference\` proves all nine 'core' Greeks (delta\_spot, vega, rho\_dom, rho\_for, theta, gamma, vanna, volga, charm) agree with central finite differences of the price function to 1e-5 relative / 1e-7 absolute (first-order) or 1e-4 / 1e-6 (second-order), across ≥126 rows. \`second\_order\_wing\_greeks\_match\_fd\` proves the remaining four (speed=∂gamma/∂S, zomma=∂gamma/∂σ, color=∂gamma/∂T, delta\_forward=∂\[V\_fwd\]/∂F) against their defining derivatives, completing the full 13-Greek set. \`put\_call\_parity\_across\_regimes\` proves C−P = S·e^{-r\_f·T} − K·e^{-r\_d·T} across ≥7 regimes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.full\_greek\_set\_matches\_finite\_difference` (hash `fdff0fffb0d6e344`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.put\_call\_parity\_across\_regimes` (hash `8429141a6d1d4755`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.second\_order\_wing\_greeks\_match\_fd` (hash `f3657a8b745175d7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.heston.call\_monotone\_non\_increasing\_in\_strike\_and\_positive

- **claim** (`cl\_fec8384d7a4a85eb`): call\_monotone\_non\_increasing\_in\_strike\_and\_positive verifies the static no-arbitrage condition ∂C/∂K ≤ 0 for the Heston pricer across both Carr-Madan and COS implementations. Over all parameter sets in param\_sweep, maturities {0.5, 1.0, 2.0, 3.0}y, and strikes K ∈ \[60, 160\] step 5, every call price must be non-negative (\> -1e-12) and non-increasing in K (C(K+5) ≤ C(K) + 1e-9).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.heston.call\_monotone\_non\_increasing\_in\_strike\_and\_positive` (hash `26f0fd929c87fc20`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.linear.sign

- **claim** (`cl\_02a1c763bf8f81a8`): In the linear parity test module, \`sign\` maps the \`celnet-linear\` \`Side\` enum to the numeric sign convention used by the independent oracle: \`Side::Buy\` → \`+1.0\_f64\`, \`Side::Sell\` → \`-1.0\_f64\`. This is the canonical sign bridge that the four call-sites use when computing oracle PnL or payoff directional sign, ensuring that the parity comparison uses the same algebraic convention as the production pricer.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.linear.sign` (hash `24875f2f29fe0806`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.lsv.Gk.price

- **claim** (`cl\_8814a7a011e3abef`): \`Gk::price\` (the LSV-test closed-form oracle) computes the Black-Scholes-Merton formula in the forward measure: F = S·e^{(r\_d−r\_f)T}, df = e^{-r\_d T}; d1 = (ln(F/K) + ½σ²T)/(σ√T), d2 = d1 − σ√T; Call = df·(F·N(d1)−K·N(d2)); Put = df·(K·N(-d2)−F·N(-d1)). It serves as the zero-skew LSV calibration oracle: when σ\_LSV is flat the engine must match this formula exactly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.lsv.Gk.price` (hash `69651d3c2f972c1f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.civil\_from\_days

- **claim** (`cl\_71b7112d42a40992`): civil\_from\_days is the inverse of days\_from\_civil: given a 1970-epoch day count z, it returns (year, month, day) in the proleptic Gregorian calendar using the Euclidean-affine algorithm (Hinnant 2010). The derivation uses: era = ⌊(z+719468)/146097⌋; day-of-era, year-of-era, day-of-year, month-of-period and day-of-month are all derived by pure integer arithmetic with no library calls. Civil month is adjusted so January–February belong to the prior year's period.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.civil\_from\_days` (hash `68db4d164171e1e9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.days\_from\_civil

- **claim** (`cl\_11bf028eee0ee34c`): days\_from\_civil converts a proleptic Gregorian (year, month, day) triple to a 1970-epoch day count using the Euclidean-affine algorithm (Hinnant 2010). January and February are treated as months 11–12 of the prior year (y−1). The formula: era = ⌊y/400⌋, yoe = y−400·era, doy = (153·(m\>2 ? m−3 : m+9)+2)/5+d−1, doe = 365·yoe+yoe/4−yoe/100+doy, result = 146097·era+doe−719468. All arithmetic is pure integer, no library calls.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.days\_from\_civil` (hash `de9f8f4ab29bc86e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.holiday\_for

- **claim** (`cl\_1d19488f40e42a64`): \`holiday\_for\` is the currency-dispatching oracle in the pair\_universe parity test: it routes a 3-letter ISO currency code to the correct independent Gregorian holiday function for the settlement-calendar validation suite. The covered codes and their calendars are: USD → \`us\_holiday\`, EUR → \`target2\_holiday\`, MXN → \`mexico\_holiday\`, ZAR → \`southafrica\_holiday\`, NOK → \`norway\_holiday\`, SEK → \`sweden\_holiday\`, and XAU/XAG/XPT/XPD/GBP → \`uk\_holiday\` (Loco-London metals settle on the London calendar, matching market convention). Any other code causes a controlled \`panic!\` — the function is deliberately non-exhaustive to surface missing coverage rather than silently accept wrong calendars.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.holiday\_for` (hash `a54583808a85a04a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.independent\_spot

- **claim** (`cl\_f5b3e5027dbac76b`): \`independent\_spot\` computes the spot settlement date from a trade date (y,m,d) by advancing exactly \`lag\` business days using the pair-specific settlement-centre calendar determined by \`legs\_for\`. It is the oracle for \`settlement\_date\` in the production \`pair\_meta\` — sharing no code — and is used only inside parity tests verifying spot-lag correctness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.independent\_spot` (hash `76e2b870ec8c44ce`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.legs\_for

- **claim** (`cl\_7ff91f82364c9a1f`): \`legs\_for\` maps an FX pair string to the set of settlement-centre currency codes for spot-date computation: base and quote centres are always included; if neither is USD, USD is added as the common correspondent centre. Loco-London metals (XAU, XAG, XPT, XPD) are mapped to their own GBP-equivalent centre. Any unlisted currency panics — the oracle is intentionally narrow to prevent silent miscalculation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.legs\_for` (hash `0df6b82a7a4ae089`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.uk\_holiday

- **claim** (`cl\_cc72ad8d91619721`): \`uk\_holiday\` is the independent Gregorian oracle for London (UK) bank holidays, used by the pair\_universe parity suite to validate spot settlement dates for GBP and Loco-London precious metals (XAU, XAG, XPT, XPD). It applies the bump-forward observance rule (Saturday → Monday+2, Sunday → Monday+1) to New Year's Day and the Christmas/Boxing Day cluster. The seven covered rules are: New Year's Day (Jan 1, bump-forward observed), Good Friday (Easter-2), Easter Monday (Easter+1), Early May (1st Monday May), Spring Bank Holiday (last Monday May), Summer Bank Holiday (last Monday August), and Christmas+Boxing Day pair (Dec 25+26, bump-forward observed with collision guard: if Boxing falls on the same observed date as Christmas, Boxing advances one further weekday).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.uk\_holiday` (hash `7e12082a98701630`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.us\_holiday

- **claim** (`cl\_cc68497b116e0875`): us\_holiday is a pure predicate returning true iff (y, m, d) is a US federal public holiday observed on a weekday, implementing nearest-weekday observance (Saturday→Friday, Sunday→Monday) for fixed-date holidays and nth-weekday rules for floating ones. The complete set covered: New Year's Day (Jan 1), MLK Day (3rd Mon Jan), Presidents'/Washington Day (3rd Mon Feb), Memorial Day (last Mon May — 5th Monday if its day ≤ 31 else 4th), Juneteenth (Jun 19, only y≥2021), Independence Day (Jul 4), Labor Day (1st Mon Sep), Thanksgiving (4th Thu Nov), Christmas (Dec 25). Columbus/Veterans Day are explicitly excluded, matching the Fedwire calendar. The function calls only pure helpers (weekday, nth\_weekday), performs no I/O and reads no global state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pair\_universe.us\_holiday` (hash `afbcba6675263f70`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.pivot\_wire.style

- **claim** (`cl\_4ded342937335d51`): In the pivot\_wire parity test module, \`style\` is a total exhaustive bijection from \`RedemptionStyle\` to \`TarfRedemption\`: \`RedemptionStyle::FullGain\` maps to \`TarfRedemption::FullGain\` and \`RedemptionStyle::CappedGain\` maps to \`TarfRedemption::CappedGain\`. It is a pure type-bridge with no allocation, state, or I/O, ensuring the parity harness can construct TARF pricer inputs from the \`RedemptionStyle\` enum used in test scenarios without any silent coercion.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.pivot\_wire.style` (hash `ae4aebf8de135fd7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.brownian\_bridge\_reproduces\_discrete\_covariance

- **claim** (`cl\_d9f880e82b62b531`): brownian\_bridge\_reproduces\_discrete\_covariance verifies that the BrownianBridge weight matrix A satisfies Cov(W(tᵢ), W(tⱼ)) = min(tᵢ, tⱼ) to within 1e-12 for all step counts m ∈ {1,2,4,8,16,17,32} and all index pairs (i,j). Specifically it asserts ‖Aᵢ · Aⱼ − min(tᵢ,tⱼ)‖ \< 1e-12, confirming that the bridge's linear map correctly encodes Brownian covariance structure regardless of irregular step sizes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.brownian\_bridge\_reproduces\_discrete\_covariance` (hash `04aa32f67f95ef7c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.compositions

- **claim** (`cl\_c6f798ba893f12ca`): compositions generates all ordered compositions of a non-negative integer m into exactly s non-negative parts by recursive enumeration. The base case s == 1 returns \[\[m\]\]; the recursive case iterates first ∈ \[0, m\] and prepends first to each composition of (m − first) into (s − 1) parts. The total count is C(m+s−1, s−1) (stars-and-bars). Used by check\_net\_balanced to enumerate all (0,m,s)-net test projections.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.compositions` (hash `3f758ca6523a9118`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.plain\_mc\_rmse

- **claim** (`cl\_f5650ac01e80afc7`): \`plain\_mc\_rmse\` computes the root-mean-square error (RMSE) of plain pseudo-random Monte Carlo pricing against an exact closed-form price. For each of \`c.reps\` independent repetitions it draws a fresh \`PseudoNormals\` PRNG seeded by \`c.base\_seed.wrapping\_add(r.wrapping\_mul(0x100\_0001))\` (ensuring per-rep seed independence without correlation), runs \`c.budget\` paths through the Brownian bridge \`c.bb\`, accumulates discounted payoffs via \`discounted\_payoff\`, averages them to form an MC estimate \`est\`, then accumulates the squared error \`(est - c.exact)²\`. The final return value is \`sqrt(sum\_of\_sq\_err / reps)\` — the population RMSE across all repetitions. This function serves as the reference baseline in the QMC variance-reduction test: it establishes the plain-MC RMSE floor against which the Quasi-MC path is compared.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.plain\_mc\_rmse` (hash `5bac3efb3273bafb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.sobol\_dim1\_is\_van\_der\_corput\_base2

- **claim** (`cl\_035b620e37b60782`): \`sobol\_dim1\_is\_van\_der\_corput\_base2\` proves two structural properties of \`SobolSequence\`: (1) for every i in 0..512, \`point\_u32(i)\[0\]\` equals the bit-reversal of the Gray code \`i ^ (i \>\> 1)\` — the base-2 van der Corput radical inverse; (2) for each k in 1..=8, the first 2^k points partition the dyadic grid {0, 1/2^k, …, (2^k−1)/2^k} with no cell hit twice — a verified (0,k,1)-net property.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc.sobol\_dim1\_is\_van\_der\_corput\_base2` (hash `80ff257a0e5db50d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.asian\_plain\_mc\_rmse

- **claim** (`cl\_3ca2df427d0995c6`): asian\_plain\_mc\_rmse computes the root-mean-square error of a plain (pseudo-random) Monte Carlo Asian-option price estimator over \`reps\` independent replications of \`budget\` paths each, using antithetic-free PseudoNormals draws through a BrownianBridge discretisation. The return value is sqrt(Σ(est\_r − exact)² / reps), i.e., the empirical RMSE against the exact reference, used by the RQMC convergence-order tests to confirm O(N^{-1/2}) plain-MC convergence vs O(N^{-1}) RQMC.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.asian\_plain\_mc\_rmse` (hash `5b401938302d9c0f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.gaussian\_plain\_mc\_rmse

- **claim** (`cl\_f4d2e8ea61d6d586`): gaussian\_plain\_mc\_rmse measures the RMSE of a plain Monte Carlo estimator for a D-dimensional Gaussian moment integral over \`reps\` independent replications of \`budget\` pseudo-random samples. Each replication uses a fresh PseudoNormals stream seeded with base\_seed + r·0x100\_0001. Returns sqrt(Σ(est\_r − exact)²/reps), used as the plain-MC baseline in the RQMC convergence-order comparison.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.gaussian\_plain\_mc\_rmse` (hash `d377a2c7c27aa8c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.gaussian\_rqmc\_rmse

- **claim** (`cl\_d9206c8864c7d92c`): gaussian\_rqmc\_rmse measures the RMSE and grand mean of a Randomised QMC (RQMC) estimator for the same D-dimensional Gaussian moment integral as gaussian\_plain\_mc\_rmse, using scrambled Sobol with Owen-style random shift. Each replication draws budget points from \`seq.stream(seed\_r)\`, applies the inverse normal CDF (inv\_norm\_cdf) pointwise, and evaluates gaussian\_moment\_integrand. Returns (rmse, grand\_mean); the grand\_mean is used to verify unbiasedness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.qmc\_highdim.gaussian\_rqmc\_rmse` (hash `d73892664e297131`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.raft\_snapshot.rebind

- **claim** (`cl\_bb531a6a57136fa9`): \`rebind\` is a test-harness utility that retries \`TcpListener::bind(addr)\` in a tight loop until it succeeds or \`assert\_within\_deadline\` trips, sleeping 10 ms between attempts. It is used exclusively by Raft snapshot tests that need to reclaim a loopback port after node shutdown, where the OS TIME\_WAIT window can cause transient EADDRINUSE.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.raft\_snapshot.rebind` (hash `9fc209f21ed97e53`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.strategy.gk

- **claim** (`cl\_c6f30ed2b3581e41`): The strategy-test oracle \`gk\` is a second independent Garman-Kohlhagen implementation (no \`celnet\_vanilla\` import): Call = S·e^{-r\_f T}·N(d1) − K·e^{-r\_d T}·N(d2); Put = K·e^{-r\_d T}·N(-d2) − S·e^{-r\_f T}·N(-d1), with d1 and d2 identical to the standard formula. Its two callers are multi-leg strategy parity tests, establishing that strategy prices equal the sum of per-leg GK values.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.strategy.gk` (hash `7feb51ae1251c937`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-parity.tests.structured.accumulator\_continuous\_monitoring\_knocks\_out\_more\_than\_discrete

- **claim** (`cl\_c5c990f15ca2e46c`): Rows 16–19 (parity) — four structured-product gates prove the second-generation / TARF book in the open: (16) \`quanto\_closed\_form\_matches\_mc\_and\_collapses\_at\_zero\_correlation\`: quanto vanilla and digital closed forms reproduce an independent MC within 4·σ\_MC+1e-6; at zero correlation ρ=0 the quanto drift vanishes and the quanto price equals the plain vanilla to 1e-12, i.e. \`quanto\_vanilla\_price(opt, &e, QuantoParams::new(σ\_fx,0)) == vanilla\_price(opt, &i)\`. (17) lookback closed forms (floating- and fixed-strike) cross-validated by MC, plus the optionality invariant lookback ≥ vanilla. (18) TARF gap-risk: FullGain settlement is strictly costlier than CappedGain, with a positive expected overshoot on FullGain and zero on CappedGain. (19) Accumulator: continuous (Brownian-bridge) monitoring knocks out more than discrete fixing-only monitoring, so fewer fixings settle.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.accumulator\_continuous\_monitoring\_knocks\_out\_more\_than\_discrete` (hash `0e283ea3a4702e2f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.lookback\_closed\_form\_matches\_mc\_and\_dominates\_vanilla` (hash `809b775e4d689841`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.quanto\_closed\_form\_matches\_mc\_and\_collapses\_at\_zero\_correlation` (hash `eb67496e5b1f6dc7`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.tarf\_gap\_risk\_premium\_is\_priced\_and\_signed` (hash `977f5d7f360ce088`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.error.PluginError.fmt

- **claim** (`cl\_0a9fccdd0ba6a8b5`): PluginError::fmt produces a human-readable prefix-tagged string for each variant: 'invalid input: {w}', 'unsupported: {w}', 'did not converge: {w}', 'calibration failed: {w}', or 'not found: {w}'. The mapping is exhaustive and bijective — every variant carries exactly one distinct prefix.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.error.PluginError.fmt` (hash `b31a3f818801c3ee`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmilePricer.price

- **claim** (`cl\_0ce3f1dbc0bce9fb`): FlatSmilePricer::price implements the generalized Black-Scholes formula: price = S·e^{(b−r)t}·N(d1) − K·e^{−rt}·N(d2) for a call, and K·e^{−rt}·N(−d2) − S·e^{(b−r)t}·N(−d1) for a put, where b is the carry rate and r is the discount rate. For FX inputs (b = r\_dom − r\_for, r = r\_dom) the discounting is byte-identical to the two-factor df\_for / df\_dom form. Validation is always invoked first via Self::validate(inputs)?.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmilePricer.price` (hash `451517ed70f03a9b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmilePricer.validate

- **claim** (`cl\_c3a05e80b45292a1`): FlatSmilePricer::validate enforces a strict domain-guard contract before any pricing: it returns PluginError::InvalidInput if spot, strike, or time-to-expiry is non-finite or ≤ 0, or if either carry rate (discount\_rate() / carry\_rate()) is non-finite, or if vol is non-finite or ≤ 0. All five guard conditions must pass before Ok(()) is returned; any single failure short-circuits via the \`?\`-ready Err path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmilePricer.validate` (hash `54023ece080a2176`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.reference\_call

- **claim** (`cl\_8279fe6d6d7bff65`): reference\_call(inputs: &CarryInputs) -\> f64 is the canonical closed-form carry-generalized call pricer used as the ground-truth oracle in plugin-api tests. It computes: r = inputs.carry.discount\_rate(), b = inputs.carry.carry\_rate(), d₁ = \[ln(S/K) + (b + 0.5σ²)t\]/(σ√t), d₂ = d₁ − σ√t, call = S·e^{(b−r)t}·Φ(d₁) − K·e^{−rt}·Φ(d₂). This is the generalized Garman-Kohlhagen/Black formula parameterized through the carry seam: for FX Carry::FxRates, r = r\_dom and b = r\_dom − r\_for, reproducing the exact FX two-rate arithmetic; for Carry::CostOfCarry, r and b are the stored fields directly. The function is pure (reads only &CarryInputs, routes all transcendentals through celnet\_core::math: sqrt, ln, exp, norm\_cdf; no I/O, no mutation, no allocation).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.reference\_call` (hash `54487c8c55b70c0f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.lib.fx\_rates

- **claim** (`cl\_cae699df4b2883cb`): fx\_rates extracts the (r\_dom, r\_for) pair from a CarryInputs whose carry arm is FxRates, and panics (unreachable!) if the arm is CostOfCarry. It is a pure destructuring accessor — no allocation, no side-effects — used exclusively inside test helpers to disambiguate the carry arm before bumping individual rates.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.lib.fx\_rates` (hash `347a748a2f2db332`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.butterfly\_check

- **claim** (`cl\_66d7d77176ab37a6`): BUTTERFLY no-arbitrage hard-reject on the plugin-API smile contract (ANALYTICS-SPEC §3.4 strike-convexity axis; the SmileModel::check\_no\_arbitrage default path). \`smile::butterfly\_check(model, forward, t, strikes)\` enforces strike-convexity of undiscounted call prices: over every consecutive strike triple (kl,km,kr) on a strictly-increasing grid it forms the second difference \`C(kl) - 2·C(km) + C(kr)\` of \`undiscounted\_call\` at each strike's model-implied vol, and returns Err(Unsupported("butterfly arbitrage")) when that second difference is negative beyond a rounding tolerance (\`is\_close(.,0,1e-9,1e-12)\`) — a negative call convexity is a negative risk-neutral density = butterfly arbitrage. It also hard-rejects non-finite/non-positive forward or t, fewer than 3 strikes, and any non-strictly-increasing or NaN strike (NaN compares false to its neighbour, so it fails the monotone-grid guard). Pure validator: reads its args + the model, returns PluginResult\<()\>, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.butterfly\_check` (hash `2e1282fa32de161c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.undiscounted\_call

- **claim** (`cl\_355021bb87010f93`): undiscounted\_call(forward, k, v, t) computes the undiscounted Black call value off the forward: C = F·Φ(d₁) − K·Φ(d₂), where d₁ = \[ln(F/K) + 0.5v²t\]/(v√t) and d₂ = d₁ − v√t. When vsqt = v·√t ≤ 0 it degenerates to intrinsic (F−K)⁺. It is pure (reads only its four f64 arguments, routes sqrt/ln/norm\_cdf through celnet\_core::math, no I/O, no mutation, no allocation). Used exclusively by butterfly\_check as the density test oracle over the model's own smile vol.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.undiscounted\_call` (hash `8266b5e74eb0be68`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.canonicalize

- **claim** (`cl\_aa9e872e3c044208`): abi::canonicalize(x: f64) -\> f64 is a pure NaN-normalization function: if x.is\_nan() it returns f64::from\_bits(CANONICAL\_NAN\_BITS), otherwise it returns x unchanged. This is the single boundary canonicalization applied to every f64 crossing the host/guest interface — both on import arguments entering the guest and on the price return value exiting the guest.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.canonicalize` (hash `040cbf767cb4be99`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.carry\_to\_abi

- **claim** (`cl\_489caa5ae50563b3`): carry\_to\_abi encodes a Carry enum into the three-word ABI tuple (kind:i32, a:f64, b:f64) used at the host/guest boundary: Carry::FxRates{r\_dom,r\_for} → (CARRY\_KIND\_FX\_RATES, r\_dom, r\_for) and Carry::CostOfCarry{r,b} → (CARRY\_KIND\_COST\_OF\_CARRY, r, b). The integer tag is the sole discriminant the guest reads to branch between FX-rate and cost-of-carry semantics; the two f64 words are always the ordered pair of the enum's fields, preserving all information losslessly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.carry\_to\_abi` (hash `ecbaf6269956f445`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.input\_to\_bytes

- **claim** (`cl\_76b5dcd420f8e5f9`): abi::input\_to\_bytes(inputs: &CarryInputs) -\> \[u8; INPUT\_BYTES\] serializes a CarryInputs struct to a fixed-size little-endian byte array. The layout is: 6 × 8-byte f64 fields (spot, strike, vol, t, carry\_0, carry\_1) each canonicalized before encoding via to\_bits().to\_le\_bytes(), followed by two i32 discriminants packed after the numeric block (underlying\_to\_abi(&inputs.underlying) at base, carry\_kind at base+4). This is the sole serialization format shared between host and Wasm guest.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.input\_to\_bytes` (hash `f4328395d3ec47fc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.opt\_from\_abi

- **claim** (`cl\_2125840151ce3a99`): opt\_to\_abi and opt\_from\_abi form a bijection for OptionType: Call↔0 and Put↔1. opt\_to\_abi maps every OptionType to its ABI i32 tag exhaustively (no default arm). opt\_from\_abi is the partial inverse: 0→Some(Call), 1→Some(Put), any other i32→None. Together they define the stable, lossless encoding of option direction across the host/guest ABI boundary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.opt\_from\_abi` (hash `081b877567192f6b`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.opt\_to\_abi` (hash `c1133dcec9734a13`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.underlying\_to\_abi

- **claim** (`cl\_461cf916545383a1`): underlying\_to\_abi maps every Underlying variant to a distinct integer asset-class discriminant: Fx→UNDERLYING\_CLASS\_FX, Metal→UNDERLYING\_CLASS\_METAL, Equity→UNDERLYING\_CLASS\_EQUITY, Commodity→UNDERLYING\_CLASS\_COMMODITY, DigitalAsset→UNDERLYING\_CLASS\_DIGITAL\_ASSET. This is the sole point where the host signals the asset class to the guest; it is exhaustive and const, so no new variant can be silently ignored at compile time.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.underlying\_to\_abi` (hash `62e8fc1ea807e08c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel\<M\>.price

- **claim** (`cl\_53a57fcf0afebc85`): CAPABILITY (plugin Tier-0 native dispatch): celnet-plugin-host::native::NativeModel\<M\>::price is the Tier-0 native plugin pricing dispatch — a pure, side-effect-free delegation that forwards (OptionType, &CarryInputs) to the inner PricingModel and returns HostResult\<f64\> with no host-side I/O, allocation-mutation, or logging. It proves a user-supplied native model and the fuel-metered Tier-2 wasm-sandboxed model both register and dispatch through the ONE PricingModel::price(opt, CarryInputs) seam of the same ModelRegistry, so native and wasm twins agree bit-for-bit through one registry (native\_and\_wasm\_twins\_agree\_through\_one\_registry); the Tier-2 sandbox path itself is fuel-metered and is therefore deliberately NOT claimed pure here.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel\<M\>.price` (hash `123f57703c4376c5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel\<M\>.price\_and\_greeks

- **claim** (`cl\_52c4be9b881df2b0`): CAPABILITY (plugin Tier-0 native dispatch): NativeModel\<M\>.price\_and\_greeks is the Tier-0 native plugin-host dispatch entry behind the frozen HostModel trait — pure (&self, OptionType, &CarryInputs) -\> HostResult\<CarryGreeks\>, forwarding to the in-process registered model with zero sandboxing overhead. Pairs with the Tier-2 wasmi fuel-metered sandbox (WasmModel) under one ModelRegistry; the native-and-wasm twins agree through that one registry. No WRITES on the dispatch path — the gate self-invalidates if the native forwarder gains a side effect.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel\<M\>.price\_and\_greeks` (hash `36205fc83b9bec2b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.insert

- **claim** (`cl\_85c874d73fdd2ac5`): ModelRegistry::insert enforces a unique-id invariant: it rejects any model whose ModelDescriptor.id already exists in the registry with HostError::Model(PluginError::InvalidInput("duplicate model id")), and otherwise appends the model and its descriptor to parallel vecs, returning the new ModelId. ModelRegistry::model performs O(n) lookup by scanning the descriptor vec for the matching id, returning the corresponding entry or HostError::Model(PluginError::NotFound("model id")).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.insert` (hash `c60ed017d0771f7d`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.model` (hash `dfd89dba127adfdb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.replay.rate\_parts

- **claim** (`cl\_988334f5d267bec1`): rate\_parts decomposes a RateSensitivities value into the three-word tuple (tag:u8, a:f64, b:f64) used in the replay bit-equality check: Fx{rho\_dom,rho\_for}→(0,rho\_dom,rho\_for), Carry{discount\_rho,carry\_rho}→(1,discount\_rho,carry\_rho). The u8 tag distinguishes the two rate regimes; combined with the two f64 fields it provides a complete, lossless serialisation used by greeks\_bits\_eq to assert replay determinism.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.replay.rate\_parts` (hash `f4a70e660d079f82`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.AbiStatus.from\_code

- **claim** (`cl\_bf4b7e33aa91bc1a`): AbiStatus::from\_code decodes the i32 return code of a guest pricing call into either Ok(()) or a typed PluginError: 0→Ok, -2→PluginError::Unsupported (guest lacks Greeks), -3→PluginError::DidNotConverge, and all other negative values (including -1, the conventional domain-rejection code) → PluginError::InvalidInput. This mapping is the definitive ABI contract for guest return codes and determines how model errors are surfaced to callers.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.AbiStatus.from\_code` (hash `7415dbf9a82299c9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.classify\_call\_error

- **claim** (`cl\_5a6974cd837e8773`): classify\_call\_error translates a wasmi::Error into a typed HostError without panicking or hanging: OutOfFuel→HostError::FuelExhausted, GrowthOperationLimited→HostError::ResourceLimit (payload is the short description), and any other trap or non-trap error→HostError::Trapped. This is the sole site that converts raw wasmi error codes into the host's public error taxonomy, ensuring every guest execution failure is a bounded, typed outcome.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.classify\_call\_error` (hash `01cd2d3707e5e7c3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.sandbox\_config

- **claim** (`cl\_d9d1a7160b5abe48`): ADR (plugin-host sandbox = wasmi, fuel-metered, narrowed feature set). \`sandbox\_config()\` is the single source of the guest-execution sandbox policy and is a pure builder: it constructs a fresh wasmi \`Config\`, enables \`consume\_fuel(true)\` (deterministic instruction metering, the basis of the per-call FuelBudget that bounds even a \`(start)\` function — see WasmModel::load), and explicitly disables the unused proposals (memory64, bulk-memory, reference-types, tail-call) to narrow the accepted module surface. It reads and writes no external state — its result depends only on the wasmi defaults — so the sandbox policy is reproducible call-to-call. DECISION/RATIONALE: the Tier-2 user-plugin host is built on wasmi (a pure-Rust, no-unsafe, no-JIT interpreter) rather than wasmtime: wasmi gives deterministic fuel metering and a small, auditable, JIT-free attack surface that suits a mission-critical pricing host where a plugin must be sandboxed and time-bounded, accepting interpreter throughput for that safety. Tier-0 native models run un-sandboxed for the hot path; untrusted user code is confined here. (Memory: plugin-host=wasmi; wasmtime rejected.)
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.sandbox\_config` (hash `e0302691ebc4548a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.TrivialForward.price

- **claim** (`cl\_80cc3ffe91faa175`): TrivialForward::price computes the FX forward intrinsic value for a call or put without any volatility term: price = sign·(S·df\_for − K·df\_dom) where sign=+1 for Call and −1 for Put. This is the zero-vol limit of the GBS formula, used as a reference implementation to validate that the host correctly passes spot, strike, and both discount factors to the guest through the ABI.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.TrivialForward.price` (hash `e616c6324d2e85a9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.TrivialForward.price\_and\_greeks

- **claim** (`cl\_55a5095d85be2c16`): TrivialForward::price\_and\_greeks returns the exact analytic sensitivities of the zero-vol forward payoff for FX underlyings: delta\_spot=sign·df\_for, delta\_forward=sign, gamma=vanna=volga=charm=speed=zomma=color=0, vega=0, theta=sign·(r\_dom·K·df\_dom − r\_for·S·df\_for), rho\_dom=sign·K·T·df\_dom, rho\_for=sign·(−S·T·df\_for), with RateSensitivities::Fx populated from r\_dom/r\_for read via fx\_rates. This validates that the host reads the carry-field words at the correct ABI offsets (32/40) and reconstructs the Fx sensitivity struct correctly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.TrivialForward.price\_and\_greeks` (hash `50b68f72c6cc431c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.fx\_rates

- **claim** (`cl\_65290e5f919cf115`): fx\_rates is a test-fixture extractor that panics with 'unreachable: host fixtures are FX' if the carry is not FxRates — it enforces that all baseline sandbox test inputs use the FX-rate carry arm, making the panic a guard against accidentally exercising the cost-of-carry path in FX-oriented tests. For FxRates inputs it returns (r\_dom, r\_for) exactly as stored.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.fx\_rates` (hash `5a73fc77797e0cc0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.gbs\_oracle

- **claim** (`cl\_f7a0a9c8d91f9320`): gbs\_oracle implements the independent reference formula for the Garman-Kohlhagen/generalised Black-Scholes (GBS) model used as the golden-record oracle in sandbox tests: d1 = (ln(S/K) + (b + ½σ²)T) / (σ√T), d2 = d1 − σ√T, price = sign·(S·e^{(b−r)T}·N(sign·d1) − K·e^{−rT}·N(sign·d2)) where sign=+1 for Call and −1 for Put. The tolerance asserted against the guest is ≤1e-12 relative error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-plugin-host.tests.sandbox.gbs\_oracle` (hash `58e35cc48e4a6e99`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.AtmConvention.from

- **claim** (`cl\_c4011ff357b653d7`): The wire→types AtmConvention mapping is a total 1:1 lift of the two ATM conventions: \`From\<WireAtmConvention\> for AtmConvention\` matches AtmForward→AtmForward and DeltaNeutralStraddle→DeltaNeutralStraddle explicitly, no wildcard. DeltaNeutralStraddle is the dominant interbank ATM (its strike is the premium-adjusted-aware F·e^{±½σ²T}, docs/CONVENTIONS.md), distinct from AtmForward (K=F). This seam carries that selection from the wire onto \`celnet\_types::AtmConvention\` with no possibility of silent enum drift. Pure: a match returning the mapped enum, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.AtmConvention.from` (hash `e903737d2dd40385`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.Cut.from

- **claim** (`cl\_74633e830a125578`): The wire→types Cut mapping is a total 1:1 lift of the two expiry cuts: \`From\<WireCut\> for Cut\` matches NewYork1000→NewYork1000 and Tokyo1500→Tokyo1500 explicitly, no wildcard. NY 10:00 is the standard interbank OTC cut; Tokyo 15:00 is standard for JPY-region/Asian business (docs/CONVENTIONS.md). The cut is per-(pair,tenor) configuration, not a global default; this seam carries it from the wire onto \`celnet\_types::Cut\` with no silent drift. Pure: a match returning the mapped enum, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Cut.from` (hash `dd98a13158f4a32b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.DayCount.from

- **claim** (`cl\_2bb7205a853dffe2`): The wire→types DayCount mapping is a total 1:1 lift of the two day-count bases: \`From\<WireDayCount\> for DayCount\` matches Act365Fixed→Act365Fixed and Act360→Act360 explicitly, no wildcard. ACT/365-fixed is kept distinct from ACT/360 because vol-time accrual (ACT/365) must not be conflated with money-market accrual basis (ACT/360) — a deliberate separation (docs/CONVENTIONS.md). This seam carries the basis from the wire onto \`celnet\_types::DayCount\`. Pure: a match returning the mapped enum, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DayCount.from` (hash `75c2df964bb50955`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.DeltaConvention.from

- **claim** (`cl\_a9e832fd4fe68c0d`): The wire→types DeltaConvention mapping is a total, order-preserving 1:1 lift of the four FX delta conventions: \`From\<WireDeltaConvention\> for DeltaConvention\` matches every wire variant explicitly (SpotUnadjusted→SpotUnadjusted, ForwardUnadjusted→ForwardUnadjusted, SpotPremiumAdjusted→SpotPremiumAdjusted, ForwardPremiumAdjusted→ForwardPremiumAdjusted) with no wildcard arm, so the proto enum and the celnet-types enum can never silently drift — adding a delta convention on either side is a compile error here. This is the single conversion seam carrying the per-(pair,tenor) delta convention from the wire onto \`celnet\_types::DeltaConvention\` (the convention encoded in the type, never a global default; docs/CONVENTIONS.md). Pure: a match over the input value returning the mapped enum, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DeltaConvention.from` (hash `047ec0879a8b9873`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.Metal.from

- **claim** (`cl\_6d8a61847cd4d4aa`): \`WireMetal::from(Metal)\` and \`Metal::from(WireMetal)\` are symmetric, infallible bijections across all four precious-metal identifiers: \`Gold\`, \`Silver\`, \`Platinum\`, \`Palladium\`. Both directions are exhaustive, guaranteeing lossless round-trip.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Metal.from` (hash `30247f94a6aa3943`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireMetal.from` (hash `cbe0d942f5d5bcc1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.OptionType.from

- **claim** (`cl\_a9811706e90a8ef6`): \`WireOptionType::from(OptionType)\` and \`OptionType::from(WireOptionType)\` are symmetric, infallible bijections: \`Call ↔ Call\`, \`Put ↔ Put\`. Together they guarantee lossless round-trip for the 2-variant option-type enum across the wire boundary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.OptionType.from` (hash `244d5b7e0189de55`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireOptionType.from` (hash `701428b1c7359f73`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.PremiumStyle.from

- **claim** (`cl\_dba740516024d905`): The wire→types PremiumStyle mapping is a total, order-preserving 1:1 lift of the four premium quotation styles: \`From\<WirePremiumStyle\> for PremiumStyle\` matches every wire variant explicitly (DomesticPips→DomesticPips, PercentForeign→PercentForeign, PercentDomestic→PercentDomestic, ForeignPips→ForeignPips) with no wildcard, so the proto and celnet-types premium-style enums cannot drift. This carries the premium style — which decides whether the premium is paid in the FOR/base ccy and therefore carries FX risk (the \`is\_premium\_adjusted\` distinction, docs/CONVENTIONS.md) — from the wire onto \`celnet\_types::PremiumStyle\`. Pure: a match returning the mapped enum, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.PremiumStyle.from` (hash `4c13ce8fd992f041`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.RateSensitivities.try\_from

- **claim** (`cl\_24944c11574ac461`): \`WireRateSensitivities::from(RateSensitivities)\` and \`RateSensitivities::try\_from(WireRateSensitivities)\` are an asymmetric pair encoding interest-rate sensitivity vectors: the FX arm carries \`(rho\_dom, rho\_for)\` under \`rate\_sensitivities::Sensitivities::Fx\`; the carry arm carries \`(discount\_rho, carry\_rho)\` under \`rate\_sensitivities::Sensitivities::Carry\`. The \`from\` direction is infallible; \`try\_from\` fails with \`WireError::MissingField { field: "RateSensitivities.sensitivities" }\` when the oneof is unset.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.RateSensitivities.try\_from` (hash `de8cccde8b059217`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireRateSensitivities.from` (hash `53b18f9fa3f959b4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.Settlement.from

- **claim** (`cl\_68098cedce70028d`): The wire→types Settlement mapping is a total 1:1 lift of the two settlement styles: \`From\<WireSettlement\> for Settlement\` matches Deliverable→Deliverable and NonDeliverable→NonDeliverable explicitly, no wildcard. NonDeliverable encodes an NDO that cash-settles at a published fixing (EMTA/WMR), versus a physically Deliverable option (docs/CONVENTIONS.md). This seam carries the settlement style from the wire onto \`celnet\_types::Settlement\` with no silent enum drift. Pure: a match returning the mapped enum, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Settlement.from` (hash `14a3055e69b393dd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.SettlementStyle.from

- **claim** (`cl\_168997056873c9af`): \`WireSettlementStyle::from(SettlementStyle)\` and \`SettlementStyle::from(WireSettlementStyle)\` are symmetric bijections for the two crypto settlement styles: \`Linear ↔ Linear\` (premium and P&L in notional ccy), \`InverseCoin ↔ InverseCoin\` (premium and P&L in the coin itself).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.SettlementStyle.from` (hash `a3a97b1973b0fb83`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireSettlementStyle.from` (hash `11bc047780875d65`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.SmileModel.try\_from

- **claim** (`cl\_15cb0e006cf31500`): \`WireSmileModel::from(SmileModel)\` is an infallible bijection across all five smile-model variants: \`MarketHedge\`, \`StochasticVol\`, \`Parametric\`, \`ParametricSurface\`, \`ExtendedSurface\`. \`SmileModel::try\_from(WireSmileModel)\` is the exhaustive inverse — always \`Ok\` because the input already carries a valid discriminant (no unknown-tag path at this level).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.SmileModel.try\_from` (hash `1540f0e61401d413`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireSmileModel.from` (hash `b4a7ac980a8dff51`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.Tenor.try\_from

- **claim** (`cl\_6c11b7849e78c897`): \`WireTenor::from(Tenor)\` serialises a \`Tenor\` to the wire representation with the following encoding contract: zero-count spot tenors (\`Overnight\`, \`TomNext\`, \`SpotNext\`) set \`count=0\` and \`broken\_date=None\`; counted tenors (\`Weeks(n)\`, \`Months(n)\`, \`Years(n)\`) set \`unit\` and widen \`n: u16\` to \`count: u32\`; \`Imm(n)\` widens \`n: u8\` to \`count: u32\`; \`BrokenDate(b)\` sets \`count=0\` and \`broken\_date=Some(WireBrokenDate::from(b))\`. The reverse (\`Tenor::try\_from\`) narrows \`count\` back with range checks, making to-wire infallible and from-wire fallible.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Tenor.try\_from` (hash `98292453431b0ea6`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireTenor.from` (hash `68eb39cdb71c9075`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.Underlying.try\_from

- **claim** (`cl\_284b36023f7aac4f`): \`Underlying::try\_from(WireUnderlying)\` is the fallible inverse of \`WireUnderlying::from\`: it matches the oneof \`ref\` field and delegates inner validation to asset-class \`try\_from\` impls (\`CcyPair::try\_from\`, \`MetalPair::try\_from\`, \`EquityRef::try\_from\`, \`CommodityRef::try\_from\`); \`DigitalAsset\` uses infallible \`CryptoPair::from\`. A missing \`ref\` (oneof unset / \`None\`) produces \`WireError::MissingField { field: "Underlying.ref" }\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Underlying.try\_from` (hash `4710d15bd1cab0d7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireAtmConvention.from

- **claim** (`cl\_ee99b479aca8b413`): \`WireAtmConvention::from(AtmConvention)\` is an infallible bijection for the two ATM convention variants: \`AtmForward ↔ AtmForward\`, \`DeltaNeutralStraddle ↔ DeltaNeutralStraddle\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireAtmConvention.from` (hash `8a44b6fe9acb4d63`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireCarryModel.from

- **claim** (`cl\_b9dc8493cec85796`): \`WireCarryModel::from(Carry)\` encodes the two-arm carry model with a field-stripping contract: for \`Carry::FxRates { r\_for, .. }\` only the foreign rate \`r\_for\` is serialised onto the wire (the domestic discount rate \`r\_dom\` is carried out-of-band on \`MarketContext\`/\`VanillaInputs\` and is intentionally dropped here); for \`Carry::CostOfCarry { b, .. }\` the cost-of-carry scalar \`b\` is wrapped in \`carry\_model::Model::Generalized(CostOfCarry { b })\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireCarryModel.from` (hash `7aece199a058ab48`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireCut.from

- **claim** (`cl\_89b98d9f5f84727a`): \`WireCut::from(Cut)\` is an infallible bijection for the two FX expiry cut variants: \`NewYork1000 ↔ NewYork1000\`, \`Tokyo1500 ↔ Tokyo1500\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireCut.from` (hash `65cc1b39db5079cd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireDayCount.from

- **claim** (`cl\_5aa1462e906e394c`): \`WireDayCount::from(DayCount)\` is an infallible bijection for the two supported day-count conventions: \`Act365Fixed ↔ Act365Fixed\`, \`Act360 ↔ Act360\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireDayCount.from` (hash `211b8b98286969cb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireDeltaConvention.from

- **claim** (`cl\_63d5d7b83deb2a41`): \`WireDeltaConvention::from(DeltaConvention)\` is an infallible bijection across all four FX delta conventions: \`SpotUnadjusted\`, \`ForwardUnadjusted\`, \`SpotPremiumAdjusted\`, \`ForwardPremiumAdjusted\`. The match is exhaustive (no wildcard), so the compiler enforces that future variants must be mapped.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireDeltaConvention.from` (hash `1300d8971e36eb65`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireError.fmt

- **claim** (`cl\_71450f56f22386c8`): \`WireError::fmt\` produces a distinct, human-readable message for all six error variants: \`InvalidCcy\` → \`"invalid currency code for \\\`{field}\\\`: {value:?}"\`, \`UnknownEnum\` → \`"unknown {kind} enum tag: {tag}"\`, \`MissingField\` → \`"missing required field \\\`{field}\\\`"\`, \`OutOfRange\` → \`"value {value} out of range for \\\`{field}\\\`"\`, \`WrongUnderlying\` → \`"{underlying} underlying is not valid for a {product\_family} product"\`, \`InvalidTerms\` → \`"invalid {product\_family} terms: {constraint}"\`. The match is exhaustive, so adding a new variant is a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireError.fmt` (hash `54cb37e2f30e6a6d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WirePremiumStyle.from

- **claim** (`cl\_d868dbab946b7780`): \`WirePremiumStyle::from(PremiumStyle)\` is an infallible bijection across all four FX premium quotation styles: \`DomesticPips\`, \`PercentForeign\`, \`PercentDomestic\`, \`ForeignPips\`. Exhaustive match, no wildcard.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WirePremiumStyle.from` (hash `c995db5e0624ea2f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireSettlement.from

- **claim** (`cl\_f4257468eacff3f9`): \`WireSettlement::from(Settlement)\` is an infallible bijection for the two settlement modes: \`Deliverable ↔ Deliverable\`, \`NonDeliverable ↔ NonDeliverable\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireSettlement.from` (hash `c3c8c30ea26d831f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireUnderlying.from

- **claim** (`cl\_2fa525728f0cacee`): \`WireUnderlying::from(Underlying)\` fans out to asset-class-specific wire constructors — \`WireUnderlying::fx(WireCcyPair)\`, \`::metal(WireMetalPair)\`, \`::equity(WireEquityRef)\`, \`::commodity(WireCommodityRef)\`, \`::digital\_asset(WireCryptoPair)\` — delegating inner field conversion to the corresponding \`From\` impls. The function is infallible because all inner conversions are infallible at this stage.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.WireUnderlying.from` (hash `7f05d5f388befadd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.cut\_round\_trips

- **claim** (`cl\_b6b44fb4bedab9ba`): \`cut\_round\_trips\` pins the wire↔types Cut conversion as a total round-trip: for every \`celnet\_types::Cut\` variant (NewYork1000, Tokyo1500) \`Cut::from(WireCut::from(c)) == c\`. This guards that the two \`From\` directions stay mutually inverse, so the cut convention survives a wire encode/decode unchanged — the one-contract guarantee for the expiry-cut convention (no versioning, single current mapping). Pure test: constructs values and asserts equality, mutating no external state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.cut\_round\_trips` (hash `41e8f99c4b06995f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.settlement\_round\_trips

- **claim** (`cl\_c3ec2cb23c53937a`): \`settlement\_round\_trips\` pins the wire↔types Settlement conversion as a total round-trip: for every \`celnet\_types::Settlement\` variant (Deliverable, NonDeliverable) \`Settlement::from(WireSettlement::from(s)) == s\`. It guards that the two \`From\` directions remain mutually inverse so the deliverable/non-deliverable (NDO cash-settled) distinction survives a wire encode/decode unchanged — the single-contract guarantee for the settlement convention. Pure test: constructs values and asserts equality, no external mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.settlement\_round\_trips` (hash `1127f1902a74d004`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_deliverable\_underlying

- **claim** (`cl\_402926d2c1359854`): PILLAR (GUIDE.md guardrail 9 — "No versioned APIs: exactly ONE clean, current contract"; guardrail 8 — purpose-named, vendor-neutral). \`validate\_deliverable\_underlying(underlying)\` is a pure, total projection of the single wire \`Underlying\` oneof onto the deliverable linear-forward book: it reads only the borrowed \`WireUnderlying\` and returns a \`Result\`, mutating nothing (no WRITES edges). On the one unversioned contract it accepts exactly the deliverable leg-pair arms — Fx and Metal (decoding to \`Underlying::Fx\`/\`Underlying::Metal\`) — and for every cross-asset arm (Equity, Commodity, DigitalAsset) returns a typed \`WireError::WrongUnderlying { product\_family: "deliverable forward", .. }\` rather than silently coercing the asset-class identity under deliverable-FX arithmetic; an absent ref yields \`WireError::MissingField\`. It is the deliverable-book twin of the already-governed \`validate\_fx\_underlying\` (cl\_1ff652eea3ffb73e) on the same single contract — there is no schema\_version, no N/N-1 negotiation: a malformed or wrong-asset request is rejected, never version-coerced. Self-invalidating: if this acquires a side effect the purity gate flips it off.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_deliverable\_underlying` (hash `45b4d9665143bc0d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_fx\_underlying

- **claim** (`cl\_fedeb5bce77637ac`): ADR-0007 (one clean unversioned contract — proto side). \`validate\_fx\_underlying\` is a pure, total projection of the wire \`Underlying\` oneof onto the FX-option product: it reads only the borrowed \`WireUnderlying\` and returns a \`Result\`, mutating nothing. It accepts the Fx and Metal arms (decoding to \`Underlying::Fx\`/\`Underlying::Metal\`), and for every cross-asset arm (Equity, Commodity, DigitalAsset) it returns a typed \`WireError::WrongUnderlying\` rather than discarding the asset-class identity or silently coercing it under FX arithmetic; an absent ref yields \`WireError::MissingField\`. The match enumerates all oneof arms explicitly (no wildcard), so adding a new asset class to the wire is a compile error here, not a silent mis-route. DECISION/RATIONALE: there is exactly ONE current wire contract and no version negotiation — the rich cross-asset \`underlying\` oneof carries the single asset-class discriminator, and (where a legacy \`pair\` key still appears) precedence is fixed at underlying≻pair (see instrument\_underlying\_from\_json), never an N/N-1 schema\_version handshake. The contract evolves in place and deploys as one uniform version. (Guardrail: no versioned APIs; one clean current contract.)
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_fx\_underlying` (hash `9949bdbe09803a14`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_listed\_future\_terms

- **claim** (`cl\_678645eb0faa23cb`): On the one wire contract, a listed-future option's terms are accepted only when both maturities are well-formed and consistently ordered: \`validate\_listed\_future\_terms\` requires the option \`expiry\_years\` to be finite and strictly positive, and the future's \`future\_expiry\_years\` to be finite and \`\>= expiry\_years\` (the underlying future must outlive the option), else it returns \`WireError::InvalidTerms\`; a missing \`future\_symbol\` yields \`WireError::MissingField\` and an out-of-range margining tag yields \`WireError::UnknownEnum\`. Pure: it reads only the wire option and the expiry argument and returns a \`Result\`, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_listed\_future\_terms` (hash `f78664be87825ba6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_perpetual\_terms

- **claim** (`cl\_a3f799a9069b0fed`): On the one wire contract, a perpetual-option instrument must carry \`expiry\_years == 0\` (a perpetual has no expiry): \`validate\_perpetual\_terms\` rejects any non-zero value with \`WireError::InvalidTerms\`. Because the check is \`expiry\_years != 0.0\` (which is also true for NaN), a NaN expiry is rejected, never silently waved through as "no expiry". Pure: it reads only its \`f64\` argument and returns a \`Result\`, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate\_perpetual\_terms` (hash `db17d3f7c6702c8a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-proto.src.helpers.CarryModel.fx\_r\_for

- **claim** (`cl\_57752ac4cf679cca`): \`CarryModel::fx\_r\_for\` extracts the foreign-rate scalar from the \`FX\` carry arm of the protobuf \`CarryModel\` message: returns \`Some(fx.r\_for)\` if \`self.model == Some(carry\_model::Model::Fx(fx))\`, and \`None\` for all other arms (generalized cost-of-carry) or an unset model. This is the primary accessor used by the pricer to retrieve the FX foreign discount rate from a serialised carry model.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.CarryModel.fx\_r\_for` (hash `82f71810e1a4cc59`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-proto.src.helpers.RateSensitivities.fx\_rhos

- **claim** (`cl\_f567fa45d5d4eba7`): The wire/proto rate-sensitivity contract preserves FX's TWO-rho structure end-to-end: \`RateSensitivities::fx\_rhos\` returns \`Some((rho\_dom, rho\_for))\` only when the rate-sensitivities oneof is the \`Fx\` variant, surfacing BOTH the domestic-rate sensitivity ∂V/∂r\_d and the foreign-rate sensitivity ∂V/∂r\_f as a distinct pair (never a single collapsed equity-style rho), and \`None\` otherwise. This is the on-the-wire counterpart to the engine's \`celnet-vanilla::greeks\` two-rho output (Garman-Kohlhagen 1983; ANALYTICS-SPEC §2.1): the foreign rate enters as the continuous dividend yield on the foreign-currency leg, the two rhos carry opposite signs, and the typed oneof prevents any client from reading a one-rho FX sensitivity. Pure: it reads only \`&self.sensitivities\` and constructs an Option tuple, performing no allocation, I/O, or external mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.RateSensitivities.fx\_rhos` (hash `eae802f684bb463a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_commodity

- **claim** (`cl\_57693604d59a6042`): The four \`Underlying\` accessors (\`as\_fx\`, \`as\_metal\`, \`as\_equity\`, \`as\_commodity\`) implement a mutually-exclusive discriminated-union projection over the protobuf \`Underlying.ref\` oneof: each returns \`Some(&T)\` if and only if the stored variant matches the requested asset class, and \`None\` for every other variant including \`None\` (unset). Because the oneof can hold at most one arm, at most one accessor returns \`Some\` for any given \`Underlying\` message — the set of all four accessors partitions the non-None population into disjoint asset classes. Source pattern: \`Some(underlying::Ref::Fx(p)) =\> Some(p), Some(Metal\|Equity\|Commodity\|DigitalAsset\|None) =\> None\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_commodity` (hash `4003c877b594e1da`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_equity` (hash `e61542e7ad4493e8`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_fx` (hash `5a1148954cdf6c0c`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_metal` (hash `a81de52b44dfa2f4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_digital\_asset

- **claim** (`cl\_a5194770c2d0abcb`): \`Underlying::as\_digital\_asset\` is the exhaustive, total projection of the underlying oneof onto its digital-asset arm: it returns \`Some(&CryptoPair)\` only for the \`DigitalAsset\` variant and \`None\` for every other asset class (Fx, Metal, Equity, Commodity) and for an absent ref. The match arms enumerate all variants explicitly (no wildcard), so adding a new asset class to the one contract is a compile error here rather than a silent mis-projection. Pure: it borrows \`&self\` and returns a borrow, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:34Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as\_digital\_asset` (hash `12f7369a32474021`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:34Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:34Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.build

- **claim** (`cl\_0018e31f986ba688`): \`BrownianBridge::new(m, t\_total)\` constructs the bridge plan by bisection: it first places W(T) = W(t\_{m-1}) conditioned on W(0)=0 with std = sqrt(t\_{m-1}), then recursively bisects interior index intervals. Each \`BridgeStep\` records (out, left, right, left\_w, right\_w, std) where left\_w = 1 − frac, right\_w = frac = (t\_mid−t\_left)/(t\_right−t\_left), std = sqrt((t\_mid−t\_left)(t\_right−t\_mid)/(t\_right−t\_left)). The resulting plan has exactly m steps covering every time index, and \`BrownianBridge::build(&z, &mut path)\` executes it in plan order as \`path\[out\] = left\_w·path\[left\] + right\_w·path\[right\] + std·z\[k\]\`, consuming m independent N(0,1) draws.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.build` (hash `2a4beaa1fa769451`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.new` (hash `62fb07a776449b17`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.bisect` (hash `dfa7cdc6903503b9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.weight\_matrix

- **claim** (`cl\_31703fcb0b5395f3`): BrownianBridge::weight\_matrix() materialises the lower-triangular Cholesky factor L of the discrete Brownian covariance matrix C\[i\]\[j\] = min(t\_i, t\_j) by replaying the construction plan column-by-column with unit normal inputs. The method feeds each k-th canonical basis vector e\_k through the bridge recurrence (mean = left\_w \* left\_val + right\_w \* right\_val; std contribution only at step k), then transposes the m×m result so that A\[i\]\[k\] = cols\[k\]\[i\]. The algebraic identity (L L^T)\[i\]\[j\] = min(t\_i, t\_j) holds to within 1e-12 absolute error for all tested step counts m and horizons T (verified by \`covariance\_identity\` and \`bridge\_factorization\_reproduces\_brownian\_covariance\`). The principal-bisection convention forces the first column to satisfy L\[i\]\[0\] = t\_i / sqrt(T), encoding the terminal-normal loading.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.weight\_matrix` (hash `fa710c60b0d0c45e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.covariance\_identity

- **claim** (`cl\_719cb242b0690d5f`): covariance\_identity() is a self-contained unit oracle that asserts weight\_matrix() produces an exact Brownian covariance factor: for every pair (i, j) in 0..m, the dot product of rows i and j of L equals min(t\_i, t\_j) to within 1e-12 absolute tolerance. This is verified for m ∈ {1, 2, 4, 7, 16} with T=2.0, covering non-power-of-two step counts.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.covariance\_identity` (hash `ae76dbd5bad15160`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.lib.rqmc\_estimate

- **claim** (`cl\_dca95a3b8a93a446`): \`rqmc\_estimate\` derives independent per-replication scramble seeds via \`splitmix(base\_seed.wrapping\_add((r as u64).wrapping\_mul(0x9e37\_79b9\_7f4a\_7c15)))\`, where splitmix is the Murmur3/SplitMix64 finalizer. This makes each replication's scrambled Sobol sequence statistically independent of the others; the inter-replication mean and sample variance of the \`replications\` per-replication averages provide the estimate and its standard error (std\_error = sqrt(sample\_var / replications)). With replications=1 the standard error is NaN (not estimable from one replication).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.lib.rqmc\_estimate` (hash `f10360d438ad30b6`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.lib.splitmix` (hash `6bd479bf6c3a9dab`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolSequence.new

- **claim** (`cl\_56a97b5aacea8694`): \`SobolSequence::new(dim)\` initialises direction numbers for up to MAX\_DIM dimensions from the embedded Joe-Kuo table. Dimension 1 uses the identity numbers v\_k = 2^{31-k} (van der Corput sequence). Dimensions 2..=dim apply the Joe-Kuo recurrence on 32-bit left-justified integers V\[k\] = m\_k · 2^{32-k}: for k \> s, V\[k\] = V\[k-s\] XOR (V\[k-s\] \>\> s) XOR ⊕\_{i=1}^{s−1} a\_i·V\[k-i\], where a\_i = (a \>\> (s-1-i)) & 1. The constructor panics for dim=0 or dim \> MAX\_DIM and is otherwise pure: no side effects, no I/O, no shared mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolSequence.new` (hash `e978cdecc09469c3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolStream\<'\_\>.next\_point

- **claim** (`cl\_de47bb1bf10389ed`): \`SobolStream::next\_point(&mut self, out: &mut \[f64\])\` advances the stateful Gray-code Sobol iterator by one point. For i=0 all state integers are zeroed (the all-zero Gray-code point); for i\>0, c = trailing\_zeros(i) and each coordinate j XORs in direction number v\[j\]\[c\]. Each state integer is then Owen-scrambled per dimension and mapped to (0,1) via \`u32\_to\_open\_unit(u) = (u as f64 + 0.5) \* 2^{-32}\`, guaranteeing the open unit interval (never 0 or 1, so inv\_norm\_cdf never produces ±∞). The mutation is entirely to self.state and out — no allocation, no shared mutation beyond the stream itself.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolStream\<'\_\>.next\_point` (hash `9371340fd9d6a013`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.u32\_to\_open\_unit` (hash `5b2aa1418288d335`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.owen\_scramble\_u32

- **claim** (`cl\_0c7547da6bdefabe`): \`owen\_scramble\_u32(x: u32, dim: u64, seed: u64) -\> u32\` applies a bit-by-bit Owen scramble to Sobol integer \`x\` for dimension \`dim\` under scramble key \`seed\`. It processes all 32 bits MSB-first: for each depth \`d\`, flip = LSB of mix64(base ^ (d\<\<40) ^ (prefix\<\<1)), out\_bit = in\_bit XOR flip, prefix accumulates emitted bits. The function is pure (reads only its three scalar arguments, no shared mutation) and deterministically preserves the dyadic-prefix structure required for the scrambled sequence to remain (t,s)-equidistributed: the scrambled prefix of k bits depends only on the original k-bit prefix, never on deeper bits.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.owen\_scramble\_u32` (hash `5ded3e8060fb7860`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.stream\_matches\_direct\_unscrambled

- **claim** (`cl\_dbe8f0be4ad614fb`): stream\_matches\_direct\_unscrambled() proves that the Sobol streaming interface (point\_u32) is algebraically equivalent to the direct gray-code recurrence. It independently reconstructs the unscrambled integer state by replaying the XOR recurrence state\[j\] ^= v\[j\]\[trailing\_zeros(i)\] for i=1..255, then asserts bit-identical equality with seq.point\_u32(i) at every step. This pins the streaming implementation to the standard binary reflected gray code construction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.stream\_matches\_direct\_unscrambled` (hash `fc4aa97eaa716b50`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.bridge\_factorization\_reproduces\_brownian\_covariance

- **claim** (`cl\_1855e3d9b5060e6c`): bridge\_factorization\_reproduces\_brownian\_covariance() is the comprehensive Brownian Bridge oracle test. It asserts: (1) the grid times are exactly t\_i = (i+1)\*T/m (bit-identical comparison via to\_bits()); (2) L L^T = C to 1e-12 absolute error; (3) the terminal-normal column satisfies L\[i\]\[0\] = t\_i/sqrt(T); (4) for m=8 the bisection pivot is the midpoint (index 3, t=T/2) with conditional standard deviation sqrt(t\*(T-t)/T) = sqrt(T)/2; (5) calling build() on canonical basis vectors reproduces columns of weight\_matrix() bit-for-bit, proving the hot-path and matrix methods are algebraically identical.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.bridge\_factorization\_reproduces\_brownian\_covariance` (hash `b5c7398085f87ee3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.inverse\_normal\_matches\_reference

- **claim** (`cl\_3e9d5a5214dfb8a7`): inv\_norm\_cdf is verified by inverse\_normal\_matches\_reference() to satisfy three independent correctness criteria: (1) Φ⁻¹(0.5) == 0.0 exactly (bit-identical); (2) five published tail quantiles (p ∈ {0.975, 0.95, 0.99, 0.995, 0.999}) match to 1e-13 relative error, and the symmetric negatives also match; (3) a round-trip against the independent reference Φ(x) = ½·erfc(−x/√2) holds to 1e-13 relative across 14 points from the deep tail (p=1e-12) to 0.999; (4) frozen bit patterns at both branch break-points (p=0.02425 and 1−0.02425) are pinned as \`0xbfff913f9b7aa943\` / \`0x3fff913f9b7aa943\`, proving each branch is taken as documented.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.inverse\_normal\_matches\_reference` (hash `15a4aaebb57d9cb4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.low\_discrepancy\_reference\_points

- **claim** (`cl\_fe921bc88969c1ce`): low\_discrepancy\_reference\_points() pins the first eight dim-0 Sobol points against hand-derived gray-code radical-inverse values \[0.5, 0.75, 0.25, 0.375, 0.875, 0.625, 0.125, 0.1875\] (bit-exact). It additionally asserts: (1) point 0 is the origin in every coordinate (gray code g(0)=0); (2) the first 16 dim-0 points form the complete equidistributed set {k/16 : k=0..15} with no duplicates; (3) every Joe-Kuo dimension has a leading direction integer m\_1=1, so point 1 is 0.5 in all dimensions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.low\_discrepancy\_reference\_points` (hash `d8663be59812c7c8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.rqmc\_estimate\_integrates\_monomials

- **claim** (`cl\_8c1c8fe11ae32165`): rqmc\_estimate\_integrates\_monomials() validates the randomised QMC estimator (rqmc\_estimate) by checking that E\[u^k\] for u = Φ(W(t\_idx)/sqrt(t\_idx)) (a uniform(0,1) variate) converges to the correct moment: E\[u\] = 1/2 and E\[u^2\] = 1/3. The test uses 4096 QMC points and 8 scramble replicates; the estimator must hit within 1e-4 absolute for every m ∈ 1..=4 and for both the terminal and first grid coordinates. The standard error must be finite and positive, confirming variance reduction relative to plain Monte Carlo.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.rqmc\_estimate\_integrates\_monomials` (hash `2214ba5b9770226d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.scrambled\_point\_is\_deterministic\_and\_in\_unit\_cube

- **claim** (`cl\_0276476620f87fba`): scrambled\_point\_is\_deterministic\_and\_in\_unit\_cube() asserts three invariants of SobolSequence::scrambled\_point(): (1) repeated calls with the same (i, seed) pair return bit-identical results across all 64 tested points; (2) every scrambled coordinate is strictly inside the open unit interval (0, 1) — the half-LSB bias prohibits the boundary value 0.0 exactly; (3) different scramble seeds (7 vs 8) produce at least one differing coordinate somewhere in the 64×8 grid, proving seed diversity is effective.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-qmc.tests.sequence\_oracle.scrambled\_point\_is\_deterministic\_and\_in\_unit\_cube` (hash `319e16e0e4dc5871`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.BootstrapError.fmt

- **claim** (`cl\_15346ef107c5ee32`): BootstrapError::fmt produces a structured, human-readable diagnostic for each failure mode of OIS curve bootstrapping: NoQuotes → static str "no calibrating OIS quotes supplied"; ForwardStartingQuote → "a quote schedule does not start at the curve origin (spot)"; NonIncreasingMaturity → "quote maturities must be strictly increasing"; Solve(e) → wraps the inner SolverError as "pillar root-solve failed: {e}"; Curve(e) → wraps the inner CurveError as "assembled curve rejected: {e}". The last two variants propagate nested error context via write! rather than f.write\_str, preserving the full causal chain for diagnostics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.BootstrapError.fmt` (hash `d1e5d4999754a9e2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.SOLVE\_MAX\_ITER

- **claim** (`cl\_5d176d3987c8b410`): bootstrap\_ois performs sequential single-pillar OIS bootstrap: for each quote in strictly increasing maturity order it finds the continuously-compounded zero rate z at that maturity by solving ois\_par\_rate(curve\_candidate, schedule) = target via Brent's method, where the candidate curve extends the already-bootstrapped pillars by one point DF = exp(-z \* T). Convergence search bracket is \[Z\_LO, Z\_HI\] with tolerance SOLVE\_TOL and SOLVE\_MAX\_ITER iterations.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.SOLVE\_MAX\_ITER` (hash `52517b4caa7291a4`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.SOLVE\_TOL` (hash `6d3a89a6940697fe`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.Z\_HI` (hash `a43794be0f89e9b1`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.Z\_LO` (hash `3f4509fb93de1f2b`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.bootstrap\_ois` (hash `6bc874a6ab5396a5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.discount\_factor

- **claim** (`cl\_2f8982b7a76a32fe`): Curve::from\_log\_linear\_dfs stores each pillar as ln(DF) so that piecewise-linear interpolation in ln(DF) space is identical to log-linear interpolation of discount factors: DF(t) = exp(ln\_df(t)), where ln\_df(t) is linearly interpolated between the bounding nodes (a, b) as a.ln\_df + slope\*(t - a.t), slope = (b.ln\_df - a.ln\_df)/(b.t - a.t). The origin pillar (t=0, DF=1) is mandatory and ln(1)=0 is stored at index 0.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.discount\_factor` (hash `9f1259e2069fc57f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.from\_log\_linear\_dfs` (hash `f39accd805f4e304`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.ln\_df` (hash `0e695ef1759ed482`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.discount\_factor

- **claim** (`cl\_3d002246895b5bea`): Curve::discount\_factor returns Df(1.0) for t \<= 0 (spot and past dates); for t \> 0 it returns exp(ln\_df(t)). ln\_df dispatches on the Interpolation scheme; the LogLinearDf branch (ln\_df\_log\_linear) is piecewise-linear log interpolation with constant extrapolation beyond the last pillar, achieved by bracket(t).clamp(1, n-1) (so t below the first or above the last pillar clamps to the boundary segment).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.discount\_factor` (hash `9f1259e2069fc57f`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.ln\_df` (hash `0e695ef1759ed482`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.forward\_rate\_continuous

- **claim** (`cl\_10b17131f0a44593`): Curve::forward\_rate\_continuous computes the continuously-compounded forward rate over \[t1, t2\] as (ln(DF(t1)) - ln(DF(t2))) / (t2 - t1), which follows directly from DF(t1)/DF(t2) = exp(r\_fwd \* (t2-t1)).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.forward\_rate\_continuous` (hash `8e3da801392958f6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.forward\_rate\_simple

- **claim** (`cl\_6d2430bc37771afe`): Curve::forward\_rate\_simple computes the simple (money-market) forward rate over \[t1, t2\] as (DF(t1)/DF(t2) - 1) / (t2 - t1), the standard IBOR/OIS simple-rate definition.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.forward\_rate\_simple` (hash `82adf430f3cd7f69`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.from\_zero\_rates

- **claim** (`cl\_e6e7448dc9a31a47`): Curve::from\_zero\_rates converts each (t, z) pillar to DF = exp(-z\*t) and prepends the mandatory origin (0, 1.0) before delegating to from\_log\_linear\_dfs. The conversion is the exact inverse of zero\_rate: z = -ln(DF)/t =\> DF = exp(-z\*t).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.from\_zero\_rates` (hash `106b0578dce393cc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.instantaneous\_forward

- **claim** (`cl\_fc087deef70891d1`): Curve::instantaneous\_forward dispatches on the curve's Interpolation scheme: for LogLinearDf it returns segment\_forward(t) = -(b.ln\_df - a.ln\_df)/(b.t - a.t), the piecewise-constant forward of the bounding log-linear segment (constant within each inter-pillar interval, jumps at pillars); for MonotoneConvexForward it returns monotone\_convex\_forward(t), a continuous interpolated instantaneous forward.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.instantaneous\_forward` (hash `ba6a4558392e1ebe`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.segment\_forward` (hash `2af9b84a9ff33aaa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.zero\_rate

- **claim** (`cl\_f97def12bcec6504`): Curve::zero\_rate implements the continuously-compounded zero rate identity: R(t) = -ln(DF(t))/t. At t ≈ 0 (within ORIGIN\_TOL) it returns the instantaneous forward rate of the first segment to avoid division by zero.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve.zero\_rate` (hash `9d6ccfe533262d65`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.ORIGIN\_TOL` (hash `3b3211d21c096e4b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-rates.src.curve.CurveError.fmt

- **claim** (`cl\_92d89a42c98e9bf9`): CurveError::fmt maps each variant to a precise, allocation-free diagnostic string that encodes the exact structural invariant violated by an ill-formed discount-factor curve: TooFewPillars → "curve needs an origin pillar plus at least one dated pillar"; MissingOrigin → "first pillar must be the curve origin (t = 0, DF = 1)"; NonMonotonicTime → "pillar times must be strictly increasing"; NonPositiveDiscountFactor → "discount factors must be strictly positive"; NonPositiveZeroTime → "zero-rate pillars require strictly positive time". The implementation contains no writes, no allocation, and no side-effects — it is a pure match-to-static-str dispatch ending in f.write\_str(msg).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.curve.CurveError.fmt` (hash `d9d29c749b2bfcb5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-rates.src.ois.OisSchedule.new

- **claim** (`cl\_1d27240acd9d1ab8`): OisSchedule::new validates: (1) periods non-empty, (2) all accrual year-fractions strictly positive, (3) pay times strictly increasing with schedule.start strictly before the first pay time. Any violation returns a typed ScheduleError (Empty / NonPositiveAccrual / StartNotBeforeFirstPay / NonIncreasingPayTimes).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:34Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.OisSchedule.new` (hash `8d433f9e1b637631`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:34Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:34Z

## github.com-soarsa-celnet.crates.celnet-rates.src.ois.ScheduleError.fmt

- **claim** (`cl\_16d6690828dbb9ff`): ScheduleError::fmt maps each OIS schedule validation variant to a precise, allocation-free diagnostic string encoding the structural contract violated: Empty → "OIS schedule has no fixed-leg periods"; StartNotBeforeFirstPay → "first payment must be strictly after the effective start"; NonIncreasingPayTimes → "payment times must be strictly increasing"; NonPositiveAccrual → "period accruals must be strictly positive". The implementation is a pure match-to-static-str dispatch with no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.ScheduleError.fmt` (hash `a10a3452757ba559`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_annuity

- **claim** (`cl\_cc849aa86cbc70bb`): ois\_annuity computes the fixed-leg annuity as the sum over all schedule periods of (accrual\_fraction \* discount\_factor(pay\_time)): A = Σ\_i α\_i \* DF(T\_i), where α\_i is the ACT/360 accrual year-fraction and T\_i is the pay time.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_annuity` (hash `9892435f967eb0c2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_par\_rate

- **claim** (`cl\_20c7acc52ae44bc5`): ois\_par\_rate implements the standard OIS par-rate identity: par = (DF(start) - DF(maturity)) / A, where A = ois\_annuity. At par the floating-leg PV (DF(start) - DF(maturity)) equals the fixed-leg PV (par \* A), so ois\_pv = 0 at fixed\_rate = par.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_par\_rate` (hash `f78d6aa0f66242d3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_pv` (hash `0d9fe3d5b2d6002e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_pv

- **claim** (`cl\_3644f0ea59df7d04`): ois\_pv computes the PV of a receive-fixed OIS swap as: PV = N \* (K \* A - (DF(start) - DF(maturity))), where K is the fixed rate, A is the annuity, and (DF(start) - DF(maturity)) is the floating-leg PV under OIS discounting. Positive PV means the fixed leg exceeds the floating leg value.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.ois.ois\_pv` (hash `0d9fe3d5b2d6002e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-rates.src.risk.ONE\_BP

- **claim** (`cl\_c4659d51d069a8ff`): pv01 is a pure analytic sensitivity: PV01 = N \* A \* 1bp, where A = ois\_annuity and 1bp = ONE\_BP = 0.0001. It equals the first-order fixed-rate sensitivity dPV/dK \* 1bp = N\*A\*1bp, without re-bootstrapping the curve.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.risk.ONE\_BP` (hash `9d5eca9c819d2a7a`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rates.src.risk.pv01` (hash `68a7238b25051662`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-rates.src.solver.SolverError.fmt

- **claim** (`cl\_330c317604e39924`): SolverError::fmt maps the two root-finder failure modes to precise, allocation-free diagnostic strings that encode the mathematical pre/post-condition violated: NoBracket → "endpoints do not bracket a root (f(a) and f(b) share a sign)", documenting that the bisection pre-condition f(a)·f(b) \< 0 was not met; NoConvergence → "root-finder hit its iteration limit before converging", documenting that the iteration bound was exhausted. Implementation is a pure match-to-static-str dispatch with no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rates.src.solver.SolverError.fmt` (hash `9afce03c411d102b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.candidate\_log\_ok

- **claim** (`cl\_aade5dbb9be6114a`): SAFETY/CONSENSUS — Raft election restriction / log up-to-date check (deliverable: replog-quorum-durability). NodeCore::candidate\_log\_ok is a pure, deterministic read-only predicate implementing the §5.4.1 leader-completeness pre-condition: a vote is granted only if the candidate's log is at least as up-to-date as the voter's — first by last-term (a strictly higher candidate term wins), and on equal last-terms by length (cand\_len \>= my\_len), with the EMPTY\_LOG sentinel mapped to length 0 and u128 arithmetic preventing index overflow. It reads only &self.log (last\_term/last\_index) plus the two candidate scalars; no mutation, I/O, or allocation. This is the complementary half of leader\_advance\_commit: together they guarantee a committed entry is never lost to a stale-log leader. Self-invalidates if the (term-then-length) ordering is weakened.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.candidate\_log\_ok` (hash `4fe19e493e2e5356`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.leader\_advance\_commit

- **claim** (`cl\_e78974b44b1f7bd6`): SAFETY/CONSENSUS — Raft commit safety (the §5.4.2 leader-completeness rule): NodeCore::leader\_advance\_commit advances the commit watermark only for a log index n that satisfies BOTH conditions — (1) it is replicated on a quorum (holders = 1 self + peers whose match\_index \>= n, counted against majority = cluster\_size/2 + 1), AND (2) the entry at n is from the leader's OWN current term (self.log.term\_at(n) == Some(current\_term)); entries from earlier terms are skipped and never directly committed. The guard \`if self.role != Role::Leader { return }\` makes commit-advancement a leader-only action. This is what stops a committed-then-uncommitted divergence under leader churn: a bare-majority replication of a stale-term entry does not commit. (This is a leader-state-advancing method, not a side-effect-free accessor — the invariant is the majority-AND-current-term gating it enforces, verified against leader\_advance\_commit\_requires\_a\_current\_term\_majority.)
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.leader\_advance\_commit` (hash `68745dd875cbb9c3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-replog.src.entry.EntryError.fmt

- **claim** (`cl\_fd78e93fb1004f0f`): EntryError::fmt renders exactly two fixed static strings: 'log entry truncated' for the Truncated variant and 'log entry crc mismatch' for the Crc variant. The output is fully determined by the variant — no heap allocation, no I/O, no mutable state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.entry.EntryError.fmt` (hash `082dc8d25592742a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-replog.src.log.Log.last\_index

- **claim** (`cl\_972537489fc96bc5`): Log.last\_index returns the logical index of the last retained entry as base\_index + (terms.len() - 1), or snapshot\_index when the log is empty (all entries compacted away), or None when neither entries nor a snapshot exist. The formula is: \`if terms.len() \> 0 { Some(base\_index + terms.len() - 1) } else { snapshot\_index }\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.log.Log.last\_index` (hash `fe2b4b36bf421ac0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-replog.src.log.Log.last\_term

- **claim** (`cl\_c422051eed65f207`): Log.last\_term returns the term of the last retained entry (terms.last()), or snapshot\_term when no entries are held (0 when no snapshot has ever been installed, the snapshot's included term otherwise). It is used by the candidate log-comparison predicate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.log.Log.last\_term` (hash `a731cf51c5bf24f9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-replog.src.persist.decode

- **claim** (`cl\_2cc656a9099910b7`): Raft persistent-state corruption safety (deliverable: replog-quorum-durability). persist::decode is a pure, deterministic deserializer: given a byte slice it validates exact RECORD\_LEN, recomputes crc32 over the body and rejects any mismatch by returning None, then reconstructs (current\_term, voted\_for, commit\_index) with no I/O and no mutation of any shared state. Purity + CRC validation is the safety invariant — a torn or bit-flipped persisted vote/term/commit record can never be silently accepted, so quorum-commit durability and single-vote-per-term election safety survive a crash mid-fsync. Pure (no WRITES edges); self-invalidates on change.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.persist.decode` (hash `a5abb2744256cc81`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-replog.src.persist.encode

- **claim** (`cl\_35aed985a9f31bd7`): SAFETY — Raft persistent-state encode is the CRC-protected inverse of decode (deliverable: replog-quorum-durability). persist::encode(&PersistentState) is a pure, deterministic serializer producing a fixed RECORD\_LEN frame: it lays out current\_term, voted\_for (value + present-flag), commit\_index (value + present-flag) in little-endian with explicit 2-byte CRC alignment padding (debug\_assert on BODY\_LEN), then appends crc32(body). Because the byte layout and CRC are a deterministic function of the state alone — no I/O, no mutation, no clock/RNG — encode is the exact round-trip partner of the already-claimed persist::decode: a flipped byte recomputes a different CRC and decode rejects it, so torn or corrupt term/vote/commit records read as absent rather than as forged consensus state. Self-invalidates if the layout or CRC coverage changes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.persist.encode` (hash `9aaf542b487fb6b2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-replog.src.state.BookState.decode

- **claim** (`cl\_de3ac11d189e4c2f`): BookState.decode deserialises a flat array snapshot of the book: \[8-byte count-LE\]\[count × 16 bytes: (key: u64-LE, value: f64-bits-LE)\]. It uses checked\_mul for the expected byte count to prevent overflow on a malformed count. The result is a BTreeMap\<u64,f64\> with entries in key-sorted order (BTreeMap insertion preserves none; sorted order is a property of BTreeMap iteration, not insertion order here).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.state.BookState.decode` (hash `430208fff5dc2a06`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-replog.src.state.BookUpdate.decode

- **claim** (`cl\_78e8a368cb6dedfe`): BookUpdate.decode is the exact inverse of BookUpdate.encode: it reads the discriminant byte, then parses key (8 bytes, u64-LE) and value/delta (8 bytes, f64 via from\_bits) for tags 0/1, or key only for tag 2. Returns UpdateError::Truncated for a short buffer and UpdateError::UnknownTag(n) for unrecognised discriminants.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.state.BookUpdate.decode` (hash `111fcdba7a168a4c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-replog.src.state.BookUpdate.encode

- **claim** (`cl\_2c7eaeac9ef049ef`): BookUpdate.encode serialises three update variants to a compact little-endian byte stream: tag 0x00 = Set (key: u64-LE, value: f64-via-to\_bits-LE), tag 0x01 = Add (key: u64-LE, delta: f64-via-to\_bits-LE), tag 0x02 = Remove (key: u64-LE only, 9 bytes total). All f64 values are transmitted as their IEEE-754 bit pattern (to\_bits) so NaN bit patterns round-trip exactly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.state.BookUpdate.encode` (hash `192faf44cb99100d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-replog.src.state.UpdateError.fmt

- **claim** (`cl\_bd5762170eaf30c7`): UpdateError::fmt covers exactly two variants: Truncated yields 'book update truncated' and UnknownTag(t) yields 'book update unknown tag {t}' where t is the raw tag value embedded inline. Output is deterministic and allocation-free for Truncated; UnknownTag performs a single integer format.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.state.UpdateError.fmt` (hash `fa00c0fa04e96871`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-replog.src.wire.WireError.fmt

- **claim** (`cl\_80ff55df23c35f67`): WireError::fmt covers exactly three variants with deterministic, human-readable messages: Io wraps the inner error with prefix 'wire io: {e}'; FrameTooLarge embeds the byte count as 'wire frame too large: {n}'; Malformed emits the static string 'wire frame malformed'. Every reachable variant is matched — the compiler enforces exhaustiveness.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.src.wire.WireError.fmt` (hash `f37baaefccdf41de`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-replog.tests.decode\_fuzz.book\_update\_bit\_eq

- **claim** (`cl\_aa927f88592b5ef0`): book\_update\_bit\_eq: a pure, bit-exact equality predicate for BookUpdate that compares float-carrying variants (Set, Add) via raw IEEE-754 bits (\`to\_bits()\`) rather than \`==\`, making it correct for NaN and negative-zero. Set: \`ka==kb && va.to\_bits()==vb.to\_bits()\`; Add: \`ka==kb && da.to\_bits()==db.to\_bits()\`; Remove: \`ka==kb\`; cross-variant: always false.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-replog.tests.decode\_fuzz.book\_update\_bit\_eq` (hash `4d025da9f41ff668`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-rfq.src.internal.InternalPricerSource.new

- **claim** (`cl\_aeca764db4cfaea2`): InternalPricerSource::two\_way(&self) -\> TwoWay is the symmetric mid-spread quote constructor: bid = mid - half\_spread, offer = mid + half\_spread. The constructor clamps half\_spread to max(half\_spread, 0.0), so a negative input is silently zeroed, guaranteeing bid \<= offer for any finite mid. The function reads only &self and constructs a new TwoWay with no I/O or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.internal.InternalPricerSource.new` (hash `ce4f1729e5e7e575`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.internal.InternalPricerSource.two\_way` (hash `d8dc303e4fafe0fa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-rfq.src.internal.always\_quotes\_deterministically

- **claim** (`cl\_fb662169a8087f2a`): always\_quotes\_deterministically verifies that InternalPricerSource (the native dealer) is referentially transparent: two sequential calls with the same RfqRequest and timeout produce bit-identical QuoteSourceReply::Quote values. The mid-price is derived as \`spread\_mid = mid\_rate - half\_spread\` (asserted: \`(price.bid - 0.0080).abs() \< 1e-12\` for mid=0.0085, half\_spread=0.0005). The epoch\_nanos field is taken verbatim from the seeded clock (42) and valid\_until\_nanos = epoch\_nanos.saturating\_add(valid\_for\_nanos) = 1\_042.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.internal.always\_quotes\_deterministically` (hash `0f1b876997aa3ae0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-rfq.src.panel.SideKey\<'\_\>.partial\_cmp\_total

- **claim** (`cl\_008b1103bc81993f`): rank\_side(rows, now\_nanos, side) is the pure best-LP selector: it filters rows to those whose valid\_until\_nanos \>= now\_nanos (last-look gate — stale quotes are categorically excluded from winning), then picks the min under SideKey::partial\_cmp\_total. The SideKey for Bid negates the price (-bid) so that min-selection gives the highest bid; for Offer the price is taken straight so min gives the lowest offer. Tie-break is lexicographic: (better\_price, earlier epoch\_nanos, smaller lp\_id) — fully deterministic with no NaN ambiguity (non-finite prices lose to any finite price via partial\_cmp\_total). Returns None when all rows are stale or the slice is empty.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.SideKey\<'\_\>.partial\_cmp\_total` (hash `0fae175009559d9b`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.rank\_side` (hash `a3c54d3530805275`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.side\_key` (hash `217717af0028e806`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-rfq.src.panel.check\_winner

- **claim** (`cl\_654f15a23274c9fc`): check\_winner(rows, side, winner) is the post-ranking consistency guard: if a winner LP-id was declared it MUST appear in the responder rows, else PanelError::WinnerNotAResponder is returned. This prevents a phantom winner — an LP-id that timed out or declined — from appearing in the final RankedPanel. The function reads only its arguments, allocates only the error string on the failure path, and always returns Ok(()) when winner is None.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.check\_winner` (hash `0af3f3af4d037c3f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.cube.Cube.group\_value

- **claim** (`cl\_a51aab711ff31887`): \`Cube::group\_value\` resolves the dimension key for a fact: for \`Desk\` it prefers the hierarchy's \`desk\_of(book)\` pointer (so a re-org reflowing without re-tagging individual facts), falling back to \`key.desk\`; for \`Entity\` it prefers \`entity\_of(location)\`, falling back to \`key.entity\`; all other dimensions (\`Trader\`, \`Book\`, \`Location\`, \`Underlying\`) read directly from the \`FactKey\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.cube.Cube.group\_value` (hash `5fd54099ddb1940a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.dimension.FactKey.group\_value

- **claim** (`cl\_52259f2d6c8c9fde`): \`FactKey::group\_value\` maps each \`DimensionId\` variant to a \`u64\` discriminant by widening the newtype's inner \`u32\`: \`Trader→trader.0\`, \`Book→book.0\`, \`Desk→desk.0\`, \`Location→location.0\`, \`Entity→entity.0\`, \`Underlying→underlying\_group\_value(&self.underlying)\`. This is a pure exhaustive match with no allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.dimension.FactKey.group\_value` (hash `9d7b68c2932e372e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.exotic.ExoticKind.unit\_price

- **claim** (`cl\_b18b47e88416b5a4`): \`ExoticKind::unit\_price\` is the single dispatch point from the exotic risk layer into the \`celnet-exotics\` pricing library: it converts \`VanillaInputs\` to \`celnet\_exotics::ExoticInputs\` via \`Into\`, then delegates to \`single\_barrier\_price\` for \`SingleBarrier\` or \`digital\_price\` for \`Digital\`. All notional scaling is done by the calling \`ExoticLeg\`; this method returns a raw unit price.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.exotic.ExoticKind.unit\_price` (hash `34910f16570ba6e3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.exotic.ExoticLeg.canonical\_greeks

- **claim** (`cl\_4fc45fcfa3cebfc4`): \`ExoticLeg::canonical\_greeks\` computes a full ten-field Greek strip for single-barrier and digital exotic legs via finite differences on \`ExoticKind::unit\_price\`. Bump sizes are calibrated on a √ε scale: \`ds = spot\_bump(spot)\` (adaptive), \`dv = 1e-4\`, \`dt = clamp(t × 1e-4, 1e-6, t/2)\`. Standard central differences give delta, vega, theta (sign-negated: \`−∂V/∂T\`), and volga; mixed central differences give gamma, speed (4-point), vanna, charm, zomma, and color. For \`Digital\` variants the closed-form \`digital\_greeks\` replaces FD delta, gamma, and vega to avoid round-off. All outputs are scaled by notional.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.exotic.ExoticLeg.canonical\_greeks` (hash `1d9896db632d6fc7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.CorrelationScenario.scale

- **claim** (`cl\_49c983cbffc46e16`): \`CorrelationScenario::scale\` applies the FRTB SA correlation scenario scaling to a base correlation \`rho\`: High → \`min(1.25ρ, 1.0)\` (stressed up, capped); Medium → \`ρ\` (unchanged); Low → \`max(2ρ − 1, 0.75ρ)\` (the larger of a floor-shifted and a proportionally reduced value, ensuring the result never exceeds the base and is bounded below by \`0.75ρ\`).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.CorrelationScenario.scale` (hash `a2279591d85cedc4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.ResidualKind.weight

- **claim** (`cl\_c46494ab2b2a12d9`): \`ResidualKind::weight\` returns the FRTB residual risk add-on weight for each bucket class: \`ExoticUnderlying\` → 0.01 (1%), \`OtherResidual\` → 0.001 (0.1%), \`None\` → 0.0. These constants match FRTB SA RRAO calibration (CRR2/Basel III.2 §§ RBC25.16-17).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.ResidualKind.weight` (hash `a939a2cf00e9e14e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.SbmCharge.under

- **claim** (`cl\_3be1dc0a7274bb1c`): \`SbmCharge::under\` is a pure selector: given a \`CorrelationScenario\`, it returns the pre-computed SBM capital charge for that scenario stored on \`SbmCharge\` (High → \`self.high\`, Medium → \`self.medium\`, Low → \`self.low\`). This enables FRTB-mandated worst-case reporting by comparing all three scenario charges without recomputing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.SbmCharge.under` (hash `09d7cfd4f63735d6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.SbmParams\<G\>.cross\_term

- **claim** (`cl\_55b1f3ccac098519`): \`SbmParams::cross\_term\` computes the FRTB SBM inter-bucket cross-term: \`Σ\_{b≠c} γ\_{bc}(scenario) × S\_b × S\_c\` where \`γ\_{bc}\` is the scaled inter-bucket correlation from \`CorrelationScenario::scale\` and \`S\_b\` is the net weighted sensitivity for bucket \`b\`. Diagonal entries (\`b == c\`) are explicitly skipped. This is a pure O(n²) double loop with no allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.SbmParams\<G\>.cross\_term` (hash `5981aa94a2146d32`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature\_class

- **claim** (`cl\_aa96f91c49fb0076`): curvature\_class(buckets, gamma) -\> SbmCharge applies the FRTB MAR21.5.2 cross-bucket curvature aggregation under all three correlation scenarios. For each scenario: K\_total = sqrt(max(0, sum\_b(K\_b^2) + sum\_{b≠c}(gamma\_scaled^2 \* psi(CVR\_b,CVR\_c) \* CVR\_b \* CVR\_c))) where psi(CVR\_b,CVR\_c) = 0 iff both CVR are negative (MAR21.5.2(4)), else 1; gamma is squared (not linear) for curvature; and gamma is scenario-scaled before squaring. Returns SbmCharge{high, medium, low}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature\_class` (hash `c7826925f19469be`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature\_legs

- **claim** (`cl\_ded35ba19d685f03`): curvature\_legs(pricer, positions, rw) -\> (cvr\_up, cvr\_down) is the FRTB MAR21 curvature CVR computation for a vanilla node: base PV and two relative spot reprices (×(1±rw)), linear term = sum\_i(delta\_spot\_i × notional\_i × rw × spot\_i). CVR\_up = -((reprice\_up - base) - linear); CVR\_down = -((reprice\_down - base) + linear). Only the spot is shocked; carry, vol, time, and strike are held fixed. The same formula is implemented in vanilla\_curvature\_legs via node\_value/node\_value\_shocked helpers, which is the canonical path used for FRTB curvature bucket construction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature\_legs` (hash `aa9041cc2b35569b`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.vanilla\_curvature\_legs` (hash `735351ffd47c70b4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.quadratic\_form

- **claim** (`cl\_ab527a0ef4a3765f`): quadratic\_form(ws: &\[f64\], rho: F) -\> f64 computes the FRTB intra-bucket capital formula: sqrt(max(0, sum\_i(ws\_i^2) + sum\_{i\<j}(2\*rho(i,j)\*ws\_i\*ws\_j))). It is the kernel used for both the delta/vega SBM bucket charge (via SbmParams::class\_charge) and the vega-bucket correlation\_weighted\_vega function. The max(0,·) guard prevents imaginary results when the cross-term sum dominates the diagonal (can occur under the Low correlation scenario where scaled ρ can be negative).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.quadratic\_form` (hash `f489b31ca2d31914`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.correlation\_weighted\_vega` (hash `f112a5ae5aea9fe1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.residual\_addon

- **claim** (`cl\_3492f658f79c8ef0`): residual\_addon(instruments: &\[ResidualInstrument\]) -\> f64 is the FRTB RRAO (Residual Risk Add-On) charge: a pure linear sum of \|notional\_i\| \* kind.weight() over all residual instruments. ResidualKind::OtherResidual carries weight 0.001 (10bp); ResidualKind::ExoticUnderlying carries 0.01 (100bp); ResidualKind::None carries 0.0 (vanilla, excluded). No correlation, no squaring — the RRAO is a gross-notional additive charge by BCBS design.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.residual\_addon` (hash `aa935ea2b3140b81`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.from\_position

- **claim** (`cl\_7745a1f24dd36c0f`): \`PositionSensitivity::from\_position\` dispatches on underlying type: FX/metal positions use a reverse-mode AAD sweep (\`adjoint\_greeks\`) for a one-evaluation gradient, yielding \`delta\_spot\`, \`gamma\`, \`vega\`, \`volga\`, \`vanna\`, \`discount\_rho = rho\_dom × n\`, \`carry\_rho = −rho\_for × n\` (FX carry basis: b = r\_dom − r\_for, so ∂V/∂b = −rho\_for). Non-FX positions delegate to \`pricer.price\_greeks\`; \`RateSensitivities::Fx\` from a non-FX leaf is defensively normalized to the carry basis. Failing pricer calls return \`Self::zero(spot)\`, never panicking.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.from\_position` (hash `71723d600c98003f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.taylor\_pnl

- **claim** (`cl\_1f1bf25bc77ee2b5`): DELIVERABLE risk/pnl-attribution = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/L finding "P&L attribution (Greeks-based P&L explain) exists nowhere in the platform"; reconciled against the live graph — it now EXISTS as celnet-risk-cube). \`PositionSensitivity::taylor\_pnl(scenario)\` is a pure, side-effect-free Greeks-based P&L-explain primitive: it returns the second-order Taylor P&L of a position under a Scenario as delta\_spot·dS + ½·gamma·dS² + vega·dvol + ½·volga·dvol² + vanna·dS·dvol + discount\_rho·discount\_abs + carry\_rho·carry\_abs (dS = spot·spot\_rel), reading only its own sensitivity fields and the scenario — no writes, no allocation, no I/O. This is exactly the cross-Greek P&L explain the competitive positioning claims; it is invoked by the risk-cube non-additive roll-up. SELF-INVALIDATING: any change to taylor\_pnl's body that introduced a write/allocation/I/O side effect (e.g. a stateful attribution accumulator) would flip the WRITES gate and stale this claim, and if the Greeks-based explain were ever removed/relocated the anchor would unresolve.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.taylor\_pnl` (hash `8ea0fe74b4994b2c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.base\_scenario\_is\_bitwise\_identity

- **claim** (`cl\_e15bb51ef06ca88c`): \`Scenario::base().apply(inputs)\` is the bitwise identity: every field of the returned \`CarryInputs\` compares equal bit-for-bit to the original, including the carry arm variant (FxRates vs CostOfCarry). The named constructors \`Scenario::spot(δ)\` and \`Scenario::vol(δ)\` exclusively set \`spot\_rel\` and \`vol\_abs\` respectively, leaving the other three adjustment fields exactly zero.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.base\_scenario\_is\_bitwise\_identity` (hash `2f1d20b40a3f8ecb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.historical\_var\_es

- **claim** (`cl\_ec9bfe63f8b92547`): historical\_var\_es(pricer, positions, scenarios, alpha) -\> VarEs computes full bump-and-revalue historical VaR/ES: for each Scenario it calls node\_pnl (sum of position\_pnl over all positions through the CarryPricer seam), collects a Vec\<f64\> of per-scenario P&Ls, then delegates to the shared quantile\_var\_es kernel. No Greeks or Taylor approximation — every scenario is a full reprice. Deterministic given identical scenario ordering; returns VarEs{var:0,es:0} for an empty scenario slice.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.historical\_var\_es` (hash `46843d0f1045ce53`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.node\_pnl` (hash `017a953143d8341f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.quantile\_var\_es

- **claim** (`cl\_ceb76c2bcb1bfdcc`): quantile\_var\_es(pnl: &mut \[f64\], alpha: f64) -\> VarEs is the single shared quantile kernel used by BOTH the historical full-reprice path (historical\_var\_es) and the sensitivity Taylor path (sensitivity\_var\_es). It sorts pnl ascending, computes tail = floor((1-alpha)\*n).max(1).min(n), then ES = -(sum(pnl\[..tail\]) / tail) and VaR = -pnl\[tail-1\], both floored at 0. The tail index is floor not ceil, so the boundary is strict — a loss exactly at the alpha quantile is excluded from the ES average. The function takes a &mut slice (in-place sort) and has no I/O or shared-state side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.quantile\_var\_es` (hash `7b499ca106b90f69`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity\_profile\_is\_adjoint\_greeks\_scaled\_by\_notional

- **claim** (`cl\_bc893f51c4ec25df`): PositionSensitivity::sensitivity\_profile carries adjoint Greeks (delta\_spot, gamma, vega, volga, vanna, discount\_rho, carry\_rho) scaled by notional\_base. The scaling is bitwise-exact: each field equals the corresponding adjoint Greek multiplied by notional, confirmed by to\_bits() equality in the test. carry\_rho = -(rho\_for \* notional) — a sign flip from the raw Greek so that a positive carry\_rho always means sensitivity to the foreign rate in the direction that increases PV.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity\_profile\_is\_adjoint\_greeks\_scaled\_by\_notional` (hash `9c921359b7faa7f0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity\_var\_es

- **claim** (`cl\_7623c6704f2ba678`): sensitivity\_var\_es(pricer, positions, scenarios, alpha) -\> VarEs is the adjoint (sensitivity-based) VaR/ES path. It makes ONE Greeks sweep via node\_sensitivities (one CarryPricer call per position), then for each Scenario approximates node P&L as sum\_i(PositionSensitivity\_i.taylor\_pnl(s)) — a second-order Taylor expansion in spot and vol shocks. Delegates to the same quantile\_var\_es kernel as the historical path. The separation of the single-sweep sensitivity computation from the per-scenario summation is the efficiency invariant: O(P) pricer calls instead of O(P×S).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity\_var\_es` (hash `044f855b3db8de20`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.shift\_carry

- **claim** (`cl\_37520f50a1a51f22`): \`shift\_carry\` translates absolute rate scenario bumps into the correct carry-arm fields: for \`FxRates { r\_dom, r\_for }\` it sets \`r\_dom += discount\_abs\` and \`r\_for += discount\_abs − carry\_abs\`, preserving the cost-of-carry identity \`b = r\_dom − r\_for\` under independent additive shocks to the discount rate and the carry basis; for \`CostOfCarry { r, b }\` it shifts \`r += discount\_abs\` and \`b += carry\_abs\` directly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.shift\_carry` (hash `8762b6c73f3a513f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-risk-cube.src.scenario\_grid.asymmetric\_grid\_indexing\_is\_row\_major

- **claim** (`cl\_04b96b679a56f5ad`): ScenarioAxes row-major layout: \`NodeScenarioGrid\` stores PV values in a flat \`pv\` array with index \`i \* n\_vol + j\` where \`i\` indexes spot multiplier and \`j\` indexes vol bump. The \`pv(i, j)\` accessor enforces this and the test verifies that for a 2×3 grid all six nodes are distinct and each matches the direct analytic re-price \`Σ notional × black(spot \* sm, strike, vol + vb, t, r\_dom, r\_for)\` to absolute tolerance 1e-6. The analytic path always has \`on\_gpu = false\` and \`std\_err == 0.0\` bitwise at every node.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-cube.src.scenario\_grid.asymmetric\_grid\_indexing\_is\_row\_major` (hash `4cf9ef16068c5d28`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetError.fmt

- **claim** (`cl\_3e79b02fbce40f0f`): FleetError::fmt renders exactly two variants: Route(e) formats as "fleet routing error: {e}" (delegating to the inner error's Display); ShardUnavailable(r) formats as "fleet shard {r.0} unavailable" (printing the raw ReplicaId integer). These are the only human-visible error strings for fleet-layer failures.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetError.fmt` (hash `f77048e85680577b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetError.source

- **claim** (`cl\_eb3ac5c7638fc455`): FleetError::source implements the std::error::Error source chain: Route(e) returns Some(e) — the inner routing error is surfaced as the cause; ShardUnavailable has no inner cause and returns None. This correctly propagates error chains for the Route variant while correctly terminating the chain for the leaf ShardUnavailable variant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetError.source` (hash `53a2d8a7d46bd7ee`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.fan\_in\_additive

- **claim** (`cl\_d0dc17de1bd68fb5`): \`FleetReducer::fan\_in\_additive\` computes the additive firm-node roll-up — net Greeks + vega ladder — by initialising from the first shard's \`local\_aggregate\` then calling \`merge\_additive\` on each subsequent shard in ascending-replica order. An empty shard list returns \`Cube::new().firm\_aggregate(pillars)\` rather than failing. The method is pure with respect to external state (no I/O, no shared mutation); its output depends only on \`&self\` and \`pillars\`. This is the cheap path: additive Greek/vega aggregation with O(shards) work and no constituent-position allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.fan\_in\_additive` (hash `f1984d2b00d52368`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetTopology.parse

- **claim** (`cl\_bf65df376f41dd3f`): \`FleetTopology::parse(mode, backends)\` is the startup topology selector: it returns \`FleetTopology::Distributed { endpoints }\` only when \`mode == "distributed"\` AND the comma-separated \`backends\` string yields at least one non-empty trimmed endpoint; otherwise it always falls back to \`FleetTopology::InProcess\`. There is no third variant and no error path — an invalid or empty distributed config silently degrades to in-process rather than failing. Pure: reads only its two \`&str\` inputs, returns \`FleetTopology\`, no mutation or I/O.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetTopology.parse` (hash `74e4d2dafb88a5e9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.cross\_asset\_numeraire

- **claim** (`cl\_1b7035818ae3ec38`): cross\_asset\_numeraire extracts a settlement currency from a non-FX Underlying via a fixed priority chain: equity → equity.currency; commodity → commodity.currency; digital-asset → Ccy::parse(quote) falling back to Ccy::USD; all other arms → Ccy::USD. The function is pure and has no side-effects; it is the single authoritative mapping used when building cross-asset risk positions to assign a numeraire to non-FX legs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.cross\_asset\_numeraire` (hash `83ec6dcecdd23a3c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.natural\_owner\_of

- **claim** (`cl\_08a5740ba1f31bb0`): \`partition\_key\_of(fact)\` is the canonical HRW partition key function: it builds a \`PartitionKey\` from the fact's underlying currency pair (\`partition\_pair\_of(&fact.key.underlying)\`) scoped by the entity tenant id (\`TenantId(u64::from(fact.key.entity.0))\`). The two-component key (pair + tenant) ensures co-residency of same-entity same-pair risk across shards — the load-bearing routing invariant pinned by the \`entity\_pair\_cell\_is\_co\_resident\` test. Pure: reads \`&RiskFact\`, returns \`PartitionKey\`, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.natural\_owner\_of` (hash `a0ad99607e5e760e`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_key\_of` (hash `ab4362b39e747bc0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_facts

- **claim** (`cl\_5030d9f6352defcc`): PILLAR (GUIDE.md guardrail 6 + 11 — "scale to investment-banking-sized portfolios"; horizontal scale-out / many-instrument batch). \`partition\_facts(facts, replicas)\` is the pure entry that spreads a risk-fact set across the live replica fleet: it is a thin, side-effect-free delegation to \`partition\_facts\_with(.., partition\_key\_of)\` (reads only its two borrowed slices/sets, mutates nothing, no I/O, no WRITES edges). Each fact is assigned to the HRW \`natural\_owner\` of its partition key, producing a \`FleetReducer\` whose logical shards form a DISJOINT cover of the input (every fact lands in exactly one shard — verified by \`partition\_is\_disjoint\_cover\`) with a deterministic reduction order (shards sorted by ascending replica id), so a fleet of N nodes reduces a firm-sized portfolio to the SAME aggregate as a single node bit-for-bit (\`single\_shard\_fan\_out\_is\_bit\_identical\`). This is the data-parallel scale-out seam that makes IB-sized cube/risk reduction horizontally partitionable without changing the answer. Self-invalidating: any side effect introduced into the partitioning entry flips the purity gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_facts` (hash `cb7ff7f899f465e2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_facts

- **claim** (`cl\_d641420b80bb0a11`): \`partition\_facts\_with\` routes each \`RiskFact\` to its HRW-natural owner using a caller-supplied \`PartitionStrategy\`, then sorts shards by ascending \`ReplicaId\` for deterministic reduction order. It fails closed — returning \`Err(RouteError::EmptySet)\` — if the replica set is empty or if \`natural\_owner\` returns \`None\`. The sort-by-key at the end (\`shards.sort\_by\_key(\|s\| s.replica().0)\`) is the single invariant that makes \`fan\_in\_additive\`'s summation order reproducible regardless of insertion order. Pure in the sense that for identical \`(facts, replicas, strategy)\` inputs the same \`FleetReducer\` shard partition is produced; no I/O, no global mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_facts` (hash `cb7ff7f899f465e2`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition\_facts\_with` (hash `58469a8ed77ff0bc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.carry

- **claim** (`cl\_b1a9289fd9a9a0d4`): \`PositionRisk::carry\` is a \`const fn\` total constructor for a cross-asset carry-priced position: it moves the underlying, option type, base notional and \`CarryInputs\` into the struct and fixes \`quoted\_delta\` and \`premium\_style\` to \`None\`. Being \`const fn\` it is side-effect-free by construction (no allocation, I/O, or mutation of external state) — a pure normalization entry point into the risk-cube leaf model.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.carry` (hash `d03dc2fc398f8991`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.numeraire\_ccy

- **claim** (`cl\_c09c2370f18d7772`): PositionRisk::numeraire\_ccy returns the quote-leg currency of the position's underlying, dispatching by asset class: for Fx and Metal underlyings it calls as\_ccy\_pair().map(\|p\| p.quote); for Equity it returns Some(e.currency); for Commodity it returns Some(c.currency); for DigitalAsset it attempts Ccy::parse(&p.quote) and returns None when the quote leg is a non-fiat coin identifier rather than a recognized ISO 4217 currency. This last case is the only one that can legitimately return None and represents the deferred coin-numeraire case documented in crate docs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.numeraire\_ccy` (hash `d2288d1b7a5200fe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize

- **claim** (`cl\_94d4d8da98286738`): \`canonicalize(pos)\` is the zero-configuration entry point for cross-asset Greek projection: \`pub fn canonicalize(pos: &PositionRisk) -\> Result\<CanonicalLeaf, CarryPriceError\> { canonicalize\_with(&crate::AssetPricer, GreekEngine::default(), pos) }\`. It is a pure forwarder (no writes, no I/O) that fixes the pricer to the static \`AssetPricer\` dispatch table and the engine to the analytic default. Every caller that does not need to inject a custom pricer or the adjoint engine variant goes through this single entry. Pure: it reads only its \`&PositionRisk\` argument and returns \`Result\<CanonicalLeaf, CarryPriceError\>\`, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize` (hash `ed7f30f4369a8326`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize\_with

- **claim** (`cl\_f07ccccd15872211`): CAPABILITY (carry-seam deliverable, cross-asset Greeks projection): canonicalize\_with is the single pure seam that projects any asset class's leaf Greek strip into the carry-neutral CanonicalLeaf — one virtual price\_greeks call through the CarryPricer trait (asset class matched inside the leaf adapter, not in this hot path, per ADR-0008), the FX-only adjoint engine taken only for an FX/metal underlying and otherwise the analytic strip, canonical spot-unadjusted/premium-excluded delta re-derived through the named convention for FX, and every Greek scaled by notional. Pure: builds CanonicalLeaf from (pricer, engine, pos) refs with no WRITES; self-invalidates on any side-effecting change to the projection.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize\_with` (hash `1fb31e3091606193`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.add

- **claim** (`cl\_355ffc5a17ec598b`): CurrencyExposure::add accumulates a per-currency exposure into a fixed-capacity inline array of at most CAP slots. It returns true in exactly three cases: (1) amount == 0.0 (zero is a no-op and always succeeds), (2) ccy already occupies a slot — the amount is netted in-place (leg.amount += amount), or (3) a free (None) slot is claimed. It returns false without panicking or mutating state when all CAP slots are occupied by distinct currencies and ccy is not among them. The array is never reallocated; capacity overflow is a hard boolean signal, not a silent data loss or panic.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.add` (hash `285c0f8e25cd0e8e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.add\_leaf\_delta

- **claim** (`cl\_62b4f9d96b967089`): \`Numeraire::from\_leaves\` converts a slice of \`CanonicalLeaf\`s into a single-numeraire \`Numeraire\` struct: for each leaf it nets the spot-unadjusted premium-excluded delta into a \`CurrencyExposure\` vector (both base and quote legs for FX/metal pairs; quote leg only for cross-asset underlyings), accumulates \`premium\_numeraire\` and \`vega\_numeraire\` by converting each leaf's \`premium\_quote\` and \`vega\` through the \`SpotResolver\` via \`convert()\`. A missing FX rate returns \`Err(NumeraireError::MissingRate)\` and an invalid rate (non-finite or ≤ 0) returns \`Err(NumeraireError::InvalidRate)\` — neither silently drops to zero. The delta\_numeraire is the scalar sum of the full \`CurrencyExposure\` vector converted to numeraire. The CAP overflow (\> 32 distinct currencies) is caught by \`debug\_assert!\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.add\_leaf\_delta` (hash `d56dd93a4e36e431`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.in\_numeraire` (hash `7f1a9cb7840ffa04`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.Numeraire.from\_leaves` (hash `e96a784c68e7b8d1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.NumeraireError.fmt

- **claim** (`cl\_21431a919831d2b9`): NumeraireError::fmt encodes exactly two currency-conversion failure modes: MissingRate(c) → 'no conversion rate from {c} into the reporting numeraire' (rate absent); InvalidRate(c) → 'non-finite or non-positive conversion rate for {c}' (rate present but mathematically invalid). The currency code c is always embedded so the offending currency is traceable from the formatted error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.NumeraireError.fmt` (hash `41eb82517c109d14`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.convert

- **claim** (`cl\_cdbf6246c522d89e`): \`convert(amount, ccy, resolver)\` is the single atom for currency conversion into the reporting numeraire: it calls \`resolver.rate\_into\_numeraire(ccy)\` returning \`Option\<f64\>\`, fails with \`NumeraireError::MissingRate(ccy)\` if absent, then hard-validates the rate is finite and strictly positive (fails with \`NumeraireError::InvalidRate(ccy)\` otherwise), and returns \`amount \* rate\`. No zero-rate silent pass-through, no NaN propagation. Pure: reads its three args, returns \`Result\<f64, NumeraireError\>\`, mutates nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.convert` (hash `317c1f03e5bbd336`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.CommodityLeaf.lower

- **claim** (`cl\_dcb2dc9d4cc488d1`): CommodityLeaf::lower is a pure type-narrowing gate: it returns Err(CarryPriceError::UnsupportedUnderlying) if the underlying is not a commodity asset, returns Err(CarryPriceError::UnsupportedCarry) if the carry variant is FxRates, and otherwise constructs a celnet\_commodity\_vanilla::CommodityInputs forwarding {spot, strike, vol, t, carry} verbatim. No computation is performed beyond validation; all field values are passed through unchanged.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.CommodityLeaf.lower` (hash `c0041800a098cacb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.CryptoLeaf.lower

- **claim** (`cl\_d763dd9d6d786949`): CryptoLeaf::lower is a pure type-narrowing gate symmetric to CommodityLeaf::lower: it returns Err(CarryPriceError::UnsupportedUnderlying) if the underlying is not a digital asset, returns Err(CarryPriceError::UnsupportedCarry) if the carry variant is FxRates, and otherwise constructs a celnet\_crypto\_vanilla::LinearInputs forwarding {spot, strike, vol, t, carry} verbatim. The two leaf lowerers share identical guard structure but differ in the underlying type probe (as\_digital\_asset vs as\_commodity) and the output type (LinearInputs vs CommodityInputs).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.CryptoLeaf.lower` (hash `39a20ebfc60cef9e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.EquityLeaf.lower

- **claim** (`cl\_0d6bce1c2a6c011b`): \`EquityLeaf::lower\` enforces a typed carry-seam discipline: it accepts only \`Carry::CostOfCarry { r, b }\` under an equity underlying and hard-rejects \`Carry::FxRates\` (which would silently mis-map FX two-rate carry onto an equity pricer) with \`CarryPriceError::UnsupportedCarry\`. The dividend yield is reconstructed as \`q = r - b\` (i.e. \`b = r - q\` in cost-of-carry form) and passed as the equity pricer's \`q\` argument with \`repo = 0.0\`. An equity underlying carrying \`Carry::FxRates\` is a malformed input, not a silent approximation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.EquityLeaf.lower` (hash `81baa7f47e52adf2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-router.src.hash.fold64

- **claim** (`cl\_9f581826a56e1688`): fold64(acc, lane) chains one 64-bit lane into a running accumulator via: rotated = acc.rotate\_left(23); mix64(rotated.wrapping\_add(mix64(lane))). The 23-bit rotation before addition ensures successive lanes occupy different bit positions before mixing, defeating trivial cancellation of equal lanes. fold64 is order-sensitive: fold64(fold64(0,1),2) != fold64(fold64(0,2),1) (tested by fold\_is\_order\_sensitive).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.hash.fold64` (hash `59ac7c888590ed80`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-router.src.hash.mix64

- **claim** (`cl\_4f122b79b63321d7`): mix64 is the splitmix64 avalanching finalizer: z = (z ^ (z\>\>30)) \* 0xbf58476d1ce4e5b9; z = (z ^ (z\>\>27)) \* 0x94d049bb133111eb; z ^ (z\>\>31). It is bijective (never collapses distinct inputs), branch-free, const-evaluable, and produces ~32-bit average Hamming distance on single-bit input perturbations. This property makes per-replica rendezvous weights behave as independent uniform draws.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.hash.mix64` (hash `569f8b447fd04688`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-router.src.hash.rendezvous\_weight

- **claim** (`cl\_869f0d13916ce2f0`): PILLAR (GUIDE.md guardrail 6 + 11 — "Design for horizontal scale-out from day one"; "scale-out aware"). \`rendezvous\_weight(replica\_seed, key\_digest)\` is the pure deterministic core of the platform's horizontal scale-out: a \`const fn\` computing the Highest-Random-Weight (HRW / rendezvous-hashing) score for one (replica, partition-key) pair as \`mix64(fold64(replica\_seed, key\_digest))\`. It reads only its two u64 inputs and returns a u64 — no allocation, no I/O, no mutation, no WRITES edges — so it is referentially transparent and the per-replica scores are reproducible on every node. This is the primitive that makes book/risk sharding deterministic and minimal-disruption under membership change: \`PartitionMap::natural\_owner\` takes the argmax of this weight over the live replica set to assign each partition key its stable owner, so adding/removing a replica re-homes only the keys whose argmax moved (the HRW property), never a global reshuffle. Self-invalidating: if the entanglement/mixing changes (anything beyond a pure two-u64 fold) the WRITES gate flips this claim off.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.hash.rendezvous\_weight` (hash `a0e606429308cbe3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-router.src.key.PartitionKey.digest

- **claim** (`cl\_7a8fcf2b07d5f820`): PartitionKey::digest() produces a deterministic u64 fingerprint by: (1) packing the six ASCII bytes of the CCY pair into a u64 lane, (2) avalanche-mixing it with mix64, (3) folding in the tenant id (tagged with a high-bit presence sentinel) and the book id in sequence via fold64. The result is fixed for a given (pair, tenant?, book?) triple and is the sole input to all routing weight computations. Distinct pairs always produce distinct digests (tested by distinct\_pairs\_distinct\_digests); adding/changing a subkey changes the digest (tested by subkeys\_change\_digest).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.key.PartitionKey.digest` (hash `1fce450548023771`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-router.src.key.tagged

- **claim** (`cl\_921bdb9c00926e87`): The \`tagged\` const fn encodes an optional 64-bit sub-key so that absence and any concrete value are provably distinguishable: \`None\` returns the sentinel \`0xA5A5\_A5A5\_A5A5\_A5A5\` (alternating bit pattern, unreachable by the \`Some\` path's output distribution); \`Some(v)\` returns \`mix64(v ^ 0x5555\_5555\_5555\_5555)\` which XOR-masks the value before mixing to break low-entropy clustering around zero. The function is \`const\` and has no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.key.tagged` (hash `0cb918205fe3e4ea`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap\<'a\>.natural\_owner

- **claim** (`cl\_ce9c06d12e46fea9`): PILLAR (GUIDE.md guardrail 6 + 11 — horizontal scale-out / shard ownership). \`PartitionMap::natural\_owner(key)\` is the pure, side-effect-free realization of HRW shard assignment: for a partition key it digests the key, scores every live replica with \`rendezvous\_weight(replica.seed, digest)\`, and returns the argmax \`ReplicaId\` — ties broken deterministically by the smaller replica id (\`.then\_with(\|\| a.0.0.cmp(&b.0.0))\`) so the owner is a total, deterministic function of (key, membership). It reads only \`&self\` (the replica set) and the key, allocates nothing in the hot path and has no WRITES edges. This is the routing decision that lets the platform fan a book/risk workload across N nodes with a stable, minimal-disruption owner per key (the day-one horizontal scale-out requirement): every node computes the same owner independently, no central coordinator, and a membership change re-homes only the keys whose argmax moved. Self-invalidating: any side effect or non-deterministic tie-break introduced here flips the WRITES/purity gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap\<'a\>.natural\_owner` (hash `440c9f51118186d1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap\<'a\>.route

- **claim** (`cl\_8452d28d33f87a5a`): PartitionMap::route() determines the natural owner in a single linear pass over all replicas by maintaining an argmax of rendezvous\_weight(r.id.seed(), digest), with a deterministic tie-break on id (lower id wins when weights are equal). It then applies the three-tier cascade: (1) natural owner up → Primary; (2) owner down, declared standby healthy → Standby; (3) otherwise re-runs the argmax restricted to up replicas → HrwFallback. This guarantees that only 1/N keys move in expectation on a single-replica failure, and surviving keys (natural owner still up) are never disturbed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap\<'a\>.route` (hash `0602b7865d8867be`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-router.src.map.RouteError.fmt

- **claim** (`cl\_f9eb32dac31eb654`): \`RouteError::fmt\` renders \`RouteError::NoHealthyReplica\` as the string \`"no healthy replica to route to"\` and \`RouteError::EmptySet\` as \`"replica set is empty"\`. These strings are the operator-visible error messages surfaced when routing fails; they must not be changed without updating downstream alerting and client error handling.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.map.RouteError.fmt` (hash `1810107a3a30c976`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-router.src.map.healthy\_standby

- **claim** (`cl\_e0e82cfe729b1058`): \`healthy\_standby\` returns \`Some(sb)\` if and only if the owner replica has a non-None \`standby\` field AND the standby's \`Health\` in the replica set is \`Health::Up\`. In all other cases (no standby configured, or standby exists but is not \`Up\`) it returns \`None\`. The function is a pure read over shared references with no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.map.healthy\_standby` (hash `ac7caee457577379`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-router.src.replica.MembershipError.fmt

- **claim** (`cl\_a9dcf5be333091af`): \`MembershipError::fmt\` renders \`MembershipError::DuplicateId(id)\` as \`"duplicate replica id {id.0}"\` and \`MembershipError::UnknownStandby(id)\` as \`"standby {id.0} is not a member of the set"\`. These are the only two membership validation failures the router can produce during replica set construction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.src.replica.MembershipError.fmt` (hash `15200dd6538745c8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-router.tests.router\_mutation.oracle\_argmax

- **claim** (`cl\_4f16bd56cc6fbaa4`): The \`oracle\_argmax\` reference function implements the Highest Random Weight (HRW / Rendezvous) argmax rule for a set of replica ids against a key digest: for each \`id\` compute \`weight = mix64(fold64(mix64(id), digest))\`, then return the id with the highest weight; ties are broken by lowest id (the \`bw \> w \|\| (bw == w && bid \<= id)\` guard keeps the incumbent). This is the independent specification that \`PartitionMap::route\` must match for its primary routing decisions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.tests.router\_mutation.oracle\_argmax` (hash `a675619e4d0dd284`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-router.tests.router\_mutation.oracle\_tagged

- **claim** (`cl\_cab066a3cf9f0771`): The \`oracle\_tagged\` oracle mirrors the production \`tagged\` const fn exactly: \`None -\> 0xA5A5\_A5A5\_A5A5\_A5A5\`, \`Some(v) -\> oracle\_mix64(v ^ 0x5555\_5555\_5555\_5555)\`. Its bit-for-bit agreement with \`tagged\` is verified transitively by \`digest\_matches\_independent\_spec\_bit\_for\_bit\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-router.tests.router\_mutation.oracle\_tagged` (hash `05e92322d3dbe0a6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-server.examples.scale\_harness.InstrumentSpec.to\_wire\_pub

- **claim** (`cl\_afbe4aeb6878ba17`): InstrumentSpec::to\_wire\_pub always constructs an FX-path proto Instrument for the scale harness: underlying is celnet\_proto::Underlying::fx, tenor is 1Y, side is TwoWay, product is a Vanilla Call with an absolute strike (from strike\_for or the spec's own Absolute strike). The function panics if the spec's underlying is not an FX pair.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.examples.scale\_harness.InstrumentSpec.to\_wire\_pub` (hash `66c66b6bf9ea8891`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.examples.scale\_harness.strike\_for

- **claim** (`cl\_db937cdc6643c02c`): strike\_for returns a fixture ATM-approximate strike for each canonical scale-harness currency pair: EUR/USD-\>1.10, GBP/USD-\>1.27, AUD/USD-\>0.66, USD/JPY-\>150.0, any other pair-\>1.0.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.examples.scale\_harness.strike\_for` (hash `604c5945db87b928`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.config.fix\_connections.AcceptorKind.parse

- **claim** (`cl\_b2e9031e155711bc`): AcceptorKind::parse accepts only the single string 'options' (case-insensitive, trimmed) and maps it to AcceptorKind::Options; all other inputs return None. This reflects that Options is the only currently supported FIX acceptor product kind.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.config.fix\_connections.AcceptorKind.parse` (hash `c8f4252005decd10`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.config.fix\_connections.FixConnectionDef.validate

- **claim** (`cl\_7e6dcb4ed1ea3c4a`): FixConnectionDef::validate enforces that id, name, sender\_comp\_id, and target\_comp\_id are all non-empty after trimming, and that bind\_addr parses as a valid host:port via socket\_addr(). All four string checks return Err with a descriptive message on failure; the bind\_addr check returns a formatted Err on failure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.config.fix\_connections.FixConnectionDef.validate` (hash `382f4467d21b5803`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.config.identity.Role.as\_str

- **claim** (`cl\_375789e479698389`): Role::as\_str is a total, allocation-free bijection from the two-variant Role enum to static string slices: Role::Admin maps to "admin" and Role::Trader maps to "trader". The match is exhaustive with no fallback arm, so any new variant causes a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.config.identity.Role.as\_str` (hash `ec2170edc096aab2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.config.identity.Role.parse

- **claim** (`cl\_89045b168f4e2dee`): Role::parse is a case-insensitive, trim-normalising parser: it maps 'admin' to Role::Admin and 'trader' to Role::Trader; all other inputs return None. The comparison is performed after trim() and to\_ascii\_lowercase(), so whitespace and case variations are accepted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.config.identity.Role.parse` (hash `8981f7c88350e21c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.config.identity.slugify

- **claim** (`cl\_e2342a0dfdc4293a`): identity::slugify is the infallible variant: it pre-trims whitespace with \`s.trim()\`, then applies the same alphanumeric-lowercase / run-collapse algorithm, and trims both leading and trailing dashes via \`trim\_matches('-')\`. Always returns a String (may be empty for an all-punctuation input).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.config.identity.slugify` (hash `6db9deaf3f86cd2e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.core\_link.BarrierTopology.style

- **claim** (`cl\_39b25eead43ef2d8`): BarrierTopology::style is a pure const mapping: DownAndOut and UpAndOut both return BarrierStyle::KnockOut; DownAndIn and UpAndIn both return BarrierStyle::KnockIn. The function is a compile-time-eligible \`const fn\` covering all four variants.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.core\_link.BarrierTopology.style` (hash `200921e1acadef28`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.core\_link.observe\_live

- **claim** (`cl\_c4d5a29a4b3d77cd`): observe\_live computes the live scalar value for a market observable from a MarketState: AtmVol returns Smile::implied\_vol at ATM (forward==strike); Spot returns st.spot; Forward returns st.forward(); RiskReversal returns call\_vol − put\_vol at the requested delta magnitude; Butterfly returns 0.5\*(call\_vol + put\_vol) − atm\_vol. The delta-to-strike inversion for wing observables uses celnet\_vanilla::strike\_from\_delta under the market's delta convention, and returns None if inversion fails.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.core\_link.observe\_live` (hash `5c7c72d7f7245e08`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.lsv\_pricer.decode\_option\_type

- **claim** (`cl\_650246a9bd11092f`): decode\_option\_type (lsv\_pricer) maps wire enum tags Call→OptionType::Call and Put→OptionType::Put, returning PriceError::UnknownEnum{kind:"OptionType", tag} for any unrecognized tag. The two-variant exhaustive match means no silent default is applied.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.lsv\_pricer.decode\_option\_type` (hash `47992611a73e3614`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.lsv\_pricer.vanilla\_strike

- **claim** (`cl\_19645141be1abf17`): vanilla\_strike rejects delta-keyed strikes at the LSV layer with a domain error, and rejects non-positive or non-finite absolute strikes. It returns (OptionType, f64) only when the strike spec is Spec::Strike(k) with k \> 0.0 and k.is\_finite().
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.lsv\_pricer.vanilla\_strike` (hash `df730c046186ccfa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.PriceError.fmt

- **claim** (`cl\_d0e51e084dd4ce81`): PriceError::fmt produces human-readable, structured error messages for all seven variants, each naming the relevant field/context: MissingField names the field, UnknownEnum names the kind and tag, Domain includes the reason, UnsupportedModel names both model and product with guidance, and LinearProductNotAnOption names the product and the responsible subsystem (linear book).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.PriceError.fmt` (hash `3e509690eb5ecb8c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.carry\_greeks\_to\_greeks

- **claim** (`cl\_43fab6efffbf0bb4`): carry\_greeks\_to\_greeks projects CarryGreeks into the wire Greeks struct. The rho bijection is: if rates == Carry { discount\_rho, carry\_rho } then rho\_dom = discount\_rho + carry\_rho, rho\_for = −carry\_rho; if rates == Fx { rho\_dom, rho\_for } the values are forwarded unchanged. All other fields are copied 1-to-1.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.carry\_greeks\_to\_greeks` (hash `1338fa79aa893706`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_fixing\_source

- **claim** (`cl\_6debf7f8af4ec02c`): decode\_fixing\_source is an exhaustive, pure bijection from the six proto FixingSource wire enum tags to the matching celnet\_types::FixingSource domain values: KrwKftc18, TwdTaipei, InrRbiRef, BrlPtax, ClpDolarObs, CopTrm. An unrecognized tag returns PriceError::UnknownEnum{kind:"FixingSource", tag}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_fixing\_source` (hash `28b36d9dcbfb8541`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_linear\_side

- **claim** (`cl\_9137adf509c90ccf`): decode\_linear\_side rejects Side::TwoWay with a domain error ("a linear product needs a definite BUY/SELL side, not TWO\_WAY"), maps Buy→LinearSide::Buy and Sell→LinearSide::Sell, and returns PriceError::UnknownEnum for any unrecognized tag. This enforces that linear products always carry a definite directional side.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_linear\_side` (hash `68014812e4e2c12f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_settlement\_style

- **claim** (`cl\_41d12bea8071f11d`): Wire→domain SettlementStyle decode (docs/CONVENTIONS.md determinism + docs/INTERFACES.md enum contract): decode\_settlement\_style is a pure total function of the i32 wire tag — it round-trips through celnet\_proto::SettlementStyle::try\_from then the domain SettlementStyle, lowering Linear→CryptoSettlementStyle::Linear and InverseCoin→CryptoSettlementStyle::InverseCoin; an out-of-range tag is a typed PriceError::UnknownEnum, never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the enum mapping or error path changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode\_settlement\_style` (hash `a1c51a9c3c100c73`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.fx\_wire\_greeks

- **claim** (`cl\_7d1d34819874ea4e`): fx\_wire\_greeks projects CarryGreeks into wire Greeks identically to carry\_greeks\_to\_greeks but is called on the FX path (where the Fx arm is the expected branch). The Carry arm is a fallback that projects losslessly via rho\_dom = discount\_rho + carry\_rho, rho\_for = −carry\_rho.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.fx\_wire\_greeks` (hash `bfde9a1a3888350e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.is\_cross\_asset

- **claim** (`cl\_27fbdccb688af790`): CAPABILITY (cross-asset carry-seam reach): is\_cross\_asset is the pure routing predicate that decides — by Underlying variant alone (Equity \| Commodity \| DigitalAsset) — which instruments leave the FX vanilla path for the cross-asset leaf engines (equity generalized-BSM, commodity Black-76, crypto inverse/linear) on the shared carry seam. FX/metal stays on the vanilla path (metal lease rate modelled as the FX foreign rate), byte-identically. Side-effect-free classifier: reads only the &Underlying, returns bool, no WRITES — the gate self-invalidates if the variant set or the routing shifts.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.is\_cross\_asset` (hash `3e2a359d520ef186`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.price\_listed\_future\_option

- **claim** (`cl\_820da9cb6ff51f18`): DELIVERABLE proto/new-payoff-shapes (ListedFutureOption arm) = DONE (RC cut a0817d6; arm 31). \`price\_listed\_future\_option(o, market, expiry)\` is the side-effect-free server lowering of a wire listed-future option onto the Black-76 commodity-on-future leaf: it first runs the canonical term validator (\`validate\_listed\_future\_terms\` — present \`future\_symbol\`, known margining tag, \`future\_expiry\_years \>= expiry\_years \> 0\`), rejects a non-positive/non-finite strike as \`PriceError::Domain\` and an out-of-range margining tag as \`PriceError::UnknownEnum\` (never clamped/defaulted), maps the margining enum to \`CommodityMargining\`, builds \`CommodityInputs::on\_future(spot, strike, vol, expiry, r\_dom)\`, and returns the leaf Greeks via \`greeks\_with\_margining\`. Pure: it reads only \`(o, market, expiry)\` and returns \`Result\<Priced, PriceError\>\`, mutating no external state. The booked future\_symbol is contract identity, not a pricing input. This is the second of the two genuinely-new payoff shapes the backlog tracked as OPEN; now built + golden/parity-gated across all 5 clients, with futures-style honest-zero discount-rho asserted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price\_listed\_future\_option` (hash `23810671bbd0a4db`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.product\_name

- **claim** (`cl\_750834a8ea32ed78`): CAPABILITY (carry-seam deliverable, one-contract product-family reach): product\_name is the pure total over the full Product enumeration — it names all 24 product arms reachable through the one unversioned contract (vanilla, strategy, single/double barrier, digital, touch, variance\_swap, volatility\_swap, asian\_option, forward\_start, cliquet, quanto, tarf, pivot, accumulator, lookback, window\_barrier, american, basket, fx\_forward, fx\_swap, ndf, perpetual\_option, listed\_future\_option) with no fallback arm. Pure: maps &Product to a &'static str with no WRITES; the exhaustive match self-invalidates the moment a product arm is added or removed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.product\_name` (hash `c9d075fbdeb42a69`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.resolve\_strike

- **claim** (`cl\_9cc85bb241428536`): resolve\_strike converts a StrikeOrDelta spec into an absolute strike: a Spec::Strike(k) variant is returned as-is (identity); a Spec::Delta(d) variant is inverted via celnet\_vanilla::strike\_from\_delta using the market context (spot, vol, expiry\_years) and the caller's delta convention, propagating any inversion failure as PriceError::DeltaSolve.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.resolve\_strike` (hash `d80d693b60baa4dc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.pricer.wire\_error\_to\_price\_error

- **claim** (`cl\_a6534c5a2cfb28bb`): wire\_error\_to\_price\_error is a total, non-panicking lowering from all WireError variants to PriceError: UnknownEnum→UnknownEnum, MissingField→MissingField, InvalidTerms→Domain(constraint), InvalidCcy→Domain(static msg), OutOfRange→Domain(static msg), WrongUnderlying→Domain(static msg). No variant is unhandled.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.pricer.wire\_error\_to\_price\_error` (hash `9a083a00f5eab376`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.readiness.ServiceState.from\_tag

- **claim** (`cl\_ea01cf35d9a58222`): ServiceState::from\_tag is a const, pure, total function mapping u8 tag values to ServiceState variants: 1→Ready, 2→Draining, and any other value (including 0)→Starting. The wildcard default of Starting means unrecognized tags never error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.readiness.ServiceState.from\_tag` (hash `b9dc9daa63543eae`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.readiness.ServiceState.tag

- **claim** (`cl\_8e93e02107d2be0b`): ServiceState::tag encodes the three lifecycle states as a stable, injective u8 discriminant: Starting→0, Ready→1, Draining→2. The mapping is a const fn with no branches outside the exhaustive match, so the tag is guaranteed unique and total over the enum.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.readiness.ServiceState.tag` (hash `6637a46bceec9722`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.access.DeskScope.allows

- **claim** (`cl\_2f220d7adf8a1a5b`): DeskScope::allows() is a pure three-way predicate: \`All\` permits any desk name, \`Desk(d)\` permits only exact string equality with \`owner\_desk\`, and \`Deskless\` permits only the empty-string desk. It has no side effects and allocates nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.access.DeskScope.allows` (hash `a9e65ccead16728c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.access.ResolvedCaller.desk\_scope

- **claim** (`cl\_ddae5a1226ff4e0e`): ResolvedCaller::desk\_scope derives the caller's DeskScope from their user record: an unauthenticated caller (None) and any admin user both receive DeskScope::All; a non-admin user with a desk\_id receives DeskScope::Desk(desk\_id); a non-admin user with no desk\_id receives DeskScope::Deskless. This is the access-control boundary that prevents non-admin traders from seeing other desks' data.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.access.ResolvedCaller.desk\_scope` (hash `f22a81015ff97ae4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.access.mode\_from\_env\_or

- **claim** (`cl\_969b251f1b4a0e23`): mode\_from\_env\_or reads the CELNET\_ACCESS\_MODE environment variable and maps "permissive" -\> AccessMode::Permissive, "enforce" -\> AccessMode::Enforce, and any other value (including unset) -\> the provided default. This is the single authoritative resolution point for the access mode override; the demo edge passes Permissive as default, all production paths pass Enforce.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.access.mode\_from\_env\_or` (hash `21f4b9c56987ddd8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.access.principal\_label

- **claim** (`cl\_a46f85ede7445816`): principal\_label produces a human-readable diagnostic string for an EntitlementPrincipal: None → "absent"; grant\_all with denies → "grant-all(+N denies)"; scoped → "scoped(M grants, N denies)". The function is pure and allocation-only (no I/O, no mutation).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.access.principal\_label` (hash `3657d9bf9912d0a9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.attribution.resolve

- **claim** (`cl\_ec5cb2808e2f1a9a`): attribution::resolve() produces an AttributionRecord for a new quote: the server's maker auto-pricer identity is always installed as \`quoted\_by\`. When a client-side attribution is supplied, its \`quoted\_by\` seat becomes the \`held\_by\` (the requesting client holds the position). Without a client attribution, \`held\_by\` is None. This is a pure transformation with no I/O.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.attribution.resolve` (hash `3cd3e6bed8ad8c49`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.auth.role\_from\_wire

- **claim** (`cl\_dd9cc0567b2cbd6d`): role\_from\_wire maps the proto i32 discriminant bijectively to domain Role: UserRole::Admin -\> Ok(Role::Admin), UserRole::Trader -\> Ok(Role::Trader), any unrecognised value -\> Err(Status::invalid\_argument). The function is a total, exhaustive match with no side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.auth.role\_from\_wire` (hash `726b8b2cb9e6a483`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.auth.role\_to\_wire

- **claim** (`cl\_56c7b662696a95b8`): role\_to\_wire is the left inverse of role\_from\_wire on the valid domain: Role::Admin -\> UserRole::Admin as i32, Role::Trader -\> UserRole::Trader as i32. The mapping is total and free of allocation or side-effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.auth.role\_to\_wire` (hash `174745086ee2ed81`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix.instrument\_from\_descriptor

- **claim** (`cl\_4c88fc967ef5ca22`): instrument\_from\_descriptor constructs a canonical Instrument protobuf for a FIX new-order: it always sets Side::TwoWay, a 1,000,000 base-ccy notional, no tenor (None), PricingModel::Default, and wraps the OptionDescriptor's call/put type and absolute strike into a Vanilla product. The function is a pure mapping with no I/O or state access.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix.instrument\_from\_descriptor` (hash `92aaabdfedf36dc2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.direction\_to\_wire

- **claim** (`cl\_ed55eaaed1239a77`): direction\_to\_wire is a pure, exhaustive bijection from the two-variant FixDirection enum to the corresponding FixMsgDirection proto enum: Inbound→FixMsgDirection::Inbound, Outbound→FixMsgDirection::Outbound.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.direction\_to\_wire` (hash `9b6de777a6040043`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.kind\_from\_wire

- **claim** (`cl\_e8e180045cfa054f`): kind\_from\_wire is a total, pure mapping: i32 wire value -\> Result\<AcceptorKind, Status\>. It accepts three valid discriminants (FixAcceptorKind::Options -\> AcceptorKind::Options, FixedIncomeQuote -\> AcceptorKind::FixedIncomeQuote, FixedIncomeStream -\> AcceptorKind::FixedIncomeStream) and returns Status::invalid\_argument for every other value. No silent fallback or default exists.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.kind\_from\_wire` (hash `db70525029fd699f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.slugify

- **claim** (`cl\_cf6e0588e4963b9b`): fix\_admin::slugify converts a display name to a lowercase URL-safe slug: alphanumeric characters are lowercased and emitted verbatim; runs of non-alphanumeric characters collapse to a single '-'; trailing dashes are trimmed (trim\_end\_matches). Returns None if the result is empty.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_admin.slugify` (hash `b932c2e5a5ce71b1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_monitor.FixDirection.as\_str

- **claim** (`cl\_c0a41fdbcd40e408`): FixDirection::as\_str is a total, allocation-free bijection: FixDirection::Inbound maps to "inbound" and FixDirection::Outbound maps to "outbound". The exhaustive match ensures no unhandled variant can exist at compile time.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_monitor.FixDirection.as\_str` (hash `8e5c800c6640b542`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_monitor.label\_for

- **claim** (`cl\_be4ab2d80fb07df4`): label\_for maps FIX MsgType tag values to human-readable names as a pure total function with a two-level fallback: the empty string '' maps to 'Unknown' and any unrecognised code maps to 'Other'. The covered set is the session-admin + RFQ/quote lifecycle subset: Heartbeat(0), TestRequest(1), ResendRequest(2), Reject(3), SequenceReset(4), Logout(5), Logon(A), QuoteRequest(R), Quote(S), QuoteResponse(AG), QuoteAcknowledgement(AJ), MassQuoteAcknowledgement(b), NewOrderSingle(D), NewOrderMultileg(AB), ExecutionReport(8), OrderCancelReject(9), BusinessMessageReject(j). The function is allocation-free — all return values are &'static str.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.fix\_monitor.label\_for` (hash `df422a74e1cc95f6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.forward.serve\_mode

- **claim** (`cl\_bfdbff21613809da`): forward::serve\_mode is a total, pure two-way dispatch: Some(fleet) -\> Serve::Forward(fleet), None -\> Serve::Local. An absent fleet always yields local serving; a present fleet always yields forwarding.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.forward.serve\_mode` (hash `b686c87bc5ea99b5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.pin.resolve\_pinned\_vol

- **claim** (`cl\_169733a5ce0d7eb1`): Surface-version pinning REJECTS an unknown version rather than silently pricing off the live surface. \`resolve\_pinned\_vol\` short-circuits to the live market (no echo) when no surface\_version is pinned; otherwise it requires an FX \`underlying\` (else invalid\_argument) and looks the pin up via book.pinned\_vol. Three outcomes are total: Ok(Some(vol)) ⇒ price against the marked vol and echo the version; Ok(None) ⇒ the version EXISTS but did not mark THIS pair, so keep the live vol yet still echo the (valid) version; Err(PinError::UnknownVersion) ⇒ the version was never marked, returned as Status::failed\_precondition — the pin is refused, never honoured against an arbitrary surface. This makes a pinned RFQ/RFS deterministic: it reprices against the exact marked model or fails closed. Pure: reads book/version/instrument/market and returns Result\<PinnedVol, Status\>, constructing new PinnedVol/MarketContext values (with\_vol) and mutating no external state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.pin.resolve\_pinned\_vol` (hash `09b4bc05eb29c0ed`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.pricefanout.spot\_at

- **claim** (`cl\_0d88d48333ae9e28`): \`spot\_at(seed, tick\_seq, base\_spot)\` is a pure deterministic spot-price generator for the price fan-out stream. It computes: \`mixed = splitmix64(seed XOR (tick\_seq \* 0x2545\_F491\_4F6C\_DD1D))\`, then \`u = unit\_signed(mixed)\` (a signed uniform in (-1,1)), and returns \`base\_spot \* (u \* STREAM\_BUMP + 1.0)\`. The same \`(seed, tick\_seq)\` pair always produces the same price; distinct pairs produce independent draws.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.pricefanout.spot\_at` (hash `87fb5a8c0997f474`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.aggregate.resolved\_group\_value

- **claim** (`cl\_3ef19c2c5f547dfe`): resolved\_group\_value resolves the u64 group key for a risk fact under a given dimension, with two hierarchy-aware overrides: for DimensionId::Desk it looks up the book's owning desk via Hierarchy::desk\_of and uses the desk's raw id if found (falling back to the fact's own group\_value when no desk mapping exists); for Entity it similarly lifts via entity\_of(location). All other dimensions delegate directly to fact.key.group\_value(dim).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.aggregate.resolved\_group\_value` (hash `fbd08074ce8ce9d2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.dimension\_of

- **claim** (`cl\_4bbf5ac089994326`): dimension\_of() translates a wire i32 \`RiskDimension\` discriminant to an \`Option\<DimensionId\>\`. The \`Firm\` dimension maps to \`None\` (no sub-grouping key), all other dimensions map to \`Some(DimensionId::\*)\`. Returns \`Status::invalid\_argument\` if the i32 is not a known variant.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.dimension\_of` (hash `e4992fd6ad01bdf2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.dimension\_to\_wire

- **claim** (`cl\_483663256b753532`): dimension\_to\_wire is an exhaustive pure bijection from the six DimensionId variants (Trader, Book, Desk, Underlying, Location, Entity) to their corresponding RiskDimension proto enum i32 values via a cast. No default arm exists; adding a new DimensionId variant causes a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.dimension\_to\_wire` (hash `ca35a2b9dbe45edc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.enforcement\_to\_wire

- **claim** (`cl\_596855c7f919c71c`): enforcement\_to\_wire is a pure, exhaustive bijection from the two-variant Enforcement enum to WireEnforcement proto i32 values: Enforcement::Soft→WireEnforcement::Soft as i32, Enforcement::Hard→WireEnforcement::Hard as i32.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.enforcement\_to\_wire` (hash `2a80756e05b70118`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.limit\_metric\_to\_wire

- **claim** (`cl\_411df27707a20b0d`): limit\_metric\_to\_wire is a pure, total mapping from LimitMetric enum variants to a wire triple (LimitMetricKind, Option\<WireVegaPillar\>, u32). VegaBucket carries a non-None pillar; TenorVega carries its tenor\_days in the u32 slot; all scalar Greeks and risk measures carry (kind, None, 0). The mapping covers all 11 variants exhaustively.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.limit\_metric\_to\_wire` (hash `2280a2a7bc263cce`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.rag\_to\_wire

- **claim** (`cl\_f014b8f36312c113`): rag\_to\_wire is a total, exhaustive bijection from the four RagStatus variants (Green, Amber, Red, Breach) to the corresponding WireRag proto discriminants, returned as i32. Every variant is covered; no default branch exists.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.rag\_to\_wire` (hash `855ee92fc0bafd02`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.rule\_of

- **claim** (`cl\_7bb6ef772b59ab68`): rule\_of folds a proto EntitlementRule's scopes into a domain Rule via conjunction: it starts from Rule::firm() (the permissive root) and calls \`.and(dim, scope.value)\` for each scope whose dimension resolves to Some; FIRM-pinned scopes (None dimension) are no-ops. An empty rule returns the firm root unchanged.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.rule\_of` (hash `cd55e2224ee126f2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.scope\_to\_rule

- **claim** (`cl\_2dc79f7e543a257f`): scope\_to\_rule converts a RiskScope to a celnet\_limits Rule: an absent/None dimension yields Rule::firm() (firm-wide limit); any recognised dimension yields Rule::on(dim, scope.value); an unknown enum tag propagates invalid\_argument from dimension\_of. The function is a thin, stateless adapter between the wire and the limit-tree rule vocabulary.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.convert.scope\_to\_rule` (hash `b2654ef3f1389cdc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.mod.RiskEdge.serve\_mode

- **claim** (`cl\_2c578a8e4a4e435d`): RiskEdge::serve\_mode returns Serve::Direct for InProcess topology, Serve::Federate(fleet) for a connected Distributed topology, and Err(Status::unavailable) if the Distributed topology has not been connected (fleet is None) — making the connection pre-condition explicit at the call site.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.mod.RiskEdge.serve\_mode` (hash `98285dc323312425`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.mod.limit\_scope\_of

- **claim** (`cl\_99b612401605471e`): limit\_scope\_of maps a wire RiskScope to a typed LimitScope: None dimension yields LimitScope::Firm; known dimensions (Trader, Book, Desk, Location, Entity) wrap the value field in the corresponding strongly-typed Id newtype (u32::try\_from with 0 fallback). An Underlying dimension is explicitly rejected with Status::invalid\_argument because a CCY\_PAIR scope cannot be addressed by a bare u64 value alone.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.mod.limit\_scope\_of` (hash `ed3fc39690cb35c8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.risk.store.seat\_name

- **claim** (`cl\_907ad6b6c3e5bf1e`): seat\_name extracts a stable string key from a BookId's Owner seat: Trader(t) -\> t.clone(), AutoPricer(p) -\> p.clone(), absent seat -\> "unknown-seat". The function is total (covers all cases via None fallback).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.store.seat\_name` (hash `f7af8c72379e106e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.stream.pair\_of\_client\_message

- **claim** (`cl\_298873a5e81873cb`): pair\_of\_client\_message extracts the CcyPair from a ClientStreamMessage for exactly two message variants: Subscribe (via instrument.underlying.as\_fx()) and MarketSeriesSubscribe (via underlying.as\_fx()). All other message variants return None, making it a safe, pure filter that never mutates state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.stream.pair\_of\_client\_message` (hash `3c5586921a6f9095`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.stream.seq\_of

- **claim** (`cl\_1e89e9bb09acb36c`): seq\_of() extracts the sequence number from a ServerStreamMessage: returns \`Some(s.sequence)\` for a Snapshot, \`Some(u.sequence)\` for an Update, and \`None\` for any other variant. It is a pure projection.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.stream.seq\_of` (hash `0762b52181fbe461`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.stream.snapshot\_tokens

- **claim** (`cl\_4a235971203b556d`): snapshot\_tokens() extracts the tradable-token slice from a ServerStreamMessage. It returns the \`tradable\` vec from a Snapshot variant, and an empty Vec for any other message variant. It is a pure projection with no state or side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.stream.snapshot\_tokens` (hash `e8cc8637e73ae57d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.stream.snapshot\_tokens\_update

- **claim** (`cl\_057734fbc6bdb0e0`): snapshot\_tokens\_update extracts the tradable-token list from a ServerStreamMessage: it returns the Update variant's tradable field verbatim if the message holds an Update, and an empty Vec for every other variant (Snapshot, Heartbeat, StreamEnd, etc.). The function never mutates state and always returns a clone of the wire tokens.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.stream.snapshot\_tokens\_update` (hash `6f4718fc100dfe5b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.surface.bump\_factor

- **claim** (`cl\_3a050adaeb3a9cb9`): bump\_factor() applies a one-sided finite-difference bump to a WireMarketContext for the given shock axis. Spot is bumped relatively (\`spot \* CROSS\_SPOT\_REL\`); vol and rates are bumped absolutely (\`CROSS\_VOL\_ABS\`, \`CROSS\_RATE\_ABS\`); the Time axis returns a zero step size (time is rolled via expiry by the caller, not the market context). Returns the bumped context and the step size h used, enabling the caller to form centered or forward finite differences.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.bump\_factor` (hash `6ace29de4e21775b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.surface.decode\_smile\_model

- **claim** (`cl\_31e579a005f01348`): decode\_smile\_model maps an optional wire SmileModel tag to the internal SmileModel enum: a missing field (None) defaults to SmileModel::MarketHedge (backward-compatible default). All five proto variants (MarketHedge, StochasticVol, Parametric, ParametricSurface, ExtendedSurface) are covered one-to-one. An out-of-range tag is rejected with Status::invalid\_argument before any surface operation proceeds.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.decode\_smile\_model` (hash `13e1016274e20780`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.surface.expected\_wire\_tag

- **claim** (`cl\_c3a6e9813203947b`): expected\_wire\_tag is a pure, exhaustive bijection from the five SmileModel variants to their proto wire integer tags: MarketHedge→0, StochasticVol→1, Parametric→2, ParametricSurface→3, ExtendedSurface→4. It is intentionally independent from the production wire\_smile\_model function so a shared bug cannot cause both to pass tests simultaneously.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.expected\_wire\_tag` (hash `a34a07e1cd3737f9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.surface.model\_label

- **claim** (`cl\_2be3704c1a82aa26`): model\_label is a pure, exhaustive mapping from SmileModel enum to a &'static str label covering all five variants: MarketHedge-\>'market-hedge', StochasticVol-\>'stochastic-vol', Parametric-\>'parametric', ParametricSurface-\>'parametric-surface', ExtendedSurface-\>'extended-surface'. Used to provide model-origin transparency in surface responses.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.model\_label` (hash `54f5ef7a4b5134de`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.services.surface.wire\_smile\_model

- **claim** (`cl\_1a1dd93169dde8db`): wire\_smile\_model is a total, bijective mapping from the internal SmileModel enum to the corresponding celnet\_proto::SmileModel variant, covering all five variants (MarketHedge, StochasticVol, Parametric, ParametricSurface, ExtendedSurface) with no default arm.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.wire\_smile\_model` (hash `14472ca823b4b9d5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.american\_decodes\_from\_json

- **claim** (`cl\_36bde7a244eb8b9a`): american\_decodes\_from\_json verifies two codec contracts: (a) an American/continuous option with absent lsm\_paths decodes with lsm\_paths==0 and bermudan\_dates empty; (b) a Bermudan variant with explicit bermudan\_dates array of 4 year-fractions, lsm\_paths=100\_000, lsm\_exercise\_dates=50, and lsm\_seed=7 round-trips all four fields with bit-exact year-fraction values.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.american\_decodes\_from\_json` (hash `571b61f070284f97`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.american\_from\_json

- **claim** (`cl\_bc8320054ff66561`): american\_from\_json deserializes a JSON object under key "american" into an AmericanOption proto, applying three invariants: (1) missing or null "bermudan\_dates" always yields an empty Vec (never an error); (2) lsm\_paths and lsm\_exercise\_dates are bounds-checked via u32::try\_from and return a descriptive error string if out of range; (3) absent optional LSM fields default to zero via u64\_or\_zero. The function is a pure JSON→proto transcription with no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.american\_from\_json` (hash `cc333aa600c882d0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.attribution\_to\_json

- **claim** (`cl\_8ef09b148d42ea24`): attribution\_to\_json serializes an AttributionRecord to a JSON object containing only the present optional fields: "quotedBy" (via book\_id\_to\_json), "heldBy" (via book\_id\_to\_json), "won" (bool), and "lpCount" (u32). Absent/None fields are omitted entirely from the output object.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.attribution\_to\_json` (hash `c53993eb65a67e50`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.basket\_instrument\_round\_trips\_from\_json

- **claim** (`cl\_f6249c77889455c1`): basket\_instrument\_round\_trips\_from\_json verifies that a two-leg basket instrument (EUR/USD 0.5 weight + GBP/USD 0.5 weight, 4-element correlation matrix \[1.0, 0.4, 0.4, 1.0\], BasketKind::WorstOf, mc\_paths=8192, mc\_replications=16) round-trips through instrument\_from\_json with bit-exact f64 preservation for spot, vol, and strike values.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.basket\_instrument\_round\_trips\_from\_json` (hash `73dfe1ecb06a1d4b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.fx\_forward\_instrument\_round\_trips\_from\_json

- **claim** (`cl\_cfa3499e72fa8a4d`): fx\_forward\_instrument\_round\_trips\_from\_json verifies bit-exact round-trip of an FX forward instrument through instrument\_from\_json: contract\_rate=1.25, notional=1\_000\_000.0, and side=Buy (enum discriminant 0) are all preserved exactly, with f64 fields compared via to\_bits() to guard against any lossy parse path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.fx\_forward\_instrument\_round\_trips\_from\_json` (hash `595b3bc7fa421bd2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.fx\_swap\_instrument\_round\_trips\_from\_json

- **claim** (`cl\_6c387127fc176001`): fx\_swap\_instrument\_round\_trips\_from\_json verifies that a JSON FX-swap instrument decodes via instrument\_from\_json with exact IEEE-754 bit-identical field preservation: near.side == Buy (0), far.side == Sell (1), and near.notional.to\_bits() == 2\_000\_000.0\_f64.to\_bits(). The two-leg side inversion (near Buy / far Sell) is the invariant being sealed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.fx\_swap\_instrument\_round\_trips\_from\_json` (hash `7512afca6b70dff7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.instrument\_underlying\_from\_json

- **claim** (`cl\_6ad3e3ace5aeaf45`): One clean unversioned contract with a single underlying discriminator: when decoding an instrument, \`instrument\_underlying\_from\_json\` treats the rich cross-asset \`underlying\` oneof as authoritative whenever it is present and non-null (it carries the asset-class discriminator), and consults the legacy FX \`pair\` key only when no \`underlying\` is present — projecting it to \`Underlying::fx\`. There is exactly one precedence order (underlying ≻ pair), not a versioned negotiation. Pure: it reads the JSON map and returns a \`Result\`, mutating no state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.instrument\_underlying\_from\_json` (hash `5470f97c3981ea7c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.leg\_from\_json

- **claim** (`cl\_c8387cb03551a909`): DELIVERABLE proto/strategy-per-leg-expiry = OPEN (Round-2 P3/M finding; reconciled against the live graph — still genuinely open at this round). \`leg\_from\_json\` is the pure (side-effect-free) server decoder of one strategy/structure leg off the JSON wire: it reads only the borrowed serde Value and constructs a \`Leg { option\_type, strike, side, ratio }\`, mutating no shared state. It is the current-state witness that the wire \`Leg\` carries NO per-leg expiry/tenor field — every leg shares the enclosing Instrument expiry — so single-expiry-only multi-leg structures (a 1M-vs-3M calendar/diagonal spread cannot be booked as one net-premium ticket) remain unexpressible on the one unversioned contract. SELF-INVALIDATES: when a per-leg \`tenor\`/\`expiry\` field is added to the wire Leg and decoded here, this decoder's content hash changes and the claim flips stale, signalling the gap closed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.leg\_from\_json` (hash `2d0358c2c845f555`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.mark\_surface\_request\_from\_json

- **claim** (`cl\_3487f198a89fe72e`): DELIVERABLE surface/crypto-strike-axis-surfacing = OPEN (first post-RC fast-follow; RC anchor a0817d6). \`mark\_surface\_request\_from\_json(o)\` is the pure JSON→MarkSurfaceRequest decoder and it documents the CURRENT wire shape of the mark-surface contract: it accepts ONLY \`pair\` (optional), a required \`broker\_quotes\` array of delta-space (RR/BF) broker quote sets, \`conventions\`, and an optional \`smile\_model\`. There is NO \`quote\_basis\`/\`StrikeQuoteSet\` strike-axis quote field — so although the strike-axis surface LEAF (fit\_strike\_slice/strike\_surface) is built and parity-gated, a crypto surface STILL cannot be marked by strike from any client. Closing the fast-follow means adding a strike-axis quote oneof here (and the SurfaceEdge strike-slice ingestion → strike\_surface), which will change this function's node content and STALE this claim — the staleness firing IS the done-signal for the surfacing deliverable. Pure: it reads only the JSON map and returns \`Result\<MarkSurfaceRequest\>\`, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.mark\_surface\_request\_from\_json` (hash `c7947e6c6611c546`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.ndf\_instrument\_round\_trips\_from\_json

- **claim** (`cl\_e8b07f955266d859`): ndf\_instrument\_round\_trips\_from\_json verifies exact JSON decode of an NDF instrument: contract\_rate bit-identical to 5.1, fixing==FixingSource::BrlPtax (discriminant 3), settlement\_ccy=="USD". The fixing source integer-to-enum mapping is the load-bearing invariant being sealed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.ndf\_instrument\_round\_trips\_from\_json` (hash `5b58f66880abbd9b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_f64

- **claim** (`cl\_bb110db515bf96e3`): opt\_f64 maps a JSON map field to Option\<f64\>: absent key or JSON null both yield None; any other value delegates to serde\_json's as\_f64(). Mirrors opt\_u64 with identical structure for f64.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_f64` (hash `d24735f968814904`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_nested

- **claim** (`cl\_96ff70c30051a5f7`): opt\_nested decodes an optional nested JSON object field via a caller-supplied decoder f: absent key or JSON null yield Ok(None); a present value passes through f and wraps the result in Some. This is the sole gateway for optional object fields in the codec — it never inspects field internals directly.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_nested` (hash `9f6c5a7d43d1c06f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_smile\_model

- **claim** (`cl\_a79c52e318bb39b8`): opt\_smile\_model decodes an optional SmileModel field that accepts two wire forms: a JSON integer (numeric discriminant) or one of the four canonical string tags (SMILE\_MODEL\_MARKET\_HEDGE, SMILE\_MODEL\_STOCHASTIC\_VOL, SMILE\_MODEL\_PARAMETRIC, SMILE\_MODEL\_PARAMETRIC\_SURFACE). Absent, null, unknown strings, or non-numeric/non-string types all yield None.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_smile\_model` (hash `a1a0ccc9b519d95f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_u64

- **claim** (`cl\_6cbb5f5adc5071af`): opt\_u64 maps a JSON map field to Option\<u64\>: absent key or JSON null both yield None; any other value delegates to serde\_json's as\_u64(). The function is a pure, two-branch read with no allocation or side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.opt\_u64` (hash `d2d79a986fc8f65f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.owner\_to\_json

- **claim** (`cl\_20263fd46f6b18f5`): owner\_to\_json serialises a proto Owner to a JSON object containing exactly one key: 'trader' (string) when the seat is Trader, or 'autoPricer' (string) when the seat is AutoPricer. An absent seat yields an empty object. No other keys are emitted.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.owner\_to\_json` (hash `c51bd13ab3599738`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.pivot\_from\_json

- **claim** (`cl\_ca9ccff0ee3d4d64`): DELIVERABLE pivot-wire-surfacing = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P1/M finding "Pivot TRA is engine-only — no proto arm, no golden vector, no parity row, unreachable from any client"; reconciled against the live graph). \`pivot\_from\_json\` is the pure (side-effect-free) server decoder that lowers a wire pivot instrument off the JSON edge into the celnet-exotics \`Pivot { option\_type, strike, pivot, target, leverage, redemption, schedule, mc\_pairs, mc\_seed }\` MC spec — proving the pivot TARGET-redemption-accumulator product is now reachable on the one unversioned contract (engine celnet-exotics::pivot::pivot\_tra\_price, GUI pricePivot, golden/parity celnet-parity::pivot\_wire with the pivot==strike→TARF degenerate collapse and code-disjoint MC oracle). SELF-INVALIDATES on any change to this decoder.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.pivot\_from\_json` (hash `484afb44ab43a785`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.product\_from\_json

- **claim** (`cl\_309589372d94a048`): product\_from\_json dispatches on the presence of a product key in the JSON map (vanilla, strategy, single\_barrier, … listed\_future\_option — 24 variants) via a first-match-wins if/else-if chain mirroring the proto oneof. If no key matches, it returns a descriptive error naming all 24 accepted keys. Pure read.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.product\_from\_json` (hash `377220505890b7de`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.server\_stream\_message\_to\_json

- **claim** (`cl\_4a369b3618187b90`): server\_stream\_message\_to\_json serialises a ServerStreamMessage to a tagged JSON value, returning None if the message field is absent. Every variant of the server\_stream\_message::Message oneof is covered: Snapshot, Update, Heartbeat (includes all zero-alloc observability fields: conflation\_drops, server\_price\_p50/p99/p999\_nanos, surface\_version, correlation\_id), StreamEnd, Executed, StreamReject, MarketSeriesSnapshot, and MarketSeriesPoint. The result is always wrapped in codec::tagged so the type field matches what a WS client pattern-matches on.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.server\_stream\_message\_to\_json` (hash `5d4c07685e5d6bdf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.smile\_model\_label

- **claim** (`cl\_f14cd7f3bf037051`): smile\_model\_label maps each SmileModel proto discriminant to a vendor-neutral kebab-case label: MarketHedge-\>'market-hedge', StochasticVol-\>'stochastic-vol', Parametric-\>'parametric', ParametricSurface-\>'parametric-surface', ExtendedSurface-\>'extended-surface'; any unrecognised tag returns 'unknown'. The function is total, pure, and returns a static string.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.smile\_model\_label` (hash `0de039cf9e94b407`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.tagged

- **claim** (`cl\_e4f100f9e66f3777`): tagged(tag, body) produces a JSON object whose 'type' key is set to the given tag discriminator. When body is already a JSON Object the tag is inserted directly into that object's map; otherwise the body is wrapped under the key 'value' first. The result always carries exactly one 'type' field equal to tag.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.tagged` (hash `8dad18f43eb2da01`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.underlying\_object\_from\_json

- **claim** (`cl\_09ead42fbed4c74f`): underlying\_object\_from\_json dispatches on exactly one present key among {fx, metal, equity, commodity, digital\_asset} in the 'underlying' JSON object and delegates to the matching typed constructor. If none of the five keys is present the function returns an error; if two are present, the first matching arm wins (priority order: fx \> metal \> equity \> commodity \> digital\_asset).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.underlying\_object\_from\_json` (hash `fc0fddf40a0103df`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.vanilla\_instrument\_round\_trips\_from\_json

- **claim** (`cl\_8bc33faabeda245b`): vanilla\_instrument\_round\_trips\_from\_json confirms that the WS codec decodes a vanilla JSON instrument with bit-exact f64 preservation: expiry\_years 1.0 and strike 1.12 both satisfy to\_bits equality after the JSON→proto round-trip, proving no lossy float conversion.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.vanilla\_instrument\_round\_trips\_from\_json` (hash `fdcdc3fb9337789f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave1\_products\_decode\_from\_json

- **claim** (`cl\_9d1eca2601b74499`): wave1\_products\_decode\_from\_json verifies that the WS JSON codec correctly decodes VarianceSwap (strike\_vol bit-exact), VolatilitySwap, and AsianOption (observations, strike, elapsed\_weight all bit-exact f64) from their respective JSON discriminators.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave1\_products\_decode\_from\_json` (hash `b2b2a23d686b81e8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave2\_products\_decode\_from\_json

- **claim** (`cl\_06b1aeec58effd3d`): wave2\_products\_decode\_from\_json verifies the WS codec for ForwardStart (moneyness/reset bit-exact), plain Cliquet (optional floor/cap all None when absent), clamped Cliquet (local\_floor=Some(0.0), local\_cap=Some(0.03), mc\_pairs/mc\_seed exact), and Quanto (payoff discriminant, strike/correlation bit-exact f64).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave2\_products\_decode\_from\_json` (hash `5975d35c96cfbc68`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave3\_products\_decode\_from\_json

- **claim** (`cl\_08ae3e673358a7b6`): wave3\_products\_decode\_from\_json verifies WS codec round-trips for TARF (redemption enum, target bit-exact, schedule fixing\_years length, mc\_pairs/mc\_seed), Pivot (all TARF fields plus pivot level bit-exact), Accumulator (monitoring enum, barrier bit-exact, schedule length), continuous Lookback (Floating style, Continuous monitoring), and discrete Lookback (Fixed style, Discrete monitoring, strike/observations/mc knobs exact).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.wave3\_products\_decode\_from\_json` (hash `fd106a77ab7487c5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.window\_barrier\_and\_pricing\_model\_decode\_from\_json

- **claim** (`cl\_6a0d1b1773681582`): window\_barrier\_and\_pricing\_model\_decode\_from\_json verifies two contracts: (1) an explicit pricing\_model:1 in the JSON decodes to PricingModel::LocalStochVol, and WindowBarrier fields (barrier, side, window\_start, window\_end, mc\_pairs, mc\_steps, mc\_seed) are all preserved exactly; (2) an absent pricing\_model field decodes to PricingModel::Default (proto3 zero), preserving backward compatibility.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.window\_barrier\_and\_pricing\_model\_decode\_from\_json` (hash `91fa3305130c3d49`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.limits.transport\_config

- **claim** (`cl\_c333db16d3200911`): DELIVERABLE ws-edge-resource-caps = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2/3 P3/S finding "WS edge accepts connections with default tungstenite limits (64 MiB messages) — no explicit frame/message caps"; reconciled against the live graph — the streaming edge now installs explicit caps via celnet-server::ws::limits). \`transport\_config()\` is a pure, side-effect-free builder that returns the tungstenite WebSocketConfig the WS accept path uses: it sets max\_message\_size = Some(TRANSPORT\_MESSAGE\_CAP\_BYTES), max\_frame\_size = Some(TRANSPORT\_MESSAGE\_CAP\_BYTES) and max\_write\_buffer\_size = MAX\_WRITE\_BUFFER\_BYTES (overriding the 64 MiB tungstenite default), with no writes/allocation/I/O. Its sibling oversize\_close / oversize\_reject\_text emit the typed 1009 (CloseCode::Size) reject, and cap\_ordering\_holds pins the contract message fits under the cap — closing the bounded-resource-discipline deviation on the streaming edge. SELF-INVALIDATING: if the WS edge stopped supplying an explicit bounded config (anchor removed/renamed) or the builder grew a side effect, this claim would unresolve or flip the WRITES gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.limits.transport\_config` (hash `9c29f4852ab69d3d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-server.src.ws.mod.decode\_stream\_control

- **claim** (`cl\_aadbc4a07a73583d`): decode\_stream\_control maps a WebSocket stream-control JSON object to a ClientStreamMessage: it covers authenticate, subscribe, modify, unsubscribe, resync, execute, market\_series\_subscribe, market\_series\_unsubscribe, and heartbeat (the heartbeat path synthesises a zero-value Heartbeat proto rather than decoding fields). An unrecognised type tag returns CodecError with the literal text "unknown stream-control type \`{other}\`".
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.src.ws.mod.decode\_stream\_control` (hash `a327ed8897c20466`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-server.tests.cross\_asset\_ws.instrument\_json

- **claim** (`cl\_8fea37e771488ca3`): instrument\_json constructs a WS vanilla-option instrument JSON value from a GoldenVector, mapping 'CALL'/'PUT' string terms to celnet\_proto::OptionType integer discriminants, and conditionally injecting settlement\_style=InverseCoin when the vector's settlement\_style term equals 'INVERSE\_COIN'. Any unrecognised option\_type panics — there is no silent default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.tests.cross\_asset\_ws.instrument\_json` (hash `40c6e16e34358439`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-server.tests.cross\_asset\_ws.underlying\_json

- **claim** (`cl\_340734f78679d321`): underlying\_json maps a GoldenVector's family to the correct JSON discriminator shape used in cross-asset WS tests: 'equity\_option' → {equity:{symbol:{ticker,venue},currency}}, 'commodity\_option' → {commodity:{symbol:{ticker},currency:'USD'}}, 'crypto\_option' → {digital\_asset:{base,quote}} (splitting a 6-char pair token at position 3). Any other family panics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.tests.cross\_asset\_ws.underlying\_json` (hash `46e17bd578d99645`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.tests.multi\_dealer.law\_winner

- **claim** (`cl\_849f0e5c0114fa71`): law\_winner computes the best-bid or best-offer dealer from a panel by linear scan with deterministic tie-break: for bid\_side=true the winner maximises row\_price; for bid\_side=false it minimises row\_price; ties are broken by lexicographically smallest lp\_id. Panics if the panel is empty.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.tests.multi\_dealer.law\_winner` (hash `2eecd77eeb3c0b25`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-server.tests.risk\_federation.assert\_nonadditive\_eq

- **claim** (`cl\_017a89d984373efb`): assert\_nonadditive\_eq compares two NonAdditiveRisk protos with an absolute+relative tolerance of 1e-9 (tolerance = 1e-9 + 1e-9 \* b.abs()) for three fields: var, es, and curvature\_spot. Option presence must match exactly — a Some/None mismatch panics. The function is a pure assertion with no return value.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-server.tests.risk\_federation.assert\_nonadditive\_eq` (hash `bf30d4ff87f4534f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.ArbitrageReport.is\_arbitrage\_free

- **claim** (`cl\_0856e9121becb78d`): Analytics-correctness (arbitrage-gate deliverable): celnet-surface::arbitrage::ArbitrageReport::is\_arbitrage\_free is the pure hard-reject predicate combining all three no-arbitrage axes within tolerance tol — butterfly/density (min\_butterfly ≥ −tol), vertical/call-spread monotonicity (max\_vertical\_increase ≤ tol), and risk-neutral density positivity (min\_density ≥ −tol). A slice/surface is accepted only when every axis is within tol; any single breach makes the report not arbitrage-free. Pure: deterministic in (&self, tol), reads only the report's reduced extrema, no WRITES edges.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.ArbitrageReport.is\_arbitrage\_free` (hash `7657b1678d69761f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.check\_slice

- **claim** (`cl\_5b68870661865ef8`): \`check\_slice\` is the per-slice butterfly + vertical no-arbitrage primitive (ANALYTICS-SPEC §3.4) that VolSurface::arbitrage\_report calls at each sampled maturity. Over a strictly-ascending strike grid it returns an ArbitrageReport with \`min\_density\` (the minimum second-difference risk-neutral density (down−2·mid+up)/h² via implied\_density; a negative density is a butterfly violation), \`min\_butterfly\` = the h²-scaled butterfly spread, and \`max\_vertical\_increase\` = the largest call-price rise from a lower strike to the next (a positive increase is a vertical-spread violation, since calls must be monotone non-increasing in K). It asserts grid.len()≥3, h\>0, strictly-ascending strikes, and grid\[0\]\>h as preconditions. This is the slice-local half of the surface arbitrage gate; the calendar (cross-tenor) dimension is checked separately. Pure: it reads the smile + grid + scalars and returns the report value, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.check\_slice` (hash `e5fe133b2d20c894`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward\_call

- **claim** (`cl\_3e0c15d7ded029f0`): Analytics-correctness (arbitrage-gate deliverable, vertical/calendar oracle): celnet-surface::arbitrage::forward\_call is the pure undiscounted Black forward-call value F·N(d1) − K·N(d2) evaluated at the smile's own implied vol σ(K,F,t). It is the closed-form oracle the arbitrage gates are checked against: its K-derivative is −N(d2) ∈ \[−1,0\] (the vertical/call-spread bound, forward\_call\_strike\_slope), and its second K-difference is the butterfly/density check; calendar-monotonicity is verified by comparing this value across maturities. Pure: deterministic in (&Smile, strike, forward, t), no WRITES edges.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward\_call` (hash `8ce4e1efc7a0d9df`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward\_call\_strike\_slope

- **claim** (`cl\_1cf20cb81ac02cbb`): \`forward\_call\_strike\_slope\` returns the sticky (∂σ/∂K-ignoring) strike-derivative of the forward call price as \`-Φ(d2)\`, where \`d2 = d1 - σ√t\` from the smile's implied vol at the strike. Since \`Φ ∈ \[0,1\]\`, the returned slope lies in \`\[-1, 0\]\` — the no-arbitrage bound on a call's monotone-decreasing strike profile. Pure: it reads the smile and scalar inputs and returns an \`f64\`, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward\_call\_strike\_slope` (hash `e09fd91d88078aff`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.forward

- **claim** (`cl\_bd54b3efc15f5498`): CalibratedSmile::forward extracts the calibrated forward price from each model variant without computation: MarketHedge delegates to its inner forward(), StochasticVol reads params().forward, and all three parametric variants (Parametric, ParametricSurface, ExtendedSurface) read the field sl.forward directly. This guarantees that the forward embedded in every CalibratedSmile equals the MarketContext forward used during calibration.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.forward` (hash `d18cbd99cc2596dc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.implied\_vol

- **claim** (`cl\_2e63b80ecb32459c`): CalibratedSmile::implied\_vol is a uniform dispatch to the underlying model's implied\_vol(strike, forward, t) method across all five CalibratedSmile variants (MarketHedge, StochasticVol, Parametric, ParametricSurface, ExtendedSurface). The contract — including the Vol return type and the (strike, forward, t) signature — is identical for every variant; callers need not pattern-match on the model kind.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.implied\_vol` (hash `1a49cadfbaa9653f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.model

- **claim** (`cl\_ea45368c0460a33e`): CalibratedSmile::model is a pure read-only discriminant accessor that returns the SmileModel tag corresponding to the active variant without any allocation or computation. It enables callers to branch on the model kind for display or routing without pattern-matching on the full enum.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.CalibratedSmile.model` (hash `6a56b6b24fc30f6f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.build\_model\_smile

- **claim** (`cl\_2624897a00850e7b`): CAPABILITY (carry-seam deliverable, smile-family reach): build\_model\_smile is the single pure selector that reaches all FIVE smile families through one contract — MarketHedge (vanna-volga market-hedge), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each routed to its calibrator (build\_smile/fit\_sabr/fit\_svi/fit\_ssvi/fit\_essvi) and wrapped in the typed CalibratedSmile, with no calibration leaking outside the match. Pure: derives the CalibratedSmile from (model, ctx, quotes) refs with no WRITES; self-invalidates if a family arm acquires a side effect.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.build\_model\_smile` (hash `543646d679cd5a0a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_sabr

- **claim** (`cl\_b4f717243a975537`): CAPABILITY (smile family 1/5 — SABR): fit\_sabr is the SABR smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -\> Result\<StochasticVolSmile\>. Seeds (alpha, rho, nu) from the ATM level, 25-delta risk-reversal skew, and butterfly convexity, then a Gauss-Newton fit reproduces the total-variance anchors; beta is pinned (SABR\_BETA). No I/O, no input mutation, no WRITES — the gate self-invalidates if the calibration grows a side effect. One of the five selectable smile families (SABR, raw-SVI, SSVI, Vanna-Volga, parametric).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_sabr` (hash `a2831d296fe24e06`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_ssvi

- **claim** (`cl\_058e951140df4266`): CAPABILITY (smile family 3/5 — SSVI): fit\_ssvi is the SSVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -\> Result\<ParametricSlice\>. Pins theta to the ATM total variance and fits (rho, phi) to the wings by Gauss-Newton, projecting phi every iteration to satisfy the Gatheral-Jacquier Thm 4.2 butterfly sufficient conditions (theta\*phi\*(1+\|rho\|) \< 4 and theta\*phi^2\*(1+\|rho\|) \<= 4); eta = phi\*theta^gamma reconstructs the surface phi. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_ssvi` (hash `c70420f58dbef9f5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_svi

- **claim** (`cl\_3e57136453a333c3`): CAPABILITY (smile family 2/5 — raw SVI): fit\_svi is the raw-SVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -\> Result\<ParametricSlice\>. Fits (b, rho, m, sigma) by Gauss-Newton with the level a pinned so the ATM anchor (k=0) reproduces w\_atm exactly; the no-arbitrage projection runs each iteration, and a degenerate (negative minimum total variance) fit is rejected via the validating constructor rather than panicking. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit\_svi` (hash `df8c1cbbab928449`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.extended\_surface.ExtendedSlice.is\_calendar\_free\_with

- **claim** (`cl\_e83f8fc75dbf627c`): CALENDAR no-arbitrage gate in the SVI/extended parameterization (ANALYTICS-SPEC §3.4 calendar axis). \`ExtendedSlice::is\_calendar\_free\_with(next)\` decides whether an adjacent later-maturity SVI slice is calendar-arbitrage-free relative to self via the standard Gatheral–Jacquier no-crossing conditions on the raw-SVI params: (1) ATM total variance non-decreasing — \`next.theta \>= self.theta - SLACK\`; (2) wing curvature non-decreasing — \`next.psi \>= self.psi - SLACK\`; (3) the skew-change bound — \`\|next.rho·next.psi - self.rho·self.psi\| \<= (next.psi - self.psi) + SLACK\`, which bounds how fast the skew may rotate so total-variance smiles do not cross at any moneyness. All three must hold (\`SLACK = 1e-12\`) or the pair is flagged. Pure: reads &self and &next, returns bool, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.extended\_surface.ExtendedSlice.is\_calendar\_free\_with` (hash `51bd75dc77999ca7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.lib.build\_smile

- **claim** (`cl\_f832bd1e371a826c`): CAPABILITY (5 smile families): celnet-surface::build\_smile is the production smile-construction entry — a pure, side-effect-free calibration that returns Result\<MarketHedgeSmile, CalibrationError\> from a MarketContext + MarketQuotes with no I/O, allocation-driven mutation, or logging. It is the MarketHedge (vanna-volga) baseline of the five selectable SmileModel families exposed through the one contract — MarketHedge (vanna-volga), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each constructible into a VolSurface that reports its own model() and prices a finite positive implied vol across the strike grid (proven by each\_smile\_family\_is\_selectable). The carry forward feeding every family is the carry-seam bits (context\_forward\_is\_carry\_seam\_bits).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.lib.build\_smile` (hash `177930c5c15ac8b5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.market\_hedge.MarketHedgeSmile.corrections

- **claim** (`cl\_3620f9b28195fd56`): CAPABILITY (smile family 4/5 — Vanna-Volga): MarketHedgeSmile.corrections is the Vanna-Volga smile-family analytic entry — pure (&self, strike) -\> (D1, D2). Returns the first-order (D1 = p\*(sigma1-sigma0) + q\*(sigma3-sigma0)) and second-order (D2, the d1\*d2-weighted squared vol gaps at the 25-delta wings) market-hedge corrections from the three benchmark vols, the construction underlying the broker-quoted three-point smile. Reads only &self, returns a tuple, no WRITES — the gate self-invalidates if the correction formula grows a side effect. One of the five selectable smile families.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.market\_hedge.MarketHedgeSmile.corrections` (hash `bfaad318e6be943e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.src.parametric.ParametricSlice.butterfly\_density\_factor

- **claim** (`cl\_d0cfcdcb1348add5`): ParametricSlice::butterfly\_density\_factor(k) computes the Breeden-Litzenberger density factor for an SVI total-variance slice at log-moneyness k. The exact formula is: \`(1 − k·w'/2w)² − (w'²/4)·(1/w + 1/4) + w''/2\` where w = total\_variance(k), w' = d\_total\_variance(k), w'' = d2\_total\_variance(k). A strictly positive value at all k implies the slice has no butterfly arbitrage (no negative implied density). The SVI first derivative is \`w'(k) = b·(ρ + (k−m)/√((k−m)²+σ²))\` and second derivative follows analytically. Pure: reads only self fields (a, b, rho, m, sigma); no I/O.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.parametric.ParametricSlice.butterfly\_density\_factor` (hash `8b8c22d125672d73`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-surface.src.parametric.ParametricSlice.d\_total\_variance` (hash `2e729e8209d08642`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.quotes.DeltaPillar.signed

- **claim** (`cl\_abe99660f4b4904a`): DeltaPillar::signed converts a magnitude delta to a signed delta by returning +magnitude for calls and −magnitude for puts, encoding the market convention that put deltas are negative and call deltas are positive. Used when the same pillar magnitude (e.g. 0.25) must be applied to a specific option type's delta equation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.DeltaPillar.signed` (hash `2b3de116edaeabfd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.atm\_convention

- **claim** (`cl\_d5a9e496b03d1d65`): Market-context atm-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::atm\_convention is a pure projection returning self.conventions.atm unchanged — the AtmConvention threaded into ATM-strike resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.atm\_convention` (hash `74d2f84a8f2244a3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.delta\_convention

- **claim** (`cl\_1dd92833acc1c4a3`): Market-context delta-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::delta\_convention is a pure projection returning self.conventions.delta unchanged — the DeltaConvention threaded into strike↔delta quote resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.delta\_convention` (hash `a6892d71fe6fb23d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.strike\_at\_delta

- **claim** (`cl\_58199fb0fa1a8ed2`): Analytics-correctness (guarded-root-find deliverable): celnet-surface::quotes::MarketContext::strike\_at\_delta resolves a delta pillar to a strike through the guarded inversion strike\_from\_delta under the context's own delta\_convention, signing the target delta with pillar.signed(opt) (call \> 0, put \< 0). It returns Result\<f64, DeltaSolveError\> — the guarded root-find surfaces its failure mode (e.g. an unreachable premium-adjusted target beyond the delta cap) as a typed error rather than a silent/wrong root. Pure: deterministic in (&self, opt, pillar, vol) given the context, no WRITES edges.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.strike\_at\_delta` (hash `2cd8ac762cccde08`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.black\_vol

- **claim** (`cl\_53b7877108c31de8`): StochasticVolParams::black\_vol(strike) computes the standard SABR lognormal-expansion implied vol (Hagan et al. 2002/2014). The formula is \`(α/denom) · (z/x(z)) · B\_factor\` where: \`denom = (F·K)^((1−β)/2) · \[1 + (1−β)²/24·(ln F/K)² + (1−β)⁴/1920·(ln F/K)⁴\]\`; \`z = (ν/α)·denom\_base·ln(F/K)\`; \`x(z) = ln((√(1−2ρz+z²)+z−ρ)/(1−ρ))\`; \`B\_factor = 1 + \[(1−β)²/24·(α/fk\_pow)² + ¼ρβν·α/fk\_pow + (2−3ρ²)/24·ν²\]·t\`. ATM (\|F−K\| ≤ 1e-12·F) returns \`(α/fk\_pow)·B\_factor\` (z/x(z) → 1). The z → 0 removable singularity is guarded by the series \`1/(1 − ½ρz + (2−3ρ²)/12·z²)\`. Pure: reads only its \`StochasticVolParams\` fields (α, β, ρ, ν, forward, t); no I/O or mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.black\_vol` (hash `f3338b3eea32c4bf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.risk\_neutral\_density

- **claim** (`cl\_5a94f94ff641ca71`): StochasticVolParams::risk\_neutral\_density(strike) returns the SABR risk-neutral density via a change-of-variable from strike space to the SABR coordinate u. The mapping is: \`y = y\_coordinate(strike)\`, \`dy/dK = 1/(α·K^β)\`; for ν \> 1e-10: \`z = ν·y\`, \`u = (1/ν)·ln((√(1−2ρz+z²)+z−ρ)/(1−ρ))\`, \`du/dy = 1/√(1−2ρz+z²)\`; for ν ≤ 1e-10 (Black limit): \`u = y\`, \`du/dy = 1\`. The density is then \`φ(u/√t)/√t · \|du/dy\| · \|dy/dK\|\` where φ is the standard normal PDF. Returns 0 for strike ≤ 0. Pure: reads only its struct fields; no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.risk\_neutral\_density` (hash `d026d852601b7e75`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.implied\_black\_vol

- **claim** (`cl\_3439aa9d7fcfa0ed`): implied\_black\_vol inverts a forward Black call price to implied volatility via bisection over \[1e-6, 5.0\]. It returns None for any price outside the no-arbitrage interior (call ≤ intrinsic + 1e-15 or call ≥ forward), and also returns None if even the upper bound vol=5.0 fails to produce a price ≥ call. Convergence criterion is \|price(mid) − call\| \< 1e-12 or bracket width \< 1e-12; the returned vol is always finite when Some.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.implied\_black\_vol` (hash `85070e11459a9dc7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strangle.build\_trial\_smile

- **claim** (`cl\_2030654e489b000c`): STRANGLE-CALIBRATION guarded trial-smile constructor (ANALYTICS-SPEC §1.4 — the mandatory smile-strangle calibration that reprices the broker/market strangle). \`build\_trial\_smile(ctx, atm\_vol, atm\_strike, pillar, rr, smile\_strangle)\` builds one candidate vanna-volga smile for a trial σ\_ss inside the calibration bracketing loop: it resolves the three wings via \`smile\_wings\`, then HARD-REJECTS out-of-domain trials as \`Err(CalibrationError::DegenerateQuote)\` rather than panicking or returning an invalid Ok — (1) a \|RR\| too large relative to ATM+σ\_ss drives a wing vol \<= 0, caught by \`!(put\_vol\>0 && call\_vol\>0)\`; (2) the fallible \`MarketHedgeSmile::try\_new\` enforces the strike ordering K1\<K2\<K3 (put wing \< ATM \< call wing) and returns the same error if a pathological trial inverts it. This keeps the guarded root-find total: a degenerate trial steps the bracket away, and a terminal degenerate state surfaces as a typed error, never a panic. Pure: reads ctx/scalars/pillar, returns Result\<MarketHedgeSmile, CalibrationError\>, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.build\_trial\_smile` (hash `bcd4caaca7e82245`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strangle.market\_strangle

- **claim** (`cl\_1443f4d0921f3a71`): The broker (market) butterfly is NOT the smile butterfly — the \#1 production FX-vol bug (ANALYTICS-SPEC §1.4). \`market\_strangle\` defines what brokers actually trade: a SINGLE vol \`strangle\_vol = atm\_vol + quote.butterfly\` applied to BOTH the call AND put pillar strikes (call\_strike and put\_strike each resolved via strike\_at\_delta at that one vol), and its price is call+put at that single vol. This is the calibration TARGET that the recovered smile must reprice — it is deliberately distinct from the arithmetic smile-strangle (the per-wing-vol convexity), and the two differ materially for high-RR/EM pairs. Treating the quoted BF as the arithmetic 25Δ smile-strangle silently biases the wings and breaks 10Δ reproduction. Pure: reads ctx/atm\_vol/quote, returns Result\<MarketStrangle, CalibrationError\>, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.market\_strangle` (hash `7d79883590cedd34`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strangle.smile\_wings

- **claim** (`cl\_26ec9f0d00be9f87`): The smile wings carry DISTINCT per-wing vols — the counterpart to the single-vol broker strangle (ANALYTICS-SPEC §1.4, Reiswich-Wystup/Clark). \`smile\_wings\` builds the recovered 25Δ/10Δ wings from the trial smile-strangle σ\_ss as \`call\_vol = σ\_ATM + σ\_ss + ½RR\` and \`put\_vol = σ\_ATM + σ\_ss − ½RR\` (the risk-reversal RR splits the two wings antisymmetrically), then resolves each wing's strike at ITS OWN vol via strike\_at\_delta. Because call and put wings get different vols (unlike market\_strangle's single vol on both strikes), the explicit broker→smile calibration step is mandatory: a non-zero RR makes the smile-strangle differ from the broker butterfly. Pure: reads ctx/atm\_vol/pillar/rr/smile\_strangle, returns a Result tuple, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.smile\_wings` (hash `2c724bba5bdabb1f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.anchors

- **claim** (`cl\_d27cb730d7a45f7f`): \`anchors(forward, t, quotes)\` is a pure projection: it maps each \`StrikeQuote\` in the slice to its log-moneyness \`k = ln(strike/forward)\` and to its total variance \`w = vol²·t\`, returning \`(Vec\<f64\>, Vec\<f64\>)\`. No allocation beyond the two output vecs; no I/O or mutation. The log-moneyness / total-variance coordinate change is the exact same transform the FX delta-axis calibrator performs after its pillar calibration (\`calibrate.rs::anchors\`), so the two front-ends share a common inner coordinate system.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.anchors` (hash `b496a822427f3257`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.fit\_strike\_slice

- **claim** (`cl\_4e1532ff049e230a`): DELIVERABLE surface/crypto-leaf (the LEAF half) = DONE (RC cut c5a5efc). \`fit\_strike\_slice(ctx, quotes)\` is the pure strike-axis (log-moneyness, NOT FX delta-space RR/BF) smile calibrator that lets a crypto/equity surface be marked from a raw strike-vol grid: it seeds an SVI-style 3-parameter slice deterministically (vertex at the lowest observed total variance, width from the k-span), runs a fixed-iteration projected Gauss-Newton inner solve, then ray-projects onto the butterfly-admissible set (a no-op for any arbitrage-free-reproducible quote set), returning \`Result\<StrikeSliceFit, CalibrationError\>\` with the slice plus rms/max reproduction error in absolute vols. Pure: it borrows \`(&StrikeSliceContext, &StrikeQuoteSlice)\`, mutates only local state, performs no I/O, and is fully deterministic (no RNG). This is the surface LEAF the backlog split out of W3-crypto; the remaining OPEN half is the WIRE surfacing (no strike-axis quote\_basis on MarkSurfaceRequest yet — see mark\_surface\_request\_from\_json), tracked as surface/crypto-strike-axis-surfacing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.fit\_strike\_slice` (hash `0d79fdc9e9400a68`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.inner\_solve

- **claim** (`cl\_ff784f3459dab895`): \`inner\_solve(ks, ws, m, sigma)\` is the pure closed-form WLS linear solver for the inner SVI problem with fixed \`(m, σ)\`: it accumulates the 6 sufficient statistics \`(Σ1, Σy, Σz, Σyy, Σyz, Σzz, Σw, Σyw, Σzw)\` where \`y=(k−m)/σ\` and \`z=√(y²+1)\`, assembles the 3×3 normal-equation matrix \`\[\[n,Σy,Σz\],\[Σy,Σyy,Σyz\],\[Σz,Σyz,Σzz\]\]\`, and calls \`solve3\` to produce \`(a, c, d)\`. Returns \`None\` on a singular normal matrix or any non-finite solution element; no side effects or allocation beyond the statistic accumulators. This is the innermost linear-algebra leg of the two-step SVI fit: the outer Gauss–Newton loop iterates over \`(m, σ)\` and calls \`inner\_solve\` at each step via the \`residuals\` closure in \`fit\_strike\_slice\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.inner\_solve` (hash `b55dd81043ac715e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.passes\_admission\_scan

- **claim** (`cl\_c88e7ac1c9eccdca`): \`passes\_admission\_scan(slice)\` evaluates the Durrleman butterfly-density condition on a uniform grid of \`ADMISSION\_SCAN\_SAMPLES\` log-strike points spanning \`\[m − ADMISSION\_SCAN\_SPAN, m + ADMISSION\_SCAN\_SPAN\]\` centred on the SVI vertex \`m\`, returning \`false\` immediately if any sample's \`butterfly\_density\_factor(k) \< −ADMISSION\_SCAN\_TOL\`. A slice passes iff the minimum sampled density is ≥ −ADMISSION\_SCAN\_TOL; the flat-market baseline (s=0 in \`admit\_or\_shrink\`) is admissible by construction, so the scan is always decidable without pathology. Pure: reads only the \`ParametricSlice\` argument, no mutation, no allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.passes\_admission\_scan` (hash `74ee86a64494c4a8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.svi\_w\_reference

- **claim** (`cl\_eb4f78ac690dfcb6`): \`svi\_w\_reference(k, a, b, rho, m, sigma)\` is the closed-form SVI raw total-variance oracle (Gatheral–Jacquier 2014 eq. 3.1): \`w(k) = a + b·(ρ·(k−m) + √((k−m)²+σ²))\`. It is a pure function of its six scalar arguments with no side-effects, no allocation, and no WRITES edges. It is intentionally independent of \`ParametricSlice::total\_variance\` (oracle-independence discipline) so the test suite can cross-check the fitted surface against the reference formula without circularity. Two copies exist — one in \`src/strike\_quotes.rs\` (used by BTC-fixture unit tests) and a bit-identical copy in \`tests/strike\_axis\_oracle.rs\` (used by the ETH-fixture integration oracle) — both implement the same identity.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.svi\_w\_reference` (hash `2b3ea9d64dcdc2b9`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-surface.tests.strike\_axis\_oracle.svi\_w\_reference` (hash `aa3a1d1db2641c81`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.well\_posed\_slice

- **claim** (`cl\_d6dc3b3848d07c7a`): \`well\_posed\_slice(a, c, d, m, sigma, forward, t)\` converts inner parameters \`(a, c, d)\` to the \`(a, b, ρ)\` SVI parameterisation via \`b = d/σ\` and \`ρ = clamp(c/d, −0.999999, 0.999999)\` (clamped away from ±1 to keep the ATM variance strictly positive), computes \`w\_min = a + b·σ·√(1−ρ²)\` (the SVI global minimum), and returns \`None\` when any parameter is non-finite, \`σ ≤ 0\`, or \`w\_min \< −1e-12\`; otherwise constructs \`ParametricSlice::new(a, b, ρ, m, σ, forward, t)\`. Pure: a guarded constructor with no side effects. The \`FLAT\_D\_FLOOR\` threshold gates whether \`ρ\` is computed from \`c/d\` or clamped to \`0.0\` when \`d\` is near-zero (flat-skew market).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.strike\_quotes.well\_posed\_slice` (hash `7e4d364d11e68a0f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-surface.src.surface.VolSurface\<S, C\>.arbitrage\_report

- **claim** (`cl\_3d3edbb666550ac2`): The three FX no-arbitrage gates are computed together over the whole surface and are hard-reject inputs (ANALYTICS-SPEC §3.4). \`VolSurface::arbitrage\_report\` produces a SurfaceArbitrageReport with all three diagnostics: (1) BUTTERFLY — \`min\_density\`, the minimum over sampled maturities/strikes of the second-difference risk-neutral density (a negative value ⇒ butterfly arbitrage); (2) VERTICAL — \`max\_vertical\_increase\`, the worst call-price increase across ascending strikes (a positive value ⇒ vertical-spread arbitrage, calls must be non-increasing in K); (3) CALENDAR — \`min\_calendar\_increment\`, the minimum cross-slice total-variance increment at the wing+ATM log-moneyness (a negative value ⇒ calendar arbitrage, total variance must be non-decreasing in T). Per-maturity work delegates to check\_slice; cross-slice calendar to term.min\_calendar\_increment. A surface is arbitrage-free only when min\_density≥0, max\_vertical\_increase≤0, and min\_calendar\_increment≥0 — these are gating thresholds, not advisory. Pure: reads &self + sampling params, returns the report value, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.surface.VolSurface\<S, C\>.arbitrage\_report` (hash `b0291274dde24012`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.CalendarClock.business\_time

- **claim** (`cl\_2c9c98e9ba85993f`): DELIVERABLE surface/event-weighted-clock = OPEN (Round-2 P2/M finding, tracked). \`CalendarClock::business\_time(&self, t)\` is the identity business-time map \`tau(t) = t\` — it returns its argument verbatim, with no weekend/holiday compression and no scheduled-event (central-bank-meeting / fixing) weighting. It is the ONLY BusinessClock implementation in the workspace (the trait seam exists but has a single identity impl), so term-structure interpolation is currently a no-op on the event-weighted clock that ANALYTICS-SPEC §3.6 specifies as market standard. Pure: it reads only \`(&self, t)\` and returns \`t\`, mutating nothing. Closing the deliverable means adding a real event/calendar-weighted BusinessClock impl; the new impl (and any change to this identity body) will change this method's node content and STALE this claim — the staleness firing is the done-signal. This is intentionally a no-arbitrage-safe placeholder, NOT a faked depth claim.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.CalendarClock.business\_time` (hash `e9dd09ef4b480512`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.forward\_at

- **claim** (`cl\_51d134d62f8b8ae3`): TermStructure::forward\_at interpolates and extrapolates the forward price in log-linear (continuous compounding) space over calendar time t. Before the first pillar it returns the first pillar's forward; after the last it returns the last pillar's forward. In the interior it computes frac = (t − tl)/(th − tl) and returns exp(ln(fl) + frac\*(ln(fh) − ln(fl))), which equals fl^(1−frac) \* fh^frac — a geometric (log-linear) interpolation that preserves no-arbitrage ordering of forwards.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.forward\_at` (hash `7fddb2147a91a19d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.implied\_vol

- **claim** (`cl\_b7d2cf5c11157aff`): TermStructure::implied\_vol(strike, t) converts from strike / calendar-time to Black implied vol by routing through total variance: \`f = forward\_at(t)\`, \`k = ln(strike/f)\`, \`w = total\_variance(k, t)\`, returning \`Vol(sqrt((w/t).max(0.0)))\`. The \`.max(0.0)\` clamp makes the output well-defined even under marginal floating-point calendar-arbitrage; the caller is responsible for ensuring the surface is calendar-free before relying on exact values. Pure: no writes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.implied\_vol` (hash `03a54c3678c10672`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.is\_calendar\_free

- **claim** (`cl\_5e8c34e5736c6192`): CALENDAR no-arbitrage gate (ANALYTICS-SPEC §3.4 — the cross-tenor third arbitrage axis, distinct from the per-slice butterfly+vertical gate in check\_slice, which the check\_slice claim explicitly delegates here). \`TermStructure::is\_calendar\_free(k, tol)\` is the hard-reject calendar gate: it returns true iff \`min\_calendar\_increment(k, 256) \>= -tol\`, i.e. total variance w(k,t)=σ²·t is non-decreasing in maturity along a fixed strike. A strict total-variance crossing (a longer-dated pillar carrying LESS total variance than a shorter one) is a calendar-spread arbitrage and is rejected — e.g. 6M@15vol (w=0.01125) vs 1Y@10vol (w=0.01) flags. This is the term-axis member of the three FX no-arbitrage gates (butterfly/calendar/vertical) that are hard-reject inputs. Pure: reads &self pillars and returns a bool, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.is\_calendar\_free` (hash `4279c93c0b418df6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.min\_calendar\_increment

- **claim** (`cl\_9fb2ee5cca8036a0`): CALENDAR-arbitrage MEASURE primitive (ANALYTICS-SPEC §3.4) behind the calendar no-arbitrage gate. \`TermStructure::min\_calendar\_increment(k, samples)\` pins one strike from the near-pillar log-moneyness (\`strike = pillars\[0\].forward · e^k\`), then walks that FIXED strike across \`samples\` maturities t0..t1 — crucially re-converting to EACH maturity's own log-moneyness \`k\_t = ln(strike / forward\_at(t))\` before reading \`total\_variance(k\_t, t)\` — and returns the minimum forward increment of total variance w between consecutive maturity samples. A negative minimum increment is exactly a total-variance crossing = calendar-spread arbitrage; is\_calendar\_free rejects when this is below -tol. Fixing the cash strike (not the moneyness) across tenors is the correct no-arb test under term-varying forwards. Pure: reads &self, returns f64, mutating nothing (it does assert samples\>=2 as a precondition, but performs no writes).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure\<S, C\>.min\_calendar\_increment` (hash `0d88a515a8746642`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-surface.tests.fit\_pins.cost\_of

- **claim** (`cl\_5c4208368e80add0`): cost\_of computes the sum-of-squared calibration residuals for a CalibratedSmile at a set of (log-moneyness, total-variance) anchor points. For StochasticVol it uses absolute vol residuals r = black\_vol(strike) − vol; for Parametric/ParametricSurface/ExtendedSurface it uses total-variance residuals r = slice.total\_variance(k) − w. MarketHedge is not a fitted model and triggers unreachable!(). Used by the fit-pin tests to confirm convergence quality.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.tests.fit\_pins.cost\_of` (hash `7d7fa73d0662dcc8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-surface.tests.fx\_fit\_pin.computed\_pins

- **claim** (`cl\_a4b85d5b8357a172`): computed\_pins materialises the 2×5×5 regression table for the FX surface calibration path. For each of 2 quote sets and each of the supported smile models (MODELS), it calls build\_model\_smile against a fixed MarketContext (r\_dom=0.02, r\_for=0.01, t=1.0) then evaluates smile.implied\_vol at five relative strikes STRIKE\_RATIOS × forward, storing each result as raw f64 bits via \`.to\_bits()\`. The output is therefore a bitwise-exact snapshot of the calibration pipeline: any algorithmic change that alters a single implied-vol by even an ULP will change the returned bits, making this the source of truth for the fx\_calibration\_bits\_are\_frozen regression gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.tests.fx\_fit\_pin.computed\_pins` (hash `19ba94d0359ad238`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-surface.tests.quote\_grid\_fuzz.convention\_from\_picks

- **claim** (`cl\_88c2fcffa2f11d84`): convention\_from\_picks constructs a ConventionRecord by mapping two single-byte inputs to a fully-specified FX market convention: atm\_pick % 2 selects between DeltaNeutralStraddle (0) and AtmForward (1); delta\_pick % 4 selects among the four delta conventions (SpotUnadjusted / SpotPremiumAdjusted / ForwardUnadjusted / ForwardPremiumAdjusted). All remaining fields are fixed constants: PremiumStyle::DomesticPips, Cut::NewYork1000, DayCount::Act365Fixed (option year-fraction), DayCount::Act360 (domestic and foreign rate day-count), Settlement::Deliverable. This gives a deterministic, exhaustive 8-combination coverage of ATM-convention × delta-convention space for fuzz-driven quote-grid tests.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-surface.tests.quote\_grid\_fuzz.convention\_from\_picks` (hash `f4573fd004b4a845`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_decreasing\_in\_strike

- **claim** (`cl\_f6361b704257567f`): assert\_decreasing\_in\_strike enforces strike monotonicity: call value is non-increasing in strike and put value is non-decreasing in strike. Bump is 1e-3 \* i.strike; lo and hi are computed via price at strike ∓ bump. The assertions are hi \<= lo + ABS (call) and hi + ABS \>= lo (put) — the sign conventions are the exact mirror of assert\_increasing\_in\_spot, reflecting put-call symmetry about the strike axis.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_decreasing\_in\_strike` (hash `0532c2f4cc17e601`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_greek\_matches\_fd

- **claim** (`cl\_23f88709b92f0791`): assert\_greek\_matches\_fd validates each analytic Greek against a central finite-difference approximation using bump sizes tuned per sensitivity: hs = 1e-4 \* spot for spot-perturbations, hv = 1e-5 for vol, ht = 1e-5 \* max(t, 1) for time, hr = 1e-6 for rates. Theta is defined as −∂V/∂T (note the sign flip applied to the FD result). Second-order Greeks (Gamma, Vanna, Volga, Charm) are computed by differencing the first-order analytic sensitivities (delta\_spot and vega) rather than price, giving second-order FD accuracy. Agreement is tested via is\_close(analytic, numeric, rel, abs) with caller-supplied tolerances.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_greek\_matches\_fd` (hash `d507904b30bc863e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_increasing\_in\_maturity

- **claim** (`cl\_c32ceabaf0296835`): assert\_increasing\_in\_maturity enforces the conditional time-value monotonicity law, gated on the carry regime: before bumping time it pre-checks that the call condition r\_dom + ABS \>= r\_for (non-negative carry) or put condition r\_for + ABS \>= r\_dom (non-positive carry) holds, then bumps maturity by 1e-4 \* i.t and asserts hi + ABS \>= lo for both option types. This makes the precondition violation explicit rather than silently producing a false pass or false fail on out-of-regime inputs.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_increasing\_in\_maturity` (hash `3bf395b7c15b648b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_increasing\_in\_spot

- **claim** (`cl\_5cb2a5440dec055d`): assert\_increasing\_in\_spot enforces the model-free monotonicity law: call value is non-decreasing in spot and put value is non-increasing in spot. It bumps spot by 1e-3 \* i.spot in each direction, computes lo = price(opt, spot-bump) and hi = price(opt, spot+bump), then asserts hi + ABS \>= lo for calls and hi \<= lo + ABS for puts, where ABS is the shared absolute tolerance. The function is a pure read — no mutable state — and panics on any violation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-testkit.src.invariants.assert\_increasing\_in\_spot` (hash `21583f9e8b4e0a43`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.carry\_rate

- **claim** (`cl\_5fb26d3514bfdbf5`): Carry::carry\_rate() is a pure accessor over the asset-class-agnostic carry seam (FxRates{r\_dom,r\_for} \| CostOfCarry{r,b}). Every pricing engine consumes the Carry seam rather than raw FX rate fields, and there is no hot-path match on Carry/Underlying. This keeps one asset-class-agnostic pricing contract across vanilla/exotics/surface/risk and the crypto/equity/commodity leaves (ADR-0008).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.carry\_rate` (hash `3fee7410abdfb52d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.rate\_sensitivities

- **claim** (`cl\_1e0e0fc8ccda6be1`): Arm-type selector: \`Carry::FxRates\`→\`RateSensitivities::Fx{rho\_dom:a,rho\_for:b}\`; \`Carry::CostOfCarry\`→\`RateSensitivities::Carry{discount\_rho:a,carry\_rho:b}\`. A third Carry arm = exactly one new match arm here. Pure.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.rate\_sensitivities` (hash `39330585673b9778`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.yield\_rate

- **claim** (`cl\_7b19599bc2068061`): Carry::yield\_rate() returns the stored foreign/yield rate (r\_for) VERBATIM — it is a pure accessor with no side effects. FX bit-identity depends on this: foreign-rate reads must use yield\_rate() directly and must NEVER be reconstructed as discount\_rate() - carry\_rate(), which would perturb the FX hot path. Enforced by the ADR-0008 carry-seam architecture (docs/adr/ADR-0008-multi-asset-carry-architecture.md); the carry-seam lowering guard rejects raw cost-of-carry for FX (see celnet-core/src/carry.rs fx\_lowering\_rejects\_cost\_of\_carry).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.yield\_rate` (hash `93411113b7ad7b67`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.FixingSource.code

- **claim** (`cl\_030bff2e129deaca`): FixingSource::code(self) -\> &'static str is a const pure total function mapping each of the 6 EM exotic-fixing sources to its canonical ISDA/market-convention code string: KrwKftc18→"KRW.KFTC18", TwdTaipei→"TWD.TAIPEI", InrRbiRef→"INR.RBIB", BrlPtax→"BRL.PTAX", ClpDolarObs→"CLP.DOLAROBS", CopTrm→"COP.TRM". The fixing\_source\_codes\_are\_distinct\_and\_nonempty test (loop over FixingSource::all()) asserts every code is non-empty and every pair is distinct, so the code strings are a compile-verified bijection.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.FixingSource.code` (hash `a6b87f613e0a994a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks

- **claim** (`cl\_885a55ce82669f85`): Greeks is a 14-field f64 struct encoding the full first/second/third-order option sensitivities output by every analytics leaf in the platform: price (PV), delta\_spot, delta\_forward, gamma, vega (per 1.0 absolute vol), theta (per year, −∂V/∂T), rho\_dom, rho\_for, vanna (∂²V/∂S∂σ), volga/vomma (∂²V/∂σ²), charm (∂delta\_spot/∂T), speed (∂³V/∂S³), zomma (∂gamma/∂σ), color (∂gamma/∂T). Greeks::price\_only(price) is the canonical constructor for price-only paths. This struct is the uniform output contract across all asset-class pricers.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks` (hash `0a0cc5c3ccbdea86`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks.price\_only` (hash `2695acfdc3e4c698`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Metal.ccy

- **claim** (`cl\_7f6b0499f9843446`): Metal::ccy is a const bijective mapping from each Metal variant to its ISO 4217 / LBMA currency code: Gold→XAU, Silver→XAG, Platinum→XPT, Palladium→XPD. The mapping is exhaustive and total — every Metal variant maps to exactly one Ccy, and the inverse (Metal::from\_ccy) reconstructs the original Metal from that Ccy. This bijection is the foundation of the precious-metal numeraire identity throughout the pricing stack.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Metal.ccy` (hash `c9df096a9e1369df`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Metal.from\_ccy

- **claim** (`cl\_f07d8017af8a62dd`): Metal::from\_ccy is the const inverse of Metal::ccy: given a Ccy it returns Some(Metal) for XAU→Gold, XAG→Silver, XPT→Platinum, XPD→Palladium, and None for any other currency. Because Ccy's inner \[u8;3\] is not const-comparable with ==, the match operates on raw bytes (e.g. \`\[b'X', b'A', b'U'\]\`). This makes from\_ccy usable in const contexts while remaining correct for the full ISO code set.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Metal.from\_ccy` (hash `0f731cc36c2739d2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.MetalPair.as\_ccy\_pair

- **claim** (`cl\_230f04879ec8c15b`): Underlying::as\_ccy\_pair(&self) -\> Option\<CcyPair\> is a const pure total accessor: Fx(p) returns Some(p); Metal(m) returns Some(m.as\_ccy\_pair()) — delegating to MetalPair::as\_ccy\_pair which constructs CcyPair::new(metal.ccy(), quote) (the ISO-X-prefixed metal Ccy, e.g. XAU, as base). Equity/Commodity/DigitalAsset return None. Called by 5 callers in the analytics and risk-cube layers that need to resolve an Underlying to a tradeable FX/metal rate pair.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.MetalPair.as\_ccy\_pair` (hash `ae4dd8765b409b32`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_ccy\_pair` (hash `45ef423a2fd5e4c3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.MetalPair.from\_ccy\_pair

- **claim** (`cl\_235bf5f45586f23e`): MetalPair::from\_ccy\_pair lifts Metal::from\_ccy to a CcyPair: it returns Some(MetalPair::new(metal, pair.quote)) iff the base currency of the pair is a recognized precious metal (XAU/XAG/XPT/XPD), and None otherwise. The quote currency is preserved verbatim. This const function is the canonical way to classify a raw CcyPair as a metal pair without heap allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.MetalPair.from\_ccy\_pair` (hash `56643888ddd9a0da`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.flip

- **claim** (`cl\_c817211f56997f3c`): OptionType::flip(self) -\> Self is a const pure involution: Call maps to Put and Put maps to Call. It is the put-call symmetry operator used in premium-adjustment transformations and put-from-call/parity conversions. Zero callers in production code but referenced in the premium\_adjusted\_flag test; its total absence of side effects and exhaustive match make it a compile-time-checkable involution.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.flip` (hash `05f6111cb331e5fb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.sign

- **claim** (`cl\_a5af85874785ca69`): OptionType::sign(self) -\> f64 is a const pure function returning +1.0 for Call and -1.0 for Put. It is used as the φ multiplier throughout the closed-form pricing formulae (e.g. φ·(F·N(φ·d1) − K·N(φ·d2))). Called by 11 callers across the analytics crates — it is the canonical signed-direction encoding for the option type.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.sign` (hash `62772d44d69165f5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.flip\_orientation

- **claim** (`cl\_a532c13ff6e7163d`): \`PremiumStyle::flip\_orientation\` is a self-inverse (involution) mapping the premium style under base/quote currency-pair inversion: DomesticPips↔ForeignPips and PercentForeign↔PercentDomestic. Applying it twice is the identity (DomesticPips→ForeignPips→DomesticPips; PercentForeign→PercentDomestic→PercentForeign), so re-quoting the same option in the inverted pair orientation and back recovers the original premium style exactly — the orientation-invariance property the pair-universe view relies on (docs/CONVENTIONS.md §pair universe). It is a \`const fn\` total match with no wildcard. Pure: reads only \`self\`, returns the flipped enum, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.flip\_orientation` (hash `7e2915c407c13068`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.is\_premium\_adjusted

- **claim** (`cl\_f78b5960e4be7f0a`): \`PremiumStyle::is\_premium\_adjusted\` is the canonical predicate deciding whether the premium carries FX risk: it returns true exactly for the FOR/base-ccy-denominated styles \`PercentForeign \| ForeignPips\` and false for the DOM-ccy styles \`DomesticPips \| PercentDomestic\`. A FOR-ccy premium carries FX risk, which is why these styles drive premium-adjusted delta (the non-monotone premium-adjusted call delta of docs/CONVENTIONS.md). The classification is a \`const fn matches!\` over \`self\` with no wildcard, so a new PremiumStyle variant forces this distinction to be revisited rather than defaulting silently. Pure: reads only \`self\`, returns a bool, no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.is\_premium\_adjusted` (hash `48cb616799fda4f2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.RateSensitivities.flat\_rhos

- **claim** (`cl\_b93f172c8dca59b6`): Single-source bijection from the two \`RateSensitivities\` variants to the flat \`(rho\_dom, rho\_for)\` pair at the server flatten boundary. \`Fx{rho\_dom,rho\_for}\`→\`(rho\_dom,rho\_for)\` unchanged; \`Carry{discount\_rho,carry\_rho}\`→\`(discount\_rho+carry\_rho, -carry\_rho)\`, recovering \`r\_dom=discount+carry\`, \`r\_for=-carry\`. Pure (reads only self).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.RateSensitivities.flat\_rhos` (hash `40b4194f2d5d7127`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_commodity

- **claim** (`cl\_7f089e9e51ef4e9d`): Underlying::as\_commodity is a non-const variant-narrowing accessor that returns Some(&CommodityRef) iff the underlying is the Commodity arm, and None otherwise. Returns a reference because CommodityRef is not Copy. Called by 3 sites as the commodity pricing dispatch gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_commodity` (hash `c423f6ec638f3b08`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_digital\_asset

- **claim** (`cl\_98a56232bfbff50a`): Underlying::as\_digital\_asset is a non-const variant-narrowing accessor that returns Some(&CryptoPair) iff the underlying is the DigitalAsset arm, and None otherwise. Returns a reference because CryptoPair is not Copy. Called by 3 sites as the crypto/digital-asset pricing dispatch gate.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_digital\_asset` (hash `5ec31683f3d9f712`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_equity

- **claim** (`cl\_52445ea114f6ee33`): Underlying::as\_equity is a non-const variant-narrowing accessor that returns Some(&EquityRef) iff the underlying is the Equity arm, and None otherwise. Returns a reference (not a copy) because EquityRef is not Copy. This is the primary dispatch gate for the equity pricing path; called by 3 sites.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_equity` (hash `c3ee9895c218b4d8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_fx

- **claim** (`cl\_26bf51f3f12ad4e3`): Underlying::as\_fx is a const variant-narrowing accessor that returns Some(CcyPair) iff the underlying is the Fx arm, and None for all other arms (Metal, Equity, Commodity, DigitalAsset). Because CcyPair is Copy, the value is returned by copy, not reference, making this usable in const contexts. It is called by 10 downstream sites and is the primary dispatch gate for the FX pricing path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_fx` (hash `170771b97dcc1cc3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_metal

- **claim** (`cl\_92afcc8cf4d2ebd2`): Underlying::as\_metal is a const variant-narrowing accessor that returns Some(MetalPair) iff the underlying is the Metal arm, and None otherwise. MetalPair is Copy so the value is returned by copy. This separates the precious-metal pricing path from the FX and other asset-class paths without any allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as\_metal` (hash `fc23fff3e67f956a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.fmt

- **claim** (`cl\_38ed7bb46fcbd162`): Underlying::fmt (Display impl) delegates formatting to each variant's own Display impl without introducing any separator or wrapper text: Fx prints the CcyPair, Metal prints the MetalPair, Equity prints the EquityRef, Commodity prints the CommodityRef, DigitalAsset prints the CryptoPair. The match is exhaustive across all five arms, so the Display representation is always asset-class-specific and human-readable.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.fmt` (hash `7a2a8862e8f99a56`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.df\_dom

- **claim** (`cl\_0eb41473403e3e70`): VanillaInputs::df\_dom(&self) -\> f64 computes the domestic discount factor e^{-r\_dom·t} = libm::exp(-self.r\_dom \* self.t); VanillaInputs::df\_for(&self) -\> f64 computes the foreign discount factor e^{-r\_for·t} = libm::exp(-self.r\_for \* self.t). Both delegate to rust-lang/libm for cross-platform bit-identical results (same determinism contract as celnet\_core::math::exp). These are the standard continuously-compounded spot discount factors for the domestic and foreign rates respectively.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.df\_dom` (hash `89b5945866d02ae3`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.df\_for` (hash `7f32ef4c3b82b178`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.forward

- **claim** (`cl\_d609eff53dbc3d92`): VanillaInputs::forward(&self) -\> f64 computes the FX forward price F = S·e^{(r\_dom − r\_for)·t} = self.spot \* libm::exp((self.r\_dom - self.r\_for) \* self.t). This is the interest-rate-parity forward for a two-rate FX pair; it is also the carry-neutral forward for the Garman-Kohlhagen parameterisation. Delegates to libm::exp for cross-platform bit-identity.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.forward` (hash `32a82c2279d5bcf9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-types.src.lib.rate\_sensitivities\_arms

- **claim** (`cl\_3f7a8cc3a8d8824b`): rate\_sensitivities\_arms is a test that documents and enforces the FX–carry decomposition identity for RateSensitivities: given a discount rate sensitivity rho\_dom\_discount and a carry rate sensitivity carry\_rho, the FX arm satisfies rho\_dom = rho\_dom\_discount + carry\_rho and rho\_for = -carry\_rho. The test uses binary-exact constants (0.25 + 0.125 = 0.375) so the assertions are mathematically exact (no floating-point approximation error). This encodes the structural invariant that the FX rate sensitivities are a linear decomposition of discount and carry contributions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-types.src.lib.rate\_sensitivities\_arms` (hash `6f66882bf4eb5ae0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.adjoint.adjoint\_greeks

- **claim** (`cl\_669f211f8675b4a8`): \`adjoint\_greeks\` computes the full 14-field Greeks via AAD (Algorithmic Adjoint Differentiation). The forward pass records a \`Tape\` struct (all intermediate scalars needed for reverse differentiation); the first reverse pass (\`reverse(&tp, 1.0)\`) produces the first-order adjoints — delta\_spot=∂V/∂S, vega=∂V/∂σ, theta=−∂V/∂T, rho\_dom=∂V/∂r\_dom, rho\_for=∂V/∂r\_for — in a single backward sweep (O(1), same cost as a forward eval). Second-order gamma and vanna are obtained by hand-composing a second reverse pass over the delta\_spot expression (genuine reverse-over-reverse, not finite-difference), using the chain-rule edges stored on the tape: gamma=df\_for·φ(d1)·(1/(S·vsqt)), vanna=df\_for·φ(d1)·((σT)/vsqt−d1/σ). volga=vega·(−d1)·∂d1/∂σ. The mixed/higher-order tail (charm, speed, zomma, color) is taken from the analytic \`greeks\` path. The AAD price is bit-identical to \`price\` (pinned by \`aad\_price\_bit\_identical\`). Pure: reads (OptionType, &VanillaInputs), returns Greeks, no writes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.adjoint.adjoint\_greeks` (hash `8fea80335b49a6a8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.atm.atm\_strike

- **claim** (`cl\_5cb3e12fa39a8eaa`): Delta-Neutral-Straddle (DNS) ATM strike SIGN-FLIPS with the premium-adjusted delta convention — the single most surface-corrupting convention bug if mislocated. \`atm\_strike\` returns F·exp(+½σ²t) for unadjusted delta (SpotUnadjusted \| ForwardUnadjusted) but F·exp(-½σ²t) for premium-adjusted delta (SpotPremiumAdjusted \| ForwardPremiumAdjusted) — the DNS strike sits ABOVE the forward when unadjusted and BELOW it when premium-adjusted (opposite sign of the ½σ²t drift), exactly per ANALYTICS-SPEC §1.3. AtmForward simply returns the forward. The match on (AtmConvention, DeltaConvention) is exhaustive over both enums, so the half-variance sign is never defaulted. Pure: a total function of (atm, delta\_conv, forward, vol, t) returning f64, no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.atm.atm\_strike` (hash `34870ca9e2d9af07`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta

- **claim** (`cl\_7fbca34bc4bd03d1`): \`delta\` implements all four FX delta conventions over a shared \`delta\_aux\` precomputation. For SpotUnadjusted/ForwardUnadjusted: Δ = factor·N(±d1) (where factor = df\_for for spot, 1 for forward). For SpotPremiumAdjusted/ForwardPremiumAdjusted: Δ\_call = factor·(K/F)·N(d2); Δ\_put = Δ\_call − factor·(K/F) — premium-adjusted delta keys on N(d2) and carries the K/F ratio, which makes it non-monotone in K (the call has a maximum) and underpins the \`strike\_from\_delta\` reachability check. The match over DeltaConvention is exhaustive with no wildcard. Pure: reads (DeltaConvention, OptionType, &VanillaInputs), returns f64, no writes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta` (hash `362147f3ff17a466`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta\_aux

- **claim** (`cl\_9ad58a18cafcc069`): delta\_aux computes the shared intermediate quantities needed by all four delta conventions in a single pass: d1 = (ln(S/K) + (r\_dom − r\_for + ½σ²)T) / (σ√T), d2 = d1 − σ√T, factor = exp(−r\_for·T) for spot conventions (SpotUnadjusted, SpotPremiumAdjusted) and 1.0 for forward conventions (ForwardUnadjusted, ForwardPremiumAdjusted), and k\_over\_f = K/F. The function never branches on OptionType; the spot/forward distinction is the sole determinant of \`factor\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta\_aux` (hash `2e8bf2b438b19e2f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta\_d\_strike

- **claim** (`cl\_9c66e1050d833497`): Analytics-correctness (premium-adjusted-delta deliverable): celnet-vanilla::delta::delta\_d\_strike is a pure analytic ∂Δ/∂K. For the unadjusted conventions (Spot/Forward-Unadjusted) call and put deltas differ by a K-independent constant, so the strike-slope is the single term factor·φ(d1)·∂d1/∂K with ∂d1/∂K = ∂d2/∂K = −1/(K σ√T). For the premium-adjusted conventions (Spot/Forward-PremiumAdjusted) Δ\_call = factor·(K/F)·N(d2), so the slope follows the product rule in K (F is K-independent): factor·(N(d2)/F + (K/F)·φ(d2)·∂d2/∂K); the put slope is the call slope minus factor/F since Δ\_put = Δ\_call − factor·(K/F). This convention-branching strike-derivative is what makes the premium-adjusted delta non-monotone in strike (it underpins the guarded delta→strike root-find). Pure: deterministic in (conv, opt, &VanillaInputs), no WRITES edges.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta\_d\_strike` (hash `6929d44e35efcd5c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.premium\_adjusted\_call\_delta\_max

- **claim** (`cl\_def91c83a9a0916d`): Premium-adjusted call delta is NON-MONOTONE in strike — it has a maximum-delta strike with two strikes mapping to the same delta, so the strike↔delta root-find must be guarded and bracketed on the correct branch (ANALYTICS-SPEC §3.5). \`premium\_adjusted\_call\_delta\_max\` computes that delta-max strike as the stationary point: it solves g(d2)=N(d2)·σ√T−φ(d2)=0 (g increasing in d2, root at small positive d2) by bisection, then maps the root d2\* back to the strike K = F·exp(−½σ²T − d2\*·σ√T). This is the cap the solver must respect: a target delta above the achievable max is unreachable, and a naive monotone Brent/Newton would converge to the wrong branch or diverge. Pure: reads \`&VanillaInputs\`, returns the cap strike as f64 via a fixed-iteration bisection, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.premium\_adjusted\_call\_delta\_max` (hash `405fe7656ad1869d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.aux

- **claim** (`cl\_34b0fdac95b4b240`): \`aux\` is the internal precomputation kernel: it computes the four canonical option-pricing intermediates from \`&VanillaInputs\` — sqt=√T (via libm::sqrt), vsqt=σ·√T, d1=(ln(S/K)+(r\_dom−r\_for+½σ²)·T)/vsqt (via libm::ln), d2=d1−vsqt — and packages them in \`Aux\`. Both \`price\` and \`greeks\` call \`aux\` first and re-use these values throughout; computing them once avoids duplicate transcendental evaluations on the hot path. Pure: reads only &VanillaInputs, returns Aux, no allocation, no writes.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.aux` (hash `09f9c22e7398f849`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.greeks

- **claim** (`cl\_65b624944374aec1`): FX has exactly TWO rhos, never one. \`greeks\` returns both \`rho\_dom = ∂V/∂r\_d\` (call: K·t·e^{-r\_d t}·N(d2); put: -K·t·e^{-r\_d t}·N(-d2)) and \`rho\_for = ∂V/∂r\_f\` (call: -S·t·e^{-r\_f t}·N(d1); put: +S·t·e^{-r\_f t}·N(-d1)) as distinct fields of the Greeks struct — the foreign rate r\_f enters as the continuous dividend yield on the foreign-currency asset (Garman-Kohlhagen 1983), so a single equity-style "rho" is meaningless and is never exposed (ANALYTICS-SPEC §2.1). The two rho signs are opposite (domestic-rate up raises a call, foreign-rate up lowers it), so collapsing them would cancel real rate risk. Pure: it reads \`opt\` and \`&VanillaInputs\` and returns a \`Greeks\` value, mutating nothing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.greeks` (hash `79b7ca5b3998d5eb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.price

- **claim** (`cl\_2ebe84233d0cbf08`): GUARDRAIL/HOT-CORE — the FX vanilla pricing kernel \`price\` is allocation-free and lock-free by construction: it reads spot/strike discounted by df\_for()/df\_dom(), evaluates the closed form via norm\_cdf on the precomputed aux (d1,d2), and returns the f64 price for Call/Put — no allocation (alloc\_in\_loop=0, no Vec/Box), no loop (loop\_depth=0), no I/O, no logging, no locks. This is the pinned zero-alloc hot core: it is the leaf kernel the engine's hot pricing loop calls (in\_degree 13), and the engine's \`hot\_pricing\_loop\_allocates\_zero\` / \`hot\_pricing\_under\_concurrent\_publish\_allocates\_zero\` tests (a custom counting global allocator asserting zero allocations on the hot path) hold precisely because kernels like this allocate nothing. Pure: it reads &VanillaInputs and the OptionType and returns the f64 price, mutating nothing — telemetry/logging is offloaded off this path, never inlined into it.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.price` (hash `534b5ba2ca8e086c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.premium.premium\_from\_domestic\_pips

- **claim** (`cl\_1c49cec5601f447b`): PremiumStyle is the FX quotation-units axis and premium\_from\_domestic\_pips is its canonical converter (docs/CONVENTIONS.md PremiumStyle → premium units; docs/ANALYTICS-SPEC premium quotation). Given a price already in domestic pips (v\_dpips), it exhaustively maps the four PremiumStyle variants to their quoted unit: DomesticPips passes the raw PV through unchanged; PercentForeign divides by spot (per unit of foreign/base notional); PercentDomestic divides by strike (per unit of domestic/quote notional at strike); ForeignPips divides by spot·strike. The match is exhaustive over PremiumStyle, so no style is defaulted, and the DomesticPips arm is the identity (domestic\_pips\_is\_the\_raw\_pv). Pure: a total function of (style, v\_dpips, spot, strike) returning f64 with no writes/allocation/IO; deterministic under the f64 CPU-canonical/libm rule. Self-invalidates if the PremiumStyle variant set or any per-style scale factor changes (WRITES gate).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.premium.premium\_from\_domestic\_pips` (hash `4e15490dc71a9a8e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.bracket

- **claim** (`cl\_16fbef9181090b41`): \`bracket\` is the branch-aware bracketing primitive that makes the strike↔delta solve safe under the NON-MONOTONE premium-adjusted call delta (ANALYTICS-SPEC §3.5). It detects the premium-adjusted convention (SpotPremiumAdjusted \| ForwardPremiumAdjusted) and, for a Call, computes the delta-max strike via \`premium\_adjusted\_call\_delta\_max\`: a target\_delta above delta\_max+1e-12 is rejected as DeltaSolveError::Unreachable (the cap is the reachability boundary), and the returned bracket is deliberately pinned to the DECREASING (OTM) branch — lo=K\_max where Δ=delta\_max≥target ⇒ g(lo)≥0, hi expanded by doubling until g(hi)≤0 as K→∞ where Δ→0 — so the downstream root-find can never land on the ascending (ITM) branch that maps a different strike to the same delta. For unadjusted/put cases delta is monotone, so it geometrically expands \[tiny\_strike, f\] outward toward the shrinking-residual side until a sign change is found, returning Unreachable after 64 unsuccessful doublings. Pure: reads (conv,opt,target\_delta,&VanillaInputs,f,&at-closure) and returns Result\<(f64,f64),DeltaSolveError\>; it allocates nothing and mutates no external state (only loop-local lo/hi/iters).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.bracket` (hash `3cd23ae565b69be3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.reachable\_targets

- **claim** (`cl\_97a36f35e8aaf99e`): reachable\_targets produces the canonical set of solvable delta targets for a given (convention, option type, inputs) triple. For premium-adjusted call conventions it caps all magnitudes at 90% of the peak delta (delta evaluated at K\_max from premium\_adjusted\_call\_delta\_max), because the premium-adjusted call delta is non-monotone and its peak is the hard reachability ceiling. For all other convention/type pairs it applies no cap. Calls receive positive magnitudes; puts receive negative magnitudes from the same fixed grid \[0.005, 0.01, 0.05, 0.10, 0.25, 0.40, 0.45\].
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.reachable\_targets` (hash `dcd21c43dfb59d89`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.strike\_from\_delta

- **claim** (`cl\_9a4410f058d7a0b5`): \`strike\_from\_delta\` is the GUARDED strike↔delta root-find the non-monotone premium-adjusted delta demands (ANALYTICS-SPEC §3.5): it never runs a naive monotone Newton/Brent that could converge to the wrong branch. It first enforces sign discipline (a Call target\_delta\<0 or Put target\_delta\>0 ⇒ DeltaSolveError::WrongSign), then delegates the bracket to \`bracket\` — which, for the premium-adjusted call, calls \`premium\_adjusted\_call\_delta\_max\` to obtain the delta-max cap, returns DeltaSolveError::Unreachable for a target above the achievable max, and pins the bracket onto the correct (OTM, decreasing) branch above K\_max. The inner loop is a Brent-lite: it keeps the sign-straddling bracket \[lo,hi\] (debug\_assert glo·ghi≤0) as the safety net and only accepts a Newton step using delta\_d\_strike when it lands strictly inside (lo,hi), else falls back to bisection — so it is bracket-guaranteed convergent and cannot escape onto the wrong delta branch. Returns DeltaSolveError::NoConvergence rather than a wrong root if iteration stalls. Pure: it reads (conv,opt,target\_delta,&VanillaInputs), mutates only stack-local copies of the inputs (inp.strike) to evaluate delta/slope, performs no I/O, allocation, or external mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.strike\_from\_delta` (hash `8eb4664a1d6e1bea`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-xva.src.cva.XvaResult.total\_adjustment

- **claim** (`cl\_47e692755e1b305b`): XvaResult::total\_adjustment() = cva − dva + fva: the algebraically-signed aggregate valuation adjustment applied to the clean price (CVA reduces value, DVA increases it, FVA is additive as a funding cost on net exposure). Pure: reads only &self, returns f64, no mutation or allocation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.cva.XvaResult.total\_adjustment` (hash `035adcfa2b7dcec3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.crates.celnet-xva.src.cva.compute\_xva

- **claim** (`cl\_12e9d8b047d9d4da`): compute\_xva implements the standard discrete unilateral CVA/DVA/FVA formulas (Gregory, The xVA Challenge, 2015; Brigo-Morini-Pallavicini, 2013): CVA = LGD\_c · Σ\_k D(t\_k) · EPE(t\_k) · (S\_c(t\_{k-1}) − S\_c(t\_k)); DVA = LGD\_o · Σ\_k D(t\_k) · ENE(t\_k) · (S\_o(t\_{k-1}) − S\_o(t\_k)); FVA = funding\_spread · Σ\_k D(t\_k) · (EPE(t\_k) − ENE(t\_k)) · Δt\_k · S\_c(t\_k) · S\_o(t\_k). LGDs are hard-asserted to \[0,1\] (panic otherwise). The function reads only &XvaInputs and returns XvaResult — no mutation, no I/O, no allocation beyond the return value.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.cva.compute\_xva` (hash `0152ff7f8a6fdb15`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.deterministic

- **claim** (`cl\_36f31cab86d068d1`): ExposureProfile::deterministic is a validated constructor for externally-supplied EPE/ENE profiles (closed-form or scenario-override use): it asserts grid/epe/ene are equal-length and non-empty, grid\[0\] == 0, grid is strictly increasing, and every EPE/ENE entry is non-negative (panics on any violation). It then computes the discount vector as exp(−r\_dom·t\_k) over the supplied grid. This is the entry point for the closed-form XVA oracle tests that bypass Monte Carlo.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.deterministic` (hash `2a0ae0fd3420a818`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.simulate

- **claim** (`cl\_3d306733f6738baf`): ExposureProfile::simulate drives spot under risk-neutral GBM with Sobol low-discrepancy normals from celnet\_qmc::SobolSequence (one point per path, dimension = steps, each coordinate driving one time-step): drift = (r\_dom − r\_for − ½σ²)·Δt, vol\_step = σ·√Δt; spot\_{k+1} = spot\_k · exp(drift + vol\_step · Φ⁻¹(u\_k)). At each grid date t\_k the full netting set is repriced via NettingSet::net\_value(t\_k, spot), and EPE(t\_k) = E\[max(V,0)\], ENE(t\_k) = E\[max(−V,0)\] are computed as path-average sums. The discount vector at grid node t\_k is exp(−r\_dom · t\_k). Bit-reproducibility is guaranteed by the seeded Sobol stream (seed + steps fix the entire uniform sequence).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.simulate` (hash `1ba1eb086814c461`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettedTrade.mark

- **claim** (`cl\_71bfa17777138041`): NettedTrade::mark(t\_obs, spot, r\_dom, r\_for) is the per-trade mark-to-market at observation time t\_obs: it computes remaining time τ = expiry − t\_obs and returns 0.0 for any matured trade (τ ≤ 0), otherwise delegates to celnet\_vanilla::price via VanillaInputs::new(spot, strike, vol, τ, r\_dom, r\_for) scaled by notional. NettingSet::net\_value sums these marks across all trades, implementing plain algebraic netting (signed sum) within the set. Pure: reads only &self and scalar inputs, returns f64, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettedTrade.mark` (hash `d9ca377a6f2c82ca`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettingSet.net\_value` (hash `4613c5c380694601`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.cumulative\_hazard

- **claim** (`cl\_7542f4c3ab3fff36`): SurvivalCurve::survival(t) = exp(−Λ(t)) where Λ(t) is the piecewise-constant cumulative hazard. It is a pure total function: \`libm::exp(-self.cumulative\_hazard(t))\` with no mutation, no I/O. cumulative\_hazard(t) integrates the piecewise-constant λ segments up to t via an O(n) scan over (pillar, hazard) pairs, clipping each segment at min(pillar, t), and extrapolates the final hazard flat beyond the last finite pillar — so S(t) is continuous, monotone non-increasing, and equals 1 at t=0. Negative or non-finite t panics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.cumulative\_hazard` (hash `86439d18c4948285`, resolved)
  - `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.survival` (hash `ff7159d8222ec84e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.flat

- **claim** (`cl\_7b54519b1b0dee21`): SurvivalCurve::flat(lambda) constructs a constant-hazard survival curve representing S(t) = exp(−λ·t) for all t ≥ 0. It asserts lambda is finite and non-negative (panics otherwise) and stores a single pillar at f64::INFINITY, so cumulative\_hazard evaluates to λ·t for any finite t via the flat-extrapolation branch. This is the closed-form reference curve used by the XVA oracle tests.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.flat` (hash `af85fdc64b1d5a39`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.marginal\_default

- **claim** (`cl\_487344cda94420d9`): SurvivalCurve::marginal\_default(a, b) = S(a) − S(b): the marginal probability of default in the interval (a, b\]. It asserts a ≤ b, then returns survival(a) − survival(b). This is the interval default-probability atom consumed by compute\_xva's outer loop (dp\_c = s\_c\_prev − s\_c corresponds to marginal\_default over each grid interval). Pure: reads only &self and two f64 scalars, returns f64, no mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.marginal\_default` (hash `f6969ac6d81e17f9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.crates.celnet-xva.tests.closed\_form\_oracle.vanilla\_ref

- **claim** (`cl\_603a404ceb974eef`): vanilla\_ref is the independent closed-form oracle for Garman-Kohlhagen FX vanilla pricing used throughout the XVA test suite. Given spot S, strike K, flat vol σ, time-to-expiry τ, domestic rate r\_d and foreign rate r\_f it computes: vsqt = σ√τ, d1 = (ln(S/K) + (r\_d − r\_f + ½σ²)τ) / vsqt, d2 = d1 − vsqt, then Call = S·e^{−r\_f τ}·Φ(d1) − K·e^{−r\_d τ}·Φ(d2), Put = K·e^{−r\_d τ}·Φ(−d2) − S·e^{−r\_f τ}·Φ(−d1). Its sole purpose is to serve as a mutation-detectable oracle: any mutation of the production netting-set mark plumbing must produce a discrepancy against this independently derived reference.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.crates.celnet-xva.tests.closed\_form\_oracle.vanilla\_ref` (hash `3e3aaffbefd474a9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.e2e.corpus.cryptoUnderlier

- **claim** (`cl\_b6be7a420568e2d9`): cryptoUnderlier (e2e corpus) converts a 6-character token to the canonical underlier string: the first 3 chars are the base coin, the last 3 are the quote. For \`INVERSE\_COIN\` it appends \`:inverse\`; for \`LINEAR\` no suffix. Any other settlement style or a non-6-char token throws an Error, making the corpus strictly typed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.e2e.corpus.cryptoUnderlier` (hash `4a0353a375b99d3d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.e2e.corpus.equityUnderlier

- **claim** (`cl\_b00cfd5daefa77d6`): equityUnderlier maps exactly three equity tickers to their canonical venue-qualified string: SPX → "SPX@XCBO:USD", AAPL → "AAPL@XNAS:USD", STOXX → "STOXX@XEUR:EUR". Any other ticker throws immediately with no fallback.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.e2e.corpus.equityUnderlier` (hash `3fb4875397915c3f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.e2e.corpus.fixingToken

- **claim** (`cl\_bdf30260044b0405`): fixingToken maps exactly six NDF fixing-source variant names to their canonical fixing tokens: KrwKftc18 → "KRW.KFTC18", TwdTaipei → "TWD.TAIPEI", InrRbiRef → "INR.RBIB", BrlPtax → "BRL.PTAX", ClpDolarObs → "CLP.DOLAROBS", CopTrm → "COP.TRM". Any unknown variant throws with no fallback.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.e2e.corpus.fixingToken` (hash `4c4b1f4f8706e6a1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.e2e.corpus.listedFutureUnderlier

- **claim** (`cl\_3d56f4cd3795a156`): listedFutureUnderlier maps exactly two listed-future underlying tokens to venue-qualified strings: WTI → "WTI@:USD" (venue-less commodity form) and ES → "ES@XCME:USD". Any other token throws immediately.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.e2e.corpus.listedFutureUnderlier` (hash `dde3e3d882dca74c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.e2e.corpus.loadVectors

- **claim** (`cl\_4f3eb286643a94f1`): loadVectors reads all \*.json files from VECTORS\_DIR synchronously, parses each as a GoldenVector array, and concatenates them into one flat array. Non-JSON files are skipped (endsWith check). The function is deterministic given the directory contents.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.e2e.corpus.loadVectors` (hash `bcc386a8e641c4d7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.contract.instrumentCodec.cliquetFromWire

- **claim** (`cl\_9fbc3ffde5376407`): cliquetFromWire deserializes a WireObject to a Cliquet using presence-tracking for the four optional clamp fields (\`localFloor\`, \`localCap\`, \`globalFloor\`, \`globalCap\`): a key absent from the wire object is never written to the struct (no spurious \`0\`-clamp). Required fields (\`optionType\`, \`moneyness\`, \`periods\`, \`mcPairs\`, \`mcSeed\`) are always decoded.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.instrumentCodec.cliquetFromWire` (hash `db882e13281be070`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.src.contract.instrumentCodec.instrumentFromWire

- **claim** (`cl\_17e349dbc4c874a9`): instrumentFromWire enforces a strict one-arm constraint on the product oneof: it filters PRODUCT\_DECODERS by presence in the wire object and throws WireDecodeError if exactly one arm is not present. Optional fields (tenor, underlying, solve, settlement\_style, pricing\_model) are decoded only when their wire key is present, preserving proto3 zero-value semantics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.instrumentCodec.instrumentFromWire` (hash `bd591d3e67510c8c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.contract.instrumentCodec.underlyingFromWire

- **claim** (`cl\_db87f77d13b799be`): underlyingFromWire decodes the Underlying protobuf oneof from wire JSON by checking the presence of exactly one of the five discriminant keys: fx, metal, equity, commodity, digital\_asset. It throws WireDecodeError if none is set. All five arms extract a shared settlementCcy from the top-level wire object, making that field mandatory for every underlying kind.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.instrumentCodec.underlyingFromWire` (hash `ac36610817f5b0f7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.contract.riskCodec.attributionDisplayFromWire

- **claim** (`cl\_437c72707c5fd6c2`): attributionDisplayFromWire decodes a wire \`attribution\` object into a human-readable \`"held \<book\>/\<who\> \| quoted \<book\>/\<who\>"\` string (or \`undefined\` when absent). The \`who\` field is resolved with priority: \`trader\` string first, then \`autoPricer\` prefixed with \`"auto:"\`, otherwise empty. Returns \`undefined\` (not an empty string) when no attribution fields are present.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.riskCodec.attributionDisplayFromWire` (hash `f46a46504a1dd130`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.src.contract.riskCodec.nonAdditiveRiskFromWire

- **claim** (`cl\_217ca523a9ab0a2f`): nonAdditiveRiskFromWire decodes a NonAdditiveRisk from wire by copying only the present optional fields (var, es, var\_alpha, curvature\_spot), using optNum for each. Absent fields are not set on the result, preserving the partial-presence contract of non-additive risk metrics.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.riskCodec.nonAdditiveRiskFromWire` (hash `1020fca7bb3e5c33`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.contract.riskCodec.numToBigInt

- **claim** (`cl\_5c89e34aa00d768e`): numToBigInt (both wsCodec and riskCodec copies are byte-identical) converts a wire field to bigint via three cases: finite number → BigInt(Math.trunc(v)); already bigint → pass-through; non-empty string → BigInt(v) with fallback to 0n on parse error. Any other type returns 0n. Math.trunc prevents rounding of large JSON numbers before BigInt conversion.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.riskCodec.numToBigInt` (hash `03d6e1196352e71e`, resolved)
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.numToBigInt` (hash `19fc03916d001e4e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.contract.wsCodec.cliquetToWire

- **claim** (`cl\_2b456e4054c00eda`): cliquetToWire mirrors cliquetFromWire's presence-tracking in the encoding direction: \`mc\_seed\` is encoded as a plain JSON number (\`Number(c.mcSeed)\`) because seeds fit the JS safe-integer range and bigint would not JSON-serialise. The four optional clamp fields are emitted only when their TypeScript value is not \`undefined\` — an absent clamp sends no wire key, not a \`0\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.cliquetToWire` (hash `cd5d48c0108d4735`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.contract.wsCodec.instrumentToWire

- **claim** (`cl\_26d0f702f33bf1ae`): instrumentToWire serialises an Instrument to WireObject following proto3 zero-value suppression: settlement\_style is omitted when LINEAR (proto zero), pricing\_model is omitted when DEFAULT (proto zero), tenor is omitted for the perpetual arm (tenorless, expiry\_years=0), and underlying is omitted when absent. The product oneof is encoded under exactly one key matching the proto field name for 24 arms (vanilla through listed\_future\_option).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.instrumentToWire` (hash `28d2bf0ce607a2f4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.contract.wsCodec.quoteFromWire

- **claim** (`cl\_495812ce35910499`): quoteFromWire decodes a WireObject into a Quote struct. Three fields are conditionally present and set only when the server sends them: correlationId (optBigInt), surfaceVersion (optBigInt), and priceStdError (optNum). The priceStdError field is decoded as undefined when absent, ensuring the spill never fabricates a precision claim for non-Monte-Carlo products.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.quoteFromWire` (hash `4bbfeaf6ddce481e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.excel.src.contract.wsCodec.requoteLargeIntegers

- **claim** (`cl\_165373f1ef3cfda8`): requoteLargeIntegers rewrites a raw JSON string in a single character-by-character pass, quoting only pure integer literals that exceed the JS safe integer range; floating-point literals (containing \`.\`, \`e\`, or \`E\`) and integers within safe range are left unchanged. A string-tracking state machine ensures digits inside JSON string values are never touched. The result is valid JSON where large integers are encoded as quoted strings, allowing numToBigInt to recover the exact bigint value.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.requoteLargeIntegers` (hash `b63f3fcf0548cd96`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.contract.wsCodec.underlyingToWire

- **claim** (`cl\_d6e5e1c912e6107a`): underlyingToWire is a pure serialisation function that converts a typed Underlying value to a WireObject for transmission. It preserves the settlementCcy field on every output arm unchanged (const settlement\_ccy = u.settlementCcy is unconditionally injected into every returned object). The five arms cover: fx (copies base/quote directly), metal (encodes the metal enum via e.metal.toWire), equity (exposes symbol.ticker+symbol.venue+currency), commodity (exposes symbol.ticker+symbol.venue+currency), and digitalAsset (emits a snake\_case digital\_asset key matching the wire schema). The camelCase→snake\_case rename (digitalAsset → digital\_asset) is the only structural transformation; all numeric/string leaf values are forwarded verbatim.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.contract.wsCodec.underlyingToWire` (hash `06201e00209b193c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.functions.functions.premiumSpillFor

- **claim** (`cl\_dfde47a6c8fc64df`): premiumSpillFor routes a priced Quote to the correct SpillMatrix formatter based on product kind: "varianceSwap" → formatVarSwapSpill with quote.resolvedStrike as fairVariance; "volatilitySwap" → formatVolSwapSpill with quote.resolvedStrike as fairVol; all other products → formatPremiumSpill with quote.greeks.price as premium. The priceStdError (via stdErrorFor) is included only when present on the quote (MC-priced products), never fabricated.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.functions.premiumSpillFor` (hash `bd5909973d07fb6b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.functions.shapeSeriesRequest

- **claim** (`cl\_f4c960f8946eef2f`): shapeSeriesRequest normalizes delta inputs: a value ≥ 1 is divided by 100 (treating it as a percent-delta like 25 → 0.25), while a value \< 1 is passed as-is. SPOT observables require no tenor or delta; non-SPOT observables require a tenor; RISK\_REVERSAL and BUTTERFLY additionally require a delta.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.functions.shapeSeriesRequest` (hash `4eed6d2cdf6b6c47`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.excel.src.functions.functions.stdErrorFor

- **claim** (`cl\_e5e5579d13dc408d`): stdErrorFor is a pure switch over product kind that gates MC standard error surfacing precisely: cliquet/lookback/american expose priceStdError only when their respective isMonteCarlo predicates return true; exact closed-form families (vanilla, strategy, digital, touch, asian, forwardStart, quanto, varianceSwap, volatilitySwap, perpetualOption, listedFutureOption) always return undefined; MC-path-dependent families (singleBarrier, doubleBarrier, windowBarrier, tarf, pivot, accumulator, basket, fxForward, fxSwap, ndf) always return priceStdError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.functions.stdErrorFor` (hash `bfc8421e463d2a98`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.instrumentSpec.strategyFamily

- **claim** (`cl\_6e10073f2a1c4680`): strategyFamily returns a FamilySpec whose build function enforces that a product-name-bound StrategyKind (boundKind) is never silently overridden by a contradicting ("kind", …) term — the contradiction throws ShapingError. The term kind is accepted only when it matches boundKind or when boundKind is undefined (generic STRATEGY family).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.instrumentSpec.strategyFamily` (hash `2dcf925cf28a1582`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.canonicalProduct

- **claim** (`cl\_4f24df56cda61e30`): canonicalProduct produces a deterministic, order-stable JSON-serialisable representation of every Product variant for use as a subscription coalescing key. All bigint fields (\`mcSeed\`) are stringified via \`.toString()\` so the result is always JSON-safe. Strategy legs and basket legs are mapped to minimal tuples to ensure two structurally identical instruments always produce the same key.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.canonicalProduct` (hash `9f34adbda6843e46`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.formatRfqPanelSpill

- **claim** (`cl\_250d8a9b47a2cbe9`): formatRfqPanelSpill produces a rectangular SpillMatrix with a fixed header \["lp\_id","bid","offer","valid\_until","best"\], one row per LP line (BEST\_BID and/or BEST\_OFFER markers joined with "+" in the best column), a quote\_id row, and a convention-footer row. Output shape is always rectangular (padded by the rectangular() helper).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.formatRfqPanelSpill` (hash `aa1e74979102a47e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.formatSurfaceCubeSpill

- **claim** (`cl\_016aa8946d31e361`): formatSurfaceCubeSpill builds a pivot-table SpillMatrix over (tenor × delta) surface points: the union of all delta pillars forms the column headers (sorted ascending), each tenor row carries its tenorYears as the row label and vols indexed by delta (empty string for missing pillars), and a convention footer is appended as the last row.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.formatSurfaceCubeSpill` (hash `c59419807abda695`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseAccumulatorMonitoring

- **claim** (`cl\_ee0d9dd836f4e411`): parseAccumulatorMonitoring normalises raw input (trim + toUpperCase) and accepts the aliases DISCRETE/DISC/D (defaulting empty string to DISCRETE) and CONTINUOUS/CONT/C, returning the canonical AccumulatorMonitoring enum value. Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseAccumulatorMonitoring` (hash `b8ecdaaaf712fd91`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseAsianMethod

- **claim** (`cl\_1fa5d42cca27b4c5`): parseAsianMethod normalises and accepts CURRAN (default for empty input) and TW/TURNBULL\_WAKEMAN/TURNBULL-WAKEMAN/TURNBULLWAKEMAN aliases, returning the canonical AsianMethod. Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseAsianMethod` (hash `14df5e95456551fd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseAveragingStyle

- **claim** (`cl\_32960f1778ad0837`): parseAveragingStyle normalises and accepts DISCRETE/D (default for empty) and CONTINUOUS/CONT/C aliases, returning the canonical AveragingStyle. Any other value throws ShapingError. The implementation is structurally identical to parseAccumulatorMonitoring.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseAveragingStyle` (hash `4c161061c93ffff5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseBarrierKind

- **claim** (`cl\_c8df1ead70aa568f`): parseBarrierKind normalises and accepts KNOCK\_IN/KNOCK-IN/KNOCKIN/KI/IN aliases (returning "KNOCK\_IN") and KNOCK\_OUT/KNOCK-OUT/KNOCKOUT/KO/OUT aliases (returning "KNOCK\_OUT"). Empty string defaults to KNOCK\_IN. Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseBarrierKind` (hash `1e0991dbc8a60316`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseBarrierSide

- **claim** (`cl\_1208dd9ebcce6483`): parseBarrierSide normalises a raw string to exactly one of two BarrierSide literals: absent/empty/"UP"/"U" all map to "UP"; "DOWN"/"DN"/"D" map to "DOWN". Any other input throws ShapingError. The function is case-insensitive via \`.trim().toUpperCase()\` applied before the switch.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseBarrierSide` (hash `069b2cb54607e267`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseBasketKind

- **claim** (`cl\_f8049877a71690a5`): parseBasketKind maps a raw string to one of three BasketKind literals with a default of "BASKET" when input is absent or empty. "BASKET"/"WEIGHTED"/"SUM" → "BASKET"; "BEST\_OF"/"BEST-OF"/"BESTOF"/"BEST"/"MAX" → "BEST\_OF"; "WORST\_OF"/"WORST-OF"/"WORSTOF"/"WORST"/"MIN" → "WORST\_OF". Any unrecognised value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseBasketKind` (hash `ed2170bad823edca`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseCryptoPair

- **claim** (`cl\_c430a78125f3f65e`): parseCryptoPair splits a raw string into a {base, quote} CryptoPair using two strategies in order: (1) split on first separator matching /\[-/\_\]/ → extract two parts; (2) if no separator, peel a trailing known numeraire from the ordered list \[USDT, USDC, USD, EUR, BTC, ETH\] (requiring the base to be non-empty). If neither strategy yields both parts, throws ShapingError. Input is uppercased and trimmed before parsing.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseCryptoPair` (hash `835b17341520ed55`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseDigitalStyle

- **claim** (`cl\_636143a2ceb5845d`): parseDigitalStyle maps a raw string to one of two DigitalStyle literals, defaulting to "CASH\_OR\_NOTHING" when absent or empty. "CASH\_OR\_NOTHING"/"CASH-OR-NOTHING"/"CASH"/"C" → "CASH\_OR\_NOTHING"; "ASSET\_OR\_NOTHING"/"ASSET-OR-NOTHING"/"ASSET"/"A" → "ASSET\_OR\_NOTHING". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseDigitalStyle` (hash `61000f26b2e29276`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseLookbackMonitoring

- **claim** (`cl\_5f700e1cc367c61f`): parseLookbackMonitoring maps a raw string to one of two LookbackMonitoring literals, defaulting to "CONTINUOUS" when absent or empty. "CONTINUOUS"/"CONT"/"C" → "CONTINUOUS"; "DISCRETE"/"DISC"/"D" → "DISCRETE". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseLookbackMonitoring` (hash `06e458cb91a274af`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseLookbackStyle

- **claim** (`cl\_88baf02d1bab9212`): parseLookbackStyle maps a raw string to one of two LookbackStyle literals, defaulting to "FLOATING" when absent or empty. "FLOATING"/"FLOAT"/"FL" → "FLOATING"; "FIXED"/"FIX"/"FX" → "FIXED". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseLookbackStyle` (hash `9119bbcac38a4cdd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseMargining

- **claim** (`cl\_223f9f1dd0926eb2`): parseMargining maps a raw string to one of two Margining literals, defaulting to "EQUITY\_STYLE" when absent or empty. Before matching, punctuation (\`.\`, \`\_\`, whitespace, \`-\`) is stripped via \`.replace(/\[.\_\\s-\]/g, "")\`. "EQUITYSTYLE"/"EQUITY"/"UPFRONT" → "EQUITY\_STYLE"; "FUTURESSTYLE"/"FUTURES"/"DAILY" → "FUTURES\_STYLE". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseMargining` (hash `0085b9b45f8e8c92`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseMonitoringStyle

- **claim** (`cl\_d95f67943920215d`): parseMonitoringStyle maps a raw string to one of two MonitoringStyle literals, defaulting to "CONTINUOUS" when absent or empty. "CONTINUOUS"/"CONT"/"C" → "CONTINUOUS"; "DISCRETE"/"DISC"/"D" → "DISCRETE". Any other value throws ShapingError. Identical alias set to parseLookbackMonitoring but returns MonitoringStyle (a distinct type).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseMonitoringStyle` (hash `a33093b0c3dbec93`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseObservable

- **claim** (`cl\_a376e66e0fde54eb`): parseObservable maps a raw string to one of five MarketObservable literals. "ATM"/"ATMVOL"/"ATM\_VOL"/"VOL" → "ATM\_VOL"; "SPOT" → "SPOT"; "RR"/"RISK\_REVERSAL"/"RISKREVERSAL" → "RISK\_REVERSAL"; "BF"/"FLY"/"BUTTERFLY" → "BUTTERFLY"; "FWD"/"FORWARD" → "FORWARD". Input is required (no undefined default); any unrecognised value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseObservable` (hash `c63b476c8da768cc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parsePricingModel

- **claim** (`cl\_2c0a64850b558c15`): parsePricingModel maps a raw string to one of two PricingModel literals, defaulting to "DEFAULT" when absent or empty. "DEFAULT"/"ANALYTIC"/"CLOSED\_FORM"/"CLOSED-FORM" → "DEFAULT"; "LSV"/"LOCAL\_STOCH\_VOL"/"LOCAL-STOCH-VOL"/"LOCAL\_STOCHASTIC\_VOLATILITY" → "LOCAL\_STOCH\_VOL". Any other value throws ShapingError enumerating PRICING\_MODEL\_MEMBERS.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parsePricingModel` (hash `e8939bc74226930e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseQuantoPayoff

- **claim** (`cl\_5b76950c82b27a53`): parseQuantoPayoff maps a raw string to one of two QuantoPayoff literals, defaulting to "VANILLA" when absent or empty. "VANILLA"/"V"/"OPT" → "VANILLA"; "DIGITAL"/"DIG"/"D" → "DIGITAL". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseQuantoPayoff` (hash `607cfdb4544ef719`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseRfqPanelFlag

- **claim** (`cl\_28b96e42ee95c2b6`): parseRfqPanelFlag coerces a boolean \| string \| undefined to a plain boolean. undefined and false → false; true → true; the string is trimmed and uppercased: empty string → false; "PANEL" or "TRUE" → true; "FALSE" → false; any other string throws ShapingError. This is the only shaping parser whose input may be a native boolean.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseRfqPanelFlag` (hash `cd77a85e58c63ab7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseRiskDimension

- **claim** (`cl\_6d88177e180c6ca7`): parseRiskDimension maps a raw string to one of seven RiskDimension literals, defaulting to "FIRM" when absent or empty. "FIRM" → "FIRM"; "TRADER" → "TRADER"; "BOOK" → "BOOK"; "DESK" → "DESK"; "PAIR"/"CCYPAIR"/"CCY\_PAIR" → "CCY\_PAIR"; "LOCATION"/"LOC" → "LOCATION"; "ENTITY"/"LE" → "ENTITY". Any unrecognised value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseRiskDimension` (hash `bd0a33f70adb8803`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseSmileModel

- **claim** (`cl\_a7b472093b71dec6`): parseSmileModel maps a raw string to one of five SmileModel literals, defaulting to "MARKET\_HEDGE" when absent or empty. "VV"/"VANNA\_VOLGA"/"MARKET\_HEDGE" variants → "MARKET\_HEDGE"; "SABR"/"STOCHASTIC\_VOL" variants → "STOCHASTIC\_VOL"; "SVI"/"PARAMETRIC" → "PARAMETRIC"; "SSVI"/"PARAMETRIC\_SURFACE" variants → "PARAMETRIC\_SURFACE"; "ESSVI"/"EXTENDED"/"EXTENDED\_SURFACE" variants → "EXTENDED\_SURFACE". Unknown input throws ShapingError enumerating SMILE\_MODEL\_MEMBERS.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseSmileModel` (hash `353927cd2fb81436`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseStrategyKind

- **claim** (`cl\_9ad5cca747fa81bc`): parseStrategyKind maps a raw string to one of four StrategyKind literals. Before matching, all non-alphanumeric characters are stripped via \`.replace(/\[^A-Z0-9\]/g, "")\`. "RISKREVERSAL"/"RR" → "RISK\_REVERSAL"; "STRADDLE" → "STRADDLE"; "STRANGLE" → "STRANGLE"; "SEAGULL" → "SEAGULL". Input is required (not optional). Any unrecognised value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseStrategyKind` (hash `14a02952c0e2cbe9`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseStrikeOrDelta

- **claim** (`cl\_f4bf05959e78533e`): parseStrikeOrDelta accepts a number or string and returns a discriminated union {kind:"strike", strike} \| {kind:"delta", delta}. For numbers: must be finite and positive or throws ShapingError; yields kind:"strike". For strings: "ATM"/"DNS" → delta=0; a positive finite numeric string → kind:"strike"; regex /^(\\d+(?:\\.\\d+)?)\\s\*D\\s\*(\[CP\])$/ captures percent delta — percent must be in (0,100) and is signed: call side → +pct/100, put side → -pct/100.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseStrikeOrDelta` (hash `7704aedb2155a087`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseTarfRedemption

- **claim** (`cl\_e053094430ab2494`): parseTarfRedemption maps a raw string to one of two TarfRedemption literals, defaulting to "FULL\_GAIN" when absent or empty. "FULL\_GAIN"/"FULL"/"F" → "FULL\_GAIN"; "CAPPED\_GAIN"/"CAPPED"/"C" → "CAPPED\_GAIN". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseTarfRedemption` (hash `fd54b50a2c0b9c5f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseTenor

- **claim** (`cl\_ed703d03cac10e53`): parseTenor parses a tenor string into a {tenor:{unit, count}, expiryYears} ParsedTenor. "ON" and "O/N" map to {unit:"OVERNIGHT", count:1, expiryYears:YEARS\_PER\_UNIT.OVERNIGHT}. Otherwise matches regex /^(\\d+)\\s\*(\[WMY\])$/ → unit W→"WEEKS", M→"MONTHS", Y→"YEARS"; expiryYears = count × YEARS\_PER\_UNIT\[unit\]. Count must be positive or throws ShapingError. Unmatched input throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseTenor` (hash `833b92937aa062c7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.parseTouchKind

- **claim** (`cl\_3cfbea1e780938aa`): parseTouchKind maps a raw string to one of four TouchKind literals, defaulting to "ONE\_TOUCH" when absent or empty. ONE\_TOUCH aliases: "ONE\_TOUCH"/"ONE-TOUCH"/"ONETOUCH"/"OT"/"ONE"; NO\_TOUCH aliases: "NO\_TOUCH"/"NO-TOUCH"/"NOTOUCH"/"NT"/"NO"; DOUBLE\_NO\_TOUCH aliases: "DOUBLE\_NO\_TOUCH"/"DOUBLE-NO-TOUCH"/"DNT"; DOUBLE\_ONE\_TOUCH aliases: "DOUBLE\_ONE\_TOUCH"/"DOUBLE-ONE-TOUCH"/"DOT". Any other value throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.parseTouchKind` (hash `df8be4f0925482b2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeAccumulator

- **claim** (`cl\_aa6d19db11b3a7d4`): shapeAccumulator enforces a strict ordering invariant: the accumulator barrier must be strictly above the pivot (\`args.barrier \> pivot\`), and leverage must be ≥ 0. The pivot must be supplied as an absolute level (not a delta); a delta-form input throws ShapingError. The function always produces a TWO\_WAY side and delegates fixing schedule construction to equalFixingYears + shapeFixingNotional.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeAccumulator` (hash `db91226a9078ac20`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeAmerican

- **claim** (`cl\_6bf318956d79062a`): shapeAmerican implements a contradiction-rejection rule for exercise style: an explicit AMERICAN style with a positive bermudanSteps count is a typed error (throws ShapingError). A positive steps count without an explicit style or with an explicit BERMUDAN style selects BERMUDAN and generates equally-spaced Bermudan dates via equalBermudanDates. BERMUDAN requires steps ≥ 1. The strike must be an absolute level, not a delta.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeAmerican` (hash `5bce9a7ba98351de`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeAsianOption

- **claim** (`cl\_ef411e0a99f9512a`): shapeAsianOption requires an absolute strike level (not a delta) and validates elapsedWeight in the range \[0, 1) (\`elapsedWeight \>= 0 && elapsedWeight \< 1\`). For DISCRETE averaging, observations must be a positive integer (≥ 1). elapsedAvg must be finite and ≥ 0. The function is pure — no IO, no state mutation.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeAsianOption` (hash `9211cca65a454481`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeBarrier

- **claim** (`cl\_a2a0e87d1de2ea0e`): shapeBarrier dispatches between single-barrier and double-barrier products based on the presence of \`args.upperBarrier\`. For a double barrier, a \`side\` argument is rejected (throws ShapingError) and upper must be strictly above lower (\`upper \> lower\`). The \`pricingModel\` field is attached uniformly to the instrument regardless of barrier count. DEFAULT pricing model preserves wire byte-identity.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeBarrier` (hash `e4cb58a5aa376b4b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeBasket

- **claim** (`cl\_70c5efb64d80e527`): shapeBasket validates that the correlation matrix is exactly n×n (where n = number of legs), each leg row has at least 5 elements \[pair, weight, spot, vol, rFor\], spot must be \> 0, vol must be ≥ 0, and strike must be a positive absolute level. All numeric fields are validated as finite. The function pure-maps the flat correlation array by row-major linearization.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeBasket` (hash `2d1edce1e28b4516`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeCalibration

- **claim** (`cl\_2d91ce7e219d154c`): shapeCalibration validates ATM vol strictly in the range (0, 5) in absolute terms (\`atmVol \> 0 && atmVol \< 5\`), rr25/bf25 must both be finite, and rr10/bf10 are only required when both are supplied together (hasTenDelta). When 10-delta inputs are absent, rr10 and bf10 default to 0 and hasTenDelta is false.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeCalibration` (hash `adcc4fbc02546717`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeCliquet

- **claim** (`cl\_5ba79fcb546f81fd`): shapeCliquet enforces floor/cap ordering: localFloor must not exceed localCap, and globalFloor must not exceed globalCap. The mcSeed is converted to BigInt (\`BigInt(mcSeedRaw)\`) before placement on the wire object. Clamp fields (localFloor/localCap/globalFloor/globalCap) are only attached when explicitly supplied (presence-optional proto semantics), so absent clamps are genuinely unconstrained.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeCliquet` (hash `8e99b02f64ee0f3c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeFixingSource

- **claim** (`cl\_cc6031fd5a769b2c`): shapeFixingSource is a pure exhaustive switch that normalizes fixing-source strings to canonical enum values by uppercasing and stripping dots/underscores/spaces. It maps 6 EM-currency fixing sources: KRW\_KFTC18, TWD\_TAIPEI, INR\_RBI\_REF, BRL\_PTAX, CLP\_DOLAR\_OBS, COP\_TRM. Any unrecognized input throws ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeFixingSource` (hash `c4dc2c3fb60bad65`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeMetal

- **claim** (`cl\_7ea810fda5ce4bb6`): shapeMetal is a pure exhaustive switch normalizing precious-metal identifiers (GOLD/XAU, SILVER/XAG, PLATINUM/XPT, PALLADIUM/XPD) to canonical Metal enum values via uppercase normalization. Any unrecognized input throws ShapingError. The function has no side effects.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeMetal` (hash `d9951a3e142297e7`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapePivot

- **claim** (`cl\_c7daea77e593e813`): shapePivot enforces three positive-value constraints: pivot must be a positive level, target must be a positive cumulative gain, and leverage must be ≥ 0. The strike must be an absolute level (not a delta). Like TARF, the Pivot product uses a fixing schedule (equalFixingYears + shapeFixingNotional) and supports MC path budget/seed fields.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapePivot` (hash `0762dc8b82b23b9d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeQuanto

- **claim** (`cl\_f635b804bf88046e`): shapeQuanto validates conversionVol in the range \[0, 5) in absolute terms and correlation in the range \[-1, 1\]. The strike must be an absolute level. These three guards — finite strike, vol range, and correlation range — are the complete validation contract for a quanto instrument frame.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeQuanto` (hash `0d7246f86c1f7a70`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeReportingNumeraire

- **claim** (`cl\_3cd5e5ca63cdc23f`): shapeReportingNumeraire silently skips rows where the currency cell is empty/null/undefined and silently skips the numeraire currency itself (treated as implicitly 1.0). Invalid 3-letter currency codes or non-positive rates throw ShapingError. Currency codes are normalized to uppercase 3-letter form and validated by the regex \`/^\[A-Z\]{3}$/\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeReportingNumeraire` (hash `b95bf1541745912e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeSettlementStyle

- **claim** (`cl\_58920abb5f5a481f`): shapeSettlementStyle defaults to LINEAR when the input is undefined (\`raw ?? "LINEAR"\`). It normalizes the input by trimming, uppercasing, and stripping separators, then maps to exactly two values: LINEAR or INVERSE\_COIN (accepting INVERSE and COIN as aliases). All other inputs throw ShapingError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeSettlementStyle` (hash `869055346168d2ad`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeStrategy

- **claim** (`cl\_6c60f18968a5e1a3`): shapeStrategy enforces exact leg count from the STRATEGY\_TEMPLATE\_LEGS lookup table — the number of supplied legs must equal the template's expected count exactly. Each leg's ratio defaults to 1.0 when absent, and must be a positive number (ratio \<= 0 throws ShapingError because direction is encoded on the side field, not via a negative ratio).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeStrategy` (hash `8612648d9b8dc945`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeTarf

- **claim** (`cl\_d9aaf027347f0f45`): shapeTarf requires an absolute strike (not a delta), target \> 0, and leverage ≥ 0. It builds a fixing schedule via equalFixingYears + shapeFixingNotional, then assembles a Tarf with redemption style, MC budget (mcPairs/mcSeed), and sets side to TWO\_WAY.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeTarf` (hash `caf93d68d7585490`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.excel.src.functions.shaping.shapeTouch

- **claim** (`cl\_b47c8981e047b87f`): shapeTouch dispatches on isDoubleTouch(kind): double-touch kinds (DNT/DOT) require upperBarrier and enforce upper \> lower; single-touch kinds reject any supplied upperBarrier (throws ShapingError). The upperBarrier field on the Touch wire object is set to 0 for single-touch products.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.shapeTouch` (hash `cd8e0d14bb1c752a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.excel.src.functions.shaping.underlyingPairProjection

- **claim** (`cl\_15c2d2276ed2b994`): underlyingPairProjection is a pure, total function that maps every variant of the Underlying discriminated union to a CcyPair without side effects. The five exhaustive arms are: fx → u.fx (already a CcyPair {base, quote}); metal → {base: METAL\_ISO\_CODE\[u.metal.metal\], quote: u.metal.quote} (ISO code looked up from a static map); equity → {base: u.equity.symbol.ticker, quote: u.equity.currency}; commodity → {base: u.commodity.symbol.ticker, quote: u.commodity.currency}; digitalAsset → {base: u.digitalAsset.base, quote: u.digitalAsset.quote}. No network, state, or external side-effects; the switch is exhaustive so the function never throws for a valid Underlying.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.functions.shaping.underlyingPairProjection` (hash `78e36282d2603fa5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.taskpane.capability.classForUnderlying

- **claim** (`cl\_7161eb7872d84a7f`): classForUnderlying is a total, exhaustive pure switch from the five \`Underlying\['kind'\]\` discriminants to \`AssetClass\`: \`fx\` → \`"FX"\`, \`metal\` → \`"METAL"\`, \`equity\` → \`"EQUITY"\`, \`commodity\` → \`"COMMODITY"\`, \`digitalAsset\` → \`"CRYPTO"\`. TypeScript exhaustiveness ensures no runtime default is needed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.taskpane.capability.classForUnderlying` (hash `461c2572c629338b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.excel.src.taskpane.dealerPanel.lastLookRemaining

- **claim** (`cl\_425def0f2a07ade0`): lastLookRemaining computes a \[0,1\] ratio of last-look window remaining. It returns 0 when windowSeconds ≤ 0 or the deadline has passed, 1 when time remaining ≥ the full window, and remainingNanos / windowNanos otherwise. BigInt arithmetic is used for the nanos comparison to avoid JS safe-integer overflow; only the final ratio conversion calls Number().
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.taskpane.dealerPanel.lastLookRemaining` (hash `f00183770c341e79`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.excel.src.taskpane.taskpane.parseTerms

- **claim** (`cl\_90efa2a59b086b2e`): parseTerms splits a textarea string into TermsRow\[\] following the same key/value grammar that CELNET.INSTRUMENT reads: lines are split on '\\n', each non-blank line tokenised on whitespace, the first token is kept as a string (the key), subsequent tokens are coerced to numbers when finite, otherwise kept as strings. Blank lines are skipped. The shaper owns all validation; this function is input-only.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.taskpane.taskpane.parseTerms` (hash `ae64a14e85ecbc83`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.excel.src.transport.connection.subscriptionIdOf

- **claim** (`cl\_841a614cf704f929`): subscriptionIdOf extracts the subscription id from an RFS server frame's \`subscription.value\` field, normalizing both number and bigint wire types to bigint. Returns undefined for frames with no subscription field or a non-object subscription. This normalization is required because JSON transport delivers integers as JS numbers while the binary/proto path can deliver bigint.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.src.transport.connection.subscriptionIdOf` (hash `abc987f8352e79cf`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.excel.test.instrumentPolymorphic.test.legacyInstrumentOf

- **claim** (`cl\_52f989b1f1a351b5`): legacyInstrumentOf is the preserved verbatim dispatch of the retired per-product CELNET.\* worksheet functions (BARRIER, TARF, etc.). It calls the same production shapers those functions called with identical arguments, serving as the parity reference for the polymorphic instrument test. Four families (strategy, pivot, perpetual\_option, listed\_future\_option) are hand-built because they never had a per-product worksheet function.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.excel.test.instrumentPolymorphic.test.legacyInstrumentOf` (hash `9925223d2b68705c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.fuzz.fuzz\_targets.fix\_frame\_decode.structured\_roundtrip

- **claim** (`cl\_a3c69cfb859444fd`): DELIVERABLE fix-decoder-fuzz-target = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "celnet-fix violates verification-contract clause (f): no fuzz target for the only byte parser fed external-counterparty bytes"; reconciled against the live graph). \`structured\_roundtrip\` is the pure (side-effect-free) differential oracle inside the now-present fuzz/fuzz\_targets/fix\_frame\_decode.rs harness: it drives the production celnet-fix FrameCursor::parse over adversarial/arbitrary byte frames and asserts the structured-decode↔re-encode round-trip and in-domain invariants, giving the external-counterparty FIX byte parser the fuzz coverage that VERIFICATION-CONTRACT.md clause (f) mandates. SELF-INVALIDATES on any change to this fuzz oracle.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.fuzz.fuzz\_targets.fix\_frame\_decode.structured\_roundtrip` (hash `7535e960eb52239f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.e2e.goldenCorpus.fixingSource

- **claim** (`cl\_e1dd433008a82c21`): fixingSource (goldenCorpus) maps the corpus NDF fixing variant name (e.g. "BrlPtax") to the GUI contract FixingSource token (e.g. "BRL\_PTAX"). The six supported variants are KrwKftc18, TwdTaipei, InrRbiRef, BrlPtax, ClpDolarObs, CopTrm; any other variant throws an Error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.e2e.goldenCorpus.fixingSource` (hash `00a0a74d21ac41ad`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette

- **claim** (`cl\_0f1c173e3d895ae9`): CommandPalette is the ⌘K command-launcher overlay: a scrim-backed modal dialog with a fuzzy-matched (fuzzyMatch) search input over the caller-supplied Command list, ranking and showing the top results as a keyboard-navigable listbox. It renders null when closed, clears query/active on open, and runs the selected command's run() on Enter/click. It is the central action surface (pairs, workspaces, actions) styled through CSS-Module tokens (scrim/palette/item) rather than inline literals.
- **kind**: ui:component:dialog · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette` (hash `484117592f4b4340`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette

- **claim** (`cl\_cdb7139bd0f1b184`): CommandPalette is a labelled modal dialog implementing the ARIA APG combobox/listbox pattern: the surface is role="dialog" aria-modal="true" aria-label="command palette"; results render as role="listbox" with role="option" rows carrying aria-selected on the active item. Keyboard handling (ArrowUp/ArrowDown to move the active option, Enter to run, Escape to close) plus open-time focus of the search input make it fully keyboard-operable. This is the ⌘K command surface, so its dialog+listbox a11y contract is the reference for overlay launchers.
- **kind**: a11y:labeled · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:30Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette` (hash `484117592f4b4340`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:30Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:30Z

## github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid

- **claim** (`cl\_1af42cd2a535459e`): DataGrid is the single reusable virtualized data-grid primitive (generic over the row datum T): row-windowing (useVirtualWindow) and column-windowing (columnWindow) render only the visible slice, with optional grouping (flattenGroups + collapsible group rows), sortable columns, and roving-tabindex keyboard navigation. It reads row height and cell padding from the density tokens (--row-h, --cell-pad-x/y) via useRowHeight, so the comfortable/compact density axis applies without a JS branch. The blotter (StreamWorkspace), risk and book workspaces all compose this one component rather than re-implementing a grid.
- **kind**: ui:component:grid · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid` (hash `251fb848a0c1b5d5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid

- **claim** (`cl\_8ddf733e2436d681`): DataGrid is a WAI-ARIA APG composite grid: its root carries role="grid" with aria-label (the caller-supplied \`label\`), aria-rowcount and aria-colcount; the header band is role="row"/aria-rowindex=1 with role="columnheader" cells that expose aria-sort (ascending\|descending\|none) on sortable columns; body rows are role="row" with role="gridcell" cells, and group headers add aria-expanded. A single roving tabindex (one cell tabIndex=0, the rest -1) plus arrow/Home/End/PageUp/PageDown key handling makes it keyboard-operable as one composite widget. This is the shared virtualized grid powering the blotter/risk/book workspaces, so the grid-pattern a11y contract is centralized here.
- **kind**: a11y:role · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:30Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid` (hash `251fb848a0c1b5d5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:30Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:30Z

## github.com-soarsa-celnet.gui.src.components.FixSpecModal.safeHref

- **claim** (`cl\_ed858d0247e44ac7`): safeHref is a URL allowlist guard: doc-relative paths beginning './' are rewritten to '/fix/\<rest\>'; fragment and root-relative hrefs ('\#', '/') pass through; absolute URLs with protocol in {http:, https:, mailto:} pass through; everything else (including unparseable URLs) returns undefined. No href is fabricated — undefined signals rejection to the caller.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.FixSpecModal.safeHref` (hash `6143e560bd7076b8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.components.GreeksStrip.GreeksStrip

- **claim** (`cl\_50f5e41fe1a846e3`): GreeksStrip is the inline option-risk readout: a primary row of GreekCell tiles (delta/gamma/vega/theta) plus a disclosure button (aria-expanded + aria-label="toggle full Greeks") that reveals the secondary Greeks. It is asset-class-aware — rhoGreeksFor(assetClass) relabels the rate-rho Greeks per the active underlier's class (FX default) — so the same strip serves FX/equity/commodity/crypto tickets. Numerics render through GreekCell on the mono token face; the strip itself carries no raw color literals.
- **kind**: ui:component:strip · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.GreeksStrip.GreeksStrip` (hash `68338c932247fafd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.components.GreeksStrip.rhoGreeksFor

- **claim** (`cl\_7ac6728702517bc9`): rhoGreeksFor(cls) maps an AssetClass to the asset-class-specific pair of rate-rho GreekDef entries, relabelling rhoDom/rhoFor to their correct carry identity: FX/METAL→domestic+foreign rho pair; EQUITY→rate+dividend-yield rho; COMMODITY→rate+net-carry rho; CRYPTO→rate+funding rho. No new math — the wire fields rhoDom/rhoFor are unchanged; only labels differ.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.GreeksStrip.rhoGreeksFor` (hash `517cb3260aa51e8d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.components.ScopeSwitcher.orgChildLabels

- **claim** (`cl\_8c50d8dabc63534d`): orgChildLabels(level, path) is a pure read of ORG\_SCAFFOLD: when level==='desk' it returns every desk label; when level==='book' it returns the books of the desk named by the first 'desk' crumb in path, or the union of all books if no desk crumb is present.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.ScopeSwitcher.orgChildLabels` (hash `d89178847bf1ab98`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.components.SignInDialog.SignInDialog

- **claim** (`cl\_ac26fa3b971c409e`): SignInDialog is a properly-labelled modal: the panel is role="dialog" aria-modal="true" aria-labelledby pointing at the generated id of its \<h2\> title (via React.useId), it focuses the email field on open and closes on Escape, and surfaces auth errors in a role="alert" live region. The labelledby-to-title wiring (not a hard-coded string) keeps the accessible name in lockstep with the visible heading.
- **kind**: a11y:labeled · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:30Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.SignInDialog.SignInDialog` (hash `9e1a01bb8d991743`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:30Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:30Z

## github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge

- **claim** (`cl\_d35e87e573642db9`): StatusBadge is the small stream-health status pill: a single \<span\> driven purely by the StreamHealth enum, selecting a glyph (GLYPH\[health\]) and a per-state CSS-Module modifier class (styles\[health.toLowerCase()\]) that colors it from the semantic tokens. It is stateless and presentational, and is the only component with committed Storybook stories (StatusBadge.stories.tsx → components-statusbadge--\* in the static index), making it the visual-regression and token-rendering reference fixture.
- **kind**: ui:component:badge · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge` (hash `3c0c3e1ee3b8d638`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge

- **claim** (`cl\_d8746b7fbd2727bd`): StatusBadge renders its stream-health glyph as role="img" with an aria-label (and matching title) derived from the StreamHealth state, so the purely-visual glyph is announced as a single labelled image to assistive tech rather than read as a bare emoji character. It is the StatusBadge story exercised by gui/storybook-static (components-statusbadge--\* entries: Default/Healthy/Resyncing/Stale/All States), making it the canonical visual + a11y reference for stream-health status surfaces.
- **kind**: a11y:role · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:30Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge` (hash `3c0c3e1ee3b8d638`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:30Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:30Z

## github.com-soarsa-celnet.gui.src.data.mockSource.applyShock

- **claim** (`cl\_3ac50ba0540b36b6`): applyShock returns a new MarketContext with exactly one field bumped according to the ShockFactor. For relative bumps, \`new = base \* (1 + step)\`; for absolute, \`new = base + step\`. VOL is floor-clamped to 0.001 to prevent non-positive variance. TIME is a no-op (handled by expiry roll in the caller). All other fields are passed through unchanged via object spread.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.mockSource.applyShock` (hash `29528c1861b86815`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.data.mockSource.freezeStrikes

- **claim** (`cl\_a814f003194c1e81`): freezeStrikes resolves delta-specified strikes to absolute levels for vanilla, strategy (all legs), single-/double-/window-barrier (the embedded vanilla strike only). All other product kinds (digital, touch, variance/volatility swap, Asian, forward-start, cliquet, quanto, TARF, pivot, accumulator, lookback, American, basket, fxForward, fxSwap, NDF, perpetualOption, listedFutureOption) carry no delta-able strike and are returned unchanged.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.mockSource.freezeStrikes` (hash `d6dd53ffe08cd43b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.mockSource.rankSide

- **claim** (`cl\_2b10e8f3a9f4c4ce`): rankSide(rows, side) returns the lpId of the best dealer on one panel side using the engine's deterministic ranking law: highest bid (or lowest offer) wins; ties are broken lexicographically by lpId ascending. Returns empty string if rows is empty.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.mockSource.rankSide` (hash `a370f4671225cb3a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.gui.src.data.pricing.americanBinomialValue

- **claim** (`cl\_b1ddf1a922355d3c`): americanBinomialValue prices American and Bermudan options on a CRR binomial tree with AMERICAN\_BINOMIAL\_STEPS steps. The FX risk-neutral up-probability is \`p = (exp((r\_d - r\_f)dt) - d) / (u - d)\` where \`u = exp(σ√dt)\`, \`d = 1/u\`. Backward induction applies early exercise at every layer (American, \`exerciseLayers === null\`) or only at the supplied Set of layer indices (Bermudan). At exercisable nodes, \`value\[j\] = max(continuation, intrinsic(S))\` where continuation is \`disc \* (p\*value\[j+1\] + q\*value\[j\])\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.americanBinomialValue` (hash `081c0c36696d4168`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.data.pricing.bermudanExerciseLayers

- **claim** (`cl\_b24c3986c6c3f1fe`): bermudanExerciseLayers maps Bermudan exercise dates (year-fractions in (0, t\]) to step indices on a binomial tree of given size. Each date is snapped to \`round((date/t) \* steps)\`, kept only when in \[1, steps\]. The terminal layer (expiry = steps) is always included unconditionally. Dates outside (0, t\] are silently ignored, matching the server's domain rule.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.bermudanExerciseLayers` (hash `78f28229a0ce8494`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.data.pricing.dntSurvival

- **claim** (`cl\_75df04ee87fe6c74`): dntSurvival computes the double-no-touch survival probability using the method-of-images series (image count ±DNT\_IMAGE\_TERMS). The result is clamped to \[0,1\] by three guards: each inner weighted() clamps to 0 on non-positive CDF differences; the series sum is clamped to \[0,1\]; and the result is additionally capped by the minimum of two single-wall hit-probability caps (1 − singleWallHitProb). Spot on or outside the corridor immediately returns 0.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.dntSurvival` (hash `79910c458d674acb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.data.pricing.doubleKnockOutValue

- **claim** (`cl\_e3ad89eee6e4afcd`): doubleKnockOutValue implements the Ikeda-Kunitomo method-of-images series for a double-knock-out option: drift=(1+μ)·σ√T, μ=b/σ²−½, μ₁=2(μ+1). Each iteration n ∈ \[−DKO\_TERMS, +DKO\_TERMS\] contributes a direct image (S·(U/L)^{2n}) and a lower-wall mirror image (L^{2n+2}/(U^{2n}·S)); both asset and cash legs are summed separately with φ=±1 for call/put orientation. The result is floored at 0 via Math.max(sum,0). Spot on or outside the corridor returns 0 immediately.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.doubleKnockOutValue` (hash `0c6cf39dc51ecc5b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.pricing.lookbackContinuousValue

- **claim** (`cl\_1ad3dd122c15358c`): lookbackContinuousValue implements the exact closed-form lookback option price. For FLOATING style (running extremum = S at inception) it applies the standard floating-strike lookback formula with reflection term: call = S\*e^{-r\_for\*t}\*N(a1) - S\*e^{-r\_dom\*t}\*N(a2) + reflection, where a1 = (b + sigma^2/2)t / (sigma\*sqrt(t)), a2 = a1 - sigma\*sqrt(t), b = r\_dom - r\_for. For FIXED style (Conze-Viswanathan) it uses the fixed-strike formula conditioned on whether k \>= s or k \< s for calls, and k \<= s or k \> s for puts, each with a reflection term involving (s/k)^{-2b/sigma^2}.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.lookbackContinuousValue` (hash `89ba6c09eb361be2`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.pricing.pivotPathBankPv

- **claim** (`cl\_7d6de65843c81956`): pivotPathBankPv computes one antithetic-signed path's discounted bank PV for a Pivot/TRA structure. Per fixing k: log-spot is advanced by (r\_dom - r\_for - ½σ²)·dt + σ·√dt·s·z\[k\]; the intrinsic gain c includes leverage gearing when spot is on the adverse side of the pivot; accumulation stops and the function returns early when a fixing's gain reaches or breaches the target (redemption applied). Adverse fixings (c\<0) add to bankPv as receipts.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.pivotPathBankPv` (hash `a098a01830154c6c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.data.pricing.pricePerpetual

- **claim** (`cl\_faaedcaf024eb278`): pricePerpetual computes the exact closed-form price and full analytic Greek strip (delta, gamma, vega, rhoDom, rhoFor) for a perpetual American vanilla. It throws on r\_dom\<0 (no finite value) and on a perpetual call with b\>r (diverges). The free boundary S\_b = K·y/(y−1) determines: beyond S\_b the value equals intrinsic exactly (delta=±1); in the continuation region V = \|S\_b−K\|·(S/S\_b)^y, with Greeks via exact chain rule through the characteristic root. Theta and time-tenor Greeks are identically zero by the time-homogeneous construction.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.pricePerpetual` (hash `54bb92f799133cf5`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.data.pricing.tarfPathBankPv

- **claim** (`cl\_9b9cebc6f9c4e45e`): tarfPathBankPv computes the discounted bank PV along a single Monte-Carlo TARF path with antithetic sign s. At each fixing: if the signed gain breaches the remaining target, the structure redeems immediately — settling either the full gain or only the gap-to-target per spec.redemption ('FULL\_GAIN' vs. partial), discounting at exp(-rDom \* (k+1) \* dtYears), and returning early. Adverse fixings (signed \< 0) add spec.leverage \* \|signed\| \* df to bankPv. The function returns early on redemption, guaranteeing no subsequent fixings are priced.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.tarfPathBankPv` (hash `2f2c04e8b0432fa4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.data.pricing.touchValue

- **claim** (`cl\_edd82c1f2e96a946`): touchValue dispatches exhaustively over four Touch.kind variants (ONE\_TOUCH, NO\_TOUCH, DOUBLE\_NO\_TOUCH, DOUBLE\_ONE\_TOUCH), delegating each to its own closed-form pricing helper — oneTouchValue, noTouchValue, doubleNoTouchValue, doubleTouchValue — and returns the resulting premium. The switch is exhaustive; no default branch is needed.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.pricing.touchValue` (hash `2b593145c232578c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.data.seed.strategyLegs

- **claim** (`cl\_ca09ea1e08121a99`): strategyLegs (seed.ts) returns the canonical convention-delta default legs for each StrategyKind: RISK\_REVERSAL = \[25dC BUY, 25dP SELL\]; STRANGLE = \[10dC BUY, 10dP BUY\]; STRADDLE = \[50dC BUY, 50dP BUY\]; SEAGULL = \[25dC BUY, 10dC SELL, 25dP SELL\]. All strikes use delta convention. The companion templateDefaultLegs in strategy.tsx carries an identical ladder — the docstring explicitly states the round-trip test pins them byte-for-byte.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.seed.strategyLegs` (hash `008c04026eaeaa29`, resolved)
  - `github.com-soarsa-celnet.gui.src.products.strategy.templateDefaultLegs` (hash `0d64d547ba4e101b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.data.seed.tenorYearsToTenor

- **claim** (`cl\_966dcc9f27eae22b`): tenorYearsToTenor maps a fractional-year duration to the coarsest tenor unit that covers it, using fixed thresholds: ≤2/365 → OVERNIGHT(1), \<25/365 → WEEKS(round(y×52)), \<360/365 → MONTHS(round(y×12)), else YEARS(max(1,round(y))). The result is always a non-zero-count Tenor.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.seed.tenorYearsToTenor` (hash `f59cc3302082a382`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.seed.underlyingPairProjection

- **claim** (`cl\_7adbd00bfad560b4`): underlyingPairProjection extracts a CcyPair from any Underlying kind: fx uses the fx field directly; metal builds {base: METAL\_ISO\_CODE\[metal\], quote}; equity uses {ticker, currency}; commodity uses {ticker, currency}; digitalAsset uses {base, quote}. The mapping is exhaustive and allocation-minimal (no intermediate objects beyond the returned pair).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.seed.underlyingPairProjection` (hash `3f54a247e377b28f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.data.surface.legsOf

- **claim** (`cl\_5fcf1b6c391be6bb`): legsOf extracts the smile-representative strike legs from an Instrument for use in vega-weighted smile reads. It returns one leg for vanilla, singleBarrier, doubleBarrier, windowBarrier, digital, and american products (using their embedded vanilla/strike field). Strategy products return all their legs. The following product families return an empty array (ATM fallback): touch, varianceSwap, volatilitySwap, asianOption, forwardStart, cliquet, quanto, tarf, pivot, accumulator, lookback, basket, fxForward, fxSwap, ndf, perpetualOption, listedFutureOption.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.surface.legsOf` (hash `e70c913b655da297`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.data.surface.modelWingShape

- **claim** (`cl\_29cb2c07768831ad`): modelWingShape returns per-model wing-shape coefficients (convexity, asymmetry, wingPower) used to reshape the 10d/25d smile for display. The coefficients keep ATM exact (weight 0 at ATM) and preserve the RR sign. Values: MARKET\_HEDGE=(1.0, 1.0, 2.0); STOCHASTIC\_VOL=(1.12, 0.96, 2.2); PARAMETRIC=(0.9, 1.04, 1.8); PARAMETRIC\_SURFACE=(1.04, 0.92, 2.1); EXTENDED\_SURFACE=(1.08, 1.0, 2.15).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.surface.modelWingShape` (hash `4ce9ad067169453e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.data.surface.sampleSmilePoints

- **claim** (`cl\_4d8e2df8f8e330d8`): sampleSmilePoints performs linear interpolation of calibrated smile volatility on the signed-delta axis: it sorts smile.points by delta, clamps to the wing vols at both extremes, and for the bracketing segment \[a, b\] returns a.vol + (b.vol - a.vol) \* t where t = (delta - a.delta) / (b.delta - a.delta), guarding against zero-width spans (t = 0). Falls back to brokerQuotes.atmVol when no calibrated points exist or no bracket is found.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.surface.sampleSmilePoints` (hash `eac8d9ccdf2b484d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.data.surface.sampleSurface

- **claim** (`cl\_6fe82313245882cd`): sampleSurface performs bilinear surface interpolation: it finds the bracketing tenor pair (lo, hi) in the sorted smiles array, computes weight w = (tenorYears - lo.tenorYears) / (hi.tenorYears - lo.tenorYears) (clamped to 0 for zero span), calls sampleSmilePoints for both tenors, and returns vLo + (vHi - vLo) \* w. Returns 0 for an empty surface.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.surface.sampleSurface` (hash `70c728cac9aff781`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.attributionFromWire

- **claim** (`cl\_00d64efce960abfa`): attributionFromWire decodes an optional wire 'attribution' object into an AttributionRecord. It decodes 'quotedBy', 'heldBy' (via bookIdFromWire), 'won' (boolean), and 'lpCount' (number), setting only the fields present on the wire. If none of the fields decoded, it returns undefined rather than an empty object — explicitly honoring an 'honest empty-state, not {}' contract.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.attributionFromWire` (hash `06a701c7a6cec131`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.attributionToWire

- **claim** (`cl\_cc0aacfbcdd77d6f`): attributionToWire serializes an AttributionRecord to a WireObject, omitting fields that are undefined or falsy (quotedBy/heldBy use truthiness; won/lpCount use explicit \`!== undefined\`). It is the strict inverse of attributionFromWire for any non-empty record.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.attributionToWire` (hash `270528a8e7686825`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.instrumentToWire

- **claim** (`cl\_f8fce017b61ca6fa`): instrumentToWire serialises every Instrument variant to a WireObject using an exhaustive switch on product.kind. Proto3 zero-value fields are omitted: settlement\_style LINEAR is never emitted; pricing\_model DEFAULT is never emitted; an absent tenor emits no \`tenor\` key (perpetual option canonical shape). Every wire field name is the proto snake\_case field name, and enum values are their numeric proto tags. The function covers all 24 product arms (vanilla, strategy, singleBarrier, doubleBarrier, digital, touch, varianceSwap, volatilitySwap, asianOption, forwardStart, cliquet, quanto, tarf, pivot, accumulator, lookback, windowBarrier, american, basket, fxForward, fxSwap, ndf, perpetualOption, listedFutureOption).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.instrumentToWire` (hash `88c5cc5ebcc81e27`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.nonAdditiveRiskFromWire

- **claim** (`cl\_d95d2e3a04a2fe2d`): nonAdditiveRiskFromWire deserialises a NonAdditiveRisk from a WireObject using presence-tracking: each field (var, es, var\_alpha, curvature\_spot) is read via optNum and written to the output only if not undefined, so an absent wire field never produces a spurious zero on the domain object.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.nonAdditiveRiskFromWire` (hash `c7a55d723c4791c4`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.numToBigInt

- **claim** (`cl\_e28311f300f1897a`): numToBigInt coerces a WireObject field to bigint with three-way type dispatch: finite JS number -\> BigInt(Math.trunc(v)); already a bigint -\> returned as-is; non-empty string -\> BigInt(v) with parse errors caught and mapped to 0n. All other types (undefined, null, object) return 0n. Math.trunc ensures no rounding surprises on integer-valued floats.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.numToBigInt` (hash `2f58aedda4fe76ea`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.ownerFromWire

- **claim** (`cl\_beb2f6bd4fc1a70d`): ownerFromWire(o) decodes an Owner union from a wire object: if o\['owner'\] is absent or non-object it returns undefined; if o\['owner'\]\['trader'\] is a string it returns {kind:'trader', trader}; if o\['owner'\]\['autoPricer'\] is a string it returns {kind:'autoPricer', autoPricer}; otherwise undefined. Never fabricates a value.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.ownerFromWire` (hash `d2197b48bab947c1`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.quoteFromWire

- **claim** (`cl\_daa38be629edaac7`): quoteFromWire(o) deserializes a wire Quote object. The required fields (quoteId, idempotencyKey, price, greeks, conventions, resolvedStrike, epochNanos, validUntilNanos) are always decoded. Optional fields correlationId, surfaceVersion, attribution, and priceStdError are decoded only when present on the wire object, preserving the invariant that closed-form products never carry priceStdError.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.quoteFromWire` (hash `81ff60946f26a26d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.requoteLargeIntegers

- **claim** (`cl\_d421e4e2ef32d899`): requoteLargeIntegers(raw) is a single-pass state machine that rewrites JSON integer literals exceeding the JS safe integer range into quoted strings (so JSON.parse yields strings that numToBigInt can recover as bigint), while leaving string contents, fractional numbers, exponent numbers, and structure tokens untouched. It tracks in-string state to skip digits inside JSON string values.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.requoteLargeIntegers` (hash `9d8b29aef87be6ca`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.snapshotFromWire

- **claim** (`cl\_7e04e8e0e1075922`): snapshotFromWire decodes a WireObject into a Snapshot by composing typed sub-decoders: subscriptionIdFromWire, numToBigInt (sequence, epochNanos), twoWayFromWire (price), greeksFromWire, num (vol, resolvedStrike), conventionsFromWire, tradableVec. Optional fields (surfaceVersion, correlationId, attribution) are attached only when present in the wire object via optBigInt / attributionFromWire.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.snapshotFromWire` (hash `cf95b5e1ce0c6b2a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.data.wsCodec.underlyingToWire

- **claim** (`cl\_afdd78b79654f66e`): underlyingToWire serialises an Underlying to a WireObject: it exhaustively matches all five Underlying kinds, propagating the settlementCcy field on every arm. The metal arm additionally converts the metal enum via e.metal.toWire(); all other fields are copied verbatim. The function is pure and has exactly one caller.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsCodec.underlyingToWire` (hash `9f5689a7cae13b90`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.data.wsTransport.subscriptionIdOf

- **claim** (`cl\_59f3f3cb100373ac`): subscriptionIdOf extracts the subscription id from a WireObject as a bigint: it reads o\['subscription'\]\['value'\], accepting both number (coerced via BigInt()) and bigint; returns undefined for absent, non-object, or non-numeric values. This is the sole path by which server RFS frames are correlated to local subscriptions.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.data.wsTransport.subscriptionIdOf` (hash `1fb6ec6d55cd7285`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.hooks.useStreamSession.distillObservability

- **claim** (`cl\_b222191a449c6e68`): distillObservability reduces a Map\<bigint,Heartbeat\> into a ServerObservability by accumulating the sum of all conflationDrops, the worst (max) serverPriceP99Nanos across all beats, and the most-recent beat (highest epochNanos) for surfaceVersion/correlationId. An empty map returns the constant EMPTY\_OBSERVABILITY without allocating.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.hooks.useStreamSession.distillObservability` (hash `f544ec90137dc66b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.hooks.useStreamSession.rejectText

- **claim** (`cl\_4fce98fe9881e9db`): rejectText(reason) is an exhaustive pure map from the three last-look rejection reasons to human-readable strings: EXPIRED→'last-look window expired', UNKNOWN\_TOKEN→'token no longer live', ALREADY\_CONSUMED→'already traded'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.hooks.useStreamSession.rejectText` (hash `12436de68914dd8b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.assetUniverse.searchUnderliers

- **claim** (`cl\_cd1607179dc6e398`): searchUnderliers filters and ranks UnderlierRow entries with a two-stage fuzzy match: the query must subsequence-match row.haystack (miss → excluded), and highlight indices are taken from the match against row.label (if the label doesn't match, indices are \[\]). Empty query returns all rows with score 0. Results are sorted descending by score then ascending by label.localeCompare.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.assetUniverse.searchUnderliers` (hash `258d889f548b4ca8`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.lib.assetUniverse.underlierAssetClass

- **claim** (`cl\_89e82072c908e10c`): underlierAssetClass is a pure total function that maps every Underlying.kind discriminant to its AssetClass string: fx→'FX', metal→'METAL', equity→'EQUITY', commodity→'COMMODITY', digitalAsset→'CRYPTO'. The switch is exhaustive.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.assetUniverse.underlierAssetClass` (hash `a914727454d3f842`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.lib.commands.buildCommands

- **claim** (`cl\_0b03620a610dd834`): buildCommands constructs the live Command array from COMMAND\_META, filtering out any command whose action is unavailable in the current CommandContext (e.g. 'scope-drill-down' when canDrillScope is false). Workspace-jump commands are generated from the RAIL constant (one per rail entry, id 'ws-{r.id}'). The output order mirrors COMMAND\_META order, not the insertion order of the action map.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.commands.buildCommands` (hash `d4744e25d21f76ac`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.lib.commands.resolveChord

- **claim** (`cl\_90f68ee220986434`): resolveChord(e, digitCount) resolves a keyboard event to a command id (and optional rail index) by matching against COMMAND\_META chord definitions. Three chord kinds are supported: 'meta' (meta+fixed key), 'metaDigit' (meta+1–9, producing ws-\<railId\> + railIndex when idx \< digitCount), and 'plain' (bare key). Returns null if no chord matches.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.commands.resolveChord` (hash `3c01df339e46b3f6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.lib.format.deltaConvChip

- **claim** (`cl\_0fb2b56967e12bc3`): deltaConvChip is a pure, exhaustive mapping from the four DeltaConvention enum values to their display strings: SPOT\_UNADJUSTED→"spot Δ", FORWARD\_UNADJUSTED→"fwd Δ", SPOT\_PREMIUM\_ADJUSTED→"spot Δ pa", FORWARD\_PREMIUM\_ADJUSTED→"fwd Δ pa".
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.format.deltaConvChip` (hash `16eb210685f62ca6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.lib.format.fmtPnlAdaptive

- **claim** (`cl\_b5ad454a8ccd188a`): fmtPnlAdaptive formats a number into a compact signed string with adaptive scale: \|value\| ≥ 1,000,000 → 'm' suffix (1 decimal unless ≥ 10M where 0); ≥ 1,000 → 'k' suffix (1 decimal unless ≥ 10k where 0); ≥ 1 → integer; \< 1 → 2 decimal places. The sign prefix is '−' (Unicode minus) for negatives, '+' for positives, and empty for zero. Returns "0" for exact zero.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.format.fmtPnlAdaptive` (hash `67463ec084b11a17`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.lib.format.premiumChip

- **claim** (`cl\_00706e5d4b53bb15`): premiumChip(c) is an exhaustive pure map from PremiumStyle to its short display label: DOMESTIC\_PIPS→'dom pips', PERCENT\_FOREIGN→'% for', PERCENT\_DOMESTIC→'% dom', FOREIGN\_PIPS→'for pips'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.format.premiumChip` (hash `057d8c4abab1cebb`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.lib.format.premiumUnit

- **claim** (`cl\_3ab0dd762b28e034`): premiumUnit(c) maps Conventions.premiumStyle to its unit suffix string: PERCENT\_FOREIGN→'%', PERCENT\_DOMESTIC→'% dom', DOMESTIC\_PIPS→'pips', FOREIGN\_PIPS→'for pips'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.format.premiumUnit` (hash `bc0d3d16e24a9134`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.lib.format.sideVerb

- **claim** (`cl\_e33046bc2d6ea599`): sideVerb is an exhaustive pure mapping from the three-variant Side union to display strings: BUY → 'Buy', SELL → 'Sell', TWO\_WAY → 'Two-way'. No default branch — TypeScript exhaustiveness is enforced.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.format.sideVerb` (hash `4246e12d558b4873`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.grid.columnWindow

- **claim** (`cl\_4b477f443ac66e1e`): columnWindow computes a virtualised column render window from prefix-sum column edges. It returns the first/last rendered column indices (with one overscan column on each side by default), the total grid width, and padLeft/padRight pixel amounts for CSS padding. When viewport is 0 (pre-measure), the right boundary is set to the edge of the overscan-head columns so the grid paints enough columns to measure. The result satisfies \`padLeft + sum(widths\[start..end\]) + padRight == totalWidth\`.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.grid.columnWindow` (hash `8e40ad5dc1fd5eff`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.lib.grid.flattenGroups

- **claim** (`cl\_d395bcd25516f245`): flattenGroups is a generic variant of flatten for the DataGrid layer: it emits a group header (with key, label, and row count) for every RowGroup, then emits data rows for each group only if isCollapsed(model, g.key) is false. The group header always carries the count of total rows regardless of collapse state.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.grid.flattenGroups` (hash `609891e9b3b764b3`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.grid.moveRoving

- **claim** (`cl\_ede1087e1691e156`): moveRoving is a pure state reducer that moves the roving-focus cell within a grid. It maps RovingKey to a (nextRow, nextCol) displacement, then clamps: row is clamped to \[HEADER\_ROW, max(HEADER\_ROW, rowCount-1)\]; col is clamped to \[0, colCount-1\]. Ctrl+Home targets (0, 0) (row then clamped to HEADER\_ROW); Ctrl+End moves to (lastRow, lastCol). PageUp/PageDown step by pageSize (minimum 1). Returns identity if colCount \<= 0.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.grid.moveRoving` (hash `672e4c978f691d85`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.savedViews.decodeView

- **claim** (`cl\_6e72e7f9d0cef7b9`): decodeView is the inverse of encodeView: it parses a URLSearchParams (or query string) into a ViewState by delegating scope to decodeScopePath and guarding workspace/groupBy with type-narrowing predicates. Absent optional analytics keys (model/measures/axes/trend) produce an empty AnalyticsSelection, never undefined fields with placeholder values. An unrecognised workspace token defaults to "stream".
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.savedViews.decodeView` (hash `90263bbff29af003`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.lib.savedViews.encodeView

- **claim** (`cl\_b711b18a966b5366`): encodeView is the inverse of decodeView: it serialises a ViewState into a URLSearchParams by always writing the workspace key, conditionally writing scope (only if non-empty) and groupBy (only if not "none"), and conditionally writing each of the four analytics keys (model/measures/axes/trend) only when present. The zero-default-omission policy ensures round-trippable minimal query strings.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.savedViews.encodeView` (hash `ed498f7f857ebc28`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.lib.scope.decodeScopePath

- **claim** (`cl\_e653ad5b255865e9`): decodeScopePath parses a '\>'-separated token into a ScopeNode\[\] path always rooted at FIRM\_SCOPE\_ROOT. Each crumb must be exactly one level below the current tail (strict descending walk); any crumb that fails the isScopeLevel or childLevel check is silently dropped, making the parser tolerant of corrupt tokens while always returning a valid, rooted path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.scope.decodeScopePath` (hash `f1021d6bd5bd389a`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.scope.scopeReducer

- **claim** (`cl\_997ff732c78c17d6`): scopeReducer is a pure Redux-style reducer over ScopeState. drillDown appends a child level only when childLevel returns non-null (terminal pair level is a no-op). drillUp slices the path to the clamped depth and calls reconcileGroupBy to relax the groupBy axis. setGroupBy is idempotent (returns same state reference when groupBy is unchanged). reset always returns INITIAL\_SCOPE.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.scope.scopeReducer` (hash `df3683289d1b38aa`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.lib.trend.tenorForYears

- **claim** (`cl\_d353d35796e26c60`): tenorForYears converts a year fraction to the canonical Tenor struct using threshold-based bucketing: non-finite or \<= ONE\_BIZ\_DAY\_YEARS+EPS → OVERNIGHT/1; days \< 28-EPS → WEEKS/round(days/7); years \< 1-EPS → MONTHS/round(years\*12); otherwise YEARS/round(years). All counts are clamped to a minimum of 1 via Math.max.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.trend.tenorForYears` (hash `a48f09f0e548069b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.lib.trend.tenorLabel

- **claim** (`cl\_e7240c3e6f9b10b2`): tenorLabel formats a year fraction as a trader display string using FX market conventions: ON / TN / SN for the three overnight/tom-next/spot-next thresholds; nW for weeks (\< 28 days); nM for months clamped to 11 (\< 1 year); nY for whole years (within 0.02y); n.1Y for fractional years. Non-finite or non-positive input returns '—'.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.trend.tenorLabel` (hash `c1862e8fbd061a19`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.trend.tenorYearsOf

- **claim** (`cl\_82ab76aba27d7a9f`): tenorYearsOf converts every Tenor unit to a year fraction using exact or approximated calendar conventions: OVERNIGHT = ONE\_BIZ\_DAY\_YEARS; TOM\_NEXT = TN\_YEARS; SPOT\_NEXT = SN\_YEARS; WEEKS = count\*7/365; MONTHS = count/12; YEARS = count; IMM = Math.max(1,count)\*0.25 (display approximation — server resolves the true 3rd-Wednesday date); BROKEN\_DATE = 1/12 (approximation — server resolves the real date).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.trend.tenorYearsOf` (hash `a27b96c8c0ab4c38`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.lib.universe.searchUniverse

- **claim** (`cl\_fa64738f0762ea69`): searchUniverse applies the same two-stage fuzzy match as searchUnderliers but over Universe.all (CcyPair entries): haystack match gates inclusion, label match provides highlight indices (empty when the label doesn't match), sorted by score desc then label.localeCompare asc. When the query is empty all pairs are returned with score 0 and no highlight.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.lib.universe.searchUniverse` (hash `e5360c9392457732`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:37Z

## github.com-soarsa-celnet.gui.src.products.PayoffChart.describeShape

- **claim** (`cl\_e50753e351de4047`): describeShape classifies a payoff vector into exactly three shapes based on the min/max over the payoff array: all-non-positive (max≤0 and min\<0) → "capped-downside"; all-non-negative (min≥0 and max\>0) → "limited-downside"; otherwise → "two-sided". The direction prefix is "short" for SELL and "long" otherwise. The result is used as an aria-label.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.PayoffChart.describeShape` (hash `e8104b7cdaebd370`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.products.PayoffChart.payoffAtExpiry

- **claim** (`cl\_9b4d786d0903029c`): payoffAtExpiry(structureId, params, spotGrid) maps payoffAt over the entire spotGrid, returning null early if any spot returns null (non-terminal structure) or the structureId is absent from TERMINAL\_STRUCTURES.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.PayoffChart.payoffAtExpiry` (hash `74817afdce6e8db0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.products.basket.basketInceptionAggregate

- **claim** (`cl\_a808a3e962e40ac2`): basketInceptionAggregate computes the inception aggregate level for a basket: \`Σ(weight\_a \* spot\_a)\` for BASKET, \`max(weight\_a \* spot\_a)\` for BEST\_OF, \`min(weight\_a \* spot\_a)\` for WORST\_OF (0 for an empty leg list). This is the natural at-the-money strike default so the basket prices roughly at-the-money on first build.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.basket.basketInceptionAggregate` (hash `d15cfc815c2eb08e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.gui.src.products.capability.classForUnderlying

- **claim** (`cl\_c18ec35c304bfc62`): classForUnderlying is an exhaustive, pure switch from the five Underlying kind discriminants to their AssetClass string: 'fx' → 'FX', 'metal' → 'METAL', 'equity' → 'EQUITY', 'commodity' → 'COMMODITY', 'digitalAsset' → 'CRYPTO'. TypeScript exhaustiveness ensures any new Underlying kind produces a compile error.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.capability.classForUnderlying` (hash `7ae219c85b429a3b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.products.capability.galleryCardStates

- **claim** (`cl\_923835e7cfa4b47d`): galleryCardStates returns a Map\<id,CardState\> for the product gallery filtered to an AssetClass. FX and METAL pass all specs through as "available" (full FX engine, no dimming). For equity/commodity/crypto: a spec is "available" if its builder classes include the target class; "hidden" if a sibling spec of the same kind IS available for that class (prevents duplicate cards); otherwise the capability reason from priceability() is used, with "priceable" mapped to "hidden".
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.capability.galleryCardStates` (hash `3f82721fb81529fe`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetInputsFor

- **claim** (`cl\_d8d411f4105327ba`): crossAssetInputsFor maps a non-FX Underlying to a CrossAssetInputs overlay — FX always returns null; metal/equity/commodity always force settlementStyle to "LINEAR" regardless of the caller-supplied argument; digitalAsset (crypto) preserves the caller's settlementStyle. Returns null for FX underlyings.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetInputsFor` (hash `c5ad74cd610cabff`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetOverlayFor

- **claim** (`cl\_0eeb862e5efd913a`): crossAssetOverlayFor returns a CrossAssetOverlay (carrying the underlying + settlementStyle) only for equity, commodity, and digitalAsset underlyings; it returns undefined for fx and metal, because those route to the native FX engine and require no cross-asset overlay.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetOverlayFor` (hash `0ac49b5b700cef5f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetUnderlying

- **claim** (`cl\_3527b6340bae59d5`): crossAssetUnderlying is a total, pure bijection from CrossAssetInputs to an Underlying discriminated union: EQUITY → {kind:"equity", equity:{symbol:{ticker,venue},currency}}, COMMODITY → {kind:"commodity", commodity:{symbol:{ticker,venue},currency}}, CRYPTO → {kind:"digitalAsset", digitalAsset:{base,quote}}, METAL → {kind:"metal", metal:{metal,quote}}. All string fields are normalised via .trim().toUpperCase() before packing. It is the exact inverse of crossAssetInputsFor for the non-FX cases.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.crossAsset.crossAssetUnderlying` (hash `14f9b08f822a08c6`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.products.strategyLegEditor.legLawMessage

- **claim** (`cl\_4aaffed8190d3d66`): legLawMessage maps each LegLawViolation variant to a trader-facing English string. For LEG\_COUNT it computes the signed difference (actual - required) and directs the user to add or remove the exact number of legs. All other cases return fixed strings that name the violated constraint and how to fix it. The function is total over the LegLawViolation discriminant union.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.strategyLegEditor.legLawMessage` (hash `8f6d10458f72a2c0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.products.strategyLegEditor.legLawViolations

- **claim** (`cl\_c918eb7c68ce31f3`): legLawViolations checks strategy structural rules in order: (1) every leg must have ratio \> 0 (POSITIVE\_RATIO, per-leg); (2) leg count must match templateLegCount(template) — if not, returns early with only count + ratio violations; (3) pairwise laws are only applied once the count is correct. RISK\_REVERSAL requires opposite option types and opposite sides; STRANGLE/STRADDLE require opposite types and same side, with STRADDLE requiring a shared strike and STRANGLE requiring distinct strikes; SEAGULL requires at least one CALL and one PUT leg, and at least one BUY and one SELL leg.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.strategyLegEditor.legLawViolations` (hash `afbcb957a9651ddc`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.products.strategyLegEditor.parseStrikeEntry

- **claim** (`cl\_cb49de8da2163717`): parseStrikeEntry(raw, optionType) parses a trader strike input string into a typed StrikeEntry result. 'ATM' maps to a delta strike of ±0.5 (sign = optionType). A positive finite number becomes an absolute strike level. A delta notation 'NNc'/'NNp' (e.g. '25C') is accepted only when pct ∈ (0,100) and the letter matches optionType; the signed delta is +(pct/100) for calls and -(pct/100) for puts. Every invalid case returns ok:false with a discriminated error kind (EMPTY, NON\_POSITIVE\_LEVEL, UNRECOGNIZED, DELTA\_OUT\_OF\_RANGE, DELTA\_LETTER\_MISMATCH).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.strategyLegEditor.parseStrikeEntry` (hash `076500bccf693157`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.products.strategyLegEditor.strikeEntryMessage

- **claim** (`cl\_9e953ac52e3597ff`): strikeEntryMessage maps every StrikeEntryError variant to a precise, actionable trader-facing string. DELTA\_LETTER\_MISMATCH includes both the mismatched raw input and the correct suffix ('dC' or 'dP') for the actual leg option type, derived from e.optionType.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.strategyLegEditor.strikeEntryMessage` (hash `03a9f19e8fa81b32`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

## github.com-soarsa-celnet.gui.src.products.strategyLegEditor.templateName

- **claim** (`cl\_d6f43f29cf384d8c`): templateName maps every StrategyTemplate variant to its trader-facing display string: VANILLA → 'vanilla', RISK\_REVERSAL → 'risk reversal', STRANGLE → 'strangle', STRADDLE → 'straddle', SEAGULL → 'seagull'. Exhaustive switch — no default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.products.strategyLegEditor.templateName` (hash `79557541a223f6bd`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.workspaces.BookWorkspace.mergeVegaLadder

- **claim** (`cl\_20b4c905e8a49841`): mergeVegaLadder aggregates vega ladder entries across all RiskNodes into a single book-level ladder keyed by \`${tenorDays}\|${deltaBp}\`. Entries with the same pillar key are summed. The output is sorted ascending by tenorDays, with ties broken by descending \|vega\| (largest absolute exposure first).
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:40Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.BookWorkspace.mergeVegaLadder` (hash `4f63184dc4ebb221`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:40Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:40Z

## github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.metricByTenor

- **claim** (`cl\_7e2895ee0ddcaf2a`): metricByTenor derives ATM vol, 25-delta risk-reversal, and 25-delta butterfly per tenor from a calibrated CubeGrid using purely arithmetic recombination of the server's already-calibrated smile pillars: RR25 = call(+0.25d) - put(-0.25d); BF25 = 0.5\*(call+put) - ATM(-0.5d). Tenors are matched within +/-2/365 years; delta pillars within +/-1e-6. A null is returned for any tenor missing the required pillars. This is a read-only display derivation, not a re-calibration.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:42Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.metricByTenor` (hash `90c6ac29236abea0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:42Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:42Z

## github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.valueBand

- **claim** (`cl\_23c936576c9ebf8e`): valueBand scans a nullable number array and returns the {min,max} range, or null when the array contains no finite values. For a degenerate single-value array it manufactures a non-zero band: signed values get {-\|v\|‖-1e-6, \|v\|‖1e-6}, unsigned get {v, v+1e-6}, so consumers always have a valid gradient range.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.valueBand` (hash `4f8eda8620a0293e`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.volBand

- **claim** (`cl\_3f7b8aee79d33f05`): volBand scans all cells of a CubeGrid and returns the strict {min,max} vol range, returning null when the grid is null, has no finite values, or all cells share the same vol (max \> min required). It does not manufacture a synthetic band for the degenerate case.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:35Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.CubeWorkspace.volBand` (hash `068e86cc58aeab6b`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:35Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:35Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.buildGroups

- **claim** (`cl\_a53aad3a44e82946`): buildGroups partitions StreamRows by the GroupBy key (via groupKeyOf), preserving first-seen stream order across groups so the desk taxonomy is stable under live ticks. Within each group, rows are sorted by the active SortState (ascending or descending by rowSortValue) only when a sort is active; otherwise group-row order is preserved. Each group carries an aggregate computed over its original (pre-sort) rows.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.buildGroups` (hash `d4e89bc02863f002`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.flatten

- **claim** (`cl\_493970318c4c3ff2`): flatten (StreamWorkspace) produces a single FlatItem\[\] list from groups and a collapsed set: every group always contributes a header item; a group's row items are included only if its id is not in the collapsed set. This is the input to the single virtualiser that windows the whole stream workspace.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:38Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.flatten` (hash `93392904b80ade4c`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:38Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.fmtTrendValue

- **claim** (`cl\_bef05d65080d5dbc`): fmtTrendValue dispatches formatting by TrendUnit: "premium" → fmtPremiumPct, "vol" → fmtVolPoint, "rate" → fmtRate, "vega"/"pnl" → value.toFixed(2). Each TrendUnit maps to exactly one formatter with no shared default.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:37Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.fmtTrendValue` (hash `27f734301924f54d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:37Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:38Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.rowSortValue

- **claim** (`cl\_5fc5cea788a66eb8`): rowSortValue extracts a single numeric scalar from a StreamRow for three mutually-exclusive sort keys: 'mid' returns (bid + offer) / 2, 'delta' returns greeks.deltaSpot, and 'sigma' returns row.vol. The switch is exhaustive over the SortKey union — no default branch exists.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.rowSortValue` (hash `b31b996f0a61a0a0`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.tenorLabel

- **claim** (`cl\_ee1b9a94750a6714`): StreamWorkspace.tenorLabel formats a StreamRow's Tenor struct to a display string, returning 'PERP' for perpetual options (no tenor). For other units it formats OVERNIGHT→ON, TOM\_NEXT→TN, SPOT\_NEXT→SN, WEEKS→nW, MONTHS→nM, YEARS→nY, IMM→nIMM, BROKEN\_DATE→ISO-date string (YYYY-MM-DD with zero-padded month/day) or 'broken' when brokenDate is absent.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:36Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.tenorLabel` (hash `1c50ed71d3746506`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:36Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:36Z

## github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.trendUnitCaption

- **claim** (`cl\_bdd65250e79a9bcc`): trendUnitCaption is a pure total function from TrendMode to a human-readable unit string, covering all 8 modes: PREMIUM→'premium', ATM\_VOL→'ATM vol', RR→'25Δ RR', BF→'25Δ BF', SPOT→'spot', FORWARD→'fwd', VEGA→'vega', PNL→'P&L'. The switch is exhaustive with no default fallback.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:41Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.StreamWorkspace.trendUnitCaption` (hash `a6c3f67e50a627af`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:41Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:41Z

## github.com-soarsa-celnet.gui.src.workspaces.TicketWorkspace.previewStrike

- **claim** (`cl\_fd84a5d66a84a947`): previewStrike(inputs, atmForward) extracts the primary chart kink strike from type-erased instrument inputs: it first looks for a top-level numeric 'strike' field \> 0, then walks the 'legs' array for the first leg with a 'strike.kind===\\"strike\\"' entry with a positive numeric value, and falls back to atmForward if neither is found.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:39Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.src.workspaces.TicketWorkspace.previewStrike` (hash `5ee8560ff8b5a24f`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:39Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:39Z

## github.com-soarsa-celnet.gui.test.scope.test.refReducer

- **claim** (`cl\_5e9ae70ce6ae42dc`): refReducer(state, action) is a reference implementation of the scope reducer used in tests. It is structurally independent of the production reducer: 'reset' returns a fixed firm-root path; 'setGroupBy' returns unchanged path with new groupBy; 'drillDown' appends the next LADDER level if not already at the terminal; 'drillUp' clamps depth to \[1, path.length\] and applies refRelax to the resulting shorter path.
- **kind**: invariant:pure · **state**: active · **confidence**: 
- **author**: celnet-knowledge · **created**: 2026-06-29T23:30:43Z
- **anchors**:
  - `github.com-soarsa-celnet.gui.test.scope.test.refReducer` (hash `5b857790e6ee414d`, resolved)
- **provenance**:
  - (none) → draft by agent (authored) @ 2026-06-29T23:30:43Z
  - draft → active by agent (stage-1 gate passed) @ 2026-06-29T23:30:43Z

