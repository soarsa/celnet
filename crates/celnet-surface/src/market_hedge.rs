//! Vanna-Volga smile — the FX-market-standard light interpolation.
//!
//! Provenance (doc-only): the construction is the Castagna-Mercurio (2007)
//! vanna-volga method, with the **second approximation** for the implied
//! volatility (the form that stays asymptotically constant at extreme strikes,
//! `docs/ANALYTICS-SPEC.md` §3.1). It needs **no numerical calibration**: from
//! three benchmark strikes (a put wing, the ATM, a call wing) and their
//! volatilities it constructs the volatility at any strike `K` so that the smile
//! reprices all three benchmarks exactly.
//!
//! # Idea
//!
//! For a target option at strike `K`, build the unique portfolio of the three
//! benchmark options that has the same Black-Scholes **vega, vanna and volga** as
//! the target at the ATM (flat-vol) volatility. The market over-/under-prices
//! that hedge relative to flat vol because the wings carry the quoted smile; the
//! difference is the vanna-volga *cost*, which, added to the flat-vol price and
//! re-implied, gives the smile volatility. The weights solve a 3×3 system whose
//! closed form reduces — via the log-moneyness ratios below — to two correction
//! terms `D1` (a first-order, RR-driven skew term) and `D2` (a second-order,
//! BF-driven convexity term).
//!
//! # Second-approximation implied vol
//!
//! With `σ₀ = σ_ATM`, `d₁(K)`, `d₂(K)` the Black-Scholes args at the ATM vol, and
//! the two correction terms `D1(K)`, `D2(K)` (built from the three pillar vols and
//! the log-moneyness weights `p`, `q` below),
//!
//! ```text
//!   σ(K) = σ₀ + ( −σ₀ + sqrt( σ₀² + d₁d₂·(2σ₀·D1 + D2) ) ) / (d₁d₂).
//! ```
//!
//! As `K → 0` or `K → ∞` the `d₁d₂` denominator dominates and `σ(K)` tends to a
//! finite constant, the property that makes the *second* approximation safe to
//! extrapolate (the first approximation diverges in the wings).
//!
//! This model implements [`celnet_core::Smile`] so the vanilla/exotics engines
//! consume it through the trait, never the concrete type. Vanna-Volga is **not**
//! globally arbitrage-free (high RR/BF, short tenors can produce negative density
//! in the wings); [`crate::arbitrage`] provides the checks the surface layer runs
//! to detect that and fall back to an arbitrage-free model.

use celnet_core::Smile;
use celnet_core::is_close;
use celnet_core::math::{ln, sqrt};
use celnet_types::Vol;

/// Three benchmark `(strike, vol)` pillars, ordered `K₁ < K₂ < K₃`, that anchor a
/// vanna-volga smile (typically the `dΔ` put wing, the ATM, and the `dΔ` call
/// wing). `K₂` is the ATM strike and `σ₂` the ATM volatility used as `σ₀`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketHedgeSmile {
    /// Benchmark strikes, strictly increasing `K₁ < K₂ < K₃`.
    strikes: [f64; 3],
    /// Benchmark volatilities at [`Self::strikes`].
    vols: [f64; 3],
    /// Reference forward the pillars were built against (sanity/extrapolation).
    forward: f64,
    /// Reference vol-time the pillars were built against.
    t: f64,
}

impl MarketHedgeSmile {
    /// Construct a vanna-volga smile from three ordered benchmark pillars.
    ///
    /// `strikes` must be strictly increasing and positive; `vols` strictly
    /// positive; `forward` and `t` positive. `strikes[1]`/`vols[1]` are the ATM
    /// pillar (`σ₀`).
    ///
    /// # Panics
    ///
    /// Panics if the strikes are not strictly increasing and positive, if any
    /// vol is non-positive, or if `forward`/`t` are non-positive — these are
    /// construction-time programming errors, not runtime market conditions.
    ///
    /// Use [`Self::try_new`] for any smile built from **calibration-derived** or
    /// **market-derived** volatilities, which can be degenerate (e.g. a
    /// non-positive wing vol for an extreme risk-reversal): a library smile must
    /// never panic on market input.
    #[must_use]
    pub fn new(strikes: [f64; 3], vols: [f64; 3], forward: f64, t: f64) -> Self {
        match Self::try_new(strikes, vols, forward, t) {
            Some(s) => s,
            None => panic!(
                "vanna-volga benchmark pillars must be well-posed (strictly-increasing positive \
                 strikes, strictly-positive vols, positive forward/t): \
                 strikes={strikes:?}, vols={vols:?}, F={forward}, t={t}"
            ),
        }
    }

