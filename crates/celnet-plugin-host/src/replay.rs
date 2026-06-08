//! Deterministic replay harness — the bit-identity proof for Tier-2 models.
//!
//! Determinism (R1 of `docs/PLUGIN-HOST-ALT.md`) is a *contract*, not a hope:
//! identical inputs through a sandboxed model must produce **bit-identical**
//! output, forever and on every platform. This harness replays a fixed market
//! snapshot through a [`HostModel`] `runs` times and asserts every output `f64`
//! is `to_bits`-equal across runs — the same model evaluated twice agrees to the
//! last bit, including any NaN (canonicalized at the boundary).
//!
//! # Cross-platform requirement
//!
//! Bit-identity *across machines* (our aarch64-apple-darwin dev box and the
//! Linux CI runner) additionally requires that all transcendentals are the
//! correctly-rounded, platform-independent `rust-lang/libm` ones — which the
//! contract mandates and the [`crate::host`] capability surface enforces by
//! routing the guest's `exp`/`ln`/`sqrt`/`norm_*` through `celnet_core::math`. A
//! guest that imports those (rather than relying on a host FPU intrinsic) is
//! reproducible on any target wasmi runs on. The replay invariant also folds in
//! fuel accounting: the same fuel budget interrupts at the same instruction, so a
//! near-budget model replays identically too.

use celnet_core::{CarryGreeks, CarryInputs};
use celnet_types::{OptionType, RateSensitivities};

use crate::error::HostError;
use crate::model::HostModel;

/// A fixed market snapshot to replay: an option type and its [`CarryInputs`].
///
/// Deliberately `Copy` and self-contained so a snapshot can be checked into a
/// golden file and replayed unchanged across releases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// The option type to price.
    pub opt: OptionType,
    /// The market inputs.
    pub inputs: CarryInputs,
}

impl Snapshot {
    /// Construct a replay snapshot.
    #[must_use]
    pub const fn new(opt: OptionType, inputs: CarryInputs) -> Self {
        Self { opt, inputs }
    }
}

/// Whether two `f64`s are bit-identical (`to_bits`-equal).
///
/// Used instead of `==` so two canonicalized NaNs compare *equal* (they share the
/// canonical bit pattern) and `-0.0`/`+0.0` compare *unequal* — exactly the
/// reproducibility notion replay must enforce. Never use `==`/`!=` here.
#[must_use]
pub fn bits_eq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

/// The asset-class tag (`0 = Fx`, `1 = Carry`) and the two rate scalars of a
/// [`RateSensitivities`], so the rate block can be compared bit-for-bit *and* by
/// its discriminant (a same-value-but-different-arm result must not compare
/// equal).
const fn rate_parts(r: RateSensitivities) -> (u8, f64, f64) {
    match r {
        RateSensitivities::Fx { rho_dom, rho_for } => (0, rho_dom, rho_for),
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => (1, discount_rho, carry_rho),
    }
}

/// Whether two [`CarryGreeks`] are bit-identical field-for-field, including the
/// carry-tagged rate block (both its discriminant and its two scalars).
#[must_use]
pub fn greeks_bits_eq(a: &CarryGreeks, b: &CarryGreeks) -> bool {
    let neutral = |g: &CarryGreeks| {
        [
            g.price,
            g.delta_spot,
            g.delta_forward,
            g.gamma,
            g.vega,
            g.theta,
            g.vanna,
            g.volga,
            g.charm,
            g.speed,
            g.zomma,
            g.color,
        ]
    };
    let (fa, fb) = (neutral(a), neutral(b));
    let neutral_eq = fa.iter().zip(fb.iter()).all(|(x, y)| bits_eq(*x, *y));
    let (ta, r0a, r1a) = rate_parts(a.rates);
    let (tb, r0b, r1b) = rate_parts(b.rates);
    neutral_eq && ta == tb && bits_eq(r0a, r0b) && bits_eq(r1a, r1b)
}

