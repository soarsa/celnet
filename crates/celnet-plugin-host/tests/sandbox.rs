//! Tier-2 sandbox conformance + gate tests for `celnet-plugin-host`.
//!
//! These are the four WS-G gates from `docs/PLUGIN-HOST-ALT.md` §5, plus the
//! boundary obligations, exercised against **real** hand-written core-Wasm guest
//! fixtures (compiled from WAT via the `wat` crate — no mocks):
//!
//! 1. **Capability denial** — a module importing anything outside the granted set
//!    fails to load.
//! 2. **Fuel exhaustion bounded** — an infinite-loop guest traps within its fuel
//!    budget and returns a typed error, never hanging (wrapped in a thread+timeout
//!    that proves boundedness).
//! 3. **Replay bit-identity** — repeated runs are `to_bits`-equal.
//! 4. **Tier-0 == Tier-2 interchangeability** — a native and a Wasm twin
//!    constructed to share float-op order route through one registry and agree to
//!    the bit (bit-identity holds because op order + libm match, not as a general
//!    property of any two pricers).
//!
//! Plus the resource-exhaustion hardening: a guest cannot grow linear memory or
//! declare a minimum past the documented byte cap (it traps / is rejected as a
//! typed `ResourceLimit`, never OOMs the host), disabled Wasm proposals fail
//! validation, and a guest `(start)` function is fuel-bounded so it cannot run
//! unmetered.
//!
//! Every test is timeout-bounded (the harness runs them under a global nextest
//! slow-timeout; the fuel, resource and start tests additionally self-bound with
//! a watchdog thread via [`bounded`]).

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use celnet_core::math;
use celnet_plugin_api::{
    GreekSupport, ModelDescriptor, ModelId, ModelKind, ModelRegistry as _, PluginError,
    PricingModel,
};
use celnet_plugin_host::{
    FuelBudget, HostError, HostModel, ModelRegistry, Snapshot, WasmModel, abi, assert_agree, host,
    replay,
};
use celnet_types::{Greeks, OptionType, VanillaInputs};

// ---------------------------------------------------------------------------
// WAT guest fixtures (real core-Wasm modules built at test time).
// ---------------------------------------------------------------------------

fn wasm(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("fixture WAT must compile to core wasm")
}

/// Conformant discounted-forward pricer. Imports only `celnet_math.exp`.
const FWD_PRICER_WAT: &str = r#"
(module
  (import "celnet_math" "exp" (func $exp (param f64) (result f64)))
  (memory (export "memory") 1)
  (func $df_dom (param $in i32) (result f64)
    (call $exp (f64.mul (f64.neg (f64.load offset=32 (local.get $in)))
                        (f64.load offset=24 (local.get $in)))))
  (func $df_for (param $in i32) (result f64)
    (call $exp (f64.mul (f64.neg (f64.load offset=40 (local.get $in)))
                        (f64.load offset=24 (local.get $in)))))
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param $opt i32) (param $in i32) (param $len i32) (result f64)
    (local $s_disc f64) (local $k_disc f64)
    (local.set $s_disc (f64.mul (f64.load (local.get $in)) (call $df_for (local.get $in))))
    (local.set $k_disc (f64.mul (f64.load offset=8 (local.get $in)) (call $df_dom (local.get $in))))
    (if (result f64) (i32.eqz (local.get $opt))
      (then (f64.sub (local.get $s_disc) (local.get $k_disc)))
      (else (f64.sub (local.get $k_disc) (local.get $s_disc)))))
  (func (export "celnet_price_greeks")
        (param $opt i32) (param $in i32) (param $inlen i32) (param $out i32) (param $outlen i32)
        (result i32)
    (local $dfd f64) (local $dff f64) (local $s f64) (local $k f64) (local $t f64)
    (local $sign f64) (local $price f64)
    (local.set $s (f64.load (local.get $in)))
    (local.set $k (f64.load offset=8 (local.get $in)))
    (local.set $t (f64.load offset=24 (local.get $in)))
    (local.set $dfd (call $df_dom (local.get $in)))
    (local.set $dff (call $df_for (local.get $in)))
    (local.set $sign (if (result f64) (i32.eqz (local.get $opt))
      (then (f64.const 1)) (else (f64.const -1))))
    (local.set $price (f64.mul (local.get $sign)
      (f64.sub (f64.mul (local.get $s) (local.get $dff))
               (f64.mul (local.get $k) (local.get $dfd)))))
    (f64.store (local.get $out) (local.get $price))
    (f64.store offset=8  (local.get $out) (f64.mul (local.get $sign) (local.get $dff)))
    (f64.store offset=16 (local.get $out) (local.get $sign))
    (f64.store offset=24 (local.get $out) (f64.const 0))
    (f64.store offset=32 (local.get $out) (f64.const 0))
    (f64.store offset=40 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.sub
          (f64.mul (f64.mul (f64.load offset=32 (local.get $in)) (local.get $k)) (local.get $dfd))
          (f64.mul (f64.mul (f64.load offset=40 (local.get $in)) (local.get $s)) (local.get $dff)))))
    (f64.store offset=48 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.mul (f64.mul (local.get $k) (local.get $t)) (local.get $dfd))))
    (f64.store offset=56 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.mul (f64.neg (f64.mul (local.get $s) (local.get $t))) (local.get $dff))))
    (f64.store offset=64  (local.get $out) (f64.const 0))
    (f64.store offset=72  (local.get $out) (f64.const 0))
    (f64.store offset=80  (local.get $out) (f64.const 0))
    (f64.store offset=88  (local.get $out) (f64.const 0))
    (f64.store offset=96  (local.get $out) (f64.const 0))
    (f64.store offset=104 (local.get $out) (f64.const 0))
    (i32.const 0)))