    /// Fallible constructor: build a vanna-volga smile from three benchmark
    /// pillars, returning `None` instead of panicking when the inputs are not
    /// well-posed.
    ///
    /// Returns `None` if `strikes` are not strictly increasing and positive, if
    /// any `vol` is non-positive (`≤ 0` or non-finite), or if `forward`/`t` are
    /// non-positive. This is the constructor to use anywhere the pillar
    /// volatilities come from a **calibration** or directly from **market**
    /// data, where a degenerate quote can force a non-positive wing vol — such
    /// inputs are out-of-domain and must be rejected, never paniced on.
    #[must_use]
    pub fn try_new(strikes: [f64; 3], vols: [f64; 3], forward: f64, t: f64) -> Option<Self> {
        let strikes_ok = strikes.iter().all(|&k| k.is_finite())
            && strikes[0] > 0.0
            && strikes[0] < strikes[1]
            && strikes[1] < strikes[2];
        let vols_ok = vols.iter().all(|&v| v.is_finite() && v > 0.0);
        let scale_ok = forward.is_finite() && t.is_finite() && forward > 0.0 && t > 0.0;
        if strikes_ok && vols_ok && scale_ok {
            Some(Self {
                strikes,
                vols,
                forward,
                t,
            })
        } else {
            None
        }
    }

    /// The ATM (central) volatility `σ₀`.
    #[must_use]
    pub fn atm_vol(&self) -> f64 {
        self.vols[1]
    }

    /// The three benchmark strikes `[K₁, K₂, K₃]`.
    #[must_use]
    pub fn benchmark_strikes(&self) -> [f64; 3] {
        self.strikes
    }

    /// The three benchmark volatilities `[σ₁, σ₂, σ₃]`.
    #[must_use]
    pub fn benchmark_vols(&self) -> [f64; 3] {
        self.vols
    }

    /// The reference forward.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.forward
    }

    /// The reference vol-time.
    #[must_use]
    pub fn reference_t(&self) -> f64 {
        self.t
    }

    /// `d₁` and `d₂` of strike `K` at vol `vol`, forward `forward`, time `t`.
    ///
    /// Forward-based Black args: `d₁ = [ln(F/K) + ½σ²t]/(σ√t)`, `d₂ = d₁ − σ√t`.
    #[inline]
    fn d12(forward: f64, strike: f64, vol: f64, t: f64) -> (f64, f64) {
        let vsqt = vol * sqrt(t);
        let d1 = (ln(forward / strike) + 0.5 * vol * vol * t) / vsqt;
        (d1, d1 - vsqt)
    }

    /// The vanna-volga log-moneyness interpolation weights for strike `K`.
    ///
    /// `p` weights the put-wing pillar `K₁`, `q` the call-wing pillar `K₃`
    /// (the ATM pillar `K₂` carries weight `1 − p − q`). They are the products of
    /// log-moneyness ratios that arise from solving the 3×3 vega-matching system
    /// in the Castagna-Mercurio closed form:
    ///
    /// ```text
    ///   p(K) = ln(K₂/K)·ln(K₃/K) / [ln(K₂/K₁)·ln(K₃/K₁)],
    ///   q(K) = ln(K/K₁)·ln(K/K₂) / [ln(K₃/K₁)·ln(K₃/K₂)].
    /// ```
    #[inline]
    fn weights(&self, strike: f64) -> (f64, f64) {
        let [k1, k2, k3] = self.strikes;
        let p = (ln(k2 / strike) * ln(k3 / strike)) / (ln(k2 / k1) * ln(k3 / k1));
        let q = (ln(strike / k1) * ln(strike / k2)) / (ln(k3 / k1) * ln(k3 / k2));
        (p, q)
    }

    /// The two correction terms `D1(K)` (first-order / skew) and `D2(K)`
    /// (second-order / convexity) of the second-approximation implied-vol
    /// formula, evaluated at the ATM vol `σ₀`.
    ///
    /// `D1` is the smile interpolation of the *vol surplus* over `σ₀` at the
    /// three pillars; `D2` is the second-order vega-weighted convexity term in
    /// `d₁d₂`. Together they feed the closed-form `σ(K)` below.
    #[inline]
    fn corrections(&self, strike: f64) -> (f64, f64) {
        let s0 = self.vols[1];
        let (p, q) = self.weights(strike);

        // First-order term: D1 = p·σ₁ + (1−p−q)·σ₀ + q·σ₃ − σ₀
        //                       = p·(σ₁−σ₀) + q·(σ₃−σ₀).
        let d1_term = p * (self.vols[0] - s0) + q * (self.vols[2] - s0);

        // Second-order term: D2 = p·d₁(K₁)d₂(K₁)·(σ₁−σ₀)²
        //                        + q·d₁(K₃)d₂(K₃)·(σ₃−σ₀)².
        let (a1, a2) = Self::d12(self.forward, self.strikes[0], s0, self.t);
        let (c1, c2) = Self::d12(self.forward, self.strikes[2], s0, self.t);
        let dv1 = self.vols[0] - s0;
        let dv3 = self.vols[2] - s0;
        let d2_term = p * a1 * a2 * dv1 * dv1 + q * c1 * c2 * dv3 * dv3;

        (d1_term, d2_term)
    }

    /// The smile volatility at strike `strike` via the Castagna-Mercurio second
    /// approximation. This is the [`Smile`] evaluation specialised to this
    /// model's reference forward/time; the trait method delegates here.
    ///
    /// At a benchmark strike it returns that benchmark's vol to machine
    /// precision (exact repricing); between and beyond the wings it interpolates
    /// /extrapolates with the asymptotically-constant second-approximation form.
    #[must_use]
    pub fn vol_at(&self, strike: f64) -> f64 {
        let s0 = self.vols[1];
        let (d1, d2) = Self::d12(self.forward, strike, s0, self.t);
        let prod = d1 * d2;

        let (big_d1, big_d2) = self.corrections(strike);

        // Degenerate guard: at the ATM strike d₁d₂ → −¼σ²t·(…)/… can pass through
        // small magnitudes; when |d₁d₂| underflows the formula reduces to the
        // first-order interpolation σ₀ + D1 (the limit of the second approx).
        if prod.abs() < 1e-14 {
            return s0 + big_d1;
        }

        let radicand = s0 * s0 + prod * (2.0 * s0 * big_d1 + big_d2);
        // The radicand is non-negative for well-posed quotes; clamp at zero to
        // stay real if pathological wings push it slightly negative (the
        // arbitrage checks in `crate::arbitrage` are the place that *rejects*
        // such a smile — here we must still return a finite number).
        let root = sqrt(radicand.max(0.0));
        s0 + (-s0 + root) / prod
    }
}