/// The outcome of a replay: the canonical first-run output plus how many runs
/// agreed bit-for-bit.
#[derive(Debug, Clone, Copy)]
pub struct ReplayOutcome {
    /// The price from the first run; every subsequent run matched it `to_bits`.
    pub price: f64,
    /// The Greeks from the first run; every subsequent run matched them `to_bits`.
    pub greeks: CarryGreeks,
    /// Number of replay runs performed (all bit-identical to the first).
    pub runs: usize,
}

/// Replay `snapshot` through `model` `runs` times, asserting bit-identity, and
/// return the (canonical) output.
///
/// Returns `Err(ReplayError::Diverged)` if any run's price or Greeks differ at
/// the bit level from the first run, or propagates any [`HostError`] the model
/// raises. `runs` is clamped to at least 2 (a single run cannot prove identity).
///
/// # Errors
/// - [`ReplayError::Host`] if the model errors on any run.
/// - [`ReplayError::Diverged`] if outputs are not bit-identical across runs.
pub fn replay(
    model: &dyn HostModel,
    snapshot: Snapshot,
    runs: usize,
) -> Result<ReplayOutcome, ReplayError> {
    let runs = runs.max(2);
    let first_price = model
        .price(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;
    let first_greeks = model
        .price_and_greeks(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;

    for _ in 1..runs {
        let p = model
            .price(snapshot.opt, &snapshot.inputs)
            .map_err(ReplayError::Host)?;
        if !bits_eq(p, first_price) {
            return Err(ReplayError::Diverged("price"));
        }
        let g = model
            .price_and_greeks(snapshot.opt, &snapshot.inputs)
            .map_err(ReplayError::Host)?;
        if !greeks_bits_eq(&g, &first_greeks) {
            return Err(ReplayError::Diverged("greeks"));
        }
    }

    Ok(ReplayOutcome {
        price: first_price,
        greeks: first_greeks,
        runs,
    })
}

/// Assert two independently-loaded models agree on a snapshot to the bit.
///
/// This is the cross-instance / cross-tier identity check: load the same model
/// twice (or a native and a Wasm twin) and prove they price a snapshot
/// `to_bits`-identically.
///
/// Bit-identity between a Tier-0 and a Tier-2 model is **not** a general property
/// of any two pricers of the same payoff — it holds only when both perform the
/// *same* sequence of IEEE-754 operations through the *same* `rust-lang/libm`.
/// When that holds (as for the deliberately op-order-matched twin in
/// `tests/sandbox.rs`), this check is exact; for an arbitrary pair, agreement is
/// expected within the models' documented numerical tolerance instead, and a
/// tolerance-based comparator — not this bit-exact one — is the right gate.
///
/// # Errors
/// - [`ReplayError::Host`] if either model errors.
/// - [`ReplayError::Diverged`] if their outputs are not bit-identical.
pub fn assert_agree(
    a: &dyn HostModel,
    b: &dyn HostModel,
    snapshot: Snapshot,
) -> Result<(), ReplayError> {
    let pa = a
        .price(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;
    let pb = b
        .price(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;
    if !bits_eq(pa, pb) {
        return Err(ReplayError::Diverged("price"));
    }
    let ga = a
        .price_and_greeks(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;
    let gb = b
        .price_and_greeks(snapshot.opt, &snapshot.inputs)
        .map_err(ReplayError::Host)?;
    if !greeks_bits_eq(&ga, &gb) {
        return Err(ReplayError::Diverged("greeks"));
    }
    Ok(())
}

/// Why a replay or agreement check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplayError {
    /// The model raised a host/sandbox error during replay.
    Host(HostError),
    /// Outputs were not bit-identical; the payload names the divergent field
    /// group (`"price"` or `"greeks"`).
    Diverged(&'static str),
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReplayError::Host(e) => write!(f, "replay host error: {e}"),
            ReplayError::Diverged(what) => {
                write!(f, "replay diverged at {what} (not bit-identical)")
            }
        }
    }
}

impl core::error::Error for ReplayError {}
