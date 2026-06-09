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

use celnet_proto::{instrument, owner, strike_or_delta};
use celnet_types::{
    AtmConvention, CcyPair, Cut, DayCount, DeltaConvention, Greeks, OptionType, PremiumStyle,
    Settlement, SmileModel, Tenor,
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

/// The seat responsible for a quoted/traded line — a human trading seat or an
/// automated pricer — the typed form of the wire [`celnet_proto::Owner`]. Modelled
/// as a sum type so a who's-trading roll-up treats human and machine flow uniformly
/// while still distinguishing them for governance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    /// A human trading seat (a trader / desk-member identifier).
    Trader(String),
    /// An automated pricer (a pricer / strategy identifier) — auto-quoted flow.
    AutoPricer(String),
}

impl Seat {
    /// A human trading seat keyed by `id`.
    pub fn trader(id: impl Into<String>) -> Self {
        Seat::Trader(id.into())
    }

    /// An automated pricer keyed by `id`.
    pub fn auto_pricer(id: impl Into<String>) -> Self {
        Seat::AutoPricer(id.into())
    }

    fn to_wire(&self) -> celnet_proto::Owner {
        let seat = match self {
            Seat::Trader(id) => owner::Seat::Trader(id.clone()),
            Seat::AutoPricer(id) => owner::Seat::AutoPricer(id.clone()),
        };
        celnet_proto::Owner { seat: Some(seat) }
    }

    fn from_wire(w: &celnet_proto::Owner) -> ClientResult<Self> {
        match w.seat.as_ref() {
            Some(owner::Seat::Trader(id)) => Ok(Seat::Trader(id.clone())),
            Some(owner::Seat::AutoPricer(id)) => Ok(Seat::AutoPricer(id.clone())),
            None => Err(ClientError::MissingField("Owner.seat")),
        }
    }
}

/// The book a quoted/traded line belongs to and the [`Seat`] that owns it — the
/// typed form of the wire [`celnet_proto::BookId`]. This is the identity dimension
/// the who's-trading risk roll-up keys on (a book is a cube dimension).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookId {
    /// The stable book identifier (desk/book key, e.g. `"EM-VOL-1"`).
    pub book: String,
    /// The seat that owns the book — a human trader or an auto-pricer.
    pub owner: Seat,
}

impl BookId {
    /// A book `book` owned by `owner`.
    pub fn new(book: impl Into<String>, owner: Seat) -> Self {
        Self {
            book: book.into(),
            owner,
        }
    }

    fn to_wire(&self) -> celnet_proto::BookId {
        celnet_proto::BookId {
            book: self.book.clone(),
            owner: Some(self.owner.to_wire()),
        }
    }

    fn from_wire(w: &celnet_proto::BookId) -> ClientResult<Self> {
        let owner = w
            .owner
            .as_ref()
            .ok_or(ClientError::MissingField("BookId.owner"))
            .and_then(Seat::from_wire)?;
        Ok(Self {
            book: w.book.clone(),
            owner,
        })
    }
}

/// The who's-trading attribution chain carried alongside a quote/trade — the typed
/// form of the wire [`celnet_proto::AttributionRecord`].
///
/// A caller sets `quoted_by` on an [`crate::Rfq`] request or a stream
/// [`crate::StreamSession::subscribe`] to declare the requesting seat; the server
/// resolves the full chain (the maker that priced the line in `quoted_by`, the
/// requesting seat in `held_by` once a trade books, the `won` flag, and the
/// LP-in-competition `lp_count`) and echoes it on the [`Quote`] / [`Execution`] /
/// stream snapshot / fill so a blotter's attribution column has its identity. Every
/// part beyond `quoted_by` is presence-tracked — absent until the server knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    /// The book/seat that quoted this line (priced and showed the market).
    pub quoted_by: BookId,
    /// The book/seat that holds the position once the trade books, if known.
    pub held_by: Option<BookId>,
    /// Whether this quote won the trade in competition, if the outcome is known
    /// (`Some(true)` ⇒ this seat won; `Some(false)` ⇒ another LP won).
    pub won: Option<bool>,
    /// The number of liquidity providers in competition for this line, if reported
    /// (`Some(1)` ⇒ sole quote).
    pub lp_count: Option<u32>,
}

impl Attribution {
    /// An attribution chain asserting only the requesting/quoting `seat` — the
    /// minimal form a client supplies on a request to declare who it is trading on
    /// behalf of. The server resolves the rest of the chain.
    #[must_use]
    pub fn quoted_by(quoted_by: BookId) -> Self {
        Self {
            quoted_by,
            held_by: None,
            won: None,
            lp_count: None,
        }
    }

    pub(crate) fn to_wire(&self) -> celnet_proto::AttributionRecord {
        celnet_proto::AttributionRecord {
            quoted_by: Some(self.quoted_by.to_wire()),
            held_by: self.held_by.as_ref().map(BookId::to_wire),
            won: self.won,
            lp_count: self.lp_count,
        }
    }

    pub(crate) fn from_wire(w: &celnet_proto::AttributionRecord) -> ClientResult<Self> {
        let quoted_by = w
            .quoted_by
            .as_ref()
            .ok_or(ClientError::MissingField("AttributionRecord.quoted_by"))
            .and_then(BookId::from_wire)?;
        let held_by = w.held_by.as_ref().map(BookId::from_wire).transpose()?;
        Ok(Self {
            quoted_by,
            held_by,
            won: w.won,
            lp_count: w.lp_count,
        })
    }
}

/// The smile/surface calibration model a mark or scenario is computed under — the
/// typed form of the wire [`celnet_proto::SmileModel`] (re-exporting
/// [`celnet_types::SmileModel`]).
///
/// The names describe the *purpose* of each model, never a vendor or method
/// (guardrail #8); the mathematical provenance lives in the doc comments. The
/// default ([`Calibration::MarketHedge`]) preserves the server's current
/// calibration behaviour when a request does not select a model.
pub type Calibration = SmileModel;

/// Encode a [`Calibration`] selector to its wire tag.
pub(crate) fn calibration_to_wire(c: Calibration) -> i32 {
    celnet_proto::SmileModel::from(c) as i32
}

/// Decode a wire [`celnet_proto::SmileModel`] tag into the typed [`Calibration`].
/// An unknown tag (a contract the client does not understand) maps to the
/// market-hedge baseline rather than failing — the calibration family is
/// provenance, not a correctness gate, and the smile itself is already valid.
pub(crate) fn calibration_from_wire(tag: i32) -> Calibration {
    match celnet_proto::SmileModel::try_from(tag) {
        Ok(wire) => SmileModel::try_from(wire).unwrap_or(SmileModel::MarketHedge),
        Err(_) => SmileModel::MarketHedge,
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

/// The booking / pricing model an instrument is priced under — the typed form of
/// the wire `PricingModel`. A *pricing directive* (not an API version): the same
/// instrument prices identically under the same model on every transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PricingModel {
    /// The default analytic / closed-form engine. Byte-identical to the contract
    /// before this directive existed; the default when unset.
    #[default]
    Default,
    /// The local-stochastic-volatility booking model (particle-calibrated
    /// leverage over a stochastic-variance backbone, ADI-PDE / Monte-Carlo).
    /// Supported only for vanilla, single-barrier knock-out, and window-barrier;
    /// any other product is a hard `INVALID_ARGUMENT`.
    LocalStochVol,
}

impl PricingModel {
    fn to_wire(self) -> celnet_proto::PricingModel {
        match self {
            PricingModel::Default => celnet_proto::PricingModel::Default,
            PricingModel::LocalStochVol => celnet_proto::PricingModel::LocalStochVol,
        }
    }
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

/// The payoff terms of a single-barrier option: the underlying vanilla
/// option/strike, the knock style, the barrier side, the barrier level, and the
/// rebate. Built via [`BarrierTerms::new`] then optionally [`BarrierTerms::rebate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarrierTerms {
    /// Call or put for the underlying payoff.
    pub option: OptionType,
    /// The strike (absolute or delta).
    pub strike: StrikeSpec,
    /// Knock-in or knock-out.
    pub kind: BarrierKind,
    /// Barrier above or below spot at inception.
    pub barrier_side: BarrierSide,
    /// The barrier level (quote per 1 unit of base).
    pub barrier: f64,
    /// Rebate paid on the barrier event (`0.0` for a plain barrier).
    pub rebate: f64,
}

impl BarrierTerms {
    /// A plain (zero-rebate) single barrier with the given payoff, knock style,
    /// side, and barrier level.
    #[must_use]
    pub fn new(
        option: OptionType,
        strike: StrikeSpec,
        kind: BarrierKind,
        barrier_side: BarrierSide,
        barrier: f64,
    ) -> Self {
        Self {
            option,
            strike,
            kind,
            barrier_side,
            barrier,
            rebate: 0.0,
        }
    }

    /// Set the rebate paid on the barrier event.
    #[must_use]
    pub fn rebate(mut self, rebate: f64) -> Self {
        self.rebate = rebate;
        self
    }
}

/// The payoff terms of a double-barrier option: the underlying vanilla
/// option/strike, the knock style, the lower/upper corridor barriers, and the
/// rebate. Built via [`DoubleBarrierTerms::new`] then optionally
/// [`DoubleBarrierTerms::rebate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoubleBarrierTerms {
    /// Call or put for the underlying payoff.
    pub option: OptionType,
    /// The strike (absolute or delta).
    pub strike: StrikeSpec,
    /// Knock-in or knock-out (applied to whichever barrier is touched).
    pub kind: BarrierKind,
    /// The lower barrier level.
    pub lower: f64,
    /// The upper barrier level.
    pub upper: f64,
    /// Rebate paid on the barrier event (`0.0` for a plain barrier).
    pub rebate: f64,
}

impl DoubleBarrierTerms {
    /// A plain (zero-rebate) double barrier with the given payoff, knock style,
    /// and `[lower, upper]` corridor.
    #[must_use]
    pub fn new(
        option: OptionType,
        strike: StrikeSpec,
        kind: BarrierKind,
        lower: f64,
        upper: f64,
    ) -> Self {
        Self {
            option,
            strike,
            kind,
            lower,
            upper,
            rebate: 0.0,
        }
    }

    /// Set the rebate paid on the barrier event.
    #[must_use]
    pub fn rebate(mut self, rebate: f64) -> Self {
        self.rebate = rebate;
        self
    }
}

/// The payoff terms of a digital (binary) option: the option direction, the
/// strike, the settlement style, and the fixed payout. Built via
/// [`DigitalTerms::cash_or_nothing`] / [`DigitalTerms::asset_or_nothing`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigitalTerms {
    /// Call or put (above-strike vs below-strike payoff).
    pub option: OptionType,
    /// The strike (absolute level).
    pub strike: f64,
    /// Cash-or-nothing vs asset-or-nothing.
    pub style: DigitalStyle,
    /// The fixed payout amount (domestic ccy for cash-or-nothing; ignored for
    /// asset-or-nothing, which pays one unit of the asset).
    pub payout: f64,
}

