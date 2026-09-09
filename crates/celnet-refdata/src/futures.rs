//! The curated **US Treasury futures** contract reference universe — the listed
//! benchmark contracts an interest-rate hedge is expressed in.
//!
//! # Why this exists
//!
//! A corporate bond is not hedged with itself; it is hedged with a benchmark, in
//! practice a front-month Treasury future, sized by the DV01 ratio
//!
//! ```text
//! contracts = portfolio_DV01 / contract_DV01
//! ```
//!
//! so the platform needs (a) the contract to exist as a first-class instrument with
//! a stable canonical id, and (b) an honest **DV01 per contract** — the denominator
//! of that ratio. A wrong DV01 silently mis-sizes every hedge, so nothing in this
//! module is a hand-typed sensitivity: every DV01 is **derived** from published
//! contract terms through the real [`celnet_bond`] analytics leaf (see
//! [`TreasuryFutureSpec::dv01_per_contract`]).
//!
//! # Sourced contract terms
//!
//! Every static figure below (face value, minimum price increment, deliverable
//! maturity window, notional coupon, last-trading-day and delivery rules) is taken
//! from the **CBOT rulebook chapters** that govern each contract, published by CME
//! Group:
//!
//! | Contract | Rulebook chapter |
//! |---|---|
//! | 2-Year Note (`ZT`, floor `TU`) | CBOT Ch. 21 — Short-Term U.S. Treasury Note Futures |
//! | 5-Year Note (`ZF`, floor `FV`) | CBOT Ch. 20 — Medium-Term U.S. Treasury Note Futures |
//! | 10-Year Note (`ZN`) | CBOT Ch. 19 — U.S. Treasury Note Futures (6½ to 8-Year) |
//! | Ultra 10-Year Note (`TN`) | CBOT Ch. 26 — Ultra 10-Year U.S. Treasury Note Futures |
//! | Treasury Bond (`ZB`, floor `US`) | CBOT Ch. 18 — U.S. Treasury Bond Futures |
//! | Ultra Treasury Bond (`UB`) | CBOT Ch. 40 — Ultra U.S. Treasury Bond Futures |
//!
//! Rule references are cited inline on each field of [`ContractTerms`]. The listed
//! quarterly cycle (the nearest three contracts of the Mar/Jun/Sep/Dec cycle) is from
//! CME Group's *Understanding Treasury Futures* (Table 2) rather than the rulebook,
//! which is silent on listed months — flagged there too.
//!
//! # The DV01 derivation (read this before trusting the number)
//!
//! There is no such thing as a *published constant* DV01 for a Treasury future: the
//! live basis-point value of a contract is the DV01 of its cheapest-to-deliver
//! security divided by that security's conversion factor, and it moves every day with
//! the level of yields and with which security is cheapest to deliver.
//!
//! What **is** fixed by the contract is the **notional deliverable**: the rulebook
//! defines the conversion factor `c` as *"the price at which a note with the same
//! time to maturity as said note, and with the same coupon rate as said note … will
//! yield 6% per annum"*. A security whose coupon equals that 6% notional yield prices
//! at par, so its conversion factor is exactly `1.000`, and the standard identity
//!
//! ```text
//! DV01_future = DV01_deliverable / conversion_factor
//! ```
//!
//! collapses to the DV01 of the notional deliverable itself. This module therefore
//! defines the contract's notional deliverable as
//!
//! * a **6% semi-annual** Treasury (the rulebook's notional coupon),
//! * whose remaining term is the **midpoint of the published deliverable window**
//!   (a neutral point in the basket — never the cheapest or dearest end),
//! * valued on the **first calendar day of the delivery month**, which is exactly the
//!   date the rulebook measures remaining term to maturity from,
//!
//! and derives the DV01 by pricing that real cashflow schedule through
//! [`celnet_bond::dv01`] (an analytic first derivative, not a bump-and-reprice).
//! The result is scaled from *per 100 face* to *per contract* by the contract's
//! point value (`face / 100`).
//!
//! ## What this DV01 is and is not
//!
//! [`TreasuryFutureSpec::dv01_per_contract`] takes the **yield to evaluate at**,
//! because the sensitivity genuinely depends on it. Two calls matter:
//!
//! * `dv01_per_contract(NOTIONAL_YIELD)` — the contract's *standardized* DV01 at its
//!   own 6% notional yield. Stable, reproducible, and the right figure to display.
//! * `dv01_per_contract(y)` with `y` from the live curve — the figure to **size a
//!   hedge with**. At today's sub-6% yield levels the standardized figure understates
//!   the live basis-point value by roughly 10–20%, so a hedge sized off the
//!   standardized number over-hedges. Pass the live yield.
//!
//! Neither figure is the exchange's daily published basis-point value: the delivery
//! option and the identity of the cheapest-to-deliver security are **not** modelled
//! here, and pretending otherwise would be a fabricated precision. What is modelled
//! is exact given the notional-deliverable idealisation the conversion factor itself
//! is built on.

use crate::model::CivilYmd;
use celnet_bond::{AccrualBasis, Bond, BondError, PaymentFrequency, dv01};
use celnet_calendar::{BusinessCalendar, CentreId};
use celnet_types::Rate;
use time::{Date, Duration, Month};

/// The notional coupon / conversion-factor yield every CBOT Treasury futures
/// contract standardises on: the conversion factor is the price at which the
/// delivered security yields **6% per annum** (CBOT Rules 18101.B, 19101.B,
/// 20101.B, 21101.B, 26101.B, 40101.B — identical wording in all six).
pub const NOTIONAL_YIELD: f64 = 0.06;

/// The number of contract months listed at any time: the nearest three of the
/// Mar/Jun/Sep/Dec quarterly cycle (CME Group, *Understanding Treasury Futures*,
/// Table 2 — "The first three consecutive contracts in the March, June, September,
/// and December quarterly cycle"). The rulebook chapters are silent on listed
/// months, so this is the one figure here sourced from CME education material
/// rather than the binding rule text.
pub const LISTED_CONTRACT_MONTHS: usize = 3;

/// The delivery month the committed listed cycle starts from.
///
/// Like the bundled securities-master snapshot, the futures universe is a **fixed
/// reference set, not a clock-derived one**: the server seeds its registry from it at
/// boot while the LP simulator resolves its streaming plan from it minutes or days
/// later, and a wall-clock roll between those two reads would silently desynchronise
/// the tradeable set from the quotable set. Rolling the cycle is therefore a
/// deliberate, reviewed edit of this constant (mirroring how the Treasury snapshot is
/// refreshed), not an implicit consequence of time passing.
pub const LISTED_CYCLE_START: CivilYmd = CivilYmd::new(2026, 9, 1);

/// Which of the two CBOT delivery/expiry conventions a contract follows. The split
/// is exactly short-end notes versus everything else, and the two rules always move
/// together, so they are carried as one convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryConvention {
    /// **2-Year and 5-Year notes.** Trading in an expiring contract runs to the
    /// *last business day of the contract's month of expiration* (CBOT 20102.F,
    /// 21102.F), and the delivery window extends to *the third business day
    /// following the last business day* of that month (CBOT 20103, 21103).
    TradesThroughMonthEnd,
    /// **10-Year, Ultra 10-Year, Bond and Ultra Bond.** *"No trades in an expiring
    /// contract shall be made during the last 7 business days of the contract's
    /// named month of expiration"* (CBOT 18102.F, 19102.F, 26102.F, 40102.F) — so
    /// the last trading day is the seventh business day preceding the last business
    /// day of the month. The delivery window ends on the *last business day* of the
    /// month (CBOT 18103, 19103, 26103, 40103).
    CeasesSevenBusinessDaysBeforeMonthEnd,
}

impl DeliveryConvention {
    /// The last trading day for a delivery month whose last business day is `lbd`.
    fn last_trading_day(self, cal: &ExchangeCalendar, lbd: Date) -> Date {
        match self {
            Self::TradesThroughMonthEnd => lbd,
            Self::CeasesSevenBusinessDaysBeforeMonthEnd => {
                // Step back over the seven closed business days.
                (0..7).fold(lbd, |d, _| cal.prev_business_day(d))
            }
        }
    }

