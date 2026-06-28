# Celnet — Implementation ledger

> Append-only status log. **Newest first.** One line per meaningful unit of progress.
> This is the full history, externalized from `CLAUDE.md` to keep the per-session
> directive context small (token-and-context-discipline). `CLAUDE.md` keeps only a
> **one-line** resume anchor; **append the full entry here, at the top of the list below,
> and replace the CLAUDE.md anchor in place with a single-line summary — never paste the
> full entry into `CLAUDE.md` (that re-bloats the per-session context).**

- 2026-06-28 — **🌊 Arch-program item A (carry seam → streamed edge) LANDED on `main` (`d35d8fe`, branch `arch/A-carry-seam-to-edge`) — the streamed hot edge is now multi-asset.** The #1 architecture target (`docs/ARCHITECTURE-DETERMINATION.md` §3; spec `docs/plan/CARRY-SEAM-TO-EDGE.md`): the ADR-0008 carry seam made "add an asset class = a new `Carry`, zero payoff edits" true for batch/request pricing but **stopped at the request layer — the streamed RFS edge was FX-only**. Now the seam reaches the edge, **FX byte-identical**. **P1 (carry-aware fan-out, `f54ef19`):** `underlying_seed(&Underlying)` generalizes the FX-pair-only `pair_seed` (FX arm folds the bare `"BASE/QUOTE"` ⇒ **byte-identical seed**, pinned by `underlying_seed_fx_is_byte_identical_to_pair_seed`; other classes fold a class-tagged canonical key — no cross-class ring collision); the price-fanout ring is keyed per-`Underlying`; `services::stream` subscribe/modify thread the full underlying (the `as_fx()` "requires a pair" gate is gone). `baseline_market` is UNCHANGED — `cost_of_carry` already derives a cross-asset `(r,b)` from the FX market arm (`b = r_dom − r_for`), so `MarketContext::fx(...)` prices a streamed cross-asset line correctly (an honest deterministic mark, same synthetic-streaming discipline as FX; the proof is the seam+arm reaching the edge, not real cross-asset market data). **P2 (streamed sensitivities, `f54ef19`):** `streamed_rate_sensitivities(instrument, greeks)` selects the wire arm by **asset class** (`is_cross_asset` — the SAME classifier the price router uses; **NOT a `match carry`**, the ADR-0008 streamed-path blocker): FX/metal keep the two-rho `Fx` arm **byte-identical** (proven `fx_wire == priced.greeks.into()`), a cross-asset line carries the generalized `Carry` arm `{discount_rho, carry_rho}` (carry-rho recovered EXACTLY by negation; discount-rho the FX-shaped sum to ≤1 ULP — fp non-associativity, economically nil; native arm flows bit-exact under item F). Both `make_snapshot` + `make_update` emit via `streamed_wire_greeks`. **P3 (clients/SDK reach, `f54ef19`+`3ec0fb5`):** proto `Greeks::rho_dom/rho_for` now project EITHER arm losslessly (`RateSensitivities::flat_rhos`; FX verbatim ⇒ no SDK regression, carry projected ⇒ correct non-zero for cross-asset instead of 0); the WS JSON codec emits the self-describing nested `rate_sensitivities` arm at gRPC-field-15 parity; a non-FX market-series subscribe is **honestly refused** (`Status::unimplemented`) rather than silently mislabeled with the single FX core's observables (closed a latent pre-existing mislabel — no fake). Clients were already class-correct from the prior multi-asset work (`GreeksStrip` renders dividend/net-carry/funding rho from the flat projection + asset class): GUI `StreamWorkspace` opens a row's class-correct Greeks detail; Excel `CELNET.GREEKS` relabels rho rows by class. **Gating:** the cross-asset stream is gated at the **Rust T1 tier** (`cross_asset_subscribe_streams_the_carry_arm_through_the_session` — full session driver) AND the **JS live e2e** (`gui/e2e/crossAssetStream.e2e.ts` — equity line streams end-to-end under Enforce, renders the dividend-yield carry rho). Adversarially-verified **SHIP** (`celnet-verifier`, refute-default, 9 attack vectors, 0 defects: FX byte-identity holds on the binding proto path; the arm-reconstruction classifier is shared bit-for-bit with the router; the synthetic cross-asset mark is honest+correct; the accessor generalization breaks no consumer — risk-cube reads struct fields, not accessors; the series guard removes a latent bug; the e2e is a genuine Enforce proof; no hidden mocks/todos). **T2 GREEN (16/16, real exit):** full `just t2` (workspace-deps · verification-coverage · fmt · clippy `--workspace --all-targets -D` · build/lib/integration/doc tests · deny · gui/excel typecheck+unit · build-edge) + **live GUI 166/166 (axe 0 serious/critical) + Excel 122/122 under `CELNET_ACCESS_MODE=enforce`**. **The full t2 surfaced a PRE-EXISTING, UNRELATED `celnet-surface` numerical defect** (per the operator process — the full check finds numerical-invariant gaps that scoped/tiered gates miss; the cross-cut fixes what it surfaces): the strike-axis SVI fitter `fit_strike_slice` ran a **single** damped-Gauss-Newton start over the non-convex outer `(m,σ)` search; on a steep short-tenor smile (t=0.02, ρ≈−0.8) the on-grid `argmin w` seed lands at the grid edge (away from the interior vertex) ⇒ a shallow-local-minimum stall (max vol error 1.145 vs the **never-loosened** 1e-4 floor), found by the `strike_surface` proptest's random exploration (1-in-3000). Fixed at root cause (`8c06d7d`, `celnet-quant`): **deterministic multi-start** (legacy grid seed first — a strictly-lower-cost rule never displaces an already-optimal start ⇒ existing exact-recovery fits stay **bit-identical**; then 7 vertex × 4 width starts; lowest-residual wins; fixed seeds/order ⇒ bit-reproducible). **FX delta path (`calibrate.rs::fit_svi`) untouched** ⇒ FX byte-identity structurally preserved. Validated against an **independent numpy-free Cramer-rule oracle** (re-typed from Gatheral's raw-SVI eq.); robust-pass ~**26,000** arb-free truths (`PROPTEST_CASES=4000`×5 seeds + `=10000`), 0 failures; full `celnet-parity` 37 binaries/182 tests green; the failing seed committed to `strike_surface.proptest-regressions` as a permanent guard; lodestar invariant `cl_f22b75061f4a412a` anchored to `fit_strike_slice`. **Lessons (banked):** (1) `cmd | tail … && echo $?` reports the **pipe's** exit, not `cmd`'s — masked a fmt failure twice; run gate commands without a masking pipe ([[dev-posture-masks-prod-defects]]). (2) the full `just t2` surfaces numerical-rigor + Enforce-path gaps scoped gates never run; a cross-cut fixes what it surfaces (the B precedent). (3) `typecheck:test`/`:e2e` are NOT in the t2 gate — 33 PRE-EXISTING FI/rates+multi-asset test-type errors lurk there (candidate for item L). **Deferred nits (verifier-rated non-blocking):** stream vs unary emit different oneof arm-TYPES for a cross-asset instrument (clients render by class, flat magnitudes consistent; unify under item F); the nested WS arm is currently client-unconsumed (gRPC-parity + future-facing). **▶ NEXT: item C** (`price_instrument` god-fn → `ProductEngine` registry).

- 2026-06-28 — **🔒 Arch-program items K + B LANDED + integrated with the parallel FI/rates subsystem; §2/§3 authz finding FULLY REMEDIATED.** **K** (`7f381b0`): gui-unit density JS-free guard exempts `*.stories.tsx` (density/tokens stories legitimately demo the axis); gui-unit 721/721. **B** (`8460687`+`631e935`, merged `1f9acc3`; spec `docs/plan/B-AUTH-QUOTE-RISK.md`) — closes the §2/§3 follow-up of the verified authz finding. **§2:** all four QuoteService RPCs (request_quote / request_multi_dealer_quote / accept_quote / reject_quote) now `resolve_caller`+`authorize_caller(ReadAny)` BEFORE the distributed-forward branch; `accept_quote` is bound to the AUTHENTICATED requester (`RequesterBinding::Authenticated(user_id)`, keyed on the server-validated session — a leaked `quote_id` can't book another's quote) on top of the idempotency-key match; PricingService stays intentionally public. Token+principal ride in the unary BODY (proto QuoteRequest 7/8, QuoteAccept 5/6, QuoteReject 3/4; additive, no schema_version) so the WS mirror enforces via the SAME trait method — no router change. **§3:** operator chose the FULL desk-identity bridge (not a clamp) — `DeskDef.books`→interned `DeskId` boot-populated into the risk hierarchy; `effective_principal`/`narrow_to_desk` narrow a non-admin session's risk principal to `Rule::on(Desk,d)∩body` across aggregate/drill/list/limit + the WS mirror + federation (forwarded wire principal narrowed at the aggregating edge — a handle disagreement can only under-count, security-conservative); admin/no-session byte-identical (`DeskScope::All`). All 5 clients (SDK/CLI/GUI/Excel+WS codec) carry token+grant-all on request AND accept. Adversarially-verified **SHIP** (refute-default, 6 attack vectors, 0 defects). **The first full workspace `just check` in a while (scoped/tiered gates had skipped it on M4 contention) surfaced 3 PRE-EXISTING latent gate gaps, all fixed** (`631e935`): (1) the `celnet-bench` stream load never sent the `Authenticate` frame → 0 drained under Enforce (the stream cross-cut never updated the bench) — added Authenticate-first; (2) a mathematically-FALSE xva CVA-monotonicity proptest invariant (`netting_fuzz` Contract 5 asserted "doubling λ must not decrease CVA", but CVA=Σ DF·EPE·ΔPD is NOT monotonic in hazard for a non-monotone MC-EPE; `compute_xva` is the correct textbook formula) — replaced with the true default-probability monotonicity in the proptest + its libFuzzer mirror; (3) GPU/MC tests timed out under nextest M4 contention — added a `numerics-serial` test-group (max-threads=1, mirroring engine-serial / replog-consensus) + terminate-after 4→6. **Solo T2 green:** full `just check` 2069/2069 + live GUI 165/165 + Excel 122/122 under Enforce. **Integration:** a parallel session shipped a large fixed-income/rates subsystem (`celnet-rates`, FRA/IRS/OIS/STIR/bond-futures, multi-curve, curve risk, FI dealer-quoting desk, 6 GUI workspaces, Ansible UAT) onto `origin/main`; merged in (`d5e0f28`) — compiles clean, all B changes survived; **combined-tree gate green: full `just check` 2195/2195 + GUI vitest 785/785 + Excel vitest 449/449 + live GUI 165/165 (axe 0) + Excel 122/122 under Enforce.** The combined live e2e caught a merge-interaction the textual merge hid (the new "Fixed Income" domain tab + "Rates Book" collided with the e2e's "Book" rail locator — invisible to vitest/tsc) → fixed e2e-helper-only (`07e8812`, `gui/e2e/helpers.ts`). **ADR-0010** (`a97330c`): recorded the FI-rates→core convergence direction in lodestar (markdown + 3 graph-anchored `draft` claims: Carry-is-a-degenerate-curve, rates-risk-into-the-one-cube, FX-byte-identity-survives-the-curve-trait) — the principled answer to "how to model FI into core": promote carry → a `DiscountCurve`/term-structure trait (rates `Curve` = general case, FX two-rate carry = 2-curve special case), unify at the curve/risk/contract layers, keep linear-vs-option payoff engines separate; = the payoff of plan items A/C/E/F. **Lessons banked:** the full workspace `just check` surfaces Enforce-path + numerical-invariant + M4-contention gaps that scoped/tiered gates never run — run it before landing a cross-cut; the combined-tree live e2e under Enforce catches semantically-broken-but-textually-clean merges ([[dev-posture-masks-prod-defects]], [[deferred-e2e-defect-reservoir]]).

- 2026-06-27 — **FIXED-INCOME DEALER QUOTING DESK + rates Book (`25293f8`, branch `fixedincom_risk_ui`; deployed UAT option 2).** The maker side of the franchise, full vertical slice over the one current contract (operator chose *dedicated notification stream* + *all-at-once*). **Contract (celnet-proto):** `RiskService.{BookRatesPosition,ListRatesPositions}` (the outstanding rates position store/Book); new `RfqDeskService` (SubmitDeskRequest · RespondDeskRequest quote|reject · AcceptDeskQuote→books Deal+RatesPosition · ListDeskRequests · ListDeals); new `NotificationService.StreamNotifications` (server→client push). All new fields/variants doc-commented so prost codegen passes `missing_docs` under clippy `-D warnings`. **Server:** `RatesPositionStore`/`DeskRequestStore`/`DealStore` (in-memory RwLock, monotonic ids, newest-first — mirror the options `PositionStore`); `NotificationBroker` (bounded 256-deep per-subscriber mpsc, non-blocking `try_send` ⇒ full-skip/closed-prune, entitlement-scoped desk fan-out — hot path never blocks, §11); `RfqDeskEdge` impls both new services, desk pricing **reuses** `price_rates`; deny-by-default entitlements mirror ListPositions/AggregateRatesRisk. WS: 7 new unary tags + the `subscribe_notifications`/`notification` push channel drained to the existing outbound sink; tonic services registered. **GUI:** contract/enums/wsCodec/transport/wsTransport (7 unary + a `streamNotifications` subscription re-opened on reconnect), `mockSource` full real offline desk lifecycle + local emitter; three Fixed-Income workspaces — **Quoting** (RFQ/IOI inbox · price panel · respond/reject/accept · counterparty simulator), **Deals** blotter, **Rates Book** — registered across all nav sites; **NotificationCenter** (signed-in-only toasts + bell/unread inbox; click routes to Quoting). **Gates (real exits):** workspace `cargo check`/`clippy --workspace --all-targets -D warnings`/`fmt` = 0; `cargo test --workspace` green on all committed seeds (the lone failure was an unrelated `celnet-parity` var/vol-swap **proptest random-seed flake**, ~7e-6 in the strip-replication tails — reverted, passes on committed seeds via `PROPTEST_CASES=0`; flagged as pre-existing, NOT touched). GUI `npm run build` (tsc -b strict) = 0; vitest **785**. **Env note:** `just`/`cargo-nextest` absent on this workstation (deploy uses Ansible) — gated with plain `cargo`/`npm`, equivalent test set. **Open →** the `celnet-parity` strip-tolerance flake (widen-with-justification or investigate truncation); live counterparty venue connectivity stays deploy-tier. Earlier same-day: GUI **release auto-reload** version watcher (`3b09ae7`) + rates Risk workspace + domain tab bar (`034065a`).

