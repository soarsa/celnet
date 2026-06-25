# Verified knowledge — anchored mirror (committed source of truth)

Durable, anchor-carrying record of every Tier-1 lodestar claim. lodestar's
event log does not persist anchors, so THIS file is what survives a fresh clone or
projection loss. Rebuild the live projection: `python3 tools/lodestar/replay-knowledge.py`.

**172 claims** — kinds: a11y:labeled=2, a11y:role=2, design:token=1, invariant=19, invariant:pure=143, spec:satisfies=1, ui:component:badge=1, ui:component:dialog=1, ui:component:grid=1, ui:component:strip=1

states: active=151, draft=21

---

### 1. `a11y:labeled` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette`

CommandPalette is a labelled modal dialog implementing the ARIA APG combobox/listbox pattern: the surface is role="dialog" aria-modal="true" aria-label="command palette"; results render as role="listbox" with role="option" rows carrying aria-selected on the active item. Keyboard handling (ArrowUp/ArrowDown to move the active option, Enter to run, Escape to close) plus open-time focus of the search input make it fully keyboard-operable. This is the ⌘K command surface, so its dialog+listbox a11y contract is the reference for overlay launchers.

### 2. `a11y:labeled` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.SignInDialog.SignInDialog`

SignInDialog is a properly-labelled modal: the panel is role="dialog" aria-modal="true" aria-labelledby pointing at the generated id of its <h2> title (via React.useId), it focuses the email field on open and closes on Escape, and surfaces auth errors in a role="alert" live region. The labelledby-to-title wiring (not a hard-coded string) keeps the accessible name in lockstep with the visible heading.

### 3. `a11y:role` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid`

DataGrid is a WAI-ARIA APG composite grid: its root carries role="grid" with aria-label (the caller-supplied `label`), aria-rowcount and aria-colcount; the header band is role="row"/aria-rowindex=1 with role="columnheader" cells that expose aria-sort (ascending|descending|none) on sortable columns; body rows are role="row" with role="gridcell" cells, and group headers add aria-expanded. A single roving tabindex (one cell tabIndex=0, the rest -1) plus arrow/Home/End/PageUp/PageDown key handling makes it keyboard-operable as one composite widget. This is the shared virtualized grid powering the blotter/risk/book workspaces, so the grid-pattern a11y contract is centralized here.

### 4. `a11y:role` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge`

StatusBadge renders its stream-health glyph as role="img" with an aria-label (and matching title) derived from the StreamHealth state, so the purely-visual glyph is announced as a single labelled image to assistive tech rather than read as a bare emoji character. It is the StatusBadge story exercised by gui/storybook-static (components-statusbadge--* entries: Default/Healthy/Resyncing/Stale/All States), making it the canonical visual + a11y reference for stream-health status surfaces.

### 5. `design:token` (draft)
Anchors: `github.com-soarsa-celnet.gui.src.components.Button.Button`, `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid`

Token-driven styling (DRY single source of truth). Button and DataGrid render exclusively through CSS-Module classes whose declarations reference the Celnet Aurora design tokens by their emitted CSS custom properties (var(--token)) — e.g. Button.module.css carries 32 var(--…) references and zero raw color literals, DataGrid.module.css likewise. Every such custom property (--bg-base, --accent, --space-3, --r-md, --row-h, …) is the cssVar formalized in gui/design-tokens.json (W3C DTCG format), itself a verbatim formalization of gui/src/design/tokens.css. A repo-wide scan of all 46 gui/src CSS Modules found zero raw hex/hsl color literals and exactly one rgba() (a text-shadow in CubeWorkspace.module.css). The only inline literals in component TSX are token-read fallbacks (root.getPropertyValue('--…') || '#…' in Sparkline.tsx / SmileChart.tsx), which read the token first. Editing a token value in tokens.css therefore propagates to these components with no per-component restyle — the invariant the styling-DRIFT derive-rule guards.

### 6. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.core_load.CoreBudget.architecture_1_2`

PILLAR (CLAUDE.md guardrails 6 + 11 — scale & performance are requirements: the pricing hot core must stream prices to high-performance counterparties at the latency budgets in docs/ARCHITECTURE.md §1.2). `CoreBudget::architecture_1_2` is the single, authoritative encoding of the per-price hot-core tail-latency budget: a parameterless `const fn` returning `Self { p50_ns: 2_000, p99_ns: 10_000, p999_ns: 25_000 }` — i.e. the 2µs/10µs/25µs p50/p99/p99.9 envelope from ARCHITECTURE §1.2. Being a `const fn` over literals it is pure and side-effect-free by construction (no allocation, I/O, or mutation); it has no WRITES edges. The bench load harness (CoreReport::budget_breaches) measures the live core against exactly this struct, so the latency pillar is reconciled against a single source of truth rather than asserted. SELF-INVALIDATING: editing any budget constant (or the signature) shifts this anchor and flips the claim stale, re-opening review against the doc.

### 7. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.core_load.CoreReport.budget_breaches`

PILLAR (CLAUDE.md guardrails 6 + 11 — the latency budget in docs/ARCHITECTURE.md §1.2 is enforced, not aspirational). `CoreReport::budget_breaches` is the pure, deterministic detector that turns a measured hot-core latency profile into the set of budget violations: it reads only `&self` (the measured `latency` p50/p99/p99.9 and the `budget` ceilings), forms the three (metric, measured_ns, ceiling) tuples, and returns a freshly-allocated `Vec<BudgetBreach>` containing exactly those percentiles where measured > ceiling. No `&mut`, no I/O, no shared/global mutation — allocating the returned Vec is not an externally-observable side effect; it has no WRITES edges. Determinism is the load-bearing property: the same measured profile against the same budget always flags the same breaches, so a CI gate cannot non-reproducibly hide a regression past the §1.2 envelope. SELF-INVALIDATING: introducing a write or changing the comparison shifts this anchor and flips the claim stale.

### 8. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.surface_rebuild.SurfaceBudget.architecture_1_2`

PILLAR (CLAUDE.md guardrails 6 + 11 — IB-sized portfolios / many-instrument batch must rebuild surfaces within the docs/ARCHITECTURE.md §1.2 budget). `SurfaceBudget::architecture_1_2` is the authoritative encoding of the per-surface-rebuild p99 latency budget: a parameterless `const fn` returning `Self { p99_ns: 150_000 }` — the 150µs p99 surface-rebuild ceiling from ARCHITECTURE §1.2. Being a `const fn` over a literal it is pure and side-effect-free by construction (no allocation, I/O, or mutation); it has no WRITES edges. The surface_rebuild load harness (SurfaceReport::budget_breaches) measures each model's live rebuild p99 against exactly this struct, so the surface-scale pillar is reconciled against a single source of truth. SELF-INVALIDATING: editing the constant or signature shifts this anchor and flips the claim stale.

### 9. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.surface_rebuild.SurfaceReport.budget_breaches`

PILLAR (CLAUDE.md guardrails 6 + 11 — many-instrument surface rebuild must stay inside the docs/ARCHITECTURE.md §1.2 p99 budget). `SurfaceReport::budget_breaches` is the pure, deterministic detector over a multi-model surface-rebuild report: it reads only `&self`, iterates the measured per-model `latency.p99_ns`, filters those exceeding `budget.p99_ns`, and returns a freshly-allocated `Vec<BudgetBreach>` (each carrying the static model name, the "p99" metric, the measured ns and the ceiling). No `&mut`, no I/O, no shared/global mutation — allocating the returned Vec is not an externally-observable side effect; it has no WRITES edges. Determinism guarantees a per-model p99 regression past the §1.2 surface budget is flagged reproducibly. SELF-INVALIDATING: a write or comparison change shifts this anchor and flips the claim stale.

### 10. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryInputs.discount_df`

CarryInputs::discount_df() is a pure accessor delegating to self.carry.discount_df(self.t) — no side effects, no mutation. For an FX Carry::FxRates carry this is byte-identical to VanillaInputs::df_dom (the FX two-rate discount), since the underlying Carry::discount_df reads r_dom verbatim. This is the generalized carry-tagged discount used by the cross-asset pricing seam (celnet-core CarryInputs / CarryPricer), proved byte-identical for FX by fx_carry_inputs_byte_identical (ADR-0008).

### 11. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_vanilla_inputs`

fx_vanilla_inputs(inputs) is a pure (side-effect-free) lowering from the generalized CarryInputs to the FX leaf's VanillaInputs. It accepts only FX-family underlyings (Underlying::Fx | Underlying::Metal — a metal's lease rate is modelled as the FX foreign rate) carried by Carry::FxRates, and typed-rejects every other underlying (Equity/Commodity/DigitalAsset) or a Carry::CostOfCarry carry with CarryPriceError rather than silently mis-pricing under FX arithmetic. The rate mapping is a verbatim field copy of (r_dom, r_for) — no recomputation — so the resulting forward/df_dom/df_for are byte-identical to CarryInputs::forward/discount_df (proved by fx_carry_inputs_byte_identical; ADR-0008 metals byte-identity).

### 12. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.exp`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": "Transcendentals via rust-lang/libm (correctly-rounded) for bit-identical cross-platform results"). `celnet_core::math::exp` is the canonical e^x transcendental wrapper: it is `pub fn exp(x: f64) -> f64 { libm::exp(x) }` — a pure (side-effect-free, no WRITES) total delegation to the correctly-rounded rust-lang/libm routine rather than std's platform libm, so every consumer (e.g. norm_pdf, Carry::forward_factor/discount_df via libm::exp) gets a single deterministic, cross-platform-identical exponential. The whole pricing core routes through this one wrapper so f64 results are byte-reproducible regardless of host C library.

### 13. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.ln`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": transcendentals via rust-lang/libm for bit-identical cross-platform results). `celnet_core::math::ln` is the canonical natural-logarithm transcendental wrapper: it is `pub fn ln(x: f64) -> f64 { libm::log(x) }` — a pure (side-effect-free, no WRITES) total delegation to the correctly-rounded rust-lang/libm `log` routine (note libm::log == ln, not log10). Routing every ln(·) (e.g. log-moneyness ln(F/K) in d1/d2) through this one libm-backed wrapper keeps the f64 CPU-canonical result identical across hosts, never std's platform-dependent ln.

### 14. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.sqrt`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": transcendentals via rust-lang/libm for bit-identical cross-platform results; f64 is the CPU-canonical scalar). `celnet_core::math::sqrt` is the canonical square-root wrapper: it is `pub fn sqrt(x: f64) -> f64 { libm::sqrt(x) }` — a pure (side-effect-free, no WRITES) total delegation to rust-lang/libm. Routing every sqrt(·) (e.g. sigma*sqrt(T) total variance/vol-time scaling) through this one libm-backed wrapper makes the f64 result byte-identical across platforms rather than depending on the host math library.

### 15. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc_cut`

ADR-0007 (one clean unversioned contract — engine hot-upgrade handoff, ENCODE side; the inverse of dec_cut). `enc_cut(v: Cut) -> u8` is the pure, total encoder of the expiry-Cut discriminant into the single handoff byte image: NewYork1000->0, Tokyo1500->1, with no wildcard arm, so adding a Cut variant is a compile-time break rather than a silent mis-encode. It reads only its `Cut` argument and writes nothing (pure: no side effects, no allocation). Because there is exactly ONE current contract (guardrail 9: no schema_version, no N/N-1 negotiation), enc_cut/dec_cut are a fixed mutually-inverse codec pair consumed by serialize_state/restore_state — a hot-upgrade deploys a single uniform version with no mixed-version window, so the byte mapping needs no version tag. This is the decision/trade-off: a tagless 1-byte discriminant (cheapest, deterministic) is sound precisely because the unversioned-contract rule removes any back-compat obligation.

### 16. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.restore_state`

ADR-0007 (one unversioned contract — engine hot-upgrade handoff, decode side). `restore_state(bytes)` is the single deterministic DECODER of the engine handoff byte image and the exact inverse of `serialize_state`: it is a pure function of its borrowed `&[u8]` (it reads no global/external state and mutates none — the only allocations are the returned BookState::entries Vec and the reconstructed MarketState), so the same bytes always yield the same (MarketState, BookState). It validates the fixed self-describing layout — a leading MAGIC sentinel (BadMagic on mismatch), then market scalars (spot, r_dom, r_for, t), conventions, the three smile benchmark pillars + reference forward/time from which MarketHedgeSmile::new reconstructs an identical smile, then the length-prefixed book — and calls Reader::finish() to reject trailing bytes, never a silent default. DECISION/RATIONALE: there is NO schema_version field and NO N/N-1 negotiation; hot-upgrade carries state across a code swap through this ONE current format with only a MAGIC discriminant. An upgrade deploys a single uniform engine version (old build serializes, new build restores) so there is no mixed-version window — the format evolves in place rather than versioning, consistent with the platform-wide single-unversioned-contract decision and proved exact by roundtrip_restores_identical_state / restored_state_reprices_identically. (Guardrail: no versioned APIs; hot-upgradable single-version estate. Pairs with cl_d9376a70dfdbd751 on serialize_state.)

### 17. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.ExoticInputs.carry_df`

ExoticInputs::carry_df() is a pure accessor computing the yield/foreign discount factor e^{-q*t} as exp(-self.carry.yield_rate() * self.t) — no side effects. Crucially it reads the yield rate q through Carry::yield_rate(), which for Carry::FxRates returns the STORED r_for verbatim; it must NEVER reconstruct q as discount_rate() - carry_rate() (r_dom - (r_dom - r_for) does not round-trip bit-for-bit). This keeps the foreign-leg discount e^{-r_for*t} byte-identical to the FX two-rate form on the exotics carry seam (ADR-0008 FX bit-identity).

### 18. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.gpu.GpuBackend.label`

ADR (GPU abstraction = wgpu baseline + CPU-SIMD fallback, CUDA optional) — the wgpu side. `GpuBackend::label(&self)` is a pure accessor (reads only &self, mutates nothing; its only allocation is the returned String): it reports the live wgpu adapter identity — `self.context.backend_name` when a GPU context was acquired — and otherwise falls back to "<cpu label> (fallback)" by delegating to the inner CpuBackend. It is the wgpu-side twin of CpuBackend::label (cl_555e1a46705ef9aa) within the shared `PricingBackend` trait: both implement the same simulate_paths/reduce_payoff/price_vanilla contract behind one interface, with Philox path index i fixed across backends so results reconcile modulo the f32/f64 element type. DECISION/RATIONALE: the GPU strategy is wgpu (Metal/Vulkan/DX12) as the open, permissively-licensed baseline chosen OVER a CubeCL/CUDA-first design — wgpu keeps the runtime dependency set fully open-source and portable across the M4/Metal dev box and Linux/Vulkan CI, with an optional CUDA backend behind the same trait and the f64 CPU path always available as oracle and fallback. label() encodes exactly that runtime selection: a concrete adapter name when wgpu binds a device, the "(fallback)" CPU identity when none is present — so the same single-trait code runs portably whether or not a GPU adapter exists. (Guardrail: no commercial products; open GPU stack with wgpu first-class. Metal lacks f64, so cross-backend agreement is asserted to f32 tolerance, never bit-identity.)

### 19. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.host.guest_store_limits`

ADR / ROADMAP §6.5 plugin trust tiers (untrusted wasm guest = default, sandboxed + fuel-metered; wasmi chosen over wasmtime; abi_stable banned). `guest_store_limits() -> StoreLimits` is the single source of the deny-by-default guest RESOURCE policy and a pure builder (param-free, no side effects, no reads of mutable state): it caps memory_size=MAX_GUEST_MEMORY_BYTES, table_elements, memories, tables, instances to fixed maxima and sets trap_on_grow_failure(true) so a guest that tries to exceed its budget TRAPS (surfaced as HostError::ResourceLimit) rather than allocating. This is the trade-off behind the wasmi-over-wasmtime decision: wasmi is a pure-Rust, no-JIT, dependency-light interpreter whose StoreLimiter + consume_fuel give bounded CPU AND bounded memory with no native codegen attack surface — the deliberate cost is interpreter throughput, accepted because untrusted third-party plugins run here while the trusted first-party path is Tier-0 native (NativeModel::price), so the hot first-party path pays nothing for the sandbox.

