//! Outlier / divergence detection across multiple sources for one
//! `(pair, tenor)` slice, reported in **vol points**.
//!
//! When several feeds quote the same slice, a robust blender must (a) measure
//! how far each source sits from the consensus, and (b) flag a source whose
//! disagreement exceeds a tolerance so a stale or mispriced feed never silently
//! poisons the fair mid. The natural disagreement metric for an FX smile is the
//! distance in **vol points** between a source's five characteristic vols
//! (`σ_ATM`, the `25Δ` and — when present — `10Δ` call/put wing vols, recovered
//! from RR/BF) and the consensus of the same five quantities.
//!
//! ## Robust consensus
//!
//! The consensus per quantity is the **median** across sources, not the mean:
//! the median has a 50% breakdown point, so a single wild source cannot drag the
//! reference toward itself the way a mean would. Each source's divergence is the
//! maximum absolute deviation (in vol points) of its five characteristic vols
//! from the per-quantity medians — an L∞ smile distance that is large whenever
//! *any* part of the smile disagrees, which is exactly the failure mode
//! (a good ATM but a stale wing) a mean-of-RMSE metric would mask.
//!
//! ## Minimum sources / the dual-feed case (explicit limitation)
//!
//! Median-consensus outlier rejection is only *meaningful* with at least three
//! sources. With **two** sources the median is their midpoint, so both sit
//! exactly equidistant from the consensus: either both clear the tolerance or
//! both breach it, and the rule cannot say *which* feed is wrong — outlier
//! rejection is **undecidable** in the common dual-feed case. With **one** source
//! there is nothing to compare against. To avoid spuriously excluding a feed (or
//! excluding both and leaving no mid), gating is therefore suppressed below
//! [`MIN_SOURCES_FOR_GATING`]: deviations are still measured and reported (so
//! operators see the disagreement magnitude), but no source is `flagged`, and the
//! report's [`DivergenceReport::gating_undecidable`] flag is set. The blend then
//! falls back to staleness-only weighting over all sources. This matches the
//! Hampel/Rousseeuw–Croux requirement that a robust scale (here the median, and
//! the MAD-scaled threshold below) needs enough points to be defined.
//!
//! ## MAD-scaled threshold
//!
//! In addition to the absolute vol-point tolerance, a source is only flagged when
//! its deviation also exceeds a **MAD-scaled** threshold
//! `k · 1.4826 · MAD` (the consistency-corrected median-absolute-deviation scale
//! estimate; `k = 3` is the customary outlier cutoff). Requiring *both* the
//! absolute floor and the data-adaptive MAD bound prevents flagging when the
//! whole panel is genuinely dispersed (wide but agreeing market) while still
//! catching a single feed that stands apart from a tight consensus.
//!
//! ## Method provenance (doc-only)
//!
//! Median-based robust consensus and absolute-deviation outlier rules are
//! standard robust statistics (Hampel; Rousseeuw & Croux MAD). Identifiers here
//! are purpose-named and vendor/research-neutral.

use celnet_surface::MarketQuotes;

use crate::normalize::{NormalizedSlice, SourceId};

/// One vol = `0.01` absolute volatility = `1.0` vol point. Divergences are
/// reported in vol points (the desk's natural unit).
const VOL_POINT: f64 = 0.01;

/// Minimum number of sources for median-consensus outlier gating to be
/// meaningful. With fewer (the dual-feed / single-feed case) the median is the
/// midpoint and outlier rejection is undecidable, so gating is suppressed and
/// [`DivergenceReport::gating_undecidable`] is set (see the module docs).
pub const MIN_SOURCES_FOR_GATING: usize = 3;

/// Consistency factor making the median-absolute-deviation a consistent
/// estimator of the standard deviation under normality (`1 / Φ⁻¹(0.75)`).
const MAD_TO_SIGMA: f64 = 1.482_602_218_505_602;

/// Customary robust outlier cutoff in MAD-scaled sigmas (a source is flagged
/// only when its deviation also exceeds `MAD_K · σ̂_MAD`).
const MAD_K: f64 = 3.0;

/// The five characteristic vols of a smile slice, in absolute vol units, used as
/// the comparison vector across sources: ATM, plus the `25Δ` and (optionally)
/// `10Δ` call/put wing vols recovered from the risk-reversal / butterfly.
///
/// The wing vols follow the broker (market) decomposition
/// `σ_call = σ_ATM + ½·RR + BF`, `σ_put = σ_ATM − ½·RR + BF`. This is the
/// arithmetic smile (not the calibrated smile) and is intentionally so: it is a
/// *comparison* coordinate that is monotone in the quoted RR/BF, so two sources
/// quoting the same ATM/RR/BF map to the same vector — the right invariant for
/// divergence, independent of any model calibration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmileVols {
    /// ATM vol.
    pub atm: f64,
    /// `25Δ` call vol.
    pub call_25: f64,
    /// `25Δ` put vol.
    pub put_25: f64,
    /// `10Δ` call vol (`NaN`-free; `None` when the source has no `10Δ` wing).
    pub call_10: Option<f64>,
    /// `10Δ` put vol.
    pub put_10: Option<f64>,
}

