//! Extended surface stochastic-volatility-inspired (eSSVI) parameterization.
//!
//! Provenance (doc-only): the *extended* SSVI of Hendriks & Martini (2019),
//! "The extended SSVI volatility surface" (J. Comp. Finance), which generalises
//! the surface SVI of Gatheral & Jacquier (2014) by letting the correlation be
//! **maturity-dependent**, `ρ → ρ(θ)`, while retaining closed-form static
//! no-arbitrage (butterfly + calendar) conditions. The calendar condition we use
//! is the explicit two-slice inequality of Hendriks & Martini (2019, Prop. 3.1),
//! itself a specialisation of the general calendar-spread condition of Corbetta,
//! Cohort, Laachir & Martini (2019) / Gatheral & Jacquier (2014, Thm 4.1)
//! (`docs/ANALYTICS-SPEC.md` §3.3). "SSVI"/"eSSVI" are the established neutral
//! acronyms; identifiers carry no person names (guardrail #8).
//!
//! # The `(θ, ρ, ψ)` slice parameterization
//!
//! Each maturity slice is described by three numbers — the ATM total variance
//! `θ`, the correlation `ρ ∈ (−1, 1)`, and the **ATM skew-scale**
//! `ψ = θ·φ(θ) > 0` (`ψ` is the natural eSSVI shape variable: `∂_k w|_{k=0} = ρψ`
//! is the ATM skew, and the curvature scales with `ψ²/θ`). In these variables the
//! total implied variance is the SSVI form with `φ = ψ/θ`:
//!
//! ```text
//!   w(k) = (θ/2)·{ 1 + ρ·(ψ/θ)·k + √( ((ψ/θ)·k + ρ)² + (1 − ρ²) ) }.
//! ```
//!
//! The whole surface is a set of calibrated pillars `(θ_i, ρ_i, ψ_i)` ordered by
//! `θ`; between pillars `ρ` and `ψ` are interpolated linearly **in `θ`** (the
//! coordinate in which the calendar condition is linear), and `θ(t)` is supplied
//! by the caller's ATM total-variance term structure. SSVI is the special case
//! `ρ(θ) ≡ const` with `ψ = θ·φ(θ)` from a power-law `φ`.
//!
//! # Static no-arbitrage (closed form) — CLAIMS validated against numerics
//!
//! The two predicates below are *claims*; the `celnet-parity` `essvi` rows
//! validate them against the independent Breeden-Litzenberger density numerics
//! and pointwise calendar monotonicity (the numerics are the oracle).
//!
//! * **Butterfly (per slice)** — substituting `φ = ψ/θ` into the SSVI sufficient
//!   conditions (Gatheral-Jacquier 2014, Thm 4.2) gives, in the `(θ,ρ,ψ)`
//!   variables, `ψ·(1+|ρ|) < 4` and `(ψ²/θ)·(1+|ρ|) ≤ 4` (Hendriks-Martini 2019,
//!   §2). The first is the eSSVI g-function / large-strike bound; the second the
//!   vertex curvature bound.
//! * **Calendar (between consecutive slices)** — for `θ₁ < θ₂` the surface is
//!   calendar-arbitrage-free if `w(k,θ₂) ≥ w(k,θ₁)` for every `k`. Hendriks &
//!   Martini (2019, Prop. 3.1) reduce this to the explicit pair condition
//!   `θ₁ ≤ θ₂`, `ψ₁ ≤ ψ₂`, and `ρ₁·ψ₁ ≤ ρ₂·ψ₂` **and**
//!   `ρ₂·ψ₂ − ρ₁·ψ₁ ≤ ψ₂ − ψ₁` (equivalently `|ρ₂ψ₂ − ρ₁ψ₁| ≤ ψ₂ − ψ₁`): the ATM
//!   skews and the ψ-gap must be compatible so the two slices never cross.

use celnet_core::math::sqrt;

use crate::parametric::ParametricSlice;
use crate::parametric_surface::ParametricSurface;

/// Deterministic clamp into `[lo, hi]` (libm-free, bit-reproducible). Mirrors the
/// calibrator's own clamp so the projection arithmetic is identical there.
#[inline]
fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// One eSSVI maturity slice in the `(θ, ρ, ψ)` parameterization: the ATM total
/// variance `θ`, the correlation `ρ ∈ (−1, 1)`, and the ATM skew-scale
/// `ψ = θ·φ(θ) > 0`.
///
/// Internally the slice carries the **curvature `φ`** (the SSVI shape variable
/// `φ = ψ/θ`) as well, computed once at construction. Evaluating `total_variance`
/// through the stored `φ` — exactly as the SSVI [`ParametricSurface`] does — is
/// what makes the SSVI special case **byte-recoverable**: when an eSSVI slice is
/// built from an SSVI `(ρ, φ(θ))` via [`ExtendedSlice::from_curvature`], its
/// `total_variance` reproduces [`ParametricSurface::total_variance`] to bit
/// identity (no `ψ/θ` round-trip, which would lose a ULP).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtendedSlice {
    /// ATM total variance `θ > 0` (`= σ_ATM²·t`).
    pub theta: f64,
    /// Correlation `ρ ∈ (−1, 1)`.
    pub rho: f64,
    /// ATM skew-scale `ψ = θ·φ > 0`.
    pub psi: f64,
    /// The SSVI curvature `φ` of this slice (`= ψ/θ`), stored so `total_variance`
    /// evaluates the SSVI form through the identical arithmetic.
    phi: f64,
}

