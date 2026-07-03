//! Combined (joint options + fixed-income) **tail-risk** vocabulary for the SDK —
//! fluent builders that lay a cross-risk-class portfolio and its aligned scenario set
//! onto the `RiskService.CombinedTailRisk` contract, plus the domain result type.
//!
//! This is the client parity of the landed server-side C2c unified risk cube
//! ([`celnet_risk_cube::combined_tail_risk`]) — the "true single VaR engine": ONE
//! non-additive tail over a portfolio's vanilla FX option legs AND its linear-FI
//! (OIS-swap) legs, by full joint bump-and-revalue over aligned
//! `(option-shock, rate-shift)` scenarios, plus the FI per-tenor signed key-rate DV01
//! ladder and signed parallel DV01. Every number is computed SERVER-SIDE (the
//! api-first parity rule: the SDK never reduces the tail itself); the builders here
//! only encode intent onto the one wire contract and decode the typed result.
//!
//! The portfolio travels INLINE on the request (a pure calculation — no store read,
//! no live-market read), so the same book yields the identical result on every
//! replica. The request supports three shapes, each with an ergonomic constructor:
//!
//! * [`CombinedTailRiskQuery::fi_only`] — an FI-only book (rate-shock scenarios) that
//!   reduces to the standalone fixed-income VaR/ES.
//! * [`CombinedTailRiskQuery::options_only`] — an options-only book (spot/vol-shock
//!   scenarios) that reduces to the options VaR/ES.
//! * [`CombinedTailRiskQuery::joint`] — a mixed book whose joint tail shows the
//!   cross-risk-class diversification (the joint VaR is NOT the naive sum of the two
//!   marginal VaRs — one market state per scenario nets the two P&Ls).
//!
//! Mirrors the fixed-income pricing vocabulary ([`crate::rates`]) and the
//! surface-scenario/axis vocabulary ([`crate::surface_vocab`]): the trader writes
//! intent and the builders produce the proto messages
//! [`crate::Client::combined_tail_risk`] sends.

use celnet_proto::{
    CombinedTailRiskRequest, CombinedTailRiskResponse, JointTailScenario, OisFixedPeriod,
    OisSwapLeg, TailRiskCurvePillar, TailRiskFiPosition, TailRiskOptionLeg, tail_risk_fi_position,
};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle};

use crate::error::{ClientError, ClientResult};
use crate::surface_vocab::MarketContext;

// ---------------------------------------------------------------------------
// portfolio inputs — the typed builders
// ---------------------------------------------------------------------------

/// One vanilla FX option leg of a combined tail-risk portfolio, valued from its raw
/// Garman-Kohlhagen inputs.
///
/// The canonical convention-free risk is re-derived SERVER-SIDE from these inputs
/// (never a convention-baked Greek); the quoted [`DeltaConvention`] / [`PremiumStyle`]
/// travel as provenance only and do not change the number (they default to the
/// interbank standard spot-unadjusted / domestic-pips and are overridable). Carried as
/// the flat two-rate FX carry (`r_dom`/`r_for`), matching the joint-tail FX leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TailOptionLeg {
    pair: CcyPair,
    option_type: OptionType,
    notional_base: f64,
    spot: f64,
    strike: f64,
    vol: f64,
    expiry_years: f64,
    r_dom: f64,
    r_for: f64,
    quoted_delta: DeltaConvention,
    premium_style: PremiumStyle,
}

impl TailOptionLeg {
    /// A vanilla FX **call** on `pair` of signed base-currency notional `notional_base`
    /// (positive = long the option), struck at `strike` and expiring in `expiry_years`
    /// (vol-time), valued off `market` (spot / vol / the two continuous rates).
    #[must_use]
    pub fn call(
        pair: CcyPair,
        notional_base: f64,
        strike: f64,
        expiry_years: f64,
        market: MarketContext,
    ) -> Self {
        Self::new(
            pair,
            OptionType::Call,
            notional_base,
            strike,
            expiry_years,
            market,
        )
    }

    /// A vanilla FX **put** — the [`TailOptionLeg::call`] counterpart.
    #[must_use]
    pub fn put(
        pair: CcyPair,
        notional_base: f64,
        strike: f64,
        expiry_years: f64,
        market: MarketContext,
    ) -> Self {
        Self::new(
            pair,
            OptionType::Put,
            notional_base,
            strike,
            expiry_years,
            market,
        )
    }

