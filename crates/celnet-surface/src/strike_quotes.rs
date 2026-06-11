//! Strike-axis (log-moneyness) smile slice from exchange-chain quotes.
//!
//! Digital-asset venues quote European options on a **strike grid** per expiry
//! (e.g. the Deribit BTC/ETH chains — deribit.com/kb: European, coin-settled,
//! expiry cut **08:00 UTC**, premium in coins), not at FX delta pillars. The
//! slice's `t` is the continuous (24×7) year fraction to the venue's UTC expiry
//! cut — crypto has no weekend/holiday skip, so the identity
//! [`crate::termstructure::CalendarClock`] is the correct business clock and no
//! new clock is needed. The forward is formed **only** through the
//! [`celnet_types::Carry`] seam (`F = S·e^{b·t}`, with `b = r − funding`
//! assembled by the crypto leaf's `funding_carry`); no function in this module
//! inspects the carry variant (ADR-0008 no-match-carry).
//!
//! # The two-front-end seam (CRYPTO-SURFACE-LEAF-SPEC §2.3)
//!
//! This module is the **strike front-end** onto the asset-class-neutral
//! `(k = ln(K/F), w = σ²·t)` total-variance plane:
//!
//! ```text
//! FX delta front-end (calibrate.rs):  MarketQuotes ─calibrate_pillar─▶ (k, w) ─fit─▶ ParametricSlice
//! Strike front-end (this module):     StrikeQuoteSlice ─ln(K/F), σ²t─▶ (k, w) ─fit─▶ ParametricSlice
//! ```
//!
//! Both lower into the **same** validated [`ParametricSlice`], so everything
//! downstream — [`celnet_core::Smile`], the Durrleman density factor and Lee
//! wing bound, [`crate::arbitrage`], [`crate::termstructure`],
//! [`crate::surface::VolSurface`] — is shared, already-gated machinery. The
//! basis is selected at **compile time** by the front-end type; there is no
//! runtime quote-basis enum and no per-asset-class `match` in the calibration
//! path (the F5/F6 anti-pattern ADR-0008 removed).
//!
//! # Why not [`crate::quotes::MarketContext`] (decision note)
//!
//! `MarketContext` is already carry-neutral in its *market state*, but it
//! requires a resolved FX `ConventionRecord` and carries the delta↔strike
//! machinery; a strike-quoted chain has no delta convention to resolve. Making
//! the record optional would churn the ~60 constructor call sites across ~12
//! crates (ADR-0008 §4.4) for no numerical gain, so the strike front-end gets
//! the additive light context [`StrikeSliceContext`] instead — zero FX risk.
//!
//! # Fit provenance (doc-only)
//!
//! Raw-SVI slice of Gatheral (2004); the dimension-reduced "quasi-explicit"
//! calibration of De Marco & Martini (2009): the inner `(a, c, d)` problem is
//! linear least squares solved exactly via the 3×3 normal equations, the outer
//! search runs over `(m, σ)` only, on the verbatim-shared deterministic
//! Gauss-Newton kit ([`crate::fitmath`]). All arithmetic routes through
//! [`celnet_core::math`] (libm) with fixed iteration budgets — bit-reproducible,
//! the crate's existing determinism contract. Calibration is the marking path,
//! not the pinned zero-alloc hot core; `Vec` use matches `calibrate.rs`.

use celnet_core::math::{ln, sqrt};
use celnet_types::Carry;

use crate::fitmath::{clamp, gauss_newton_2, solve3};
use crate::parametric::ParametricSlice;
use crate::strangle::CalibrationError;
use crate::surface::{SmileModel, VolSurface};
use crate::termstructure::{CalendarClock, TenorPillar};

/// One exchange-chain quote: a strike and its Black implied volatility.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeQuote {
    /// Strike `K` (quote currency per unit of underlying; USD for a coin chain).
    pub strike: f64,
    /// Black implied volatility (absolute, e.g. `0.65` = 65 vols).
    pub vol: f64,
}

/// Minimum quotes for a well-posed five-parameter slice fit.
pub const MIN_STRIKE_QUOTES: usize = 5;

/// A validated one-expiry strike-gridded quote set (ascending strikes).
#[derive(Debug, Clone, PartialEq)]
pub struct StrikeQuoteSlice {
    quotes: Vec<StrikeQuote>,
}

