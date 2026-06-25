# Verified knowledge — anchored mirror (committed source of truth)

Durable, anchor-carrying record of every Tier-1 lodestar claim. Rebuild the live
projection: `python3 tools/lodestar/replay-knowledge.py`.

**465 claims** — kinds: a11y:labeled=2, a11y:role=2, design:token=1, invariant=154, invariant:pure=298, spec:satisfies=4, ui:component:badge=1, ui:component:dialog=1, ui:component:grid=1, ui:component:strip=1

states: active=247, draft=168, stale=50

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
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.benches.iai_instructions.instruction_gate`, `github.com-soarsa-celnet.crates.celnet-bench.benches.iai_instructions.soft_regression_limits`

instruction_gate constructs a per-benchmark LibraryBenchmarkConfig that combines the shared soft-regression limits (SOFT_INSTRUCTION_REGRESSION_PCT on Ir, SOFT_ESTIMATED_CYCLES_REGRESSION_PCT on EstimatedCycles, both sourced from soft_regression_limits()) with a per-benchmark hard absolute ceiling on Ir. This dual-layer design means every iai-callgrind benchmark gets a CI-fail-on-regression soft guard AND a hard absolute Ir ceiling below which the operation must stay regardless of history.

### 7. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.bin.bench_gate.main`

CAPABILITY SUMMARY — celnet-bench is the performance-proof crate for the celnet FX-options platform. It turns the latency/throughput budgets from docs/ARCHITECTURE.md §1.2 into reproducible, asserted proof across four distinct gate arms: (1) in-core absolute budget (price + full 13-Greek set, p50 ≤ 2µs / p99 ≤ 10µs / p99.9 ≤ 25µs, measured pinned/elevated via `core_load`); (1b) surface-rebuild absolute budget (per-pair all-tenors VV + SSVI recompute p99 ≤ 150µs, via `surface_rebuild`); (2) wire-path relative regression gate (round-trip RFQ latency under concurrent RFS streaming load, relative to a committed JSON baseline, via `wire`); (3) fleet §11 SLO relative regression gate (cross-shard federation overhead, publish→snapshot lag, conflation correctness, many-subscriber fan-out spread, via `fleet_slo`). All four arms are wired into the `bench_gate` binary which exits non-zero on any breach. The crate also provides shared, allocation-free input fixtures (`representative_inputs`, `representative_batch`, `sweep_inputs`) consumed identically by benches and unit tests, and an iai-callgrind instruction-count gate (`iai_instructions`) with soft-regression limits on `Ir` and `EstimatedCycles`.

### 8. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.core_load.CoreBudget.architecture_1_2`

PILLAR (CLAUDE.md guardrails 6 + 11 — scale & performance are requirements: the pricing hot core must stream prices to high-performance counterparties at the latency budgets in docs/ARCHITECTURE.md §1.2). `CoreBudget::architecture_1_2` is the single, authoritative encoding of the per-price hot-core tail-latency budget: a parameterless `const fn` returning `Self { p50_ns: 2_000, p99_ns: 10_000, p999_ns: 25_000 }` — i.e. the 2µs/10µs/25µs p50/p99/p99.9 envelope from ARCHITECTURE §1.2. Being a `const fn` over literals it is pure and side-effect-free by construction (no allocation, I/O, or mutation); it has no WRITES edges. The bench load harness (CoreReport::budget_breaches) measures the live core against exactly this struct, so the latency pillar is reconciled against a single source of truth rather than asserted. SELF-INVALIDATING: editing any budget constant (or the signature) shifts this anchor and flips the claim stale, re-opening review against the doc.

### 9. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.core_load.CoreReport.budget_breaches`

PILLAR (CLAUDE.md guardrails 6 + 11 — the latency budget in docs/ARCHITECTURE.md §1.2 is enforced, not aspirational). `CoreReport::budget_breaches` is the pure, deterministic detector that turns a measured hot-core latency profile into the set of budget violations: it reads only `&self` (the measured `latency` p50/p99/p99.9 and the `budget` ceilings), forms the three (metric, measured_ns, ceiling) tuples, and returns a freshly-allocated `Vec<BudgetBreach>` containing exactly those percentiles where measured > ceiling. No `&mut`, no I/O, no shared/global mutation — allocating the returned Vec is not an externally-observable side effect; it has no WRITES edges. Determinism is the load-bearing property: the same measured profile against the same budget always flags the same breaches, so a CI gate cannot non-reproducibly hide a regression past the §1.2 envelope. SELF-INVALIDATING: introducing a write or changing the comparison shifts this anchor and flips the claim stale.

### 10. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.core_load.core_load_measures_and_passes_1_2_budget`

The in-suite core_load test (core_load_measures_and_passes_1_2_budget) deliberately does NOT assert the strict §1.2 absolute ceilings (p50 ≤ 2µs / p99 ≤ 10µs / p99.9 ≤ 25µs): cargo nextest saturates all cores so measured latencies reflect sibling-test contention, not the hot path. Instead it asserts a 100× gross-sanity ceiling (CoreBudget::architecture_1_2().p99_ns × 100) — a contention-robust floor that only a catastrophic regression breaches. The strict gate runs only in the dedicated core_load binary / bench_gate arm 1 / CI core-load-gate lane on a pinned thread.

### 11. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.fleet_slo.compare_to_baseline`

compare_to_baseline (fleet_slo) checks 8 named latency metrics (federation.direct.p99, federation.federated.p99, publish_snapshot.p99, publish_snapshot.p99.9, conflation.stalled_p99, fan_out.window.p99, fan_out.window.p99.9, federation.added_p99) as relative regressions: ceiling = max(baseline, 0.001) × (1 + tolerance). A degenerate zero baseline is floored at 0.001 to avoid dividing meaning out of near-zero values. Returns a Vec<FleetBreach> (empty = pass); caller bench_gate arm 3 exits non-zero on any breach.

### 12. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.fleet_slo.measure_fan_out`

measure_fan_out measures broadcast fan-out spread across N subscribers using N sync_channel<u64> channels (one per subscriber). Each subscriber blocks on recv (no busy-spin), so N readers on a core-constrained host sleep until the producer fans an epoch — they never starve each other. The delivery window per round is max_stamp − fan_start across all N subscribers, recorded into a 1ns–1s HdrHistogram at 3 significant figures. The FanOutSpread result carries p50/p99 window (µs) and p99.9 (µs); it is gated by fleet_slo arm 3 in bench_gate against the committed loopback baseline at the configured tolerance.

### 13. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.surface_rebuild.SurfaceBudget.architecture_1_2`

PILLAR (CLAUDE.md guardrails 6 + 11 — IB-sized portfolios / many-instrument batch must rebuild surfaces within the docs/ARCHITECTURE.md §1.2 budget). `SurfaceBudget::architecture_1_2` is the authoritative encoding of the per-surface-rebuild p99 latency budget: a parameterless `const fn` returning `Self { p99_ns: 150_000 }` — the 150µs p99 surface-rebuild ceiling from ARCHITECTURE §1.2. Being a `const fn` over a literal it is pure and side-effect-free by construction (no allocation, I/O, or mutation); it has no WRITES edges. The surface_rebuild load harness (SurfaceReport::budget_breaches) measures each model's live rebuild p99 against exactly this struct, so the surface-scale pillar is reconciled against a single source of truth. SELF-INVALIDATING: editing the constant or signature shifts this anchor and flips the claim stale.

### 14. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.surface_rebuild.SurfaceReport.budget_breaches`

PILLAR (CLAUDE.md guardrails 6 + 11 — many-instrument surface rebuild must stay inside the docs/ARCHITECTURE.md §1.2 p99 budget). `SurfaceReport::budget_breaches` is the pure, deterministic detector over a multi-model surface-rebuild report: it reads only `&self`, iterates the measured per-model `latency.p99_ns`, filters those exceeding `budget.p99_ns`, and returns a freshly-allocated `Vec<BudgetBreach>` (each carrying the static model name, the "p99" metric, the measured ns and the ceiling). No `&mut`, no I/O, no shared/global mutation — allocating the returned Vec is not an externally-observable side effect; it has no WRITES edges. Determinism guarantees a per-model p99 regression past the §1.2 surface budget is flagged reproducibly. SELF-INVALIDATING: a write or comparison change shifts this anchor and flips the claim stale.

### 15. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.tests.architectural_invariant.in_process_price_resolves_locally_without_any_router`

The architectural invariant test in_process_price_resolves_locally_without_any_router proves that the per-tick hot price path never requires the router/federation layer: it starts a plain in-process Edge (FleetTopology::InProcess, no CELNET_FLEET_BACKENDS env var, no fleet handle), issues a real PriceRequest over loopback gRPC, and asserts `greeks.price.is_finite() && greeks.price > 0.0`. Because there is no router, the ONLY way the call can succeed is via the Serve::Local arm — a regression that accidentally routes would fail with no forwarding target.

### 16. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.calendar.BusinessCalendar.is_business_day`, `github.com-soarsa-celnet.crates.celnet-calendar.src.calendar.BusinessCalendar.with_centres`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.calendar_for`

`calendar_for(pair) -> BusinessCalendar` applies the FX convention that cross pairs (neither leg is USD) get a three-centre calendar (base + quote + USD); USD-leg pairs get a two-centre calendar (base + quote only). A date is a business day only when ALL constituent centres are open — the intersection semantics enforced by `BusinessCalendar::is_business_day` (`self.centres().all(|c| !c.is_non_business(date))`). `with_centres` de-duplicates centre IDs inline so repeated entries (e.g. USD appearing twice in a USD-leg pair after the cross guard) never produce duplicate checks.

### 17. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.delivery_date`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.expiry_for_tenor`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.schedule`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.spot_date`

CAPABILITY SUMMARY — celnet-calendar is the FX date-arithmetic engine for the pricing stack. Its public contract is: `schedule(pair, horizon, tenor) -> Result<FxSchedule, TenorError>` is the single top-level entry point; it returns (horizon, spot, expiry, delivery, vol_anchor) for any Tenor variant (Overnight, TomNext, SpotNext, Weeks, Months, Years, Imm(n), BrokenDate). The vol_anchor is `horizon` for ON/TN (so they never produce zero or negative vol-time) and `spot` for all other tenors. All internal date arithmetic flows through `BusinessCalendar` (multi-centre intersection semantics), `RollRule::ModifiedFollowing`, and the centre-specific `SettlementCentre::is_holiday` dispatch. New currency pairs plug in by extending `centre_for`/`is_t_plus_one_pair`; no change to `schedule` or any downstream pricer is required.

### 18. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.SettlementCentre.is_holiday`, `github.com-soarsa-celnet.crates.celnet-calendar.src.holiday.SettlementCentre.new`

`SettlementCentre::is_holiday(date)` is the exhaustive holiday-dispatch switch for all 12 supported settlement centres: UnitedStates, Target2, UnitedKingdom, Japan, Switzerland, Australia, Canada, NewZealand, Mexico, SouthAfrica, Norway, Sweden. Each arm delegates to a dedicated per-jurisdiction function (e.g. `is_japan_holiday` handles the full Japan rule-set including citizens' holidays, equinox days, and transfer holidays). Adding a new currency's settlement centre requires adding a new `CentreId` variant and a new arm here — the exhaustive match makes omission a compile error.

### 19. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.roll.RollRule.adjust`

`RollRule::adjust` implements the four standard date-roll conventions as a pure match over `RollRule`: Unadjusted (identity), Following (forward to next business day), Preceding (back to previous), and ModifiedFollowing (forward unless the result crosses a month boundary, in which case fall back to preceding). The ModifiedFollowing branch is: `let fwd = following(cal, date); if fwd.month() != date.month() { preceding(cal, date) } else { fwd }`. This is the canonical roll used by all week/month/year/IMM/BrokenDate tenor arms in `expiry_for_tenor`.

### 20. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.cli.dispatch`, `github.com-soarsa-celnet.crates.celnet-cli.src.main.main`

celnet-cli is the single-binary CLI for the entire celnet FX-options platform. Its public contract is the `dispatch` function: given a parsed `Cli` value and a `Write` sink, it routes all 14 commands (Price, Surface, Exotic, Basket, Forward, Swap, Ndf, Perpetual, FutureOption, Convention, Risk/Aggregate/Drill/Positions/Limits, Stream, Rfq) to the appropriate domain modules and writes a human-readable report. `main` calls `dispatch`, locks stdout, and maps `DispatchError` to `ExitCode::FAILURE` with a stderr message — zero panics on valid input. New commands plug in by adding a `Command` variant, a corresponding `Args` struct via clap, and a dispatch arm.

### 21. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.cli.dispatch`

The `dispatch` function enforces all cross-field domain invariants before calling any pricing engine: (1) exactly one of --strike, --delta, or --atm must be supplied for Price/Stream/Rfq commands — a missing specifier returns `DispatchError::Invalid`; (2) `--settlement-style inverse-coin` is rejected unless `--asset crypto`; (3) struck exotic families (vanilla, digital, barrier, asian, quanto, tarf, pivot, fixed-strike lookback, american) require an explicit `--strike` via `strike_for(family)?`, while unstruck families carry `0.0` silently; (4) the window-barrier product rejects the `--model analytic` flag because it has no closed form.

### 22. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.cli.dispatch_risk`, `github.com-soarsa-celnet.crates.celnet-cli.src.risk.aggregate_query_carries_var_and_curvature`

The `dispatch_risk` function routes the four `RiskKind` variants (Aggregate, Drill, Positions, Limits) to the matching `risk::run_*` SDK calls, threading through a common `RiskCommon` (endpoint, numeraire currency, FX rates) built by `risk_common`. The `AggregateReq` carries VaR shock scenarios (`var_shocks: Vec<f64>`), a confidence level (`var_alpha: f64`), and a curvature risk weight (`curvature_risk_weight: f64`), confirming the CLI exposes the full FRTB-aligned risk query surface.

### 23. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.risk.entitlements`, `github.com-soarsa-celnet.crates.celnet-cli.src.risk.no_entitlement_flags_is_grant_all_default`

The `entitlements` helper in the `risk` module implements a three-state entitlement model: (1) no `--grant` and no `--deny` flags → returns `None`, which the SDK interprets as grant-all (parity with GUI/SDK/Excel clients); (2) one or more `--grant` flags → `Entitlements::scoped()` base with each grant added; (3) `--deny` flags alone → `Entitlements::grant_all()` base with denies layered on top. This ensures the CLI never silently over-restricts access when no flags are provided.

### 24. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.lib.Client.aggregate_risk`, `github.com-soarsa-celnet.crates.celnet-client.src.lib.Client.drill_risk`, `github.com-soarsa-celnet.crates.celnet-client.src.risk.principal_or_grant_all`

`Client::aggregate_risk` and `Client::drill_risk` are the two entitlement-gated risk query entry points. `aggregate_risk` sends an `AggregateRisk` RPC built via `risk::aggregate_request(&AggregateQuery)` and decodes the response into a `RiskAggregate`; `drill_risk` drills one node into its children via `DrillRisk` RPC. Both calls pass entitlements through `principal_or_grant_all`: when no explicit `Entitlements` principal is provided, `Entitlements::grant_all()` is serialised — the authenticating gateway overrides this in production.

### 25. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.lib.Client.get_smile`, `github.com-soarsa-celnet.crates.celnet-client.src.surface_vocab.Smile.atm_vol`

`Client::get_smile` fetches a single volatility smile slice: it sends a `GetSmileRequest` carrying a `CcyPair`, `tenor_years: f64`, and `Conventions` to `SurfaceService/GetSmile`, then decodes the response into a typed `Smile`. The `Smile` value carries a vector of `SmilePoint` (delta, vol) pairs at a single tenor; `vol_at_delta` and `atm_vol` are the primary read accessors.

### 26. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.lib.MultiDealerRfq`, `github.com-soarsa-celnet.crates.celnet-client.src.lib.MultiDealerRfq.accept`, `github.com-soarsa-celnet.crates.celnet-client.src.vocab.RankedPanel.best_bid`, `github.com-soarsa-celnet.crates.celnet-client.src.vocab.RankedPanel.best_offer`

`MultiDealerRfq::accept` is the multi-dealer best-bid/offer click-to-trade path: it delegates to `accept_line(panel, side, String::new())`, which serialises and sends an `AcceptLine` carrying the `RankedPanel`'s winning dealer row. `RankedPanel::best_bid` and `best_offer` resolve the winning dealer by looking up `best_bid_lp_id` in the dealer map, returning `None` when no quote is present — ensuring no aliasing to a neighbour's quote.

### 27. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.rfs.Subscription.execute`

`Subscription::execute` implements the click-to-trade protocol for streamed lines with a race-free waiter registration guarantee: the `oneshot` receiver is inserted into the shared `waiters` map under a monotonic `correlation_id` BEFORE the `Execute` frame is sent over the control channel, so the demultiplexing driver can never deliver the outcome before the receiver is in place. If the send fails (stream closed), the waiter is cleaned up immediately and `ClientError::StreamClosed` is returned.

### 28. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.rfs.drive_session`, `github.com-soarsa-celnet.crates.celnet-client.src.rfs.reconnect_session`

`drive_session` is the long-lived async loop that multiplexes a streaming-price session. It selects over two branches: control frames from the caller (forwarded to the live gRPC stream) and inbound server frames (demultiplexed via `handle_frame`). When `handle_frame` signals `SessionFlow::Reconnect`, `reconnect_session` is called: it immediately fails all in-flight click waiters with `ClientError::Reconnected`, re-opens the gRPC stream, resets each subscription's `last_seq` to 0, re-sends all `Subscribe` frames, and emits a `StreamEvent::Reconnected` on each subscription channel.

### 29. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.CommodityInputs.on_future`, `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.CommodityInputs.on_spot`, `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.greeks_with_margining`

CAPABILITY SUMMARY — celnet-commodity-vanilla is the commodity/futures-options analytics leaf on the carry seam. Its public contract is (CommodityInputs, OptionType, Margining) → (price f64 | CarryGreeks) with two distinct margining modes dispatched through greeks_with_margining: EquityStyle (discounted Black-76, used for physically-settled exchange options) and FuturesStyle (undiscounted Black-76, used for CME-style daily-margined contracts). The crate exposes two smart constructors — CommodityInputs::on_future (sets b=0, spot:=future price) and CommodityInputs::on_spot (sets b=r−convenience_yield) — that encode the standard cost-of-carry reparameterization. New arms plug in by calling greeks_with_margining or price_with_margining with the appropriate Margining tag; no switch is needed anywhere else in the pricing stack.

### 30. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.has_pair_profile`, `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair_meta`, `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.resolve`

CAPABILITY SUMMARY — celnet-conventions is the single authoritative registry for all FX-options market conventions. Its primary public contract is `resolve(pair, tenor) -> ResolvedConvention`: for covered pairs it returns a `ConventionRecord` sourced from a static `PairProfile` (tagged `ResolutionSource::PairProfile`); for any unknown pair it falls back to a region-derived default (tagged `ResolutionSource::RegionDefault`). Callers plug into the registry by calling `resolve` or `pair_meta`; the `ResolutionSource` tag lets consumers distinguish authoritative per-pair data from region defaults.

### 31. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryInputs.discount_df`

CarryInputs::discount_df() is a pure accessor delegating to self.carry.discount_df(self.t) — no side effects, no mutation. For an FX Carry::FxRates carry this is byte-identical to VanillaInputs::df_dom (the FX two-rate discount), since the underlying Carry::discount_df reads r_dom verbatim. This is the generalized carry-tagged discount used by the cross-asset pricing seam (celnet-core CarryInputs / CarryPricer), proved byte-identical for FX by fx_carry_inputs_byte_identical (ADR-0008).

### 32. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryPricer.price`, `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryPricer.price_greeks`, `github.com-soarsa-celnet.crates.celnet-core.src.lib.Smile.implied_vol`, `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf`

CAPABILITY SUMMARY — celnet-core is the zero-IO, zero-alloc pure-domain foundation layer of the celnet pricing platform. Its public contract is three seams: (1) the `Smile` trait — the abstraction between surface-construction (`celnet-surface`: Vanna-Volga/SABR/SVI) and all consumers that need an implied Black vol at a given strike; (2) the `CarryPricer` trait — the object-safe pricing seam that each asset-class leaf implements to price against `CarryInputs` (containing an `Underlying` + `Carry` tag) with `price(opt, inputs) -> Result<f64, CarryPriceError>` and `price_greeks(opt, inputs) -> Result<CarryGreeks, CarryPriceError>`; (3) the `math` module — deterministic libm-backed transcendental wrappers (`exp`, `ln`, `sqrt`, `norm_cdf`, `norm_pdf`) that guarantee bit-identical f64 results across platforms. The crate is `#![forbid(unsafe_code)]` with zero runtime dependencies beyond `celnet-types` and `libm`. A new asset-class arm plugs in by implementing `CarryPricer`; a new surface model plugs in by implementing `Smile` — neither requires changes to this crate.

### 33. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_vanilla_inputs`

fx_vanilla_inputs(inputs) is a pure (side-effect-free) lowering from the generalized CarryInputs to the FX leaf's VanillaInputs. It accepts only FX-family underlyings (Underlying::Fx | Underlying::Metal — a metal's lease rate is modelled as the FX foreign rate) carried by Carry::FxRates, and typed-rejects every other underlying (Equity/Commodity/DigitalAsset) or a Carry::CostOfCarry carry with CarryPriceError rather than silently mis-pricing under FX arithmetic. The rate mapping is a verbatim field copy of (r_dom, r_for) — no recomputation — so the resulting forward/df_dom/df_for are byte-identical to CarryInputs::forward/discount_df (proved by fx_carry_inputs_byte_identical; ADR-0008 metals byte-identity).

### 34. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.lib.FlatSmile.implied_vol`, `github.com-soarsa-celnet.crates.celnet-core.src.lib.Smile.atm_forward_vol`, `github.com-soarsa-celnet.crates.celnet-core.src.lib.Smile.implied_vol`, `github.com-soarsa-celnet.crates.celnet-core.src.lib.flat_smile_is_constant`

The `Smile` trait is the decoupling seam between vol-surface construction and all pricing consumers. `Smile::implied_vol(&self, strike, forward, t) -> Vol` returns the arbitrage-aware Black implied vol at a given (strike, forward, t); implementations MUST be arbitrage-aware per the doc contract. `Smile::atm_forward_vol(&self, forward, t) -> Vol` is a provided hot-path convenience that defaults to `self.implied_vol(forward, forward, t)`. `FlatSmile` is the canonical degenerate implementation — it returns `Vol(self.vol)` for every (strike, forward, t) and is used as the test oracle and flat-surface building block throughout the platform. `flat_smile_is_constant` asserts vol is unchanged off-ATM. The engine (`celnet-engine::PricingCore`) and exotic pricers (`celnet-exotics`) depend only on `&dyn Smile` — never on a concrete surface model — so SABR/SVI/VV models are swappable without touching pricing code.

### 35. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.exp`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": "Transcendentals via rust-lang/libm (correctly-rounded) for bit-identical cross-platform results"). `celnet_core::math::exp` is the canonical e^x transcendental wrapper: it is `pub fn exp(x: f64) -> f64 { libm::exp(x) }` — a pure (side-effect-free, no WRITES) total delegation to the correctly-rounded rust-lang/libm routine rather than std's platform libm, so every consumer (e.g. norm_pdf, Carry::forward_factor/discount_df via libm::exp) gets a single deterministic, cross-platform-identical exponential. The whole pricing core routes through this one wrapper so f64 results are byte-reproducible regardless of host C library.

### 36. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.ln`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": transcendentals via rust-lang/libm for bit-identical cross-platform results). `celnet_core::math::ln` is the canonical natural-logarithm transcendental wrapper: it is `pub fn ln(x: f64) -> f64 { libm::log(x) }` — a pure (side-effect-free, no WRITES) total delegation to the correctly-rounded rust-lang/libm `log` routine (note libm::log == ln, not log10). Routing every ln(·) (e.g. log-moneyness ln(F/K) in d1/d2) through this one libm-backed wrapper keeps the f64 CPU-canonical result identical across hosts, never std's platform-dependent ln.

### 37. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.sqrt`

Determinism rule (docs/INTERFACES.md §"Determinism rules baked into the interfaces": transcendentals via rust-lang/libm for bit-identical cross-platform results; f64 is the CPU-canonical scalar). `celnet_core::math::sqrt` is the canonical square-root wrapper: it is `pub fn sqrt(x: f64) -> f64 { libm::sqrt(x) }` — a pure (side-effect-free, no WRITES) total delegation to rust-lang/libm. Routing every sqrt(·) (e.g. sigma*sqrt(T) total variance/vol-time scaling) through this one libm-backed wrapper makes the f64 result byte-identical across platforms rather than depending on the host math library.

### 38. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.core.PricingCore.drain`, `github.com-soarsa-celnet.crates.celnet-engine.src.core.PricingCore.run`

PricingCore::run() is the engine's main loop: it spins on drain() (budgeted batch pop from the rtrb request ring) until the AtomicBool `stop` is observed Relaxed-true, then performs a final quiesce loop calling drain() until empty (bounded by ring capacity), ensuring every request enqueued before stop was set is priced and pushed to the response ring before returning. The loop calls std::hint::spin_loop() on idle (zero-priced drain) to hint the CPU without parking — preserving the sub-microsecond re-observation of `stop`.

### 39. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.core.PricingCore.price`, `github.com-soarsa-celnet.crates.celnet-engine.src.core.PricingCore.run`, `github.com-soarsa-celnet.crates.celnet-engine.src.journal.recover`, `github.com-soarsa-celnet.crates.celnet-engine.src.rt.StateHandle.publish`

CAPABILITY SUMMARY — celnet-engine is the hot-path FX-options pricing engine: it exposes PricingCore as the single entry point for pricing vanilla options against a live smile surface, publishes top-of-book results atomically via a seqlock (PriceSnapshot), persists position state through DurableBook/journal (crash-recoverable and bit-identical on replay), and supports lock-free hot market-state reloads via StateHandle (arc-swap). A new pricing arm plugs in by: (1) constructing a PricingCore with an initial MarketState; (2) calling run() with an rtrb SPSC request/response ring; (3) publishing updated MarketState ticks into StateHandle::publish() from a separate thread with no locking; (4) opening a DurableBook for journal-backed crash recovery via recover().

### 40. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.core.PricingCore.price`, `github.com-soarsa-celnet.crates.celnet-engine.src.core.price_matches_direct_vanilla_at_smile_vol`

PricingCore::price(&mut self, req: PriceRequest) -> PriceResponse computes a single vanilla option price in four deterministic steps: (1) load the current MarketState via arc-swap (lock-free reader); (2) call smile.implied_vol(req.strike, forward, st.t) to look up the smile-surface Black vol for the request strike; (3) form VanillaInputs{spot, strike, vol, t, r_dom, r_for} and call celnet_vanilla::greeks(option_type, &inputs); (4) write the resulting PriceSnapshot into the top-of-book seqlock and increment the priced counter. The test price_matches_direct_vanilla_at_smile_vol confirms that resp.greeks.price == celnet_vanilla::greeks(…, &VanillaInputs{…, smile_vol, …}).price to 1e-12 relative / 1e-14 absolute tolerance.

### 41. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.core.hot_reload_observed_without_locking`, `github.com-soarsa-celnet.crates.celnet-engine.src.rt.StateHandle.load`, `github.com-soarsa-celnet.crates.celnet-engine.src.rt.StateHandle.publish`

StateHandle::publish(&self, state: MarketState) replaces the engine's live market state lock-free via arc-swap (self.inner.store(Arc::new(state))). StateHandle::load(&self) -> Arc<MarketState> reads it (self.inner.load_full()). PricingCore::price() calls self.reader.load() on every request, so a market-state update published from any thread is immediately visible to the pricing hot path on the next request with no locking and no stale read. The test hot_reload_observed_without_locking confirms: after publish(st2) with a higher spot, the next core.price() returns a higher call price.

### 42. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.enc_cut`

ADR-0007 (one clean unversioned contract — engine hot-upgrade handoff, ENCODE side; the inverse of dec_cut). `enc_cut(v: Cut) -> u8` is the pure, total encoder of the expiry-Cut discriminant into the single handoff byte image: NewYork1000->0, Tokyo1500->1, with no wildcard arm, so adding a Cut variant is a compile-time break rather than a silent mis-encode. It reads only its `Cut` argument and writes nothing (pure: no side effects, no allocation). Because there is exactly ONE current contract (guardrail 9: no schema_version, no N/N-1 negotiation), enc_cut/dec_cut are a fixed mutually-inverse codec pair consumed by serialize_state/restore_state — a hot-upgrade deploys a single uniform version with no mixed-version window, so the byte mapping needs no version tag. This is the decision/trade-off: a tagless 1-byte discriminant (cheapest, deterministic) is sound precisely because the unversioned-contract rule removes any back-compat obligation.

### 43. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.restore_state`

ADR-0007 (one unversioned contract — engine hot-upgrade handoff, decode side). `restore_state(bytes)` is the single deterministic DECODER of the engine handoff byte image and the exact inverse of `serialize_state`: it is a pure function of its borrowed `&[u8]` (it reads no global/external state and mutates none — the only allocations are the returned BookState::entries Vec and the reconstructed MarketState), so the same bytes always yield the same (MarketState, BookState). It validates the fixed self-describing layout — a leading MAGIC sentinel (BadMagic on mismatch), then market scalars (spot, r_dom, r_for, t), conventions, the three smile benchmark pillars + reference forward/time from which MarketHedgeSmile::new reconstructs an identical smile, then the length-prefixed book — and calls Reader::finish() to reject trailing bytes, never a silent default. DECISION/RATIONALE: there is NO schema_version field and NO N/N-1 negotiation; hot-upgrade carries state across a code swap through this ONE current format with only a MAGIC discriminant. An upgrade deploys a single uniform engine version (old build serializes, new build restores) so there is no mixed-version window — the format evolves in place rather than versioning, consistent with the platform-wide single-unversioned-contract decision and proved exact by roundtrip_restores_identical_state / restored_state_reprices_identically. (Guardrail: no versioned APIs; hot-upgradable single-version estate. Pairs with cl_d9376a70dfdbd751 on serialize_state.)

### 44. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.journal.DurableBook.mark`, `github.com-soarsa-celnet.crates.celnet-engine.src.journal.last_mark_wins`, `github.com-soarsa-celnet.crates.celnet-engine.src.journal.recover`, `github.com-soarsa-celnet.crates.celnet-engine.src.journal.replay_into_state`

recover(path) -> Result<(MarketState, BookState), RecoveryError> is the crash-recovery entry point: it opens the on-disk journal at `path`, replays all events via replay_into_state() (which dispatches each decoded payload to BookState::push for BookLine events or records the last MarkState), and returns the final (MarketState, BookState) pair. Recovery fails with RecoveryError::NoMarkedState if the journal contains no MarkState record (nothing to price against). DurableBook::mark(&mut self, market: MarketState) appends an encoded MarkState record to the journal; on replay the last mark seen wins (last_mark_wins test: two marks at spot=1.10 and 1.25, recover returns spot=1.25 bit-identical).

### 45. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.rt.Seqlock<T>.read`, `github.com-soarsa-celnet.crates.celnet-engine.src.rt.Seqlock<T>.store`

Seqlock<T>::store(&self, value: T) is the single-writer, multi-reader publish primitive for PriceSnapshot top-of-book. Write protocol: (1) load current even seq; (2) store seq+1 with Release (marks write in-progress, odd); (3) write payload word-by-word (relaxed per-word atomics); (4) store seq+2 with Release (even, marks complete). Seqlock<T>::read(&self) -> T retries until it observes an even `before` sequence and confirms `before == after` (both Acquire loads), rejecting any torn read. In debug_assertions mode a writer_active flag traps concurrent store() calls. This is the zero-lock mechanism that lets the reader thread observe PriceSnapshot without any mutex.

### 46. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.entitled_cube`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.new`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.prune`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.grant_all`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.scoped`

celnet-entitlements is the deny-by-default risk-entitlement crate for the celnet platform. Its public contract is: (1) construct a `Principal` via `Principal::grant_all()` (all-access, deny-overridable) or `Principal::scoped().grant(Rule::on(dim, val))` (explicit allow-list, deny by default when no grants match); (2) bind it with a `Hierarchy` into an `EntitlementFilter`; (3) call `EntitlementFilter::prune(&facts) -> Vec<RiskFact>` to filter a fact slice, or `EntitlementFilter::entitled_cube(facts) -> Cube` to build an entitlement-gated risk aggregation cube. A new entitlement arm plugs in by supplying a `Principal` built from `Rule` scopes and wiring it into `EntitlementFilter::new`; no other extension points are required.

### 47. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.lib.scoped_principal_pruned_before_aggregation_no_leakage`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.lib.scoped_principal_with_no_grants_admits_nothing`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.scoped`

Scoped-principal no-leakage guarantee: a `Principal::scoped()` principal with a single `grant(Rule::on(DimensionId::Desk, 1))` prunes facts to exactly those whose `Book → Desk` hierarchy resolves to desk 1. The resulting `entitled_cube` delta sum equals the sum of desk-1 facts only, and is strictly different from the full-firm total (desk-2 magnitude does not leak), verified by `scoped_principal_pruned_before_aggregation_no_leakage`. A `scoped()` principal with no grants admits nothing, verified by `scoped_principal_with_no_grants_admits_nothing`.

### 48. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.american.american_fd`

american_fd solves the American/Bermudan option pricing LCP via a Crank-Nicolson/Rannacher PSOR finite-difference scheme on an exponentially-spaced log-spot grid. It applies Rannacher smoothing (full implicit θ=1) for the first grid.rannacher_steps time steps then switches to Crank-Nicolson (θ=0.5) for the remainder. For Bermudan-exercisable layers it solves the PSOR LCP (V ≥ intrinsic g) via step_psor; for between-date layers it runs the plain European implicit step. Far-field boundary conditions are applied before each step via FarField::apply, with the early-exercise floor only on exercisable layers. Returns the price interpolated at the current spot from the FD grid.

### 49. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.american.american_lsm`

american_lsm implements the Longstaff-Schwartz regression-based Monte Carlo for American/Bermudan options using scrambled Sobol QMC (via SobolSequence + BrownianBridge) with degree-3 polynomial regression in normalised moneyness x=S/K (not raw S, to keep the design matrix well-conditioned — raw spot³ ≈ 10⁶ would bias the least-squares solve). Backward induction exercises path p at date k when intrinsic(s) >= estimated continuation value from ridge_regress. Carry accessors are read once before the path loops. Returns LsmEstimate{price, std_error} as the mean of per-path discounted cashflows.

### 50. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single_barrier_price`, `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.ExoticInputs.new`, `github.com-soarsa-celnet.crates.celnet-exotics.src.lsv.LsvModel.calibrate`, `github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.tarf_price`

CAPABILITY SUMMARY (celnet-exotics): The crate is the full exotic-products pricing engine for the Celnet platform, providing closed-form, PDE (ADI/PSOR FD), and Monte Carlo (QMC Sobol + antithetic) pricers for: single/double barriers (Reiner-Rubinstein closed form + rebate decomposition), digital/touch options, TARFs and accumulators (antithetic-pair Euler MC), lookback options (Conze-Viswanathan fixed + floating closed form), Asian options (Turnbull-Wakeman analytic + MC), forward-starts and cliquets, American/Bermudan options (PSOR FD + Longstaff-Schwartz MC), perpetual options, multi-asset baskets (scrambled Sobol QMC with Cholesky correlation), and the LSV (Local-Stochastic Volatility) model via particle calibration + ADI PDE. The public contract seam is ExoticInputs, which carries the generalized Carry struct (ADR-0008) so all pricers are asset-class-agnostic. The market-hedge overlay (Vanna-Volga) adds smile cost on top of any flat-vol pricer. A new product arm plugs in by accepting &ExoticInputs and calling i.carry_rate()/discount_df()/carry_df() for drift/discount — never matching on Carry directly.

### 51. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.ExoticInputs.carry_df`

ExoticInputs::carry_df() is a pure accessor computing the yield/foreign discount factor e^{-q*t} as exp(-self.carry.yield_rate() * self.t) — no side effects. Crucially it reads the yield rate q through Carry::yield_rate(), which for Carry::FxRates returns the STORED r_for verbatim; it must NEVER reconstruct q as discount_rate() - carry_rate() (r_dom - (r_dom - r_for) does not round-trip bit-for-bit). This keeps the foreign-leg discount e^{-r_for*t} byte-identical to the FX two-rate form on the exotics carry seam (ADR-0008 FX bit-identity).

### 52. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.cholesky`, `github.com-soarsa-celnet.crates.celnet-exotics.src.multiasset.price_basket`

price_basket prices a multi-asset basket option via scrambled Sobol QMC with Brownian bridge construction and Cholesky correlation. It runs cfg.replications independent scrambles (each a fresh SobolSequence::stream seed via derive_seed) of cfg.budget paths. Across-leg correlation is applied per step by Cholesky::apply to independent per-step normals before passing to bridge.build — this propagates ρ through the full path covariance. The inter-replication variance of rep_means is the std_error denominator, requiring ≥2 scrambles. Rejects non-PSD correlation matrices (cholesky returns CorrelationError). Scratch buffers (u, z_step, x_step, z_corr, path, w_terminal) are reused across paths — zero per-path allocation in the hot loop beyond the Sobol point.

### 53. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.mem.PayloadCell<T>.read`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.received`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.skipped`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.try_recv`

Consumer::try_recv implements the seqlock reader protocol with exact conflation (skip) accounting. On each call it: (1) loads `head` with Acquire; if `cursor >= head` returns `Empty`. (2) Computes `oldest_live = head - capacity`; if `cursor < oldest_live` fast-forwards cursor and accumulates `self.skipped += gap`. (3) Enters a spin loop: loads `stamp_before` (Acquire), checks it equals `want = seq << 1`; if not, re-derives the frontier and either laps forward, returns Empty, or spin-retries a transient mid-write. On a matching stamp, copies the payload via `slot.value.read()`, issues an `Acquire` fence (to prevent the aarch64 payload-load/stamp-recheck reorder), then re-checks `stamp_after`; a mismatch discards the copy and retries. On a clean double-stamp match, increments `cursor` and `self.received` and returns `Ok(value)`. The `skipped` counter records exactly the number of items conflated since construction.

### 54. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.mem.PayloadCell<T>.write`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Inner`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Producer<T>.publish`

Producer::publish uses a true two-stamp seqlock writer protocol: (1) store `writing = (seq << 1) | 1` with Release to raise the in-progress flag, (2) write the payload into the slot's UnsafeCell via PayloadCell::write, (3) store `stable = seq << 1` with Release to close the write window, (4) advance the global `head` (AtomicU64, cache-padded) with Release as the last step. The two Release stamp stores straddle the payload write so any concurrent consumer copy that overlaps the write observes either the odd in-progress stamp or a before/after stamp mismatch, and retries. Single-producer ownership makes the UnsafeCell write safe with no locks.

### 55. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing<T>.consumer`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing<T>.into_producer`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing<T>.new`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing<T>.producer`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.try_recv`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Producer<T>.subscribe_from_head`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Producer<T>.subscribe_from_start`

celnet-fanout provides a lock-free, allocation-free, single-producer / multi-consumer (SPMC) broadcast ring for `T: Copy` values. The public contract is: `BroadcastRing::new(capacity)` allocates once (rounding capacity up to the next power-of-two, minimum 2); `BroadcastRing::producer()` / `into_producer()` vends the unique `Producer`; `Producer::subscribe_from_start()` / `subscribe_from_head()` vend arbitrarily many independent `Consumer` handles (cheaply cloneable via `Arc` ref-count). After construction, the hot path (`publish` + `try_recv`) is zero-alloc and lock-free for all consumers. A new downstream arm plugs in by calling `producer.subscribe_from_start()` (to replay buffered history) or `subscribe_from_head()` (to start from 'now'), then polling `consumer.try_recv()` in its own thread.

### 56. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.BroadcastRing<T>.new`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Inner`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Inner<T>.new`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.UNWRITTEN`

BroadcastRing::new rounds the requested capacity up to the nearest power of two (minimum 2) via `capacity.next_power_of_two().max(2)`. Inner::new asserts `capacity.is_power_of_two()` and stores `mask = capacity - 1` for branchless slot index computation `idx = seq & mask`. All slots are pre-allocated in a `Box<[Slot<T>]>` with stamps initialized to `UNWRITTEN` (a sentinel that no valid even stamp can equal). The cache-padded `head: CachePadded<AtomicU64>` is initialized to 0 and is the single shared coordination point between producer and all consumers.

### 57. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.clone`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Producer<T>.subscribe_from_head`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Producer<T>.subscribe_from_start`

Producer::subscribe_from_start creates a Consumer with `cursor = 0`, enabling replay of all still-buffered history up to the ring's capacity. Producer::subscribe_from_head creates a Consumer with `cursor = head` (loaded with Acquire), skipping all already-published items and starting from the next publish. Both constructors share the same `Arc<Inner<T>>` reference without any allocation of new ring storage. Consumer::clone is also provided (`in_degree=293`), cloning the Arc reference and copying the current cursor and counters — enabling fan-out to N independent downstream consumers from a single subscription point.

### 58. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.try_recv`, `github.com-soarsa-celnet.crates.celnet-fanout.src.ring.Consumer<T>.try_recv_batch`

Consumer::try_recv_batch drains up to `out.len()` items from the ring into a caller-supplied `&mut [T]` slice in a single call, returning the number of items written. It delegates entirely to repeated `try_recv()` calls and stops on the first `RecvError::Empty`. The caller owns the output buffer; no allocation occurs inside the method. This is the preferred batch-drain API for throughput-sensitive consumers (e.g. the FX price-fanout path).

### 59. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.acceptor.Acceptor<S, Q>.run`, `github.com-soarsa-celnet.crates.celnet-fix.src.dialect_fx.decode_strategy`, `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.validate`, `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameCursor<'a>.parse`, `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameEncoder.finish`, `github.com-soarsa-celnet.crates.celnet-fix.src.initiator.Initiator<S>.request_and_lift`, `github.com-soarsa-celnet.crates.celnet-fix.src.session.Session<S>.on_inbound`, `github.com-soarsa-celnet.crates.celnet-fix.src.session.Session<S>.send_app`

CAPABILITY SUMMARY — celnet-fix is the FIX 4.4 session + FX-options dialect adapter. Its public contract has four layers: (1) Framing — FrameCursor::parse validates a raw byte slice as a well-formed FIX 4.4 frame (BeginString, BodyLength, CheckSum, MsgType present), returning a zero-copy cursor; FrameEncoder::finish assembles 8=/9=/body/10= and stamps the checksum; (2) Dictionary — validate(frame) checks required tags per MsgType and PossDupFlag conditional presence, returning MsgType or DictError; (3) Session — Session<S>::on_inbound dispatches admin (Logon/Logout/TestRequest/ResendRequest/SequenceReset) and delivers app messages (QuoteRequest/Quote/MassQuote/NewOrderSingle/ExecutionReport) through a SessionAction with seq-gap detection and FileStore/InMemoryStore persistence; Session::send_app stamps outbound sequence and stores for replay; (4) FX dialect — decode_strategy/decode_option decode multi-leg/single-leg option strategies from FrameCursor; inputs_for+price_leg bridge OptionDescriptor+MarketSnapshot into VanillaInputs for the pricer. Initiator::request_and_lift / Acceptor::run are the async TCP entry points. A new LP adapter adds a QuoteSource impl; a new message type adds a build_* function and a dictionary entry; the session layer is not modified.

### 60. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.dialect_fx.decode_strategy`

decode_strategy(frame, leg_tenor) decodes a FIX 4.4 NewOrderMultileg/QuoteRequest body into a StrategyPackage: it reads NoLegs(555) to learn the leg count, then walks every field in the frame treating LegSymbol(600) as a leg-group delimiter (FIX repeating-group convention without a dedicated group parser). Each LegBuilder accumulates strike, put/call, ratio, side, and other leg fields; build() calls leg_tenor(i) to resolve the tenor. Rejects if declared count ≠ actual legs built, or any leg has an invalid field. The leg_tenor callback is the caller's hook for resolving tenors from out-of-band context (e.g. the QuoteRequestView tenor fields).

### 61. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.dictionary.validate`

dictionary::validate is the FIX 4.4 message-content gate: given a parsed FrameCursor it (1) resolves MsgType from tag 35 bytes via MsgType::from_bytes, rejecting unknown types; (2) checks that every tag in required_tags(mt) is present (skipping tag 35, already guaranteed by the framer); (3) enforces the conditional PossDupFlag rule: PossDupFlag(43)=Y requires OrigSendingTime(122) to be present; (4) iterates all fields and for each known tag runs type_ok(spec.field_type, value) — unknown/custom tags are tolerated per the single-contract guardrail. Returns MsgType on success, DictError::UnknownMsgType / MissingRequired{tag} / BadType{tag,field_type} on failure.

### 62. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.initiator.Initiator<S>.request_and_lift`

Initiator::request_and_lift is the async FIX RFQ client protocol driver: over an AsyncRead+AsyncWrite stream it (1) sends a Logon and awaits the mirror Logon before proceeding; (2) sends the caller-supplied QuoteRequest via Session::send_app; (3) on receipt of a Quote, records bid/offer/quote_id into InitiatorResult; under LiftOffer/HitBid policy it immediately sends a NewOrderSingle; (4) on receipt of ExecutionReport while awaiting_exec, records fill result and exits. LiftPolicy::Observe exits after the quote without sending an order. All outbound frames are sequence-stamped by the underlying Session.

### 63. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.session.Session<S>.on_inbound`

Session<S>::on_inbound is the FIX session state machine: it (1) calls FrameCursor::parse then dictionary::validate, propagating typed errors; (2) enforces CompID cross-check (tag 49 inbound = configured target, tag 56 inbound = configured sender) rejecting mismatches; (3) processes Logon/SequenceReset/Logout unconditionally outside seq-gap logic; (4) for ordinary messages detects a gap (seq > expected_in) and emits a ResendRequest(7=expected_in, 16=0) without advancing, or silently drops duplicates (seq < expected_in); (5) delivers QuoteRequest/Quote/MassQuote/QuoteCancel/NewOrderSingle/NewOrderMultileg/ExecutionReport to the caller via SessionAction::deliver; (6) handles TestRequest by echoing a Heartbeat with the TestReqID, and ResendRequest by replaying stored app messages gap-filling admin gaps. Returns SessionAction (outbound frames + optional delivered MsgType).

### 64. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.transport.FrameReader<R>.next_frame`

FrameReader::next_frame is the async TCP ingress gate: it reads from the inner AsyncRead in 4096-byte chunks into an internal Vec<u8>, searches for a complete FIX frame boundary (10=xxx<SOH> trailer), and enforces a hard MAX_FIX_MESSAGE_BYTES inbound size cap before extending the buffer. Exceeding the cap returns ErrorKind::InvalidData (tearing down the connection in the Acceptor run-loop). EOF mid-frame returns UnexpectedEof; clean EOF on an empty buffer returns Ok(None). A complete frame is drained from the buffer and returned as Vec<u8> ready for FrameCursor::parse.

### 65. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.accumulator_client_pv_mc`

`accumulator_client_pv_mc` prices a barrier-knock-out accumulator from the client's perspective under two monitoring regimes. Under `Discrete` monitoring each fixing is a hard knock-out: if S_k ≥ barrier the path terminates. Under `Continuous` monitoring a Brownian-bridge survival probability `1 − exp(−2·ln(B/S_{k−1})·ln(B/S_k) / (σ²dt))` is accumulated per segment, weighting each coupon. The coupon at each fixing is `S_k − pivot` (above pivot) or `−leverage·(pivot − S_k)` (below), scaled by notional and per-fixing discount factor.

### 66. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.asian_arithmetic_mc`

`asian_arithmetic_mc` prices an arithmetic-average-rate option via antithetic-variate Monte Carlo with SplitMix64 / Welford online accumulation. Each antithetic pair shares the same normal draw vector z but simulates S under +z and −z, halving variance relative to a plain MC. The path is simulated in log-space (ln S_k = ln S_{k−1} + drift + σ√dt·±z_k) for numerical stability. Returns an `McEstimate{price, std_error}` with std_error computed by the Welford online algorithm.

### 67. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.basket_mc`

`basket_mc` prices basket, best-of, and worst-of options via antithetic-variate MC with Cholesky-correlated log-normal legs. Correlated normals are produced by multiplying the iid normal vector z by the lower-triangular Cholesky factor L (corr_z = L·z), one lower-triangular inner product per leg per path. The three aggregation modes (Basket: weighted sum, BestOf: max, WorstOf: min) share the same path simulation loop, selected by the `BasketKind` discriminant.

### 68. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cliquet_clamped_mc`

`cliquet_clamped_mc` prices a capped/floored cliquet via antithetic MC with per-period local floors/caps and optional global floor/cap applied to the accumulated sum. Each period return is `max(cp·(S_k/S_{k−1} − moneyness), 0)` clamped by `[local_floor, local_cap]`; the sum is then clamped by `[global_floor, global_cap]` before discounting. The opening-spot scaling weights each leg's return by the period's opening spot, matching the plain-strip convention of `cliquet_plain_price`.

### 69. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.lookback_discrete_mc`

`lookback_discrete_mc` prices floating and fixed lookback options using Brownian-bridge extremum simulation (Beaglehole–Dybvig–Zhou 1997 / Glasserman 2003): for each sub-interval [t_{k−1}, t_k] a uniform U is drawn alongside the normal step, and the segment extremum is `exp(½·(ln_prev+ln_next ± √((ln_next−ln_prev)² − 2σ²dt·ln U)))`. This eliminates the O(1/√n) under-sampling bias of a naive node-max. Floating payoff = S_T − S_min (call) / S_max − S_T (put); fixed payoff = (S_max − K)⁺ / (K − S_min)⁺.

### 70. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.pivot_tra_bank_pv_mc`

`pivot_tra_bank_pv_mc` prices a pivot-TARF from the bank's perspective: at each fixing the intrinsic `gain_sign·(S_k − strike)` is scaled by leverage when the spot is on the unfavourable side of the pivot. When cumulative client gain reaches the target the path terminates; under `TarfRedemption::CappedGain` only the remaining target is settled, under `FullGain` the full coupon is paid. The bank PV sign convention is: client-gain legs subtract from bank_pv, client-loss legs add.

### 71. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.backend.PathSpec.gbm`, `github.com-soarsa-celnet.crates.celnet-gpu.src.gpu.GpuBackend.dispatch`, `github.com-soarsa-celnet.crates.celnet-gpu.src.gpu.GpuBackend.gpu_context`, `github.com-soarsa-celnet.crates.celnet-gpu.src.lib.public_surface_composes`, `github.com-soarsa-celnet.crates.celnet-gpu.src.pathwise.PathwiseGreeksPricer.estimate`, `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.ScenarioPricer.dispatch`

CAPABILITY SUMMARY — celnet-gpu is the wgpu-backed GPU/CPU Monte Carlo pricing substrate. Its public contract is: (1) PathSpec::gbm(spot,vol,t,r_dom,r_for,paths,steps,seed) constructs a GBM path spec with drift = r_dom − r_for; (2) GpuBackend::price_vanilla(&spec, &payoff) dispatches one workgroup-256 compute pass per path, reads back two f32 partials (sum, sum_sq) per workgroup via a bounded-poll GPU_DISPATCH_TIMEOUT, widens to f64 via pairwise_sum, and returns a Reduction (price = discount * sum/paths, variance); (3) ScenarioPricer prices a full spot×vol Cartesian ScenarioAxes grid in a single dispatch (workgroups = x_blocks × total_nodes) using common random numbers; (4) PathwiseGreeksPricer::estimate returns a GreeksEstimate with five simultaneous estimators (call_price, pathwise_delta, pathwise_vega, digital_price, lr_digital_delta); (5) GpuBackend::gpu_context() returns None when no wgpu adapter is available, triggering automatic transparent CPU fallback. New pricing arms plug in by constructing a PathSpec and calling the relevant Pricer — the GPU dispatch path is not modified.

### 72. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.gpu.GpuBackend.label`

ADR (GPU abstraction = wgpu baseline + CPU-SIMD fallback, CUDA optional) — the wgpu side. `GpuBackend::label(&self)` is a pure accessor (reads only &self, mutates nothing; its only allocation is the returned String): it reports the live wgpu adapter identity — `self.context.backend_name` when a GPU context was acquired — and otherwise falls back to "<cpu label> (fallback)" by delegating to the inner CpuBackend. It is the wgpu-side twin of CpuBackend::label (cl_555e1a46705ef9aa) within the shared `PricingBackend` trait: both implement the same simulate_paths/reduce_payoff/price_vanilla contract behind one interface, with Philox path index i fixed across backends so results reconcile modulo the f32/f64 element type. DECISION/RATIONALE: the GPU strategy is wgpu (Metal/Vulkan/DX12) as the open, permissively-licensed baseline chosen OVER a CubeCL/CUDA-first design — wgpu keeps the runtime dependency set fully open-source and portable across the M4/Metal dev box and Linux/Vulkan CI, with an optional CUDA backend behind the same trait and the f64 CPU path always available as oracle and fallback. label() encodes exactly that runtime selection: a concrete adapter name when wgpu binds a device, the "(fallback)" CPU identity when none is present — so the same single-trait code runs portably whether or not a GPU adapter exists. (Guardrail: no commercial products; open GPU stack with wgpu first-class. Metal lacks f64, so cross-backend agreement is asserted to f32 tolerance, never bit-identity.)

### 73. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.greeks.pathwise_lr_greeks`, `github.com-soarsa-celnet.crates.celnet-gpu.src.pathwise.PathwiseGreeksPricer.estimate`, `github.com-soarsa-celnet.crates.celnet-gpu.src.pathwise.pathwise_greeks_match_analytic`

pathwise_lr_greeks (WGSL compute shader, greeks.wgsl @workgroup_size(256)) computes five per-path estimators in a single GBM terminal-value draw: (0) call payoff max(S_T−K,0); (1) pathwise delta = 1_{S_T>K}·S_T/S_0 (d(payoff)/d(spot) via chain rule); (2) pathwise vega = 1_{S_T>K}·S_T·(√T·z − σT) (d(payoff)/d(σ)); (3) digital payoff 1_{S_T>K}; (4) likelihood-ratio delta of digital = 1_{S_T>K}·z/(S_0·σ√T). The Sobol draw is `z = inv_norm_cdf(u32_to_open_unit(sobol_coord(sobol_base+p)))`. PathwiseGreeksPricer::estimate averages each column (discount-adjusted) via pairwise_sum, returning a GreeksEstimate. The test `pathwise_greeks_match_analytic` validates delta and vega against the closed-form GBM sensitivities.

### 74. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.path.asian_payoff`, `github.com-soarsa-celnet.crates.celnet-gpu.src.path.path_asian`

asian_payoff (WGSL, path.wgsl) prices an Asian (arithmetic-average) option on the GPU using Brownian-bridge reconstruction: it draws m Sobol coordinates (dimension j=0..m), maps each to a standard normal z[j] via inv_norm_cdf, then reconstructs the Brownian path via W(t_l) = sum_k bridge_a[l*m+k]*z[k] (a linear map uploaded from celnet-qmc's bridge matrix A). The GBM log-price at monitoring date t_l is ln_spot + (drift − 0.5σ²)t_l + σ·W(t_l). The arithmetic average of exp(ln_S) over l=0..m is then used in the payoff max(sign*(avg−strike), 0). The bridge matrix A is the Brownian-bridge factor that concentrates low-discrepancy variation in early (high-importance) Sobol dimensions.

### 75. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.ScenarioPricer.cpu_grid`, `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.gpu_grid_reconciles_with_cpu_oracle`, `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.grid_is_monotone_in_spot_under_crn`, `github.com-soarsa-celnet.crates.celnet-gpu.src.scenario.mc_scenario`

mc_scenario (WGSL compute shader, scenario.wgsl @workgroup_size(256)) prices a full spot×vol scenario grid in a single GPU dispatch using common random numbers (CRN): each path p draws one shared normal z via counter_normal(p, 0, 0) (Philox CBRNG keyed by path index alone), then each grid node (i,j) applies its shocked GBM parameters (spot = base_spot*spot_mult[i], vol = base_vol+vol_bump[j]) to that same z, so prices across the grid are coupled by CRN variance reduction. The workgroup performs a two-level parallel reduction (payoff + payoff² into shared scratch arrays, then a halving stride loop, writing (sum, sum_sq) per workgroup-block per node to group_sums). The CPU oracle ScenarioPricer::cpu_grid replicates this exactly — drawing normals once and iterating over nodes — verified by `gpu_grid_reconciles_with_cpu_oracle` and `grid_is_monotone_in_spot_under_crn`.

### 76. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.HestonParams`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.MarketInputs`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr_madan`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.cos`

CAPABILITY SUMMARY. celnet-heston is the self-contained Heston stochastic-volatility pricer: it exposes two independent, cross-validated closed-form pricing routes — `cos` (Fang–Oosterlee 2008 cosine-series) and `carr_madan` (Carr–Madan damped-integrand FFT-style) — both accepting `(OptionType, &MarketInputs, &HestonParams) -> f64`. The crate owns its own minimal `Complex` arithmetic (no external num-complex dep), a 16-point Gauss–Legendre panel integrator, and the overflow-stable `char_exponent` CF kernel. A new caller plugs in by constructing `MarketInputs::new(spot, strike, t, r_dom, r_for)` and `HestonParams::new(kappa, theta, vol_of_vol, rho, v0)` then calling either pricer; puts and calls are both returned via exact put–call parity so neither route has a separate put path. The two pricers cross-validate each other in the `carr_madan_and_cos_agree` and `put_call_parity_internal` tests shipped in the crate.

### 77. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.aggregate.blend`, `github.com-soarsa-celnet.crates.celnet-integration.src.egress.EgressGovernor.drain_to`, `github.com-soarsa-celnet.crates.celnet-integration.src.lib.pipeline`, `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.normalize`, `github.com-soarsa-celnet.crates.celnet-integration.src.subscriber.ResilientSubscriber.ingest_frame`

celnet-integration is the vendor-feed ingestion seam: it provides a linear pipeline (parse JSON vendor message → normalize to NormalizedSlice → multi-source blend to BlendedSlice) exposed as the `pipeline` function, plus stateful components (ResilientSubscriber, EgressGovernor) for live feed subscription and downstream price delivery. A new feed arm plugs in by producing VendorSmileMessage JSON and supplying its domestic rate; the normalize/blend/egress stages are shared and convention-validated for all arms. The three deployment modes (Standalone, Edge, Relay) are encapsulated behind EdgeBuilder.

### 78. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.divergence.divergence_report`, `github.com-soarsa-celnet.crates.celnet-integration.src.divergence.median`

divergence_report() flags a feed source as divergent only when (a) there are at least MIN_SOURCES_FOR_GATING sources (gating_undecidable=false), (b) the source's L∞ deviation over the smile pillar vols exceeds the absolute tolerance_vol_points threshold, AND (c) it exceeds the data-adaptive MAD-scaled bound MAD_K × MAD_TO_SIGMA × mad / VOL_POINT. The dual gate (absolute floor + panel-relative MAD) ensures a source is never flagged merely because the whole panel is genuinely dispersed. The consensus is the per-quantity median over all sources; a single or dual feed never triggers exclusion (gating_undecidable path).

### 79. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.egress.EgressGovernor.drain_to`, `github.com-soarsa-celnet.crates.celnet-integration.src.egress.EgressGovernor.offer`

EgressGovernor enforces key-based conflation and a token-bucket rate limit on downstream price delivery. offer() conflates same-key updates in O(1) — if a key is already pending, the newer-seq update replaces the older one (counted in EgressMetrics::conflated); if the ordered queue is at capacity, the newcomer is dropped (dropped_capacity). drain_to() dequeues and publishes at most floor(tokens) updates per call (one token per delivery), refilling via the configured drain_rate_per_sec; a sink error re-queues the failed update at the front to preserve age order without consuming the token.

### 80. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.normalize`

normalize() enforces a three-layer convention contract before accepting any vendor quote: (1) delta convention must match the resolved canonical record for the pair/tenor; (2) ATM convention must match; (3) the premium_in_foreign flag must be internally consistent with the declared delta convention (via declared_is_premium_adjusted) AND agree with the resolved premium_style. A mismatch on any dimension returns a typed NormalizeError (DeltaConventionMismatch / AtmConventionMismatch / PremiumFlagInconsistent / PremiumStyleMismatch) rather than silently accepting a mis-priced slice. The foreign rate is re-derived as r_for = r_dom − ln(F/S)/t so the forward F = S·e^{(r_dom−r_for)·t} is reproduced exactly from the feed's spot and outright.

### 81. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.subscriber.ResilientSubscriber.ingest_frame`

ResilientSubscriber::ingest_frame() enforces strict-monotone sequencing per subscription key: a Snapshot unconditionally establishes the baseline (last_seq = seq, awaiting_snapshot = false); a Delta is accepted only when seq == last_seq + 1, returns FrameOutcome::Duplicate for seq ≤ last (never delivered out of order), and triggers FrameOutcome::Gap + awaiting_snapshot=true for seq > last+1 (a hole), gating all subsequent deltas until the next Snapshot re-syncs the key. Deltas arriving before any Snapshot (awaiting_snapshot=true initial state) are also treated as Gap.

### 82. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.EventCodec`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.append`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.compact`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.open`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.replay`

celnet-journal is a durable append-only event-log crate providing crash-safe, sequence-ordered persistence for domain state machines. Its public seam is: Journal::open (create-or-recover a file, healing any torn tail on startup), Journal::append (write one CRC-framed, fsync'd record, returning its u64 sequence number), Journal::replay (iterate all durable records in sequence order via a callback closure), and Journal::compact (atomically replace the log with snapshot + residual). The EventCodec trait is the plug-in seam: a new consumer plugs in by implementing EventCodec::encode (deterministic bytes from event) and EventCodec::decode (event from bytes) then calling Journal::append/replay with the encoded form. The Journal struct carries no generics itself — the codec seam is exercised at the call site, not baked into the log type.

### 83. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.append`

Journal::append guarantees single-write atomicity and per-record durability: the entire framed record (sync word + header + payload + CRC, built by frame_record into a single buffer) is written via a single write_all call so a crash produces either nothing or a detectable torn-tail prefix, never an interleaved partial frame. `sync_data()` is called before updating in-memory state (`end_offset`, `next_sequence`), so the sequence number is committed only after the record reaches stable storage. Payloads are bounded to MAX_PAYLOAD_LEN (64 MiB); exceeding that returns `JournalError::PayloadTooLarge` before any I/O.

### 84. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.compact`

Journal::compact performs a crash-safe log replacement via a 5-step atomic protocol: (1) materialize residual records (sequence > watermark) from the live log; (2) build the fresh image in memory as frame_snapshot + residual data records; (3) write the image to a sibling `.tmp` file and fsync it; (4) atomically POSIX-rename the tmp over the live path then fsync the parent directory to make the rename durable; (5) re-open the now-compacted log to refresh cursor state. A crash after the rename but before the fsync leaves the new file durable on any filesystem that guarantees rename atomicity. A crash before the rename leaves the old log intact (the tmp is removed on next open). The watermark must not exceed `last_sequence()`; compacting an empty log or with an over-claiming watermark returns `JournalError::CorruptInterior`.

### 85. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.open`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.read_one`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.sync_parent_dir`

Journal::open provides crash-safe recovery: it scans the file front-to-back (sync-word, CRC, strict sequence monotonicity); detects a torn tail (short sync-word read, wrong sync word bytes, or body-short CRC-position) and heals it by truncating the file to `scan.good_end_offset` followed by `sync_data()`; then calls `sync_parent_dir` once to make the directory entry durable (covering first-create and any truncation). Interior corruption — an intact sync word with a complete body whose CRC fails, or a non-monotonic sequence on a CRC-valid record — is surfaced as `JournalError::CorruptInterior` rather than silently healed. After recovery the append cursor is positioned at `good_end_offset` and `next_sequence` is seeded from the scan result.

### 86. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.Journal.replay`

Journal::replay opens an independent read-only file handle (separate from the append handle) and iterates via read_one from offset 0. It yields both RecordKind::Snapshot (first, if present) and RecordKind::Data records to the callback f, in sequence order; it stops cleanly on Eof or TornTail and propagates CorruptInterior via `?`. The caller is responsible for interpreting a leading Snapshot record as the state baseline and subsequent Data records as incremental events. The replay is guaranteed byte-identical to the original append order because sequence monotonicity is enforced on every record at write time and re-validated on every read.

### 87. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.read_one`, `github.com-soarsa-celnet.crates.celnet-journal.src.lib.read_snapshot`

read_one is the core decode kernel: it discriminates torn-tail from interior corruption by the completeness of the frame body. A missing/short sync word → TornTail (no record started here). An intact 8-byte sync word followed by a complete header + payload + CRC trailer whose CRC fails → CorruptInterior (bit-rot of a fully-written record, not a torn tail). A valid CRC with a non-monotonic sequence number → CorruptInterior. An oversized length field (> MAX_PAYLOAD_LEN but ≠ SNAPSHOT_MARKER) → TornTail (corruption of the final length field, not a fully-written record). The SNAPSHOT_MARKER sentinel in the length field routes to read_snapshot, which enforces that a snapshot record is legal only at log position 0 (at_start=true) and applies the same CRC / torn-tail discrimination logic.

### 88. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.NonAdditiveExposure.from_scenarios`, `github.com-soarsa-celnet.crates.celnet-limits.src.check.pre_trade_check`, `github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitTree.set`, `github.com-soarsa-celnet.crates.celnet-limits.src.tree.ScopePath.resolve`

CAPABILITY — celnet-limits is the risk-limit enforcement crate for the celnet FX-options platform. Its public contract provides: (1) a `LimitTree` keyed by `LimitScope` that stores hard/soft `LimitSpec` caps per Greek/risk metric; (2) `pre_trade_check` for synchronous pre-trade gate logic (Accept/Warn/Reject) across the full 7-level scope hierarchy resolved by `ScopePath::resolve`; (3) `post_trade_check`/`check_scope` for post-trade monitoring and escalation; (4) `NonAdditiveExposure` for scenario-based VaR/ES/stop-loss metrics that cannot be aggregated additively. A new limit type plugs in by adding a `LimitMetric` variant and a corresponding arm in `exposure_of`.

### 89. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.NonAdditiveExposure.from_scenarios`

NonAdditiveExposure::from_scenarios computes portfolio-level VaR and Expected Shortfall by delegating to `celnet_risk_cube::Cube::node_var_es(pricer, node, scenarios, alpha)`. This is the only path that populates the `var` and `es` fields; the `stop_loss` field is always `None` from this constructor and must be set separately via `with_stop_loss`. VaR/ES are non-additive: they cannot be summed across positions, so they must always be computed at the aggregate `NodeAggregate` level, never by summing position-level contributions.

### 90. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.check_scope`, `github.com-soarsa-celnet.crates.celnet-limits.src.check.post_trade_check`

post_trade_check folds all per-scope limit checks into a `ScopeMonitor` carrying the worst `RagStatus` (Green→Amber→Red→Breach) and worst `EscalationStatus` (Clear→SoftBreach→HardBreach). The worst status is computed by `max` over all checks; any hard-breach check elevates escalation to `EscalationStatus::HardBreach`. This gives the post-trade monitoring layer a single summary signal per scope without discarding the per-limit detail.

### 91. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.pre_trade_check`

pre_trade_check walks every scope in the resolved `ScopePath` (up to 7: Trader→Book→Desk→CcyPair→Location→Entity→Firm), skipping scopes with no registered limit. For each registered limit it applies the incremental trade to a clone of the current `NodeAggregate`, calls `exposure_of` then `LimitSpec::classify`, and accumulates `LimitCheck`s. The final `PreTradeDecision` is: `Reject` if any hard breach, `Warn` if any soft breach (and no hard), otherwise `Accept`. The node_at and nonadditive_at closures are caller-supplied, so the engine is pure given those inputs.

### 92. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.tree.LimitTree.set`

LimitTree::set enforces a one-limit-per-metric-per-scope invariant via upsert: if a `LimitSpec` with the same `metric` already exists at the given scope, it is replaced in-place; otherwise it is appended. This means a second `set` call for the same (scope, metric) pair supersedes rather than accumulates, so the tree always holds at most one limit per metric per scope. The tree is an unsorted Vec of (scope, Vec<LimitSpec>) pairs; scope lookup is O(n) in the number of registered scopes.

### 93. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.forward.pv`, `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.outright`, `github.com-soarsa-celnet.crates.celnet-linear.src.swap.pv`

CAPABILITY SUMMARY — celnet-linear is the linear-products leaf of the celnet pricing platform: it prices FX outright forwards, FX swaps (two-legged near+far), and non-deliverable forwards (NDFs) against the shared carry seam. Its public contract is three modules — forward (outright PV + greeks), swap (two-leg PV + swap points), ndf (cash-settled NDF via Ndf struct + FixingSource) — all built over the single validated input type LinearInputs::outright / LinearInputs::with_far. A new instrument arm plugs in by constructing a LinearInputs (validated via LinearInputs::validate: positive notional, non-negative settle times) with the appropriate Underlying + Carry tags, then calling pv / greeks (forward), swap::pv / swap_points (swap), or Ndf::new + Ndf::pv (NDF). The crate has no I/O, no allocation in its hot path, and no state beyond the immutable inputs struct.

### 94. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.audit.AuditDrain.commit_blocking`, `github.com-soarsa-celnet.crates.celnet-observability.src.audit.AuditDrain.drain_available`, `github.com-soarsa-celnet.crates.celnet-observability.src.audit.AuditDrain.try_commit`, `github.com-soarsa-celnet.crates.celnet-observability.src.audit.audit_channel`

audit_channel constructs a matched (AuditSink, AuditDrain) pair sharing an Arc<AuditShared> for atomic counters (next_sequence, accepted). AuditDrain.try_commit is non-blocking (try_recv); AuditDrain.commit_blocking blocks until the next record or sender disconnect; AuditDrain.drain_available loops try_commit to exhaustion. All three drain variants call `account` to update highest_committed, enabling lossless gap detection.

### 95. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.audit.AuditSink.record`

AuditSink.record assigns a globally-monotone sequence number to every AuditRecord via SeqCst fetch_add on shared.next_sequence before sending it over the unbounded channel; on channel closure it returns AuditClosed (wrapping the unsent record). The SeqCst ordering guarantees that every concurrent caller sees a strictly increasing, gap-free sequence, enabling the drain to detect losses.

### 96. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.audit.audit_channel`, `github.com-soarsa-celnet.crates.celnet-observability.src.channel.HotProbe.publish`, `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.record_ns`

CAPABILITY SUMMARY — celnet-observability provides three disjoint, zero-alloc-hot-path observability pillars: (1) a bounded SPSC telemetry ring (HotProbe/TelemetryDrain) for sub-microsecond HotSample capture from the pricing hot core, (2) an unbounded audit channel (AuditSink/AuditDrain) with monotone sequence numbers for lossless compliance-grade audit trails, and (3) HdrHistogram-backed latency recorders (LatencyRecorder/LatencyByKind) with coordinated-omission correction. All three offload to the drain/consumer side; the hot producer path is lock-free and allocation-free, satisfying CLAUDE.md guardrail 11.

### 97. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.channel.HotProbe.publish`

HotProbe.publish stamps each HotSample with a monotonically wrapping sequence number (self.seq = self.seq.wrapping_add(1)), stores it into the lock-free ring via rtrb push (which never allocates), increments shared.published on success via Relaxed atomic, and increments shared.dropped on ring-full — returning a bool to the caller indicating whether the sample was accepted. It never blocks or allocates.

### 98. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.channel.TelemetryDrain.drain`

TelemetryDrain.drain processes at most `budget` HotSamples per call from the lock-free ring, calling `account` (gap-detection) on each sample's sequence number before forwarding to the sink closure, and returns the count actually drained. The budget bound makes drain safe to call from a time-critical event loop without unbounded latency.

### 99. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.logging.AuditRecord.emit`, `github.com-soarsa-celnet.crates.celnet-observability.src.logging.build_json_subscriber`, `github.com-soarsa-celnet.crates.celnet-observability.src.logging.init_json_subscriber`

build_json_subscriber constructs a tracing-subscriber JSON formatter with flattened events, RFC 3339 UTC timestamps, env-filter-driven level filtering, and optionally span context and source location — all controlled by LogConfig. init_json_subscriber installs it as the process-global default (returning SubscriberInstallError if already set). AuditRecord.emit writes a structured `tracing::info!` event at LogClass::Security level carrying request_id, idempotency_key, tenant, instrument, op, premium, and sequence fields, coupling the audit trail to the structured log stream.

### 100. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.vanilla_price_matches_closed_form`, `github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.price_and_greeks_are_bit_identical`, `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.full_greek_set_matches_finite_difference`

CAPABILITY SUMMARY — celnet-parity is the competitive-parity verification harness: a pure integration-test crate (zero public pricing surface, `#![forbid(unsafe_code)]`) whose 19 numbered rows prove the claims in docs/CAPABILITIES-VS-COMPETITION.md are continuously *true*, not merely asserted. Every row exercises the production crates (`celnet-vanilla`, `celnet-exotics`, `celnet-surface`, etc.) through their public APIs exactly as a downstream consumer would. A regression in any row makes `cargo nextest run -p celnet-parity` fail, so the competitive matrix cannot silently rot — new product arms plug in by adding a new row to the map in lib.rs and a corresponding test file under tests/. The crate carries no runtime code; its only output is a pass/fail gate over the production crates.

### 101. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.calibration.Calibration.calibrate`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmileCalibration.calibrate`

Calibration::calibrate must be deterministic (fixed targets → bit-identical model), route numerics through celnet_core::math, reject ill-posed inputs with PluginError::InvalidInput (non-positive forward/t, empty targets, all-zero weights), and return PluginError::DidNotConverge if the iteration budget is exceeded or PluginError::CalibrationFailed if it converges outside tolerance. The reference FlatSmileCalibration implementation fits a constant vol as the weighted mean vol̄ = Σwᵢvᵢ / Σwᵢ in exactly one iteration (iterations=1), returning rms_residual = √(Σwᵢ(vᵢ−v̄)²/Σwᵢ) and max_residual = max|vᵢ−v̄|. CalibrationReport carries (iterations: u32, rms_residual: f64, max_residual: f64).

### 102. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.calibration.Calibration.calibrate`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.pricing.PricingModel.price`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.registry.ModelRegistry.descriptor`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.registry.ModelRegistry.provides`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.SmileModel.check_no_arbitrage`

CAPABILITY SUMMARY — celnet-plugin-api is the user-extensibility SDK for the celnet platform. It exposes three object-safe trait seams — PricingModel, SmileModel (: Smile), and Calibration — plus a ModelRegistry trait and a WIT world (`celnet-plugin`) that structurally mirrors them. A new model plugs in by: (1) implementing PricingModel::descriptor + price (and optionally price_and_greeks) for pricing; (2) implementing SmileModel::descriptor + Smile::implied_vol for surface models; (3) implementing Calibration::descriptor + calibrate for surface fitters. The registry routes by ModelId + ModelKind via ModelRegistry::provides/descriptor/of_kind. First-party native models and wasmi-sandboxed Wasm plugins are interchangeable behind the same registry — PricingModel::price takes &CarryInputs (carry-tagged, generalized: Carry::FxRates for FX, Carry::CostOfCarry for equity/commodity) and returns PluginResult<f64>. The contract is side-effect-free and allocation-free on the hot path; determinism (same inputs → bit-identical output via celnet_core::math) is a contract obligation enforced by the replay harness.

### 103. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.FlatSmilePricer.price_and_greeks`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.pricing.PricingModel.price_and_greeks`

PricingModel::price_and_greeks has a canonical default implementation that returns PluginResult::Err(PluginError::Unsupported("model does not produce Greeks")) — models that advertise Greek support in their ModelDescriptor MUST override it. When overridden, the implementation is expected to compute price and the full carry-tagged CarryGreeks (delta_spot, delta_forward, gamma, vega, theta, rho_dom/rho_for, vanna, volga, charm, speed, zomma, color) in a single pass, sharing the d1/d2 intermediate block to avoid recomputation. FlatSmilePricer (the reference implementation) overrides price_and_greeks and computes all 13 Greeks analytically in one pass from the shared d1/d2/sqt/vsqt block.

### 104. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.SmileModel.check_no_arbitrage`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.butterfly_check`, `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.undiscounted_call`

SmileModel::check_no_arbitrage provides a model-agnostic default static (butterfly) arbitrage check: it evaluates the undiscounted Black call price C(K) = F·Φ(d₁) − K·Φ(d₂) (via undiscounted_call, where d₁ = [ln(F/K) + 0.5v²t]/(v√t)) at each strike using the model's own implied_vol for that strike, then asserts second-difference C(Kᵢ₋₁) − 2·C(Kᵢ) + C(Kᵢ₊₁) ≥ −1e-9 (tolerance via is_close) over all consecutive triples. Requires forward > 0, t > 0, at least three strictly-increasing strikes. Returns PluginError::Unsupported("butterfly arbitrage") on negative density; PluginError::InvalidInput on grid/domain failures. A model that is arbitrage-free by construction may override with a cheaper proof.

### 105. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.host.guest_store_limits`

ADR / ROADMAP §6.5 plugin trust tiers (untrusted wasm guest = default, sandboxed + fuel-metered; wasmi chosen over wasmtime; abi_stable banned). `guest_store_limits() -> StoreLimits` is the single source of the deny-by-default guest RESOURCE policy and a pure builder (param-free, no side effects, no reads of mutable state): it caps memory_size=MAX_GUEST_MEMORY_BYTES, table_elements, memories, tables, instances to fixed maxima and sets trap_on_grow_failure(true) so a guest that tries to exceed its budget TRAPS (surfaced as HostError::ResourceLimit) rather than allocating. This is the trade-off behind the wasmi-over-wasmtime decision: wasmi is a pure-Rust, no-JIT, dependency-light interpreter whose StoreLimiter + consume_fuel give bounded CPU AND bounded memory with no native codegen attack surface — the deliberate cost is interpreter throughput, accepted because untrusted third-party plugins run here while the trusted first-party path is Tier-0 native (NativeModel::price), so the hot first-party path pays nothing for the sandbox.

### 106. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.host.install_capabilities`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.host.is_granted`

install_capabilities registers exactly the five math host functions onto a wasmi Linker under the module name "celnet_math": exp, ln, sqrt, norm_pdf, norm_cdf. Each wrapper applies abi::canonicalize to both the input and the output — NaN is normalized in both directions at the host/guest boundary. is_granted is the pure allowlist predicate that checks a (module, name) pair against the GRANTED_IMPORTS static slice; it is the single enforcement point used both at load-time pre-check and at link-time.

### 107. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.model.HostModel.price`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.model.HostModel.price_and_greeks`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.insert`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.load_wasm`

CAPABILITY SUMMARY — celnet-plugin-host is the Tier-2 wasmi sandbox host for user-supplied pricing models. Its public contract has two seams: (1) ModelRegistry, which accepts native Rust models via register_native / load_wasm and routes price/price_and_greeks calls to any registered model by ModelId; (2) WasmModel, which loads and validates a .wasm blob against a strictly-enumerated capability surface (exactly five math imports: celnet_math::{exp, ln, sqrt, norm_pdf, norm_cdf}), metered per call by a FuelBudget. A new model arm plugs in by implementing the HostModel trait (descriptor + price + price_and_greeks) and calling ModelRegistry::insert or load_wasm.

### 108. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.replay.assert_agree`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.replay.replay`

replay::replay(model, snapshot, runs) is the determinism-verification harness: it calls price and price_and_greeks on the same snapshot a minimum of 2 times (runs.max(2)), and returns ReplayError::Diverged if any subsequent result is not bit-identical to the first via bits_eq / greeks_bits_eq. assert_agree(a, b, snapshot) extends this to cross-model agreement: it calls both a and b on the same snapshot and returns ReplayError::Diverged if their outputs differ bit-for-bit — used to verify that a native and Wasm twin produce identical results.

### 109. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.WasmModel.load`

WasmModel::load enforces a two-phase capability gate before any guest code runs: (1) it iterates module.imports() and returns HostError::CapabilityDenied for any import not in GRANTED_IMPORTS, naming the offending "module::name" precisely; (2) it seeds the wasmi Store with the per-call FuelBudget BEFORE instantiate_and_start so that any module (start) function is fuel-bounded (an infinite start traps as FuelExhausted, not a hang). Each subsequent price/price_and_greeks call refuels back to the full budget via WasmModel::refuel, so start-time consumption does not eat into a later call's SLA.

### 110. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.WasmModel.price`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.WasmModel.price_and_greeks`

WasmModel::price marshals CarryInputs to a flat little-endian byte buffer via input_to_bytes, calls the guest export 'price' with (opt_to_abi(opt), in_ptr, in_len), and canonicalizes the returned f64 at the host boundary — any NaN is normalized to CANONICAL_NAN_BITS. WasmModel::price_and_greeks additionally calls the guest export 'scratch_out' to obtain the output pointer, calls 'price_greeks', reads GREEKS_BYTES back via read_guest, and deserializes them with abi::greeks_from_bytes. Both paths call WasmModel::refuel before each call to reset fuel to the full FuelBudget.

### 111. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.map_instantiation_error`

ADR (plugin-host sandbox = wasmi, fuel-metered; chosen over wasmtime) — the error-taxonomy side. `map_instantiation_error(e: &wasmi::Error) -> HostError` is the pure, total classifier (reads only the borrowed wasmi error, mutates nothing; its only allocations are the returned short() strings) that lowers a wasmi LOAD/instantiate fault into the host's stable error enum: TrapCode::OutOfFuel -> FuelExhausted, TrapCode::GrowthOperationLimited -> ResourceLimit, "resource limiter denied" -> ResourceLimit, an unknown/missing import -> CapabilityDenied (deny-by-default capability surface), everything else -> InvalidModule. This mapping is the concrete embodiment of the wasmi decision: it depends on wasmi's specific as_trap_code()/TrapCode vocabulary, so the host error contract is defined in terms of the chosen interpreter's fault model — the trade-off being that swapping engines would re-write this seam, accepted because wasmi's pure-Rust trap codes give deterministic, panic-free, hang-free fault classification (no JIT codegen failure modes to model).

### 112. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.build.main`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.GetSmile`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.MarkSurface`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.Price`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.RequestQuote`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.Scenario`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.StreamSession`

CAPABILITY SUMMARY — celnet-proto is the single, unversioned wire contract for the entire Celnet platform (ADR-0007: one clean current contract, no schema_version, no N/N-1 negotiation). It exposes five logical message families — vocabulary (enums + value messages mirroring celnet_types one-to-one), instrument (a unified Instrument oneof covering vanilla, Strategy, SingleBarrier/DoubleBarrier/WindowBarrier, Digital, Touch, VarianceSwap, VolatilitySwap, AsianOption, ForwardStart, Cliquet, Quanto, Tarf, Pivot, Accumulator, Lookback, AmericanOption, BasketOption, PerpetualOption, ListedFutureOption), quote (RFQ lifecycle: QuoteRequest → Quote → QuoteAccept/QuoteReject → Execution), stream (multiplexed bidirectional RFS via StreamService::StreamSession carrying ClientStreamMessage/ServerStreamMessage), and surface (GetSmile/MarkSurface/Scenario workflows). Seven gRPC services are generated: PricingService, QuoteService, StreamService, RiskService, SurfaceService, FixAdminService, AuthService. All message types implement prost::Message; build.rs uses protox (pure-Rust compiler, no system protoc) + tonic-build with skip_protoc_run() for hermetic, reproducible code generation. New arms plug in by evolving celnet.proto and updating every dependent in the same change — zero mixed-version windows.

### 113. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.AcceptQuote`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.RejectQuote`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.RequestMultiDealerQuote`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.RequestQuote`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_multi_dealer_quote`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_rfq_lifecycle`

The RFQ lifecycle is encoded as four sequenced wire messages sharing an idempotency_key string: QuoteRequest (client idempotency_key + Instrument + Conventions + optional correlation_id/surface_version/AttributionRecord) → Quote (quote_id + TwoWayPrice + Greeks + resolved_strike + epoch_nanos + valid_until_nanos) → QuoteAccept (quote_id + idempotency_key + Side + lp_id, the lp_id field doubles as the multi-dealer LP selector) / QuoteReject (quote_id + reason) → Execution (execution_id + quote_id + traded_premium + epoch_nanos). The multi-dealer path extends Quote to MultiDealerQuote (dealers: Vec<DealerQuote> ordered best-offer-first, best_bid_lp_id, best_offer_lp_id); QuoteAccept's lp_id field carries the chosen LP.

### 114. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.GetSmile`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.MarkSurface`, `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.Scenario`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_scenario_grid`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_surface_messages`

The surface workflow encodes three request-response pairs: (1) GetSmileRequest (pair + tenor_years + conventions) → Smile (SmilePoint vector on delta axis + BrokerQuoteSet with ATM/RR/BF at 25Δ and optionally 10Δ + ArbReport with butterfly_arbitrage_free, calendar_arbitrage_free, worst_density, SmileModel tag); (2) MarkSurfaceRequest (pair + Vec<BrokerQuoteSet> + conventions + optional SmileModel) → MarkSurfaceResponse (surface_version + Vec<Smile> + epoch_nanos); (3) ScenarioRequest (Instrument + base_market + multi-axis ShockAxis grid with Factor enum: Spot/Vol/Time/RateDom/RateFor, each axis carrying relative bool + steps Vec<f64>, plus optional RiskBucketRequest for VegaBucket/CrossGamma/theta-roll decomposition) → ScenarioResponse (ScenarioPoint vector + optional BucketedRisk).

### 115. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.proto.celnet.StreamSession`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_market_series_messages`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_stream_messages`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_stream_reject_each_reason`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_tradable_token_each_side`

The multiplexed RFS (rate-for-stream) channel uses a single bidirectional gRPC stream (StreamService::StreamSession). Client-to-server messages (ClientStreamMessage oneof) are: Subscribe (subscription_id + Instrument + throttle_nanos + optional correlation_id/surface_version/AttributionRecord), Modify (re-baseline in place, no re-subscribe needed), Unsubscribe, Resync (last_sequence — server-assisted gap recovery), Execute (token + idempotency_key, click-to-trade via TradableToken), Heartbeat, and the market-series sub-family (MarketSeriesSubscribe/Unsubscribe). Server-to-client messages (ServerStreamMessage oneof) are: Snapshot (sequence=1 + full Greeks + TradableTokens), Update (sequence delta, Greeks, TradableTokens), StreamEnd (Reason::Lagged for slow consumers), StreamReject (Reason enum: Expired/UnknownToken/AlreadyConsumed for token-execution failures), Executed (click-to-trade confirmation), Heartbeat (carries telemetry: conflation_drops, server_price_p50/p99/p999_nanos, surface_version), MarketSeriesSnapshot/MarketSeriesPoint.

### 116. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.carry_fx_maps_r_for_only`, `github.com-soarsa-celnet.crates.celnet-proto.src.convert.fx_vanilla_inputs_byte_identical_round_trip`

MarketContext::fx(spot, vol, r_dom, r_for) is the ergonomic FX constructor: it sets discount_rate=r_dom and carry=CarryModel::fx(r_for) (which wraps r_for in FxRates inside the carry oneof). The accessors r_dom() and r_for() return exactly the stored bits — r_dom() returns self.discount_rate, r_for() returns carry.as_ref().and_then(CarryModel::fx_r_for).unwrap_or(0.0) — preserving bit-identical round-trips relative to the former flat-scalar form. with_spot, with_vol, with_r_dom, and with_r_for each produce a new MarketContext with exactly one field replaced, no arithmetic, making them pure bump-constructors for finite-difference Greeks.

### 117. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.listed_future_terms_validity_matrix`, `github.com-soarsa-celnet.crates.celnet-proto.src.convert.perpetual_terms_require_zero_expiry`

validate_perpetual_terms(expiry_years) enforces that a PerpetualOption's enclosing Instrument.expiry_years is exactly 0.0, rejecting NaN via the != 0.0 test (since NaN != 0.0 is true, a NaN expiry is refused rather than silently accepted as 'no expiry'). validate_listed_future_terms(option, expiry_years) enforces the term ordering future_expiry_years >= expiry_years > 0 (both finite), and also validates the future_symbol presence and the Margining enum tag. Both guards return WireError::InvalidTerms on violation — never clamping or defaulting the offending value.

### 118. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.underlying_round_trips_fx`, `github.com-soarsa-celnet.crates.celnet-proto.src.convert.underlying_round_trips_metal`

The Underlying smart constructors (Underlying::fx, ::metal, ::equity, ::commodity, ::digital_asset) each stamp the settlement_ccy field from the pair's/ref's quote (numeraire) leg at construction time — e.g. Underlying::fx clones pair.quote as settlement_ccy before wrapping the pair in underlying::Ref::Fx. This means settlement_ccy is never a separate caller concern: it is derived deterministically from the asset reference and stored once, so downstream message consumers can read settlement_ccy without re-deriving it from the pair legs.

### 119. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_american_bermudan_instruments`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_cross_asset_instruments`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_perpetual_and_listed_future_instruments`, `github.com-soarsa-celnet.crates.celnet-proto.src.lib.round_trip_window_barrier_and_pricing_model`

The Instrument oneof product field (proto field tag) encodes a 24-arm product taxonomy, all sharing the outer Instrument envelope (underlying, tenor, expiry_years, quantity, side, solve, pricing_model, settlement_style). The PricingModel selector (default=0=Default, LocalStochVol=1) travels on the same Instrument message and is silently-default-safe: assert_eq!(PricingModel::Default as i32, 0) is asserted in the round-trip tests, so an unset pricing_model is byte-identical to Default. Similarly SettlementStyle::Linear as i32 == 0, Margining::EquityStyle as i32 == 0, and ExerciseStyle::American as i32 == 0, meaning all proto3 zero-defaults have semantically meaningful names for the overwhelmingly common case.

### 120. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.new`, `github.com-soarsa-celnet.crates.celnet-qmc.src.lib.rqmc_estimate`, `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolSequence.new`, `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolSequence.stream`

CAPABILITY SUMMARY — celnet-qmc is the randomized quasi-Monte Carlo substrate for all path-dependent option pricing in celnet. Its public contract is: (1) `SobolSequence::new(dim)` builds a `dim`-dimensional scrambled Sobol sequence from the embedded Joe-Kuo direction-number table (up to MAX_DIM); (2) `BrownianBridge::new(m, t_total)` computes a bisection-order bridge plan for `m` uniform time steps; (3) `rqmc_estimate(bridge, budget, replications, base_seed, payoff)` drives the full RQMC loop — per-replication scramble seeds derived via splitmix, Sobol stream → inverse-normal → bridge build → payoff accumulation — and returns an `RqmcResult` with estimate + inter-replication standard error. A new pricing arm plugs in by passing a `FnMut(&[f64]) -> f64` payoff closure to `rqmc_estimate`; no other crate state is required.

### 121. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.compact_to`

NodeCore::compact_to durably snapshots the BookState as-of a requested boundary (clamped to last_applied so only committed+applied entries are captured). It reconstructs the state machine at the boundary by starting from the prior snapshot's state (if its last_included_index matches the current log base) and replaying the retained committed prefix up to that boundary — not from the live applied state, which may include entries above the boundary. Then it atomically: (1) writes the Snapshot to disk; (2) discards the log prefix up to and including the boundary via Log::discard_prefix. Returns None without side effects if there is nothing applied yet or if the boundary is already snapshotted.

### 122. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.install_snapshot`

NodeCore::install_snapshot applies a leader-sent snapshot in safe durable order: (1) write the snapshot to disk first; (2) reshape the durable log to the boundary (Log::install_snapshot retains a matching tail or discards the whole log); (3) only if the log actually advanced, reseed applied BookState and watermarks (last_applied, commit_index) from the snapshot. A stale install (log already at or past the boundary, or last_applied already past the boundary) is a no-op returning Ok(None). After reseeding, any retained committed tail above the boundary is immediately applied via apply_committed — making the post-install applied state identical to a fresh boot from the same snapshot.

### 123. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.applied_state`, `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.boot`, `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.boot_on`, `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.compact`, `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.propose`, `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.wait_for_commit`

celnet-replog is the replicated-log substrate for the celnet platform: it implements a self-contained Raft consensus engine (RaftNode) that replicates a BookState key→f64 price book across a cluster of N nodes. The public seam is: RaftNode::boot/boot_on (start node + background threads), RaftNode::propose (leader-only append of a BookUpdate), RaftNode::wait_for_commit (poll until a proposed index is committed), RaftNode::compact/compact_applied (snapshot + log truncation), RaftNode::applied_state/applied_bits (read the committed applied state). A new consumer arm plugs in by: (1) calling boot_on with a TcpListener, journal path, peer SocketAddrs, cluster_size, and RaftConfig; (2) proposing BookUpdate entries only from the leader (propose returns None if not leader); (3) reading the committed BookState via applied_state after wait_for_commit confirms durability.

### 124. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.RaftNode.boot_on`

RaftNode::boot_on performs snapshot-aware crash recovery in a strict three-phase order (Raft §7): (1) if a durable snapshot exists, seed BookState and watermarks (last_applied, commit_index) from it and adopt its (last_included_index, last_included_term) boundary on the Log so absolute indices over the discarded prefix remain correct; (2) replay only the retained committed tail strictly above the snapshot boundary up to the durable commit watermark; (3) entries above the watermark are NOT applied on boot — the leader re-drives their commit via AppendEntries. The commit watermark is durably persisted (PersistStore), so a restarted node knows exactly which prefix is committed and the applied state is to_bits-identical to a full-log replay.

### 125. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.handle_append_entries`

handle_append_entries implements the five-step Raft §5.3 follower protocol: (1) reject if leader term < current term; (2) adopt a higher term and revert to Follower; (3) reject on log-matching failure (matches_prev); (4) idempotently reconcile entries via Log::reconcile (skip identical prefix, truncate conflicting tail, append new tail); (5) advance commit_index to min(leader_commit, last_log_index) and durably persist the watermark. Every reply carries the follower's current term so a stale leader observing a higher term can step down.

### 126. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.log.Log.reconcile`

Log::reconcile is the idempotent Raft log-reconciliation kernel: it skips any prefix of incoming entries already present with an identical term (pure append idempotency), then on the first divergent entry (same index, different term = conflict, or beyond the end = new tail) truncates the durable tail at that absolute index (translated to a physical position as cut_index - base_index) and appends the remainder. A debug_assert enforces that the cut never falls below the snapshot boundary — a committed, snapshotted prefix is never overwritten. Returns Ok(last_index) on success.

### 127. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.state.BookState.apply`, `github.com-soarsa-celnet.crates.celnet-replog.src.state.BookUpdate`

BookState is the replicated state machine: a HashMap<u64, f64> (instrument key → price/value) that accepts BookUpdate commands via apply. BookUpdate has three variants: Set {key, value} (last-writer-wins exact store), Add {key, delta} (IEEE-754 add in log order, 0.0 if absent — deterministic because all nodes replay the same ordered log), and Remove {key} (no-op if absent). The state is serialized to/from a compact binary wire format via encode/decode, and to_bits maps each f64 value through f64::to_bits for bitwise-exact comparison across nodes.

### 128. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.wire.Message`, `github.com-soarsa-celnet.crates.celnet-replog.src.wire.read_frame_or_idle`

The wire protocol uses a 4-byte big-endian length-prefix framing (MAX_FRAME_LEN = 64 MiB). read_frame_or_idle performs a non-blocking first-byte read: a WouldBlock/TimedOut on the length prefix returns FrameRead::Idle (the peer is live but has nothing to send — used by the tick loop to distinguish idle heartbeat intervals from real I/O); any subsequent read after the first byte uses a blocking read with the prior timeout restored. Frames above MAX_FRAME_LEN are rejected with WireError::FrameTooLarge before the body is read. The Message enum carries seven variants: AppendEntries, AppendReply, RequestVote (with pre_vote flag implementing the Ongaro §9.6 Pre-Vote optimisation), VoteReply, InstallSnapshot, StatusRequest, Status.

### 129. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-rfq.src.lp_fix.FixLpAdapter.run_cycle`, `github.com-soarsa-celnet.crates.celnet-rfq.src.lp_fix.ccy_bytes`, `github.com-soarsa-celnet.crates.celnet-rfq.src.lp_fix.fix_symbol`, `github.com-soarsa-celnet.crates.celnet-rfq.src.lp_fix.push_strike`

FixLpAdapter::run_cycle encodes a Celnet RfqRequest into a FIX QuoteRequest (MsgType D) over a real TCP loopback session and lifts the response back to TwoWay. The FIX instrument block encoding is: tag 55 (Symbol) = 6-byte CCYPAIR (e.g. b"EURUSD"), tag 460 = b"4" (Product=CURRENCY), tag 167 = b"FXVO" (deliverable FX vanilla option), tag 201 = b"1" Call / b"0" Put, tag 947 (StrikeCurrency) = 3-byte quote currency bytes, tag 1194 = b"0" (European exercise). The strike tag uses push_strike which encodes strike as scaled integer + fractional decimal string. Returns Ok(None) when the LP returns a partial (bid-only or offer-only) or declines.

### 130. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.MultiDealerEngine.new`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.MultiDealerEngine.request`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.QuoteSource.request`

CAPABILITY SUMMARY — celnet-rfq is the concurrent multi-dealer RFQ engine. Its public contract is: (1) MultiDealerEngine::new(sources: Vec<Box<dyn QuoteSource>>) assembles an arbitrary panel of LP adapters behind a single async trait; (2) MultiDealerEngine::request(&self, request, deadline, now_nanos) -> Result<RankedPanel, PanelError> fans out in parallel to every source with a hard per-source timeout, drops non-responders and timeouts, applies last-look staleness filtering, ranks best-bid and best-offer independently with deterministic tie-break, and returns a RankedPanel carrying all rows + per-side winner ids. New LP adapters plug in by implementing QuoteSource::lp_id + QuoteSource::request; the engine itself is not modified.

### 131. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.cube.accumulate_fact`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.cube.group_by_orders_first_seen_and_routes_exotic_legs`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.dimension.fx_pair_group_value_is_the_documented_48bit_packing`

Cube::group_by partitions facts by any DimensionId (Trader, Book, Desk, Underlying, etc.) in first-seen order, routing each RiskFact to either the vanilla positions slice or the exotic_legs slice based on fact.measure.exotic. accumulate_fact ensures every leaf (vanilla and exotic) contributes to net_greeks and vega_ladder regardless of routing, so additive Greeks roll up correctly across mixed vanilla/exotic nodes. The fx_pair_group_value encodes the Underlying dimension key as a 48-bit big-endian ASCII packing of the 6-character currency code (e.g. EURUSD → 0x455552555344), with metal pairs (XAUUSD) packing through the same projection — group keys are stable u64 values, always < 2^48.

### 132. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.cube.accumulate_fact`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.assemble_capital`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.historical_var_es`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity_var_es`

CAPABILITY SUMMARY — celnet-risk-cube is the risk-aggregation and regulatory-capital crate. Its public seam is the `Cube` (upsert/group_by), three non-additive VaR/ES lenses (historical full-reprice, sensitivity Taylor, curvature), and the FRTB Sensitivity-Based Method assembler (`assemble_capital`). A new asset arm plugs in by: (1) implementing `CarryPricer` for repricing, (2) calling `Cube::upsert` with `RiskFact` objects, and (3) supplying `SbmParams`/`CurvatureBucket` slices to `assemble_capital`. The additive greeks layer (delta, vega ladder) and the non-additive layer (VaR, ES, curvature CVR) are kept strictly separate — additive fields roll up linearly across all assets; VaR/ES are computed per-node by quantile reduction over a shared `quantile_var_es` kernel used identically by both the historical full-reprice path and the sensitivity Taylor path.

### 133. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb_params.low_scenario_075_floor_rederived`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb_params.standard_frtb_params_match_cited_text`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb_params.vega_maturity_kernel_matches_cited_formula`

StandardFrtbParams encodes all FRTB MAR21 FX risk-weight and correlation constants with hand-typed paragraph citations and bitwise-exact assertions guarding against floating-point drift. Key values: fx_delta_rw = 0.15 (MAR21.88); fx_delta_rw_liquid = 0.15/sqrt(2) (MAR21.88 relief); fx_vega_rw = min(0.55*sqrt(40/10),1) = 1.0 (MAR21.93/.94); vega_corr_alpha = 0.01 (MAR21.94, decay kernel exp(-0.01*|T_i-T_j|/min(T_i,T_j))); fx_curvature_rw = 0.15 (MAR21.98). The Low correlation scenario scales rho as max(2rho-1, 0.75*rho) per MAR21.6(2). Any future edit to these constants must re-verify against the cited paragraphs.

### 134. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.scenario_grid.analytic_pv_grid`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.scenario_grid.gpu_pv_grid`

gpu_pv_grid / analytic_pv_grid produce a NodeScenarioGrid over a spot×vol Cartesian grid (ScenarioAxes). analytic_pv_grid uses the deterministic closed-form pricer (same GBM inputs as the GPU kernel, f32 axis values widened to f64 via f64::from to match GPU grid points exactly); gpu_pv_grid uses GBM Monte Carlo (PathSpec::gbm) with common random numbers, accumulating per-position variance conservatively as an upper-bound standard error. A non-FX/non-metal position returns None from fx_vanilla_inputs and is silently skipped in both paths — their grid kernels are named separate GPU workloads per docs/GPU-AT-SCALE-PLAN.md. on_gpu=true iff the ScenarioPricer is backed by a GPU device.

### 135. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.firm_curvature_spot`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.firm_var_es`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.firm_var_es_sensitivity`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.summing_shard_vars_overstates_firm_var`

VaR/ES and curvature-spot are NON-ADDITIVE measures that require a re-gathered firm node, not a per-shard sum. `FleetReducer::firm_var_es` calls `gather_firm_node` (which fan-ins constituent positions from all shards) then passes the resulting firm node to `Cube::node_var_es`. The `summing_shard_vars_overstates_firm_var` test explicitly asserts `firm_var < naive_shard_sum` — the diversification / sub-additivity invariant — proving that callers must NEVER compute VaR by summing per-shard VaRs. The same pattern applies to `firm_curvature_spot` (via `Cube::node_curvature_spot`) and `firm_var_es_sensitivity` (via `Cube::node_var_es_sensitivity`).

### 136. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetTopology.parse`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.ShardRiskSource.shard_ids`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.fan_out_aggregate`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.fan_out_aggregate_over`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_facts`

CAPABILITY SUMMARY — celnet-risk-fleet is the distributed risk-aggregation crate. Its public contract: (1) `partition_facts` / `partition_facts_with` route a flat `[RiskFact]` slice into a `FleetReducer` by HRW-based partition key `(pair, tenant)`, failing closed on an empty replica set; (2) `FleetReducer` exposes `fan_in_additive` (additive Greek/vega-ladder fan-in), `gather_firm_node` (constituent re-gather for non-additive measures), `firm_var_es`, `firm_var_es_sensitivity`, and `firm_curvature_spot`; (3) the `ShardRiskSource` object-safe trait + `fan_out_aggregate_over` / `fan_in_additive_over` / `gather_firm_node_over` extend the same fan-out/fan-in pattern to distributed backends (returning `FleetError::ShardUnavailable` on an unreachable shard); (4) `fan_out_aggregate` / `fan_out_aggregate_over` are the one-shot entry points that produce a `FleetAggregate` (firm node + VaR/ES + curvature-spot + shard_count). `FleetTopology::parse` selects `InProcess` vs `Distributed` at startup from a `(mode, backends)` string pair.

### 137. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.ShardRiskSource.shard_additive`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.ShardRiskSource.shard_constituents`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.ShardRiskSource.shard_ids`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.fan_in_additive_over`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.gather_firm_node_over`

The `ShardRiskSource` trait is the abstraction seam for in-process vs distributed shard access. It exposes three methods: `shard_ids` (sorted replica list — doc: 'fan-in summation order is reproducible'), `shard_additive` (cheap net-Greeks + vega-ladder path; doc: 'callers must NOT rely on `.positions`/`.leaves` being populated'), and `shard_constituents` (full position-retaining roll-up for non-additive re-gather; doc: 'heavier O(positions); call only when a non-additive measure is needed'). `fan_in_additive_over` uses `shard_additive`; `gather_firm_node_over` uses `shard_constituents`; both fold via `merge_additive` over `shard_ids()` order. A shard not found in the source returns `FleetError::ShardUnavailable`.

### 138. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize`, `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.Numeraire.from_leaves`, `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.dispatch`

CAPABILITY (crate summary): celnet-risk-normalize is the cross-asset risk normalisation layer between the raw carry-seam pricers and portfolio-level risk consumers. Its public seam is three-stage: (1) `canonicalize(pos)` / `canonicalize_with(pricer, engine, pos)` — projects any asset-class `PositionRisk` through the `CarryPricer` trait to a `CanonicalLeaf` (notional-scaled Greeks, spot-unadjusted premium-excluded delta, premium currency, price); (2) the `dispatch` / `AssetPricer` subsystem — fan-out over the `LEAVES` static array of `CarryPricer` impls (FxLeaf, EquityLeaf, CommodityLeaf, CryptoLeaf) with error-priority escalation; (3) `Numeraire::from_leaves` — converts a slice of `CanonicalLeaf`s into a single-numeraire risk summary (delta_vector, delta_numeraire, premium_numeraire, vega_numeraire) via a caller-supplied `SpotResolver`. New asset-class arms plug in by implementing `CarryPricer` and adding a leaf to `LEAVES`; the `PositionRisk.carry` / `PositionRisk.fx` constructors are the two entry points into the position type.

### 139. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.backpressure.InflightLimiter.try_admit`, `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap<'a>.route`, `github.com-soarsa-celnet.crates.celnet-router.src.replica.ReplicaSet.new`

celnet-router provides stateless, allocation-free partition-to-replica routing for the celnet fleet. Its public contract is: (1) PartitionKey::digest() produces a u64 fingerprint; (2) PartitionMap::route(key) returns a Route{replica, natural_owner, reason} following a three-tier priority — Primary (natural owner up), Standby (declared hot standby up), HrwFallback (best healthy replica by rendezvous weight) — or RouteError if no replica is reachable; (3) InflightLimiter::try_admit(replica) enforces a per-replica concurrency cap with lock-free CAS. A new arm plugs in by: constructing a ReplicaSet (duplicate-id and dangling-standby validated at construction), wrapping it in PartitionMap, and optionally pairing it with an InflightLimiter built from the same ReplicaSet.

### 140. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.backpressure.InflightLimiter.try_admit`, `github.com-soarsa-celnet.crates.celnet-router.src.backpressure.Permit.drop`

InflightLimiter::try_admit(replica) enforces a per-replica in-flight cap using a lock-free CAS loop on an AtomicU32 counter indexed by replica slot. If the current counter >= cap the request is shed immediately (Admission::Shed{replica, cap}); otherwise it atomically increments and returns Admission::Admitted(Permit). Permit::drop() decrements the counter with saturating_sub(1) via a CAS loop, preventing counter underflow on double-release. An unknown replica id fails closed (returns Shed with cap=0). The cap is never exceeded under concurrent admits.

### 141. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.replica.ReplicaSet.new`

ReplicaSet::new() enforces two structural invariants at construction time: (a) no two replicas share the same ReplicaId (returns MembershipError::DuplicateId); (b) every declared standby id references a member of the same set (returns MembershipError::UnknownStandby). These checks run once at set-build time, so all subsequent routing methods can assume a valid, self-consistent membership snapshot without re-validating.

### 142. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.main.main`, `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price_instrument`, `github.com-soarsa-celnet.crates.celnet-server.src.services.access.authorize_caller`, `github.com-soarsa-celnet.crates.celnet-server.src.ws.mod.accept_loop`

CAPABILITY SUMMARY — celnet-server is the runtime edge that owns the entire server-side lifecycle: WebSocket accept loop, gRPC service handlers (pricing, risk aggregation, quote, session, FIX admin), the multi-product pricer, and all supporting services (access control, price fan-out, click-trade, readiness drain). Its public contract is the unversioned celnet-proto gRPC + WebSocket JSON codec seam. A new product arm plugs in by adding a branch to `price_instrument` and a codec round-trip in `ws/codec.rs`; a new service plugs in by wiring into `WsServices` and calling `authorize_caller` at its entry point.

### 143. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price_instrument`

`price_instrument` is the single dispatch function for all 24 product arms (Vanilla, Strategy, SingleBarrier, DoubleBarrier, Digital, Touch, VarianceSwap, VolatilitySwap, AsianOption, ForwardStart, Cliquet, Quanto, Tarf, Pivot, Accumulator, Lookback, American, Basket, WindowBarrier, FxForward, FxSwap, Ndf, PerpetualOption, ListedFutureOption). It enforces three routing invariants before dispatch: (1) expiry guard — every product except PerpetualOption requires `expiry_years > 0` and finite; PerpetualOption requires `expiry_years == 0` exactly (validated by `validate_perpetual_terms`); (2) asset-class routing — if the underlying decodes to a cross-asset class (equity/commodity/crypto), dispatch goes to the cross-asset leaf crates and LocalStochVol is refused with `PriceError::UnsupportedModel`; (3) carry guard — the FX path refuses a non-FX (cost-of-carry) `market.carry` arm with `PriceError::Domain`. The booking-model selector routes `LocalStochVol` products through `price_instrument_lsv` before the product match.

### 144. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.access.authorize`, `github.com-soarsa-celnet.crates.celnet-server.src.services.access.authorize_caller`

`authorize_caller` is the session-aware entitlement gate layered on top of `authorize`. If the `ResolvedCaller` carries an authenticated session (`caller.user.is_some()`), it enforces role: `RequiredAuthority::Admin` requires `user.is_admin()`, otherwise returns `Err(Status::permission_denied)`; an authenticated non-admin satisfies `ReadAny`. If there is no session it delegates `ReadAny` to `authorize` (mode-gated) and always denies `Admin` in Enforce mode regardless of any entitlement principal. All decisions are emitted as `AccessAudit` records.

### 145. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.access.authorize`

`authorize` is the primitive entitlement gate: given `AccessMode`, an optional `EntitlementPrincipal`, a resource name, and a correlation id, it returns `Ok(())` if the principal is present and well-formed, or if mode is `Permissive` and principal is absent. It returns `Err(Status::unauthenticated)` if mode is `Enforce` and principal is absent (`PrincipalAbsent`), and `Err(status)` for a malformed principal. Every decision is emitted as an `AccessAudit` record regardless of outcome.

### 146. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.clicktrade.mint_two_way`

`mint_two_way` is the click-trade token issuance entry point: given a `TwoWayLine` (line_id, sequence, bid, offer), it first calls `ledger.clear_live()` to retire the prior sequence's tokens, then mints up to two `MintedToken` values — a Sell-side token bound to the bid (only if `bid > 0.0`) and a Buy-side token bound to the offer — both with `valid_until = now_nanos + validity_nanos`. Each token is registered in the `TokenLedger` before being returned. This guarantees a click always books the current quoted premium and never a stale one.

### 147. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.pricefanout.producer_loop`

`producer_loop` is the single-threaded price fan-out engine: it runs one dedicated OS thread per `PriceFanout` instance that (a) services all pending `SubscribeRequest` messages non-blocking (creating a `BroadcastRing` per new currency pair on first subscription, then handing callers a head-positioned consumer), and (b) drives all live `PairProducer` instances once per `PRODUCER_INTERVAL`. The thread parks for `PRODUCER_INTERVAL / 2` between iterations to avoid a busy-spin. When the subscribe channel disconnects the loop exits immediately. Slow consumers are not blocked: the ring conflates (overwrites) rather than back-pressuring.

### 148. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.risk.aggregate.aggregate_nodes`

`aggregate_nodes` converts a `Cube` (position risk store) into a wire-serialisable `Vec<WireRiskNode>`, either as a single firm-level aggregate (`dim == None` → `cube.firm_aggregate`) or grouped by a specific `DimensionId` (`cube.group_by`). The vega grid is constructed from the caller-supplied `pillars`. Each `NodeAggregate` is converted to wire via `node_to_wire(resolver, nonadditive)`, which collapses non-additive Greeks to the numeraire and may return `failed_precondition` if a required FX rate is missing.

### 149. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.apply_shock`

`apply_shock` produces a copy of `WireMarketContext` with one market factor shifted by `step`: Spot → `m.spot = adjust(spot)`, Vol → `m.vol = adjust(vol)`, RateDom → `m.with_r_dom(adjust(r_dom()))`, RateFor → `m.with_r_for(adjust(r_for()))`. The Time axis is a no-op in `apply_shock` — theta roll is handled by the calling scenario loop's expiry roll instead. `adjust(x)` is `x*(1+step)` for relative shocks and `x+step` for absolute shocks.

### 150. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.surface.arb_report`

`arb_report` produces a static arbitrage health report for a calibrated smile over a symmetric 9-point strike grid `forward * (0.80 + 0.05*(i-1))` with bump `h = forward * 0.01`. Butterfly-arbitrage-free is declared when `min_density >= -1e-6` AND `min_butterfly >= -1e-6`; calendar-arbitrage-free when `max_vertical_increase <= 1e-6`. The report always carries a typed `smile_model` field (the authoritative provenance enum) alongside the human-readable note string.

### 151. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.mod.accept_loop`

`accept_loop` is the WebSocket connection lifecycle entry point: it runs a `loop { listener.accept().await }` and for each successful TCP accept, clones the shared `WsServices` handle and spawns an independent Tokio task calling `serve_connection`. Transient accept errors (e.g. fd exhaustion) yield via `tokio::task::yield_now()` and retry — they do not kill the listener. The listener is only stopped when the edge shuts down and aborts the task externally.

### 152. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSlice.from_curvature`, `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSlice.is_butterfly_free`, `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSlice.is_calendar_free_with`, `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSurface.calibrate`

ExtendedSurface::calibrate(pillars) fits an eSSVI surface in two passes: (1) per-pillar butterfly-projected damped fit via `fit_pillar(theta, quotes)` — each pillar is independently fit to (theta, rho, psi=theta·phi) parameters; (2) forward calendar sweep — for i≥1 each fitted slice is replaced by `prev.project_after(cur.theta, cur.rho, cur.psi)`, enforcing calendar monotonicity. The input pillars must be strictly increasing in theta (ATM total variance) and all theta > 0. The `ExtendedSlice::from_curvature(theta, rho, phi)` constructor enforces `theta > 0`, `|rho| < 1`, `phi > 0` and stores `psi = theta·phi`. The eSSVI closed-form butterfly and calendar conditions (`is_butterfly_free`, `is_calendar_free_with`) operate on these four parameters.

### 153. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strike_quotes.admit_or_shrink`

admit_or_shrink is the strike-axis calibration guard that ensures every SVI slice admitted to the surface passes the no-arbitrage admission scan. It linearly interpolates between a flat-market baseline `(a_flat, 0, 0)` and the fitted `(a, c, d)` parameters using a scalar s ∈ [0,1], binary-searches for the largest s (up to ADMISSION_SHRINK_ITERS bisection steps) for which `well_posed_slice(...).map(passes_admission_scan)` is `Some(true)`, and returns that slice. The flat-market baseline (s=0) is admissible by construction, so the function never returns `None` due to FP pathology — only surfaces a caller error. This is the no-panic safety contract for the strike-axis front-end.

### 154. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.FixingSource.code`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.sign`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.is_premium_adjusted`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.RateSensitivities`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.new`

CAPABILITY SUMMARY — celnet-types is the single shared-types crate for the entire celnet platform. Its public contract is: (1) the cross-asset instrument taxonomy (Underlying enum covering Fx/Metal/Equity/Commodity/DigitalAsset variants); (2) the canonical vanilla-option input record VanillaInputs (spot, strike, vol, t, r_dom, r_for) with libm-backed discount-factor and forward helpers; (3) the unified Carry enum (FxRates|CostOfCarry) with carry_rate/forward_factor/discount_df accessors that every analytics leaf consumes; (4) the full Greeks struct (14 first/second/third-order sensitivities); (5) the RateSensitivities enum (Fx rho_dom/rho_for | generalized discount_rho/carry_rho); (6) OptionType (Call/Put) with sign/flip; (7) PremiumStyle (PA/PU) with is_premium_adjusted/flip_orientation; (8) FixingSource enum (6 EM fixings with canonical code() strings). New analytics arms plug into the platform by consuming these types; the crate itself has zero pricing logic.

### 155. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.discount_df`

Carry::discount_df(t) is a pure function computing the numeraire discount factor e^{-r*t} via libm::exp(-self.discount_rate() * t). It has no side effects and no state mutation — its output depends only on (self, t). Because discount_rate() reads r_dom verbatim for Carry::FxRates, the FX discount factor produced here is byte-identical to VanillaInputs::df_dom (the FX two-rate form), preserving FX bit-identity across the carry seam (ADR-0008). CarryInputs::discount_df and ExoticInputs::discount_df both delegate to this method.

### 156. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.discount_rate`

Carry::discount_rate() is a pure const accessor returning the numeraire/discount rate r used in e^{-r*t}: the stored r_dom verbatim for Carry::FxRates and the stored r verbatim for Carry::CostOfCarry. No side effects, no allocation, no Carry construction — a plain field read by match. This is the single asset-class-agnostic source of the discount rate on the carry seam; every leaf (vanilla/exotics/surface/risk and the crypto/equity/commodity leaves) reads r through here rather than matching on Carry or Underlying (ADR-0008 carry-seam architecture, docs/adr/ADR-0008-multi-asset-carry-architecture.md). For FX, discount_rate() == r_dom exactly, which underpins the FX df_dom byte-identity.

### 157. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.forward_factor`

Carry::forward_factor(t) is a pure function computing the outright forward factor e^{b*t} via libm::exp(self.carry_rate() * t), where b is the net cost-of-carry (carry_rate() == r_dom - r_for for Carry::FxRates, == b for Carry::CostOfCarry). Multiplying spot by this factor yields the forward F = S*e^{b*t}; it has no side effects and depends only on (self, t). For FX this reproduces VanillaInputs::forward bit-for-bit, the single forward-construction primitive shared by every carry-seam leaf (ADR-0008).

### 158. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.cva.compute_xva`, `github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.simulate`, `github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettingSet.net_value`, `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.survival`

CAPABILITY SUMMARY (celnet-xva): the crate computes the three standard counterparty-risk valuation adjustments — CVA (credit), DVA (debit/own-credit), and FVA (funding) — for a synthetic netting set of vanilla FX options. The pipeline is: (1) ExposureProfile::simulate drives spot under risk-neutral GBM with Sobol low-discrepancy normals, reprices each NettedTrade via celnet_vanilla::price, nets within the set, and reduces across paths to EPE(t_k)/ENE(t_k) profiles; (2) SurvivalCurve (flat or piecewise-constant hazard) supplies survival probabilities S(t) = exp(−Λ(t)); (3) compute_xva aggregates over the time grid to produce XvaResult {cva, dva, fva}. The public seam is (XvaInputs → compute_xva → XvaResult), where XvaInputs borrows an ExposureProfile plus counterparty/own SurvivalCurves and LGDs. The crate is #![forbid(unsafe_code)] and explicitly out of scope are collateral/CSA, wrong-way risk, and live credit-curve wiring — those are deploy-gated extensions.

### 159. `invariant` (draft)
Anchors: `github.com-soarsa-celnet.gui.src.components.Button`, `github.com-soarsa-celnet.gui.src.components.GreeksStrip`, `github.com-soarsa-celnet.gui.src.components.Panel`, `github.com-soarsa-celnet.gui.src.components.PriceTile`, `github.com-soarsa-celnet.gui.src.components.Sparkline`

Baseline Storybook stories exist co-located with the 5 most important exported components under gui/src/components: Button, PriceTile, Panel, Sparkline, and GreeksStrip. Each story file (*.stories.tsx) uses the Meta/StoryObj pattern from @storybook/react, references only Aurora design tokens (CSS custom properties from --bg-*, --text-*, --bid, --offer, --space-*, etc.) — never raw hex or inline color literals — and is additive (the component files themselves are unmodified). The StatusBadge.stories.tsx scaffold story pre-existed; these 5 are the new lane-components deliverable.

### 160. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.benches.aad.bump_first_order`

bump_first_order computes all five first-order Greeks (delta, vega, theta, rho_dom, rho_for) via symmetric finite difference using step sizes h_s=1e-5×spot (relative), h_v=1e-5, h_t=1e-6, h_r=1e-6. Theta is sign-flipped (desk convention: −∂V/∂T). This numerical oracle is used in the aad bench to cross-validate adjoint_greeks against central differences — the benchmark both times and validates correctness.

### 161. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.benches.iai_instructions.soft_regression_limits`

DELIVERABLE bench/iai-instruction-gate-no-limits = LANDED (backlog tracker still lists it OPEN as a Round-2 P1/S finding; reconciled against the live graph). `soft_regression_limits` is a pure (side-effect-free) builder returning the iai-callgrind regression config: it constructs Callgrind::default().soft_limits([(EventKind::Ir, SOFT_INSTRUCTION_REGRESSION_PCT), (EventKind::EstimatedCycles, SOFT_ESTIMATED_CYCLES_REGRESSION_PCT)]) with no external writes. The Round-2 gap (the iai instruction-count regression gate was structurally unable to fail because NO RegressionConfig/soft_limit/hard_limit existed — the lane always exited 0) is CLOSED: the gate now carries explicit per-EventKind percentage soft limits on instructions (Ir) and estimated cycles, applied via instruction_gate, so an instruction-count regression beyond the band now flags. SELF-INVALIDATING: removing or editing the limit construction shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.

### 162. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.lib.batch_builder_is_sane`, `github.com-soarsa-celnet.crates.celnet-bench.src.lib.representative_batch`

representative_batch() constructs a 64-strike surface-slice (BATCH_STRIKES=64) as a linear moneyness ladder spanning ±35% around the forward ([0.65, 1.35] × forward), with a symmetric quadratic-in-log-moneyness vol smile: vol = base_vol + 0.6 × (ln(K/F))². This ensures ATM, skew, and deep-wing strikes all exercise the full d1/d2 range. The batch builder is validated by batch_builder_is_sane, which asserts strictly-increasing positive strikes bracketing the forward plus finite, non-negative prices and 13 finite Greeks for every fixture.

### 163. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.lib.representative_inputs`

representative_inputs() returns a single canonical at-the-money EUR/USD-style vanilla option fixture: VanillaInputs::new(spot=1.10, strike=1.10, vol=0.095, t=0.5, r_dom=0.025, r_for=0.015). This is the exact input the hot-path benchmarks price; it is consumed by 9 callers across benches and unit tests, ensuring published benchmark numbers and test-suite numbers are the same workload.

### 164. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-bench.src.lib.sweep_inputs`

sweep_inputs() returns a SWEEP_LEN-element sequence of VanillaInputs where spot drifts ±5% (base×[0.95, 1.05]), moneyness spans [0.75, 1.25] (strike = forward × moneyness), and vol ranges [0.07, 0.13] — all linear in the index fraction. This smooth, varied sweep is the working set the coordinated-omission-aware core_load histogram runs over; its coverage of the realistic liquid parameter window is validated by sweep_inputs_is_varied_and_smooth.

### 165. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.actual_days`

`actual_days` is the signed day-count primitive underlying every `DayCount` accrual: it returns `(end - start).whole_days()` as an `i64`, so it is signed (negative when end precedes start) and counts whole days only. This signedness is what makes `year_fraction` anti-symmetric under interval reversal; it is the sole bridge from the `time::Date` calendar type into the ACT/365 and ACT/360 numerators. Pure: reads two dates, returns i64, no side effects.

### 166. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.signed_when_reversed`

`signed_when_reversed` pins the anti-symmetry of the day-count year fraction: with `DayCount::Act365Fixed`, `year_fraction(basis, 2024-01-01, 2023-01-01)` equals −1.0 (asserted via `assert_close!`, the sanctioned float comparator, never `==`). It guards that a reversed accrual interval yields the exact negative year fraction — the property exotic/vol-time accrual relies on for signed time spans — and that the 2024→2023 span is exactly 365 days over the ACT/365 denominator. Pure test: builds dates and asserts via assert_close!, mutating no external state.

### 167. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.daycount.year_fraction`

`year_fraction` is the canonical realization of the `DayCount` convention: it divides the actual day count by the basis-selected denominator — 365.0 for `DayCount::Act365Fixed`, 360.0 for `DayCount::Act360` — via an exhaustive match with no wildcard, returning a `Time`. Because the day count is signed (`actual_days` = end − start in whole days), an end strictly before start yields a negative year fraction (the reversed-interval property), so the function is anti-symmetric in (start,end) by construction. This is the one place the celnet-types `DayCount` enum becomes a numeric accrual factor; ACT/365-fixed (vol-time) and ACT/360 (money-market) are kept deliberately distinct (docs/CONVENTIONS.md). Pure: reads basis and the two dates, returns Time, no mutation.

### 168. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.expiry_for_tenor`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.imm_date`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.roll_period`

`expiry_for_tenor(pair, horizon, spot, tenor)` maps the full `Tenor` enum to an expiry `Date` with per-variant anchor rules: (1) Overnight → next business day after horizon; (2) TomNext → business day after ON expiry; (3) SpotNext → business day after spot; (4) Weeks(n) → spot + n weeks, ModifiedFollowing-adjusted; (5) Months(n)/Years(n) → spot + period via `roll_period` (end-of-month rule if spot is last business day of its month, otherwise ModifiedFollowing); (6) Imm(n) → nth third-Wednesday of the Mar/Jun/Sep/Dec cycle strictly after horizon, ModifiedFollowing-adjusted (ordinal zero returns `Err(TenorError::ImmOrdinalZero)`); (7) BrokenDate → civil date parsed from the broken-date tag, ModifiedFollowing-adjusted. All standard-ladder tenors (Weeks/Months/Years/Imm) are anchored on `spot`, not `horizon`.

### 169. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.imm_date`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.third_wednesday`

`imm_date(horizon, n) -> Date` returns the nth IMM date (third Wednesday of a quarterly Mar/Jun/Sep/Dec cycle) strictly after `horizon` by scanning forward from `first_imm_month_on_or_after(horizon.month())` in 3-month steps via `next_imm_month`, counting only candidates strictly greater than horizon. `n` is 1-based; `n == 0` is rejected upstream by `expiry_for_tenor` with `TenorError::ImmOrdinalZero`. No allocation; terminates in at most `n` quarter-cycle iterations.

### 170. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.is_t_plus_one_pair`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.spot_date`, `github.com-soarsa-celnet.crates.celnet-calendar.src.fx.spot_lag_days`

`spot_date(pair, horizon) -> Date` adds exactly `spot_lag_days(pair)` business days (T+1 for USD/CAD, USD/TRY, USD/RUB, USD/PHP; T+2 for all other pairs) to `horizon` using the pair's composite `BusinessCalendar`. `spot_lag_days` returns `1` when `is_t_plus_one_pair` matches, otherwise `2`. No side effects; deterministic over inputs.

### 171. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.cli.resolve_priced_expiry`

DELIVERABLE cli-stream-rfq-tenor-expiry-drift = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "--tenor 3M silently streams a 1Y-priced quote labelled 3M"; reconciled against the live graph). `resolve_priced_expiry(pair, tenor, explicit, horizon)` is a pure, deterministic function (reads only its borrowed args, returns Result<f64, DispatchError>, mutates nothing) that DERIVES the priced expiry-year-fraction from the requested tenor via celnet_conventions::vol_year_fraction over the pair's calendar, and — when an explicit --expiry-years is also supplied — rejects any value that drifts from the tenor-derived anchor beyond the abs/rel tolerance with DispatchError. The label and the priced expiry can no longer drift apart silently across the CLI stream/rfq seams (regression-pinned by stream_and_rfq_reject_a_contradictory_tenor_expiry_pair and priced_expiry_derives_from_tenor_via_the_conventions_calendar). SELF-INVALIDATES on any change to this resolver.

### 172. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.price.atm_forward_strike_equals_forward`

The `price` module's ATM-Forward strike resolver sets the solved strike equal to the theoretical forward price `spot * exp((r_dom - r_for) * t)` to within 1e-12: `StrikeSpec::Atm { atm: AtmConvention::AtmForward, .. }` resolves to `VanillaInputs::forward()` on the same market parameters.

### 173. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.price.delta_spec_round_trips_to_target_delta`

The delta-spec solver in the `price` module is self-inverse with tolerance 1e-9: solving a strike from a 25Δ target via `StrikeSpec::Delta { target: 0.25, convention }` and then computing the convention delta of that result recovers exactly 0.25. The resolved strike additionally matches `celnet_vanilla::strike_from_delta` called directly on the same inputs to 1e-14, confirming the CLI adds no solver indirection.

### 174. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.price.outright_matches_direct_vanilla`

The `price` module's `run` function, exercised by `outright_matches_direct_vanilla`, produces price/vega/gamma and a convention delta that are bit-for-bit identical (tolerance 1e-14) to the `celnet_vanilla::greeks` and `celnet_vanilla::convention_delta` functions called directly on the same `VanillaInputs`. This cross-check proves the CLI's market-to-inputs pipeline introduces zero numerical drift for the outright-strike case.

### 175. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.format_panel`, `github.com-soarsa-celnet.crates.celnet-cli.src.rfq.ladder_prices_round_trip_bit_for_bit`

The `format_panel` function in the `rfq` module renders RFQ panel rows using Rust's `{}` (shortest-round-trip) float formatter, guaranteeing that parsing the printed bid/offer recovers the exact `f64` bit pattern. This is verified by `ladder_prices_round_trip_bit_for_bit`: `col(row, "bid").to_bits() == (0.1_f64 + 0.2).to_bits()` and `col(row, "offer").to_bits() == 0.32_f64.to_bits()`, including the canonical `0.1 + 0.2` rounding case.

### 176. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-cli.src.surface.atm_vol_recovered_at_atm_strike`, `github.com-soarsa-celnet.crates.celnet-cli.src.surface.calibrated_slice_is_arbitrage_free`

The `surface` module's smile calibration pipeline guarantees arbitrage-freedom on a standard 25Δ broker slice: `SurfaceResult::arbitrage.is_arbitrage_free(1e-6)` must hold for any benign EUR/USD-style input (ATM=10.5%, RR25=-0.5%, BF25=0.2%). Additionally, the calibrated smile reprices the ATM vol at the ATM strike to within 1e-9 (`r.smile.implied_vol(r.atm_strike, r.forward, 1.0).0 ≈ atm_vol`), verifying the pipeline's internal consistency.

### 177. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.idempotency.splitmix64`

`splitmix64(z: u64) -> u64` is a pure bijective mixer used to expand OS-seeded entropy into idempotency-key halves. It applies three fixed multiply-xorshift rounds: `z += 0x9E3779B97F4A7C15; z = (z ^ z>>30) * 0xBF58476D1CE4E5B9; z = (z ^ z>>27) * 0x94D049BB133111EB; z ^ z>>31`. All operations are wrapping. This is the standard SplitMix64 finaliser (Vigna 2015); its output is uniformly distributed over all 64-bit values for any non-repeating input sequence.

### 178. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.rfs.is_gap`, `github.com-soarsa-celnet.crates.celnet-client.src.rfs.is_stale`

`is_gap(last_good, observed) -> bool` is a pure sequence-integrity predicate: returns `true` iff a baseline is established (`last_good != 0`) AND `observed > last_good + 1`, meaning at least one sequence number was skipped. Returns `false` before a baseline is established (no gap can be declared on the very first frame). The complementary `is_stale` returns `true` iff baseline is established AND `observed <= last_good`.

### 179. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-client.src.surface_vocab.Smile.atm_vol`, `github.com-soarsa-celnet.crates.celnet-client.src.surface_vocab.vol_at_delta_matches_within_tolerance_without_pillar_aliasing`

`Smile::atm_vol(&self) -> Option<f64>` is a pure accessor that returns the ATM volatility by querying `vol_at_delta(0.50)`. `vol_at_delta` performs exact pillar lookup with an absolute tolerance `DELTA_MATCH_ABS`: queries within that tolerance band of a pillar resolve to that pillar's vol; queries between pillars return `None` with no aliasing or fallthrough to adjacent pillars. `atm_vol` is therefore `None` when the 0.50-delta pillar is absent from the smile.

### 180. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.CommodityInputs.on_future`, `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.future_equals_spot_reparameterization`

`CommodityInputs::on_future` is the canonical smart constructor for an option priced directly on a futures price: it sets spot := future (the futures price), carry b := 0.0 (no drift on a futures price under risk-neutral measure), and r to the discount rate. The test `future_equals_spot_reparameterization` pins the equivalence: on_future(F, K, σ, t, r) produces the same price as on_spot(S, K, σ, t, r, convenience) when F = S·e^{b·t} — confirming the two constructors are equivalent reparameterizations of the same model, not two distinct models. const fn: evaluates to a struct literal at compile time, no writes/allocation/IO.

### 181. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.aux`

`aux` is the shared Black-76 precomputation kernel: given CommodityInputs it computes σ√t, the carry-adjusted forward F = S·e^{b·t} (via CommodityInputs::forward), the discount factor df = e^{−r·t} (via CommodityInputs::discount_df), and the canonical log-moneyness d1 = [ln(F/K) + ½σ²t] / (σ√t) and d2 = d1 − σ√t using libm-routed ln/sqrt for cross-platform determinism. All five pricing/Greeks functions (price, greeks, futures_style_price, futures_style_greeks, forward_delta) read exclusively from this Aux struct so the critical-path arithmetic is computed once. Pure: reads &CommodityInputs, returns Aux, no writes/allocation/IO.

### 182. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.forward_delta`, `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.forward_delta_helper_matches_strip`

`forward_delta` returns the driftless (forward) delta ∂V/∂F = df·Φ(d1) for a Call and df·(Φ(d1)−1) for a Put — the Black-76 forward-space sensitivity used as the standardised delta quote for commodity options (where hedging is via the futures contract, not the spot). It is distinct from the spot delta in `greeks` which carries the additional e^{bt} factor. The test `forward_delta_helper_matches_strip` asserts it is bitwise identical to the delta_forward field extracted from `greeks` for both EquityStyle variants. Pure: reads (OptionType, &CommodityInputs), returns f64, no writes/allocation/IO.

### 183. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures_style_is_undiscounted_and_rate_invariant`, `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.futures_style_price`

`futures_style_price` is the undiscounted Black-76 closed form for futures-style (CME daily-margined) commodity options: Call = F·Φ(d1) − K·Φ(d2), Put = K·Φ(−d2) − F·Φ(−d1), with no discount factor. The test `futures_style_is_undiscounted_and_rate_invariant` pins two invariants: (1) futures_style_price × e^{−r·t} == price bitwise (the discounted form is exactly df×undiscounted), and (2) futures_style_price is bitwise invariant to changes in r at fixed b — the discount rate is entirely absent from the formula, so the futures-style price has zero discount-rho. Pure: reads (OptionType, &CommodityInputs), returns f64, no writes/allocation/IO.

### 184. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.greeks`

`greeks` computes the full 13-field CarryGreeks for equity-style (discounted) commodity options under the cost-of-carry parameterization. Key formulas: price = df·[F·Φ(±d1) − K·Φ(±d2)]; delta_spot = e^{(b−r)t}·Φ(±d1) (chain rule ∂V/∂S = e^{bt}·∂V/∂F with ∂V/∂F = df·Φ(±d1)); gamma = e^{2bt}·df·φ(d1)/(F·σ√t); discount_rho = −t·price (∂V/∂r at fixed b, since V = e^{−rt}·[…] independent of r in forward space); carry_rho = ±t·F·df·Φ(±d1) (only F = S·e^{bt} depends on b); theta includes the pdf term df·F·φ(d1)·σ/(2√t) plus carry/financing legs. All sensitivities are cross-validated against central finite differences in `greeks_vs_finite_difference`. Pure: no writes, no allocation, no IO.

### 185. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-commodity-vanilla.src.lib.price`

CAPABILITY (commodity cross-asset leaf): celnet-commodity-vanilla::price is the Black-76 commodity/future-option pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &CommodityInputs) that discounts the forward directly (no spot carry), reconciled to Haug's published Black-76 reference and an independent QuantLib-pinned oracle. It is the commodity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.

### 186. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.lib.schedule`, `github.com-soarsa-celnet.crates.celnet-conventions.src.lib.vol_year_fraction`

`vol_year_fraction(pair, horizon, tenor) -> Result<f64, TenorError>` computes the ACT/365-fixed year fraction for volatility time: it calls `resolve(pair, tenor)` to get the convention record (which always carries `day_count_vol = DayCount::Act365Fixed` per `record_at`), builds an `FxSchedule` via `schedule`, and returns `year_fraction(Act365Fixed, sch.vol_anchor, sch.expiry)`. The result is the standard FX vol-time measure used as input to all volatility surface and pricing functions.

### 187. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_consistent`

Convention cross-field consistency invariant (docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention coupling): ConventionRecord::is_consistent is a pure total predicate asserting self.premium_style.is_premium_adjusted() == self.is_delta_premium_adjusted() — i.e. a record is consistent exactly when its premium style and its delta convention agree on premium-adjustment. const fn, no writes/allocation/IO; deterministic. Self-invalidates if either underlying mapping or this coupling changes (WRITES gate).

### 188. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_delta_forward`

Convention forward-delta axis (docs/CONVENTIONS.md DeltaConvention; short tenors quote spot delta, long tenors switch to forward/driftless delta). `ConventionRecord::is_delta_forward` is a pure total function of self.delta — returns true exactly for the two forward variants DeltaConvention::ForwardUnadjusted and DeltaConvention::ForwardPremiumAdjusted, false for the two spot variants. It is the orthogonal counterpart to is_delta_premium_adjusted: together the (is_delta_forward, is_delta_premium_adjusted) pair decomposes DeltaConvention into its two independent boolean axes (the axes premium_adjusted_of recomposes). const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 189. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_delta_premium_adjusted`

Convention premium-adjusted mapping (docs/CONVENTIONS.md DeltaConvention→premium-style): ConventionRecord::is_delta_premium_adjusted is a pure total function of self.delta — returns true exactly for the two premium-adjusted variants DeltaConvention::SpotPremiumAdjusted and DeltaConvention::ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 190. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.record.ConventionRecord.is_non_deliverable`

Settlement→deliverability mapping (docs/CONVENTIONS.md Settlement enum → celnet-types): ConventionRecord::is_non_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, false otherwise. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the Settlement variant set or this match arm changes (WRITES gate).

### 191. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_non_deliverable`

Pair-universe settlement classification (docs/CONVENTIONS.md Settlement → registry PairMeta): PairMeta::is_non_deliverable is a pure total function of self.settlement — returns true exactly for Settlement::NonDeliverable, mirroring ConventionRecord::is_non_deliverable so the registry-level and resolved-record classifications agree. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or Settlement variants change (WRITES gate).

### 192. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_precious_metal`

Pair-universe asset-class classification (docs/CONVENTIONS.md InstrumentClass → registry PairMeta): PairMeta::is_precious_metal is a pure total function of self.instrument — returns true exactly for InstrumentClass::PreciousMetal (the loco-London metal-leg pairs), false for fiat. const fn, no writes/allocation/IO; deterministic. Self-invalidates if the match arm or InstrumentClass variants change (WRITES gate).

### 193. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairMeta.is_self_consistent`

Pair-universe structural self-consistency (docs/CONVENTIONS.md PairMeta entry invariants): PairMeta::is_self_consistent is a pure total predicate over its own fields returning false on any contradiction — NDF terms present iff non-deliverable; the cash-settlement currency (when present) is one of the pair's two legs; the premium currency is one of the pair's legs and agrees with the premium-adjusted flag; and the spot lag is the canonical T+1 or T+2. No writes/allocation/IO; deterministic, reads only self. Self-invalidates if the invariant set or any field-coupling changes (WRITES gate).

### 194. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.inverted`

`PairProfile.inverted()` flips a profile for an inverted quote orientation: it swaps `day_count_accrual_for` and `day_count_accrual_dom`, calls `premium_style.flip_orientation()`, and crucially leaves `ndf` and `metal_leg` unchanged — enforcing that the NDF fixing reference + physical settlement currency and the precious-metal lease leg are intrinsic pair properties invariant under quote direction (e.g. USDKRW and KRWUSD share the same KFTC18 fixing and USD settlement).

### 195. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.PairProfile.record_at`, `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is_long_tenor`

`PairProfile.record_at(tenor)` produces a `ConventionRecord` with the vol day-count unconditionally set to `DayCount::Act365Fixed` (§1.5 of the convention spec), regardless of pair or tenor. The delta convention is derived as `premium_adjusted_of(is_long_tenor(tenor), premium_adjusted)`, so short tenors and long tenors receive different delta conventions while all other fields come directly from the profile.

### 196. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.accrual_basis`

Accrual day-count is a CURRENCY property, not a pair property (docs/CONVENTIONS.md DayCount → celnet-types::DayCount). `accrual_basis(ccy)` is a pure total function mapping each Ccy to its money-market money accrual basis: GBP, AUD and NZD accrue ACT/365-fixed (DayCount::Act365Fixed); every other currency accrues ACT/360 (DayCount::Act360). Because it keys on the single currency leg (not the pair), AUD as the foreign leg accrues ACT/365 whether the pair is a covered major (AUDUSD), a covered G10 cross (AUDJPY), or a region-default-derived uncovered cross (AUDPLN) — the single source of truth the registry's accrual_basis_is_single_source_of_truth test pins. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the currency→basis mapping changes (WRITES gate).

### 197. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize`

`canonicalize(pair) -> Option<Canonical>` performs orientation-agnostic lookup: it first tries the direct key; if not in `COVERED_PAIRS` it tries the flipped pair (base and quote swapped) and sets `Canonical.flipped = true`. Returns `None` if neither orientation is covered. This means `pair_profile` and all downstream callers are indifferent to quote orientation.

### 198. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.canonicalize`, `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair_profile`, `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.resolve`

`resolve(pair, tenor)` is the primary two-path resolution function: it calls `pair_profile(pair)` which attempts a direct-key then flipped-key lookup in the static `COVERED_PAIRS` table via `canonicalize`; on a hit it calls `profile.record_at(tenor)` with `ResolutionSource::PairProfile`; on a miss it falls back to `region_default(pair, tenor)` with `ResolutionSource::RegionDefault`. The function is pure: no I/O, no global mutation, deterministic over its inputs.

### 199. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.is_long_tenor`

The spot-vs-forward delta switch is a one-year tenor threshold (docs/CONVENTIONS.md DeltaConvention: short tenors quote spot delta, long tenors switch to forward/driftless delta). `is_long_tenor(tenor)` is a pure total predicate returning tenor_days(tenor) > 365 — STRICTLY greater, so exactly-one-year tenors (Tenor::Years(1) and Tenor::Months(12), both 365 days) classify as SHORT (spot delta) and 18M / 2Y classify as LONG (forward delta), exactly as long_tenor_threshold_is_one_year pins. This boolean is the `forward` axis fed to premium_adjusted_of in region_default. const-foldable, no writes/allocation/IO; deterministic. Self-invalidates if the threshold or tenor_days mapping changes (WRITES gate).

### 200. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.pair_meta`

`pair_meta(pair) -> Option<PairMeta>` aggregates the full convention bundle for a covered pair: it calls `pair_profile` then `premium_ccy_of` to derive the premium currency (base ccy when premium-adjusted, quote ccy otherwise), and collects `spot_lag_days`, `atm`, `premium_style`, `premium_ccy`, `premium_adjusted`, `cut`, `settlement`, `ndf`, `instrument`, and `metal_leg` into a single `PairMeta` struct. Returns `None` for uncovered pairs. With 11 callers it is the primary rich-metadata entry point.

### 201. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.premium_adjusted_of`

DeltaConvention is the cartesian product of two independent booleans (docs/CONVENTIONS.md DeltaConvention; docs/CONVENTIONS.md PremiumStyle⇔DeltaConvention). `premium_adjusted_of(forward, premium_adjusted)` is the const-fn total constructor that composes the (forward?, premium-adjusted?) flags back into the four-variant enum: (false,false)→SpotUnadjusted, (false,true)→SpotPremiumAdjusted, (true,false)→ForwardUnadjusted, (true,true)→ForwardPremiumAdjusted. The match is exhaustive over both booleans, so no combination is defaulted — it is the exact inverse of the record predicates is_delta_forward (the `forward` axis) and is_delta_premium_adjusted (the `premium_adjusted` axis). Pure: no writes/allocation/IO; deterministic. Self-invalidates if the DeltaConvention variant set or the flag→variant mapping changes (WRITES gate).

### 202. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.profile_for_canonical`

`profile_for_canonical(k)` builds a `PairProfile` from the 6-byte canonical key by (1) looking up the `PairSpec` in `COVERED_PAIRS`; (2) mapping `SpecKind::NonDeliverable(fixing)` to `NdfTerms { fixing, settlement_ccy: Ccy::USD }` (NDF pairs always settle in USD); (3) for precious metals, overriding `day_count_accrual_for` to `DayCount::Act360` (loco-London bullion basis) and attaching a `MetalLeg { metal, lease_day_count: Act360, loco_london: true }`; (4) hard-coding `AtmConvention::DeltaNeutralStraddle` for all covered pairs.

### 203. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region_default`

The fall-through (uncovered-pair) convention record is assembled deterministically from per-currency and per-region rules (docs/CONVENTIONS.md house-default conventions; ResolutionSource::RegionDefault). `region_default(pair, tenor)` is a pure total function building a ConventionRecord with: cut = Tokyo1500 for a Tokyo-region pair else NewYork1000 (via region_of); premium_style = PercentForeign; delta = premium_adjusted_of(is_long_tenor(tenor), premium_style.is_premium_adjusted()) — so the spot/forward axis follows the tenor and the premium-adjusted axis follows the premium style; atm = DeltaNeutralStraddle; day_count_vol = Act365Fixed; the foreign and domestic accrual day-counts = accrual_basis(pair.base) and accrual_basis(pair.quote) respectively (per-currency, not per-pair); settlement = Deliverable. No combination is defaulted ad hoc — every field is a documented function of (pair, tenor). const-style assembly, no writes/allocation/IO; deterministic. Self-invalidates if any of the composed mapping helpers or the default field set changes (WRITES gate).

### 204. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.region_of`

The expiry-cut region is decided by the QUOTE currency (docs/CONVENTIONS.md Cut → New York 10:00 vs Tokyo 15:00). `region_of(pair)` is a pure total function returning Region::Tokyo exactly when pair.quote == Ccy::JPY, else Region::NewYork — the JPY-region/Asian business books the Tokyo 15:00 cut, every other pair the New York 10:00 cut. It keys on the quote leg only (the JPY pairs are quoted XXXJPY), so the region/cut is a deterministic function of the pair, never of spot or tenor. const-foldable, no writes/allocation/IO. Self-invalidates if the region-selection rule or Region/Ccy variant set changes (WRITES gate).

### 205. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-conventions.src.registry.tenor_days`

tenor_days is the exhaustive nominal-horizon primitive that drives the short/long delta-convention classification (docs/CONVENTIONS.md tenor axis). It is a pure total function over the whole Tenor enum returning u32 days: the pre-spot short end Overnight | TomNext | SpotNext → 1 (and BrokenDate → 1, the conservative short default since the exact pricing axis is set later by the pricer from the resolved expiry, not here); Weeks(w) → 7·w; Months(m) → (365·m + 6)/12 (the 365/12 ≈ 30.4167 days-per-month rounded to nearest day, so Months(12) = 365 and Months(6) = 183); Years(y) → 365·y; Imm(n) → (3·n·365 + 6)/12 (≈ 3 months per IMM step). The match is exhaustive over Tenor, so no variant is defaulted, and it is consumed by is_long_tenor (>365 ⇒ forward delta). const-foldable integer arithmetic, no writes/allocation/IO; deterministic. Self-invalidates if the Tenor variant set or any per-variant day formula changes (WRITES gate).

### 206. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.CarryInputs.forward`

`CarryInputs::forward(&self) -> f64` computes the outright forward price as `spot * carry.forward_factor(t)`, where `Carry::forward_factor(t)` returns `e^{b*t}` via `libm::exp(carry_rate() * t)`. For FX this is `spot * e^{(r_dom - r_for)*t}` — the two-rate FX forward. For cost-of-carry underlyings it is `spot * e^{b*t}`. Pure: reads &self, no side effects, no WRITES. This is byte-identical to `VanillaInputs::forward` on the FX path (proved by `fx_carry_inputs_byte_identical`), making `CarryInputs` a drop-in generalization of `VanillaInputs` for cross-asset consumers.

### 207. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_carry_greeks`

`fx_carry_greeks(g: &Greeks) -> CarryGreeks` is the pure field-copy lifting function from the FX leaf's `Greeks` struct to the generalized `CarryGreeks` on the carry seam. It copies all 14 fields verbatim: price, delta_spot, delta_forward, gamma, vega, theta, vanna, volga, charm, speed, zomma, color, and packages the FX rate sensitivities as `RateSensitivities::Fx { rho_dom, rho_for }`. No arithmetic, no branch, no allocation — a structural repackaging. The byte-identity of this lift is verified by `fx_carry_greeks_lifts_byte_identically` (assert_eq! on to_bits() for every field). Pure: reads only &Greeks, no WRITES.

### 208. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_carry_inputs_byte_identical`

The carry-seam FX byte-identity is enforced by fx_carry_inputs_byte_identical: the FX arm of Carry (FxRates) lowers to VanillaInputs with forward/df_dom/df_for bit-identical (to_bits) to the native FX leaf, and the CostOfCarry arm is rejected (UnsupportedCarry). This is a pure byte-identity gate over the carry seam (ADR-0008). Supersedes a withdrawn spec:satisfies probe whose design-target sentinel did not resolve in this build.

### 209. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.compare.is_close`

Determinism rule — `is_close` is the canonical float comparator and never uses `==` semantics that would misbehave on the special cases: it returns false whenever either operand is NaN (NaN is never close to anything, including itself); it short-circuits true on bitwise `a == b`, which deliberately also makes +0.0 and -0.0 close even at zero tolerance and makes equal infinities close; it returns false for unequal infinities; otherwise it accepts when the absolute difference is within `abs` OR within `rel * max(|a|,|b|)` (a combined absolute-or-relative band). Tolerances are debug-asserted finite and non-negative. This is the single comparator behind `assert_close!`; the interface determinism rule (docs/INTERFACES.md) is that all float comparison flows through it — never a bare `==`, never an assert on a NaN payload. Pure: reads only its four f64 args, returns a bool.

### 210. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf`

Determinism rule — `norm_cdf` computes the standard-normal CDF as `0.5 * libm::erfc(-x * INV_SQRT_2)`, routing the transcendental through `rust-lang/libm` (correctly-rounded) rather than the platform libm, so the result is bit-identical across targets (the cross-platform determinism guarantee of docs/INTERFACES.md). Using the complementary error function `erfc` on `-x·1/√2` keeps the deep left tail stable (no catastrophic cancellation), which is why the tail tests pass. f64 is the CPU-canonical scalar. Pure: maps one f64 to one f64 via libm, no side effects, no state.

### 211. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf_deep_tail_matches_reference`

Determinism rule — `norm_cdf_deep_tail_matches_reference` pins `norm_cdf` against high-precision reference values deep in the left tail: Φ(−1)=0.15865525393145705, Φ(−5)=2.866515718791939e-7, Φ(−10)=7.619853024160525e-24, each via `assert_close!` with explicit rel/abs tolerances (never `==`). Because `norm_cdf` routes through `libm::erfc` (correctly-rounded), these exact-digit references encode the bit-stable, cross-platform tail behaviour; a regression that dropped the erfc routing (reintroducing catastrophic cancellation) would fail here. Pure test: evaluates norm_cdf and asserts, no mutation.

### 212. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_cdf_tail_symmetry_and_no_underflow`

Determinism rule — `norm_cdf_tail_symmetry_and_no_underflow` guards the tail-stability that the `libm::erfc` routing in `norm_cdf` buys: it checks the reflection identity Φ(−x)=1−Φ(x) at x∈{3,4,5} (the largest x where the RHS is still representable before it underflows to 0), pins Φ(−15)=3.670966199312858e-51 and Φ(−20)=2.753624118606331e-89 against high-precision references, and asserts Φ(−37)>0 (≈5.7e-300, never flushed to zero). All comparisons go through `assert_close!`, never `==`. A regression that reintroduced the cancellation-prone 1−Φ(x) form on the direct path would be caught. Pure test: evaluates norm_cdf and asserts, no mutation.

### 213. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-core.src.math.norm_pdf`

Determinism rule — `norm_pdf` computes the standard-normal density as `INV_SQRT_2PI * exp(-0.5 * x * x)` where `exp` is the crate's `libm`-backed wrapper, so the transcendental is correctly-rounded and bit-identical across platforms (docs/INTERFACES.md cross-platform determinism). The argument is symmetric in x (x·x), so norm_pdf is exactly even. f64 is the CPU-canonical type. Pure: maps one f64 to one f64, no side effects.

### 214. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.funding.funding_carry`

`funding_carry(r, funding) -> Carry` is the canonical Carry constructor for crypto: it builds `Carry::CostOfCarry { r, b: r - funding }` so the net cost-of-carry b equals the difference between the risk-free rate and the perpetual funding rate. When funding == r the carry rate is zero and the forward equals spot (zero-drift, Black-76 limit). The function is called by 14 callers — it is the standard entry point for both inverse and linear crypto inputs. Pure: returns a new `Carry` from two `f64` scalars with no WRITES, no allocation, no I/O.

### 215. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.aux`

`inverse::aux(i)` precomputes the five shared scalars for the coin-margined closed form: `sqt=√t`, `vsqt=σ√t`, `f=S·e^{b·t}` (forward), `df=e^{-r·t}` (discount factor), `s2t=σ²·t`, and the three log-moneyness distances `d1=(ln(F/K)+½σ²t)/(σ√t)`, `d2=d1-σ√t`, `d3=d1-2σ√t`, plus `es2t=e^{σ²t}`. The inverse formula uses d2 and d3 (not d1) as the CDF arguments — d3 = d1 - 2σ√t is specific to the coin-margined payoff, absent from both the FX/linear and the standard Black-Scholes aux. Pure: reads `&InverseInputs`, returns `Aux`, no WRITES.

### 216. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.convexity_sandwich_vs_linear_is_signed`

The convexity sandwich test pins a signed directional inequality between the coin-margined and linear prices: for all tested parameters, `inverse::price(Call)*S < linear::price(Call)` strictly, and `inverse::price(Put)*S > linear::price(Put)` strictly (tolerance 1e-6). This is the anti-circular guard — a naive V_lin/S₀ rescale of the linear price would produce equality at both legs, making both differences zero and failing both assertions. The test thus verifies that the inverse formula is not a trivial rescale of the linear one. Pure validator: reads from value arguments, no mutation.

### 217. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.greeks`

`inverse::greeks(opt, i)` is the complete analytic Greek strip for the coin-margined vanilla. It introduces the convexity amplitude `amp = A = (K/F)·e^{σ²t}` and `ψ₃ = A·Φ(φ·d3)`, exploiting the exact lognormal pdf identity `A·φ(d3) = φ(d2)` (because d2-d3=σ√t) so all pdf cross-terms cancel into clean closed forms. Key sensitivities: `price = φ·df·(Φ(φd2) - ψ₃)`, `delta_spot = φ·df·ψ₃/S`, `delta_forward = φ·df·ψ₃/F`, `discount_rho = -t·price`, `carry_rho = φ·df·t·ψ₃`, `vega = df·φ(d2)·√t - φ·df·2σt·ψ₃`, `gamma = df·φ(d2)·u/S - φ·df·2·ψ₃/S²` (u=1/(S·σ√t)). Returns `InverseGreeks { coin: CarryGreeks, usd_equivalent: price*S }`. Pure: reads `(OptionType, &InverseInputs)`, no WRITES, no allocation.

### 218. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.inverse.price`

CAPABILITY (crypto inverse/coin-margined leaf): celnet-crypto-vanilla::inverse::price is the inverse (coin-margined, 1/S_T payoff) crypto vanilla pricing entry — a pure, side-effect-free closed form taking (OptionType, &InverseInputs) whose value is expressed in the coin numeraire via the k/F and e^{sigma^2 t} convexity terms (norm_cdf of d2/d3), reconciled to an independent oracle with a signed convexity sandwich. It is the crypto inverse capability's projection target on the carry seam through the one contract; the sibling linear (USDT-margined) crypto path collapses to the Black-76 forward limit at zero carry. No I/O, allocation, logging, or mutation.

### 219. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.aux`, `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.price`

`linear::price(opt, i)` is the USDT/coin-quoted (linear-margined) crypto vanilla closed form: it computes `s_disc = spot·e^{-r_for·t}` and `k_disc = strike·e^{-r_dom·t}` where `(r_dom, r_for) = (r, r-b)` extracted from the Carry via `fx_equiv_rates`, then prices as Call = s_disc·Φ(d1) - k_disc·Φ(d2) and Put = k_disc·Φ(-d2) - s_disc·Φ(-d1). The `d1/d2` use the spot-space log-moneyness formula `d1 = (ln(S/K) + (r_dom - r_for + ½σ²)·t) / (σ√t)` (comment: same operation order as the FX GK leaf, so price is bit-identical to the FX leaf under matching rates). Pure: reads `(OptionType, &LinearInputs)`, returns `f64`, no WRITES, allocation, or I/O.

### 220. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.fx_equiv_rates`

`linear::fx_equiv_rates(i) -> (r_dom, r_for)` recovers the GK-equivalent domestic/foreign rate pair from the unified Carry: `r_dom = carry.discount_rate()` and `r_for = r_dom - carry.carry_rate()` (= r - b = funding rate). This is the seam that maps the crypto funding-rate carry convention onto the same two-rate spot-discounting formula as the FX GK leaf, making `linear::price` bit-identical to `celnet-vanilla::price` under matching rates (as the comment in `aux` documents). Pure: reads `&LinearInputs`, returns `(f64, f64)`, no WRITES.

### 221. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.linear.greeks`

`linear::greeks(opt, i)` is the complete analytic Greek strip for the USDT-margined (GK-equivalent) crypto vanilla. It computes price in the same spot-space order as `linear::price` so `greeks().price` is `to_bits`-identical to the standalone price call. Spot delta = e^{bt}·df·Φ(±d1); forward delta = df·Φ(±d1); gamma = e^{2bt}·df·φ(d1)/(F·σ√t); vega = df·F·√t·φ(d1); discount_rho = -t·price; carry_rho (∂V/∂b) = ±t·F·df·Φ(±d1). The theta algebra uses the identity F·φ(d1)=K·φ(d2) to collapse to a single positive pdf term `df·F·φ(d1)·σ/(2√t)`. Returns `CarryGreeks` (the same struct as FX/commodity). Pure: reads `(OptionType, &LinearInputs)`, no WRITES, no allocation.

### 222. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-crypto-vanilla.src.settlement.route_price`

CAPABILITY (carry-seam deliverable, crypto leaf reach): route_price is the pure crypto settlement-style dispatcher that reaches both crypto vanilla leaves on the shared Carry seam — SettlementStyle::Linear → linear::price (GK-funding, USDT/coin-quoted) and SettlementStyle::InverseCoin → inverse::price (inverse/coin-margined 1/S_T payoff) — selecting the leaf by settlement style and forwarding the same (spot, strike, vol, t, Carry). Pure: returns the leaf price from value/ref args with no WRITES; self-invalidates if either leaf arm gains a side effect.

### 223. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.dec_cut`

Engine-handoff Cut codec (docs/CONVENTIONS.md Cut enum; engine state serialization): dec_cut is the pure total inverse of enc_cut over the Cut discriminant — decodes byte 0→Cut::NewYork1000, 1→Cut::Tokyo1500, and any other byte to Err(HandoffError::BadDiscriminant), never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the discriminant assignment or Cut variant set changes (WRITES gate).

### 224. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-engine.src.handoff.serialize_state`

ADR-0007 (one unversioned contract — engine hot-upgrade handoff). `serialize_state` is the single, deterministic encoder of the engine's live state (MarketState + BookState) into the handoff byte image: it is a pure function of its two borrowed inputs (it reads no global/external state and mutates none — the only allocation is the returned Vec<u8>), so the same (market, book) always yields byte-identical output. The image is a fixed self-describing layout — MAGIC header, market scalars, conventions, the three smile benchmark pillars + reference forward/time (exactly the state from which MarketHedgeSmile::new reconstructs an identical smile), then the length-prefixed book — and `restore_state` is its exact inverse (round-trip proven by roundtrip_restores_identical_state / restored_state_reprices_identically). DECISION/RATIONALE: hot-upgrade carries state across a code swap through this ONE current handoff format with a MAGIC sentinel and NO schema_version field and NO N/N-1 negotiation — consistent with the platform-wide single-unversioned-contract decision (ADR-0007). An upgrade deploys a single uniform engine version: the old build serializes, the new build restores; there is no mixed-version window to negotiate, so the format evolves in place rather than versioning. (Guardrail: no versioned APIs; hot-upgradable single-version estate.)

### 225. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessMode.is_permissive`, `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.default_mode_is_enforce`

`AccessMode` defaults to `Enforce` (`AccessMode::default() == AccessMode::Enforce`), verified by the `default_mode_is_enforce` test. In `Enforce` mode `is_permissive()` returns `false`. This is the production posture — the system never silently opens access due to an absent or misconfigured principal.

### 226. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.decision.AccessReason.decision`

DELIVERABLE entitlements-trust-boundary-audit = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/M finding "entitlements trust boundary inverted — omitted principal ⇒ grant-all — and the documented per-decision audit is unimplemented"; Round-4 fixed the client-default half at 87c6f77; reconciled against the live graph). `AccessReason::decision(self)` is the pure, total, const reason→decision mapping that makes every access outcome first-class and auditable: PrincipalAsserted / PermissiveAbsent / SessionAuthenticated → Allow, and PrincipalAbsent / MalformedPrincipal / SessionInsufficientRole → Deny. A genuinely-absent or malformed principal now resolves to Deny (deny-by-default at the boundary), and each AccessReason is the per-decision audit datum emitted via the celnet-observability AuditSink — closing both halves of the finding (the inverted boundary and the missing per-decision audit; pinned by reason_determines_decision + the server entitlements_boundary::decision_records test). SELF-INVALIDATES on any change to this decision mapping.

### 227. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.admits`

SAFETY — entitlement cube-pruning admission is deny-by-default at the fact level (deliverable: entitlements-deny-by-default). EntitlementFilter::admits(fact) is a pure, deterministic predicate that delegates to the deny-first Principal::admits(hierarchy, &fact.key): it returns false on any matching deny rule (deny wins over grant) and otherwise requires an explicit grant whose every scope covers the fact — so an un-granted fact is never admitted. It reads only &self (borrowed principal + hierarchy) and the fact key; no mutation, I/O, or allocation. This is the exact per-fact gate that entitled_cube / prune apply when projecting a risk cube to a principal, so an information-barrier breach cannot leak a fact the principal was not explicitly granted. Self-invalidates if admits stops delegating to the deny-first principal evaluation.

### 228. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.entitled_cube`

`EntitlementFilter::entitled_cube` builds an entitlement-gated `Cube` in a single pass: it constructs a `Cube::with_hierarchy(self.hierarchy.clone())` then upserts each fact that passes `self.admits(&fact)`. Facts that do not pass the principal are silently dropped; no placeholder rows are inserted. The result is the minimum-information cube visible to the principal — guaranteed to contain no data from denied scopes.

### 229. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.filter.EntitlementFilter<'a>.prune`

`EntitlementFilter::prune` is a pure filter over a `RiskFact` slice: it returns `facts.iter().filter(|f| self.admits(f)).cloned().collect()`. The output length is always <= input length and the relative order of surviving facts is preserved (stable filter, no sort). This is the canonical synchronous entitlement gate used in 6 call-sites.

### 230. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.principal.Principal.admits`

Entitlement visibility is deny-by-default and information-barrier-first: `Principal::admits` evaluates the deny rules before anything else and returns false on any deny match (deny wins over grant), then admits unconditionally only for the grant-all principal (`all == true`), and otherwise requires at least one grant rule to cover the fact. A non-grant-all principal with no matching grant admits nothing. The decision is a pure read over `denies`/`grants`/`all` and the supplied hierarchy+key; it mutates no state.

### 231. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Rule.covers`

A single entitlement `Rule` covers a fact only if EVERY one of its scopes covers it (`scopes.iter().all`) — scope conjunction, so adding a scope narrows a rule, never widens it. The match is a pure read over the rule's scopes against the hierarchy and fact key; no state is mutated.

### 232. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.Scope.covers`

A single `Scope` covers a fact iff the fact's resolved group value on the scope's dimension equals the scope's value (`resolved_group_value(hierarchy, key, dimension) == value`) — an exact equality on one hierarchy dimension, the atom from which `Rule::covers` (scope-conjunction) and `Principal::admits` are built. Pure: it only reads the hierarchy and fact key.

### 233. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-entitlements.src.scope.resolved_group_value`

Entitlements deny-by-default scope-resolution safety (deliverable: entitlements-deny-by-default). resolved_group_value is a pure, deterministic resolver: given (&Hierarchy, &FactKey, DimensionId) it maps Desk/Entity through the hierarchy (falling back to the key's own group value when no parent edge exists) and passes every other dimension straight through key.group_value, with no I/O and no mutation of its borrowed inputs. Determinism is the safety property — a grant's scope.covers test resolves a fact to exactly the same governing node on every evaluation, so a principal can never be admitted to a scope the deny-by-default rule did not actually grant. Pure (no WRITES edges); self-invalidates on change.

### 234. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.carry`

EquityInputs::carry() is the pure, side-effect-free accessor for the net cost of carry b = r − q − repo: it reads the three stored fields and returns their scalar difference. This scalar b is the single carry coordinate threading through d1, the forward (S·e^{b·t}), and all carry-tagged Greeks. By convention repo=0.0 for an unencumbered name, which collapses b to r−q (continuous dividend yield form).

### 235. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.discount_df`

EquityInputs::discount_df() is the pure numeraire discount factor e^{−r·t}: it delegates to celnet_core::math::exp (libm, correctly-rounded) with no mutation or I/O. Called by 39 downstream consumers and by price() as k_disc = strike·discount_df(). Because r is the independent discount axis (separate from carry b), this method's pure-function semantics are load-bearing for the clean partial derivatives in greeks(): the discount_rho formula holds analytically only when r enters the price solely through e^{-r·t} and e^{(b-r)·t}, which requires b to be independently varied — enforced by with_r_fixed_b.

### 236. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.EquityInputs.forward`

EquityInputs::forward() is the pure equity forward pricer: F = S·e^{b·t} where b = carry() = r−q−repo and the exponential routes through celnet_core::math::exp (libm-backed, correctly-rounded). No side effects or mutation — a read-only scalar computation. It is called by 45+ downstream consumers (surface rebuilds, risk ladder nodes, carry-seam nodes) making it the canonical equity forward reference.

### 237. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.aux`

aux(i) is the pure precomputation kernel for the generalized-BSM equity pricer: given EquityInputs it computes d1 = [ln(S/K) + (b + ½σ²)·t] / (σ√t) where b = carry() = r − q − repo is the net cost of carry, d2 = d1 − σ√t, and returns the Aux struct {d1, d2, sqt=√t, vsqt=σ√t}. No side effects, no allocation, no I/O — a pure closed-form function of its argument. Both price() and greeks() call this exactly once and re-use the cached (d1, d2, sqt, vsqt), so transcendental cost is paid once per pricing call.

### 238. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.greeks`

greeks(opt, i) is the complete, pure generalized-BSM Greeks engine for equity vanilla options: it returns EquityGreeks containing price, delta_spot (∂V/∂S = e^{(b-r)t}·Φ(±d1)), delta_forward (∂V/∂F = Φ(±d1)), gamma (e^{(b-r)t}·φ(d1)/(S·σ√t)), vega (S·e^{(b-r)t}·√t·φ(d1)), theta (generalized-BSM carry-tagged decay), discount_rho (∂V/∂r holding b fixed), carry_rho (∂V/∂b holding r — the dividend-rho), vanna (−e^{(b-r)t}·φ(d1)·d2/σ), volga (vega·d1·d2/σ), charm (∂Δ/∂T), speed (∂γ/∂S), zomma (∂γ/∂σ), and color (∂γ/∂T). The two rate sensitivities are INDEPENDENT partials in the (r, b) basis: since d1/d2 depend on b but not r, the φ-terms cancel in carry_rho, yielding clean closed forms. No I/O, no mutation, no allocation beyond the returned struct.

### 239. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.hull_index_option_reference`

hull_index_option_reference() is the externally-pinned numerical oracle for the generalized-BSM equity pricer: for S=930, K=900, r=0.08, q=0.03, σ=0.20, T=1/6 yr, repo=0 it asserts call=51.832956796490860 and put=14.550996773772400 (both to 1e-9 relative and absolute). This is an independently-computed full-precision result — not round-tripped from the crate under test — providing a ground-truth anchor that is source-stable (the specific reference values appear in the test source verbatim). Validated by assert_close! to 9 decimal places.

### 240. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.price`

CAPABILITY (equity cross-asset leaf): celnet-equity-vanilla::price is the generalized-BSM equity vanilla pricing entry on the carry seam — a pure, side-effect-free closed form taking (OptionType, &EquityInputs) where the dividend yield enters as the carry b = r - q, so the no-dividend limit collapses to standard Black-Scholes (proven by no_dividend_limit_is_standard_bsm) and the leaf reconciles to an independent QuantLib-pinned BSM oracle. Heavily re-used (in_degree 147) as the equity capability's projection target through the one contract. No I/O, allocation, logging, or mutation.

### 241. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.with_b_via_q`, `github.com-soarsa-celnet.crates.celnet-equity-vanilla.src.lib.with_r_fixed_b`

with_r_fixed_b(i, r) is the pure finite-difference perturbation helper for discount_rho (∂V/∂r at fixed b): to hold b = r−q−repo constant while shifting r, it adjusts q by the same delta (q' = q + (r_new − r_old)) so that carry() is unchanged. with_b_via_q(i, q) is the complementary helper for carry_rho (∂V/∂b at fixed r): it directly replaces q in the struct, since ∂b/∂q = −1 so ∂V/∂b = −∂V/∂q. Both are pure (no side effects, no I/O, return a new EquityInputs by struct-update). These helpers are used by the FD oracle in the test suite to verify the closed-form discount_rho and carry_rho against central differences.

### 242. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.asian.turnbull_wakeman_price`

turnbull_wakeman_price is the Turnbull-Wakeman moment-matching Asian pricer: it computes future_moments(i, spec) to get the first and second moments (ex, ex2) of the remaining average, applies seasoned_match to account for any already-settled fixings (adjusting the effective strike k_eff = spec.strike − fixed), then prices via black_on_average(spec.option, ex, ex2, k_eff, df) — a Black-76 formula on the lognormal approximation of the arithmetic average. Pure: reads &ExoticInputs + AnalyticAsian, returns f64, no WRITES.

### 243. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.barrier.single_barrier_price`

single_barrier_price decomposes a single-barrier option with optional rebate into two independent legs: (1) bare = single_barrier_no_rebate(i, kind, strike, barrier) — the core Reiner-Rubinstein reflection formula; (2) rebate leg — for KnockOut, a one-touch paying rebate at hit (one_touch_price(i, barrier, rebate, RebateTiming::AtHit)); for KnockIn, a no-touch paying rebate at expiry if the barrier is never touched (no_touch_price). Returns bare + reb. When rebate == 0.0, returns bare immediately without pricing the touch. Pure: reads &ExoticInputs + SingleBarrier, returns f64, no WRITES.

### 244. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.digital.digital_price`

digital_price is the closed-form dual-style digital kernel: given (kind, i) it reads (d1,d2) from d12(i), df_dom = i.discount_df(), df_for = i.carry_df(), and returns — CashOrNothing Call: df_dom·Φ(d2); CashOrNothing Put: df_dom·Φ(−d2); AssetOrNothing Call: S·df_for·Φ(d1); AssetOrNothing Put: S·df_for·Φ(−d1). The match is exhaustive over (DigitalStyle, OptionType) with no wildcard, enforcing that cash digitals discount with the numeraire factor and asset digitals with the yield factor. Pure: reads &ExoticInputs, returns f64, no WRITES.

### 245. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.forward_start.cliquet_price_plain`, `github.com-soarsa-celnet.crates.celnet-exotics.src.forward_start.forward_start_price`

forward_start_price prices a forward-starting option as S·e^{−q·t₁}·unit_spot_value(i, option, moneyness, residual), where t₁=spec.reset is the reset date, q is read via i.carry_df_at(spec.reset) (the yield/foreign discount factor to reset, stored r_for verbatim for FX — byte-identical to e^{−r_for·t₁}), and residual = spec.expiry − spec.reset is the remaining tenor. unit_spot_value gives the normalized option value per unit spot at a moneyness strike. cliquet_price_plain sums n forward-start legs over consecutive schedule intervals for an uncapped ratchet cliquet — valid only when Cliquet::is_plain() holds (debug_assert). Pure: reads ExoticInputs, no WRITES.

### 246. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.inputs.carry_vanilla_price_at`

CAPABILITY (cross-asset carry-seam reach): carry_vanilla_price_at is the generalized closed-form pricing kernel of the cross-asset carry seam — a pure, side-effect-free generalized-Black-Scholes-Merton evaluation parameterized by a generalized Carry (cost-of-carry b = r - q via Carry::discount_rate/yield_rate). Because every asset family lowers onto this one Carry-parameterized kernel (FX as r_dom/r_for, equity as r/dividend-yield, commodity as Black-76 r/b, crypto-linear as r/funding), the SAME pure kernel reaches vanilla/exotics/surface/risk across the FX, equity, commodity, and crypto/digital-asset and linear leaves. It performs no I/O, allocation, logging, or mutation: d1/d2 and discounted spot/strike are computed and one branch on OptionType returns the price.

### 247. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.fixed_lookback_price`, `github.com-soarsa-celnet.crates.celnet-exotics.src.lookback.floating_lookback_price`

floating_lookback_price and fixed_lookback_price are pure Conze-Viswanathan closed-form lookback pricers over the carry seam: both read b = carry_rate(), df_dom = discount_df(), df_for = carry_df() — byte-identical for FX. floating_lookback_price prices the option on the running extremum ξ=S at inception: Call = S·df_for·Φ(a1) − S·df_dom·Φ(a2) + S·df_dom·(σ²/2b)·[Φ(−a1+2b√T/σ) − e^{bT}·Φ(−a1)]. fixed_lookback_price branches on K≷S to select between the standard Conze-Viswanathan form (K≥S) and the intrinsic-lock form (K<S), each with the corresponding reflection term σ²/(2b)·[±(S/K)^{−2b/σ²}·Φ(d1−2b√T/σ·…) ∓ e^{bT}·Φ(d1)]. Both are pure (no WRITES, no allocation).

### 248. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.lsv.LsvModel.calibrate`

DELIVERABLE lsv/market-calibration-frontend = PARTIAL / still OPEN (backlog tracker docs/WORLD-CLASS-BACKLOG.md Round-2 P2/L: "LSV booking model has no market-calibration front end: no Heston-backbone NLS calibration and no mixing-weight (eta) tuning to touch/DNT quotes"; reconciled against the live graph). What EXISTS today: `LsvModel::calibrate(inputs, var, iv, spot_grid, cfg)` is a pure, side-effect-free constructor that builds an LsvModel by delegating to particle::calibrate_leverage(iv, &var, spot_grid, spot, t, cfg) — a PARTICLE leverage-function calibration to an ImpliedVolSurface — and stores {inputs, var, leverage}, no I/O/writes/allocation-in-loop in the constructor itself. The OPEN gap the finding names is NOT closed here: this calibrates the local-vol LEVERAGE to a given IV surface; it does NOT do a Heston-backbone nonlinear-least-squares calibration of the variance params, and the mixing weight (eta) is taken from VarianceParams rather than tuned to touch/DNT market quotes. SELF-INVALIDATING: when a Heston-NLS + mixing-eta market-calibration front end lands, this method's signature/body changes (it would take market touch/DNT quotes and tune var/eta), flipping or unresolving this claim — the signal that the deliverable's remaining half closed.

### 249. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.market_hedge_overlay.hedge_smile_cost`

DELIVERABLE exotics/vanna-volga-overlay-magnitude-unvalidated = LANDED (backlog tracker still lists it OPEN as a Round-2 P2/M finding; reconciled against the live graph). `hedge_smile_cost` is the pure (side-effect-free) vanna-volga market-hedge smile-overlay cost: it reads ExoticSensitivities + the broker RR/BF marks and returns the overlay cost with no external writes. The Round-2 gap (only flat-smile/sign/scaling tests; the spec-mandated VV-vs-replication magnitude cross-validation unimplemented) is CLOSED: celnet-parity::vv_magnitude::engine_overlay_matches_replicating_portfolio_oracle_in_magnitude now pins the engine overlay against a CODE-DISJOINT replicating-portfolio oracle (oracle_cost) within a derived 20% magnitude band, with <=1% relative agreement on the cross-Greeks (vanna/volga) and a materiality floor + sign-agreement guard across the product set, backed by the golden oracle hedge_smile_overlay_cost. SELF-INVALIDATING: any edit to the overlay arithmetic shifts this anchor and flips the claim stale, re-opening the reconciliation; a write-introducing regression also flips it.

### 250. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.market_hedge_overlay.hedge_smile_cost`, `github.com-soarsa-celnet.crates.celnet-exotics.src.market_hedge_overlay.hedge_smile_overlay`

hedge_smile_overlay is the Vanna-Volga smile-cost overlay: it computes raw = vanna * market.vanna_price + volga * market.volga_price (hedge_smile_cost), multiplies by survival.probability (the barrier-survival weight, clamped to [0,1] to prevent over-hedging near the barrier), and returns OverlayResult{flat_vol_price, hedge_smile_cost: survival.probability*raw, smile_price: flat_vol_price + cost}. The survival weighting accounts for the reduced probability that an exotic product survives to expiry — reducing the smile correction proportionally. Pure: reads inputs and returns OverlayResult, no WRITES.

### 251. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.perpetual.perpetual_price`

DELIVERABLE proto/new-payoff-shapes (PerpetualOption arm) = DONE (RC cut a0817d6; arm 30). `perpetual_price(opt, i)` is a pure, total closed-form valuation of an American perpetual option: it dispatches on the `valuation(opt, i)` result and returns spot for a never-exercised call, strike for a never-exercised put, the intrinsic `opt.sign()*(spot-strike)` when immediately exercised, and the continuation `value` otherwise — every `Valuation` arm enumerated, no wildcard, so the match is exhaustive and a new regime is a compile error rather than a silent fall-through. Pure: it borrows `&PerpetualInputs`, performs no I/O/allocation/mutation, and returns `Result<f64, PerpetualError>`. Verified non-circularly against an independent root-bracketing re-derivation (closed_form_matches_independent_bisection_rederivation) in the same module. This is one of the two genuinely-new payoff shapes the backlog tracked as OPEN; it is now built + golden/parity-gated + surfaced across all 5 clients.

### 252. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.tarf.tarf_price`

DELIVERABLE exotics/qmc-pathwise-wiring = OPEN (Round-2 P2/M finding; reconciled against the live graph — still genuinely open at this round). `tarf_price` is a pure (side-effect-free) Monte-Carlo TARF valuation: it reads ExoticInputs/Tarf/TarfMcConfig, builds local per-fixing buffers, and returns a TarfResult with no external writes. The OPEN gap: the path generator is STILL the plain `CounterRng` antithetic Philox stream (`CounterRng::new(cfg.seed, 0, pair, 0)` + inverse_cdf), NOT the scrambled-Sobol / Brownian-bridge QMC stack in celnet-qmc that already feeds american.rs/multiasset.rs. The path-dependent pricers (tarf/accumulator/lookback/quanto/pivot) therefore forgo the low-discrepancy variance reduction the QMC crate provides. SELF-INVALIDATING: when this pricer is rewired onto celnet-qmc (Sobol/bridge) the function body changes and this claim flips stale, signalling the deliverable has closed; a write-introducing regression also flips it.

### 253. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-exotics.src.touch.one_touch_price`

DELIVERABLE exotics/one-touch-at-hit-pairing-flip = LANDED (backlog tracker still lists it OPEN as the Round-2 P0; reconciled against the live graph). `one_touch_price` (-> one_touch_with_side) is a pure (side-effect-free) closed-form one-touch valuation: it reads ExoticInputs/barrier/rebate/timing and returns a price with no external writes. The Round-2 P0 (~28% high / 2x-on-far-barriers at-hit pairing flip + circular golden oracle) is FIXED-AT-ROOT: the at-hit branch is now pinned to an INDEPENDENT first-passage quadrature reference (at_hit_matches_independent_first_passage_quadrature, 1e-12) and guarded by a non-circular family — discounted-hit-probability sandwich (at_hit_sandwiched_by_discounted_hit_probability), t->inf perpetual-discounted-hit limit (at_hit_t_infinity_is_perpetual_discounted_hit_factor), barrier continuity (at_hit_continuous_at_the_barrier), zero-rate collapse to deferred (zero_discount_rate_collapses_at_hit_to_deferred), and barrier monotonicity. SELF-INVALIDATING: a regression that re-introduces a WRITES side effect, or any re-pairing edit that shifts these anchors, flips this claim stale, re-opening the reconciliation.

### 254. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.dialect_fx.inputs_for`, `github.com-soarsa-celnet.crates.celnet-fix.src.dialect_fx.price_leg`

inputs_for(desc, snap) is the pure bridge from FIX-decoded option descriptor + live market snapshot to VanillaInputs: `VanillaInputs::new(snap.spot, desc.strike, snap.vol, snap.t, snap.r_dom, snap.r_for)`. It reads only its two arguments and allocates nothing. price_leg(desc, snap, pricer) composes it with a VanillaPricer fn-pointer: `pricer(desc.option_type, &inputs_for(desc, snap))`, returning the single-leg option price as f64. These two functions are the seam between the FIX wire representation and the celnet-pricer analytics kernel.

### 255. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameCursor<'a>.parse`

FrameCursor::parse is the FIX 4.4 frame validator: given a raw &[u8] it (1) rejects frames shorter than 21 bytes; (2) asserts tag 8 = "FIX.4.4"; (3) reads tag 9 (BodyLength) as the declared byte-count; (4) locates the tag-10 checksum field via find_checksum_field, computes actual_body_len = cs_field_start − body_start and rejects if actual ≠ declared; (5) recomputes checksum(&raw[..cs_field_start]) mod-256 and rejects on mismatch; (6) scans the body for tag 35 (MsgType) and rejects if absent. All errors are typed FrameError variants (TooShort, MissingBeginString, UnsupportedBeginString, MissingBodyLength, BadBodyLength, BodyLengthMismatch{declared,actual}, MissingCheckSum, BadCheckSum, CheckSumMismatch{declared,computed}, MissingMsgType). The function reads only its argument and allocates nothing (zero-copy cursor over the input slice).

### 256. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.framing.FrameEncoder.finish`

FrameEncoder::finish serialises a complete FIX 4.4 frame from the accumulated body: it prepends `8=FIX.4.4<SOH>9=<bodylen><SOH>`, appends the body, then computes checksum(&out) over all preceding bytes and appends `10=<3-digit-padded checksum><SOH>`. The result is a freshly allocated Vec<u8> with pre-sized capacity (body.len() + 24). finish() only reads self and calls no I/O — its output is the unique well-formed wire frame corresponding to the encoder's accumulated tags.

### 257. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-fix.src.framing.checksum`

SAFETY/WIRE — FIX session-layer frame integrity is the standard FIX BodyLength/CheckSum (tag 10) modulo-256 sum: checksum folds every byte of the message-up-to-and-including the SOH before tag 10 with wrapping u32 addition and returns (acc & 0xFF) as u8 — the canonical FIX checksum that is always rendered as a 3-digit field and validated on inbound frames (rejects_corrupted_checksum, checksum_is_mod_256). A counterparty frame whose recomputed mod-256 checksum does not match the transmitted tag-10 value is rejected at framing, so a corrupted/truncated FIX message never reaches order/quote handling. Pure: it reads only the input byte slice and returns the u8 checksum, mutating nothing.

### 258. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76_price`

SAFETY/ORACLE — black76_price is the independent golden reference for futures-style (forward-measure) options (the Black-76 closed form), NOT the production engine's own pricer: for t<=0 it returns the discounted intrinsic exp(-r·t)·max(sign·(F-K),0); otherwise the standard Black-76 with vsqt=vol·√t, d1=(ln(F/K)+½σ²t)/vsqt, d2=d1-vsqt, discount df=exp(-r·t), Call=df·(F·N(d1)-K·N(d2)) and the Put put-call complement. It is pure and deterministic over its 6 scalar inputs (libm transcendentals only, no I/O/mutation/allocation), so it is a trustworthy can-disagree oracle gating commodity / listed-future-option parity against the engine. Self-invalidates if the closed form drifts.

### 259. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.black76_undiscounted_price`

Golden-oracle gate (Black-76 commodity/forward reference): black76_undiscounted_price is a pure closed-form function of (cp, forward, strike, vol, t) using only libm math and the pure xerf_norm_cdf, with the t<=0 intrinsic-payoff branch. No writes, no I/O, deterministic — the QuantLib-pinned reference price the parity suite gates production engines against must be a pure function of its inputs.

### 260. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cholesky`

SAFETY/ORACLE — cholesky is the correlation-decomposition primitive underpinning the independent basket / multi-asset Monte-Carlo golden references: it computes the lower-triangular factor L of a symmetric positive-definite correlation matrix via the classic doubly-indexed recurrence (sum a[i][j] - Σ_k l[i][k]·l[j][k], diagonal = √sum, off-diagonal = sum/l[j][j]). It is pure and deterministic over its input matrix (no I/O, no mutation of the argument, no RNG), and it is fail-closed: a non-positive-definite matrix trips assert!(sum > 0.0) and panics rather than silently emitting a NaN/garbage factor that would corrupt every correlated-path draw. This makes the oracle's correlated scenarios reproducible and trustworthy as a can-disagree reference. Self-invalidates if the PD guard is removed.

### 261. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.cliquet_plain_price`

`cliquet_plain_price` prices a plain cliquet as a sum of `periods` evenly-spaced forward-start option legs, each with reset at t_{k−1} and expiry at t_k = k·expiry/periods, accumulated without local or global caps. The closed-form sum is exact for flat GBM vol.

### 262. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto_inverse_price`

Golden-oracle gate (inverse/coin-margined crypto reference): crypto_inverse_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — carry b=r-funding, forward, the (K/F)e^{sigma^2 t} amplitude correction for the 1/S_T inverse payoff, libm math and pure xerf_norm_cdf only. No writes, no I/O, deterministic; the reference price gating the inverse crypto engine must depend solely on its inputs.

### 263. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.crypto_linear_price`

Golden-oracle gate (linear/GK-funding crypto reference): crypto_linear_price is a pure closed-form function of (cp, spot, strike, vol, t, r, funding) — applies carry b=r-funding to the forward then delegates to the pure black76_price. No writes, no I/O, deterministic; the linear crypto reference price the parity suite uses is a pure function of its inputs.

### 264. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.det3`

Golden-oracle numerical kernel: det3 computes the 3x3 determinant by cofactor expansion over a borrowed [[f64;3];3] with no writes, no I/O — a pure function of the matrix. Determinism of this kernel underpins the correctness of the oracle routines (e.g. correlation/quanto projections) that the parity gate trusts as ground truth.

### 265. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.discounted_first_passage_integral`, `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.one_touch_at_hit_price`

`one_touch_at_hit_price` prices a sideless one-touch option using the inverse-Gaussian first-passage-time distribution: it computes z = ln(barrier/spot), drift ν = (r_dom − r_for) − ½σ², then returns rebate × ∫₀ᵀ e^{−r·t} f_τ(t) dt via a 48-panel log-halving Gauss–Legendre quadrature (`discounted_first_passage_integral`). The degenerate boundary spot == barrier short-circuits to exactly `rebate` (immediate certain hit); spot strictly beyond the level is treated as the live contract on the opposite side, never as a breach.

### 266. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.discounted_first_passage_integral`

`discounted_first_passage_integral` evaluates ∫₀ᵀ e^{−r·t} · (|z|/(σ√(2π))) · t^{−3/2} · exp(−(z−νt)²/(2σ²t)) dt by 48-panel geometric (log-halving) Gauss–Legendre quadrature, concentrating panels near t = 0 where the integrand is steepest. An exponent guard at −700 prevents Inf·0 from the t^{−3/2} prefactor at near-zero t. The function is stateless and deterministic over its six scalar inputs.

### 267. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.equity_bsm_price`

SAFETY/ORACLE — equity_bsm_price is the independent golden reference for generalized Black-Scholes-Merton equity vanillas (carry b = r - q - repo), NOT the engine's own pricer: for t<=0 it returns intrinsic max(sign·(S-K),0); otherwise d1=(ln(S/K)+(b+½σ²)t)/(σ√t), d2=d1-σ√t, with cost-of-carry-discounted spot s_disc=S·exp((b-r)t) and rate-discounted strike k_disc=K·exp(-r·t), Call=s_disc·N(d1)-k_disc·N(d2) and the Put complement. It is pure and deterministic over its 8 scalar inputs (libm only, no I/O/mutation/allocation), serving as a can-disagree oracle gating equity-vanilla parity (dividend yield + repo carry) against the engine. Self-invalidates if the carry decomposition drifts.

### 268. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.floating_lookback_price`

`floating_lookback_price` computes the closed-form price of a floating-strike lookback option (call: S_T − S_min; put: S_max − S_T) using the Goldman–Sosin–Gatto formula. With b = r_dom − r_for and two_b_over_sig2 = 2b/σ², the call is `S·df_for·N(a1) − S·df_dom·N(a2) + S·df_dom·(σ²/2b)·[N(−a1+two_b_over_sig2·σ√T) − e^{bT}·N(−a1)]` and the put is the symmetric complement.

### 269. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.forward_start_price`

`forward_start_price` prices a forward-start option as `e^{−r_for·reset} · spot · V_unit`, where V_unit is a unit-spot vanilla (gk_price with spot=1, strike=moneyness, maturity=expiry−reset). If expiry ≤ reset the option has already started and the payoff collapses to the intrinsic max(cp·(1−moneyness), 0). The formula is exact for a GBM model with flat vol.

### 270. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx_forward_pv`

Golden-oracle gate (FX forward PV reference): fx_forward_pv is a pure closed-form function of (side, spot, strike, notional, t, r_dom, r_for) — side*notional*(spot*e^{-r_for t} - strike*e^{-r_dom t}), dual-discounted, no writes, no I/O, deterministic. The FX-forward reference PV the parity suite pins must depend solely on its inputs.

### 271. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx_swap_points`

Golden-oracle gate (FX swap points reference): fx_swap_points is a pure closed-form function of (spot, near_t, far_t, r_dom, r_for) — spot*(e^{b*far_t}-e^{b*near_t}) with carry b=r_dom-r_for, no writes, no I/O, deterministic. The FX-swap points reference the parity suite pins must depend solely on its inputs.

### 272. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.fx_swap_pv`

`fx_swap_pv` prices an FX swap as the algebraic sum of two offsetting forward legs: a near leg with side `near_side` at `near_t` and a far leg with side `−near_side` at `far_t`, both evaluated by `fx_forward_pv`. This makes the swap PV the difference of two discounted FX-forward residuals, consistent with the convention that the near and far legs carry opposite sign on the domestic-currency notional.

### 273. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.gk_price`

SAFETY/ORACLE — gk_price is the independent golden reference (the Garman-Kohlhagen two-rate FX vanilla closed form) against which the production engine is validated, NOT the engine's own pricer: for t>0 it computes d1=(ln(S/K)+(r_dom-r_for+0.5*vol^2)*t)/(vol*sqrt(t)), d2=d1-vol*sqrt(t), df_dom=e^{-r_dom*t}, df_for=e^{-r_for*t}, and returns S*df_for*N(d1)-K*df_dom*N(d2) for a Call (put by symmetry); for t<=0 it returns the discounted intrinsic max(sign*(S-K),0). It deliberately re-derives the price from first principles with its own norm_cdf so a parity test (e.g. vanilla_price_and_greeks_match_quantlib, also pinned to published QuantLib numbers) can disagree with the engine — the anti-circular-oracle property: numerical correctness is checked against this reference, never merely asserted plausible. Pure: it reads only its scalar args and returns the f64 price, mutating nothing.

### 274. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.hedge_smile_overlay_cost`

`hedge_smile_overlay_cost` computes the smile-overlay hedge cost for a target vega/vanna/volga exposure vector using Cramer's rule on the 3×3 pillar Greeks matrix. It assembles a matrix A where A[i][j] is the j-th Greek (vega, vanna, volga) of the i-th pillar vanilla, inverts it via `det3`, solves Aw = g for weights w, then computes cost = Σ w_i · (pillar_price_at_pillar_vol − pillar_price_at_flat_vol). Returns `HedgeOverlayOracleError::SingularHedgeMatrix` if |det| < 1e-12.

### 275. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.ndf_pv`

Golden-oracle gate (NDF PV reference): ndf_pv is a pure closed-form function of (side, spot, strike, notional, t, r_dom, r_for) — a non-deliverable forward prices as the deliverable forward, delegating to the pure fx_forward_pv. No writes, no I/O, deterministic; the NDF reference PV the parity suite pins is a pure function of its inputs.

### 276. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.perpetual_american_price`

`perpetual_american_price` computes the perpetual American option value via the characteristic-equation root ψ(y) = ½σ²y(y−1) + by − r = 0, solved to full f64 precision by 200-step bisection. Call: returns None if b > r (divergent), `Some(spot)` if b ≥ r in the sub-ULP limit (no finite exercise boundary), else `(B−K)·(S/B)^y` below boundary B = Ky/(y−1) or intrinsic above. Put uses the negative root y₂ < 0 with analogous boundary; r=0 is handled by exact factorization yielding y = 1 − 2b/σ².

### 277. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto_digital_price`

`quanto_digital_price` prices a FX-quanto cash-or-nothing digital using the same quanto drift correction as `quanto_vanilla_price`: it applies r_for_adj = r_for + ρ·σ·σ_conv, computes d2 directly, and returns df·N(±d2). The formula is the exact BSM limit for a cash digital with quanto-adjusted drift.

### 278. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-golden.src.oracle.quanto_vanilla_price`

`quanto_vanilla_price` prices a FX-quanto vanilla by replacing r_for with r_for − (−ρ·σ_asset·σ_fx), i.e. the quanto carry adjustment is −ρ·σ·σ_conv, then delegating to `gk_price`. This is the standard quanto drift correction: the asset grows at r_dom − (r_for − ρσσ_conv) under the domestic risk-neutral measure.

### 279. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.as_normal.as_inverse_brackets_libm_inverse`, `github.com-soarsa-celnet.crates.celnet-gpu.src.path.inv_norm_cdf`

inv_norm_cdf(p: f32) -> f32 (WGSL, path.wgsl) is the inverse normal CDF used in the Sobol QMC path generator. It implements the Acklam rational-approximation with three regions (tail p < 0.02425, central, upper tail), followed by one Halley-step refinement: `x = x - (norm_cdf(x)-p)/norm_pdf(x) / (1 + 0.5*x*(norm_cdf(x)-p)/norm_pdf(x))`. The function is pure (no writes, no allocation). It is the GPU-side counterpart of the CPU Acklam implementation in celnet-qmc, and the two are kept bit-comparable within f32 precision by the test `as_inverse_brackets_libm_inverse`.

### 280. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as_erf_oracle_brackets_golden`, `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.erf_as`, `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.norm_cdf_f32`

erf_as(x: f32) -> f32 (WGSL, batch.wgsl) is the GPU f32 error-function, implementing Abramowitz & Stegun formula 7.1.26: `t = 1/(1+p*|x|)`, Horner-evaluated 5-term polynomial, `y = 1 - poly*exp(-x^2)`, reflected for x<0 via `s = sign(x)`. The coefficients are bit-identical to the CPU oracle `erf_as_oracle` in batch.rs (verified by `as_erf_oracle_brackets_golden`). norm_cdf_f32 wraps it as `0.5*(1 + erf_as(x * INV_SQRT_2))`. Both functions are pure (no writes).

### 281. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.as_erf_price_bound`

PILLAR (CLAUDE.md guardrails 5 + 6 + 7 — numerical code is VALIDATED against a derived bound, never merely asserted plausible; the GPU scale path uses open methods and Metal lacks f64 so the f32 path is rigorously bounded). `as_erf_price_bound(b)` is the pure, closed-form per-instrument absolute-error envelope for the f32/`as`-erf batch GPU kernel: it returns `(s_disc + k_disc) * 0.5 * AS_ERF_MAX_ABS_ERR` with `AS_ERF_MAX_ABS_ERR = 1.5e-7`, where the discounted-spot and discounted-strike legs scale the worst-case erf approximation error into a price tolerance. The many-instrument GPU batch path (CLAUDE.md guardrail 6 — IB-sized portfolios / high-throughput scale-out) is reconciled three-way against the exact f64 oracle WITHIN this analytic bound, so the precision claim is proven rather than assumed. The function is side-effect-free: it reads only the borrowed BatchInstrument and computes a scalar via libm-backed exp, mutating nothing.

### 282. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.batch_reconciles_three_way`, `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.cpu_batch_with_as_erf`, `github.com-soarsa-celnet.crates.celnet-gpu.src.batch.gk_price_as_oracle`

gk_price_as_oracle(b: &BatchInstrument) -> f64 is the CPU closed-form GBM (Garman-Kohlhagen) pricer used as the validation oracle for the GPU batch path: d1 = (ln(spot/strike) + (r_dom − r_for + 0.5σ²)T) / (σ√T), d2 = d1 − σ√T, price = spot*exp(−r_for*T)*N(sign*d1) − strike*exp(−r_dom*T)*N(sign*d2) for call (sign=+1) or put (sign=−1), clamped to zero. All transcendentals route through celnet_core::math (libm-backed, deterministic). cpu_batch_with_as_erf maps this oracle over a &[BatchInstrument] slice. The test `batch_reconciles_three_way` asserts GPU batch, CPU path Monte Carlo, and this oracle agree within `as_erf_price_bound`.

### 283. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.counter_rng.counter_block`, `github.com-soarsa-celnet.crates.celnet-gpu.src.counter_rng.counter_round`

counter_block(counter: [u32;4], key: [u32;2]) -> [u32;4] is the Philox-2×32 CBRNG block function: it applies PHILOX_ROUNDS iterations of counter_round (32×32→64 multiply / hi-lo split / XOR mixing) with a Weyl key schedule (key[0] += PHILOX_KEY_BUMP_0; key[1] += PHILOX_KEY_BUMP_1 each round), producing 4 statistically-independent u32 output words from a (counter, key) pair. It is pure: reads only its two array arguments, no allocation or I/O. The counter address encodes (path_index, stream_index) so distinct (path,stream) pairs map to non-overlapping random streams.

### 284. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.CpuBackend.label`

ADR (GPU abstraction = wgpu baseline + CPU-SIMD fallback, CUDA optional). `CpuBackend::label` returns the constant identity "cpu-f64": a pure accessor (reads nothing, mutates nothing) that names the f64 CPU oracle within the shared `PricingBackend` trait. That trait is the portability seam — `CpuBackend` and the wgpu `GpuBackend` both implement the same simulate_paths/reduce_payoff/price_vanilla contract, with Philox path index i fixed across backends so results reconcile (modulo the f32/f64 element type) — which is exactly what makes the GPU backend swappable behind one interface. DECISION/RATIONALE: the GPU strategy is wgpu (Metal/Vulkan/DX12) as the open, permissively-licensed baseline, with the CPU-SIMD path as the always-available f64 oracle and reconciliation reference, and an optional CUDA backend behind the same trait — chosen over a CubeCL/CUDA-first design because wgpu keeps the runtime dependency-set fully open-source and portable across the M4/Metal dev box and Linux/Vulkan CI. The label encodes the key portability caveat the design must respect: the CPU oracle is f64 while the wgpu/Metal path is f32 (Metal lacks f64), so cross-backend agreement is asserted to the f32 tolerance, never bit-identity. (Guardrail: no commercial products; open GPU stack with wgpu first-class.)

### 285. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.pairwise_sum`, `github.com-soarsa-celnet.crates.celnet-gpu.src.cpu.pairwise_sum_is_order_stable`

pairwise_sum(xs: &[f64]) -> f64 is the numerically-stable recursive summation used for all Monte Carlo reduction readbacks. For slices of length ≤ 64 it uses a sequential accumulator (cache-friendly base case); for longer slices it recursively splits at the midpoint. This avoids catastrophic cancellation in large path counts compared to a naive left-fold. It is pure (reads only xs, no allocation of its own, self-recursive). The test `pairwise_sum_is_order_stable` confirms the result is independent of call-site ordering.

### 286. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-gpu.src.path.sobol_coord`

sobol_coord(i: u32, j: u32) -> u32 (WGSL, path.wgsl) computes the j-th coordinate of the i-th Sobol' quasi-random point via Gray-code enumeration: g = i ^ (i >> 1), then XORs the direction numbers dir_nums[j*32 + k] for every set bit k of g. The docstring states it is bit-identical to `celnet_qmc::SobolSequence::point_u32`, making it the GPU mirror of the CPU Sobol engine. It is pure (no writes).

### 287. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr_madan`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr_madan_upper`

carr_madan(opt, m, p) prices a European FX option via the Carr–Madan damped-integrand method, working in log-moneyness κ = ln(K/S₀) to avoid forming the large ln(S₀) phase. It evaluates ψ(v) = φ_ret(v−(α+1)i) / (α²+α−v²+i(2α+1)v) with damping constant α = CM_ALPHA, then integrates Re[e^{−ivκ}·ψ(v)] from 0 to an adaptively chosen upper limit via Gauss–Legendre quadrature. The call price is df_d·S₀·e^{−ακ}/π·∫…dv; puts are obtained via exact put–call parity. The upper integration limit and panel count are both chosen adaptively (carr_madan_upper + oscillation count) to keep truncation and quadrature error below double precision.

### 288. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.carr_madan_upper`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.effective_log_variance`

carr_madan_upper(m, p) determines the Carr–Madan integration upper bound by taking the larger of two regime-specific decay envelopes — a Gaussian estimate sqrt(2·ln(10¹⁶)/E[variance]) covering small-σ regimes, and an exponential-tail estimate ln(10¹⁶)/rate (rate = v₀/(σ√(1−ρ²)) + κθT√(1−ρ²)/σ) covering slow-decaying high-σ regimes — applying a 1.5× safety margin and clamping to [50, 20 000]. The threshold ln(10¹⁶)≈36.84 ensures the integrand envelope is ≤1e−16 at the upper limit in either regime.

### 289. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.char_exponent`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.heston_c4`

heston_c4(p, t) computes the 4th cumulant of the Heston log-return distribution via a 5-point central-difference approximation of the 4th derivative of the real log-CF ln φ_ret at u=0, using h=1e-2: d4 = (ψ(2h) − 4ψ(h) + 6ψ(0) − 4ψ(−h) + ψ(−2h)) / h⁴. By evaluating char_exponent at real frequencies it is independent of the COS series, so it cannot mask a COS bug. Market inputs are neutralised (spot=strike=1, rates=0) since spot/strike/rates only shift c₁ and do not affect centred cumulants.

### 290. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.char_exponent`

char_exponent(u: Complex, m: &MarketInputs, p: &HestonParams) -> Complex computes ln φ_ret(u), the log of the Heston return-space characteristic function, using an overflow-stable factoring: it forms w = exp(−d·t) (bounded, |w|≤1 since Re(d)≥0) and then computes A = (u²+iu)(1−w) / [d(1+w) + ξ(1−w)] and B via e^{−dt/2} kept inside the log, so the large e^{Re(d)·t/2} factor that would overflow cosh/sinh at high |u| or long t cancels before any floating-point operation. Result: φ_ret = exp(iuμt − (κθρt/σ)·iu − v₀·A + (2κθ/σ²)·ln B). No I/O, no allocation, no mutation — the function reads only its three arguments.

### 291. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.cos`, `github.com-soarsa-celnet.crates.celnet-heston.src.lib.heston_c4`

cos(opt, m, p) prices a European FX option via the Fang–Oosterlee (2008) COS series. It computes Heston cumulants c₁ (mean log-return) and c₂ (variance) per Fang–Oosterlee Table 11, then the 4th cumulant c₄ via numerical 4th-difference of char_exponent (see heston_c4). The truncation range is [c₁ ± L·√(|c₂|+√|c₄|)] — including c₄ is essential for fat-tailed regimes (high σ, long T). The put leg is always priced directly (call coefficients evaluate e^{hi} at the wide right edge and lose precision); the call is recovered by exact put–call parity C = P + S·e^{−r_f T} − K·e^{−r_d T}. N=128 cosine terms are summed with the n=0 half-weight Fourier convention.

### 292. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-heston.src.lib.gauss_legendre`

gauss_legendre(f, lo, hi, n_panels) integrates a real-valued function using a composite 16-point Gauss–Legendre rule. It divides [lo,hi] into n_panels equal sub-intervals and applies the 8-node symmetric GL rule (GL16_X/GL16_W, exploiting symmetry: one loop over k=0..8 evaluates f(mid+dx)+f(mid−dx)) to each panel, accumulating acc += GL16_W[k]*(f(mid+dx)+f(mid−dx)) then returning acc*half. No heap allocation; no branching; a purely functional numeric kernel.

### 293. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.aggregate.BlendConfig.staleness_weight`, `github.com-soarsa-celnet.crates.celnet-integration.src.aggregate.blend`

BlendConfig::staleness_weight() computes the exponential decay weight 2^{−age/half_life} = exp(−ln2 · age_secs / staleness_half_life_secs) for a source observation timestamp relative to the valuation time, clamping negative age (future-dated ticks) to zero. Default half-life is 30 s; a source two half-lives old receives weight 0.25. blend() normalizes these weights over non-excluded sources so the surviving contributors sum to 1.0.

### 294. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.declared_is_premium_adjusted`

Vendor-feed normalization re-derives the premium-adjusted axis at the integration seam (docs/CONVENTIONS.md DeltaConvention→premium-style; docs/CELER-INTEGRATION.md vendor normalization). `declared_is_premium_adjusted(d)` is a pure const-fn total predicate over DeltaConvention — true exactly for SpotPremiumAdjusted and ForwardPremiumAdjusted, false for the unadjusted Spot/Forward variants — identical in meaning to ConventionRecord::is_delta_premium_adjusted but defined on the integration normalize path where an incoming vendor delta convention is checked against the resolved house record. The two definitions must agree variant-for-variant; drift here would mis-normalize a vendor feed's premium-adjusted flag. Pure: no writes/allocation/IO; deterministic. Self-invalidates if the enum→bool mapping or DeltaConvention variant set changes (WRITES gate).

### 295. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-integration.src.normalize.vol_time`

vol_time() computes ACT/365-fixed calendar vol-time for a pair/tenor anchored at the feed's observation timestamp (via vol_year_fraction over the real spot→expiry schedule), falling back to nominal_year_fraction only when no horizon date can be derived from the nanosecond timestamp. This means the canonical surface input is always built on a well-defined, calendar-exact vol-time — never a nominal months×30 approximation when the observation date is available.

### 296. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.build_table`

Journal CRC-32 table safety (deliverable: journal-durability). build_table is a const fn computing the reflected IEEE CRC-32 (polynomial 0xEDB88320) lookup table purely from compile-time constants — referentially transparent by construction, no I/O, no mutable global state escaping the function, identical on every build and every platform. This is the deterministic root the entire journal torn-tail / corruption-rejection guarantee rests on: a stable table means a stable CRC. Pure (no WRITES edges); self-invalidates if the table derivation changes.

### 297. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.crc32.crc32`

SAFETY — journal record integrity is a standard CRC-32 (IEEE 802.3 reflected polynomial): crc32 seeds 0xFFFF_FFFF, folds each byte through the 256-entry reflected lookup TABLE (crc = (crc >> 8) ^ TABLE[(crc ^ b) & 0xFF]), and finalizes with the XOR-out 0xFFFF_FFFF — the bit-exact reflected CRC-32 whose known-answer vectors (e.g. "123456789" => 0xCBF43926) are pinned by the crate's own vectors test. Every framed journal record carries this checksum over sync-word+header+payload (frame_record appends crc32(frame).to_le_bytes()), so any single-bit flip in a persisted record changes the CRC and the record is rejected on replay rather than silently mis-applied to recovered book/market state. Pure: it reads only the input byte slice and returns the u32 checksum, mutating nothing.

### 298. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame_record`

Journal CRC/torn-tail safety (deliverable: journal-durability). frame_record is a pure, deterministic framing function: given (sequence, payload_len, payload) it builds the sync-word + little-endian header + payload + trailing CRC-32 byte-for-byte with no I/O, no shared mutation, and no observable side effect. Determinism is the load-bearing invariant — the recovery reader recomputes the same CRC over the same framed prefix, so any torn or corrupted tail fails the checksum identically on every replay. Pure (no WRITES edges); self-invalidates if framing gains state.

### 299. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-journal.src.lib.frame_snapshot`

Journal snapshot framing safety (deliverable: journal-durability). frame_snapshot is a pure, deterministic framing function: given (watermark, snap_len, snapshot) it emits sync-word + snapshot-marker + little-endian watermark/len + snapshot bytes + trailing CRC-32 with no I/O and no side effect, sharing the identical crc32 trailer discipline as the record path so the two framings cannot drift. Determinism guarantees the recovery reader's recomputed CRC matches bit-for-bit, rejecting any torn snapshot tail on replay. Pure (no WRITES edges); self-invalidates on code change.

### 300. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.LimitCheck.is_hard_breach`

SAFETY — pre-trade hard-breach gating (deliverable: limits-breach-detection). LimitCheck::is_hard_breach is a pure, deterministic predicate: it returns true iff BOTH the limit's enforcement == Enforcement::Hard AND its RagStatus utilization status is_breach() — the conjunction that distinguishes a rejectable hard breach from a soft (advisory) warning. It reads only &self (the limit spec + the precomputed utilization), performs no I/O, no mutation, no allocation; identical inputs always yield the identical verdict. This is the exact gate pre_trade_check / post_trade_check funnel through to decide PreTradeDecision::Reject, so a hard limit is enforced (a soft one only warns). Self-invalidates if the enforcement/status conjunction is ever weakened.

### 301. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.exposure_of`

Pre-trade limit-breach exposure safety (deliverable: limits-breach-detection). exposure_of is a pure, deterministic, total function over LimitMetric: it reads the aggregated net Greeks, vega ladder, gross concentration, and non-additive VaR/ES/StopLoss out of borrowed &NodeAggregate / &NonAdditiveExposure and returns the scalar exposure with no I/O and no mutation of any input. Determinism is the load-bearing safety property — pre-trade and post-trade checks (its four callers) measure the same metric against the same limit cap identically, so a breach can never be hidden by a non-reproducible reading. Pure (no WRITES edges); self-invalidates on change.

### 302. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.check.gross_concentration`

Pre-trade/post-trade gross concentration exposure is a pure read-only reduction: gross_concentration folds the abs of each leaf greek (delta_base or vega per ConcentrationMetric) over node.leaves with no writes. The limit-breach concentration metric is therefore a deterministic function of the aggregate snapshot alone — the safety property a pre-trade limit check relies on.

### 303. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.classify`

SAFETY — pre-trade limit breach is hard-classified at the cap, not approximated: LimitSpec::classify maps a projected exposure to a RagStatus by exact threshold order — ratio > 1.0 (exposure strictly over cap) is RagStatus::Breach; ratio >= red is Red; ratio >= amber is Amber; else Green. Breach is the ONLY band above the cap, so a hard limit can never be silently under-classified as a mere Red warning. classify is the single pure breach-decision atom (it reads &self spec + the f64 exposure and returns a Utilization, mutating nothing); pre_trade_check builds on it and turns any is_hard_breach into PreTradeDecision::Reject. The strict-greater-than at the cap means an exposure exactly AT the cap (ratio == 1.0) is Red, not Breach — utilisation up to and including the cap is permitted, beyond it is rejected.

### 304. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.limit.LimitSpec.utilization`

SAFETY — limit utilization is a total, fail-closed ratio (deliverable: limits-breach-detection). LimitSpec::utilization(exposure) is a pure deterministic function: it returns |exposure|/cap when cap > 0, f64::INFINITY when cap == 0 (or negative) and |exposure| > 0, and 0.0 only when both are zero. The zero-cap → INFINITY branch is the fail-closed safety property: a zero cap is treated as an instant breach, never as an unbounded/divide-by-zero allowance, so a misconfigured zero limit can never silently admit risk. It reads only &self and the f64 argument; no I/O, mutation, or allocation. Self-invalidates if the zero-cap branch stops returning INFINITY.

### 305. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-limits.src.tree.ScopePath.resolve`

ScopePath::resolve deterministically maps a `FactKey` + `Hierarchy` to a fixed 7-element ordered scope list: [Trader(key.trader), Book(key.book), Desk(hierarchy.desk_of(key.book).unwrap_or(key.desk)), CcyPair(scope_pair_of(&key.underlying)), Location(key.location), Entity(hierarchy.entity_of(key.location).unwrap_or(key.entity)), Firm]. The Hierarchy overrides desk and entity via parent-pointer lookup (`desk_of`/`entity_of`), falling back to the fact's own ids when no parent is registered. This is a pure function with no I/O or mutation.

### 306. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.forward.fair_forward`

fair_forward(inputs) returns the at-market forward rate F = spot * e^{b * near_settle_t} (carry.forward_factor delegates to libm::exp). It is the zero-PV strike: a forward struck at this rate has PV = 0, verified bit-for-bit by fair_forward_has_zero_pv_to_bits (assert_eq!(pv(...at_fair...).to_bits(), 0_f64.to_bits())). The function reads only &LinearInputs and writes nothing — pure and side-effect-free.

### 307. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.forward.fd_central`, `github.com-soarsa-celnet.crates.celnet-linear.src.forward.greeks`

greeks(inputs) computes the complete analytic Greek vector {pv, delta, rho_dom, rho_for, theta} for an FX outright forward in closed form, routing all transcendentals through celnet_core::math::exp (libm-backed, bit-identical to the Carry accessors) for cross-platform reproducibility. The closed forms are: delta = sign*N*e^{-r_for*t}; rho_dom = sign*N*K*t*e^{-r_dom*t}; rho_for = -sign*N*S*t*e^{-r_for*t}; theta = sign*N*(-r_for*S*e^{-r_for*t} + r_dom*K*e^{-r_dom*t}). All five outputs are independently cross-checked against central finite differences (fd_central: (f(x+h)-f(x-h))/(2h)) in greeks_match_central_finite_difference. The function reads only its &LinearInputs argument and writes nothing — pure and side-effect-free.

### 308. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.forward.oracle_pv`, `github.com-soarsa-celnet.crates.celnet-linear.src.forward.pv`, `github.com-soarsa-celnet.crates.celnet-linear.src.forward.pv_at`

pv_at(inputs, t) is the single closed-form present-value kernel for an FX outright forward at time t: PV = sign(side) * N * df_dom(t) * (F(t) - K), where F(t) = spot * e^{b*t} (forward via carry.forward_factor), df_dom(t) = e^{-r_dom*t} (via carry.discount_df), K = contract_rate, N = notional, sign = +1 Buy / -1 Sell. It reads only its two arguments, has no side effects, no allocation, no I/O — pure by inspection. Both forward::pv and swap::pv delegate entirely to this kernel. The formula is cross-validated against oracle_pv (an independent flat expansion: sign*N*(spot*e^{-r_for*t} - K*e^{-r_dom*t})) byte-for-byte in pv_matches_independent_discount_bond_route.

### 309. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.discount_df`, `github.com-soarsa-celnet.crates.celnet-linear.src.inputs.LinearInputs.forward`

LinearInputs::forward(t) and LinearInputs::discount_df(t) are the two primitive carry-seam accessors on the inputs struct: forward(t) = self.spot * self.carry.forward_factor(t) (i.e. S*e^{b*t}); discount_df(t) = self.carry.discount_df(t) (i.e. e^{-r_dom*t}). Both delegate entirely to celnet_core::carry::Carry, so they are byte-identical to CarryInputs::forward / CarryInputs::discount_df — proved by forward_and_df_match_core_carry_inputs_byte_for_byte. This identity is the structural seam ensuring the linear and options leaves share one pricing substrate.

### 310. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.ndf.Ndf.pv`

CAPABILITY (linear leaf — FX forward/swap/NDF): celnet-linear::ndf::Ndf::pv is the non-deliverable-forward present-value entry of the linear-products leaf — a pure, side-effect-free closed form (delegating to pv_at over LinearInputs at the near settle tenor) with no I/O, allocation-mutation, or logging. The NDF cash-settled PV equals the deliverable forward PV (proven by ndf_pv_equals_deliverable_forward_pv), and LinearInputs::forward/discount are byte-identical to the core CarryInputs (forward_and_df_match_core_carry_inputs_byte_for_byte), so the linear capability (FX forward/swap/NDF) sits on the same carry seam as the option leaves and reaches the one contract.

### 311. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.swap.leg_rates`, `github.com-soarsa-celnet.crates.celnet-linear.src.swap.swap_points`

swap_points(inputs) returns the FX swap-point differential: far_forward - near_forward, where both legs use the same carry curve (F(t) = spot * e^{b*t}). Swap points are positive when the far rate exceeds the near rate (base currency at a forward premium, r_dom > r_for), verified by swap_points_sign_matches_carry. Returns SwapError::MissingFarLeg when far_settle_t is absent. Pure: reads only &LinearInputs, no writes.

### 312. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-linear.src.swap.pv`, `github.com-soarsa-celnet.crates.celnet-linear.src.swap.swap_inp`

swap::pv(inputs) prices a two-legged FX swap as the algebraic sum of two outright forward PVs evaluated at different settle times: near_pv = pv_at(inputs, near_settle_t) on the stated side, far_pv = pv_at(far_leg, far_t) on the opposite side (far_leg clones the inputs with side.opposite()). It returns SwapError::MissingFarLeg if far_settle_t is absent. The two-leg decomposition is independently verified by pv_equals_independent_two_leg_sum and the opposite-leg netting property by equal_dates_opposite_legs_net_to_zero.

### 313. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.channel.telemetry_channel`

PILLAR (CLAUDE.md guardrail 11 — zero-cost observability, telemetry offloads over a BOUNDED queue so the pinned hot core stays alloc/lock/log-free). `telemetry_channel(capacity)` is the single constructor of the hot-path-to-drain seam: it builds an rtrb single-producer/single-consumer RingBuffer of FIXED `capacity.max(1)` (the bounded queue) and returns the (HotProbe, TelemetryDrain) pair sharing one Arc<Shared>. The hot side (HotProbe) only pushes HotSamples into the pre-sized ring and never blocks or allocates per sample; backpressure is absorbed by dropping/counting gaps, never by stalling the pricing core. The function itself is a pure constructor — its output depends only on `capacity`, it mutates no shared/global state and has no observable side effect beyond returning the owned channel ends.

### 314. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.p99_ns`

PILLAR (CLAUDE.md guardrail 11 — mission-critical ops instrumentation with HdrHistogram p50/p99/p99.9, without diminishing hot-path performance). `LatencyRecorder::p99_ns` is the canonical p99 tail-latency readout: a pure read-only accessor returning `self.percentile_ns(99.0)` over the recorded latency histogram in nanoseconds. It reads the histogram, mutates nothing, and has no side effects — the p99/tail telemetry is computed off the bounded drain, never on the zero-alloc pricing core. This is the latency-budget observability surface referenced by docs/ARCHITECTURE.md §1.2 / docs/SCALE-OUT.md.

### 315. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.percentile_ns`

PILLAR (CLAUDE.md guardrail 11 — instrument for mission-critical ops with HdrHistogram p50/p99/p99.9, without diminishing performance). `LatencyRecorder::percentile_ns(q)` is the single quantile-readout primitive that the named p50_ns/p99_ns/p999_ns accessors all delegate to: it returns `self.hist.value_at_quantile(q / 100.0)` — the HdrHistogram value at the q-th percentile, in nanoseconds (q is a percentage, converted to a [0,1] quantile). It is a pure read-only accessor: it reads the recorded latency histogram and mutates nothing, so quantile readout never touches the zero-alloc pricing hot path (recording is offloaded; this only reads the already-merged histogram). This is the tail-latency budget surface in docs/ARCHITECTURE.md §1.2.

### 316. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.latency.LatencyRecorder.with_expected_interval`

LatencyRecorder.with_expected_interval constructs an HdrHistogram with 3 significant figures over the range [1 ns, MAX_NS] (60 s), enabling coordinated-omission-corrected recording. It gracefully degrades to an unconstrained 3-sig-fig histogram on the theoretically-impossible constructor failure, keeping mission-critical code panic-free.

### 317. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.record.HotSample.new`

HotSample.new is a const fn that initializes a HotSample with seq=0 (stamped by HotProbe.publish at enqueue time), class=ErrorClass::Ok (overridable via with_class), and _pad=0 for POD alignment. The struct is 32 bytes and Copy, satisfying the zero-alloc constraint on the hot path.

### 318. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-observability.src.record.TickRate.ticks_to_nanos`

TickRate.ticks_to_nanos converts a raw hardware-tick count to nanoseconds using round-to-nearest arithmetic in u128 to avoid overflow and sub-tick truncation bias: `(ticks * 1_000_000_000 + ticks_per_sec/2) / ticks_per_sec`. The rounding comment explicitly cites p99.9 fairness as the motivation.

### 319. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.broker_smile.high_rr_em_case_reprices`, `github.com-soarsa-celnet.crates.celnet-parity.tests.broker_smile.smile_reprices_broker_strangle`

Rows 10–11 (parity) — `smile_reprices_broker_strangle` and `high_rr_em_case_reprices` prove the broker→smile calibration reprices the market strangle (a call+put priced at a single vol σ_ATM+BF at the broker wing strikes) to 1e-8 relative / 1e-10 absolute, AND that the naive arithmetic-butterfly smile (the documented '#1 production bug' in docs/CAPABILITIES-VS-COMPETITION.md) *misprices* the same strangle by a demonstrably larger error. Asserting the naive misprice exceeds a threshold while the calibrated one is within tolerance proves the calibration is a real correction, not a tautology — on both a G10 benign slice and a high-risk-reversal EM case.

### 320. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.atm_dns_strike_is_delta_neutral`, `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.strike_delta_roundtrip_all_conventions`

Row 2 (parity) — `strike_delta_roundtrip_all_conventions` proves the convention-aware strike↔delta solver round-trips in all four FX delta conventions (SpotUnadjusted, ForwardUnadjusted, SpotPremiumAdjusted, ForwardPremiumAdjusted) for both calls and puts at the 25Δ and 10Δ wings: solve `strike_from_delta(conv, opt, target, &inputs)` then re-read `convention_delta(conv, opt, &solved_inputs)` and require agreement to 1e-9 relative / 1e-10 absolute, for ≥50 rows. `atm_dns_strike_is_delta_neutral` additionally proves the delta-neutral-straddle ATM strike satisfies call_delta + put_delta = 0 to 1e-9 in unadjusted conventions, and the ATMF strike equals the outright forward F=S·e^{(r_d−r_f)T} to 1e-12.

### 321. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.premium_adjusted_call_delta_is_guarded`

Row 3 (parity) — `premium_adjusted_call_delta_is_guarded` proves the premium-adjusted call delta non-monotone guard: (a) a target above the attainable ceiling (delta at strike K_max where ∂Δ/∂K=0, computed by `premium_adjusted_call_delta_max`) returns `Err(Unreachable)` — not a silently wrong strike — and (b) a target at 50% of the ceiling is reachable and round-trips to 1e-9. This is the non-monotone primitive Bloomberg/Fenics bury; Celnet exposes and gates it.

### 322. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.conventions.vanilla_price_matches_closed_form`

Row 1 (parity) — `vanilla_price_matches_closed_form` proves the production pricer `celnet_vanilla::price` reproduces the independent closed-form Garman-Kohlhagen formula C = S·e^{-r_f·T}·N(d1) − K·e^{-r_d·T}·N(d2), P = K·e^{-r_d·T}·N(-d2) − S·e^{-r_f·T}·N(-d1), d1=(ln(S/K)+(r_d−r_f+½σ²)T)/(σ√T), d2=d1−σ√T, computed in the test via a separate expression grouping (not the same code path), to tolerance 1e-12 relative / 1e-14 absolute across ≥14 reference-market × option-side rows. This is the pricing floor every incumbent meets behind closed doors; Celnet meets it in the open.

### 323. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.price_and_greeks_are_bit_identical`, `github.com-soarsa-celnet.crates.celnet-parity.tests.determinism.smile_and_exotics_are_bit_identical`

Row 15 (parity) — `price_and_greeks_are_bit_identical` proves bit-for-bit reproducibility (IEEE-754 `to_bits()` equality) of the price and full 13-Greek set via three non-vacuous checks: (a) reconstruct inputs from a shortest-round-trip decimal snapshot string (no shared provenance) and recompute — proves value-determinism not identity-dependence; (b) cross-thread: 8 spawned threads each rebuild inputs from the snapshot and compute independently, all must agree to the bit — catches hidden global/thread-local state; (c) committed golden-bit table for two textbook regimes (e.g. call price bits `0x4024_e6b2_e3d5_4dc0` for S=K=100, σ=20%, T=1, r_d=5%) — catches cross-run/cross-build ULP regressions. `smile_and_exotics_are_bit_identical` extends the same three-mode check to the broker smile and analytic exotic pricers.

### 324. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.barriers_match_quantlib`, `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.digitals_match_quantlib`, `github.com-soarsa-celnet.crates.celnet-parity.tests.exotics.touches_and_dnt_match_quantlib`

Rows 12–14 (parity) — `digitals_match_quantlib`, `touches_and_dnt_match_quantlib`, and `barriers_match_quantlib` prove the first-generation exotic pricers reproduce an independent QuantLib 1.42.1 reference to 1e-9 relative / 1e-10 absolute (and 1e-6 / 1e-8 for the double-barrier reflection series) across: European digitals (cash-or-nothing and asset-or-nothing, both directions), one-touch / no-touch / double-no-touch / double-touch, all eight single-barrier flavours (up/down × in/out × call/put), and the double knock-out/knock-in. The QuantLib-sourced CSV tables live in celnet-golden; this crate re-runs the checks through the public celnet-exotics API.

### 325. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.full_greek_set_matches_finite_difference`, `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.put_call_parity_across_regimes`, `github.com-soarsa-celnet.crates.celnet-parity.tests.greeks.second_order_wing_greeks_match_fd`

Rows 4–5 (parity) — `full_greek_set_matches_finite_difference` proves all nine 'core' Greeks (delta_spot, vega, rho_dom, rho_for, theta, gamma, vanna, volga, charm) agree with central finite differences of the price function to 1e-5 relative / 1e-7 absolute (first-order) or 1e-4 / 1e-6 (second-order), across ≥126 rows. `second_order_wing_greeks_match_fd` proves the remaining four (speed=∂gamma/∂S, zomma=∂gamma/∂σ, color=∂gamma/∂T, delta_forward=∂[V_fwd]/∂F) against their defining derivatives, completing the full 13-Greek set. `put_call_parity_across_regimes` proves C−P = S·e^{-r_f·T} − K·e^{-r_d·T} across ≥7 regimes.

### 326. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.accumulator_continuous_monitoring_knocks_out_more_than_discrete`, `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.lookback_closed_form_matches_mc_and_dominates_vanilla`, `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.quanto_closed_form_matches_mc_and_collapses_at_zero_correlation`, `github.com-soarsa-celnet.crates.celnet-parity.tests.structured.tarf_gap_risk_premium_is_priced_and_signed`

Rows 16–19 (parity) — four structured-product gates prove the second-generation / TARF book in the open: (16) `quanto_closed_form_matches_mc_and_collapses_at_zero_correlation`: quanto vanilla and digital closed forms reproduce an independent MC within 4·σ_MC+1e-6; at zero correlation ρ=0 the quanto drift vanishes and the quanto price equals the plain vanilla to 1e-12, i.e. `quanto_vanilla_price(opt, &e, QuantoParams::new(σ_fx,0)) == vanilla_price(opt, &i)`. (17) lookback closed forms (floating- and fixed-strike) cross-validated by MC, plus the optionality invariant lookback ≥ vanilla. (18) TARF gap-risk: FullGain settlement is strictly costlier than CappedGain, with a positive expected overshoot on FullGain and zero on CappedGain. (19) Accumulator: continuous (Brownian-bridge) monitoring knocks out more than discrete fixing-only monitoring, so fewer fixings settle.

### 327. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.example.reference_call`

reference_call(inputs: &CarryInputs) -> f64 is the canonical closed-form carry-generalized call pricer used as the ground-truth oracle in plugin-api tests. It computes: r = inputs.carry.discount_rate(), b = inputs.carry.carry_rate(), d₁ = [ln(S/K) + (b + 0.5σ²)t]/(σ√t), d₂ = d₁ − σ√t, call = S·e^{(b−r)t}·Φ(d₁) − K·e^{−rt}·Φ(d₂). This is the generalized Garman-Kohlhagen/Black formula parameterized through the carry seam: for FX Carry::FxRates, r = r_dom and b = r_dom − r_for, reproducing the exact FX two-rate arithmetic; for Carry::CostOfCarry, r and b are the stored fields directly. The function is pure (reads only &CarryInputs, routes all transcendentals through celnet_core::math: sqrt, ln, exp, norm_cdf; no I/O, no mutation, no allocation).

### 328. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.butterfly_check`

BUTTERFLY no-arbitrage hard-reject on the plugin-API smile contract (ANALYTICS-SPEC §3.4 strike-convexity axis; the SmileModel::check_no_arbitrage default path). `smile::butterfly_check(model, forward, t, strikes)` enforces strike-convexity of undiscounted call prices: over every consecutive strike triple (kl,km,kr) on a strictly-increasing grid it forms the second difference `C(kl) - 2·C(km) + C(kr)` of `undiscounted_call` at each strike's model-implied vol, and returns Err(Unsupported("butterfly arbitrage")) when that second difference is negative beyond a rounding tolerance (`is_close(.,0,1e-9,1e-12)`) — a negative call convexity is a negative risk-neutral density = butterfly arbitrage. It also hard-rejects non-finite/non-positive forward or t, fewer than 3 strikes, and any non-strictly-increasing or NaN strike (NaN compares false to its neighbour, so it fails the monotone-grid guard). Pure validator: reads its args + the model, returns PluginResult<()>, mutating nothing.

### 329. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-api.src.smile.undiscounted_call`

undiscounted_call(forward, k, v, t) computes the undiscounted Black call value off the forward: C = F·Φ(d₁) − K·Φ(d₂), where d₁ = [ln(F/K) + 0.5v²t]/(v√t) and d₂ = d₁ − v√t. When vsqt = v·√t ≤ 0 it degenerates to intrinsic (F−K)⁺. It is pure (reads only its four f64 arguments, routes sqrt/ln/norm_cdf through celnet_core::math, no I/O, no mutation, no allocation). Used exclusively by butterfly_check as the density test oracle over the model's own smile vol.

### 330. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.canonicalize`

abi::canonicalize(x: f64) -> f64 is a pure NaN-normalization function: if x.is_nan() it returns f64::from_bits(CANONICAL_NAN_BITS), otherwise it returns x unchanged. This is the single boundary canonicalization applied to every f64 crossing the host/guest interface — both on import arguments entering the guest and on the price return value exiting the guest.

### 331. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.abi.input_to_bytes`

abi::input_to_bytes(inputs: &CarryInputs) -> [u8; INPUT_BYTES] serializes a CarryInputs struct to a fixed-size little-endian byte array. The layout is: 6 × 8-byte f64 fields (spot, strike, vol, t, carry_0, carry_1) each canonicalized before encoding via to_bits().to_le_bytes(), followed by two i32 discriminants packed after the numeric block (underlying_to_abi(&inputs.underlying) at base, carry_kind at base+4). This is the sole serialization format shared between host and Wasm guest.

### 332. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel<M>.price`

CAPABILITY (plugin Tier-0 native dispatch): celnet-plugin-host::native::NativeModel<M>::price is the Tier-0 native plugin pricing dispatch — a pure, side-effect-free delegation that forwards (OptionType, &CarryInputs) to the inner PricingModel and returns HostResult<f64> with no host-side I/O, allocation-mutation, or logging. It proves a user-supplied native model and the fuel-metered Tier-2 wasm-sandboxed model both register and dispatch through the ONE PricingModel::price(opt, CarryInputs) seam of the same ModelRegistry, so native and wasm twins agree bit-for-bit through one registry (native_and_wasm_twins_agree_through_one_registry); the Tier-2 sandbox path itself is fuel-metered and is therefore deliberately NOT claimed pure here.

### 333. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.native.NativeModel<M>.price_and_greeks`

CAPABILITY (plugin Tier-0 native dispatch): NativeModel<M>.price_and_greeks is the Tier-0 native plugin-host dispatch entry behind the frozen HostModel trait — pure (&self, OptionType, &CarryInputs) -> HostResult<CarryGreeks>, forwarding to the in-process registered model with zero sandboxing overhead. Pairs with the Tier-2 wasmi fuel-metered sandbox (WasmModel) under one ModelRegistry; the native-and-wasm twins agree through that one registry. No WRITES on the dispatch path — the gate self-invalidates if the native forwarder gains a side effect.

### 334. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.insert`, `github.com-soarsa-celnet.crates.celnet-plugin-host.src.registry.ModelRegistry.model`

ModelRegistry::insert enforces a unique-id invariant: it rejects any model whose ModelDescriptor.id already exists in the registry with HostError::Model(PluginError::InvalidInput("duplicate model id")), and otherwise appends the model and its descriptor to parallel vecs, returning the new ModelId. ModelRegistry::model performs O(n) lookup by scanning the descriptor vec for the matching id, returning the corresponding entry or HostError::Model(PluginError::NotFound("model id")).

### 335. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-plugin-host.src.wasm.sandbox_config`

ADR (plugin-host sandbox = wasmi, fuel-metered, narrowed feature set). `sandbox_config()` is the single source of the guest-execution sandbox policy and is a pure builder: it constructs a fresh wasmi `Config`, enables `consume_fuel(true)` (deterministic instruction metering, the basis of the per-call FuelBudget that bounds even a `(start)` function — see WasmModel::load), and explicitly disables the unused proposals (memory64, bulk-memory, reference-types, tail-call) to narrow the accepted module surface. It reads and writes no external state — its result depends only on the wasmi defaults — so the sandbox policy is reproducible call-to-call. DECISION/RATIONALE: the Tier-2 user-plugin host is built on wasmi (a pure-Rust, no-unsafe, no-JIT interpreter) rather than wasmtime: wasmi gives deterministic fuel metering and a small, auditable, JIT-free attack surface that suits a mission-critical pricing host where a plugin must be sandboxed and time-bounded, accepting interpreter throughput for that safety. Tier-0 native models run un-sandboxed for the hot path; untrusted user code is confined here. (Memory: plugin-host=wasmi; wasmtime rejected.)

### 336. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.AtmConvention.from`

The wire→types AtmConvention mapping is a total 1:1 lift of the two ATM conventions: `From<WireAtmConvention> for AtmConvention` matches AtmForward→AtmForward and DeltaNeutralStraddle→DeltaNeutralStraddle explicitly, no wildcard. DeltaNeutralStraddle is the dominant interbank ATM (its strike is the premium-adjusted-aware F·e^{±½σ²T}, docs/CONVENTIONS.md), distinct from AtmForward (K=F). This seam carries that selection from the wire onto `celnet_types::AtmConvention` with no possibility of silent enum drift. Pure: a match returning the mapped enum, mutating nothing.

### 337. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Cut.from`

The wire→types Cut mapping is a total 1:1 lift of the two expiry cuts: `From<WireCut> for Cut` matches NewYork1000→NewYork1000 and Tokyo1500→Tokyo1500 explicitly, no wildcard. NY 10:00 is the standard interbank OTC cut; Tokyo 15:00 is standard for JPY-region/Asian business (docs/CONVENTIONS.md). The cut is per-(pair,tenor) configuration, not a global default; this seam carries it from the wire onto `celnet_types::Cut` with no silent drift. Pure: a match returning the mapped enum, no mutation.

### 338. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DayCount.from`

The wire→types DayCount mapping is a total 1:1 lift of the two day-count bases: `From<WireDayCount> for DayCount` matches Act365Fixed→Act365Fixed and Act360→Act360 explicitly, no wildcard. ACT/365-fixed is kept distinct from ACT/360 because vol-time accrual (ACT/365) must not be conflated with money-market accrual basis (ACT/360) — a deliberate separation (docs/CONVENTIONS.md). This seam carries the basis from the wire onto `celnet_types::DayCount`. Pure: a match returning the mapped enum, no mutation.

### 339. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.DeltaConvention.from`

The wire→types DeltaConvention mapping is a total, order-preserving 1:1 lift of the four FX delta conventions: `From<WireDeltaConvention> for DeltaConvention` matches every wire variant explicitly (SpotUnadjusted→SpotUnadjusted, ForwardUnadjusted→ForwardUnadjusted, SpotPremiumAdjusted→SpotPremiumAdjusted, ForwardPremiumAdjusted→ForwardPremiumAdjusted) with no wildcard arm, so the proto enum and the celnet-types enum can never silently drift — adding a delta convention on either side is a compile error here. This is the single conversion seam carrying the per-(pair,tenor) delta convention from the wire onto `celnet_types::DeltaConvention` (the convention encoded in the type, never a global default; docs/CONVENTIONS.md). Pure: a match over the input value returning the mapped enum, mutating nothing.

### 340. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.PremiumStyle.from`

The wire→types PremiumStyle mapping is a total, order-preserving 1:1 lift of the four premium quotation styles: `From<WirePremiumStyle> for PremiumStyle` matches every wire variant explicitly (DomesticPips→DomesticPips, PercentForeign→PercentForeign, PercentDomestic→PercentDomestic, ForeignPips→ForeignPips) with no wildcard, so the proto and celnet-types premium-style enums cannot drift. This carries the premium style — which decides whether the premium is paid in the FOR/base ccy and therefore carries FX risk (the `is_premium_adjusted` distinction, docs/CONVENTIONS.md) — from the wire onto `celnet_types::PremiumStyle`. Pure: a match returning the mapped enum, no mutation.

### 341. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.Settlement.from`

The wire→types Settlement mapping is a total 1:1 lift of the two settlement styles: `From<WireSettlement> for Settlement` matches Deliverable→Deliverable and NonDeliverable→NonDeliverable explicitly, no wildcard. NonDeliverable encodes an NDO that cash-settles at a published fixing (EMTA/WMR), versus a physically Deliverable option (docs/CONVENTIONS.md). This seam carries the settlement style from the wire onto `celnet_types::Settlement` with no silent enum drift. Pure: a match returning the mapped enum, no mutation.

### 342. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.cut_round_trips`

`cut_round_trips` pins the wire↔types Cut conversion as a total round-trip: for every `celnet_types::Cut` variant (NewYork1000, Tokyo1500) `Cut::from(WireCut::from(c)) == c`. This guards that the two `From` directions stay mutually inverse, so the cut convention survives a wire encode/decode unchanged — the one-contract guarantee for the expiry-cut convention (no versioning, single current mapping). Pure test: constructs values and asserts equality, mutating no external state.

### 343. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.settlement_round_trips`

`settlement_round_trips` pins the wire↔types Settlement conversion as a total round-trip: for every `celnet_types::Settlement` variant (Deliverable, NonDeliverable) `Settlement::from(WireSettlement::from(s)) == s`. It guards that the two `From` directions remain mutually inverse so the deliverable/non-deliverable (NDO cash-settled) distinction survives a wire encode/decode unchanged — the single-contract guarantee for the settlement convention. Pure test: constructs values and asserts equality, no external mutation.

### 344. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_deliverable_underlying`

PILLAR (CLAUDE.md guardrail 9 — "No versioned APIs: exactly ONE clean, current contract"; guardrail 8 — purpose-named, vendor-neutral). `validate_deliverable_underlying(underlying)` is a pure, total projection of the single wire `Underlying` oneof onto the deliverable linear-forward book: it reads only the borrowed `WireUnderlying` and returns a `Result`, mutating nothing (no WRITES edges). On the one unversioned contract it accepts exactly the deliverable leg-pair arms — Fx and Metal (decoding to `Underlying::Fx`/`Underlying::Metal`) — and for every cross-asset arm (Equity, Commodity, DigitalAsset) returns a typed `WireError::WrongUnderlying { product_family: "deliverable forward", .. }` rather than silently coercing the asset-class identity under deliverable-FX arithmetic; an absent ref yields `WireError::MissingField`. It is the deliverable-book twin of the already-governed `validate_fx_underlying` (cl_1ff652eea3ffb73e) on the same single contract — there is no schema_version, no N/N-1 negotiation: a malformed or wrong-asset request is rejected, never version-coerced. Self-invalidating: if this acquires a side effect the purity gate flips it off.

### 345. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_fx_underlying`

ADR-0007 (one clean unversioned contract — proto side). `validate_fx_underlying` is a pure, total projection of the wire `Underlying` oneof onto the FX-option product: it reads only the borrowed `WireUnderlying` and returns a `Result`, mutating nothing. It accepts the Fx and Metal arms (decoding to `Underlying::Fx`/`Underlying::Metal`), and for every cross-asset arm (Equity, Commodity, DigitalAsset) it returns a typed `WireError::WrongUnderlying` rather than discarding the asset-class identity or silently coercing it under FX arithmetic; an absent ref yields `WireError::MissingField`. The match enumerates all oneof arms explicitly (no wildcard), so adding a new asset class to the wire is a compile error here, not a silent mis-route. DECISION/RATIONALE: there is exactly ONE current wire contract and no version negotiation — the rich cross-asset `underlying` oneof carries the single asset-class discriminator, and (where a legacy `pair` key still appears) precedence is fixed at underlying≻pair (see instrument_underlying_from_json), never an N/N-1 schema_version handshake. The contract evolves in place and deploys as one uniform version. (Guardrail: no versioned APIs; one clean current contract.)

### 346. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_listed_future_terms`

On the one wire contract, a listed-future option's terms are accepted only when both maturities are well-formed and consistently ordered: `validate_listed_future_terms` requires the option `expiry_years` to be finite and strictly positive, and the future's `future_expiry_years` to be finite and `>= expiry_years` (the underlying future must outlive the option), else it returns `WireError::InvalidTerms`; a missing `future_symbol` yields `WireError::MissingField` and an out-of-range margining tag yields `WireError::UnknownEnum`. Pure: it reads only the wire option and the expiry argument and returns a `Result`, mutating nothing.

### 347. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.convert.validate_perpetual_terms`

On the one wire contract, a perpetual-option instrument must carry `expiry_years == 0` (a perpetual has no expiry): `validate_perpetual_terms` rejects any non-zero value with `WireError::InvalidTerms`. Because the check is `expiry_years != 0.0` (which is also true for NaN), a NaN expiry is rejected, never silently waved through as "no expiry". Pure: it reads only its `f64` argument and returns a `Result`, mutating nothing.

### 348. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.RateSensitivities.fx_rhos`

The wire/proto rate-sensitivity contract preserves FX's TWO-rho structure end-to-end: `RateSensitivities::fx_rhos` returns `Some((rho_dom, rho_for))` only when the rate-sensitivities oneof is the `Fx` variant, surfacing BOTH the domestic-rate sensitivity ∂V/∂r_d and the foreign-rate sensitivity ∂V/∂r_f as a distinct pair (never a single collapsed equity-style rho), and `None` otherwise. This is the on-the-wire counterpart to the engine's `celnet-vanilla::greeks` two-rho output (Garman-Kohlhagen 1983; ANALYTICS-SPEC §2.1): the foreign rate enters as the continuous dividend yield on the foreign-currency leg, the two rhos carry opposite signs, and the typed oneof prevents any client from reading a one-rho FX sensitivity. Pure: it reads only `&self.sensitivities` and constructs an Option tuple, performing no allocation, I/O, or external mutation.

### 349. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-proto.src.helpers.Underlying.as_digital_asset`

`Underlying::as_digital_asset` is the exhaustive, total projection of the underlying oneof onto its digital-asset arm: it returns `Some(&CryptoPair)` only for the `DigitalAsset` variant and `None` for every other asset class (Fx, Metal, Equity, Commodity) and for an absent ref. The match arms enumerate all variants explicitly (no wildcard), so adding a new asset class to the one contract is a compile error here rather than a silent mis-projection. Pure: it borrows `&self` and returns a borrow, mutating nothing.

### 350. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.build`, `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.BrownianBridge.new`, `github.com-soarsa-celnet.crates.celnet-qmc.src.bridge.bisect`

`BrownianBridge::new(m, t_total)` constructs the bridge plan by bisection: it first places W(T) = W(t_{m-1}) conditioned on W(0)=0 with std = sqrt(t_{m-1}), then recursively bisects interior index intervals. Each `BridgeStep` records (out, left, right, left_w, right_w, std) where left_w = 1 − frac, right_w = frac = (t_mid−t_left)/(t_right−t_left), std = sqrt((t_mid−t_left)(t_right−t_mid)/(t_right−t_left)). The resulting plan has exactly m steps covering every time index, and `BrownianBridge::build(&z, &mut path)` executes it in plan order as `path[out] = left_w·path[left] + right_w·path[right] + std·z[k]`, consuming m independent N(0,1) draws.

### 351. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.lib.rqmc_estimate`, `github.com-soarsa-celnet.crates.celnet-qmc.src.lib.splitmix`

`rqmc_estimate` derives independent per-replication scramble seeds via `splitmix(base_seed.wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)))`, where splitmix is the Murmur3/SplitMix64 finalizer. This makes each replication's scrambled Sobol sequence statistically independent of the others; the inter-replication mean and sample variance of the `replications` per-replication averages provide the estimate and its standard error (std_error = sqrt(sample_var / replications)). With replications=1 the standard error is NaN (not estimable from one replication).

### 352. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.normal.inv_norm_cdf`

`inv_norm_cdf(p: f64) -> f64` computes Φ⁻¹(p), the standard normal quantile, via a three-region Acklam rational approximation seeded at the correct asymptotic regime, followed by a single Halley-polish step: e = Φ(x) − p, u = e / φ(x), x ← x − u / (1 + 0.5·x·u). The function is pure (no I/O, no mutation, no allocation) and handles all boundary cases: p ∉ [0,1] or NaN → NaN, p=0 → −∞, p=1 → +∞. It is the sole inverse-normal transform in the RQMC path; all uniform Sobol points pass through it before being fed to BrownianBridge::build.

### 353. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolSequence.new`

`SobolSequence::new(dim)` initialises direction numbers for up to MAX_DIM dimensions from the embedded Joe-Kuo table. Dimension 1 uses the identity numbers v_k = 2^{31-k} (van der Corput sequence). Dimensions 2..=dim apply the Joe-Kuo recurrence on 32-bit left-justified integers V[k] = m_k · 2^{32-k}: for k > s, V[k] = V[k-s] XOR (V[k-s] >> s) XOR ⊕_{i=1}^{s−1} a_i·V[k-i], where a_i = (a >> (s-1-i)) & 1. The constructor panics for dim=0 or dim > MAX_DIM and is otherwise pure: no side effects, no I/O, no shared mutation.

### 354. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.SobolStream<'_>.next_point`, `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.u32_to_open_unit`

`SobolStream::next_point(&mut self, out: &mut [f64])` advances the stateful Gray-code Sobol iterator by one point. For i=0 all state integers are zeroed (the all-zero Gray-code point); for i>0, c = trailing_zeros(i) and each coordinate j XORs in direction number v[j][c]. Each state integer is then Owen-scrambled per dimension and mapped to (0,1) via `u32_to_open_unit(u) = (u as f64 + 0.5) * 2^{-32}`, guaranteeing the open unit interval (never 0 or 1, so inv_norm_cdf never produces ±∞). The mutation is entirely to self.state and out — no allocation, no shared mutation beyond the stream itself.

### 355. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-qmc.src.sobol.owen_scramble_u32`

`owen_scramble_u32(x: u32, dim: u64, seed: u64) -> u32` applies a bit-by-bit Owen scramble to Sobol integer `x` for dimension `dim` under scramble key `seed`. It processes all 32 bits MSB-first: for each depth `d`, flip = LSB of mix64(base ^ (d<<40) ^ (prefix<<1)), out_bit = in_bit XOR flip, prefix accumulates emitted bits. The function is pure (reads only its three scalar arguments, no shared mutation) and deterministically preserves the dyadic-prefix structure required for the scrambled sequence to remain (t,s)-equidistributed: the scrambled prefix of k bits depends only on the original k-bit prefix, never on deeper bits.

### 356. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.candidate_log_ok`

SAFETY/CONSENSUS — Raft election restriction / log up-to-date check (deliverable: replog-quorum-durability). NodeCore::candidate_log_ok is a pure, deterministic read-only predicate implementing the §5.4.1 leader-completeness pre-condition: a vote is granted only if the candidate's log is at least as up-to-date as the voter's — first by last-term (a strictly higher candidate term wins), and on equal last-terms by length (cand_len >= my_len), with the EMPTY_LOG sentinel mapped to length 0 and u128 arithmetic preventing index overflow. It reads only &self.log (last_term/last_index) plus the two candidate scalars; no mutation, I/O, or allocation. This is the complementary half of leader_advance_commit: together they guarantee a committed entry is never lost to a stale-log leader. Self-invalidates if the (term-then-length) ordering is weakened.

### 357. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.election.NodeCore.leader_advance_commit`

SAFETY/CONSENSUS — Raft commit safety (the §5.4.2 leader-completeness rule): NodeCore::leader_advance_commit advances the commit watermark only for a log index n that satisfies BOTH conditions — (1) it is replicated on a quorum (holders = 1 self + peers whose match_index >= n, counted against majority = cluster_size/2 + 1), AND (2) the entry at n is from the leader's OWN current term (self.log.term_at(n) == Some(current_term)); entries from earlier terms are skipped and never directly committed. The guard `if self.role != Role::Leader { return }` makes commit-advancement a leader-only action. This is what stops a committed-then-uncommitted divergence under leader churn: a bare-majority replication of a stale-term entry does not commit. (This is a leader-state-advancing method, not a side-effect-free accessor — the invariant is the majority-AND-current-term gating it enforces, verified against leader_advance_commit_requires_a_current_term_majority.)

### 358. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.persist.decode`

Raft persistent-state corruption safety (deliverable: replog-quorum-durability). persist::decode is a pure, deterministic deserializer: given a byte slice it validates exact RECORD_LEN, recomputes crc32 over the body and rejects any mismatch by returning None, then reconstructs (current_term, voted_for, commit_index) with no I/O and no mutation of any shared state. Purity + CRC validation is the safety invariant — a torn or bit-flipped persisted vote/term/commit record can never be silently accepted, so quorum-commit durability and single-vote-per-term election safety survive a crash mid-fsync. Pure (no WRITES edges); self-invalidates on change.

### 359. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-replog.src.persist.encode`

SAFETY — Raft persistent-state encode is the CRC-protected inverse of decode (deliverable: replog-quorum-durability). persist::encode(&PersistentState) is a pure, deterministic serializer producing a fixed RECORD_LEN frame: it lays out current_term, voted_for (value + present-flag), commit_index (value + present-flag) in little-endian with explicit 2-byte CRC alignment padding (debug_assert on BODY_LEN), then appends crc32(body). Because the byte layout and CRC are a deterministic function of the state alone — no I/O, no mutation, no clock/RNG — encode is the exact round-trip partner of the already-claimed persist::decode: a flipped byte recomputes a different CRC and decode rejects it, so torn or corrupt term/vote/commit records read as absent rather than as forged consensus state. Self-invalidates if the layout or CRC coverage changes.

### 360. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-rfq.src.internal.InternalPricerSource.new`, `github.com-soarsa-celnet.crates.celnet-rfq.src.internal.InternalPricerSource.two_way`

InternalPricerSource::two_way(&self) -> TwoWay is the symmetric mid-spread quote constructor: bid = mid - half_spread, offer = mid + half_spread. The constructor clamps half_spread to max(half_spread, 0.0), so a negative input is silently zeroed, guaranteeing bid <= offer for any finite mid. The function reads only &self and constructs a new TwoWay with no I/O or mutation.

### 361. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.SideKey<'_>.partial_cmp_total`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.rank_side`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.side_key`

rank_side(rows, now_nanos, side) is the pure best-LP selector: it filters rows to those whose valid_until_nanos >= now_nanos (last-look gate — stale quotes are categorically excluded from winning), then picks the min under SideKey::partial_cmp_total. The SideKey for Bid negates the price (-bid) so that min-selection gives the highest bid; for Offer the price is taken straight so min gives the lowest offer. Tie-break is lexicographic: (better_price, earlier epoch_nanos, smaller lp_id) — fully deterministic with no NaN ambiguity (non-finite prices lose to any finite price via partial_cmp_total). Returns None when all rows are stale or the slice is empty.

### 362. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.check_winner`

check_winner(rows, side, winner) is the post-ranking consistency guard: if a winner LP-id was declared it MUST appear in the responder rows, else PanelError::WinnerNotAResponder is returned. This prevents a phantom winner — an LP-id that timed out or declined — from appearing in the final RankedPanel. The function reads only its arguments, allocates only the error string on the failure path, and always returns Ok(()) when winner is None.

### 363. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature_class`

curvature_class(buckets, gamma) -> SbmCharge applies the FRTB MAR21.5.2 cross-bucket curvature aggregation under all three correlation scenarios. For each scenario: K_total = sqrt(max(0, sum_b(K_b^2) + sum_{b≠c}(gamma_scaled^2 * psi(CVR_b,CVR_c) * CVR_b * CVR_c))) where psi(CVR_b,CVR_c) = 0 iff both CVR are negative (MAR21.5.2(4)), else 1; gamma is squared (not linear) for curvature; and gamma is scenario-scaled before squaring. Returns SbmCharge{high, medium, low}.

### 364. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.curvature_legs`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.vanilla_curvature_legs`

curvature_legs(pricer, positions, rw) -> (cvr_up, cvr_down) is the FRTB MAR21 curvature CVR computation for a vanilla node: base PV and two relative spot reprices (×(1±rw)), linear term = sum_i(delta_spot_i × notional_i × rw × spot_i). CVR_up = -((reprice_up - base) - linear); CVR_down = -((reprice_down - base) + linear). Only the spot is shocked; carry, vol, time, and strike are held fixed. The same formula is implemented in vanilla_curvature_legs via node_value/node_value_shocked helpers, which is the canonical path used for FRTB curvature bucket construction.

### 365. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.quadratic_form`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.correlation_weighted_vega`

quadratic_form(ws: &[f64], rho: F) -> f64 computes the FRTB intra-bucket capital formula: sqrt(max(0, sum_i(ws_i^2) + sum_{i<j}(2*rho(i,j)*ws_i*ws_j))). It is the kernel used for both the delta/vega SBM bucket charge (via SbmParams::class_charge) and the vega-bucket correlation_weighted_vega function. The max(0,·) guard prevents imaginary results when the cross-term sum dominates the diagonal (can occur under the Low correlation scenario where scaled ρ can be negative).

### 366. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.frtb.residual_addon`

residual_addon(instruments: &[ResidualInstrument]) -> f64 is the FRTB RRAO (Residual Risk Add-On) charge: a pure linear sum of |notional_i| * kind.weight() over all residual instruments. ResidualKind::OtherResidual carries weight 0.001 (10bp); ResidualKind::ExoticUnderlying carries 0.01 (100bp); ResidualKind::None carries 0.0 (vanilla, excluded). No correlation, no squaring — the RRAO is a gross-notional additive charge by BCBS design.

### 367. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.PositionSensitivity.taylor_pnl`

DELIVERABLE risk/pnl-attribution = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/L finding "P&L attribution (Greeks-based P&L explain) exists nowhere in the platform"; reconciled against the live graph — it now EXISTS as celnet-risk-cube). `PositionSensitivity::taylor_pnl(scenario)` is a pure, side-effect-free Greeks-based P&L-explain primitive: it returns the second-order Taylor P&L of a position under a Scenario as delta_spot·dS + ½·gamma·dS² + vega·dvol + ½·volga·dvol² + vanna·dS·dvol + discount_rho·discount_abs + carry_rho·carry_abs (dS = spot·spot_rel), reading only its own sensitivity fields and the scenario — no writes, no allocation, no I/O. This is exactly the cross-Greek P&L explain the competitive positioning claims; it is invoked by the risk-cube non-additive roll-up. SELF-INVALIDATING: any change to taylor_pnl's body that introduced a write/allocation/I/O side effect (e.g. a stateful attribution accumulator) would flip the WRITES gate and stale this claim, and if the Greeks-based explain were ever removed/relocated the anchor would unresolve.

### 368. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.historical_var_es`, `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.node_pnl`

historical_var_es(pricer, positions, scenarios, alpha) -> VarEs computes full bump-and-revalue historical VaR/ES: for each Scenario it calls node_pnl (sum of position_pnl over all positions through the CarryPricer seam), collects a Vec<f64> of per-scenario P&Ls, then delegates to the shared quantile_var_es kernel. No Greeks or Taylor approximation — every scenario is a full reprice. Deterministic given identical scenario ordering; returns VarEs{var:0,es:0} for an empty scenario slice.

### 369. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.quantile_var_es`

quantile_var_es(pnl: &mut [f64], alpha: f64) -> VarEs is the single shared quantile kernel used by BOTH the historical full-reprice path (historical_var_es) and the sensitivity Taylor path (sensitivity_var_es). It sorts pnl ascending, computes tail = floor((1-alpha)*n).max(1).min(n), then ES = -(sum(pnl[..tail]) / tail) and VaR = -pnl[tail-1], both floored at 0. The tail index is floor not ceil, so the boundary is strict — a loss exactly at the alpha quantile is excluded from the ES average. The function takes a &mut slice (in-place sort) and has no I/O or shared-state side effects.

### 370. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity_profile_is_adjoint_greeks_scaled_by_notional`

PositionSensitivity::sensitivity_profile carries adjoint Greeks (delta_spot, gamma, vega, volga, vanna, discount_rho, carry_rho) scaled by notional_base. The scaling is bitwise-exact: each field equals the corresponding adjoint Greek multiplied by notional, confirmed by to_bits() equality in the test. carry_rho = -(rho_for * notional) — a sign flip from the raw Greek so that a positive carry_rho always means sensitivity to the foreign rate in the direction that increases PV.

### 371. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-cube.src.nonadditive.sensitivity_var_es`

sensitivity_var_es(pricer, positions, scenarios, alpha) -> VarEs is the adjoint (sensitivity-based) VaR/ES path. It makes ONE Greeks sweep via node_sensitivities (one CarryPricer call per position), then for each Scenario approximates node P&L as sum_i(PositionSensitivity_i.taylor_pnl(s)) — a second-order Taylor expansion in spot and vol shocks. Delegates to the same quantile_var_es kernel as the historical path. The separation of the single-sweep sensitivity computation from the per-scenario summation is the efficiency invariant: O(P) pricer calls instead of O(P×S).

### 372. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetReducer.fan_in_additive`

`FleetReducer::fan_in_additive` computes the additive firm-node roll-up — net Greeks + vega ladder — by initialising from the first shard's `local_aggregate` then calling `merge_additive` on each subsequent shard in ascending-replica order. An empty shard list returns `Cube::new().firm_aggregate(pillars)` rather than failing. The method is pure with respect to external state (no I/O, no shared mutation); its output depends only on `&self` and `pillars`. This is the cheap path: additive Greek/vega aggregation with O(shards) work and no constituent-position allocation.

### 373. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.FleetTopology.parse`

`FleetTopology::parse(mode, backends)` is the startup topology selector: it returns `FleetTopology::Distributed { endpoints }` only when `mode == "distributed"` AND the comma-separated `backends` string yields at least one non-empty trimmed endpoint; otherwise it always falls back to `FleetTopology::InProcess`. There is no third variant and no error path — an invalid or empty distributed config silently degrades to in-process rather than failing. Pure: reads only its two `&str` inputs, returns `FleetTopology`, no mutation or I/O.

### 374. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.natural_owner_of`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_key_of`

`partition_key_of(fact)` is the canonical HRW partition key function: it builds a `PartitionKey` from the fact's underlying currency pair (`partition_pair_of(&fact.key.underlying)`) scoped by the entity tenant id (`TenantId(u64::from(fact.key.entity.0))`). The two-component key (pair + tenant) ensures co-residency of same-entity same-pair risk across shards — the load-bearing routing invariant pinned by the `entity_pair_cell_is_co_resident` test. Pure: reads `&RiskFact`, returns `PartitionKey`, no mutation.

### 375. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_facts`, `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_facts_with`

`partition_facts_with` routes each `RiskFact` to its HRW-natural owner using a caller-supplied `PartitionStrategy`, then sorts shards by ascending `ReplicaId` for deterministic reduction order. It fails closed — returning `Err(RouteError::EmptySet)` — if the replica set is empty or if `natural_owner` returns `None`. The sort-by-key at the end (`shards.sort_by_key(|s| s.replica().0)`) is the single invariant that makes `fan_in_additive`'s summation order reproducible regardless of insertion order. Pure in the sense that for identical `(facts, replicas, strategy)` inputs the same `FleetReducer` shard partition is produced; no I/O, no global mutation.

### 376. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.lib.partition_facts`

PILLAR (CLAUDE.md guardrail 6 + 11 — "scale to investment-banking-sized portfolios"; horizontal scale-out / many-instrument batch). `partition_facts(facts, replicas)` is the pure entry that spreads a risk-fact set across the live replica fleet: it is a thin, side-effect-free delegation to `partition_facts_with(.., partition_key_of)` (reads only its two borrowed slices/sets, mutates nothing, no I/O, no WRITES edges). Each fact is assigned to the HRW `natural_owner` of its partition key, producing a `FleetReducer` whose logical shards form a DISJOINT cover of the input (every fact lands in exactly one shard — verified by `partition_is_disjoint_cover`) with a deterministic reduction order (shards sorted by ascending replica id), so a fleet of N nodes reduces a firm-sized portfolio to the SAME aggregate as a single node bit-for-bit (`single_shard_fan_out_is_bit_identical`). This is the data-parallel scale-out seam that makes IB-sized cube/risk reduction horizontally partitionable without changing the answer. Self-invalidating: any side effect introduced into the partitioning entry flips the purity gate.

### 377. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.PositionRisk.carry`

`PositionRisk::carry` is a `const fn` total constructor for a cross-asset carry-priced position: it moves the underlying, option type, base notional and `CarryInputs` into the struct and fixes `quoted_delta` and `premium_style` to `None`. Being `const fn` it is side-effect-free by construction (no allocation, I/O, or mutation of external state) — a pure normalization entry point into the risk-cube leaf model.

### 378. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize`

`canonicalize(pos)` is the zero-configuration entry point for cross-asset Greek projection: `pub fn canonicalize(pos: &PositionRisk) -> Result<CanonicalLeaf, CarryPriceError> { canonicalize_with(&crate::AssetPricer, GreekEngine::default(), pos) }`. It is a pure forwarder (no writes, no I/O) that fixes the pricer to the static `AssetPricer` dispatch table and the engine to the analytic default. Every caller that does not need to inject a custom pricer or the adjoint engine variant goes through this single entry. Pure: it reads only its `&PositionRisk` argument and returns `Result<CanonicalLeaf, CarryPriceError>`, mutating nothing.

### 379. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.leaf.canonicalize_with`

CAPABILITY (carry-seam deliverable, cross-asset Greeks projection): canonicalize_with is the single pure seam that projects any asset class's leaf Greek strip into the carry-neutral CanonicalLeaf — one virtual price_greeks call through the CarryPricer trait (asset class matched inside the leaf adapter, not in this hot path, per ADR-0008), the FX-only adjoint engine taken only for an FX/metal underlying and otherwise the analytic strip, canonical spot-unadjusted/premium-excluded delta re-derived through the named convention for FX, and every Greek scaled by notional. Pure: builds CanonicalLeaf from (pricer, engine, pos) refs with no WRITES; self-invalidates on any side-effecting change to the projection.

### 380. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.add_leaf_delta`, `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.CurrencyExposure.in_numeraire`, `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.Numeraire.from_leaves`

`Numeraire::from_leaves` converts a slice of `CanonicalLeaf`s into a single-numeraire `Numeraire` struct: for each leaf it nets the spot-unadjusted premium-excluded delta into a `CurrencyExposure` vector (both base and quote legs for FX/metal pairs; quote leg only for cross-asset underlyings), accumulates `premium_numeraire` and `vega_numeraire` by converting each leaf's `premium_quote` and `vega` through the `SpotResolver` via `convert()`. A missing FX rate returns `Err(NumeraireError::MissingRate)` and an invalid rate (non-finite or ≤ 0) returns `Err(NumeraireError::InvalidRate)` — neither silently drops to zero. The delta_numeraire is the scalar sum of the full `CurrencyExposure` vector converted to numeraire. The CAP overflow (> 32 distinct currencies) is caught by `debug_assert!`.

### 381. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.numeraire.convert`

`convert(amount, ccy, resolver)` is the single atom for currency conversion into the reporting numeraire: it calls `resolver.rate_into_numeraire(ccy)` returning `Option<f64>`, fails with `NumeraireError::MissingRate(ccy)` if absent, then hard-validates the rate is finite and strictly positive (fails with `NumeraireError::InvalidRate(ccy)` otherwise), and returns `amount * rate`. No zero-rate silent pass-through, no NaN propagation. Pure: reads its three args, returns `Result<f64, NumeraireError>`, mutates nothing.

### 382. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.EquityLeaf.lower`

`EquityLeaf::lower` enforces a typed carry-seam discipline: it accepts only `Carry::CostOfCarry { r, b }` under an equity underlying and hard-rejects `Carry::FxRates` (which would silently mis-map FX two-rate carry onto an equity pricer) with `CarryPriceError::UnsupportedCarry`. The dividend yield is reconstructed as `q = r - b` (i.e. `b = r - q` in cost-of-carry form) and passed as the equity pricer's `q` argument with `repo = 0.0`. An equity underlying carrying `Carry::FxRates` is a malformed input, not a silent approximation.

### 383. `invariant:pure` (draft)
Anchors: `github.com-soarsa-celnet.crates.celnet-risk-normalize.src.pricer.dispatch`

`dispatch` is the asset-class router that implements the no-silent-fallback contract: it iterates over the static `LEAVES` array calling the supplied closure on each `&dyn CarryPricer`; on `Ok(v)` it returns immediately; on `Err(UnsupportedCarry)` it records that error as the most informative failure (it wins over the generic `UnsupportedUnderlying` default); on `Err(UnsupportedUnderlying)` it skips silently. If no leaf claims the underlying it returns `Err(UnsupportedUnderlying)` or `Err(UnsupportedCarry)` — never a silent proxy price. The docstring states this explicitly: "Never returns a silent fallback price."

### 384. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.hash.fold64`

fold64(acc, lane) chains one 64-bit lane into a running accumulator via: rotated = acc.rotate_left(23); mix64(rotated.wrapping_add(mix64(lane))). The 23-bit rotation before addition ensures successive lanes occupy different bit positions before mixing, defeating trivial cancellation of equal lanes. fold64 is order-sensitive: fold64(fold64(0,1),2) != fold64(fold64(0,2),1) (tested by fold_is_order_sensitive).

### 385. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.hash.mix64`

mix64 is the splitmix64 avalanching finalizer: z = (z ^ (z>>30)) * 0xbf58476d1ce4e5b9; z = (z ^ (z>>27)) * 0x94d049bb133111eb; z ^ (z>>31). It is bijective (never collapses distinct inputs), branch-free, const-evaluable, and produces ~32-bit average Hamming distance on single-bit input perturbations. This property makes per-replica rendezvous weights behave as independent uniform draws.

### 386. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.hash.rendezvous_weight`

PILLAR (CLAUDE.md guardrail 6 + 11 — "Design for horizontal scale-out from day one"; "scale-out aware"). `rendezvous_weight(replica_seed, key_digest)` is the pure deterministic core of the platform's horizontal scale-out: a `const fn` computing the Highest-Random-Weight (HRW / rendezvous-hashing) score for one (replica, partition-key) pair as `mix64(fold64(replica_seed, key_digest))`. It reads only its two u64 inputs and returns a u64 — no allocation, no I/O, no mutation, no WRITES edges — so it is referentially transparent and the per-replica scores are reproducible on every node. This is the primitive that makes book/risk sharding deterministic and minimal-disruption under membership change: `PartitionMap::natural_owner` takes the argmax of this weight over the live replica set to assign each partition key its stable owner, so adding/removing a replica re-homes only the keys whose argmax moved (the HRW property), never a global reshuffle. Self-invalidating: if the entanglement/mixing changes (anything beyond a pure two-u64 fold) the WRITES gate flips this claim off.

### 387. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.key.PartitionKey.digest`

PartitionKey::digest() produces a deterministic u64 fingerprint by: (1) packing the six ASCII bytes of the CCY pair into a u64 lane, (2) avalanche-mixing it with mix64, (3) folding in the tenant id (tagged with a high-bit presence sentinel) and the book id in sequence via fold64. The result is fixed for a given (pair, tenant?, book?) triple and is the sole input to all routing weight computations. Distinct pairs always produce distinct digests (tested by distinct_pairs_distinct_digests); adding/changing a subkey changes the digest (tested by subkeys_change_digest).

### 388. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap<'a>.natural_owner`

PILLAR (CLAUDE.md guardrail 6 + 11 — horizontal scale-out / shard ownership). `PartitionMap::natural_owner(key)` is the pure, side-effect-free realization of HRW shard assignment: for a partition key it digests the key, scores every live replica with `rendezvous_weight(replica.seed, digest)`, and returns the argmax `ReplicaId` — ties broken deterministically by the smaller replica id (`.then_with(|| a.0.0.cmp(&b.0.0))`) so the owner is a total, deterministic function of (key, membership). It reads only `&self` (the replica set) and the key, allocates nothing in the hot path and has no WRITES edges. This is the routing decision that lets the platform fan a book/risk workload across N nodes with a stable, minimal-disruption owner per key (the day-one horizontal scale-out requirement): every node computes the same owner independently, no central coordinator, and a membership change re-homes only the keys whose argmax moved. Self-invalidating: any side effect or non-deterministic tie-break introduced here flips the WRITES/purity gate.

### 389. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-router.src.map.PartitionMap<'a>.route`

PartitionMap::route() determines the natural owner in a single linear pass over all replicas by maintaining an argmax of rendezvous_weight(r.id.seed(), digest), with a deterministic tie-break on id (lower id wins when weights are equal). It then applies the three-tier cascade: (1) natural owner up → Primary; (2) owner down, declared standby healthy → Standby; (3) otherwise re-runs the argmax restricted to up replicas → HrwFallback. This guarantees that only 1/N keys move in expectation on a single-replica failure, and surviving keys (natural owner still up) are never disturbed.

### 390. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.decode_settlement_style`

Wire→domain SettlementStyle decode (docs/CONVENTIONS.md determinism + docs/INTERFACES.md enum contract): decode_settlement_style is a pure total function of the i32 wire tag — it round-trips through celnet_proto::SettlementStyle::try_from then the domain SettlementStyle, lowering Linear→CryptoSettlementStyle::Linear and InverseCoin→CryptoSettlementStyle::InverseCoin; an out-of-range tag is a typed PriceError::UnknownEnum, never a silent default. No writes/allocation/IO; deterministic. Self-invalidates if the enum mapping or error path changes (WRITES gate).

### 391. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.is_cross_asset`

CAPABILITY (cross-asset carry-seam reach): is_cross_asset is the pure routing predicate that decides — by Underlying variant alone (Equity | Commodity | DigitalAsset) — which instruments leave the FX vanilla path for the cross-asset leaf engines (equity generalized-BSM, commodity Black-76, crypto inverse/linear) on the shared carry seam. FX/metal stays on the vanilla path (metal lease rate modelled as the FX foreign rate), byte-identically. Side-effect-free classifier: reads only the &Underlying, returns bool, no WRITES — the gate self-invalidates if the variant set or the routing shifts.

### 392. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price_listed_future_option`

DELIVERABLE proto/new-payoff-shapes (ListedFutureOption arm) = DONE (RC cut a0817d6; arm 31). `price_listed_future_option(o, market, expiry)` is the side-effect-free server lowering of a wire listed-future option onto the Black-76 commodity-on-future leaf: it first runs the canonical term validator (`validate_listed_future_terms` — present `future_symbol`, known margining tag, `future_expiry_years >= expiry_years > 0`), rejects a non-positive/non-finite strike as `PriceError::Domain` and an out-of-range margining tag as `PriceError::UnknownEnum` (never clamped/defaulted), maps the margining enum to `CommodityMargining`, builds `CommodityInputs::on_future(spot, strike, vol, expiry, r_dom)`, and returns the leaf Greeks via `greeks_with_margining`. Pure: it reads only `(o, market, expiry)` and returns `Result<Priced, PriceError>`, mutating no external state. The booked future_symbol is contract identity, not a pricing input. This is the second of the two genuinely-new payoff shapes the backlog tracked as OPEN; now built + golden/parity-gated across all 5 clients, with futures-style honest-zero discount-rho asserted.

### 393. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.pricer.product_name`

CAPABILITY (carry-seam deliverable, one-contract product-family reach): product_name is the pure total over the full Product enumeration — it names all 24 product arms reachable through the one unversioned contract (vanilla, strategy, single/double barrier, digital, touch, variance_swap, volatility_swap, asian_option, forward_start, cliquet, quanto, tarf, pivot, accumulator, lookback, window_barrier, american, basket, fx_forward, fx_swap, ndf, perpetual_option, listed_future_option) with no fallback arm. Pure: maps &Product to a &'static str with no WRITES; the exhaustive match self-invalidates the moment a product arm is added or removed.

### 394. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.pin.resolve_pinned_vol`

Surface-version pinning REJECTS an unknown version rather than silently pricing off the live surface. `resolve_pinned_vol` short-circuits to the live market (no echo) when no surface_version is pinned; otherwise it requires an FX `underlying` (else invalid_argument) and looks the pin up via book.pinned_vol. Three outcomes are total: Ok(Some(vol)) ⇒ price against the marked vol and echo the version; Ok(None) ⇒ the version EXISTS but did not mark THIS pair, so keep the live vol yet still echo the (valid) version; Err(PinError::UnknownVersion) ⇒ the version was never marked, returned as Status::failed_precondition — the pin is refused, never honoured against an arbitrary surface. This makes a pinned RFQ/RFS deterministic: it reprices against the exact marked model or fails closed. Pure: reads book/version/instrument/market and returns Result<PinnedVol, Status>, constructing new PinnedVol/MarketContext values (with_vol) and mutating no external state.

### 395. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.services.pricefanout.spot_at`

`spot_at(seed, tick_seq, base_spot)` is a pure deterministic spot-price generator for the price fan-out stream. It computes: `mixed = splitmix64(seed XOR (tick_seq * 0x2545_F491_4F6C_DD1D))`, then `u = unit_signed(mixed)` (a signed uniform in (-1,1)), and returns `base_spot * (u * STREAM_BUMP + 1.0)`. The same `(seed, tick_seq)` pair always produces the same price; distinct pairs produce independent draws.

### 396. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.instrument_underlying_from_json`

One clean unversioned contract with a single underlying discriminator: when decoding an instrument, `instrument_underlying_from_json` treats the rich cross-asset `underlying` oneof as authoritative whenever it is present and non-null (it carries the asset-class discriminator), and consults the legacy FX `pair` key only when no `underlying` is present — projecting it to `Underlying::fx`. There is exactly one precedence order (underlying ≻ pair), not a versioned negotiation. Pure: it reads the JSON map and returns a `Result`, mutating no state.

### 397. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.leg_from_json`

DELIVERABLE proto/strategy-per-leg-expiry = OPEN (Round-2 P3/M finding; reconciled against the live graph — still genuinely open at this round). `leg_from_json` is the pure (side-effect-free) server decoder of one strategy/structure leg off the JSON wire: it reads only the borrowed serde Value and constructs a `Leg { option_type, strike, side, ratio }`, mutating no shared state. It is the current-state witness that the wire `Leg` carries NO per-leg expiry/tenor field — every leg shares the enclosing Instrument expiry — so single-expiry-only multi-leg structures (a 1M-vs-3M calendar/diagonal spread cannot be booked as one net-premium ticket) remain unexpressible on the one unversioned contract. SELF-INVALIDATES: when a per-leg `tenor`/`expiry` field is added to the wire Leg and decoded here, this decoder's content hash changes and the claim flips stale, signalling the gap closed.

### 398. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.mark_surface_request_from_json`

DELIVERABLE surface/crypto-strike-axis-surfacing = OPEN (first post-RC fast-follow; RC anchor a0817d6). `mark_surface_request_from_json(o)` is the pure JSON→MarkSurfaceRequest decoder and it documents the CURRENT wire shape of the mark-surface contract: it accepts ONLY `pair` (optional), a required `broker_quotes` array of delta-space (RR/BF) broker quote sets, `conventions`, and an optional `smile_model`. There is NO `quote_basis`/`StrikeQuoteSet` strike-axis quote field — so although the strike-axis surface LEAF (fit_strike_slice/strike_surface) is built and parity-gated, a crypto surface STILL cannot be marked by strike from any client. Closing the fast-follow means adding a strike-axis quote oneof here (and the SurfaceEdge strike-slice ingestion → strike_surface), which will change this function's node content and STALE this claim — the staleness firing IS the done-signal for the surfacing deliverable. Pure: it reads only the JSON map and returns `Result<MarkSurfaceRequest>`, mutating nothing.

### 399. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.codec.pivot_from_json`

DELIVERABLE pivot-wire-surfacing = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P1/M finding "Pivot TRA is engine-only — no proto arm, no golden vector, no parity row, unreachable from any client"; reconciled against the live graph). `pivot_from_json` is the pure (side-effect-free) server decoder that lowers a wire pivot instrument off the JSON edge into the celnet-exotics `Pivot { option_type, strike, pivot, target, leverage, redemption, schedule, mc_pairs, mc_seed }` MC spec — proving the pivot TARGET-redemption-accumulator product is now reachable on the one unversioned contract (engine celnet-exotics::pivot::pivot_tra_price, GUI pricePivot, golden/parity celnet-parity::pivot_wire with the pivot==strike→TARF degenerate collapse and code-disjoint MC oracle). SELF-INVALIDATES on any change to this decoder.

### 400. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-server.src.ws.limits.transport_config`

DELIVERABLE ws-edge-resource-caps = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2/3 P3/S finding "WS edge accepts connections with default tungstenite limits (64 MiB messages) — no explicit frame/message caps"; reconciled against the live graph — the streaming edge now installs explicit caps via celnet-server::ws::limits). `transport_config()` is a pure, side-effect-free builder that returns the tungstenite WebSocketConfig the WS accept path uses: it sets max_message_size = Some(TRANSPORT_MESSAGE_CAP_BYTES), max_frame_size = Some(TRANSPORT_MESSAGE_CAP_BYTES) and max_write_buffer_size = MAX_WRITE_BUFFER_BYTES (overriding the 64 MiB tungstenite default), with no writes/allocation/I/O. Its sibling oversize_close / oversize_reject_text emit the typed 1009 (CloseCode::Size) reject, and cap_ordering_holds pins the contract message fits under the cap — closing the bounded-resource-discipline deviation on the streaming edge. SELF-INVALIDATING: if the WS edge stopped supplying an explicit bounded config (anchor removed/renamed) or the builder grew a side effect, this claim would unresolve or flip the WRITES gate.

### 401. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.ArbitrageReport.is_arbitrage_free`

Analytics-correctness (arbitrage-gate deliverable): celnet-surface::arbitrage::ArbitrageReport::is_arbitrage_free is the pure hard-reject predicate combining all three no-arbitrage axes within tolerance tol — butterfly/density (min_butterfly ≥ −tol), vertical/call-spread monotonicity (max_vertical_increase ≤ tol), and risk-neutral density positivity (min_density ≥ −tol). A slice/surface is accepted only when every axis is within tol; any single breach makes the report not arbitrage-free. Pure: deterministic in (&self, tol), reads only the report's reduced extrema, no WRITES edges.

### 402. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.check_slice`

`check_slice` is the per-slice butterfly + vertical no-arbitrage primitive (ANALYTICS-SPEC §3.4) that VolSurface::arbitrage_report calls at each sampled maturity. Over a strictly-ascending strike grid it returns an ArbitrageReport with `min_density` (the minimum second-difference risk-neutral density (down−2·mid+up)/h² via implied_density; a negative density is a butterfly violation), `min_butterfly` = the h²-scaled butterfly spread, and `max_vertical_increase` = the largest call-price rise from a lower strike to the next (a positive increase is a vertical-spread violation, since calls must be monotone non-increasing in K). It asserts grid.len()≥3, h>0, strictly-ascending strikes, and grid[0]>h as preconditions. This is the slice-local half of the surface arbitrage gate; the calendar (cross-tenor) dimension is checked separately. Pure: it reads the smile + grid + scalars and returns the report value, mutating nothing.

### 403. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward_call`

Analytics-correctness (arbitrage-gate deliverable, vertical/calendar oracle): celnet-surface::arbitrage::forward_call is the pure undiscounted Black forward-call value F·N(d1) − K·N(d2) evaluated at the smile's own implied vol σ(K,F,t). It is the closed-form oracle the arbitrage gates are checked against: its K-derivative is −N(d2) ∈ [−1,0] (the vertical/call-spread bound, forward_call_strike_slope), and its second K-difference is the butterfly/density check; calendar-monotonicity is verified by comparing this value across maturities. Pure: deterministic in (&Smile, strike, forward, t), no WRITES edges.

### 404. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.arbitrage.forward_call_strike_slope`

`forward_call_strike_slope` returns the sticky (∂σ/∂K-ignoring) strike-derivative of the forward call price as `-Φ(d2)`, where `d2 = d1 - σ√t` from the smile's implied vol at the strike. Since `Φ ∈ [0,1]`, the returned slope lies in `[-1, 0]` — the no-arbitrage bound on a call's monotone-decreasing strike profile. Pure: it reads the smile and scalar inputs and returns an `f64`, mutating nothing.

### 405. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.build_model_smile`

CAPABILITY (carry-seam deliverable, smile-family reach): build_model_smile is the single pure selector that reaches all FIVE smile families through one contract — MarketHedge (vanna-volga market-hedge), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each routed to its calibrator (build_smile/fit_sabr/fit_svi/fit_ssvi/fit_essvi) and wrapped in the typed CalibratedSmile, with no calibration leaking outside the match. Pure: derives the CalibratedSmile from (model, ctx, quotes) refs with no WRITES; self-invalidates if a family arm acquires a side effect.

### 406. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_sabr`

CAPABILITY (smile family 1/5 — SABR): fit_sabr is the SABR smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<StochasticVolSmile>. Seeds (alpha, rho, nu) from the ATM level, 25-delta risk-reversal skew, and butterfly convexity, then a Gauss-Newton fit reproduces the total-variance anchors; beta is pinned (SABR_BETA). No I/O, no input mutation, no WRITES — the gate self-invalidates if the calibration grows a side effect. One of the five selectable smile families (SABR, raw-SVI, SSVI, Vanna-Volga, parametric).

### 407. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_ssvi`

CAPABILITY (smile family 3/5 — SSVI): fit_ssvi is the SSVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<ParametricSlice>. Pins theta to the ATM total variance and fits (rho, phi) to the wings by Gauss-Newton, projecting phi every iteration to satisfy the Gatheral-Jacquier Thm 4.2 butterfly sufficient conditions (theta*phi*(1+|rho|) < 4 and theta*phi^2*(1+|rho|) <= 4); eta = phi*theta^gamma reconstructs the surface phi. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.

### 408. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.calibrate.fit_svi`

CAPABILITY (smile family 2/5 — raw SVI): fit_svi is the raw-SVI smile-family calibrator entry — pure (&MarketContext, &MarketQuotes) -> Result<ParametricSlice>. Fits (b, rho, m, sigma) by Gauss-Newton with the level a pinned so the ATM anchor (k=0) reproduces w_atm exactly; the no-arbitrage projection runs each iteration, and a degenerate (negative minimum total variance) fit is rejected via the validating constructor rather than panicking. No I/O, no input mutation, no WRITES — gate self-invalidates on any side effect. One of the five selectable smile families.

### 409. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.extended_surface.ExtendedSlice.is_calendar_free_with`

CALENDAR no-arbitrage gate in the SVI/extended parameterization (ANALYTICS-SPEC §3.4 calendar axis). `ExtendedSlice::is_calendar_free_with(next)` decides whether an adjacent later-maturity SVI slice is calendar-arbitrage-free relative to self via the standard Gatheral–Jacquier no-crossing conditions on the raw-SVI params: (1) ATM total variance non-decreasing — `next.theta >= self.theta - SLACK`; (2) wing curvature non-decreasing — `next.psi >= self.psi - SLACK`; (3) the skew-change bound — `|next.rho·next.psi - self.rho·self.psi| <= (next.psi - self.psi) + SLACK`, which bounds how fast the skew may rotate so total-variance smiles do not cross at any moneyness. All three must hold (`SLACK = 1e-12`) or the pair is flagged. Pure: reads &self and &next, returns bool, mutating nothing.

### 410. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.lib.build_smile`

CAPABILITY (5 smile families): celnet-surface::build_smile is the production smile-construction entry — a pure, side-effect-free calibration that returns Result<MarketHedgeSmile, CalibrationError> from a MarketContext + MarketQuotes with no I/O, allocation-driven mutation, or logging. It is the MarketHedge (vanna-volga) baseline of the five selectable SmileModel families exposed through the one contract — MarketHedge (vanna-volga), StochasticVol (SABR), Parametric (SVI), ParametricSurface (SSVI), ExtendedSurface (eSSVI) — each constructible into a VolSurface that reports its own model() and prices a finite positive implied vol across the strike grid (proven by each_smile_family_is_selectable). The carry forward feeding every family is the carry-seam bits (context_forward_is_carry_seam_bits).

### 411. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.market_hedge.MarketHedgeSmile.corrections`

CAPABILITY (smile family 4/5 — Vanna-Volga): MarketHedgeSmile.corrections is the Vanna-Volga smile-family analytic entry — pure (&self, strike) -> (D1, D2). Returns the first-order (D1 = p*(sigma1-sigma0) + q*(sigma3-sigma0)) and second-order (D2, the d1*d2-weighted squared vol gaps at the 25-delta wings) market-hedge corrections from the three benchmark vols, the construction underlying the broker-quoted three-point smile. Reads only &self, returns a tuple, no WRITES — the gate self-invalidates if the correction formula grows a side effect. One of the five selectable smile families.

### 412. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.parametric.ParametricSlice.butterfly_density_factor`, `github.com-soarsa-celnet.crates.celnet-surface.src.parametric.ParametricSlice.d_total_variance`

ParametricSlice::butterfly_density_factor(k) computes the Breeden-Litzenberger density factor for an SVI total-variance slice at log-moneyness k. The exact formula is: `(1 − k·w'/2w)² − (w'²/4)·(1/w + 1/4) + w''/2` where w = total_variance(k), w' = d_total_variance(k), w'' = d2_total_variance(k). A strictly positive value at all k implies the slice has no butterfly arbitrage (no negative implied density). The SVI first derivative is `w'(k) = b·(ρ + (k−m)/√((k−m)²+σ²))` and second derivative follows analytically. Pure: reads only self fields (a, b, rho, m, sigma); no I/O.

### 413. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.atm_convention`

Market-context atm-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::atm_convention is a pure projection returning self.conventions.atm unchanged — the AtmConvention threaded into ATM-strike resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).

### 414. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.delta_convention`

Market-context delta-convention accessor (docs/INTERFACES.md surface quote contract): MarketContext::delta_convention is a pure projection returning self.conventions.delta unchanged — the DeltaConvention threaded into strike↔delta quote resolution. No writes/allocation/IO; deterministic, side-effect-free read. Self-invalidates if the accessor stops being a direct field projection (WRITES gate).

### 415. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.quotes.MarketContext.strike_at_delta`

Analytics-correctness (guarded-root-find deliverable): celnet-surface::quotes::MarketContext::strike_at_delta resolves a delta pillar to a strike through the guarded inversion strike_from_delta under the context's own delta_convention, signing the target delta with pillar.signed(opt) (call > 0, put < 0). It returns Result<f64, DeltaSolveError> — the guarded root-find surfaces its failure mode (e.g. an unreachable premium-adjusted target beyond the delta cap) as a typed error rather than a silent/wrong root. Pure: deterministic in (&self, opt, pillar, vol) given the context, no WRITES edges.

### 416. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.black_vol`

StochasticVolParams::black_vol(strike) computes the standard SABR lognormal-expansion implied vol (Hagan et al. 2002/2014). The formula is `(α/denom) · (z/x(z)) · B_factor` where: `denom = (F·K)^((1−β)/2) · [1 + (1−β)²/24·(ln F/K)² + (1−β)⁴/1920·(ln F/K)⁴]`; `z = (ν/α)·denom_base·ln(F/K)`; `x(z) = ln((√(1−2ρz+z²)+z−ρ)/(1−ρ))`; `B_factor = 1 + [(1−β)²/24·(α/fk_pow)² + ¼ρβν·α/fk_pow + (2−3ρ²)/24·ν²]·t`. ATM (|F−K| ≤ 1e-12·F) returns `(α/fk_pow)·B_factor` (z/x(z) → 1). The z → 0 removable singularity is guarded by the series `1/(1 − ½ρz + (2−3ρ²)/12·z²)`. Pure: reads only its `StochasticVolParams` fields (α, β, ρ, ν, forward, t); no I/O or mutation.

### 417. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.stochvol.StochasticVolParams.risk_neutral_density`

StochasticVolParams::risk_neutral_density(strike) returns the SABR risk-neutral density via a change-of-variable from strike space to the SABR coordinate u. The mapping is: `y = y_coordinate(strike)`, `dy/dK = 1/(α·K^β)`; for ν > 1e-10: `z = ν·y`, `u = (1/ν)·ln((√(1−2ρz+z²)+z−ρ)/(1−ρ))`, `du/dy = 1/√(1−2ρz+z²)`; for ν ≤ 1e-10 (Black limit): `u = y`, `du/dy = 1`. The density is then `φ(u/√t)/√t · |du/dy| · |dy/dK|` where φ is the standard normal PDF. Returns 0 for strike ≤ 0. Pure: reads only its struct fields; no mutation.

### 418. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.build_trial_smile`

STRANGLE-CALIBRATION guarded trial-smile constructor (ANALYTICS-SPEC §1.4 — the mandatory smile-strangle calibration that reprices the broker/market strangle). `build_trial_smile(ctx, atm_vol, atm_strike, pillar, rr, smile_strangle)` builds one candidate vanna-volga smile for a trial σ_ss inside the calibration bracketing loop: it resolves the three wings via `smile_wings`, then HARD-REJECTS out-of-domain trials as `Err(CalibrationError::DegenerateQuote)` rather than panicking or returning an invalid Ok — (1) a |RR| too large relative to ATM+σ_ss drives a wing vol <= 0, caught by `!(put_vol>0 && call_vol>0)`; (2) the fallible `MarketHedgeSmile::try_new` enforces the strike ordering K1<K2<K3 (put wing < ATM < call wing) and returns the same error if a pathological trial inverts it. This keeps the guarded root-find total: a degenerate trial steps the bracket away, and a terminal degenerate state surfaces as a typed error, never a panic. Pure: reads ctx/scalars/pillar, returns Result<MarketHedgeSmile, CalibrationError>, mutating nothing.

### 419. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.market_strangle`

The broker (market) butterfly is NOT the smile butterfly — the #1 production FX-vol bug (ANALYTICS-SPEC §1.4). `market_strangle` defines what brokers actually trade: a SINGLE vol `strangle_vol = atm_vol + quote.butterfly` applied to BOTH the call AND put pillar strikes (call_strike and put_strike each resolved via strike_at_delta at that one vol), and its price is call+put at that single vol. This is the calibration TARGET that the recovered smile must reprice — it is deliberately distinct from the arithmetic smile-strangle (the per-wing-vol convexity), and the two differ materially for high-RR/EM pairs. Treating the quoted BF as the arithmetic 25Δ smile-strangle silently biases the wings and breaks 10Δ reproduction. Pure: reads ctx/atm_vol/quote, returns Result<MarketStrangle, CalibrationError>, mutating nothing.

### 420. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strangle.smile_wings`

The smile wings carry DISTINCT per-wing vols — the counterpart to the single-vol broker strangle (ANALYTICS-SPEC §1.4, Reiswich-Wystup/Clark). `smile_wings` builds the recovered 25Δ/10Δ wings from the trial smile-strangle σ_ss as `call_vol = σ_ATM + σ_ss + ½RR` and `put_vol = σ_ATM + σ_ss − ½RR` (the risk-reversal RR splits the two wings antisymmetrically), then resolves each wing's strike at ITS OWN vol via strike_at_delta. Because call and put wings get different vols (unlike market_strangle's single vol on both strikes), the explicit broker→smile calibration step is mandatory: a non-zero RR makes the smile-strangle differ from the broker butterfly. Pure: reads ctx/atm_vol/pillar/rr/smile_strangle, returns a Result tuple, mutating nothing.

### 421. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.strike_quotes.fit_strike_slice`

DELIVERABLE surface/crypto-leaf (the LEAF half) = DONE (RC cut c5a5efc). `fit_strike_slice(ctx, quotes)` is the pure strike-axis (log-moneyness, NOT FX delta-space RR/BF) smile calibrator that lets a crypto/equity surface be marked from a raw strike-vol grid: it seeds an SVI-style 3-parameter slice deterministically (vertex at the lowest observed total variance, width from the k-span), runs a fixed-iteration projected Gauss-Newton inner solve, then ray-projects onto the butterfly-admissible set (a no-op for any arbitrage-free-reproducible quote set), returning `Result<StrikeSliceFit, CalibrationError>` with the slice plus rms/max reproduction error in absolute vols. Pure: it borrows `(&StrikeSliceContext, &StrikeQuoteSlice)`, mutates only local state, performs no I/O, and is fully deterministic (no RNG). This is the surface LEAF the backlog split out of W3-crypto; the remaining OPEN half is the WIRE surfacing (no strike-axis quote_basis on MarkSurfaceRequest yet — see mark_surface_request_from_json), tracked as surface/crypto-strike-axis-surfacing.

### 422. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.surface.VolSurface<S, C>.arbitrage_report`

The three FX no-arbitrage gates are computed together over the whole surface and are hard-reject inputs (ANALYTICS-SPEC §3.4). `VolSurface::arbitrage_report` produces a SurfaceArbitrageReport with all three diagnostics: (1) BUTTERFLY — `min_density`, the minimum over sampled maturities/strikes of the second-difference risk-neutral density (a negative value ⇒ butterfly arbitrage); (2) VERTICAL — `max_vertical_increase`, the worst call-price increase across ascending strikes (a positive value ⇒ vertical-spread arbitrage, calls must be non-increasing in K); (3) CALENDAR — `min_calendar_increment`, the minimum cross-slice total-variance increment at the wing+ATM log-moneyness (a negative value ⇒ calendar arbitrage, total variance must be non-decreasing in T). Per-maturity work delegates to check_slice; cross-slice calendar to term.min_calendar_increment. A surface is arbitrage-free only when min_density≥0, max_vertical_increase≤0, and min_calendar_increment≥0 — these are gating thresholds, not advisory. Pure: reads &self + sampling params, returns the report value, mutating nothing.

### 423. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.CalendarClock.business_time`

DELIVERABLE surface/event-weighted-clock = OPEN (Round-2 P2/M finding, tracked). `CalendarClock::business_time(&self, t)` is the identity business-time map `tau(t) = t` — it returns its argument verbatim, with no weekend/holiday compression and no scheduled-event (central-bank-meeting / fixing) weighting. It is the ONLY BusinessClock implementation in the workspace (the trait seam exists but has a single identity impl), so term-structure interpolation is currently a no-op on the event-weighted clock that ANALYTICS-SPEC §3.6 specifies as market standard. Pure: it reads only `(&self, t)` and returns `t`, mutating nothing. Closing the deliverable means adding a real event/calendar-weighted BusinessClock impl; the new impl (and any change to this identity body) will change this method's node content and STALE this claim — the staleness firing is the done-signal. This is intentionally a no-arbitrage-safe placeholder, NOT a faked depth claim.

### 424. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure<S, C>.implied_vol`

TermStructure::implied_vol(strike, t) converts from strike / calendar-time to Black implied vol by routing through total variance: `f = forward_at(t)`, `k = ln(strike/f)`, `w = total_variance(k, t)`, returning `Vol(sqrt((w/t).max(0.0)))`. The `.max(0.0)` clamp makes the output well-defined even under marginal floating-point calendar-arbitrage; the caller is responsible for ensuring the surface is calendar-free before relying on exact values. Pure: no writes.

### 425. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure<S, C>.is_calendar_free`

CALENDAR no-arbitrage gate (ANALYTICS-SPEC §3.4 — the cross-tenor third arbitrage axis, distinct from the per-slice butterfly+vertical gate in check_slice, which the check_slice claim explicitly delegates here). `TermStructure::is_calendar_free(k, tol)` is the hard-reject calendar gate: it returns true iff `min_calendar_increment(k, 256) >= -tol`, i.e. total variance w(k,t)=σ²·t is non-decreasing in maturity along a fixed strike. A strict total-variance crossing (a longer-dated pillar carrying LESS total variance than a shorter one) is a calendar-spread arbitrage and is rejected — e.g. 6M@15vol (w=0.01125) vs 1Y@10vol (w=0.01) flags. This is the term-axis member of the three FX no-arbitrage gates (butterfly/calendar/vertical) that are hard-reject inputs. Pure: reads &self pillars and returns a bool, mutating nothing.

### 426. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-surface.src.termstructure.TermStructure<S, C>.min_calendar_increment`

CALENDAR-arbitrage MEASURE primitive (ANALYTICS-SPEC §3.4) behind the calendar no-arbitrage gate. `TermStructure::min_calendar_increment(k, samples)` pins one strike from the near-pillar log-moneyness (`strike = pillars[0].forward · e^k`), then walks that FIXED strike across `samples` maturities t0..t1 — crucially re-converting to EACH maturity's own log-moneyness `k_t = ln(strike / forward_at(t))` before reading `total_variance(k_t, t)` — and returns the minimum forward increment of total variance w between consecutive maturity samples. A negative minimum increment is exactly a total-variance crossing = calendar-spread arbitrage; is_calendar_free rejects when this is below -tol. Fixing the cash strike (not the moneyness) across tenors is the correct no-arb test under term-varying forwards. Pure: reads &self, returns f64, mutating nothing (it does assert samples>=2 as a precondition, but performs no writes).

### 427. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.carry_rate`

Carry::carry_rate() is a pure accessor over the asset-class-agnostic carry seam (FxRates{r_dom,r_for} | CostOfCarry{r,b}). Every pricing engine consumes the Carry seam rather than raw FX rate fields, and there is no hot-path match on Carry/Underlying. This keeps one asset-class-agnostic pricing contract across vanilla/exotics/surface/risk and the crypto/equity/commodity leaves (ADR-0008).

### 428. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Carry.yield_rate`

Carry::yield_rate() returns the stored foreign/yield rate (r_for) VERBATIM — it is a pure accessor with no side effects. FX bit-identity depends on this: foreign-rate reads must use yield_rate() directly and must NEVER be reconstructed as discount_rate() - carry_rate(), which would perturb the FX hot path. Enforced by the ADR-0008 carry-seam architecture (docs/adr/ADR-0008-multi-asset-carry-architecture.md); the carry-seam lowering guard rejects raw cost-of-carry for FX (see celnet-core/src/carry.rs fx_lowering_rejects_cost_of_carry).

### 429. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.FixingSource.code`

FixingSource::code(self) -> &'static str is a const pure total function mapping each of the 6 EM exotic-fixing sources to its canonical ISDA/market-convention code string: KrwKftc18→"KRW.KFTC18", TwdTaipei→"TWD.TAIPEI", InrRbiRef→"INR.RBIB", BrlPtax→"BRL.PTAX", ClpDolarObs→"CLP.DOLAROBS", CopTrm→"COP.TRM". The fixing_source_codes_are_distinct_and_nonempty test (loop over FixingSource::all()) asserts every code is non-empty and every pair is distinct, so the code strings are a compile-verified bijection.

### 430. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.Greeks.price_only`

Greeks is a 14-field f64 struct encoding the full first/second/third-order option sensitivities output by every analytics leaf in the platform: price (PV), delta_spot, delta_forward, gamma, vega (per 1.0 absolute vol), theta (per year, −∂V/∂T), rho_dom, rho_for, vanna (∂²V/∂S∂σ), volga/vomma (∂²V/∂σ²), charm (∂delta_spot/∂T), speed (∂³V/∂S³), zomma (∂gamma/∂σ), color (∂gamma/∂T). Greeks::price_only(price) is the canonical constructor for price-only paths. This struct is the uniform output contract across all asset-class pricers.

### 431. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.MetalPair.as_ccy_pair`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.Underlying.as_ccy_pair`

Underlying::as_ccy_pair(&self) -> Option<CcyPair> is a const pure total accessor: Fx(p) returns Some(p); Metal(m) returns Some(m.as_ccy_pair()) — delegating to MetalPair::as_ccy_pair which constructs CcyPair::new(metal.ccy(), quote) (the ISO-X-prefixed metal Ccy, e.g. XAU, as base). Equity/Commodity/DigitalAsset return None. Called by 5 callers in the analytics and risk-cube layers that need to resolve an Underlying to a tradeable FX/metal rate pair.

### 432. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.flip`

OptionType::flip(self) -> Self is a const pure involution: Call maps to Put and Put maps to Call. It is the put-call symmetry operator used in premium-adjustment transformations and put-from-call/parity conversions. Zero callers in production code but referenced in the premium_adjusted_flag test; its total absence of side effects and exhaustive match make it a compile-time-checkable involution.

### 433. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.OptionType.sign`

OptionType::sign(self) -> f64 is a const pure function returning +1.0 for Call and -1.0 for Put. It is used as the φ multiplier throughout the closed-form pricing formulae (e.g. φ·(F·N(φ·d1) − K·N(φ·d2))). Called by 11 callers across the analytics crates — it is the canonical signed-direction encoding for the option type.

### 434. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.flip_orientation`

`PremiumStyle::flip_orientation` is a self-inverse (involution) mapping the premium style under base/quote currency-pair inversion: DomesticPips↔ForeignPips and PercentForeign↔PercentDomestic. Applying it twice is the identity (DomesticPips→ForeignPips→DomesticPips; PercentForeign→PercentDomestic→PercentForeign), so re-quoting the same option in the inverted pair orientation and back recovers the original premium style exactly — the orientation-invariance property the pair-universe view relies on (docs/CONVENTIONS.md §pair universe). It is a `const fn` total match with no wildcard. Pure: reads only `self`, returns the flipped enum, no mutation.

### 435. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.PremiumStyle.is_premium_adjusted`

`PremiumStyle::is_premium_adjusted` is the canonical predicate deciding whether the premium carries FX risk: it returns true exactly for the FOR/base-ccy-denominated styles `PercentForeign | ForeignPips` and false for the DOM-ccy styles `DomesticPips | PercentDomestic`. A FOR-ccy premium carries FX risk, which is why these styles drive premium-adjusted delta (the non-monotone premium-adjusted call delta of docs/CONVENTIONS.md). The classification is a `const fn matches!` over `self` with no wildcard, so a new PremiumStyle variant forces this distinction to be revisited rather than defaulting silently. Pure: reads only `self`, returns a bool, no side effects.

### 436. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.df_dom`, `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.df_for`

VanillaInputs::df_dom(&self) -> f64 computes the domestic discount factor e^{-r_dom·t} = libm::exp(-self.r_dom * self.t); VanillaInputs::df_for(&self) -> f64 computes the foreign discount factor e^{-r_for·t} = libm::exp(-self.r_for * self.t). Both delegate to rust-lang/libm for cross-platform bit-identical results (same determinism contract as celnet_core::math::exp). These are the standard continuously-compounded spot discount factors for the domestic and foreign rates respectively.

### 437. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-types.src.lib.VanillaInputs.forward`

VanillaInputs::forward(&self) -> f64 computes the FX forward price F = S·e^{(r_dom − r_for)·t} = self.spot * libm::exp((self.r_dom - self.r_for) * self.t). This is the interest-rate-parity forward for a two-rate FX pair; it is also the carry-neutral forward for the Garman-Kohlhagen parameterisation. Delegates to libm::exp for cross-platform bit-identity.

### 438. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.adjoint.adjoint_greeks`

`adjoint_greeks` computes the full 14-field Greeks via AAD (Algorithmic Adjoint Differentiation). The forward pass records a `Tape` struct (all intermediate scalars needed for reverse differentiation); the first reverse pass (`reverse(&tp, 1.0)`) produces the first-order adjoints — delta_spot=∂V/∂S, vega=∂V/∂σ, theta=−∂V/∂T, rho_dom=∂V/∂r_dom, rho_for=∂V/∂r_for — in a single backward sweep (O(1), same cost as a forward eval). Second-order gamma and vanna are obtained by hand-composing a second reverse pass over the delta_spot expression (genuine reverse-over-reverse, not finite-difference), using the chain-rule edges stored on the tape: gamma=df_for·φ(d1)·(1/(S·vsqt)), vanna=df_for·φ(d1)·((σT)/vsqt−d1/σ). volga=vega·(−d1)·∂d1/∂σ. The mixed/higher-order tail (charm, speed, zomma, color) is taken from the analytic `greeks` path. The AAD price is bit-identical to `price` (pinned by `aad_price_bit_identical`). Pure: reads (OptionType, &VanillaInputs), returns Greeks, no writes.

### 439. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.atm.atm_strike`

Delta-Neutral-Straddle (DNS) ATM strike SIGN-FLIPS with the premium-adjusted delta convention — the single most surface-corrupting convention bug if mislocated. `atm_strike` returns F·exp(+½σ²t) for unadjusted delta (SpotUnadjusted | ForwardUnadjusted) but F·exp(-½σ²t) for premium-adjusted delta (SpotPremiumAdjusted | ForwardPremiumAdjusted) — the DNS strike sits ABOVE the forward when unadjusted and BELOW it when premium-adjusted (opposite sign of the ½σ²t drift), exactly per ANALYTICS-SPEC §1.3. AtmForward simply returns the forward. The match on (AtmConvention, DeltaConvention) is exhaustive over both enums, so the half-variance sign is never defaulted. Pure: a total function of (atm, delta_conv, forward, vol, t) returning f64, no side effects.

### 440. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta`

`delta` implements all four FX delta conventions over a shared `delta_aux` precomputation. For SpotUnadjusted/ForwardUnadjusted: Δ = factor·N(±d1) (where factor = df_for for spot, 1 for forward). For SpotPremiumAdjusted/ForwardPremiumAdjusted: Δ_call = factor·(K/F)·N(d2); Δ_put = Δ_call − factor·(K/F) — premium-adjusted delta keys on N(d2) and carries the K/F ratio, which makes it non-monotone in K (the call has a maximum) and underpins the `strike_from_delta` reachability check. The match over DeltaConvention is exhaustive with no wildcard. Pure: reads (DeltaConvention, OptionType, &VanillaInputs), returns f64, no writes.

### 441. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.delta_d_strike`

Analytics-correctness (premium-adjusted-delta deliverable): celnet-vanilla::delta::delta_d_strike is a pure analytic ∂Δ/∂K. For the unadjusted conventions (Spot/Forward-Unadjusted) call and put deltas differ by a K-independent constant, so the strike-slope is the single term factor·φ(d1)·∂d1/∂K with ∂d1/∂K = ∂d2/∂K = −1/(K σ√T). For the premium-adjusted conventions (Spot/Forward-PremiumAdjusted) Δ_call = factor·(K/F)·N(d2), so the slope follows the product rule in K (F is K-independent): factor·(N(d2)/F + (K/F)·φ(d2)·∂d2/∂K); the put slope is the call slope minus factor/F since Δ_put = Δ_call − factor·(K/F). This convention-branching strike-derivative is what makes the premium-adjusted delta non-monotone in strike (it underpins the guarded delta→strike root-find). Pure: deterministic in (conv, opt, &VanillaInputs), no WRITES edges.

### 442. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.delta.premium_adjusted_call_delta_max`

Premium-adjusted call delta is NON-MONOTONE in strike — it has a maximum-delta strike with two strikes mapping to the same delta, so the strike↔delta root-find must be guarded and bracketed on the correct branch (ANALYTICS-SPEC §3.5). `premium_adjusted_call_delta_max` computes that delta-max strike as the stationary point: it solves g(d2)=N(d2)·σ√T−φ(d2)=0 (g increasing in d2, root at small positive d2) by bisection, then maps the root d2* back to the strike K = F·exp(−½σ²T − d2*·σ√T). This is the cap the solver must respect: a target delta above the achievable max is unreachable, and a naive monotone Brent/Newton would converge to the wrong branch or diverge. Pure: reads `&VanillaInputs`, returns the cap strike as f64 via a fixed-iteration bisection, mutating nothing.

### 443. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.aux`

`aux` is the internal precomputation kernel: it computes the four canonical option-pricing intermediates from `&VanillaInputs` — sqt=√T (via libm::sqrt), vsqt=σ·√T, d1=(ln(S/K)+(r_dom−r_for+½σ²)·T)/vsqt (via libm::ln), d2=d1−vsqt — and packages them in `Aux`. Both `price` and `greeks` call `aux` first and re-use these values throughout; computing them once avoids duplicate transcendental evaluations on the hot path. Pure: reads only &VanillaInputs, returns Aux, no allocation, no writes.

### 444. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.greeks`

FX has exactly TWO rhos, never one. `greeks` returns both `rho_dom = ∂V/∂r_d` (call: K·t·e^{-r_d t}·N(d2); put: -K·t·e^{-r_d t}·N(-d2)) and `rho_for = ∂V/∂r_f` (call: -S·t·e^{-r_f t}·N(d1); put: +S·t·e^{-r_f t}·N(-d1)) as distinct fields of the Greeks struct — the foreign rate r_f enters as the continuous dividend yield on the foreign-currency asset (Garman-Kohlhagen 1983), so a single equity-style "rho" is meaningless and is never exposed (ANALYTICS-SPEC §2.1). The two rho signs are opposite (domestic-rate up raises a call, foreign-rate up lowers it), so collapsing them would cancel real rate risk. Pure: it reads `opt` and `&VanillaInputs` and returns a `Greeks` value, mutating nothing.

### 445. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.lib.price`

GUARDRAIL/HOT-CORE — the FX vanilla pricing kernel `price` is allocation-free and lock-free by construction: it reads spot/strike discounted by df_for()/df_dom(), evaluates the closed form via norm_cdf on the precomputed aux (d1,d2), and returns the f64 price for Call/Put — no allocation (alloc_in_loop=0, no Vec/Box), no loop (loop_depth=0), no I/O, no logging, no locks. This is the pinned zero-alloc hot core: it is the leaf kernel the engine's hot pricing loop calls (in_degree 13), and the engine's `hot_pricing_loop_allocates_zero` / `hot_pricing_under_concurrent_publish_allocates_zero` tests (a custom counting global allocator asserting zero allocations on the hot path) hold precisely because kernels like this allocate nothing. Pure: it reads &VanillaInputs and the OptionType and returns the f64 price, mutating nothing — telemetry/logging is offloaded off this path, never inlined into it.

### 446. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.premium.premium_from_domestic_pips`

PremiumStyle is the FX quotation-units axis and premium_from_domestic_pips is its canonical converter (docs/CONVENTIONS.md PremiumStyle → premium units; docs/ANALYTICS-SPEC premium quotation). Given a price already in domestic pips (v_dpips), it exhaustively maps the four PremiumStyle variants to their quoted unit: DomesticPips passes the raw PV through unchanged; PercentForeign divides by spot (per unit of foreign/base notional); PercentDomestic divides by strike (per unit of domestic/quote notional at strike); ForeignPips divides by spot·strike. The match is exhaustive over PremiumStyle, so no style is defaulted, and the DomesticPips arm is the identity (domestic_pips_is_the_raw_pv). Pure: a total function of (style, v_dpips, spot, strike) returning f64 with no writes/allocation/IO; deterministic under the f64 CPU-canonical/libm rule. Self-invalidates if the PremiumStyle variant set or any per-style scale factor changes (WRITES gate).

### 447. `invariant:pure` (stale)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.bracket`

`bracket` is the branch-aware bracketing primitive that makes the strike↔delta solve safe under the NON-MONOTONE premium-adjusted call delta (ANALYTICS-SPEC §3.5). It detects the premium-adjusted convention (SpotPremiumAdjusted | ForwardPremiumAdjusted) and, for a Call, computes the delta-max strike via `premium_adjusted_call_delta_max`: a target_delta above delta_max+1e-12 is rejected as DeltaSolveError::Unreachable (the cap is the reachability boundary), and the returned bracket is deliberately pinned to the DECREASING (OTM) branch — lo=K_max where Δ=delta_max≥target ⇒ g(lo)≥0, hi expanded by doubling until g(hi)≤0 as K→∞ where Δ→0 — so the downstream root-find can never land on the ascending (ITM) branch that maps a different strike to the same delta. For unadjusted/put cases delta is monotone, so it geometrically expands [tiny_strike, f] outward toward the shrinking-residual side until a sign change is found, returning Unreachable after 64 unsuccessful doublings. Pure: reads (conv,opt,target_delta,&VanillaInputs,f,&at-closure) and returns Result<(f64,f64),DeltaSolveError>; it allocates nothing and mutates no external state (only loop-local lo/hi/iters).

### 448. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-vanilla.src.solver.strike_from_delta`

`strike_from_delta` is the GUARDED strike↔delta root-find the non-monotone premium-adjusted delta demands (ANALYTICS-SPEC §3.5): it never runs a naive monotone Newton/Brent that could converge to the wrong branch. It first enforces sign discipline (a Call target_delta<0 or Put target_delta>0 ⇒ DeltaSolveError::WrongSign), then delegates the bracket to `bracket` — which, for the premium-adjusted call, calls `premium_adjusted_call_delta_max` to obtain the delta-max cap, returns DeltaSolveError::Unreachable for a target above the achievable max, and pins the bracket onto the correct (OTM, decreasing) branch above K_max. The inner loop is a Brent-lite: it keeps the sign-straddling bracket [lo,hi] (debug_assert glo·ghi≤0) as the safety net and only accepts a Newton step using delta_d_strike when it lands strictly inside (lo,hi), else falls back to bisection — so it is bracket-guaranteed convergent and cannot escape onto the wrong delta branch. Returns DeltaSolveError::NoConvergence rather than a wrong root if iteration stalls. Pure: it reads (conv,opt,target_delta,&VanillaInputs), mutates only stack-local copies of the inputs (inp.strike) to evaluate delta/slope, performs no I/O, allocation, or external mutation.

### 449. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.cva.XvaResult.total_adjustment`

XvaResult::total_adjustment() = cva − dva + fva: the algebraically-signed aggregate valuation adjustment applied to the clean price (CVA reduces value, DVA increases it, FVA is additive as a funding cost on net exposure). Pure: reads only &self, returns f64, no mutation or allocation.

### 450. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.cva.compute_xva`

compute_xva implements the standard discrete unilateral CVA/DVA/FVA formulas (Gregory, The xVA Challenge, 2015; Brigo-Morini-Pallavicini, 2013): CVA = LGD_c · Σ_k D(t_k) · EPE(t_k) · (S_c(t_{k-1}) − S_c(t_k)); DVA = LGD_o · Σ_k D(t_k) · ENE(t_k) · (S_o(t_{k-1}) − S_o(t_k)); FVA = funding_spread · Σ_k D(t_k) · (EPE(t_k) − ENE(t_k)) · Δt_k · S_c(t_k) · S_o(t_k). LGDs are hard-asserted to [0,1] (panic otherwise). The function reads only &XvaInputs and returns XvaResult — no mutation, no I/O, no allocation beyond the return value.

### 451. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.deterministic`

ExposureProfile::deterministic is a validated constructor for externally-supplied EPE/ENE profiles (closed-form or scenario-override use): it asserts grid/epe/ene are equal-length and non-empty, grid[0] == 0, grid is strictly increasing, and every EPE/ENE entry is non-negative (panics on any violation). It then computes the discount vector as exp(−r_dom·t_k) over the supplied grid. This is the entry point for the closed-form XVA oracle tests that bypass Monte Carlo.

### 452. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.exposure.ExposureProfile.simulate`

ExposureProfile::simulate drives spot under risk-neutral GBM with Sobol low-discrepancy normals from celnet_qmc::SobolSequence (one point per path, dimension = steps, each coordinate driving one time-step): drift = (r_dom − r_for − ½σ²)·Δt, vol_step = σ·√Δt; spot_{k+1} = spot_k · exp(drift + vol_step · Φ⁻¹(u_k)). At each grid date t_k the full netting set is repriced via NettingSet::net_value(t_k, spot), and EPE(t_k) = E[max(V,0)], ENE(t_k) = E[max(−V,0)] are computed as path-average sums. The discount vector at grid node t_k is exp(−r_dom · t_k). Bit-reproducibility is guaranteed by the seeded Sobol stream (seed + steps fix the entire uniform sequence).

### 453. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettedTrade.mark`, `github.com-soarsa-celnet.crates.celnet-xva.src.netting.NettingSet.net_value`

NettedTrade::mark(t_obs, spot, r_dom, r_for) is the per-trade mark-to-market at observation time t_obs: it computes remaining time τ = expiry − t_obs and returns 0.0 for any matured trade (τ ≤ 0), otherwise delegates to celnet_vanilla::price via VanillaInputs::new(spot, strike, vol, τ, r_dom, r_for) scaled by notional. NettingSet::net_value sums these marks across all trades, implementing plain algebraic netting (signed sum) within the set. Pure: reads only &self and scalar inputs, returns f64, no mutation.

### 454. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.cumulative_hazard`, `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.survival`

SurvivalCurve::survival(t) = exp(−Λ(t)) where Λ(t) is the piecewise-constant cumulative hazard. It is a pure total function: `libm::exp(-self.cumulative_hazard(t))` with no mutation, no I/O. cumulative_hazard(t) integrates the piecewise-constant λ segments up to t via an O(n) scan over (pillar, hazard) pairs, clipping each segment at min(pillar, t), and extrapolates the final hazard flat beyond the last finite pillar — so S(t) is continuous, monotone non-increasing, and equals 1 at t=0. Negative or non-finite t panics.

### 455. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.flat`

SurvivalCurve::flat(lambda) constructs a constant-hazard survival curve representing S(t) = exp(−λ·t) for all t ≥ 0. It asserts lambda is finite and non-negative (panics otherwise) and stores a single pillar at f64::INFINITY, so cumulative_hazard evaluates to λ·t for any finite t via the flat-extrapolation branch. This is the closed-form reference curve used by the XVA oracle tests.

### 456. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.crates.celnet-xva.src.survival.SurvivalCurve.marginal_default`

SurvivalCurve::marginal_default(a, b) = S(a) − S(b): the marginal probability of default in the interval (a, b]. It asserts a ≤ b, then returns survival(a) − survival(b). This is the interval default-probability atom consumed by compute_xva's outer loop (dp_c = s_c_prev − s_c corresponds to marginal_default over each grid interval). Pure: reads only &self and two f64 scalars, returns f64, no mutation.

### 457. `invariant:pure` (active)
Anchors: `github.com-soarsa-celnet.fuzz.fuzz_targets.fix_frame_decode.structured_roundtrip`

DELIVERABLE fix-decoder-fuzz-target = LANDED (backlog tracker docs/WORLD-CLASS-BACKLOG.md still lists it OPEN as the Round-2 P2/S finding "celnet-fix violates verification-contract clause (f): no fuzz target for the only byte parser fed external-counterparty bytes"; reconciled against the live graph). `structured_roundtrip` is the pure (side-effect-free) differential oracle inside the now-present fuzz/fuzz_targets/fix_frame_decode.rs harness: it drives the production celnet-fix FrameCursor::parse over adversarial/arbitrary byte frames and asserts the structured-decode↔re-encode round-trip and in-domain invariants, giving the external-counterparty FIX byte parser the fuzz coverage that VERIFICATION-CONTRACT.md clause (f) mandates. SELF-INVALIDATES on any change to this fuzz oracle.

### 458. `spec:satisfies` (draft)
Anchors: `design-target:docs/acceptance/carry-seam.acceptance.json`, `github.com-soarsa-celnet.crates.celnet-core.src.carry.fx_vanilla_inputs`

Carry-seam FX byte-identity (carry-seam deliverable, ADR-0008): celnet-core::carry::fx_vanilla_inputs lowers the Carry::FxRates arm to VanillaInputs byte-identically (forward/df_dom/df_for to_bits-equal to the direct VanillaInputs::new) and rejects the generalized Carry::CostOfCarry arm on the FX lowering path — no silent fallback. Acceptance assertions in docs/acceptance/carry-seam.acceptance.json.

### 459. `spec:satisfies` (draft)
Anchors: `design-target:docs/acceptance/rfq-multidealer.acceptance.json`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.MultiDealerEngine.request`

Deliverable rfq-multidealer is satisfied: concurrent fan-out + ranked panel.

### 460. `spec:satisfies` (draft)
Anchors: `design-target:docs/acceptance/rfq-multidealer.acceptance.json`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.MultiDealerEngine.request`

Deliverable rfq-multidealer is satisfied: MultiDealerEngine fans an RfqRequest out to every registered QuoteSource concurrently and ranks responses into a best-bid/offer panel with deterministic tie-break.

### 461. `spec:satisfies` (draft)
Anchors: `design-target:docs/acceptance/rfq-multidealer.acceptance.json`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.MultiDealerEngine.request`, `github.com-soarsa-celnet.crates.celnet-rfq.src.panel.QuoteSource.request`

Deliverable rfq-multidealer: MultiDealerEngine::request fans an RfqRequest out to every registered QuoteSource concurrently and ranks responses into a best-bid/offer panel with a deterministic tie-break; new LPs implement the QuoteSource seam without touching the engine. Acceptance target docs/acceptance/rfq-multidealer.acceptance.json. (Behavioral assertions DEFER to draft until execute-to-verify is enabled.)

### 462. `ui:component:badge` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.StatusBadge.StatusBadge`

StatusBadge is the small stream-health status pill: a single <span> driven purely by the StreamHealth enum, selecting a glyph (GLYPH[health]) and a per-state CSS-Module modifier class (styles[health.toLowerCase()]) that colors it from the semantic tokens. It is stateless and presentational, and is the only component with committed Storybook stories (StatusBadge.stories.tsx → components-statusbadge--* in the static index), making it the visual-regression and token-rendering reference fixture.

### 463. `ui:component:dialog` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.CommandPalette.CommandPalette`

CommandPalette is the ⌘K command-launcher overlay: a scrim-backed modal dialog with a fuzzy-matched (fuzzyMatch) search input over the caller-supplied Command list, ranking and showing the top results as a keyboard-navigable listbox. It renders null when closed, clears query/active on open, and runs the selected command's run() on Enter/click. It is the central action surface (pairs, workspaces, actions) styled through CSS-Module tokens (scrim/palette/item) rather than inline literals.

### 464. `ui:component:grid` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.DataGrid.DataGrid`

DataGrid is the single reusable virtualized data-grid primitive (generic over the row datum T): row-windowing (useVirtualWindow) and column-windowing (columnWindow) render only the visible slice, with optional grouping (flattenGroups + collapsible group rows), sortable columns, and roving-tabindex keyboard navigation. It reads row height and cell padding from the density tokens (--row-h, --cell-pad-x/y) via useRowHeight, so the comfortable/compact density axis applies without a JS branch. The blotter (StreamWorkspace), risk and book workspaces all compose this one component rather than re-implementing a grid.

### 465. `ui:component:strip` (active)
Anchors: `github.com-soarsa-celnet.gui.src.components.GreeksStrip.GreeksStrip`

GreeksStrip is the inline option-risk readout: a primary row of GreekCell tiles (delta/gamma/vega/theta) plus a disclosure button (aria-expanded + aria-label="toggle full Greeks") that reveals the secondary Greeks. It is asset-class-aware — rhoGreeksFor(assetClass) relabels the rate-rho Greeks per the active underlier's class (FX default) — so the same strip serves FX/equity/commodity/crypto tickets. Numerics render through GreekCell on the mono token face; the strip itself carries no raw color literals.

