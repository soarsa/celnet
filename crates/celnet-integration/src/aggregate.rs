//! Multi-source aggregation — blend N normalized sources for one `(pair, tenor)`
//! into a single fair mid, with **time-weighted staleness decay** and
//! **divergence-aware** outlier handling, emitting a quality report.
//!
//! ## What the blender does
//!
//! Given several [`NormalizedSlice`]s for the *same* `(pair, tenor)` (each from a
//! different feed, observed at a different instant), it produces one canonical
//! blended slice whose quotes are a weighted combination of the sources. Two
//! independent weightings combine multiplicatively:
//!
//! 1. **Staleness decay** — a source observed `Δt` seconds before the valuation
//!    instant is down-weighted by an exponential `e^{−Δt/τ}` with a configurable
//!    half-life. A feed that has gone quiet contributes geometrically less the
//!    older its last tick, so a fresh source dominates a stale one without a hard
//!    cut-off (`docs/CELNET-INTEGRATION.md` §1.2: WS feeds disconnect; the blend
//!    must degrade gracefully).
//! 2. **Divergence gating** — a source flagged by [`crate::divergence`] as an
//!    outlier (its smile sits more than the tolerance, in vol points, from the
//!    robust median consensus) is **excluded** from the mid so a stale or
//!    mispriced wing never poisons the fair value; it still appears in the
//!    quality report so operators see *which* source disagreed and by how much.
//!
//! The blended quote vector (ATM, RR/BF at each pillar) is a weighted mean over
//! the surviving sources; because the canonical [`MarketQuotes`] are linear in
//! ATM/RR/BF, blending the quotes is equivalent to blending the implied smile
//! vols, so the result is itself a well-formed broker quote set that surface
//! construction consumes unchanged.
//!
//! ## Method provenance (doc-only)
//!
//! Exponential time-decay weighting is the standard recency kernel; combined
//! with robust (median-consensus) outlier gating it yields a fault-tolerant
//! consolidated mid. Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::math::exp;
use celnet_surface::{MarketContext, MarketQuotes, RiskReversalButterfly};

use crate::divergence::{DivergenceReport, divergence_report};
use crate::normalize::{NormalizedSlice, SourceId};

/// Configuration for a blend: the valuation instant, the staleness half-life,
/// and the divergence tolerance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendConfig {
    /// Valuation instant in epoch nanoseconds. A source's staleness is
    /// `valuation − observed`; sources observed *after* this instant are treated
    /// as zero-age (no future-dating penalty).
    pub valuation_nanos: i64,
    /// Staleness half-life in seconds: a source `half_life` seconds old gets
    /// exactly half the weight of a just-observed source. Must be positive.
    pub staleness_half_life_secs: f64,
    /// Divergence tolerance in **vol points**: a source whose smile sits more
    /// than this from the robust consensus is excluded from the mid.
    pub divergence_tolerance_vol_points: f64,
}

impl BlendConfig {
    /// A sensible default: 30-second half-life, 1.0-vol-point divergence gate.
    #[must_use]
    pub fn new(valuation_nanos: i64) -> Self {
        Self {
            valuation_nanos,
            staleness_half_life_secs: 30.0,
            divergence_tolerance_vol_points: 1.0,
        }
    }

    /// Exponential staleness weight for a source observed at `observed_nanos`:
    /// `2^{−Δt/half_life}` with `Δt = max(0, valuation − observed)` seconds.
    #[must_use]
    pub fn staleness_weight(&self, observed_nanos: i64) -> f64 {
        let age_nanos = (self.valuation_nanos - observed_nanos).max(0);
        let age_secs = age_nanos as f64 * 1e-9;
        // 2^{−age/half} = e^{−ln2·age/half}.
        exp(-core::f64::consts::LN_2 * age_secs / self.staleness_half_life_secs)
    }
}

/// The per-source weighting that produced the blend, for the quality report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceWeight {
    /// The source.
    pub source: SourceId,
    /// Age of the source in seconds at the valuation instant.
    pub age_secs: f64,
    /// Staleness decay factor applied (`∈ (0, 1]`).
    pub staleness_weight: f64,
    /// Whether the source was excluded from the mid by the divergence gate.
    pub excluded: bool,
    /// Final normalized weight in the blend (`0` for excluded sources; the
    /// surviving weights sum to `1`).
    pub final_weight: f64,
}

