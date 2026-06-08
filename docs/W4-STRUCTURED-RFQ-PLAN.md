# W4 — Structured products + multi-dealer RFQ workflow: staged execution plan

> Wave W4 of [MASTER-EVOLUTION-PROGRAM.md](MASTER-EVOLUTION-PROGRAM.md) §5. Two disjoint
> tracks: **Track A** completes the structured-product catalogue (Pivot + two new payoff
> shapes), **Track B** builds the multi-dealer RFQ aggregation workflow (SynOption's true
> moat). The single live backlog stays [WORLD-CLASS-BACKLOG.md](WORLD-CLASS-BACKLOG.md)
> (items `[W4] exotics/pivot`, `[W4] proto/new-payoff-shapes`, `[W4] workflow/multi-dealer-rfq`).
> Every product gate obeys [VERIFICATION-CONTRACT.md](VERIFICATION-CONTRACT.md) (a)–(g).
>
> **Precondition:** the W1 contract is frozen — `Underlying`/`CarryModel`/`RateSensitivities`
> on the wire, a Carry-based pricing trait in `celnet-core`, `celnet-vanilla` the FX leaf. This
> plan ASSUMES W1 and is purely additive on top of it (ADR-0008). It is READ-ONLY planning
> running alongside W1 implementation — no code is touched here.

## 0. Governing principles (binding)

1. **ADR-0008 no-workaround test.** Track A's new payoff engines call `inputs.forward(t)` /
   `inputs.discount_df(t)` and the vol surface ONLY — never `match carry { … }`. A carry branch
   in payoff code is a review blocker. The new shapes are asset-class-AGNOSTIC; FX is the default
   `Underlying`/`CarryModel` arm, so an FX caller is unchanged.