    /// The last delivery day for a delivery month whose last business day is `lbd`.
    fn last_delivery_day(self, cal: &ExchangeCalendar, lbd: Date) -> Date {
        match self {
            Self::TradesThroughMonthEnd => cal.add_business_days(lbd, 3),
            Self::CeasesSevenBusinessDaysBeforeMonthEnd => lbd,
        }
    }
}

/// The static, contract-code-invariant terms of one Treasury futures product — the
/// figures that are identical across every delivery month of that product.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContractTerms {
    /// The current electronic product code (`ZT`/`ZF`/`ZN`/`TN`/`ZB`/`UB`). An
    /// opaque public contract code carried as **data**, not as a product identifier.
    pub symbol: &'static str,
    /// The legacy open-outcry symbol, where CME's own published material states one
    /// verbatim (`TU`/`FV`/`US`); empty when it could not be sourced verbatim rather
    /// than asserted from folklore.
    pub legacy_floor_symbol: &'static str,
    /// The display label stem, e.g. `US 10Y T-NOTE FUT`.
    pub label: &'static str,
    /// The face value at maturity of the deliverable, per contract (CBOT §…102.B).
    pub face_value: f64,
    /// The minimum price increment for an **outright** trade, in points
    /// (CBOT §…102.C). Intermonth spreads may trade a finer increment on some
    /// contracts; only the outright grid is modelled.
    pub tick_size_points: f64,
    /// The shortest remaining term to maturity, in months, a security may have and
    /// still be deliverable (CBOT §…101.A).
    pub deliverable_min_months: u32,
    /// The longest remaining term to maturity, in months, a deliverable security may
    /// have (CBOT §…101.A). Where the rule caps the *original* term rather than the
    /// remaining term, the effective remaining cap is that original-term limit;
    /// see [`TREASURY_FUTURES_TERMS`] for the per-contract derivation.
    pub deliverable_max_months: u32,
    /// Which expiry / delivery-window rule the contract follows.
    pub delivery: DeliveryConvention,
}

impl ContractTerms {
    /// The value of one full price point, per contract: `face / 100`, since a price
    /// is quoted in points of 100 face (CBOT §…102.C — "par shall be on the basis of
    /// 100 points, with each point equal to $2,000 [resp. $1,000] per contract").
    #[must_use]
    pub fn point_value(&self) -> f64 {
        self.face_value / 100.0
    }

    /// The cash value of one minimum price increment, per contract.
    #[must_use]
    pub fn tick_value(&self) -> f64 {
        self.tick_size_points * self.point_value()
    }

    /// The remaining term, in whole months, of the contract's **notional
    /// deliverable**: the midpoint of the published deliverable window. Deliberately
    /// neutral — it is neither the cheapest nor the dearest end of the basket, so the
    /// derived DV01 carries no directional bias. See the module docs.
    #[must_use]
    pub fn reference_deliverable_months(&self) -> u32 {
        (self.deliverable_min_months + self.deliverable_max_months) / 2
    }
}

/// The six curated CBOT US Treasury futures products.
///
/// Every figure is from the rulebook chapter named in the module docs. Two
/// upper-maturity bounds are *derived* rather than quoted, and are called out here
/// because the derivation matters:
///
/// * **5-Year (`ZF`)** — Rule 20101.A bounds the *remaining* term only from below
///   ("not less than 4 years 2 months") and caps the *original* term at 5 years
///   3 months. A security's remaining term can never exceed its original term, so
///   63 months is the exact implied remaining-term cap.
/// * **Ultra Bond (`UB`)** — Rule 40101.A imposes **no** upper bound at all
///   ("remaining term to maturity of at least 25 years"). The effective cap is the
///   longest security the U.S. Treasury issues, the 30-year bond, hence 360 months.
///   This is the one bound in the table that comes from issuance practice rather
///   than from a rule.
pub const TREASURY_FUTURES_TERMS: [ContractTerms; 6] = [
    ContractTerms {
        // CBOT Ch. 21. Unit $200,000 (21102.B); tick one-eighth of one thirty-second
        // of one point = $7.8125 (21102.C — note this is an EIGHTH, not the quarter
        // the 5-Year uses); deliverable original term ≤ 5y3m and remaining term
        // ≥ 1y9m and ≤ 2y (21101.A).
        symbol: "ZT",
        legacy_floor_symbol: "TU",
        label: "US 2Y T-NOTE FUT",
        face_value: 200_000.0,
        tick_size_points: 1.0 / 256.0,
        deliverable_min_months: 21,
        deliverable_max_months: 24,
        delivery: DeliveryConvention::TradesThroughMonthEnd,
    },
    ContractTerms {
        // CBOT Ch. 20. Unit $100,000 (20102.B); tick one-quarter of one
        // thirty-second of one point = $7.8125 (20102.C); deliverable original term
        // ≤ 5y3m and remaining term ≥ 4y2m (20101.A) — see the doc comment for the
        // 63-month derivation.
        symbol: "ZF",
        legacy_floor_symbol: "FV",
        label: "US 5Y T-NOTE FUT",
        face_value: 100_000.0,
        tick_size_points: 1.0 / 128.0,
        deliverable_min_months: 50,
        deliverable_max_months: 63,
        delivery: DeliveryConvention::TradesThroughMonthEnd,
    },
    ContractTerms {
        // CBOT Ch. 19. Unit $100,000 (19102.B); outright tick one-half of one
        // thirty-second of one point = $15.625 (19102.C); deliverable original term
        // ≤ 10y and remaining term ≥ 6y6m and < 8y (19101.A).
        symbol: "ZN",
        legacy_floor_symbol: "",
        label: "US 10Y T-NOTE FUT",
        face_value: 100_000.0,
        tick_size_points: 1.0 / 64.0,
        deliverable_min_months: 78,
        deliverable_max_months: 96,
        delivery: DeliveryConvention::CeasesSevenBusinessDaysBeforeMonthEnd,
    },
    ContractTerms {
        // CBOT Ch. 26. Unit $100,000 (26102.B); outright tick one-half of one
        // thirty-second of one point = $15.625 (26102.C); deliverable original term
        // ≤ 10y and remaining term ≥ 9y5m (26101.A, the text in force from the
        // March 2026 contract month onward) — the 120-month cap is the original-term
        // limit, as for the 5-Year.
        symbol: "TN",
        legacy_floor_symbol: "",
        label: "US ULTRA 10Y T-NOTE FUT",
        face_value: 100_000.0,
        tick_size_points: 1.0 / 64.0,
        deliverable_min_months: 113,
        deliverable_max_months: 120,
        delivery: DeliveryConvention::CeasesSevenBusinessDaysBeforeMonthEnd,
    },
    ContractTerms {
        // CBOT Ch. 18. Unit $100,000 (18102.B); outright tick one thirty-second of
        // one point = $31.25 (18102.C); deliverable remaining term ≥ 15y and < 25y
        // (18101.A). No original-term limit for the bond contracts.
        symbol: "ZB",
        legacy_floor_symbol: "US",
        label: "US T-BOND FUT",
        face_value: 100_000.0,
        tick_size_points: 1.0 / 32.0,
        deliverable_min_months: 180,
        deliverable_max_months: 300,
        delivery: DeliveryConvention::CeasesSevenBusinessDaysBeforeMonthEnd,
    },
    ContractTerms {
        // CBOT Ch. 40. Unit $100,000 (40102.B — one hundred thousand, NOT the
        // $200,000 of the 2-Year); outright tick one thirty-second of one point =
        // $31.25 (40102.C); deliverable remaining term ≥ 25y (40101.A) — see the doc
        // comment for the 360-month derivation.
        symbol: "UB",
        legacy_floor_symbol: "",
        label: "US ULTRA T-BOND FUT",
        face_value: 100_000.0,
        tick_size_points: 1.0 / 32.0,
        deliverable_min_months: 300,
        deliverable_max_months: 360,
        delivery: DeliveryConvention::CeasesSevenBusinessDaysBeforeMonthEnd,
    },
];

