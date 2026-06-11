# Celnet — Verification Contract (per-asset-class, enforceable)

> The mandatory, written gate set every new product / asset-class must satisfy
> before it is **done**. This replaces the tribal "the FX recipe" with one
> enforceable contract. It is the standing reference for
> [MASTER-EVOLUTION-PROGRAM.md](MASTER-EVOLUTION-PROGRAM.md) §4 (api-first parity
> gate), §5 **W0** (verification & hygiene foundation), and §6 convergence lens 3
> (completeness — every proto arm reachable + parity-gated), and it closes the
> **[W0] verify/verification-contract-doc** item in
> [WORLD-CLASS-BACKLOG.md](WORLD-CLASS-BACKLOG.md).
>
> **Scope of "product":** every arm of the single unversioned
> `Instrument.product` oneof in `crates/celnet-proto/proto/celnet.proto`. "Asset
> class" widens the same gate set to a new `Underlying` family (FX → crypto →
> equity → commodity). No versioned contract exists (guardrail #9), so there is
> exactly one current shape to verify.

---

## 0. The bar in one sentence

A product is **done** only when its price (and reported sensitivities) is proven
**correct against an independent, model-disjoint oracle**, **frozen** so it cannot
silently drift, **reachable and identical from every client** (server == SDK ==
CLI == Excel == GUI booting a real edge), within its **performance budget** where
one applies, **hardened** where it decodes untrusted bytes or is numeric-core,
and any deploy-bound aspect carries an explicit **validation-scope statement**.

Each sub-section below is a hard gate. The CI lint
`tools/check-verification-coverage.mjs` (recipe `just verification-coverage`,
wired into `just check`) mechanically enforces gates **(b)/(a)** and **(c)** for
every oneof arm, plus the **client-exposure half of (d)** — every family is
either exposed by each client conformance suite or explicitly declared
not-exposed with a reason in that client's manifest; the remaining gates are
enforced by the named test suites and the milestone checklist in
MASTER-EVOLUTION-PROGRAM.md §7.

---

## (a) An INDEPENDENT, model-disjoint oracle  — MANDATORY

Every price (and every closed-form Greek) is validated against a reference
derived by a **genuinely different route** from the production pricer. Acceptable
oracle classes, in preference order:

1. **QuantLib 1.42.1** (open-source golden oracle — guardrail #7) frozen into
   `crates/celnet-golden/` tables and consumed by `crates/celnet-parity/tests/`.
   Used for vanilla and the first-generation exotics (digitals, all-eight
   single-barriers, double knock-out/in, touches) — see
   `crates/celnet-parity/tests/exotics.rs` and the grid self-checks under
   `crates/celnet-golden/tests/*_grid.rs`.
2. **A second closed form** reached by a different derivation (e.g. a damped Carr-
   Madan integral vs a Fang-Oosterlee COS expansion for Heston —
   `crates/celnet-parity/tests/heston.rs`; or the flat-σ `K_var == σ²` limit of
   the variance-swap replication strip — `var_vol_swap.rs`).
3. **A published reference value**, hand-pinned with its citation in the test
   (e.g. Levy-1992 basket in `basket.rs`; Longstaff-Schwartz-2001 Table-1 put
   `2.314` for American; Albrecher-2007 little-Heston-trap anchor).
4. **A code-disjoint Monte-Carlo** written independently in the test (its own RNG
   / scheme), gated within the **reported MC standard error** — never to closed-
   form precision (e.g. Curran/Turnbull-Wakeman Asian vs in-test MC in
   `asian.rs`; clamped cliquet vs independent clamped MC in `forward_start.rs`).

### The anti-circular-oracle rule (NON-NEGOTIABLE)

The oracle must not share the formula, the constants, or the bug under test. A
"second implementation" that re-derives the **same** expression is a
**circular oracle** — it passes while both are wrong. Concretely:

- Do **not** copy the production algebra into the test. Reach the reference by an
  independent identity, limit, structural inequality, or external value.
- Re-derive any regulatory / textbook **constants from the primary source text**
  in the test, do not lift them from the implementation.
- Prefer at least one oracle that is *qualitatively* different (a limit, a
  monotonicity, a put-call parity, a structural sandwich) so a shared algebraic
  slip cannot hide.

#### Cautionary tale — the FRTB `0.75ρ` circular oracle (real, caught here)

In the FRTB-SA work (`crates/celnet-risk-cube/frtb.rs` /
`crates/celnet-parity/tests/frtb.rs`), the low-correlation scenario was coded as
`ρ_low = max(2ρ − 1, 0)` — **missing the MAR21.6(2) `0.75ρ` floor**
(`ρ_low = max(2ρ − 1, 0.75ρ)`; material for ρ < 0.8 — FX γ = 0.6 must give 0.45,
not 0.2). The "independent" longhand oracle in the test had re-derived the **same
wrong formula**, so the self-check passed *while wrong*. The fix was threefold and
is now the template:

1. fix the production code **and** the oracle, and
2. add `correlation_scenario_transform_matches_basel_constants` pinning the
   transform to **BCBS hand-computed values taken from the MAR text**, so the
   constant can never silently drift again, and
3. treat any oracle that mirrors the implementation's algebra as **not
   independent** — it does not satisfy gate (a).

The lesson: an oracle is only independent if it can disagree. Always include at
least one external-value or qualitative-limit gate that does not reuse the
production expression.

---

## (b) A frozen golden CSV or closed-form reference  — MANDATORY

The accepted values are **frozen on disk** so a future edit that changes a number
fails loudly:

- QuantLib-derived tables live under `crates/celnet-golden/` (frozen CSV /
  table modules; `crates/celnet-golden/src/`), regenerated only deliberately via
  `crates/celnet-golden/src/bin/gen_vectors.rs` and re-gated by
  `crates/celnet-golden/tests/vectors_selfcheck.rs`.
- A closed-form reference value (published constant or analytic limit) is pinned
  **in the parity test** with its citation, and additionally captured in the
  family's golden vector (gate (c)) as the cross-client frozen value.

A "frozen" value carries its **honest tolerance**: closed-form vs closed-form is
tight (≈1e-9 … last-bit `to_bits`); an MC family carries a **positive
`price_std_error`** and is gated within a stderr band, never to closed-form
precision (the corpus self-check asserts MC families carry a positive stderr —
see `MC_FAMILIES` in `crates/celnet-golden/src/vectors.rs`).

---

## (c) A cross-client golden VECTOR  — MANDATORY

Every family has a **language-neutral golden vector** at
`crates/celnet-golden/vectors/<family>.json`, engine-generated and each value
cross-checked by its parity oracle. The vector is the single artifact every
client conformance harness asserts against, so server/SDK/CLI/Excel/GUI cannot
diverge silently.

Each record carries: a unique `id`, the `family` tag (one of the 18 oneof arm
names), `underlying`, `tenor`, `market`, `terms`, and `expected` (`price`, the
Greek strip where applicable, `price_std_error` for MC families, and the `oracle`
provenance string). The corpus is **FROZEN**; the family set must equal the proto
oneof arm set exactly (asserted by `vectors_selfcheck.rs` against the `FAMILIES`
constant and enforced by `tools/check-verification-coverage.mjs`).

> Authoring tip: read two existing vectors to learn the shape — e.g.
> `vanilla.json` (closed-form, full Greek strip, `oracle: quantlib-…`) and a
> Monte-Carlo family with a non-null `price_std_error`.

---

## (d) Cross-client conformance (server == SDK == CLI == Excel == GUI)  — MANDATORY

The capability must be **reachable from every client and produce the same number**
as the frozen vector, proven against a **real, freshly-booted edge** (not an
in-process fake):

- **Server / SDK / CLI:** Rust conformance + the parity rows in
  `crates/celnet-parity/tests/` drive the public pricer; the SDK
  (`celnet-client`) and CLI (`celnet-cli`) e2e tests boot a real edge (the
  `demo_edge` / `gui/e2e/demoEdge.ts` pattern) and assert SDK == CLI == server.
- **GUI:** Playwright real-edge e2e under `gui/e2e/` boots the real `demo_edge`
  and drives ticket → price for the product.
- **Excel:** the **real-edge** conformance suite (the **[W0] verify/excel-real-edge**
  item) — Excel must dial the real wire, not only its in-process `FakeSocket`.

`docs/CLIENT-PARITY-MATRIX.md` is generated **from** the passing harness, never
hand-asserted. A capability is not "done" until a row exists per
(asset-class, product) and is green from every client.

---

## (e) The performance budget where applicable  — MANDATORY WHEN ON A BUDGETED PATH

If the product sits on a latency/throughput-budgeted path
(`docs/ARCHITECTURE.md` §1.2; `docs/SCALE-OUT.md` §11), it must meet its budget,
measured by the truth-benches and held by a regression gate:

- In-core price+Greek loop: `crates/celnet-bench` `core_load` (absolute p50/p99/
  p99.9 HdrHistogram) + `bench_gate` (absolute + relative).
- Wire-path RFQ round-trip: `just bench-wire` / `just bench-gate` against the
  committed `crates/celnet-bench/baselines/`.
- Surface rebuild / fleet SLO: the `bench_gate` arms + `fleet_slo` (labelled
  **LOOPBACK** — see the honest-boundary statement, gate (g)).

A product with no closed form whose only path is an offline analytic call (e.g. a
batch FRTB roll-up) is exempt from the hot-path budget but must still not regress
its own bench where one exists. Record the applicable budget (or "not on a
budgeted path") in the parity row's module doc.

---

## (f) Fuzz / mutation where the crate decodes untrusted bytes or is numeric-core  — MANDATORY FOR THOSE CRATES

- **Mutation gate** (kill-rate floor) for the numeric-core crates: `just
  mutants-gate-{vanilla,surface,exotics,risk-cube,xva}` (audited equivalence sets
  in `.config/mutants-*.toml`; ≥90% non-equivalent kill-rate — see
  `docs/HARDENING.md`). A new numeric crate adds its own gate.
- **Fuzz target** for any crate that decodes **untrusted bytes** (wire / journal /
  replicated-log / proto decoders): a `cargo-fuzz` target under `fuzz/` (e.g.
  `just fuzz-vanilla`) plus gated proptests. A new decoder ships with a fuzz
  target before it is "done".

If a product introduces neither numeric core nor a byte decoder, state that
explicitly in its parity row module doc (so the omission is a decision, not a
gap).

---

## (g) A deployment / validation-scope statement for any deploy-bound aspect  — MANDATORY WHEN ANY ASPECT IS DEPLOY-BOUND

Anything that **cannot be proven in-repo** is designed + seamed + ADR'd in-repo
and validated at deploy — **never claimed in-repo**. The product's docs and the
relevant test/bench must carry the verbatim honest-boundary statement naming what
is proven here vs deferred. The standing deploy-bound set (unchanged discipline):

- **NVIDIA absolute GPU throughput / ≤50ms exotic / Workload-A·B absolutes** —
  Apple M4 Metal lacks f64, so in-repo GPU proves **correctness (f32 ↔ f64 ↔
  golden) + a host-local ratio only**; absolutes are CUDA-deploy-gated (G8).
- **Cross-host wire p99 / kernel-bypass NIC / §11 absolute wire SLOs** — loopback
  proves the compute + framing + relative regression (upper bound on compute,
  lower bound on cross-host wire); absolutes are deploy-gated.
- **Live JVM Celer estate lifecycle** — integration is seamed + ADR'd; live
  behaviour is deploy/live-gated.
- **Raft §6 dynamic membership** — documented as the next increment, not half-built.

A deploy-bound aspect with **no** scope statement is a contract violation, even if
every in-repo gate is green.

---

## How the lint enforces (b)/(a), (c) and the client-exposure half of (d)

`tools/check-verification-coverage.mjs` (run by `just verification-coverage`,
itself a dependency of `just check`):

1. parses the `oneof product { … }` block inside `message Instrument` in
   `crates/celnet-proto/proto/celnet.proto` and extracts every arm's snake_case
   field name — the canonical family key (no hard-coded arm list; the proto is
   authoritative);
2. for each family asserts **(i)** a non-empty `crates/celnet-golden/vectors/<family>.json`
   whose every record's `family` tag matches, **and (ii)** a curated
   family → `crates/celnet-parity/tests/<file>.rs` mapping that resolves to a real
   file containing `#[test]` rows that reference the family;
3. repeats both assertions for every cross-asset option family derived from the
   non-FX `Underlying.ref` oneof arms (`equity_option` / `commodity_option` /
   `crypto_option`) — a new cross-asset underlying cannot ship uncovered;
4. enforces the **CLIENT axis**: every product arm and cross-asset family must
   be accounted for in each client exposure manifest — **exposed** by the
   client suite (`FAMILIES_COVERED` in `gui/test/conformance.test.ts`;
   `EXCEL_FAMILIES` in `excel/e2e/corpus.ts`) or **explicitly declared
   not-exposed with a reason** (`FAMILIES_NOT_EXPOSED_BY_GUI`, every member
   named in its per-family reason doc block; the Excel suite's own derived
   `FAMILIES_NOT_EXPOSED = ALL_FAMILIES \ EXCEL_FAMILIES`, every not-exposed
   `ALL_FAMILIES` entry headed by a rationale comment). A family in neither
   list — one that would carry a vector and a parity row yet ship with **no
   client able to price it** — fails the lint naming the family and the client;
5. exits non-zero listing any arm missing a vector, a parity row, and/or an
   honest client-exposure declaration.

**Adding a new product arm therefore requires, in one change:** the proto oneof
field, the golden vector, the parity row, the one-line map entry pointing at
that row, and a deliberate per-client exposure decision (a client suite row, or
a reasoned not-exposed declaration in that client's manifest). The lint then
keeps them honest forever. There is intentionally no catch-all — an unmapped
arm fails. **The lint is never weakened to pass; the missing artifact is
added.**

### Current coverage status (as enforced today)

The lint is REAL — it parses the proto and the client manifests and checks disk
on every run. At the time of writing it is **green on every axis**: all **23**
product-oneof arms and all **3** cross-asset option families carry both a
golden vector and a `celnet-parity` row (the historical `strategy` / `american`
gaps were closed by `crates/celnet-parity/tests/strategy.rs` and `american.rs`),
and every family is accounted for on the client axis — GUI: 18 exposed + 8
declared not-exposed with per-family reasons; Excel: 23 exposed + 3 declared
not-exposed with a rationale comment. A future gap on any axis fails
`just verification-coverage` (and therefore `just check`) naming the family —
the remediation is always to add the missing artifact or the honest reasoned
declaration, never to weaken the lint.

---

## Checklist (paste into a new product/asset-class PR)

- [ ] (a) Independent, model-disjoint oracle — and it can *disagree* (no circular
      oracle; constants re-derived from primary source).
- [ ] (b) Frozen golden table / pinned closed-form reference with honest tolerance.
- [ ] (c) `crates/celnet-golden/vectors/<family>.json` cross-client vector(s).
- [ ] (d) server == SDK == CLI == Excel == GUI against a real edge; parity-matrix row.
- [ ] (e) Performance budget met + regression-gated (or "not on a budgeted path").
- [ ] (f) Mutation gate (numeric core) and/or fuzz target (byte decoder) — or an
      explicit "neither applies".
- [ ] (g) Honest validation-scope statement for any deploy-bound aspect.
- [ ] `just verification-coverage` green (arm ⇄ vector ⇄ parity-row ⇄ per-client
      exposure declaration), `just check`
      prints `All gates passed.`
