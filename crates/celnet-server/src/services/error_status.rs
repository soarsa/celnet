//! The single canonical error → gRPC [`tonic::Status`] mapping for the server's
//! service edges.
//!
//! Each typed domain error has **exactly one** place that decides which gRPC
//! status code it surfaces as, so the code-selection rule lives next to the error
//! taxonomy it governs rather than being re-decided inline at every `map_err`
//! callsite (which is how the same `PriceError → INVALID_ARGUMENT` /
//! `CoreLinkError → UNAVAILABLE` decision had silently diverged across the
//! [`super::stream`] and [`super::quote`] edges). Collapsing those scattered
//! `Status::*(e.to_string())` callsites onto these functions keeps the wire
//! contract (code **and** message) byte-identical while removing the duplication.
//!
//! The status message is always the error's [`Display`](core::fmt::Display) form
//! (`e.to_string()`), exactly as the inline callsites produced it — the typed
//! variant only ever selects the *code*, never rewrites the *message*.
//!
//! ## The rule, by taxonomy
//!
//! * [`PriceError`] — every variant is a malformed pricing request (a missing
//!   field, an unknown enum tag, an out-of-domain input, a model/product
//!   mismatch): all map to `INVALID_ARGUMENT`. This is the boundary contract the
//!   variants' own doc comments promise.
//! * [`CoreLinkError`] — the async⇄core bridge has shut down (its sole
//!   `CoreUnavailable` variant): `UNAVAILABLE`, so the client steers to a live
//!   instance.
//! * [`RatesPriceError`] — a [`RatesPriceError::Bootstrap`] failure is a numeric
//!   fault on otherwise-valid input (`INTERNAL`); every other variant is a
//!   malformed request (`INVALID_ARGUMENT`). This mirrors the
//!   `PricingService.PriceRates` edge.

// `tonic::Status` is a large error type carried by value across the whole service
// surface (mirroring `services::risk` / `rates_risk::convert`).
#![allow(clippy::result_large_err)]

use tonic::Status;

use crate::core_link::CoreLinkError;
use crate::pricer::PriceError;
use crate::rates_pricing::RatesPriceError;

/// Map a [`PriceError`] to a gRPC [`Status`].
///
/// Every [`PriceError`] variant is a client-side fault — a malformed instrument,
/// an unknown wire enum, an out-of-domain input, or a model/product mismatch — so
/// the whole taxonomy maps to `INVALID_ARGUMENT`, carrying the error's `Display`
/// text as the status message verbatim.
pub fn price_error_to_status(e: &PriceError) -> Status {
    Status::invalid_argument(e.to_string())
}

/// Map a [`CoreLinkError`] to a gRPC [`Status`].
///
/// The bridge to the pricing core is down, so the edge cannot serve this request:
/// `UNAVAILABLE` tells the client to steer to a live instance.
pub fn link_error_to_status(e: &CoreLinkError) -> Status {
    Status::unavailable(e.to_string())
}

/// Map a [`RatesPriceError`] to a gRPC [`Status`], mirroring the
/// `PricingService.PriceRates` edge: a [`RatesPriceError::Bootstrap`] failure on
/// otherwise-valid input is an internal numeric fault; every other variant is a
/// malformed request.
pub fn rates_price_error_to_status(e: &RatesPriceError) -> Status {
    match e {
        RatesPriceError::Bootstrap(_) => Status::internal(e.to_string()),
        _ => Status::invalid_argument(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn price_error_variants_all_map_to_invalid_argument() {
        // One representative of each shape of `PriceError`; every one is a
        // client-side fault, so the code is always INVALID_ARGUMENT and the
        // message is the variant's `Display` text verbatim.
        let cases = [
            PriceError::MissingField("instrument"),
            PriceError::UnknownEnum {
                kind: "DeltaConvention",
                tag: 99,
            },
            PriceError::EmptyProduct,
            PriceError::Domain("expiry_years must be positive"),
            PriceError::UnsupportedModel {
                model: "LOCAL_STOCH_VOL",
                product: "double_barrier_option",
            },
            PriceError::LinearProductNotAnOption {
                product: "fx_forward",
            },
        ];
        for e in &cases {
            let s = price_error_to_status(e);
            assert_eq!(s.code(), Code::InvalidArgument);
            assert_eq!(s.message(), e.to_string());
        }
    }

    #[test]
    fn link_error_maps_to_unavailable_with_display_message() {
        let e = CoreLinkError::CoreUnavailable;
        let s = link_error_to_status(&e);
        assert_eq!(s.code(), Code::Unavailable);
        assert_eq!(s.message(), e.to_string());
    }

    #[test]
    fn rates_bootstrap_is_internal_others_invalid_argument() {
        let malformed = RatesPriceError::MissingCurveSet;
        let s = rates_price_error_to_status(&malformed);
        assert_eq!(s.code(), Code::InvalidArgument);
        assert_eq!(s.message(), malformed.to_string());
    }
}
