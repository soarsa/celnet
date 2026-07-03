//! The linear fixed-income pricing-model contract — the FI analog of
//! [`crate::PricingModel`].
//!
//! [`RatesPricingModel`] is the seam a house/user model implements to price a
//! **linear** fixed-income instrument (an OIS, a vanilla IRS, a FRA, or a cash
//! bond) off a discount curve, so fixed-income analytics are user-extensible on
//! the *same* registry the option [`crate::PricingModel`] rides — the FI half of
//! the uniform-asset-class dispatch (ADR-0021). Where a [`crate::PricingModel`]
//! receives the generalized carry-tagged [`celnet_core::CarryInputs`] and returns
//! the option Greek strip, a [`RatesPricingModel`] receives the instrument's
//! neutral economic [`RatesTerms`] plus the calibrating discount curve as
//! [`RatesCurvePillar`]s, and returns the linear-FI [`RatesMeasures`]
//! (PV / par / PV01 / DV01 / key-rate ladder).
//!
//! # Why the neutral vocabulary
//!
//! This crate is intentionally dependency-light (only [`celnet_types`] and
//! [`celnet_core`], never a wire/proto or a rates-engine crate — see the crate
//! docs), so the contract names its **own** neutral vocabulary
//! ([`RatesProductKind`], [`RatesFrequency`], [`RatesAccrualBasis`],
//! [`RatesTerms`]) rather than any server-internal or wire type. That is exactly
//! how [`crate::PricingModel`] names [`celnet_core::CarryInputs`] and not the
//! FX-only wire `Vanilla`: the SDK contract is the reduced economic form, and the
//! platform's edge is responsible for translating its wire request into this
//! neutral shape (and the returned measures back). A model authored against this
//! contract prices identically whether registered natively (Tier-0) or, in a
//! later increment, hosted in the sandbox.
//!
//! # Determinism obligations (contract, not convention)
//!
//! As for [`crate::PricingModel`]: implementors must be deterministic (identical
//! inputs ⇒ bit-identical output), route transcendentals through
//! `celnet_core::math`, and never compare floats with `==` or assert on `NaN`.

use celnet_types::{BrokenDate, DayCount};

use crate::descriptor::ModelDescriptor;
use crate::error::PluginResult;

/// Which linear fixed-income product family a [`RatesPricingModel`] prices.
///
/// A house model is registered against exactly one kind (a desk's OIS model and
/// its bond model are distinct registrations), so the dispatch resolves the
/// registered model **by kind** — the FI counterpart of resolving the active
/// option pricing model. An unregistered kind falls through to the native path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RatesProductKind {
    /// An overnight-index (OIS) swap.
    Ois,
    /// A vanilla fixed-vs-float interest-rate swap.
    Irs,
    /// A forward-rate agreement.
    Fra,
    /// A fixed-coupon cash bond.
    Bond,
}

/// A leg payment frequency (the SDK's neutral fixed-income vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RatesFrequency {
    /// One payment per year.
    Annual,
    /// Two payments per year.
    SemiAnnual,
    /// Four payments per year.
    Quarterly,
}

/// A leg accrual basis (the day-count superset that additionally carries the
/// 30/360 bond basis, which [`DayCount`] does not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RatesAccrualBasis {
    /// Actual/360.
    Act360,
    /// Actual/365 (fixed).
    Act365Fixed,
    /// 30/360 (bond basis).
    Thirty360BondBasis,
}

/// One calibrating discount-curve pillar handed to a rates model: the pillar's
/// ACT/365F year-fraction `t` from curve time-0 to its final pay date, and the
/// observed par rate at that pillar. A model bootstraps its own discount curve
/// from the ordered pillar set exactly as the native path does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatesCurvePillar {
    /// The pillar's year-fraction (ACT/365F) from curve time-0.
    pub t: f64,
    /// The observed par rate at the pillar (decimal, e.g. `0.042` = 4.2%).
    pub par_rate: f64,
}

impl RatesCurvePillar {
    /// Construct a curve pillar from its time and par rate.
    #[must_use]
    pub const fn new(t: f64, par_rate: f64) -> Self {
        Self { t, par_rate }
    }
}