impl Smile for MarketHedgeSmile {
    /// Implied vol at `strike`. The `forward`/`t` arguments let the engine
    /// re-evaluate the same smile at a (close) re-derived forward/time; the
    /// vanna-volga corrections are recomputed against the supplied `forward`/`t`
    /// while the benchmark pillars stay fixed, so a consumer holding only the
    /// trait sees a self-consistent smile.
    fn implied_vol(&self, strike: f64, forward: f64, t: f64) -> Vol {
        // Re-evaluate against the caller's forward/time if they differ from the
        // reference (the engine may pass a freshly-derived forward). Build a
        // transient view with the requested forward/time; the benchmark vols and
        // strikes are sticky-delta anchors and do not move.
        // Exact-identity fast path: route the equality through `is_close` with
        // zero tolerances (the platform's sanctioned way to test exact float
        // equality, NaN-safe) rather than a literal `==`.
        if is_close(forward, self.forward, 0.0, 0.0) && is_close(t, self.t, 0.0, 0.0) {
            Vol(self.vol_at(strike))
        } else {
            let view = MarketHedgeSmile {
                strikes: self.strikes,
                vols: self.vols,
                forward,
                t,
            };
            Vol(view.vol_at(strike))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    /// A representative skewed smile: put wing 12.5, ATM 11, call wing 11.8.
    fn smile() -> MarketHedgeSmile {
        // EURUSD-like 1Y: F ≈ 1.11, strikes spread around it.
        MarketHedgeSmile::new([1.02, 1.11, 1.20], [0.125, 0.11, 0.118], 1.11, 1.0)
    }

    /// The smile reprices each benchmark exactly (the defining VV property).
    #[test]
    fn reprices_benchmarks_exactly() {
        let s = smile();
        for i in 0..3 {
            let v = s.vol_at(s.strikes[i]);
            assert!(
                is_close(v, s.vols[i], 1e-10, 1e-12),
                "benchmark {i}: got {v}, want {}",
                s.vols[i]
            );
        }
    }

    /// Through the trait, the ATM strike returns the ATM vol and matches `vol_at`.
    #[test]
    fn trait_matches_inherent() {
        let s = smile();
        let k = 1.07;
        let via_trait = s.implied_vol(k, s.forward(), s.reference_t()).0;
        assert!(is_close(via_trait, s.vol_at(k), 1e-14, 1e-15));
        let atm = s.atm_forward_vol(s.forward(), s.reference_t()).0;
        assert!(is_close(atm, s.atm_vol(), 1e-10, 1e-12));
    }

    /// The smile is finite and positive across a wide strike grid, and stays
    /// bounded (asymptotically constant) far in the wings — the second-approx
    /// property that the first approximation lacks.
    #[test]
    fn finite_and_bounded_in_wings() {
        let s = smile();
        let deep_low = s.vol_at(0.30);
        let deep_high = s.vol_at(4.0);
        assert!(deep_low.is_finite() && deep_low > 0.0);
        assert!(deep_high.is_finite() && deep_high > 0.0);
        // Far wings should not blow up beyond a sane multiple of ATM.
        assert!(deep_low < 1.0 && deep_high < 1.0);
    }

    /// A skewed smile has σ(put wing) > σ(call wing) here (negative skew),
    /// monotone ordering between the pillars in the interior.
    #[test]
    fn interior_interpolation_is_smooth() {
        let s = smile();
        // Between K1 and K2 the vol lies between the two pillar vols (no spikes).
        let mid = s.vol_at(0.5 * (s.strikes[0] + s.strikes[1]));
        let (lo, hi) = (s.vols[1].min(s.vols[0]), s.vols[1].max(s.vols[0]));
        assert!(
            mid >= lo - 1e-3 && mid <= hi + 1e-3,
            "interior vol {mid} should sit near [{lo},{hi}]"
        );
    }
}