    fn new(
        pair: CcyPair,
        option_type: OptionType,
        notional_base: f64,
        strike: f64,
        expiry_years: f64,
        market: MarketContext,
    ) -> Self {
        Self {
            pair,
            option_type,
            notional_base,
            spot: market.spot,
            strike,
            vol: market.vol,
            expiry_years,
            r_dom: market.r_dom,
            r_for: market.r_for,
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
        }
    }

    /// Record the [`DeltaConvention`] the leg was quoted under (provenance; the
    /// canonical spot-unadjusted risk is unaffected).
    #[must_use]
    pub fn quoted_delta(mut self, convention: DeltaConvention) -> Self {
        self.quoted_delta = convention;
        self
    }

    /// Record the [`PremiumStyle`] the leg was quoted under (provenance; the canonical
    /// risk is unaffected).
    #[must_use]
    pub fn premium_style(mut self, style: PremiumStyle) -> Self {
        self.premium_style = style;
        self
    }

    fn to_wire(self) -> TailRiskOptionLeg {
        TailRiskOptionLeg {
            pair: Some(celnet_proto::CcyPair::from(self.pair)),
            option_type: celnet_proto::OptionType::from(self.option_type) as i32,
            notional_base: self.notional_base,
            spot: self.spot,
            strike: self.strike,
            vol: self.vol,
            t: self.expiry_years,
            r_dom: self.r_dom,
            r_for: self.r_for,
            quoted_delta: celnet_proto::DeltaConvention::from(self.quoted_delta) as i32,
            premium_style: celnet_proto::PremiumStyle::from(self.premium_style) as i32,
        }
    }
}

/// One accrual period of an OIS fixed leg, in curve year-fraction coordinates
/// (the SDK form of the wire `OisFixedPeriod`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OisAccrualPeriod {
    /// Payment time (period end) in curve year fractions; strictly increasing across
    /// the schedule.
    pub pay: f64,
    /// Year-fraction accrual for the period (e.g. ACT/360); strictly positive.
    pub accrual: f64,
}

impl OisAccrualPeriod {
    /// One accrual period paying at `pay` (curve year fractions) with day-count
    /// `accrual`.
    #[must_use]
    pub fn new(pay: f64, accrual: f64) -> Self {
        Self { pay, accrual }
    }

    fn to_wire(self) -> OisFixedPeriod {
        OisFixedPeriod {
            pay: self.pay,
            accrual: self.accrual,
        }
    }
}

/// One linear-FI leg of a combined tail-risk portfolio.
///
/// Currently the fixed-vs-OIS swap arm (valued by the self-discounting OIS identity
/// `N·(K·A − (DF(start) − DF(maturity)))` off the shocked discount curve); the arm set
/// grows additively (one contract, no versioning), mirroring
/// `celnet_rates_risk::FiPosition`.
#[derive(Debug, Clone, PartialEq)]
pub enum TailFiPosition {
    /// A fixed-vs-OIS swap with an explicit fixed-leg accrual schedule.
    OisSwap {
        /// The swap effective (start) time in curve year-fraction coordinates.
        start: f64,
        /// The ordered fixed-leg accrual periods (the final period's `pay` is maturity).
        periods: Vec<OisAccrualPeriod>,
        /// The fixed rate `K` paid/received (decimal, e.g. `0.033`).
        fixed_rate: f64,
        /// The swap notional `N` in curve-currency units (positive).
        notional: f64,
        /// `true` to receive fixed (the raw OIS-PV sign), `false` to pay fixed.
        receive_fixed: bool,
    },
}

impl TailFiPosition {
    /// A fixed-vs-OIS swap with an explicit accrual `schedule` (curve year fractions),
    /// `fixed_rate` `K`, `notional` `N`, and side (`receive_fixed`).
    #[must_use]
    pub fn ois_swap(
        start: f64,
        schedule: impl IntoIterator<Item = OisAccrualPeriod>,
        fixed_rate: f64,
        notional: f64,
        receive_fixed: bool,
    ) -> Self {
        Self::OisSwap {
            start,
            periods: schedule.into_iter().collect(),
            fixed_rate,
            notional,
            receive_fixed,
        }
    }

    /// A `years`-year annual OIS **receiving fixed** at `fixed_rate` on `notional`:
    /// start at time 0, one accrual period per year (period `i` pays at year `i` with a
    /// unit year-fraction accrual). The ergonomic form of the common vanilla swap.
    #[must_use]
    pub fn receive_fixed_annual(years: u32, fixed_rate: f64, notional: f64) -> Self {
        Self::annual(years, fixed_rate, notional, true)
    }