### 20. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.map_instantiation_error`

ADR (plugin-host sandbox = wasmi, fuel-metered; chosen over wasmtime) — the error-taxonomy side. `map_instantiation_error(e: &wasmi::Error) -> HostError` is the pure, total classifier (reads only the borrowed wasmi error, mutates nothing; its only allocations are the returned short() strings) that lowers a wasmi LOAD/instantiate fault into the host's stable error enum: TrapCode::OutOfFuel -> FuelExhausted, TrapCode::GrowthOperationLimited -> ResourceLimit, "resource limiter denied" -> ResourceLimit, an unknown/missing import -> CapabilityDenied (deny-by-default capability surface), everything else -> InvalidModule. This mapping is the concrete embodiment of the wasmi decision: it depends on wasmi's specific as_trap_code()/TrapCode vocabulary, so the host error contract is defined in terms of the chosen interpreter's fault model — the trade-off being that swapping engines would re-write this seam, accepted because wasmi's pure-Rust trap codes give deterministic, panic-free, hang-free fault classification (no JIT codegen failure modes to model).

### 21. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.discount_df`

Carry::discount_df(t) is a pure function computing the numeraire discount factor e^{-r*t} via libm::exp(-self.discount_rate() * t). It has no side effects and no state mutation — its output depends only on (self, t). Because discount_rate() reads r_dom verbatim for Carry::FxRates, the FX discount factor produced here is byte-identical to VanillaInputs::df_dom (the FX two-rate form), preserving FX bit-identity across the carry seam (ADR-0008). CarryInputs::discount_df and ExoticInputs::discount_df both delegate to this method.

### 22. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.discount_rate`

Carry::discount_rate() is a pure const accessor returning the numeraire/discount rate r used in e^{-r*t}: the stored r_dom verbatim for Carry::FxRates and the stored r verbatim for Carry::CostOfCarry. No side effects, no allocation, no Carry construction — a plain field read by match. This is the single asset-class-agnostic source of the discount rate on the carry seam; every leaf (vanilla/exotics/surface/risk and the crypto/equity/commodity leaves) reads r through here rather than matching on Carry or Underlying (ADR-0008 carry-seam architecture, docs/adr/ADR-0008-multi-asset-carry-architecture.md). For FX, discount_rate() == r_dom exactly, which underpins the FX df_dom byte-identity.

### 23. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.forward_factor`

Carry::forward_factor(t) is a pure function computing the outright forward factor e^{b*t} via libm::exp(self.carry_rate() * t), where b is the net cost-of-carry (carry_rate() == r_dom - r_for for Carry::FxRates, == b for Carry::CostOfCarry). Multiplying spot by this factor yields the forward F = S*e^{b*t}; it has no side effects and depends only on (self, t). For FX this reproduces VanillaInputs::forward bit-for-bit, the single forward-construction primitive shared by every carry-seam leaf (ADR-0008).

### 24. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.gui.src.components.Button`, `github.com-soarsa-celnet.gui.src.components.GreeksStrip`, `github.com-soarsa-celnet.gui.src.components.Panel`, `github.com-soarsa-celnet.gui.src.components.PriceTile`, `github.com-soarsa-celnet.gui.src.components.Sparkline`

Baseline Storybook stories exist co-located with the 5 most important exported components under gui/src/components: Button, PriceTile, Panel, Sparkline, and GreeksStrip. Each story file (*.stories.tsx) uses the Meta/StoryObj pattern from @storybook/react, references only Aurora design tokens (CSS custom properties from --bg-*, --text-*, --bid, --offer, --space-*, etc.) — never raw hex or inline color literals — and is additive (the component files themselves are unmodified). The StatusBadge.stories.tsx scaffold story pre-existed; these 5 are the new lane-components deliverable.

### 25. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.benches.iai_instructions.soft_regression_limits`

DELIVERABLE bench/iai-instruction-gate-no-limits = LANDED (backlog tracker still lists it OPEN as a Round-2 P1/S finding; reconciled against the live graph). `soft_regression_limits` is a pure (side-effect-free) builder returning the iai-callgrind regression config: it constructs Callgrind::default().soft_limits([(EventKind::Ir, SOFT_INSTRUCTION_REGRESSION_PCT), (EventKind::EstimatedCycles, SOFT_ESTIMATED_CYCLES_REGRESSION_PCT)]) with no external writes. The Round-2 gap (the iai instruction-count regression gate was structurally unable to fail because NO RegressionConfig/soft_limit/hard_limit existed — the lane always exited 0) is CLOSED: the gate now carries explicit per-EventKind percentage soft limits on instructions (Ir) and estimated cycles, applied via instruction_gate, so an instruction-count regression beyond the band now flags. SELF-INVALIDATING: removing or editing the limit construction shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.

### 26. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.actual_days`

`actual_days` is the signed day-count primitive underlying every `DayCount` accrual: it returns `(end - start).whole_days()` as an `i64`, so it is signed (negative when end precedes start) and counts whole days only. This signedness is what makes `year_fraction` anti-symmetric under interval reversal; it is the sole bridge from the `time::Date` calendar type into the ACT/365 and ACT/360 numerators. Pure: reads two dates, returns i64, no side effects.

### 27. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.signed_when_reversed`

`signed_when_reversed` pins the anti-symmetry of the day-count year fraction: with `DayCount::Act365Fixed`, `year_fraction(basis, 2024-01-01, 2023-01-01)` equals −1.0 (asserted via `assert_close!`, the sanctioned float comparator, never `==`). It guards that a reversed accrual interval yields the exact negative year fraction — the property exotic/vol-time accrual relies on for signed time spans — and that the 2024→2023 span is exactly 365 days over the ACT/365 denominator. Pure test: builds dates and asserts via assert_close!, mutating no external state.

### 28. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.year_fraction`

`year_fraction` is the canonical realization of the `DayCount` convention: it divides the actual day count by the basis-selected denominator — 365.0 for `DayCount::Act365Fixed`, 360.0 for `DayCount::Act360` — via an exhaustive match with no wildcard, returning a `Time`. Because the day count is signed (`actual_days` = end − start in whole days), an end strictly before start yields a negative year fraction (the reversed-interval property), so the function is anti-symmetric in (start,end) by construction. This is the one place the celnet-types `DayCount` enum becomes a numeric accrual factor; ACT/365-fixed (vol-time) and ACT/360 (money-market) are kept deliberately distinct (docs/CONVENTIONS.md). Pure: reads basis and the two dates, returns Time, no mutation.

### 29. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.cli.resolve_priced_expiry`

DELIVERABLE cli-stream-rfq-tenor-expiry-drift = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "--tenor 3M silently streams a 1Y-priced quote labelled 3M"; reconciled against the live graph). `resolve_priced_expiry(pair, tenor, explicit, horizon)` is a pure, deterministic function (reads only its borrowed args, returns Result<f64, DispatchError>, mutates nothing) that DERIVES the priced expiry-year-fraction from the requested tenor via celnet_conventions::vol_year_fraction over the pair's calendar, and — when an explicit --expiry-years is also supplied — rejects any value that drifts from the tenor-derived anchor beyond the abs/rel tolerance with DispatchError. The label and the priced expiry can no longer drift apart silently across the CLI stream/rfq seams (regression-pinned by stream_and_rfq_reject_a_contradictory_tenor_expiry_pair and priced_expiry_derives_from_tenor_via_the_conventions_calendar). SELF-INVALIDATES on any change to this resolver.

### 30. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price`

CAPABILITY (commodity cross-asset leaf): celnet-commodity-vanilla::price is the Black-76 commodity/future-option pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &CommodityInputs) that discounts the forward directly (no spot carry), reconciled to Haug's published Black-76 reference and an independent QuantLib-pinned oracle. It is the commodity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.

### 31. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_consistent`

Convention cross-field consistency invariant (docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention coupling): ConventionRecord::is_consistent is a pure total predicate asserting self.premium_style.is_premium_adjusted() == self.is_delta_premium_adjusted() — i.e. a record is consistent exactly when its premium style and its delta convention agree on premium-adjustment. const fn, no writes/allocation/IO; deterministic. Self-invalidates if either underlying mapping or this coupling changes (WRITES gate).

### 32. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_delta_forward`

Convention forward-delta axis (docs/CONVENTIONS.md DeltaConvention; short tenors quote spot delta, long tenors switch to forward/driftless delta). `ConventionRecord::is_delta_forward` is a pure total function of self.delta — returns true exactly for the two forward variants DeltaConvention::ForwardUnadjusted and DeltaConvention::ForwardPremiumAdjusted, false for the two spot variants. It is the orthogonal counterpart to is_delta_premium_adjusted: together the (is_delta_forward, is_delta_premium_adjusted) pair decomposes DeltaConvention into its two independent boolean axes (the axes premium_adjusted_of recomposes). const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 33. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_delta_premium_adjusted`

Convention premium-adjusted mapping (docs/CONVENTIONS.md DeltaConvention→premium-style): ConventionRecord::is_delta_premium_adjusted is a pure total function of self.delta — returns true exactly for the two premium-adjusted variants DeltaConvention::SpotPremiumAdjusted and DeltaConvention::ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 34. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_non_deliverable`

Settlement→deliverability mapping (docs/CONVENTIONS.md Settlement enum → celnet-types): ConventionRecord::is_non_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, false otherwise. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the Settlement variant set or this match arm changes (WRITES gate).

### 35. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_non_deliverable`

Pair-universe settlement classification (docs/CONVENTIONS.md Settlement → registry PairMeta): PairMeta::is_non_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, mirroring ConventionRecord::is_non_deliverable so the registry-level and resolved-record classifications agree. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or Settlement variants change (WRITES gate).

### 36. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_precious_metal`

Pair-universe asset-class classification (docs/CONVENTIONS.md InstrumentClass → registry PairMeta): PairMeta::is_precious_metal is a pure total function of self.instrument — returns true exactly for InstrumentClass::PreciousMetal (the loco-London metal-leg pairs), false for fiat. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or InstrumentClass variants change (WRITES gate).

### 37. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_self_consistent`

Pair-universe structural self-consistency (docs/CONVENTIONS.md PairMeta entry invariants): PairMeta::is_self_consistent is a pure total predicate over its own fields returning false on any contradiction — NDF terms present iff non-deliverable; the cash-settlement currency (when present) is one of the pair's two legs; the premium currency is one of the pair's legs and agrees with the premium-adjusted flag; and the spot lag is the canonical T+1 or T+2. No writes/allocation/IO; deterministic, reads only self. Self-invalidates if the invariant set or any field-coupling changes (WRITES gate).

### 38. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.accrual_basis`

Accrual day-count is a CURRENCY property, not a pair property (docs/CONVENTIONS.md DayCount → celnet-types::DayCount). `accrual_basis(ccy)` is a pure total function mapping each Ccy to its money-market money accrual basis: GBP, AUD and NZD accrue ACT/365-fixed (DayCount::Act365Fixed); every other currency accrues ACT/360 (DayCount::Act360). Because it keys on the single currency leg (not the pair), AUD as the foreign leg accrues ACT/365 whether the pair is a covered major (AUDUSD), a covered G10 cross (AUDJPY), or a region-default-derived uncovered cross (AUDPLN) — the single source of truth the registry's accrual_basis_is_single_source_of_truth test pins. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the currency→basis mapping changes (WRITES gate).

### 39. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is_long_tenor`

The spot-vs-forward delta switch is a one-year tenor threshold (docs/CONVENTIONS.md DeltaConvention: short tenors quote spot delta, long tenors switch to forward/driftless delta). `is_long_tenor(tenor)` is a pure total predicate returning tenor_days(tenor) > 365 — STRICTLY greater, so exactly-one-year tenors (Tenor::Years(1) and Tenor::Months(12), both 365 days) classify as SHORT (spot delta) and 18M / 2Y classify as LONG (forward delta), exactly as long_tenor_threshold_is_one_year pins. This boolean is the `forward` axis fed to premium_adjusted_of in region_default. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the threshold or tenor_days mapping changes (WRITES gate).

### 40. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.premium_adjusted_of`

DeltaConvention is the cartesian product of two independent booleans (docs/CONVENTIONS.md DeltaConvention; docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention). `premium_adjusted_of(forward, premium_adjusted)` is the const-fn total constructor that composes the (forward?, premium-adjusted?) flags back into the four-variant enum: (false,false)→SpotUnadjusted, (false,true)→SpotPremiumAdjusted, (true,false)→ForwardUnadjusted, (true,true)→ForwardPremiumAdjusted. The match is exhaustive over both booleans, so no combination is defaulted — it is the exact inverse of the record predicates is_delta_forward (the `forward` axis) and is_delta_premium_adjusted (the `premium_adjusted` axis). Pure: no writes/allocation/IO; deterministic. Self-invalidates if the DeltaConvention variant set or the flag→variant mapping changes (WRITES gate).

### 41. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region_default`

The fall-through (uncovered-pair) convention record is assembled deterministically from per-currency and per-region rules (docs/CONVENTIONS.md house-default conventions; ResolutionSource::RegionDefault). `region_default(pair, tenor)` is a pure total function building a ConventionRecord with: cut = Tokyo1500 for a Tokyo-region pair else NewYork1000 (via region_of); premium_style = PercentForeign; delta = premium_adjusted_of(is_long_tenor(tenor), premium_style.is_premium_adjusted()) — so the spot/forward axis follows the tenor and the premium-adjusted axis follows the premium style; atm = DeltaNeutralStraddle; day_count_vol = Act365Fixed; the foreign and domestic accrual day-counts = accrual_basis(pair.base) and accrual_basis(pair.quote) respectively (per-currency, not per-pair); settlement = Deliverable. No combination is defaulted ad hoc — every field is a documented function of (pair, tenor). const-style assembly, no writes/allocation/IO; deterministic. Self-invalidates if any of the composed mapping helpers or the default field set changes (WRITES gate).

### 42. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region_of`

The expiry-cut region is decided by the QUOTE currency (docs/CONVENTIONS.md Cut → New York 10:00 vs Tokyo 15:00). `region_of(pair)` is a pure total function returning Region::Tokyo exactly when pair.quote == Ccy::JPY, else Region::NewYork — the JPY-region/Asian business books the Tokyo 15:00 cut, every other pair the New York 10:00 cut. It keys on the quote leg only (the JPY pairs are quoted XXXJPY), so the region/cut is a deterministic function of the pair, never of spot or tenor. const-foldable, no writes/allocation/IO. Self-invalidates if the region-selection rule or Region/Ccy variant set changes (WRITES gate).

### 43. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.tenor_days`

tenor_days is the exhaustive nominal-horizon primitive that drives the short/long delta-convention classification (docs/CONVENTIONS.md tenor axis). It is a pure total function over the whole Tenor enum returning u32 days: the pre-spot short end Overnight | TomNext | SpotNext → 1 (and BrokenDate → 1, the conservative short default since the exact pricing axis is set later by the pricer from the resolved expiry, not here); Weeks(w) → 7·w; Months(m) → (365·m + 6)/12 (the 365/12 ≈ 30.4167 days-per-month rounded to nearest day, so Months(12) = 365 and Months(6) = 183); Years(y) → 365·y; Imm(n) → (3·n·365 + 6)/12 (≈ 3 months per IMM step). The match is exhaustive over Tenor, so no variant is defaulted, and it is consumed by is_long_tenor (>365 ⇒ forward delta). const-foldable integer arithmetic, no writes/allocation/IO; deterministic. Self-invalidates if the Tenor variant set or any per-variant day formula changes (WRITES gate).