"#;

/// Imports a capability the no-WASI linker never grants.
const FORBIDDEN_IMPORT_WAT: &str = r#"
(module
  (import "forbidden_host" "now" (func $now (result i64)))
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64)
    (f64.convert_i64_s (call $now)))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32)
    (i32.const 0)))
"#;

/// `celnet_price` never terminates.
const INFINITE_LOOP_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64)
    (loop $forever (br $forever))
    (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32)
    (loop $forever (br $forever))
    (i32.const 0)))
"#;

/// Emits a non-canonical (signalling-payload) NaN as its price/greek.
const NONCANONICAL_NAN_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64)
    (f64.reinterpret_i64 (i64.const 0x7ff0000000000001)))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32)
    (f64.store (local.get 3) (f64.reinterpret_i64 (i64.const 0x7ff0000000000001)))
    (i32.const 0)))
"#;

/// Always rejects via the negative-status ABI path.
const REJECTING_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64) (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32)
    (i32.const -1)))
"#;

/// A module missing the required `memory` export.
const NO_MEMORY_WAT: &str = r#"
(module
  (func (export "celnet_scratch_in") (result i32) (i32.const 0))
  (func (export "celnet_scratch_out") (result i32) (i32.const 0))
  (func (export "celnet_price") (param i32 i32 i32) (result f64) (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32) (i32.const 0)))
"#;

/// `celnet_price` tries to grow linear memory by 4 GiB worth of pages
/// (65536 pages) — far past the documented `MAX_GUEST_MEMORY_BYTES` cap. With
/// `trap_on_grow_failure`, the over-budget grow traps rather than returning -1,
/// so the host surfaces a typed `ResourceLimit` error instead of OOMing.
const MEMORY_BOMB_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64)
    (drop (memory.grow (i32.const 65535)))
    (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32)
    (drop (memory.grow (i32.const 65535)))
    (i32.const 0)))
"#;

/// Declares a linear memory whose **minimum** (4096 pages = 256 MiB) already
/// exceeds the budget, so instantiation itself must be rejected — a guest cannot
/// reserve an over-budget working set up front.
const HUGE_MIN_MEMORY_WAT: &str = r#"
(module
  (memory (export "memory") 4096)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64) (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32) (i32.const 0)))
"#;

/// Uses a `bulk_memory` instruction (`memory.fill`). The sandbox config disables
/// the proposal, so this must fail validation at load with `InvalidModule`.
const BULK_MEMORY_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64)
    (memory.fill (i32.const 0) (i32.const 0) (i32.const 16))
    (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32) (i32.const 0)))
"#;

/// A well-behaved guest that runs a `(start)` function. The start merely writes a
/// constant into memory — bounded work — proving the host fuels the store before
/// instantiation so a start runs metered and a benign one still loads cleanly.
const BENIGN_START_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func $init (i32.store (i32.const 0) (i32.const 42)))
  (start $init)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64) (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32) (i32.const 0)))