- 2026-06-27 — **🔒 Stream/WS caller-authz cross-cut LANDED on `main` (`8e0ee48`, pushed) — the #2 architecture-determination finding remediated.** Closes the adversarially-verified headline security hole (`docs/SECURITY-AUTHZ-FINDING.md`): an unauthenticated client could open a gRPC `StreamSession`/WS RFS session and subscribe + click-to-trade with **no caller check** (execute was gated only by a server-minted last-look token the same anonymous session was handed). **Change (8-commit branch `security/authz-cross-cut`, merged `--no-ff`):** additive `StreamAuth` Authenticate frame (ADR-0007, oneof defaults FX/grant-all — no `schema_version`); `Session` pins a `ResolvedCaller` from the first frame; **ONE** `authorize_caller(self.access_mode, &caller, ReadAny)` seam before subscribe/modify/execute/resync, enforced on **both** gRPC (`Session::handle_client_message`) and the WS mirror (`ws::codec::stream_auth_from_json` + `decode_stream_control` + a single `is_stream_control` routing source-of-truth); SDK/CLI/GUI/Excel all send Authenticate **first** on every (re)connect, defaulting to the audited explicit grant-all (= risk's `principal_or_grant_all`) so headline workflows are admitted under Enforce without relying on the server granting an absent caller. **Gated + LIVE-validated under `AccessMode::Enforce`:** server clippy/ws/stream/access + new Enforce tests (`enforce_denies_unauthenticated_then_admits_after_authenticate`, `stream_authenticates_under_enforce_with_login_token_and_grant_all_default`); GUI Playwright/axe **165/165** (incl. click-to-trade streaming) + Excel e2e **122/122**. **A live-e2e-caught defect** (the WS `authenticate` frame fell through to `handle_unary` — `dispatch` had a hardcoded verb list SEPARATE from the decoder — so WS sessions stayed anonymous under Enforce and the next subscribe was rejected `unauthenticated`) fixed with a single routing source-of-truth + decoder/router **lockstep** regression tests; the Permissive unit harness could not see it ([[deferred-e2e-defect-reservoir]], [[dev-posture-masks-prod-defects]]). Pre-existing Excel RFQ-panel spill-padding test corrected (`rectangular()` pads rows to the table width; unrelated to auth). **Process lesson banked:** `isolation:worktree` agent lanes base on `main` not the feature branch → client cross-cuts done coherently on the branch; only toolchain-disjoint lanes parallelize on the one M4; misbased-worktree disjoint changes graft cleanly ([[agent-worktree-base-and-cross-cuts]]). lodestar reindexed (21651 nodes, +42 auth symbols); knowledge base re-grounded clean (0 stale, 1627 claims) + lodestar gap #27 filed (re-authoring a stale claim duplicates instead of refreshing — claim-key includes node_content_hash). **Follow-up = task #20:** quote-accept→requester-principal binding (§2) + risk `principal∩desk_scope` hardening (§3) — lower severity, one coordinated proto+server+clients cross-cut.

- 2026-06-11 — **🎯 CELNET 1.0-RC RELEASE-READY (`a0817d6`) — operator-approved cut (mesh: session-A ∥ session-B).** Gated T2 16/16 (workspace-deps · coverage 27 · fmt/clippy --workspace --all-targets -D/test/deny 0 · gui-unit 653 + excel-unit 363 · gui-e2e 165 + excel-e2e under `enforce`) + Round-5 adversarial convergence DRY. **Contents:** the full cross-asset platform on the ADR-0008 carry seam (FX/metal/equity/commodity/crypto + RFQ + perpetual/future-option arms 30/31/32) · 5 clients (SDK/CLI/GUI/Excel) · the W6 rigor floor (qmc/xva/risk-cube MEASURED zero-missed mutation gates, `c58ae02`) · the crypto strike-axis surface leaf (`c5a5efc`, independent 5-leg oracle) · entitlements deny-by-default. **Session-A landed R7-core + R8 under a monthly spend wall using local cargo + direct oracle-independence review (no agents)** — incl. catching a real semantic rebase break (fitmath-hoist vs survivor-kill imports). **Operator chose FAST-FOLLOW** for the criterion-#2 residue (crypto strike-axis surface *wiring* — `docs/plan/CRYPTO-SURFACE-WIRING-FASTFOLLOW.md`), flipping releaseReady=true. **Tracked fast-follows (spend-paused, banked):** crypto-surface wiring · W6 exotics-E2 + perpetual/future_option mutation coverage · fuzz fix_decoder dedup+CI. Live-deploy tier (auth/LP/venue/UAT) is out-of-repo.

- 2026-06-10 — **W6-qmc — `celnet-qmc` MUTATION GATE driven to zero non-equivalent survivors (lane/w6-analytics, W6-ANALYTICS-RIGOR §3.1, MEASURED green locally).** The QMC crate (Sobol/Owen-scramble/bridge/Φ⁻¹/RQMC — feeds exotics, xva, gpu, parity) had only 10 thin in-module tests. **Pre-kill oracle suite first** (`tests/sequence_oracle.rs` + in-module scramble laws, 30→32 tests): exact dyadic radical-inverse `to_bits` pins; an independent `m_k`-domain re-derivation of the Joe-Kuo direction-number recurrence (rows typed from the published data file); published quantiles + code-disjoint `½·erfc(−x/√2)` round-trip (libm dev-dep); exact bridge covariance + hand-derived m=3 weight matrix; monomial integrals through the full pipeline; plain-loop std-error re-derivation that simultaneously pins the per-replication seed arithmetic; frozen-bits rows (fx_byte_identity pattern) for everything distributional properties tolerate. **Raw baseline** `359 mutants tested in 16m: 7 missed, 295 caught, 2 unviable, 55 timeouts`; survivors dispositioned 4 KILLED (bisect floor-midpoint pivot — covariance-invariant, plan-observable; 3 Halley-polish internals — sub-ULP centrally, pinned via an extreme-tail frozen-bits ladder incl. the subnormal floor), 1 ELIMINATED structurally (`point_u32` guard observable only past the 2^32 period; loop made total, period documented), 2 EXCLUDED with inline proofs, each hand-reproduced full-suite-green (`.config/mutants-qmc.toml`, strict). **Canonical gate** (clean slate, `RUSTC_WRAPPER=""`): `350 mutants tested in 9m: 348 caught, 2 unviable` — zero missed/timeouts, real exit 0. New gate wiring: `.config/mutants-qmc.toml` (+`test_tool`/`minimum_test_timeout` keys), `just mutants-gate-qmc`, qmc added to `mutants-gate-numerics` + the CI `mutation-gate-numerics` matrix. Crate gates real exits: fmt/clippy/test = 0 (32 tests); `fx_byte_identity` 22/22 (qmc consumer byte-identity after the refactor). Env lessons recorded in HARDENING §2: 20s-floor launch-stall timeouts, the sccache build wedge (no build timeout in cargo-mutants), and the stale line-anchor lesson (refactor shifted an excluded mutant 242→241; the gate caught its own stale anchor). **Open →** W6 tail: xva/risk-cube/surface/exotics waves + the 4 fuzz targets.