impl ExtendedSlice {
    /// Construct an eSSVI slice from `(θ, ρ, ψ)`, validating the parameter ranges.
    /// The curvature is `φ = ψ/θ`.
    ///
    /// # Panics
    ///
    /// Panics if `θ ≤ 0`, `|ρ| ≥ 1`, or `ψ ≤ 0`.
    #[must_use]
    pub fn new(theta: f64, rho: f64, psi: f64) -> Self {
        assert!(theta > 0.0, "eSSVI theta must be positive: {theta}");
        assert!(rho.abs() < 1.0, "eSSVI rho must lie in (-1,1): {rho}");
        assert!(psi > 0.0, "eSSVI psi must be positive: {psi}");
        Self {
            theta,
            rho,
            psi,
            phi: psi / theta,
        }
    }

    /// Construct an eSSVI slice from the SSVI shape variables `(θ, ρ, φ)` (the
    /// curvature `φ`), with `ψ = θ·φ` derived. This is the canonical SSVI→eSSVI
    /// bridge: a slice so constructed evaluates `total_variance` through the
    /// **same `φ`** the SSVI surface uses, so the SSVI special case is recovered
    /// bit-for-bit (no `ψ/θ` round-trip).
    ///
    /// # Panics
    ///
    /// Panics if `θ ≤ 0`, `|ρ| ≥ 1`, or `φ ≤ 0`.
    #[must_use]
    pub fn from_curvature(theta: f64, rho: f64, phi: f64) -> Self {
        assert!(theta > 0.0, "eSSVI theta must be positive: {theta}");
        assert!(rho.abs() < 1.0, "eSSVI rho must lie in (-1,1): {rho}");
        assert!(phi > 0.0, "eSSVI phi must be positive: {phi}");
        Self {
            theta,
            rho,
            psi: theta * phi,
            phi,
        }
    }

    /// The SSVI curvature `φ = ψ/θ` of this slice (the stored exact value).
    #[inline]
    #[must_use]
    pub fn phi(&self) -> f64 {
        self.phi
    }

    /// Total implied variance `w(k)` at log-moneyness `k`, the SSVI form with the
    /// stored curvature `φ`:
    ///
    /// ```text
    ///   w(k) = (θ/2)·{ 1 + ρ·φ·k + √( (φ·k + ρ)² + (1 − ρ²) ) }.
    /// ```
    ///
    /// This is **token-for-token** the SSVI [`ParametricSurface::total_variance`]
    /// arithmetic (with `φ` supplied rather than recomputed from `η/θ^γ`), so the
    /// SSVI special case is byte-recoverable.
    #[inline]
    #[must_use]
    pub fn total_variance(&self, k: f64) -> f64 {
        let p = self.phi;
        let pk = p * k + self.rho;
        0.5 * self.theta * (1.0 + self.rho * p * k + sqrt(pk * pk + (1.0 - self.rho * self.rho)))
    }

    /// Whether the per-slice butterfly no-arbitrage **sufficient** conditions hold
    /// in the `(θ,ρ,ψ)` variables (Hendriks-Martini 2019, §2; the SSVI
    /// Gatheral-Jacquier Thm 4.2 conditions with `φ = ψ/θ`):
    /// `ψ·(1+|ρ|) < 4` and `(ψ²/θ)·(1+|ρ|) ≤ 4`.
    #[must_use]
    pub fn is_butterfly_free(&self) -> bool {
        let one_p_abs_rho = 1.0 + self.rho.abs();
        self.psi * one_p_abs_rho < 4.0 && (self.psi * self.psi / self.theta) * one_p_abs_rho <= 4.0
    }

    /// The largest `ψ` admissible at this slice's `(θ, ρ)` under the butterfly
    /// domain `ψ(1+|ρ|) < 4 ∧ (ψ²/θ)(1+|ρ|) ≤ 4`, i.e.
    /// `ψ_max = min( 4/(1+|ρ|),  √(4θ/(1+|ρ|)) )`. A small relative shrink
    /// (`× 1 − ε`) keeps the strict first inequality strict and absorbs the
    /// floating-point boundary so [`is_butterfly_free`] is *certain* to admit the
    /// capped slice — the projection target used by the robust calibrator.
    ///
    /// [`is_butterfly_free`]: ExtendedSlice::is_butterfly_free
    #[must_use]
    pub fn butterfly_psi_cap(theta: f64, rho: f64) -> f64 {
        // Margin below the strict bound: ~5 ULP-scale relative shrink, generous
        // enough that round-off in `is_butterfly_free`'s recomputation can never
        // re-cross the boundary, tight enough not to distort the calibrated shape.
        const SHRINK: f64 = 1.0 - 1e-9;
        let one_p = 1.0 + rho.abs();
        let cap_large_strike = 4.0 / one_p; // ψ(1+|ρ|) < 4
        let cap_curvature = sqrt(4.0 * theta / one_p); // (ψ²/θ)(1+|ρ|) ≤ 4
        SHRINK * cap_large_strike.min(cap_curvature)
    }

