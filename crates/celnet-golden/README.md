# celnet-golden

Frozen QuantLib reference validation — the **numerical gate** for Celnet pricers.

This crate carries committed CSV reference tables of FX-option prices and Greeks
computed by [QuantLib](https://www.quantlib.org/) (an open-source, BSD-style
licensed analytics library used purely as an *offline oracle* — never a runtime
dependency) and a Rust test suite that asserts the Celnet pricers reproduce those
numbers to documented tolerances.

The committed CSV is the oracle; the test is the gate. The tables are **frozen**:
they are regenerated only deliberately, so any drift in a Celnet pricer shows up
as a failing test rather than a silently-mutated reference.

## Pinned oracle

- **QuantLib 1.42.1**, via the Python bindings in the venv at `~/.celnet-goldenv`.
- Engine: `GarmanKohlagenProcess` + `AnalyticEuropeanEngine` (vanilla),
  `AnalyticBarrierEngine` (single barrier), `CashOrNothingPayoff` /
  `AssetOrNothingPayoff` via `AnalyticEuropeanEngine` (digitals),
  `AnalyticDoubleBarrierBinaryEngine` (touch / no-touch / DNT / double-touch —
  single touches via the wide-corridor limit), `AnalyticDoubleBarrierEngine`
  (double knock-out, with knock-in derived as `vanilla − KO`).
- Frozen evaluation context: eval date 2026-06-15, Actual/365-Fixed day count,
  `NullCalendar`. Maturities are snapped to an integral number of Act/365F days
  so the year fraction QuantLib sees equals the `t` written to the CSV.

## Data tables (`data/`)

| File | Rows | Contents |
|------|------|----------|
| `vanilla_gk.csv` | 3200 | price + spot delta, gamma, vega, theta, rho_dom, rho_for over spot × moneyness × vol × maturity × rate-pair × {call,put} |
| `barrier_gk.csv` | 288 | analytic single-barrier prices over all eight flavours (down/up × in/out × call/put), zero rebate |
| `digital_gk.csv` | 192 | analytic digital prices over both styles (cash-/asset-or-nothing) × call/put |
| `touch_gk.csv` | 1224 | one-touch / no-touch / double-no-touch / double-touch (at-expiry rebate) over corridor/barrier × vol × maturity × rate-pair |
| `double_barrier_gk.csv` | 432 | double-barrier knock-out (and knock-in complement) of a vanilla over corridor × strike × vol × maturity × rate-pair × {call,put} |
| `heston_fo.csv` | 4 | Heston stochastic-vol European-vanilla **published** reference prices (Fang–Oosterlee 2008 §5.3, Tables 4/5), `T ∈ {1, 10}` × {call, put} |

The first five tables are QuantLib-sourced. The sixth (`heston_fo.csv`) is
**hand-pinned from the published literature**, because QuantLib is not available
in this build environment (see "Heston: published-literature oracle" below) —
narrowed honestly to authoritative published constants rather than a fabricated
oracle.

All six tables are actively gated against the Celnet pricers:

- `vanilla_gk.csv` → `celnet-vanilla` (`tests/vanilla_grid.rs`).
- `barrier_gk.csv` → `celnet-exotics::single_barrier_price` (`tests/barrier_grid.rs`).
  This is the *independent* oracle the exotics crate's own in/out-parity test
  cannot be: `KI + KO = vanilla` holds by construction in the Celnet code (KO is
  defined as `vanilla − KI`), so parity cannot catch a wrong knock-in block
  selection — QuantLib, computing every flavour independently, can.
- `digital_gk.csv` → `celnet-exotics::digital_price` (`tests/digital_grid.rs`),
  for both settlement styles. The Celnet pricer returns the value per one payout
  unit, so the oracle row is scaled by its `payout`.
- `touch_gk.csv` → `celnet-exotics::{one_touch_price, no_touch_price,
  double_no_touch_price, double_touch_price}` (`tests/touch_grid.rs`). The
  *independent* oracle the exotics crate's own complementarity checks cannot be:
  `no_touch = df − one_touch` and `double_touch = df − dnt` hold by construction,
  so they cannot catch a common-mode survival-series error — QuantLib's separate
  double-barrier-binary reflection series can. Single touches are the
  wide-corridor limit (one wall pushed ≳ 12σ√T away).
