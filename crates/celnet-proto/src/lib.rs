//! Celnet wire contract — the single, current, trader-shaped message schema for
//! the pricing / quote / RFQ / RFS / surface / risk surface (work-stream WS-0,
//! gate G0).
//!
//! There is exactly **one clean, current contract** (ADR-0007): no
//! `schema_version` field, no version-negotiation handshake, no N/N-1
//! back-compat shims. Zero-downtime upgrades use blue-green / full cutover, so a
//! message on the wire is always interpreted against *this* schema; evolving the
//! contract means changing the `.proto` and every dependent in the same change.
//!
//! The messages **and** the gRPC service stubs (client + server) are generated
//! from `proto/celnet.proto` by the pure-Rust `protox` compiler driving
//! `tonic-build`/`prost-build` in `build.rs` — no system `protoc` is required,
//! keeping the build hermetic and reproducible on every platform.
//!
//! ## Contract shape
//!
//! The schema is organized as five logical families inside one package
//! (`celnet.wire`), modelling how an FX-options desk actually works:
//!
//! * **vocabulary** — enums and value messages mirroring [`celnet_types`]
//!   one-to-one ([`CcyPair`], [`OptionType`], the convention enums, [`Tenor`],
//!   [`Conventions`], and the 14-member [`Greeks`] set) so the on-wire form and
//!   the in-process DTOs never drift apart.
//! * **instrument** — the unified [`Instrument`] `oneof` every workflow speaks:
//!   vanilla, multi-leg [`Strategy`] (risk-reversal / strangle / straddle /
//!   seagull), [`SingleBarrier`] / [`DoubleBarrier`], [`Digital`], and [`Touch`]
//!   (one-touch / no-touch / double-no-touch / double-one-touch), carrying a
//!   [`Quantity`], a [`Side`], and an optional [`Solve`] directive.
//! * **quote** — the RFQ lifecycle: [`QuoteRequest`] (client idempotency key +
//!   instrument, optional `correlation_id` + pinned `surface_version`) →
//!   [`Quote`] (two-way bid/offer + Greeks + `quote_id` + `valid_until_nanos`) →
//!   [`QuoteAccept`] / [`QuoteReject`] → [`Execution`].
//! * **stream** — the multiplexed RFS [`StreamService::StreamSession`]: ONE
//!   bidirectional channel carrying many subscriptions, each keyed by a
//!   [`SubscriptionId`]. [`ClientStreamMessage`] is [`Subscribe`] / [`Modify`]
//!   (re-baseline in place) / [`Unsubscribe`] / [`Resync`] / [`Execute`]
//!   (click-to-trade) / [`Heartbeat`]; [`ServerStreamMessage`] is [`Snapshot`]
//!   then sequenced [`Update`] deltas (each stamped with [`TradableToken`]s for
//!   click-to-trade) + [`Heartbeat`] + [`StreamEnd`] + [`Executed`] /
//!   [`StreamReject`]. A monotonic per-subscription sequence drives gap
//!   detection and server-assisted resync.
//! * **surface** — the surface workflow: [`GetSmileRequest`] → [`Smile`]
//!   (delta-axis vols + [`BrokerQuoteSet`] + [`ArbReport`]),
//!   [`MarkSurfaceRequest`] → [`MarkSurfaceResponse`], and [`ScenarioRequest`] →
//!   [`ScenarioResponse`]: a spot/vol/rate/**time** (theta-roll) shock grid of
//!   repriced [`Greeks`] plus an optional [`BucketedRisk`] decomposition
//!   ([`VegaBucket`] by (tenor, delta-pillar), [`CrossGamma`], theta roll).
//!
//! Conversions between the wire vocabulary and the [`celnet_types`] DTOs live in
//! [`convert`].
//!
//! ## Services
//!
//! The generated tonic stubs are surfaced under their package modules:
//! [`pricing_service_client`] / [`pricing_service_server`],
//! [`quote_service_client`] / [`quote_service_server`],
//! [`stream_service_client`] / [`stream_service_server`], and
//! [`surface_service_client`] / [`surface_service_server`].
//!
//! ## Encoding / decoding
//!
//! Every message implements [`prost::Message`], so encoding and decoding go
//! through that trait directly:
//!
//! ```
//! use celnet_proto::{Heartbeat, SubscriptionId};
//! use prost::Message as _;
//!
//! let hb = Heartbeat {
//!     subscription: Some(SubscriptionId { value: 7 }),
//!     sequence: 42,
//!     epoch_nanos: 1_717_000_000_000_000_000,
//!     ..Default::default()
//! };
//! let bytes = hb.encode_to_vec();
//! let back = Heartbeat::decode(bytes.as_slice()).unwrap();
//! assert_eq!(hb, back);
//! ```

#![forbid(unsafe_code)]

// The generated module. `tonic-build` writes one file per proto `package`,
// named `celnet.wire.rs`; we surface its contents at the crate root so callers
// import `celnet_proto::Instrument` rather than a nested path.
mod generated {
    include!(concat!(env!("OUT_DIR"), "/celnet.wire.rs"));
}

pub use generated::*;

