//! FRTB Standardised-Approach **GIRR (general interest rate risk) delta** capital charge.
//!
//! The regulatory sensitivities-based-method (SbM) delta charge for fixed-income rate risk, per the
//! consolidated Basel Framework **MAR21** (source standard: BCBS d457, *Minimum capital requirements
//! for market risk*, Jan 2019). This is the second increment of the central-core FI risk workstream
//! (Phase C2b): it consumes the per-tenor rate sensitivities the C2a scenario engine and
//! [`celnet_rates::OisRisk`] already produce (the key-rate ladder) and reduces them to the prescribed
//! GIRR delta capital number the risk cube will aggregate in C2c.
//!
//! # The computation (MAR21)
//!
//! 1. **Net sensitivities to the prescribed vertices.** GIRR delta risk factors live on the risk-free
//!    yield curve(s) of each currency at ten fixed tenor vertices (MAR21.53): `0.25, 0.5, 1, 2, 3, 5,
//!    10, 15, 20, 30` years ([`GIRR_VERTICES`]). Sensitivities to the same `(currency, curve, vertex)`
//!    risk factor are summed to the net sensitivity `s_k` (MAR21.8 defines `s_k` as the value change
//!    per unit of the risk-free rate, i.e. the 1 bp PV change divided by `0.0001`).
//! 2. **Risk-weight** each net sensitivity: `WS_k = RW_k · s_k`, with the prescribed per-vertex delta
//!    risk weights (MAR21.53, [`GIRR_DELTA_RISK_WEIGHTS`]).
//! 3. **Bucket charge** — a GIRR bucket is a currency. Within a bucket the weighted sensitivities are
//!    aggregated with the prescribed intra-bucket correlation `ρ_kl` (MAR21.55/56):
//!    `K_b = sqrt(max(0, Σ_k WS_k² + Σ_k Σ_{l≠k} ρ_kl · WS_k · WS_l))`.
//! 4. **Cross-bucket** — different-currency buckets aggregate with `γ_bc = 50%` (MAR21.59):
//!    `Delta = sqrt(Σ_b K_b² + Σ_b Σ_{c≠b} γ_bc · S_b · S_c)`, `S_b = Σ_k WS_k`, using the MAR21.4(4)
//!    cap/floor fallback `S_b = max[min(Σ WS_k, K_b), −K_b]` when the naive radicand is negative.
//!
//! Because the SbM prescribes three **correlation scenarios** (MAR21.6 — medium, high, low), each
//! computation is parameterised by [`CorrelationScenario`]; the regulatory GIRR-delta figure is the
//! per-scenario charge under each, aggregated to the overall SbM maximum at the cross-risk-class level
//! (C2c). [`girr_delta_charge`] computes one scenario; [`girr_delta_charges_all`] returns all three.
//!
//! # Scope (Phase C2b) and deferrals
//!
//! - **In scope:** the GIRR **delta** charge — vertex mapping, the exact MAR21 risk-weight and
//!   correlation tables, intra-bucket `K_b`, and cross-bucket aggregation with the `S_b` cap/floor,
//!   under all three correlation scenarios; oracle-validated against the published tables and an
//!   independent hand computation.
//! - **Deferred (C2b-extension):** GIRR **vega** and **curvature** charges; the optional national
//!   discretion to divide the risk weights by `√2` for a specified set of liquid currencies
//!   (MAR21.53) — only the base (un-divided) weights are encoded here; GIRR sub-curves that do *not*
//!   correlate at the 99.90% cross-curve factor (inflation at 40%, cross-currency basis at 0%) — every
//!   [`CurveId`] modelled here is a risk-free yield curve, which correlate cross-curve at 99.90%.
//! - **Deferred (C2c):** integration into the `celnet-risk-cube` non-additive path, sign-normalising
//!   the position P&L conventions, and exposing the key-rate axis end to end.
//!
//! Method/standard provenance lives in prose only, never in identifiers (GUIDE.md §8). Deterministic:
//! no RNG; the only transcendental is the correlation `exp`, and all summations run over sorted risk
//! factors/buckets so a fixed input is bit-reproducible.

use celnet_types::Ccy;

// ---------------------------------------------------------------------------------------------
// Prescribed regulatory tables (MAR21 — verified against the published standard, never derived
// from the engine). The oracle test `published_tables_match_the_standard` asserts each literal
// against an independent transcription of the BCBS MAR21 tables.
// ---------------------------------------------------------------------------------------------

/// The ten prescribed GIRR delta risk-free-curve vertices, in years (MAR21.53).
///
/// A sensitivity computed at any other tenor is linearly interpolated onto these by
/// [`map_ladder_to_vertices`]. Strictly increasing.
pub const GIRR_VERTICES: [f64; 10] = [0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0];