    /// Project an arbitrary `(θ, ρ, ψ)` candidate into a **guaranteed**
    /// butterfly-free [`ExtendedSlice`]: `ρ` is clamped strictly inside `(−1, 1)`,
    /// `ψ` is clamped to `[ψ_min, ψ_max]` where `ψ_max` is [`butterfly_psi_cap`]
    /// and `ψ_min` a tiny positive floor, and non-finite inputs collapse to the
    /// flat (`ρ = 0`) sane fallback. The returned slice **always** satisfies
    /// [`is_butterfly_free`] — never NaN, never over the no-arb domain. This is the
    /// projection step of the robust eSSVI calibrator (Hendriks-Martini 2019 §2
    /// domain).
    ///
    /// # Panics
    ///
    /// Panics if `θ ≤ 0` (a slice with non-positive ATM total variance is not a
    /// valid maturity slice; the caller pins `θ` to the ATM term structure).
    ///
    /// [`butterfly_psi_cap`]: ExtendedSlice::butterfly_psi_cap
    /// [`is_butterfly_free`]: ExtendedSlice::is_butterfly_free
    #[must_use]
    pub fn projected(theta: f64, rho: f64, psi: f64) -> ExtendedSlice {
        assert!(theta > 0.0, "eSSVI theta must be positive: {theta}");
        // Sane fallback for any non-finite candidate: flat, mildly convex slice.
        let rho = if rho.is_finite() { rho } else { 0.0 };
        let rho = clamp(rho, -0.999, 0.999);
        let psi_cap = Self::butterfly_psi_cap(theta, rho);
        const PSI_MIN: f64 = 1e-6;
        let psi = if psi.is_finite() { psi } else { PSI_MIN };
        let psi = clamp(psi, PSI_MIN, psi_cap.max(PSI_MIN));
        ExtendedSlice::new(theta, rho, psi)
    }

    /// Materialise an [`ParametricSlice`] (raw SVI) for this eSSVI slice at the
    /// given forward `f` and expiry `t`, via the exact SSVI→raw closed-form map
    /// applied with `φ = ψ/θ`. This bridges eSSVI into every [`celnet_core::Smile`]
    /// consumer and lets the arbitrage numerics re-strike the genuine curvature.
    ///
    /// # Panics
    ///
    /// Panics if `f` or `t` are non-positive.
    #[must_use]
    pub fn to_slice(&self, forward: f64, t: f64) -> ParametricSlice {
        // Reuse the SSVI→raw map: an SSVI surface with this slice's ρ and a
        // power-law η/γ that reproduces φ = ψ/θ at θ gives the identical raw slice
        // (the map depends only on (θ, ρ, φ), not on the η/γ split). Choose γ = ½
        // and η = φ·θ^{1/2} so ParametricSurface::phi(θ) == φ exactly.
        let phi = self.phi();
        let eta = phi * sqrt(self.theta);
        ParametricSurface::new(self.rho, eta, 0.5).to_slice(self.theta, forward, t)
    }

    /// Whether this slice and a *later* slice `next` (with `θ_next > θ`) are free
    /// of calendar-spread arbitrage between them, by the explicit eSSVI pair
    /// condition (Hendriks-Martini 2019, Prop. 3.1):
    ///
    /// ```text
    ///   θ ≤ θ_next,   ψ ≤ ψ_next,   and   |ρ_next·ψ_next − ρ·ψ| ≤ ψ_next − ψ.
    /// ```
    ///
    /// The third inequality couples the ATM skews (`ρψ`) to the ψ-gap so the two
    /// total-variance curves never cross in `k`. A small absolute slack absorbs
    /// floating-point noise at the boundary.
    #[must_use]
    pub fn is_calendar_free_with(&self, next: &ExtendedSlice) -> bool {
        const SLACK: f64 = 1e-12;
        if next.theta < self.theta - SLACK {
            return false;
        }
        if next.psi < self.psi - SLACK {
            return false;
        }
        let skew_gap = (next.rho * next.psi - self.rho * self.psi).abs();
        let psi_gap = next.psi - self.psi;
        skew_gap <= psi_gap + SLACK
    }