"#;

/// A hostile guest whose `(start)` function never terminates. Because the host
/// seeds fuel before `instantiate_and_start`, the start runs metered and must
/// trap with `FuelExhausted` at load — never hang.
const INFINITE_START_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func $spin (loop $forever (br $forever)))
  (start $spin)
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param i32 i32 i32) (result f64) (f64.const 0))
  (func (export "celnet_price_greeks") (param i32 i32 i32 i32 i32) (result i32) (i32.const 0)))
"#;

// ---------------------------------------------------------------------------
// Native Tier-0 twin of the discounted-forward pricer.
//
// Each method reproduces the EXACT float-operation order of the corresponding
// WAT export (using the same libm `exp`), so the two tiers are bit-identical
// per method — the basis of the interchangeability gate.
// ---------------------------------------------------------------------------

const FWD_ID: ModelId = ModelId("celnet.test.discounted-forward");

#[derive(Debug, Clone, Copy)]
struct TrivialForward;

impl TrivialForward {
    fn df_dom(i: &VanillaInputs) -> f64 {
        math::exp(-i.r_dom * i.t)
    }
    fn df_for(i: &VanillaInputs) -> f64 {
        math::exp(-i.r_for * i.t)
    }
}

impl PricingModel for TrivialForward {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(FWD_ID, ModelKind::Pricing, GreekSupport::FULL)
    }

    fn price(&self, opt: OptionType, i: &VanillaInputs) -> Result<f64, PluginError> {
        // Mirrors `celnet_price`: s_disc, k_disc, then branch.
        let s_disc = i.spot * Self::df_for(i);
        let k_disc = i.strike * Self::df_dom(i);
        Ok(match opt {
            OptionType::Call => s_disc - k_disc,
            OptionType::Put => k_disc - s_disc,
        })
    }

    fn price_and_greeks(&self, opt: OptionType, i: &VanillaInputs) -> Result<Greeks, PluginError> {
        // Mirrors `celnet_price_greeks`: sign*(s*dff - k*dfd) and the analytic
        // sensitivities of that payoff.
        let dfd = Self::df_dom(i);
        let dff = Self::df_for(i);
        let sign = match opt {
            OptionType::Call => 1.0_f64,
            OptionType::Put => -1.0_f64,
        };
        let price = sign * (i.spot * dff - i.strike * dfd);
        Ok(Greeks {
            price,
            delta_spot: sign * dff,
            delta_forward: sign,
            gamma: 0.0,
            vega: 0.0,
            theta: sign * (i.r_dom * i.strike * dfd - i.r_for * i.spot * dff),
            rho_dom: sign * (i.strike * i.t * dfd),
            rho_for: sign * (-(i.spot * i.t) * dff),
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        })
    }
}

fn fwd_descriptor() -> ModelDescriptor {
    ModelDescriptor::new(FWD_ID, ModelKind::Pricing, GreekSupport::FULL)
}

fn market() -> VanillaInputs {
    VanillaInputs::new(1.2150, 1.2000, 0.11, 0.75, 0.043, 0.011)
}

// ---------------------------------------------------------------------------
// Gate 1 — capability denial.
// ---------------------------------------------------------------------------

#[test]
fn capability_denial_unknown_import_fails_to_load() {
    let bytes = wasm(FORBIDDEN_IMPORT_WAT);
    let err = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap_err();
    match err {
        HostError::CapabilityDenied(what) => {
            assert!(
                what.contains("forbidden_host") && what.contains("now"),
                "denial must name the offending import, got {what:?}"
            );
        }
        other => panic!("expected CapabilityDenied, got {other:?}"),
    }
}

#[test]
fn granted_import_set_is_exactly_the_math_surface() {
    // The audited capability surface: only the five celnet_math primitives.
    assert_eq!(host::GRANTED_IMPORTS.len(), 5);
    assert!(host::is_granted("celnet_math", "exp"));
    assert!(host::is_granted("celnet_math", "norm_cdf"));
    // Nothing ambient is ever granted.
    assert!(!host::is_granted("wasi_snapshot_preview1", "fd_write"));
    assert!(!host::is_granted("celnet_math", "rand"));
    assert!(!host::is_granted("env", "now"));
}

