//! The Tier-2 capability surface — the *only* authority a Wasm guest receives.
//!
//! A wasmi [`Linker`] resolves a guest module's imports against the set of host
//! functions registered on it. We register **only** an explicit, audited set of
//! deterministic pricing primitives and link **no WASI**, so a guest has **zero
//! ambient authority**: no clock, no RNG, no threads, no filesystem, no network
//! (requirement R2 of `docs/PLUGIN-HOST-ALT.md`). Any import the guest declares
//! that is not in this set fails to link — the capability-denial behaviour the
//! gate proves.
//!
//! The granted primitives are the `celnet_core::math` (`rust-lang/libm`)
//! transcendentals, exposed under the `"celnet_math"` import module. Routing the
//! guest's `exp`/`ln`/`sqrt`/`norm_pdf`/`norm_cdf` through the *same* libm the
//! native tier uses is what makes a Tier-2 model bit-identical to its Tier-0
//! twin — the determinism contract (§6). A guest is free to compile its own libm
//! instead and import nothing; the reference guest imports these so the host is
//! the single source of transcendental truth.

use celnet_core::math;
use wasmi::{Linker, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder};

use crate::abi::canonicalize;

/// The import module name under which the host's pricing primitives live.
///
/// Vendor- and provenance-neutral: it names the *purpose* (Celnet math), never a
/// library or author.
pub const MATH_MODULE: &str = "celnet_math";

/// The exact, exhaustive set of host functions a Tier-2 guest may import.
///
/// Anything outside this list is denied at link time. Kept as a constant so the
/// capability surface is auditable in one place and asserted by tests.
pub const GRANTED_IMPORTS: &[(&str, &str)] = &[
    (MATH_MODULE, "exp"),
    (MATH_MODULE, "ln"),
    (MATH_MODULE, "sqrt"),
    (MATH_MODULE, "norm_pdf"),
    (MATH_MODULE, "norm_cdf"),
];

/// Whether `(module, name)` is a host-granted capability.
#[must_use]
pub fn is_granted(module: &str, name: &str) -> bool {
    GRANTED_IMPORTS
        .iter()
        .any(|&(m, n)| m == module && n == name)
}

/// Register the entire capability surface onto `linker`.
///
/// Each shim canonicalizes its argument and result so a non-canonical NaN can
/// neither enter nor leave a host primitive, preserving the replay invariant
/// across the boundary. The functions are pure and side-effect-free; `T` is the
/// guest's store data type (the host carries no per-call state in these shims).
///
/// # Errors
/// Returns the underlying [`wasmi::errors::LinkerError`] if a name is defined
/// twice (a host bug, not reachable with the constant set above).
pub fn install_capabilities<T>(linker: &mut Linker<T>) -> Result<(), wasmi::Error> {
    // Each primitive is the libm-backed `celnet_core::math` function, wrapped so
    // the boundary NaN-canonicalization rule holds in both directions.
    linker.func_wrap(MATH_MODULE, "exp", |x: f64| {
        canonicalize(math::exp(canonicalize(x)))
    })?;
    linker.func_wrap(MATH_MODULE, "ln", |x: f64| {
        canonicalize(math::ln(canonicalize(x)))
    })?;
    linker.func_wrap(MATH_MODULE, "sqrt", |x: f64| {
        canonicalize(math::sqrt(canonicalize(x)))
    })?;
    linker.func_wrap(MATH_MODULE, "norm_pdf", |x: f64| {
        canonicalize(math::norm_pdf(canonicalize(x)))
    })?;
    linker.func_wrap(MATH_MODULE, "norm_cdf", |x: f64| {
        canonicalize(math::norm_cdf(canonicalize(x)))
    })?;
    Ok(())
}