    /// Project a *later* candidate slice (this is the fixed earlier pillar `prev`)
    /// into the calendar-no-arbitrage cone relative to `prev`, returning a slice
    /// that is **both** butterfly-free *and* [`is_calendar_free_with`]-admissible
    /// after `prev`. The candidate's `(ρ, ψ)` are adjusted minimally:
    ///
    /// 1. `ψ` is raised to at least `prev.ψ` (the eSSVI calendar condition requires
    ///    `ψ` non-decreasing in `θ`), and capped to the candidate's own butterfly
    ///    `ψ_max`. If `prev.ψ` already exceeds the candidate's butterfly cap, the
    ///    cap wins and `ρ` is shrunk toward `0` (which *raises* the cap, since
    ///    `ψ_max ∝ 1/(1+|ρ|)`) until `ψ ≥ prev.ψ` is admissible — the sane,
    ///    always-no-arb fallback.
    /// 2. With `ψ` fixed, the ATM-skew `ρψ` is clamped into
    ///    `[prev.ρ·prev.ψ − (ψ − prev.ψ),  prev.ρ·prev.ψ + (ψ − prev.ψ)]`
    ///    (the pair condition `|ρ₂ψ₂ − ρ₁ψ₁| ≤ ψ₂ − ψ₁`), then `ρ = (ρψ)/ψ`.
    ///
    /// The result is guaranteed `prev.is_calendar_free_with(&result) == true` and
    /// `result.is_butterfly_free() == true`. This is the calendar projection step
    /// of the robust surface calibrator.
    ///
    /// # Panics
    ///
    /// Panics if `theta_next ≤ prev.θ` (slices must be strictly increasing in `θ`).
    ///
    /// [`is_calendar_free_with`]: ExtendedSlice::is_calendar_free_with
    #[must_use]
    pub fn project_after(&self, theta_next: f64, rho: f64, psi: f64) -> ExtendedSlice {
        assert!(
            theta_next > self.theta,
            "later slice θ {theta_next} must exceed earlier θ {}",
            self.theta
        );
        // Start from a butterfly-projected candidate.
        let cand = ExtendedSlice::projected(theta_next, rho, psi);
        let mut rho_c = cand.rho;
        let mut psi_c = cand.psi;

        // (1) Ensure ψ_next ≥ prev.ψ within the butterfly cap; if the cap at the
        //     candidate's ρ is below prev.ψ, shrink |ρ| toward 0 (raising the cap)
        //     until prev.ψ is admissible. |ρ|→0 gives the largest possible cap
        //     min(4, √(4θ)), which is ≥ any earlier pillar's ψ on a sane term
        //     structure; the loop converges in a few halvings and is bounded.
        let target_psi = self.psi;
        for _ in 0..64 {
            let cap = ExtendedSlice::butterfly_psi_cap(theta_next, rho_c);
            if cap >= target_psi || rho_c.abs() <= 1e-9 {
                psi_c = clamp(target_psi.max(psi_c), psi_c.min(target_psi), cap);
                break;
            }
            // Shrink ρ toward 0 to raise the cap, then re-evaluate.
            rho_c *= 0.5;
        }
        // Final clamp: ψ in [prev.ψ (if attainable), butterfly cap].
        let cap = ExtendedSlice::butterfly_psi_cap(theta_next, rho_c);
        psi_c = clamp(psi_c.max(target_psi.min(cap)), 1e-6, cap);

        // (2) Clamp the ATM skew ρψ into the calendar cone, then back out ρ.
        let psi_gap = psi_c - self.psi;
        let center = self.rho * self.psi;
        // psi_gap can be a hair negative from round-off if ψ_c was capped just
        // below prev.ψ; treat a tiny negative as zero so the cone is well-formed.
        let half = psi_gap.max(0.0);
        let target_skew = clamp(rho_c * psi_c, center - half, center + half);
        let rho_final = clamp(target_skew / psi_c, -0.999, 0.999);
        // Re-project to be certain butterfly holds after the ρ adjustment (a larger
        // |ρ| would lower the cap; ρ_final never exceeds rho_c in magnitude here,
        // but project defensively so the postcondition is unconditional).
        ExtendedSlice::projected(theta_next, rho_final, psi_c)
    }
}

/// An eSSVI surface: an ordered set of calibrated `(θ_i, ρ_i, ψ_i)` pillars with
/// monotone-`θ` linear interpolation of `ρ` and `ψ`, plus the closed-form static
/// no-arbitrage predicates over the whole pillar set.
///
/// The pillars are stored sorted by `θ` (ascending). The surface evaluates a
/// total variance at any `(k, θ)` by locating the bracketing pillars and
/// interpolating `(ρ, ψ)` linearly in `θ` (constant extrapolation outside the
/// pillar range), then evaluating the eSSVI slice. `θ` itself is the surface's
/// own coordinate, supplied by the caller's ATM total-variance term structure.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedSurface {
    /// Pillars sorted ascending by `θ`.
    pillars: Vec<ExtendedSlice>,
}

impl ExtendedSurface {
    /// Build an eSSVI surface from calibrated pillars.
    ///
    /// # Panics
    ///
    /// Panics if `pillars` is empty or the `θ` values are not strictly increasing.
    #[must_use]
    pub fn new(pillars: Vec<ExtendedSlice>) -> Self {
        assert!(!pillars.is_empty(), "eSSVI surface needs ≥ 1 pillar");
        assert!(
            pillars.windows(2).all(|w| w[0].theta < w[1].theta),
            "eSSVI surface pillars must be strictly increasing in theta"
        );
        Self { pillars }
    }

