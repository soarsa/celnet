# Deliverable / capability map — what already exists (extend, don't duplicate)

Derived from the verified `CAPABILITY SUMMARY` claims in the knowledge base. Before building
a new capability, check here + `knowledge_get <governed symbol>` for the contract. Formal
`spec:satisfies` acceptance targets live in `docs/acceptance/` (verified-active gating pends
execute-to-verify; see lodestar ticket).

**23 capabilities mapped.**

## `celnet-bench`
**Governed symbols:** `main`

celnet-bench is the performance-proof crate for the celnet FX-options platform. It turns the latency/throughput budgets from docs/ARCHITECTURE.md §1.2 into reproducible, asserted proof across four distinct gate arms: (1) in-core absolute budget (price + full 13-Greek set, p50 ≤ 2µs / p99 ≤ 10µs / p99.9 ≤ 25µs, measured pinned/elevated via `core_load`); (1b) surface-rebuild absolute budget (per-pair all-tenors VV + SSVI recompute p99 ≤ 150µs, via `surface_rebuild`); (2) wire-path relative regression gate (round-trip RFQ latency under concurrent RFS streaming load, relative to a committed JSON baseline, via `wire`); (3) fleet §11 SLO relative regression gate (cross-shard federation overhead, publish→snapshot lag, conflation correctness, many-subscriber fan-out spread, via `fleet_slo`). All four arms are wired into the `bench_gate` binary which exits non-zero on any breach. The crate also provides shared, allocation-free input fixtures (`representative_inputs`, `representative_batch`, `sweep_inputs`) consumed identically by benches and unit tests, and an iai-callgrind instruction-count gate (`iai_instructions`) with soft-regression limits on `Ir` and `EstimatedCycles`.

## `celnet-calendar`
**Governed symbols:** `delivery_date`, `expiry_for_tenor`, `schedule`, `spot_date`

celnet-calendar is the FX date-arithmetic engine for the pricing stack. Its public contract is: `schedule(pair, horizon, tenor) -> Result<FxSchedule, TenorError>` is the single top-level entry point; it returns (horizon, spot, expiry, delivery, vol_anchor) for any Tenor variant (Overnight, TomNext, SpotNext, Weeks, Months, Years, Imm(n), BrokenDate). The vol_anchor is `horizon` for ON/TN (so they never produce zero or negative vol-time) and `spot` for all other tenors. All internal date arithmetic flows through `BusinessCalendar` (multi-centre intersection semantics), `RollRule::ModifiedFollowing`, and the centre-specific `SettlementCentre::is_holiday` dispatch. New currency pairs plug in by extending `centre_for`/`is_t_plus_one_pair`; no change to `schedule` or any downstream pricer is required.

## `celnet-commodity-vanilla`
**Governed symbols:** `on_future`, `on_spot`, `greeks_with_margining`

celnet-commodity-vanilla is the commodity/futures-options analytics leaf on the carry seam. Its public contract is (CommodityInputs, OptionType, Margining) → (price f64 | CarryGreeks) with two distinct margining modes dispatched through greeks_with_margining: EquityStyle (discounted Black-76, used for physically-settled exchange options) and FuturesStyle (undiscounted Black-76, used for CME-style daily-margined contracts). The crate exposes two smart constructors — CommodityInputs::on_future (sets b=0, spot:=future price) and CommodityInputs::on_spot (sets b=r−convenience_yield) — that encode the standard cost-of-carry reparameterization. New arms plug in by calling greeks_with_margining or price_with_margining with the appropriate Margining tag; no switch is needed anywhere else in the pricing stack.

## `celnet-conventions`
**Governed symbols:** `has_pair_profile`, `pair_meta`, `resolve`

celnet-conventions is the single authoritative registry for all FX-options market conventions. Its primary public contract is `resolve(pair, tenor) -> ResolvedConvention`: for covered pairs it returns a `ConventionRecord` sourced from a static `PairProfile` (tagged `ResolutionSource::PairProfile`); for any unknown pair it falls back to a region-derived default (tagged `ResolutionSource::RegionDefault`). Callers plug into the registry by calling `resolve` or `pair_meta`; the `ResolutionSource` tag lets consumers distinguish authoritative per-pair data from region defaults.