impl StrikeQuoteSlice {
    /// Validate and wrap an exchange-chain quote set.
    ///
    /// # Panics
    ///
    /// Panics on structural invalidity (house style, cf. [`ParametricSlice::new`]):
    /// fewer than [`MIN_STRIKE_QUOTES`] quotes; strikes not finite, not strictly
    /// increasing, or non-positive; vols non-positive / non-finite.
    #[must_use]
    pub fn new(quotes: Vec<StrikeQuote>) -> Self {
        assert!(
            quotes.len() >= MIN_STRIKE_QUOTES,
            "a strike slice needs at least {MIN_STRIKE_QUOTES} quotes; got {}",
            quotes.len()
        );
        for q in &quotes {
            assert!(
                q.strike.is_finite() && q.strike > 0.0,
                "strikes must be finite and positive: {}",
                q.strike
            );
            assert!(
                q.vol.is_finite() && q.vol > 0.0,
                "vols must be finite and positive: {}",
                q.vol
            );
        }
        assert!(
            quotes.windows(2).all(|w| w[0].strike < w[1].strike),
            "strikes must be strictly increasing"
        );
        Self { quotes }
    }

    /// The validated quotes, ascending in strike.
    #[must_use]
    pub fn quotes(&self) -> &[StrikeQuote] {
        &self.quotes
    }

    /// Number of quotes in the slice.
    #[must_use]
    pub fn len(&self) -> usize {
        self.quotes.len()
    }

    /// Always `false`: a slice carries at least [`MIN_STRIKE_QUOTES`] quotes by
    /// construction (present for the `len`/`is_empty` API convention).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// The carry-seam market state for one strike-quoted slice: spot, the
/// cost-of-carry producer, and vol-time. The strike axis needs no delta/ATM
/// conventions — deliberately *not* [`crate::quotes::MarketContext`] (which
/// requires a resolved FX `ConventionRecord`); see the module-doc decision note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeSliceContext {
    /// Spot price of the underlying.
    pub spot: f64,
    /// Cost-of-carry model (`Carry::CostOfCarry { r, b = r − funding }` for a
    /// digital asset; any [`Carry`] is accepted — the seam is asset-class-blind).
    pub carry: Carry,
    /// Vol-time to the venue expiry cut, years (continuous 24×7 day count).
    pub t: f64,
}

impl StrikeSliceContext {
    /// Construct a strike-slice market context.
    ///
    /// # Panics
    ///
    /// Panics on non-positive / non-finite `spot` or `t`.
    #[must_use]
    pub fn new(spot: f64, carry: Carry, t: f64) -> Self {
        assert!(
            spot.is_finite() && spot > 0.0,
            "spot must be finite and positive: {spot}"
        );
        assert!(
            t.is_finite() && t > 0.0,
            "vol-time must be finite and positive: {t}"
        );
        Self { spot, carry, t }
    }

    /// Outright forward `F = S·e^{b·t}` — the IDENTICAL op sequence as
    /// [`crate::quotes::MarketContext::forward`] (`spot * carry.forward_factor(t)`),
    /// so the two front-ends agree on the forward to the bit for the same carry.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * self.carry.forward_factor(self.t)
    }

    /// Total variance `σ²·t` helper (mirrors
    /// [`crate::quotes::MarketContext::atm_total_variance`]).
    #[must_use]
    pub fn total_variance(&self, vol: f64) -> f64 {
        vol * vol * self.t
    }
}

/// A fitted strike-axis slice plus its reproduction diagnostics. The fit never
/// hides quality behind a threshold: callers gate on the reported errors
/// (as [`strike_surface`] does with its explicit budget).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeSliceFit {
    /// The fitted arbitrage-projected raw-SVI slice (anchored at `F`, `t`).
    pub slice: ParametricSlice,
    /// Root-mean-square absolute vol error over the input quotes.
    pub rms_vol_error: f64,
    /// Worst absolute vol error over the input quotes.
    pub max_vol_error: f64,
}

/// Below this `d = b·σ` the inner solution is a flat slice (`b = 0`) and the
/// skew `ρ` is indeterminate — mapped to `0`.
const FLAT_D_FLOOR: f64 = 1e-12;

/// The hostile constant residual returned when the inner linear solve fails:
/// the Gauss-Newton residual-decrease guard then rejects the step
/// deterministically (it cannot improve on any finite accepted state).
const SOLVER_REJECT_RESIDUAL: f64 = 1e6;

/// Log-moneyness half-span (around the vertex `m`) of the extraction-time
/// Durrleman butterfly admission scan — matches the span the crate's slice
/// tests certify.
const ADMISSION_SCAN_SPAN: f64 = 2.0;
/// Samples of the admission scan (`±span` grid; spacing ≈ 1e-3, at least as
/// fine as the `min_butterfly_density_factor(_, 4096)` certification grid).
const ADMISSION_SCAN_SAMPLES: usize = 4097;
/// Admission tolerance on the scanned density factor (FP noise floor; an
/// arbitrage-free slice has `g ≥ 0` exactly).
const ADMISSION_SCAN_TOL: f64 = 1e-9;
/// Fixed bisection budget of the admission ray projection (deterministic;
/// resolves the ray parameter to 2⁻³²).
const ADMISSION_SHRINK_ITERS: usize = 32;