### 44. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_carry_inputs_byte_identical`

The carry-seam FX byte-identity is enforced by fx_carry_inputs_byte_identical: the FX arm of Carry (FxRates) lowers to VanillaInputs with forward/df_dom/df_for bit-identical (to_bits) to the native FX leaf, and the CostOfCarry arm is rejected (UnsupportedCarry). This is a pure byte-identity gate over the carry seam (ADR-0008). Supersedes a withdrawn spec:satisfies probe whose design-target sentinel did not resolve in this build.

### 45. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.compare.is_close`

Determinism rule — `is_close` is the canonical float comparator and never uses `==` semantics that would misbehave on the special cases: it returns false whenever either operand is NaN (NaN is never close to anything, including itself); it short-circuits true on bitwise `a == b`, which deliberately also makes +0.0 and -0.0 close even at zero tolerance and makes equal infinities close; it returns false for unequal infinities; otherwise it accepts when the absolute difference is within `abs` OR within `rel * max(|a|,|b|)` (a combined absolute-or-relative band). Tolerances are debug-asserted finite and non-negative. This is the single comparator behind `assert_close!`; the interface determinism rule (docs/INTERFACES.md) is that all float comparison flows through it — never a bare `==`, never an assert on a NaN payload. Pure: reads only its four f64 args, returns a bool.

### 46. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf`

Determinism rule — `norm_cdf` computes the standard-normal CDF as `0.5 * libm::erfc(-x * INV_SQRT_2)`, routing the transcendental through `rust-lang/libm` (correctly-rounded) rather than the platform libm, so the result is bit-identical across targets (the cross-platform determinism guarantee of docs/INTERFACES.md). Using the complementary error function `erfc` on `-x·1/√2` keeps the deep left tail stable (no catastrophic cancellation), which is why the tail tests pass. f64 is the CPU-canonical scalar. Pure: maps one f64 to one f64 via libm, no side effects, no state.

### 47. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf_deep_tail_matches_reference`

Determinism rule — `norm_cdf_deep_tail_matches_reference` pins `norm_cdf` against high-precision reference values deep in the left tail: Φ(−1)=0.15865525393145705, Φ(−5)=2.866515718791939e-7, Φ(−10)=7.619853024160525e-24, each via `assert_close!` with explicit rel/abs tolerances (never `==`). Because `norm_cdf` routes through `libm::erfc` (correctly-rounded), these exact-digit references encode the bit-stable, cross-platform tail behaviour; a regression that dropped the erfc routing (reintroducing catastrophic cancellation) would fail here. Pure test: evaluates norm_cdf and asserts, no mutation.

### 48. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf_tail_symmetry_and_no_underflow`

Determinism rule — `norm_cdf_tail_symmetry_and_no_underflow` guards the tail-stability that the `libm::erfc` routing in `norm_cdf` buys: it checks the reflection identity Φ(−x)=1−Φ(x) at x∈{3,4,5} (the largest x where the RHS is still representable before it underflows to 0), pins Φ(−15)=3.670966199312858e-51 and Φ(−20)=2.753624118606331e-89 against high-precision references, and asserts Φ(−37)>0 (≈5.7e-300, never flushed to zero). All comparisons go through `assert_close!`, never `==`. A regression that reintroduced the cancellation-prone 1−Φ(x) form on the direct path would be caught. Pure test: evaluates norm_cdf and asserts, no mutation.

### 49. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_pdf`

Determinism rule — `norm_pdf` computes the standard-normal density as `INV_SQRT_2PI * exp(-0.5 * x * x)` where `exp` is the crate's `libm`-backed wrapper, so the transcendental is correctly-rounded and bit-identical across platforms (docs/INTERFACES.md cross-platform determinism). The argument is symmetric in x (x·x), so norm_pdf is exactly even. f64 is the CPU-canonical type. Pure: maps one f64 to one f64, no side effects.

### 50. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.price`

CAPABILITY (crypto inverse/coin-margined leaf): celnet-crypto-vanilla::inverse::price is the inverse (coin-margined, 1/S_T payoff) crypto vanilla pricing entry — a pure, side-effect-free closed form taking (OptionType, &InverseInputs) whose value is expressed in the coin numeraire via the k/F and e^{sigma^2 t} convexity terms (norm_cdf of d2/d3), reconciled to an independent oracle with a signed convexity sandwich. It is the crypto inverse capability's projection target on the carry seam through the one contract; the sibling linear (USDT-margined) crypto path collapses to the Black-76 forward limit at zero carry. No I/O, allocation, logging, or mutation.

### 51. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.settlement.route_price`

CAPABILITY (carry-seam deliverable, crypto leaf reach): route_price is the pure crypto settlement-style dispatcher that reaches both crypto vanilla leaves on the shared Carry seam — SettlementStyle::Linear → linear::price (GK-funding, USDT/coin-quoted) and SettlementStyle::InverseCoin → inverse::price (inverse/coin-margined 1/S_T payoff) — selecting the leaf by settlement style and forwarding the same (spot, strike, vol, t, Carry). Pure: returns the leaf price from value/ref args with no WRITES; self-invalidates if either leaf arm gains a side effect.

### 52. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec_cut`

Engine-handoff Cut codec (docs/CONVENTIONS.md Cut enum; engine state serialization): dec_cut is the pure total inverse of enc_cut over the Cut discriminant — decodes byte 0→Cut::NewYork1000, 1→Cut::Tokyo1500, and any other byte to Err(HandoffError::BadDiscriminant), never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the discriminant assignment or Cut variant set changes (WRITES gate).

### 53. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.serialize_state`

ADR-0007 (one unversioned contract — engine hot-upgrade handoff). `serialize_state` is the single, deterministic encoder of the engine's live state (MarketState + BookState) into the handoff byte image: it is a pure function of its two borrowed inputs (it reads no global/external state and mutates none — the only allocation is the returned Vec<u8>), so the same (market, book) always yields byte-identical output. The image is a fixed self-describing layout — MAGIC header, market scalars, conventions, the three smile benchmark pillars + reference forward/time (exactly the state from which MarketHedgeSmile::new reconstructs an identical smile), then the length-prefixed book — and `restore_state` is its exact inverse (round-trip proven by roundtrip_restores_identical_state / restored_state_reprices_identically). DECISION/RATIONALE: hot-upgrade carries state across a code swap through this ONE current handoff format with a MAGIC sentinel and NO schema_version field and NO N/N-1 negotiation — consistent with the platform-wide single-unversioned-contract decision (ADR-0007). An upgrade deploys a single uniform engine version: the old build serializes, the new build restores; there is no mixed-version window to negotiate, so the format evolves in place rather than versioning. (Guardrail: no versioned APIs; hot-upgradable single-version estate.)

### 54. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.decision`

DELIVERABLE entitlements-trust-boundary-audit = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/M finding "entitlements trust boundary inverted — omitted principal ⇒ grant-all — and the documented per-decision audit is unimplemented"; Round-4 fixed the client-default half at 87c6f77; reconciled against the live graph). `AccessReason::decision(self)` is the pure, total, const reason→decision mapping that makes every access outcome first-class and auditable: PrincipalAsserted / PermissiveAbsent / SessionAuthenticated → Allow, and PrincipalAbsent / MalformedPrincipal / SessionInsufficientRole → Deny. A genuinely-absent or malformed principal now resolves to Deny (deny-by-default at the boundary), and each AccessReason is the per-decision audit datum emitted via the celnet-observability AuditSink — closing both halves of the finding (the inverted boundary and the missing per-decision audit; pinned by reason_determines_decision + the server entitlements_boundary::decision_records test). SELF-INVALIDATES on any change to this decision mapping.

### 55. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.admits`

SAFETY — entitlement cube-pruning admission is deny-by-default at the fact level (deliverable: entitlements-deny-by-default). EntitlementFilter::admits(fact) is a pure, deterministic predicate that delegates to the deny-first Principal::admits(hierarchy, &fact.key): it returns false on any matching deny rule (deny wins over grant) and otherwise requires an explicit grant whose every scope covers the fact — so an un-granted fact is never admitted. It reads only &self (borrowed principal + hierarchy) and the fact key; no mutation, I/O, or allocation. This is the exact per-fact gate that entitled_cube / prune apply when projecting a risk cube to a principal, so an information-barrier breach cannot leak a fact the principal was not explicitly granted. Self-invalidates if admits stops delegating to the deny-first principal evaluation.

### 56. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.admits`

Entitlement visibility is deny-by-default and information-barrier-first: `Principal::admits` evaluates the deny rules before anything else and returns false on any deny match (deny wins over grant), then admits unconditionally only for the grant-all principal (`all == true`), and otherwise requires at least one grant rule to cover the fact. A non-grant-all principal with no matching grant admits nothing. The decision is a pure read over `denies`/`grants`/`all` and the supplied hierarchy+key; it mutates no state.

### 57. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Rule.covers`

A single entitlement `Rule` covers a fact only if EVERY one of its scopes covers it (`scopes.iter().all`) — scope conjunction, so adding a scope narrows a rule, never widens it. The match is a pure read over the rule's scopes against the hierarchy and fact key; no state is mutated.

### 58. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Scope.covers`

A single `Scope` covers a fact iff the fact's resolved group value on the scope's dimension equals the scope's value (`resolved_group_value(hierarchy, key, dimension) == value`) — an exact equality on one hierarchy dimension, the atom from which `Rule::covers` (scope-conjunction) and `Principal::admits` are built. Pure: it only reads the hierarchy and fact key.

### 59. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.resolved_group_value`

Entitlements deny-by-default scope-resolution safety (deliverable: entitlements-deny-by-default). resolved_group_value is a pure, deterministic resolver: given (&Hierarchy, &FactKey, DimensionId) it maps Desk/Entity through the hierarchy (falling back to the key's own group value when no parent edge exists) and passes every other dimension straight through key.group_value, with no I/O and no mutation of its borrowed inputs. Determinism is the safety property — a grant's scope.covers test resolves a fact to exactly the same governing node on every evaluation, so a principal can never be admitted to a scope the deny-by-default rule did not actually grant. Pure (no WRITES edges); self-invalidates on change.

### 60. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.price`

CAPABILITY (equity cross-asset leaf): celnet-equity-vanilla::price is the generalized-BSM equity vanilla pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &EquityInputs) where the dividend yield enters as the carry b = r - q, so the no-dividend limit collapses to standard Black-Scholes (proven by no_dividend_limit_is_standard_bsm) and the leaf reconciles to an independent QuantLib-pinned BSM oracle. Heavily re-used (in_degree 147) as the equity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.

### 61. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.carry_vanilla_price_at`

CAPABILITY (cross-asset carry-seam reach): carry_vanilla_price_at is the generalized closed-form pricing kernel of the cross-asset carry seam — a pure, side-effect-free generalized-Black-Scholes-Merton evaluation parameterized by a generalized Carry (cost-of-carry b = r - q via Carry::discount_rate/yield_rate). Because every asset family lowers onto this one Carry-parameterized kernel (FX as r_dom/r_for, equity as r/dividend-yield, commodity as Black-76 r/b, crypto-linear as r/funding), the SAME pure kernel reaches vanilla/exotics/surface/risk across the FX, equity, commodity, and crypto/digital-asset and linear leaves. It performs no I/O, allocation, logging, or mutation: d1/d2 and discounted spot/strike are computed and one branch on OptionType returns the price.

### 62. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.lsv.LsvModel.calibrate`

DELIVERABLE lsv/market-calibration-frontend = PARTIAL / still OPEN (backlog tracker docs/WORLD-CLASS-BACKLOG.md Round-2 P2/L: "LSV booking model has no market-calibration front end: no Heston-backbone NLS calibration and no mixing-weight (eta) tuning to touch/DNT quotes"; reconciled against the live graph). What EXISTS today: `LsvModel::calibrate(inputs, var, iv, spot_grid, cfg)` is a pure, side-effect-free constructor that builds an LsvModel by delegating to particle::calibrate_leverage(iv, &var, spot_grid, spot, t, cfg) — a PARTICLE leverage-function calibration to an ImpliedVolSurface — and stores {inputs, var, leverage}, no I/O/writes/allocation-in-loop in the constructor itself. The OPEN gap the finding names is NOT closed here: this calibrates the local-vol LEVERAGE to a given IV surface; it does NOT do a Heston-backbone nonlinear-least-squares calibration of the variance params, and the mixing weight (eta) is taken from VarianceParams rather than tuned to touch/DNT market quotes. SELF-INVALIDATING: when a Heston-NLS + mixing-eta market-calibration front end lands, this method's signature/body changes (it would take market touch/DNT quotes and tune var/eta), flipping or unresolving this claim — the signal that the deliverable's remaining half closed.

### 63. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.market_hedge_overlay.hedge_smile_cost`

DELIVERABLE exotics/vanna-volga-overlay-magnitude-unvalidated = LANDED (backlog tracker still lists it OPEN as a Round-2 P2/M finding; reconciled against the live graph). `hedge_smile_cost` is the pure (side-effect-free) vanna-volga market-hedge smile-overlay cost: it reads ExoticSensitivities + the broker RR/BF marks and returns the overlay cost with no external writes. The Round-2 gap (only flat-smile/sign/scaling tests; the spec-mandated VV-vs-replication magnitude cross-validation unimplemented) is CLOSED: celnet-parity::vv_magnitude::engine_overlay_matches_replicating_portfolio_oracle_in_magnitude now pins the engine overlay against a CODE-DISJOINT replicating-portfolio oracle (oracle_cost) within a derived 20% magnitude band, with <=1% relative agreement on the cross-Greeks (vanna/volga) and a materiality floor + sign-agreement guard across the product set, backed by the golden oracle hedge_smile_overlay_cost. SELF-INVALIDATING: any edit to the overlay arithmetic shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.

### 64. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual_price`

DELIVERABLE proto/new-payoff-shapes (PerpetualOption arm) = DONE (RC cut a0817d6; arm 30). `perpetual_price(opt, i)` is a pure, total closed-form valuation of an American perpetual option: it dispatches on the `valuation(opt, i)` result and returns spot for a never-exercised call, strike for a never-exercised put, the intrinsic `opt.sign()*(spot-strike)` when immediately exercised, and the continuation `value` otherwise — every `Valuation` arm enumerated, no wildcard, so the match is exhaustive and a new regime is a compile error rather than a silent fall-through. Pure: it borrows `&PerpetualInputs`, performs no I/O/allocation/mutation, and returns `Result<f64, PerpetualError>`. Verified non-circularly against an independent root-bracketing re-derivation (closed_form_matches_independent_bisection_rederivation) in the same module. This is one of the two genuinely-new payoff shapes the backlog tracked as OPEN; it is now built + golden/parity-gated + surfaced across all 5 clients.

### 65. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.tarf_price`

DELIVERABLE exotics/qmc-pathwise-wiring = OPEN (Round-2 P2/M finding; reconciled against the live graph — still genuinely open at this round). `tarf_price` is a pure (side-effect-free) Monte-Carlo TARF valuation: it reads ExoticInputs/Tarf/TarfMcConfig, builds local per-fixing buffers, and returns a TarfResult with no external writes. The OPEN gap: the path generator is STILL the plain `CounterRng` antithetic Philox stream (`CounterRng::new(cfg.seed, 0, pair, 0)` + inverse_cdf), NOT the scrambled-Sobol / Brownian-bridge QMC stack in celnet-qmc that already feeds american.rs/multiasset.rs. The path-dependent pricers (tarf/accumulator/lookback/quanto/pivot) therefore forgo the low-discrepancy variance reduction the QMC crate provides. SELF-INVALIDATING: when this pricer is rewired onto celnet-qmc (Sobol/bridge) the function body changes and this claim flips stale, signalling the deliverable has closed; a write-introducing regression also flips it.

