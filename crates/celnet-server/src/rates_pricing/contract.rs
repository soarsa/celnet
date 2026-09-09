//! The central pricing + risk **contract**, re-seated onto the linear fixed-income
//! leaf (ADR-0017 **Phase B**).
//!
//! This is the FI counterpart of the options re-seat in
//! [`crate::pricer::contract`] (Phase A1 FX vanilla / A2 cross-asset / A2b exotics):
//! it realises the `celnet_core` contract ([`celnet_core::contract`]) on the
//! *linear* fixed-income paradigm — a bootstrapped discount curve, an OIS, and a
//! cash bond — **wrapping** the established `celnet-rates` / `celnet-bond` engines
//! verbatim (never reimplementing their math, `docs/CLAUDE.md`):
//!
//! * [`RatesCurveResolver`] wraps the curve-bootstrap step ([`bootstrap_ois`]) as a
//!   [`MarketResolver`], producing a [`ResolvedMarket`] whose `discount` leg is the
//!   bootstrapped [`celnet_rates::Curve`] (which impls [`celnet_types::DiscountCurve`]) —
//!   the FI counterpart of the FX `FxSurfaceResolver`. `foreign`/`vol`/`spot` are
//!   `None`: a pure-rates market is single-curve and carries no spot or volatility.
//! * [`RatesOisEngine`] and [`BondEngine`] implement [`Priceable`] — the leaf-level
//!   re-seat. Each `price` calls the identical established pricing body
//!   ([`super::price_ois`] wrapping [`celnet_rates::ois_risk`]/[`ois_par_rate`];
//!   [`celnet_bond::price_from_curve`]/[`bond_risk`]), so the FI numbers are
//!   byte-for-byte unchanged; each `risk` reports the unified tagged
//!   [`RiskMeasure::RateLadder`].
//!
//! # Linear FI stays linear (no carry-kernel, no `Underlying` hack)
//!
//! A swap / bond is **not** an option: it is priced by discounting fixed cashflows,
//! not by the asset-class-agnostic carry option kernel. So the FI leaves do NOT go
//! through the FX carry seam, do NOT touch the `Underlying` enum, and report the
//! purely-additive [`RiskMeasure::RateLadder`] tag (PV + parallel PV01 / DV01), the
//! linear-FI arm of the unified measure — never the nonlinear
//! [`RiskMeasure::OptionGreeks`] strip.
//!
//! # Where the key-rate ladder lives (the request tier, not core)
//!
//! [`celnet_core::contract::RateLadder`] is the **additive, allocation-free, `Copy`**
//! scalar measure `{pv, pv01, dv01}` that flows into the unified risk cube; its
//! `combine`/`Add` is the linear roll-up seam. The per-calibrating-instrument
//! **key-rate ladder** ([`OisRisk::key_rate`](celnet_rates::OisRisk)) is a
//! dynamically-sized owned `Vec` whose cross-instrument roll-up needs a common
//! bucket axis — a Phase-C (risk-cube) concern. Folding it into the additive core
//! measure would (a) force `celnet-core` to allocate and drop `Copy` (rippling the
//! untouched options path) and (b) prematurely commit to a positional bucket
//! semantics Phase C owns. So — exactly as the core contract doc already
//! provisioned ("the per-tenor key-rate bucket ladder … is populated by the FI leaf
//! at the request tier in Phase B") — the ladder is carried **here, at the request
//! tier**, in the leaf's [`Priceable::Priced`] result (the wire
//! [`celnet_proto::RatesPricingResult`], which `celnet-server` may allocate), and
//! [`Priceable::risk`] lifts the scalar `{pv, pv01, dv01}` into the core measure —
//! precisely mirroring how the options leaves carry the full Greek strip in
//! `Priced` and lift a `CarryGreeks` view in `risk`.

use celnet_bond::{Bond, BondError, BondRisk, bond_risk, price_from_curve};
use celnet_core::contract::{MarketResolver, Priceable, RateLadder, ResolvedMarket, RiskMeasure};
use celnet_proto::{RatesPriceRequest, RatesPricingResult, rates_instrument};
use celnet_rates::{AccrualBasis, Curve, OisQuote, bootstrap_ois};
use time::Date;

use super::{
    RatesPriceError, build_quotes, price_bond_instrument, price_fra, price_irs, price_ois,
    resolve_date,
};