    /// A `years`-year annual OIS **paying fixed** — the
    /// [`TailFiPosition::receive_fixed_annual`] counterpart.
    #[must_use]
    pub fn pay_fixed_annual(years: u32, fixed_rate: f64, notional: f64) -> Self {
        Self::annual(years, fixed_rate, notional, false)
    }

    fn annual(years: u32, fixed_rate: f64, notional: f64, receive_fixed: bool) -> Self {
        let periods = (1..=years)
            .map(|i| OisAccrualPeriod::new(f64::from(i), 1.0))
            .collect();
        Self::OisSwap {
            start: 0.0,
            periods,
            fixed_rate,
            notional,
            receive_fixed,
        }
    }

    fn to_wire(&self) -> TailRiskFiPosition {
        let Self::OisSwap {
            start,
            periods,
            fixed_rate,
            notional,
            receive_fixed,
        } = self;
        TailRiskFiPosition {
            position: Some(tail_risk_fi_position::Position::OisSwap(OisSwapLeg {
                start: *start,
                periods: periods.iter().map(|p| p.to_wire()).collect(),
                fixed_rate: *fixed_rate,
                notional: *notional,
                receive_fixed: *receive_fixed,
            })),
        }
    }
}

/// The base discount curve the FI legs reprice off and the rate shocks perturb, as
/// continuously-compounded zero-rate pillars (an already-bootstrapped discount curve).
///
/// Distinct from [`crate::UsdSofrCurve`], which sends PAR-OIS quotes the server
/// bootstraps: the combined-tail wire carries the zero curve DIRECTLY (mirroring
/// `celnet_rates_risk::RatePillars`). Add pillars in strictly increasing time order;
/// the origin `(0, DF = 1)` is implicit. Required (≥ 1 pillar) even for an
/// options-only request — the FI key-rate ladder is then measured off it and is
/// all-zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscountCurve {
    pillars: Vec<(f64, f64)>,
}

impl DiscountCurve {
    /// An empty discount curve; add its pillars with [`DiscountCurve::pillar`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a `(time, continuously-compounded zero rate)` pillar. Times must be strictly
    /// increasing and positive (validated server-side).
    #[must_use]
    pub fn pillar(mut self, time_years: f64, zero_rate: f64) -> Self {
        self.pillars.push((time_years, zero_rate));
        self
    }

    /// The number of pillars — the length a per-pillar rate-shift vector must match
    /// (see [`JointShock::parallel_rate`] / [`JointShock::rate_shifts`]).
    #[must_use]
    pub fn pillar_count(&self) -> usize {
        self.pillars.len()
    }

    fn to_wire(&self) -> Vec<TailRiskCurvePillar> {
        self.pillars
            .iter()
            .map(|&(t, zero_rate)| TailRiskCurvePillar { t, zero_rate })
            .collect()
    }
}

/// One joint cross-risk-class scenario: an options spot / vol / carry shock applied to
/// the option legs AND a per-pillar rate shift applied to the FI legs, as ONE market
/// state (one historical draw / one scenario date, aligned by scenario index).
///
/// The two shocks share a scenario index, so the portfolio's per-scenario P&L is the
/// SUM of the option-leg and FI-leg P&L — the basis of the joint (diversifying)
/// VaR/ES. Mirrors `celnet_risk_cube::JointScenario`. Build the shocked state fluently
/// from [`JointShock::base`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JointShock {
    spot_rel: f64,
    vol_abs: f64,
    discount_abs: f64,
    carry_abs: f64,
    rate_shifts: Vec<f64>,
}

impl JointShock {
    /// The base market state — no shock on either risk class (an unshocked scenario is
    /// a deliberate, meaningful draw, not a silent default). Refine it fluently.
    #[must_use]
    pub fn base() -> Self {
        Self::default()
    }

    /// Set the relative spot shock (`0.01` = +1%: `spot *= 1 + spot_rel`).
    #[must_use]
    pub fn spot_rel(mut self, spot_rel: f64) -> Self {
        self.spot_rel = spot_rel;
        self
    }

    /// Set the absolute vol shock in vol units (`0.01` = +1 vol point: `vol += vol_abs`).
    #[must_use]
    pub fn vol_abs(mut self, vol_abs: f64) -> Self {
        self.vol_abs = vol_abs;
        self
    }