## `celnet-core`
**Governed symbols:** `price`, `price_greeks`, `implied_vol`, `norm_cdf`

celnet-core is the zero-IO, zero-alloc pure-domain foundation layer of the celnet pricing platform. Its public contract is three seams: (1) the `Smile` trait — the abstraction between surface-construction (`celnet-surface`: Vanna-Volga/SABR/SVI) and all consumers that need an implied Black vol at a given strike; (2) the `CarryPricer` trait — the object-safe pricing seam that each asset-class leaf implements to price against `CarryInputs` (containing an `Underlying` + `Carry` tag) with `price(opt, inputs) -> Result<f64, CarryPriceError>` and `price_greeks(opt, inputs) -> Result<CarryGreeks, CarryPriceError>`; (3) the `math` module — deterministic libm-backed transcendental wrappers (`exp`, `ln`, `sqrt`, `norm_cdf`, `norm_pdf`) that guarantee bit-identical f64 results across platforms. The crate is `#![forbid(unsafe_code)]` with zero runtime dependencies beyond `celnet-types` and `libm`. A new asset-class arm plugs in by implementing `CarryPricer`; a new surface model plugs in by implementing `Smile` — neither requires changes to this crate.

## `celnet-engine`
**Governed symbols:** `price`, `run`, `recover`, `publish`

celnet-engine is the hot-path FX-options pricing engine: it exposes PricingCore as the single entry point for pricing vanilla options against a live smile surface, publishes top-of-book results atomically via a seqlock (PriceSnapshot), persists position state through DurableBook/journal (crash-recoverable and bit-identical on replay), and supports lock-free hot market-state reloads via StateHandle (arc-swap). A new pricing arm plugs in by: (1) constructing a PricingCore with an initial MarketState; (2) calling run() with an rtrb SPSC request/response ring; (3) publishing updated MarketState ticks into StateHandle::publish() from a separate thread with no locking; (4) opening a DurableBook for journal-backed crash recovery via recover().

## `celnet-exotics`
**Governed symbols:** `single_barrier_price`, `new`, `calibrate`, `tarf_price`

(celnet-exotics): The crate is the full exotic-products pricing engine for the Celnet platform, providing closed-form, PDE (ADI/PSOR FD), and Monte Carlo (QMC Sobol + antithetic) pricers for: single/double barriers (Reiner-Rubinstein closed form + rebate decomposition), digital/touch options, TARFs and accumulators (antithetic-pair Euler MC), lookback options (Conze-Viswanathan fixed + floating closed form), Asian options (Turnbull-Wakeman analytic + MC), forward-starts and cliquets, American/Bermudan options (PSOR FD + Longstaff-Schwartz MC), perpetual options, multi-asset baskets (scrambled Sobol QMC with Cholesky correlation), and the LSV (Local-Stochastic Volatility) model via particle calibration + ADI PDE. The public contract seam is ExoticInputs, which carries the generalized Carry struct (ADR-0008) so all pricers are asset-class-agnostic. The market-hedge overlay (Vanna-Volga) adds smile cost on top of any flat-vol pricer. A new product arm plugs in by accepting &ExoticInputs and calling i.carry_rate()/discount_df()/carry_df() for drift/discount — never matching on Carry directly.

## `celnet-fix`
**Governed symbols:** `run`, `decode_strategy`, `validate`, `parse`, `finish`, `request_and_lift`, `on_inbound`, `send_app`