/// Map a projected inner solution to the validated raw-SVI slice — `b = d/σ`,
/// `ρ = c/d` (`0` for a flat slice; clamped to the open constructor domain, a
/// no-op for any market-plausible fit) — with the well-posedness guards
/// mirroring `fit_svi`, computed exactly as the validating constructor computes
/// them so construction cannot panic. `None` only for an ill-posed candidate.
fn well_posed_slice(
    a: f64,
    c: f64,
    d: f64,
    m: f64,
    sigma: f64,
    forward: f64,
    t: f64,
) -> Option<ParametricSlice> {
    let b = d / sigma;
    let rho = if d > FLAT_D_FLOOR {
        clamp(c / d, -0.999_999, 0.999_999)
    } else {
        0.0
    };
    let w_min = a + b * sigma * sqrt(1.0 - rho * rho);
    if !(a.is_finite() && b.is_finite() && rho.is_finite() && m.is_finite() && sigma.is_finite())
        || sigma <= 0.0
        || w_min < -1e-12
    {
        return None;
    }
    Some(ParametricSlice::new(a, b, rho, m, sigma, forward, t))
}

/// Whether the slice passes the Durrleman butterfly admission scan: the
/// production density factor `g(k)` ([`ParametricSlice::butterfly_density_factor`],
/// the published butterfly-no-arbitrage law `g ≥ 0`) on the deterministic
/// `m ± span` grid, with early exit on the first violation (the same
/// accept/reject decision as `min_butterfly_density_factor ≥ −tol`).
fn passes_admission_scan(slice: &ParametricSlice) -> bool {
    let lo = slice.m - ADMISSION_SCAN_SPAN;
    let hi = slice.m + ADMISSION_SCAN_SPAN;
    for i in 0..ADMISSION_SCAN_SAMPLES {
        let k = lo + (hi - lo) * (i as f64) / ((ADMISSION_SCAN_SAMPLES - 1) as f64);
        if slice.butterfly_density_factor(k) < -ADMISSION_SCAN_TOL {
            return false;
        }
    }
    true
}

