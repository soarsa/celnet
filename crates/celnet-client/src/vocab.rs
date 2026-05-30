//! The typed, ergonomic vocabulary the SDK speaks — celnet domain terms, never
//! raw proto.
//!
//! A caller builds requests from [`celnet_types`] vocabulary (a [`CcyPair`], a
//! [`Tenor`], the convention enums) and an [`InstrumentSpec`], and receives typed
//! result structs ([`Quote`], [`Execution`], [`Smile`], [`ScenarioGrid`],
//! [`PricedLine`]) whose numeric fields are plain `f64`s carrying
//! [`celnet_types::Greeks`] — not `Option<…>`-wrapped proto messages. The mapping
//! between this vocabulary and the wire ([`celnet_proto`]) lives entirely here, so
//! the proto types never leak into a caller's code and the two layers cannot
//! drift.
//!
//! The instrument builder mirrors the unified wire `Instrument` oneof (vanilla /
//! strategy / barrier / digital / touch) but in a fluent, strongly-typed form: a
//! [`StrikeSpec`] is `Absolute(k)` or `Delta(d)` (never a bare `f64` with an
//! implicit meaning), a [`Strategy`] carries typed [`Leg`]s, and a [`Conventions`]
//! is assembled from the [`celnet_types`] convention enums with sensible
//! market-standard defaults.

use celnet_proto::{instrument, strike_or_delta};
use celnet_types::{
    AtmConvention, CcyPair, Cut, DayCount, DeltaConvention, Greeks, OptionType, PremiumStyle,
    Settlement, Tenor,
};

use crate::error::{ClientError, ClientResult};

/// The trade conventions a quote / stream / surface is expressed under — the
/// celnet vocabulary form of the wire `Conventions`. Carrying these explicitly on
/// every request is a Celnet differentiator: the caller always states *why* a
/// delta or premium is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conventions {
    /// The delta convention a reported / requested delta is measured against.
    pub delta: DeltaConvention,
    /// The ATM strike convention.
    pub atm: AtmConvention,
    /// The premium quotation style bids/offers are expressed in.
    pub premium: PremiumStyle,
    /// The expiry cut (fixing time).
    pub cut: Cut,
    /// The vol-time day-count basis.
    pub day_count: DayCount,
    /// The settlement style (deliverable / non-deliverable).
    pub settlement: Settlement,
}

impl Conventions {
    /// The market-standard interbank OTC default for a major pair: spot
    /// premium-unadjusted delta, ATM-forward, domestic-pips premium, NY 10:00
    /// cut, Act/365F, deliverable. A desk overrides only what differs.
    #[must_use]
    pub fn major_default() -> Self {
        Self {
            delta: DeltaConvention::SpotUnadjusted,
            atm: AtmConvention::AtmForward,
            premium: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count: DayCount::Act365Fixed,
            settlement: Settlement::Deliverable,
        }
    }

    /// Builder override: set the delta convention.
    #[must_use]
    pub fn with_delta(mut self, delta: DeltaConvention) -> Self {
        self.delta = delta;
        self
    }

    /// Builder override: set the ATM convention.
    #[must_use]
    pub fn with_atm(mut self, atm: AtmConvention) -> Self {
        self.atm = atm;
        self
    }

    /// Builder override: set the premium style.
    #[must_use]
    pub fn with_premium(mut self, premium: PremiumStyle) -> Self {
        self.premium = premium;
        self
    }

    /// Encode to the wire conventions message.
    #[must_use]
    pub(crate) fn to_wire(self) -> celnet_proto::Conventions {
        celnet_proto::Conventions {
            delta_convention: celnet_proto::DeltaConvention::from(self.delta) as i32,
            atm_convention: celnet_proto::AtmConvention::from(self.atm) as i32,
            premium_style: celnet_proto::PremiumStyle::from(self.premium) as i32,
            cut: celnet_proto::Cut::from(self.cut) as i32,
            day_count: celnet_proto::DayCount::from(self.day_count) as i32,
            settlement: celnet_proto::Settlement::from(self.settlement) as i32,
        }
    }

