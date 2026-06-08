# W3 — Crypto / digital-asset options: staged execution plan

> The single biggest "exceed SynOption" move (matches the Synchro/OrBit asset-class reach):
> add digital-asset options as a **new leaf** on the W1 multi-asset seam — `Underlying::DigitalAsset`
> + a 24×7 calendar + the inverse/coin-settled payoff + funding carry + a crypto surface leaf.
> Governed by ADR-0008 (identity / Carry-as-forward-discount-producer / asset-class-AGNOSTIC payoff)
> and MASTER-EVOLUTION-PROGRAM §1/§5 (W3, P1, XL). The single backlog truth stays
> `docs/WORLD-CLASS-BACKLOG.md` (`[W3] crypto/digital-asset-class`); the per-product gate set is
> `docs/VERIFICATION-CONTRACT.md`. **ASSUMES the W1 contract is frozen** (Underlying/CarryModel/
> RateSensitivities on the wire; the `CarryPricer` trait in `celnet-core`; `celnet-vanilla` the FX
> leaf). This is the execution plan; it adds nothing to the proto/clients until W1 lands.

## 0. The crux & the governing principle

The crux of crypto is **not** the linear-settled (USD/USDT-margined) contract — that is plain
generalized-BSM and falls straight out of W1's `Carry::CostOfCarry { r, b }` with `b = r − funding`.
The crux is the **inverse / coin-margined** contract (the classic Deribit BTC/ETH option): the
premium, the payoff, and the P&L are all denominated in the **base coin**, not in USD. An inverse
option on `S` (USD-per-coin) with USD-strike `K` pays, per contract, `max(φ(S_T − K), 0) / S_T`
coins. That `1/S_T` factor makes the payoff a **non-linear function of the terminal price** with a
genuine **convexity/measure** subtlety — it is *not* a rescaled vanilla, and getting it wrong is the
single most likely place to ship a silent mis-price. This plan is precise about the measure below
and gates it with a code-disjoint MC plus an independent closed form, not a re-derivation of the
production algebra (the FRTB-0.75ρ circular-oracle lesson).

**Governing principle (ADR-0008, binding):** the crypto leaf is a new `Carry`/`MarketState` builder
+ a new payoff leaf reachable through `forward()`/`discount()`; **no payoff engine branches on the
`Carry` variant.** The linear-settled crypto vanilla reuses the *agnostic* generalized-BSM payoff
math unchanged (only the carry assembly `b = r − funding` differs). The inverse-settled contract is
a genuinely **new payoff shape** (the coin-denomination transform), so it gets its own pricer in the
new leaf — it does **not** smuggle a `match carry { … }` into the FX/agnostic engine. FX stays
byte-identical (this wave touches no FX arithmetic).

## 1. New crate: `celnet-crypto-vanilla`

A new leaf crate, sibling to `celnet-vanilla` (the FX leaf). It implements the `CarryPricer` trait
from `celnet-core` for `Underlying::DigitalAsset`, the way `FxPricer` does for `Underlying::Fx`.

- **Auto-join:** new crates auto-join the workspace via `members = ["crates/*"]` in the root
  `Cargo.toml`. It MUST also be registered in root `[workspace.dependencies]`
  (`celnet-crypto-vanilla = { path = "crates/celnet-crypto-vanilla" }`), and every internal dep it
  takes (`celnet-core`, `celnet-types`, `celnet-conventions`, `celnet-calendar`, `celnet-qmc` for
  the MC oracle path if any production MC is needed) is referenced as `.workspace = true` — the
  W0 path-dep lint (no internal path-dep outside the registry) rejects an ad-hoc `path = …`.
- **Deps (one-way, acyclic):** `{celnet-core, celnet-types, celnet-conventions, celnet-calendar}`.
  No dep on `celnet-vanilla` (the leaves are siblings, not a chain). The closed-form-vs-MC oracle
  lives in `celnet-parity` (dev-dep), not here.