/// Admit the extracted inner solution, or **ray-project it onto the
/// butterfly-admissible set**.
///
/// The box projection (steps 1–4 of `project_inner`) is necessary but not
/// sufficient for butterfly no-arbitrage: an arbitrageable (e.g. V-shaped)
/// quote grid can pull the least-squares solution into a `g < 0` slice while
/// every box constraint — including the Lee cap at its boundary — holds. The
/// fit must never emit such a slice, so extraction walks the candidate along
/// the straight ray toward the **flat-market fit** `(a_flat = mean(w), c = d = 0)`
/// — which is always admissible (`w ≡ a_flat > 0` ⇒ `g ≡ 1`) — taking the
/// largest admissible ray parameter by fixed-budget bisection (deterministic,
/// [`ADMISSION_SHRINK_ITERS`] halvings). Along the ray the box constraints are
/// preserved (`|c|`,`d` scale together; the Lee slope shrinks; the variance
/// floor is bounded below by `(1−s)·a_flat`).
///
/// For any quote set an arbitrage-free slice can reproduce, the unconstrained
/// optimum already passes the scan and this is a **no-op** (gated by the exact
/// round-trip and parity tests); for a hostile grid it deterministically caps
/// the chase, surfacing the misfit through the reported vol errors instead of
/// through an arbitrageable surface.
fn admit_or_shrink(
    a: f64,
    c: f64,
    d: f64,
    a_flat: f64,
    m: f64,
    sigma: f64,
    forward: f64,
    t: f64,
) -> Option<ParametricSlice> {
    let candidate = |s: f64| {
        well_posed_slice(
            (1.0 - s) * a_flat + s * a,
            s * c,
            s * d,
            m,
            sigma,
            forward,
            t,
        )
    };
    if let Some(slice) = candidate(1.0)
        && passes_admission_scan(&slice)
    {
        return Some(slice);
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..ADMISSION_SHRINK_ITERS {
        let s = 0.5 * (lo + hi);
        match candidate(s) {
            Some(slice) if passes_admission_scan(&slice) => lo = s,
            _ => hi = s,
        }
    }
    // `lo = 0` (the flat-market fit) is admissible by construction; the filter
    // is a belt-and-braces guard against FP pathology, surfaced as an error by
    // the caller rather than an invalid slice.
    candidate(lo).filter(passes_admission_scan)
}

/// Lower the strike-gridded quotes onto the neutral anchor plane:
/// `k_i = ln(K_i/F)`, `w_i = σ_i²·t` — the identical lowering the FX delta
/// front-end performs after its pillar calibration (`calibrate.rs::anchors`).
fn anchors(forward: f64, t: f64, quotes: &StrikeQuoteSlice) -> (Vec<f64>, Vec<f64>) {
    let ks = quotes
        .quotes()
        .iter()
        .map(|q| ln(q.strike / forward))
        .collect();
    let ws = quotes.quotes().iter().map(|q| q.vol * q.vol * t).collect();
    (ks, ws)
}

/// Solve the inner linear least-squares problem exactly for fixed `(m, σ)`
/// (the dimension reduction): with `y_i = (k_i − m)/σ`, `z_i = √(y_i² + 1)`,
/// raw-SVI total variance is `w = a + c·y + d·z` (`c = bρσ`, `d = bσ`) —
/// **linear** in `(a, c, d)`. The 3×3 normal equations are accumulated in
/// deterministic ascending-index order (plain `f64` sums) and solved by the
/// shared [`crate::fitmath::solve3`]. Returns the **unprojected** `(a, c, d)`;
/// `None` on a singular system or non-finite solution.
fn inner_solve(ks: &[f64], ws: &[f64], m: f64, sigma: f64) -> Option<(f64, f64, f64)> {
    let n = ks.len() as f64;
    let (mut sy, mut sz, mut syy, mut syz, mut szz) = (0.0_f64, 0.0, 0.0, 0.0, 0.0);
    let (mut sw, mut syw, mut szw) = (0.0_f64, 0.0, 0.0);
    for (&k, &w) in ks.iter().zip(ws) {
        let y = (k - m) / sigma;
        let z = sqrt(y * y + 1.0);
        sy += y;
        sz += z;
        syy += y * y;
        syz += y * z;
        szz += z * z;
        sw += w;
        syw += y * w;
        szw += z * w;
    }
    let lhs = [[n, sy, sz], [sy, syy, syz], [sz, syz, szz]];
    let [a, c, d] = solve3(&lhs, &[sw, syw, szw])?;
    (a.is_finite() && c.is_finite() && d.is_finite()).then_some((a, c, d))
}

/// Project the inner solution into the slice no-arbitrage box. Each clamp is a
/// **no-op when the unconstrained optimum is admissible** (exact ground truth
/// recovers exactly):
///
/// 1. `d ← max(d, 0)`; if `d == 0` then `c ← 0` (flat slice, `b = 0`);
/// 2. `|ρ| ≤ 1`: if `|c| > d` then `c ← sign(c)·d`;
/// 3. Lee wing bound `b(1+|ρ|) ≤ 2` ⇔ `(d + |c|)/σ ≤ 2` (the same
///    dimensionless constant 2 the FX `project_svi` enforces — no `t`): scale
///    `c, d` by `2σ/(d + |c|)`, preserving `ρ`, capping the slope;
/// 4. non-negative minimum variance `w_min = a + √(d² − c²) ≥ 0` (step 2
///    guarantees `d ≥ |c|`): if violated, lift `a ← a − w_min`.
///
/// This projection is applied **identically** inside the Gauss-Newton residual
/// closure and at final extraction — a mismatch would silently bias fits while
/// staying green on mild data (the sharp numerical edge; the parity oracle's
/// independent-linear-algebra leg guards it).
fn project_inner(a: f64, c: f64, d: f64, sigma: f64) -> (f64, f64, f64) {
    let d = d.max(0.0);
    let mut c = if d == 0.0 { 0.0 } else { c };
    if c.abs() > d {
        c = c.signum() * d;
    }
    let mut d = d;
    if (d + c.abs()) / sigma > 2.0 {
        let s = 2.0 * sigma / (d + c.abs());
        c *= s;
        d *= s;
    }
    let w_min = a + sqrt(d * d - c * c);
    let a = if w_min < 0.0 { a - w_min } else { a };
    (a, c, d)
}

/// Fit a raw-SVI slice to one expiry's strike-gridded quotes.
///
/// Anchors each quote on the neutral `(k, w)` plane against the **carry-seam
/// forward** [`StrikeSliceContext::forward`], then runs the dimension-reduced
/// calibration (module doc): exact inner linear solve in `(a, c, d)` under the
/// no-arbitrage box projection, outer deterministic damped Gauss-Newton over
/// `(m, σ)` only (`m ∈ [k₁ − 0.5, kₙ + 0.5]`, `σ ∈ [10⁻⁴, 5]` — the FX
/// `project_svi` σ cap; seeds `m₀ = k_{argmin w}` first occurrence,
/// `σ₀ = clamp(0.25·(kₙ − k₁), 10⁻², 1)`), and finally the **Durrleman
/// `g ≥ 0` admission** of the extracted slice (see `admit_or_shrink` — the fit
/// never emits a butterfly-arbitrageable slice; misfit surfaces through the
/// reported vol errors instead).
///
/// The returned [`StrikeSliceFit`] carries the reproduction diagnostics; this
/// function does **not** gate on them (callers decide — see [`strike_surface`]).
///
/// # Errors
///
/// [`CalibrationError::DegenerateQuote`] when the extraction cannot form a
/// well-posed slice (singular inner system at the optimum, non-finite
/// parameters, or a negative minimum total variance beyond tolerance).
pub fn fit_strike_slice(
    ctx: &StrikeSliceContext,
    quotes: &StrikeQuoteSlice,
) -> Result<StrikeSliceFit, CalibrationError> {
    let forward = ctx.forward();
    let t = ctx.t;
    let (ks, ws) = anchors(forward, t, quotes);
    let n = ks.len();

    // Deterministic seeds: the vertex near the lowest observed total variance
    // (first occurrence), the width from the observed k-span.
    let mut min_i = 0;
    for (i, &w) in ws.iter().enumerate() {
        if w < ws[min_i] {
            min_i = i;
        }
    }
    let mut m = ks[min_i];
    let mut sigma = clamp(0.25 * (ks[n - 1] - ks[0]), 1e-2, 1.0);

    let (m_lo, m_hi) = (ks[0] - 0.5, ks[n - 1] + 0.5);
    let project = |m: f64, sigma: f64| (clamp(m, m_lo, m_hi), clamp(sigma, 1e-4, 5.0));

    // The flat-market anchor of the admission ray projection: the mean input
    // total variance (deterministic ascending-index accumulation; strictly
    // positive since every quoted vol is).
    let a_flat = ws.iter().sum::<f64>() / n as f64;

    // The projected-profile residual: the inner solve INCLUDING the projection,
    // identical to the projected solution the final extraction starts from.
    let residuals = |m: f64, sigma: f64| -> Vec<f64> {
        match inner_solve(&ks, &ws, m, sigma) {
            Some((a, c, d)) => {
                let (a, c, d) = project_inner(a, c, d, sigma);
                ks.iter()
                    .zip(&ws)
                    .map(|(&k, &w)| {
                        let y = (k - m) / sigma;
                        let z = sqrt(y * y + 1.0);
                        a + c * y + d * z - w
                    })
                    .collect()
            }
            None => vec![SOLVER_REJECT_RESIDUAL; n],
        }
    };

    gauss_newton_2(&mut m, &mut sigma, residuals, (m_lo, m_hi), project);

    // Extraction: recompute the projected inner solution at the final (m, σ) —
    // bitwise the state the accepted residuals evaluated — then admit it (or
    // ray-project it onto the butterfly-admissible set; a no-op for any
    // arbitrage-free-reproducible quote set).
    let Some((a, c, d)) = inner_solve(&ks, &ws, m, sigma) else {
        return Err(CalibrationError::DegenerateQuote);
    };
    let (a, c, d) = project_inner(a, c, d, sigma);
    let Some(slice) = admit_or_shrink(a, c, d, a_flat, m, sigma, forward, t) else {
        return Err(CalibrationError::DegenerateQuote);
    };

    // Reproduction diagnostics in absolute vols over the input quotes.
    let mut sum_sq = 0.0;
    let mut max_vol_error = 0.0_f64;
    for (&k, q) in ks.iter().zip(quotes.quotes()) {
        let vol_fit = sqrt(slice.total_variance(k) / t);
        let err = (vol_fit - q.vol).abs();
        sum_sq += err * err;
        max_vol_error = max_vol_error.max(err);
    }
    let rms_vol_error = sqrt(sum_sq / n as f64);

    Ok(StrikeSliceFit {
        slice,
        rms_vol_error,
        max_vol_error,
    })
}

/// Assemble per-expiry strike-axis fits into the unified re-strikable surface
/// ([`SmileModel::Parametric`] pillars on the identity clock — the 24×7 clock).
///
/// `max_vol_error` is the per-slice reproduction gate (absolute vols); a slice
/// exceeding it (or with a non-finite error) maps to
/// [`CalibrationError::NoConvergence`] — never a silent garbage pillar.
/// Cross-expiry calendar no-arbitrage is *reported* by the existing
/// [`VolSurface::arbitrage_report`] machinery, consistent with the FX surface:
/// assemble, report, let the publish gate decide — never silently repair.
///
/// # Errors
///
/// Any per-slice [`CalibrationError`], or [`CalibrationError::NoConvergence`]
/// for a slice whose reproduction exceeds `max_vol_error`.
///
/// # Panics
///
/// Panics (delegated to the term-structure constructor) if `pillars` is empty
/// or the expiries are not strictly increasing.
pub fn strike_surface(
    pillars: &[(StrikeSliceContext, StrikeQuoteSlice)],
    max_vol_error: f64,
) -> Result<VolSurface<ParametricSlice, CalendarClock>, CalibrationError> {
    let mut tenor_pillars = Vec::with_capacity(pillars.len());
    for (ctx, quotes) in pillars {
        let fit = fit_strike_slice(ctx, quotes)?;
        if !fit.max_vol_error.is_finite() || fit.max_vol_error > max_vol_error {
            return Err(CalibrationError::NoConvergence);
        }
        tenor_pillars.push(TenorPillar::new(fit.slice, ctx.forward(), ctx.t));
    }
    Ok(VolSurface::new(SmileModel::Parametric, tenor_pillars))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_conventions::resolve;
    use celnet_core::math::exp;
    use celnet_crypto_vanilla::funding_carry;
    use celnet_types::{CcyPair, Tenor};

    use crate::quotes::MarketContext;

    /// Raw-SVI total variance, re-typed from the published equation (Gatheral
    /// 2004; Gatheral & Jacquier 2014 eq. 3.1) — NOT a call into
    /// `ParametricSlice::total_variance` (oracle-independence discipline).
    fn svi_w_reference(k: f64, a: f64, b: f64, rho: f64, m: f64, sigma: f64) -> f64 {
        a + b * (rho * (k - m) + ((k - m) * (k - m) + sigma * sigma).sqrt())
    }

    /// A strike grid of `n` log-spaced strikes on `[lo, hi]` with vols generated
    /// from the reference SVI formula at the context's carry-seam forward.
    fn reference_grid(
        ctx: &StrikeSliceContext,
        truth: (f64, f64, f64, f64, f64),
        lo: f64,
        hi: f64,
        n: usize,
    ) -> StrikeQuoteSlice {
        let (a, b, rho, m, sigma) = truth;
        let f = ctx.forward();
        let quotes = (0..n)
            .map(|i| {
                let strike = lo * (hi / lo).powf(i as f64 / (n - 1) as f64);
                let k = (strike / f).ln();
                let w = svi_w_reference(k, a, b, rho, m, sigma);
                StrikeQuote {
                    strike,
                    vol: (w / ctx.t).sqrt(),
                }
            })
            .collect();
        StrikeQuoteSlice::new(quotes)
    }

    /// The BTC-scale O1 fixture: funding carry through the seam, exact raw-SVI
    /// ground truth, 13 log-spaced strikes 40k–90k.
    fn btc_fixture() -> (
        StrikeSliceContext,
        (f64, f64, f64, f64, f64),
        StrikeQuoteSlice,
    ) {
        let ctx = StrikeSliceContext::new(60_000.0, funding_carry(0.05, 0.02), 0.25);
        let truth = (0.012, 0.08, -0.4, 0.02, 0.15);
        let quotes = reference_grid(&ctx, truth, 40_000.0, 90_000.0, 13);
        (ctx, truth, quotes)
    }

    /// O4 both halves: the context forward is the carry seam to the bit —
    /// (i) `funding_carry` reproduces `S·e^{(r−funding)·t}` byte-identically;
    /// (ii) for `Carry::FxRates` the strike context matches the FX
    /// `MarketContext::forward` byte-identically — the strike context accepts
    /// any carry with no asset-class branch (neutrality proof, not just a
    /// crypto fact).
    #[test]
    fn context_forward_is_carry_seam_bits() {
        let (spot, r, funding, t) = (60_000.0, 0.05, 0.02, 0.25);
        let ctx = StrikeSliceContext::new(spot, funding_carry(r, funding), t);
        // The oracle recomputes b = r − funding with the identical expression,
        // through libm directly (independent of the seam accessors).
        assert_eq!(
            ctx.forward().to_bits(),
            (spot * libm::exp((r - funding) * t)).to_bits(),
            "funding-carry forward must be byte-identical to S·e^{{(r−funding)t}}"
        );

        let carry = Carry::FxRates {
            r_dom: 0.02,
            r_for: 0.01,
        };
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        let s_ctx = StrikeSliceContext::new(1.10, carry, 1.0);
        let m_ctx = MarketContext::new(1.10, carry, 1.0, conv);
        assert_eq!(
            s_ctx.forward().to_bits(),
            m_ctx.forward().to_bits(),
            "FxRates forward must match the FX MarketContext to the bit"
        );
        // The σ²t helper mirrors the FX context's helper to the bit too.
        assert_eq!(
            s_ctx.total_variance(0.1042).to_bits(),
            m_ctx.atm_total_variance(0.1042).to_bits()
        );
    }

    /// The `(k, w)` lowering is pinned to the published formulas with `to_bits`
    /// equality against an in-test re-computation.
    #[test]
    fn anchors_match_hand_computed() {
        let (ctx, _, quotes) = btc_fixture();
        let f = ctx.forward();
        let (ks, ws) = anchors(f, ctx.t, &quotes);
        for (i, q) in quotes.quotes().iter().enumerate() {
            assert_eq!(ks[i].to_bits(), ln(q.strike / f).to_bits(), "k_{i}");
            assert_eq!(ws[i].to_bits(), (q.vol * q.vol * ctx.t).to_bits(), "w_{i}");
        }
    }

    /// O1 (BTC scale): an exact arb-free raw-SVI grid round-trips — the fitter
    /// never sees the truth parameters, only `(K_i, σ_i)`; recovery failure
    /// would be loud disagreement.
    #[test]
    fn exact_svi_grid_round_trips() {
        let (ctx, truth, quotes) = btc_fixture();
        let (a, b, rho, m, sigma) = truth;
        let fit = fit_strike_slice(&ctx, &quotes).unwrap();

        // On-grid reproduction to half a micro-vol.
        assert!(
            fit.max_vol_error <= 5e-7,
            "on-grid max vol error {} must be ≤ 5e-7",
            fit.max_vol_error
        );
        assert!(fit.rms_vol_error <= fit.max_vol_error);

        // Off-grid total variance agrees with the reference formula.
        for j in 0..=40 {
            let k = -0.6 + 1.2 * f64::from(j) / 40.0;
            let w_fit = fit.slice.total_variance(k);
            let w_truth = svi_w_reference(k, a, b, rho, m, sigma);
            assert!(
                (w_fit - w_truth).abs() <= 1e-7,
                "off-grid w at k={k}: fit {w_fit} vs truth {w_truth}"
            );
        }

        // Parameter recovery.
        assert!((fit.slice.b - b).abs() / b <= 1e-4, "b: {}", fit.slice.b);
        assert!(
            (fit.slice.sigma - sigma).abs() / sigma <= 1e-4,
            "sigma: {}",
            fit.slice.sigma
        );
        assert!(
            (fit.slice.rho - rho).abs() <= 1e-4,
            "rho: {}",
            fit.slice.rho
        );
        assert!((fit.slice.m - m).abs() <= 1e-4, "m: {}", fit.slice.m);
        assert!((fit.slice.a - a).abs() <= 1e-6, "a: {}", fit.slice.a);

        // The slice is anchored at the carry-seam forward and expiry.
        assert_eq!(fit.slice.forward.to_bits(), ctx.forward().to_bits());
        assert_eq!(fit.slice.t.to_bits(), ctx.t.to_bits());
    }

    /// A constant-vol grid reproduces the flat vol (the inner clamps are no-ops
    /// and the linear solve is exact on a flat target).
    #[test]
    fn flat_grid_fits_flat() {
        let ctx = StrikeSliceContext::new(3_000.0, funding_carry(0.04, 0.01), 0.5);
        let f = ctx.forward();
        let vol = 0.55;
        let quotes = StrikeQuoteSlice::new(
            (0..9)
                .map(|i| StrikeQuote {
                    strike: f * exp(-0.4 + 0.8 * f64::from(i) / 8.0),
                    vol,
                })
                .collect(),
        );
        let fit = fit_strike_slice(&ctx, &quotes).unwrap();
        assert!(
            fit.max_vol_error <= 1e-10,
            "flat reproduction error {}",
            fit.max_vol_error
        );
        for &k in &[-0.5, -0.1, 0.0, 0.2, 0.5] {
            let v = sqrt(fit.slice.total_variance(k) / ctx.t);
            assert!((v - vol).abs() <= 1e-10, "flat vol at k={k}: {v}");
        }
        assert!(fit.slice.is_butterfly_free(2.0, 1e-6));
        assert!(fit.slice.satisfies_wing_bound());
    }

    /// The fit is deterministic: two identical runs produce bit-identical
    /// slices and evaluations (the crate's bit-reproducibility contract).
    #[test]
    fn fit_is_bit_reproducible() {
        let (ctx, _, quotes) = btc_fixture();
        let fa = fit_strike_slice(&ctx, &quotes).unwrap();
        let fb = fit_strike_slice(&ctx, &quotes).unwrap();
        for (x, y) in [
            (fa.slice.a, fb.slice.a),
            (fa.slice.b, fb.slice.b),
            (fa.slice.rho, fb.slice.rho),
            (fa.slice.m, fb.slice.m),
            (fa.slice.sigma, fb.slice.sigma),
            (fa.rms_vol_error, fb.rms_vol_error),
            (fa.max_vol_error, fb.max_vol_error),
        ] {
            assert_eq!(x.to_bits(), y.to_bits());
        }
        let f = ctx.forward();
        for x in [0.7, 0.9, 1.0, 1.1, 1.4] {
            assert_eq!(
                fa.slice.vol_at(f * x).to_bits(),
                fb.slice.vol_at(f * x).to_bits()
            );
        }
    }

    /// A put-skewed grid yields a negative-ρ slice with a higher put-wing vol.
    #[test]
    fn skewed_grid_reproduces_skew_direction() {
        let ctx = StrikeSliceContext::new(3_000.0, funding_carry(0.04, 0.01), 0.5);
        let f = ctx.forward();
        let quotes = reference_grid(
            &ctx,
            (0.015, 0.25, -0.55, 0.0, 0.25),
            f * exp(-0.4),
            f * exp(0.4),
            11,
        );
        let fit = fit_strike_slice(&ctx, &quotes).unwrap();
        assert!(
            fit.slice.rho < 0.0,
            "rho {} must be negative",
            fit.slice.rho
        );
        let put_wing = fit.slice.vol_at(f * exp(-0.3));
        let call_wing = fit.slice.vol_at(f * exp(0.3));
        assert!(
            put_wing > call_wing,
            "put-skew: put wing {put_wing} must exceed call wing {call_wing}"
        );
    }

    /// A hostile V-shaped grid (its total-variance slope exceeds the Lee cap —
    /// no arbitrage-free smile can reproduce it): the projection refuses to
    /// chase the arbitrage, so the fit returns Ok with a LARGE reproduction
    /// error and the fitted slice still passes both no-arbitrage notions.
    #[test]
    fn hostile_grid_stays_butterfly_free() {
        let ctx = StrikeSliceContext::new(50_000.0, funding_carry(0.04, 0.01), 1.0);
        let f = ctx.forward();
        let quotes = StrikeQuoteSlice::new(
            (0..9)
                .map(|i| {
                    let k = -0.45 + 0.9 * f64::from(i) / 8.0;
                    StrikeQuote {
                        strike: f * exp(k),
                        vol: 0.25 + 1.8 * k.abs(),
                    }
                })
                .collect(),
        );
        let fit = fit_strike_slice(&ctx, &quotes).unwrap();
        assert!(
            fit.max_vol_error > 1e-2,
            "the V cannot be reproduced arbitrage-free: error {} must be large",
            fit.max_vol_error
        );
        assert!(
            fit.slice.is_butterfly_free(2.0, 1e-6),
            "projected fit must stay butterfly-free; min g = {}",
            fit.slice.min_butterfly_density_factor(2.0, 4096)
        );
        assert!(fit.slice.satisfies_wing_bound());
    }

    /// `strike_surface` maps an over-budget pillar to `NoConvergence` — never a
    /// silent garbage pillar.
    #[test]
    fn surface_gates_bad_pillar() {
        let ctx = StrikeSliceContext::new(50_000.0, funding_carry(0.04, 0.01), 1.0);
        let f = ctx.forward();
        let hostile = StrikeQuoteSlice::new(
            (0..9)
                .map(|i| {
                    let k = -0.45 + 0.9 * f64::from(i) / 8.0;
                    StrikeQuote {
                        strike: f * exp(k),
                        vol: 0.25 + 1.8 * k.abs(),
                    }
                })
                .collect(),
        );
        assert_eq!(
            strike_surface(&[(ctx, hostile)], 1e-4).unwrap_err(),
            CalibrationError::NoConvergence
        );
    }

    #[test]
    #[should_panic(expected = "at least 5 quotes")]
    fn too_few_quotes_panics() {
        let _ = StrikeQuoteSlice::new(
            (1..5)
                .map(|i| StrikeQuote {
                    strike: f64::from(i) * 100.0,
                    vol: 0.5,
                })
                .collect(),
        );
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn unsorted_strikes_panic() {
        let mut quotes: Vec<StrikeQuote> = (1..=5)
            .map(|i| StrikeQuote {
                strike: f64::from(i) * 100.0,
                vol: 0.5,
            })
            .collect();
        quotes.swap(1, 3);
        let _ = StrikeQuoteSlice::new(quotes);
    }

    #[test]
    #[should_panic(expected = "vols must be finite and positive")]
    fn nonpositive_vol_panics() {
        let mut quotes: Vec<StrikeQuote> = (1..=5)
            .map(|i| StrikeQuote {
                strike: f64::from(i) * 100.0,
                vol: 0.5,
            })
            .collect();
        quotes[2].vol = 0.0;
        let _ = StrikeQuoteSlice::new(quotes);
    }

    #[test]
    #[should_panic(expected = "strikes must be finite and positive")]
    fn nonpositive_strike_panics() {
        let mut quotes: Vec<StrikeQuote> = (1..=5)
            .map(|i| StrikeQuote {
                strike: f64::from(i) * 100.0,
                vol: 0.5,
            })
            .collect();
        quotes[0].strike = -100.0;
        let _ = StrikeQuoteSlice::new(quotes);
    }

    #[test]
    #[should_panic(expected = "spot must be finite and positive")]
    fn nonpositive_spot_panics() {
        let _ = StrikeSliceContext::new(0.0, funding_carry(0.05, 0.02), 0.25);
    }

    #[test]
    #[should_panic(expected = "vol-time must be finite and positive")]
    fn nonpositive_t_panics() {
        let _ = StrikeSliceContext::new(60_000.0, funding_carry(0.05, 0.02), 0.0);
    }
}