/// The resolved **rates conventions** a linear-FI [`ResolvedMarket`] carries — the
/// FI counterpart of the FX `ConventionSet`.
///
/// A linear rates market has no delta/ATM/premium option conventions; what it does
/// carry is the fixed-leg **accrual basis** the par-quote convention assumes (ACT/360
/// for USD-SOFR). It is a genuinely-populated, `Copy` handle (not a `()` stand-in)
/// so the produced [`ResolvedMarket`] is a real market context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RatesConventions {
    /// The fixed-leg day-count / accrual basis the calibrating par quotes assume.
    pub(crate) accrual: AccrualBasis,
}

impl RatesConventions {
    /// Assemble the rates conventions from the fixed-leg accrual basis.
    #[must_use]
    pub(crate) const fn new(accrual: AccrualBasis) -> Self {
        Self { accrual }
    }
}

/// A request-tier [`MarketResolver`] that resolves a linear-FI market by
/// **bootstrapping** the calibrating quotes into a discount [`Curve`] — the FI
/// curve-bootstrap counterpart of the FX marked-surface pin (`FxSurfaceResolver`).
///
/// It captures the resolution once (the bootstrapped self-discounting curve plus the
/// fixed-leg conventions) and lends it as a [`ResolvedMarket`] on
/// [`MarketResolver::resolve`] — the standard lending pattern, so `celnet-core` need
/// never own the request's curve. The resolved market is single-curve
/// (`foreign`/`vol`/`spot` = `None`): a pure-rates discounting market carries no
/// second (asset) leg, spot, or volatility.
pub(crate) struct RatesCurveResolver {
    /// The resolved discount curve (bootstrapped from the calibrating quotes, or
    /// supplied directly for the cash-bond case).
    curve: Curve,
    /// The resolved fixed-leg conventions the produced market carries.
    conventions: RatesConventions,
}

impl RatesCurveResolver {
    /// Resolve a market by **bootstrapping** the calibrating OIS `quotes` into a
    /// self-discounting curve (the OIS curve-resolution seam). `accrual` is the
    /// fixed-leg basis the produced market advertises.
    ///
    /// # Errors
    ///
    /// Propagates a [`RatesPriceError::Bootstrap`] if the sequential bootstrap fails.
    pub(crate) fn from_quotes(
        quotes: &[OisQuote],
        accrual: AccrualBasis,
    ) -> Result<Self, RatesPriceError> {
        let curve = bootstrap_ois(quotes)?;
        Ok(Self {
            curve,
            conventions: RatesConventions::new(accrual),
        })
    }

    /// Resolve a market from an **already-bootstrapped** discount curve (the
    /// cash-bond case, where the curve is handed in rather than calibrated here).
    /// The [`Curve`] is `Arc`-backed, so the clone is a single reference-count bump.
    #[must_use]
    pub(crate) fn from_curve(curve: Curve, accrual: AccrualBasis) -> Self {
        Self {
            curve,
            conventions: RatesConventions::new(accrual),
        }
    }
}

impl MarketResolver for RatesCurveResolver {
    type Request = ();
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Error = RatesPriceError;

    fn resolve(
        &self,
        _request: &(),
    ) -> Result<ResolvedMarket<'_, RatesConventions>, RatesPriceError> {
        Ok(ResolvedMarket::new(
            &self.curve, // Curve: DiscountCurve — the unified discounting seam
            None,        // single-curve pure-rates market: no foreign/asset leg
            None,        // no volatility (linear product)
            None,        // no spot (pure-rates)
            &self.conventions,
        ))
    }
}

/// The per-request context an [`RatesOisEngine`] prices against: the calibrating
/// quotes (needed by [`celnet_rates::ois_risk`] to re-bootstrap under bumps for the
/// DV01 / key-rate ladder), the curve reference date (to rebuild the priced swap's
/// schedule), and the decoded OIS instrument.
pub(crate) struct RatesOisCtx<'a> {
    /// The calibrating OIS quotes the priced swap's curve is bootstrapped from.
    pub(crate) quotes: &'a [OisQuote],
    /// The curve reference (spot-anchor) date.
    pub(crate) reference: Date,
    /// The decoded OIS instrument being priced.
    pub(crate) ois: &'a celnet_proto::OisInstrument,
}

/// The linear-rates **OIS** leaf re-seated onto the unified contract. Its `price`
/// is byte-identical to [`super::price_ois`] (the established
/// [`RatesPricingResult`] path); its `risk` lifts the scalar `{pv, pv01, dv01}` into
/// the additive [`RiskMeasure::RateLadder`]. The full per-instrument key-rate ladder
/// rides in the [`Priced`](Priceable::Priced) (`RatesPricingResult::key_rate_ladder`)
/// at the request tier.
pub(crate) struct RatesOisEngine;