### 66. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one_touch_price`

DELIVERABLE exotics/one-touch-at-hit-pairing-flip = LANDED (backlog tracker still lists it OPEN as the Round-2 P0; reconciled against the live graph). `one_touch_price` (-> one_touch_with_side) is a pure (side-effect-free) closed-form one-touch valuation: it reads ExoticInputs/barrier/rebate/timing and returns a price with no external writes. The Round-2 P0 (~28% high / 2x-on-far-barriers at-hit pairing flip + circular golden oracle) is FIXED-AT-ROOT: the at-hit branch is now pinned to an INDEPENDENT first-passage quadrature reference (at_hit_matches_independent_first_passage_quadrature, 1e-12) and guarded by a non-circular family — discounted-hit-probability sandwich (at_hit_sandwiched_by_discounted_hit_probability), t->inf perpetual-discounted-hit limit (at_hit_t_infinity_is_perpetual_discounted_hit_factor), barrier continuity (at_hit_continuous_at_the_barrier), zero-rate collapse to deferred (zero_discount_rate_collapses_at_hit_to_deferred), and barrier monotonicity. SELF-INVALIDATING: a regression that re-introduces a WRITES side effect, or any re-pairing edit that shifts these anchors, flips this claim stale, re-opening the reconciliation.

### 67. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.framing.checksum`

SAFETY/WIRE — FIX session-layer frame integrity is the standard FIX BodyLength/CheckSum (tag 10) modulo-256 sum: checksum folds every byte of the message-up-to-and-including the SOH before tag 10 with wrapping u32 addition and returns (acc & 0xFF) as u8 — the canonical FIX checksum that is always rendered as a 3-digit field and validated on inbound frames (rejects_corrupted_checksum, checksum_is_mod_256). A counterparty frame whose recomputed mod-256 checksum does not match the transmitted tag-10 value is rejected at framing, so a corrupted/truncated FIX message never reaches order/quote handling. Pure: it reads only the input byte slice and returns the u8 checksum, mutating nothing.

### 68. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76_price`

SAFETY/ORACLE — black76_price is the independent golden reference for futures-style (forward-measure) options (the Black-76 closed form), NOT the production engine's own pricer: for t<=0 it returns the discounted intrinsic exp(-r·t)·max(sign·(F-K),0); otherwise the standard Black-76 with vsqt=vol·√t, d1=(ln(F/K)+½σ²t)/vsqt, d2=d1-vsqt, discount df=exp(-r·t), Call=df·(F·N(d1)-K·N(d2)) and the Put put-call complement. It is pure and deterministic over its 6 scalar inputs (libm transcendentals only, no I/O/mutation/allocation), so it is a trustworthy can-disagree oracle gating commodity / listed-future-option parity against the engine. Self-invalidates if the closed form drifts.

### 69. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76_undiscounted_price`

Golden-oracle gate (Black-76 commodity/forward reference): black76_undiscounted_price is a pure closed-form function of (cp, forward, strike, vol, t) using only libm math and the pure xerf_norm_cdf, with the t<=0 intrinsic-payoff branch. No writes, no I/O, deterministic — the QuantLib-pinned reference price the parity suite gates production engines against must be a pure function of its inputs.

### 70. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cholesky`

SAFETY/ORACLE — cholesky is the correlation-decomposition primitive underpinning the independent basket / multi-asset Monte-Carlo golden references: it computes the lower-triangular factor L of a symmetric positive-definite correlation matrix via the classic doubly-indexed recurrence (sum a[i][j] - Σ_k l[i][k]·l[j][k], diagonal = √sum, off-diagonal = sum/l[j][j]). It is pure and deterministic over its input matrix (no I/O, no mutation of the argument, no RNG), and it is fail-closed: a non-positive-definite matrix trips assert!(sum > 0.0) and panics rather than silently emitting a NaN/garbage factor that would corrupt every correlated-path draw. This makes the oracle's correlated scenarios reproducible and trustworthy as a can-disagree reference. Self-invalidates if the PD guard is removed.

### 71. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto_inverse_price`

Golden-oracle gate (inverse/coin-margined crypto reference): crypto_inverse_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — carry b=r-funding, forward, the (K/F)e^{sigma^2 t} amplitude correction for the 1/S_T inverse payoff, libm math and pure xerf_norm_cdf only. No writes, no I/O, deterministic; the reference price gating the inverse crypto engine must depend solely on its inputs.

### 72. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto_linear_price`

Golden-oracle gate (linear/GK-funding crypto reference): crypto_linear_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — applies carry b=r-funding to the forward then delegates to the pure black76_price. No writes, no I/O, deterministic; the linear crypto reference price the parity suite uses is a pure function of its inputs.

### 73. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.det3`

Golden-oracle numerical kernel: det3 computes the 3x3 determinant by cofactor expansion over a borrowed [[f64;3];3] with no writes, no I/O — a pure function of the matrix. Determinism of this kernel underpins the correctness of the oracle routines (e.g. correlation/quanto projections) that the parity gate trusts as ground truth.

### 74. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.equity_bsm_price`

SAFETY/ORACLE — equity_bsm_price is the independent golden reference for generalized Black-Scholes-Merton equity vanillas (carry b = r - q - repo), NOT the engine's own pricer: for t<=0 it returns intrinsic max(sign·(S-K),0); otherwise d1=(ln(S/K)+(b+½σ²)t)/(σ√t), d2=d1-σ√t, with cost-of-carry-discounted spot s_disc=S·exp((b-r)t) and rate-discounted strike k_disc=K·exp(-r·t), Call=s_disc·N(d1)-k_disc·N(d2) and the Put complement. It is pure and deterministic over its 8 scalar inputs (libm only, no I/O/mutation/allocation), serving as a can-disagree oracle gating equity-vanilla parity (dividend yield + repo carry) against the engine. Self-invalidates if the carry decomposition drifts.

### 75. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx_forward_pv`

Golden-oracle gate (FX forward PV reference): fx_forward_pv is a pure closed-form function of (side, spot, strike, notional, t, r_dom, r_for) — side*notional*(spot*e^{-r_for t} - strike*e^{-r_dom t}), dual-discounted, no writes, no I/O, deterministic. The FX-forward reference PV the parity suite pins must depend solely on its inputs.

### 76. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx_swap_points`

Golden-oracle gate (FX swap points reference): fx_swap_points is a pure closed-form function of (spot, near_t, far_t, r_dom, r_for) — spot*(e^{b*far_t}-e^{b*near_t}) with carry b=r_dom-r_for, no writes, no I/O, deterministic. The FX-swap points reference the parity suite pins must depend solely on its inputs.

### 77. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.gk_price`

SAFETY/ORACLE — gk_price is the independent golden reference (the Garman-Kohlhagen two-rate FX vanilla closed form) against which the production engine is validated, NOT the engine's own pricer: for t>0 it computes d1=(ln(S/K)+(r_dom-r_for+0.5*vol^2)*t)/(vol*sqrt(t)), d2=d1-vol*sqrt(t), df_dom=e^{-r_dom*t}, df_for=e^{-r_for*t}, and returns S*df_for*N(d1)-K*df_dom*N(d2) for a Call (put by symmetry); for t<=0 it returns the discounted intrinsic max(sign*(S-K),0). It deliberately re-derives the price from first principles with its own norm_cdf so a parity test (e.g. vanilla_price_and_greeks_match_quantlib, also pinned to published QuantLib numbers) can disagree with the engine — the anti-circular-oracle property: numerical correctness is checked against this reference, never merely asserted plausible. Pure: it reads only its scalar args and returns the f64 price, mutating nothing.

### 78. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.ndf_pv`

Golden-oracle gate (NDF PV reference): ndf_pv is a pure closed-form function of (side, spot, strike, notional, t, r_dom, r_for) — a non-deliverable forward prices as the deliverable forward, delegating to the pure fx_forward_pv. No writes, no I/O, deterministic; the NDF reference PV the parity suite pins is a pure function of its inputs.

### 79. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as_erf_price_bound`

PILLAR (CLAUDE.md guardrails 5 + 6 + 7 — numerical code is VALIDATED against a derived bound, never merely asserted plausible; the GPU scale path uses open methods and Metal lacks f64 so the f32 path is rigorously bounded). `as_erf_price_bound(b)` is the pure, closed-form per-instrument absolute-error envelope for the f32/`as`-erf batch GPU kernel: it returns `(s_disc + k_disc) * 0.5 * AS_ERF_MAX_ABS_ERR` with `AS_ERF_MAX_ABS_ERR = 1.5e-7`, where the discounted-spot and discounted-strike legs scale the worst-case erf approximation error into a price tolerance. The many-instrument GPU batch path (CLAUDE.md guardrail 6 — IB-sized portfolios / high-throughput scale-out) is reconciled three-way against the exact f64 oracle WITHIN this analytic bound, so the precision claim is proven rather than assumed. The function is side-effect-free: it reads only the borrowed BatchInstrument and computes a scalar via libm-backed exp, mutating nothing.

### 80. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.CpuBackend.label`

ADR (GPU abstraction = wgpu baseline + CPU-SIMD fallback, CUDA optional). `CpuBackend::label` returns the constant identity "cpu-f64": a pure accessor (reads nothing, mutates nothing) that names the f64 CPU oracle within the shared `PricingBackend` trait. That trait is the portability seam — `CpuBackend` and the wgpu `GpuBackend` both implement the same simulate_paths/reduce_payoff/price_vanilla contract, with Philox path index i fixed across backends so results reconcile (modulo the f32/f64 element type) — which is exactly what makes the GPU backend swappable behind one interface. DECISION/RATIONALE: the GPU strategy is wgpu (Metal/Vulkan/DX12) as the open, permissively-licensed baseline, with the CPU-SIMD path as the always-available f64 oracle and reconciliation reference, and an optional CUDA backend behind the same trait — chosen over a CubeCL/CUDA-first design because wgpu keeps the runtime dependency-set fully open-source and portable across the M4/Metal dev box and Linux/Vulkan CI. The label encodes the key portability caveat the design must respect: the CPU oracle is f64 while the wgpu/Metal path is f32 (Metal lacks f64), so cross-backend agreement is asserted to the f32 tolerance, never bit-identity. (Guardrail: no commercial products; open GPU stack with wgpu first-class.)

### 81. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.declared_is_premium_adjusted`

Vendor-feed normalization re-derives the premium-adjusted axis at the integration seam (docs/CONVENTIONS.md DeltaConvention→premium-style; docs/CELER-INTEGRATION.md vendor normalization). `declared_is_premium_adjusted(d)` is a pure const-fn total predicate over DeltaConvention — true exactly for SpotPremiumAdjusted and ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants — identical in meaning to ConventionRecord::is_delta_premium_adjusted but defined on the integration normalize path where an incoming vendor delta convention is checked against the resolved house record. The two definitions must agree variant-for-variant; drift here would mis-normalize a vendor feed's premium-adjusted flag. Pure: no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 82. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.build_table`

Journal CRC-32 table safety (deliverable: journal-durability). build_table is a const fn computing the reflected IEEE CRC-32 (polynomial 0xEDB88320) lookup table purely from compile-time constants — referentially transparent by construction, no I/O, no mutable global state escaping the function, identical on every build and every platform. This is the deterministic root the entire journal torn-tail / corruption-rejection guarantee rests on: a stable table means a stable CRC. Pure (no WRITES edges); self-invalidates if the table derivation changes.

### 83. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.crc32`

SAFETY — journal record integrity is a standard CRC-32 (IEEE 802.3 reflected polynomial): crc32 seeds 0xFFFF_FFFF, folds each byte through the 256-entry reflected lookup TABLE (crc = (crc >> 8) ^ TABLE[(crc ^ b) & 0xFF]), and finalizes with the XOR-out 0xFFFF_FFFF — the bit-exact reflected CRC-32 whose known-answer vectors (e.g. "123456789" => 0xCBF43926) are pinned by the crate's own vectors test. Every framed journal record carries this checksum over sync-word+header+payload (frame_record appends crc32(frame).to_le_bytes()), so any single-bit flip in a persisted record changes the CRC and the record is rejected on replay rather than silently mis-applied to recovered book/market state. Pure: it reads only the input byte slice and returns the u32 checksum, mutating nothing.

### 84. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame_record`

Journal CRC/torn-tail safety (deliverable: journal-durability). frame_record is a pure, deterministic framing function: given (sequence, payload_len, payload) it builds the sync-word + little-endian header + payload + trailing CRC-32 byte-for-byte with no I/O, no shared mutation, and no observable side effect. Determinism is the load-bearing invariant — the recovery reader recomputes the same CRC over the same framed prefix, so any torn or corrupted tail fails the checksum identically on every replay. Pure (no WRITES edges); self-invalidates if framing gains state.

### 85. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame_snapshot`

Journal snapshot framing safety (deliverable: journal-durability). frame_snapshot is a pure, deterministic framing function: given (watermark, snap_len, snapshot) it emits sync-word + snapshot-marker + little-endian watermark/len + snapshot bytes + trailing CRC-32 with no I/O and no side effect, sharing the identical crc32 trailer discipline as the record path so the two framings cannot drift. Determinism guarantees the recovery reader's recomputed CRC matches bit-for-bit, rejecting any torn snapshot tail on replay. Pure (no WRITES edges); self-invalidates on code change.

### 86. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.LimitCheck.is_hard_breach`

SAFETY — pre-trade hard-breach gating (deliverable: limits-breach-detection). LimitCheck::is_hard_breach is a pure, deterministic predicate: it returns true iff BOTH the limit's enforcement == Enforcement::Hard AND its RagStatus utilization status is_breach() — the conjunction that distinguishes a rejectable hard breach from a soft (advisory) warning. It reads only &self (the limit spec + the precomputed utilization), performs no I/O, no mutation, no allocation; identical inputs always yield the identical verdict. This is the exact gate pre_trade_check / post_trade_check funnel through to decide PreTradeDecision::Reject, so a hard limit is enforced (a soft one only warns). Self-invalidates if the enforcement/status conjunction is ever weakened.

### 87. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.exposure_of`

Pre-trade limit-breach exposure safety (deliverable: limits-breach-detection). exposure_of is a pure, deterministic, total function over LimitMetric: it reads the aggregated net Greeks, vega ladder, gross concentration, and non-additive VaR/ES/StopLoss out of borrowed &NodeAggregate / &NonAdditiveExposure and returns the scalar exposure with no I/O and no mutation of any input. Determinism is the load-bearing safety property — pre-trade and post-trade checks (its four callers) measure the same metric against the same limit cap identically, so a breach can never be hidden by a non-reproducible reading. Pure (no WRITES edges); self-invalidates on change.

### 88. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.gross_concentration`

Pre-trade/post-trade gross concentration exposure is a pure read-only reduction: gross_concentration folds the abs of each leaf greek (delta_base or vega per ConcentrationMetric) over node.leaves with no writes. The limit-breach concentration metric is therefore a deterministic function of the aggregate snapshot alone — the safety property a pre-trade limit check relies on.

### 89. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.classify`

SAFETY — pre-trade limit breach is hard-classified at the cap, not approximated: LimitSpec::classify maps a projected exposure to a RagStatus by exact threshold order — ratio > 1.0 (exposure strictly over cap) is RagStatus::Breach; ratio >= red is Red; ratio >= amber is Amber; else Green. Breach is the ONLY band above the cap, so a hard limit can never be silently under-classified as a mere Red warning. classify is the single pure breach-decision atom (it reads &self spec + the f64 exposure and returns a Utilization, mutating nothing); pre_trade_check builds on it and turns any is_hard_breach into PreTradeDecision::Reject. The strict-greater-than at the cap means an exposure exactly AT the cap (ratio == 1.0) is Red, not Breach — utilisation up to and including the cap is permitted, beyond it is rejected.

