//! Tier-2 — the untrusted, deterministic, fuel-metered Wasm sandbox (wasmi).
//!
//! This is the advisory-clean replacement for the originally-planned wasmtime
//! runtime (`docs/PLUGIN-HOST-ALT.md` §2/§7). A user-supplied **core Wasm
//! module** is loaded into a wasmi [`Engine`] configured with `consume_fuel`,
//! linked against the [`crate::host`] capability surface (and **nothing else** —
//! no WASI), and called across the host-controlled [`crate::abi`] `(ptr,len)`
//! convention. Every boundary `f64` is NaN-canonicalized; a per-call fuel budget
//! is the model's compute SLA and its exhaustion is a typed
//! [`crate::HostError::FuelExhausted`], never a hang or panic.
//!
//! # Guest ABI (host-defined core-module contract)
//!
//! A conformant guest exports:
//! - `memory` — its linear memory.
//! - `celnet_scratch_in: () -> i32` — pointer to a guest buffer at least
//!   [`crate::abi::INPUT_BYTES`] long the host serializes [`CarryInputs`] into
//!   (the six `f64` numeric words then the two `i32` discriminant words).
//! - `celnet_scratch_out: () -> i32` — pointer to a guest buffer at least
//!   [`crate::abi::GREEKS_BYTES`] long the host reads [`CarryGreeks`] back from
//!   (the [`crate::abi::GREEKS_FIELDS`] `f64` words then the `i32` `rate_kind`).
//! - `celnet_price: (opt: i32, in_ptr: i32, in_len: i32) -> f64` — the price.
//! - `celnet_price_greeks: (opt: i32, in_ptr: i32, in_len: i32, out_ptr: i32,
//!   out_len: i32) -> i32` — writes the [`crate::abi::GREEKS_FIELDS`] greek words
//!   plus the trailing `rate_kind` `i32` to `out_ptr` and returns `0` on success
//!   or a negative [`AbiStatus`] code on a model-domain rejection.
//!
//! `describe` metadata travels out-of-band as the [`ModelDescriptor`] supplied at
//! load time (the host owns identity/routing), keeping the guest ABI minimal.

use core::cell::RefCell;

use celnet_core::{CarryGreeks, CarryInputs};
use celnet_plugin_api::{ModelDescriptor, PluginError};
use celnet_types::OptionType;
use wasmi::{Config, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::abi::{GREEKS_BYTES, INPUT_BYTES, input_to_bytes, opt_to_abi};
use crate::error::{HostError, HostResult};
use crate::host::{self, GuestState, is_granted};
use crate::model::HostModel;

/// Negative status codes a guest returns from `celnet_price_greeks` to signal a
/// model-domain rejection without trapping. Mapped back to the portable
/// [`PluginError`] so a Tier-2 rejection is indistinguishable from a Tier-0 one.
///
/// `0` means success; any other (negative) value maps as below. An unrecognized
/// negative code is treated as a generic invalid-input rejection rather than a
/// host fault, since it is the *guest's* verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AbiStatus {
    /// The guest rejected the inputs as outside its domain.
    InvalidInput = -1,
    /// The guest does not produce Greeks.
    Unsupported = -2,
    /// A guest numerical routine did not converge.
    DidNotConverge = -3,
}

impl AbiStatus {
    /// Translate a guest status word into a portable model error (or `Ok` for 0).
    fn from_code(code: i32) -> Result<(), PluginError> {
        match code {
            0 => Ok(()),
            -2 => Err(PluginError::Unsupported("guest does not produce Greeks")),
            -3 => Err(PluginError::DidNotConverge(
                "guest routine did not converge",
            )),
            // -1 and any other negative code: the guest's domain rejection.
            _ => Err(PluginError::InvalidInput("guest rejected inputs")),
        }
    }
}

/// Per-call fuel budget — the deterministic compute SLA for a Tier-2 model.
///
/// Fuel is wasmi's deterministic instruction-cost counter (R1-compatible, unlike
/// wall-clock interruption). Exhaustion interrupts the interpreter at the budget
/// and yields [`HostError::FuelExhausted`]. The default is generous enough for a
/// per-quote vanilla/smile/calibration evaluation yet finite, so a runaway guest
/// is always bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuelBudget(pub u64);

impl FuelBudget {
    /// A default per-call budget suited to a per-quote (not per-tick) model.
    pub const DEFAULT: FuelBudget = FuelBudget(50_000_000);
}

impl Default for FuelBudget {
    fn default() -> Self {
        FuelBudget::DEFAULT
    }
}