- `double_barrier_gk.csv` → `celnet-exotics::double_knock_out_price`
  (`tests/double_barrier_grid.rs`), KO directly and KI as `vanilla − KO`. The
  independent oracle for the Ikeda-Kunitomo image series (it caught a put-leg sign
  bug that the in-crate call-only Monte-Carlo check missed).

- `heston_fo.csv` → `celnet-heston::{carr_madan, cos}` (`tests/heston_grid.rs`).
  See below.

Each table additionally carries a structural self-check (well-formed, finite,
sane bounds) in `src/table.rs`.

### Heston: published-literature oracle (no QuantLib in this environment)

QuantLib (whose `AnalyticHestonEngine` would be the natural oracle) is **not
available** in this build environment, so the Heston golden is *narrowed
honestly*: rather than fabricate an oracle, it pins authoritative **published**
reference prices from

> Fang & Oosterlee (2008), *A Novel Pricing Method for European Options Based on
> Fourier-Cosine Series Expansions*, SIAM J. Sci. Comput. 31(2):826–848 (MPRA
> preprint 8914), §5.3.

The pinned figures are the paper's own **"Reference val."** entries from its
Tables 4 and 5 — produced by the **Carr-Madan method at `N = 2^17`**, an oracle
independent of Celnet:

- `T = 1`:  `5.785155450…`
- `T = 10`: `22.318945791…`

over eq. (53) `S0 = K = 100, r = q = 0, κ = 1.5768, σ = 0.5751, θ = 0.0398,
v0 = 0.0175, ρ = −0.5711`. Put rows are the published call under exact put-call
parity (`r = q = 0, S0 = K` ⇒ parity term is `0`, so put `=` call).

This is the *independent external* oracle the `celnet-heston` crate's own
Carr-Madan-vs-COS cross-check cannot be: two transforms of one mis-derived
characteristic function could agree on a wrong value, but a third-party published
price cannot. **Carr-Madan** is gated on every row (it reproduces `T = 10` to
~`1.5e-10`); **COS** is gated only inside its documented `≤3y` validity (the
`cos_valid` column), and a negative-control test pins that COS legitimately
diverges past the Fourier precision wall at `T = 10` so the boundary cannot
silently move. Tolerance `abs/rel = 1e-6`, set to the published precision (nine
decimals), met with ~2 orders of margin.

### QuantLib → Celnet convention mapping (verified identical units & signs)

| QuantLib | Celnet | Definition |
|----------|--------|------------|
| `NPV` | `price` | domestic premium per 1 base notional |
| `delta` | `delta_spot` | `e^{-r_f T} N(d1)` |
| `gamma` | `gamma` | `∂²V/∂S²` |
| `vega` | `vega` | `∂V/∂σ` (per 1.0 absolute vol) |
| `theta` | `theta` | `∂V/∂t` per year (= `−∂V/∂T`) |
| `rho` | `rho_dom` | `∂V/∂r_dom` |
| `dividendRho` | `rho_for` | `∂V/∂r_for` |

Note QuantLib's `GarmanKohlagenProcess(spot, foreignTS, domesticTS, volTS)` takes
the **foreign** (dividend) curve before the **domestic** (risk-free) curve.

## Tolerances

Closeness uses `celnet_core::is_close(rel, abs)` — agreement within *either* leg.
Well-conditioned quantities (price, delta, both rhos) are held to ~`1e-10`–`1e-11`
relative. Gamma/vega/theta additionally carry an absolute floor for the deep-OTM
short-dated corner of the grid where both libraries return sub-`1e-12` noise and a
relative test would be meaningless. The exact pairs are in
`tests/vanilla_grid.rs`.

## Regeneration procedure

The CSVs are **committed data**; regenerate only on a deliberate oracle change
(e.g. a QuantLib bump or a grid extension), then review the diff:

```bash
# 1. (one-time) create the QuantLib venv if it does not exist:
python3 -m venv ~/.celnet-goldenv && ~/.celnet-goldenv/bin/pip install QuantLib

# 2. regenerate the frozen tables (run from the repo root):
~/.celnet-goldenv/bin/python tools/goldgen/generate.py

# 3. re-run the gate:
just test   # or: cargo nextest run -p celnet-golden
```

The generator lives at `tools/goldgen/generate.py` (repo-root `tools/`, not inside
a crate). It prints the QuantLib version and per-table row counts; the version is
also stamped into each CSV header.
