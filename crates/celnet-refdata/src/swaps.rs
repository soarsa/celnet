//! Canonical reference data for **swap / OIS curve points**.
//!
//! A swap curve point has no CUSIP and no ISIN — its identity is the curve it belongs
//! to plus the tenor along that curve. This module owns the one canonical spelling of
//! that identity so every side of the platform joins on the same string:
//!
//! - a liquidity provider streams its two-way under this `instrument_id`;
//! - an aggregated book scopes and consolidates on it;
//! - the venue derives it from an inbound request's FIX `Symbol(55)` + tenor to look
//!   the composite up.
//!
//! Keeping the definition here — rather than re-spelling `format!("{sym}-{n}Y")` at
//! each site — is what makes those three agree. A divergence between any two of them
//! does not fail loudly; it silently produces "no composite for this instrument", which
//! reads exactly like an unfed book.
//!
//! ## Why the tenor is part of the id
//!
//! An aggregated book keys on `instrument_id`. A bare curve symbol (`USD-OIS`) would
//! consolidate every point of the curve onto a single line, so a 10-year request would
//! be quoted off 2-year liquidity — a real mispricing, not a cosmetic one. The tenor is
//! therefore part of the identity, not a side-band attribute.

/// The whole-year tenors the platform treats as **on-the-run** points of a swap curve.
///
/// This is the set a liquidity panel is expected to quote and the venue is willing to
/// auto-quote against; a request for a tenor outside it has no book line behind it and
/// is declined rather than quoted off an interpolated or fabricated level.
pub const ON_THE_RUN_SWAP_TENORS: &[u32] = &[2, 3, 5, 7, 10];

/// The canonical `instrument_id` of a `(curve symbol, whole-year tenor)` swap curve
/// point: `USD-OIS` + `5` ⇒ `USD-OIS-5Y`.
///
/// The curve symbol is trimmed so a padded FIX `Symbol(55)` (a fixed-width field is
/// routinely space-padded) resolves to the same id an LP streamed under. No case fold
/// is applied — curve symbols are upper-case by convention and folding could conflate
/// distinct curves.
#[must_use]
pub fn swap_instrument_id(curve_symbol: &str, tenor_years: u32) -> String {
    format!("{}-{}Y", curve_symbol.trim(), tenor_years)
}

/// Whether `tenor_years` is an on-the-run point of a swap curve (see
/// [`ON_THE_RUN_SWAP_TENORS`]).
#[must_use]
pub fn is_on_the_run_swap_tenor(tenor_years: u32) -> bool {
    ON_THE_RUN_SWAP_TENORS.contains(&tenor_years)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_carries_the_tenor() {
        assert_eq!(swap_instrument_id("USD-OIS", 5), "USD-OIS-5Y");
        assert_eq!(swap_instrument_id("USD-OIS", 10), "USD-OIS-10Y");
    }

    /// A space-padded FIX symbol must resolve to the id the LP streamed under, or the
    /// composite lookup silently misses.
    #[test]
    fn a_padded_curve_symbol_resolves_to_the_same_id() {
        assert_eq!(
            swap_instrument_id("  USD-OIS ", 7),
            swap_instrument_id("USD-OIS", 7)
        );
    }

    /// Distinct tenors must never collide onto one book line.
    #[test]
    fn distinct_tenors_yield_distinct_ids() {
        let ids: std::collections::BTreeSet<String> = ON_THE_RUN_SWAP_TENORS
            .iter()
            .map(|&t| swap_instrument_id("USD-OIS", t))
            .collect();
        assert_eq!(ids.len(), ON_THE_RUN_SWAP_TENORS.len());
    }

    #[test]
    fn on_the_run_membership_matches_the_published_set() {
        assert!(is_on_the_run_swap_tenor(5));
        assert!(!is_on_the_run_swap_tenor(4));
        assert!(!is_on_the_run_swap_tenor(30));
    }
}