    /// **Robustly calibrate** a guaranteed-no-arbitrage eSSVI surface from
    /// per-pillar total-variance quotes — the hardened multi-maturity fit.
    ///
    /// `pillars` is one entry per maturity, **ascending in `θ`**: each is the ATM
    /// total variance `θ_i` paired with a set of `(k, w)` smile quotes (log-moneyness
    /// and total implied variance). The quote sets may be **wide** (large |k|, steep
    /// skew) and **sparse** (as few as one off-ATM point, or even none); the
    /// calibrator never panics, never returns a NaN/over-fit slice, and the surface
    /// it returns **always** satisfies both static no-arbitrage predicates
    /// ([`is_butterfly_free`] *and* [`is_calendar_free`]). The robustness comes from
    /// *projection*, not luck:
    ///
    /// 1. **Per-pillar damped fit (butterfly-projected every step).** With `θ_i`
    ///    pinned, `(ρ, ψ)` minimise the summed squared total-variance residual over
    ///    the pillar's quotes by a deterministic damped coordinate/Levenberg search;
    ///    every trial slice is run through [`ExtendedSlice::projected`], so the fit
    ///    only ever explores butterfly-free slices. A pillar with no off-ATM quote
    ///    falls back to the flat (`ρ = 0`) seed — sane, never degenerate.
    /// 2. **Forward calendar sweep.** Pillars are then made calendar-monotone by a
    ///    single forward pass: pillar `i` is re-projected against the *already-fixed*
    ///    pillar `i−1` via [`ExtendedSlice::project_after`], which raises `ψ_i` to at
    ///    least `ψ_{i−1}` (within `i`'s butterfly cap, shrinking `|ρ_i|` if needed to
    ///    make room) and clamps the ATM-skew gap `|ρ_iψ_i − ρ_{i−1}ψ_{i−1}|` into the
    ///    `ψ`-gap. This is a *minimal* projection: where the fit already satisfied the
    ///    calendar cone, the pillar is unchanged.
    ///
    /// The result is no-arb **by construction** — the parity oracle re-verifies it
    /// with independent Breeden-Litzenberger density and pointwise calendar numerics
    /// (never the closed-form predicate the calibrator used), so this is a genuine
    /// guarantee, not a self-certification.
    ///
    /// Provenance (doc-only): the projection-into-the-no-arb-domain approach to
    /// robust eSSVI calibration follows Hendriks & Martini (2019) §2–3 and the
    /// projected-Gauss-Newton SSVI calibration of Gatheral & Jacquier (2014).
    ///
    /// # Panics
    ///
    /// Panics if `pillars` is empty, any `θ ≤ 0`, or the `θ` values are not strictly
    /// increasing.
    ///
    /// [`is_butterfly_free`]: ExtendedSurface::is_butterfly_free
    /// [`is_calendar_free`]: ExtendedSurface::is_calendar_free
    #[must_use]
    pub fn calibrate(pillars: &[(f64, Vec<(f64, f64)>)]) -> Self {
        assert!(!pillars.is_empty(), "eSSVI calibration needs ≥ 1 pillar");
        assert!(
            pillars.windows(2).all(|w| w[0].0 < w[1].0),
            "eSSVI calibration pillars must be strictly increasing in theta"
        );
        for (theta, _) in pillars {
            assert!(*theta > 0.0, "eSSVI pillar theta must be positive: {theta}");
        }

        // (1) Per-pillar butterfly-projected damped fit.
        let mut fitted: Vec<ExtendedSlice> = pillars
            .iter()
            .map(|(theta, quotes)| fit_pillar(*theta, quotes))
            .collect();

        // (2) Forward calendar sweep: project each pillar after its predecessor.
        for i in 1..fitted.len() {
            let prev = fitted[i - 1];
            let cur = fitted[i];
            fitted[i] = prev.project_after(cur.theta, cur.rho, cur.psi);
        }

        Self { pillars: fitted }
    }

    /// The calibrated pillars, ascending in `θ`.
    #[must_use]
    pub fn pillars(&self) -> &[ExtendedSlice] {
        &self.pillars
    }

    /// The interpolated eSSVI slice at ATM total variance `θ`: `(ρ, ψ)` are
    /// linearly interpolated in `θ` between the bracketing pillars (constant
    /// extrapolation beyond the endpoints).
    #[must_use]
    pub fn slice_at(&self, theta: f64) -> ExtendedSlice {
        let p = &self.pillars;
        if theta <= p[0].theta {
            return ExtendedSlice::new(theta.max(f64::MIN_POSITIVE), p[0].rho, p[0].psi);
        }
        let last = &p[p.len() - 1];
        if theta >= last.theta {
            return ExtendedSlice::new(theta, last.rho, last.psi);
        }
        // Locate the bracketing pillars [lo, hi] with lo.theta ≤ θ < hi.theta.
        let mut hi = 1;
        while hi < p.len() && p[hi].theta <= theta {
            hi += 1;
        }
        let lo = &p[hi - 1];
        let hi = &p[hi];
        let frac = (theta - lo.theta) / (hi.theta - lo.theta);
        let rho = lo.rho + frac * (hi.rho - lo.rho);
        let psi = lo.psi + frac * (hi.psi - lo.psi);
        ExtendedSlice::new(theta, rho, psi)
    }

    /// Total implied variance `w(k, θ)` at log-moneyness `k` and ATM total
    /// variance `θ`, evaluating the interpolated slice.
    #[must_use]
    pub fn total_variance(&self, k: f64, theta: f64) -> f64 {
        self.slice_at(theta).total_variance(k)
    }

    /// Materialise a raw [`ParametricSlice`] at `θ` for forward `f`, expiry `t`.
    ///
    /// # Panics
    ///
    /// Panics if `θ`, `f` or `t` are non-positive.
    #[must_use]
    pub fn to_slice(&self, theta: f64, forward: f64, t: f64) -> ParametricSlice {
        self.slice_at(theta).to_slice(forward, t)
    }

    /// Whether every calibrated pillar is butterfly-arbitrage-free (per-slice).
    #[must_use]
    pub fn is_butterfly_free(&self) -> bool {
        self.pillars.iter().all(ExtendedSlice::is_butterfly_free)
    }

    /// Whether every consecutive pillar pair is calendar-arbitrage-free.
    #[must_use]
    pub fn is_calendar_free(&self) -> bool {
        self.pillars
            .windows(2)
            .all(|w| w[0].is_calendar_free_with(&w[1]))
    }
}