impl DigitalTerms {
    /// A cash-or-nothing digital paying a fixed `payout` if in-the-money at expiry.
    #[must_use]
    pub fn cash_or_nothing(option: OptionType, strike: f64, payout: f64) -> Self {
        Self {
            option,
            strike,
            style: DigitalStyle::CashOrNothing,
            payout,
        }
    }

    /// An asset-or-nothing digital paying one unit of the asset if in-the-money at
    /// expiry (the payout field is unused for this style).
    #[must_use]
    pub fn asset_or_nothing(option: OptionType, strike: f64) -> Self {
        Self {
            option,
            strike,
            style: DigitalStyle::AssetOrNothing,
            payout: 0.0,
        }
    }
}

/// The payoff terms of a touch structure: the touch family, the lower / sole
/// barrier, the upper barrier (double structures only), and the rebate. Build via
/// the family conveniences [`TouchTerms::one_touch`] / [`TouchTerms::no_touch`] /
/// [`TouchTerms::double_no_touch`] / [`TouchTerms::double_one_touch`] so the kind
/// and the barrier shape are always consistent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchTerms {
    /// The touch family.
    pub kind: TouchKind,
    /// The (lower / sole) barrier level.
    pub lower: f64,
    /// The upper barrier level (double structures only; `0.0` and ignored for the
    /// single-barrier one-/no-touch kinds).
    pub upper: f64,
    /// The rebate paid when the touch condition is satisfied.
    pub rebate: f64,
}

impl TouchTerms {
    /// A one-touch on a single `barrier` paying `rebate` if it IS touched.
    #[must_use]
    pub fn one_touch(barrier: f64, rebate: f64) -> Self {
        Self {
            kind: TouchKind::OneTouch,
            lower: barrier,
            upper: 0.0,
            rebate,
        }
    }

    /// A no-touch on a single `barrier` paying `rebate` if it is NOT touched.
    #[must_use]
    pub fn no_touch(barrier: f64, rebate: f64) -> Self {
        Self {
            kind: TouchKind::NoTouch,
            lower: barrier,
            upper: 0.0,
            rebate,
        }
    }

    /// A double-no-touch over the `[lower, upper]` corridor paying `rebate` if
    /// NEITHER barrier is touched.
    #[must_use]
    pub fn double_no_touch(lower: f64, upper: f64, rebate: f64) -> Self {
        Self {
            kind: TouchKind::DoubleNoTouch,
            lower,
            upper,
            rebate,
        }
    }

    /// A double-one-touch over the `[lower, upper]` corridor paying `rebate` if
    /// EITHER barrier is touched.
    #[must_use]
    pub fn double_one_touch(lower: f64, upper: f64, rebate: f64) -> Self {
        Self {
            kind: TouchKind::DoubleOneTouch,
            lower,
            upper,
            rebate,
        }
    }
}

/// How the averaging observations of an arithmetic-average-rate Asian are laid
/// out across the averaging window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AveragingStyle {
    /// Equally-spaced discrete observations (a real fixing schedule).
    Discrete {
        /// Number of equally-spaced future (not-yet-fixed) observations, `≥ 1`.
        observations: u32,
    },
    /// Continuous arithmetic averaging over the window (the `n → ∞` limit).
    Continuous,
}

/// Analytic estimator used to price an arithmetic-average-rate Asian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsianMethod {
    /// Geometric-conditioning ("Curran") — the accurate default.
    Curran,
    /// Lognormal two-moment matching ("Turnbull-Wakeman").
    TurnbullWakeman,
}

/// The payoff terms of a fixed-strike arithmetic-average-rate Asian option: the
/// option direction, strike, averaging layout, analytic estimator, and the
/// seasoning state of an in-progress average. Built fluently via
/// [`AsianTerms::fresh_discrete`] / [`AsianTerms::fresh_continuous`] then
/// optionally [`AsianTerms::method`] / [`AsianTerms::seasoned`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsianTerms {
    /// Call or put on the realised arithmetic average.
    pub option: OptionType,
    /// The strike `K` (absolute level).
    pub strike: f64,
    /// How the averaging observations are laid out across the window.
    pub averaging: AveragingStyle,
    /// Analytic estimator to price with.
    pub method: AsianMethod,
    /// Realised running average of the already-fixed observations (ignored when
    /// `elapsed_weight == 0`).
    pub elapsed_avg: f64,
    /// Fraction `∈ [0, 1)` of the total average weight already accumulated.
    pub elapsed_weight: f64,
}

impl AsianTerms {
    /// A fresh discrete-fixing Asian over `observations` equally-spaced future
    /// fixings, priced by the default geometric-conditioning estimator.
    #[must_use]
    pub fn fresh_discrete(option: OptionType, strike: f64, observations: u32) -> Self {
        Self {
            option,
            strike,
            averaging: AveragingStyle::Discrete { observations },
            method: AsianMethod::Curran,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }
    }

    /// A fresh continuously-averaged Asian, priced by the default estimator.
    #[must_use]
    pub fn fresh_continuous(option: OptionType, strike: f64) -> Self {
        Self {
            option,
            strike,
            averaging: AveragingStyle::Continuous,
            method: AsianMethod::Curran,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }
    }

    /// Select the analytic estimator (default [`AsianMethod::Curran`]).
    #[must_use]
    pub fn method(mut self, method: AsianMethod) -> Self {
        self.method = method;
        self
    }

    /// Seed an in-progress (seasoned) average: a fraction `elapsed_weight` of the
    /// total weight has already fixed with realised running average `elapsed_avg`.
    #[must_use]
    pub fn seasoned(mut self, elapsed_avg: f64, elapsed_weight: f64) -> Self {
        self.elapsed_avg = elapsed_avg;
        self.elapsed_weight = elapsed_weight;
        self
    }
}

/// The single payoff a quanto wraps: a plain vanilla or a cash-or-nothing
/// digital that pays a fixed unit of the settlement currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantoPayoff {
    /// A quanto vanilla (call/put on the underlying, settlement-currency cash).
    Vanilla,
    /// A quanto cash-or-nothing digital (pays one settlement-currency unit ITM).
    Digital,
}

/// The payoff terms of a forward-start vanilla: the option direction, the
/// strike-reset multiple, and the reset date. Built via [`ForwardStartTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForwardStartTerms {
    /// Call or put.
    pub option: OptionType,
    /// Strike-reset multiple `m` (`m = 1` is the ATM-forward reset).
    pub moneyness: f64,
    /// Reset (strike-fixing) date `t₁` in years, with `0 ≤ reset ≤ expiry`.
    pub reset: f64,
}

impl ForwardStartTerms {
    /// A forward-start vanilla resetting at `reset` to `moneyness·S(reset)`.
    #[must_use]
    pub fn new(option: OptionType, moneyness: f64, reset: f64) -> Self {
        Self {
            option,
            moneyness,
            reset,
        }
    }
}

/// The terms of a cliquet (ratchet) strip: the per-period option direction and
/// reset multiple, the period count, optional local/global floor/cap clamps, and
/// the Monte-Carlo knobs used only when a clamp is present (a plain ratchet is
/// priced exactly in closed form, so the MC knobs are ignored). Built fluently
/// via [`CliquetTerms::plain`] then optionally [`CliquetTerms::local`] /
/// [`CliquetTerms::global`] / [`CliquetTerms::monte_carlo`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CliquetTerms {
    /// Call (`+1`) or put (`−1`) per-period payoff direction.
    pub option: OptionType,
    /// Per-period strike-reset multiple `m` (applied to every leg).
    pub moneyness: f64,
    /// Number of evenly-spaced ratchet periods over `[0, expiry]`; `≥ 1`.
    pub periods: u32,
    /// Optional per-period local floor on each clamped period option return.
    pub local_floor: Option<f64>,
    /// Optional per-period local cap on each clamped period option return.
    pub local_cap: Option<f64>,
    /// Optional global floor on the accumulated (summed) payoff.
    pub global_floor: Option<f64>,
    /// Optional global cap on the accumulated (summed) payoff.
    pub global_cap: Option<f64>,
    /// Antithetic Monte-Carlo path pairs for the clamped variant; `0` ⇒ server
    /// default. Ignored for a plain ratchet.
    pub mc_pairs: u32,
    /// Counter-RNG seed for the clamped Monte-Carlo estimator (bit-reproducible).
    pub mc_seed: u64,
}

impl CliquetTerms {
    /// A plain (unclamped) ratchet over `periods` evenly-spaced periods, priced
    /// exactly in closed form as the sum of forward-start legs.
    #[must_use]
    pub fn plain(option: OptionType, moneyness: f64, periods: u32) -> Self {
        Self {
            option,
            moneyness,
            periods,
            local_floor: None,
            local_cap: None,
            global_floor: None,
            global_cap: None,
            mc_pairs: 0,
            mc_seed: 0,
        }
    }

    /// Add per-period local floor/cap clamps (switches the pricer to the honest
    /// Monte-Carlo estimator that reports a standard error).
    #[must_use]
    pub fn local(mut self, floor: Option<f64>, cap: Option<f64>) -> Self {
        self.local_floor = floor;
        self.local_cap = cap;
        self
    }

    /// Add a global floor/cap on the accumulated payoff (switches to the Monte-
    /// Carlo estimator).
    #[must_use]
    pub fn global(mut self, floor: Option<f64>, cap: Option<f64>) -> Self {
        self.global_floor = floor;
        self.global_cap = cap;
        self
    }

    /// Configure the Monte-Carlo path pairs and seed used for a clamped cliquet.
    #[must_use]
    pub fn monte_carlo(mut self, pairs: u32, seed: u64) -> Self {
        self.mc_pairs = pairs;
        self.mc_seed = seed;
        self
    }
}

/// The market data a quanto needs beyond the underlying's own inputs: the
/// settlement-conversion-rate volatility and its correlation with the underlying.
/// Built via [`QuantoTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantoTerms {
    /// Whether the wrapped payoff is a vanilla or a cash-or-nothing digital.
    pub payoff: QuantoPayoff,
    /// Call or put.
    pub option: OptionType,
    /// The strike `K` (absolute level).
    pub strike: f64,
    /// Annualised volatility `σ_Z` of the settlement-conversion rate.
    pub conversion_vol: f64,
    /// Instantaneous correlation `ρ ∈ [−1, 1]` between the underlying and the
    /// settlement-conversion rate.
    pub correlation: f64,
}