### 90. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.utilization`

SAFETY — limit utilization is a total, fail-closed ratio (deliverable: limits-breach-detection). LimitSpec::utilization(exposure) is a pure deterministic function: it returns |exposure|/cap when cap > 0, f64::INFINITY when cap == 0 (or negative) and |exposure| > 0, and 0.0 only when both are zero. The zero-cap → INFINITY branch is the fail-closed safety property: a zero cap is treated as an instant breach, never as an unbounded/divide-by-zero allowance, so a misconfigured zero limit can never silently admit risk. It reads only &self and the f64 argument; no I/O, mutation, or allocation. Self-invalidates if the zero-cap branch stops returning INFINITY.

### 91. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.ndf.Ndf.pv`

CAPABILITY (linear leaf — FX forward/swap/NDF): celnet-linear::ndf::Ndf::pv is the non-deliverable-forward present-value entry of the linear-products leaf — a pure, side-effect-free closed form (delegating to pv_at over LinearInputs at the near settle tenor) with no I/O, allocation-mutation, or logging. The NDF cash-settled PV equals the deliverable forward PV (proven by ndf_pv_equals_deliverable_forward_pv), and LinearInputs::forward/discount are byte-identical to the core CarryInputs (forward_and_df_match_core_carry_inputs_byte_for_byte), so the linear capability (FX forward/swap/NDF) sits on the same carry seam as the option leaves and reaches the one contract.

### 92. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.channel.telemetry_channel`

PILLAR (CLAUDE.md guardrail 11 — zero-cost observability, telemetry offloads over a BOUNDED queue so the pinned hot core stays alloc/lock/log-free). `telemetry_channel(capacity)` is the single constructor of the hot-path-to-drain seam: it builds an rtrb single-producer/single-consumer RingBuffer of FIXED `capacity.max(1)` (the bounded queue) and returns the (HotProbe, TelemetryDrain) pair sharing one Arc<Shared>. The hot side (HotProbe) only pushes HotSamples into the pre-sized ring and never blocks or allocates per sample; backpressure is absorbed by dropping/counting gaps, never by stalling the pricing core. The function itself is a pure constructor — its output depends only on `capacity`, it mutates no shared/global state and has no observable side effect beyond returning the owned channel ends.

### 93. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.p99_ns`

PILLAR (CLAUDE.md guardrail 11 — mission-critical ops instrumentation with HdrHistogram p50/p99/p99.9, without diminishing hot-path performance). `LatencyRecorder::p99_ns` is the canonical p99 tail-latency readout: a pure read-only accessor returning `self.percentile_ns(99.0)` over the recorded latency histogram in nanoseconds. It reads the histogram, mutates nothing, and has no side effects — the p99/tail telemetry is computed off the bounded drain, never on the zero-alloc pricing core. This is the latency-budget observability surface referenced by docs/ARCHITECTURE.md §1.2 / docs/SCALE-OUT.md.

### 94. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.percentile_ns`

PILLAR (CLAUDE.md guardrail 11 — instrument for mission-critical ops with HdrHistogram p50/p99/p99.9, without diminishing performance). `LatencyRecorder::percentile_ns(q)` is the single quantile-readout primitive that the named p50_ns/p99_ns/p999_ns accessors all delegate to: it returns `self.hist.value_at_quantile(q / 100.0)` — the HdrHistogram value at the q-th percentile, in nanoseconds (q is a percentage, converted to a [0,1] quantile). It is a pure read-only accessor: it reads the recorded latency histogram and mutates nothing, so quantile readout never touches the zero-alloc pricing hot path (recording is offloaded; this only reads the already-merged histogram). This is the tail-latency budget surface in docs/ARCHITECTURE.md §1.2.

### 95. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.butterfly_check`

BUTTERFLY no-arbitrage hard-reject on the plugin-API smile contract (ANALYTICS-SPEC §3.4 strike-convexity axis; the SmileModel::check_no_arbitrage default path). `smile::butterfly_check(model, forward, t, strikes)` enforces strike-convexity of undiscounted call prices: over every consecutive strike triple (kl,km,kr) on a strictly-increasing grid it forms the second difference `C(kl) - 2·C(km) + C(kr)` of `undiscounted_call` at each strike's model-implied vol, and returns Err(Unsupported("butterfly arbitrage")) when that second difference is negative beyond a rounding tolerance (`is_close(.,0,1e-9,1e-12)`) — a negative call convexity is a negative risk-neutral density = butterfly arbitrage. It also hard-rejects non-finite/non-positive forward or t, fewer than 3 strikes, and any non-strictly-increasing or NaN strike (NaN compares false to its neighbour, so it fails the monotone-grid guard). Pure validator: reads its args + the model, returns PluginResult<()>, mutating nothing.

### 96. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel<M>.price`

CAPABILITY (plugin Tier-0 native dispatch): celnet-plugin-host::native::NativeModel<M>::price is the Tier-0 native plugin pricing dispatch — a pure, side-effect-free delegation that forwards (OptionType, &CarryInputs) to the inner PricingModel and returns HostResult<f64> with no host-side I/O, allocation-mutation, or logging. It proves a user-supplied native model and the fuel-metered Tier-2 wasm-sandboxed model both register and dispatch through the ONE PricingModel::price(opt, CarryInputs) seam of the same ModelRegistry, so native and wasm twins agree bit-for-bit through one registry (native_and_wasm_twins_agree_through_one_registry); the Tier-2 sandbox path itself is fuel-metered and is therefore deliberately NOT claimed pure here.

### 97. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel<M>.price_and_greeks`

CAPABILITY (plugin Tier-0 native dispatch): NativeModel<M>.price_and_greeks is the Tier-0 native plugin-host dispatch entry behind the frozen HostModel trait — pure (&self, OptionType, &CarryInputs) -> HostResult<CarryGreeks>, forwarding to the in-process registered model with zero sandboxing overhead. Pairs with the Tier-2 wasmi fuel-metered sandbox (WasmModel) under one ModelRegistry; the native-and-wasm twins agree through that one registry. No WRITES on the dispatch path — the gate self-invalidates if the native forwarder gains a side effect.

### 98. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.sandbox_config`

ADR (plugin-host sandbox = wasmi, fuel-metered, narrowed feature set). `sandbox_config()` is the single source of the guest-execution sandbox policy and is a pure builder: it constructs a fresh wasmi `Config`, enables `consume_fuel(true)` (deterministic instruction metering, the basis of the per-call FuelBudget that bounds even a `(start)` function — see WasmModel::load), and explicitly disables the unused proposals (memory64, bulk-memory, reference-types, tail-call) to narrow the accepted module surface. It reads and writes no external state — its result depends only on the wasmi defaults — so the sandbox policy is reproducible call-to-call. DECISION/RATIONALE: the Tier-2 user-plugin host is built on wasmi (a pure-Rust, no-unsafe, no-JIT interpreter) rather than wasmtime: wasmi gives deterministic fuel metering and a small, auditable, JIT-free attack surface that suits a mission-critical pricing host where a plugin must be sandboxed and time-bounded, accepting interpreter throughput for that safety. Tier-0 native models run un-sandboxed for the hot path; untrusted user code is confined here. (Memory: plugin-host=wasmi; wasmtime rejected.)

### 99. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.AtmConvention.from`

The wire→types AtmConvention mapping is a total 1:1 lift of the two ATM conventions: `From<WireAtmConvention> for AtmConvention` matches AtmForward→AtmForward and DeltaNeutralStraddle→DeltaNeutralStraddle explicitly, no wildcard. DeltaNeutralStraddle is the dominant interbank ATM (its strike is the premium-adjusted-aware F·e^{±½σ²T}, docs/CONVENTIONS.md), distinct from AtmForward (K=F). This seam carries that selection from the wire onto `celnet_types::AtmConvention` with no possibility of silent enum drift. Pure: a match returning the mapped enum, mutating nothing.

### 100. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Cut.from`

The wire→types Cut mapping is a total 1:1 lift of the two expiry cuts: `From<WireCut> for Cut` matches NewYork1000→NewYork1000 and Tokyo1500→Tokyo1500 explicitly, no wildcard. NY 10:00 is the standard interbank OTC cut; Tokyo 15:00 is standard for JPY-region/Asian business (docs/CONVENTIONS.md). The cut is per-(pair,tenor) configuration, not a global default; this seam carries it from the wire onto `celnet_types::Cut` with no silent drift. Pure: a match returning the mapped enum, no mutation.

### 101. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DayCount.from`

The wire→types DayCount mapping is a total 1:1 lift of the two day-count bases: `From<WireDayCount> for DayCount` matches Act365Fixed→Act365Fixed and Act360→Act360 explicitly, no wildcard. ACT/365-fixed is kept distinct from ACT/360 because vol-time accrual (ACT/365) must not be conflated with money-market accrual basis (ACT/360) — a deliberate separation (docs/CONVENTIONS.md). This seam carries the basis from the wire onto `celnet_types::DayCount`. Pure: a match returning the mapped enum, no mutation.

### 102. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DeltaConvention.from`

The wire→types DeltaConvention mapping is a total, order-preserving 1:1 lift of the four FX delta conventions: `From<WireDeltaConvention> for DeltaConvention` matches every wire variant explicitly (SpotUnadjusted→SpotUnadjusted, ForwardUnadjusted→ForwardUnadjusted, SpotPremiumAdjusted→SpotPremiumAdjusted, ForwardPremiumAdjusted→ForwardPremiumAdjusted) with no wildcard arm, so the proto enum and the celnet-types enum can never silently drift — adding a delta convention on either side is a compile error here. This is the single conversion seam carrying the per-(pair,tenor) delta convention from the wire onto `celnet_types::DeltaConvention` (the convention encoded in the type, never a global default; docs/CONVENTIONS.md). Pure: a match over the input value returning the mapped enum, mutating nothing.

### 103. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.PremiumStyle.from`

The wire→types PremiumStyle mapping is a total, order-preserving 1:1 lift of the four premium quotation styles: `From<WirePremiumStyle> for PremiumStyle` matches every wire variant explicitly (DomesticPips→DomesticPips, PercentForeign→PercentForeign, PercentDomestic→PercentDomestic, ForeignPips→ForeignPips) with no wildcard, so the proto and celnet-types premium-style enums cannot drift. This carries the premium style — which decides whether the premium is paid in the FOR/base ccy and therefore carries FX risk (the `is_premium_adjusted` distinction, docs/CONVENTIONS.md) — from the wire onto `celnet_types::PremiumStyle`. Pure: a match returning the mapped enum, no mutation.

### 104. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Settlement.from`

The wire→types Settlement mapping is a total 1:1 lift of the two settlement styles: `From<WireSettlement> for Settlement` matches Deliverable→Deliverable and NonDeliverable→NonDeliverable explicitly, no wildcard. NonDeliverable encodes an NDO that cash-settles at a published fixing (EMTA/WMR), versus a physically Deliverable option (docs/CONVENTIONS.md). This seam carries the settlement style from the wire onto `celnet_types::Settlement` with no silent enum drift. Pure: a match returning the mapped enum, no mutation.

### 105. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.cut_round_trips`

`cut_round_trips` pins the wire↔types Cut conversion as a total round-trip: for every `celnet_types::Cut` variant (NewYork1000, Tokyo1500) `Cut::from(WireCut::from(c)) == c`. This guards that the two `From` directions stay mutually inverse, so the cut convention survives a wire encode/decode unchanged — the one-contract guarantee for the expiry-cut convention (no versioning, single current mapping). Pure test: constructs values and asserts equality, mutating no external state.

### 106. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.settlement_round_trips`

`settlement_round_trips` pins the wire↔types Settlement conversion as a total round-trip: for every `celnet_types::Settlement` variant (Deliverable, NonDeliverable) `Settlement::from(WireSettlement::from(s)) == s`. It guards that the two `From` directions remain mutually inverse so the deliverable/non-deliverable (NDO cash-settled) distinction survives a wire encode/decode unchanged — the single-contract guarantee for the settlement convention. Pure test: constructs values and asserts equality, no external mutation.

### 107. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_deliverable_underlying`

PILLAR (CLAUDE.md guardrail 9 — "No versioned APIs: exactly ONE clean, current contract"; guardrail 8 — purpose-named, vendor-neutral). `validate_deliverable_underlying(underlying)` is a pure, total projection of the single wire `Underlying` oneof onto the deliverable linear-forward book: it reads only the borrowed `WireUnderlying` and returns a `Result`, mutating nothing (no WRITES edges). On the one unversioned contract it accepts exactly the deliverable leg-pair arms — Fx and Metal (decoding to `Underlying::Fx`/`Underlying::Metal`) — and for every cross-asset arm (Equity, Commodity, DigitalAsset) returns a typed `WireError::WrongUnderlying { product_family: "deliverable forward", .. }` rather than silently coercing the asset-class identity under deliverable-FX arithmetic; an absent ref yields `WireError::MissingField`. It is the deliverable-book twin of the already-governed `validate_fx_underlying` (cl_1ff652eea3ffb73e) on the same single contract — there is no schema_version, no N/N-1 negotiation: a malformed or wrong-asset request is rejected, never version-coerced. Self-invalidating: if this acquires a side effect the purity gate flips it off.

### 108. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_fx_underlying`

ADR-0007 (one clean unversioned contract — proto side). `validate_fx_underlying` is a pure, total projection of the wire `Underlying` oneof onto the FX-option product: it reads only the borrowed `WireUnderlying` and returns a `Result`, mutating nothing. It accepts the Fx and Metal arms (decoding to `Underlying::Fx`/`Underlying::Metal`), and for every cross-asset arm (Equity, Commodity, DigitalAsset) it returns a typed `WireError::WrongUnderlying` rather than discarding the asset-class identity or silently coercing it under FX arithmetic; an absent ref yields `WireError::MissingField`. The match enumerates all oneof arms explicitly (no wildcard), so adding a new asset class to the wire is a compile error here, not a silent mis-route. DECISION/RATIONALE: there is exactly ONE current wire contract and no version negotiation — the rich cross-asset `underlying` oneof carries the single asset-class discriminator, and (where a legacy `pair` key still appears) precedence is fixed at underlying≻pair (see instrument_underlying_from_json), never an N/N-1 schema_version handshake. The contract evolves in place and deploys as one uniform version. (Guardrail: no versioned APIs; one clean current contract.)

### 109. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_listed_future_terms`

On the one wire contract, a listed-future option's terms are accepted only when both maturities are well-formed and consistently ordered: `validate_listed_future_terms` requires the option `expiry_years` to be finite and strictly positive, and the future's `future_expiry_years` to be finite and `>= expiry_years` (the underlying future must outlive the option), else it returns `WireError::InvalidTerms`; a missing `future_symbol` yields `WireError::MissingField` and an out-of-range margining tag yields `WireError::UnknownEnum`. Pure: it reads only the wire option and the expiry argument and returns a `Result`, mutating nothing.

### 110. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_perpetual_terms`

On the one wire contract, a perpetual-option instrument must carry `expiry_years == 0` (a perpetual has no expiry): `validate_perpetual_terms` rejects any non-zero value with `WireError::InvalidTerms`. Because the check is `expiry_years != 0.0` (which is also true for NaN), a NaN expiry is rejected, never silently waved through as "no expiry". Pure: it reads only its `f64` argument and returns a `Result`, mutating nothing.

### 111. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.RateSensitivities.fx_rhos`

