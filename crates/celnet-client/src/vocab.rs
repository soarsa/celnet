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
            product: Product::Quanto {
                payoff: terms.payoff,
                option: terms.option,
                strike: terms.strike,
                conversion_vol: terms.conversion_vol,
                correlation: terms.correlation,
            },
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