impl QuantoTerms {
    /// A quanto on the given payoff/option/strike with the conversion-rate vol
    /// and correlation.
    #[must_use]
    pub fn new(
        payoff: QuantoPayoff,
        option: OptionType,
        strike: f64,
        conversion_vol: f64,
        correlation: f64,
    ) -> Self {
        Self {
            payoff,
            option,
            strike,
            conversion_vol,
            correlation,
        }
    }
}

/// How the redeeming (target-breaching) fixing of a TARF settles — the gap-risk
/// convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TarfRedemption {
    /// The breaching fixing pays its full intrinsic gain (the client keeps the
    /// overshoot past the target); the genuine gap exposure.
    FullGain,
    /// The breaching fixing pays only the remaining target (exact redemption).
    CappedGain,
}

/// The knock-out monitoring convention for an accumulator's up-and-out barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccumulatorMonitoring {
    /// Barrier tested only at the discrete fixing dates.
    Discrete,
    /// Barrier monitored continuously between fixings (knocks out more often).
    Continuous,
}

/// The two lookback families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookbackStyle {
    /// Floating strike: settle against the path extremum (always ≥ 0).
    Floating,
    /// Fixed strike: optimal exercise against a fixed `K`.
    Fixed,
}

/// How a lookback's running extremum is monitored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookbackMonitoring {
    /// Continuous monitoring — priced by exact closed form (no MC std-error).
    Continuous,
    /// Discrete monitoring — priced by Monte-Carlo, reporting a std-error.
    Discrete,
}

/// The early-exercise style of an option: continuous (American) or on a discrete
/// set of permitted dates (Bermudan).
#[derive(Debug, Clone, PartialEq)]
pub enum ExerciseStyle {
    /// American: exercise permitted continuously up to and including expiry.
    American,
    /// Bermudan: exercise permitted only on the given dates (year-fractions in
    /// `(0, T]`); expiry is always an exercise opportunity.
    Bermudan {
        /// The permitted exercise dates as year-fractions in `(0, T]`.
        dates: Vec<f64>,
    },
}

/// The terms of an American / Bermudan early-exercise vanilla option: the option
/// direction, the strike, the exercise style, and — when the Longstaff-Schwartz
/// Monte-Carlo engine is selected — the path budget, exercise-date resolution, and
/// seed. The default engine is the projected-SOR free-boundary finite difference
/// (exact, [`PricedLine::price_std_error`] is `None`); selecting
/// [`AmericanTerms::monte_carlo`] routes through the regression Monte-Carlo
/// engine, which surfaces a standard error. Built via [`AmericanTerms::american`]
/// / [`AmericanTerms::bermudan`].
#[derive(Debug, Clone, PartialEq)]
pub struct AmericanTerms {
    /// Call or put.
    pub option: OptionType,
    /// Strike `K` (absolute level).
    pub strike: f64,
    /// Continuous (American) or discrete-date (Bermudan) exercise.
    pub style: ExerciseStyle,
    /// Longstaff-Schwartz Monte-Carlo path count; `0` ⇒ the finite-difference
    /// engine (default, exact). `> 0` ⇒ the LSM engine (surfaces a std-error).
    pub lsm_paths: u32,
    /// For the LSM engine + American style: equally-spaced exercise opportunities
    /// (`0` ⇒ server default). Ignored for FD and Bermudan.
    pub lsm_exercise_dates: u32,
    /// Sobol scramble seed for the LSM engine (bit-reproducible).
    pub lsm_seed: u64,
}

impl AmericanTerms {
    /// An American (continuous-exercise) vanilla, priced by the finite-difference
    /// engine by default.
    #[must_use]
    pub fn american(option: OptionType, strike: f64) -> Self {
        Self {
            option,
            strike,
            style: ExerciseStyle::American,
            lsm_paths: 0,
            lsm_exercise_dates: 0,
            lsm_seed: 0,
        }
    }

    /// A Bermudan (discrete-date) vanilla over `dates` (year-fractions in
    /// `(0, T]`), priced by the finite-difference engine by default.
    #[must_use]
    pub fn bermudan(option: OptionType, strike: f64, dates: Vec<f64>) -> Self {
        Self {
            option,
            strike,
            style: ExerciseStyle::Bermudan { dates },
            lsm_paths: 0,
            lsm_exercise_dates: 0,
            lsm_seed: 0,
        }
    }

    /// Select the Longstaff-Schwartz regression Monte-Carlo engine with `paths`
    /// simulated paths and the given scramble `seed` (the FD engine is the
    /// default; this surfaces a price standard error). `exercise_dates` sets the
    /// equally-spaced exercise resolution for an American option (`0` ⇒ server
    /// default; ignored for Bermudan, whose explicit dates are the grid).
    #[must_use]
    pub fn monte_carlo(mut self, paths: u32, exercise_dates: u32, seed: u64) -> Self {
        self.lsm_paths = paths;
        self.lsm_exercise_dates = exercise_dates;
        self.lsm_seed = seed;
        self
    }
}

/// How the per-leg terminal levels of a correlated multi-asset option combine
/// into the option underlying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasketKind {
    /// Weighted arithmetic basket: the underlying is `Σ_a w_a · S_a(T)`.
    Basket,
    /// Best-of-N (rainbow max): the underlying is `max_a w_a · S_a(T)`.
    BestOf,
    /// Worst-of-N (rainbow min): the underlying is `min_a w_a · S_a(T)`.
    WorstOf,
}

impl BasketKind {
    fn to_wire(self) -> celnet_proto::BasketKind {
        match self {
            BasketKind::Basket => celnet_proto::BasketKind::Basket,
            BasketKind::BestOf => celnet_proto::BasketKind::BestOf,
            BasketKind::WorstOf => celnet_proto::BasketKind::WorstOf,
        }
    }
}

/// One leg of a correlated multi-asset (basket / best-of / worst-of) option: an
/// FX underlying with its own market data and basket weight. A multi-asset
/// instrument carries its per-leg market data IN the leg (the single-pair
/// request market context cannot hold N underlyings); the shared domestic
/// (settlement) rate comes from the request market context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasketLegTerms {
    /// The currency pair of this leg (identifies the underlying).
    pub pair: CcyPair,
    /// The leg weight `w_a` applied to `S_a(T)` (may be negative for a short leg).
    pub weight: f64,
    /// The leg spot FX level `S_a(0)`.
    pub spot: f64,
    /// The leg annualised lognormal volatility `σ_a`.
    pub vol: f64,
    /// The leg continuously-compounded foreign (base) rate `r_f,a`.
    pub r_for: f64,
}

impl BasketLegTerms {
    /// Convenience constructor.
    #[must_use]
    pub fn new(pair: CcyPair, weight: f64, spot: f64, vol: f64, r_for: f64) -> Self {
        Self {
            pair,
            weight,
            spot,
            vol,
            r_for,
        }
    }
}

/// The terms of a correlated multi-asset FX option over N currency-pair legs: a
/// weighted [`BasketKind::Basket`], or a [`BasketKind::BestOf`] /
/// [`BasketKind::WorstOf`] rainbow on the per-leg weighted terminal levels.
/// Priced by a Cholesky-correlated multi-asset GBM Monte-Carlo over the
/// scrambled-Sobol / Brownian-bridge engine, so it surfaces a standard error on
/// [`PricedLine::price_std_error`]. Multi-asset Greeks are a distinct larger
/// increment and are not returned (the strip is zeroed). Built via
/// [`BasketTerms::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct BasketTerms {
    /// The legs (one FX underlying each); at least one. A single weight-1 leg
    /// collapses to a vanilla on that leg.
    pub legs: Vec<BasketLegTerms>,
    /// The N×N instantaneous correlation matrix in ROW-MAJOR order (length N²):
    /// `(i, j)` is `correlations[i*N + j]`. Must be symmetric, unit-diagonal and
    /// positive-definite; a non-SPD matrix is rejected by the server.
    pub correlations: Vec<f64>,
    /// Call or put on the aggregated underlying.
    pub option: OptionType,
    /// The strike `K` on the aggregated underlying.
    pub strike: f64,
    /// The aggregation kind.
    pub kind: BasketKind,
    /// Scrambled-Sobol points per replication (`0` ⇒ server default).
    pub mc_paths: u32,
    /// Independent randomized scrambles (`0` ⇒ server default; `≥ 2` for a
    /// finite std-error).
    pub mc_replications: u32,
    /// Time steps per path (`0` ⇒ server default; `1` suffices for these
    /// European payoffs).
    pub mc_steps: u32,
    /// The base scramble seed (identical seeds reproduce results bit-for-bit).
    pub mc_seed: u64,
}

impl BasketTerms {
    /// A correlated multi-asset option over `legs` with the given row-major
    /// `correlations` matrix (length `legs.len()²`), aggregation `kind`, option
    /// direction and strike. The Monte-Carlo knobs default to the server defaults
    /// (`0`); refine them with [`BasketTerms::monte_carlo`].
    #[must_use]
    pub fn new(
        legs: Vec<BasketLegTerms>,
        correlations: Vec<f64>,
        kind: BasketKind,
        option: OptionType,
        strike: f64,
    ) -> Self {
        Self {
            legs,
            correlations,
            option,
            strike,
            kind,
            mc_paths: 0,
            mc_replications: 0,
            mc_steps: 0,
            mc_seed: 0,
        }
    }

    /// Refine the Monte-Carlo configuration (Sobol points per scramble, scramble
    /// count, time steps and base seed). A `0` selects the server default.
    #[must_use]
    pub fn monte_carlo(
        mut self,
        mc_paths: u32,
        mc_replications: u32,
        mc_steps: u32,
        mc_seed: u64,
    ) -> Self {
        self.mc_paths = mc_paths;
        self.mc_replications = mc_replications;
        self.mc_steps = mc_steps;
        self.mc_seed = mc_seed;
        self
    }
}

/// The terms of a Target-Redemption Forward: the favourable-side direction, the
/// strike, the cumulative knock-out target, the adverse-leg gearing, the gap-risk
/// settlement convention, the fixing schedule (count + per-fixing notional), and
/// the Monte-Carlo knobs (a TARF is always priced by MC, surfacing a standard
/// error on [`PricedLine::price_std_error`]). Built via [`TarfTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TarfTerms {
    /// The favourable-side direction (PUT = client gains when `S < strike`).
    pub option: OptionType,
    /// The strike `K` of every fixing.
    pub strike: f64,
    /// The cumulative gain target; accumulated client gain at/above it redeems.
    pub target: f64,
    /// The gearing/leverage on the adverse (loss) leg (`≥ 0`).
    pub leverage: f64,
    /// The gap-risk settlement convention of the redeeming fixing.
    pub redemption: TarfRedemption,
    /// The number of equally-spaced fixings over `[0, expiry]` (`≥ 1`).
    pub fixings: u32,
    /// The per-fixing notional.
    pub fixing_notional: f64,
    /// Antithetic Monte-Carlo path pairs; `0` ⇒ server default.
    pub mc_pairs: u32,
    /// Counter-RNG seed for the Monte-Carlo estimator (bit-reproducible).
    pub mc_seed: u64,
}