    /// Set the absolute discount-rate shock `Δr` (the option numeraire rate; `r_dom`
    /// for FX).
    #[must_use]
    pub fn discount_abs(mut self, discount_abs: f64) -> Self {
        self.discount_abs = discount_abs;
        self
    }

    /// Set the absolute net-carry shock `Δb` (`r_dom − r_for` for FX).
    #[must_use]
    pub fn carry_abs(mut self, carry_abs: f64) -> Self {
        self.carry_abs = carry_abs;
        self
    }

    /// Set the per-pillar absolute additive zero-rate shifts, aligned with the base
    /// curve (the FI `RateShock`). Its length must equal the base-curve pillar count
    /// when FI legs are present; it may be empty for an options-only request.
    #[must_use]
    pub fn rate_shifts(mut self, shifts: impl Into<Vec<f64>>) -> Self {
        self.rate_shifts = shifts.into();
        self
    }

    /// A uniform (parallel) absolute additive zero-rate shift of `shift` across
    /// `pillars` pillars — the common broad rate move.
    #[must_use]
    pub fn parallel_rate(self, pillars: usize, shift: f64) -> Self {
        self.rate_shifts(vec![shift; pillars])
    }

    fn to_wire(&self) -> JointTailScenario {
        JointTailScenario {
            spot_rel: self.spot_rel,
            vol_abs: self.vol_abs,
            discount_abs: self.discount_abs,
            carry_abs: self.carry_abs,
            rate_shifts: self.rate_shifts.clone(),
        }
    }
}

/// A fluent builder for a combined options + fixed-income tail-risk request: reduce an
/// inline cross-risk-class portfolio to ONE joint VaR/ES (plus the FI key-rate ladder
/// and signed parallel DV01) over an aligned scenario set at confidence `alpha`, all
/// SERVER-SIDE. Pass to [`crate::Client::combined_tail_risk`].
///
/// Use [`CombinedTailRiskQuery::fi_only`] for an FI-only book,
/// [`CombinedTailRiskQuery::options_only`] for an options-only book, or
/// [`CombinedTailRiskQuery::joint`] for a mixed book; extend any of them with
/// [`CombinedTailRiskQuery::option_leg`] / [`CombinedTailRiskQuery::fi_position`].
#[derive(Debug, Clone)]
pub struct CombinedTailRiskQuery {
    option_legs: Vec<TailOptionLeg>,
    fi_positions: Vec<TailFiPosition>,
    base_curve: DiscountCurve,
    scenarios: Vec<JointShock>,
    alpha: f64,
    correlation_id: Option<u64>,
}

impl CombinedTailRiskQuery {
    /// The joint options + FI tail: `option_legs` and `fi_positions` reduced together
    /// over `scenarios` off the base discount `curve` at confidence `alpha` (e.g.
    /// `0.99`; passed verbatim — `0.0` defers to the server's documented `0.99`
    /// default). Every input is explicit — no silent portfolio, scenario, or level.
    #[must_use]
    pub fn joint(
        curve: DiscountCurve,
        option_legs: impl IntoIterator<Item = TailOptionLeg>,
        fi_positions: impl IntoIterator<Item = TailFiPosition>,
        scenarios: impl IntoIterator<Item = JointShock>,
        alpha: f64,
    ) -> Self {
        Self {
            option_legs: option_legs.into_iter().collect(),
            fi_positions: fi_positions.into_iter().collect(),
            base_curve: curve,
            scenarios: scenarios.into_iter().collect(),
            alpha,
            correlation_id: None,
        }
    }

    /// The FI-only tail (empty option book): `fi_positions` reduced over rate-shock
    /// `scenarios` off `curve` at `alpha` → the fixed-income VaR/ES (plus the key-rate
    /// ladder and signed parallel DV01). Reduces to the standalone rate VaR.
    #[must_use]
    pub fn fi_only(
        curve: DiscountCurve,
        fi_positions: impl IntoIterator<Item = TailFiPosition>,
        scenarios: impl IntoIterator<Item = JointShock>,
        alpha: f64,
    ) -> Self {
        Self::joint(curve, [], fi_positions, scenarios, alpha)
    }

