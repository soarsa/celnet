//! Deterministic auxiliary math built on [`celnet_core::math`].
//!
//! The shared core math module exposes only `exp`, `ln`, `sqrt`, `norm_pdf` and
//! `norm_cdf` (the primitives the vanilla layer needs). The smile models in this
//! crate additionally need a real power. To keep the **single source of
//! determinism** (`docs/CONVENTIONS.md`: transcendentals route through
//! `rust-lang/libm` via `celnet_core::math`) it is composed from `exp`/`ln`
//! rather than pulling in a second transcendental implementation, so it stays
//! bit-identical across OS/arch exactly like the core primitives.

use celnet_core::math::{exp, ln};

/// `x` raised to the real power `y`, via `exp(y·ln x)` for `x > 0`.
///
/// Defined for strictly-positive `x` (the smile models raise positive
/// forwards/strikes to a CEV exponent, and exponentiate `e` for the
/// log-moneyness ↔ strike map). For `x == 0` returns `0` when `y > 0` and `+∞`
/// otherwise, matching the limiting behaviour of `xʸ`.
#[inline]
#[must_use]
pub(crate) fn powf(x: f64, y: f64) -> f64 {
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    exp(y * ln(x))
}