impl TarfTerms {
    /// A TARF with the given economics, an equally-spaced `fixings`-point schedule
    /// of unit per-fixing notional, and a server-default MC budget.
    #[must_use]
    pub fn new(
        option: OptionType,
        strike: f64,
        target: f64,
        leverage: f64,
        redemption: TarfRedemption,
        fixings: u32,
    ) -> Self {
        Self {
            option,
            strike,
            target,
            leverage,
            redemption,
            fixings,
            fixing_notional: 1.0,
            mc_pairs: 0,
            mc_seed: 0,
        }
    }

    /// Set the per-fixing notional.
    #[must_use]
    pub fn fixing_notional(mut self, notional: f64) -> Self {
        self.fixing_notional = notional;
        self
    }

    /// Configure the Monte-Carlo path pairs and seed.
    #[must_use]
    pub fn monte_carlo(mut self, pairs: u32, seed: u64) -> Self {
        self.mc_pairs = pairs;
        self.mc_seed = seed;
        self
    }
}

/// The terms of an accumulator: the pivot strike, the up-and-out knock-out
/// barrier, the below-pivot gearing, the monitoring convention, the fixing
/// schedule, and the Monte-Carlo knobs (an accumulator is always priced by MC,
/// surfacing a standard error on [`PricedLine::price_std_error`]). Built via
/// [`AccumulatorTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccumulatorTerms {
    /// The pivot strike `K` at which the client accumulates each fixing.
    pub pivot: f64,
    /// The up-and-out knock-out barrier `B` (`B > pivot`).
    pub barrier: f64,
    /// The gearing on the below-pivot (loss) leg (`≥ 0`).
    pub leverage: f64,
    /// The knock-out monitoring convention.
    pub monitoring: AccumulatorMonitoring,
    /// The number of equally-spaced fixings over `[0, expiry]` (`≥ 1`).
    pub fixings: u32,
    /// The per-fixing notional.
    pub fixing_notional: f64,
    /// Antithetic Monte-Carlo path pairs; `0` ⇒ server default.
    pub mc_pairs: u32,
    /// Counter-RNG seed for the Monte-Carlo estimator (bit-reproducible).
    pub mc_seed: u64,
}

impl AccumulatorTerms {
    /// An accumulator with the given economics, an equally-spaced `fixings`-point
    /// schedule of unit per-fixing notional, and a server-default MC budget.
    #[must_use]
    pub fn new(
        pivot: f64,
        barrier: f64,
        leverage: f64,
        monitoring: AccumulatorMonitoring,
        fixings: u32,
    ) -> Self {
        Self {
            pivot,
            barrier,
            leverage,
            monitoring,
            fixings,
            fixing_notional: 1.0,
            mc_pairs: 0,
            mc_seed: 0,
        }
    }

    /// Set the per-fixing notional.
    #[must_use]
    pub fn fixing_notional(mut self, notional: f64) -> Self {
        self.fixing_notional = notional;
        self
    }

    /// Configure the Monte-Carlo path pairs and seed.
    #[must_use]
    pub fn monte_carlo(mut self, pairs: u32, seed: u64) -> Self {
        self.mc_pairs = pairs;
        self.mc_seed = seed;
        self
    }
}

/// The terms of a lookback option: the family (floating/fixed), the option
/// direction, the monitoring convention, and — for the discrete-monitoring
/// variant — the observation count and Monte-Carlo knobs. The continuous variant
/// prices in closed form ([`PricedLine::price_std_error`] is `None`); the discrete
/// variant prices by Monte-Carlo and surfaces a standard error. Built via
/// [`LookbackTerms::continuous`] / [`LookbackTerms::discrete`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookbackTerms {
    /// Floating- or fixed-strike family.
    pub style: LookbackStyle,
    /// Call or put.
    pub option: OptionType,
    /// Continuous (closed-form) or discrete (Monte-Carlo) monitoring.
    pub monitoring: LookbackMonitoring,
    /// The strike `K` (used only by the FIXED-strike family).
    pub strike: f64,
    /// The number of equally-spaced monitoring observations for the DISCRETE
    /// variant (`0` ⇒ server default). Ignored for CONTINUOUS monitoring.
    pub observations: u32,
    /// Antithetic Monte-Carlo path pairs for the DISCRETE variant (`0` ⇒ server
    /// default). Ignored for CONTINUOUS monitoring.
    pub mc_pairs: u32,
    /// Counter-RNG seed for the DISCRETE Monte-Carlo estimator. Ignored for
    /// CONTINUOUS monitoring.
    pub mc_seed: u64,
}

impl LookbackTerms {
    /// A continuously-monitored lookback (exact closed form, no MC std-error).
    /// `strike` is used only for the FIXED-strike family.
    #[must_use]
    pub fn continuous(style: LookbackStyle, option: OptionType, strike: f64) -> Self {
        Self {
            style,
            option,
            monitoring: LookbackMonitoring::Continuous,
            strike,
            observations: 0,
            mc_pairs: 0,
            mc_seed: 0,
        }
    }

    /// A discretely-monitored lookback (Monte-Carlo, surfacing a std-error) over
    /// `observations` equally-spaced fixings. `strike` is used only for the
    /// FIXED-strike family.
    #[must_use]
    pub fn discrete(
        style: LookbackStyle,
        option: OptionType,
        strike: f64,
        observations: u32,
    ) -> Self {
        Self {
            style,
            option,
            monitoring: LookbackMonitoring::Discrete,
            strike,
            observations,
            mc_pairs: 0,
            mc_seed: 0,
        }
    }

    /// Configure the Monte-Carlo path pairs and seed for a discrete lookback.
    #[must_use]
    pub fn monte_carlo(mut self, pairs: u32, seed: u64) -> Self {
        self.mc_pairs = pairs;
        self.mc_seed = seed;
        self
    }
}

/// The published settlement-rate option a non-deliverable forward fixes against —
/// the typed form of the wire [`celnet_proto::FixingSource`] (re-exporting
/// [`celnet_types::FixingSource`]).
///
/// This names *which* published rate the contract settles against (each EMTA /
/// ISDA per-currency template names one); it is convention identity, NOT a
/// market-data input — the live fixing VALUE is an estate-gated feed, never
/// sourced in-repo (only the identity travels on the wire).
pub type FixingSource = celnet_types::FixingSource;

/// Encode a [`FixingSource`] identity to its wire tag. Exhaustive so a future
/// fixing added to the domain enum forces a compile error here rather than a
/// silent mis-map.
fn fixing_source_to_wire(f: FixingSource) -> celnet_proto::FixingSource {
    use celnet_proto::FixingSource as W;
    match f {
        FixingSource::KrwKftc18 => W::KrwKftc18,
        FixingSource::TwdTaipei => W::TwdTaipei,
        FixingSource::InrRbiRef => W::InrRbiRef,
        FixingSource::BrlPtax => W::BrlPtax,
        FixingSource::ClpDolarObs => W::ClpDolarObs,
        FixingSource::CopTrm => W::CopTrm,
    }
}

/// The directional side a linear (forward / swap / NDF) product takes — a
/// definite BUY or SELL (a linear PV needs a sign; a two-way request is not a
/// pricing direction). Maps to the wire [`celnet_proto::Side`] on the product
/// message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardSide {
    /// Buy (long the base/asset forward): PV gains as the forward rate rises
    /// above the contract rate.
    Buy,
    /// Sell (short the base/asset forward).
    Sell,
}

impl ForwardSide {
    fn to_wire(self) -> celnet_proto::Side {
        match self {
            ForwardSide::Buy => celnet_proto::Side::Buy,
            ForwardSide::Sell => celnet_proto::Side::Sell,
        }
    }

    /// The top-level instrument [`Side`] a linear product carries (the directional
    /// side, never two-way).
    fn instrument_side(self) -> Side {
        match self {
            ForwardSide::Buy => Side::Buy,
            ForwardSide::Sell => Side::Sell,
        }
    }
}

/// The terms of an FX outright forward (deliverable): the contract (delivery)
/// rate `K`, the notional, and the direction. A linear, closed-form
/// discounted-cashflow product — PV is
/// `side · notional · discount_df(t) · (forward_rate − K)`, exact (no std-error).
/// Built via [`ForwardTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForwardTerms {
    /// The agreed contract (delivery) rate `K`, in quote per 1 unit of base/asset.
    pub contract_rate: f64,
    /// The notional amount (always positive; direction is carried by `side`).
    pub notional: f64,
    /// The direction taken (buy = long the base/asset forward).
    pub side: ForwardSide,
}

impl ForwardTerms {
    /// A forward at `contract_rate` for `notional`, taking `side`.
    #[must_use]
    pub fn new(contract_rate: f64, notional: f64, side: ForwardSide) -> Self {
        Self {
            contract_rate,
            notional,
            side,
        }
    }
}

/// The terms of an FX swap: a near leg + a far leg (two outright forwards trading
/// the opposite direction). The near leg's contract rate, notional and side
/// anchor the swap; the far leg is the opposite side at the same contract rate,
/// settling at the instrument's forward tenor (`expiry_years`), while the near
/// leg settles at the spot date. The swap PV is the sum of the two leg PVs.
/// Built via [`SwapTerms::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapTerms {
    /// The near leg (its contract rate / notional / side drive both legs).
    pub near: ForwardTerms,
}

impl SwapTerms {
    /// A swap whose near leg is `near` (the far leg is formed server-side as the
    /// opposite side at the same contract rate, settling at the forward tenor).
    #[must_use]
    pub fn new(near: ForwardTerms) -> Self {
        Self { near }
    }
}

/// The terms of a non-deliverable forward (NDF): the contract (forward) rate `K`,
/// the notional, the direction, the published settlement-rate option it fixes
/// against, and the convertible (settlement) currency. The risk-neutral PV is
/// identical to a deliverable forward of equal terms (non-deliverability changes
/// only the settlement mechanics), so it is closed-form and exact. The fixing is
/// booking/convention identity only — the live fixing VALUE is an estate-gated
/// feed, never sourced in-repo. Built via [`NdfTerms::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct NdfTerms {
    /// The agreed contract (forward) rate `K`, in settlement-ccy per 1 unit of
    /// base.
    pub contract_rate: f64,
    /// The notional amount (always positive; direction is carried by `side`).
    pub notional: f64,
    /// The direction taken (buy = long the base/asset forward).
    pub side: ForwardSide,
    /// The published settlement-rate option the contract fixes against (identity
    /// only — the live fixing value is never sourced in-repo).
    pub fixing: FixingSource,
    /// The convertible (settlement) currency the net cash settlement is paid in
    /// (a 3-letter code, e.g. `"USD"`).
    pub settlement_ccy: String,
}