celnet-fix is the FIX 4.4 session + FX-options dialect adapter. Its public contract has four layers: (1) Framing — FrameCursor::parse validates a raw byte slice as a well-formed FIX 4.4 frame (BeginString, BodyLength, CheckSum, MsgType present), returning a zero-copy cursor; FrameEncoder::finish assembles 8=/9=/body/10= and stamps the checksum; (2) Dictionary — validate(frame) checks required tags per MsgType and PossDupFlag conditional presence, returning MsgType or DictError; (3) Session — Session<S>::on_inbound dispatches admin (Logon/Logout/TestRequest/ResendRequest/SequenceReset) and delivers app messages (QuoteRequest/Quote/MassQuote/NewOrderSingle/ExecutionReport) through a SessionAction with seq-gap detection and FileStore/InMemoryStore persistence; Session::send_app stamps outbound sequence and stores for replay; (4) FX dialect — decode_strategy/decode_option decode multi-leg/single-leg option strategies from FrameCursor; inputs_for+price_leg bridge OptionDescriptor+MarketSnapshot into VanillaInputs for the pricer. Initiator::request_and_lift / Acceptor::run are the async TCP entry points. A new LP adapter adds a QuoteSource impl; a new message type adds a build_* function and a dictionary entry; the session layer is not modified.

## `celnet-gpu`
**Governed symbols:** `gbm`, `dispatch`, `gpu_context`, `public_surface_composes`, `estimate`, `dispatch`

celnet-gpu is the wgpu-backed GPU/CPU Monte Carlo pricing substrate. Its public contract is: (1) PathSpec::gbm(spot,vol,t,r_dom,r_for,paths,steps,seed) constructs a GBM path spec with drift = r_dom − r_for; (2) GpuBackend::price_vanilla(&spec, &payoff) dispatches one workgroup-256 compute pass per path, reads back two f32 partials (sum, sum_sq) per workgroup via a bounded-poll GPU_DISPATCH_TIMEOUT, widens to f64 via pairwise_sum, and returns a Reduction (price = discount * sum/paths, variance); (3) ScenarioPricer prices a full spot×vol Cartesian ScenarioAxes grid in a single dispatch (workgroups = x_blocks × total_nodes) using common random numbers; (4) PathwiseGreeksPricer::estimate returns a GreeksEstimate with five simultaneous estimators (call_price, pathwise_delta, pathwise_vega, digital_price, lr_digital_delta); (5) GpuBackend::gpu_context() returns None when no wgpu adapter is available, triggering automatic transparent CPU fallback. New pricing arms plug in by constructing a PathSpec and calling the relevant Pricer — the GPU dispatch path is not modified.

## `celnet-heston`
**Governed symbols:** `HestonParams`, `MarketInputs`, `carr_madan`, `cos`

celnet-heston is the self-contained Heston stochastic-volatility pricer: it exposes two independent, cross-validated closed-form pricing routes — `cos` (Fang–Oosterlee 2008 cosine-series) and `carr_madan` (Carr–Madan damped-integrand FFT-style) — both accepting `(OptionType, &MarketInputs, &HestonParams) -> f64`. The crate owns its own minimal `Complex` arithmetic (no external num-complex dep), a 16-point Gauss–Legendre panel integrator, and the overflow-stable `char_exponent` CF kernel. A new caller plugs in by constructing `MarketInputs::new(spot, strike, t, r_dom, r_for)` and `HestonParams::new(kappa, theta, vol_of_vol, rho, v0)` then calling either pricer; puts and calls are both returned via exact put–call parity so neither route has a separate put path. The two pricers cross-validate each other in the `carr_madan_and_cos_agree` and `put_call_parity_internal` tests shipped in the crate.

## `celnet-linear`
**Governed symbols:** `pv`, `outright`, `pv`

celnet-linear is the linear-products leaf of the celnet pricing platform: it prices FX outright forwards, FX swaps (two-legged near+far), and non-deliverable forwards (NDFs) against the shared carry seam. Its public contract is three modules — forward (outright PV + greeks), swap (two-leg PV + swap points), ndf (cash-settled NDF via Ndf struct + FixingSource) — all built over the single validated input type LinearInputs::outright / LinearInputs::with_far. A new instrument arm plugs in by constructing a LinearInputs (validated via LinearInputs::validate: positive notional, non-negative settle times) with the appropriate Underlying + Carry tags, then calling pv / greeks (forward), swap::pv / swap_points (swap), or Ndf::new + Ndf::pv (NDF). The crate has no I/O, no allocation in its hot path, and no state beyond the immutable inputs struct.