impl SmileVols {
    /// Recover the characteristic vols from canonical [`MarketQuotes`].
    #[must_use]
    pub fn from_quotes(q: &MarketQuotes) -> Self {
        let atm = q.atm_vol;
        let call_25 = atm + 0.5 * q.inner.risk_reversal + q.inner.butterfly;
        let put_25 = atm - 0.5 * q.inner.risk_reversal + q.inner.butterfly;
        let (call_10, put_10) = match q.outer {
            Some(o) => (
                Some(atm + 0.5 * o.risk_reversal + o.butterfly),
                Some(atm - 0.5 * o.risk_reversal + o.butterfly),
            ),
            None => (None, None),
        };
        Self {
            atm,
            call_25,
            put_25,
            call_10,
            put_10,
        }
    }
}

/// A per-source divergence entry: which source, how far from the consensus
/// smile it sits (in vol points, an L∞ distance), and whether that exceeds the
/// configured tolerance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceDivergence {
    /// The source this entry describes.
    pub source: SourceId,
    /// Maximum absolute deviation of the source's characteristic vols from the
    /// consensus, in **vol points** (`1.0` = one vol).
    pub max_deviation_vol_points: f64,
    /// Whether [`Self::max_deviation_vol_points`] exceeds the tolerance ⇒ the
    /// source is flagged as divergent / an outlier.
    pub flagged: bool,
}

/// A divergence report for one slice across all contributing sources.
///
/// Carries the robust per-quantity consensus smile and one [`SourceDivergence`]
/// per source, sorted by descending deviation so the worst disagreement is
/// first. Used both to weight/exclude sources in the blend and to emit a quality
/// signal to operators (which source disagrees, by how much, in vol points).
#[derive(Debug, Clone, PartialEq)]
pub struct DivergenceReport {
    /// The robust (median) consensus smile vols.
    pub consensus: SmileVols,
    /// Per-source divergence entries, worst first.
    pub sources: Vec<SourceDivergence>,
    /// The tolerance (vol points) above which a source is flagged.
    pub tolerance_vol_points: f64,
    /// Whether outlier gating was suppressed because there were too few sources
    /// (`< MIN_SOURCES_FOR_GATING`) for a median consensus to discriminate which
    /// feed is the outlier. When set, no source is `flagged` even if deviations
    /// are large; the blend falls back to staleness-only weighting and operators
    /// should treat the slice's consensus as undecided.
    pub gating_undecidable: bool,
}

impl DivergenceReport {
    /// The flagged (outlier) sources, if any.
    pub fn flagged(&self) -> impl Iterator<Item = &SourceDivergence> {
        self.sources.iter().filter(|s| s.flagged)
    }

    /// Whether every source agrees within tolerance.
    #[must_use]
    pub fn is_consensus(&self) -> bool {
        !self.sources.iter().any(|s| s.flagged)
    }

    /// The single worst-diverging source, if there is at least one source.
    #[must_use]
    pub fn worst(&self) -> Option<&SourceDivergence> {
        self.sources.first()
    }
}