#[test]
fn missing_memory_export_is_reported() {
    let bytes = wasm(NO_MEMORY_WAT);
    let err = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap_err();
    assert_eq!(err, HostError::MissingExport("memory".into()));
}

// ---------------------------------------------------------------------------
// Gate 2 — fuel exhaustion is bounded (never hangs).
// ---------------------------------------------------------------------------

#[test]
fn fuel_exhaustion_traps_within_budget_and_does_not_hang() {
    // Run the infinite-loop guest on a worker thread with a small fuel budget and
    // a generous wall-clock watchdog. If fuel metering works the call returns a
    // typed FuelExhausted long before the watchdog; if it hung, the watchdog
    // would fire and we'd fail — proving boundedness, not just correctness.
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let bytes = wasm(INFINITE_LOOP_WAT);
        let model =
            WasmModel::load(fwd_descriptor(), &bytes, FuelBudget(5_000_000)).expect("loads");
        let res = model.price(OptionType::Call, &market());
        let _ = tx.send(res);
    });

    let outcome = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("infinite-loop guest must be interrupted by fuel, not hang");
    handle.join().expect("worker thread panicked");

    assert_eq!(
        outcome,
        Err(HostError::FuelExhausted),
        "an infinite loop must exhaust fuel and return the typed error"
    );
}

#[test]
fn ample_fuel_lets_a_real_pricer_complete() {
    let bytes = wasm(FWD_PRICER_WAT);
    let model = WasmModel::load(fwd_descriptor(), &bytes, FuelBudget(50_000_000)).unwrap();
    let price = model.price(OptionType::Call, &market()).unwrap();
    // Sanity vs the native twin (closed form), not just "did not error".
    let expect = TrivialForward.price(OptionType::Call, &market()).unwrap();
    assert_eq!(price.to_bits(), expect.to_bits());
}

// ---------------------------------------------------------------------------
// Resource limits — a guest cannot exhaust host memory/tables (DoS).
//
// Each test is wrapped in a worker thread + bounded `recv_timeout`, so a guest
// that managed to *hang* (rather than being denied/trapped) would fail the test
// via the watchdog instead of stalling the suite — boundedness, not just
// correctness.
// ---------------------------------------------------------------------------

/// Run `f` on a worker thread, requiring it to finish within `secs`. Returns its
/// value; panics (failing the test) if it does not terminate in time, so a hang
/// can never pass as success.
fn bounded<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let _ = tx.send(f());
    });
    let out = rx
        .recv_timeout(Duration::from_secs(secs))
        .expect("operation must terminate within the watchdog, not hang");
    handle.join().expect("worker thread panicked");
    out
}

#[test]
fn memory_grow_beyond_budget_traps_as_resource_limit() {
    let outcome = bounded(20, || {
        let bytes = wasm(MEMORY_BOMB_WAT);
        // Loads fine (declared min is 1 page); the bomb fires only when called.
        let model = WasmModel::load_default(fwd_descriptor(), &bytes).expect("loads");
        model.price(OptionType::Call, &market())
    });
    assert_eq!(
        outcome,
        Err(HostError::ResourceLimit("growth operation limited".into())),
        "an over-budget memory.grow must trap as a typed ResourceLimit, not OOM"
    );
}

#[test]
fn declared_minimum_memory_beyond_budget_is_rejected_at_load() {
    let outcome = bounded(20, || {
        let bytes = wasm(HUGE_MIN_MEMORY_WAT);
        WasmModel::load_default(fwd_descriptor(), &bytes)
    });
    match outcome {
        Err(HostError::ResourceLimit(_)) => {}
        other => panic!("expected ResourceLimit at instantiation, got {other:?}"),
    }
}

#[test]
fn disabled_bulk_memory_proposal_fails_validation() {
    let bytes = wasm(BULK_MEMORY_WAT);
    let err = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap_err();
    assert!(
        matches!(err, HostError::InvalidModule(_)),
        "a module using the disabled bulk_memory proposal must fail validation, got {err:?}"
    );
}