/// The cached, typed guest entry points resolved once at load time.
struct Exports {
    memory: Memory,
    scratch_in: TypedFunc<(), i32>,
    scratch_out: TypedFunc<(), i32>,
    price: TypedFunc<(i32, i32, i32), f64>,
    price_greeks: TypedFunc<(i32, i32, i32, i32, i32), i32>,
}

/// Live, per-call mutable state (the store + instance) behind a `RefCell` so the
/// [`HostModel`] `&self` methods can reset fuel and execute deterministically.
struct Live {
    store: Store<GuestState>,
    exports: Exports,
}

/// A loaded, sandboxed Tier-2 pricing model.
///
/// Owns its own wasmi [`Store`] and instance; it is single-threaded (one pricing
/// worker per handle), matching how the engine fans a portfolio across workers.
/// Not `Sync` by construction (the `RefCell`), which is the intended ownership
/// model.
pub struct WasmModel {
    descriptor: ModelDescriptor,
    fuel_budget: FuelBudget,
    live: RefCell<Live>,
}

impl core::fmt::Debug for WasmModel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WasmModel")
            .field("descriptor", &self.descriptor)
            .field("fuel_budget", &self.fuel_budget)
            .finish_non_exhaustive()
    }
}

/// Build the canonical deterministic wasmi [`Config`] for the sandbox.
///
/// `consume_fuel(true)` enables the metered SLA. Beyond that we deliberately
/// **shrink the accepted Wasm feature set** to the minimum the host ABI needs,
/// turning off proposals a conformant pricing guest never uses:
///
/// - **`memory64`** — off. A 64-bit address space is a resource-exhaustion
///   surface and irrelevant to a pricer; the ABI's `(ptr,len)` words are `i32`.
/// - **`bulk_memory`** — off. The ABI moves bytes through host-driven
///   `Memory::read`/`Memory::write`, not guest `memory.copy`/`memory.fill`, so
///   the proposal (and its `memory.init`/`data.drop` allocation surface) is not
///   required.
/// - **`reference_types`** / **`tail_call`** — off. No `externref`/`funcref`
///   tables or guaranteed tail calls are part of the call-by-export ABI.
///
/// What stays on is the deterministic MVP-plus core (mutable globals, multi-value,
/// sign-extension, saturating float→int, extended const): floating point in core
/// Wasm is IEEE-754 and, with boundary NaN canonicalization plus libm
/// transcendentals, fully reproducible. Every disabled proposal makes a guest
/// using it fail validation in [`WasmModel::load`] with [`HostError::InvalidModule`]
/// rather than silently widening the attack surface.
#[must_use]
pub fn sandbox_config() -> Config {
    let mut config = Config::default();
    config.consume_fuel(true);
    // Narrow the accepted feature set: disable proposals the ABI does not use.
    config.wasm_memory64(false);
    config.wasm_bulk_memory(false);
    config.wasm_reference_types(false);
    config.wasm_tail_call(false);
    config
}

impl WasmModel {
    /// Load and instantiate a core Wasm module as a Tier-2 model under
    /// `descriptor`, granting it `fuel_budget` per call.
    ///
    /// Linking uses the [`crate::host`] capability surface and **no WASI**, so a
    /// module importing anything outside [`host::GRANTED_IMPORTS`] fails here with
    /// [`HostError::CapabilityDenied`] — the capability-denial guarantee.
    ///
    /// # Errors
    /// - [`HostError::InvalidModule`] if `wasm` is not a valid core module.
    /// - [`HostError::CapabilityDenied`] if it imports a non-granted capability.
    /// - [`HostError::MissingExport`] if a required ABI export is absent or has the
    ///   wrong signature.
    pub fn load(
        descriptor: ModelDescriptor,
        wasm: &[u8],
        fuel_budget: FuelBudget,
    ) -> HostResult<Self> {
        let engine = Engine::new(&sandbox_config());
        let module = Module::new(&engine, wasm)
            .map_err(|e| HostError::InvalidModule(short(&e.to_string())))?;

        // Capability pre-check: reject any import outside the granted set *before*
        // linking, with a precise, attributable error. (Linking would also fail,
        // but this names the offending import deterministically.)
        for import in module.imports() {
            if !is_granted(import.module(), import.name()) {
                return Err(HostError::CapabilityDenied(format!(
                    "{}::{}",
                    import.module(),
                    import.name()
                )));
            }
        }

        let mut linker: Linker<GuestState> = Linker::new(&engine);
        host::install_capabilities(&mut linker)
            .map_err(|e| HostError::InvalidModule(short(&e.to_string())))?;

        let mut store = host::new_store(&engine);
        // Fuel the store to the per-call budget *before* instantiation. wasmi
        // only exposes `instantiate_and_start`, which runs any module `(start)`
        // function inline; a fresh store starts with **0 fuel**, so without this
        // a guest `(start)` could execute unmetered. Seeding the budget here makes
        // the start path fuel-bounded exactly like a normal call — an infinite or
        // runaway `(start)` traps as [`HostError::FuelExhausted`] instead of
        // hanging the host. The first real `price`/`price_and_greeks` call resets
        // fuel back to the full budget via [`WasmModel::refuel`], so start-time
        // consumption never eats into a later call's SLA.
        store
            .set_fuel(fuel_budget.0)
            .map_err(|e| HostError::AbiViolation(short(&e.to_string())))?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| map_instantiation_error(&e))?;