```
crates/celnet-crypto-vanilla/
  Cargo.toml
  src/lib.rs          # crate doc + re-exports + the CryptoPricer (CarryPricer impl)
  src/linear.rs       # linear/USD(T)-margined vanilla: agnostic generalized-BSM via Carry
  src/inverse.rs      # inverse/coin-margined vanilla: the 1/S_T-denominated payoff + closed form
  src/funding.rs      # funding-carry assembly: b = r − funding → Carry::CostOfCarry { r, b }
  src/settlement.rs    # SettlementStyle (Linear | InverseCoin) routing within the leaf
```

`CryptoPricer` (the `CarryPricer` impl) reads the `Underlying::DigitalAsset` settlement style and
routes to `linear.rs` (calls the shared agnostic generalized-BSM via `inputs.forward()` /
`inputs.discount_df()` — zero new payoff math) or `inverse.rs` (the coin-denominated closed form).
A non-crypto `Underlying` or an FX `Carry` is rejected with the existing `CarryPriceError`
(`UnsupportedUnderlying` / `UnsupportedCarry`) — never a silent fallback.

## 2. `Underlying::DigitalAsset` + settlement style (types / proto — additive)

W1 makes `Underlying` an additive enum and `CarryModel` an oneof. W3 adds **one arm each**, FX
default preserved (no existing ticket/`CELNET.PRICE`/`celnet price EURUSD` changes):

- **`celnet-types`:** `Underlying::DigitalAsset(CryptoPair)` where `CryptoPair { base: Symbol,
  quote: Symbol, settlement: SettlementStyle }`. `Symbol` (the length-validated newtype W1 lands
  for non-3-char symbols) names `BTC`/`ETH`/`USDT`/`USDC`. `SettlementStyle { Linear, InverseCoin }`
  (deliverable/cash@fixing already exist for FX; this is the new coin-vs-cash axis). `as_fx()` stays
  `None` for the crypto arm; add `as_crypto()`.
- **`celnet-proto` (one unversioned contract, no `schema_version`):** add `CryptoPair crypto` to the
  W1 `Underlying.oneof ref { CcyPair fx = 1; CryptoPair crypto = 2; }` with a `SettlementStyle`
  enum (prefix `SETTLEMENT_STYLE_`, `…_LINEAR`/`…_INVERSE_COIN`, `…_UNSPECIFIED = 0` per protobuf
  enum-zero hygiene). The funding carry rides the **existing** W1 `CarryModel.CostOfCarry { r, b }`
  arm — **no new CarryModel arm** (funding is just how `b` is assembled, ADR-0008 §Decision-2). Use
  the `protobuf` skill for enum-prefix/field-number hygiene; gate with a convert round-trip + a
  `to_bits` byte-identity check for the FX projection (unchanged).
- The product oneof is **unchanged** — crypto vanilla is the existing `vanilla` arm under a crypto
  `Underlying` (ADR-0008: most arms become underlying-agnostic once Underlying+CarryModel
  generalize). No new product SHAPE is introduced by W3.

## 3. 24×7 calendar (`celnet-calendar`)

Crypto trades and settles 24×7 — no weekend skip, no holiday centres. Today `celnet-calendar`
hardcodes a Sat/Sun weekend (`weekday::is_weekend`) and centre-holiday logic. Add a **trading-clock
abstraction** rather than special-casing crypto inside the FX calendar:

- Add `TradingClock { BusinessDay(BusinessCalendar), Continuous }` (or an enum the resolver
  dispatches on). The `Continuous` arm: every day is a "business day"; expiry/year-fraction is the
  plain calendar-day (or actual/365-style) difference with **no roll, no weekend skip** — crypto
  expiries are exchange-fixed UTC instants (e.g. Deribit 08:00 UTC), so the day-count is continuous.