The wire/proto rate-sensitivity contract preserves FX's TWO-rho structure end-to-end: `RateSensitivities::fx_rhos` returns `Some((rho_dom, rho_for))` only when the rate-sensitivities oneof is the `Fx` variant, surfacing BOTH the domestic-rate sensitivity ∂V/∂r_d and the foreign-rate sensitivity ∂V/∂r_f as a distinct pair (never a single collapsed equity-style rho), and `None` otherwise. This is the on-the-wire counterpart to the engine's `celnet-vanilla::greeks` two-rho output (Garman-Kohlhagen 1983; ANALYTICS-SPEC §2.1): the foreign rate enters as the continuous dividend yield on the foreign-currency leg, the two rhos carry opposite signs, and the typed oneof prevents any client from reading a one-rho FX sensitivity. Pure: it reads only `&self.sensitivities` and constructs an Option tuple, performing no allocation, I/O, or external mutation.

### 112. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as_digital_asset`

`Underlying::as_digital_asset` is the exhaustive, total projection of the underlying oneof onto its digital-asset arm: it returns `Some(&CryptoPair)` only for the `DigitalAsset` variant and `None` for every other asset class (Fx, Metal, Equity, Commodity) and for an absent ref. The match arms enumerate all variants explicitly (no wildcard), so adding a new asset class to the one contract is a compile error here rather than a silent mis-projection. Pure: it borrows `&self` and returns a borrow, mutating nothing.

### 113. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.candidate_log_ok`

SAFETY/CONSENSUS — Raft election restriction / log up-to-date check (deliverable: replog-quorum-durability). NodeCore::candidate_log_ok is a pure, deterministic read-only predicate implementing the §5.4.1 leader-completeness pre-condition: a vote is granted only if the candidate's log is at least as up-to-date as the voter's — first by last-term (a strictly higher candidate term wins), and on equal last-terms by length (cand_len >= my_len), with the EMPTY_LOG sentinel mapped to length 0 and u128 arithmetic preventing index overflow. It reads only &self.log (last_term/last_index) plus the two candidate scalars; no mutation, I/O, or allocation. This is the complementary half of leader_advance_commit: together they guarantee a committed entry is never lost to a stale-log leader. Self-invalidates if the (term-then-length) ordering is weakened.

### 114. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.leader_advance_commit`

SAFETY/CONSENSUS — Raft commit safety (the §5.4.2 leader-completeness rule): NodeCore::leader_advance_commit advances the commit watermark only for a log index n that satisfies BOTH conditions — (1) it is replicated on a quorum (holders = 1 self + peers whose match_index >= n, counted against majority = cluster_size/2 + 1), AND (2) the entry at n is from the leader's OWN current term (self.log.term_at(n) == Some(current_term)); entries from earlier terms are skipped and never directly committed. The guard `if self.role != Role::Leader { return }` makes commit-advancement a leader-only action. This is what stops a committed-then-uncommitted divergence under leader churn: a bare-majority replication of a stale-term entry does not commit. (This is a leader-state-advancing method, not a side-effect-free accessor — the invariant is the majority-AND-current-term gating it enforces, verified against leader_advance_commit_requires_a_current_term_majority.)

### 115. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.persist.decode`

Raft persistent-state corruption safety (deliverable: replog-quorum-durability). persist::decode is a pure, deterministic deserializer: given a byte slice it validates exact RECORD_LEN, recomputes crc32 over the body and rejects any mismatch by returning None, then reconstructs (current_term, voted_for, commit_index) with no I/O and no mutation of any shared state. Purity + CRC validation is the safety invariant — a torn or bit-flipped persisted vote/term/commit record can never be silently accepted, so quorum-commit durability and single-vote-per-term election safety survive a crash mid-fsync. Pure (no WRITES edges); self-invalidates on change.

### 116. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.persist.encode`

SAFETY — Raft persistent-state encode is the CRC-protected inverse of decode (deliverable: replog-quorum-durability). persist::encode(&PersistentState) is a pure, deterministic serializer producing a fixed RECORD_LEN frame: it lays out current_term, voted_for (value + present-flag), commit_index (value + present-flag) in little-endian with explicit 2-byte CRC alignment padding (debug_assert on BODY_LEN), then appends crc32(body). Because the byte layout and CRC are a deterministic function of the state alone — no I/O, no mutation, no clock/RNG — encode is the exact round-trip partner of the already-claimed persist::decode: a flipped byte recomputes a different CRC and decode rejects it, so torn or corrupt term/vote/commit records read as absent rather than as forged consensus state. Self-invalidates if the layout or CRC coverage changes.

### 117. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.taylor_pnl`

DELIVERABLE risk/pnl-attribution = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/L finding "P&L attribution (Greeks-based P&L explain) exists nowhere in the platform"; reconciled against the live graph — it now EXISTS as celnet-risk-cube). `PositionSensitivity::taylor_pnl(scenario)` is a pure, side-effect-free Greeks-based P&L-explain primitive: it returns the second-order Taylor P&L of a position under a Scenario as delta_spot·dS + ½·gamma·dS² + vega·dvol + ½·volga·dvol² + vanna·dS·dvol + discount_rho·discount_abs + carry_rho·carry_abs (dS = spot·spot_rel), reading only its own sensitivity fields and the scenario — no writes, no allocation, no I/O. This is exactly the cross-Greek P&L explain the competitive positioning claims; it is invoked by the risk-cube non-additive roll-up. SELF-INVALIDATING: any change to taylor_pnl's body that introduced a write/allocation/I/O side effect (e.g. a stateful attribution accumulator) would flip the WRITES gate and stale this claim, and if the Greeks-based explain were ever removed/relocated the anchor would unresolve.

### 118. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_facts`

PILLAR (CLAUDE.md guardrail 6 + 11 — "scale to investment-banking-sized portfolios"; horizontal scale-out / many-instrument batch). `partition_facts(facts, replicas)` is the pure entry that spreads a risk-fact set across the live replica fleet: it is a thin, side-effect-free delegation to `partition_facts_with(.., partition_key_of)` (reads only its two borrowed slices/sets, mutates nothing, no I/O, no WRITES edges). Each fact is assigned to the HRW `natural_owner` of its partition key, producing a `FleetReducer` whose logical shards form a DISJOINT cover of the input (every fact lands in exactly one shard — verified by `partition_is_disjoint_cover`) with a deterministic reduction order (shards sorted by ascending replica id), so a fleet of N nodes reduces a firm-sized portfolio to the SAME aggregate as a single node bit-for-bit (`single_shard_fan_out_is_bit_identical`). This is the data-parallel scale-out seam that makes IB-sized cube/risk reduction horizontally partitionable without changing the answer. Self-invalidating: any side effect introduced into the partitioning entry flips the purity gate.

### 119. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.carry`

`PositionRisk::carry` is a `const fn` total constructor for a cross-asset carry-priced position: it moves the underlying, option type, base notional and `CarryInputs` into the struct and fixes `quoted_delta` and `premium_style` to `None`. Being `const fn` it is side-effect-free by construction (no allocation, I/O, or mutation of external state) — a pure normalization entry point into the risk-cube leaf model.

### 120. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize_with`

CAPABILITY (carry-seam deliverable, cross-asset Greeks projection): canonicalize_with is the single pure seam that projects any asset class's leaf Greek strip into the carry-neutral CanonicalLeaf — one virtual price_greeks call through the CarryPricer trait (asset class matched inside the leaf adapter, not in this hot path, per ADR-0008), the FX-only adjoint engine taken only for an FX/metal underlying and otherwise the analytic strip, canonical spot-unadjusted/premium-excluded delta re-derived through the named convention for FX, and every Greek scaled by notional. Pure: builds CanonicalLeaf from (pricer, engine, pos) refs with no WRITES; self-invalidates on any side-effecting change to the projection.

### 121. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.hash.rendezvous_weight`

PILLAR (CLAUDE.md guardrail 6 + 11 — "Design for horizontal scale-out from day one"; "scale-out aware"). `rendezvous_weight(replica_seed, key_digest)` is the pure deterministic core of the platform's horizontal scale-out: a `const fn` computing the Highest-Random-Weight (HRW / rendezvous-hashing) score for one (replica, partition-key) pair as `mix64(fold64(replica_seed, key_digest))`. It reads only its two u64 inputs and returns a u64 — no allocation, no I/O, no mutation, no WRITES edges — so it is referentially transparent and the per-replica scores are reproducible on every node. This is the primitive that makes book/risk sharding deterministic and minimal-disruption under membership change: `PartitionMap::natural_owner` takes the argmax of this weight over the live replica set to assign each partition key its stable owner, so adding/removing a replica re-homes only the keys whose argmax moved (the HRW property), never a global reshuffle. Self-invalidating: if the entanglement/mixing changes (anything beyond a pure two-u64 fold) the WRITES gate flips this claim off.

### 122. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap<'a>.natural_owner`

PILLAR (CLAUDE.md guardrail 6 + 11 — horizontal scale-out / shard ownership). `PartitionMap::natural_owner(key)` is the pure, side-effect-free realization of HRW shard assignment: for a partition key it digests the key, scores every live replica with `rendezvous_weight(replica.seed, digest)`, and returns the argmax `ReplicaId` — ties broken deterministically by the smaller replica id (`.then_with(|| a.0.0.cmp(&b.0.0))`) so the owner is a total, deterministic function of (key, membership). It reads only `&self` (the replica set) and the key, allocates nothing in the hot path and has no WRITES edges. This is the routing decision that lets the platform fan a book/risk workload across N nodes with a stable, minimal-disruption owner per key (the day-one horizontal scale-out requirement): every node computes the same owner independently, no central coordinator, and a membership change re-homes only the keys whose argmax moved. Self-invalidating: any side effect or non-deterministic tie-break introduced here flips the WRITES/purity gate.

### 123. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode_settlement_style`

Wire→domain SettlementStyle decode (docs/CONVENTIONS.md determinism + docs/INTERFACES.md enum contract): decode_settlement_style is a pure total function of the i32 wire tag — it round-trips through celnet_proto::SettlementStyle::try_from then the domain SettlementStyle, lowering Linear→CryptoSettlementStyle::Linear and InverseCoin→CryptoSettlementStyle::InverseCoin; an out-of-range tag is a typed PriceError::UnknownEnum, never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the enum mapping or error path changes (WRITES gate).

### 124. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.is_cross_asset`

CAPABILITY (cross-asset carry-seam reach): is_cross_asset is the pure routing predicate that decides — by Underlying variant alone (Equity | Commodity | DigitalAsset) — which instruments leave the FX vanilla path for the cross-asset leaf engines (equity generalized-BSM, commodity Black-76, crypto inverse/linear) on the shared carry seam. FX/metal stays on the vanilla path (metal lease rate modelled as the FX foreign rate), byte-identically. Side-effect-free classifier: reads only the &Underlying, returns bool, no WRITES — the gate self-invalidates if the variant set or the routing shifts.

### 125. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price_listed_future_option`

DELIVERABLE proto/new-payoff-shapes (ListedFutureOption arm) = DONE (RC cut a0817d6; arm 31). `price_listed_future_option(o, market, expiry)` is the side-effect-free server lowering of a wire listed-future option onto the Black-76 commodity-on-future leaf: it first runs the canonical term validator (`validate_listed_future_terms` — present `future_symbol`, known margining tag, `future_expiry_years >= expiry_years > 0`), rejects a non-positive/non-finite strike as `PriceError::Domain` and an out-of-range margining tag as `PriceError::UnknownEnum` (never clamped/defaulted), maps the margining enum to `CommodityMargining`, builds `CommodityInputs::on_future(spot, strike, vol, expiry, r_dom)`, and returns the leaf Greeks via `greeks_with_margining`. Pure: it reads only `(o, market, expiry)` and returns `Result<Priced, PriceError>`, mutating no external state. The booked future_symbol is contract identity, not a pricing input. This is the second of the two genuinely-new payoff shapes the backlog tracked as OPEN; now built + golden/parity-gated across all 5 clients, with futures-style honest-zero discount-rho asserted.

### 126. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.product_name`

CAPABILITY (carry-seam deliverable, one-contract product-family reach): product_name is the pure total over the full Product enumeration — it names all 24 product arms reachable through the one unversioned contract (vanilla, strategy, single/double barrier, digital, touch, variance_swap, volatility_swap, asian_option, forward_start, cliquet, quanto, tarf, pivot, accumulator, lookback, window_barrier, american, basket, fx_forward, fx_swap, ndf, perpetual_option, listed_future_option) with no fallback arm. Pure: maps &Product to a &'static str with no WRITES; the exhaustive match self-invalidates the moment a product arm is added or removed.

### 127. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.pin.resolve_pinned_vol`

Surface-version pinning REJECTS an unknown version rather than silently pricing off the live surface. `resolve_pinned_vol` short-circuits to the live market (no echo) when no surface_version is pinned; otherwise it requires an FX `underlying` (else invalid_argument) and looks the pin up via book.pinned_vol. Three outcomes are total: Ok(Some(vol)) ⇒ price against the marked vol and echo the version; Ok(None) ⇒ the version EXISTS but did not mark THIS pair, so keep the live vol yet still echo the (valid) version; Err(PinError::UnknownVersion) ⇒ the version was never marked, returned as Status::failed_precondition — the pin is refused, never honoured against an arbitrary surface. This makes a pinned RFQ/RFS deterministic: it reprices against the exact marked model or fails closed. Pure: reads book/version/instrument/market and returns Result<PinnedVol, Status>, constructing new PinnedVol/MarketContext values (with_vol) and mutating no external state.

### 128. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.instrument_underlying_from_json`

One clean unversioned contract with a single underlying discriminator: when decoding an instrument, `instrument_underlying_from_json` treats the rich cross-asset `underlying` oneof as authoritative whenever it is present and non-null (it carries the asset-class discriminator), and consults the legacy FX `pair` key only when no `underlying` is present — projecting it to `Underlying::fx`. There is exactly one precedence order (underlying ≻ pair), not a versioned negotiation. Pure: it reads the JSON map and returns a `Result`, mutating no state.

### 129. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.leg_from_json`

DELIVERABLE proto/strategy-per-leg-expiry = OPEN (Round-2 P3/M finding; reconciled against the live graph — still genuinely open at this round). `leg_from_json` is the pure (side-effect-free) server decoder of one strategy/structure leg off the JSON wire: it reads only the borrowed serde Value and constructs a `Leg { option_type, strike, side, ratio }`, mutating no shared state. It is the current-state witness that the wire `Leg` carries NO per-leg expiry/tenor field — every leg shares the enclosing Instrument expiry — so single-expiry-only multi-leg structures (a 1M-vs-3M calendar/diagonal spread cannot be booked as one net-premium ticket) remain unexpressible on the one unversioned contract. SELF-INVALIDATES: when a per-leg `tenor`/`expiry` field is added to the wire Leg and decoded here, this decoder's content hash changes and the claim flips stale, signalling the gap closed.

### 130. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.mark_surface_request_from_json`

DELIVERABLE surface/crypto-strike-axis-surfacing = OPEN (first post-RC fast-follow; RC anchor a0817d6). `mark_surface_request_from_json(o)` is the pure JSON→MarkSurfaceRequest decoder and it documents the CURRENT wire shape of the mark-surface contract: it accepts ONLY `pair` (optional), a required `broker_quotes` array of delta-space (RR/BF) broker quote sets, `conventions`, and an optional `smile_model`. There is NO `quote_basis`/`StrikeQuoteSet` strike-axis quote field — so although the strike-axis surface LEAF (fit_strike_slice/strike_surface) is built and parity-gated, a crypto surface STILL cannot be marked by strike from any client. Closing the fast-follow means adding a strike-axis quote oneof here (and the SurfaceEdge strike-slice ingestion → strike_surface), which will change this function's node content and STALE this claim — the staleness firing IS the done-signal for the surfacing deliverable. Pure: it reads only the JSON map and returns `Result<MarkSurfaceRequest>`, mutating nothing.