impl Priceable for RatesOisEngine {
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Ctx<'a> = RatesOisCtx<'a>;
    type Priced = RatesPricingResult;
    type Error = RatesPriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesOisCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        // The IDENTICAL established OIS pricing body — the whole of the byte-identity
        // guarantee. The engine reads its inputs from `ctx` (like the FX exotic
        // leaves read from `EngineCtx`); the ResolvedMarket rides as the produced
        // contract handle (its `discount` leg is the same bootstrapped curve
        // `price_ois` re-derives internally, deterministically identical).
        price_ois(ctx.ois, ctx.quotes, ctx.reference)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesOisCtx<'_>,
    ) -> Result<RiskMeasure, RatesPriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        // The side-signed linear ladder lifted to the unified additive measure. The
        // per-instrument key-rate ladder (`priced.key_rate_ladder`) is the request-
        // tier owned collection; the additive core measure carries the scalars.
        Ok(RiskMeasure::RateLadder(RateLadder::new(
            priced.pv,
            priced.pv01,
            priced.dv01,
        )))
    }
}

/// The per-request context a [`RatesIrsEngine`] prices against: calibrating quotes,
/// reference anchor date, and the decoded vanilla IRS instrument.
pub(crate) struct RatesIrsCtx<'a> {
    pub(crate) quotes: &'a [OisQuote],
    pub(crate) reference: Date,
    pub(crate) irs: &'a celnet_proto::VanillaIrsInstrument,
}

/// The linear-rates **vanilla IRS** leaf re-seated onto the unified contract. Its
/// `price` is byte-identical to [`super::price_irs`]; its `risk` lifts the scalar
/// `{pv, pv01, dv01}` into the additive [`RiskMeasure::RateLadder`].
pub(crate) struct RatesIrsEngine;

impl Priceable for RatesIrsEngine {
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Ctx<'a> = RatesIrsCtx<'a>;
    type Priced = RatesPricingResult;
    type Error = RatesPriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesIrsCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_irs(ctx.irs, ctx.quotes, ctx.reference)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesIrsCtx<'_>,
    ) -> Result<RiskMeasure, RatesPriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::RateLadder(RateLadder::new(
            priced.pv,
            priced.pv01,
            priced.dv01,
        )))
    }
}

/// The per-request context a [`RatesFraEngine`] prices against: calibrating quotes,
/// reference anchor date, and the decoded FRA instrument.
pub(crate) struct RatesFraCtx<'a> {
    pub(crate) quotes: &'a [OisQuote],
    pub(crate) reference: Date,
    pub(crate) fra: &'a celnet_proto::FraInstrument,
}

/// The linear-rates **FRA** leaf re-seated onto the unified contract. Its `price` is
/// byte-identical to [`super::price_fra`]; its `risk` lifts the scalar `{pv, pv01, dv01}`
/// into the additive [`RiskMeasure::RateLadder`].
pub(crate) struct RatesFraEngine;

impl Priceable for RatesFraEngine {
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Ctx<'a> = RatesFraCtx<'a>;
    type Priced = RatesPricingResult;
    type Error = RatesPriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesFraCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_fra(ctx.fra, ctx.quotes, ctx.reference)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesFraCtx<'_>,
    ) -> Result<RiskMeasure, RatesPriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::RateLadder(RateLadder::new(
            priced.pv,
            priced.pv01,
            priced.dv01,
        )))
    }
}

/// The per-request context a [`RatesBondInstrumentEngine`] prices against: calibrating quotes,
/// reference anchor date, and the wire bond instrument.
pub(crate) struct RatesBondInstrumentCtx<'a> {
    pub(crate) quotes: &'a [OisQuote],
    pub(crate) reference: Date,
    pub(crate) bond: &'a celnet_proto::BondInstrument,
}

/// The linear-rates **wire BondInstrument** leaf re-seated onto the unified contract.
/// Its `price` is byte-identical to [`super::price_bond_instrument`]; its `risk` lifts
/// `{pv, pv01, dv01}` into [`RiskMeasure::RateLadder`].
pub(crate) struct RatesBondInstrumentEngine;

impl Priceable for RatesBondInstrumentEngine {
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Ctx<'a> = RatesBondInstrumentCtx<'a>;
    type Priced = RatesPricingResult;
    type Error = RatesPriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesBondInstrumentCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_bond_instrument(ctx.bond, ctx.quotes, ctx.reference)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &RatesBondInstrumentCtx<'_>,
    ) -> Result<RiskMeasure, RatesPriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::RateLadder(RateLadder::new(
            priced.pv,
            priced.pv01,
            priced.dv01,
        )))
    }
}

