# ADR-0008 — Multi-asset architecture: identity / carry-producing market / agnostic payoff

- **Status:** Accepted (2026-06-08). Governs the W1 core wave and every later asset-class wave
  of `docs/MASTER-EVOLUTION-PROGRAM.md`.
- **Supersedes/extends:** the FX-only Layer-0 shape. Honours ADR-0007 (one unversioned
  contract — no `schema_version`) and GUIDE.md guardrails #8/#9/#10/#11.

## Context

Celnet was FX-only at three load-bearing Layer-0 seams: `celnet-types::VanillaInputs/Greeks`
(Garman-Kohlhagen `r_dom`/`r_for`, two-rho), `celnet-proto` (`CcyPair`, `{spot,vol,r_dom,r_for}`),
and `celnet-plugin-api::PricingModel` (`price(OptionType,&VanillaInputs)`). To exceed SynOption's
asset-class coverage (crypto, metals, equity/commodity, …) we must generalize these — *in place*,
one clean contract, no legacy, FX byte-identical, zero-alloc hot core preserved.

## Decision

Separate the one fused FX seam into **three orthogonal layers**, each with a single concern:

1. **Identity — `Underlying`.** A discriminated union (`Fx(CcyPair)` now; `Metal`/`DigitalAsset`/
   `Equity`/`Listed` as additive enum growth per wave) carrying asset class + settlement style
   (deliverable / cash@fixing / inverse-coin). This is the **only** place asset class is named.

2. **Market — `Carry` as a forward/discount *producer*.** `Carry` answers *how the underlying
   drifts and discounts* by producing `forward(t)` and `discount(t)` — it is **not** a variant
   that payoff engines branch on. FX two-rate, equity `(r,q)`, commodity `(r,convenience)`,
   crypto `(r,funding)` all collapse to generalized-BSM `F = S·e^{b·t}`, `df = e^{−r·t}`; they
   differ only in how `b` is assembled from named factors. `Carry::FxRates` reproduces the FX
   forward/discount **bit-for-bit** (the byte-identity invariant).

3. **Payoff — asset-class-AGNOSTIC engines.** Vanilla/barrier/Asian/… consume `forward(t)`,
   `discount(t)`, and a vol surface **only**. A new asset class is a new `Carry`/`MarketState`
   builder with **zero payoff-engine edits**. GK already computes a forward + two discount
   factors internally; we are only making *where those come from* pluggable.

### The no-workaround test (binding)

A payoff engine that does `match carry { FxRates => …, CostOfCarry => … }` is a FAILURE of the
abstraction. Payoff code calls `inputs.forward(t)` / `inputs.discount_df(t)` and nothing else
about carry. Any such branch in payoff code is a review blocker, not a shortcut to accept.

### Sensitivities: fixed hot-core shape + factor-keyed risk layer

`Greeks` lives in the pinned **zero-alloc** hot core (guardrail #11), so it keeps a fixed
`Copy`/POD shape; rate sensitivities are a small fixed enum (`RateSensitivities::Fx{rho_dom,
rho_for}` / `Carry{discount_rho,carry_rho}`), **not** an allocating factor map. The factor
decomposition into FRTB GIRR/equity/commodity buckets happens at the **risk layer**
(`celnet-risk-cube`, off the hot path), whose roll-up algebra is already factor-generic. The FX
relation is exact: `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho`, recovered by the
existing AAD adjoint. This reconciles "general" with "zero-alloc."

### Forward-compatibility (seam placement test)

- **Multi-curve ready:** a flat-rate `Carry` is the degenerate single-point curve. The P1
  `celnet-curve` (OIS/CSA discounting, projection/basis) slots **behind the same
  `forward()`/`discount()` interface** with zero payoff-engine changes — the proof the seam is
  in the right place.
- **One unversioned contract, intuitive clients:** the wire gains an `Underlying` oneof + a
  `CarryModel` oneof + `RateSensitivities`; **clients default to FX** so every existing ticket /
  `CELNET.PRICE` / `celnet price EURUSD` is unchanged — the asset-class selector is purely
  additive. Intuitive = no new friction for the FX user; first-class path for the crypto user.
- **No silent mis-pricing:** a product×underlying validity matrix → typed `INVALID_ARGUMENT`,
  never a fallback. An FX leaf handed a non-FX carry errors rather than guessing.

## Consequences

- W1 generalizes types/proto/plugin-api + server routing with FX byte-identical (the
  no-regression gate: golden CSV + W0 conformance corpus across all 5 clients, all `to_bits`).
- Later waves add asset-class **leaves** (`celnet-crypto-vanilla`, `celnet-equity-vanilla`, …)
  and `Underlying`/`CarryModel` arms — additive, disjoint, fanned out in parallel lanes once the
  W1 contract is frozen.
- The internal FX analytics crates (`celnet-exotics`/`-surface`/risk) keep using `VanillaInputs`
  unchanged — it is the FX leaf's legitimate input, **not** legacy.

## Alternatives rejected

- **Generalized-BSM `(r,b)` as the single representation everywhere** — too lossy for risk
  (cannot independently report `rho_dom` vs `rho_for`, or dividend-rho, from one `b`); kept as the
  *forward/discount* math, not the *risk-factor* representation.
- **Factor-keyed sensitivity map on `Greeks`** — allocates; violates the zero-alloc hot core.
  Pushed to the risk layer instead.
- **`Carry` threaded as a branch through every pricer** — a workaround; rejected by the
  no-workaround test above.
