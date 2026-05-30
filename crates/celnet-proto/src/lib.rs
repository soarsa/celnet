//! Celnet wire contract — the single, current message schema for the pricing /
//! quote / risk surface (work-stream WS-0, gate G0).
//!
//! There is exactly **one clean, current contract** (ADR-0007): no
//! `schema_version` field, no version-negotiation handshake, no N/N-1
//! back-compat shims. Zero-downtime upgrades use blue-green / full cutover, so a
//! message on the wire is always interpreted against *this* schema; evolving the
//! contract means changing the `.proto` and every dependent in the same change.
//!
//! The messages are generated from `proto/celnet.proto` by the pure-Rust
//! `protox` compiler driving `prost-build` in `build.rs` — no system `protoc`
//! is required, keeping the build hermetic and reproducible on every platform.
//!
//! The wire identifiers mirror the [`celnet_types`] vocabulary one-to-one
//! (`CcyPair`, `OptionType`, the convention enums, and the 14-member Greek set)
//! so the on-wire form and the in-process DTOs never drift apart. Conversions
//! between the two live in this crate (see [`convert`]).
//!
//! ## Encoding / decoding
//!
//! Every message implements [`prost::Message`], so encoding and decoding go
//! through that trait directly:
//!
//! ```
//! use celnet_proto::{Envelope, Heartbeat, envelope::Payload};
//! use prost::Message as _;
//!
//! let env = Envelope {
//!     payload: Some(Payload::Heartbeat(Heartbeat { sequence: 7, epoch_nanos: 42 })),
//! };
//! let bytes = env.encode_to_vec();
//! let back = Envelope::decode(bytes.as_slice()).unwrap();
//! assert_eq!(env, back);
//! ```

#![forbid(unsafe_code)]

// The generated module. `prost-build` writes one file per proto `package`,
// named `celnet.wire.rs`; we surface its contents at the crate root so callers
// import `celnet_proto::Envelope` rather than a nested path.
mod generated {
    include!(concat!(env!("OUT_DIR"), "/celnet.wire.rs"));
}

pub use generated::*;

pub mod convert;

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn sample_pair() -> CcyPair {
        CcyPair {
            base: "EUR".to_owned(),
            quote: "USD".to_owned(),
        }
    }

    fn sample_inputs() -> VanillaInputs {
        VanillaInputs {
            spot: 1.1000,
            strike: 1.1050,
            vol: 0.0825,
            t: 0.5,
            r_dom: 0.0450,
            r_for: 0.0300,
        }
    }

    fn sample_greeks() -> Greeks {
        Greeks {
            price: 0.012_345,
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
        }
    }

    /// Generic round-trip: encode any prost message and decode it back.
    fn round_trip<M: Message + Default + PartialEq + Clone>(msg: &M) {
        let bytes = msg.encode_to_vec();
        let decoded = M::decode(bytes.as_slice()).expect("decode must succeed");
        assert_eq!(
            *msg, decoded,
            "round-trip must preserve the message exactly"
        );
    }

    #[test]
    fn round_trip_pricing_request() {
        let msg = PricingRequest {
            request_id: 0xDEAD_BEEF,
            pair: Some(sample_pair()),
            option_type: OptionType::Call as i32,
            inputs: Some(sample_inputs()),
            delta_convention: DeltaConvention::SpotPremiumAdjusted as i32,
            premium_style: PremiumStyle::PercentForeign as i32,
        };
        round_trip(&msg);
    }

    #[test]
    fn round_trip_pricing_response() {
        let msg = PricingResponse {
            request_id: 0xDEAD_BEEF,
            greeks: Some(sample_greeks()),
            delta_convention_value: 0.4821,
        };
        round_trip(&msg);
    }

    #[test]
    fn round_trip_quote() {
        let msg = Quote {
            quote_id: 1_001,
            pair: Some(sample_pair()),
            option_type: OptionType::Put as i32,
            strike: 1.0950,
            tenor_years: 0.25,
            bid: 0.0081,
            ask: 0.0089,
            premium_style: PremiumStyle::DomesticPips as i32,
            epoch_nanos: 1_717_000_000_000_000_000,
        };
        round_trip(&msg);
    }

    #[test]
    fn round_trip_surface_snapshot() {
        let msg = SurfaceSnapshot {
            pair: Some(sample_pair()),
            points: vec![
                SmilePoint {
                    delta: -0.10,
                    tenor_years: 0.0833,
                    vol: 0.0951,
                },
                SmilePoint {
                    delta: -0.25,
                    tenor_years: 0.0833,
                    vol: 0.0883,
                },
                SmilePoint {
                    delta: 0.50,
                    tenor_years: 0.0833,
                    vol: 0.0840,
                },
                SmilePoint {
                    delta: 0.25,
                    tenor_years: 0.0833,
                    vol: 0.0869,
                },
                SmilePoint {
                    delta: 0.10,
                    tenor_years: 0.0833,
                    vol: 0.0922,
                },
            ],
            epoch_nanos: 1_717_000_000_000_000_000,
        };
        round_trip(&msg);
    }

    #[test]
    fn round_trip_heartbeat() {
        let msg = Heartbeat {
            sequence: u64::MAX,
            epoch_nanos: -1, // pre-epoch timestamps must survive the round-trip too.
        };
        round_trip(&msg);
    }

    #[test]
    fn round_trip_envelope_each_variant() {
        let variants = [
            envelope::Payload::PricingRequest(PricingRequest {
                request_id: 1,
                pair: Some(sample_pair()),
                option_type: OptionType::Call as i32,
                inputs: Some(sample_inputs()),
                delta_convention: DeltaConvention::ForwardUnadjusted as i32,
                premium_style: PremiumStyle::DomesticPips as i32,
            }),
            envelope::Payload::PricingResponse(PricingResponse {
                request_id: 1,
                greeks: Some(sample_greeks()),
                delta_convention_value: 0.5,
            }),
            envelope::Payload::Quote(Quote {
                quote_id: 2,
                pair: Some(sample_pair()),
                option_type: OptionType::Put as i32,
                strike: 1.1,
                tenor_years: 1.0,
                bid: 0.01,
                ask: 0.011,
                premium_style: PremiumStyle::PercentDomestic as i32,
                epoch_nanos: 1,
            }),
            envelope::Payload::SurfaceSnapshot(SurfaceSnapshot {
                pair: Some(sample_pair()),
                points: vec![SmilePoint {
                    delta: 0.5,
                    tenor_years: 1.0,
                    vol: 0.08,
                }],
                epoch_nanos: 1,
            }),
            envelope::Payload::Heartbeat(Heartbeat {
                sequence: 9,
                epoch_nanos: 1,
            }),
        ];
        for payload in variants {
            round_trip(&Envelope {
                payload: Some(payload),
            });
        }
    }

    #[test]
    fn empty_envelope_round_trips_to_none() {
        // An envelope with no payload is a legitimate (if unusual) zero message.
        round_trip(&Envelope { payload: None });
    }
}