#[test]
fn documented_resource_budgets_are_small_and_finite() {
    // The caps the sandbox advertises are small, finite, and exactly what the
    // limiter installs — guarding against an accidental loosening.
    assert_eq!(host::MAX_GUEST_MEMORY_BYTES, 64 * 1024 * 1024);
    assert_eq!(host::MAX_GUEST_MEMORIES, 1);
    assert_eq!(host::MAX_GUEST_TABLES, 1);
    assert_eq!(host::MAX_GUEST_INSTANCES, 1);
    const { assert!(host::MAX_GUEST_TABLE_ELEMENTS <= 4_096) };
}

// ---------------------------------------------------------------------------
// Guest `(start)` functions are fuel-bounded (no unmetered start).
// ---------------------------------------------------------------------------

#[test]
fn benign_start_function_runs_metered_and_loads() {
    let outcome = bounded(20, || {
        let bytes = wasm(BENIGN_START_WAT);
        WasmModel::load_default(fwd_descriptor(), &bytes).map(|_| ())
    });
    outcome.expect("a benign, bounded (start) must load cleanly");
}

#[test]
fn infinite_start_function_is_fuel_bounded_at_load_and_does_not_hang() {
    // The host seeds fuel before instantiate_and_start, so a runaway (start)
    // traps with FuelExhausted at load instead of hanging the host.
    let outcome = bounded(20, || {
        let bytes = wasm(INFINITE_START_WAT);
        WasmModel::load(fwd_descriptor(), &bytes, FuelBudget(5_000_000)).map(|_| ())
    });
    assert_eq!(
        outcome,
        Err(HostError::FuelExhausted),
        "an infinite (start) must exhaust the pre-seeded fuel and fail to load"
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — deterministic replay bit-identity.
// ---------------------------------------------------------------------------

#[test]
fn replay_is_bit_identical_across_runs() {
    let bytes = wasm(FWD_PRICER_WAT);
    let model = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap();
    for opt in [OptionType::Call, OptionType::Put] {
        let snap = Snapshot::new(opt, market());
        let outcome = replay::replay(&model, snap, 16).expect("replay must be bit-identical");
        assert_eq!(outcome.runs, 16);
        // Re-running the harness yields the very same bits again.
        let again = replay::replay(&model, snap, 4).unwrap();
        assert_eq!(outcome.price.to_bits(), again.price.to_bits());
        assert!(replay::greeks_bits_eq(&outcome.greeks, &again.greeks));
    }
}

#[test]
fn noncanonical_nan_is_canonicalized_at_the_boundary() {
    let bytes = wasm(NONCANONICAL_NAN_WAT);
    let model = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap();
    let price = model.price(OptionType::Call, &market()).unwrap();
    assert!(price.is_nan());
    // The guest emitted 0x7ff0000000000001; the host must collapse it to the
    // single canonical pattern so replay is bit-stable.
    assert_eq!(price.to_bits(), abi::CANONICAL_NAN_BITS);

    let g = model.price_and_greeks(OptionType::Call, &market()).unwrap();
    assert_eq!(g.price.to_bits(), abi::CANONICAL_NAN_BITS);
}

// ---------------------------------------------------------------------------
// Gate 4 — Tier-0 (native) == Tier-2 (wasm) through one registry.
// ---------------------------------------------------------------------------

#[test]
fn native_and_wasm_twins_agree_through_one_registry() {
    let bytes = wasm(FWD_PRICER_WAT);

    let mut reg = ModelRegistry::new();
    // Tier-0 native and Tier-2 wasm under DISTINCT ids (the registry rejects dup
    // ids), but the SAME pricing logic.
    let native_id = reg
        .register_native(NativeIdShim {
            inner: TrivialForward,
            id: ModelId("celnet.test.fwd.native"),
        })
        .unwrap();
    let wasm_id = reg
        .load_wasm(
            ModelDescriptor::new(
                ModelId("celnet.test.fwd.wasm"),
                ModelKind::Pricing,
                GreekSupport::FULL,
            ),
            &bytes,
            FuelBudget::default(),
        )
        .unwrap();

    // The registry routes both, tier-blind, behind the frozen discovery contract.
    assert_eq!(reg.len(), 2);
    assert!(reg.provides(native_id, ModelKind::Pricing));
    assert!(reg.provides(wasm_id, ModelKind::Pricing));

    let native = reg.model(native_id).unwrap();
    let wasmm = reg.model(wasm_id).unwrap();

    // Across a battery of markets and both option types, the two tiers price
    // bit-identically through the unified registry.
    let markets = [
        market(),
        VanillaInputs::new(100.0, 95.0, 0.2, 1.0, 0.05, 0.0),
        VanillaInputs::new(1.35, 1.40, 0.08, 2.0, 0.02, 0.018),
        VanillaInputs::new(0.85, 0.90, 0.3, 0.25, 0.01, 0.03),
    ];
    for m in markets {
        for opt in [OptionType::Call, OptionType::Put] {
            assert_agree(native, wasmm, Snapshot::new(opt, m))
                .unwrap_or_else(|e| panic!("tier-0 vs tier-2 disagreed for {opt:?} {m:?}: {e}"));
        }
    }
}

/// Wraps a native model under a chosen id (so two registry entries from the same
/// pricer can coexist without colliding on `FWD_ID`).
#[derive(Debug, Clone, Copy)]
struct NativeIdShim {
    inner: TrivialForward,
    id: ModelId,
}

impl PricingModel for NativeIdShim {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(self.id, ModelKind::Pricing, GreekSupport::FULL)
    }
    fn price(&self, opt: OptionType, i: &VanillaInputs) -> Result<f64, PluginError> {
        self.inner.price(opt, i)
    }
    fn price_and_greeks(&self, opt: OptionType, i: &VanillaInputs) -> Result<Greeks, PluginError> {
        self.inner.price_and_greeks(opt, i)
    }
}

// ---------------------------------------------------------------------------
// Registry semantics + model-domain error surfacing.
// ---------------------------------------------------------------------------

#[test]
fn registry_rejects_duplicate_ids() {
    let mut reg = ModelRegistry::new();
    reg.register_native(TrivialForward).unwrap();
    let err = reg.register_native(TrivialForward).unwrap_err();
    assert_eq!(
        err,
        HostError::Model(PluginError::InvalidInput("duplicate model id"))
    );
}

#[test]
fn unknown_model_id_is_not_found() {
    let reg = ModelRegistry::new();
    match reg.model(ModelId("nope")) {
        Err(e) => assert_eq!(e, HostError::Model(PluginError::NotFound("model id"))),
        Ok(_) => panic!("an empty registry has no models"),
    }
}

#[test]
fn guest_domain_rejection_surfaces_as_model_error_not_sandbox_fault() {
    let bytes = wasm(REJECTING_WAT);
    let model = WasmModel::load_default(fwd_descriptor(), &bytes).unwrap();
    // price() returns the sentinel 0.0 fine; the Greeks path returns status -1,
    // which the host maps to a portable model error (NOT a Trap/Fuel fault).
    let err = model
        .price_and_greeks(OptionType::Call, &market())
        .unwrap_err();
    assert_eq!(
        err,
        HostError::Model(PluginError::InvalidInput("guest rejected inputs"))
    );
}

#[test]
fn invalid_module_bytes_are_rejected() {
    let err = WasmModel::load_default(fwd_descriptor(), b"not a wasm module").unwrap_err();
    assert!(matches!(err, HostError::InvalidModule(_)), "got {err:?}");
}

// ---------------------------------------------------------------------------
// ABI unit checks (NaN canonicalization + option discriminant round-trips).
// ---------------------------------------------------------------------------

#[test]
fn abi_canonicalizes_every_nan_payload() {
    for payload in [1u64, 0xdead_beef, 0x000f_ffff_ffff_ffff] {
        let snan = f64::from_bits(0x7ff0_0000_0000_0000 | payload);
        assert!(snan.is_nan());
        assert_eq!(abi::canonicalize(snan).to_bits(), abi::CANONICAL_NAN_BITS);
    }
    // Finite values, signed zeros and infinities pass through untouched.
    assert_eq!(abi::canonicalize(-0.0).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(abi::canonicalize(0.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(abi::canonicalize(f64::INFINITY), f64::INFINITY);
    assert_eq!(abi::canonicalize(3.5), 3.5);
}

#[test]
fn abi_option_discriminant_round_trips() {
    assert_eq!(
        abi::opt_from_abi(abi::opt_to_abi(OptionType::Call)),
        Some(OptionType::Call)
    );
    assert_eq!(
        abi::opt_from_abi(abi::opt_to_abi(OptionType::Put)),
        Some(OptionType::Put)
    );
    assert_eq!(abi::opt_from_abi(2), None);
    assert_eq!(abi::opt_from_abi(-1), None);
}