- `vol_year_fraction` / expiry resolution take the clock; FX keeps `BusinessDay` (byte-identical,
  the existing path). Keep the FX `is_weekend`/holiday path untouched; the crypto path simply never
  consults it. Provenance + the honest note that *exact UTC fixing instants* are exchange-data (ENV)
  go in the module doc, not in identifiers.

## 4. The two settlement payoffs (the precise part)

Let `S` = spot (USD per 1 coin), `K` = USD strike, `F = S·e^{b·t}` the outright forward with
`b = r − funding`, `df = e^{−r·t}` the USD discount factor, `σ`, `t` as usual, `φ = +1` call /
`−1` put. `N` is the standard normal CDF, `d₁ = (ln(F/K) + ½σ²t)/(σ√t)`, `d₂ = d₁ − σ√t`.

### 4a. Linear / USD(T)-margined — the agnostic generalized-BSM path

Premium and payoff are in USD(T): `payoff = max(φ(S_T − K), 0)` USD. This is **exactly**
generalized-BSM:
`V_lin = φ·df·(F·N(φd₁) − K·N(φd₂))`.
No new math — the leaf assembles `Carry::CostOfCarry { r, b = r − funding }` and calls the shared
**asset-class-agnostic** vanilla engine through `forward()`/`discount_df()`. (ADR-0008 §Decision-3.)

### 4b. Inverse / coin-margined — the new payoff shape (be precise about the measure)

The inverse contract pays, per USD-notional-1, `max(φ(S_T − K), 0) / S_T` **coins**, and its
premium is quoted in **coins**. Its USD value is the linear value, but the *coin* premium requires
discounting the coin payoff under the correct measure. The clean, non-circular derivation:

- The coin payoff `max(φ(S_T − K),0)/S_T` is the USD payoff divided by `S_T`. Under the USD
  risk-neutral measure `Q`, the **coin price of the option** is
  `V_coin = E^Q[ df · max(φ(S_T−K),0)/S_T ] / 1` (coins), i.e. the discounted USD-measure
  expectation of the `1/S_T`-weighted payoff. Changing numeraire from USD to the coin (Radon-Nikodym
  `dQ_coin/dQ = e^{(r−b)t}·S_T/S_0` — the standard "quanto into the base asset" / share-measure
  change) turns the `1/S_T` factor into a measure with **shifted drift**, giving a clean
  closed form. Concretely, with the share/coin measure the strike and forward roles swap and:
  `V_coin = (φ/S_0)·e^{−b·t}·( N(φ d₁) − (K/F)·N(φ d₂) )` coins   *(per USD-notional-1)*,
  equivalently the USD value `V_usd_of_inverse = S_T-weighted` reconciles to the linear price only
  in the degenerate `K→0` / deterministic-`S_T` limits — the convexity term `Cov(1/S_T, payoff)` is
  **non-zero and material**, which is exactly why a naive `V_lin / S_0` is WRONG.
  *(The exact constant assembly — which of `e^{−b t}`, `e^{−r t}`, `1/S_0`, `1/F` multiplies which
  CDF — is derived in `inverse.rs` from the numeraire change with the algebra in the module doc; the
  parity oracle below deliberately does NOT reuse that assembly so it can disagree.)*