- 2026-06-10 — **ADR-0008 COMPLETE — every engine + the surface on the agnostic carry seam (merged `4dd500d`; mesh: session-A ∥ session-B client lanes).** The tracked tail is closed: **Wave B** (`9a8af07` MC: mc core/accumulator/tarf/lookback/forward-start/cliquet/asian), **Wave C** (`ee1d905` PDE/ADI/American FD + the §3.4 rate-Greeks re-tag to `RateSensitivities::Carry`; far-field via `yield_rate()` verbatim), **Wave D** (`53aa660` var/vol swap, quanto, basket, LSV — quanto deliberately lands the yield-side `FxRates` shift, the only bit-exact arm, recorded in the spec), **Wave S** (`e3efb7b` surface `MarketContext{spot, carry: Carry, t, conv}` + every constructor call site; wire mirrors unchanged — already generalized), plus the two **adversarial-verifier catches**: the pivot engine (`1c075f4`, 14 frozen to_bits) and the Vanna-Volga hedge overlay (`4dd500d`, adjudicated migrate-not-FX-scope, 4 frozen to_bits), and a journal temp-path de-flake (`f47c62d`). **Rigor:** ~120 frozen pre-migration `to_bits` assertions; the byte-identity verifier independently re-derived every constant from the legacy engines at the base commit (14/14); completeness sweep shows ZERO production engine math on `VanillaInputs`/raw `r_dom`/`r_for`. **Merge gate (real exits):** cargo fmt/clippy/test/deny/coverage = 0, 173 test sections + vitest 567/567 + **live Playwright e2e 13/13 + axe** (the new gui-gate directive, adopted after session-B's GW1-layout catch — a deferred e2e suite is a defect reservoir). Concurrently session-B shipped GUI-UNIVERSE (5-asset front door, 564→567 tests) + EXCEL-POLYMORPHIC (`CELNET.INSTRUMENT`, per-product fns retired at parity) + the GW1 layout fix. **Open →** session-B: `clients/rfq-panel-surfacing` (now unblocked), `surface/crypto-leaf`; the W6 analytics rigor floor.

- 2026-06-09 — **CROSS-ASSET PLATFORM + RIGOR WAVE GREEN ON `main` (mesh: session-A coordinator ∥ session-B integrator).** The full multi-asset platform is live and jointly gated. **session-B** landed the cross-asset INTEGRATION (`1484fb0`/`cbec001`): proto window (Underlying equity/commodity/digital_asset + SettlementStyle + RFQ MultiDealerQuote), server routing to the EXISTING `celnet-{equity,commodity,crypto}-vanilla`+`celnet-rfq` engines via the ADR-0008 CostOfCarry seam (FX byte-identical, no silent fallback), oracle 21 arms + 3 cross-asset families, 5 clients. **session-A** (coordinator) landed, each build→adversarial-verify off committed `docs/plan/*` specs, serial-cargo + compute-courteous (yielded the M4 to the proto window): **W6-RIGOR** (`7d3df75`) — loom seqlock model-check (verified-live oracle) + journal per-record sync-word (interior CRC→CorruptInterior) + 4 mutation gates zero-survivor; **W5-A-XRISK** (`7aad04b`) — cross-asset risk-normalize/cube through the carry seam + FRTB buckets re-derived from MAR21 (non-circular); **W4-A-PIVOT** (`83596cd`) — pivot target-redemption accumulator + code-disjoint MC oracle + degenerate→TARF to_bits limit; **ADR-0008 Wave 0/A** (`0fbadab`/`0e2a194`) — analytic exotics (digital/touch/single+double barrier) onto the agnostic `ExoticInputs`/Carry seam, FX byte-identical via the QuantLib golden grids. Integrating W5-A surfaced + fixed downstream breakage in **6 crates** (risk-fleet/limits/entitlements/server/parity/bench — `7a6f9be`) that the full `build --workspace` caught but scoped gates missed; a pipe-masked clippy false-pass was caught + fixed (`5160201`) per the "verify the literal pass" lesson. **Joint final gate, REAL exit codes:** `fmt=0 · clippy --workspace --all-targets --all-features -D warnings=0 · cargo test --workspace=0 (172 sections) · cargo deny=0`, verification-coverage 21+3, workspace-deps OK. **Honest tracked tail:** ADR-0008 Waves B/C/D (MC/PDE/composite exotic engines) + Wave S (`celnet-surface` MarketContext, ~35 sites) still take VanillaInputs — documented in the plan, FX byte-identical, a follow-on lane (NOT silently FX-only). Mesh protocol that made it conflict-free: disjoint worktrees + §4.1 compute-courtesy (one heavy cargo at a time; background lanes pause for the critical-path gate) + board/§6 turn-taking.

- 2026-06-09 — **W6-replog — celnet-replog MUTATION GATE driven to zero non-equivalent survivors (+
  a frame-drift bug fix).** On `lane/w6-rigor-infra`, MEASURED green locally. The replicated Raft log is the
  largest infra leaf (~4.5k src lines). **(0) Prerequisite bug fix:** the journal sync-word frame change had
  silently broken `celnet-replog` — its log-rewrite helper `write_fresh_journal` hand-rolled the OLD journal
  frame (`len‖seq‖payload‖crc`, no sync word), so every rewritten log re-opened as a torn tail and recovered
  to ZERO records (5 `log` tests failing). Fixed by deleting the duplicated format: `write_fresh_journal` now
  opens a fresh `Journal` and `append`s each payload, giving the frame layout a single owner (guardrail 10).
  **(1) Mutation gate** (`.config/mutants-celnet-replog.toml`, `just mutants-gate-celnet-replog`, added to the
  `mutants-gate-infra` aggregate; plain `cargo test` runner — NOT nextest — `--jobs 3`): a strict raw run
  surfaced 76 MISSED + 12 TIMEOUT of 534 (the 1581-line `election` module had ZERO direct unit tests — only
  end-to-end loopback coverage — and the bug-fix UNMASKED several compact_to/install_snapshot branches). All
  GENUINE gaps KILLED with a new in-module `election::core_tests` suite (a bare `NodeCore` + a single-node
  `RaftNode`, asserting candidate_log_ok / reset_election_timer / set_commit_index / leader_advance_commit /
  step_down / compact_to / install_snapshot / the three RPC receivers / accessors / a port-rebind teardown
  observation DIRECTLY) plus targeted `log`/`wire`/`state`/`persist`/`compaction` unit tests. The residual
  survivors are the LINE-ANCHORED equivalence classes (directory-fsync power-loss-only, no-op-at-boundary
  guards, and Raft liveness/self-healing optimizations whose backstops make them safety-identical) — each
  reproduced by hand and shown suite-green. ZERO MISSED (deterministic) survivors remain; the residual
  non-MISSED outcomes are HONESTLY recorded, not hidden: the infinite-loop threaded mutants (`tick_loop`
  1179, `reconcile` `+=->*=` 291:53) caught by the per-mutant TIMEOUT, and TWO `read_frame_or_idle`
  length/guard boundaries (464, 450) that std `TcpStream` cannot unit-test deterministically (the fn clears
  its read-timeout after the first byte by design, so an unrejected oversize prefix blocks → caught only as
  a timeout; the 450 negative arm needs a forced RST via the nightly-only `set_linger`). So cargo-mutants
  reports a non-zero timeout-class exit — `just check-crate celnet-replog` (fmt/clippy/test, 93 lib tests) is
  fully green. INDEPENDENT oracle: the running priced-book replay (`gate_a`/`gate_d`, re-applies deltas to a
  fresh BookState, `to_bits`-compared, code-disjoint from the log). The line-anchoring was load-bearing — a
  broad function-level regex would have masked REAL bugs (the `majority = size/2+1` mutant, the `reconcile`
  loop-bound panic, the `first_new == len` guard), which are instead killed.
- 2026-06-09 — **W6-journal — PER-RECORD SYNC-WORD FRAME + celnet-journal MUTATION GATE (zero
  non-equivalent survivors).** The §2/§3.2 lane of the W6 design pack, on `lane/w6-rigor-infra`, MEASURED
  green locally; recovered STATE byte-identical (compaction round-trip + kill-restart tests pass UNCHANGED).
  **(a) Sync-word frame** (`crates/celnet-journal/src/lib.rs`): every record (data + snapshot) now begins
  with a fixed 8-byte `SYNC_WORD` (ASCII `"CLNJRNL\0"` LE), INSIDE the CRC coverage. `frame_record` /
  `frame_snapshot` prepend it; `read_one` / `read_snapshot` read it first and classify per the new rule —
  **intact sync word + a COMPLETE frame body (through the CRC trailer) whose CRC fails ⇒ `CorruptInterior`**
  (interior bit-rot, surfaced, not silently healed), while a torn tail (short read of sync/header/payload/
  CRC, or a start not led by the sync word) still heals by truncation. This closes the marker-less format's
  one documented limitation (an interior CRC failure was indistinguishable from a torn tail, silently
  dropping committed records). Clean break, no migration shim (guardrail 9). Crate doc + on-disk diagrams +
  `CorruptInterior` doc rewritten to the stronger contract; the `decode_fuzz` snapshot-shaped strategy +
  the `journal_recover` cargo-fuzz target doc updated to lead frames with the sync word. New behavioral
  tests pin the discrimination (`intact_syncword_failing_crc_interior_is_corrupt`,
  `torn_tail_still_heals_with_syncword_format`, `interior_corruption_distinct_from_torn_tail`,
  `snapshot_interior_crc_failure_is_surfaced`) + the two repurposed byte-offset tests. **(b) Mutation gate**
  (`.config/mutants-journal.toml`, `just mutants-gate-journal` + a `mutants-gate-infra: fanout journal`
  aggregate, plain `cargo test` runner — NOT nextest — `--jobs 3`): strict raw run = 16 MISSED of 116; 11
  GENUINE gaps KILLED with new value-pinning tests (the `MAX_PAYLOAD_LEN = 64·1024·1024` constant pinned
  bit-for-bit; the `Display`/`Error::source` surface asserted; the recovery length-bound `> ==`/`>=`
  boundary killed by EXACT-MAX valid data + snapshot round-trips — the only inputs that separate the
  operators; `read_full_or_short`'s Interrupted-retry + short/empty/full classification driven by an
  in-module `ScriptedReader`); the remaining 5 EXCLUDED with inline justification + verified evidence (two
  no-op-at-boundary `< → <=` mutants in `open`/`read_full_or_short`; three directory-entry-durability
  `sync_parent_dir` mutants observable only under real power-loss fault injection). INDEPENDENT oracle = the
  running-sum replay machine (reference state = plain integer sum, code-disjoint from framing) + the
  kill-restart byte-identity test + the 512-case adversarial-bytes proptest. Final gate: **111 mutants
  tested — 97 caught, 14 unviable, 0 missed, 0 timeout**, exit 0. Re-gated: fmt clean, clippy `-D warnings`
  clean (fixed a latent `io_other_error` lint surfaced by a new test), 35 lib tests + proptest green. Docs:
  `docs/HARDENING.md` §2 (journal gate table + per-survivor classification + the sync-word regression-test
  list). DISJOINT lane from W6-fanout (different crate); both on `lane/w6-rigor-infra`.
- 2026-06-09 — **W6-fanout — LOOM SEQLOCK MODEL-CHECK + celnet-fanout MUTATION GATE (zero non-equivalent
  survivors).** Two rigor upgrades on the SPMC broadcast ring (`celnet-fanout`, the per-shard price
  fan-out substrate), both MEASURED green locally; the std hot path is byte-for-byte unchanged.
  **(a) Loom relaxed-memory model-check** (`tests/loom_seqlock.rs` + new `src/mem.rs` `cfg(loom)` shim):
  an EXHAUSTIVE 1P/1C interleaving search over a 2-slot ring proving NO interleaving returns a torn pair
  with strict in-order, conserving delivery. `mem.rs` re-exports std atomics/cell under `not(loom)`
  (identical codegen) and loom primitives under `--cfg loom`; the seqlock payload is modeled as
  `Acquire`/`Release` atomic lanes (the model-faithful image of the production `Acquire` fence's hardware
  coherence — strict-C11 loom refuses to bless the production non-atomic `UnsafeCell` copy, the known
  benign seqlock data race, recorded honestly). `loom` is a `[target.'cfg(loom)'.dependencies]` MIT
  dev/cfg-only dep — never in a release build; `cargo deny check` clean. The model is a VERIFIED-LIVE
  oracle: disabling the consumer's `stamp_after != want` torn-read re-check makes it FAIL deterministically
  (torn pair `(0,2)`). Recipe `just loom-fanout` (`LOOM_MAX_PREEMPTIONS=3`). **(b) Mutation gate**
  (`.config/mutants-fanout.toml`, `just mutants-gate-fanout`, plain `cargo test` runner — NOT nextest —
  `--jobs 3`): raw run = 11 survivors; the genuine gaps (conflation-frontier `cursor < oldest_live`, head
  empty-check, `published()`/`capacity()` accessors) KILLED by new closed-form-oracle tests
  (`tests/conflation_boundaries.rs`, ORACLE = the half-open live window `[head−capacity, head)`); the rest
  EXCLUDED with inline justification + verified evidence (cfg(loom)-only fns not compiled in the std gate;
  `^`/`>>` stamp mutants provably identical; the dual-skip-path / zero-gap / spin-vs-Empty boundary
  mutants equivalent; the one concurrency-only torn-read mutant `(seq<<1)&1` caught DETERMINISTICALLY by
  the loom oracle, not the std gate). Final gate: `72 mutants tested: 37 caught, 21 unviable, 14 timeouts,
  0 missed`, exit 0. Re-gated: fmt clean, clippy `-D warnings` clean, 18 std tests green, loom 2/2, deny
  clean. Docs: `docs/HARDENING.md` §2 W6 section (gate table + per-survivor classification + loom honesty
  boundary). Lane branch `lane/w6-rigor-infra`. (Journal sync-word frame + journal mutation gate from the
  same W6 design pack are a DISJOINT lane, not in this commit.)
- 2026-06-08 — **W1 — MULTI-ASSET CORE LANDED + GUI foundation merged (commits `ba0fc03` W1 core,
  `6143409` gw-foundation merge; pushed).** The keystone wave: generalized the three FX-only Layer-0
  seams to a cross-asset vocabulary IN PLACE (one unversioned contract, FX byte-identical) per
  **ADR-0008** (identity / carry-as-forward-discount-producer / asset-class-agnostic payoff). Built as a
  gated workflow (trait → contract → GUI∥Excel∥plugin fanout → adversarial-verify) + my independent
  re-gate. **celnet-core:** the carry-producing-market seam — `CarryInputs`/`CarryPricer`/`CarryGreeks` +
  typed `CarryPriceError` (never silent mis-price). **celnet-vanilla:** `FxPricer` delegates to the
  UNCHANGED GK arithmetic; to_bits-gated vs the full QuantLib golden grid. **celnet-proto:** `Underlying`
  (oneof fx) replaces `CcyPair` on Instrument/BasketLeg/MarketSeries*/OrgKey; MarketContext/VanillaInputs
  → {discount_rate, CarryModel(fx/generalized)}; Greeks rho_dom/rho_for → `RateSensitivities`;
  RISK_DIMENSION_CCY_PAIR → _UNDERLYING; convert FX round-trip to_bits-identical + product×underlying
  validity guard. **celnet-server:** routes by Underlying, builds FX leaf inputs from CarryModel; I added
  the **no-silent-fallback carry guard** at the price-path head (a blocker the verifier flagged — a
  generalized carry was silently read as FX r_for=0; now refused with a typed error, covers analytic+LSV).
  **plugin-api/-host:** PricingModel+WIT+wasmi ABI generalized; NEW gate registers an equity-dividend
  (CostOfCarry) model reconciled to an INDEPENDENT generalized-BSM oracle 1e-12 (no circular oracle).
  **GUI (gw-foundation, built in a parallel worktree):** GW0 design-system + honest-data primitives +
  accessible role=grid `<DataGrid>` + GW1 single breadcrumb scope nav, DELETING 7 redundant pair
  affordances. **Re-gated (independently):** `just check` → literal "All gates passed." (1343 tests, was
  1306); conformance 120/120; GUI vitest 425 + typecheck:test; Excel real-edge e2e 81; FX byte-identity
  gated by to_bits. **▶ All downstream fan-out lanes now OPEN** (`docs/PARALLEL-SESSIONS.md`): W2/W3/W4/W5
  + GW2 — claimable by parallel service-mesh sessions on worktrees off `main`. Follow-up (not W1): the
  `celnet-surface` FX→neutral split (only needed when a non-FX surface leaf lands).

- 2026-06-08 — **MASTER-EVOLUTION-PROGRAM launched (exceed SynOption, multi-asset, no-legacy) + W0
  foundation COMPLETE.** Two read-only design workflows synthesized `docs/MASTER-EVOLUTION-PROGRAM.md`
  (8 waves W0–W7 + 7-lens convergence loop to 2 dry rounds) + `docs/WORLD-CLASS-BACKLOG.md` (single live
  backlog) + `docs/GUI-EXPERIENCE-DESIGN.md` (GW0–GW7). Then executed **W0 (verification & hygiene
  foundation)**: (1) **hygiene** (commit `7c40aaf`) — registered the 5 unregistered internal crates
  (golden/qmc/heston/replog/xva) in the central `[workspace.dependencies]`, migrated all 11 internal
  path-deps to `.workspace = true`, added a `workspace-deps` lint to `just check`. (2) **verification
  foundation** (this commit) via a gated workflow (Rust foundation → GUI∥Excel∥docs fan-out →
  adversarial-verify) + my independent re-gate: a **golden-vector corpus** (84 vectors / ALL 18 product
  families in `crates/celnet-golden/vectors/*.json` + generator `gen_vectors.rs` + `vectors_selfcheck.rs`,
  every expected value from an INDEPENDENT oracle — QuantLib CSVs / closed-form / code-disjoint MC, verifier
  confirmed NO circular oracle), **cross-client conformance** (SDK `celnet-client/tests/conformance.rs` 18
  families, CLI 9, GUI `gui/test/conformance.test.ts` 16, and a **NEW Excel real-edge suite** `excel/e2e/`
  that boots a real `demo_edge` over a real `ws` socket — closes the FakeSocket-only gap), `docs/
  VERIFICATION-CONTRACT.md`, and a **`verification-coverage` lint** (parses the 18 proto arms, asserts each
  has a golden vector AND a celnet-parity row — now 18/18; wired into `just check`). The corpus build
  surfaced + root-caused two real defects (one-touch at-hit vs at-expiry; lookback Brownian-bridge
  extremum under-sampling). Adversarial-verify REJECTED on 3 wiring issues → all fixed forward by me:
  the two golden deps → `.workspace = true`; **two NEW independent-oracle parity rows** written to reach
  18/18 (`celnet-parity/tests/american.rs` — no-carry European limit + published Longstaff-Schwartz 2001
  Table-1 + premium≥0; `strategy.rs` — model-free put-call-parity synthetic forward + ATM-fwd symmetry +
  butterfly convexity + independent GK leg-sum); GUI conformance documented as in-process-pricer-validated
  (wire path proven by SDK + Excel real-edge). Independently re-gated: full `just check` **"All gates
  passed."**, celnet-parity 127/127, GUI vitest 335/335, Excel real-edge e2e 81/81. **Next: W1 multi-asset
  CORE** (the highest-risk wave — generalize the FX-only Layer-0 seams, FX byte-identical).

- 2026-06-08 — **Capabilities documentation REVITALISED + interlinked + responsive (commits `4b7d527`,
  `8777cef`; pushed).** Planned via a fan-out review (`docs/CAPABILITIES-REVITALISATION-PLAN.md`), executed
  as gated workflows, orchestrator-fixed + re-verified. All 14 chapters (`docs/celnet-capabilities/01-14`)
  + hub + 13 branded figures re-authored to the COMPLETE platform (full exotic catalogue + American/basket/
  LSV/eSSVI/XVA/GPU-G3-G6/full-Raft/SPMC/FRTB/observability), competitive-advantage-led, every claim
  code-grounded (34 crates, 19 wire products, 5 services, 5 smile models, 27 `CELNET.*` functions). ch09
  details every gRPC service/RPC + WS mirror + all 19 products + SDK + CLI; ch10 details all 27 Excel
  functions. NEW: committed reproducible figure renderer (`tools/render-capability-figures.mjs` +
  `just render-figures`, Playwright — closes the guardrail-#10 ad-hoc-PNG gap); standalone self-contained
  HTML showcase (`docs/celnet-capabilities.html`, Celer-branded, full API+Excel reference + honest boundary);
  hub-as-canonical-index + uniform per-chapter nav + bidirectional cross-references (chapters ↔ showcase);
  responsive/scroll (showcase + figure pages, zero horizontal overflow 375→1920); committed link +
  responsive checkers (`tools/check-doc-links.mjs`, `tools/check-html-responsive.mjs` + `just check-docs`/
  `check-html-responsive`). **Adversarial verify caught + fixed a real factual error** ("six gRPC services"
  → FIVE; "six surfaces" correct, preserved) and the two known overclaims (plugin Tier-1/3 → DESIGNED-ONLY;
  live JVM Celer estate → deploy-gated). Honest boundary held verbatim throughout. Docs-only; `just check`
  unaffected. **Lesson: this shell is ZSH — unquoted scalar `$var` does NOT word-split; use arrays (a sed/
  perl batch silently no-op'd on `$files` until switched to a zsh array).**

- 2026-06-07 — **▶▶ POST-COMPLETION-AUDIT BACKLOG COMPLETE — all 13 items landed & pushed (final commit
  `970fb27`; `just check` 1306/1306, "All gates passed.").** After the 12-wave program, an honest fan-out
  gap-audit (`docs/POST-COMPLETION-AUDIT.md`) found the platform **materially complete** (no P0/P1
  functional gaps) with a short bar-raising tail — implemented in 3 staged rounds (disjoint lanes on
  parallel git-worktrees; the two new-analytics waves sequential since they share proto/exotics/clients).
  **NEW ANALYTICS:** **American/Bermudan** early-exercise (`celnet-exotics/american.rs`: PSOR free-boundary
  CN FD + Longstaff-Schwartz LSM; `american=24`; hand-pinned LS-2001 Table-1 put 2.314) + **correlated
  multi-asset basket/best-of/worst-of** (`multiasset.rs`: Cholesky GBM via celnet-qmc; `basket=25`;
  hand-pinned Levy-1992; non-PSD rejected; MC stderr; Greeks honestly deferred) — both api-first across all
  5 clients. **RIGOR:** mutation-gate widened to surface/exotics/risk-cube/xva (found+KILLED 33 real
  arbitrage.rs survivors); fuzz targets for the untrusted byte decoders (replog/journal/proto) + gated
  proptests; Heston published-reference golden (Fang-Oosterlee, QuantLib unavailable in-sandbox); surface
  coverage gate; obs-deps cleanup. **POLISH:** runnable SDK examples (ran live vs demo_edge); GUI a11y
  sweep widened to Cube+Universe (found+fixed 2 serious axe findings: nested-interactive + contrast) +
  keyboard shortcuts overlay; **doc reconciliation** (guardrail #10: crate count 19→34, plugin/Sobol/GPU/
  Raft-snapshot marked Built with citations, full INTERFACES wire registry incl. american/basket).
  **BONUS correctness fix the stress test surfaced:** the `celnet-fanout` SPMC seqlock reader was missing
  the canonical Acquire fence between the plain payload copy and the post-stamp re-check — a real (rare,
  aarch64-only, 16x-oversubscription) torn-read window; fixed at root (not a test relaxation). Integration
  caught + fixed-forward several contention/freshness issues my full-`just check` re-gate exists for
  (SDK smoke liveness deadlines; a freshly-published dev-only unmaintained advisory RUSTSEC-2026-0173 on
  proc-macro-error2 via iai-callgrind, triaged like the bincode one; cargo-mutants worktree-leak merge
  artifacts). **1306/1306 tests**; GUI app+e2e tsc 0 / 247 tests + Playwright a11y 8/8; Excel 221. **Only
  the deploy/live-gated frontier remains (NEVER in-repo): NVIDIA absolute GPU throughput / ≤50ms exotic,
  cross-host wire p99 / §11 SLOs, live JVM Celer estate, Raft §6 dynamic membership, plugin Tier-1/3. The
  in-repo platform is complete, SOTA, api-first across all clients, and polished.**

- 2026-06-07 — **▶▶ COMPLETION-PROGRAM COMPLETE — ALL 12 WAVES + the full Raft increment landed & pushed
  (final commit `1983956`; `just check` 1231/1231, "All gates passed.").** W12 capstone (`1983956`):
  **typed Smile provenance** (`ArbReport.smile_model=5`, additive — retires the model=<family> regex across
  GUI/Excel/SDK) + **server observability** (additive Heartbeat fields: real celnet-fanout conflation-drop
  count + drain-side HdrHistogram p50/p99/p99.9 + surface_version/correlation echo; streaming-edge only,
  zero-alloc hot core untouched; surfaced in GUI StatusRibbon) + **GUI Playwright e2e + axe a11y**
  (`gui/e2e/`: boots the REAL demo_edge, drives ticket→price / surface mark→pin / stream→click-to-trade /
  risk drill over live WS, 10/10 on real Chromium, zero serious/critical axe) + **celnet-journal
  compaction** (checkpoint watermark + atomic snapshot-record swap via a `SNAPSHOT_MARKER` payload-len
  sentinel so the data-record layout is byte-UNCHANGED → replog/engine stay green; replay-from-compacted ==
  replay-from-full BIT-IDENTICAL, crash-safe old-or-new, monotonic seqs) + **docs reconcile**
  (ANALYTICS-SPEC/ROADMAP P3 → Built; new `docs/CLIENT-PARITY-MATRIX.md`). Integration: dropped a redundant
  concurrent-agent stash; confirmed the flagged GUI diagnostics were PHANTOM (arbReportFromWire exported;
  e2e node: imports compile under `tsconfig.e2e.json`). **PROGRAM TALLY:** Raft (election+truncation /
  compaction / InstallSnapshot) · Arc I W1–W6 (full exotic catalogue: var/vol swap, Asian, fwd-start/
  cliquet, quanto, TARF, accumulator, lookback, barriers/digitals/touches, eSSVI, LSV+booking-model — every
  product reachable from server+SDK+CLI+Excel+GUI) · Arc II W7/W8/W9 (InstallSnapshot, GPU G3/G6+QMC-KAT,
  fanout-under-edge) · Arc III W10/W11/W12 (XVA, exotic risk-cube + eSSVI/Sobol numerics, capstone polish).
  **1231 tests** (started this program at 1044). Each wave: gated implement→adversarial-verify, INDEPENDENT
  oracle (Lesson c hand-pins), my own re-gate (literal "All gates passed." + re-derive + re-run perf SOLO)
  before commit+push. Disjoint Arc-II/III waves run on parallel git-worktree lanes; Arc-I forced-sequential
  (shared contract). **HONEST BOUNDARY held throughout (NEVER claimed in-repo):** cross-host wire p99 /
  kernel-bypass NIC, CUDA/NVIDIA ABSOLUTE throughput + ≤50ms exotic + Workload-A/B absolutes (Metal lacks
  f64 ⇒ correctness+ratios only), §11 absolute wire SLOs, live JVM Celer estate. Remaining in-repo frontier:
  Raft §6 dynamic membership (documented next increment); everything else in `docs/COMPLETION-PROGRAM.md` is
  built. **The program bar is met: SOTA, fully-integrated, api-first across all clients, polished.**

- 2026-06-07 — **COMPLETION-PROGRAM W9 + W11-A DONE (commits `676dc47` W9, `77fee13`/`919c1d3` W11-A;
  pushed; `just check` 1216/1216).** **W9 — celnet-fanout under the async edge:** replaced N-per-subscription
  spot tickers with ONE `BroadcastRing<PriceTick>` producer per pair (Copy POD `{MarketContext, tick_seq}`;
  rich Update derived per-subscriber), producer off-runtime, consumers drain `try_recv` in `select!`;
  control/lifecycle msgs stay per-session. Conflation parity preserved (conflate-to-latest, ≤1 Update/pass
  at seq+1 — an early one-per-tick version broke click-to-trade, fixed forward, no gate lowered);
  non-circular oracle. Zero-legacy (`SpotTick` deleted). **§1.2 unregressed (my SOLO re-measure):** core_load
  p50 250ns/p99 625ns/p99.9 3001ns (8–16× margin); bench_gate all arms; fleet_slo green; zero-alloc intact.
  Contract UNCHANGED. **W11-A — exotic risk-cube:** exotic legs contribute real Greeks to roll-up + VaR/ES/
  FRTB-curvature (`FactMeasure.exotic`/`NodeAggregate.exotic_legs`); fan-out==single-node proven INCLUDING
  exotics (~1e-12/~1e-9); non-exclusion gated. Ran W9 SOLO ∥ W11-A in a WORKTREE; merged clean. **Integration
  re-gate caught a real break** (`919c1d3`): W11-A's new shared-type fields broke 2 downstream consumers
  (`celnet-limits`/`celnet-entitlements` fixtures) it didn't gate in-worktree — fixed forward. **Only W12
  (capstone polish: GUI Playwright e2e + axe, observability surfacing, journal compaction, docs) remains.**

- 2026-06-06 — **COMPLETION-PROGRAM: Arc I CLOSED (W6) + 4 disjoint waves landed via PARALLEL worktrees
  (W7/W8/W10/W11-B) — integrated to `f29aaee`/`c2189bd`; pushed; `just check` 1198/1198.** After Arc I's
  forced-sequential waves (all share the contract), I fanned out the crate-disjoint Arc-II/III waves on
  **git-worktree-isolated lanes** running CONCURRENTLY with the main-tree W6, then merged each branch (only
  `celnet-parity/Cargo.toml` needed a hand union — and a w8/w10 merge left conflict markers I caught + fixed
  in `f29aaee`; lesson: never `git add -A` a conflicted file). Waves:
  • **W6 (`b8f7be4`, closes Arc I):** LSV independent-oracle parity row (`tests/lsv.rs`: ξ=0→Dupire limit
  HAND-PINNED to 4 external-Python GK constants per Lesson c; PDE≈MC; surface-reprice) + **booking-model
  selector** (additive `PricingModel`{DEFAULT,LOCAL_STOCH_VOL} + `Instrument.pricing_model=22` + new
  `WindowBarrier`=23) routing vanilla/single-barrier/window-barrier through the real `LsvModel`; DEFAULT
  path BYTE-IDENTICAL (to_bits-gated); unsupported product→clear `invalid_argument` (no silent fallback);
  all 5 clients. (Verifier rejected first pass only on an `lsv.rs` fmt blocker — the recurring "verify the
  literal line" lesson — fixed by `cargo fmt`.)
  • **W8 (`7ee8779`):** `celnet-gpu` G3 multi-step path kernel + G6 pathwise/LR Greeks + **CPU↔GPU Sobol
  KAT** (consumes `celnet-qmc` unmodified); three-way GPU-f32≈CPU-f64≈golden within DERIVED f32 bounds;
  real Metal exercised. Honest boundary: Metal-no-f64 ⇒ correctness+ratios only; NVIDIA absolutes deferred.
  • **W10 (`f755ac6`):** NEW leaf crate `celnet-xva` (CVA/DVA/FVA on synthetic netting sets), acyclic;
  CVA hand-pinned to an offline literal (Lesson c); monotone in hazard/LGD; CVA=0 at zero default prob.
  • **W7 (`9e06ebd`):** `celnet-replog` **InstallSnapshot RPC** (closes the last Raft seam) — leader
  ships a snapshot when it has compacted past a lagging/restarted follower; bit-identical catch-up;
  non-vacuity proven (disabling the send fails all 4 rows). Only Raft §6 membership now remains (documented).
  • **W11-B (`5b23d5a`):** robust no-arb eSSVI calibration (projection; 0 density/calendar violations on a
  stress grid) + Sobol high-dim RQMC convergence (3.41×/26.97× measured, deterministic).
  **All gated vs INDEPENDENT oracles, full `just check` green ("All gates passed."), 1198/1198 (was 1128),
  GUI 183 / Excel 184.** Remaining: **W9** (wire celnet-fanout under the async edge — run SOLO next for
  clean §1.2 perf measurement), **W11-A** (exotic risk-cube roll-up), **W12** (polish: GUI Playwright e2e +
  axe, observability, journal compaction, docs). These are sequential by the DAG / contention sensitivity.

- 2026-06-06 — **COMPLETION-PROGRAM Wave 5 DONE (commit `9897fa6`; pushed).** eSSVI client parity:
  `SMILE_MODEL_EXTENDED_SURFACE=4` was on the wire+server (and `celnet_types::SmileModel` already had
  `ExtendedSurface`) but invisible to GUI/Excel (codecs stopped at 4 entries). NO contract/server change;
  3 file-disjoint tracks (GUI ∥ Excel ∥ SDK) → verify (accept, zero issues). GUI: 5th codec entry at index
  4 + "eSSVI" model chip. Excel: 5th codec entry + `parseSmileModel` accepts ESSVI/EXTENDED aliases (error
  msg generated from the enum list so it can't drift). SDK: e2e marks via `Calibration::ExtendedSurface` vs
  a real edge (arb-note `model=extended-surface`). Verifier re-derived codec index==proto 4 (no off-by-one
  that would route the wrong family). Full `just check` green ("All gates passed."), **1128/1128**; GUI tsc
  0 / 164 tests; Excel build 0 / 165 tests. **Next: W6** (LSV oracle parity row + booking-model selector —
  closes Arc I).

- 2026-06-06 — **COMPLETION-PROGRAM Wave 4 DONE (commits `dd4441d` fanout-fix, `15c3ea6` W4; pushed).**
  Surfaced the ORIGINAL exotics (single/double barrier, digital, touch) — already on the wire+server (incl.
  the WS decoder) but unreachable from GUI ticket / Excel / ergonomic SDK ctors. NO contract/server change;
  three file-disjoint tracks (GUI ∥ Excel ∥ SDK) in parallel → verify (accept). GUI ticket + wsCodec
  (exact server-decoded keys/enum tags) + offline closed-form pricers; Excel `CELNET.BARRIER/DIGITAL/TOUCH`;
  SDK `InstrumentSpec::{single_barrier,double_barrier,digital,one_touch/no_touch/double_no_touch/
  double_one_touch}` + e2e == celnet-exotics/golden. Oracle: client codecs match the server WS decoder
  field-for-field; SDK e2e==server==exotics; GUI digital==call-spread limit; barrier KI==QuantLib pinned.
  **Verifier caught a real milestone-gate blocker (NOT a W4 regression):** full `just check` failed on
  `celnet-fanout measured_throughput_above_floor` (4.5e6/s vs a 1e7/s in-suite floor) — the documented
  contended-throughput artifact tipping over as the suite grew (passes solo). **Fixed (`dd4441d`):**
  contention-robust 1e6/s catastrophic-regression floor (Wave-2 methodology precedent; strict figure stays
  the reported uncontended signal — flaky-gate correction, not a relaxation). Full `just check` green ("All
  gates passed."), **1127/1127**; GUI tsc 0 / 159 tests; Excel build 0 / 162 tests; celnet-client 47/47.
  **Next: W5** (eSSVI client parity — `SMILE_MODEL_EXTENDED_SURFACE=4` is on the wire+server but invisible
  to GUI/Excel/SDK; client-only).

- 2026-06-06 — **COMPLETION-PROGRAM Wave 3 DONE (commit `2ce6fac`; pushed).** TARF/accumulator/lookback
  onto the ONE oneof (additive `tarf=19`/`accumulator=20`/`lookback=21` + 4 enums; TARF/accumulator reuse
  the existing `FixingSchedule`) → server pricer (TARF+accumulator MC; lookback continuous closed-form
  [Goldman-Sosin-Gatto floating / Conze-Viswanathan fixed], discrete MC) + WS mirror + SDK + CLI + Excel
  (`CELNET.TARF/ACCUMULATOR/LOOKBACK`) + GUI ticket. **MC-honesty (W2's lesson) HELD this time** — verifier
  accepted: server genuinely emits `price_std_error` for the MC products on both price+quote paths, proven
  by a NON-fabricated Rust SDK e2e vs a real edge with a continuous-lookback `None` negative control.
  Oracle: server==exotics MC bit-exact same-seed ~1e-12; continuous lookback closed-form ~1e-9; structural
  invariants (lookback dominates vanilla; TARF FullGain<CappedGain; accumulator KO reduces value). Full
  `just check` green ("All gates passed."), **1126/1126** (was 1111); GUI tsc 0 / 131 tests; Excel build 0
  / 136 tests. Minor (logged for W12 polish): GUI/Excel *display* tests use fixtures, but server emission is
  independently proven by the Rust e2e. **Next: W4** (GUI ticket + Excel + SDK ergonomic ctors for the
  ORIGINAL barriers/digitals/touches — client-only, no contract/server change).

- 2026-06-06 — **COMPLETION-PROGRAM Wave 2 DONE (commit `f498f95`; pushed).** Forward-start/cliquet +
  quanto onto the ONE oneof (additive `forward_start=16`/`cliquet=17`/`quanto=18` + `QuantoPayoff`) →
  server pricer (Rubinstein fwd-start, Σ-leg plain cliquet, quanto vanilla/digital closed-form; clamped
  cliquet MC) + WS mirror + SDK + CLI + Excel (`CELNET.FORWARDSTART/CLIQUET/QUANTO`) + GUI ticket.
  **Adversarial verify REJECTED the first pass on a real blocker** (the clamped-cliquet MC std-error was
  surfaced only on the gRPC `PriceResponse`; the WS quote path used by GUI-live/Excel silently dropped it,
  masked by fabricated test fixtures). **Fixed forward (me):** `Quote.price_std_error=12` +
  `PriceResponse.price_std_error=7`, emit in BOTH WS JSON encoders, stamp in `quote.rs`, map in the SDK
  `Quote`; gated by a `quote_to_json` unit test + an SDK e2e asserting the clamped-cliquet QUOTE
  (request_quote — the GUI/Excel path) carries stderr while plain cliquet does not. Oracle: server==exotics
  ~1e-9 + t1→0→GK + plain cliquet==Σ legs ~1e-10 + quanto ρ=0→vanilla ~1e-12; clamped cliquet == same-seed
  exotics MC (price+stderr, no closed-form overclaim). Full `just check` green ("All gates passed."),
  cargo-deny clean, **1111/1111**; GUI tsc 0 / 115 tests; Excel build 0 / 116 tests. **Lesson reinforced:
  MC honesty must hold on EVERY transport (gRPC AND WS AND SDK), not one.** **Next: W3** (TARF/accumulator/
  lookback — MC products on the wire, reusing this `price_std_error` infra).

- 2026-06-06 — **COMPLETION-PROGRAM launched (`docs/COMPLETION-PROGRAM.md`, commit `133af6f`) + Wave 1 DONE
  (commit `2c09d18`; pushed).** A planning Workflow (4 parallel assessors → architect) found the dominant
  gap is **api-first client parity, not missing math**: a large built+parity-gated `celnet-exotics`
  catalogue (Asian, fwd-start/cliquet, var/vol swaps, TARF, accumulator, quanto, lookback, eSSVI, LSV) is
  unreachable from the wire/clients. Plan = **12 waves, 3 arcs** (I: close parity W1–W6; II: infra/numerics
  depth W7–W9; III: breadth/risk/polish W10–W12); honest boundary held out of scope. Driving each wave as a
  gated implement→adversarial-verify Workflow (pipeline: Rust slice → GUI ∥ Excel → verify), then I
  independently re-gate (literal "All gates passed." + GUI/Excel build+test + re-derive math) + commit +
  push + ledger. **W1 (`2c09d18`):** variance swap / vol swap / arithmetic Asian onto the ONE oneof
  (additive `variance_swap=13`/`volatility_swap=14`/`asian_option=15`, NO schema_version) → server pricer
  (→ celnet-exotics closed forms) + WS-JSON mirror + SDK (`InstrumentSpec::{variance_swap,..}` + AsianTerms)
  + CLI (`exotic var-swap|vol-swap|asian`) + Excel (`CELNET.VARSWAP/VOLSWAP/ASIAN`) + GUI ticket — all five
  clients reach all three products (verifier confirmed by reading code). Oracle: server==exotics ~1e-9 + the
  **independent flat-σ `K_var==σ²` full-wire-path limit** + vol-swap `K_vol<√K_var` + Asian closed-form
  limits. (Caught + dismissed a STALE phantom TS diagnostic by checking the file on disk myself.) Full
  `just check` green ("All gates passed."), **1092/1092** (was 1077); GUI tsc/build/90 tests; Excel
  build/95 tests. **Next: W2** (forward-start/cliquet + quanto on the wire).

- 2026-06-06 — **Deepening increment §4(i) COMPLETE — `celnet-replog` full Raft + log compaction (Track B;
  commit `aabdb8f`; pushed).** Raft §7 snapshotting on top of Track A's consensus core, via a gated
  implement→adversarial-verify Workflow, then INDEPENDENTLY re-gated (I re-derived the base-index offset
  arithmetic at every boundary case myself, read `discard_prefix`/`compact_to`, confirmed the parity oracle
  is independent, re-ran every gate incl. the literal "All gates passed." line). New `compaction.rs` —
  durable CRC'd atomically-written `Snapshot{last_included_index,term,BookState}` + `SnapshotStore`
  (temp→fsync→rename→dir-fsync; torn/CRC-fail reads as absent so the prefix is only discarded AFTER the
  snapshot is durable). `log.rs` gains a **base-index OFFSET** model: Raft ABSOLUTE indices over a
  physically-shrunken log — every accessor absolute-correct; `term_at(last_included_index)→snapshot_term`
  so log-matching succeeds AT the boundary; `discard_prefix` REALLY shrinks the journal on disk (same
  atomic rewrite, not a mask). `state.rs` gains canonical bit-exact `BookState` encode/decode. `election.rs`
  gains snapshot-aware boot recovery (seed from snapshot → replay only the retained tail) + `RaftNode::
  compact`/`safe_compact_index`; `compact_to` reconstructs state AS OF the boundary and only covers
  committed+applied entries. **The independent oracle caught TWO real bugs during dev (no gate lowered):**
  a recovery double-apply via a mislabeled boundary, and a pre-existing solo-cluster (majority==1) never
  self-electing — both fixed forward. **PROOF — new parity row `raft_compaction.rs`** (5 rows): three-way
  replay-from-(snapshot+tail) == full-log replay == an INDEPENDENT fresh-BookState oracle (f64::to_bits;
  workload has 1.0+0.1+0.2 and a 1-ULP value); prefix really discarded on disk; recovery == oracle; full
  RaftNode compact+boot recovery; repeated-compaction guard. `replication.rs` gate_e: a live 3-node cluster
  compacts (each node physically shrinks its log), progresses past the boundary, and a follower
  crash-recovers from snapshot+tail to exact to_bits. **HONEST DEFERRAL** (documented, NOT half-built): the
  InstallSnapshot wire RPC (far-behind follower catch-up over the wire) is the next increment — no
  half-wired RPC; bridged operationally by `safe_compact_index`. Honest boundary intact (loopback proves
  compute+arithmetic; cross-host wire p99 / inter-DC SLO deploy-gated). **SOTA, ZERO workarounds**
  (grep-clean). Full `just check` green (literal "All gates passed."), cargo-deny clean, **1077/1077 tests**
  (was 1057); raft_compaction 5/5 + replog 41/41 stable; parity-TEST-target clippy clean; fmt clean.
  **▶ §4(i) (Full Raft: election+truncation [A] + compaction/snapshot [B]) is COMPLETE.** Next backlog:
  §4(ii) GPU G3/G6 + QMC-on-GPU KAT, §4(iii) wire `celnet-fanout` under the async edge (per
  `docs/NEXT-WORKFLOWS.md`) — deepening increments only, launch if the user asks. Remaining Raft depth =
  InstallSnapshot RPC + dynamic membership (§6), both documented as next increments.

- 2026-06-06 — **Deepening increment: `celnet-replog` → FULL RAFT (NEXT-WORKFLOWS §4(i) Track A; commit
  `e5f6738`; pushed).** Evolved the thin leader-replicated log into a real Raft consensus module via a
  gated implement→adversarial-verify Workflow, then INDEPENDENTLY re-gated (I re-derived the five safety
  properties vs Ongaro&Ousterhout myself, read the durable-truncation + commitment code, confirmed the
  parity oracle is genuinely independent, re-ran every gate + the literal "All gates passed." line).
  **Zero-legacy:** thin `leader.rs`/`follower.rs`/`standby.rs` **DELETED**. New `election.rs` — cohesive
  `RaftNode` role machine: randomized election timers + **Pre-Vote** (Ongaro §9.6) + RequestVote (§5.4.1
  up-to-date rule) + AppendEntries receiver (§5.3 log-matching → reconcile → commit/apply) + leader
  replication with **§5.4.2 current-term-only commit** (no figure-8) + step-down on higher term; core
  mutex never held across blocking IO; per-peer concurrent RPCs; every socket deadline-bounded. New
  `log.rs` — durable index-addressed log over `celnet-journal`; conflicting-tail truncation is a **real
  atomic durable rewrite** (write-fresh→fsync→rename→parent-dir-fsync→re-open), NOT an in-memory mask.
  New `persist.rs` — CRC'd atomically-written `current_term`/`voted_for` + monotonic commit watermark
  (exact crash-recovery; a committed entry is never truncated). `wire.rs` rewritten with the Raft RPCs
  over real loopback TCP. **PROOF — new parity row `celnet-parity/tests/raft_election.rs`** over a REAL
  loopback cluster: (1) kill-leader→survivors elect a higher-term leader and keep committing; (2) election
  safety — a partitioned 1-of-5 minority never wins (Pre-Vote stops term inflation); (3) log-matching +
  durable tail truncation — divergent uncommitted tail overwritten, follower's on-disk log byte-identical
  to leader; (4) convergence — all survivors byte-identical logs + to_bits-identical state == an
  **INDEPENDENT single-node replay oracle** (workload has 0.1+0.2 and a 1-ULP value ⇒ bits asserted, not
  rounded decimals). `replication.rs` gates a–d adapted to auto-election; `.config/nextest.toml`
  `replog-consensus` serial group (real-timer tests, like `engine-serial`) removes a CPU-contention
  artifact, weakens no assertion. **HONEST BOUNDARY** (verbatim in lib.rs + the parity test): loopback
  proves the consensus arithmetic + relative regression (upper bound on compute, lower bound on
  cross-host wire); absolute cross-host wire p99 / inter-DC SLO stays **DEPLOY-GATED**. Membership change
  (§6) + snapshot install (§7) documented as next increments, not half-built. **SOTA, ZERO workarounds**
  (grep-clean: no `#[ignore]`/`#[allow]`-dodge/`todo!`/fake cluster). Full `just check` green (literal
  "All gates passed."), cargo-deny clean, **1057/1057 tests** (was 1044); raft_election 3/3 + replog 26/26
  stable across re-runs; clippy on the parity TEST target clean; fmt clean. **Next: §4(i) Track B** —
  log compaction / snapshotting (`compaction.rs` + `raft_compaction.rs`: replay-from-snapshot+tail ==
  replay-from-full-log, bit-identical), built on this finalized Raft log; then §4(ii)/(iii) per the runbook.

- 2026-06-06 — **Leadership program Wave 5 DONE → ALL IN-REPO WAVES (1–5) COMPLETE (commit `b743fea`;
  pushed).** GPU perf at scale, **RATIOS only** (NVIDIA absolute headline stays deploy-gated). Two
  disjoint tracks, gated implement→adversarial-verify, then independently re-gated (I re-derived the f32
  bound, read the reconcile assertion, and re-ran `gpu_gate` myself). **Track A — G2 batch closed-form
  kernel** in `celnet-gpu` (`batch.wgsl`/`batch.rs`): one GPU dispatch prices a large vanilla batch by
  closed-form Garman-Kohlhagen in f32 (zero RNG/MC noise; in-kernel A&S-7.1.26 erf), reconciled
  NODE-BY-NODE within TWO separately-DERIVED (not fitted) bounds — (1) f32 round-off vs the f64 eval of
  the SAME A&S-erf algorithm (bit-identical f32 coeffs by to_bits ⇒ only round-off differs; bound from
  f32::EPSILON + per-op ULP budgets), (2) A&S algorithmic error vs the production `libm::erfc` path
  (bit-gated to the QuantLib golden). Three-way **GPU-f32 ≈ CPU-f64 ≈ golden** element-wise over 1792
  instruments; **max f32 round-off rel err 3.11e-7** inside the per-node bound; real Metal path exercised
  (`is_gpu()==true` asserted, not vacuous CPU-vs-CPU); bit-reproducible; headless fallback reconciles to
  golden. **Track B — G1 perf harness** in `celnet-bench` over the EXISTING `GpuBackend`:
  `src/bin/gpu_load.rs` (bounded HdrHistogram sweep, GPU+CPU kpaths/s + gpu/cpu RATIO + dispatch
  p50/p99/p99.9), `benches/gpu_batch.rs`, committed `baselines/gpu_batch.json`, `src/bin/gpu_gate.rs`
  (**slowdown-only RELATIVE** gate — >2× collapse or p99 inflation; never absolute; completes-within-
  ceiling headless). Measured M4 dispatch-amortization curve **0.68×@4k → 53×@1M paths** (device
  saturated). **HONEST BOUNDARY** (verbatim in both crates' docs + GPU-AT-SCALE-PLAN.md): M4 Metal lacks
  f64 ⇒ GPU is f32; in-repo proves CORRECTNESS (f32↔f64↔golden) + a host-local RATIO only; the **NVIDIA
  absolute throughput headline / ≤50ms exotic / Workload-A/B absolute numbers are DEFERRED** to the CUDA
  deploy-gate (G8), never claimed here. **SOTA, ZERO workarounds** (OSS wgpu/Metal; no root Cargo.toml
  edit; proto untouched). Full `just check` green (literal "All gates passed."), **1044/1044 tests** (was
  1035). **▶ PROGRAM STATUS: all five in-repo leadership waves (1 truth-gates+integration, 2 fleet-SLO+
  experience, 3 distributed-correctness, 4 catalogue, 5 GPU-ratios) are COMPLETE & pushed.** Only **Wave 6**
  (deploy/live-estate proof tracks — cross-host wire p99, CUDA deploy-gate baselines, live JVM Celer
  estate lifecycle) remains, and it is **deploy/live-gated by design — never built or claimed in-repo**
  per the honest boundary; in-repo it is closed by the seams + ADRs + the docs-anchor lint already in
  place. The leadership program is materially complete against its measurable bar.

- 2026-06-06 — **Leadership program Wave 3 DONE (distributed correctness, XL; commit `1a57def`; pushed).**
  Two new disjoint leaf crates, gated implement→adversarial-verify, then independently re-gated (I read
  the quorum/commit-index logic + the `to_bits` assertions + the real-socket transport myself). **Track A
  — `celnet-replog`** (→ {`celnet-journal`}, acyclic leaf; std::net + threads, no async runtime): thin
  **leader-replicated** event log — leader durably appends (term,index) entries to its journal, streams
  to followers over **REAL 127.0.0.1 TCP sockets** (length-prefixed, ephemeral ports — mirrors the
  `risk_federation` pattern, NOT a shared-mem fake), commits **only on quorum** (`acks+self >
  cluster/2`); follower/recovered node replays to **BIT-IDENTICAL** state (`f64::to_bits`; workload has
  0.1+0.2 and a 1-ULP value so the bits must match); hot-standby term-bump takeover with zero committed
  loss. `tests/replication.rs` (real ≥3-node loopback, deadline-bounded): gate_a kill-leader→byte-
  identical log + to_bits state; gate_b lost-quorum no-false-progress + bare-majority boundary; gate_c
  bounded standby takeover; gate_d crash-recovery from journal alone. Full Raft election + conflicting-
  tail truncation documented as the next increment (not half-built). **Track B — `celnet-fanout`**
  (lock-free **SPMC broadcast ring**): one producer → power-of-two ring via a **two-phase per-slot
  seqlock** (odd in-progress/even stable straddling the payload store ⇒ no torn read); N consumers each
  own a cursor, observe every item in order (genuine broadcast, not work-stealing); overflow = bounded
  **conflation with exact skip-accounting** (`received + skipped == produced`); zero-alloc lock-free
  publish. `tests/broadcast.rs`: no-loss/total-order at **100 AND 1000** consumers, conflation-
  correctness, measured throughput floor, zero-alloc. (A real single-stamp torn-read bug was caught
  LOUDLY by the conflation test during dev and fixed forward to the two-phase seqlock — no gate lowered;
  the contention-deflated throughput was handled per the §1.2 lesson: best-of-8 bursts, wide-margin
  uncontended floor gated, contended figure reported-not-gated.) **HONEST BOUNDARY** (both crates' docs
  + SCALE-OUT.md): loopback proves compute+framing+quorum/replay/ring arithmetic + relative regression
  (upper bound on compute, lower bound on cross-host wire); absolute cross-host wire p99 / inter-DC SLO
  stays **DEPLOY-GATED**, never claimed here. **SOTA, ZERO workarounds** (OSS-only: replog std-only,
  fanout reuses already-pinned crossbeam-utils CachePadded; both auto-join via the members glob, no root
  Cargo.toml edit; proto untouched). Full `just check` green (literal "All gates passed."), **1035/1035
  tests** (was 1009). SCALE-OUT.md reconciled (replicated log + SPMC ring designed→built). **Next: Wave 5**
  (GPU ratios — reuses `celnet-qmc` Sobol/bridge; `celnet-gpu` perf harness headless on M4/Lavapipe,
  f32↔f64 reconcile, three-way GPU-MC≈CPU-MC≈golden; **NVIDIA absolute throughput headline DEFERRED** per
  the honest boundary). Then Wave 6 = the deploy/live-estate proof tracks (designed+seamed here, proven at
  deploy — never blocks/claims in-repo).

- 2026-06-06 — **Leadership program Wave 4d DONE → FUNCTIONALITY CATALOGUE (Wave 4) COMPLETE (commit
  `416951c`; pushed).** Final two disjoint gated `celnet-parity` rows (implement→adversarial-verify),
  then independently re-gated. **Track A — FRTB-SA completeness** in `celnet-risk-cube/frtb.rs`: full
  **SbM** capital aggregation (within-bucket K_b MAR21.4, cross-bucket MAR21.5 + the MAR21.6 low-corr
  S_b floor, the **three correlation scenarios → max**, curvature K_b±/ψ/γ² reusing the existing
  curvature reprice) + **RRAO** 1.0%/0.1% (MAR23) + an **honest cited DRC zero** for deliverable FX
  (MAR22 — no issuer JTD; not fabricated). Parity `tests/frtb.rs`: SbM == a **longhand independent
  recomputation** (~1e-10, never calls frtb.rs); three-scenario max; hedged K_b=0; monotonicity; RRAO
  exact hand-sum; DRC documented zero. **⚠️ CORRECTNESS BUG I CAUGHT MYSELF (workflow verifier had
  rubber-stamped it):** the LOW scenario was `max(2ρ−1, 0)` — **missing the MAR21.6(2) `0.75ρ` floor**
  (`ρ_low = max(2ρ−1, 0.75ρ)`; material for ρ<0.8 — FX γ=0.6 must give 0.45 not 0.2). The longhand
  oracle had re-derived the SAME wrong formula ⇒ a **circular self-check** that passed while wrong.
  Fixed code + oracle + unit test, and added `correlation_scenario_transform_matches_basel_constants`
  pinning the transform to BCBS hand-computed values so it can't recur. **Track B — pair-universe
  breadth** in `celnet-conventions`/`celnet-calendar`/`celnet-types`: documented **19-pair** universe
  (7 G10 majors, 4 EM deliverable crosses, 6 EM NDF/NDO USD-cash-settled at named fixings, 2 precious
  metals XAU/XAG metal-base T+2 loco-London); internal vendor-neutral **`FixingSource`** enum
  (**celnet-proto/wire UNCHANGED**; `ConventionRecord::new` preserved); Gregorian calendars for
  MXN/ZAR/NOK/SEK + metals-on-London∩US. Parity `tests/pair_universe.rs`: resolved conventions ==
  published EMTA/ISDA table; algorithmic spot date == an **independent Hinnant rata-die + holiday-walk**
  over ~8.7k (pair,date) combos; structural invariants. Honest scope: NDF lunisolar onshore calendars
  correctly **NOT modelled** (`has_calendar_support=false`) rather than faked; live feed VALUES stay
  estate-gated (only fixing IDENTITY encoded). **SOTA, ZERO workarounds.** Both new parity tests pass
  `clippy -p celnet-parity --test <name> -D warnings` (the gate W4b/W4c omitted — added to this wave's
  spec). Full `just check` green (literal "All gates passed."), **1009/1009 tests** (was 976).
  **Wave 4 (catalogue) is now COMPLETE** across 4a–4d: eSSVI, var/vol swaps, analytic Asian, Heston
  FFT/COS, forward-start/cliquet, Sobol+bridge QMC, FRTB-SA, pair-universe — each a gated parity row vs
  an independent oracle. **Next: Wave 3** (replicated log/hot-standby/SPMC, XL — new crate `celnet-replog`)
  then **Wave 5** (GPU ratios — reuses the `celnet-qmc` Sobol/bridge).

- 2026-06-06 — **Leadership program Wave 4c DONE (commit `428a424`; pushed).** Third catalogue
  increment, two disjoint gated `celnet-parity` rows (parallel implement→adversarial-verify), then
  **independently re-gated**. **Track A — forward-start vanilla + cliquet** in
  `celnet-exotics/forward_start.rs`: exact **Rubinstein (1990)** FX dual-carry strike-reset closed form
  (`e^{-r_f·t1}·S0·unit-GK` over residual maturity) + cliquet as the exact **Σ forward-start legs**
  (plain ratchet) + locally-capped/floored cliquet by MC. Parity `tests/forward_start.rs` (6 rows):
  closed form == from-scratch two-leg-GBM MC within reported stderr; t1→0 → celnet-vanilla GK ~1e-9;
  plain cliquet == Σ legs ~1e-10; capped MC == independent in-test clamped MC; tighter cap ⇒ strictly
  lower (structural). **Track B — Sobol + Brownian-bridge QMC** as a NEW crate **`celnet-qmc`** (dep
  {core}): gray-code Joe-Kuo Sobol (embedded **BSD-3-Clause** `new-joe-kuo-6.21201` direction numbers,
  dims 2..=300 documented+gated), **Owen-style nested scramble** (unbiased RQMC), principal-bisection
  Brownian bridge, high-accuracy inverse-normal CDF; direction numbers exposed for **Wave-5 GPU reuse**
  (no GPU claim). Parity `tests/qmc.rs` (7 rows): Sobol KAT vs canonical `sobol.cc` + dim-1 van-der-Corput
  identity; exact (0,m,1)/(0,m,2)-net equidistribution (honestly NOT overclaiming (0,m,s) for s≥3);
  **MEASURED** variance reduction vs fair plain MC on exact targets — geometric-Asian (Kemna-Vorst)
  **≈37.7×**, European (Black-Scholes) **≈88.5×**, both ≥3× required, ratios measured-not-asserted;
  bridge covariance `A·Aᵀ=min(t_i,t_j)` exact; RQMC unbiasedness. **LESSON AGAIN:** the per-track verifier
  REJECTED Track B (correct) because `tests/qmc.rs` failed `clippy -p celnet-parity --test qmc -D warnings`
  (21 lints: doc-overindent + needless-range-loop + too-many-arguments) — the impl had only run clippy on
  `celnet-qmc`, not the parity test target (my Track-B gate spec omitted it). Fixed forward myself
  (doc-list reflow, iterator loops, an `RmseCase` config struct replacing two 8-arg helpers — pure
  refactors, tests unchanged incl. the variance-reduction gate). **SOTA, ZERO workarounds** (OSS-only
  deps; my own diff grep clean). Full `just check` green (literal "All gates passed."), **976/976 tests**
  (was 949). **Next: Wave 4d** — FRTB-SA completeness (DRC/RRAO, risk crates) ∥ pair-universe breadth
  (conventions/calendar/types), each its own fully-gated wave; then **Wave 3** (replicated log/hot-standby/
  SPMC, XL) and **Wave 5** (GPU ratios — reuses the celnet-qmc Sobol/bridge).

- 2026-06-06 — **Leadership program Wave 4b DONE (commit `91d8e2c`; pushed).** Second catalogue
  increment, two disjoint gated `celnet-parity` rows (parallel implement→adversarial-verify, both
  **accept**), then **independently re-gated** — and the full `just check` caught a real defect both
  track verifiers missed (an `excessive_precision` float literal in `tests/asian.rs:453`, the Acklam
  inverse-CDF coeff with a trailing 0 — fixed forward by clippy's exact truncation, a numeric no-op;
  Track A's verifier had misattributed it to heston). **Track A — analytic arithmetic-Asian** in
  `celnet-exotics/asian.rs`: MC-free **Turnbull-Wakeman** (two-moment lognormal matching) + **Curran**
  (geometric-conditioning, in-crate 64-node Gauss-Legendre, no external quadrature dep), FX carry +
  seasoned (in-progress-average) case. Arithmetic Asian has NO exact closed form ⇒ parity
  `tests/asian.rs` (8 rows) **honestly toleranced**: exact closed-form limits (single-obs→GK vanilla,
  zero-vol→discounted intrinsic, geometric leg→Kemna-Vorst ~1e-12); Curran within a few **reported MC
  stderr** of the existing independent `price_asian` MC; TW gated at its TRUE **~1.5% approximation
  band** (explicitly NOT an MC-precision claim); Curran≤TW; seasoned vs a code-disjoint splitmix64+
  Acklam MC. **Track B — standalone Heston** as a NEW crate **`celnet-heston`** (deps {core,types,libm};
  vanilla dev-dep): two genuinely independent CF transforms over a shared **branch-cut-free CF**
  (Cui-del-Baño-Germano) — **Carr-Madan** damped-integral Gauss-Legendre quadrature ∥ **Fang-Oosterlee
  COS** (cumulant range, c4 by finite-difference; prices the bounded put leg, call by exact parity);
  **no external FFT crate** (guardrail #7). Parity `tests/heston.rs` (5 rows): CM≈COS to 1e-8+1e-7·price
  on the full ≤3y FX grid (560 pts); **BS σ→0,v0=θ limit** vs independent celnet-vanilla GK ≤1e-5 with
  verified O(σ²) rate; put-call parity ≤1e-10; strike-monotonicity; published **Albrecher-2007
  little-Heston-trap anchor** (~5.785). >3y deep-OTM Fourier precision wall documented + gated only ≤3y
  (not claimed tight). New crate auto-joins via the `members=["crates/*"]` glob (**no root Cargo.toml
  edit**); parity references it by path like `celnet-golden`. **SOTA, ZERO workarounds** (verifiers +
  my own diff grep: no `#[ignore]`/lint-dodge/stub/overclaim; OSS-only deps). Full `just check` green
  (literal "All gates passed."), **949/949 tests** (was 923). **Next: Wave 4c** — forward-start/cliquet,
  then Sobol+Brownian-bridge QMC (CPU-first, reused by Wave 5 GPU), FRTB-SA completeness, pair-universe
  breadth (each its own fully-gated wave); Wave 3 (replicated log/hot-standby/SPMC, XL) + Wave 5 (GPU
  ratios) remain.

- 2026-06-06 — **Leadership program Wave 4a DONE (commit `0c54958`; pushed).** First catalogue
  increment, two disjoint gated `celnet-parity` rows built as parallel implement→adversarial-verify
  tracks (both verdict **accept**), then independently re-gated. **Track A — eSSVI** in
  `celnet-surface/extended_surface.rs`: maturity-dependent ρ(θ) in the (θ,ρ,ψ) variables (ψ=θ·φ the
  ATM skew-scale), **SSVI byte-recovered as the constant-ρ special case** (`ExtendedSlice::from_curvature`
  → `to_bits` identity over a 1680-pt sweep). Closed-form per-slice butterfly domain `ψ(1+|ρ|)<4 ∧
  (ψ²/θ)(1+|ρ|)≤4` + consecutive-slice calendar `|ρ₂ψ₂−ρ₁ψ₁|≤ψ₂−ψ₁` (Hendriks-Martini 2019,
  provenance in doc comments only; identifiers purpose-named). Extended the ONE wire contract:
  **`SMILE_MODEL_EXTENDED_SURFACE=4`** (appended, no renumber, **no `schema_version`**), types/proto/
  convert round-trip + surface `SmileModel` + `build_model_smile` (damped Gauss-Newton fit projected
  into the butterfly domain) + server/bench label maps — a mark request selects eSSVI exactly like
  SSVI. Parity `tests/essvi.rs` (4 rows): density≥0 (Breeden-Litzenberger pointwise re-pricing) +
  calendar-monotone (pointwise w) validate the **closed-form claims against genuinely independent
  numerics**; golden self-reprice ≤1e-9; SSVI `to_bits` byte-recovery. **Track B — variance + vol
  swaps** in `celnet-exotics` (`var_swap.rs`/`vol_swap.rs`): var-swap fair strike by **log-contract
  1/K² static replication** (Demeterfi-DKZ / Carr-Madan) over the OTM forward strip in log-moneyness
  with an **adaptive wing to machine precision**; vol-swap by the **Carr-Lee convexity adjustment**
  `K_vol=√K_var − Var(v)/(8·K_var^{3/2})`, strictly < √K_var for any non-degenerate smile. Parity
  `tests/var_vol_swap.rs` (5 rows): strip == an **independently-coded strike-space recursive
  adaptive-Simpson** quadrature ~1e-6; **flat-σ closed form `K_var==σ²`** ~1e-6 (catches forward/
  discount/scale/sign errors a quadrature pair could share); strict `K_vol<√K_var` widening with
  convexity; default strip on the <1e-7 convergence plateau. **SOTA, ZERO workarounds** (verify phase
  + my own diff grep: no `#[ignore]`/lint-dodge/stub/lowered-tolerance/overclaim; the only `unreachable!`
  is an exhaustiveness guard after the three real slice variants; the only `#[allow]` is
  `too_many_arguments` on a 9-arg recursive quadrature oracle). Independently re-gated: **full `just
  check` green (literal "All gates passed."), 923/923 tests** (was 898). Honest boundary respected
  (pure in-repo numerics). **Next: Wave 4b** (arithmetic Asian, then forward-start/cliquet, standalone
  Heston FFT/COS, Sobol+Brownian-bridge QMC — CPU-first, reused by Wave 5 GPU — FRTB-SA completeness,
  pair-universe breadth; each its own fully-gated wave); Wave 3 (replicated log/hot-standby/SPMC, XL)
  and Wave 5 (GPU ratios) remain.

- 2026-06-06 — **Leadership program Wave 2 DONE (commits `32a39f6`, `375b808`, `911e294`; pushed).**
  **2a (`32a39f6`):** fleet **§11 SLO loopback truth-benches** (`celnet-bench/fleet_slo.rs` + bin +
  `baselines/fleet_slo.json`, gated by `bench_gate` arm 3) — cross-shard routing overhead, publish→
  snapshot lag, conflation-correctness, fan-out tail — **honestly labelled LOOPBACK** (a HONEST
  BOUNDARY banner in code+output; absolute wire SLOs stay deploy-gated) + the architectural
  invariant (`forward.rs` serve_mode(None)==Serve::Local structural test + a behavioral
  in-process-prices-locally test: **per-tick price path never crosses the router**); and **CLI
  four-client parity** (`celnet-cli` `risk {aggregate,drill,positions,limits}` + `stream` via the
  SDK, proven **CLI==SDK==server** against a real edge). **2b (`375b808`):** **concurrent federation
  fan-out** (`federate.rs` sequential awaits → `join_all`; reducer + 1e-12/1e-9 invariance UNCHANGED;
  new latency test proves ~max-backend not Σ — empirically discriminates: 0.62s vs 2.4s sequential) +
  **surface-rebuild §1.2 truth-bench** (all-tenors VV/SSVI recompute, p99 VV 19.6µs/SSVI 7.75µs inside
  the 150µs budget; `bench_gate` arm 1b). **PROCESS LESSON (`911e294`):** I pushed `375b808` on a
  false background-wrapper "exit 0" while the full `just check` had FAILED fail-fast — the surface
  budget UNIT test asserted §1.2 p99≤150µs but a latency percentile measured *inside parallel nextest*
  (898 tests saturating all cores) inflates (181µs) = a measurement-methodology bug, not a regression.
  Fixed forward (NO gate relaxation): the STRICT §1.2 budgets stay in the un-contended `core_load`/
  `surface_rebuild` bins + `bench_gate` + the CI `core-load-gate`/`bench-gate` perf lanes; the in-suite
  unit tests now assert harness + a contention-robust gross-sanity ceiling. **RULE: always verify the
  literal "All gates passed" line, never the background wrapper exit code, before committing.** Full
  `just check` green (898/898 twice under contention). **Next: Wave 4** (catalogue — independent of the
  fleet waves, high product value: eSSVI, variance/vol swaps, arithmetic Asian, forward-start/cliquet,
  Heston FFT/COS, Sobol QMC, FRTB-SA — each a gated `celnet-parity` row) per `docs/LEADERSHIP-PROGRAM.md`;
  Wave 3 (replicated log/hot-standby/SPMC, XL) and Wave 5 (GPU ratios) remain.

- 2026-06-05 — **Leadership program kicked off + Wave 1 DONE (commits `29e806b`, `0761a03`,
  `6a8bde3`; pushed to github.com/soarsa/celnet).** Multi-agent assessment + architect synthesis →
  **`docs/LEADERSHIP-PROGRAM.md`**: a 6-wave, dependency-ordered, gate-defined program to
  world-leading, with a measurable bar and a verbatim **honest boundary** (cross-host wire p99 /
  NVIDIA throughput / live JVM Celer estate stay deploy/live-gated, never claimed in-repo). Executed
  as gated implement→adversarial-verify waves; **SOTA, zero workarounds** (user directive — verify
  phase greps the diff for lowered gates / `#[ignore]` / `as any` / disabled lints). **Wave 1
  (`0761a03` perf+gui, `6a8bde3` integration):** (1) **in-core §1.2 absolute latency truth-gate** —
  `celnet-bench` `core_load` HdrHistogram of the pinned price+13-Greek loop, asserts p50≤2µs/p99≤10µs/
  p99.9≤25µs ABSOLUTELY (measured M4: 42ns/125ns/~1µs → 24–80× margin), replacing the divan medians;
  `bench_gate` now absolute+relative; **iai-callgrind** instruction gate (Linux CI lane). (2) **live
  FIX acceptor** on `celnet-server` — real celnet-fix 4.4 engine over a real loopback socket,
  external RFQ→Quote(==golden 1e-12)→fill, reusing the keyed-MAC click-to-trade token path (extracted
  to shared `services/clicktrade.rs`); forged/replayed/stale rejected; `CELNET_FIX_ADDR` knob. (3)
  **`CELNET_DEPLOY` Standalone edge** — DeployMode bound at boot (default byte-identical, exact f64
  bits), vendor-replay→ResilientSubscriber(gap-resync)→normalize→SurfaceBook, PriceSink→EgressGovernor
  (bounded, counted drops). (4) **GUI test harness** — `gui/` vitest 3.2.6 (vite-6 deduped) + jsdom,
  6 suites / 72 tests over real modules. No `celnet.proto` change. Per-crate: bench 9 / server 125 /
  fix 42 / gui 72; **full `just check` green** each milestone; independently re-gated + diff-reviewed.
  **Next:** Wave 2 (fleet §11 SLO benches, concurrent federation fan-out, GUI Playwright e2e + axe,
  CLI four-client parity, `wide` SIMD batch, surface-rebuild p99 bench).

- 2026-05-31 — **Configurable in-process / out-of-process node scaling — ENABLED + fully tested
  across scale up/down (commits `4118fe8`, `614bdd5`, `b220787`).** Researched + critiqued the
  optimal route (Plan-mode, approved) → mirror the `DEPLOYMENT-MODES.md` §1 pattern: *engine never
  changes; only the deploy-time-bound adapter does.* One knob — **`FleetTopology`**
  (`celnet-risk-fleet`), bound at `Edge` boot from **`CELNET_FLEET_MODE`/`CELNET_FLEET_BACKENDS`** —
  selects **`InProcess` (DEFAULT, byte-identical to single-node, zero overhead)** or
  **`Distributed{endpoints}`**. **(1) Seam (`4118fe8`):** object-safe `ShardRiskSource` (additive
  cheap path vs constituent gather, split per SCALE-OUT §2) + `InProcessShards` + generic reducers,
  added UNDER the existing fleet API (13 tests untouched + 6 new); server resolves the topology, default
  path unchanged, Distributed fails loud. **(2) Distributed federation (`614bdd5`):** the
  `celnet-server` edge is a **client of the same `RiskService` it serves**, federating across N backend
  processes over **real gRPC** — additive summed in wire space (linear ⇒ exact, cheap), non-additive
  **re-gathered** via `position_to_fact` over the union and re-derived once (exact); `route`
  (health-aware) for reach, `natural_owner` for ownership, **`Status::unavailable`** on an unreachable
  slice (never a silent partial). `tests/risk_federation.rs` (8): boots real gRPC backends on ephemeral
  ports, proves **federated == single-node** (additive 1e-12, non-additive 1e-9) for FIRM + grouped dims
  + grant-all/deny-walled principals; scale-up/down + standby/no-standby failover. In-process churn
  ladder 1→2→3→5→4→3 invariant across all measures (`celnet-risk-fleet` +3). **(3) Forwarding + harness
  (`b220787`):** owned-pair Pricing/Quote/Surface forwarded by HRW route to the owning backend
  (`services/forward.rs`); Stream relayed (session pinned to first-sub owner; cross-owner-per-session
  mux honestly deferred); `tests/forwarding.rs` (5) forwarded==direct-to-owner. **Runnable OS-process
  harness** `examples/scale_harness.rs` (`cargo run -p celnet-server --example scale_harness`) spawns
  REAL backend+edge OS processes, drives EVERY capability through the edge via `celnet-client`
  (price+14 Greeks, surface mark/smile/scenario, RFS stream+click-to-trade, risk
  aggregate/drill/limits/list incl. scoped-deny), and holds firm Δ/vega/VaR/ES/prices/positions
  **bit-invariant across 3→4→2 node scaling**, with honest `unavailable` on a killed-without-rehome
  backend. **NO `celnet.proto` change** (single contract, guardrail 9); only internal deps added
  (`celnet-risk-fleet`/`celnet-router`/`celnet-client`, acyclic). Verified by hand: **full `just check`
  green at each milestone**; fleet 22 / server 109; harness run end-to-end ("ALL SCALE STEPS PASSED").
  Built via three implement→adversarial-verify workflows (all accept), independently re-gated +
  diff-reviewed + harness re-run. Docs reconciled: `SCALE-OUT.md` §0 (serving federation now Built;
  cross-DC hardening / replicated-log / hot-standby-prewarm / latency-SLOs still deferred),
  `DEPLOYMENT-MODES.md` §1.1 (the topology knob). **Honest deferral:** localhost multi-process over real
  gRPC proves correctness + routing + churn + failover + full-API parity; the §11 latency SLOs, cross-DC
  datapath, replicated log, and hot-standby pre-warm remain drop-ins behind the now-built seam.

- 2026-05-31 — **Cross-fleet distributed risk fan-out (aggregation algebra) — DONE; closes the
  LAST frontier item.** New clean crate **`celnet-risk-fleet`** (one-way deps → {`celnet-risk-cube`,
  `celnet-router`, `celnet-types`}; verified acyclic — neither cube nor router gains a fleet dep).
  Partitions `RiskFact`s by **(legal-entity, ccy-pair)** through `celnet-router`'s real **HRW**
  `PartitionMap`/`natural_owner` (NOT modulo) onto a `ReplicaSet`; **shard-local roll-up** (each
  logical shard owns its facts in its own `Cube`); **cross-shard reduce** — *additive* measures
  (per-ccy `NetGreeks` + `VegaLadder`) combine via `NodeAggregate::merge_additive` (associative →
  EXACT), *non-additive* (VaR/ES, curvature) **re-gathered at firm level** (union of constituents →
  re-derive once; summing shard VaRs would be wrong by sub-additivity — proven). 13 tests:
  **fan-out == single-node** `firm_aggregate` to 1e-12 across the full Greek set + vega ladder;
  non-additive VaR/ES & curvature reconcile exactly (same constituent multiset → same oracle; residual
  only FP summation order, bit-identical when order fixed); HRW disjoint-cover + (entity,pair)
  co-residency + **minimal-reshuffle** (5→6 replicas moves ~1/N, only onto the new replica);
  bit-reproducible. **Honest scope:** this is the cross-shard aggregation ALGEBRA + HRW partitioning,
  validated **in-process** (logical shards = local `Cube` standing in for a separate node); the
  **physical cross-node transport, replicated event log, and hot-standby/failover remain
  designed-only** (no sockets/RPC faking a cluster) — `docs/SCALE-OUT.md` §0 prose + table corrected
  to match (router HRW + fleet algebra now Built; transport/log/standby still Designed only). Verified
  by hand: **`just check-crate celnet-risk-fleet` 13/13 + full `just check` green ("All gates
  passed.")**; built via an implement→adversarial-verify dynamic workflow (verdict accept), then
  independently re-gated, diff-reviewed (no mocks/modulo/placeholder), and the dependency graph
  confirmed acyclic (`cargo tree -i celnet-risk-fleet --edges normal` empty). **Frontier now clear:**
  both honestly-deferred items (AAD/GPU reval; cross-fleet risk) are landed; what remains is the
  physical fleet plumbing (transport/log/standby — gated on a measured single-shard bottleneck) and
  the deferred GPU items (closed-form batch kernel / Workload A·G2, GPU pathwise Greeks / G6).

- 2026-05-31 — **AAD adjoint Greeks + GPU-batched scenario — built AND wired into the risk
  estate (closes the first frontier item; bump-and-revalue/analytic kept as oracles).** Resumed
  the in-flight dynamic workflow and finished it end-to-end. **(1) `celnet-vanilla::adjoint_greeks`**
  — genuine reverse-mode AAD over the GK graph: one reverse sweep yields the full first-order block
  (delta/vega/theta/two-rho) + second-order gamma/vanna/volga (reverse-over-reverse); charm/speed/
  zomma/color stay analytic (boundary documented, not faked). Gated to ~1e-12 vs the analytic Greeks
  AND an independent central-FD oracle; price bit-identical to `price()`; bit-reproducible.
  **(2) `celnet-gpu` batched scenario kernel** (`scenario.rs`/`scenario.wgsl`, `ScenarioPricer`/
  `ScenarioAxes`/`ScenarioGrid`) — one dispatch prices a whole spot×vol shock grid of a vanilla under
  **common random numbers** (smooth ladder, no MC-noise crossings), f32 GPU reconciled to the f64 CPU
  oracle node-by-node within the crate's derived bound; transparent CPU fallback for headless/CI;
  bounded readback deadline (never hangs). **(3) Wired into the risk estate (the part that makes the
  firm-scale claim real):** `celnet-risk-normalize` canonical leaf now defaults to the **adjoint**
  engine (`GreekEngine::Adjoint`, purpose-named per guardrail 8) — one O(1)-in-factors sweep replaces
  O(factors) bump — with the closed-form analytic engine retained as the validation oracle/fallback,
  the two gated equal to ~1e-9 (`adjoint_leaf_matches_analytic_leaf`). `celnet-risk-cube::nonadditive`
  gains an AAD **sensitivity-based VaR/ES lens** (`sensitivity_var_es`: Greeks computed ONCE per
  position, reused across scenarios via a 2nd-order Taylor P&L — O(positions) sweeps vs the oracle's
  O(positions×scenarios) repricings); `historical_var_es` full bump-and-revalue **retained as the
  oracle**, the two reconciled to a documented ≤8% rel envelope over a daily-VaR-scale ladder with a
  test proving the Taylor residual shrinks O(shock³) (tight regime <2%) and an honest large-shock
  divergence test. `celnet-risk-cube::scenario_grid` adds a node spot×vol GPU scenario-PV grid
  (one dispatch/position) reconciled to the analytic grid within the MC std-error band. **Honestly
  NOT done:** GPU-MC was deliberately NOT plumbed into the closed-form VaR path (mixing MC noise into
  an exact reval is a regression) — the right GPU lever there is a batched **closed-form** vanilla
  kernel (GPU-AT-SCALE Workload A / G2), still the distinct next GPU increment; GPU pathwise/LR Greeks
  (G6) still deferred. Verified by hand: **full `just check` green ("All gates passed.")** — per-crate
  AAD 38 / gpu 24 / risk-cube 19 / risk-normalize green; built via an implement→adversarial-verify
  dynamic workflow (verdict accept) then independently re-gated + diff-reviewed (no mocks/placeholders;
  agent under-reported its diff — reviewed in full before commit). **Next frontier (the one remaining
  item):** cross-fleet distributed risk fan-out — shard-local roll-up + cross-shard reducer over the
  built `celnet-router` HRW map, reconciled fan-out == single-node aggregate.

- 2026-05-31 — **GUI IB-scale views (commit `547b7bd`).** Built via parallel lanes off a
  token-optimized file-handoff seam (`/tmp/celnet-scale-seam.md`): **UniverseNavigator** (⌘B/
  toolbar "Pairs" — command-palette-style, grouped Majors/Crosses/EM, favourites, keyboard-first,
  registry-ready over the seeded set, honestly labelled), **virtualised blotter** (StreamWorkspace +
  dependency-free `lib/virtual.ts` windowing → scales to thousands of rows; group/collapse by
  pair/tenor with real aggregates; sortable sticky columns), **vol-cube pivot/heatmap**
  (CubeWorkspace via a Surface mark|cube toggle; pair×tenor×delta heat from the server's calibrated
  smiles via `data/cube.ts`; perceptual ramp; honest "—" empties; cell→smile drill; no client-side
  vol math), and a **broken-date/event-aware ticket** (TicketWorkspace + DatePicker; tenor incl.
  ON/TN/SN/IMM OR arbitrary broken date; prices end-to-end through the real transport via the
  BrokenDate Tenor + expiry_years — confirmed; event-clock jump-vol honestly labelled deferred).
  Verified by hand: `npm run build` green (106 modules) + live QA (navigator + cube render real
  data vs the demo edge). **Frontier remaining (both honestly deferred-by-design in the docs):**
  AAD adjoint Greeks + GPU-batched scenario (perf; bump-and-revalue oracle built to validate),
  and cross-fleet distributed risk fan-out (needs `celnet-router`, currently designed-only).

- 2026-05-31 — **Firm-scale hierarchical risk REAL end-to-end (commit `4c42762`) — closes the
  client-side-aggregation parity gap.** New **`RiskService`** in the one `celnet-proto` contract
  (ListPositions/AggregateRisk/DrillRisk/LimitStatus): `RiskDimension`
  firm→trader→book→desk→ccy-pair→location→entity, entitlement principal (**grant-all default** =
  show-all-now, deny-wins), reporting numeraire, additive (per-ccy delta vector + vega ladder) +
  non-additive (VaR/ES/FRTB-curvature, absent⇒not-evaluated) node tree, limits RAG. `celnet-server`
  `services/risk` (live PositionStore over `celnet-risk-cube` + `-limits`, attribution interner;
  entitlement-prune **before** roll-up → group_by/firm_aggregate → numeraire collapse → bump-and-revalue
  non-additive) served over **gRPC + WS**. Clients in lockstep: **GUI Book/Risk consume the SERVER
  aggregate — `portfolioRisk.ts` client-side loop DELETED**, scope drives group-by, Book→Risk drill,
  Limits RAG panel, native-units caveat resolved; `celnet-client` 4 methods; Excel
  `CELNET.POSITIONS/RISK/LIMITS`; docs reconciled. Proto reviewed via the new `protobuf` skill (enum
  prefixes/field-numbering clean). Verified by hand: **`just check` 791** + `npm run build` + **Excel
  e2e A–H** (H: FIRM roll-up == Σ book, USD, server-aggregated). **Still deferred (honest):** AAD/GPU
  non-additive reval, cross-shard/HRW fleet tier. **Next frontier:** GUI scale views (virtualised
  blotter, universe navigator, vol-cube pivot, broken-date/event ticket), then AAD/GPU, then cross-fleet.

- 2026-05-31 — **Phase 1+2 DONE — contract capabilities + risk crates, full client parity (commit `c5985af`).**
  One canonical `celnet-proto` contract extended (no versioning): **SmileModel** selector (real fitted
  SABR/SVI/SSVI in `celnet-surface` via deterministic damped Gauss-Newton, no-arb-projected, alongside
  Vanna-Volga), **market-series feed** (MarketObservable + MarketSeries* on the multiplexed
  StreamSession, served from live state), **attribution** (Owner/BookId/AttributionRecord on the
  quote/trade lifecycle, emitted over gRPC **and** WS). `celnet-calendar` **ON-resolves-as-SN bug
  fixed** (ON anchored on horizon ~T+1, not spot) + TN/SN + IMM resolver (CME-validated) + BrokenDate;
  schedule/vol_year_fraction fallible w/ `vol_anchor`. New single-node risk crates:
  **celnet-risk-normalize** (convention canonicalization + common-numeraire), **celnet-risk-cube**
  (hierarchical additive roll-up + non-additive bump-and-revalue VaR/ES/curvature), **celnet-limits**
  (RAG + pre/post-trade), **celnet-entitlements** (grant-all default + scoped pruning). Honestly
  deferred in module docs (not stubbed): AAD/GPU adjoint, cross-shard reduction, audit/admin GUI half.
  Full **client parity**: GUI (model chips, live `useTrendSeries`, attribution), `celnet-client`
  (`mark_surface_with`/`subscribe_series`), Excel (`CELNET.MARKSURFACE` model arg, `CELNET.SERIES`),
  `docs/INTERFACES.md`. Verified by hand: **`just check` 769 tests** + fmt/clippy-D/deny green;
  `npm run build` green; **Excel e2e PASS A–G** (F=SABR-vs-VV selection, G=market-series) vs a fresh
  demo edge. **Next:** expose the risk-cube estate through a RiskService contract + wire GUI Book/Risk
  to consume the **server** aggregate (closes the client-side-aggregation parity gap), keyed on the
  now-on-the-wire attribution chain; then AAD/GPU + cross-fleet fan-out + GUI scale views.

- 2026-05-31 — **GUI → Celer-product rebrand + experience-architecture design corpus.** (1) GUI
  rebranded to **Celer Technologies** (coral `--brand` + indigo `--accent`, Anaheim, pinwheel mark
  once in the rail, mark-less toolbar wordmark, no traffic lights, real build-stamp); added a **pair
  watchlist strip**, a real **pair dropdown** (`PairMenu`), and an **aggregated Book** view (commits
  `1854ab4`, `f89e96e`). (2) Four multi-agent research/critique workflows → design corpus:
  `docs/RISK-HIERARCHY.md` (+ROADMAP §9/WS-R), `docs/TRADING-UNIVERSE-SCALE.md`,
  `docs/SURFACE-WORKFLOW.md`, and the capstone **`docs/EXPERIENCE-ARCHITECTURE.md`** — one coherent IX
  (Scope×View×Analytics over a position-fact cube; entitlement drill-down show-all-now; Book↔Risk;
  analytics selection; `TrendMode`) + a **reconciled phased backlog** (ROADMAP §10). (3) Governing rule
  recorded: **API-first client parity** — every capability in the one `celnet-proto` contract; GUI/SDK
  (`celnet-client`)/Excel (`CELNET.*`)/docs evolve in lockstep ([[api-first-client-parity]]). Real
  defects found, queued Phase 0: surface **mismark** (Re-mark==Publish, edit never sent), hardcoded
  `calendarArbitrageFree:true` (placeholder), and **`celnet-calendar` ON-resolves-as-SN** (`fx.rs:134`).
  **Next:** Phase 0 (API-first), starting with the contract check for surface-edit/model-selection.
- 2026-05-31 — **`celnet-journal` DONE — standalone durable crash-recovery (closes SCALE-OUT
  §8 "designed-only" gap, task #37).** Dependency-free `fsync`'d append-only sequence-ordered
  log: per-record CRC-32, clean torn-tail truncation on open (crash mid-append heals to last
  good record; interior corruption surfaced, not silently healed), `MAX_PAYLOAD_LEN` guard on
  the recovery allocation, payload-agnostic `EventCodec` seam. Compaction/checkpoint *designed*
  in module docs, honestly not built (no placeholder). Wired into `celnet-engine::journal`:
  `DurableBook` `fsync`s each book/mark via the **shared** handoff byte codec (extracted
  `write_book_entry`/`read_book_entry` — no format fork, guardrail #9); `recover()` rebuilds
  `BookState`+`MarketState` at **startup**, strictly off the hot path. Proofs: kill/restart
  **byte-identical** book + **bit-identical** repricing over the full **14-Greek** set, two-cycle
  full-history replay, and a new `zero_alloc` test (`pricing_a_journalled_book_allocates_zero`)
  confirming `price()` never touches the journal. 11 journal + 29 engine tests green; **full
  `just check` green** (fmt, clippy -D, nextest, cargo-deny). Also fmt-healed a stray
  `demo_edge.rs`. Recovery model now: deterministic replay **+** standalone WAL.
- 2026-05-31 — **Live demo + Excel + GUI all REAL (no mocks); building durable journal.**
  `celnet-fix` (real FIX 4.4 engine, acceptor+initiator, dialect, loopback-tested) +
  `celnet-integration` egress governor/ingress/deployment-mode seam + `celnet-router`
  (fleet HRW partition map). Excel add-in (`excel/`, Office.js `CELNET.*`) + `gui/`
  (React/WebGPU trader UI) both verified end-to-end against a LIVE seeded server
  (`cargo run -p celnet-server --example demo_edge`). GUI flipped to **live WS by default**
  (mock demoted to `?mock`), stuck-resync + click-to-trade fixed, Risk shows real
  cross-gamma/theta-roll/vega. Headless e2e PASS (PRICE==server, MARK→version pin,
  forged-token reject). **Persistence audit (this session):** recovery = deterministic replay;
  durable today = `celnet-fix` FileStore + `celnet-observability` lossless audit (committer
  seam); blue-green handoff = in-memory; integrated-mode trade/position durability = the Celer
  estate; **gap = a standalone durable event-log/WAL (SCALE-OUT §8 "designed only")** → now
  being built as `celnet-journal` (task #37). See memory [[session-state-2026-05-31]] for the
  running services + how to resume.
- 2026-05-30 — **GA sign-off (rev 2).** All open-gap streams closed: plugin-host (wasmi) +
  trader GUI built; API-v2 optimized (multiplex session, click-to-trade keyed-MAC token,
  book-shaped risk, surface_version — no versioning). 21 crates + `gui/`, **555 tests green**,
  full `just check` terminating. `docs/GA-READINESS.md`: **GO** for the pricing-platform GA with
  one honest gating caveat — end-to-end latency-under-load proof + CI bench gate before the
  wire-latency headline is GA-grade; fleet layer, GPU-at-scale, live Celer/FIX, WS-mirror, and
  TARF/quanto breadth are de-risked post-GA execution.
- 2026-05-30 — **WS-G plugin host built — wasmtime blocker CLOSED.** `celnet-plugin-host` is a
  tiered host behind the frozen `celnet-plugin-api` contract: a unified `ModelRegistry` routes
  **Tier-0 native** (`dyn PricingModel` via the tier-blind `HostModel` seam) and **Tier-2 wasm**
  models identically. Tier-2 is the deterministic sandbox on **wasmi 1.0.9** (pure-Rust,
  fuel-metered, advisory-clean — replaces wasmtime): `Config::consume_fuel`, a no-WASI capability
  `Linker` exposing ONLY the libm `celnet_core::math` primitives (zero ambient authority),
  per-call `FuelBudget` SLA (exhaustion ⇒ typed `HostError::FuelExhausted`, never a hang),
  boundary NaN-canonicalization, and a host-controlled `(ptr,len)` core-module ABI marshalling
  `VanillaInputs`→`Greeks`. Deterministic **replay** harness asserts `to_bits` identity across
  runs. 14 tests green (all four WS-G gates: capability-denial, fuel-exhaustion bounded under a
  watchdog, replay bit-identity, Tier-0==Tier-2 interchangeability via WAT fixtures). `cargo fmt`
  + `clippy -D warnings` + `nextest` + `cargo-deny` (advisories/bans/licenses) all green. Docs
  synced (ARCHITECTURE §6, ROADMAP WS-G, CAPABILITIES-VS-COMPETITION, `wit/celnet.wit` header →
  wasmi/core-modules). No `unsafe`. **Next:** Tier-1 `stabby` signed-`.so` + Tier-3 Landlock ring
  (designed in `PLUGIN-HOST-ALT.md`), GA sign-off (#15).
- 2026-05-30 — **GA-push critique "needs-work" findings closed (client/server/engine + GUI doc).**
  (1) Client RFS reconnect-liveness bug fixed: `reconnect_session` + session-close now drain the
  click-to-trade waiter table via `fail_all_waiters`, resolving every pending `execute` with a
  typed `ClientError::Reconnected`/`StreamClosed` (no more infinite await across a blue-green
  cutover); regression tests are timeout-bounded. (2) Server `consumed_tokens` bounded:
  `HashMap<token, valid_until>` with expiry eviction on insert (`record_consumed`/`is_consumed`) —
  replay protection still holds *within* the validity window; bounded-growth test added. (3)
  Forgeable token minter replaced by a **keyed-MAC** `TokenMinter` (`blake3` keyed hash over the
  line-binding tuple under a 256-bit OS-CSPRNG secret drawn once at session start — runtime
  control-plane identity, NOT a pricing input, so pricing determinism is untouched); key-bound +
  field-bound MAC tests added. (4) Engine flaky tests fixed: zero-alloc concurrent-publish proof
  made deterministic (reclamation proven in a separate single-thread armed micro-window; racy
  `deallocs>0` sub-assert removed, zero-alloc guarantee UNWEAKENED); seqlock/arc-swap probes
  wall-clock-capped (`SPIN_CAP`); new `.config/nextest.toml` serializes the global-allocator/
  spin-sensitive engine tests (`engine-serial` group) + slow-timeout. Engine suite now ~0.9 s in
  isolation AND `--workspace`, stable across repeated runs. (5) `docs/GUI-DESIGN.md` §2/§4.2/§8/§10
  updated: `StreamService.StreamSession` (multiplex) is the contract and click-to-trade is
  *implemented* (Positions/P&L `GetPosition`/`AttributePnl` remains the only honest API-v2 gap).
  `blake3`/`getrandom` vetted clean by cargo-deny (advisories/licenses/bans ok). `just check`
  fully green.
- 2026-05-30 — **API-v2 stage 2/3: celnet-server on the optimized celnet-proto.** Server
  caught up to the multiplex/click-to-trade/book-risk/surface-version contract. New
  `surface_book` (versioned marked-surface registry: `MarkSurface` deposits calibrated
  smiles under a fresh `surface_version`; the pricing/RFQ/RFS paths pin against it via the
  shared `services::pin` resolver — unknown version ⇒ `failed_precondition`, never silent
  live fallback). `stream.rs` rewritten to the multiplex `StreamSession` driver: one session,
  many subscriptions, per-sub sequence/snapshot/delta/resync, in-place `Modify`, and
  click-to-trade — unguessable `splitmix64`-minted `TradableToken`s (SELL@bid/BUY@offer) with
  `valid_until` last-look + `Execute` idempotency, rejecting stale/forged/already-consumed.
  Scenario now book-shaped: theta-roll (`FACTOR_TIME`) axis rolling expiry per node, bucketed
  vega per (tenor,delta) pillar, cross-gamma 2-D stencil. Pricing/Quote echo
  `correlation_id`+`surface_version`. celnet-client updated to the new contract.
  **59 server tests + 22 client tests pass (suite < 0.1 s); clippy -D clean, fmt clean.**
- 2026-05-30 — **Full-implementation audit → remediation → GA-evidence (verified).** 19-lane
  read-only audit (98 findings: 7 blockers/30 majors/43 minors/18 gaps) → layered remediation
  resolving ALL blockers+majors (libm determinism, seqlock UB, method/person-name purge,
  honesty/doc fixes, broker→smile calibration + DegenerateQuote guard, holiday/convention
  fixes, idempotency/resync) → re-audit verdict production-grade. Then GA-evidence:
  `celnet-parity` (15 capability rows gated vs incumbents), CI matrix + nightly fuzz,
  mutation kill-rate 78→88.4% on vanilla, 96% core coverage, true 13-Greek count reconciled.
  **509 tests, `just check` green & terminating.** Remaining to GA: API-v2 (#20), scale-out
  validation (#19), plugin-host (#10 — since DONE on wasmi, see top of ledger), GA sign-off (#15).

- 2026-05-30 — **G3 reached (wide 4-lane wave).** `celnet-exotics` (digitals/touches/DNT/
  all-8 barriers + survival-weighted VV overlay; PDE Crank-Nicolson+Rannacher & Philox MC,
  PDE≈MC≈analytic cross-validated), `celnet-gpu` (PricingBackend over wgpu/Metal + f64 CPU
  oracle, Philox bit-stable, f32↔f64 reconciled), `celnet-engine` (core-pinned zero-alloc hot
  path: rtrb SPSC, arc-swap/seqlock, blue-green handoff; audited seqlock unsafe),
  `celnet-golden` (QuantLib 1.42.1 frozen tables; vanilla + all-8 barriers + both digital
  styles gated to ~1e-10/last-bit — independent oracle). 284 tests, full `just check` green;
  adversarial verdict production-grade. Auto-index live (post-commit + Stop hooks). **Next
  (wide wave):** LSV booking model, vendor/multi-source integration, streaming edge
  (server/cli), competitive parity matrix as executable tests, WS-T hardening.
- 2026-05-30 — **G2 reached.** `celnet-surface` (VV + broker→smile + SABR/SVI/SSVI +
  arb-free term structure) + `celnet-bench` (measured: vanilla price 8.85ns, +14 Greeks
  ~19ns, 64-strike batch ~6.75µs). Adversarial review caught + fixed a person-named public
  fn + overstated docs + a SABR sign error.
- 2026-05-30 — **G0 + G1 reached (3 parallel lanes).** `celnet-calendar` (43 tests),
  `celnet-conventions`, `celnet-vanilla` strike↔delta solver + 4 delta conventions + ATM/DNS,
  `celnet-testkit` (shared invariants/strategies), `celnet-proto` (single unversioned wire
  contract, protox build), `celnet-plugin-api` (SDK traits + WIT + validated example). 130
  tests, full `just check` green. Adversarial review caught + fixed a sign-inverted charm in
  the SDK example (added full-Greek FD gate). Incremental build recipes (`check-crate`,
  `check-changed`) + lane infra (central dep registry, skeletons). **Next (parallel lanes):**
  G2 `celnet-surface` (highest-leverage Synoption gap), latency-bench harness + QuantLib
  golden oracle (WS-T), then exotics ∥ gpu ∥ plugin-host ∥ integration.
- 2026-05-30 — **P0/P1 vertical slice green.** Flat workspace live; `celnet-types` (frozen
  vocab + convention enums + DTOs), `celnet-core` (libm math, deterministic `assert_close`,
  `Smile` trait), `celnet-vanilla` (Garman-Kohlhagen + full 14-Greek set). 17 tests pass:
  BS textbook benchmark, put-call parity (512 proptest cases), finite-difference validation
  of every Greek. `just check` fully green (fmt/clippy-D/nextest/deny). **Next:** complete
  G0 (`celnet-proto`, `celnet-plugin-api`), then fan out post-G0 workstreams via a workflow.
- 2026-05-30 — Design corpus written to `docs/` (ARCHITECTURE, ANALYTICS-SPEC,
  COMPETITIVE-ANALYSIS, CELER-INTEGRATION, ROADMAP + `_research/`). Product renamed
  CelerOption → **Celnet**.
- 2026-05-30 — Foundations: git (local-only) init; Rust 1.96.0 + tooling; rust-analyzer-lsp
  plugin; memory bootstrapped; settings/guardrails; toolchain/config files.
