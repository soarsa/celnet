//! Celnet market-data integration — bring external vendor FX-options feeds into
//! the canonical Celnet surface input, and blend multiple sources into one fair,
//! divergence-checked mid (work-stream WS-H, `docs/CELNET-INTEGRATION.md` §1).
//!
//! # The ingest → normalize → blend → canonical-surface-input pipeline
//!
//! External FX-options feeds publish a *delta-space* smile per `(pair, tenor)` —
//! an ATM vol plus `25Δ` (and, for liquid pairs, `10Δ`) risk-reversal and
//! butterfly wings — together with spot, forward points (or an outright), and,
//! for non-deliverable pairs, an NDF fixing, each in the feed's own units and
//! convention (the FMD FXO 2.0-style shapes of `docs/CELNET-INTEGRATION.md` §1.1;
//! the feed carries no published convention spec, open question 5). This crate
//! turns those messages into the canonical surface input that
//! [`celnet_surface`] construction consumes:
//!
//! 1. **Ingest** ([`vendor`]) — decode the feed body into a neutral
//!    [`vendor::VendorSmileMessage`] (percent vols, forward points, a
//!    self-declared convention descriptor). Pure wire shapes, no numerics.
//! 2. **Normalize** ([`normalize`]) — map a message onto canonical
//!    [`celnet_surface::MarketQuotes`] + [`celnet_surface::MarketContext`] via
//!    the [`celnet_conventions`] layer: percent→absolute, points→outright,
//!    tenor/pair parsing, and a **convention cross-check** (the feed's declared
//!    delta convention must match the canonical one Celnet resolves for the
//!    `(pair, tenor)`, else error — convention error dwarfs model error).
//! 3. **Blend** ([`aggregate`]) — combine N normalized sources for the same
//!    `(pair, tenor)` into one fair mid with **time-weighted staleness decay**
//!    and **divergence-aware** outlier exclusion ([`divergence`]), emitting a
//!    quality report (which source disagrees, by how much, in vol points).
//! 4. **Canonical surface input** — the blended slice *is* a well-formed broker
//!    quote set + market context; [`pipeline`] runs the whole chain and hands it
//!    to [`celnet_surface`] (e.g. [`celnet_surface::build_smile`]) unchanged.
//!
//! # Design constraints honoured
//!
//! * **Vendor-neutral public types.** The wire model is a *data-shape adapter*
//!   named for its purpose (a vendor option-smile message); no vendor,
//!   competitor or product name appears in any identifier (the feed *format* is
//!   referenced only in docs).
//! * **Convention-correct ingest.** Every delta/premium/cut choice goes through
//!   the resolved [`celnet_conventions::ConventionRecord`]; mislabelled feeds are
//!   rejected, not silently mispriced.
//! * **Fault tolerance.** Staleness decay degrades a quiet feed gracefully and
//!   the divergence gate stops a single mispriced source from poisoning the mid —
//!   matching the WS-disconnect / skip-while-full realities of the estate
//!   (`docs/CELNET-INTEGRATION.md` §0, §1.2).
//! * **Determinism & scale.** All transcendentals route through
//!   [`celnet_core::math`]; normalized slices are `Copy` POD so the hot path is
//!   allocation-light and the blender is a single linear pass over sources.
//!
//! # Method provenance (doc-only)
//!
//! Exponential recency weighting (time-decay kernel) and median-consensus robust
//! outlier gating (Hampel; Rousseeuw & Croux MAD) are standard. Identifiers stay
//! purpose-named and vendor/research-neutral.

#![forbid(unsafe_code)]

pub mod aggregate;
pub mod deployment;
pub mod divergence;
pub mod egress;
pub mod normalize;
pub mod subscriber;
pub mod vendor;

