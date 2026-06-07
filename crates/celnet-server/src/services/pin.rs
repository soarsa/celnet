//! Shared `surface_version` pinning: resolve an optional pinned surface version
//! into the effective market vol a request prices against, and the version to echo
//! back on the reply.
//!
//! The wire contract carries an optional `surface_version` on the pricing, RFQ,
//! RFS, and scenario requests. When present it pins the price to a specific
//! [`crate::surface_book::SurfaceBook`] mark (a *data* selector, not an API
//! version — CLAUDE.md rule 9), so the result reproduces exactly the surface the
//! desk marked. When absent the request prices against the engine's current live
//! mark and the reply echoes the live surface version (if the edge tracks one) or
//! nothing.
//!
//! Resolution substitutes the marked at-forward volatility for the live ATM vol in
//! the [`celnet_proto::MarketContext`] the [`crate::pricer`] consumes: the marked
//! surface's ATM-at-forward vol is the headline anchor a quote/stream/scenario is
//! tied to, so a re-price against the same version reproduces the same headline
//! premium independent of any later live re-mark. An unknown pinned version is a
//! hard error (`failed_precondition`) — the pin cannot be honoured — never a
//! silent fallback to the live mark.

#![allow(clippy::result_large_err)]

use celnet_proto::{CcyPair, Instrument, MarketContext};
use tonic::Status;

use crate::surface_book::{PinError, SurfaceBook};

/// The result of resolving a request's `surface_version`: the (possibly
/// vol-overridden) market context to price against, and the surface version to
/// echo on the reply.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PinnedVol {
    /// The market context to price the request against. Equal to the input market
    /// when unpinned; carries the marked at-forward vol when pinned.
    pub(crate) market: MarketContext,
    /// The surface version to echo on the reply (presence-tracked on the wire):
    /// `Some(v)` for a honoured pin, `None` for a live-mark price.
    pub(crate) echo_version: Option<u64>,
}

/// The continuously-compounded forward `F = S · e^{(r_dom − r_for)·T}` the marked
/// smile is anchored on. Uses `celnet_core::math::exp` for determinism.
fn forward(market: &MarketContext, expiry_years: f64) -> f64 {
    market.spot * celnet_core::math::exp((market.r_dom - market.r_for) * expiry_years)
}

/// Resolve an optional pinned `surface_version` for an `instrument` against a
/// `market`.
///
/// * `surface_version == None` ⇒ price against the live `market`, echo `None`.
/// * `surface_version == Some(v)` and `v` marked this pair/tenor ⇒ override
///   `market.vol` with the marked at-forward vol and echo `Some(v)`.
/// * `surface_version == Some(v)` but `v` marked no slice for this pair ⇒ honour
///   the pin's *version* (echo `Some(v)`) yet keep the live vol (the version is
///   valid but did not cover this pair — the desk marked other pairs under it).
/// * `surface_version == Some(v)` and `v` was never marked ⇒
///   `failed_precondition` (the pin cannot be honoured).
///
/// # Errors
///
/// [`Status::failed_precondition`] if the pinned version was never marked.
pub(crate) fn resolve_pinned_vol(
    book: &SurfaceBook,
    surface_version: Option<u64>,
    instrument: &Instrument,
    market: &MarketContext,
) -> Result<PinnedVol, Status> {
    let Some(version) = surface_version else {
        return Ok(PinnedVol {
            market: *market,
            echo_version: None,
        });
    };

    // A pin must name a pair (the marked surface is keyed per pair).
    let CcyPair { base, quote } = instrument
        .pair
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("a pinned surface_version requires a `pair`"))?;

    let f = forward(market, instrument.expiry_years);
    match book.pinned_vol(version, base, quote, instrument.expiry_years, f) {
        Ok(Some(vol)) => Ok(PinnedVol {
            market: MarketContext { vol, ..*market },
            echo_version: Some(version),
        }),
        // The version exists but did not mark this pair: honour the (valid) pin
        // version on the echo while pricing against the live vol for this pair.
        Ok(None) => Ok(PinnedVol {
            market: *market,
            echo_version: Some(version),
        }),
        Err(PinError::UnknownVersion(v)) => Err(Status::failed_precondition(format!(
            "pinned surface_version {v} was never marked; cannot honour the pin"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{StrikeOrDelta, Vanilla, instrument, strike_or_delta};

    fn instrument(strike: f64) -> Instrument {
        Instrument {
            pair: Some(CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            }),
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::TwoWay as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(instrument::Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
        }
    }

    fn market() -> MarketContext {
        MarketContext {
            spot: 1.10,
            vol: 0.20,
            r_dom: 0.02,
            r_for: 0.01,
        }
    }

    #[test]
    fn unpinned_passes_market_through_and_echoes_none() {
        let book = SurfaceBook::new();
        let resolved = resolve_pinned_vol(&book, None, &instrument(1.10), &market()).unwrap();
        assert_eq!(resolved.echo_version, None);
        assert_eq!(resolved.market.vol.to_bits(), market().vol.to_bits());
    }

    #[test]
    fn unknown_pinned_version_is_failed_precondition() {
        let book = SurfaceBook::new();
        let err = resolve_pinned_vol(&book, Some(7), &instrument(1.10), &market()).unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn pinned_version_without_this_pair_keeps_live_vol_but_echoes_version() {
        use celnet_conventions::ConventionRecord;
        use celnet_surface::{
            MarketContext as SurfCtx, MarketQuotes, SmileModel, build_model_smile,
        };
        use celnet_types::{
            AtmConvention, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement,
        };

        let book = SurfaceBook::new();
        let v = book.next_version();
        // The version exists and marks GBP/USD — but not the EUR/USD instrument.
        let record = ConventionRecord::new(
            DeltaConvention::SpotUnadjusted,
            AtmConvention::AtmForward,
            PremiumStyle::DomesticPips,
            Cut::NewYork1000,
            DayCount::Act365Fixed,
            DayCount::Act365Fixed,
            DayCount::Act365Fixed,
            Settlement::Deliverable,
        );
        let ctx = SurfCtx::new(1.25, 0.02, 0.01, 1.0, record);
        let smile = build_model_smile(
            SmileModel::MarketHedge,
            &ctx,
            &MarketQuotes::three_point(0.10, -0.004, 0.002),
        )
        .unwrap();
        let fwd = smile.forward();
        book.deposit(v, "GBP", "USD", 1.0, fwd, smile);

        let resolved = resolve_pinned_vol(&book, Some(v), &instrument(1.10), &market()).unwrap();
        assert_eq!(resolved.echo_version, Some(v));
        assert_eq!(
            resolved.market.vol.to_bits(),
            market().vol.to_bits(),
            "an unmarked pair keeps the live vol while echoing the valid pin version"
        );
    }
}
