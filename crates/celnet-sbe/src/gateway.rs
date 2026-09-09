//! SBE Protocol Gateway Transcoder.
//!
//! Provides zero-loss translation between internal SBE flyweight streams,
//! Protobuf v3 contracts (`celnet-proto`), and JSON WebSocket tap frames.
//!
//! Enables Tier 4 edge gateways (Web GUI, Excel Add-in, Python SDK) to receive
//! high-frequency ticks and quotes produced by Tier 0 and Tier 1.
#![deny(missing_docs)]

use crate::{OptionQuote, OptionQuoteFlyweight, PriceTick};

/// Transcoder bridging SBE messages to external protocol envelopes.
pub struct SbeGatewayTranscoder;

impl SbeGatewayTranscoder {
    /// Transcode an SBE `OptionQuoteFlyweight` to a Protobuf `celnet_proto::Quote`.
    pub fn flyweight_to_proto_quote(fw: &OptionQuoteFlyweight<'_>) -> celnet_proto::Quote {
        let g = fw.greeks();
        celnet_proto::Quote {
            quote_id: fw.quote_id(),
            idempotency_key: format!("SBE-Q-{}", fw.quote_id()),
            price: Some(celnet_proto::TwoWayPrice {
                bid: fw.bid_price(),
                offer: fw.ask_price(),
            }),
            greeks: Some(celnet_proto::Greeks {
                price: g.price,
                delta_spot: g.delta_spot,
                delta_forward: g.delta_forward,
                gamma: g.gamma,
                vega: g.vega,
                theta: g.theta,
                rate_sensitivities: Some(celnet_proto::RateSensitivities::fx(g.rho_dom, g.rho_for)),
                vanna: g.vanna,
                volga: g.volga,
                charm: g.charm,
                speed: g.speed,
                zomma: g.zomma,
                color: g.color,
            }),
            conventions: Some(celnet_proto::Conventions {
                delta_convention: celnet_proto::DeltaConvention::SpotPremiumAdjusted as i32,
                atm_convention: celnet_proto::AtmConvention::DeltaNeutralStraddle as i32,
                premium_style: celnet_proto::PremiumStyle::PercentForeign as i32,
                cut: celnet_proto::Cut::NewYork1000 as i32,
                day_count: celnet_proto::DayCount::Act365Fixed as i32,
                settlement: celnet_proto::Settlement::Deliverable as i32,
            }),
            resolved_strike: fw.resolved_strike(),
            epoch_nanos: fw.epoch_nanos(),
            valid_until_nanos: fw.valid_until_nanos(),
            correlation_id: None,
            surface_version: Some(fw.surface_version()),
            attribution: None,
            price_std_error: None,
            pricing_provenance: None,
        }
    }

    /// Transcode an SBE `OptionQuote` to a Protobuf `celnet_proto::Quote`.
    pub fn quote_to_proto(quote: &OptionQuote) -> celnet_proto::Quote {
        let g = &quote.greeks;
        celnet_proto::Quote {
            quote_id: quote.quote_id,
            idempotency_key: format!("SBE-Q-{}", quote.quote_id),
            price: Some(celnet_proto::TwoWayPrice {
                bid: quote.bid_price,
                offer: quote.ask_price,
            }),
            greeks: Some(celnet_proto::Greeks {
                price: g.price,
                delta_spot: g.delta_spot,
                delta_forward: g.delta_forward,
                gamma: g.gamma,
                vega: g.vega,
                theta: g.theta,
                rate_sensitivities: Some(celnet_proto::RateSensitivities::fx(g.rho_dom, g.rho_for)),
                vanna: g.vanna,
                volga: g.volga,
                charm: g.charm,
                speed: g.speed,
                zomma: g.zomma,
                color: g.color,
            }),
            conventions: Some(celnet_proto::Conventions {
                delta_convention: celnet_proto::DeltaConvention::SpotPremiumAdjusted as i32,
                atm_convention: celnet_proto::AtmConvention::DeltaNeutralStraddle as i32,
                premium_style: celnet_proto::PremiumStyle::PercentForeign as i32,
                cut: celnet_proto::Cut::NewYork1000 as i32,
                day_count: celnet_proto::DayCount::Act365Fixed as i32,
                settlement: celnet_proto::Settlement::Deliverable as i32,
            }),
            resolved_strike: quote.resolved_strike,
            epoch_nanos: quote.epoch_nanos,
            valid_until_nanos: quote.valid_until_nanos,
            correlation_id: None,
            surface_version: Some(quote.surface_version),
            attribution: None,
            price_std_error: None,
            pricing_provenance: None,
        }
    }

    /// Serialize an SBE `OptionQuote` to a JSON string for WebSocket distribution.
    pub fn quote_to_json(quote: &OptionQuote) -> Result<String, serde_json::Error> {
        serde_json::to_string(quote)
    }

    /// Serialize an SBE `PriceTick` to a JSON string for WebSocket distribution.
    pub fn tick_to_json(tick: &PriceTick) -> Result<String, serde_json::Error> {
        serde_json::to_string(tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_transcode_option_quote() {
        let quote = OptionQuote {
            quote_id: 12345,
            epoch_nanos: 1_725_450_000_000_000_000,
            valid_until_nanos: 1_725_450_005_000_000_000,
            bid_price: 0.01234,
            ask_price: 0.01238,
            resolved_strike: 1.0850,
            surface_version: 7,
            greeks: celnet_types::Greeks {
                price: 0.012345,
                delta_spot: 0.4821,
                delta_forward: 0.4933,
                gamma: 2.118,
                vega: 0.305,
                theta: -0.018,
                rho_dom: 0.061,
                rho_for: -0.058,
                vanna: -0.072,
                volga: 0.144,
                charm: 0.0009,
                speed: -1.21,
                zomma: 0.33,
                color: 0.0004,
            },
        };

        let proto = SbeGatewayTranscoder::quote_to_proto(&quote);
        assert_eq!(proto.quote_id, 12345);
        assert_eq!(proto.price.as_ref().unwrap().bid, 0.01234);
        assert_eq!(proto.price.as_ref().unwrap().offer, 0.01238);
        assert_eq!(proto.greeks.as_ref().unwrap().vega, 0.305);

        let json = SbeGatewayTranscoder::quote_to_json(&quote).expect("json succeeds");
        assert!(json.contains("\"quote_id\":12345"));
        assert!(json.contains("\"bid_price\":0.01234"));
    }
}
