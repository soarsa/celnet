//! Deterministic floating-point comparison.
//!
//! Mission-critical numerics must **never** compare floats with `==` and must
//! **never** assert on NaN bit patterns (NaN payloads are non-deterministic).
//! All comparisons go through [`is_close`] / [`assert_close!`], which combine an
//! absolute tolerance (for values near zero) with a relative tolerance (for
//! large values).

/// Returns `true` when `a` and `b` agree within either an absolute tolerance
/// `abs` or a relative tolerance `rel` (relative to the larger magnitude).
///
/// `NaN` never compares close (to anything, including itself); infinities are
/// close only when identical.
#[must_use]
#[allow(clippy::float_cmp)] // exact-equality fast path is intentional and correct here
pub fn is_close(a: f64, b: f64, rel: f64, abs: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return false;
    }
    if a == b {
        return true; // covers equal infinities and exact equality
    }
    if a.is_infinite() || b.is_infinite() {
        return false;
    }
    let diff = (a - b).abs();
    diff <= abs || diff <= rel * a.abs().max(b.abs())
}

/// Default relative tolerance for [`assert_close!`].
pub const DEFAULT_REL: f64 = 1e-9;
/// Default absolute tolerance for [`assert_close!`].
pub const DEFAULT_ABS: f64 = 1e-12;

/// Assert two floats are close (see [`is_close`]).
///
/// Forms: `assert_close!(a, b)` (default tolerances) or
/// `assert_close!(a, b, rel, abs)`.
#[macro_export]
macro_rules! assert_close {
    ($a:expr, $b:expr) => {
        $crate::assert_close!($a, $b, $crate::DEFAULT_REL, $crate::DEFAULT_ABS)
    };
    ($a:expr, $b:expr, $rel:expr, $abs:expr) => {{
        let a: f64 = $a;
        let b: f64 = $b;
        assert!(
            $crate::is_close(a, b, $rel, $abs),
            "assert_close failed: {a} vs {b} (rel={}, abs={}, |diff|={})",
            $rel,
            $abs,
            (a - b).abs()
        );
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closeness() {
        assert!(is_close(1.0, 1.0 + 1e-13, DEFAULT_REL, DEFAULT_ABS));
        assert!(!is_close(1.0, 1.0001, DEFAULT_REL, DEFAULT_ABS));
        assert!(is_close(0.0, 1e-15, DEFAULT_REL, DEFAULT_ABS));
        assert!(is_close(1e9, 1e9 + 1.0, DEFAULT_REL, DEFAULT_ABS));
    }

    #[test]
    fn nan_and_inf() {
        assert!(!is_close(f64::NAN, f64::NAN, 1.0, 1.0));
        assert!(is_close(f64::INFINITY, f64::INFINITY, 0.0, 0.0));
        assert!(!is_close(f64::INFINITY, 1e300, 1.0, 1.0));
    }

    #[test]
    fn macro_forms() {
        assert_close!(2.0_f64, 2.0);
        assert_close!(1.0_f64, 1.05, 0.1, 1e-12);
    }
}