## `celnet-observability`
**Governed symbols:** `audit_channel`, `publish`, `record_ns`

celnet-observability provides three disjoint, zero-alloc-hot-path observability pillars: (1) a bounded SPSC telemetry ring (HotProbe/TelemetryDrain) for sub-microsecond HotSample capture from the pricing hot core, (2) an unbounded audit channel (AuditSink/AuditDrain) with monotone sequence numbers for lossless compliance-grade audit trails, and (3) HdrHistogram-backed latency recorders (LatencyRecorder/LatencyByKind) with coordinated-omission correction. All three offload to the drain/consumer side; the hot producer path is lock-free and allocation-free, satisfying CLAUDE.md guardrail 11.

## `celnet-parity`
**Governed symbols:** `vanilla_price_matches_closed_form`, `price_and_greeks_are_bit_identical`, `full_greek_set_matches_finite_difference`

celnet-parity is the competitive-parity verification harness: a pure integration-test crate (zero public pricing surface, `#![forbid(unsafe_code)]`) whose 19 numbered rows prove the claims in docs/CAPABILITIES-VS-COMPETITION.md are continuously *true*, not merely asserted. Every row exercises the production crates (`celnet-vanilla`, `celnet-exotics`, `celnet-surface`, etc.) through their public APIs exactly as a downstream consumer would. A regression in any row makes `cargo nextest run -p celnet-parity` fail, so the competitive matrix cannot silently rot — new product arms plug in by adding a new row to the map in lib.rs and a corresponding test file under tests/. The crate carries no runtime code; its only output is a pass/fail gate over the production crates.

## `celnet-plugin-api`
**Governed symbols:** `calibrate`, `price`, `descriptor`, `provides`, `check_no_arbitrage`

celnet-plugin-api is the user-extensibility SDK for the celnet platform. It exposes three object-safe trait seams — PricingModel, SmileModel (: Smile), and Calibration — plus a ModelRegistry trait and a WIT world (`celnet-plugin`) that structurally mirrors them. A new model plugs in by: (1) implementing PricingModel::descriptor + price (and optionally price_and_greeks) for pricing; (2) implementing SmileModel::descriptor + Smile::implied_vol for surface models; (3) implementing Calibration::descriptor + calibrate for surface fitters. The registry routes by ModelId + ModelKind via ModelRegistry::provides/descriptor/of_kind. First-party native models and wasmi-sandboxed Wasm plugins are interchangeable behind the same registry — PricingModel::price takes &CarryInputs (carry-tagged, generalized: Carry::FxRates for FX, Carry::CostOfCarry for equity/commodity) and returns PluginResult<f64>. The contract is side-effect-free and allocation-free on the hot path; determinism (same inputs → bit-identical output via celnet_core::math) is a contract obligation enforced by the replay harness.

## `celnet-plugin-host`
**Governed symbols:** `price`, `price_and_greeks`, `insert`, `load_wasm`

celnet-plugin-host is the Tier-2 wasmi sandbox host for user-supplied pricing models. Its public contract has two seams: (1) ModelRegistry, which accepts native Rust models via register_native / load_wasm and routes price/price_and_greeks calls to any registered model by ModelId; (2) WasmModel, which loads and validates a .wasm blob against a strictly-enumerated capability surface (exactly five math imports: celnet_math::{exp, ln, sqrt, norm_pdf, norm_cdf}), metered per call by a FuelBudget. A new model arm plugs in by implementing the HostModel trait (descriptor + price + price_and_greeks) and calling ModelRegistry::insert or load_wasm.

## `celnet-proto`
**Governed symbols:** `main`, `GetSmile`, `MarkSurface`, `Price`, `RequestQuote`, `Scenario`, `StreamSession`