impl NdfTerms {
    /// An NDF at `contract_rate` for `notional`, taking `side`, fixing against
    /// `fixing` and cash-settling in `settlement_ccy`.
    #[must_use]
    pub fn new(
        contract_rate: f64,
        notional: f64,
        side: ForwardSide,
        fixing: FixingSource,
        settlement_ccy: impl Into<String>,
    ) -> Self {
        Self {
            contract_rate,
            notional,
            side,
            fixing,
            settlement_ccy: settlement_ccy.into(),
        }
    }
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
    /// A variance swap (fair-variance-strike replication). The response carries
    /// the fair variance strike `K_var` (in `price`/`resolved_strike`) and its
    /// realised-vol equivalent `√K_var` (in `vol`).
    VarianceSwap {
        /// The fixed variance strike as a volatility (variance = `strike_vol²`);
        /// `0.0` to read the fair strike off the response.
        strike_vol: f64,
    },
    /// A volatility swap (convexity-adjusted fair-vol strike). The response
    /// carries the fair vol strike `K_vol` (in `price`/`resolved_strike`/`vol`).
    VolatilitySwap {
        /// The fixed volatility strike (absolute vol); `0.0` to read the fair
        /// strike off the response.
        strike_vol: f64,
    },
    /// A fixed-strike arithmetic-average-rate Asian option.
    AsianOption {
        /// Call or put on the realised arithmetic average.
        option: OptionType,
        /// The strike `K` (absolute level).
        strike: f64,
        /// How the averaging observations are laid out across the window.
        averaging: AveragingStyle,
        /// Analytic estimator to price with.
        method: AsianMethod,
        /// Realised running arithmetic average of the already-fixed observations
        /// (ignored when `elapsed_weight == 0`).
        elapsed_avg: f64,
        /// Fraction `∈ [0, 1)` of the total average weight already accumulated by
        /// the fixed observations; `0` ⇒ a fresh average.
        elapsed_weight: f64,
    },
    /// A forward-start vanilla: the strike fixes at `reset` to `moneyness·S(reset)`
    /// and pays the vanilla payoff at expiry.
    ForwardStart {
        /// Call or put.
        option: OptionType,
        /// Strike-reset multiple `m` (`m = 1` is the ATM-forward reset).
        moneyness: f64,
        /// Reset (strike-fixing) date `t₁` in years, with `0 ≤ reset ≤ expiry`.
        reset: f64,
    },
    /// A cliquet (ratchet): a strip of forward-start legs. The plain (unclamped)
    /// ratchet prices in closed form; any clamp switches to a Monte-Carlo
    /// estimator whose standard error is surfaced on [`PricedLine::price_std_error`].
    Cliquet {
        /// Call or put per-period payoff direction.
        option: OptionType,
        /// Per-period strike-reset multiple `m`.
        moneyness: f64,
        /// Number of evenly-spaced ratchet periods over `[0, expiry]`.
        periods: u32,
        /// Optional per-period local floor on each clamped period option return.
        local_floor: Option<f64>,
        /// Optional per-period local cap on each clamped period option return.
        local_cap: Option<f64>,
        /// Optional global floor on the accumulated payoff.
        global_floor: Option<f64>,
        /// Optional global cap on the accumulated payoff.
        global_cap: Option<f64>,
        /// Antithetic Monte-Carlo path pairs for the clamped variant (`0` ⇒
        /// server default); ignored for a plain ratchet.
        mc_pairs: u32,
        /// Counter-RNG seed for the clamped Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// A quanto option (vanilla or cash-or-nothing digital), settlement-currency
    /// converted at a fixed rate via the quanto-drift adjustment.
    Quanto {
        /// Whether the wrapped payoff is a vanilla or a digital.
        payoff: QuantoPayoff,
        /// Call or put.
        option: OptionType,
        /// The strike `K` (absolute level).
        strike: f64,
        /// Annualised volatility `σ_Z` of the settlement-conversion rate.
        conversion_vol: f64,
        /// Correlation `ρ ∈ [−1, 1]` between the underlying and the conversion rate.
        correlation: f64,
    },
    /// A Target-Redemption Forward (strip of geared fixings with a knock-out
    /// target). Always priced by Monte-Carlo; the std-error is surfaced on
    /// [`PricedLine::price_std_error`].
    Tarf {
        /// The favourable-side direction.
        option: OptionType,
        /// The strike `K` of every fixing.
        strike: f64,
        /// The cumulative gain target.
        target: f64,
        /// The gearing on the adverse (loss) leg.
        leverage: f64,
        /// The gap-risk settlement convention of the redeeming fixing.
        redemption: TarfRedemption,
        /// The number of equally-spaced fixings over `[0, expiry]`.
        fixings: u32,
        /// The per-fixing notional.
        fixing_notional: f64,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ server default).
        mc_pairs: u32,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// An accumulator (periodic pivot accumulation with an up-and-out barrier).
    /// Always priced by Monte-Carlo; the std-error is surfaced on
    /// [`PricedLine::price_std_error`].
    Accumulator {
        /// The pivot strike `K`.
        pivot: f64,
        /// The up-and-out knock-out barrier `B` (`B > pivot`).
        barrier: f64,
        /// The gearing on the below-pivot (loss) leg.
        leverage: f64,
        /// The knock-out monitoring convention.
        monitoring: AccumulatorMonitoring,
        /// The number of equally-spaced fixings over `[0, expiry]`.
        fixings: u32,
        /// The per-fixing notional.
        fixing_notional: f64,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ server default).
        mc_pairs: u32,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// A lookback option (floating/fixed strike). The continuous variant prices
    /// in closed form ([`PricedLine::price_std_error`] is `None`); the discrete
    /// variant prices by Monte-Carlo and surfaces a std-error.
    Lookback {
        /// Floating- or fixed-strike family.
        style: LookbackStyle,
        /// Call or put.
        option: OptionType,
        /// Continuous (closed-form) or discrete (Monte-Carlo) monitoring.
        monitoring: LookbackMonitoring,
        /// The strike `K` (used only by the FIXED-strike family).
        strike: f64,
        /// The number of discrete observations (`0` ⇒ server default; ignored for
        /// continuous monitoring).
        observations: u32,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ server default; ignored for
        /// continuous monitoring).
        mc_pairs: u32,
        /// Counter-RNG seed for the discrete Monte-Carlo estimator (ignored for
        /// continuous monitoring).
        mc_seed: u64,
    },
    /// A window knock-out barrier (continuously monitored only inside a calendar
    /// window). Priced only under [`PricingModel::LocalStochVol`]; the ADI-PDE
    /// engine when `mc_pairs == 0` (exact, no std-error), the Monte-Carlo engine
    /// when `mc_pairs > 0` (surfaces a std-error on [`PricedLine::price_std_error`]).
    WindowBarrier {
        /// Call or put for the underlying terminal payoff.
        option: OptionType,
        /// The strike `K` (absolute).
        strike: f64,
        /// The barrier level `H`.
        barrier: f64,
        /// Barrier above (up-and-out) or below (down-and-out) spot at inception.
        side: BarrierSide,
        /// The start of the active window in years from inception.
        window_start: f64,
        /// The end of the active window in years from inception.
        window_end: f64,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ ADI PDE, exact; `> 0` ⇒
        /// Monte-Carlo with a std-error).
        mc_pairs: u32,
        /// Monte-Carlo time steps (`0` ⇒ server default; ignored when
        /// `mc_pairs == 0`).
        mc_steps: u32,
        /// Counter-RNG seed for the Monte-Carlo estimator (ignored when
        /// `mc_pairs == 0`).
        mc_seed: u64,
    },
    /// An American / Bermudan early-exercise vanilla. Priced by the projected-SOR
    /// free-boundary finite difference (default, exact, no std-error) or — when
    /// `lsm_paths > 0` — the Longstaff-Schwartz regression Monte-Carlo (surfaces a
    /// std-error on [`PricedLine::price_std_error`]).
    American {
        /// Call or put.
        option: OptionType,
        /// Strike `K` (absolute).
        strike: f64,
        /// Continuous (American) or discrete-date (Bermudan) exercise.
        style: ExerciseStyle,
        /// Longstaff-Schwartz path count (`0` ⇒ FD engine, exact; `> 0` ⇒ LSM).
        lsm_paths: u32,
        /// LSM equally-spaced exercise opportunities for American style (`0` ⇒
        /// server default; ignored for FD and Bermudan).
        lsm_exercise_dates: u32,
        /// Sobol scramble seed for the LSM engine (ignored for FD).
        lsm_seed: u64,
    },
    /// A correlated multi-asset FX option (weighted basket / best-of / worst-of)
    /// over N currency-pair legs. Priced by Cholesky-correlated multi-asset GBM
    /// Monte-Carlo (surfaces a std-error on [`PricedLine::price_std_error`]);
    /// multi-asset Greeks are deferred (the strip is zeroed).
    Basket {
        /// The legs (one FX underlying each), each with its own market data.
        legs: Vec<BasketLegTerms>,
        /// The row-major N×N correlation matrix (length N²).
        correlations: Vec<f64>,
        /// Call or put on the aggregated underlying.
        option: OptionType,
        /// The strike `K` on the aggregated underlying.
        strike: f64,
        /// The aggregation kind.
        kind: BasketKind,
        /// Scrambled-Sobol points per replication (`0` ⇒ server default).
        mc_paths: u32,
        /// Independent randomized scrambles (`0` ⇒ server default).
        mc_replications: u32,
        /// Time steps per path (`0` ⇒ server default).
        mc_steps: u32,
        /// The base scramble seed.
        mc_seed: u64,
    },
    /// An FX outright forward (deliverable): a linear, closed-form
    /// discounted-cashflow product priced by the dedicated linear book (NOT the
    /// option engine). Exact ⇒ no [`PricedLine::price_std_error`]. Valid for a
    /// deliverable underlying; the server rejects a non-deliverable pair.
    FxForward {
        /// The contract (delivery) rate `K`.
        contract_rate: f64,
        /// The notional (always positive; direction is `side`).
        notional: f64,
        /// The directional side.
        side: ForwardSide,
    },
    /// An FX swap: a near leg + a far leg (two outright forwards, opposite sides).
    /// The PV is the sum of the two leg PVs. Deliverable underlying only.
    FxSwap {
        /// The near (shorter-dated, spot-settling) leg's contract rate.
        contract_rate: f64,
        /// The near leg's notional.
        notional: f64,
        /// The near leg's directional side (the far leg is the opposite side).
        side: ForwardSide,
    },
    /// A non-deliverable forward (NDF): cash-settled in the convertible currency
    /// at a named fixing. Risk-neutral PV identical to a deliverable forward of
    /// equal terms. Valid ONLY for a non-deliverable underlying.
    Ndf {
        /// The contract (forward) rate `K`.
        contract_rate: f64,
        /// The notional (always positive; direction is `side`).
        notional: f64,
        /// The directional side.
        side: ForwardSide,
        /// The published settlement-rate option the contract fixes against.
        fixing: FixingSource,
        /// The convertible (settlement) currency.
        settlement_ccy: String,
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
            Product::VarianceSwap { strike_vol } => {
                instrument::Product::VarianceSwap(celnet_proto::VarianceSwap {
                    strike_vol: *strike_vol,
                })
            }
            Product::VolatilitySwap { strike_vol } => {
                instrument::Product::VolatilitySwap(celnet_proto::VolatilitySwap {
                    strike_vol: *strike_vol,
                })
            }
            Product::AsianOption {
                option,
                strike,
                averaging,
                method,
                elapsed_avg,
                elapsed_weight,
            } => {
                let (averaging_tag, observations) = match averaging {
                    AveragingStyle::Discrete { observations } => {
                        (celnet_proto::AveragingStyle::Discrete, *observations)
                    }
                    AveragingStyle::Continuous => (celnet_proto::AveragingStyle::Continuous, 0),
                };
                instrument::Product::AsianOption(celnet_proto::AsianOption {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: *strike,
                    averaging: averaging_tag as i32,
                    observations,
                    method: match method {
                        AsianMethod::Curran => celnet_proto::AsianMethod::Curran,
                        AsianMethod::TurnbullWakeman => celnet_proto::AsianMethod::TurnbullWakeman,
                    } as i32,
                    elapsed_avg: *elapsed_avg,
                    elapsed_weight: *elapsed_weight,
                })
            }
            Product::ForwardStart {
                option,
                moneyness,
                reset,
            } => instrument::Product::ForwardStart(celnet_proto::ForwardStart {
                option_type: celnet_proto::OptionType::from(*option) as i32,
                moneyness: *moneyness,
                reset: *reset,
            }),
            Product::Cliquet {
                option,
                moneyness,
                periods,
                local_floor,
                local_cap,
                global_floor,
                global_cap,
                mc_pairs,
                mc_seed,
            } => instrument::Product::Cliquet(celnet_proto::Cliquet {
                option_type: celnet_proto::OptionType::from(*option) as i32,
                moneyness: *moneyness,
                periods: *periods,
                local_floor: *local_floor,
                local_cap: *local_cap,
                global_floor: *global_floor,
                global_cap: *global_cap,
                mc_pairs: *mc_pairs,
                mc_seed: *mc_seed,
            }),
            Product::Quanto {
                payoff,
                option,
                strike,
                conversion_vol,
                correlation,
            } => instrument::Product::Quanto(celnet_proto::Quanto {
                payoff: match payoff {
                    QuantoPayoff::Vanilla => celnet_proto::QuantoPayoff::Vanilla,
                    QuantoPayoff::Digital => celnet_proto::QuantoPayoff::Digital,
                } as i32,
                option_type: celnet_proto::OptionType::from(*option) as i32,
                strike: *strike,
                conversion_vol: *conversion_vol,
                correlation: *correlation,
            }),
            Product::Tarf {
                option,
                strike,
                target,
                leverage,
                redemption,
                fixings,
                fixing_notional,
                mc_pairs,
                mc_seed,
            } => instrument::Product::Tarf(celnet_proto::Tarf {
                option_type: celnet_proto::OptionType::from(*option) as i32,
                strike: *strike,
                target: *target,
                leverage: *leverage,
                redemption: match redemption {
                    TarfRedemption::FullGain => celnet_proto::TarfRedemption::FullGain,
                    TarfRedemption::CappedGain => celnet_proto::TarfRedemption::CappedGain,
                } as i32,
                schedule: Some(celnet_proto::FixingSchedule {
                    fixing_years: equal_fixing_years(*fixings),
                    fixing_notional: *fixing_notional,
                }),
                mc_pairs: *mc_pairs,
                mc_seed: *mc_seed,
            }),
            Product::Accumulator {
                pivot,
                barrier,
                leverage,
                monitoring,
                fixings,
                fixing_notional,
                mc_pairs,
                mc_seed,
            } => instrument::Product::Accumulator(celnet_proto::Accumulator {
                pivot: *pivot,
                barrier: *barrier,
                leverage: *leverage,
                monitoring: match monitoring {
                    AccumulatorMonitoring::Discrete => {
                        celnet_proto::AccumulatorMonitoring::Discrete
                    }
                    AccumulatorMonitoring::Continuous => {
                        celnet_proto::AccumulatorMonitoring::Continuous
                    }
                } as i32,
                schedule: Some(celnet_proto::FixingSchedule {
                    fixing_years: equal_fixing_years(*fixings),
                    fixing_notional: *fixing_notional,
                }),
                mc_pairs: *mc_pairs,
                mc_seed: *mc_seed,
            }),
            Product::Lookback {
                style,
                option,
                monitoring,
                strike,
                observations,
                mc_pairs,
                mc_seed,
            } => instrument::Product::Lookback(celnet_proto::Lookback {
                style: match style {
                    LookbackStyle::Floating => celnet_proto::LookbackStyle::Floating,
                    LookbackStyle::Fixed => celnet_proto::LookbackStyle::Fixed,
                } as i32,
                option_type: celnet_proto::OptionType::from(*option) as i32,
                monitoring: match monitoring {
                    LookbackMonitoring::Continuous => celnet_proto::LookbackMonitoring::Continuous,
                    LookbackMonitoring::Discrete => celnet_proto::LookbackMonitoring::Discrete,
                } as i32,
                strike: *strike,
                observations: *observations,
                mc_pairs: *mc_pairs,
                mc_seed: *mc_seed,
            }),
            Product::WindowBarrier {
                option,
                strike,
                barrier,
                side,
                window_start,
                window_end,
                mc_pairs,
                mc_steps,
                mc_seed,
            } => instrument::Product::WindowBarrier(celnet_proto::WindowBarrier {
                vanilla: Some(celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(*strike)),
                    }),
                }),
                barrier: *barrier,
                side: match side {
                    BarrierSide::Up => celnet_proto::BarrierSide::Up,
                    BarrierSide::Down => celnet_proto::BarrierSide::Down,
                } as i32,
                window_start: *window_start,
                window_end: *window_end,
                mc_pairs: *mc_pairs,
                mc_steps: *mc_steps,
                mc_seed: *mc_seed,
            }),
            Product::American {
                option,
                strike,
                style,
                lsm_paths,
                lsm_exercise_dates,
                lsm_seed,
            } => {
                let (exercise_style, bermudan_dates) = match style {
                    ExerciseStyle::American => (celnet_proto::ExerciseStyle::American, Vec::new()),
                    ExerciseStyle::Bermudan { dates } => {
                        (celnet_proto::ExerciseStyle::Bermudan, dates.clone())
                    }
                };
                instrument::Product::American(celnet_proto::AmericanOption {
                    option_type: celnet_proto::OptionType::from(*option) as i32,
                    strike: *strike,
                    exercise_style: exercise_style as i32,
                    bermudan_dates,
                    lsm_paths: *lsm_paths,
                    lsm_exercise_dates: *lsm_exercise_dates,
                    lsm_seed: *lsm_seed,
                })
            }
            Product::Basket {
                legs,
                correlations,
                option,
                strike,
                kind,
                mc_paths,
                mc_replications,
                mc_steps,
                mc_seed,
            } => instrument::Product::Basket(celnet_proto::BasketOption {
                legs: legs
                    .iter()
                    .map(|l| celnet_proto::BasketLeg {
                        underlying: Some(celnet_proto::Underlying::fx(
                            celnet_proto::CcyPair::from(l.pair),
                        )),
                        weight: l.weight,
                        spot: l.spot,
                        vol: l.vol,
                        r_for: l.r_for,
                    })
                    .collect(),
                correlations: correlations.clone(),
                option_type: celnet_proto::OptionType::from(*option) as i32,
                strike: *strike,
                kind: kind.to_wire() as i32,
                mc_paths: *mc_paths,
                mc_replications: *mc_replications,
                mc_steps: *mc_steps,
                mc_seed: *mc_seed,
            }),
            Product::FxForward {
                contract_rate,
                notional,
                side,
            } => instrument::Product::FxForward(celnet_proto::FxForward {
                contract_rate: *contract_rate,
                notional: *notional,
                side: side.to_wire() as i32,
            }),
            Product::FxSwap {
                contract_rate,
                notional,
                side,
            } => {
                // The near leg anchors the swap: its contract rate / notional /
                // side drive both legs. The far leg trades the opposite side at the
                // same contract rate, settling at the instrument's forward tenor —
                // the server forms it from the near leg, so the `far` message is
                // populated as the opposite side for a complete, self-describing
                // wire instrument (the server reads only `near`'s economics).
                let near = celnet_proto::FxForward {
                    contract_rate: *contract_rate,
                    notional: *notional,
                    side: side.to_wire() as i32,
                };
                let far = celnet_proto::FxForward {
                    contract_rate: *contract_rate,
                    notional: *notional,
                    side: match side {
                        ForwardSide::Buy => ForwardSide::Sell,
                        ForwardSide::Sell => ForwardSide::Buy,
                    }
                    .to_wire() as i32,
                };
                instrument::Product::FxSwap(celnet_proto::FxSwap {
                    near: Some(near),
                    far: Some(far),
                })
            }
            Product::Ndf {
                contract_rate,
                notional,
                side,
                fixing,
                settlement_ccy,
            } => instrument::Product::Ndf(celnet_proto::Ndf {
                contract_rate: *contract_rate,
                notional: *notional,
                side: side.to_wire() as i32,
                fixing: fixing_source_to_wire(*fixing) as i32,
                settlement_ccy: settlement_ccy.clone(),
            }),
        }
    }
}