### 131. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.pivot_from_json`

DELIVERABLE pivot-wire-surfacing = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P1/M finding "Pivot TRA is engine-only — no proto arm, no golden vector, no parity row, unreachable from any client"; reconciled against the live graph). `pivot_from_json` is the pure (side-effect-free) server decoder that lowers a wire pivot instrument off the JSON edge into the celnet-exotics `Pivot { option_type, strike, pivot, target, leverage, redemption, schedule, mc_pairs, mc_seed }` MC spec — proving the pivot TARGET-redemption-accumulator product is now reachable on the one unversioned contract (engine celnet-exotics::pivot::pivot_tra_price, GUI pricePivot, golden/parity celnet-parity::pivot_wire with the pivot==strike→TARF degenerate collapse and code-disjoint MC oracle). SELF-INVALIDATES on any change to this decoder.

### 132. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.limits.transport_config`

DELIVERABLE ws-edge-resource-caps = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2/3 P3/S finding "WS edge accepts connections with default tungstenite limits (64 MiB messages) — no explicit frame/message caps"; reconciled against the live graph — the streaming edge now installs explicit caps via celnet-server::ws::limits). `transport_config()` is a pure, side-effect-free builder that returns the tungstenite WebSocketConfig the WS accept path uses: it sets max_message_size = Some(TRANSPORT_MESSAGE_CAP_BYTES), max_frame_size = Some(TRANSPORT_MESSAGE_CAP_BYTES) and max_write_buffer_size = MAX_WRITE_BUFFER_BYTES (overriding the 64 MiB tungstenite default), with no writes/allocation/I/O. Its sibling oversize_close / oversize_reject_text emit the typed 1009 (CloseCode::Size) reject, and cap_ordering_holds pins the contract message fits under the cap — closing the bounded-resource-discipline deviation on the streaming edge. SELF-INVALIDATING: if the WS edge stopped supplying an explicit bounded config (anchor removed/renamed) or the builder grew a side effect, this claim would unresolve or flip the WRITES gate.

### 133. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.ArbitrageReport.is_arbitrage_free`

Analytics-correctness (arbitrage-gate deliverable): celnet-surface::arbitrage::ArbitrageReport::is_arbitrage_free is the pure hard-reject predicate combining all three no-arbitrage axes within tolerance tol — butterfly/density (min_butterfly ≥ −tol), vertical/call-spread monotonicity (max_vertical_increase ≤ tol), and risk-neutral density positivity (min_density ≥ −tol). A slice/surface is accepted only when every axis is within tol; any single breach makes the report not arbitrage-free. Pure: deterministic in (&self, tol), reads only the report's reduced extrema, no WRITES edges.

### 134. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.check_slice`

`check_slice` is the per-slice butterfly + vertical no-arbitrage primitive (ANALYTICS-SPEC §3.4) that VolSurface::arbitrage_report calls at each sampled maturity. Over a strictly-ascending strike grid it returns an ArbitrageReport with `min_density` (the minimum second-difference risk-neutral density (down−2·mid+up)/h² via implied_density; a negative density is a butterfly violation), `min_butterfly` = the h²-scaled butterfly spread, and `max_vertical_increase` = the largest call-price rise from a lower strike to the next (a positive increase is a vertical-spread violation, since calls must be monotone non-increasing in K). It asserts grid.len()≥3, h>0, strictly-ascending strikes, and grid[0]>h as preconditions. This is the slice-local half of the surface arbitrage gate; the calendar (cross-tenor) dimension is checked separately. Pure: it reads the smile + grid + scalars and returns the report value, mutating nothing.

### 135. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward_call`

Analytics-correctness (arbitrage-gate deliverable, vertical/calendar oracle): celnet-surface::arbitrage::forward_call is the pure undiscounted Black forward-call value F·N(d1) − K·N(d2) evaluated at the smile's own implied vol σ(K,F,t). It is the closed-form oracle the arbitrage gates are checked against: its K-derivative is −N(d2) ∈ [−1,0] (the vertical/call-spread bound, forward_call_strike_slope), and its second K-difference is the butterfly/density check; calendar-monotonicity is verified by comparing this value across maturities. Pure: deterministic in (&Smile, strike, forward, t), no WRITES edges.

### 136. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward_call_strike_slope`

`forward_call_strike_slope` returns the sticky (∂σ/∂K-ignoring) strike-derivative of the forward call price as `-Φ(d2)`, where `d2 = d1 - σ√t` from the smile's implied vol at the strike. Since `Φ ∈ [0,1]`, the returned slope lies in `[-1, 0]` — the no-arbitrage bound on a call's monotone-decreasing strike profile. Pure: it reads the smile and scalar inputs and returns an `f64`, mutating nothing.

### 137. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.build_model_smile`

CAPABILITY (carry-seam deliverable, smile-family reach): build_model_smile is the single pure selector that reaches all FIVE smile families through one contract — MarketHedge (vanna-volga market-hedge), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each routed to its calibrator (build_smile/fit_sabr/fit_svi/fit_ssvi/fit_essvi) and wrapped in the typed CalibratedSmile, with no calibration leaking outside the match. Pure: derives the CalibratedSmile from (model, ctx, quotes) refs with no WRITES; self-invalidates if a family arm acquires a side effect.

### 138. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_sabr`

CAPABILITY (smile family 1/5 — SABR): fit_sabr is the SABR smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<StochasticVolSmile>. Seeds (alpha, rho, nu) from the ATM level, 25-delta risk-reversal skew, and butterfly convexity, then a Gauss-Newton fit reproduces the total-variance anchors; beta is pinned (SABR_BETA). No I/O, no input mutation, no WRITES — the gate self-invalidates if the calibration grows a side effect. One of the five selectable smile families (SABR, raw-SVI, SSVI, Vanna-Volga, parametric).

### 139. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_ssvi`

CAPABILITY (smile family 3/5 — SSVI): fit_ssvi is the SSVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<ParametricSlice>. Pins theta to the ATM total variance and fits (rho, phi) to the wings by Gauss-Newton, projecting phi every iteration to satisfy the Gatheral-Jacquier Thm 4.2 butterfly sufficient conditions (theta*phi*(1+|rho|) < 4 and theta*phi^2*(1+|rho|) <= 4); eta = phi*theta^gamma reconstructs the surface phi. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.

### 140. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_svi`

CAPABILITY (smile family 2/5 — raw SVI): fit_svi is the raw-SVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<ParametricSlice>. Fits (b, rho, m, sigma) by Gauss-Newton with the level a pinned so the ATM anchor (k=0) reproduces w_atm exactly; the no-arbitrage projection runs each iteration, and a degenerate (negative minimum total variance) fit is rejected via the validating constructor rather than panicking. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.

### 141. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSlice.is_calendar_free_with`

CALENDAR no-arbitrage gate in the SVI/extended parameterization (ANALYTICS-SPEC §3.4 calendar axis). `ExtendedSlice::is_calendar_free_with(next)` decides whether an adjacent later-maturity SVI slice is calendar-arbitrage-free relative to self via the standard Gatheral–Jacquier no-crossing conditions on the raw-SVI params: (1) ATM total variance non-decreasing — `next.theta >= self.theta - SLACK`; (2) wing curvature non-decreasing — `next.psi >= self.psi - SLACK`; (3) the skew-change bound — `|next.rho·next.psi - self.rho·self.psi| <= (next.psi - self.psi) + SLACK`, which bounds how fast the skew may rotate so total-variance smiles do not cross at any moneyness. All three must hold (`SLACK = 1e-12`) or the pair is flagged. Pure: reads &self and &next, returns bool, mutating nothing.

### 142. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.lib.build_smile`

CAPABILITY (5 smile families): celnet-surface::build_smile is the production smile-construction entry — a pure, side-effect-free calibration that returns Result<MarketHedgeSmile, CalibrationError> from a MarketContext + MarketQuotes with no I/O, allocation-driven mutation, or logging. It is the MarketHedge (vanna-volga) baseline of the five selectable SmileModel families exposed through the one contract — MarketHedge (vanna-volga), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each constructible into a VolSurface that reports its own model() and prices a finite positive implied vol across the strike grid (proven by each_smile_family_is_selectable). The carry forward feeding every family is the carry-seam bits (context_forward_is_carry_seam_bits).

### 143. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.market_hedge.MarketHedgeSmile.corrections`

CAPABILITY (smile family 4/5 — Vanna-Volga): MarketHedgeSmile.corrections is the Vanna-Volga smile-family analytic entry — pure (&self, strike) -> (D1, D2). Returns the first-order (D1 = p*(sigma1-sigma0) + q*(sigma3-sigma0)) and second-order (D2, the d1*d2-weighted squared vol gaps at the 25-delta wings) market-hedge corrections from the three benchmark vols, the construction underlying the broker-quoted three-point smile. Reads only &self, returns a tuple, no WRITES — the gate self-invalidates if the correction formula grows a side effect. One of the five selectable smile families.

### 144. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.atm_convention`

Market-context atm-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::atm_convention is a pure projection returning self.conventions.atm unchanged — the AtmConvention threaded into ATM-strike resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).

### 145. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.delta_convention`

Market-context delta-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::delta_convention is a pure projection returning self.conventions.delta unchanged — the DeltaConvention threaded into strike↔delta quote resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).

### 146. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.strike_at_delta`

Analytics-correctness (guarded-root-find deliverable): celnet-surface::quotes::MarketContext::strike_at_delta resolves a delta pillar to a strike through the guarded inversion strike_from_delta under the context's own delta_convention, signing the target delta with pillar.signed(opt) (call > 0, put < 0). It returns Result<f64, DeltaSolveError> — the guarded root-find surfaces its failure mode (e.g. an unreachable premium-adjusted target beyond the delta cap) as a typed error rather than a silent/wrong root. Pure: deterministic in (&self, opt, pillar, vol) given the context, no WRITES edges.

### 147. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.build_trial_smile`

STRANGLE-CALIBRATION guarded trial-smile constructor (ANALYTICS-SPEC §1.4 — the mandatory smile-strangle calibration that reprices the broker/market strangle). `build_trial_smile(ctx, atm_vol, atm_strike, pillar, rr, smile_strangle)` builds one candidate vanna-volga smile for a trial σ_ss inside the calibration bracketing loop: it resolves the three wings via `smile_wings`, then HARD-REJECTS out-of-domain trials as `Err(CalibrationError::DegenerateQuote)` rather than panicking or returning an invalid Ok — (1) a |RR| too large relative to ATM+σ_ss drives a wing vol <= 0, caught by `!(put_vol>0 && call_vol>0)`; (2) the fallible `MarketHedgeSmile::try_new` enforces the strike ordering K1<K2<K3 (put wing < ATM < call wing) and returns the same error if a pathological trial inverts it. This keeps the guarded root-find total: a degenerate trial steps the bracket away, and a terminal degenerate state surfaces as a typed error, never a panic. Pure: reads ctx/scalars/pillar, returns Result<MarketHedgeSmile, CalibrationError>, mutating nothing.

### 148. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.market_strangle`

The broker (market) butterfly is NOT the smile butterfly — the #1 production FX-vol bug (ANALYTICS-SPEC §1.4). `market_strangle` defines what brokers actually trade: a SINGLE vol `strangle_vol = atm_vol + quote.butterfly` applied to BOTH the call AND put pillar strikes (call_strike and put_strike each resolved via strike_at_delta at that one vol), and its price is call+put at that single vol. This is the calibration TARGET that the recovered smile must reprice — it is deliberately distinct from the arithmetic smile-strangle (the per-wing-vol convexity), and the two differ materially for high-RR/EM pairs. Treating the quoted BF as the arithmetic 25Δ smile-strangle silently biases the wings and breaks 10Δ reproduction. Pure: reads ctx/atm_vol/quote, returns Result<MarketStrangle, CalibrationError>, mutating nothing.

### 149. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.smile_wings`

The smile wings carry DISTINCT per-wing vols — the counterpart to the single-vol broker strangle (ANALYTICS-SPEC §1.4, Reiswich-Wystup/Clark). `smile_wings` builds the recovered 25Δ/10Δ wings from the trial smile-strangle σ_ss as `call_vol = σ_ATM + σ_ss + ½RR` and `put_vol = σ_ATM + σ_ss − ½RR` (the risk-reversal RR splits the two wings antisymmetrically), then resolves each wing's strike at ITS OWN vol via strike_at_delta. Because call and put wings get different vols (unlike market_strangle's single vol on both strikes), the explicit broker→smile calibration step is mandatory: a non-zero RR makes the smile-strangle differ from the broker butterfly. Pure: reads ctx/atm_vol/pillar/rr/smile_strangle, returns a Result tuple, mutating nothing.

### 150. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strike_quotes.fit_strike_slice`

DELIVERABLE surface/crypto-leaf (the LEAF half) = DONE (RC cut c5a5efc). `fit_strike_slice(ctx, quotes)` is the pure strike-axis (log-moneyness, NOT FX delta-space RR/BF) smile calibrator that lets a crypto/equity surface be marked from a raw strike-vol grid: it seeds an SVI-style 3-parameter slice deterministically (vertex at the lowest observed total variance, width from the k-span), runs a fixed-iteration projected Gauss-Newton inner solve, then ray-projects onto the butterfly-admissible set (a no-op for any arbitrage-free-reproducible quote set), returning `Result<StrikeSliceFit, CalibrationError>` with the slice plus rms/max reproduction error in absolute vols. Pure: it borrows `(&StrikeSliceContext, &StrikeQuoteSlice)`, mutates only local state, performs no I/O, and is fully deterministic (no RNG). This is the surface LEAF the backlog split out of W3-crypto; the remaining OPEN half is the WIRE surfacing (no strike-axis quote_basis on MarkSurfaceRequest yet — see mark_surface_request_from_json), tracked as surface/crypto-strike-axis-surfacing.

### 151. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.surface.VolSurface<S, C>.arbitrage_report`

The three FX no-arbitrage gates are computed together over the whole surface and are hard-reject inputs (ANALYTICS-SPEC §3.4). `VolSurface::arbitrage_report` produces a SurfaceArbitrageReport with all three diagnostics: (1) BUTTERFLY — `min_density`, the minimum over sampled maturities/strikes of the second-difference risk-neutral density (a negative value ⇒ butterfly arbitrage); (2) VERTICAL — `max_vertical_increase`, the worst call-price increase across ascending strikes (a positive value ⇒ vertical-spread arbitrage, calls must be non-increasing in K); (3) CALENDAR — `min_calendar_increment`, the minimum cross-slice total-variance increment at the wing+ATM log-moneyness (a negative value ⇒ calendar arbitrage, total variance must be non-decreasing in T). Per-maturity work delegates to check_slice; cross-slice calendar to term.min_calendar_increment. A surface is arbitrage-free only when min_density≥0, max_vertical_increase≤0, and min_calendar_increment≥0 — these are gating thresholds, not advisory. Pure: reads &self + sampling params, returns the report value, mutating nothing.

### 152. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.CalendarClock.business_time`

DELIVERABLE surface/event-weighted-clock = OPEN (Round-2 P2/M finding, tracked). `CalendarClock::business_time(&self, t)` is the identity business-time map `tau(t) = t` — it returns its argument verbatim, with no weekend/holiday compression and no scheduled-event (central-bank-meeting / fixing) weighting. It is the ONLY BusinessClock implementation in the workspace (the trait seam exists but has a single identity impl), so term-structure interpolation is currently a no-op on the event-weighted clock that ANALYTICS-SPEC §3.6 specifies as market standard. Pure: it reads only `(&self, t)` and returns `t`, mutating nothing. Closing the deliverable means adding a real event/calendar-weighted BusinessClock impl; the new impl (and any change to this identity body) will change this method's node content and STALE this claim — the staleness firing is the done-signal. This is intentionally a no-arbitrage-safe placeholder, NOT a faked depth claim.