/// One fully-specified listed Treasury futures contract: a product's static
/// [`ContractTerms`] bound to a delivery month, with the month's dates resolved on
/// the exchange business-day calendar.
///
/// Built only by [`treasury_futures_universe`] / [`contracts_for_delivery_month`]
/// from validated inputs, so every date here is a real calendar date and every
/// derived figure is reproducible.
#[derive(Debug, Clone, PartialEq)]
pub struct TreasuryFutureSpec {
    /// The canonical internal `instrument_id` — the identity shared across the
    /// reference registry, the wire, and the LP feed, exactly as a Treasury's CUSIP
    /// is. It is the public contract code: product symbol + delivery-month letter +
    /// two-digit year, e.g. `ZNZ26` for the December 2026 10-Year Note contract.
    /// Two digits (rather than the single digit Globex shows) so the id stays
    /// unambiguous beyond a decade.
    pub instrument_id: String,
    /// A short, human-friendly blotter/GUI name, e.g. `US 10Y T-NOTE FUT Dec-26`.
    pub name: String,
    /// The static product terms this contract instantiates.
    pub terms: ContractTerms,
    /// ISO 4217 pricing/settlement currency — `USD` for every CBOT Treasury future.
    pub currency: &'static str,
    /// The region label, `us` (the same taxonomy the government-bond specs carry).
    pub region: &'static str,
    /// The sub-asset-type label, `government_future`.
    pub sub_asset_type: &'static str,
    /// The **first calendar day** of the delivery month. This is the date the
    /// rulebook measures a deliverable's remaining term to maturity from, and hence
    /// the valuation date of the notional deliverable.
    pub delivery_month_start: CivilYmd,
    /// The first delivery day — the first business day of the delivery month
    /// (CBOT §…103).
    pub first_delivery_date: CivilYmd,
    /// The last day the contract trades, per its [`DeliveryConvention`].
    pub last_trading_date: CivilYmd,
    /// The last delivery day, per its [`DeliveryConvention`].
    pub last_delivery_date: CivilYmd,
    /// The settlement calendar-centre labels (the same label vocabulary the
    /// government-bond specs use).
    pub calendars: Vec<&'static str>,
}

/// Why a derived figure could not be produced for a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutureSpecError {
    /// A stored civil date is not a real calendar date (never for a spec built by
    /// this module — surfaced rather than panicking).
    BadDate,
    /// The analytics leaf rejected the notional deliverable's cashflow schedule.
    Schedule(BondError),
    /// The supplied yield is not a finite number.
    BadYield,
}

impl core::fmt::Display for FutureSpecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadDate => f.write_str("a contract date is not a real calendar date"),
            Self::Schedule(e) => write!(f, "notional deliverable schedule rejected: {e}"),
            Self::BadYield => f.write_str("the reference yield is not finite"),
        }
    }
}

impl core::error::Error for FutureSpecError {}

impl TreasuryFutureSpec {
    /// The cash value of one full price point, per contract (`face / 100`).
    #[must_use]
    pub fn point_value(&self) -> f64 {
        self.terms.point_value()
    }

    /// The cash value of one minimum price increment, per contract.
    #[must_use]
    pub fn tick_value(&self) -> f64 {
        self.terms.tick_value()
    }

    /// The maturity date of the contract's **notional deliverable**: the first
    /// calendar day of the delivery month plus the deliverable window's midpoint
    /// term. See the module docs for why this security, and not a live
    /// cheapest-to-deliver, defines the contract's standardized sensitivity.
    ///
    /// # Errors
    /// [`FutureSpecError::BadDate`] if the stored delivery month is not a real date.
    pub fn notional_deliverable_maturity(&self) -> Result<CivilYmd, FutureSpecError> {
        let start = to_date(self.delivery_month_start).ok_or(FutureSpecError::BadDate)?;
        let months = i32::try_from(self.terms.reference_deliverable_months())
            .map_err(|_| FutureSpecError::BadDate)?;
        Ok(from_date(celnet_calendar::add_months(start, months)))
    }