    /// Decode the wire conventions, mapping out-of-range enum tags to a
    /// [`ClientError::Wire`].
    pub(crate) fn from_wire(w: &celnet_proto::Conventions) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let delta = celnet_proto::DeltaConvention::try_from(w.delta_convention)
            .map_err(|_| WireError::UnknownEnum {
                kind: "DeltaConvention",
                tag: w.delta_convention,
            })?
            .into();
        let atm = celnet_proto::AtmConvention::try_from(w.atm_convention)
            .map_err(|_| WireError::UnknownEnum {
                kind: "AtmConvention",
                tag: w.atm_convention,
            })?
            .into();
        let premium = celnet_proto::PremiumStyle::try_from(w.premium_style)
            .map_err(|_| WireError::UnknownEnum {
                kind: "PremiumStyle",
                tag: w.premium_style,
            })?
            .into();
        let cut = celnet_proto::Cut::try_from(w.cut)
            .map_err(|_| WireError::UnknownEnum {
                kind: "Cut",
                tag: w.cut,
            })?
            .into();
        let day_count = celnet_proto::DayCount::try_from(w.day_count)
            .map_err(|_| WireError::UnknownEnum {
                kind: "DayCount",
                tag: w.day_count,
            })?
            .into();
        let settlement = celnet_proto::Settlement::try_from(w.settlement)
            .map_err(|_| WireError::UnknownEnum {
                kind: "Settlement",
                tag: w.settlement,
            })?
            .into();
        Ok(Self {
            delta,
            atm,
            premium,
            cut,
            day_count,
            settlement,
        })
    }
}

/// A strike expressed either as an absolute level or as a signed convention delta
/// (e.g. `Delta(0.25)` for a 25-delta call, `Delta(-0.25)` for a 25-delta put) —
/// the typed form of the wire `StrikeOrDelta`. A trader quotes "the 25-delta"
/// without ever computing the strike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrikeSpec {
    /// An absolute strike (quote per 1 unit of base).
    Absolute(f64),
    /// A signed convention delta, resolved to a strike server-side under the
    /// request conventions.
    Delta(f64),
}

impl StrikeSpec {
    fn to_wire(self) -> celnet_proto::StrikeOrDelta {
        let spec = match self {
            StrikeSpec::Absolute(k) => strike_or_delta::Spec::Strike(k),
            StrikeSpec::Delta(d) => strike_or_delta::Spec::Delta(d),
        };
        celnet_proto::StrikeOrDelta { spec: Some(spec) }
    }
}

/// The trade side a leg or top-level instrument takes — the typed form of the
/// wire `Side`. `TwoWay` requests a two-way (bid/offer) market.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Buy the instrument / leg.
    Buy,
    /// Sell the instrument / leg.
    Sell,
    /// Request a two-way bid/offer market (no directional side).
    TwoWay,
}

impl Side {
    pub(crate) fn to_wire(self) -> celnet_proto::Side {
        match self {
            Side::Buy => celnet_proto::Side::Buy,
            Side::Sell => celnet_proto::Side::Sell,
            Side::TwoWay => celnet_proto::Side::TwoWay,
        }
    }

    pub(crate) fn from_wire(w: celnet_proto::Side) -> Self {
        match w {
            celnet_proto::Side::Buy => Side::Buy,
            celnet_proto::Side::Sell => Side::Sell,
            celnet_proto::Side::TwoWay => Side::TwoWay,
        }
    }
}

/// One leg of a multi-leg strategy: a call/put at a strike spec, a side, and a
/// notional ratio relative to the structure's base notional.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leg {
    /// Call or put for this leg.
    pub option: OptionType,
    /// The leg strike (absolute or delta).
    pub strike: StrikeSpec,
    /// Buy or sell this leg.
    pub side: Side,
    /// The leg ratio relative to the base notional (e.g. `1.0`, or `2.0` for a 1×2).
    pub ratio: f64,
}