celnet-proto is the single, unversioned wire contract for the entire Celnet platform (ADR-0007: one clean current contract, no schema_version, no N/N-1 negotiation). It exposes five logical message families — vocabulary (enums + value messages mirroring celnet_types one-to-one), instrument (a unified Instrument oneof covering vanilla, Strategy, SingleBarrier/DoubleBarrier/WindowBarrier, Digital, Touch, VarianceSwap, VolatilitySwap, AsianOption, ForwardStart, Cliquet, Quanto, Tarf, Pivot, Accumulator, Lookback, AmericanOption, BasketOption, PerpetualOption, ListedFutureOption), quote (RFQ lifecycle: QuoteRequest → Quote → QuoteAccept/QuoteReject → Execution), stream (multiplexed bidirectional RFS via StreamService::StreamSession carrying ClientStreamMessage/ServerStreamMessage), and surface (GetSmile/MarkSurface/Scenario workflows). Seven gRPC services are generated: PricingService, QuoteService, StreamService, RiskService, SurfaceService, FixAdminService, AuthService. All message types implement prost::Message; build.rs uses protox (pure-Rust compiler, no system protoc) + tonic-build with skip_protoc_run() for hermetic, reproducible code generation. New arms plug in by evolving celnet.proto and updating every dependent in the same change — zero mixed-version windows.

## `celnet-qmc`
**Governed symbols:** `new`, `rqmc_estimate`, `new`, `stream`

celnet-qmc is the randomized quasi-Monte Carlo substrate for all path-dependent option pricing in celnet. Its public contract is: (1) `SobolSequence::new(dim)` builds a `dim`-dimensional scrambled Sobol sequence from the embedded Joe-Kuo direction-number table (up to MAX_DIM); (2) `BrownianBridge::new(m, t_total)` computes a bisection-order bridge plan for `m` uniform time steps; (3) `rqmc_estimate(bridge, budget, replications, base_seed, payoff)` drives the full RQMC loop — per-replication scramble seeds derived via splitmix, Sobol stream → inverse-normal → bridge build → payoff accumulation — and returns an `RqmcResult` with estimate + inter-replication standard error. A new pricing arm plugs in by passing a `FnMut(&[f64]) -> f64` payoff closure to `rqmc_estimate`; no other crate state is required.

## `celnet-rfq`
**Governed symbols:** `new`, `request`, `request`

celnet-rfq is the concurrent multi-dealer RFQ engine. Its public contract is: (1) MultiDealerEngine::new(sources: Vec<Box<dyn QuoteSource>>) assembles an arbitrary panel of LP adapters behind a single async trait; (2) MultiDealerEngine::request(&self, request, deadline, now_nanos) -> Result<RankedPanel, PanelError> fans out in parallel to every source with a hard per-source timeout, drops non-responders and timeouts, applies last-look staleness filtering, ranks best-bid and best-offer independently with deterministic tie-break, and returns a RankedPanel carrying all rows + per-side winner ids. New LP adapters plug in by implementing QuoteSource::lp_id + QuoteSource::request; the engine itself is not modified.

## `celnet-risk-cube`
**Governed symbols:** `accumulate_fact`, `assemble_capital`, `historical_var_es`, `sensitivity_var_es`

celnet-risk-cube is the risk-aggregation and regulatory-capital crate. Its public seam is the `Cube` (upsert/group_by), three non-additive VaR/ES lenses (historical full-reprice, sensitivity Taylor, curvature), and the FRTB Sensitivity-Based Method assembler (`assemble_capital`). A new asset arm plugs in by: (1) implementing `CarryPricer` for repricing, (2) calling `Cube::upsert` with `RiskFact` objects, and (3) supplying `SbmParams`/`CurvatureBucket` slices to `assemble_capital`. The additive greeks layer (delta, vega ladder) and the non-additive layer (VaR, ES, curvature CVR) are kept strictly separate — additive fields roll up linearly across all assets; VaR/ES are computed per-node by quantile reduction over a shared `quantile_var_es` kernel used identically by both the historical full-reprice path and the sensitivity Taylor path.

## `celnet-risk-fleet`
**Governed symbols:** `parse`, `shard_ids`, `fan_out_aggregate`, `fan_out_aggregate_over`, `partition_facts`