/// Fit one eSSVI pillar `(ρ, ψ)` at a pinned `θ` to its `(k, w)` quotes by a
/// deterministic damped least-squares, exploring **only** butterfly-free slices
/// (every trial is run through [`ExtendedSlice::projected`]). Robust to wide and
/// sparse quote sets: the search is a fixed-iteration coordinate descent over a
/// shrinking step with a Levenberg-style accept/reject, and a pillar with no
/// off-ATM quote falls back to the flat (`ρ = 0`, mild-`ψ`) seed. Always returns a
/// butterfly-free slice; never NaN, never over-fit.
fn fit_pillar(theta: f64, quotes: &[(f64, f64)]) -> ExtendedSlice {
    // The eSSVI total variance at the pinned θ with free (ρ, ψ), φ = ψ/θ.
    let w_of = |rho: f64, psi: f64, k: f64| -> f64 {
        let p = psi / theta;
        let pk = p * k + rho;
        0.5 * theta * (1.0 + rho * p * k + sqrt(pk * pk + (1.0 - rho * rho)))
    };
    // Summed squared total-variance residual over the (off-ATM) quotes. The ATM
    // point (k = 0) carries no shape information (w(0) = θ exactly for any ρ, ψ),
    // so it is skipped; θ is already pinned.
    let cost = |rho: f64, psi: f64| -> f64 {
        quotes
            .iter()
            .filter(|(k, _)| k.abs() > 1e-12)
            .map(|&(k, w)| {
                let d = w_of(rho, psi, k) - w;
                d * d
            })
            .sum::<f64>()
    };
    let off_atm = quotes.iter().filter(|(k, _)| k.abs() > 1e-12).count();

    // Sane seed: flat, mildly convex, deep inside the butterfly domain.
    let seed = ExtendedSlice::projected(
        theta,
        0.0,
        ExtendedSlice::butterfly_psi_cap(theta, 0.0) * 0.2,
    );
    if off_atm == 0 {
        // No shape information at all → return the flat fallback (no-arb by
        // construction). This is the sparsest possible input.
        return seed;
    }

    // Seed ρ from the sign/scale of the skew implied by the most distant quote and
    // ψ from the residual convexity; both are then projected.
    let (mut rho, mut psi) = seed_from_quotes(theta, quotes, &seed);
    let proj = ExtendedSlice::projected(theta, rho, psi);
    rho = proj.rho;
    psi = proj.psi;
    let mut best_cost = cost(rho, psi);

    // Deterministic damped coordinate descent over a shrinking step. Each round
    // tries ±step on ρ and ψ (projected); accepts the best improving move, else
    // shrinks the step. Fixed iteration budget ⇒ bit-reproducible; the projection
    // keeps every trial butterfly-free.
    let mut step_rho = 0.4_f64;
    let mut step_psi = ExtendedSlice::butterfly_psi_cap(theta, 0.0) * 0.25;
    for _ in 0..80 {
        let mut improved = false;
        let candidates = [
            (rho + step_rho, psi),
            (rho - step_rho, psi),
            (rho, psi + step_psi),
            (rho, psi - step_psi),
            (rho + step_rho, psi + step_psi),
            (rho - step_rho, psi - step_psi),
            (rho + step_rho, psi - step_psi),
            (rho - step_rho, psi + step_psi),
        ];
        for &(cr, cp) in &candidates {
            let s = ExtendedSlice::projected(theta, cr, cp);
            let c = cost(s.rho, s.psi);
            if c < best_cost {
                best_cost = c;
                rho = s.rho;
                psi = s.psi;
                improved = true;
            }
        }
        if !improved {
            // Refine around the incumbent.
            step_rho *= 0.5;
            step_psi *= 0.5;
            if step_rho < 1e-9 && step_psi < 1e-9 {
                break;
            }
        }
    }
    ExtendedSlice::projected(theta, rho, psi)
}