pub use aggregate::{BlendConfig, BlendError, BlendQuality, BlendedSlice, SourceWeight, blend};
pub use deployment::{
    DeploymentMode, Edge, EdgeBuilder, StandaloneSink, StandaloneSinkError, StandaloneSource,
    StandaloneTransport, frame_message,
};
pub use divergence::{DivergenceReport, SmileVols, SourceDivergence, divergence_report};
pub use egress::{
    DistributorEgress, EgressConfig, EgressError, EgressGovernor, EgressMetrics, MonotonicClock,
    NanoClock, PriceKey, PriceSink, PriceUpdate,
};
pub use normalize::{NormalizeError, NormalizedSlice, SourceId, normalize};
pub use subscriber::{
    FeedFrame, FeedTransport, MarketDataSource, ResilientSubscriber, SubscriberStats,
    SubscriptionKey, TickConsumer, subscription_set,
};
pub use vendor::{
    VendorSmileMessage, WireAtmConvention, WireConventions, WireDeltaConvention, WireForward,
    WireWing,
};

/// The end-to-end ingest→normalize→blend pipeline for one `(pair, tenor)`.
///
/// Decodes each JSON feed body into a [`VendorSmileMessage`], normalizes it to a
/// canonical [`NormalizedSlice`] under the resolved conventions (using `r_dom`,
/// the domestic discount rate from the curve layer, to imply the foreign rate so
/// the feed forward is reproduced exactly), then blends the sources into one
/// fair mid per `cfg`. The returned [`BlendedSlice`] is the canonical surface
/// input that [`celnet_surface::build_smile`] consumes directly.
///
/// # Errors
///
/// Returns [`PipelineError`] on a malformed feed body, a normalization failure
/// (bad pair/tenor/value or convention mismatch), or a blend failure
/// (no/mixed/all-excluded sources, bad config).
pub fn pipeline(
    bodies: &[&str],
    r_dom: f64,
    cfg: BlendConfig,
) -> Result<BlendedSlice, PipelineError> {
    let mut slices = Vec::with_capacity(bodies.len());
    for body in bodies {
        let msg = VendorSmileMessage::from_json(body).map_err(PipelineError::Decode)?;
        slices.push(normalize(&msg, r_dom).map_err(PipelineError::Normalize)?);
    }
    blend(&slices, cfg).map_err(PipelineError::Blend)
}

/// As [`pipeline`], but over already-decoded messages (the server lane decodes
/// off the socket and hands messages in). Normalizes and blends.
///
/// # Errors
///
/// Returns [`PipelineError::Normalize`] or [`PipelineError::Blend`].
pub fn pipeline_messages(
    messages: &[VendorSmileMessage],
    r_dom: f64,
    cfg: BlendConfig,
) -> Result<BlendedSlice, PipelineError> {
    let mut slices = Vec::with_capacity(messages.len());
    for msg in messages {
        slices.push(normalize(msg, r_dom).map_err(PipelineError::Normalize)?);
    }
    blend(&slices, cfg).map_err(PipelineError::Blend)
}

/// A failure anywhere along the ingest→normalize→blend pipeline.
#[derive(Debug)]
pub enum PipelineError {
    /// A feed body could not be decoded into a [`VendorSmileMessage`].
    Decode(serde_json::Error),
    /// A message could not be normalized to a canonical slice.
    Normalize(NormalizeError),
    /// The normalized sources could not be blended.
    Blend(BlendError),
}

impl core::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PipelineError::Decode(e) => write!(f, "feed decode error: {e}"),
            PipelineError::Normalize(e) => write!(f, "normalize error: {e}"),
            PipelineError::Blend(e) => write!(f, "blend error: {e}"),
        }
    }
}