celnet-risk-fleet is the distributed risk-aggregation crate. Its public contract: (1) `partition_facts` / `partition_facts_with` route a flat `[RiskFact]` slice into a `FleetReducer` by HRW-based partition key `(pair, tenant)`, failing closed on an empty replica set; (2) `FleetReducer` exposes `fan_in_additive` (additive Greek/vega-ladder fan-in), `gather_firm_node` (constituent re-gather for non-additive measures), `firm_var_es`, `firm_var_es_sensitivity`, and `firm_curvature_spot`; (3) the `ShardRiskSource` object-safe trait + `fan_out_aggregate_over` / `fan_in_additive_over` / `gather_firm_node_over` extend the same fan-out/fan-in pattern to distributed backends (returning `FleetError::ShardUnavailable` on an unreachable shard); (4) `fan_out_aggregate` / `fan_out_aggregate_over` are the one-shot entry points that produce a `FleetAggregate` (firm node + VaR/ES + curvature-spot + shard_count). `FleetTopology::parse` selects `InProcess` vs `Distributed` at startup from a `(mode, backends)` string pair.

## `celnet-server`
**Governed symbols:** `main`, `price_instrument`, `authorize_caller`, `accept_loop`

celnet-server is the runtime edge that owns the entire server-side lifecycle: WebSocket accept loop, gRPC service handlers (pricing, risk aggregation, quote, session, FIX admin), the multi-product pricer, and all supporting services (access control, price fan-out, click-trade, readiness drain). Its public contract is the unversioned celnet-proto gRPC + WebSocket JSON codec seam. A new product arm plugs in by adding a branch to `price_instrument` and a codec round-trip in `ws/codec.rs`; a new service plugs in by wiring into `WsServices` and calling `authorize_caller` at its entry point.

## `celnet-types`
**Governed symbols:** `Carry`, `code`, `Greeks`, `sign`, `is_premium_adjusted`, `RateSensitivities`, `Underlying`, `new`

celnet-types is the single shared-types crate for the entire celnet platform. Its public contract is: (1) the cross-asset instrument taxonomy (Underlying enum covering Fx/Metal/Equity/Commodity/DigitalAsset variants); (2) the canonical vanilla-option input record VanillaInputs (spot, strike, vol, t, r_dom, r_for) with libm-backed discount-factor and forward helpers; (3) the unified Carry enum (FxRates|CostOfCarry) with carry_rate/forward_factor/discount_df accessors that every analytics leaf consumes; (4) the full Greeks struct (14 first/second/third-order sensitivities); (5) the RateSensitivities enum (Fx rho_dom/rho_for | generalized discount_rho/carry_rho); (6) OptionType (Call/Put) with sign/flip; (7) PremiumStyle (PA/PU) with is_premium_adjusted/flip_orientation; (8) FixingSource enum (6 EM fixings with canonical code() strings). New analytics arms plug into the platform by consuming these types; the crate itself has zero pricing logic.

## `celnet-xva`
**Governed symbols:** `compute_xva`, `simulate`, `net_value`, `survival`

(celnet-xva): the crate computes the three standard counterparty-risk valuation adjustments — CVA (credit), DVA (debit/own-credit), and FVA (funding) — for a synthetic netting set of vanilla FX options. The pipeline is: (1) ExposureProfile::simulate drives spot under risk-neutral GBM with Sobol low-discrepancy normals, reprices each NettedTrade via celnet_vanilla::price, nets within the set, and reduces across paths to EPE(t_k)/ENE(t_k) profiles; (2) SurvivalCurve (flat or piecewise-constant hazard) supplies survival probabilities S(t) = exp(−Λ(t)); (3) compute_xva aggregates over the time grid to produce XvaResult {cva, dva, fva}. The public seam is (XvaInputs → compute_xva → XvaResult), where XvaInputs borrows an ExposureProfile plus counterparty/own SurvivalCurves and LGDs. The crate is #![forbid(unsafe_code)] and explicitly out of scope are collateral/CSA, wrong-way risk, and live credit-curve wiring — those are deploy-gated extensions.