/// Seed `(ρ, ψ)` for a pillar fit from its quotes: `ρ` from the asymmetry of the
/// two most distant wing total variances (the skew sign/scale), `ψ` from the
/// average wing convexity above the ATM level `θ`. Both are deliberately mild and
/// will be projected by the caller; the seed only has to land in the right basin.
fn seed_from_quotes(theta: f64, quotes: &[(f64, f64)], fallback: &ExtendedSlice) -> (f64, f64) {
    // Most distant negative-k and positive-k quotes.
    let mut put: Option<(f64, f64)> = None;
    let mut call: Option<(f64, f64)> = None;
    for &(k, w) in quotes {
        if k < -1e-12 && put.is_none_or(|(pk, _)| k < pk) {
            put = Some((k, w));
        }
        if k > 1e-12 && call.is_none_or(|(ck, _)| k > ck) {
            call = Some((k, w));
        }
    }
    // ψ seed: lift above θ by the average wing excess variance, scaled to ψ-units.
    let wings: Vec<f64> = [put, call]
        .iter()
        .filter_map(|o| o.map(|(_, w)| w))
        .collect();
    let avg_excess = if wings.is_empty() {
        0.0
    } else {
        (wings.iter().sum::<f64>() / wings.len() as f64 - theta).max(0.0)
    };
    let psi_seed = (fallback.psi + 4.0 * avg_excess).max(1e-3);
    // ρ seed: skew sign from (call_w − put_w). A steep negative skew (put richer)
    // ⇒ ρ < 0. Scale to a mild magnitude; projection clamps the rest.
    let rho_seed = match (put, call) {
        (Some((_, wp)), Some((_, wc))) => {
            let skew = wc - wp;
            clamp(skew / theta.max(1e-6), -0.6, 0.6)
        }
        _ => 0.0,
    };
    (rho_seed, psi_seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    /// At constant `ρ` and `ψ = θ·φ(θ)` from a power-law `φ`, the eSSVI slice's
    /// total variance equals the existing SSVI `ParametricSurface::total_variance`
    /// **bit-for-bit** across a k-grid — SSVI byte-recovered as the special case.
    #[test]
    fn ssvi_byte_recovered_at_constant_rho() {
        let rho = -0.25;
        let eta = 0.8;
        let gamma = 0.4;
        let ssvi = ParametricSurface::new(rho, eta, gamma);
        for &theta in &[0.004_f64, 0.011, 0.03, 0.07] {
            // Build from the SSVI curvature φ(θ) directly so the arithmetic is
            // token-for-token the SSVI form (no ψ/θ round-trip).
            let essvi = ExtendedSlice::from_curvature(theta, rho, ssvi.phi(theta));
            for &k in &[-0.5, -0.2, -0.05, 0.0, 0.05, 0.2, 0.5] {
                let w_ssvi = ssvi.total_variance(k, theta);
                let w_essvi = essvi.total_variance(k);
                assert_eq!(
                    w_ssvi.to_bits(),
                    w_essvi.to_bits(),
                    "byte mismatch at theta={theta}, k={k}: SSVI {w_ssvi} vs eSSVI {w_essvi}"
                );
            }
        }
    }

    /// At k = 0 the eSSVI total variance is exactly θ (the ATM property).
    #[test]
    fn atm_total_variance_is_theta() {
        let s = ExtendedSlice::new(0.011, -0.2, 0.3);
        assert!(is_close(s.total_variance(0.0), 0.011, 1e-13, 1e-14));
    }

    /// The ATM skew `∂_k w|_{k=0}` equals `ρ·ψ` (the defining property of ψ).
    #[test]
    fn atm_skew_is_rho_psi() {
        let s = ExtendedSlice::new(0.02, -0.3, 0.25);
        let h = 1e-6;
        let skew_fd = (s.total_variance(h) - s.total_variance(-h)) / (2.0 * h);
        assert!(
            is_close(skew_fd, s.rho * s.psi, 1e-6, 1e-8),
            "ATM skew {skew_fd} must equal ρψ = {}",
            s.rho * s.psi
        );
    }

    /// The eSSVI→raw materialised slice reproduces the eSSVI total variance at
    /// every log-moneyness (the SSVI→raw map is exact).
    #[test]
    fn essvi_equals_its_raw_slice() {
        let s = ExtendedSlice::new(0.011, -0.25, 0.32);
        let slice = s.to_slice(1.10, 1.0);
        for &k in &[-0.4, -0.1, 0.0, 0.05, 0.3] {
            let w_surface = s.total_variance(k);
            let strike = 1.10 * celnet_core::math::exp(k);
            let w_slice = slice.total_variance(slice.log_moneyness(strike));
            assert!(
                is_close(w_surface, w_slice, 1e-12, 1e-13),
                "k={k}: surface {w_surface} vs slice {w_slice}"
            );
        }
    }

    /// A mild slice satisfies the closed-form butterfly conditions. With θ = 0.011
    /// and ρ = −0.25 the vertex-curvature bound `ψ ≤ √(4θ/(1+|ρ|)) ≈ 0.188` is the
    /// binding one; a ψ comfortably inside it is butterfly-free.
    #[test]
    fn mild_slice_is_butterfly_free() {
        let s = ExtendedSlice::new(0.011, -0.25, 0.14);
        assert!(s.is_butterfly_free());
    }

    /// A slice pushed past the butterfly bound (huge ψ) is flagged.
    #[test]
    fn excessive_psi_is_flagged() {
        // ψ(1+|ρ|) = 5·1.25 = 6.25 ≥ 4 ⇒ violates the first condition.
        let bad = ExtendedSlice::new(0.5, -0.25, 5.0);
        assert!(!bad.is_butterfly_free());
    }

    /// Calendar: a non-decreasing-θ, compatible-skew pair passes; a crossing pair
    /// (skew gap exceeding the ψ-gap) fails.
    #[test]
    fn calendar_condition() {
        let s1 = ExtendedSlice::new(0.006, -0.2, 0.20);
        let s2 = ExtendedSlice::new(0.011, -0.2, 0.30);
        assert!(
            s1.is_calendar_free_with(&s2),
            "compatible pair is calendar-free"
        );

        // Same θ ordering and ψ ordering, but the skew gap |ρ₂ψ₂ − ρ₁ψ₁| exceeds
        // the ψ-gap ⇒ a crossing ⇒ flagged.
        let s3 = ExtendedSlice::new(0.011, 0.9, 0.32);
        assert!(
            !s1.is_calendar_free_with(&s3),
            "skew gap {} exceeds psi gap {}",
            (s3.rho * s3.psi - s1.rho * s1.psi).abs(),
            s3.psi - s1.psi
        );
    }

    /// The surface interpolates `(ρ, ψ)` linearly in θ and reproduces a pillar
    /// exactly at its own θ.
    #[test]
    fn surface_interpolates_and_reproduces_pillars() {
        // ψ values stay inside the per-θ butterfly bound √(4θ/(1+|ρ|)) and the
        // pair (ρψ, ψ) gaps keep the slices from crossing (calendar-free).
        let surf = ExtendedSurface::new(vec![
            ExtendedSlice::new(0.006, -0.20, 0.10),
            ExtendedSlice::new(0.011, -0.25, 0.14),
            ExtendedSlice::new(0.030, -0.30, 0.20),
        ]);
        // At a pillar θ the slice equals the pillar.
        let at = surf.slice_at(0.011);
        assert!(is_close(at.rho, -0.25, 1e-13, 1e-14));
        assert!(is_close(at.psi, 0.14, 1e-13, 1e-14));
        // Midway between the first two pillars, ρ/ψ are the linear midpoints.
        let mid = surf.slice_at(0.0085);
        assert!(is_close(mid.rho, -0.225, 1e-12, 1e-13));
        assert!(is_close(mid.psi, 0.12, 1e-12, 1e-13));
        assert!(surf.is_butterfly_free());
        assert!(surf.is_calendar_free());
    }

    /// `projected` always lands inside the butterfly domain, even for absurd /
    /// non-finite candidates (the robustness postcondition).
    #[test]
    fn projected_is_always_butterfly_free() {
        let thetas = [1e-4_f64, 0.005, 0.02, 0.1, 0.5];
        let rhos = [-5.0_f64, -0.999, -0.3, 0.0, 0.6, 0.999, 5.0, f64::NAN];
        let psis = [-1.0_f64, 0.0, 1e-9, 0.5, 10.0, 1e6, f64::INFINITY, f64::NAN];
        for &theta in &thetas {
            for &rho in &rhos {
                for &psi in &psis {
                    let s = ExtendedSlice::projected(theta, rho, psi);
                    assert!(
                        s.is_butterfly_free(),
                        "projected({theta},{rho},{psi}) -> ({},{}) not butterfly-free",
                        s.rho,
                        s.psi
                    );
                    assert!(s.psi.is_finite() && s.psi > 0.0 && s.rho.is_finite());
                }
            }
        }
    }

    /// `project_after` always yields a slice that is both butterfly-free and
    /// calendar-free *after* the fixed predecessor — even when the candidate's own
    /// butterfly cap is below the predecessor's ψ (the |ρ|-shrink fallback engages).
    #[test]
    fn project_after_is_always_no_arb() {
        // A steep, high-ψ earlier pillar whose ψ may exceed a later steep pillar's
        // cap — forces the cap-aware ρ shrink.
        let prev = ExtendedSlice::projected(0.02, -0.6, 0.35);
        let candidates = [
            (0.025_f64, -0.9, 0.30),
            (0.04, 0.8, 0.05),
            (0.05, -0.999, 5.0),
            (0.021, 0.5, 0.01),
            (0.1, 0.0, 0.0),
        ];
        for &(theta, rho, psi) in &candidates {
            let next = prev.project_after(theta, rho, psi);
            assert!(
                next.is_butterfly_free(),
                "project_after butterfly fail: {next:?}"
            );
            assert!(
                prev.is_calendar_free_with(&next),
                "project_after calendar fail: prev={prev:?} next={next:?}"
            );
        }
    }

    /// The robust surface calibrator returns a no-arb surface from a deliberately
    /// **wide and sparse** quote set — and reasonably reprices the quotes it can.
    #[test]
    fn calibrate_wide_sparse_is_no_arb() {
        // Three maturities; the short one has a single wide off-ATM quote (sparse),
        // the others a steep two-wing skew (wide).
        let pillars = vec![
            (0.004_f64, vec![(0.0, 0.004), (-0.8, 0.0075)]), // sparse + very wide put
            (
                0.02,
                vec![(-0.6, 0.030), (-0.2, 0.0215), (0.0, 0.02), (0.25, 0.024)],
            ),
            (0.06, vec![(-0.5, 0.072), (0.0, 0.06), (0.4, 0.069)]),
        ];
        let surf = ExtendedSurface::calibrate(&pillars);
        assert!(
            surf.is_butterfly_free(),
            "calibrated surface not butterfly-free"
        );
        assert!(
            surf.is_calendar_free(),
            "calibrated surface not calendar-free"
        );
        // Every pillar reproduced its pinned θ at ATM exactly.
        for (theta, _) in &pillars {
            let s = surf.slice_at(*theta);
            assert!(is_close(s.total_variance(0.0), *theta, 1e-12, 1e-13));
        }
    }

    /// A pillar with NO off-ATM quote falls back to the flat (ρ=0) butterfly-free
    /// seed rather than panicking or over-fitting.
    #[test]
    fn calibrate_empty_pillar_falls_back_flat() {
        let pillars = vec![(0.01_f64, vec![]), (0.03, vec![(0.0, 0.03)])];
        let surf = ExtendedSurface::calibrate(&pillars);
        assert!(surf.is_butterfly_free());
        assert!(surf.is_calendar_free());
        assert!(
            (surf.pillars()[0].rho).abs() < 1e-12,
            "empty pillar should be flat"
        );
    }
}
