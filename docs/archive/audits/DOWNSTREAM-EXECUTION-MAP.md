# Downstream Execution Map — dependency + parallel-lane sequencing AFTER W1

> The single sequencing/concurrency map for everything **after** the in-flight W1 multi-asset
> core wave. It does not restate the waves — it answers three operational questions per
> wave/track: **(1) what must land first** (the dependency), **(2) which disjoint crate/file
> lane does it own** (so crate-disjoint lanes can run CONCURRENTLY on git-worktrees, with every
> shared-crate serialization point — proto / exotics / clients — called out exactly as the
> prior COMPLETION/LEADERSHIP programs learned), **(3) the milestone boundary** (a wave = one
> asset class or one cross-cutting capability fully landed across all 5 clients, all gates
> green). It closes with a recommended **stage-by-stage concurrency schedule** — which 2–3
> lanes to run in parallel at each stage to deliver fastest SAFELY.
>
> Source plans (read-only inputs, not duplicated here): `docs/MASTER-EVOLUTION-PROGRAM.md`
> (§2 sequencing rule, §5 wave plan, §7 milestones), `docs/W2-LINEAR-PLAN.md`,
> `docs/W3-CRYPTO-PLAN.md`, `docs/W4-STRUCTURED-RFQ-PLAN.md`,
> `docs/W5-CROSSASSET-RISK-PLAN.md`, `docs/GW-FOUNDATION-PLAN.md`. The single backlog truth
> remains `docs/WORLD-CLASS-BACKLOG.md`.

---

## 0. The two structural facts that drive all sequencing

**Fact 1 — W1 freezes the four interface seams; everything downstream depends ONLY on that
freeze.** MASTER §2's binding sequencing rule: the four interface-crate generalizations
(`celnet-types` Underlying/Carry/Sensitivities, the `celnet-core` Carry-based pricing trait,
`celnet-proto` Underlying/CarryModel/RateSensitivities + product×underlying validity matrix,
`celnet-plugin-api` generalized PricingModel) land **together in one coordinated wave** with FX
proven byte-identical, **then** per-asset-class leaves fan out. After W1 there is no further
*core* contract churn — only **additive** arms (new `Underlying` arms, new product oneof arms,
new `CarryModel`/surface-quote arms). This is the property that makes downstream parallelism
safe: the frozen seam is the synchronization barrier; lanes touch additive, disjoint regions.