/// Maximum linear-memory **bytes** a single Tier-2 guest may hold (initial +
/// grown). 64 MiB = 1024 Wasm pages — generous for a per-quote pricer's working
/// set (snapshot buffers, smile node arrays, a calibration scratchpad) yet a
/// hard ceiling so a hostile or buggy guest cannot exhaust host memory via
/// `memory.grow` or a huge declared minimum. A grow past this cap traps
/// deterministically as [`crate::HostError::ResourceLimit`]; a declared minimum
/// past it is rejected at instantiation.
pub const MAX_GUEST_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Maximum number of elements across a guest's table(s). The pricing ABI is
/// call-by-export and needs no `call_indirect` dispatch table at all, so this is
/// deliberately tiny — enough to admit a small indirect-call/function-pointer
/// table a toolchain might emit, never enough to be a memory-exhaustion vector.
pub const MAX_GUEST_TABLE_ELEMENTS: usize = 4_096;

/// Maximum number of linear memories a guest may instantiate. The ABI uses a
/// single exported `memory`; multi-memory is disabled in [`crate::wasm::sandbox_config`],
/// so one is the only legitimate value. Capped explicitly as defence in depth.
pub const MAX_GUEST_MEMORIES: usize = 1;

/// Maximum number of tables a guest may instantiate. One suffices for any
/// `call_indirect` a toolchain emits; more is never required by the ABI.
pub const MAX_GUEST_TABLES: usize = 1;

/// Maximum number of instances a guest module may create within its store. Each
/// [`crate::wasm::WasmModel`] instantiates exactly one module, so one is the
/// legitimate budget.
pub const MAX_GUEST_INSTANCES: usize = 1;

/// Build the static [`StoreLimits`] enforcing the documented Tier-2 resource
/// budgets ([`MAX_GUEST_MEMORY_BYTES`], [`MAX_GUEST_TABLE_ELEMENTS`], …).
///
/// `trap_on_grow_failure(true)` makes an over-budget `memory.grow`/`table.grow`
/// raise a deterministic trap (mapped to [`crate::HostError::ResourceLimit`])
/// rather than returning the Wasm `-1` sentinel — so a guest cannot silently
/// observe-and-spin on a denied allocation, and the host never OOMs or hangs.
#[must_use]
pub fn guest_store_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MAX_GUEST_MEMORY_BYTES)
        .table_elements(MAX_GUEST_TABLE_ELEMENTS)
        .memories(MAX_GUEST_MEMORIES)
        .tables(MAX_GUEST_TABLES)
        .instances(MAX_GUEST_INSTANCES)
        .trap_on_grow_failure(true)
        .build()
}

/// Per-call store data for a Tier-2 model.
///
/// The store carries **no ambient authority** — no handles, clocks, or RNG. Its
/// only field is the [`StoreLimits`] that wasmi consults (via [`Store::limiter`])
/// on every memory/table/instance allocation, so resource budgets are enforced
/// in-engine rather than relying on the guest's declared maxima.
#[derive(Debug)]
pub struct GuestState {
    /// The resource ceiling wasmi enforces for this guest. Not ambient authority:
    /// it can only *deny* allocations, never grant the guest anything.
    limits: StoreLimits,
}

impl Default for GuestState {
    fn default() -> Self {
        Self {
            limits: guest_store_limits(),
        }
    }
}

impl GuestState {
    /// Borrow the resource limiter wasmi consults for this store.
    fn limiter(&mut self) -> &mut dyn ResourceLimiter {
        &mut self.limits
    }
}

/// Construct a store with the no-authority [`GuestState`] and install its
/// [`ResourceLimiter`] so guest linear-memory bytes, table elements, memories,
/// tables and instances are all capped to the documented budgets.
///
/// The limiter is wired before the module is instantiated, so even a guest that
/// declares an over-budget minimum memory — or runs a `(start)` function that
/// grows memory — is bounded from the very first allocation.
#[must_use]
pub fn new_store(engine: &wasmi::Engine) -> Store<GuestState> {
    let mut store = Store::new(engine, GuestState::default());
    store.limiter(GuestState::limiter);
    store
}