        let exports = resolve_exports(&instance, &mut store)?;
        Ok(Self {
            descriptor,
            fuel_budget,
            live: RefCell::new(Live { store, exports }),
        })
    }

    /// Load with the default per-call [`FuelBudget`].
    ///
    /// # Errors
    /// As [`WasmModel::load`].
    pub fn load_default(descriptor: ModelDescriptor, wasm: &[u8]) -> HostResult<Self> {
        Self::load(descriptor, wasm, FuelBudget::default())
    }

    /// The per-call fuel budget this model runs under.
    #[must_use]
    pub const fn fuel_budget(&self) -> FuelBudget {
        self.fuel_budget
    }

    /// Reset the store's fuel to the per-call budget. Called before every guest
    /// invocation so each call gets the full deterministic SLA and fuel
    /// accounting is part of the replay invariant.
    fn refuel(live: &mut Live, budget: FuelBudget) -> HostResult<()> {
        live.store
            .set_fuel(budget.0)
            .map_err(|e| HostError::AbiViolation(short(&e.to_string())))
    }

    /// Serialize `inputs` into the guest's input scratch buffer, returning the
    /// `(ptr, len)` the host passes to the guest entry point.
    fn marshal_inputs(live: &mut Live, inputs: &CarryInputs) -> HostResult<(i32, i32)> {
        let ptr = live
            .exports
            .scratch_in
            .call(&mut live.store, ())
            .map_err(|e| classify_call_error(&e))?;
        let bytes = input_to_bytes(inputs);
        write_guest(&live.exports.memory, &mut live.store, ptr, &bytes)?;
        Ok((ptr, INPUT_BYTES as i32))
    }
}

impl HostModel for WasmModel {
    fn descriptor(&self) -> ModelDescriptor {
        self.descriptor
    }

    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> HostResult<f64> {
        let mut guard = self.live.borrow_mut();
        let live = &mut *guard;
        Self::refuel(live, self.fuel_budget)?;
        let (in_ptr, in_len) = Self::marshal_inputs(live, inputs)?;
        let raw = live
            .exports
            .price
            .call(&mut live.store, (opt_to_abi(opt), in_ptr, in_len))
            .map_err(|e| classify_call_error(&e))?;
        Ok(crate::abi::canonicalize(raw))
    }

    fn price_and_greeks(&self, opt: OptionType, inputs: &CarryInputs) -> HostResult<CarryGreeks> {
        let mut guard = self.live.borrow_mut();
        let live = &mut *guard;
        Self::refuel(live, self.fuel_budget)?;
        let (in_ptr, in_len) = Self::marshal_inputs(live, inputs)?;
        let out_ptr = live
            .exports
            .scratch_out
            .call(&mut live.store, ())
            .map_err(|e| classify_call_error(&e))?;
        let status = live
            .exports
            .price_greeks
            .call(
                &mut live.store,
                (
                    opt_to_abi(opt),
                    in_ptr,
                    in_len,
                    out_ptr,
                    GREEKS_BYTES as i32,
                ),
            )
            .map_err(|e| classify_call_error(&e))?;
        AbiStatus::from_code(status)?;

        let mut buf = [0u8; GREEKS_BYTES];
        read_guest(&live.exports.memory, &live.store, out_ptr, &mut buf)?;
        crate::abi::greeks_from_bytes(&buf)
            .ok_or_else(|| HostError::AbiViolation("greeks buffer length mismatch".into()))
    }
}