/// The full quality report accompanying a blend: per-source weights, the
/// divergence report, and a count of contributing sources.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendQuality {
    /// Per-source weighting (staleness, exclusion, final weight).
    pub weights: Vec<SourceWeight>,
    /// The divergence report (consensus smile + per-source deviation in vol pts).
    pub divergence: DivergenceReport,
    /// Number of sources that actually contributed to the mid (not excluded).
    pub contributing: usize,
}

impl BlendQuality {
    /// Whether the blend used every input source (none excluded).
    #[must_use]
    pub fn all_contributed(&self) -> bool {
        self.weights.iter().all(|w| !w.excluded)
    }
}

/// The result of blending: the consolidated canonical surface input plus its
/// quality report.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendedSlice {
    /// The blended canonical broker quotes (the fair mid smile).
    pub quotes: MarketQuotes,
    /// The market context for the slice (state shared across sources; the spot
    /// and rates are themselves a staleness-weighted blend over the sources).
    pub context: MarketContext,
    /// The quality / divergence report for this blend.
    pub quality: BlendQuality,
}

/// Errors from blending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlendError {
    /// No sources were supplied.
    NoSources,
    /// The sources did not all describe the same `(pair, tenor)`.
    MixedSlices,
    /// Every source was excluded by the divergence gate, leaving no mid.
    AllExcluded,
    /// The configuration was invalid (e.g. a non-positive half-life).
    BadConfig,
}

impl core::fmt::Display for BlendError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BlendError::NoSources => write!(f, "no sources supplied to blend"),
            BlendError::MixedSlices => write!(f, "sources describe different (pair, tenor) slices"),
            BlendError::AllExcluded => write!(f, "all sources excluded by divergence gate"),
            BlendError::BadConfig => write!(f, "invalid blend configuration"),
        }
    }
}

impl std::error::Error for BlendError {}