**Fact 2 — there are exactly three shared-crate serialization points, recurring in every wave.**
The prior programs learned these the hard way (the W4-Wave merge-marker incident; the
risk-cube shared-field break that hit downstream consumers; the "two waves that share proto/
exotics/clients run sequentially" rule). Concretely:

| Shared serialization point | Why it serializes | The rule (from prior programs) |
|---|---|---|
| **`celnet-proto/proto/celnet.proto`** | every wave that adds a product arm / `Underlying` arm / RPC edits the ONE file; concurrent edits collide on field numbers + merge markers | additive proto edits are **staged first within a wave and serialized across concurrently-running waves** — never two open proto edits at once. Field numbers reserved up-front per wave. `protobuf` skill for hygiene; convert round-trip + FX `to_bits` byte-identity gate each edit. |
| **`celnet-exotics`** | W4 Track A (pivot/perpetual/listed-future) and W6 (2nd-gen exotics) both extend it | exotic-extending waves serialize **with each other** (W4-A before W6 depth); they are disjoint from crypto/linear/equity leaves and the risk cube. |
| **the 5 clients** (`celnet-client` SDK, `celnet-cli`, `excel/`, `gui/`, server routing) | every "fully landed" wave surfaces through all five; the polymorphic Excel set + GUI Structuring workspace + SDK builders are shared files | client surfacing is the **last stage of each wave** and is the cross-wave merge point; two waves' client edits serialize at the surfacing stage even when their engine lanes ran concurrently. The 5-client conformance harness is the cross-wave integration gate. |

Everything else — new leaf crates (`celnet-linear`, `celnet-crypto-vanilla`,
`celnet-equity-vanilla`, `celnet-commodity-vanilla`, `celnet-rfq`), the
`celnet-risk-normalize`/`-cube` generalization, the conventions/calendar breadth, the
`celnet-surface` leaves, the GUI substrate — is **crate-/dir-disjoint** and runs on independent
git-worktree lanes with no merge conflict, provided the three serialization points above are
respected.

---

## 1. Per-wave / per-track dependency + lane + milestone

Each row: **Dep** (what must land first) · **Lane** (the disjoint crate/file tree it owns) ·
**Serialization** (shared-crate touch points that force ordering) · **Milestone** (the
all-5-clients-green boundary).

### W0 — Verification & hygiene foundation (P0; runs alongside / before W1)
- **Dep:** none beyond the current tree. This is the FLOOR that must exist before the multi-asset
  fan-out so the disjoint-lane property holds.
- **Lane:** `crates/celnet-golden/vectors/*` (golden-vector corpus + `gen_vectors`/
  `vectors_selfcheck`), `tools/check-verification-coverage.mjs` (the proto-arm⇄parity-row⇄vector
  lint), the Excel real-edge conformance harness, `docs/VERIFICATION-CONTRACT.md`, and the
  **workspace-dep registry cleanup + `just workspace-deps` path-dep lint** (register
  `celnet-xva`/`-heston`/`-qmc`; switch 11 internal path-deps to `.workspace = true`).
- **Serialization:** the path-dep lint and `[workspace.dependencies]` registry touch root
  `Cargo.toml` — do this FIRST and alone (it protects every later lane's auto-join). No proto edit.
- **Milestone:** FX 5-client conformance corpus green; coverage lint green at the current arm
  count; path-dep lint green. Raises the floor; no new product.

### W2 — FX linear products + pair/metals breadth (P0/P2)
- **Dep:** W1 frozen contract (Underlying::Fx, Carry, RateSensitivities; the Carry pricing trait).
  W0 path-dep lint + coverage lint must be green (Track A registers a new crate).
- **Track A lane — linear book:** NEW crate `crates/celnet-linear/` (forward/swap/NDF closed-form
  PV+Greeks reading `carry.forward_factor`/`discount_df`); `crates/celnet-parity/tests/linear.rs`;
  golden vectors `fx_forward/fx_swap/ndf.json`.
- **Track B lane — pair/metals breadth:** `celnet-types` (additive `Underlying::Metal`/`MetalPair`/
  `Metal` + `MetalPair↔CcyPair`), `celnet-conventions` (registry → >75 + XPT/XPD + metal crosses),
  `celnet-calendar` (new Gregorian centres + loco-London∩quote∩USD), `tests/pair_universe.rs`
  (extended PUBLISHED table + independent rata-die walk).
- **Serialization:** ONE proto touch (S2) — adds the `Underlying.metal` arm AND the
  `fx_forward`/`fx_swap`/`ndf` product arms + `Side` in a single edit; **this serializes Track A
  and Track B at exactly that point** (W2 plan §0/§5). Tracks A and B otherwise run on parallel
  worktree lanes (disjoint: `celnet-linear`+golden+parity vs `celnet-types`+conventions+calendar).
  Client surfacing (S5) is the shared-clients stage.
- **Milestone:** forward/swap/NDF + a metal-cross price identical server==SDK==CLI==Excel==GUI vs a
  freshly-booted edge; `verification-coverage` 21/21; full `just check` "All gates passed."

### W3 — Crypto / digital-asset options (P1, XL)
- **Dep:** W1 frozen contract. Reuses `celnet-linear` UNCHANGED for crypto linear products (so W2
  Track A is a soft prerequisite for the crypto *linear* convenience, but not for the crypto leaf
  itself). No dependency on W2 Track B.
- **Lane:** NEW crate `crates/celnet-crypto-vanilla/` (linear via agnostic generalized-BSM +
  `inverse.rs` coin-measure payoff + `funding.rs`); `celnet-calendar` `TradingClock::Continuous`
  (24×7); `celnet-surface` strike/log-moneyness smile leaf; `tests/crypto.rs` + golden vectors
  `crypto_vanilla_{linear,inverse}.json`.
- **Serialization:** ONE proto touch (S7) — adds `CryptoPair`/`SettlementStyle` to the W1
  `Underlying` oneof; **no new product arm** (crypto vanilla rides the existing `vanilla` arm).
  `celnet-calendar` is also touched by W2 Track B (new fiat centres) — if W2-B and W3 run
  concurrently, the calendar edits are disjoint (W2-B adds centres/holidays; W3 adds the
  `TradingClock` enum + Continuous arm) but **review the calendar merge** as a soft contention
  point. `celnet-surface` is touched by both W3 (crypto leaf) and W5 is not — surface is W3-owned.
- **Milestone:** ≥1 linear + ≥1 inverse crypto vanilla reachable + identical across all 5 clients
  vs frozen vectors; the inverse `1/S_T` measure gated three independent ways (splitmix64 MC +
  Gauss-Hermite quadrature + `V_inverse·S_0 > V_linear` sandwich); "All gates passed."

### W4 — Structured products + multi-dealer RFQ workflow (P1)
- **Dep:** W1 frozen contract. Track A's cross-asset basket sandwich uses the W1 `BasketLeg`
  embedding `Underlying`+`CarryModel`. Independent of W2/W3.
- **Track A lane — structured catalogue:** `crates/celnet-exotics/src/pivot.rs` (+ perpetual /
  listed-future payoff adapters) reusing the existing MC + QMC stack; golden vectors
  `pivot/perpetual/listed_future_option.json`; `tests/{pivot,perpetual,listed_future_option}.rs`.
- **Track B lane — RFQ workflow:** NEW crate `crates/celnet-rfq/` (deps {types,proto,fix,core},
  NOT engine) — `panel.rs`/`lp_fix.rs`/`internal.rs`; loopback ≥3-LP oracle.
- **Serialization:** the proto file is touched by **both** tracks — Track A adds three product
  arms (pivot=26/perpetual=27/listed_future_option=28), Track B adds the `MultiDealerQuote` message
  + `RequestMultiDealerQuote` RPC. The W4 plan §"Staged increments" mandates **A's product arms
  first, then B's QuoteService addition** — the two proto edits SERIALIZE, then the lanes diverge.
  Track A also serializes on **`celnet-exotics`** against any other exotics-extending wave (W6).
  Client surfacing is the shared-clients stage; RFQ is **vector-exempt** (a workflow, not a oneof
  arm — stated in its parity row).
- **Milestone:** pivot/perpetual/listed-future reachable + == frozen vectors across 5 clients; the
  RFQ ranked panel identical server==SDK==CLI==GUI vs a real edge with the synthetic LP panel
  (live LP + MAS-RMO = ENV); "All gates passed."

### W5 — Cross-asset risk + equity/commodity leaves (P1/P2)
- **Dep:** W1 frozen contract. The new leaves use the W1 carry seam; the risk-fact widening uses
  the W1 `Underlying`/`RateSensitivities`. Independent of W2/W3/W4 at the crate level (the equity/
  commodity vanilla leaves do not depend on linear/crypto/exotics).
- **Track A lane — cross-asset risk:** `celnet-risk-normalize` (`AssetClass` + `UnderlyingRef`,
  generalize `CanonicalLeaf`/`PositionRisk`), `celnet-risk-cube/src/frtb.rs` (GIRR/equity/commodity/
  CSR bucket DATA + factor projection — algebra UNCHANGED); `tests/cross_asset_risk_cube.rs` +
  `frtb_cross_asset.rs`.
- **Track B lane — equity/commodity leaves:** NEW crates `crates/celnet-equity-vanilla/` +
  `crates/celnet-commodity-vanilla/` (generalized-BSM / Black-76 via Carry); golden tables +
  vectors `equity_vanilla.json`/`commodity_vanilla.json`; `tests/{equity,commodity}_vanilla.rs`.
- **Serialization:** ONE proto touch — adds `equity_vanilla`/`commodity_vanilla` product arms +
  `Underlying::{Equity,Commodity/Listed}` + the `RISK_DIMENSION_UNDERLYING` rename. Track A's
  `RiskFact` widening and Track B's leaves touch **different files** so they run on parallel
  worktree lanes; they re-converge only when Track A's non-additive re-pricers route to Track B's
  leaves (W5 plan S5 — so **Track B leaves should land before Track A's S5 mixed-asset roll-up**;
  internal ordering, not a cross-wave gate). The binding invariant: the FX `firm_aggregate ==
  single-node` 1e-12 parity stays byte-green after the fact widening (the no-regression gate).
- **Milestone:** equity + commodity vanilla reachable + == frozen vectors across 5 clients; mixed
  FX+equity+commodity firm roll-up == independent single-node recomputation (1e-12 additive / 1e-9
  non-additive); FRTB GIRR/equity/commodity/CSR longhand-oracle-gated with per-class BCBS-constant
  pins (the 0.75ρ template, negative control proving the gate disagrees); "All gates passed."

### W6 — Rigor uplift + 2nd-gen exotics depth (P2/P3)
- **Dep:** the asset-class superset (W2–W5) landed — W6 raises the mutation/fuzz/coverage floor
  ACROSS the now-larger exotics/surface/risk/MC surface area, and adds 2nd-gen exotics depth on top
  of the W4 catalogue.
- **Lane:** `.config/mutants-*.toml` + fuzz targets per numeric crate (mutation/fuzz/coverage —
  largely config + test files, low conflict); `celnet-exotics` 2nd-gen depth (KIKO / double-touch /
  corridor variance / fader / power).
- **Serialization:** `celnet-exotics` (serializes with W4 Track A — W4-A first). The mutation/fuzz
  config lane is disjoint from everything and can run concurrently with late W5/W4 client work.
- **Milestone:** ≥90% kill-rate + a fuzz target per numeric crate; 2nd-gen exotics PDE≈MC≈analytic
  gated; reachable across 5 clients where they are products; "All gates passed."

### W7 — Convergence rounds (recurring, terminal)
- **Dep:** all prior waves. Runs the §6 seven-lens read-only critique loop on
  `docs/WORLD-CLASS-BACKLOG.md`.
- **Lane:** read-only critique (no crate ownership); fixes are routed to whichever owning lane.
- **Serialization:** none (read-only); fix-forward commits respect the same three serialization
  points.
- **Milestone:** STOP when TWO consecutive full rounds are dry (zero new genuine in-repo gaps).

### GW0 / GW1 — GUI foundation (GUI-only; independent track, runs ANYTIME)
- **Dep:** none on the Rust waves — GW0/GW1 touch **no Rust crate, no Cargo.toml, no proto, no
  SDK/CLI/Excel source**. GW6 (multi-asset GUI) is the one GUI wave gated on master W1 and is OUT of
  GW0/GW1 scope.
- **Lane:** `gui/src/**` only (design system + a11y `<DataGrid>` substrate; three-axis navigation +
  command registry + saved views). The substrate that later asset-class GUI surfacing composes onto.
- **Serialization:** none against the Rust lanes (fully disjoint tree). The cross-client conformance
  harness must stay green (no numeric regression) at each GUI milestone — that is the only coupling.
- **Milestone:** GW0 — `<DataGrid>` `role=grid` reversal proven (axe + Playwright keyboard e2e);
  GW1 — three-axis nav + command registry + saved views, "All gates passed." (Rust unaffected) +
  GUI vitest/Playwright/axe + 5-client conformance green.

---

## 2. Recommended concurrency schedule (fastest, SAFELY)

The rule throughout: **at most ONE open `celnet.proto` edit at a time across all running lanes**,
**at most ONE open `celnet-exotics` edit**, and **client surfacing of two waves never merges
simultaneously** — these are the three hard serialization points. GUI (GW0/GW1) is Rust-disjoint
and runs as a continuous independent track. Within a wave, the wave plan's own staged increments
apply; this schedule sequences the WAVES against each other.

| Stage | Run in parallel (worktree lanes) | Shared serialization points to respect |
|---|---|---|
| **S-pre** | **W0** (verification + workspace-dep registry/path-dep lint) ALONE first · **GW0** (GUI substrate) in parallel (Rust-disjoint) | W0 owns root `Cargo.toml` (registry/lint) — finish before any new-crate lane starts. No proto edit. |
| **S-A** *(after W1 freezes)* | **W2-A** `celnet-linear` ∥ **W2-B** conventions/metals/types ∥ **GW1** (GUI nav) | The W2 proto touch (Underlying.metal + linear arms + Side) serializes W2-A↔W2-B at that single step; sequence it as W2-S2, then the lanes diverge. GW1 disjoint. |
| **S-B** *(W2 milestone pushed)* | **W3** crypto leaf ∥ **W5-B** equity+commodity leaves ∥ **W4-B** `celnet-rfq` | Three disjoint NEW-crate lanes + risk leaves; **NO proto edit may overlap** — stagger each wave's single proto touch (W3 `CryptoPair`; W5 equity/commodity arms; W4-B `QuoteService`) so only one is open at a time. `celnet-calendar` touched by W3 (TradingClock) — disjoint from W2-B's centre edits but review the merge. |
| **S-C** | **W4-A** exotics (pivot/perpetual/listed-future) ∥ **W5-A** cross-asset risk fact + FRTB buckets | W4-A owns `celnet-exotics` (serializes vs W6 later). W5-A owns `celnet-risk-normalize`/`-cube`. Their proto touches (W4-A product arms; W5 risk-dimension rename) stagger — one open at a time. W5-A's FX no-regression 1e-12 gate is the binding invariant. |
| **S-D** *(client surfacing — the merge point)* | Surface W2/W3/W4/W5 through the **5 clients** SEQUENTIALLY per wave (SDK builders → CLI → Excel polymorphic set → GUI Structuring workspace → server routing) | This is the shared-clients serialization point: two waves' client edits do NOT merge simultaneously. Run the 5-client conformance harness as the cross-wave integration gate after each wave's surfacing. |
| **S-E** | **W6** rigor uplift + 2nd-gen exotics depth (mutation/fuzz config lane ∥ exotics-depth lane) | W6 exotics depth serializes AFTER W4-A on `celnet-exotics`. Mutation/fuzz config lane is disjoint and may overlap late S-D client work. |
| **S-F** | **W7** convergence rounds (read-only seven-lens) | No crate ownership; fixes route to owning lanes respecting all three serialization points. Stop on 2 dry rounds. |

**Why this is the fast-but-safe ordering.** S-B runs the three highest-value disjoint NEW-crate
lanes (crypto — the biggest "exceed SynOption" move; equity/commodity leaves; RFQ engine)
concurrently because they share NO crate except the proto file, which is staggered. S-C overlaps
the two heaviest in-place generalizations (exotics catalogue; cross-asset risk) which own
different crates (`celnet-exotics` vs the risk crates) and stagger their proto touches. The single
true bottleneck is **client surfacing (S-D)** — the shared SDK/CLI/Excel/GUI files plus server
routing — so all waves' engine/oracle work is front-loaded into S-A/S-B/S-C on disjoint lanes, and
only the thin surfacing layer is serialized. GUI substrate (GW0/GW1) runs as a continuous
independent track from S-pre, so the multi-asset GUI surfacing in S-D composes onto a finished
`<DataGrid>`/Structuring substrate rather than blocking on it.

---

## 3. Critical path (longest dependency chain to "world-class, all waves done")

1. **W0** (verification floor + workspace-dep registry/path-dep lint) — must precede every
   new-crate lane.
2. **W1** (multi-asset core wave) — the single hard barrier; freezes the four interface seams;
   gated entirely on "FX byte-identical". Highest-risk single change.
3. **W3** (crypto, XL) — the biggest asset-class move and the riskiest oracle (the inverse `1/S_T`
   measure, gated three independent ways); on the critical path as the marquee capability.
4. **W5** (cross-asset risk + equity/commodity, XL) — the risk-fact generalization with the binding
   FX `firm_aggregate==single-node` 1e-12 no-regression gate + the FRTB 0.75ρ-class constant gates.
5. **Client surfacing (S-D)** — the shared 5-client/server-routing merge point that serializes all
   waves' "fully landed across 5 clients" milestones.
6. **W6** (rigor uplift across the now-larger surface area + 2nd-gen exotics depth on top of W4-A).
7. **W7** (convergence loop until 2 consecutive dry rounds — the terminal stop).

W2 and W4 are NOT on the critical path (lower-risk breadth / additive workflow) and overlap W3/W5
on disjoint lanes. GW0/GW1 are off the Rust critical path entirely (independent GUI track), but
GW6 (multi-asset GUI, not in these plans) is gated on W1.

---

## 4. The hard-won lessons this map encodes (from the prior programs)

- **Stagger proto edits; never two open at once.** Field numbers are reserved per wave; each proto
  edit is a single staged step gated by convert round-trip + FX `to_bits` byte-identity. (W4
  merge-marker incident; "two waves sharing the proto run sequentially".)
- **`celnet-exotics` serializes exotics-extending waves** (W4-A before W6 depth).
- **Client surfacing is the cross-wave merge point** — front-load all engine/oracle work onto
  disjoint lanes; serialize only the thin SDK/CLI/Excel/GUI/server-routing layer; the 5-client
  conformance harness is the integration gate.
- **Verify the literal `All gates passed.` line**, never the background-wrapper exit code, before
  any milestone push.
- **Re-derive every external constant from primary text in the oracle** (the FRTB 0.75ρ
  circular-oracle bug) — applies to W5's per-class BCBS pins and W3's inverse-measure derivation.
- **Clippy the parity TEST target** (`clippy -p celnet-parity --test <name> -D warnings`), not just
  the crate — a recurring milestone-gate blocker.
- **A shared-type field widening breaks downstream consumers** (the risk-cube fixtures incident) —
  W5's `RiskFact` widening must re-run all downstream FX risk parity rows byte-green.
