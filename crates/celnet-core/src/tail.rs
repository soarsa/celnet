//! The one platform-wide VaR / Expected-Shortfall **tail reducer**.
//!
//! A single quantile reduction over a per-scenario P&L slice (a loss is a negative
//! P&L), shared by every risk engine so the whole platform has exactly ONE VaR/ES
//! convention: the FX/cross-asset options risk cube's non-additive path
//! (`celnet-risk-cube`) and the fixed-income rate-scenario engine
//! (`celnet-rates-risk`) both reduce their scenario P&L through this function.
//!
//! # Provenance (central-core Phase C2c)
//!
//! Extracted verbatim from the two deliberately convention-identical private
//! reducers that already existed — the cube's private `quantile_var_es` and
//! `celnet-rates-risk`'s `rate_var_es` — so there is no longer a second copy that can
//! silently drift. Both call sites now delegate here and map the result into their
//! own public `VarEs` / `RateVarEs` view type; the arithmetic below is byte-identical
//! to both prior bodies (same sort, same tail index, same sign and zero-floor), which
//! is what makes the fold `to_bits`-identical on the untouched options path and
//! ≤1e-12-faithful on the FI path.
//!
//! Zero-allocation: the reduction sorts the caller's slice in place and never
//! allocates, so it is admissible in `celnet-core` (which cannot allocate).

/// A Value-at-Risk / Expected-Shortfall pair — both non-negative loss magnitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TailVarEs {
    /// Value-at-Risk: the `alpha`-quantile loss (a positive number is a loss).
    pub var: f64,
    /// Expected Shortfall: the mean loss in the tail at or beyond the VaR quantile.
    pub es: f64,
}

/// The shared VaR/ES tail reduction over a P&L slice (a loss is a negative P&L).
///
/// Sorts ascending, takes the `alpha`-tail boundary loss as VaR and the mean of the
/// tail losses as ES; both are floored at zero. The tail count is `⌊(1 − alpha)·n⌋`,
/// floored at one and capped at `n`, so at least the single worst scenario always
/// contributes. An empty slice yields `{var: 0, es: 0}`.
///
/// Deterministic and bit-reproducible for a fixed `(pnl, alpha)` (a total sort with a
/// stable tie handling; no RNG).
#[must_use]
pub fn tail_var_es(pnl: &mut [f64], alpha: f64) -> TailVarEs {
    if pnl.is_empty() {
        return TailVarEs { var: 0.0, es: 0.0 };
    }
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = pnl.len();
    let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1).min(n);
    let tail_sum: f64 = pnl[..tail].iter().sum();
    let es = -(tail_sum / tail as f64);
    let var = -pnl[tail - 1];
    TailVarEs {
        var: var.max(0.0),
        es: es.max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pnl_is_zero() {
        assert_eq!(tail_var_es(&mut [], 0.99), TailVarEs { var: 0.0, es: 0.0 });
    }

    /// The canonical worked distribution both former reducers pinned: n = 8,
    /// alpha = 0.75 ⇒ tail = ⌊0.25·8⌋ = 2; the two worst are −100 and −80, so
    /// VaR = 80 (the tail boundary) and ES = mean(100, 80) = 90.
    #[test]
    fn var_es_on_the_known_distribution() {
        let mut pnl = vec![-100.0, 20.0, -30.0, 10.0, -80.0, 5.0, 40.0, -60.0];
        let got = tail_var_es(&mut pnl, 0.75);
        assert!((got.var - 80.0).abs() < 1e-12, "var {}", got.var);
        assert!((got.es - 90.0).abs() < 1e-12, "es {}", got.es);
    }

    /// alpha = 0.875 ⇒ tail = ⌊0.125·8⌋ = 1: VaR = ES = the single worst loss.
    #[test]
    fn var_es_single_tail_element() {
        let mut pnl = vec![-100.0, 20.0, -30.0, 10.0, -80.0, 5.0, 40.0, -60.0];
        let got = tail_var_es(&mut pnl, 0.875);
        assert!((got.var - 100.0).abs() < 1e-12);
        assert!((got.es - 100.0).abs() < 1e-12);
    }

    /// A profitable tail is not a loss — both measures floor at zero.
    #[test]
    fn all_gains_floor_to_zero() {
        let mut pnl = vec![10.0, 20.0, 5.0, 40.0];
        assert_eq!(tail_var_es(&mut pnl, 0.75), TailVarEs { var: 0.0, es: 0.0 });
    }
}