/// Resolve and type-check the required guest exports once at load time.
fn resolve_exports(instance: &Instance, store: &mut Store<GuestState>) -> HostResult<Exports> {
    let memory = instance
        .get_memory(&*store, "memory")
        .ok_or_else(|| HostError::MissingExport("memory".into()))?;
    let scratch_in = typed::<(), i32>(instance, store, "celnet_scratch_in")?;
    let scratch_out = typed::<(), i32>(instance, store, "celnet_scratch_out")?;
    let price = typed::<(i32, i32, i32), f64>(instance, store, "celnet_price")?;
    let price_greeks =
        typed::<(i32, i32, i32, i32, i32), i32>(instance, store, "celnet_price_greeks")?;
    Ok(Exports {
        memory,
        scratch_in,
        scratch_out,
        price,
        price_greeks,
    })
}

/// Fetch a typed export, attributing a missing/mistyped one as a host error.
fn typed<Params, Results>(
    instance: &Instance,
    store: &mut Store<GuestState>,
    name: &'static str,
) -> HostResult<TypedFunc<Params, Results>>
where
    Params: wasmi::WasmParams,
    Results: wasmi::WasmResults,
{
    instance
        .get_typed_func::<Params, Results>(&*store, name)
        .map_err(|_| HostError::MissingExport(name.into()))
}

/// Write `bytes` into guest linear memory at `ptr`, bounds-checked by wasmi.
fn write_guest(
    memory: &Memory,
    store: &mut Store<GuestState>,
    ptr: i32,
    bytes: &[u8],
) -> HostResult<()> {
    let offset =
        usize::try_from(ptr).map_err(|_| HostError::AbiViolation("negative ptr".into()))?;
    memory
        .write(store, offset, bytes)
        .map_err(|e| HostError::AbiViolation(short(&e.to_string())))
}

/// Read `buf.len()` bytes from guest linear memory at `ptr`, bounds-checked.
fn read_guest(
    memory: &Memory,
    store: &Store<GuestState>,
    ptr: i32,
    buf: &mut [u8],
) -> HostResult<()> {
    let offset =
        usize::try_from(ptr).map_err(|_| HostError::AbiViolation("negative ptr".into()))?;
    memory
        .read(store, offset, buf)
        .map_err(|e| HostError::AbiViolation(short(&e.to_string())))
}

/// Classify a wasmi call error into the host taxonomy.
///
/// Fuel exhaustion is detected via the [`wasmi::TrapCode::OutOfFuel`] trap code
/// and mapped to the typed [`HostError::FuelExhausted`]; a resource-cap denial
/// (`memory.grow`/`table.grow` past the budget, which traps because the store
/// limiter is built with `trap_on_grow_failure`) maps to
/// [`HostError::ResourceLimit`]. Every other trap becomes [`HostError::Trapped`].
/// None of these is ever a panic or a hang.
fn classify_call_error(e: &wasmi::Error) -> HostError {
    match e.as_trap_code() {
        Some(wasmi::TrapCode::OutOfFuel) => HostError::FuelExhausted,
        Some(wasmi::TrapCode::GrowthOperationLimited) => {
            HostError::ResourceLimit(short(&e.to_string()))
        }
        _ => HostError::Trapped(short(&e.to_string())),
    }
}

/// Map an instantiation error, attributing an unresolved import (which should
/// have been caught by the pre-check, but is mapped precisely here too) to a
/// capability denial, an over-budget declared resource (e.g. a linear-memory or
/// table minimum past the store limiter's cap, denied during the initial
/// allocation) to [`HostError::ResourceLimit`], a fuel-exhausted module `(start)`
/// to [`HostError::FuelExhausted`], and anything else to an invalid-module fault.
fn map_instantiation_error(e: &wasmi::Error) -> HostError {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
        return HostError::FuelExhausted;
    }
    if e.as_trap_code() == Some(wasmi::TrapCode::GrowthOperationLimited) {
        return HostError::ResourceLimit(short(&e.to_string()));
    }
    let msg = e.to_string();
    if msg.contains("resource limiter denied") {
        HostError::ResourceLimit(short(&msg))
    } else if msg.contains("cannot find")
        || msg.contains("unknown import")
        || msg.contains("imported")
    {
        HostError::CapabilityDenied(short(&msg))
    } else {
        HostError::InvalidModule(short(&msg))
    }
}

/// Trim a runtime error string to a short, log-friendly, provenance-neutral
/// single line (cap length so a hostile module can't bloat an error).
fn short(s: &str) -> String {
    let line = s.lines().next().unwrap_or(s);
    let trimmed: String = line.chars().take(160).collect();
    trimmed
}