impl Leg {
    /// A unit (ratio 1.0) leg.
    #[must_use]
    pub fn unit(option: OptionType, strike: StrikeSpec, side: Side) -> Self {
        Self {
            option,
            strike,
            side,
            ratio: 1.0,
        }
    }

    fn to_wire(self) -> celnet_proto::Leg {
        celnet_proto::Leg {
            option_type: celnet_proto::OptionType::from(self.option) as i32,
            strike: Some(self.strike.to_wire()),
            side: self.side.to_wire() as i32,
            ratio: self.ratio,
        }
    }
}

/// The recognized multi-leg strategy template — the typed form of the wire
/// `StrategyKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyKind {
    /// Risk reversal: long one wing, short the opposite wing.
    RiskReversal,
    /// Strangle: both out-of-the-money wings.
    Strangle,
    /// Straddle: a call and a put at the same (ATM) strike.
    Straddle,
    /// Seagull: a three-leg collar-with-wing.
    Seagull,
}

impl StrategyKind {
    fn to_wire(self) -> celnet_proto::StrategyKind {
        match self {
            StrategyKind::RiskReversal => celnet_proto::StrategyKind::RiskReversal,
            StrategyKind::Strangle => celnet_proto::StrategyKind::Strangle,
            StrategyKind::Straddle => celnet_proto::StrategyKind::Straddle,
            StrategyKind::Seagull => celnet_proto::StrategyKind::Seagull,
        }
    }
}

/// Barrier crossing semantics — knock-in or knock-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierKind {
    /// The option activates if the barrier is touched.
    KnockIn,
    /// The option extinguishes if the barrier is touched.
    KnockOut,
}

/// Where a single barrier sits relative to spot at inception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierSide {
    /// Barrier above spot at inception (up-and-*).
    Up,
    /// Barrier below spot at inception (down-and-*).
    Down,
}

/// The touch family for one-/no-/double-touch structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchKind {
    /// Pays if the barrier IS touched before expiry.
    OneTouch,
    /// Pays if the barrier is NOT touched before expiry.
    NoTouch,
    /// Pays if NEITHER of two barriers is touched.
    DoubleNoTouch,
    /// Pays if EITHER of two barriers is touched.
    DoubleOneTouch,
}

/// Digital (binary) settlement style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitalStyle {
    /// Pays a fixed cash amount if in-the-money at expiry.
    CashOrNothing,
    /// Pays one unit of the asset if in-the-money at expiry.
    AssetOrNothing,
}

/// The product payoff of an instrument — the typed form of the wire `Instrument`
/// `product` oneof. Exactly one variant is built per instrument.
#[derive(Debug, Clone, PartialEq)]
pub enum Product {
    /// A vanilla European option.
    Vanilla {
        /// Call or put.
        option: OptionType,
        /// The strike (absolute or delta).
        strike: StrikeSpec,
    },
    /// A multi-leg vol strategy.
    Strategy {
        /// The strategy template the legs realize.
        kind: StrategyKind,
        /// The legs, in book order.
        legs: Vec<Leg>,
    },
    /// A single-barrier knock-in / knock-out option.
    SingleBarrier {
        /// Call or put for the underlying payoff.
        option: OptionType,
        /// The strike (absolute or delta).
        strike: StrikeSpec,
        /// Knock-in or knock-out.
        kind: BarrierKind,
        /// Barrier above or below spot at inception.
        side: BarrierSide,
        /// The barrier level.
        barrier: f64,
        /// Rebate paid on the barrier event.
        rebate: f64,
    },
    /// A double-barrier option bounded by a lower and upper barrier.
    DoubleBarrier {
        /// Call or put for the underlying payoff.
        option: OptionType,
        /// The strike (absolute or delta).
        strike: StrikeSpec,
        /// Knock-in or knock-out.
        kind: BarrierKind,
        /// The lower barrier level.
        lower: f64,
        /// The upper barrier level.
        upper: f64,
        /// Rebate paid on the barrier event.
        rebate: f64,
    },
    /// A digital (binary) option.
    Digital {
        /// Call or put.
        option: OptionType,
        /// The strike (absolute level).
        strike: f64,
        /// Cash-or-nothing vs asset-or-nothing.
        style: DigitalStyle,
        /// The fixed payout amount.
        payout: f64,
    },
    /// A touch structure (one-/no-/double-no-/double-one-touch).
    Touch {
        /// The touch family.
        kind: TouchKind,
        /// The (lower / sole) barrier level.
        lower: f64,
        /// The upper barrier level (double structures only; ignored otherwise).
        upper: f64,
        /// The rebate paid when the touch condition is satisfied.
        rebate: f64,
    },
}