/// The prescribed GIRR delta risk weights per vertex (MAR21.53), aligned index-for-index with
/// [`GIRR_VERTICES`]: `1.7%, 1.7%, 1.6%, 1.3%, 1.2%, 1.1%, 1.1%, 1.1%, 1.1%, 1.1%`.
///
/// These are the current in-force values (BCBS d457, 2019). The earlier (2016, BCBS d352)
/// calibration was `√2 ×` larger; the optional MAR21.53 `√2` divisor for a specified set of liquid
/// currencies is a deferred national-discretion extension and is *not* applied here.
pub const GIRR_DELTA_RISK_WEIGHTS: [f64; 10] = [
    0.017, 0.017, 0.016, 0.013, 0.012, 0.011, 0.011, 0.011, 0.011, 0.011,
];

/// The decay parameter `θ` in the intra-bucket delta correlation (MAR21.55): `θ = 3%`.
pub const GIRR_CORRELATION_THETA: f64 = 0.03;

/// The intra-bucket delta correlation floor (MAR21.55): `40%`.
pub const GIRR_CORRELATION_FLOOR: f64 = 0.40;

/// The multiplier applied to the intra-bucket correlation for two sensitivities in the **same
/// currency but different risk-free curves** (MAR21.56): `99.90%`.
pub const GIRR_DIFFERENT_CURVE_FACTOR: f64 = 0.999;

/// The cross-bucket delta correlation `γ_bc` between two different-currency GIRR buckets
/// (MAR21.59): `50%`.
pub const GIRR_CROSS_BUCKET_GAMMA: f64 = 0.50;

/// One basis point in absolute rate terms (`1 bp = 1e-4`). The MAR21.8 delta sensitivity is the
/// 1 bp PV change divided by this, i.e. the value change per unit rate.
const BASIS_POINT: f64 = 1e-4;

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

/// Identity of a risk-free curve within a currency bucket (OIS, an IBOR tenor curve, …).
///
/// Two GIRR delta risk factors are on the **same curve** iff their [`CurveId`] is equal; otherwise
/// their intra-bucket correlation carries the MAR21.56 `99.90%` cross-curve factor. The numeric value
/// is an opaque label — only equality matters. The discount/OIS curve is conventionally
/// [`CurveId::OIS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurveId(pub u16);

impl CurveId {
    /// The overnight-indexed (discount) curve — the default risk-free curve.
    pub const OIS: CurveId = CurveId(0);
}

/// The three prescribed SbM correlation scenarios (MAR21.6).
///
/// Each scales every prescribed correlation (both the intra-bucket `ρ_kl` and the cross-bucket
/// `γ_bc`): [`CorrelationScenario::High`] `→ min(1.25·ρ, 1)`, [`CorrelationScenario::Medium`] `→ ρ`
/// (the base prescribed value), [`CorrelationScenario::Low`] `→ max(2·ρ − 1, 0.75·ρ)`. The overall
/// SbM charge is the maximum over the three; a standalone risk class exposes all three so the
/// cross-class maximum can be taken by the aggregator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrelationScenario {
    /// `ρ → min(1.25·ρ, 1)`.
    High,
    /// `ρ → ρ` (the base prescribed correlation).
    Medium,
    /// `ρ → max(2·ρ − 1, 0.75·ρ)`.
    Low,
}

impl CorrelationScenario {
    /// The three scenarios in a fixed order `[High, Medium, Low]`.
    pub const ALL: [CorrelationScenario; 3] = [
        CorrelationScenario::High,
        CorrelationScenario::Medium,
        CorrelationScenario::Low,
    ];

    /// Apply the MAR21.6 scenario scaling to a base prescribed correlation `rho`.
    #[must_use]
    fn scale(self, rho: f64) -> f64 {
        match self {
            CorrelationScenario::High => (1.25 * rho).min(1.0),
            CorrelationScenario::Medium => rho,
            CorrelationScenario::Low => (2.0 * rho - 1.0).max(0.75 * rho),
        }
    }
}

/// A single GIRR delta sensitivity: the net rate sensitivity of a book to one `(currency, curve,
/// vertex)` risk factor.
///
/// `sensitivity` is the MAR21.8 delta sensitivity `s_k` — the change in the book's value per **unit**
/// of the risk-free rate at the vertex (equivalently, the 1 bp PV change divided by `0.0001`). Build
/// these directly with [`GirrSensitivity::new`], from a 1 bp key-rate DV01 with
/// [`GirrSensitivity::from_dv01`], or in bulk from a key-rate ladder with [`map_ladder_to_vertices`].
/// Multiple sensitivities to the *same* risk factor are netted (summed) by the charge computation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GirrSensitivity {
    bucket: Ccy,
    curve: CurveId,
    vertex: usize,
    sensitivity: f64,
}