- **Implementation:** `inverse.rs` computes `V_coin` from the closed form; the `CarryGreeks` strip's
  `price` is in **coins** (the contract's natural unit), with the USD-equivalent exposed as a
  derived field the way premium-style is already carried. Delta/gamma/vega are the coin-measure
  sensitivities (a coin-margined desk hedges in coins) — documented, not silently the USD ones.

This `1/S_T` non-linearity is the documented crux; §6 oracles gate it three independent ways.

## 5. Crypto surface leaf (`celnet-surface`)

Crypto smiles are quoted on a **strike / log-moneyness** axis (exchange option chains by strike),
NOT FX risk-reversal/butterfly delta pillars. W1 splits the surface input into
`oneof quotes { BrokerQuoteSet fx_broker; StrikeVolGrid strike_axis; MoneynessVolSlice
log_moneyness; }` and `SmilePoint.axis { delta; strike; log_moneyness; }`. W3 only needs to:

- Add a **strike/log-moneyness smile leaf** under the W1 surface core (`celnet-surface`'s neutral
  arb-free interpolator + butterfly/calendar gates are reused unchanged — they operate on `(k, w)`
  total-variance points). The crypto leaf maps the exchange strike grid into total-variance and
  feeds the same no-arb projection. No FX RR/BF conversion is involved (and `Conventions` is
  optional for the crypto underlying per W1).
- Reuse the existing SVI/SSVI/eSSVI parametric fits (they are strike-space-native) for the crypto
  smile; the FX delta-axis machinery is simply not on the crypto path.

No new surface crate — this is a leaf inside `celnet-surface`, matching ADR-0008's "surface splits
into neutral core + per-axis leaves."

## 6. INDEPENDENT oracles (per the verification contract; circular-risk flagged)

| Product | Independent oracle (method / paper) | Circular-oracle risk & mitigation |
|---------|--------------------------------------|------------------------------------|
| **Crypto linear vanilla** | (1) **GK-with-funding closed form** — the production generalized-BSM is *itself* GK with `r_dom→r`, `r_for→r−b`; so the parity oracle must NOT be the same algebra. Use the **`funding→r_for` identity**: a linear crypto vanilla with `(r, b=r−q)` is bit-identical to the FX `celnet-vanilla::price` called with `r_dom=r, r_for=q` (the existing, independently-golden FX leaf). (2) **Put-call parity** `C − P = df·(F − K)`. (3) **`funding=r ⇒ b=0` driftless limit** vs Black-76 forward price. | MEDIUM. Routing the oracle through the *already-QuantLib-golden* FX leaf with the rate identity is genuinely disjoint from the crypto leaf's own carry assembly (different code path, frozen reference). Parity gate is `to_bits` against the FX leaf, not "approximately equal". |
| **Crypto inverse vanilla** | (1) **Independent closed form** re-derived in the test from the **numeraire change** (coin measure), NOT copied from `inverse.rs` — written as `V_coin = expectation form` evaluated by Gauss-Hermite quadrature of `df·max(φ(S_T−K),0)/S_T` against the lognormal density (a quadrature, not the production CDF assembly). (2) **Code-disjoint splitmix64 MC**: simulate `S_T = S_0·exp((b−½σ²)t + σ√t·Z)` with an in-test splitmix64 RNG + Box-Muller, average `df·max(φ(S_T−K),0)/S_T`, gate within the **reported MC standard error** (never to closed-form precision — VERIFICATION-CONTRACT (a)/(b)). (3) **Limits**: `K→0` inverse call value → `df·e^{(b−r)t}·(1) = e^{−b t}/S_0`-style coin forward identity; deep-ITM/OTM monotonicity; `V_inverse·S_0 ≠ V_linear` strictly (the convexity sandwich — proves the `1/S_T` term is present, catches the naive-rescale bug). | **HIGH — this is the FRTB-0.75ρ-class trap.** The quadrature oracle and the production closed form could both encode the **same wrong measure** (e.g. both forget the share-measure drift shift). Mitigation: the **splitmix64 MC** is the disagree-capable oracle — it simulates the raw payoff `max/S_T` with zero analytic structure, so a measure error in *both* closed forms surfaces as an MC mismatch. PLUS the **structural sandwich** `V_inverse·S_0 > V_linear` for a convex payoff (Jensen on `1/S_T`) is a qualitative gate that fails loudly on a naive rescale. Require ALL THREE (MC + quadrature + sandwich) to agree before "done". |
| **Conventions (contract specs)** | **Published Deribit contract specifications** — contract unit, settlement coin, UTC expiry cut (08:00), index-price fixing definition, tick — hand-pinned in the test with the citation (VERIFICATION-CONTRACT (a) class 3, "published reference"). | LOW. Conventions are identity/structure (which coin, which cut), re-derived from the primary spec text, not computed. Live fixing VALUES are ENV (§9). |

**Anti-circular discipline (binding):** for the inverse contract, the parity test MUST contain at
least one oracle that *can disagree* with the closed form — the splitmix64 MC and the
`V_inverse·S_0 > V_linear` sandwich both satisfy this. Do not let the quadrature oracle alone gate
it (it shares the lognormal-measure assumption with production). Document the measure derivation in
`inverse.rs` AND independently in the parity test header, and pin the Deribit constants from the
spec text.

## 7. Staged GREEN increments (single driver; each commit `just check`-green & gated)

- **S0 (hygiene):** register `celnet-crypto-vanilla` in root `[workspace.dependencies]`; confirm the
  W0 path-dep lint + verification-coverage lint are green before adding code. Commit.
  *Gate:* `just check` green (no new code yet); lints pass.
- **S1 (types — additive vocabulary):** add `Underlying::DigitalAsset(CryptoPair)`, `CryptoPair`,
  `Symbol` (if not already landed by W1), `SettlementStyle` to `celnet-types`; `as_crypto()`; unit
  tests incl. FX `to_bits` round-trip unchanged. *Gate:* `just check-crate celnet-types` + FX
  byte-identity test green. Commit.
- **S2 (24×7 calendar):** add `TradingClock`/`Continuous` to `celnet-calendar`; crypto expiry +
  year-fraction with no weekend/holiday; FX `BusinessDay` path byte-identical. *Gate:*
  `just check-crate celnet-calendar`; new continuous-clock unit tests + FX-unchanged tests. Commit.
- **S3 (crypto leaf — linear first):** `celnet-crypto-vanilla` with `funding.rs` + `linear.rs` +
  the `CarryPricer` impl routing linear via the agnostic generalized-BSM. *Gate:*
  `just check-crate celnet-crypto-vanilla`; in-crate unit tests (put-call parity, funding→FX
  identity). Commit.
- **S4 (crypto leaf — inverse):** add `inverse.rs` + `settlement.rs` routing. *Gate:*
  `just check-crate celnet-crypto-vanilla`; in-crate inverse limits + the `V_inverse·S_0 > V_linear`
  sandwich. Commit.
- **S5 (parity rows — the verification contract):** add `crates/celnet-parity/tests/crypto.rs`
  (linear: FX-leaf `to_bits` identity + parity; inverse: splitmix64 MC within stderr + Gauss-Hermite
  quadrature + structural sandwich + Deribit-spec convention pins). Add the golden vectors
  `crates/celnet-golden/vectors/crypto_vanilla_linear.json` and `…_inverse.json` (engine-generated,
  each value cross-checked by the parity oracle; the inverse family carries a positive
  `price_std_error`). Add the family→test map entries to `tools/check-verification-coverage.mjs`.
  *Gate:* `just verification-coverage` green; `cargo nextest -p celnet-parity` green.
- **S6 (crypto surface leaf):** strike/log-moneyness smile leaf in `celnet-surface` over the W1
  surface core; SVI/SSVI fit reused. *Gate:* `just check-crate celnet-surface`; a crypto-smile
  no-arb (butterfly/calendar) parity row reusing the existing density/calendar checks.
- **S7 (proto + convert):** add `CryptoPair`/`SettlementStyle` to the W1 `Underlying` oneof; codec
  round-trip; **FX projection `to_bits`-identical** (no-regression gate). `protobuf` skill for enum
  hygiene. *Gate:* `just check-crate celnet-proto` + convert round-trip + W0 conformance corpus
  green on the FX set. Commit.
- **S8 (server routing + validity matrix):** route `Underlying::DigitalAsset` to the crypto leaf in
  `celnet-server`; extend the product×underlying validity matrix (crypto vanilla ✅; products not yet
  crypto-validated ⇒ typed `INVALID_ARGUMENT`, never silent fallback). *Gate:* server unit tests +
  an invalid-combo rejection test. Commit.
- **S9 (5-client surfacing — FX default preserved):** §8. *Gate:* GUI vitest + Playwright real-edge
  e2e + Excel real-edge e2e + SDK/CLI e2e all green; `CLIENT-PARITY-MATRIX.md` regenerated from the
  passing harness with the new crypto rows.
- **S10 (milestone):** full-workspace `just check` prints the literal **"All gates passed."**
  (verified, not the wrapper exit code); re-index codebase-memory; reconcile INTERFACES/ARCHITECTURE/
  CONVENTIONS/ANALYTICS-SPEC docs (crypto asset class Built, no stale "FX-only"). Branch-first
  milestone commit + push to `origin`.

## 8. Cross-client surfacing (api-first parity; FX stays the default everywhere)

- **SDK (`celnet-client`):** `Underlying::crypto("BTC","USDT", SettlementStyle::InverseCoin)` +
  `CarryModel::funding(r, funding)` (assembles `b = r − funding`); `InstrumentSpec::vanilla(...)`
  takes the generalized `Underlying` (FX-default constructors unchanged → `celnet price EURUSD`
  source-compatible in spirit). One runnable crypto example gated vs a real edge.
- **CLI (`celnet-cli`):** the W1 `--underlying`/`--asset-class` flag — `celnet price --crypto
  BTCUSDT --settlement inverse-coin --funding …`; default FX-pair parsing preserves
  `celnet price EURUSD …`.
- **Excel:** the W1 polymorphic `CELNET.PRICE(underlying, product, terms-range)` with `underlying`
  naming a crypto pair + settlement; the convention-transparency footer states the settlement coin
  and UTC cut. Asserted via the W0 **real-edge** Excel conformance suite (not the in-process
  `FakeSocket`).
- **GUI:** the W1 asset-class-aware `UniverseNavigator` gains a `DigitalAsset` bucket (BTC/ETH/…);
  the contract-derived Structuring workspace prices a crypto vanilla with zero bespoke form code;
  the surface-family switch binds to strike/log-moneyness for the crypto underlying.
- **api-first parity gate:** ≥1 crypto vanilla (one linear, one inverse) REACHABLE and identical
  from server == SDK == CLI == Excel == GUI against the frozen golden vectors (VERIFICATION-CONTRACT
  (c)/(d)).

## 9. Deploy-bound (ENV) carve-out — designed + seamed + ADR'd, never claimed in-repo

Per VERIFICATION-CONTRACT (g) and MASTER-EVOLUTION-PROGRAM §0 honest boundary, the following are
**ENV** — seamed in-repo (resilient-subscriber / fixing-source / convention identity), validated at
deploy, never blocking or claimed in-repo:

- **Live crypto exchange connectivity** (Deribit/OKX/Binance venue sessions, order entry).
- **Live crypto vol surfaces / option-chain market data** (only the surface *math* and the
  strike-axis ingestion *shape* are in-repo; live quote VALUES are ENV).
- **Live index-price fixings / settlement fixing VALUES** (only the fixing *identity* + convention,
  re-pinned from the Deribit spec, are in-repo).
- **Funding-rate live feed VALUES** (the carry *assembly* `b = r − funding` and its sensitivities
  are in-repo and gated; the live perpetual funding number is ENV).

Each carries the verbatim honest-boundary statement in the crate/module docs and the parity test
header naming what is proven here (payoff math, convention identity, carry assembly, measure
correctness) vs deferred (live values, venue lifecycle).

## 10. Out of W3 scope (later waves)

Perpetual / funding-settled crypto *structures* and option-on-listed-future are W4 payoff shapes;
crypto in the cross-asset risk cube / FRTB buckets is W5; 2nd-gen crypto exotics depth is W6. W3
delivers the asset-class *leaf* (linear + inverse vanilla + 24×7 + crypto surface) so those are
additive, not rewrites.