impl Product {
    fn to_wire(&self) -> instrument::Product {
        match self {
            Product::Vanilla { option, strike } => {
                instrument::Product::Vanilla(celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: Some(strike.to_wire()),
                })
            }
            Product::Strategy { kind, legs } => {
                instrument::Product::Strategy(celnet_proto::Strategy {
                    kind: kind.to_wire() as i32,
                    legs: legs.iter().map(|l| l.to_wire()).collect(),
                })
            }
            Product::SingleBarrier {
                option,
                strike,
                kind,
                side,
                barrier,
                rebate,
            } => instrument::Product::SingleBarrier(celnet_proto::SingleBarrier {
                vanilla: Some(celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: Some(strike.to_wire()),
                }),
                kind: match kind {
                    BarrierKind::KnockIn => celnet_proto::BarrierKind::KnockIn,
                    BarrierKind::KnockOut => celnet_proto::BarrierKind::KnockOut,
                } as i32,
                side: match side {
                    BarrierSide::Up => celnet_proto::BarrierSide::Up,
                    BarrierSide::Down => celnet_proto::BarrierSide::Down,
                } as i32,
                barrier: *barrier,
                rebate: *rebate,
                monitoring: celnet_proto::MonitoringStyle::Continuous as i32,
            }),
            Product::DoubleBarrier {
                option,
                strike,
                kind,
                lower,
                upper,
                rebate,
            } => instrument::Product::DoubleBarrier(celnet_proto::DoubleBarrier {
                vanilla: Some(celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: Some(strike.to_wire()),
                }),
                kind: match kind {
                    BarrierKind::KnockIn => celnet_proto::BarrierKind::KnockIn,
                    BarrierKind::KnockOut => celnet_proto::BarrierKind::KnockOut,
                } as i32,
                lower_barrier: *lower,
                upper_barrier: *upper,
                rebate: *rebate,
                monitoring: celnet_proto::MonitoringStyle::Continuous as i32,
            }),
            Product::Digital {
                option,
                strike,
                style,
                payout,
            } => instrument::Product::Digital(celnet_proto::Digital {
                option_type: celnet_proto::OptionType::from(*option) as i32,
                strike: *strike,
                style: match style {
                    DigitalStyle::CashOrNothing => celnet_proto::DigitalStyle::CashOrNothing,
                    DigitalStyle::AssetOrNothing => celnet_proto::DigitalStyle::AssetOrNothing,
                } as i32,
                payout: *payout,
            }),
            Product::Touch {
                kind,
                lower,
                upper,
                rebate,
            } => instrument::Product::Touch(celnet_proto::Touch {
                kind: match kind {
                    TouchKind::OneTouch => celnet_proto::TouchKind::OneTouch,
                    TouchKind::NoTouch => celnet_proto::TouchKind::NoTouch,
                    TouchKind::DoubleNoTouch => celnet_proto::TouchKind::DoubleNoTouch,
                    TouchKind::DoubleOneTouch => celnet_proto::TouchKind::DoubleOneTouch,
                } as i32,
                lower_barrier: *lower,
                upper_barrier: *upper,
                rebate: *rebate,
                monitoring: celnet_proto::MonitoringStyle::Continuous as i32,
            }),
        }
    }
}