impl GirrSensitivity {
    /// A sensitivity to the `(bucket, curve, vertex)` risk factor, where `vertex` indexes
    /// [`GIRR_VERTICES`] (`0..10`) and `sensitivity` is the MAR21.8 per-unit-rate delta `s_k`.
    ///
    /// # Errors
    ///
    /// [`GirrError::VertexOutOfRange`] if `vertex >= 10` (not a prescribed vertex).
    pub fn new(
        bucket: Ccy,
        curve: CurveId,
        vertex: usize,
        sensitivity: f64,
    ) -> Result<Self, GirrError> {
        if vertex >= GIRR_VERTICES.len() {
            return Err(GirrError::VertexOutOfRange { index: vertex });
        }
        Ok(Self {
            bucket,
            curve,
            vertex,
            sensitivity,
        })
    }

    /// A sensitivity from a **1 bp key-rate DV01** (the PV change for a `+1 bp` bump of the vertex,
    /// e.g. one entry of [`celnet_rates::OisRisk::key_rate`]): `s_k = dv01 / 0.0001`.
    ///
    /// # Errors
    ///
    /// [`GirrError::VertexOutOfRange`] if `vertex >= 10`.
    pub fn from_dv01(
        bucket: Ccy,
        curve: CurveId,
        vertex: usize,
        dv01: f64,
    ) -> Result<Self, GirrError> {
        Self::new(bucket, curve, vertex, dv01 / BASIS_POINT)
    }

    /// The currency bucket this sensitivity belongs to.
    #[must_use]
    pub fn bucket(&self) -> Ccy {
        self.bucket
    }

    /// The risk-free curve this sensitivity is on.
    #[must_use]
    pub fn curve(&self) -> CurveId {
        self.curve
    }

    /// The vertex index into [`GIRR_VERTICES`] (`0..10`).
    #[must_use]
    pub fn vertex(&self) -> usize {
        self.vertex
    }

    /// The MAR21.8 per-unit-rate delta sensitivity `s_k`.
    #[must_use]
    pub fn sensitivity(&self) -> f64 {
        self.sensitivity
    }
}

/// One point of a key-rate ladder: a **1 bp DV01** (PV change for a `+1 bp` bump) at a tenor in years.
///
/// This is exactly the shape of a [`celnet_rates::OisRisk::key_rate`] entry paired with its
/// calibrating-quote tenor, or a C2a per-pillar sensitivity paired with its pillar time. Map a slice
/// of these onto the prescribed vertices with [`map_ladder_to_vertices`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderPoint {
    /// The ladder point's tenor, in years (need not be a prescribed vertex).
    pub tenor_years: f64,
    /// The 1 bp DV01 at this tenor (PV change per `+1 bp`).
    pub dv01: f64,
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

/// The per-bucket (per-currency) GIRR delta charge components.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GirrBucketCharge {
    /// The currency this bucket aggregates.
    pub bucket: Ccy,
    /// The bucket-level charge `K_b = sqrt(max(0, Σ WS_k² + Σ_{k≠l} ρ_kl WS_k WS_l))`.
    pub k_b: f64,
    /// The signed bucket sensitivity `S_b = Σ_k WS_k`, consumed by the cross-bucket aggregation.
    pub s_b: f64,
}

/// The GIRR delta capital charge for one correlation scenario: the per-bucket components and the
/// cross-bucket aggregate the risk cube consumes (C2c).
#[derive(Clone, Debug, PartialEq)]
pub struct GirrDeltaCharge {
    /// The correlation scenario (MAR21.6) this charge was computed under.
    pub scenario: CorrelationScenario,
    /// The per-bucket components, in deterministic (currency-code) order.
    pub buckets: Vec<GirrBucketCharge>,
    /// The aggregate GIRR delta charge `Delta` for this scenario (a non-negative capital amount).
    pub charge: f64,
}

/// A failure constructing a GIRR delta sensitivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GirrError {
    /// A vertex index outside `0..10` — not one of the ten prescribed [`GIRR_VERTICES`].
    VertexOutOfRange {
        /// The offending index.
        index: usize,
    },
}

impl core::fmt::Display for GirrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VertexOutOfRange { index } => write!(
                f,
                "vertex index {index} is out of range (0..{})",
                GIRR_VERTICES.len()
            ),
        }
    }
}

impl core::error::Error for GirrError {}

// ---------------------------------------------------------------------------------------------
// Vertex mapping
// ---------------------------------------------------------------------------------------------

/// The two prescribed vertices bracketing `tenor`, with linear-interpolation weights that sum to 1.
///
/// A tenor at or below the shortest vertex maps entirely to it; at or above the longest, entirely to
/// it; in between it splits across the two neighbours by linear interpolation in tenor. Because the
/// weights sum to 1, the mapping **preserves the total sensitivity** (a tested invariant). Returns
/// `[(vertex_index, weight); 2]` (the two entries coincide at the extremes, one carrying weight 0).
fn bracketing_vertices(tenor: f64) -> [(usize, f64); 2] {
    let last = GIRR_VERTICES.len() - 1;
    if tenor <= GIRR_VERTICES[0] {
        return [(0, 1.0), (0, 0.0)];
    }
    if tenor >= GIRR_VERTICES[last] {
        return [(last, 1.0), (last, 0.0)];
    }
    // Strictly inside: find i with GIRR_VERTICES[i] <= tenor < GIRR_VERTICES[i+1].
    let mut i = 0;
    while tenor >= GIRR_VERTICES[i + 1] {
        i += 1;
    }
    let (t_lo, t_hi) = (GIRR_VERTICES[i], GIRR_VERTICES[i + 1]);
    let w_hi = (tenor - t_lo) / (t_hi - t_lo);
    [(i, 1.0 - w_hi), (i + 1, w_hi)]
}