2. **No legacy, no placeholders (#10, #2).** New arms are additive on the flat product oneof; no
   `schema_version` (#9). Pivot reuses the existing MC + `price_std_error` stack — no new RNG.
3. **Independent, non-circular oracle per product (VERIFICATION-CONTRACT (a)).** Each oracle must
   be able to *disagree*: reached by a different route (code-disjoint RNG, structural sandwich,
   degenerate limit to an already-gated product, QuantLib). The FRTB `0.75ρ` circular-oracle
   lesson is explicitly guarded below.
4. **Workspace hygiene.** No new crate is needed for W4 (Track A extends `celnet-exotics`; Track B
   adds one leaf crate `celnet-rfq`). The leaf auto-joins via `members = ["crates/*"]` and is
   registered in root `[workspace.dependencies]` as `celnet-rfq = { path = "crates/celnet-rfq" }`;
   its internal deps are `.workspace = true` (the path-dep lint added in W0 forbids inline path
   deps). `celnet-rfq` depends only on `{celnet-types, celnet-proto, celnet-fix, celnet-core}` —
   one-way acyclic; it does NOT depend on the hot `celnet-engine`.

---

## Track A — Structured products

### A1. Pivot (`crates/celnet-exotics/src/pivot.rs`)

**What it is.** The one SynOption structured type Celnet lacks. A pivot is a TARF generalization
with a **pivot strike that switches the client between two enhanced rates** (a favourable rate
above the pivot, an adverse/geared rate below — or vice-versa), with a cumulative target that
knocks the structure out, leverage on the adverse leg, and the same gap-risk redemption styles.
Modelled as a strip of periodic fixings exactly like `tarf.rs` (`Tarf`/`tarf_price`), adding a
`pivot` level distinct from the two settlement strikes.

**Numerics.** Reuse the existing MC stack to the letter — `crate::rng::CounterRng` +
`crate::normal::inverse_cdf`, antithetic pairs, the `Welford` std-error accumulator, exact GBM
terminal law per fixing (no time-step bias), per-fixing discount `e^{-r·t_k}`. Return a
`PivotResult { price, std_error, expected_redemption_fixing, expected_overshoot }` mirroring
`TarfResult`. Where continuous monitoring is offered, reuse the Sobol/Brownian-bridge engine
(`celnet-qmc::{SobolSequence, BrownianBridge}`) already consumed by `lookback.rs`/`accumulator.rs`
(`Monitoring::Continuous`). NO new RNG, NO new bridge.

**Oracle (a) — two independent routes, neither circular:**
- **Route 1 (code-disjoint MC).** A `splitmix64` reimplementation written independently IN the
  parity test (its own RNG seeding + Box-Muller or its own inverse-CDF, NOT `CounterRng`/
  `inverse_cdf`), gated within `4·(se_prod + se_oracle) + ε` — never to closed-form precision.
  This is VERIFICATION-CONTRACT oracle class 4 (code-disjoint MC), exactly the
  `forward_start.rs` clamped-cliquet pattern.
- **Route 2 (degenerate → plain TARF limit).** When the pivot is set equal to the strike (or
  pushed so the two-rate switch collapses to a single rate), the pivot payoff is **identically a
  TARF**; assert `pivot_price ≈ tarf_price` under common random numbers (shared seed) to ~MC
  precision (tight, the random draws coincide). This is a qualitative limit to an
  ALREADY-INDEPENDENTLY-GATED product (the `structured.rs` row 18 TARF), so it cannot share the
  bug under test.
- **Circular-oracle risk: MEDIUM.** The degenerate-limit oracle (Route 2) shares the
  `celnet-exotics` MC engine with the production pivot, so it tests the *payoff wiring*, not the
  *engine*. Route 1 (code-disjoint MC) is the genuinely independent engine check and is
  MANDATORY — Route 2 alone is insufficient. Also add a structural invariant (tighter target
  redeems earlier; higher leverage raises bank value) per the `tarf.rs` test template, so a shared
  algebraic slip in the strip walk cannot hide behind the same-engine limit.

### A2. New payoff SHAPES — `PerpetualOption` + `ListedFutureOption`

These are genuinely new payoff *shapes* (not new underlyings), so they need new product-oneof arms
(unlike vanilla/barrier/etc., which W1 already made underlying-agnostic).

- **`ListedFutureOption`** — an option whose underlying is a listed future, priced **Black-76**
  (`F·N(d1) − K·N(d2)` discounted, `b = 0` cost-of-carry on the future). Under ADR-0008 this is a
  pure carry choice (`CostOfCarry { r, b: 0 }`) on the agnostic vanilla engine — but the SHAPE
  (futures-settled, premium convention) is new, so it gets its own arm + a thin payoff adapter in
  `celnet-exotics/src/payoff.rs` (or a small `future_option.rs`) that constructs the zero-carry
  forward. **Oracle (a):** QuantLib Black-76 future-option engine, frozen into
  `crates/celnet-golden/` and consumed by the parity row to ~1e-10 (oracle class 1 — tight). Plus
  a put-call-parity invariant (`C − P = df·(F − K)`) as a qualitative non-circular cross-check.
- **`PerpetualOption`** — a perpetual American option (no expiry; closed-form free-boundary
  exercise: Merton/McKean perpetual-put / perpetual-call formula). **Oracle (a):** the published
  perpetual-option closed form pinned with its citation IN the test (oracle class 3 — published
  value), AND a structural cross-check that a long-but-finite American (from `american.rs` PSOR FD)
  **converges from below** to the perpetual value as `T → ∞` (qualitative limit, oracle class 2).
  **Circular-oracle risk: LOW for the future-option** (QuantLib is genuinely disjoint); **MEDIUM
  for the perpetual** — guard by re-deriving the perpetual constant (`β₁ = ½ − b/σ² + √(…)`)
  symbolically FROM the primary text in the test comment, not lifting it from the impl, and pin a
  published numeric anchor so the constant cannot silently drift (the `0.75ρ` lesson template:
  fix code + oracle + a constant-pinning test).
- **Cross-asset basket sandwich (shared A2 oracle).** Because W1 makes `BasketLeg` embed
  `Underlying`+`CarryModel`, add a **2-leg cross-asset basket** parity check reusing the existing
  `celnet-exotics::multiasset` (`price_basket`, `BasketKind::{Basket,BestOf,WorstOf}`, `cholesky`):
  assert the **worst-of ≤ single-leg ≤ best-of sandwich** against an independent in-test Cholesky
  GBM MC (its own RNG), within stderr. This is oracle class 2 (structural sandwich) and validates
  the new shapes compose under the agnostic carry seam. Circular risk LOW (model-free inequality).

### A3. Proto + clients for Track A (additive)

- **Proto (`crates/celnet-proto/proto/celnet.proto`):** append to the flat `oneof product` (after
  `basket = 25`): `Pivot pivot = 26;`, `PerpetualOption perpetual = 27;`,
  `ListedFutureOption listed_future_option = 28;` + their message bodies (Pivot reuses
  `FixingSchedule` like TARF/accumulator; `ListedFutureOption` embeds `Underlying`+`CarryModel`
  with a future ref). Use the `protobuf` skill for enum-prefix / field-number hygiene. Server
  enforces the **product × underlying validity matrix** (ADR-0008): a perpetual/listed-future on
  an incompatible `Underlying` ⇒ typed `INVALID_ARGUMENT`, never a silent fallback.
- **Golden vectors (VERIFICATION-CONTRACT (c)):** `crates/celnet-golden/vectors/pivot.json`,
  `perpetual.json`, `listed_future_option.json` — engine-generated via
  `crates/celnet-golden/src/bin/gen_vectors.rs`, re-gated by `vectors_selfcheck.rs`. Pivot is an
  **MC family** (positive `price_std_error` required by `MC_FAMILIES`); the two shapes are
  closed-form (null stderr). Family set must equal the proto oneof arm set (asserted by the lint).
- **Parity rows + lint map (VERIFICATION-CONTRACT (a)/(c) + the coverage lint):**
  `crates/celnet-parity/tests/pivot.rs`, `perpetual.rs`, `listed_future_option.rs`, each adding the
  curated `FAMILY_TO_PARITY_FILE` entry in `tools/check-verification-coverage.mjs`
  (`pivot: 'pivot.rs'`, etc.). The lint is NEVER weakened — the artifact is added. (Note: W4 should
  also close the two pre-existing `strategy`/`american` lint gaps if not already closed, since they
  block 18/18; flagged as an open question.)
- **Mutation (VERIFICATION-CONTRACT (f)):** Pivot is numeric core ⇒ extend the audited
  `just mutants-gate-exotics` set (`.config/mutants-*.toml`) to cover `pivot.rs` at the ≥90%
  non-equivalent kill floor; no new byte decoder ⇒ no new fuzz target (state explicitly in the
  module doc).

---

## Track B — Multi-dealer RFQ aggregation (`crates/celnet-rfq`)

**What it is.** Celnet is single-dealer; SynOption's actual moat is the multi-bank RFQ venue. A
`MultiDealerQuote` flow fans one `QuoteRequest` to **N quote sources** — internal Celnet pricers
AND external **LP adapters over the existing `celnet-fix` 4.4 engine** (`initiator` role:
QuoteRequest→Quote, last-look via QuoteID validity) — collects two-way bid/offer responses, ranks
the **best bid / best offer**, applies **tie-break + timeout/last-look**, and returns an audited,
ranked panel with `lp_count` / `lp_won` consistency.

**Crate layout (`crates/celnet-rfq`, new leaf):**
- `lib.rs` — public API.
- `panel.rs` — `QuoteSource` trait (object-safe; one method `request(&self, &QuoteRequest, deadline)
  -> QuoteSourceReply`), the `MultiDealerEngine` that fans out concurrently (`futures::join_all`
  over the bounded panel — the `federate.rs` concurrent-fan-out precedent), and the ranking +
  tie-break + last-look logic.
- `lp_fix.rs` — `FixLpAdapter`: a `QuoteSource` wrapping a `celnet-fix::Initiator` over a real
  loopback socket (mirrors `tests/risk_federation.rs` ephemeral-port pattern), translating
  Celnet `QuoteRequest` ⇄ FIX QuoteRequest/Quote via `celnet-fix::dialect_fx`.
- `internal.rs` — `InternalPricerSource`: a `QuoteSource` calling the in-process Celnet pricer
  (so the panel always has ≥1 native dealer; the SDK/CLI/GUI then see the SAME ranked panel).

**Ranking semantics (concrete):**
- Best **bid** = max bid premium; best **offer** = min offer premium (client lifts the cheapest
  offer / hits the highest bid). Each side's winner is the `lp_won` for that side.
- **Tie-break (deterministic):** on equal best price, prefer (1) earlier `epoch_nanos`, then
  (2) lexicographically smallest stable `lp_id`. Deterministic so the panel is reproducible and
  auditable.
- **Timeout / last-look:** each source has a per-request deadline; non-responders are dropped (NOT
  errored) and excluded from `lp_count`. A winning quote past its `valid_until_nanos` is rejected
  (last-look), the next-best promoted. `lp_count` = sources that responded in time;
  `lp_won` ⊆ responders; the engine asserts `lp_won` references a real responder (the consistency
  invariant gated below).

**Oracle (a) — in-repo loopback with ≥3 synthetic LP responders (oracle class 4 + structural):**
A test harness in `crates/celnet-rfq/tests/` (and a `celnet-parity` row) boots **≥3 synthetic LP
responders** — at least one real `FixLpAdapter` over a loopback `celnet-fix` initiator/acceptor
pair, plus deterministic in-process sources with KNOWN injected bid/offer ladders. The oracle is
the **independently-computed expected winner** from the injected ladders (the test KNOWS which LP
has the best bid/offer by construction), so the ranking can be checked exactly, not circularly:
- best-bid/best-offer selection == the injected extremum;
- tie-break determinism (inject two equal-best LPs ⇒ assert the documented preference winner);
- timeout (one LP sleeps past the deadline ⇒ dropped, `lp_count` excludes it, next-best promoted);
- last-look (winner's `valid_until_nanos` in the past ⇒ rejected, promotion);
- `lp_count`/`lp_won` consistency invariant (`lp_won` always a responder; `lp_count` == responder
  count) over a property sweep of random ladders.
- **Circular-oracle risk: LOW.** The oracle is the injected ground-truth ladder, not a second copy
  of the ranking algebra — it can disagree by construction. The FIX leg is validated independently
  by `celnet-fix`'s own loopback tests (the dialect repricing the same premium as the engine).

**Proto + clients for Track B (additive):**
- **Proto:** a new `MultiDealerQuote` message (the ranked panel: repeated per-LP `DealerQuote
  {lp_id, two_way_price, epoch_nanos, valid_until_nanos, responded}`, `best_bid_lp`, `best_offer_lp`,
  `lp_count`, `lp_won_bid`, `lp_won_offer`) and a `QuoteService` RPC
  `rpc RequestMultiDealerQuote(QuoteRequest) returns (MultiDealerQuote);` (reuses the existing
  `QuoteRequest` — additive, no contract fork). `AcceptQuote` is extended to accept a panel winner
  by `(quote_id, lp_id)` so the audited winner is the one booked.
- **Deploy-bound carve-out (VERIFICATION-CONTRACT (g) — MANDATORY statement):** **live LP-panel
  connectivity** (real bank sessions over WAN FIX) and the **regulated-venue / MAS-RMO status** are
  ENV — designed + seamed + ADR'd in-repo, validated at deploy, NEVER claimed in-repo. In-repo
  proves the aggregation/ranking/tie-break/last-look ALGORITHM + the FIX framing/dialect over
  loopback + relative regression. The crate doc and the parity test carry the verbatim
  honest-boundary statement (the existing discipline; gated by ENV flag `CELNET_LP_PANEL` for live
  endpoints, defaulting to the synthetic in-repo panel).

---

## Staged GREEN increments (single driver per track; each commit gated)

Track A and Track B touch disjoint crates (A: `celnet-exotics`/golden/parity/proto-product-arms;
B: new `celnet-rfq`/`celnet-fix`-consumer/proto-quote-service) and CAN run on parallel
git-worktree lanes EXCEPT the proto file, which both edit — so the two proto edits are serialized
(A's product arms first, then B's QuoteService addition), then lanes diverge. Each commit keeps
`just check` green and the `verification-coverage` lint at full coverage.

1. **A-S1 — Pivot engine.** `pivot.rs` + crate unit tests (reproducibility `to_bits`, structural
   invariants). Gate: `just check-crate celnet-exotics` green; `cargo fmt` + `clippy -D` clean.
2. **A-S2 — Pivot parity + golden vector.** `pivot.json` + `parity/tests/pivot.rs` (code-disjoint
   splitmix64 MC within stderr + degenerate→TARF limit + structural) + lint map entry. Gate:
   `just verification-coverage` green for `pivot`; parity row passes `clippy -p celnet-parity
   --test pivot -D warnings` (the recurring parity-test-clippy lesson); `vectors_selfcheck.rs` green.
3. **A-S3 — New shapes (proto + engines).** `perpetual` + `listed_future_option` product arms +
   payoff adapters + Black-76/perpetual closed forms + validity-matrix guard. Gate: proto convert
   round-trip; `just check-crate` for exotics + proto; QuantLib golden for the future option frozen.
4. **A-S4 — New-shape parity + vectors + cross-asset sandwich.** `perpetual.json`,
   `listed_future_option.json`, parity rows (QuantLib ~1e-10, perpetual published-constant +
   T→∞ limit, worst≤single≤best basket sandwich) + lint map entries. Gate:
   `just verification-coverage` green (all arms incl. the new three); mutation gate extended to
   `pivot.rs`.
5. **A-S5 — Track-A clients (FX default).** SDK builders (`Instrument::pivot(underlying)…`,
   `::listed_future_option(…)`, `::perpetual(…)`), CLI subcommands, Excel polymorphic
   `CELNET.PRICE(underlying, "pivot", terms-range)` (no new per-product fn — the W1 polymorphic
   set), GUI Structuring-workspace leg entries. Gate: 5-client conformance harness asserts each new
   family REACHABLE + == its frozen vector from server/SDK/CLI/Excel/GUI (real edge); GUI Playwright
   + Excel real-edge e2e green.
6. **B-S1 — `celnet-rfq` engine + loopback oracle.** New crate (registered in
   `[workspace.dependencies]`, deps `.workspace = true`) + `panel.rs`/`internal.rs`/`lp_fix.rs` +
   `tests/` loopback harness with ≥3 synthetic LPs. Gate: `just check-crate celnet-rfq` green;
   ranking/tie-break/timeout/last-look/`lp_count`-`lp_won` all asserted vs injected ground truth.
7. **B-S2 — Proto QuoteService + server wiring.** `MultiDealerQuote` message +
   `RequestMultiDealerQuote` RPC (+ WS mirror) + server fan-out (InternalPricerSource always
   present; FIX LP adapters behind `CELNET_LP_PANEL`). Gate: proto round-trip; server e2e the
   ranked panel == the loopback oracle; `celnet-parity` RFQ row added (no golden VECTOR — RFQ is a
   workflow, not a product oneof arm, so it is exempt from the `verification-coverage` arm⇄vector
   lint; this is stated explicitly in the parity row doc).
8. **B-S3 — Track-B clients (identical ranked panel).** SDK `request_multi_dealer_quote`, CLI
   `celnet rfq <underlying> <product>` printing the ranked panel, GUI multi-dealer RFQ panel
   (best bid/offer highlighted, per-LP ladder, won-by chip). Gate: GUI/CLI/SDK see the **identical
   ranked panel** (asserted against a real edge with the synthetic panel); honest-boundary banner
   shown for the live-LP/regulated-venue ENV carve-out.
9. **MILESTONE — W4 done.** Full `just check` prints the literal **"All gates passed."** (verified,
   not the wrapper exit code); 5-client conformance green; GUI vitest + Playwright + Excel real-edge
   e2e green; per-class verification contract complete for pivot/perpetual/listed-future-option;
   docs reconciled (CLIENT-PARITY-MATRIX regenerated from the harness; INTERFACES/ARCHITECTURE
   note the new arms + the RFQ service + the ENV carve-out); codebase-memory re-indexed. Branch-off
   `main`, push to `origin`.

---

## Cross-client surfacing (FX default; api-first parity)

- **SDK (`celnet-client`):** typed builders over `Underlying` — `Instrument::pivot(underlying)
  .strike(k).pivot(p).target(t).leverage(l)…` with sane MC defaults; `::listed_future_option(…)`;
  `::perpetual(…)`; `request_multi_dealer_quote(req)`. FX is the default `Underlying`, so existing
  `Instrument::vanilla(EURUSD)` ergonomics are unchanged. One runnable example per new product +
  one RFQ-panel example, gated against a real edge.
- **CLI (`celnet-cli`):** `celnet price <underlying> pivot …` / `perpetual` / `listed-future-option`
  (underlying parses as an FX pair by default, `--crypto`/`--equity` selecting other classes per
  W1); `celnet rfq <underlying> <product>` prints the ranked multi-dealer panel. Proven CLI == SDK
  == server.
- **Excel:** polymorphic `CELNET.PRICE(underlying, "pivot", terms-range)` / `"perpetual"` /
  `"listed-future-option"` over the W1 polymorphic set (no new per-product fn — #10) +
  `CELNET.RFQ(underlying, product, terms)` returning the ranked panel as a spilled range. Real-edge
  conformance suite asserts Excel == server.
- **GUI:** the W1 contract-derived **Structuring workspace** gets the three new product leg types
  with zero bespoke form code; a new **multi-dealer RFQ panel** (best bid/offer, per-LP ladder,
  won-by, live-LP ENV banner). UniverseNavigator unchanged (FX default).

---

## Verification-contract coverage summary (per new product oneof arm)

| Arm | (a) Oracle | (b) Frozen | (c) Vector | (d) 5-client | (e) Budget | (f) Mut/fuzz | (g) Deploy |
|-----|-----------|-----------|-----------|-------------|-----------|-------------|-----------|
| `pivot` | code-disjoint splitmix64 MC + degenerate→TARF limit + structural | `pivot.json` (MC, +stderr) | ✅ | ✅ | not on hot path (MC, offline) | mutation (exotics) ext.; no decoder | none |
| `perpetual` | published perpetual closed form + T→∞ American limit | `perpetual.json` (closed) | ✅ | ✅ | not on hot path | mutation (exotics) ext.; no decoder | none |
| `listed_future_option` | QuantLib Black-76 ~1e-10 + put-call parity + basket sandwich | `listed_future_option.json` | ✅ | ✅ | not on hot path | mutation (exotics) ext. | none |
| RFQ workflow (not an arm) | ≥3-LP loopback vs injected ground truth | — (workflow, vector-exempt) | — | ✅ ranked panel | — | no decoder beyond FIX (already fuzzed) | **live LP + MAS-RMO = ENV** |