pub mod convert;
pub mod helpers;

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

    fn sample_underlying() -> Underlying {
        Underlying::fx(sample_pair())
    }

    fn sample_conventions() -> Conventions {
        Conventions {
            delta_convention: DeltaConvention::SpotPremiumAdjusted as i32,
            atm_convention: AtmConvention::DeltaNeutralStraddle as i32,
            premium_style: PremiumStyle::PercentForeign as i32,
            cut: Cut::NewYork1000 as i32,
            day_count: DayCount::Act365Fixed as i32,
            settlement: Settlement::Deliverable as i32,
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
            rate_sensitivities: Some(RateSensitivities::fx(0.061, -0.058)),
            vanna: -0.072,
            volga: 0.144,
            charm: 0.0009,
            speed: -1.21,
            zomma: 0.33,
            color: 0.0004,
        }
    }

    fn sample_quantity() -> Quantity {
        Quantity {
            notional: 10_000_000.0,
            base_ccy: true,
        }
    }

    fn sample_tenor() -> Tenor {
        Tenor {
            unit: tenor::Unit::Months as i32,
            count: 3,
            broken_date: None,
        }
    }

    fn vanilla_instrument() -> Instrument {
        Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.25,
            quantity: Some(sample_quantity()),
            side: Side::TwoWay as i32,
            solve: Some(Solve {
                target: solve::Target::None as i32,
                target_premium: 0.0,
            }),
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Vanilla(Vanilla {
                option_type: OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Delta(0.25)),
                }),
            })),
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
    fn round_trip_price_request_response() {
        let req = PriceRequest {
            request_id: 0xDEAD_BEEF,
            instrument: Some(vanilla_instrument()),
            market: Some(MarketContext::fx(1.10, 0.0825, 0.045, 0.030)),
            conventions: Some(sample_conventions()),
            correlation_id: Some(0x0102_0304),
            surface_version: Some(11),
        };
        round_trip(&req);

        // Absent optionals must survive the round-trip as `None`.
        let req_no_optionals = PriceRequest {
            request_id: 7,
            instrument: Some(vanilla_instrument()),
            market: Some(MarketContext::fx(1.10, 0.0825, 0.045, 0.030)),
            conventions: Some(sample_conventions()),
            correlation_id: None,
            surface_version: None,
        };
        round_trip(&req_no_optionals);

        let resp = PriceResponse {
            request_id: 0xDEAD_BEEF,
            greeks: Some(sample_greeks()),
            resolved_strike: 1.1050,
            conventions: Some(sample_conventions()),
            correlation_id: Some(0x0102_0304),
            surface_version: Some(11),
            price_std_error: Some(2.5e-4),
        };
        round_trip(&resp);
    }

    #[test]
    fn round_trip_strategy_instrument() {
        let leg = |ot: OptionType, delta: f64, side: Side| Leg {
            option_type: ot as i32,
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Delta(delta)),
            }),
            side: side as i32,
            ratio: 1.0,
        };
        let instr = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.25,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: Some(Solve {
                target: solve::Target::Strike as i32,
                target_premium: 0.0,
            }),
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Strategy(Strategy {
                kind: StrategyKind::RiskReversal as i32,
                legs: vec![
                    leg(OptionType::Call, 0.25, Side::Buy),
                    leg(OptionType::Put, -0.25, Side::Sell),
                ],
            })),
        };
        round_trip(&instr);
    }

    #[test]
    fn round_trip_barrier_digital_touch_instruments() {
        let barrier = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::SingleBarrier(SingleBarrier {
                vanilla: Some(Vanilla {
                    option_type: OptionType::Call as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(1.10)),
                    }),
                }),
                kind: BarrierKind::KnockOut as i32,
                side: BarrierSide::Up as i32,
                barrier: 1.20,
                rebate: 0.0,
                monitoring: MonitoringStyle::Continuous as i32,
            })),
        };
        round_trip(&barrier);

        let double = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.5,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::DoubleBarrier(DoubleBarrier {
                vanilla: Some(Vanilla {
                    option_type: OptionType::Put as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(1.05)),
                    }),
                }),
                kind: BarrierKind::KnockIn as i32,
                lower_barrier: 1.00,
                upper_barrier: 1.20,
                rebate: 0.001,
                monitoring: MonitoringStyle::Discrete as i32,
            })),
        };
        round_trip(&double);

        let digital = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.25,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Digital(Digital {
                option_type: OptionType::Call as i32,
                strike: 1.12,
                style: DigitalStyle::CashOrNothing as i32,
                payout: 1_000_000.0,
            })),
        };
        round_trip(&digital);

        let touch = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.75,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Touch(Touch {
                kind: TouchKind::DoubleNoTouch as i32,
                lower_barrier: 1.05,
                upper_barrier: 1.15,
                rebate: 250_000.0,
                monitoring: MonitoringStyle::Continuous as i32,
            })),
        };
        round_trip(&touch);
    }

    #[test]
    fn round_trip_swap_and_asian_instruments() {
        let var_swap = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::VarianceSwap(VarianceSwap {
                strike_vol: 0.11,
            })),
        };
        round_trip(&var_swap);

        let vol_swap = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.5,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::VolatilitySwap(VolatilitySwap {
                strike_vol: 0.0,
            })),
        };
        round_trip(&vol_swap);

        let asian = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::AsianOption(AsianOption {
                option_type: OptionType::Call as i32,
                strike: 1.10,
                averaging: AveragingStyle::Discrete as i32,
                observations: 12,
                method: AsianMethod::Curran as i32,
                elapsed_avg: 1.095,
                elapsed_weight: 0.25,
            })),
        };
        round_trip(&asian);
    }

    #[test]
    fn round_trip_forward_start_cliquet_quanto_instruments() {
        let forward_start = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::ForwardStart(ForwardStart {
                option_type: OptionType::Call as i32,
                moneyness: 1.0,
                reset: 0.25,
            })),
        };
        round_trip(&forward_start);

        let cliquet = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Cliquet(Cliquet {
                option_type: OptionType::Call as i32,
                moneyness: 1.0,
                periods: 4,
                local_floor: Some(0.0),
                local_cap: Some(0.05),
                global_floor: None,
                global_cap: None,
                mc_pairs: 4096,
                mc_seed: 0xC119_0E70,
            })),
        };
        round_trip(&cliquet);

        let quanto = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 0.75,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Quanto(Quanto {
                payoff: QuantoPayoff::Vanilla as i32,
                option_type: OptionType::Put as i32,
                strike: 1.12,
                conversion_vol: 0.09,
                correlation: -0.3,
            })),
        };
        round_trip(&quanto);
    }

    #[test]
    fn round_trip_tarf_accumulator_lookback_instruments() {
        let tarf = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Tarf(Tarf {
                option_type: OptionType::Put as i32,
                strike: 1.10,
                target: 0.30,
                leverage: 2.0,
                redemption: TarfRedemption::FullGain as i32,
                schedule: Some(FixingSchedule {
                    fixing_years: vec![0.25, 0.5, 0.75, 1.0],
                    fixing_notional: 1.0,
                }),
                mc_pairs: 4096,
                mc_seed: 0x7A2F_0001,
            })),
        };
        round_trip(&tarf);

        // The pivot TRA (arm 32): the TARF generalized with a distinct pivot
        // kink — same schedule/redemption vocabulary, one extra level.
        let pivot = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Pivot(Pivot {
                option_type: OptionType::Call as i32,
                strike: 1.08,
                pivot: 1.13,
                target: 0.20,
                leverage: 2.5,
                redemption: TarfRedemption::CappedGain as i32,
                schedule: Some(FixingSchedule {
                    fixing_years: vec![0.25, 0.5, 0.75, 1.0],
                    fixing_notional: 1.0,
                }),
                mc_pairs: 4096,
                mc_seed: 0x9_1707_0001,
            })),
        };
        round_trip(&pivot);

        let accumulator = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Accumulator(Accumulator {
                pivot: 1.10,
                barrier: 1.16,
                leverage: 2.0,
                monitoring: AccumulatorMonitoring::Continuous as i32,
                schedule: Some(FixingSchedule {
                    fixing_years: vec![0.25, 0.5, 0.75, 1.0],
                    fixing_notional: 1.0,
                }),
                mc_pairs: 4096,
                mc_seed: 0xACC0_0001,
            })),
        };
        round_trip(&accumulator);

        let lookback = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Lookback(Lookback {
                style: LookbackStyle::Fixed as i32,
                option_type: OptionType::Call as i32,
                monitoring: LookbackMonitoring::Discrete as i32,
                strike: 1.05,
                observations: 64,
                mc_pairs: 8192,
                mc_seed: 0x100C_BAC4,
            })),
        };
        round_trip(&lookback);
    }

    #[test]
    fn round_trip_window_barrier_and_pricing_model() {
        // The window-barrier product (LSV-only) round-trips, including the
        // pricing-model selector carried on the instrument.
        let window = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::LocalStochVol as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::WindowBarrier(WindowBarrier {
                vanilla: Some(Vanilla {
                    option_type: OptionType::Call as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(1.10)),
                    }),
                }),
                barrier: 1.30,
                side: BarrierSide::Up as i32,
                window_start: 0.25,
                window_end: 0.75,
                mc_pairs: 4096,
                mc_steps: 96,
                mc_seed: 0x00C0_FFEE,
            })),
        };
        round_trip(&window);

        // The pricing-model selector also round-trips on a plain vanilla, and the
        // proto3 zero value is the DEFAULT model (so an unset field is DEFAULT).
        assert_eq!(PricingModel::Default as i32, 0);
        let mut v = vanilla_instrument();
        v.pricing_model = PricingModel::LocalStochVol as i32;
        round_trip(&v);
    }

    #[test]
    fn round_trip_american_bermudan_instruments() {
        // The American early-exercise vanilla (oneof field 24) round-trips on the
        // default model, including the LSM knobs.
        let american = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::American(AmericanOption {
                option_type: OptionType::Put as i32,
                strike: 1.10,
                exercise_style: ExerciseStyle::American as i32,
                bermudan_dates: Vec::new(),
                lsm_paths: 0,
                lsm_exercise_dates: 0,
                lsm_seed: 0,
            })),
        };
        round_trip(&american);

        // The Bermudan variant carries an explicit (repeated) date set and the
        // LSM engine knobs; the proto3 zero of `ExerciseStyle` is AMERICAN.
        assert_eq!(ExerciseStyle::American as i32, 0);
        let bermudan = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::American(AmericanOption {
                option_type: OptionType::Call as i32,
                strike: 1.05,
                exercise_style: ExerciseStyle::Bermudan as i32,
                bermudan_dates: vec![0.25, 0.5, 0.75, 1.0],
                lsm_paths: 100_000,
                lsm_exercise_dates: 50,
                lsm_seed: 0x00C0_FFEE,
            })),
        };
        round_trip(&bermudan);
    }

    #[test]
    fn round_trip_basket_instrument() {
        // The correlated multi-asset basket (oneof field 25) round-trips with its
        // per-leg market data and the row-major correlation array.
        let basket = Instrument {
            underlying: Some(sample_underlying()),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Basket(BasketOption {
                legs: vec![
                    BasketLeg {
                        underlying: Some(Underlying::fx(CcyPair {
                            base: "EUR".to_owned(),
                            quote: "USD".to_owned(),
                        })),
                        weight: 0.5,
                        spot: 1.10,
                        vol: 0.11,
                        r_for: 0.015,
                    },
                    BasketLeg {
                        underlying: Some(Underlying::fx(CcyPair {
                            base: "GBP".to_owned(),
                            quote: "USD".to_owned(),
                        })),
                        weight: 0.5,
                        spot: 1.27,
                        vol: 0.13,
                        r_for: 0.02,
                    },
                ],
                // Row-major 2×2 correlation [1, 0.4; 0.4, 1].
                correlations: vec![1.0, 0.4, 0.4, 1.0],
                option_type: OptionType::Call as i32,
                strike: 1.18,
                kind: BasketKind::WorstOf as i32,
                mc_paths: 8192,
                mc_replications: 16,
                mc_steps: 1,
                mc_seed: 0x00C0_FFEE,
            })),
        };
        round_trip(&basket);

        // proto3 zero of BasketKind is BASKET (the canonical first member).
        assert_eq!(BasketKind::Basket as i32, 0);
    }

    #[test]
    fn round_trip_perpetual_and_listed_future_instruments() {
        // The perpetual arm (oneof field 30) round-trips. A perpetual has no
        // expiry, so the instrument carries `expiry_years: 0.0` (the shape
        // `convert::validate_perpetual_terms` enforces) and no tenor label.
        let perpetual = Instrument {
            underlying: Some(sample_underlying()),
            tenor: None,
            expiry_years: 0.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::PerpetualOption(PerpetualOption {
                option_type: OptionType::Put as i32,
                strike: 1.05,
                notional: 10_000_000.0,
            })),
        };
        round_trip(&perpetual);

        // The listed-future-option arm (oneof field 31) round-trips with the
        // future's own (longer) expiry and the futures-style margining tag.
        // The underlying names the asset class; `future_symbol` the contract.
        let future_option = Instrument {
            underlying: Some(Underlying::commodity(CommodityRef::new(
                Symbol::new("BRENT", ""),
                "USD",
            ))),
            tenor: Some(sample_tenor()),
            expiry_years: 0.5,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::ListedFutureOption(
                ListedFutureOption {
                    future_symbol: Some(Symbol::new("BRN-DEC26", "IFEU")),
                    future_expiry_years: 0.55,
                    option_type: OptionType::Call as i32,
                    strike: 85.0,
                    notional: 1_000.0,
                    margining: Margining::FuturesStyle as i32,
                },
            )),
        };
        round_trip(&future_option);

        // Meaningful-zero: equity-style (upfront-premium) margining is the
        // proto3 default (tag 0), like `SettlementStyle`'s LINEAR.
        assert_eq!(Margining::EquityStyle as i32, 0);
    }

    #[test]
    fn round_trip_rfq_lifecycle() {
        let request = QuoteRequest {
            idempotency_key: "5f0c1b2e-2a4d-4f8a-9c1e-7b6a5d4c3b2a".to_owned(),
            instrument: Some(vanilla_instrument()),
            conventions: Some(sample_conventions()),
            correlation_id: Some(0xCAFE_F00D),
            surface_version: Some(42),
            attribution: Some(sample_attribution()),
            // The RFQ caller (item B §2): the validated session token + asserted
            // principal both round-trip on the wire.
            session_token: Some("sess-abc123".to_owned()),
            principal: Some(EntitlementPrincipal {
                grant_all: false,
                grants: vec![EntitlementRule {
                    scopes: vec![RiskScope {
                        dimension: RiskDimension::Desk as i32,
                        value: 7,
                    }],
                }],
                denies: vec![],
            }),
        };
        round_trip(&request);

        // The optional correlation/surface fields are presence-tracked: an
        // absent pair must round-trip back to `None`, not a sentinel zero.
        let request_no_optionals = QuoteRequest {
            idempotency_key: "no-optionals".to_owned(),
            instrument: Some(vanilla_instrument()),
            conventions: Some(sample_conventions()),
            correlation_id: None,
            surface_version: None,
            attribution: None,
            session_token: None,
            principal: None,
        };
        round_trip(&request_no_optionals);

        let quote = Quote {
            quote_id: 1_001,
            idempotency_key: request.idempotency_key.clone(),
            price: Some(TwoWayPrice {
                bid: 0.0081,
                offer: 0.0089,
            }),
            greeks: Some(sample_greeks()),
            conventions: Some(sample_conventions()),
            resolved_strike: 1.1050,
            epoch_nanos: 1_717_000_000_000_000_000,
            valid_until_nanos: 1_717_000_300_000_000_000,
            correlation_id: Some(0xCAFE_F00D),
            surface_version: Some(42),
            attribution: Some(sample_attribution()),
            price_std_error: Some(1.7e-4),
        };
        round_trip(&quote);

        let accept = QuoteAccept {
            quote_id: 1_001,
            idempotency_key: request.idempotency_key.clone(),
            side: Side::Buy as i32,
            lp_id: String::new(),
            // The accepting caller (item B §2) round-trips on the wire.
            session_token: Some("sess-abc123".to_owned()),
            principal: None,
        };
        round_trip(&accept);

        let reject = QuoteReject {
            quote_id: 1_001,
            reason: "off-market".to_owned(),
            session_token: None,
            principal: None,
        };
        round_trip(&reject);

        let execution = Execution {
            execution_id: 9_001,
            quote_id: 1_001,
            side: Side::Buy as i32,
            traded_premium: 0.0089,
            instrument: Some(vanilla_instrument()),
            epoch_nanos: 1_717_000_001_000_000_000,
            attribution: Some(sample_attribution()),
        };
        round_trip(&execution);
    }

    #[test]
    fn round_trip_multi_dealer_quote() {
        let dealer = |lp: &str, bid: f64, offer: f64| DealerQuote {
            lp_id: lp.to_owned(),
            price: Some(TwoWayPrice { bid, offer }),
            greeks: Some(sample_greeks()),
            resolved_strike: 1.1050,
            valid_until_nanos: 1_717_000_300_000_000_000,
            attribution: Some(sample_attribution()),
            price_std_error: None,
        };
        let mdq = MultiDealerQuote {
            quote_id: 7_007,
            idempotency_key: "rfq-to-many-1".to_owned(),
            // Ordered best-first on the offer (LP-A tightest), per the contract.
            dealers: vec![
                dealer("LP-A", 0.0082, 0.0088),
                dealer("LP-B", 0.0081, 0.0090),
            ],
            best_bid_lp_id: "LP-A".to_owned(),
            best_offer_lp_id: "LP-A".to_owned(),
            conventions: Some(sample_conventions()),
            epoch_nanos: 1_717_000_000_000_000_000,
            correlation_id: Some(0xCAFE_F00D),
            surface_version: Some(42),
        };
        round_trip(&mdq);

        // The optional fields are presence-tracked.
        let mdq_bare = MultiDealerQuote {
            quote_id: 7_008,
            idempotency_key: "rfq-to-many-2".to_owned(),
            dealers: Vec::new(),
            best_bid_lp_id: String::new(),
            best_offer_lp_id: String::new(),
            conventions: Some(sample_conventions()),
            epoch_nanos: 1,
            correlation_id: None,
            surface_version: None,
        };
        round_trip(&mdq_bare);

        // A multi-dealer accept carries the chosen LP line.
        round_trip(&QuoteAccept {
            quote_id: 7_007,
            idempotency_key: "rfq-to-many-1".to_owned(),
            side: Side::Buy as i32,
            lp_id: "LP-A".to_owned(),
            session_token: None,
            principal: None,
        });
    }

    #[test]
    fn round_trip_cross_asset_instruments() {
        // An inverse coin-margined digital-asset vanilla — the settlement_style
        // and digital_asset arm round-trip on the instrument.
        let crypto = Instrument {
            underlying: Some(Underlying::digital_asset(CryptoPair::new("BTC", "USDT"))),
            tenor: Some(sample_tenor()),
            expiry_years: 0.25,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::InverseCoin as i32,
            product: Some(instrument::Product::Vanilla(Vanilla {
                option_type: OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(65_000.0)),
                }),
            })),
        };
        round_trip(&crypto);

        // An equity vanilla — the equity arm round-trips; default linear style.
        let equity = Instrument {
            underlying: Some(Underlying::equity(EquityRef::new(
                Symbol::new("AAPL", "XNAS"),
                "USD",
            ))),
            tenor: Some(sample_tenor()),
            expiry_years: 1.0,
            quantity: Some(sample_quantity()),
            side: Side::Buy as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Vanilla(Vanilla {
                option_type: OptionType::Put as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(190.0)),
                }),
            })),
        };
        round_trip(&equity);

        // A commodity vanilla — the commodity arm round-trips.
        let commodity = Instrument {
            underlying: Some(Underlying::commodity(CommodityRef::new(
                Symbol::new("BRENT", ""),
                "USD",
            ))),
            tenor: Some(sample_tenor()),
            expiry_years: 0.5,
            quantity: Some(sample_quantity()),
            side: Side::Sell as i32,
            solve: None,
            pricing_model: PricingModel::Default as i32,
            settlement_style: SettlementStyle::Linear as i32,
            product: Some(instrument::Product::Vanilla(Vanilla {
                option_type: OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(85.0)),
                }),
            })),
        };
        round_trip(&commodity);

        // Meaningful-zero: the LINEAR style is the proto3 default (tag 0), so an
        // unset settlement_style is byte-identical to the linear contract.
        assert_eq!(SettlementStyle::Linear as i32, 0);
    }

    /// A sample attribution chain (human-quoted, machine-held, in competition)
    /// exercising every presence-tracked attribution field.
    fn sample_attribution() -> AttributionRecord {
        AttributionRecord {
            quoted_by: Some(BookId {
                book: "EM-VOL-1".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader("jdoe".to_owned())),
                }),
            }),
            held_by: Some(BookId {
                book: "WAREHOUSE".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::AutoPricer("auto-mm-7".to_owned())),
                }),
            }),
            won: Some(true),
            lp_count: Some(3),
        }
    }

    #[test]
    fn round_trip_stream_messages() {
        let sub = SubscriptionId { value: 77 };
        let conv = sample_conventions();

        // The two click-to-trade tokens stamped on a streamed line: SELL@bid
        // and BUY@offer, each bounded by its own validity deadline.
        let tradable = || {
            vec![
                TradableToken {
                    token: 0xA1,
                    side: Side::Sell as i32,
                    premium: 0.0081,
                    valid_until_nanos: 1_717_000_000_500_000_000,
                },
                TradableToken {
                    token: 0xA2,
                    side: Side::Buy as i32,
                    premium: 0.0089,
                    valid_until_nanos: 1_717_000_000_500_000_000,
                },
            ]
        };

        let subscribe = ClientStreamMessage {
            message: Some(client_stream_message::Message::Subscribe(Subscribe {
                subscription: Some(sub),
                instrument: Some(vanilla_instrument()),
                conventions: Some(conv),
                throttle_nanos: 1_000_000,
                correlation_id: Some(0xBEEF),
                surface_version: Some(13),
                attribution: Some(sample_attribution()),
            })),
        };
        round_trip(&subscribe);

        // Subscribe with both optionals absent — presence must round-trip.
        let subscribe_bare = ClientStreamMessage {
            message: Some(client_stream_message::Message::Subscribe(Subscribe {
                subscription: Some(sub),
                instrument: Some(vanilla_instrument()),
                conventions: Some(conv),
                throttle_nanos: 0,
                correlation_id: None,
                surface_version: None,
                attribution: None,
            })),
        };
        round_trip(&subscribe_bare);

        let modify = ClientStreamMessage {
            message: Some(client_stream_message::Message::Modify(Modify {
                subscription: Some(sub),
                instrument: Some(vanilla_instrument()),
                conventions: Some(conv),
                throttle_nanos: 2_000_000,
                surface_version: None,
            })),
        };
        round_trip(&modify);

        let resync = ClientStreamMessage {
            message: Some(client_stream_message::Message::Resync(Resync {
                subscription: Some(sub),
                last_sequence: 41,
            })),
        };
        round_trip(&resync);

        let unsubscribe = ClientStreamMessage {
            message: Some(client_stream_message::Message::Unsubscribe(Unsubscribe {
                subscription: Some(sub),
            })),
        };
        round_trip(&unsubscribe);

        let execute = ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(sub),
                token: 0xA2,
                idempotency_key: "click-1".to_owned(),
                correlation_id: Some(0xBEEF),
            })),
        };
        round_trip(&execute);

        let client_hb = ClientStreamMessage {
            message: Some(client_stream_message::Message::Heartbeat(Heartbeat {
                subscription: Some(sub),
                sequence: 42,
                epoch_nanos: 1,
                ..Default::default()
            })),
        };
        round_trip(&client_hb);

        let snapshot = ServerStreamMessage {
            message: Some(server_stream_message::Message::Snapshot(Snapshot {
                subscription: Some(sub),
                sequence: 1,
                price: Some(TwoWayPrice {
                    bid: 0.0081,
                    offer: 0.0089,
                }),
                greeks: Some(sample_greeks()),
                vol: 0.0825,
                conventions: Some(conv),
                resolved_strike: 1.1050,
                tradable: tradable(),
                surface_version: Some(13),
                correlation_id: Some(0xBEEF),
                epoch_nanos: 1,
                attribution: Some(sample_attribution()),
            })),
        };
        round_trip(&snapshot);

        let update = ServerStreamMessage {
            message: Some(server_stream_message::Message::Update(Update {
                subscription: Some(sub),
                sequence: 2,
                price: Some(TwoWayPrice {
                    bid: 0.0082,
                    offer: 0.0090,
                }),
                greeks: Some(sample_greeks()),
                vol: 0.0830,
                tradable: tradable(),
                surface_version: Some(13),
                epoch_nanos: 2,
            })),
        };
        round_trip(&update);

        let server_hb = ServerStreamMessage {
            message: Some(server_stream_message::Message::Heartbeat(Heartbeat {
                subscription: Some(sub),
                sequence: 2,
                epoch_nanos: 3,
                conflation_drops: 7,
                server_price_p50_nanos: 800,
                server_price_p99_nanos: 4200,
                server_price_p999_nanos: 9100,
                surface_version: 13,
                correlation_id: 0xBEEF,
            })),
        };
        round_trip(&server_hb);

        let executed = ServerStreamMessage {
            message: Some(server_stream_message::Message::Executed(Executed {
                subscription: Some(sub),
                token: 0xA2,
                execution_id: 9_100,
                side: Side::Buy as i32,
                traded_premium: 0.0089,
                correlation_id: Some(0xBEEF),
                epoch_nanos: 4,
                attribution: Some(sample_attribution()),
            })),
        };
        round_trip(&executed);

        let stream_reject = ServerStreamMessage {
            message: Some(server_stream_message::Message::StreamReject(StreamReject {
                subscription: Some(sub),
                token: 0xDEAD,
                reason: stream_reject::Reason::Expired as i32,
                correlation_id: None,
                epoch_nanos: 5,
            })),
        };
        round_trip(&stream_reject);

        let end = ServerStreamMessage {
            message: Some(server_stream_message::Message::StreamEnd(StreamEnd {
                subscription: Some(sub),
                reason: stream_end::Reason::Lagged as i32,
            })),
        };
        round_trip(&end);
    }

    #[test]
    fn round_trip_surface_messages() {
        let broker = BrokerQuoteSet {
            tenor_years: 0.25,
            atm_vol: 0.0840,
            rr_25: -0.0030,
            bf_25: 0.0025,
            rr_10: -0.0060,
            bf_10: 0.0080,
            has_ten_delta: true,
        };

        let get_smile = GetSmileRequest {
            pair: Some(sample_pair()),
            tenor_years: 0.25,
            conventions: Some(sample_conventions()),
        };
        round_trip(&get_smile);

        let smile = Smile {
            pair: Some(sample_pair()),
            tenor_years: 0.25,
            broker_quotes: Some(broker),
            points: vec![
                SmilePoint {
                    delta: -0.10,
                    tenor_years: 0.25,
                    vol: 0.0951,
                },
                SmilePoint {
                    delta: 0.50,
                    tenor_years: 0.25,
                    vol: 0.0840,
                },
                SmilePoint {
                    delta: 0.10,
                    tenor_years: 0.25,
                    vol: 0.0922,
                },
            ],
            conventions: Some(sample_conventions()),
            arbitrage: Some(ArbReport {
                butterfly_arbitrage_free: true,
                calendar_arbitrage_free: true,
                worst_density: 0.0,
                note: "model=stochastic-vol; no repair applied".to_owned(),
                smile_model: SmileModel::StochasticVol as i32,
            }),
            epoch_nanos: 1_717_000_000_000_000_000,
        };
        round_trip(&smile);

        let mark_req = MarkSurfaceRequest {
            pair: Some(sample_pair()),
            broker_quotes: vec![broker],
            conventions: Some(sample_conventions()),
            smile_model: Some(SmileModel::StochasticVol as i32),
        };
        round_trip(&mark_req);

        let mark_resp = MarkSurfaceResponse {
            pair: Some(sample_pair()),
            surface_version: 7,
            smiles: vec![smile],
            epoch_nanos: 1_717_000_000_000_000_000,
        };
        round_trip(&mark_resp);
    }

    #[test]
    fn round_trip_scenario_grid() {
        let req = ScenarioRequest {
            instrument: Some(vanilla_instrument()),
            base_market: Some(MarketContext::fx(1.10, 0.0825, 0.045, 0.030)),
            conventions: Some(sample_conventions()),
            axes: vec![
                ShockAxis {
                    factor: shock_axis::Factor::Spot as i32,
                    relative: true,
                    steps: vec![-0.20, -0.10, 0.0, 0.10, 0.20],
                },
                ShockAxis {
                    factor: shock_axis::Factor::Vol as i32,
                    relative: true,
                    steps: vec![0.0, 1.0],
                },
                // Theta-roll axis: roll calendar time forward (always absolute).
                ShockAxis {
                    factor: shock_axis::Factor::Time as i32,
                    relative: false,
                    steps: vec![0.0, 1.0 / 365.0, 3.0 / 365.0],
                },
            ],
            expiry_years: 0.25,
            risk_buckets: Some(RiskBucketRequest {
                vega_pillars: vec![
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: 0.50,
                        vega: 0.0,
                    },
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: 0.25,
                        vega: 0.0,
                    },
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: -0.25,
                        vega: 0.0,
                    },
                ],
                cross_gamma_pairs: vec![CrossGamma {
                    factor_a: shock_axis::Factor::Spot as i32,
                    factor_b: shock_axis::Factor::Vol as i32,
                    value: 0.0,
                }],
                roll_horizons_years: vec![1.0 / 365.0, 3.0 / 365.0],
            }),
            smile_model: Some(SmileModel::Parametric as i32),
        };
        round_trip(&req);

        // Scenario request without the book-shaped risk decomposition.
        let req_grid_only = ScenarioRequest {
            instrument: Some(vanilla_instrument()),
            base_market: Some(MarketContext::fx(1.10, 0.0825, 0.045, 0.030)),
            conventions: Some(sample_conventions()),
            axes: vec![ShockAxis {
                factor: shock_axis::Factor::Spot as i32,
                relative: true,
                steps: vec![0.0],
            }],
            expiry_years: 0.25,
            risk_buckets: None,
            smile_model: None,
        };
        round_trip(&req_grid_only);

        let resp = ScenarioResponse {
            points: vec![ScenarioPoint {
                applied_shocks: vec![-0.20, 0.0, 1.0 / 365.0],
                shocked_market: Some(MarketContext::fx(0.88, 0.0825, 0.045, 0.030)),
                greeks: Some(sample_greeks()),
                expiry_years: 0.25 - 1.0 / 365.0,
            }],
            bucketed_risk: Some(BucketedRisk {
                vega_buckets: vec![
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: 0.50,
                        vega: 0.0305,
                    },
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: 0.25,
                        vega: 0.0142,
                    },
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: -0.25,
                        vega: 0.0138,
                    },
                ],
                cross_gammas: vec![CrossGamma {
                    factor_a: shock_axis::Factor::Spot as i32,
                    factor_b: shock_axis::Factor::Vol as i32,
                    value: -0.072,
                }],
                theta_roll: vec![0.012_28, 0.012_10],
                roll_horizons_years: vec![1.0 / 365.0, 3.0 / 365.0],
            }),
        };
        round_trip(&resp);
    }

    #[test]
    fn strike_or_delta_each_variant_round_trips() {
        for spec in [
            strike_or_delta::Spec::Strike(1.105),
            strike_or_delta::Spec::Delta(0.25),
        ] {
            round_trip(&StrikeOrDelta { spec: Some(spec) });
        }
    }

    #[test]
    fn round_trip_tradable_token_each_side() {
        for (side, premium) in [(Side::Sell, 0.0081), (Side::Buy, 0.0089)] {
            round_trip(&TradableToken {
                token: 0xFEED,
                side: side as i32,
                premium,
                valid_until_nanos: 1_717_000_000_500_000_000,
            });
        }
    }

    #[test]
    fn round_trip_stream_reject_each_reason() {
        for reason in [
            stream_reject::Reason::Expired,
            stream_reject::Reason::UnknownToken,
            stream_reject::Reason::AlreadyConsumed,
        ] {
            round_trip(&StreamReject {
                subscription: Some(SubscriptionId { value: 5 }),
                token: 0xBAD,
                reason: reason as i32,
                correlation_id: Some(9),
                epoch_nanos: 1,
            });
        }
    }

    #[test]
    fn round_trip_bucketed_risk_standalone() {
        // Bucketed risk with every part populated, and with all parts empty,
        // to pin the repeated-field presence in both directions.
        let full = BucketedRisk {
            vega_buckets: vec![VegaBucket {
                tenor_years: 1.0,
                delta: -0.10,
                vega: 0.021,
            }],
            cross_gammas: vec![CrossGamma {
                factor_a: shock_axis::Factor::RateDom as i32,
                factor_b: shock_axis::Factor::Spot as i32,
                value: 0.004,
            }],
            theta_roll: vec![0.01, 0.009],
            roll_horizons_years: vec![1.0 / 365.0, 2.0 / 365.0],
        };
        round_trip(&full);
        round_trip(&BucketedRisk::default());
    }

    #[test]
    fn round_trip_executed_with_and_without_correlation() {
        let sub = SubscriptionId { value: 3 };
        round_trip(&Executed {
            subscription: Some(sub),
            token: 1,
            execution_id: 100,
            side: Side::Sell as i32,
            traded_premium: 0.0081,
            correlation_id: Some(77),
            epoch_nanos: 9,
            attribution: Some(sample_attribution()),
        });
        round_trip(&Executed {
            subscription: Some(sub),
            token: 1,
            execution_id: 100,
            side: Side::Sell as i32,
            traded_premium: 0.0081,
            correlation_id: None,
            epoch_nanos: 9,
            attribution: None,
        });
    }

    #[test]
    fn round_trip_market_series_messages() {
        let sub = SubscriptionId { value: 88 };
        // Open a market series (ATM-vol history for a 3M EURUSD pillar).
        let open = ClientStreamMessage {
            message: Some(client_stream_message::Message::MarketSeriesSubscribe(
                MarketSeriesSubscribe {
                    subscription: Some(sub),
                    underlying: Some(sample_underlying()),
                    observable: MarketObservable::AtmVol as i32,
                    tenor: Some(sample_tenor()),
                    delta: None,
                    throttle_nanos: 1_000_000,
                    history_limit: 64,
                },
            )),
        };
        round_trip(&open);

        // A wing observable carries its delta; a SPOT series carries no tenor.
        let rr = ClientStreamMessage {
            message: Some(client_stream_message::Message::MarketSeriesSubscribe(
                MarketSeriesSubscribe {
                    subscription: Some(sub),
                    underlying: Some(sample_underlying()),
                    observable: MarketObservable::RiskReversal as i32,
                    tenor: Some(sample_tenor()),
                    delta: Some(0.25),
                    throttle_nanos: 0,
                    history_limit: 0,
                },
            )),
        };
        round_trip(&rr);

        let close = ClientStreamMessage {
            message: Some(client_stream_message::Message::MarketSeriesUnsubscribe(
                MarketSeriesUnsubscribe {
                    subscription: Some(sub),
                },
            )),
        };
        round_trip(&close);

        let point = MarketSeriesPoint {
            subscription: Some(sub),
            sequence: 5,
            value: 0.0832,
            epoch_nanos: 1_717_000_000_000_000_000,
        };
        let snap = ServerStreamMessage {
            message: Some(server_stream_message::Message::MarketSeriesSnapshot(
                MarketSeriesSnapshot {
                    subscription: Some(sub),
                    sequence: 1,
                    underlying: Some(sample_underlying()),
                    observable: MarketObservable::AtmVol as i32,
                    points: vec![point],
                    epoch_nanos: 1_717_000_000_000_000_000,
                },
            )),
        };
        round_trip(&snap);
        round_trip(&ServerStreamMessage {
            message: Some(server_stream_message::Message::MarketSeriesPoint(point)),
        });
    }
}