/// Map a key-rate ladder (1 bp DV01s at arbitrary tenors) for one `(bucket, curve)` onto the ten
/// prescribed GIRR vertices, returning the per-vertex net delta sensitivities `s_k`.
///
/// Each ladder point's DV01 is converted to the MAR21.8 per-unit-rate sensitivity (`dv01 / 0.0001`)
/// and distributed across its two bracketing vertices by [`bracketing_vertices`]; contributions to
/// the same vertex accumulate. Only vertices that receive a non-zero net sensitivity are emitted, in
/// ascending vertex order. The sum of the returned sensitivities equals the sum of `dv01 / 0.0001`
/// over the ladder (the interpolation is sensitivity-preserving).
#[must_use]
pub fn map_ladder_to_vertices(
    bucket: Ccy,
    curve: CurveId,
    ladder: &[LadderPoint],
) -> Vec<GirrSensitivity> {
    let mut acc = [0.0f64; GIRR_VERTICES.len()];
    for point in ladder {
        let s = point.dv01 / BASIS_POINT;
        for (v, w) in bracketing_vertices(point.tenor_years) {
            acc[v] += w * s;
        }
    }
    acc.iter()
        .enumerate()
        .filter(|&(_, &s)| s != 0.0)
        .map(|(v, &s)| GirrSensitivity {
            bucket,
            curve,
            vertex: v,
            sensitivity: s,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Correlations
// ---------------------------------------------------------------------------------------------

/// The base (medium-scenario) intra-bucket GIRR delta correlation between two risk factors
/// `(curve1, vertex1)` and `(curve2, vertex2)` in the same currency (MAR21.55/56).
///
/// `ρ = max(exp(−θ · |T_k − T_l| / min(T_k, T_l)), 40%)` for the tenor pair, multiplied by `99.90%`
/// when the two curves differ. For the same vertex on two different curves the tenor factor is
/// `exp(0) = 1`, giving `99.90%`.
#[must_use]
fn girr_delta_correlation(curve1: CurveId, vertex1: usize, curve2: CurveId, vertex2: usize) -> f64 {
    let (t_k, t_l) = (GIRR_VERTICES[vertex1], GIRR_VERTICES[vertex2]);
    let tenor = (-GIRR_CORRELATION_THETA * (t_k - t_l).abs() / t_k.min(t_l))
        .exp()
        .max(GIRR_CORRELATION_FLOOR);
    let curve_factor = if curve1 == curve2 {
        1.0
    } else {
        GIRR_DIFFERENT_CURVE_FACTOR
    };
    tenor * curve_factor
}

// ---------------------------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------------------------

/// A netted, risk-weighted sensitivity within a bucket: `(curve, vertex, WS_k)`.
type WeightedFactor = (CurveId, usize, f64);

/// The bucket-level charge `K_b = sqrt(max(0, Σ WS_k² + Σ_{k≠l} ρ_kl WS_k WS_l))` and the signed
/// bucket sensitivity `S_b = Σ WS_k`, for the given correlation scenario.
fn bucket_charge(factors: &[WeightedFactor], scenario: CorrelationScenario) -> (f64, f64) {
    let mut sum_sq = 0.0;
    let mut s_b = 0.0;
    for &(_, _, ws) in factors {
        sum_sq += ws * ws;
        s_b += ws;
    }
    let mut cross = 0.0;
    for (i, &(c_i, v_i, ws_i)) in factors.iter().enumerate() {
        for (j, &(c_j, v_j, ws_j)) in factors.iter().enumerate() {
            if i == j {
                continue;
            }
            let rho = scenario.scale(girr_delta_correlation(c_i, v_i, c_j, v_j));
            cross += rho * ws_i * ws_j;
        }
    }
    let k_b = (sum_sq + cross).max(0.0).sqrt();
    (k_b, s_b)
}

/// The cross-bucket aggregate `Delta = sqrt(Σ_b K_b² + Σ_{b≠c} γ · S_b · S_c)` (MAR21.59), applying
/// the MAR21.4(4) fallback `S_b = max[min(Σ WS_k, K_b), −K_b]` when the naive radicand is negative.
fn cross_bucket_charge(buckets: &[GirrBucketCharge], gamma: f64) -> f64 {
    let diag: f64 = buckets.iter().map(|b| b.k_b * b.k_b).sum();
    let cross = |s: &dyn Fn(usize) -> f64| -> f64 {
        let mut acc = 0.0;
        for i in 0..buckets.len() {
            for j in 0..buckets.len() {
                if i == j {
                    continue;
                }
                acc += gamma * s(i) * s(j);
            }
        }
        acc
    };
    let radicand = diag + cross(&|i| buckets[i].s_b);
    if radicand < 0.0 {
        // Regulatory fallback: cap/floor each S_b into [−K_b, K_b]; the resulting radicand is
        // non-negative by construction (belt-and-suspenders `max(0.0)`).
        let capped = |i: usize| buckets[i].s_b.min(buckets[i].k_b).max(-buckets[i].k_b);
        (diag + cross(&capped)).max(0.0).sqrt()
    } else {
        radicand.sqrt()
    }
}

/// Group sensitivities into deterministically-ordered buckets of netted, risk-weighted factors.
///
/// Sensitivities to the same `(bucket, curve, vertex)` risk factor are summed to the net `s_k`, then
/// weighted `WS_k = RW_vertex · s_k`. Buckets are ordered by currency code and factors within a
/// bucket by `(curve, vertex)`, so every downstream summation runs in a fixed order (bit-reproducible).
fn group_buckets(sensitivities: &[GirrSensitivity]) -> Vec<(Ccy, Vec<WeightedFactor>)> {
    use std::collections::HashMap;
    let mut by_bucket: HashMap<Ccy, HashMap<(CurveId, usize), f64>> = HashMap::new();
    for s in sensitivities {
        *by_bucket
            .entry(s.bucket)
            .or_default()
            .entry((s.curve, s.vertex))
            .or_insert(0.0) += s.sensitivity;
    }
    let mut buckets: Vec<(Ccy, Vec<WeightedFactor>)> = by_bucket
        .into_iter()
        .map(|(ccy, factors)| {
            let mut fs: Vec<WeightedFactor> = factors
                .into_iter()
                .map(|((curve, vertex), s)| (curve, vertex, GIRR_DELTA_RISK_WEIGHTS[vertex] * s))
                .collect();
            fs.sort_by_key(|&(curve, vertex, _)| (curve, vertex));
            (ccy, fs)
        })
        .collect();
    buckets.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    buckets
}

/// Compute the FRTB SbM **GIRR delta** capital charge for a set of net sensitivities, under one
/// correlation scenario (MAR21.6).
///
/// Sensitivities are netted per `(currency, curve, vertex)` risk factor, risk-weighted (MAR21.53),
/// aggregated within each currency bucket via the prescribed intra-bucket correlation (MAR21.55/56)
/// into `K_b`, and across currency buckets with `γ = 50%` (MAR21.59) into the aggregate charge, using
/// the MAR21.4(4) `S_b` cap/floor when the naive radicand is negative. An empty input yields a zero
/// charge. Deterministic and bit-reproducible for a fixed input.
#[must_use]
pub fn girr_delta_charge(
    sensitivities: &[GirrSensitivity],
    scenario: CorrelationScenario,
) -> GirrDeltaCharge {
    let grouped = group_buckets(sensitivities);
    let buckets: Vec<GirrBucketCharge> = grouped
        .iter()
        .map(|(ccy, factors)| {
            let (k_b, s_b) = bucket_charge(factors, scenario);
            GirrBucketCharge {
                bucket: *ccy,
                k_b,
                s_b,
            }
        })
        .collect();
    let gamma = scenario.scale(GIRR_CROSS_BUCKET_GAMMA);
    let charge = cross_bucket_charge(&buckets, gamma);
    GirrDeltaCharge {
        scenario,
        buckets,
        charge,
    }
}

/// The GIRR delta charge under all three correlation scenarios (MAR21.6), returned in the fixed order
/// `[High, Medium, Low]` ([`CorrelationScenario::ALL`]).
///
/// The overall SbM capital number is the maximum over the three scenarios, but that maximum is taken
/// across **all** risk classes together (delta, vega, curvature of every class), so it is the risk
/// cube's job (C2c) — this returns the three GIRR-delta components that feed it.
#[must_use]
pub fn girr_delta_charges_all(sensitivities: &[GirrSensitivity]) -> [GirrDeltaCharge; 3] {
    CorrelationScenario::ALL.map(|scenario| girr_delta_charge(sensitivities, scenario))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-12;

    fn usd() -> Ccy {
        Ccy::USD
    }
    fn eur() -> Ccy {
        Ccy::EUR
    }

    // -----------------------------------------------------------------------------------------
    // (a) The encoded tables match the published MAR21 standard EXACTLY. The right-hand literals
    //     are an INDEPENDENT transcription of the BCBS d457 / MAR21 tables (confirmed against a
    //     live regulator's verbatim adoption), asserted against the engine's encoded constants.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn published_tables_match_the_standard() {
        // MAR21.53 vertices (years) and delta risk weights.
        assert_eq!(
            GIRR_VERTICES,
            [0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0]
        );
        assert_eq!(
            GIRR_DELTA_RISK_WEIGHTS,
            [
                0.017, 0.017, 0.016, 0.013, 0.012, 0.011, 0.011, 0.011, 0.011, 0.011
            ]
        );
        // MAR21.55 correlation parameters.
        assert_eq!(GIRR_CORRELATION_THETA, 0.03);
        assert_eq!(GIRR_CORRELATION_FLOOR, 0.40);
        // MAR21.56 same-currency different-curve multiplier.
        assert_eq!(GIRR_DIFFERENT_CURVE_FACTOR, 0.999);
        // MAR21.59 cross-bucket correlation.
        assert_eq!(GIRR_CROSS_BUCKET_GAMMA, 0.50);
    }

    // -----------------------------------------------------------------------------------------
    // (a') The correlation formula reproduces hand values, including the floor and the cross-curve
    //      factor, and the three MAR21.6 scenario scalings.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn correlation_formula_matches_hand_values() {
        // 2y (idx 3) vs 5y (idx 5), same curve: exp(-0.03 * 3 / 2) = exp(-0.045).
        let rho = girr_delta_correlation(CurveId::OIS, 3, CurveId::OIS, 5);
        assert!((rho - (-0.045_f64).exp()).abs() <= TOL);
        assert!((rho - 0.955_997_481_833_1).abs() <= 1e-13);

        // 0.25y (idx 0) vs 30y (idx 9), same curve: exp(-0.03 * 29.75 / 0.25) ≈ 0.028 < 0.40 ⇒ floor.
        let floored = girr_delta_correlation(CurveId::OIS, 0, CurveId::OIS, 9);
        assert_eq!(floored, GIRR_CORRELATION_FLOOR);
        assert_eq!(floored, 0.40);

        // Same vertex (5y), different curves: exp(0) = 1, times the 99.90% cross-curve factor.
        let cross_curve = girr_delta_correlation(CurveId::OIS, 5, CurveId(1), 5);
        assert_eq!(cross_curve, 0.999);

        // Different vertex AND different curve: the same-curve rho times 99.90%.
        let both = girr_delta_correlation(CurveId::OIS, 3, CurveId(1), 5);
        assert!((both - rho * 0.999).abs() <= TOL);
    }

    #[test]
    fn scenario_scaling_matches_mar21_6() {
        let rho = 0.8;
        assert!((CorrelationScenario::High.scale(rho) - 1.0).abs() <= TOL); // min(1.0, 1.25*0.8=1.0)
        assert!((CorrelationScenario::Medium.scale(rho) - 0.8).abs() <= TOL);
        // max(2*0.8-1=0.6, 0.75*0.8=0.6) = 0.6
        assert!((CorrelationScenario::Low.scale(rho) - 0.6).abs() <= TOL);

        // High caps at 1.0; a 0.999 cross-curve correlation becomes exactly 1.0 under High.
        assert!((CorrelationScenario::High.scale(0.999) - 1.0).abs() <= TOL);

        // Low branch where 0.75*rho dominates 2*rho-1 (small rho): rho=0.4 ⇒ max(-0.2, 0.3)=0.3.
        assert!((CorrelationScenario::Low.scale(0.4) - 0.3).abs() <= TOL);
    }

    // -----------------------------------------------------------------------------------------
    // (b) A worked example computed independently by hand (Python calculator, offline), hardcoded
    //     here as literals AND re-expressed as the explicit closed form — the engine matches ≤1e-12.
    //
    //     Portfolio (medium scenario):
    //       USD bucket: s(2y)=+1000, s(5y)=+2000, same curve.
    //         WS_2y = 0.013*1000 = 13, WS_5y = 0.011*2000 = 22.
    //         rho(2y,5y) = exp(-0.045) = 0.9559974818331.
    //         K_USD = sqrt(13^2 + 22^2 + 2*rho*13*22) = 34.63857040364878.
    //         S_USD = 35.
    //       EUR bucket: s(10y)=-1500, single vertex.
    //         WS_10y = 0.011*(-1500) = -16.5.  K_EUR = 16.5.  S_EUR = -16.5.
    //       Cross-bucket (gamma=0.5): radicand = K_USD^2 + K_EUR^2 + 2*0.5*35*(-16.5) = 894.5805596...
    //         Delta = sqrt(894.5805596085331) = 29.90953960876919.
    // -----------------------------------------------------------------------------------------

    fn worked_portfolio() -> Vec<GirrSensitivity> {
        vec![
            GirrSensitivity::new(usd(), CurveId::OIS, 3, 1000.0).unwrap(),
            GirrSensitivity::new(usd(), CurveId::OIS, 5, 2000.0).unwrap(),
            GirrSensitivity::new(eur(), CurveId::OIS, 6, -1500.0).unwrap(),
        ]
    }

    #[test]
    fn worked_example_matches_hand_computation() {
        let charge = girr_delta_charge(&worked_portfolio(), CorrelationScenario::Medium);

        // Buckets are in currency-code order: EUR before USD.
        assert_eq!(charge.buckets.len(), 2);
        assert_eq!(charge.buckets[0].bucket, eur());
        assert_eq!(charge.buckets[1].bucket, usd());

        // EUR: single-vertex bucket ⇒ K_b = |WS| = 16.5, S_b = -16.5 (exact literals).
        assert!((charge.buckets[0].k_b - 16.5).abs() <= TOL);
        assert!((charge.buckets[0].s_b + 16.5).abs() <= TOL);

        // USD: hand K_USD and S_USD (offline-computed literals).
        assert!((charge.buckets[1].k_b - 34.638_570_403_648_78).abs() <= 1e-11);
        assert!((charge.buckets[1].s_b - 35.0).abs() <= TOL);

        // Independent closed form of K_USD from first principles (different code path than the
        // engine's pairwise double loop).
        let rho = (-0.045_f64).exp();
        let k_usd = (13.0_f64.powi(2) + 22.0_f64.powi(2) + 2.0 * rho * 13.0 * 22.0).sqrt();
        assert!((charge.buckets[1].k_b - k_usd).abs() <= TOL);

        // The aggregate charge: offline literal AND independent closed form.
        assert!((charge.charge - 29.909_539_608_769_19).abs() <= 1e-11);
        let delta = (k_usd.powi(2) + 16.5_f64.powi(2) + 2.0 * 0.5 * 35.0 * (-16.5)).sqrt();
        assert!((charge.charge - delta).abs() <= TOL);
    }

    #[test]
    fn worked_example_high_and_low_scenarios() {
        let all = girr_delta_charges_all(&worked_portfolio());
        // ALL order is [High, Medium, Low].
        assert_eq!(all[0].scenario, CorrelationScenario::High);
        assert_eq!(all[1].scenario, CorrelationScenario::Medium);
        assert_eq!(all[2].scenario, CorrelationScenario::Low);

        // Offline references (Python calculator).
        assert!((all[0].charge - 27.845_556_198_431_375).abs() <= 1e-11);
        assert!((all[1].charge - 29.909_539_608_769_19).abs() <= 1e-11);
        assert!((all[2].charge - 31.840_008_153_533_287).abs() <= 1e-11);

        // Here the cross-bucket term is negative (S_USD·S_EUR < 0), so a HIGHER correlation gives a
        // SMALLER charge: High < Medium < Low (max over scenarios is Low, not High).
        assert!(all[0].charge < all[1].charge);
        assert!(all[1].charge < all[2].charge);
    }

    // -----------------------------------------------------------------------------------------
    // (c) Structural properties.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn single_vertex_bucket_charge_is_abs_weighted_sensitivity() {
        // One risk factor ⇒ no cross terms ⇒ K_b = |WS| = RW * |s|.
        for &(vertex, s) in &[(0usize, 500.0f64), (6, -1500.0), (9, 2000.0)] {
            let sens = vec![GirrSensitivity::new(usd(), CurveId::OIS, vertex, s).unwrap()];
            let charge = girr_delta_charge(&sens, CorrelationScenario::Medium);
            let expected = (GIRR_DELTA_RISK_WEIGHTS[vertex] * s).abs();
            assert_eq!(charge.buckets.len(), 1);
            assert!((charge.buckets[0].k_b - expected).abs() <= TOL);
            // Single bucket ⇒ aggregate = K_b (no cross-bucket term).
            assert!((charge.charge - expected).abs() <= TOL);
        }
    }

    #[test]
    fn perfectly_correlated_same_sign_factors_sum_the_weighted_sensitivities() {
        // Two same-vertex (5y) factors on DIFFERENT curves have base rho = 0.999; under the HIGH
        // scenario min(1.25*0.999, 1) = 1.0 exactly ⇒ K_b = |WS_1 + WS_2|.
        let sens = vec![
            GirrSensitivity::new(usd(), CurveId::OIS, 5, 1000.0).unwrap(),
            GirrSensitivity::new(usd(), CurveId(1), 5, 3000.0).unwrap(),
        ];
        let charge = girr_delta_charge(&sens, CorrelationScenario::High);
        let ws1 = GIRR_DELTA_RISK_WEIGHTS[5] * 1000.0;
        let ws2 = GIRR_DELTA_RISK_WEIGHTS[5] * 3000.0;
        assert!((charge.buckets[0].k_b - (ws1 + ws2).abs()).abs() <= TOL);
    }

    #[test]
    fn zero_portfolio_is_zero_charge() {
        // No sensitivities at all.
        let empty = girr_delta_charge(&[], CorrelationScenario::Medium);
        assert!(empty.buckets.is_empty());
        assert_eq!(empty.charge, 0.0);

        // All-zero sensitivities ⇒ WS all zero ⇒ K_b and charge zero.
        let zeros = vec![
            GirrSensitivity::new(usd(), CurveId::OIS, 2, 0.0).unwrap(),
            GirrSensitivity::new(eur(), CurveId::OIS, 5, 0.0).unwrap(),
        ];
        let charge = girr_delta_charge(&zeros, CorrelationScenario::Medium);
        assert_eq!(charge.charge, 0.0);
        for b in &charge.buckets {
            assert_eq!(b.k_b, 0.0);
            assert_eq!(b.s_b, 0.0);
        }
    }

    #[test]
    fn opposite_sensitivities_net_to_zero_within_a_risk_factor() {
        // Two sensitivities to the SAME (bucket, curve, vertex) net to zero.
        let sens = vec![
            GirrSensitivity::new(usd(), CurveId::OIS, 4, 1234.0).unwrap(),
            GirrSensitivity::new(usd(), CurveId::OIS, 4, -1234.0).unwrap(),
        ];
        let charge = girr_delta_charge(&sens, CorrelationScenario::Medium);
        assert_eq!(charge.charge, 0.0);
    }

    // -----------------------------------------------------------------------------------------
    // (c') The cross-bucket S_b cap/floor fallback (MAR21.4(4)). The standard GIRR vertex
    //      correlations rarely drive the naive radicand negative for realistic books, so the
    //      safeguard branch is exercised directly on the internal aggregator with constructed
    //      bucket components that force it (K_b/S_b ratios chosen to make the radicand negative).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn cross_bucket_cap_floor_fallback_is_applied() {
        // Two anti-aligned buckets whose |S_b| exceeds K_b: naive radicand goes negative.
        let buckets = vec![
            GirrBucketCharge {
                bucket: usd(),
                k_b: 1.0,
                s_b: 1.5,
            },
            GirrBucketCharge {
                bucket: eur(),
                k_b: 1.0,
                s_b: -1.5,
            },
        ];
        let gamma = 0.5;
        // Naive radicand = 1 + 1 + 2*0.5*1.5*(-1.5) = 2 - 2.25 = -0.25 < 0 ⇒ fallback engages.
        let naive = 1.0 + 1.0 + 2.0 * gamma * 1.5 * (-1.5);
        assert!(naive < 0.0);
        // Capped: S_b -> ±1 (into [-K_b, K_b]); radicand = 1 + 1 + 2*0.5*1*(-1) = 1 ⇒ charge = 1.
        let charge = cross_bucket_charge(&buckets, gamma);
        assert!((charge - 1.0).abs() <= TOL);
    }

    // -----------------------------------------------------------------------------------------
    // Vertex mapping.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn ladder_maps_and_preserves_total_sensitivity() {
        // DV01s at on- and off-vertex tenors; 4y sits between the 3y and 5y vertices.
        let ladder = vec![
            LadderPoint {
                tenor_years: 2.0,
                dv01: 5.0,
            },
            LadderPoint {
                tenor_years: 4.0,
                dv01: 8.0,
            },
            LadderPoint {
                tenor_years: 100.0,
                dv01: 3.0,
            }, // beyond 30y ⇒ maps entirely to 30y
        ];
        let sens = map_ladder_to_vertices(usd(), CurveId::OIS, &ladder);

        // Sensitivity-preserving: sum of s_k == sum of dv01/1e-4.
        let total_in: f64 = ladder.iter().map(|p| p.dv01 / BASIS_POINT).sum();
        let total_out: f64 = sens.iter().map(GirrSensitivity::sensitivity).sum();
        assert!((total_out - total_in).abs() <= 1e-6);

        // 4y splits half/half between 3y (idx 4) and 5y (idx 5): (5-4)/(5-3)=0.5 each.
        let s_of = |v: usize| {
            sens.iter()
                .find(|s| s.vertex() == v)
                .map_or(0.0, GirrSensitivity::sensitivity)
        };
        assert!((s_of(3) - 5.0 / BASIS_POINT).abs() <= 1e-6); // 2y is exactly vertex idx 3
        assert!((s_of(4) - 0.5 * 8.0 / BASIS_POINT).abs() <= 1e-6); // 3y half of the 4y point
        assert!((s_of(5) - 0.5 * 8.0 / BASIS_POINT).abs() <= 1e-6); // 5y half of the 4y point
        assert!((s_of(9) - 3.0 / BASIS_POINT).abs() <= 1e-6); // 30y gets the 100y point entirely
    }

    #[test]
    fn from_dv01_divides_by_one_basis_point() {
        let s = GirrSensitivity::from_dv01(usd(), CurveId::OIS, 6, 0.011).unwrap();
        assert!((s.sensitivity() - 110.0).abs() <= TOL); // 0.011 / 1e-4
    }

    #[test]
    fn vertex_out_of_range_is_rejected() {
        assert_eq!(
            GirrSensitivity::new(usd(), CurveId::OIS, 10, 1.0).unwrap_err(),
            GirrError::VertexOutOfRange { index: 10 }
        );
    }
}