    /// The contract's **notional deliverable** as a real [`celnet_bond::Bond`]: a
    /// 6% semi-annual Treasury maturing at [`notional_deliverable_maturity`], valued
    /// on the first calendar day of the delivery month, redeeming at 100 (so every
    /// price and sensitivity off it is per 100 face, i.e. in price points).
    ///
    /// The accrual basis is the engine's 30/360 bond basis — the same closest-basis
    /// choice the curated cash-bond universe makes, since the leaf does not model
    /// actual/actual. The choice is immaterial to the derived DV01, which is a
    /// derivative of the same schedule under the same basis.
    ///
    /// [`notional_deliverable_maturity`]: Self::notional_deliverable_maturity
    ///
    /// # Errors
    /// [`FutureSpecError::BadDate`] for an unreal stored date, or
    /// [`FutureSpecError::Schedule`] if the leaf rejects the schedule.
    pub fn notional_deliverable(&self) -> Result<Bond, FutureSpecError> {
        let settle = to_date(self.delivery_month_start).ok_or(FutureSpecError::BadDate)?;
        let maturity =
            to_date(self.notional_deliverable_maturity()?).ok_or(FutureSpecError::BadDate)?;
        Bond::new(
            settle,
            maturity,
            NOTIONAL_YIELD,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .map_err(FutureSpecError::Schedule)
    }

    /// The contract's **DV01 per contract** at `reference_yield`: the cash change in
    /// one contract's value for a +1bp parallel shift in the yield of its notional
    /// deliverable, in the contract currency (positive).
    ///
    /// Derived, never asserted:
    ///
    /// ```text
    /// DV01_contract = dv01(notional_deliverable, y)   [points per 100 face]
    ///               x point_value                     [= face / 100]
    /// ```
    ///
    /// with the conversion-factor divisor of the standard `DV01_ctd / CF` identity
    /// equal to exactly 1.000 by construction (a 6%-coupon security priced to the
    /// contract's 6% notional yield prices at par). The inner `dv01` is
    /// [`celnet_bond::dv01`] — an analytic first derivative of the real cashflow
    /// schedule.
    ///
    /// **Pass the live curve yield to size a hedge.** At `NOTIONAL_YIELD` this
    /// returns the standardized figure, which understates the live basis-point value
    /// whenever the market yields below 6%.
    ///
    /// # Errors
    /// [`FutureSpecError::BadYield`] for a non-finite yield, or the errors of
    /// [`notional_deliverable`](Self::notional_deliverable).
    pub fn dv01_per_contract(&self, reference_yield: f64) -> Result<f64, FutureSpecError> {
        if !reference_yield.is_finite() {
            return Err(FutureSpecError::BadYield);
        }
        let bond = self.notional_deliverable()?;
        let per_100 = dv01(&bond, Rate(reference_yield)).map_err(FutureSpecError::Schedule)?;
        Ok(per_100 * self.point_value())
    }

    /// The contract's standardized DV01 per contract, evaluated at its own
    /// [`NOTIONAL_YIELD`]. Stable and reproducible — the figure to *display*. See
    /// [`dv01_per_contract`](Self::dv01_per_contract) for the figure to *hedge* with.
    ///
    /// # Errors
    /// As [`dv01_per_contract`](Self::dv01_per_contract).
    pub fn dv01_at_notional_yield(&self) -> Result<f64, FutureSpecError> {
        self.dv01_per_contract(NOTIONAL_YIELD)
    }

    /// The whole number of contracts that hedges `portfolio_dv01` (cash per basis
    /// point, signed: positive for a long-duration exposure) at `reference_yield`.
    ///
    /// This is the ratio in the module docs, `portfolio_DV01 / contract_DV01`,
    /// rounded to the nearest whole contract because a fraction of a futures contract
    /// cannot be traded. The sign is the exposure's: a positive `portfolio_dv01`
    /// returns a positive count, meaning *sell* that many contracts to flatten.
    ///
    /// # Errors
    /// As [`dv01_per_contract`](Self::dv01_per_contract); also
    /// [`FutureSpecError::BadYield`] if `portfolio_dv01` is not finite.
    pub fn hedge_contracts(
        &self,
        portfolio_dv01: f64,
        reference_yield: f64,
    ) -> Result<i64, FutureSpecError> {
        if !portfolio_dv01.is_finite() {
            return Err(FutureSpecError::BadYield);
        }
        let contract_dv01 = self.dv01_per_contract(reference_yield)?;
        if contract_dv01 <= 0.0 {
            return Err(FutureSpecError::Schedule(BondError::DegeneratePeriod));
        }
        Ok((portfolio_dv01 / contract_dv01).round() as i64)
    }

    /// Snap `price` (in points) **down** to the contract's outright tick grid — the
    /// side a market maker rounds a bid to.
    #[must_use]
    pub fn round_bid_to_tick(&self, price: f64) -> f64 {
        self.snap(price, f64::floor)
    }

    /// Snap `price` (in points) **up** to the contract's outright tick grid — the
    /// side a market maker rounds an offer to. Rounding a bid down and an offer up
    /// can never invert a two-way, so a tick-snapped quote is never crossed.
    #[must_use]
    pub fn round_offer_to_tick(&self, price: f64) -> f64 {
        self.snap(price, f64::ceil)
    }

    fn snap(&self, price: f64, direction: fn(f64) -> f64) -> f64 {
        let tick = self.terms.tick_size_points;
        if !price.is_finite() || tick <= 0.0 {
            return price;
        }
        direction(price / tick) * tick
    }

    /// Whether the contract has **stopped trading** as of `as_of`: the valuation date
    /// is strictly past its [`last_trading_date`](Self::last_trading_date).
    ///
    /// An expired contract is not a market. It must not be advertised as tradeable
    /// and it must not be quoted — a venue showing a two-way in a contract that no
    /// longer trades is a fabricated market, and a hedge routed at it can never fill.
    ///
    /// An unreal stored or supplied date is treated as **expired** (fail closed): the
    /// only safe answer when the lifecycle cannot be decided is to stop trading it.
    #[must_use]
    pub fn is_expired_on(&self, as_of: CivilYmd) -> bool {
        match (to_date(as_of), to_date(self.last_trading_date)) {
            (Some(now), Some(ltd)) => now > ltd,
            _ => true,
        }
    }

    /// Whether the contract is **listed and trading** as of `as_of` — the complement
    /// of [`is_expired_on`](Self::is_expired_on). This is the predicate both the
    /// tradeable registry seed and the quoting venue must agree on, or the platform
    /// re-creates the tradeable-but-unquotable asymmetry in reverse.
    #[must_use]
    pub fn is_listed_on(&self, as_of: CivilYmd) -> bool {
        !self.is_expired_on(as_of)
    }

    /// Render `price` in the exchange's points-and-32nds display convention.
    ///
    /// A Treasury futures price is quoted in **points of 100 face and thirty-seconds
    /// of a point**, written `handle'32nds[sub]`, e.g. `110'165` = 110 + 16.5/32.
    /// The trailing sub-32nd digit encodes the fraction of a 32nd in eighths using
    /// the exchange's digit map (`0 1 2 3 5 6 7 8` for `0 ⅛ ¼ ⅜ ½ ⅝ ¾ ⅞`), so a
    /// half-32nd reads `5` and a quarter-32nd reads `2`. Contracts whose tick *is*
    /// a full 32nd (the Bond and Ultra Bond) carry no sub-digit: `115'16`.
    #[must_use]
    pub fn format_price_32nds(&self, price: f64) -> String {
        if !price.is_finite() {
            return "n/a".to_string();
        }
        /// The exchange's sub-32nd digit for each eighth of a thirty-second.
        const EIGHTHS: [&str; 8] = ["0", "1", "2", "3", "5", "6", "7", "8"];

        let handle = price.floor();
        let mut thirty_seconds = (price - handle) * 32.0;
        // Quantise to eighths of a 32nd (the finest grid any of these contracts
        // trades) before splitting, so binary representation noise cannot bump the
        // whole-32nd part.
        let eighths_total = (thirty_seconds * 8.0).round();
        thirty_seconds = eighths_total / 8.0;
        let whole = thirty_seconds.floor();
        let sub = ((thirty_seconds - whole) * 8.0).round() as usize;

        if self.terms.tick_size_points >= 1.0 / 32.0 {
            format!("{handle:.0}'{whole:02.0}")
        } else {
            format!("{handle:.0}'{whole:02.0}{}", EIGHTHS[sub.min(7)])
        }
    }
}

/// The committed listed Treasury-futures universe: every product in
/// [`TREASURY_FUTURES_TERMS`] for each of the [`LISTED_CONTRACT_MONTHS`] quarterly
/// delivery months from [`LISTED_CYCLE_START`], in a stable order (by delivery month,
/// then by the product order of the terms table).
///
/// Deterministic and allocation-only — the same records on every call, so a server
/// registry seed and an LP-SIM streaming plan always agree exactly.
#[must_use]
pub fn treasury_futures_universe() -> Vec<TreasuryFutureSpec> {
    let mut out = Vec::with_capacity(TREASURY_FUTURES_TERMS.len() * LISTED_CONTRACT_MONTHS);
    let mut month = LISTED_CYCLE_START;
    for _ in 0..LISTED_CONTRACT_MONTHS {
        out.extend(contracts_for_delivery_month(month.year, month.month));
        month = next_quarterly_month(month);
    }
    out
}

/// The committed universe restricted to the contracts still **trading** on `as_of`
/// (see [`TreasuryFutureSpec::is_listed_on`]), in the same stable order.
///
/// This is the set a venue may quote and a registry may advertise as tradeable. The
/// two must be filtered by the same predicate at the same valuation date: quoting a
/// contract that has stopped trading fabricates a market, and advertising one that no
/// venue quotes strands every hedge routed at it.
#[must_use]
pub fn listed_universe_on(as_of: CivilYmd) -> Vec<TreasuryFutureSpec> {
    treasury_futures_universe()
        .into_iter()
        .filter(|s| s.is_listed_on(as_of))
        .collect()
}

/// The **front month** of one product as of `as_of`: the nearest-delivery contract of
/// `symbol` (`ZT`/`ZF`/`ZN`/`TN`/`ZB`/`UB`) that is still trading.
///
/// This is what "hedge in the 10-Year future" has to resolve to. A hedge policy names
/// a *product*, not a delivery month — the delivery month it means is whichever
/// contract is currently the front one, and that answer changes four times a year at
/// the quarterly roll. Resolving it here, off the same committed cycle the venue
/// quotes and the registry seeds, is what keeps the three in step: the id this returns
/// is by construction one the venue is quoting.
///
/// The front month is the **nearest by delivery month among the unexpired**, not
/// merely the first listed: once the front contract stops trading (the seven-business-
/// day cessation for the 10-Year and the bonds, month end for the short-end notes) the
/// next quarterly contract becomes the front one, which is exactly the roll.
///
/// `None` when `symbol` is not a listed product, or when every listed contract for it
/// has expired (the committed cycle needs rolling — see [`LISTED_CYCLE_START`]).
#[must_use]
pub fn front_contract(symbol: &str, as_of: CivilYmd) -> Option<TreasuryFutureSpec> {
    treasury_futures_universe()
        .into_iter()
        .filter(|s| s.terms.symbol == symbol && s.is_listed_on(as_of))
        .min_by_key(|s| {
            let d = s.delivery_month_start;
            (d.year, d.month, d.day)
        })
}

/// The front month of **every** listed product as of `as_of` — the six contracts a
/// DV01 hedge picks its vehicle from, in the product order of
/// [`TREASURY_FUTURES_TERMS`]. A product whose whole listed cycle has expired is
/// omitted rather than represented by a stale contract.
#[must_use]
pub fn front_contracts(as_of: CivilYmd) -> Vec<TreasuryFutureSpec> {
    TREASURY_FUTURES_TERMS
        .iter()
        .filter_map(|t| front_contract(t.symbol, as_of))
        .collect()
}

/// Whether `id` names a listed futures **product** (`ZT`/`ZF`/`ZN`/`TN`/`ZB`/`UB`, or
/// one of the published legacy floor symbols) rather than one of that product's
/// delivery months.
///
/// This is the predicate that separates the two things a hedge policy may name. A
/// delivery month (`ZFU26`) is a specific market that stops trading on a specific day;
/// a product (`ZF`) is the standing intent "hedge in the 5-Year contract", whose answer
/// is whichever contract is currently the front one. The distinction is decidable from
/// the committed terms table alone — no parsing of month codes, so a malformed id is
/// simply not a product and is passed through untouched.
#[must_use]
pub fn is_product_symbol(id: &str) -> bool {
    TREASURY_FUTURES_TERMS.iter().any(|t| {
        t.symbol == id || (!t.legacy_floor_symbol.is_empty() && t.legacy_floor_symbol == id)
    })
}

/// The front-month **contract code** for a product symbol as of `as_of` — the id a
/// policy that names a product should actually trade today.
///
/// The companion to [`is_product_symbol`]: together they let a hedge vehicle be
/// configured once as `ZF` and resolve to `ZFU26` before the September roll and
/// `ZFZ26` after it, off the SAME committed cycle the venue quotes and the registry
/// seeds — so the id returned is by construction one the venue is quoting.
///
/// Accepts a legacy floor symbol (`FV`) as an alias for its electronic product, since
/// that is what a desk that has traded the contract for twenty years will type.
///
/// `None` when `symbol` is not a listed product, or when every listed contract for it
/// has expired — the caller must then decline rather than fabricate a contract code
/// (the committed cycle needs rolling; see [`LISTED_CYCLE_START`]).
#[must_use]
pub fn front_contract_id(symbol: &str, as_of: CivilYmd) -> Option<String> {
    let electronic = TREASURY_FUTURES_TERMS
        .iter()
        .find(|t| {
            t.symbol == symbol
                || (!t.legacy_floor_symbol.is_empty() && t.legacy_floor_symbol == symbol)
        })?
        .symbol;
    front_contract(electronic, as_of).map(|s| s.instrument_id)
}

/// The **face value of one contract** for a listed contract code (`"UBU26"`), which is the
/// unit an order quantity is denominated in on this platform's listed venue.
///
/// The venue quotes and trades in FACE, not in contract counts, so one whole lot is one
/// contract's face value (`celnet_cme_sim::contract_lot_size`) and an `OrderQty` that is
/// not a whole multiple of it is refused `NOT_A_WHOLE_LOT`. Keeping the whole aggregated
/// book in one denomination is what lets a bond and a future sit on the same panel.
///
/// Matched on the contract's own product prefix rather than the full code, so it resolves
/// for any delivery month — including one outside the currently listed cycle, which a
/// historical order still needs to size.
///
/// `None` when the code names no known product.
#[must_use]
pub fn contract_face_value(instrument_id: &str) -> Option<f64> {
    TREASURY_FUTURES_TERMS
        .iter()
        .find(|t| instrument_id.starts_with(t.symbol))
        .map(|t| t.face_value)
}

/// Every product's contract for one delivery month (`month` must be a quarterly
/// cycle month — March, June, September or December; any other month yields an
/// empty vector, since no such contract is listed).
#[must_use]
pub fn contracts_for_delivery_month(year: i32, month: u32) -> Vec<TreasuryFutureSpec> {
    let Some(code) = delivery_month_code(month) else {
        return Vec::new();
    };
    let start = CivilYmd::new(year, month, 1);
    let Some(start_date) = to_date(start) else {
        return Vec::new();
    };
    let cal = exchange_calendar();
    let lbd = last_business_day_of_month(&cal, start_date);
    let first_delivery = if cal.is_business_day(start_date) {
        start_date
    } else {
        cal.next_business_day(start_date)
    };

    TREASURY_FUTURES_TERMS
        .iter()
        .map(|terms| TreasuryFutureSpec {
            instrument_id: format!("{}{code}{:02}", terms.symbol, year.rem_euclid(100)),
            name: format!("{} {}", terms.label, start.month_year_label()),
            terms: *terms,
            currency: "USD",
            region: "us",
            sub_asset_type: "government_future",
            delivery_month_start: start,
            first_delivery_date: from_date(first_delivery),
            last_trading_date: from_date(terms.delivery.last_trading_day(&cal, lbd)),
            last_delivery_date: from_date(terms.delivery.last_delivery_day(&cal, lbd)),
            calendars: vec!["united_states"],
        })
        .collect()
}

/// The exchange business-day calendar for CBOT interest-rate products.
///
/// CBOT observes the U.S. federal settlement holidays **plus Good Friday**, on which
/// the Treasury futures market is closed. [`celnet_calendar`]'s `UnitedStates` centre
/// models the federal set (it is a settlement centre, and Good Friday is not a
/// federal banking holiday), so the Good Friday overlay is applied here rather than
/// silently producing an expiry a business day late in any March/April delivery
/// month — see [`is_good_friday`].
fn exchange_calendar() -> ExchangeCalendar {
    ExchangeCalendar {
        settlement: BusinessCalendar::single(CentreId::UnitedStates),
    }
}

/// The CBOT business-day calendar: the U.S. settlement centre plus Good Friday.
struct ExchangeCalendar {
    settlement: BusinessCalendar,
}

impl ExchangeCalendar {
    fn is_business_day(&self, date: Date) -> bool {
        self.settlement.is_business_day(date) && !is_good_friday(date)
    }