### 153. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure<S, C>.is_calendar_free`

CALENDAR no-arbitrage gate (ANALYTICS-SPEC §3.4 — the cross-tenor third arbitrage axis, distinct from the per-slice butterfly+vertical gate in check_slice, which the check_slice claim explicitly delegates here). `TermStructure::is_calendar_free(k, tol)` is the hard-reject calendar gate: it returns true iff `min_calendar_increment(k, 256) >= -tol`, i.e. total variance w(k,t)=σ²·t is non-decreasing in maturity along a fixed strike. A strict total-variance crossing (a longer-dated pillar carrying LESS total variance than a shorter one) is a calendar-spread arbitrage and is rejected — e.g. 6M@15vol (w=0.01125) vs 1Y@10vol (w=0.01) flags. This is the term-axis member of the three FX no-arbitrage gates (butterfly/calendar/vertical) that are hard-reject inputs. Pure: reads &self pillars and returns a bool, mutating nothing.

### 154. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure<S, C>.min_calendar_increment`

CALENDAR-arbitrage MEASURE primitive (ANALYTICS-SPEC §3.4) behind the calendar no-arbitrage gate. `TermStructure::min_calendar_increment(k, samples)` pins one strike from the near-pillar log-moneyness (`strike = pillars[0].forward · e^k`), then walks that FIXED strike across `samples` maturities t0..t1 — crucially re-converting to EACH maturity's own log-moneyness `k_t = ln(strike / forward_at(t))` before reading `total_variance(k_t, t)` — and returns the minimum forward increment of total variance w between consecutive maturity samples. A negative minimum increment is exactly a total-variance crossing = calendar-spread arbitrage; is_calendar_free rejects when this is below -tol. Fixing the cash strike (not the moneyness) across tenors is the correct no-arb test under term-varying forwards. Pure: reads &self, returns f64, mutating nothing (it does assert samples>=2 as a precondition, but performs no writes).

### 155. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.carry_rate`

Carry::carry_rate() is a pure accessor over the asset-class-agnostic carry seam (FxRates{r_dom,r_for} | CostOfCarry{r,b}). Every pricing engine consumes the Carry seam rather than raw FX rate fields, and there is no hot-path match on Carry/Underlying. This keeps one asset-class-agnostic pricing contract across vanilla/exotics/surface/risk and the crypto/equity/commodity leaves (ADR-0008).

### 156. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.yield_rate`

Carry::yield_rate() returns the stored foreign/yield rate (r_for) VERBATIM — it is a pure accessor with no side effects. FX bit-identity depends on this: foreign-rate reads must use yield_rate() directly and must NEVER be reconstructed as discount_rate() - carry_rate(), which would perturb the FX hot path. Enforced by the ADR-0008 carry-seam architecture (docs/adr/ADR-0008-multi-asset-carry-architecture.md); the carry-seam lowering guard rejects raw cost-of-carry for FX (see celnet-core/src/carry.rs fx_lowering_rejects_cost_of_carry).

### 157. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.flip_orientation`

`PremiumStyle::flip_orientation` is a self-inverse (involution) mapping the premium style under base/quote currency-pair inversion: DomesticPips↔ForeignPips and PercentForeign↔PercentDomestic. Applying it twice is the identity (DomesticPips→ForeignPips→DomesticPips; PercentForeign→PercentDomestic→PercentForeign), so re-quoting the same option in the inverted pair orientation and back recovers the original premium style exactly — the orientation-invariance property the pair-universe view relies on (docs/CONVENTIONS.md §pair universe). It is a `const fn` total match with no wildcard. Pure: reads only `self`, returns the flipped enum, no mutation.

### 158. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.is_premium_adjusted`

`PremiumStyle::is_premium_adjusted` is the canonical predicate deciding whether the premium carries FX risk: it returns true exactly for the FOR/base-ccy-denominated styles `PercentForeign | ForeignPips` and false for the DOM-ccy styles `DomesticPips | PercentDomestic`. A FOR-ccy premium carries FX risk, which is why these styles drive premium-adjusted delta (the non-monotone premium-adjusted call delta of docs/CONVENTIONS.md). The classification is a `const fn matches!` over `self` with no wildcard, so a new PremiumStyle variant forces this distinction to be revisited rather than defaulting silently. Pure: reads only `self`, returns a bool, no side effects.

### 159. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.atm.atm_strike`

Delta-Neutral-Straddle (DNS) ATM strike SIGN-FLIPS with the premium-adjusted delta convention — the single most surface-corrupting convention bug if mislocated. `atm_strike` returns F·exp(+½σ²t) for unadjusted delta (SpotUnadjusted | ForwardUnadjusted) but F·exp(-½σ²t) for premium-adjusted delta (SpotPremiumAdjusted | ForwardPremiumAdjusted) — the DNS strike sits ABOVE the forward when unadjusted and BELOW it when premium-adjusted (opposite sign of the ½σ²t drift), exactly per ANALYTICS-SPEC §1.3. AtmForward simply returns the forward. The match on (AtmConvention, DeltaConvention) is exhaustive over both enums, so the half-variance sign is never defaulted. Pure: a total function of (atm, delta_conv, forward, vol, t) returning f64, no side effects.

### 160. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta_d_strike`

Analytics-correctness (premium-adjusted-delta deliverable): celnet-vanilla::delta::delta_d_strike is a pure analytic ∂Δ/∂K. For the unadjusted conventions (Spot/Forward-Unadjusted) call and put deltas differ by a K-independent constant, so the strike-slope is the single term factor·φ(d1)·∂d1/∂K with ∂d1/∂K = ∂d2/∂K = −1/(K σ√T). For the premium-adjusted conventions (Spot/Forward-PremiumAdjusted) Δ_call = factor·(K/F)·N(d2), so the slope follows the product rule in K (F is K-independent): factor·(N(d2)/F + (K/F)·φ(d2)·∂d2/∂K); the put slope is the call slope minus factor/F since Δ_put = Δ_call − factor·(K/F). This convention-branching strike-derivative is what makes the premium-adjusted delta non-monotone in strike (it underpins the guarded delta→strike root-find). Pure: deterministic in (conv, opt, &VanillaInputs), no WRITES edges.

### 161. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.premium_adjusted_call_delta_max`

Premium-adjusted call delta is NON-MONOTONE in strike — it has a maximum-delta strike with two strikes mapping to the same delta, so the strike↔delta root-find must be guarded and bracketed on the correct branch (ANALYTICS-SPEC §3.5). `premium_adjusted_call_delta_max` computes that delta-max strike as the stationary point: it solves g(d2)=N(d2)·σ√T−φ(d2)=0 (g increasing in d2, root at small positive d2) by bisection, then maps the root d2* back to the strike K = F·exp(−½σ²T − d2*·σ√T). This is the cap the solver must respect: a target delta above the achievable max is unreachable, and a naive monotone Brent/Newton would converge to the wrong branch or diverge. Pure: reads `&VanillaInputs`, returns the cap strike as f64 via a fixed-iteration bisection, mutating nothing.

### 162. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.greeks`

FX has exactly TWO rhos, never one. `greeks` returns both `rho_dom = ∂V/∂r_d` (call: K·t·e^{-r_d t}·N(d2); put: -K·t·e^{-r_d t}·N(-d2)) and `rho_for = ∂V/∂r_f` (call: -S·t·e^{-r_f t}·N(d1); put: +S·t·e^{-r_f t}·N(-d1)) as distinct fields of the Greeks struct — the foreign rate r_f enters as the continuous dividend yield on the foreign-currency asset (Garman-Kohlhagen 1983), so a single equity-style "rho" is meaningless and is never exposed (ANALYTICS-SPEC §2.1). The two rho signs are opposite (domestic-rate up raises a call, foreign-rate up lowers it), so collapsing them would cancel real rate risk. Pure: it reads `opt` and `&VanillaInputs` and returns a `Greeks` value, mutating nothing.

### 163. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.price`

GUARDRAIL/HOT-CORE — the FX vanilla pricing kernel `price` is allocation-free and lock-free by construction: it reads spot/strike discounted by df_for()/df_dom(), evaluates the closed form via norm_cdf on the precomputed aux (d1,d2), and returns the f64 price for Call/Put — no allocation (alloc_in_loop=0, no Vec/Box), no loop (loop_depth=0), no I/O, no logging, no locks. This is the pinned zero-alloc hot core: it is the leaf kernel the engine's hot pricing loop calls (in_degree 13), and the engine's `hot_pricing_loop_allocates_zero` / `hot_pricing_under_concurrent_publish_allocates_zero` tests (a custom counting global allocator asserting zero allocations on the hot path) hold precisely because kernels like this allocate nothing. Pure: it reads &VanillaInputs and the OptionType and returns the f64 price, mutating nothing — telemetry/logging is offloaded off this path, never inlined into it.

### 164. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.premium.premium_from_domestic_pips`

PremiumStyle is the FX quotation-units axis and premium_from_domestic_pips is its canonical converter (docs/CONVENTIONS.md PremiumStyle → premium units; docs/ANALYTICS-SPEC premium quotation). Given a price already in domestic pips (v_dpips), it exhaustively maps the four PremiumStyle variants to their quoted unit: DomesticPips passes the raw PV through unchanged; PercentForeign divides by spot (per unit of foreign/base notional); PercentDomestic divides by strike (per unit of domestic/quote notional at strike); ForeignPips divides by spot·strike. The match is exhaustive over PremiumStyle, so no style is defaulted, and the DomesticPips arm is the identity (domestic_pips_is_the_raw_pv). Pure: a total function of (style, v_dpips, spot, strike) returning f64 with no writes/allocation/IO; deterministic under the f64 CPU-canonical/libm rule. Self-invalidates if the PremiumStyle variant set or any per-style scale factor changes (WRITES gate).

### 165. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.bracket`

`bracket` is the branch-aware bracketing primitive that makes the strike↔delta solve safe under the NON-MONOTONE premium-adjusted call delta (ANALYTICS-SPEC §3.5). It detects the premium-adjusted convention (SpotPremiumAdjusted | ForwardPremiumAdjusted) and, for a Call, computes the delta-max strike via `premium_adjusted_call_delta_max`: a target_delta above delta_max+1e-12 is rejected as DeltaSolveError::Unreachable (the cap is the reachability boundary), and the returned bracket is deliberately pinned to the DECREASING (OTM) branch — lo=K_max where Δ=delta_max≥target ⇒ g(lo)≥0, hi expanded by doubling until g(hi)≤0 as K→∞ where Δ→0 — so the downstream root-find can never land on the ascending (ITM) branch that maps a different strike to the same delta. For unadjusted/put cases delta is monotone, so it geometrically expands [tiny_strike, f] outward toward the shrinking-residual side until a sign change is found, returning Unreachable after 64 unsuccessful doublings. Pure: reads (conv,opt,target_delta,&VanillaInputs,f,&at-closure) and returns Result<(f64,f64),DeltaSolveError>; it allocates nothing and mutates no external state (only loop-local lo/hi/iters).

### 166. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.strike_from_delta`

`strike_from_delta` is the GUARDED strike↔delta root-find the non-monotone premium-adjusted delta demands (ANALYTICS-SPEC §3.5): it never runs a naive monotone Newton/Brent that could converge to the wrong branch. It first enforces sign discipline (a Call target_delta<0 or Put target_delta>0 ⇒ DeltaSolveError::WrongSign), then delegates the bracket to `bracket` — which, for the premium-adjusted call, calls `premium_adjusted_call_delta_max` to obtain the delta-max cap, returns DeltaSolveError::Unreachable for a target above the achievable max, and pins the bracket onto the correct (OTM, decreasing) branch above K_max. The inner loop is a Brent-lite: it keeps the sign-straddling bracket [lo,hi] (debug_assert glo·ghi≤0) as the safety net and only accepts a Newton step using delta_d_strike when it lands strictly inside (lo,hi), else falls back to bisection — so it is bracket-guaranteed convergent and cannot escape onto the wrong delta branch. Returns DeltaSolveError::NoConvergence rather than a wrong root if iteration stalls. Pure: it reads (conv,opt,target_delta,&VanillaInputs), mutates only stack-local copies of the inputs (inp.strike) to evaluate delta/slope, performs no I/O, allocation, or external mutation.

### 167. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.fuzz.fuzz_targets.fix_frame_decode.structured_roundtrip`

DELIVERABLE fix-decoder-fuzz-target = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "celnet-fix violates verification-contract clause (f): no fuzz target for the only byte parser fed external-counterparty bytes"; reconciled against the live graph). `structured_roundtrip` is the pure (side-effect-free) differential oracle inside the now-present fuzz/fuzz_targets/fix_frame_decode.rs harness: it drives the production celnet-fix FrameCursor::parse over adversarial/arbitrary byte frames and asserts the structured-decode↔re-encode round-trip and in-domain invariants, giving the external-counterparty FIX byte parser the fuzz coverage that VERIFICATION-CONTRACT.md clause (f) mandates. SELF-INVALIDATES on any change to this fuzz oracle.

### 168. `spec:satisfies` (draft)
Anchors: `design-target:docs/acceptance/carry-seam.acceptance.json`, `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_vanilla_inputs`

Carry-seam FX byte-identity (carry-seam deliverable, ADR-0008): celnet-core::carry::fx_vanilla_inputs lowers the Carry::FxRates arm to VanillaInputs byte-identically (forward/df_dom/df_for to_bits-equal to the direct VanillaInputs::new) and rejects the generalized Carry::CostOfCarry arm on the FX lowering path — no silent fallback. Acceptance assertions in docs/acceptance/carry-seam.acceptance.json.

### 169. `ui:component:badge` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge`

StatusBadge is the small stream-health status pill: a single <span> driven purely by the StreamHealth enum, selecting a glyph (GLYPH[health]) and a per-state CSS-Module modifier class (styles[health.toLowerCase()]) that colors it from the semantic tokens. It is stateless and presentational, and is the only component with committed Storybook stories (StatusBadge.stories.tsx → components-statusbadge--* in the static index), making it the visual-regression and token-rendering reference fixture.

### 170. `ui:component:dialog` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette`

CommandPalette is the ⌘K command-launcher overlay: a scrim-backed modal dialog with a fuzzy-matched (fuzzyMatch) search input over the caller-supplied Command list, ranking and showing the top results as a keyboard-navigable listbox. It renders null when closed, clears query/active on open, and runs the selected command's run() on Enter/click. It is the central action surface (pairs, workspaces, actions) styled through CSS-Module tokens (scrim/palette/item) rather than inline literals.

### 171. `ui:component:grid` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid`

DataGrid is the single reusable virtualized data-grid primitive (generic over the row datum T): row-windowing (useVirtualWindow) and column-windowing (columnWindow) render only the visible slice, with optional grouping (flattenGroups + collapsible group rows), sortable columns, and roving-tabindex keyboard navigation. It reads row height and cell padding from the density tokens (--row-h, --cell-pad-x/y) via useRowHeight, so the comfortable/compact density axis applies without a JS branch. The blotter (StreamWorkspace), risk and book workspaces all compose this one component rather than re-implementing a grid.

### 172. `ui:component:strip` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.GreeksStrip.GreeksStrip`

GreeksStrip is the inline option-risk readout: a primary row of GreekCell tiles (delta/gamma/vega/theta) plus a disclosure button (aria-expanded + aria-label="toggle full Greeks") that reveals the secondary Greeks. It is asset-class-aware — rhoGreeksFor(assetClass) relabels the rate-rho Greeks per the active underlier's class (FX default) — so the same strip serves FX/equity/commodity/crypto tickets. Numerics render through GreekCell on the mono token face; the strip itself carries no raw color literals.