/// Build the equally-spaced fixing year-fractions a count-based TARF/accumulator
/// term implies: `n` points at `k/n · 1` for `k = 1..=n` (the server reads only
/// the count and the per-fixing notional; the year-fractions are normalised to the
/// instrument expiry by the engine, which spaces fixings equally over `[0, T]`).
fn equal_fixing_years(fixings: u32) -> Vec<f64> {
    let n = fixings.max(1);
    (1..=n).map(|k| f64::from(k) / f64::from(n)).collect()
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
    /// The booking / pricing model the instrument is priced under. Defaults to
    /// [`PricingModel::Default`] (the analytic engine); set via
    /// [`InstrumentSpec::pricing_model`] / [`InstrumentSpec::with_lsv`].
    pub pricing_model: PricingModel,
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
    /// Set the booking / pricing model this instrument is priced under (a pricing
    /// directive, not an API version). Returns `self` for fluent chaining.
    #[must_use]
    pub fn pricing_model(mut self, model: PricingModel) -> Self {
        self.pricing_model = model;
        self
    }

    /// Route this instrument through the local-stochastic-volatility engine — a
    /// convenience for `.pricing_model(PricingModel::LocalStochVol)`. Only the
    /// LSV-supported products (vanilla, single-barrier knock-out, window-barrier)
    /// will price; any other product is rejected by the server with
    /// `INVALID_ARGUMENT`.
    #[must_use]
    pub fn with_lsv(self) -> Self {
        self.pricing_model(PricingModel::LocalStochVol)
    }

    /// A window knock-out barrier (active only inside the calendar window
    /// `[window_start, window_end] ⊆ [0, expiry_years]`). This product has no
    /// closed form and is priced **only** under the LSV model, so the spec is
    /// constructed with [`PricingModel::LocalStochVol`] already selected. Set
    /// `mc_pairs = 0` for the exact ADI-PDE price (no std-error) or `mc_pairs > 0`
    /// for the Monte-Carlo engine (which surfaces a std-error).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn window_barrier(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        option: OptionType,
        strike: f64,
        barrier: f64,
        barrier_side: BarrierSide,
        window_start: f64,
        window_end: f64,
        mc_pairs: u32,
        mc_steps: u32,
        mc_seed: u64,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::LocalStochVol,
            product: Product::WindowBarrier {
                option,
                strike,
                barrier,
                side: barrier_side,
                window_start,
                window_end,
                mc_pairs,
                mc_steps,
                mc_seed,
            },
        }
    }

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
            pricing_model: PricingModel::Default,
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
            pricing_model: PricingModel::Default,
            product: Product::Strategy { kind, legs },
        }
    }

    /// A single-barrier knock-in / knock-out option on the given pair / tenor /
    /// expiry / notional, carrying a [`BarrierTerms`] payoff spec: a vanilla
    /// option/strike that activates ([`BarrierKind::KnockIn`]) or extinguishes
    /// ([`BarrierKind::KnockOut`]) when spot touches the barrier, with a rebate
    /// on the barrier event.
    #[must_use]
    pub fn single_barrier(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: BarrierTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::SingleBarrier {
                option: terms.option,
                strike: terms.strike,
                kind: terms.kind,
                side: terms.barrier_side,
                barrier: terms.barrier,
                rebate: terms.rebate,
            },
        }
    }

    /// A double-barrier knock-in / knock-out option on the given pair / tenor /
    /// expiry / notional, carrying a [`DoubleBarrierTerms`] payoff spec: a vanilla
    /// option/strike bounded by a lower and upper barrier (the kind applies to
    /// whichever is touched), with a rebate on the barrier event.
    #[must_use]
    pub fn double_barrier(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: DoubleBarrierTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::DoubleBarrier {
                option: terms.option,
                strike: terms.strike,
                kind: terms.kind,
                lower: terms.lower,
                upper: terms.upper,
                rebate: terms.rebate,
            },
        }
    }

    /// A digital (binary) option on the given pair / tenor / expiry / notional,
    /// carrying a [`DigitalTerms`] payoff spec: pays a fixed payout if
    /// in-the-money at expiry (call ⇒ `S_T > strike`, put ⇒ `S_T < strike`),
    /// settled cash-or-nothing or asset-or-nothing per [`DigitalStyle`].
    #[must_use]
    pub fn digital(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: DigitalTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Digital {
                option: terms.option,
                strike: terms.strike,
                style: terms.style,
                payout: terms.payout,
            },
        }
    }

    /// A touch structure on the given pair / tenor / expiry / notional, carrying a
    /// [`TouchTerms`] payoff spec. Build the terms via the family conveniences
    /// [`TouchTerms::one_touch`] / [`TouchTerms::no_touch`] /
    /// [`TouchTerms::double_no_touch`] / [`TouchTerms::double_one_touch`].
    #[must_use]
    pub fn touch(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: TouchTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Touch {
                kind: terms.kind,
                lower: terms.lower,
                upper: terms.upper,
                rebate: terms.rebate,
            },
        }
    }

    /// A one-touch: pays `rebate` if spot touches the single `barrier` before
    /// expiry.
    #[must_use]
    pub fn one_touch(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        barrier: f64,
        rebate: f64,
    ) -> Self {
        Self::touch(
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            TouchTerms::one_touch(barrier, rebate),
        )
    }

    /// A no-touch: pays `rebate` if spot does NOT touch the single `barrier`
    /// before expiry.
    #[must_use]
    pub fn no_touch(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        barrier: f64,
        rebate: f64,
    ) -> Self {
        Self::touch(
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            TouchTerms::no_touch(barrier, rebate),
        )
    }

    /// A double-no-touch on the given pair / tenor / expiry / notional, carrying a
    /// double-corridor [`TouchTerms`] (build via [`TouchTerms::double_no_touch`]):
    /// pays the rebate if spot touches NEITHER corridor barrier before expiry.
    #[must_use]
    pub fn double_no_touch(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: TouchTerms,
    ) -> Self {
        Self::touch(pair, tenor, expiry_years, quantity, side, terms)
    }

    /// A double-one-touch on the given pair / tenor / expiry / notional, carrying a
    /// double-corridor [`TouchTerms`] (build via [`TouchTerms::double_one_touch`]):
    /// pays the rebate if spot touches EITHER corridor barrier before expiry.
    #[must_use]
    pub fn double_one_touch(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: TouchTerms,
    ) -> Self {
        Self::touch(pair, tenor, expiry_years, quantity, side, terms)
    }

    /// A variance swap on the given pair / tenor / expiry / notional. Pass
    /// `strike_vol = 0.0` to read the fair variance strike off the response.
    #[must_use]
    pub fn variance_swap(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        strike_vol: f64,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::VarianceSwap { strike_vol },
        }
    }

    /// A volatility swap on the given pair / tenor / expiry / notional. Pass
    /// `strike_vol = 0.0` to read the fair vol strike off the response.
    #[must_use]
    pub fn volatility_swap(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        strike_vol: f64,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::VolatilitySwap { strike_vol },
        }
    }

    /// A fixed-strike arithmetic-average-rate Asian option on the given pair /
    /// tenor / expiry / notional, carrying an [`AsianTerms`] payoff spec
    /// (averaging layout, estimator, and seasoning state).
    #[must_use]
    pub fn asian_option(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: AsianTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::AsianOption {
                option: terms.option,
                strike: terms.strike,
                averaging: terms.averaging,
                method: terms.method,
                elapsed_avg: terms.elapsed_avg,
                elapsed_weight: terms.elapsed_weight,
            },
        }
    }

    /// A forward-start vanilla on the given pair / tenor / expiry / notional,
    /// carrying a [`ForwardStartTerms`] spec: the strike fixes at `reset` to
    /// `moneyness·S(reset)` and pays at expiry.
    #[must_use]
    pub fn forward_start(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: ForwardStartTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::ForwardStart {
                option: terms.option,
                moneyness: terms.moneyness,
                reset: terms.reset,
            },
        }
    }

    /// A cliquet (ratchet) on the given pair / tenor / expiry / notional, carrying
    /// a [`CliquetTerms`] spec (period count, optional clamps, MC knobs). A plain
    /// (unclamped) ratchet prices in closed form; any clamp is priced by Monte-
    /// Carlo and the standard error is surfaced on [`PricedLine::price_std_error`].
    #[must_use]
    pub fn cliquet(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: CliquetTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Cliquet {
                option: terms.option,
                moneyness: terms.moneyness,
                periods: terms.periods,
                local_floor: terms.local_floor,
                local_cap: terms.local_cap,
                global_floor: terms.global_floor,
                global_cap: terms.global_cap,
                mc_pairs: terms.mc_pairs,
                mc_seed: terms.mc_seed,
            },
        }
    }

    /// A quanto option (vanilla or cash-or-nothing digital) on the given pair /
    /// tenor / expiry / notional, carrying a [`QuantoTerms`] spec.
    #[must_use]
    pub fn quanto(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: QuantoTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Quanto {
                payoff: terms.payoff,
                option: terms.option,
                strike: terms.strike,
                conversion_vol: terms.conversion_vol,
                correlation: terms.correlation,
            },
        }
    }

    /// A Target-Redemption Forward on the given pair / tenor / expiry / notional,
    /// carrying a [`TarfTerms`] spec. Always priced by Monte-Carlo: the standard
    /// error is surfaced on [`PricedLine::price_std_error`] / [`Quote::price_std_error`].
    #[must_use]
    pub fn tarf(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: TarfTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Tarf {
                option: terms.option,
                strike: terms.strike,
                target: terms.target,
                leverage: terms.leverage,
                redemption: terms.redemption,
                fixings: terms.fixings,
                fixing_notional: terms.fixing_notional,
                mc_pairs: terms.mc_pairs,
                mc_seed: terms.mc_seed,
            },
        }
    }

    /// An accumulator on the given pair / tenor / expiry / notional, carrying an
    /// [`AccumulatorTerms`] spec. Always priced by Monte-Carlo: the standard error
    /// is surfaced on [`PricedLine::price_std_error`] / [`Quote::price_std_error`].
    #[must_use]
    pub fn accumulator(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: AccumulatorTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Accumulator {
                pivot: terms.pivot,
                barrier: terms.barrier,
                leverage: terms.leverage,
                monitoring: terms.monitoring,
                fixings: terms.fixings,
                fixing_notional: terms.fixing_notional,
                mc_pairs: terms.mc_pairs,
                mc_seed: terms.mc_seed,
            },
        }
    }

    /// A lookback option on the given pair / tenor / expiry / notional, carrying a
    /// [`LookbackTerms`] spec. The continuous variant prices in closed form
    /// ([`PricedLine::price_std_error`] is `None`); the discrete variant prices by
    /// Monte-Carlo and surfaces a standard error.
    #[must_use]
    pub fn lookback(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: LookbackTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Lookback {
                style: terms.style,
                option: terms.option,
                monitoring: terms.monitoring,
                strike: terms.strike,
                observations: terms.observations,
                mc_pairs: terms.mc_pairs,
                mc_seed: terms.mc_seed,
            },
        }
    }

    /// An American / Bermudan early-exercise vanilla on the given pair / tenor /
    /// expiry / notional, carrying an [`AmericanTerms`] spec. The finite-difference
    /// engine prices exactly ([`PricedLine::price_std_error`] is `None`); selecting
    /// [`AmericanTerms::monte_carlo`] surfaces a standard error.
    #[must_use]
    pub fn american(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: AmericanTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::American {
                option: terms.option,
                strike: terms.strike,
                style: terms.style,
                lsm_paths: terms.lsm_paths,
                lsm_exercise_dates: terms.lsm_exercise_dates,
                lsm_seed: terms.lsm_seed,
            },
        }
    }

    /// A correlated multi-asset FX option (weighted basket / best-of / worst-of)
    /// over N currency-pair legs, carrying a [`BasketTerms`] spec. Priced by
    /// Cholesky-correlated multi-asset GBM Monte-Carlo, so it surfaces a standard
    /// error on [`PricedLine::price_std_error`]; multi-asset Greeks are deferred
    /// (the strip is zeroed). The top-level `pair` is the settlement / numeraire
    /// pair (the underlyings are the per-leg pairs); the shared domestic rate is
    /// the request market context's `r_dom`.
    #[must_use]
    pub fn basket(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        side: Side,
        terms: BasketTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side,
            pricing_model: PricingModel::Default,
            product: Product::Basket {
                legs: terms.legs,
                correlations: terms.correlations,
                option: terms.option,
                strike: terms.strike,
                kind: terms.kind,
                mc_paths: terms.mc_paths,
                mc_replications: terms.mc_replications,
                mc_steps: terms.mc_steps,
                mc_seed: terms.mc_seed,
            },
        }
    }

    /// An FX outright forward (deliverable) on the given pair / tenor / expiry /
    /// notional, carrying a [`ForwardTerms`] spec. A linear, closed-form
    /// discounted-cashflow product priced by the dedicated linear book; the
    /// instrument carries the forward's directional side at the top level (a
    /// linear PV needs a sign). The server rejects a non-deliverable pair with
    /// `INVALID_ARGUMENT` (use [`InstrumentSpec::ndf`]).
    #[must_use]
    pub fn fx_forward(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        terms: ForwardTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side: terms.side.instrument_side(),
            pricing_model: PricingModel::Default,
            product: Product::FxForward {
                contract_rate: terms.contract_rate,
                notional: terms.notional,
                side: terms.side,
            },
        }
    }

    /// An FX swap (deliverable) on the given pair / tenor / expiry / notional,
    /// carrying a [`SwapTerms`] spec. The near leg settles at the spot date and
    /// the far leg at the instrument's forward tenor (`expiry_years`), trading the
    /// opposite side at the same contract rate; the PV is the sum of the two leg
    /// PVs and the Greek strip is the net (near + far) risk. The server rejects a
    /// non-deliverable pair with `INVALID_ARGUMENT`.
    #[must_use]
    pub fn fx_swap(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        terms: SwapTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side: terms.near.side.instrument_side(),
            pricing_model: PricingModel::Default,
            product: Product::FxSwap {
                contract_rate: terms.near.contract_rate,
                notional: terms.near.notional,
                side: terms.near.side,
            },
        }
    }

    /// A non-deliverable forward on the given pair / tenor / expiry / notional,
    /// carrying an [`NdfTerms`] spec. Cash-settled in the convertible currency at
    /// a named fixing; the risk-neutral PV is identical to a deliverable forward
    /// of equal terms. Valid ONLY for a non-deliverable underlying — the server
    /// rejects a deliverable pair with `INVALID_ARGUMENT` (use
    /// [`InstrumentSpec::fx_forward`]). The fixing is convention identity only;
    /// the live fixing VALUE is an estate-gated feed, never sourced in-repo.
    #[must_use]
    pub fn ndf(
        pair: CcyPair,
        tenor: Tenor,
        expiry_years: f64,
        quantity: Quantity,
        terms: NdfTerms,
    ) -> Self {
        Self {
            pair,
            tenor,
            expiry_years,
            quantity,
            side: terms.side.instrument_side(),
            pricing_model: PricingModel::Default,
            product: Product::Ndf {
                contract_rate: terms.contract_rate,
                notional: terms.notional,
                side: terms.side,
                fixing: terms.fixing,
                settlement_ccy: terms.settlement_ccy,
            },
        }
    }

    /// Encode to the wire instrument message. `solve` is left unset (the SDK
    /// exposes solve via a dedicated future iteration; the explicit strikes the
    /// caller supplies are used as given).
    #[must_use]
    pub(crate) fn to_wire(&self) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair::from(
                self.pair,
            ))),
            tenor: Some(celnet_proto::Tenor::from(self.tenor)),
            expiry_years: self.expiry_years,
            quantity: Some(celnet_proto::Quantity {
                notional: self.quantity.notional,
                base_ccy: self.quantity.base_ccy,
            }),
            side: self.side.to_wire() as i32,
            solve: None,
            pricing_model: self.pricing_model.to_wire() as i32,
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
    /// The marked-surface version this quote was priced against, if the server
    /// reported one (the requested pin, or the live mark it resolved).
    pub surface_version: Option<u64>,
    /// The who's-trading attribution chain the server resolved for this quote, if
    /// any: who quoted it (the maker), and — once known — who holds/won it. Present
    /// iff the server attributed the line.
    pub attribution: Option<Attribution>,
    /// Monte-Carlo standard error of the quoted premium: `Some` for an MC-priced
    /// product (e.g. a clamped/floored cliquet), `None` for closed-form products.
    /// Surfaced so an SDK quote discloses the same MC uncertainty as a one-shot
    /// price ([`PricedLine::price_std_error`]) and never presents MC as exact.
    pub price_std_error: Option<f64>,
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
        let attribution = w
            .attribution
            .as_ref()
            .map(Attribution::from_wire)
            .transpose()?;
        Ok(Self {
            quote_id: w.quote_id,
            idempotency_key: w.idempotency_key,
            price,
            greeks,
            conventions,
            resolved_strike: w.resolved_strike,
            epoch_nanos: w.epoch_nanos,
            valid_until_nanos: w.valid_until_nanos,
            surface_version: w.surface_version,
            attribution,
            price_std_error: w.price_std_error,
        })
    }
}