/// The per-request context a [`BondEngine`] prices against: the bond and the
/// discount curve to price it off.
pub(crate) struct BondCtx<'a> {
    /// The fixed-coupon cash bond being priced.
    pub(crate) bond: &'a Bond,
    /// The discount curve the bond is priced off (`price_from_curve`).
    pub(crate) curve: &'a Curve,
}

/// The fully-priced result of a [`BondEngine`]: the dirty price off the curve plus
/// the full [`BondRisk`] set (DV01, Macaulay/modified duration, convexity, YTM,
/// clean price, accrued). The non-additive bond-native measures (durations,
/// convexity) ride here at the request tier, reachable through the contract via
/// [`Priceable::price`]; the additive core ladder carries only `{pv, pv01, dv01}`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BondPriced {
    /// The dirty (full) price discounted off the curve — [`price_from_curve`].
    pub dirty_price: f64,
    /// The full yield-risk set implied by that dirty price — [`bond_risk`].
    pub risk: BondRisk,
}

/// The linear-rates **cash-bond** leaf re-seated onto the unified contract. Its
/// `price` wraps [`celnet_bond::price_from_curve`] (dirty price off the curve) and
/// [`bond_risk`] (the yield-risk set off that price); its `risk` lifts the scalar
/// ladder into [`RiskMeasure::RateLadder`].
pub(crate) struct BondEngine;

impl Priceable for BondEngine {
    type Market<'a> = ResolvedMarket<'a, RatesConventions>;
    type Ctx<'a> = BondCtx<'a>;
    type Priced = BondPriced;
    type Error = BondError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &BondCtx<'_>,
    ) -> Result<BondPriced, BondError> {
        // Wrap the established cash-bond bodies verbatim: dirty price off the curve,
        // then the full yield-risk set implied by that price. The ResolvedMarket
        // rides as the produced handle (its `discount` leg is the same curve).
        let dirty_price = price_from_curve(ctx.bond, ctx.curve)?;
        let risk = bond_risk(ctx.bond, dirty_price)?;
        Ok(BondPriced { dirty_price, risk })
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, RatesConventions>,
        ctx: &BondCtx<'_>,
    ) -> Result<RiskMeasure, BondError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        // A fixed-coupon cash bond has a single rate sensitivity: lacking a separate
        // floating leg, its fixed-rate PV01 and discount DV01 coincide at the yield
        // DV01 (`BondRisk::dv01`, a positive magnitude in the `celnet-bond` sign
        // convention). The non-additive duration/convexity ride in `BondPriced`.
        Ok(RiskMeasure::RateLadder(RateLadder::new(
            priced.dirty_price,
            priced.risk.dv01,
            priced.risk.dv01,
        )))
    }
}

/// Price a [`RatesPriceRequest`] through the **central-core contract**: resolve the
/// curve ([`RatesCurveResolver`]) → [`ResolvedMarket`] → price the linear-FI leaf
/// ([`Priceable`]). This is the FI contract seam (ADR-0017 Phase B) — the linear-FI
/// counterpart of the options `Priceable` leaves — and the entry that keeps
/// [`RatesOisEngine`] (the Phase-C2 [`RiskMeasure::RateLadder`] risk seam) live and
/// proven. It is **byte-identical** to [`super::price_rates`] (gated by
/// [`tests::ois_via_contract_is_byte_identical`]): it calls the identical
/// [`build_quotes`] / [`resolve_date`] / [`price_ois`] bodies, so every measure — PV,
/// par, PV01, DV01, and the key-rate ladder — is unchanged.
///
/// The unified [`crate::pricer::PricingEngine::price_rates`] fronts the efficient
/// single-bootstrap [`super::price_rates`]; this contract entry (which additionally
/// bootstraps the resolved discount curve the OIS leaf carries as its market handle)
/// is retained as the FI risk-leaf seam, not the hot calc path.
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set, an unsupported
/// currency, malformed pillars, a missing/invalid instrument, or a numeric
/// schedule/bootstrap failure — the identical error surface as [`super::price_rates`].
pub fn price_rates_via_contract(
    req: &RatesPriceRequest,
) -> Result<RatesPricingResult, RatesPriceError> {
    let curve = req
        .curve_set
        .as_ref()
        .ok_or(RatesPriceError::MissingCurveSet)?;
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;

    // Resolve the market: bootstrap the calibrating quotes into a discount curve.
    // USD-SOFR par-OIS quotes use the ACT/360 fixed-leg accrual (`build_quotes`).
    let resolver = RatesCurveResolver::from_quotes(&quotes, AccrualBasis::Act360)?;
    let market = resolver.resolve(&())?;

    let instrument = req
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
        .ok_or(RatesPriceError::MissingInstrument)?;

    match instrument {
        rates_instrument::Instrument::Ois(ois) => {
            let ctx = RatesOisCtx {
                quotes: &quotes,
                reference,
                ois,
            };
            <RatesOisEngine as Priceable>::price(&RatesOisEngine, &market, &ctx)
        }
        rates_instrument::Instrument::Irs(irs) => {
            let ctx = RatesIrsCtx {
                quotes: &quotes,
                reference,
                irs,
            };
            <RatesIrsEngine as Priceable>::price(&RatesIrsEngine, &market, &ctx)
        }
        rates_instrument::Instrument::Fra(fra) => {
            let ctx = RatesFraCtx {
                quotes: &quotes,
                reference,
                fra,
            };
            <RatesFraEngine as Priceable>::price(&RatesFraEngine, &market, &ctx)
        }
        rates_instrument::Instrument::Bond(bond) => {
            let ctx = RatesBondInstrumentCtx {
                quotes: &quotes,
                reference,
                bond,
            };
            <RatesBondInstrumentEngine as Priceable>::price(
                &RatesBondInstrumentEngine,
                &market,
                &ctx,
            )
        }
    }
}