/// A fully-specified instrument the SDK can quote, stream, or reprice — the typed
/// form of the unified wire `Instrument`.
///
/// Built fluently from a [`CcyPair`], a [`Tenor`] + expiry year-fraction, a
/// notional [`Quantity`], a top-level [`Side`], and a [`Product`]. The
/// `expiry_years` is authoritative for pricing; the `tenor` is the trader-facing
/// label.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentSpec {
    /// The currency pair the instrument trades.
    pub pair: CcyPair,
    /// The trader-facing tenor label.
    pub tenor: Tenor,
    /// The expiry as a year fraction (authoritative for pricing).
    pub expiry_years: f64,
    /// The trade notional and its currency leg.
    pub quantity: Quantity,
    /// The top-level side (or `TwoWay` to request a two-way market).
    pub side: Side,
    /// The product payoff.
    pub product: Product,
}

/// A trade notional and the currency leg it is denominated in — the typed form of
/// the wire `Quantity`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    /// The notional amount (always positive; direction is the [`Side`]).
    pub notional: f64,
    /// True if the notional is in the base/foreign currency (CCY1); false for the
    /// quote/domestic currency (CCY2).
    pub base_ccy: bool,
}

impl Quantity {
    /// A base-currency notional (the common case: "1mm EUR" of EURUSD).
    #[must_use]
    pub fn base(notional: f64) -> Self {
        Self {
            notional,
            base_ccy: true,
        }
    }
}

impl InstrumentSpec {
    /// A vanilla European option of the given pair / tenor / expiry / notional.
    #[must_use]
    pub fn vanilla(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        option: OptionType,
        strike: StrikeSpec,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            product: Product::Vanilla { option, strike },
        }
    }

    /// A multi-leg strategy of the given pair / tenor / expiry / notional.
    #[must_use]
    pub fn strategy(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        kind: StrategyKind,
        legs: Vec<Leg>,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            product: Product::Strategy { kind, legs },
        }
    }

    /// Encode to the wire instrument message. `solve` is left unset (the SDK
    /// exposes solve via a dedicated future iteration; the explicit strikes the
    /// caller supplies are used as given).
    #[must_use]
    pub(crate) fn to_wire(&self) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            pair: Some(celnet_proto::CcyPair::from(self.pair)),
            tenor: Some(celnet_proto::Tenor::from(self.tenor)),
            expiry_years: self.expiry_years,
            quantity: Some(celnet_proto::Quantity {
                notional: self.quantity.notional,
                base_ccy: self.quantity.base_ccy,
            }),
            side: self.side.to_wire() as i32,
            solve: None,
            product: Some(self.product.to_wire()),
        }
    }
}

/// A two-way (bid / offer) market in the request's premium-style units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoWay {
    /// The bid premium (the price the maker buys at).
    pub bid: f64,
    /// The offer / ask premium (the price the maker sells at).
    pub offer: f64,
}

impl TwoWay {
    /// The arithmetic mid of the two-way.
    #[must_use]
    pub fn mid(&self) -> f64 {
        0.5 * (self.bid + self.offer)
    }

    pub(crate) fn from_wire(w: &celnet_proto::TwoWayPrice) -> Self {
        Self {
            bid: w.bid,
            offer: w.offer,
        }
    }
}

/// Decode a wire `Greeks` (which is field-identical to [`celnet_types::Greeks`]).
pub(crate) fn greeks_from_wire(w: &celnet_proto::Greeks) -> Greeks {
    Greeks::from(*w)
}