    /// The options-only tail (empty FI book): `option_legs` reduced over spot/vol-shock
    /// `scenarios` off `curve` at `alpha` → the options VaR/ES (the FI key-rate ladder
    /// is then all-zero and the parallel DV01 is `0`). Reduces to the options VaR.
    #[must_use]
    pub fn options_only(
        curve: DiscountCurve,
        option_legs: impl IntoIterator<Item = TailOptionLeg>,
        scenarios: impl IntoIterator<Item = JointShock>,
        alpha: f64,
    ) -> Self {
        Self::joint(curve, option_legs, [], scenarios, alpha)
    }

    /// Add one more vanilla FX option leg to the portfolio.
    #[must_use]
    pub fn option_leg(mut self, leg: TailOptionLeg) -> Self {
        self.option_legs.push(leg);
        self
    }

    /// Add one more linear-FI leg to the portfolio.
    #[must_use]
    pub fn fi_position(mut self, position: TailFiPosition) -> Self {
        self.fi_positions.push(position);
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    pub(crate) fn to_wire(&self, session_token: Option<String>) -> CombinedTailRiskRequest {
        CombinedTailRiskRequest {
            option_legs: self
                .option_legs
                .iter()
                .copied()
                .map(TailOptionLeg::to_wire)
                .collect(),
            fi_positions: self
                .fi_positions
                .iter()
                .map(TailFiPosition::to_wire)
                .collect(),
            base_curve: self.base_curve.to_wire(),
            scenarios: self.scenarios.iter().map(JointShock::to_wire).collect(),
            alpha: self.alpha,
            correlation_id: self.correlation_id,
            session_token,
        }
    }
}

// ---------------------------------------------------------------------------
// result — the typed decode of the wire response
// ---------------------------------------------------------------------------

/// A Value-at-Risk / Expected-Shortfall pair — both **non-negative loss magnitudes**
/// (the platform-wide tail convention). VaR is the `alpha`-quantile loss; ES is the
/// mean loss in the tail at or beyond it, so `es ≥ var ≥ 0` always. The typed form of
/// the wire `VarEs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarEs {
    /// Value-at-Risk: the `alpha`-quantile loss (a positive number is a loss).
    pub var: f64,
    /// Expected Shortfall: the mean loss in the tail at or beyond the VaR quantile.
    pub es: f64,
}

/// One point of the FI signed key-rate DV01 ladder: the signed ΔPV per +1bp up-bump of
/// one curve pillar's zero rate. The tenor is a real pillar time (fractional tenors
/// allowed) so it is an `f64`. A rate rise is a loss ⇒ negative for a long bond /
/// receive-fixed swap. The typed form of the wire `TailRiskKeyRate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TailKeyRate {
    /// The pillar tenor in years.
    pub tenor_years: f64,
    /// The signed DV01 at this tenor.
    pub dv01: f64,
}

/// The combined tail-risk result from [`crate::Client::combined_tail_risk`]: the joint
/// options + FI tail, the FI per-tenor key-rate DV01 ladder, and the FI signed parallel
/// DV01. The typed form of the wire `CombinedTailRiskResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct CombinedTailRisk {
    /// The joint options-spot/vol + FI-rate tail — ONE non-additive VaR/ES over the
    /// union of the two risk classes (the diversifying joint reduction, not a marginal
    /// sum).
    pub joint_var_es: VarEs,
    /// The FI per-tenor signed key-rate DV01 ladder, ascending by pillar tenor (empty
    /// when there are no FI legs; one point per base-curve pillar otherwise).
    pub key_rate: Vec<TailKeyRate>,
    /// The FI signed parallel DV01 (`0` when there are no FI legs; negative for a long
    /// bond / receive-fixed swap, which loses on a rate rise).
    pub fi_parallel_dv01: f64,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

impl CombinedTailRisk {
    pub(crate) fn from_wire(w: CombinedTailRiskResponse) -> ClientResult<Self> {
        let jve = w.joint_var_es.ok_or(ClientError::MissingField(
            "CombinedTailRiskResponse.joint_var_es",
        ))?;
        Ok(Self {
            joint_var_es: VarEs {
                var: jve.var,
                es: jve.es,
            },
            key_rate: w
                .key_rate
                .into_iter()
                .map(|k| TailKeyRate {
                    tenor_years: k.tenor_years,
                    dv01: k.dv01,
                })
                .collect(),
            fi_parallel_dv01: w.fi_parallel_dv01,
            correlation_id: w.correlation_id,
        })
    }
}