    fn next_business_day(&self, date: Date) -> Date {
        let mut d = date + Duration::days(1);
        while !self.is_business_day(d) {
            d += Duration::days(1);
        }
        d
    }

    fn prev_business_day(&self, date: Date) -> Date {
        let mut d = date - Duration::days(1);
        while !self.is_business_day(d) {
            d -= Duration::days(1);
        }
        d
    }

    fn add_business_days(&self, date: Date, n: u32) -> Date {
        (0..n).fold(date, |d, _| self.next_business_day(d))
    }
}

/// `last_business_day_of_month` over the exchange calendar.
fn last_business_day_of_month(cal: &ExchangeCalendar, any_day_in_month: Date) -> Date {
    let year = any_day_in_month.year();
    let month = any_day_in_month.month();
    let end = Date::from_calendar_date(year, month, month.length(year))
        .expect("month end is a real date");
    if cal.is_business_day(end) {
        end
    } else {
        cal.prev_business_day(end)
    }
}

/// Whether `date` is Good Friday — the Friday before Easter Sunday, computed with
/// the anonymous Gregorian computus (Meeus/Jones/Butcher). Exact for every year in
/// the Gregorian calendar; validated in this module's tests against the published
/// Easter dates for 2024–2030.
fn is_good_friday(date: Date) -> bool {
    easter_sunday(date.year()).is_some_and(|e| date == e - Duration::days(2))
}

/// Easter Sunday in `year` by the anonymous Gregorian computus.
fn easter_sunday(year: i32) -> Option<Date> {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    let month = Month::try_from(u8::try_from(month).ok()?).ok()?;
    Date::from_calendar_date(year, month, u8::try_from(day).ok()?).ok()
}

/// The exchange delivery-month letter for a quarterly cycle month, or `None` for a
/// month in which no Treasury futures contract is listed. `H`/`M`/`U`/`Z` are the
/// standard public month codes for March/June/September/December.
fn delivery_month_code(month: u32) -> Option<char> {
    match month {
        3 => Some('H'),
        6 => Some('M'),
        9 => Some('U'),
        12 => Some('Z'),
        _ => None,
    }
}

/// The next quarterly cycle month after `month` (which must itself be a cycle month).
fn next_quarterly_month(month: CivilYmd) -> CivilYmd {
    let total = month.year * 12 + (month.month as i32 - 1) + 3;
    CivilYmd::new(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1)
}

/// Map a civil triple onto a `time::Date`, or `None` if it is not a real date.
fn to_date(d: CivilYmd) -> Option<Date> {
    let month = u8::try_from(d.month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())?;
    Date::from_calendar_date(d.year, month, u8::try_from(d.day).ok()?).ok()
}

/// Map a `time::Date` back onto the crate's civil triple.
fn from_date(d: Date) -> CivilYmd {
    CivilYmd::new(d.year(), u32::from(u8::from(d.month())), u32::from(d.day()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The published contract terms are exactly what the rulebook says. This test is
    /// the guard on the sourced data: if any figure is edited, it must be edited here
    /// too, with a rule citation.
    #[test]
    fn published_contract_terms_match_the_rulebook() {
        // (symbol, face, tick points, tick $, min months, max months)
        let expected: [(&str, f64, f64, f64, u32, u32); 6] = [
            ("ZT", 200_000.0, 1.0 / 256.0, 7.8125, 21, 24),
            ("ZF", 100_000.0, 1.0 / 128.0, 7.8125, 50, 63),
            ("ZN", 100_000.0, 1.0 / 64.0, 15.625, 78, 96),
            ("TN", 100_000.0, 1.0 / 64.0, 15.625, 113, 120),
            ("ZB", 100_000.0, 1.0 / 32.0, 31.25, 180, 300),
            ("UB", 100_000.0, 1.0 / 32.0, 31.25, 300, 360),
        ];
        assert_eq!(TREASURY_FUTURES_TERMS.len(), expected.len());
        for (terms, (sym, face, tick, tick_val, lo, hi)) in
            TREASURY_FUTURES_TERMS.iter().zip(expected)
        {
            assert_eq!(terms.symbol, sym);
            assert!((terms.face_value - face).abs() < 1e-9, "{sym} face value");
            assert!(
                (terms.tick_size_points - tick).abs() < 1e-15,
                "{sym} tick size"
            );
            // The tick VALUE is derived (tick x face/100) and must reproduce the
            // dollar figure the rulebook states verbatim.
            assert!(
                (terms.tick_value() - tick_val).abs() < 1e-9,
                "{sym} tick value: got {}",
                terms.tick_value()
            );
            assert_eq!(terms.deliverable_min_months, lo, "{sym} deliverable floor");
            assert_eq!(terms.deliverable_max_months, hi, "{sym} deliverable cap");
            // A point is worth face/100 — $2,000 for the 2-Year, $1,000 for the rest.
            assert!((terms.point_value() - face / 100.0).abs() < 1e-9);
        }
        // Only the three legacy floor symbols CME publishes verbatim are asserted.
        let legacy: Vec<&str> = TREASURY_FUTURES_TERMS
            .iter()
            .map(|t| t.legacy_floor_symbol)
            .collect();
        assert_eq!(legacy, vec!["TU", "FV", "", "", "US", ""]);
        // Every contract standardises on the same 6% notional conversion yield.
        assert!((NOTIONAL_YIELD - 0.06).abs() < 1e-15);
    }

    /// The notional deliverable is the midpoint of the published window — asserted
    /// explicitly so a change to the window cannot silently move every DV01.
    #[test]
    fn notional_deliverable_terms_are_the_window_midpoint() {
        let months: Vec<u32> = TREASURY_FUTURES_TERMS
            .iter()
            .map(ContractTerms::reference_deliverable_months)
            .collect();
        assert_eq!(months, vec![22, 56, 87, 116, 240, 330]);
        for terms in &TREASURY_FUTURES_TERMS {
            let m = terms.reference_deliverable_months();
            assert!(
                m >= terms.deliverable_min_months && m <= terms.deliverable_max_months,
                "{}: reference term {m} outside its own deliverable window",
                terms.symbol
            );
        }
    }

    // --- the DV01 derivation, against an INDEPENDENT oracle ---------------------

    /// The closed-form modified duration of a bond trading **at par** (coupon =
    /// yield), `D_mod = (1/y)·(1 − (1 + y/2)^(−n))` for `n` semi-annual periods.
    ///
    /// Derived here from first principles — it does NOT go through `celnet_bond` —
    /// so it is a genuinely independent check on the leaf's analytic derivative for
    /// the two contracts whose notional deliverable falls exactly on a coupon date.
    fn par_bond_modified_duration(annual_yield: f64, periods: f64) -> f64 {
        (1.0 / annual_yield) * (1.0 - (1.0 + annual_yield / 2.0).powf(-periods))
    }

    #[test]
    fn dv01_matches_the_closed_form_where_the_schedule_is_whole_periods() {
        let universe = treasury_futures_universe();
        // The Bond (240 months) and Ultra Bond (330 months) notional deliverables are
        // whole multiples of the 6-month coupon period measured from the valuation
        // date, so settlement lands exactly on a coupon date and the par closed form
        // is exact. (The other four carry a front stub; they are covered by the
        // numerical-derivative test below.)
        for symbol in ["ZB", "UB"] {
            let spec = universe
                .iter()
                .find(|s| s.terms.symbol == symbol)
                .expect("contract in the universe");
            let periods = f64::from(spec.terms.reference_deliverable_months()) / 6.0;
            assert!(
                (periods - periods.round()).abs() < 1e-12,
                "{symbol}: expected a whole number of coupon periods"
            );
            // At par: DV01 per 100 face = D_mod x price(=100) x 1bp.
            let expected_per_100 =
                par_bond_modified_duration(NOTIONAL_YIELD, periods) * 100.0 * 1e-4;
            let expected = expected_per_100 * spec.point_value();
            let got = spec.dv01_at_notional_yield().expect("DV01 derives");
            assert!(
                (got - expected).abs() < 1e-6 * expected,
                "{symbol}: derived DV01 {got} vs independent closed form {expected}"
            );
        }
    }

    #[test]
    fn dv01_matches_a_numerical_reprice_for_every_contract() {
        // The analytic derivative must agree with a central difference of the leaf's
        // dirty price — a different code path, so this catches a wrong schedule or a
        // wrong scaling for the four stub-period contracts too.
        for spec in treasury_futures_universe() {
            let bond = spec.notional_deliverable().expect("schedule builds");
            for y in [0.02, 0.04, NOTIONAL_YIELD, 0.08] {
                let h = 1e-6;
                let up = celnet_bond::dirty_price(&bond, Rate(y + h)).expect("prices");
                let down = celnet_bond::dirty_price(&bond, Rate(y - h)).expect("prices");
                // -dP/dy x 1bp, per 100 face, then scaled to the contract.
                let numeric = -(up - down) / (2.0 * h) * 1e-4 * spec.point_value();
                let analytic = spec.dv01_per_contract(y).expect("DV01 derives");
                assert!(
                    (analytic - numeric).abs() < 1e-5 * numeric.abs().max(1.0),
                    "{}: analytic DV01 {analytic} vs numeric {numeric} at y={y}",
                    spec.instrument_id
                );
                assert!(
                    analytic > 0.0,
                    "{}: DV01 must be positive",
                    spec.instrument_id
                );
            }
        }
    }

    /// The derived DV01s, pinned. These are USD per contract per basis point, and
    /// they size real hedges — a silent drift here mis-sizes every corporate hedge,
    /// so the figures are frozen and any change must be a reviewed one.
    ///
    /// The `@6%` column is the standardized figure at the contract's own notional
    /// yield; the `@4%` column is what the same derivation gives at a market-like
    /// yield level, and is included because it is the column a hedge is actually
    /// sized off. The `@4%` values sit in the right neighbourhood of the exchange's
    /// published daily basis-point values for these contracts, which is the sanity
    /// check that the whole derivation is anchored — it is not asserted as an
    /// equality, because the exchange figure tracks the live cheapest-to-deliver and
    /// this one tracks the notional deliverable (see the module docs).
    #[test]
    fn standardized_dv01_per_contract_is_pinned() {
        // (symbol, DV01 @ 6% notional yield, DV01 @ 4%)
        let expected: [(&str, f64, f64); 6] = [
            ("ZT", 34.2707, 35.8447),
            ("ZF", 40.1992, 44.2820),
            ("ZN", 58.1151, 66.9464),
            ("TN", 72.5671, 86.9854),
            ("ZB", 115.5739, 160.7651),
            ("UB", 133.8721, 203.4488),
        ];
        let front = &treasury_futures_universe()[..TREASURY_FUTURES_TERMS.len()];
        for (spec, (sym, at_notional, at_market)) in front.iter().zip(expected) {
            assert_eq!(spec.terms.symbol, sym);
            let got_notional = spec.dv01_at_notional_yield().expect("derives");
            let got_market = spec.dv01_per_contract(0.04).expect("derives");
            assert!(
                (got_notional - at_notional).abs() < 5e-4,
                "{sym}: DV01 at the notional yield drifted to {got_notional}"
            );
            assert!(
                (got_market - at_market).abs() < 5e-4,
                "{sym}: DV01 at 4% drifted to {got_market}"
            );
        }
    }

    #[test]
    fn dv01_rises_with_maturity_and_falls_as_yields_rise() {
        // The curve is monotone in duration across the complex, and every contract's
        // DV01 falls as the discounting yield rises — two structural properties any
        // correct derivation must have.
        let front = &treasury_futures_universe()[..TREASURY_FUTURES_TERMS.len()];
        let dv01s: Vec<f64> = front
            .iter()
            .map(|s| s.dv01_at_notional_yield().expect("derives"))
            .collect();
        for pair in dv01s.windows(2) {
            assert!(
                pair[1] > pair[0],
                "DV01 must increase along the curve: {pair:?}"
            );
        }
        for spec in front {
            let low = spec.dv01_per_contract(0.03).expect("derives");
            let high = spec.dv01_per_contract(0.09).expect("derives");
            assert!(
                low > high,
                "{}: DV01 must fall as yields rise ({low} -> {high})",
                spec.instrument_id
            );
        }
    }

    #[test]
    fn hedge_contract_count_is_the_dv01_ratio() {
        let spec = treasury_futures_universe()
            .into_iter()
            .find(|s| s.terms.symbol == "ZN")
            .expect("a 10-Year contract");
        let contract_dv01 = spec.dv01_per_contract(0.04).expect("derives");
        // The worked example from the requirement: a $25,000/bp book.
        let expected = (25_000.0 / contract_dv01).round() as i64;
        assert_eq!(spec.hedge_contracts(25_000.0, 0.04).unwrap(), expected);
        // The sign follows the exposure, and a flat book needs no hedge.
        assert_eq!(spec.hedge_contracts(-25_000.0, 0.04).unwrap(), -expected);
        assert_eq!(spec.hedge_contracts(0.0, 0.04).unwrap(), 0);
        assert_eq!(
            spec.hedge_contracts(f64::NAN, 0.04),
            Err(FutureSpecError::BadYield)
        );
        assert_eq!(
            spec.dv01_per_contract(f64::INFINITY),
            Err(FutureSpecError::BadYield)
        );
    }

    // --- universe shape, identity and dates -------------------------------------

    #[test]
    fn the_universe_lists_every_product_for_every_cycle_month() {
        let u = treasury_futures_universe();
        assert_eq!(
            u.len(),
            TREASURY_FUTURES_TERMS.len() * LISTED_CONTRACT_MONTHS
        );

        // instrument_ids are unique — the registry and the LP feed both key on them.
        let ids: HashSet<&str> = u.iter().map(|s| s.instrument_id.as_str()).collect();
        assert_eq!(ids.len(), u.len(), "instrument_id collision");

        // The committed cycle starts Sep-26 and steps quarterly.
        assert!(ids.contains("ZNU26"), "front 10-Year contract missing");
        assert!(ids.contains("ZNZ26"));
        assert!(ids.contains("ZNH27"));
        for s in &u {
            assert_eq!(s.currency, "USD");
            assert_eq!(s.region, "us");
            assert_eq!(s.sub_asset_type, "government_future");
            assert_eq!(s.calendars, vec!["united_states"]);
            assert!(s.delivery_month_start.is_valid());
            assert!(s.first_delivery_date.is_valid());
            assert!(s.last_trading_date.is_valid());
            assert!(s.last_delivery_date.is_valid());
            assert!(s.name.contains(s.terms.label));
            // The id encodes the delivery month it belongs to.
            let code = delivery_month_code(s.delivery_month_start.month).unwrap();
            assert!(s.instrument_id.contains(code), "{}", s.instrument_id);
        }
        // A non-cycle month lists nothing.
        assert!(contracts_for_delivery_month(2026, 10).is_empty());
    }

    #[test]
    fn expiry_and_delivery_dates_follow_the_two_rulebook_conventions() {
        // December 2026. The last business day is Thu 31 Dec (Christmas Day, Fri
        // 25 Dec, is a holiday). Stepping back the seven closed business days
        // 30, 29, 28, 24, 23, 22, 21 puts the 10-Year/Bond last trading day on
        // Mon 21 Dec; the 2-Year and 5-Year trade through to the 31st.
        let dec = contracts_for_delivery_month(2026, 12);
        let by = |sym: &str| {
            dec.iter()
                .find(|s| s.terms.symbol == sym)
                .expect("contract")
                .clone()
        };
        let zn = by("ZN");
        assert_eq!(zn.last_trading_date, CivilYmd::new(2026, 12, 21));
        assert_eq!(zn.last_delivery_date, CivilYmd::new(2026, 12, 31));
        let zt = by("ZT");
        assert_eq!(zt.last_trading_date, CivilYmd::new(2026, 12, 31));
        // Three business days after 31 Dec 2026: Fri 1 Jan is New Year's Day, so
        // Mon 4, Tue 5, Wed 6 January.
        assert_eq!(zt.last_delivery_date, CivilYmd::new(2027, 1, 6));
        // 1 December 2026 is a Tuesday — a business day — so it is the first
        // delivery day for every product.
        for s in &dec {
            assert_eq!(s.first_delivery_date, CivilYmd::new(2026, 12, 1));
            assert_eq!(s.delivery_month_start, CivilYmd::new(2026, 12, 1));
        }

        // September 2026: 1 Sep is a Tuesday (Labor Day is Mon 7 Sep), the last
        // business day is Wed 30 Sep, and stepping back seven business days
        // 29, 28, 25, 24, 23, 22, 21 gives Mon 21 Sep.
        let sep = contracts_for_delivery_month(2026, 9);
        let sep_zn = sep.iter().find(|s| s.terms.symbol == "ZN").unwrap();
        assert_eq!(sep_zn.first_delivery_date, CivilYmd::new(2026, 9, 1));
        assert_eq!(sep_zn.last_trading_date, CivilYmd::new(2026, 9, 21));
        assert_eq!(sep_zn.last_delivery_date, CivilYmd::new(2026, 9, 30));
    }

    #[test]
    fn good_friday_is_excluded_from_the_exchange_calendar() {
        // The published Easter Sundays; Good Friday is two days earlier. This pins
        // the computus, which the March/April expiry dates depend on.
        let easters = [
            (2024, 3, 31),
            (2025, 4, 20),
            (2026, 4, 5),
            (2027, 3, 28),
            (2028, 4, 16),
            (2029, 4, 1),
            (2030, 4, 21),
        ];
        for (y, m, d) in easters {
            let easter = to_date(CivilYmd::new(y, m, d)).unwrap();
            assert_eq!(easter_sunday(y), Some(easter), "Easter {y}");
            assert!(
                is_good_friday(easter - Duration::days(2)),
                "Good Friday {y}"
            );
            assert!(!is_good_friday(easter));
        }
        // Good Friday 2027 falls on 26 March, inside the March-2027 expiry window,
        // so the 10-Year last trading day is 19 March rather than the 22nd a
        // federal-holidays-only calendar would give.
        let mar27 = contracts_for_delivery_month(2027, 3);
        let zn = mar27.iter().find(|s| s.terms.symbol == "ZN").unwrap();
        assert_eq!(zn.last_trading_date, CivilYmd::new(2027, 3, 19));
        // ...and the 2-Year still trades to the last business day, Wed 31 March.
        let zt = mar27.iter().find(|s| s.terms.symbol == "ZT").unwrap();
        assert_eq!(zt.last_trading_date, CivilYmd::new(2027, 3, 31));
    }

    // --- contract lifecycle: expiry, listing and the quarterly roll ---------------

    #[test]
    fn a_contract_stops_being_listed_the_day_after_it_stops_trading() {
        let sep = contracts_for_delivery_month(2026, 9);
        let zn = sep.iter().find(|s| s.terms.symbol == "ZN").unwrap();
        // Sep-26 10-Year last trades Mon 21 Sep 2026.
        assert_eq!(zn.last_trading_date, CivilYmd::new(2026, 9, 21));
        assert!(zn.is_listed_on(CivilYmd::new(2026, 9, 20)));
        // The last trading day itself still trades — expiry is strictly after it.
        assert!(zn.is_listed_on(CivilYmd::new(2026, 9, 21)));
        assert!(!zn.is_listed_on(CivilYmd::new(2026, 9, 22)));
        assert!(zn.is_expired_on(CivilYmd::new(2027, 1, 1)));
        // An unreal valuation date fails closed — never "still trading".
        assert!(zn.is_expired_on(CivilYmd::new(2026, 2, 30)));
    }

    #[test]
    fn the_front_month_rolls_when_the_front_contract_stops_trading() {
        // Well before the first cessation, the front 10-Year is the Sep-26 contract.
        let before = CivilYmd::new(2026, 6, 1);
        let front = front_contract("ZN", before).expect("a front 10-Year");
        assert_eq!(front.instrument_id, "ZNU26");

        // On its last trading day it is still the front month...
        let ltd = front.last_trading_date;
        assert_eq!(front_contract("ZN", ltd).unwrap().instrument_id, "ZNU26");
        // ...and the day after, the roll has happened: Dec-26 is the front month.
        let after = CivilYmd::new(ltd.year, ltd.month, ltd.day + 1);
        assert_eq!(front_contract("ZN", after).unwrap().instrument_id, "ZNZ26");

        // The short-end notes trade through month end, so they roll later than the
        // 10-Year in the same delivery month — the front month is per product, and
        // resolving it per product is the whole point.
        assert_eq!(front_contract("ZT", after).unwrap().instrument_id, "ZTU26");

        // Past the whole committed cycle there is no front contract, rather than a
        // stale one: the cycle needs rolling (a reviewed edit of LISTED_CYCLE_START).
        assert!(front_contract("ZN", CivilYmd::new(2030, 1, 1)).is_none());
        // An unlisted product code never resolves.
        assert!(front_contract("XX", before).is_none());
    }

    /// A hedge vehicle configured as a PRODUCT rolls itself; one configured as a
    /// delivery month does not. This is the pair a hedge policy is resolved through:
    /// `is_product_symbol` decides whether an id is standing intent or a fixed market,
    /// and `front_contract_id` answers what that intent trades today.
    #[test]
    fn a_product_symbol_resolves_to_the_front_contract_and_rolls_with_it() {
        // A product symbol is intent; a delivery month is a specific market.
        assert!(is_product_symbol("ZF"), "ZF is the 5-Year product");
        assert!(
            !is_product_symbol("ZFU26"),
            "a delivery month is NOT a product — it must never be silently re-pointed"
        );
        assert!(
            !is_product_symbol("US912810TM0"),
            "a cash bond is not a product"
        );
        assert!(!is_product_symbol(""), "an empty id is not a product");

        // A desk that has traded the contract for twenty years types the floor symbol.
        assert!(
            is_product_symbol("FV"),
            "FV is the legacy 5-Year floor symbol"
        );
        let before = CivilYmd::new(2026, 6, 1);
        assert_eq!(
            front_contract_id("FV", before).as_deref(),
            Some("ZFU26"),
            "a legacy floor symbol resolves to its electronic product's front month"
        );

        // The roll: the SAME configured vehicle points at Sep-26 before the 10-Year's
        // cessation and at Dec-26 after it, with no edit to the policy.
        assert_eq!(front_contract_id("ZN", before).as_deref(), Some("ZNU26"));
        let ltd = front_contract("ZN", before)
            .expect("a front 10-Year")
            .last_trading_date;
        let after = CivilYmd::new(ltd.year, ltd.month, ltd.day + 1);
        assert_eq!(
            front_contract_id("ZN", after).as_deref(),
            Some("ZNZ26"),
            "past the front contract's last trading day the vehicle rolls itself"
        );

        // Every id it returns is one the venue is quoting on that date — the invariant
        // that stops a rolled hedge from being routed at a dead market.
        let listed = listed_universe_on(after);
        for terms in &TREASURY_FUTURES_TERMS {
            if let Some(id) = front_contract_id(terms.symbol, after) {
                assert!(
                    listed.iter().any(|s| s.instrument_id == id),
                    "front contract {id} must be in the listed universe on that date"
                );
            }
        }

        // Declines rather than fabricates: an unlisted product, and a cycle fully past.
        assert!(front_contract_id("XX", before).is_none());
        assert!(front_contract_id("ZN", CivilYmd::new(2030, 1, 1)).is_none());
    }

    #[test]
    fn every_product_has_a_front_month_over_the_committed_cycle() {
        let as_of = CivilYmd::new(2026, 6, 1);
        let fronts = front_contracts(as_of);
        assert_eq!(fronts.len(), TREASURY_FUTURES_TERMS.len());
        // One per product, in the terms-table order, and each is genuinely listed.
        for (spec, terms) in fronts.iter().zip(TREASURY_FUTURES_TERMS.iter()) {
            assert_eq!(spec.terms.symbol, terms.symbol);
            assert!(spec.is_listed_on(as_of));
            // The front month is the nearest delivery among that product's listings.
            let nearest = treasury_futures_universe()
                .into_iter()
                .filter(|s| s.terms.symbol == terms.symbol)
                .map(|s| s.delivery_month_start)
                .min_by_key(|d| (d.year, d.month))
                .unwrap();
            assert_eq!(spec.delivery_month_start, nearest);
        }
        // Every front month carries a derivable DV01 — it is a hedge vehicle.
        for spec in &fronts {
            assert!(spec.dv01_at_notional_yield().expect("derives") > 0.0);
        }
    }

    #[test]
    fn the_listed_universe_is_the_committed_one_minus_the_expired() {
        // Before any cessation the two coincide exactly.
        let early = CivilYmd::new(2026, 6, 1);
        assert_eq!(
            listed_universe_on(early).len(),
            treasury_futures_universe().len()
        );
        // Once the Sep-26 month has run off, only the two later months remain.
        let after_sep = CivilYmd::new(2026, 10, 1);
        let rest = listed_universe_on(after_sep);
        assert_eq!(rest.len(), TREASURY_FUTURES_TERMS.len() * 2);
        assert!(rest.iter().all(|s| s.delivery_month_start.month != 9));
        // And past the whole cycle, nothing is listed.
        assert!(listed_universe_on(CivilYmd::new(2030, 1, 1)).is_empty());
    }

    /// The committed cycle must not be stale relative to the reference snapshot the
    /// rest of the platform values against. If this goes RED, roll
    /// [`LISTED_CYCLE_START`] — that is the reviewed edit the constant's docs describe.
    #[test]
    fn the_committed_cycle_is_entirely_unexpired_at_the_snapshot_valuation_date() {
        // The valuation date the securities-master snapshot and the LP simulator use.
        let snapshot = CivilYmd::new(2026, 4, 16);
        let u = treasury_futures_universe();
        assert_eq!(listed_universe_on(snapshot).len(), u.len());
        for s in &u {
            assert!(
                s.is_listed_on(snapshot),
                "{} has already expired at the snapshot date — roll LISTED_CYCLE_START",
                s.instrument_id
            );
        }
    }

    // --- quotation convention ----------------------------------------------------

    #[test]
    fn prices_snap_to_the_outright_tick_and_never_cross() {
        for spec in treasury_futures_universe() {
            let tick = spec.terms.tick_size_points;
            let mid = 111.4013;
            let bid = spec.round_bid_to_tick(mid - tick / 2.0);
            let offer = spec.round_offer_to_tick(mid + tick / 2.0);
            assert!(bid <= offer, "{}: tick snap crossed", spec.instrument_id);
            for px in [bid, offer] {
                let ticks = px / tick;
                assert!(
                    (ticks - ticks.round()).abs() < 1e-9,
                    "{}: {px} is off the tick grid",
                    spec.instrument_id
                );
            }
            // A price already on the grid is unchanged by either snap.
            let on_grid = (mid / tick).round() * tick;
            assert!((spec.round_bid_to_tick(on_grid) - on_grid).abs() < 1e-12);
            assert!((spec.round_offer_to_tick(on_grid) - on_grid).abs() < 1e-12);
        }
    }

    #[test]
    fn prices_render_in_points_and_32nds() {
        let u = treasury_futures_universe();
        let spec = |sym: &str| u.iter().find(|s| s.terms.symbol == sym).unwrap();

        // Half-32nd grid (10-Year): 110 + 16/32 and 110 + 16.5/32.
        let zn = spec("ZN");
        assert_eq!(zn.format_price_32nds(110.0 + 16.0 / 32.0), "110'160");
        assert_eq!(zn.format_price_32nds(110.0 + 16.5 / 32.0), "110'165");
        assert_eq!(zn.format_price_32nds(110.0 + 3.0 / 32.0), "110'030");

        // Whole-32nd grid (Bond / Ultra Bond) carries no sub-digit.
        assert_eq!(spec("ZB").format_price_32nds(115.0 + 16.0 / 32.0), "115'16");
        assert_eq!(spec("UB").format_price_32nds(130.0 + 31.0 / 32.0), "130'31");

        // Quarter-32nd grid (5-Year) uses the 0/2/5/7 digit map.
        let zf = spec("ZF");
        assert_eq!(zf.format_price_32nds(108.0 + 8.25 / 32.0), "108'082");
        assert_eq!(zf.format_price_32nds(108.0 + 8.75 / 32.0), "108'087");

        // Eighth-32nd grid (2-Year) uses the full 0/1/2/3/5/6/7/8 map.
        let zt = spec("ZT");
        assert_eq!(zt.format_price_32nds(102.0 + 20.125 / 32.0), "102'201");
        assert_eq!(zt.format_price_32nds(102.0 + 20.875 / 32.0), "102'208");

        assert_eq!(zn.format_price_32nds(f64::NAN), "n/a");
    }
}