/// Median of a non-empty slice of finite values (returns the mean of the two
/// central order statistics for an even count). The input is cloned and sorted;
/// callers pass small per-source vectors so this is cheap.
fn median(values: &[f64]) -> f64 {
    debug_assert!(!values.is_empty());
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Compute the divergence report for a set of same-slice sources at a given
/// tolerance (in vol points).
///
/// The consensus is the per-quantity median across sources. The `10Δ` wing is
/// included in the comparison only when present in *every* source (so an apples-
/// to-apples L∞ distance is taken over the common quantities); when sources
/// disagree on whether a `10Δ` wing exists, the comparison falls back to the
/// `{ATM, 25Δ}` triple, which all five-/three-point slices share.
///
/// # Panics
///
/// Panics if `slices` is empty — divergence is undefined with no sources.
#[must_use]
pub fn divergence_report(
    slices: &[NormalizedSlice],
    tolerance_vol_points: f64,
) -> DivergenceReport {
    assert!(
        !slices.is_empty(),
        "divergence requires at least one source"
    );

    let vols: Vec<SmileVols> = slices
        .iter()
        .map(|s| SmileVols::from_quotes(&s.quotes))
        .collect();

    let include_10 = vols
        .iter()
        .all(|v| v.call_10.is_some() && v.put_10.is_some());

    // Per-quantity median consensus.
    let atm = median(&vols.iter().map(|v| v.atm).collect::<Vec<_>>());
    let call_25 = median(&vols.iter().map(|v| v.call_25).collect::<Vec<_>>());
    let put_25 = median(&vols.iter().map(|v| v.put_25).collect::<Vec<_>>());
    let (call_10, put_10) = if include_10 {
        (
            Some(median(
                &vols.iter().map(|v| v.call_10.unwrap()).collect::<Vec<_>>(),
            )),
            Some(median(
                &vols.iter().map(|v| v.put_10.unwrap()).collect::<Vec<_>>(),
            )),
        )
    } else {
        (None, None)
    };

    let consensus = SmileVols {
        atm,
        call_25,
        put_25,
        call_10,
        put_10,
    };

    // Gating is only meaningful with enough sources for a median to discriminate
    // the outlier; below that, measure but never flag (dual-/single-feed case).
    let gating_undecidable = slices.len() < MIN_SOURCES_FOR_GATING;

    // MAD-scaled threshold: the consistency-corrected median absolute deviation
    // of each quantity, reduced over the smile to a single L∞-comparable scale,
    // so a feed is flagged only when it stands apart from a *tight* consensus and
    // not merely because the whole panel is genuinely dispersed.
    let abs_dev =
        |xs: &[f64], center: f64| -> Vec<f64> { xs.iter().map(|x| (x - center).abs()).collect() };
    let mut mad = median(&abs_dev(
        &vols.iter().map(|v| v.atm).collect::<Vec<_>>(),
        atm,
    ))
    .max(median(&abs_dev(
        &vols.iter().map(|v| v.call_25).collect::<Vec<_>>(),
        call_25,
    )))
    .max(median(&abs_dev(
        &vols.iter().map(|v| v.put_25).collect::<Vec<_>>(),
        put_25,
    )));
    if include_10 {
        mad = mad
            .max(median(&abs_dev(
                &vols.iter().map(|v| v.call_10.unwrap()).collect::<Vec<_>>(),
                call_10.unwrap(),
            )))
            .max(median(&abs_dev(
                &vols.iter().map(|v| v.put_10.unwrap()).collect::<Vec<_>>(),
                put_10.unwrap(),
            )));
    }
    // MAD-scaled sigma cutoff, in vol points. A zero MAD (tight panel) means the
    // absolute vol-point tolerance is the sole gate, which is the intended floor.
    let mad_threshold_vol_points = MAD_K * MAD_TO_SIGMA * mad / VOL_POINT;

    let mut sources: Vec<SourceDivergence> = slices
        .iter()
        .zip(&vols)
        .map(|(slice, v)| {
            let mut dev = (v.atm - atm)
                .abs()
                .max((v.call_25 - call_25).abs())
                .max((v.put_25 - put_25).abs());
            if include_10 {
                dev = dev
                    .max((v.call_10.unwrap() - call_10.unwrap()).abs())
                    .max((v.put_10.unwrap() - put_10.unwrap()).abs());
            }
            let max_deviation_vol_points = dev / VOL_POINT;
            // Flag only when gating is decidable AND the deviation breaches both
            // the absolute vol-point tolerance and the data-adaptive MAD bound.
            let flagged = !gating_undecidable
                && max_deviation_vol_points > tolerance_vol_points
                && max_deviation_vol_points > mad_threshold_vol_points;
            SourceDivergence {
                source: slice.source,
                max_deviation_vol_points,
                flagged,
            }
        })
        .collect();

    // Worst disagreement first.
    sources.sort_by(|a, b| {
        b.max_deviation_vol_points
            .partial_cmp(&a.max_deviation_vol_points)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    DivergenceReport {
        consensus,
        sources,
        tolerance_vol_points,
        gating_undecidable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::{
        VendorSmileMessage, WireAtmConvention, WireConventions, WireDeltaConvention, WireForward,
        WireWing,
    };
    use celnet_core::assert_close;

    fn slice(source: &str, atm_pct: f64, rr_pct: f64, bf_pct: f64) -> NormalizedSlice {
        let msg = VendorSmileMessage {
            pair: "EURUSD".into(),
            tenor_label: "1Y".into(),
            spot: 1.10,
            forward: WireForward::Points {
                points: 110.0,
                pip_factor: 10_000.0,
            },
            atm_vol_pct: atm_pct,
            inner: WireWing {
                delta_pct: 25.0,
                risk_reversal_pct: rr_pct,
                butterfly_pct: bf_pct,
            },
            outer: None,
            ndf_fixing: None,
            conventions: WireConventions {
                delta: WireDeltaConvention::SpotPremiumAdjusted,
                atm: WireAtmConvention::DeltaNeutralStraddle,
                premium_in_foreign: true,
            },
            source: source.into(),
            observed_at_nanos: 0,
        };
        crate::normalize::normalize(&msg, 0.02).unwrap()
    }

    #[test]
    fn smile_vols_recovers_wings() {
        let s = slice("a", 10.0, 1.0, 0.5); // atm 0.10, rr 0.01, bf 0.005
        let v = SmileVols::from_quotes(&s.quotes);
        assert_close!(v.atm, 0.10);
        assert_close!(v.call_25, 0.10 + 0.5 * 0.01 + 0.005);
        assert_close!(v.put_25, 0.10 - 0.5 * 0.01 + 0.005);
    }

    #[test]
    fn agreeing_sources_are_consensus() {
        let s = [
            slice("a", 10.0, 1.0, 0.5),
            slice("b", 10.02, 1.0, 0.5),
            slice("c", 9.98, 1.0, 0.5),
        ];
        let rep = divergence_report(&s, 0.10); // 0.10 vol-point tolerance
        assert!(rep.is_consensus(), "all within 0.10 vol: {rep:?}");
        // Consensus ATM is the median (the middle of 9.98/10.0/10.02).
        assert_close!(rep.consensus.atm, 0.10);
    }

    #[test]
    fn divergent_source_is_flagged_with_magnitude() {
        let s = [
            slice("good-a", 10.0, 1.0, 0.5),
            slice("good-b", 10.0, 1.0, 0.5),
            // 'bad' is 0.8 vol points high on ATM (and the wings shift with it).
            slice("bad", 10.8, 1.0, 0.5),
        ];
        let rep = divergence_report(&s, 0.25);
        let worst = rep.worst().unwrap();
        assert_eq!(worst.source.as_str(), "bad");
        assert!(worst.flagged, "0.8-vol outlier must be flagged");
        // Median consensus is unmoved by the single outlier (still 10.0).
        assert_close!(rep.consensus.atm, 0.10);
        // Deviation reported in vol points ~0.8.
        assert_close!(worst.max_deviation_vol_points, 0.8, 1e-9, 1e-9);
        // The two good sources are not flagged.
        assert_eq!(rep.flagged().count(), 1);
        // With three sources gating is decidable.
        assert!(!rep.gating_undecidable);
    }

    #[test]
    fn dual_feed_gating_is_undecidable_and_flags_nothing() {
        // Two feeds 4 vol points apart: a mean/median would sit halfway, leaving
        // both equidistant and neither identifiable as the outlier. Gating must
        // be suppressed (no flag) and the undecidable flag set, so the blend
        // falls back to staleness-only weighting rather than excluding a feed at
        // random or excluding both and leaving no mid.
        let s = [slice("a", 10.0, 1.0, 0.5), slice("b", 14.0, 1.0, 0.5)];
        let rep = divergence_report(&s, 0.5); // tight tolerance
        assert!(rep.gating_undecidable, "two sources ⇒ undecidable");
        assert_eq!(rep.flagged().count(), 0, "no source may be flagged");
        // Deviations are still measured (each is 2 vol points from the midpoint).
        for d in &rep.sources {
            assert_close!(d.max_deviation_vol_points, 2.0, 1e-9, 1e-9);
        }
    }

    #[test]
    fn single_feed_is_never_flagged() {
        let s = [slice("solo", 10.0, 1.0, 0.5)];
        let rep = divergence_report(&s, 0.0); // even a zero tolerance
        assert!(rep.gating_undecidable);
        assert_eq!(rep.flagged().count(), 0);
    }

    #[test]
    fn wide_but_agreeing_panel_is_not_flagged_by_mad_bound() {
        // A genuinely dispersed panel (no single outlier): the MAD-scaled bound
        // keeps every source un-flagged even though absolute deviations exceed
        // the vol-point tolerance, because no feed stands apart from the others.
        let s = [
            slice("a", 10.0, 1.0, 0.5),
            slice("b", 10.6, 1.0, 0.5),
            slice("c", 11.2, 1.0, 0.5),
            slice("d", 11.8, 1.0, 0.5),
            slice("e", 12.4, 1.0, 0.5),
        ];
        // Absolute tolerance 0.5 vol pts is breached by the extremes, but the MAD
        // is large (spread panel) so the data-adaptive bound is not breached.
        let rep = divergence_report(&s, 0.5);
        assert!(!rep.gating_undecidable);
        assert_eq!(
            rep.flagged().count(),
            0,
            "uniformly-spread panel has no outlier: {rep:?}"
        );
    }
}