/// Price a fixed-coupon cash bond off a discount curve through the **central-core
/// contract**: resolve the curve ([`RatesCurveResolver::from_curve`]) →
/// [`ResolvedMarket`] → [`BondEngine`]. The FI counterpart entry for the cash-bond
/// leaf; byte-identical to calling [`celnet_bond::price_from_curve`] + [`bond_risk`]
/// directly (gated by [`tests::bond_via_contract_is_byte_identical`]).
///
/// # Errors
///
/// Propagates [`BondError`] from the curve pricing or yield solve.
pub fn price_bond_via_contract(bond: &Bond, curve: &Curve) -> Result<BondPriced, BondError> {
    let resolver = RatesCurveResolver::from_curve(curve.clone(), bond.day_count());
    let market = resolver
        .resolve(&())
        .expect("from_curve resolution is infallible");
    let ctx = BondCtx { bond, curve };
    <BondEngine as Priceable>::price(&BondEngine, &market, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{
        AccrualBasis as WireAccrualBasis, BondInstrument, BrokenDate, CurveSet,
        DayCount as WireDayCount, FraInstrument, OisInstrument, OisPillar,
        PaymentFrequency as WirePaymentFrequency, PillarTenor, RatesInstrument, Side,
        VanillaIrsInstrument, pillar_tenor,
    };
    use celnet_rates::{PaymentFrequency, ois_risk, usd_sofr_ois_schedule};
    use celnet_types::{Rate, Time};
    use time::Month;

    fn reference() -> BrokenDate {
        BrokenDate {
            year: 2026,
            month: 6,
            day: 25,
        }
    }

    fn years_pillar(years: u32) -> PillarTenor {
        PillarTenor {
            point: Some(pillar_tenor::Point::Years(years)),
        }
    }

    fn pillar(years: u32, par_rate: f64) -> OisPillar {
        OisPillar {
            tenor: Some(years_pillar(years)),
            par_rate,
        }
    }

    fn curve_set() -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(reference()),
            ois_pillars: vec![
                pillar(1, 0.0420),
                pillar(2, 0.0410),
                pillar(5, 0.0405),
                pillar(10, 0.0415),
            ],
        }
    }

    fn request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    fn assert_result_bit_identical(got: &RatesPricingResult, want: &RatesPricingResult) {
        assert_eq!(got.pv.to_bits(), want.pv.to_bits(), "pv drifted");
        assert_eq!(
            got.par_rate.to_bits(),
            want.par_rate.to_bits(),
            "par_rate drifted"
        );
        assert_eq!(got.pv01.to_bits(), want.pv01.to_bits(), "pv01 drifted");
        assert_eq!(got.dv01.to_bits(), want.dv01.to_bits(), "dv01 drifted");
        assert_eq!(
            got.key_rate_ladder.len(),
            want.key_rate_ladder.len(),
            "ladder length drifted"
        );
        for (i, (a, b)) in got
            .key_rate_ladder
            .iter()
            .zip(&want.key_rate_ladder)
            .enumerate()
        {
            assert_eq!(a.to_bits(), b.to_bits(), "key_rate[{i}] drifted");
        }
    }

    /// THE OIS GATE: pricing an OIS THROUGH the contract
    /// ([`price_rates_via_contract`]) is `to_bits`-identical to the established
    /// [`super::price_rates`] dispatch path — PV, par, PV01, DV01, and the full
    /// key-rate ladder — for both receive-fixed and pay-fixed sides, across tenors.
    #[test]
    fn ois_via_contract_is_byte_identical() {
        for &(tenor, fixed, notional) in &[
            (5u32, 0.04, 100_000_000.0),
            (7, 0.041, 50_000_000.0),
            (2, 0.0410, 25_000_000.0),
        ] {
            for side in [Side::Sell, Side::Buy] {
                let req = request(tenor, fixed, notional, side);
                let want = super::super::price_rates(&req).expect("dispatch path prices");
                let got = price_rates_via_contract(&req).expect("contract path prices");
                assert_result_bit_identical(&got, &want);
            }
        }
    }

    /// The OIS `Priceable::risk` re-seat reports the unified `RateLadder` tag whose
    /// scalars are `to_bits`-identical to the priced `{pv, pv01, dv01}`, and — for a
    /// receive-fixed (sign `+1`) swap — match the direct `celnet_rates::ois_risk`
    /// output (the independent engine output the contract wraps), including the full
    /// key-rate ladder carried in the request-tier `Priced`.
    #[test]
    fn ois_risk_tag_matches_engine_output() {
        let curve = curve_set();
        let ref_date = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve, ref_date).unwrap();

        // The direct engine output (the independent oracle): receive-fixed native.
        let schedule = usd_sofr_ois_schedule(ref_date, 7).unwrap();
        let engine = ois_risk(&quotes, &schedule, Rate(0.041), 100_000_000.0).unwrap();

        // Through the contract: the Priced (with the ladder) and the lifted measure.
        let resolver = RatesCurveResolver::from_quotes(&quotes, AccrualBasis::Act360).unwrap();
        let market = resolver.resolve(&()).unwrap();
        let ois = OisInstrument {
            tenor_years: 7,
            fixed_rate: 0.041,
            notional: 100_000_000.0,
            side: Side::Sell as i32,
        };
        let ctx = RatesOisCtx {
            quotes: &quotes,
            reference: ref_date,
            ois: &ois,
        };
        let priced = <RatesOisEngine as Priceable>::price(&RatesOisEngine, &market, &ctx).unwrap();
        let measure = <RatesOisEngine as Priceable>::risk(&RatesOisEngine, &market, &ctx).unwrap();

        // The lifted scalar measure equals the priced scalars, bit-for-bit.
        match measure {
            RiskMeasure::RateLadder(l) => {
                assert_eq!(l.pv.to_bits(), priced.pv.to_bits());
                assert_eq!(l.pv01.to_bits(), priced.pv01.to_bits());
                assert_eq!(l.dv01.to_bits(), priced.dv01.to_bits());
                // Receive-fixed (sign +1): the contract scalars equal the direct
                // engine output.
                assert_eq!(l.pv.to_bits(), engine.pv.to_bits(), "pv vs ois_risk");
                assert_eq!(l.pv01.to_bits(), engine.pv01.to_bits(), "pv01 vs ois_risk");
                assert_eq!(l.dv01.to_bits(), engine.dv01.to_bits(), "dv01 vs ois_risk");
            }
            RiskMeasure::OptionGreeks(_) => panic!("linear FI must report the RateLadder arm"),
        }
        // The full key-rate ladder (request-tier owned collection) matches the direct
        // engine ladder, bit-for-bit.
        assert_eq!(priced.key_rate_ladder.len(), engine.key_rate.len());
        for (i, (a, b)) in priced
            .key_rate_ladder
            .iter()
            .zip(&engine.key_rate)
            .enumerate()
        {
            assert_eq!(a.to_bits(), b.to_bits(), "key_rate[{i}] vs ois_risk");
        }
    }

    /// The resolver produces a genuinely-populated, single-curve rates market: its
    /// `discount` leg reproduces the bootstrapped curve's discount factors bit-for-
    /// bit, and `foreign`/`vol`/`spot` are `None` (a pure-rates market carries no
    /// asset leg, volatility, or spot).
    #[test]
    fn resolver_builds_a_real_single_curve_market() {
        let ref_date = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve_set(), ref_date).unwrap();
        let base = bootstrap_ois(&quotes).unwrap();

        let resolver = RatesCurveResolver::from_quotes(&quotes, AccrualBasis::Act360).unwrap();
        let market = resolver.resolve(&()).unwrap();

        assert!(
            market.foreign.is_none(),
            "pure-rates market has no asset leg"
        );
        assert!(market.vol.is_none(), "linear product carries no volatility");
        assert!(market.spot.is_none(), "pure-rates market carries no spot");
        assert_eq!(market.conventions.accrual, AccrualBasis::Act360);
        for &t in &[0.5_f64, 1.0, 3.0, 7.0, 10.0] {
            assert_eq!(
                market.discount.discount_factor(t).to_bits(),
                <Curve as celnet_types::DiscountCurve>::discount_factor(&base, t).to_bits(),
                "resolved discount leg must reproduce the bootstrapped curve at t={t}"
            );
        }
    }

    /// A 3y 6% semi-annual 30/360 bond settling on a coupon date (the clean case).
    fn bond_6pct_3y() -> Bond {
        Bond::new(
            Date::from_calendar_date(2032, Month::June, 15).unwrap(),
            Date::from_calendar_date(2035, Month::June, 15).unwrap(),
            0.06,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .unwrap()
    }

    /// A flat continuously-compounded discount curve at `z`.
    fn flat_curve(z: f64) -> Curve {
        Curve::from_zero_rates(&[
            (Time(1.0), Rate(z)),
            (Time(2.0), Rate(z)),
            (Time(3.0), Rate(z)),
        ])
        .unwrap()
    }

    /// THE BOND GATE: pricing a cash bond THROUGH the contract
    /// ([`price_bond_via_contract`]) is `to_bits`-identical to calling
    /// [`celnet_bond::price_from_curve`] + [`bond_risk`] directly — the dirty price
    /// and every `BondRisk` member.
    #[test]
    fn bond_via_contract_is_byte_identical() {
        let bond = bond_6pct_3y();
        for z in [0.03_f64, 0.05, 0.072] {
            let curve = flat_curve(z);
            let want_dirty = price_from_curve(&bond, &curve).unwrap();
            let want_risk = bond_risk(&bond, want_dirty).unwrap();

            let got = price_bond_via_contract(&bond, &curve).unwrap();
            assert_eq!(
                got.dirty_price.to_bits(),
                want_dirty.to_bits(),
                "bond dirty price drifted"
            );
            assert_eq!(
                got.risk.dv01.to_bits(),
                want_risk.dv01.to_bits(),
                "bond dv01 drifted"
            );
            assert_eq!(
                got.risk.modified_duration.to_bits(),
                want_risk.modified_duration.to_bits(),
                "bond modified duration drifted"
            );
            assert_eq!(
                got.risk.macaulay_duration.to_bits(),
                want_risk.macaulay_duration.to_bits(),
                "bond macaulay duration drifted"
            );
            assert_eq!(
                got.risk.convexity.to_bits(),
                want_risk.convexity.to_bits(),
                "bond convexity drifted"
            );
            assert_eq!(
                got.risk.yield_to_maturity.0.to_bits(),
                want_risk.yield_to_maturity.0.to_bits(),
                "bond ytm drifted"
            );
        }
    }

    /// The bond `Priceable::risk` re-seat reports the unified `RateLadder` tag whose
    /// scalars are `to_bits`-identical to the priced dirty price + DV01 (a fixed-
    /// coupon bond's single rate sensitivity: PV01 ≡ DV01).
    #[test]
    fn bond_risk_tag_matches_engine_output() {
        let bond = bond_6pct_3y();
        let curve = flat_curve(0.05);
        let priced = price_bond_via_contract(&bond, &curve).unwrap();

        let resolver = RatesCurveResolver::from_curve(curve.clone(), bond.day_count());
        let market = resolver.resolve(&()).unwrap();
        let ctx = BondCtx {
            bond: &bond,
            curve: &curve,
        };
        let measure = <BondEngine as Priceable>::risk(&BondEngine, &market, &ctx).unwrap();
        match measure {
            RiskMeasure::RateLadder(l) => {
                assert_eq!(l.pv.to_bits(), priced.dirty_price.to_bits());
                assert_eq!(l.dv01.to_bits(), priced.risk.dv01.to_bits());
                assert_eq!(l.pv01.to_bits(), priced.risk.dv01.to_bits());
            }
            RiskMeasure::OptionGreeks(_) => panic!("a cash bond must report the RateLadder arm"),
        }
    }

    /// A par swap priced through the contract has ~0 PV and the pay side is exactly
    /// opposite the receive side — the linear-FI identities, preserved end-to-end.
    #[test]
    fn contract_preserves_linear_identities() {
        let probe = price_rates_via_contract(&request(5, 0.04, 100_000_000.0, Side::Sell)).unwrap();
        let at_par =
            price_rates_via_contract(&request(5, probe.par_rate, 100_000_000.0, Side::Sell))
                .unwrap();
        assert!(at_par.pv.abs() < 1e-6, "par PV not ~0: {}", at_par.pv);

        let recv = price_rates_via_contract(&request(7, 0.041, 50_000_000.0, Side::Sell)).unwrap();
        let pay = price_rates_via_contract(&request(7, 0.041, 50_000_000.0, Side::Buy)).unwrap();
        assert_eq!(pay.pv.to_bits(), (-recv.pv).to_bits());
        assert_eq!(pay.dv01.to_bits(), (-recv.dv01).to_bits());
    }

    fn irs_request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Irs(VanillaIrsInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                    fixed_frequency: WirePaymentFrequency::SemiAnnual as i32,
                    fixed_day_count: WireDayCount::Act360 as i32,
                    float_frequency: WirePaymentFrequency::Quarterly as i32,
                    float_day_count: WireDayCount::Act360 as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    fn fra_request(
        start_months: u32,
        end_months: u32,
        fixed_rate: f64,
        notional: f64,
        side: Side,
    ) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Fra(FraInstrument {
                    start_months,
                    end_months,
                    fixed_rate,
                    notional,
                    side: side as i32,
                    accrual_basis: WireAccrualBasis::Act360 as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    fn bond_request(
        coupon_rate: f64,
        maturity: (i32, u32, u32),
        redemption: f64,
        side: Side,
    ) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Bond(BondInstrument {
                    coupon_rate,
                    coupon_frequency: WirePaymentFrequency::SemiAnnual as i32,
                    day_count: WireAccrualBasis::Thirty360BondBasis as i32,
                    maturity_date: Some(BrokenDate {
                        year: maturity.0,
                        month: maturity.1,
                        day: maturity.2,
                    }),
                    redemption,
                    side: side as i32,
                    ..Default::default()
                })),
            }),
            correlation_id: None,
        }
    }

    /// THE IRS GATE: pricing an IRS through the central contract
    /// ([`price_rates_via_contract`]) is `to_bits`-identical to the established
    /// [`super::price_rates`] dispatch path — PV, par, PV01, DV01, and key-rate ladder.
    #[test]
    fn irs_via_contract_is_byte_identical() {
        for &(tenor, fixed, notional) in &[
            (5u32, 0.04, 100_000_000.0),
            (10, 0.042, 50_000_000.0),
            (2, 0.038, 25_000_000.0),
        ] {
            for side in [Side::Sell, Side::Buy] {
                let req = irs_request(tenor, fixed, notional, side);
                let want = super::super::price_rates(&req).expect("dispatch path prices");
                let got = price_rates_via_contract(&req).expect("contract path prices");
                assert_result_bit_identical(&got, &want);
            }
        }
    }

    /// THE FRA GATE: pricing a FRA through the central contract
    /// ([`price_rates_via_contract`]) is `to_bits`-identical to the established
    /// [`super::price_rates`] dispatch path.
    #[test]
    fn fra_via_contract_is_byte_identical() {
        for &(start, end, fixed, notional) in &[
            (3u32, 6u32, 0.042, 100_000_000.0),
            (6, 12, 0.041, 50_000_000.0),
            (1, 4, 0.043, 25_000_000.0),
        ] {
            for side in [Side::Sell, Side::Buy] {
                let req = fra_request(start, end, fixed, notional, side);
                let want = super::super::price_rates(&req).expect("dispatch path prices");
                let got = price_rates_via_contract(&req).expect("contract path prices");
                assert_result_bit_identical(&got, &want);
            }
        }
    }

    /// THE BOND INSTRUMENT GATE: pricing a bond through the central contract
    /// ([`price_rates_via_contract`]) is `to_bits`-identical to [`super::price_rates`].
    #[test]
    fn bond_instrument_via_contract_is_byte_identical() {
        for &(coupon, maturity, redemption) in &[
            (0.045, (2031, 6, 25), 100.0),
            (0.06, (2036, 6, 25), 100.0),
        ] {
            for side in [Side::Sell, Side::Buy] {
                let req = bond_request(coupon, maturity, redemption, side);
                let want = super::super::price_rates(&req).expect("dispatch path prices");
                let got = price_rates_via_contract(&req).expect("contract path prices");
                assert_result_bit_identical(&got, &want);
            }
        }
    }
}