/// The neutral economic terms of a linear fixed-income instrument, kind-tagged —
/// a faithful, proto-free echo of the wire instrument the edge decoded.
///
/// Every field is the trade's own economics; the `receive_fixed` / `long` flag
/// carries the client's direction so the model owns its own sign convention
/// (the native path applies the equivalent side sign after its engine call).
#[derive(Debug, Clone, PartialEq)]
pub enum RatesTerms {
    /// An OIS: whole-year tenor, fixed rate, notional, and whether the client
    /// receives fixed.
    Ois {
        /// Tenor in whole years.
        tenor_years: u32,
        /// The contract fixed rate (decimal).
        fixed_rate: f64,
        /// Notional (positive).
        notional: f64,
        /// `true` if the client receives fixed (pays float), `false` if it pays
        /// fixed.
        receive_fixed: bool,
    },
    /// A vanilla fixed-vs-float IRS with independent leg frequencies and
    /// day-counts.
    Irs {
        /// Tenor in whole years.
        tenor_years: u32,
        /// The fixed-leg rate (decimal).
        fixed_rate: f64,
        /// Notional (positive).
        notional: f64,
        /// `true` if the client receives fixed (pays float).
        receive_fixed: bool,
        /// The fixed-leg payment frequency.
        fixed_frequency: RatesFrequency,
        /// The float-leg payment frequency.
        float_frequency: RatesFrequency,
        /// The fixed-leg day-count.
        fixed_day_count: DayCount,
        /// The float-leg day-count.
        float_day_count: DayCount,
    },
    /// A FRA over the `[start_months, end_months]` window.
    Fra {
        /// Months from spot to the accrual-window start.
        start_months: u32,
        /// Months from spot to the accrual-window end (`> start_months`).
        end_months: u32,
        /// The agreed fixed rate (decimal).
        fixed_rate: f64,
        /// Notional (positive).
        notional: f64,
        /// `true` if the client receives fixed.
        receive_fixed: bool,
        /// The accrual basis of the window.
        accrual: RatesAccrualBasis,
    },
    /// A fixed-coupon cash bond.
    Bond {
        /// The annual coupon rate (decimal).
        coupon_rate: f64,
        /// The redemption (face) amount.
        redemption: f64,
        /// The coupon payment frequency.
        coupon_frequency: RatesFrequency,
        /// The coupon accrual basis.
        day_count: RatesAccrualBasis,
        /// The maturity date.
        maturity: BrokenDate,
        /// `true` if the client is long (bought) the bond, `false` if short.
        long: bool,
    },
}

impl RatesTerms {
    /// The [`RatesProductKind`] this terms variant describes.
    #[must_use]
    pub const fn product_kind(&self) -> RatesProductKind {
        match self {
            RatesTerms::Ois { .. } => RatesProductKind::Ois,
            RatesTerms::Irs { .. } => RatesProductKind::Irs,
            RatesTerms::Fra { .. } => RatesProductKind::Fra,
            RatesTerms::Bond { .. } => RatesProductKind::Bond,
        }
    }
}

/// The linear-FI measures a [`RatesPricingModel`] produces — the neutral output
/// shape the edge maps onto its wire result.
///
/// The `key_rate_ladder` is an owned, variable-length per-calibrating-pillar
/// sensitivity (empty when the model reports no curve-space decomposition, e.g. a
/// closed-form yield-space bond DV01); this is a **request-tier** measure, so the
/// allocation lives here rather than on the `Copy` option hot path.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesMeasures {
    /// Present value in the curve currency, side-signed to the client.
    pub pv: f64,
    /// The fair / break-even rate (par swap rate, FRA break-even, or bond yield),
    /// side-independent.
    pub par_rate: f64,
    /// The PV01 (present value of a 1bp fixed-rate move), side-signed.
    pub pv01: f64,
    /// The DV01 (present value of a 1bp curve move), side-signed.
    pub dv01: f64,
    /// The per-calibrating-pillar key-rate DV01 ladder (may be empty).
    pub key_rate_ladder: Vec<f64>,
}

/// A model that prices a linear fixed-income instrument off a discount curve.
///
/// The FI analog of [`crate::PricingModel`]: it declares its identity + kind
/// through a [`ModelDescriptor`] (whose [`crate::ModelKind`] is
/// [`crate::ModelKind::RatesPricing`]) so the registry can route to it, and
/// prices the instrument's [`RatesTerms`] against the calibrating curve.
pub trait RatesPricingModel {
    /// Self-description used by the registry to route work to this model. Its
    /// [`ModelDescriptor::kind`] must be [`crate::ModelKind::RatesPricing`].
    fn descriptor(&self) -> ModelDescriptor;

    /// Price `terms` against the calibrating `curve`, returning the linear-FI
    /// measures.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PluginError::InvalidInput`] for terms/curve outside the
    /// model's domain, [`crate::PluginError::Unsupported`] if the model does not
    /// price the given [`RatesProductKind`], or
    /// [`crate::PluginError::DidNotConverge`] for an internal solve failure.
    fn price(&self, terms: &RatesTerms, curve: &[RatesCurvePillar]) -> PluginResult<RatesMeasures>;
}