impl std::error::Error for PipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PipelineError::Decode(e) => Some(e),
            PipelineError::Normalize(e) => Some(e),
            PipelineError::Blend(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::{Smile, is_close};
    use celnet_surface::build_smile;

    /// A representative EURUSD 1Y vendor message.
    fn eurusd_1y(source: &str, atm_pct: f64, observed_nanos: i64) -> VendorSmileMessage {
        VendorSmileMessage {
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
            outer: Some(WireWing {
                delta_pct: 10.0,
                risk_reversal_pct: 0.80,
                butterfly_pct: 0.55,
            }),
            ndf_fixing: None,
            conventions: WireConventions {
                delta: WireDeltaConvention::SpotPremiumAdjusted,
                atm: WireAtmConvention::DeltaNeutralStraddle,
                premium_in_foreign: true,
            },
            source: source.into(),
            observed_at_nanos: observed_nanos,
        }
    }

    /// Round-trip: normalize a single representative vendor quote set, rebuild the
    /// surface, and confirm the constructed smile reprices the *same* ATM and
    /// 25Δ wings the vendor quoted (the canonical surface input is faithful).
    #[test]
    fn round_trip_rebuilds_the_same_surface() {
        let msg = eurusd_1y("feed-a", 10.5, 0);
        let slice = normalize(&msg, 0.02).unwrap();

        // Build the surface from the normalized canonical input.
        let smile = build_smile(&slice.context, &slice.quotes).unwrap();

        // The ATM vol is repriced exactly.
        let f = slice.context.forward();
        let k_atm = slice.context.atm_strike(slice.quotes.atm_vol);
        assert!(is_close(
            smile.implied_vol(k_atm, f, slice.context.t).0,
            slice.quotes.atm_vol,
            1e-9,
            1e-11
        ));

        // The 25Δ wing risk-reversal the vendor quoted is reproduced by the
        // calibrated smile (RR_25 = σ_call − σ_put).
        let cal = celnet_surface::calibrate_pillar(
            &slice.context,
            slice.quotes.atm_vol,
            slice.quotes.inner,
        )
        .unwrap();
        assert!(is_close(
            cal.call_vol - cal.put_vol,
            slice.quotes.inner.risk_reversal,
            1e-11,
            1e-12
        ));

        // And re-encoding the normalized quotes back to wire units recovers the
        // vendor's percentage inputs (lossless normalize round-trip).
        assert!(is_close(
            slice.quotes.atm_vol * 100.0,
            msg.atm_vol_pct,
            1e-12,
            1e-12
        ));
        assert!(is_close(
            slice.quotes.inner.risk_reversal * 100.0,
            msg.inner.risk_reversal_pct,
            1e-12,
            1e-12
        ));
    }

    /// End-to-end: two JSON feed bodies → blended canonical input → surface.
    #[test]
    fn pipeline_blends_two_feeds_into_a_buildable_surface() {
        let a = eurusd_1y("feed-a", 10.4, 0).to_json().unwrap();
        let b = eurusd_1y("feed-b", 10.6, 0).to_json().unwrap();
        let cfg = BlendConfig::new(0);
        let blended = pipeline(&[&a, &b], 0.02, cfg).unwrap();

        // Equal-age, agreeing feeds ⇒ mid is the mean ⇒ 10.5 vol.
        assert!(is_close(blended.quotes.atm_vol, 0.105, 1e-12, 1e-12));
        assert!(blended.quality.all_contributed());

        // The blended canonical input builds a surface that reprices its own ATM.
        let smile = build_smile(&blended.context, &blended.quotes).unwrap();
        let f = blended.context.forward();
        let k_atm = blended.context.atm_strike(blended.quotes.atm_vol);
        assert!(is_close(
            smile.implied_vol(k_atm, f, blended.context.t).0,
            blended.quotes.atm_vol,
            1e-9,
            1e-11
        ));
    }

    /// A malformed feed body surfaces as a decode error, not a panic.
    #[test]
    fn pipeline_reports_malformed_body() {
        let good = eurusd_1y("feed-a", 10.4, 0).to_json().unwrap();
        let err = pipeline(&[&good, "{ not json"], 0.02, BlendConfig::new(0)).unwrap_err();
        assert!(matches!(err, PipelineError::Decode(_)));
    }
}