/// A booking confirmation produced by accepting a quote — the typed form of the
/// wire `Execution`.
#[derive(Debug, Clone, PartialEq)]
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
    /// The who's-trading attribution chain the server resolved for this booked
    /// trade, if any: who quoted it and who now holds it. Present iff the server
    /// attributed the trade.
    pub attribution: Option<Attribution>,
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
        let attribution = w
            .attribution
            .as_ref()
            .map(Attribution::from_wire)
            .transpose()?;
        Ok(Self {
            execution_id: w.execution_id,
            quote_id: w.quote_id,
            side,
            traded_premium: w.traded_premium,
            epoch_nanos: w.epoch_nanos,
            attribution,
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
    /// For a Monte-Carlo-priced product (e.g. a clamped cliquet), the standard
    /// error of the mean of `greeks.price`; `None` for the closed-form products
    /// whose price is exact. Surfaced honestly so a caller never mistakes an MC
    /// estimate for closed-form precision.
    pub price_std_error: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SDK `InstrumentSpec::american` / `bermudan` builders encode to the wire
    /// `AmericanOption` arm (product field 24) with the right enum tags, dates, and
    /// LSM knobs — the SDK half of the five-surface api-first parity.
    #[test]
    fn american_terms_encode_to_the_wire_arm() {
        // American, FD engine (default).
        let spec = InstrumentSpec::american(
            CcyPair::parse("EURUSD").unwrap(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            AmericanTerms::american(OptionType::Put, 1.10),
        );
        match spec.to_wire().product {
            Some(instrument::Product::American(a)) => {
                assert_eq!(a.option_type, celnet_proto::OptionType::Put as i32);
                assert_eq!(a.strike.to_bits(), 1.10_f64.to_bits());
                assert_eq!(
                    a.exercise_style,
                    celnet_proto::ExerciseStyle::American as i32
                );
                assert!(a.bermudan_dates.is_empty());
                assert_eq!(a.lsm_paths, 0);
            }
            other => panic!("expected american, got {other:?}"),
        }

        // Bermudan, LSM engine.
        let berm = InstrumentSpec::american(
            CcyPair::parse("EURUSD").unwrap(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            AmericanTerms::bermudan(OptionType::Call, 1.05, vec![0.25, 0.5, 0.75, 1.0])
                .monte_carlo(100_000, 50, 0xABCD),
        );
        match berm.to_wire().product {
            Some(instrument::Product::American(a)) => {
                assert_eq!(
                    a.exercise_style,
                    celnet_proto::ExerciseStyle::Bermudan as i32
                );
                assert_eq!(a.bermudan_dates.len(), 4);
                assert_eq!(a.lsm_paths, 100_000);
                assert_eq!(a.lsm_exercise_dates, 50);
                assert_eq!(a.lsm_seed, 0xABCD);
            }
            other => panic!("expected american, got {other:?}"),
        }
    }

    /// `InstrumentSpec::basket` encodes to the wire `BasketOption` arm (product
    /// field 25) with the per-leg market data, the row-major correlation array,
    /// the BasketKind/option tags and the MC knobs — the SDK half of the
    /// five-surface api-first parity.
    #[test]
    fn basket_terms_encode_to_the_wire_arm() {
        let eurusd = CcyPair::parse("EURUSD").unwrap();
        let gbpusd = CcyPair::parse("GBPUSD").unwrap();
        let terms = BasketTerms::new(
            vec![
                BasketLegTerms::new(eurusd, 0.5, 1.10, 0.11, 0.015),
                BasketLegTerms::new(gbpusd, 0.5, 1.27, 0.13, 0.02),
            ],
            vec![1.0, 0.4, 0.4, 1.0],
            BasketKind::WorstOf,
            OptionType::Call,
            1.18,
        )
        .monte_carlo(8192, 16, 1, 0xC0FFEE);
        let spec = InstrumentSpec::basket(
            eurusd,
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            terms,
        );
        match spec.to_wire().product {
            Some(instrument::Product::Basket(b)) => {
                assert_eq!(b.legs.len(), 2);
                assert_eq!(b.legs[0].spot.to_bits(), 1.10_f64.to_bits());
                assert_eq!(b.legs[1].vol.to_bits(), 0.13_f64.to_bits());
                assert_eq!(b.correlations, vec![1.0, 0.4, 0.4, 1.0]);
                assert_eq!(b.kind, celnet_proto::BasketKind::WorstOf as i32);
                assert_eq!(b.option_type, celnet_proto::OptionType::Call as i32);
                assert_eq!(b.strike.to_bits(), 1.18_f64.to_bits());
                assert_eq!(b.mc_paths, 8192);
                assert_eq!(b.mc_replications, 16);
                assert_eq!(b.mc_seed, 0xC0FFEE);
            }
            other => panic!("expected basket, got {other:?}"),
        }
    }
}