/// A tradable two-way quote returned for an RFQ — the typed form of the wire
/// `Quote`. Carries the assigned `quote_id`, the two-way, the full Greek set, the
/// resolved strike (if a delta/solve was used), and the last-look deadline so a
/// caller knows how long it has to accept.
#[derive(Debug, Clone)]
pub struct Quote {
    /// The server-assigned stable quote id (the handle for accept/reject).
    pub quote_id: u64,
    /// The idempotency key this quote was issued under.
    pub idempotency_key: String,
    /// The two-way bid/offer in the request's premium units.
    pub price: TwoWay,
    /// The full 14-member Greek set for the quoted instrument.
    pub greeks: Greeks,
    /// The conventions the quote is expressed under (resolved server-side).
    pub conventions: Conventions,
    /// The strike the pricing resolved to (input strike, or the solved strike for
    /// a delta key).
    pub resolved_strike: f64,
    /// Publication time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
    /// Last-look validity deadline, nanoseconds since the Unix epoch (UTC). An
    /// accept after this instant is rejected as expired.
    pub valid_until_nanos: i64,
}

impl Quote {
    pub(crate) fn from_wire(w: celnet_proto::Quote) -> ClientResult<Self> {
        let price = w
            .price
            .as_ref()
            .map(TwoWay::from_wire)
            .ok_or(ClientError::MissingField("Quote.price"))?;
        let greeks = w
            .greeks
            .as_ref()
            .map(greeks_from_wire)
            .ok_or(ClientError::MissingField("Quote.greeks"))?;
        let conventions = w
            .conventions
            .as_ref()
            .ok_or(ClientError::MissingField("Quote.conventions"))
            .and_then(Conventions::from_wire)?;
        Ok(Self {
            quote_id: w.quote_id,
            idempotency_key: w.idempotency_key,
            price,
            greeks,
            conventions,
            resolved_strike: w.resolved_strike,
            epoch_nanos: w.epoch_nanos,
            valid_until_nanos: w.valid_until_nanos,
        })
    }
}

/// A booking confirmation produced by accepting a quote — the typed form of the
/// wire `Execution`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Execution {
    /// The server-assigned booking id.
    pub execution_id: u64,
    /// The quote that was traded.
    pub quote_id: u64,
    /// The side actually traded.
    pub side: Side,
    /// The premium traded (the lifted/hit side of the two-way).
    pub traded_premium: f64,
    /// Booking time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl Execution {
    pub(crate) fn from_wire(w: celnet_proto::Execution) -> ClientResult<Self> {
        let side = celnet_proto::Side::try_from(w.side)
            .map(Side::from_wire)
            .map_err(|_| {
                ClientError::Wire(celnet_proto::convert::WireError::UnknownEnum {
                    kind: "Side",
                    tag: w.side,
                })
            })?;
        Ok(Self {
            execution_id: w.execution_id,
            quote_id: w.quote_id,
            side,
            traded_premium: w.traded_premium,
            epoch_nanos: w.epoch_nanos,
        })
    }
}

/// The acknowledgement of a rejected quote — the typed form of the wire
/// `RejectAck`. A reject never books a trade, so it returns this purpose-typed ack
/// (not an [`Execution`]); after it, the quote can no longer be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectAck {
    /// The quote that was declined.
    pub quote_id: u64,
    /// Acknowledgement time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl RejectAck {
    pub(crate) fn from_wire(w: celnet_proto::RejectAck) -> Self {
        Self {
            quote_id: w.quote_id,
            epoch_nanos: w.epoch_nanos,
        }
    }
}

/// A priced line for a single instrument — the typed form of the wire
/// `PriceResponse`: the full Greek set, the resolved strike, and the conventions.
#[derive(Debug, Clone, Copy)]
pub struct PricedLine {
    /// The full 14-member Greek set.
    pub greeks: Greeks,
    /// The strike the pricing resolved to.
    pub resolved_strike: f64,
    /// The conventions the result is expressed under.
    pub conventions: Conventions,
}