/// Blend N same-slice sources into one fair-mid canonical surface input.
///
/// All `slices` must share the same `(pair, tenor)`. Each source is weighted by
/// exponential staleness decay (per `cfg`) and gated by divergence: a source
/// flagged as an outlier (smile beyond the vol-point tolerance from the robust
/// consensus) is excluded from the mid but reported. The blended quotes, market
/// context, and quality report are returned.
///
/// # Errors
///
/// Returns [`BlendError::NoSources`] if `slices` is empty,
/// [`BlendError::MixedSlices`] if the sources are not the same `(pair, tenor)`,
/// [`BlendError::BadConfig`] for a non-positive half-life, and
/// [`BlendError::AllExcluded`] if the divergence gate removes every source.
pub fn blend(slices: &[NormalizedSlice], cfg: BlendConfig) -> Result<BlendedSlice, BlendError> {
    if slices.is_empty() {
        return Err(BlendError::NoSources);
    }
    if !(cfg.staleness_half_life_secs.is_finite() && cfg.staleness_half_life_secs > 0.0) {
        return Err(BlendError::BadConfig);
    }
    let first = &slices[0];
    if slices
        .iter()
        .any(|s| s.pair != first.pair || s.tenor != first.tenor)
    {
        return Err(BlendError::MixedSlices);
    }

    let divergence = divergence_report(slices, cfg.divergence_tolerance_vol_points);

    // Map source → flagged, via the divergence report.
    let flagged = |source: SourceId| {
        divergence
            .sources
            .iter()
            .find(|d| d.source == source)
            .is_some_and(|d| d.flagged)
    };

    // Staleness weight × inclusion for each source.
    let mut raw: Vec<(usize, f64, bool, f64)> = Vec::with_capacity(slices.len());
    let mut surviving_total = 0.0;
    for (i, s) in slices.iter().enumerate() {
        let age_secs = ((cfg.valuation_nanos - s.observed_at_nanos).max(0)) as f64 * 1e-9;
        let stale_w = cfg.staleness_weight(s.observed_at_nanos);
        let excluded = flagged(s.source);
        let contributed = if excluded { 0.0 } else { stale_w };
        surviving_total += contributed;
        raw.push((i, stale_w, excluded, age_secs));
    }

    if surviving_total <= 0.0 {
        return Err(BlendError::AllExcluded);
    }

    // Normalized final weights (surviving sources sum to 1).
    let weights: Vec<SourceWeight> = raw
        .iter()
        .map(|&(i, stale_w, excluded, age_secs)| {
            let final_weight = if excluded {
                0.0
            } else {
                stale_w / surviving_total
            };
            SourceWeight {
                source: slices[i].source,
                age_secs,
                staleness_weight: stale_w,
                excluded,
                final_weight,
            }
        })
        .collect();

    // Weighted blend of the linear quote vector and the market state.
    let mut atm = 0.0;
    let mut rr25 = 0.0;
    let mut bf25 = 0.0;
    let mut spot = 0.0;
    let mut r_dom = 0.0;
    let mut r_for = 0.0;
    let mut t = 0.0;
    // Outer (10Δ) wing: blended only over the surviving sources that carry it,
    // re-normalized to those sources' weights so a missing wing does not bias it.
    let mut outer_acc: Option<(f64, f64, f64)> = None; // (rr10, bf10, weight)

    for (w, s) in weights.iter().zip(slices) {
        // Skip divergence-excluded sources by their semantic flag rather than a
        // float-equality test on the weight (an excluded source's final_weight
        // is the exact sentinel 0.0, but the flag is the source of truth).
        if w.excluded {
            continue;
        }
        let fw = w.final_weight;
        let q = &s.quotes;
        atm += fw * q.atm_vol;
        rr25 += fw * q.inner.risk_reversal;
        bf25 += fw * q.inner.butterfly;
        spot += fw * s.context.spot;
        // FX carry rates read verbatim through the seam accessors (`yield_rate`
        // returns the STORED `r_for` — never a `r_dom − b` reconstruction), so
        // the blend over `Carry::FxRates` slices is byte-identical to the
        // former two-rate field reads.
        r_dom += fw * s.context.carry.discount_rate();
        r_for += fw * s.context.carry.yield_rate();
        t += fw * s.context.t;
        if let Some(o) = q.outer {
            let (rr, bf, wsum) = outer_acc.get_or_insert((0.0, 0.0, 0.0));
            *rr += fw * o.risk_reversal;
            *bf += fw * o.butterfly;
            *wsum += fw;
        }
    }

    let quotes = match outer_acc {
        Some((rr10, bf10, wsum)) if wsum > 0.0 => MarketQuotes {
            atm_vol: atm,
            inner: RiskReversalButterfly {
                pillar: first.quotes.inner.pillar,
                risk_reversal: rr25,
                butterfly: bf25,
            },
            outer: Some(RiskReversalButterfly {
                // Re-normalize the outer wing to the sources that carried it.
                pillar: celnet_surface::DeltaPillar::TEN,
                risk_reversal: rr10 / wsum,
                butterfly: bf10 / wsum,
            }),
        },
        _ => MarketQuotes::three_point(atm, rr25, bf25),
    };

    // The blended context reuses the resolved conventions (identical across
    // same-slice sources) and the staleness-weighted market state.
    let context = MarketContext::new(
        spot,
        celnet_types::Carry::FxRates { r_dom, r_for },
        t,
        first.context.conventions,
    );

    let contributing = weights.iter().filter(|w| !w.excluded).count();

    Ok(BlendedSlice {
        quotes,
        context,
        quality: BlendQuality {
            weights,
            divergence,
            contributing,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::{
        VendorSmileMessage, WireAtmConvention, WireConventions, WireDeltaConvention, WireForward,
        WireWing,
    };
    use celnet_core::{assert_close, is_close};

    const ONE_SEC: i64 = 1_000_000_000;

    fn slice(source: &str, atm_pct: f64, observed_nanos: i64) -> NormalizedSlice {
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
                risk_reversal_pct: 0.45,
                butterfly_pct: 0.20,
            },
            outer: None,
            ndf_fixing: None,
            conventions: WireConventions {
                delta: WireDeltaConvention::SpotPremiumAdjusted,
                atm: WireAtmConvention::DeltaNeutralStraddle,
                premium_in_foreign: true,
            },
            source: source.into(),
            observed_at_nanos: observed_nanos,
        };
        crate::normalize::normalize(&msg, 0.02).unwrap()
    }

    #[test]
    fn fresh_sources_equal_weight_blend_is_the_mean() {
        let now = 1_000 * ONE_SEC;
        let s = [slice("a", 10.0, now), slice("b", 10.4, now)];
        let cfg = BlendConfig::new(now);
        let out = blend(&s, cfg).unwrap();
        // Equal age ⇒ equal weight ⇒ blended ATM is the arithmetic mean.
        assert_close!(out.quotes.atm_vol, 0.102);
        assert!(out.quality.all_contributed());
        for w in &out.quality.weights {
            assert_close!(w.final_weight, 0.5);
        }
    }

    #[test]
    fn staleness_decay_weights_old_source_down() {
        let now = 1_000 * ONE_SEC;
        // 'fresh' is current; 'stale' is two half-lives old (×0.25 weight).
        let cfg = BlendConfig {
            valuation_nanos: now,
            staleness_half_life_secs: 10.0,
            divergence_tolerance_vol_points: 5.0, // wide gate: no exclusion here
        };
        let fresh = slice("fresh", 10.0, now);
        let stale = slice("stale", 12.0, now - 20 * ONE_SEC);
        let out = blend(&[fresh, stale], cfg).unwrap();

        // Two half-lives → stale weight ≈ 0.25 of fresh → normalized 0.2 / 0.8.
        let wf = out
            .quality
            .weights
            .iter()
            .find(|w| w.source.as_str() == "fresh")
            .unwrap();
        let ws = out
            .quality
            .weights
            .iter()
            .find(|w| w.source.as_str() == "stale")
            .unwrap();
        assert!(is_close(ws.staleness_weight, 0.25, 1e-9, 1e-9));
        assert!(is_close(wf.final_weight, 0.8, 1e-9, 1e-9));
        assert!(is_close(ws.final_weight, 0.2, 1e-9, 1e-9));
        // Blended ATM is pulled toward the fresh source: 0.8·10 + 0.2·12 = 10.4.
        assert_close!(out.quotes.atm_vol, 0.104);
        // The fresh source carries strictly more weight than the stale one.
        assert!(wf.final_weight > ws.final_weight);
    }

    #[test]
    fn divergent_source_excluded_and_mid_within_remaining_spread() {
        let now = 1_000 * ONE_SEC;
        let cfg = BlendConfig {
            valuation_nanos: now,
            staleness_half_life_secs: 30.0,
            divergence_tolerance_vol_points: 0.5,
        };
        let s = [
            slice("good-a", 10.0, now),
            slice("good-b", 10.2, now),
            slice("rogue", 13.0, now), // 3 vol points high → excluded
        ];
        let out = blend(&s, cfg).unwrap();

        let rogue = out
            .quality
            .weights
            .iter()
            .find(|w| w.source.as_str() == "rogue")
            .unwrap();
        assert!(rogue.excluded, "rogue must be gated out");
        assert_close!(rogue.final_weight, 0.0);
        assert_eq!(out.quality.contributing, 2);

        // The blended mid lies strictly within the surviving sources' spread.
        let lo = 0.10_f64.min(0.102);
        let hi = 0.10_f64.max(0.102);
        assert!(
            out.quotes.atm_vol > lo - 1e-12 && out.quotes.atm_vol < hi + 1e-12,
            "mid {} must lie within [{lo}, {hi}]",
            out.quotes.atm_vol
        );
        // And it equals the mean of the two good sources (equal fresh weight).
        assert_close!(out.quotes.atm_vol, 0.101);
        // The divergence report names the rogue source as worst.
        assert_eq!(
            out.quality.divergence.worst().unwrap().source.as_str(),
            "rogue"
        );
    }

    #[test]
    fn mixed_slices_and_empty_are_errors() {
        let now = 0;
        assert_eq!(
            blend(&[], BlendConfig::new(now)).unwrap_err(),
            BlendError::NoSources
        );

        let a = slice("a", 10.0, now);
        let mut b = slice("b", 10.0, now);
        b.tenor = celnet_types::Tenor::Months(3);
        assert_eq!(
            blend(&[a, b], BlendConfig::new(now)).unwrap_err(),
            BlendError::MixedSlices
        );
    }

    #[test]
    fn blended_mid_is_within_source_spread_for_many_sources() {
        let now = 1_000 * ONE_SEC;
        let cfg = BlendConfig {
            valuation_nanos: now,
            staleness_half_life_secs: 30.0,
            divergence_tolerance_vol_points: 5.0,
        };
        let atms = [9.8, 10.0, 10.1, 10.3, 9.9];
        let slices: Vec<_> = atms
            .iter()
            .enumerate()
            .map(|(i, &a)| slice(&format!("s{i}"), a, now))
            .collect();
        let out = blend(&slices, cfg).unwrap();
        let lo = atms.iter().cloned().fold(f64::INFINITY, f64::min) * 0.01;
        let hi = atms.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * 0.01;
        assert!(out.quotes.atm_vol >= lo && out.quotes.atm_vol <= hi);
    }
}
