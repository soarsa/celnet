//! Per-family **linear fixed-income** pricing engines: the [`FiProductEngine`]
//! registry behind [`super::price_rates`], with the pluggable house-model seam.
//!
//! This is the fixed-income analog of the option [`crate::pricer::engines`]
//! module (the `ProductEngine` registry + `dispatch_live` + `HOUSE_MODELS`
//! thread-local). It gives fixed income the *same* pluggable-model dispatch
//! options already have (ADR-0021, uniform-asset-class architecture): each FI
//! family (OIS / IRS / FRA / cash bond) is a zero-sized [`FiProductEngine`]
//! wrapping its verbatim native pricing body, and a registered
//! [`celnet_plugin_host::RatesHostModel`] can **override** a family's native
//! pricer per FI product kind.
//!
//! # Byte-identity (the default path is unchanged)
//!
//! With **no** FI house model installed, [`dispatch_rates_live`] threads a `None`
//! registry and every arm is the verbatim native path — the same
//! [`super::price_ois`] / [`super::price_irs`] / [`super::price_fra`] /
//! [`super::price_bond_instrument`] body [`super::price_rates`] calls, off the
//! same [`super::build_rates_market`] quotes. So a server with no house models
//! prices bit-for-bit as before this seam existed; the empty-registry byte
//! identity is proven by the tests below. The registry lookup is a single
//! non-allocating scan consulted per arm, only when a registry is present.

use core::cell::RefCell;

use celnet_plugin_api::{
    RatesAccrualBasis, RatesCurvePillar, RatesFrequency, RatesMeasures, RatesProductKind,
    RatesTerms,
};
use celnet_plugin_host::{ModelRegistry, RatesHostModel};
use celnet_proto::{
    AccrualBasis as WireAccrualBasis, BondInstrument, DayCount as WireDayCount, FraInstrument,
    OisInstrument, PaymentFrequency as WirePaymentFrequency, RatesPriceRequest, RatesPricingResult,
    Side, VanillaIrsInstrument, rates_instrument,
};
use celnet_rates::OisQuote;
use celnet_types::{BrokenDate, DayCount};
use time::Date;

use super::{
    RatesPriceError, build_rates_market, price_bond_instrument, price_fra, price_irs, price_ois,
    rates_instrument_of,
};

/// The shared, resolved per-request inputs every [`FiProductEngine`] reads: the
/// calibrating OIS quotes, the curve reference date, and — for the pluggable arm
/// — the calling worker's installed house-model registry. Borrowed for the
/// duration of the dispatch; no allocation on the native path.
pub(super) struct RatesEngineCtx<'a> {
    /// The calibrating OIS quotes (bootstrapped/priced against by the engines).
    pub quotes: &'a [OisQuote],
    /// The curve reference (spot-anchor) date.
    pub reference: Date,
    /// The calling pricing worker's installed FI house-model registry. When a
    /// rates model is registered for the arm's [`RatesProductKind`], that arm
    /// prices through it via the plugin-host registry — priced *as an*
    /// [`FiProductEngine`], the same seam as a native arm. `None` (the default)
    /// leaves every arm on the verbatim native static path (byte-identical).
    /// Per-worker (`!Sync` by design), never shared across threads.
    pub plugin_models: Option<&'a ModelRegistry>,
}

/// One fixed-income family's pricing engine. `Instrument` is the decoded arm the
/// dispatch hands the engine; [`price`](FiProductEngine::price) holds the family's
/// pricing body. Implemented by a zero-sized unit struct per native family (so
/// dispatch is a static call with no allocation) and by [`FiPluginModelEngine`]
/// for the registered-house-model override.
pub(super) trait FiProductEngine {
    /// The decoded instrument arm this engine prices.
    type Instrument;

    /// Price the decoded instrument against the shared rates engine context.
    ///
    /// # Errors
    ///
    /// [`RatesPriceError`] for a malformed instrument, an unknown enum tag, an
    /// out-of-domain input, or (on the plugin arm) a house-model failure.
    fn price(
        &self,
        instrument: &Self::Instrument,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError>;
}

/// The native OIS engine — the verbatim [`super::price_ois`] body.
pub(super) struct OisEngine;
impl FiProductEngine for OisEngine {
    type Instrument = OisInstrument;
    fn price(
        &self,
        ois: &OisInstrument,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_ois(ois, ctx.quotes, ctx.reference)
    }
}

/// The native vanilla-IRS engine — the verbatim [`super::price_irs`] body.
pub(super) struct IrsEngine;
impl FiProductEngine for IrsEngine {
    type Instrument = VanillaIrsInstrument;
    fn price(
        &self,
        irs: &VanillaIrsInstrument,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_irs(irs, ctx.quotes, ctx.reference)
    }
}

/// The native FRA engine — the verbatim [`super::price_fra`] body.
pub(super) struct FraEngine;
impl FiProductEngine for FraEngine {
    type Instrument = FraInstrument;
    fn price(
        &self,
        fra: &FraInstrument,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_fra(fra, ctx.quotes, ctx.reference)
    }
}

/// The native cash-bond engine — the verbatim [`super::price_bond_instrument`]
/// body.
pub(super) struct BondEngine;
impl FiProductEngine for BondEngine {
    type Instrument = BondInstrument;
    fn price(
        &self,
        bond: &BondInstrument,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        price_bond_instrument(bond, ctx.quotes, ctx.reference)
    }
}

/// Prices a fixed-income arm through a **registered house rates model** resolved
/// from the plugin-host [`ModelRegistry`], rather than the built-in native body —
/// the FI counterpart of the option `PluginModelEngine`. A registered model is
/// priced *as an* [`FiProductEngine`] whose `Instrument` is the already-built
/// neutral [`RatesTerms`], so it flows through the identical dispatch seam as a
/// native arm: the calibrating curve is handed to the model as neutral
/// [`RatesCurvePillar`]s and the returned [`RatesMeasures`] are mapped back onto
/// the wire [`RatesPricingResult`]. A desk's own model legitimately differs from
/// the native path.
pub(super) struct FiPluginModelEngine<'r> {
    /// The resolved house rates-model handle (tier-blind).
    model: &'r dyn RatesHostModel,
}

impl FiProductEngine for FiPluginModelEngine<'_> {
    type Instrument = RatesTerms;
    fn price(
        &self,
        terms: &RatesTerms,
        ctx: &RatesEngineCtx<'_>,
    ) -> Result<RatesPricingResult, RatesPriceError> {
        let pillars = curve_pillars(ctx.quotes);
        // A model-domain failure is surfaced as a typed error at the boundary
        // (mapped to INVALID_ARGUMENT) — never a silent fallback to the native
        // form, exactly as the option plugin arm handles it.
        let measures = self.model.price(terms, &pillars).map_err(|_| {
            RatesPriceError::Model("registered rates model could not price this instrument")
        })?;
        Ok(measures_to_result(measures))
    }
}

/// Project the calibrating OIS quotes onto the neutral SDK curve: each pillar's
/// ACT/365F year-fraction (its schedule's final pay time) paired with its par
/// rate. The single source of the curve handoff to a registered model.
fn curve_pillars(quotes: &[OisQuote]) -> Vec<RatesCurvePillar> {
    quotes
        .iter()
        .map(|q| {
            let t = q.schedule.periods().last().map_or(0.0, |p| p.pay.0);
            RatesCurvePillar::new(t, q.par_rate.0)
        })
        .collect()
}

/// Map the neutral SDK [`RatesMeasures`] onto the wire [`RatesPricingResult`] —
/// a 1:1 field move (the measures ARE the wire result's fields).
fn measures_to_result(m: RatesMeasures) -> RatesPricingResult {
    RatesPricingResult {
        pv: m.pv,
        par_rate: m.par_rate,
        pv01: m.pv01,
        dv01: m.dv01,
        key_rate_ladder: m.key_rate_ladder,
        // Two-way + size stay proto3-zero on the priced core; the streaming edge
        // enriches them via `stream_spread`.
        ..Default::default()
    }
}

/// The receive-fixed flag for a swap/FRA `side`: `SIDE_SELL` receives fixed (the
/// engine's native convention), `SIDE_BUY` pays fixed. `SIDE_TWO_WAY` is refused
/// for an outright — the same guard [`super::side_sign`] applies.
fn receive_fixed_of(side: i32) -> Result<bool, RatesPriceError> {
    match Side::try_from(side).map_err(|_| RatesPriceError::InvalidSide)? {
        Side::Sell => Ok(true),
        Side::Buy => Ok(false),
        Side::TwoWay => Err(RatesPriceError::InvalidSide),
    }
}

/// The long/short flag for a bond `side`: `SIDE_BUY` is long, `SIDE_SELL` short —
/// the same guard [`super::bond_side_sign`] applies.
fn long_of(side: i32) -> Result<bool, RatesPriceError> {
    match Side::try_from(side).map_err(|_| RatesPriceError::InvalidSide)? {
        Side::Buy => Ok(true),
        Side::Sell => Ok(false),
        Side::TwoWay => Err(RatesPriceError::InvalidSide),
    }
}

/// Map a wire payment-frequency enum number to the neutral SDK [`RatesFrequency`].
fn neutral_frequency(w: i32) -> Result<RatesFrequency, RatesPriceError> {
    match WirePaymentFrequency::try_from(w).map_err(|_| RatesPriceError::InvalidFrequency)? {
        WirePaymentFrequency::Annual => Ok(RatesFrequency::Annual),
        WirePaymentFrequency::SemiAnnual => Ok(RatesFrequency::SemiAnnual),
        WirePaymentFrequency::Quarterly => Ok(RatesFrequency::Quarterly),
    }
}

/// Map a wire day-count enum number to the neutral [`DayCount`].
fn neutral_day_count(w: i32) -> Result<DayCount, RatesPriceError> {
    match WireDayCount::try_from(w).map_err(|_| RatesPriceError::InvalidDayCount)? {
        WireDayCount::Act365Fixed => Ok(DayCount::Act365Fixed),
        WireDayCount::Act360 => Ok(DayCount::Act360),
    }
}

/// Map a wire accrual-basis enum number to the neutral SDK [`RatesAccrualBasis`].
fn neutral_accrual(w: i32) -> Result<RatesAccrualBasis, RatesPriceError> {
    match WireAccrualBasis::try_from(w).map_err(|_| RatesPriceError::InvalidAccrualBasis)? {
        WireAccrualBasis::Act360 => Ok(RatesAccrualBasis::Act360),
        WireAccrualBasis::Act365Fixed => Ok(RatesAccrualBasis::Act365Fixed),
        WireAccrualBasis::Thirty360BondBasis => Ok(RatesAccrualBasis::Thirty360BondBasis),
    }
}

/// Build the neutral OIS [`RatesTerms`] from the wire instrument.
fn ois_terms(ois: &OisInstrument) -> Result<RatesTerms, RatesPriceError> {
    Ok(RatesTerms::Ois {
        tenor_years: ois.tenor_years,
        fixed_rate: ois.fixed_rate,
        notional: ois.notional,
        receive_fixed: receive_fixed_of(ois.side)?,
    })
}

/// Build the neutral IRS [`RatesTerms`] from the wire instrument.
fn irs_terms(irs: &VanillaIrsInstrument) -> Result<RatesTerms, RatesPriceError> {
    Ok(RatesTerms::Irs {
        tenor_years: irs.tenor_years,
        fixed_rate: irs.fixed_rate,
        notional: irs.notional,
        receive_fixed: receive_fixed_of(irs.side)?,
        fixed_frequency: neutral_frequency(irs.fixed_frequency)?,
        float_frequency: neutral_frequency(irs.float_frequency)?,
        fixed_day_count: neutral_day_count(irs.fixed_day_count)?,
        float_day_count: neutral_day_count(irs.float_day_count)?,
    })
}

/// Build the neutral FRA [`RatesTerms`] from the wire instrument.
fn fra_terms(fra: &FraInstrument) -> Result<RatesTerms, RatesPriceError> {
    Ok(RatesTerms::Fra {
        start_months: fra.start_months,
        end_months: fra.end_months,
        fixed_rate: fra.fixed_rate,
        notional: fra.notional,
        receive_fixed: receive_fixed_of(fra.side)?,
        accrual: neutral_accrual(fra.accrual_basis)?,
    })
}

/// Build the neutral cash-bond [`RatesTerms`] from the wire instrument.
fn bond_terms(bond: &BondInstrument) -> Result<RatesTerms, RatesPriceError> {
    let maturity_wire = bond
        .maturity_date
        .as_ref()
        .ok_or(RatesPriceError::MissingBondMaturity)?;
    let maturity = BrokenDate::new(
        maturity_wire.year,
        u8::try_from(maturity_wire.month).map_err(|_| RatesPriceError::InvalidBondMaturity)?,
        u8::try_from(maturity_wire.day).map_err(|_| RatesPriceError::InvalidBondMaturity)?,
    );
    Ok(RatesTerms::Bond {
        coupon_rate: bond.coupon_rate,
        redemption: bond.redemption,
        coupon_frequency: neutral_frequency(bond.coupon_frequency)?,
        day_count: neutral_accrual(bond.day_count)?,
        maturity,
        long: long_of(bond.side)?,
    })
}

/// Resolve the registered house rates model for `kind`, if a registry is present.
fn rates_model_for<'a>(
    ctx: &RatesEngineCtx<'a>,
    kind: RatesProductKind,
) -> Option<&'a dyn RatesHostModel> {
    ctx.plugin_models.and_then(|r| r.active_rates_model(kind))
}

/// Dispatch a [`RatesPriceRequest`] to its native [`FiProductEngine`], or — when
/// a house model is registered for the instrument's [`RatesProductKind`] — to
/// that model via [`FiPluginModelEngine`].
///
/// The FI analog of the option `dispatch` static match. Absent a registry (or a
/// registered model for the arm's kind) every arm is the verbatim native body, so
/// `dispatch_rates(req, None)` is `to_bits`-identical to [`super::price_rates`].
///
/// # Errors
///
/// [`RatesPriceError`] as [`super::price_rates`], plus [`RatesPriceError::Model`]
/// if a registered model fails.
pub(super) fn dispatch_rates(
    req: &RatesPriceRequest,
    plugin_models: Option<&ModelRegistry>,
) -> Result<RatesPricingResult, RatesPriceError> {
    let (quotes, reference) = build_rates_market(req)?;
    let ctx = RatesEngineCtx {
        quotes: &quotes,
        reference,
        plugin_models,
    };
    match rates_instrument_of(req)? {
        rates_instrument::Instrument::Ois(ois) => {
            match rates_model_for(&ctx, RatesProductKind::Ois) {
                Some(model) => FiPluginModelEngine { model }.price(&ois_terms(ois)?, &ctx),
                None => OisEngine.price(ois, &ctx),
            }
        }
        rates_instrument::Instrument::Irs(irs) => {
            match rates_model_for(&ctx, RatesProductKind::Irs) {
                Some(model) => FiPluginModelEngine { model }.price(&irs_terms(irs)?, &ctx),
                None => IrsEngine.price(irs, &ctx),
            }
        }
        rates_instrument::Instrument::Fra(fra) => {
            match rates_model_for(&ctx, RatesProductKind::Fra) {
                Some(model) => FiPluginModelEngine { model }.price(&fra_terms(fra)?, &ctx),
                None => FraEngine.price(fra, &ctx),
            }
        }
        rates_instrument::Instrument::Bond(bond) => {
            match rates_model_for(&ctx, RatesProductKind::Bond) {
                Some(model) => FiPluginModelEngine { model }.price(&bond_terms(bond)?, &ctx),
                None => BondEngine.price(bond, &ctx),
            }
        }
    }
}

thread_local! {
    /// The calling pricing worker's installed FI house-model registry. Per-worker
    /// by design (`!Sync`), so it lives in thread-local state — each worker
    /// installs its own set once and reads it lock-free on the hot path. Empty by
    /// default (no house models ⇒ the verbatim native dispatch). The FI analog of
    /// the option `HOUSE_MODELS`.
    static FI_HOUSE_MODELS: RefCell<Option<ModelRegistry>> = const { RefCell::new(None) };
}

/// Install the calling pricing worker's FI house-model [`ModelRegistry`],
/// activating its registered rates models on this worker's dispatch. Per-worker
/// by design (the registry is `!Sync`): call once per pricing worker at start-up.
/// Replaces any registry previously installed on this thread. The FI analog of
/// [`crate::pricer::install_house_models`].
pub fn install_fi_house_models(registry: ModelRegistry) {
    FI_HOUSE_MODELS.with(|cell| *cell.borrow_mut() = Some(registry));
}

/// The live FI dispatch entry: borrow the calling worker's installed house-model
/// registry (if any) and dispatch with it threaded through. Absent an installed
/// registry the borrow is `None` and every arm is the verbatim native dispatch
/// (byte-identical) — the FI analog of the option `dispatch_live`, and the
/// terminal [`crate::pricer::engine::PricingEngine::price_rates`] routes through.
pub(crate) fn dispatch_rates_live(
    req: &RatesPriceRequest,
) -> Result<RatesPricingResult, RatesPriceError> {
    FI_HOUSE_MODELS.with(|cell| {
        let guard = cell.borrow();
        dispatch_rates(req, guard.as_ref())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_plugin_api::example::ConstantRatesModel;
    use celnet_proto::{
        BrokenDate as WireBrokenDate, CurveSet, OisPillar, PillarTenor, RatesInstrument,
        pillar_tenor,
    };

    fn curve_set() -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(WireBrokenDate {
                year: 2026,
                month: 6,
                day: 25,
            }),
            ois_pillars: [(1u32, 0.0420), (2, 0.0410), (5, 0.0405), (10, 0.0415)]
                .into_iter()
                .map(|(y, r)| OisPillar {
                    tenor: Some(PillarTenor {
                        point: Some(pillar_tenor::Point::Years(y)),
                    }),
                    par_rate: r,
                })
                .collect(),
        }
    }

    fn request(instrument: rates_instrument::Instrument) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(instrument),
            }),
            correlation_id: None,
        }
    }

    fn ois_req() -> RatesPriceRequest {
        request(rates_instrument::Instrument::Ois(OisInstrument {
            tenor_years: 5,
            fixed_rate: 0.04,
            notional: 100_000_000.0,
            side: Side::Sell as i32,
        }))
    }

    fn irs_req() -> RatesPriceRequest {
        request(rates_instrument::Instrument::Irs(VanillaIrsInstrument {
            tenor_years: 5,
            fixed_rate: 0.041,
            notional: 50_000_000.0,
            side: Side::Buy as i32,
            fixed_frequency: WirePaymentFrequency::SemiAnnual as i32,
            fixed_day_count: WireDayCount::Act360 as i32,
            float_frequency: WirePaymentFrequency::Quarterly as i32,
            float_day_count: WireDayCount::Act360 as i32,
        }))
    }

    fn fra_req() -> RatesPriceRequest {
        request(rates_instrument::Instrument::Fra(FraInstrument {
            start_months: 3,
            end_months: 9,
            fixed_rate: 0.043,
            notional: 25_000_000.0,
            side: Side::Sell as i32,
            accrual_basis: WireAccrualBasis::Act360 as i32,
        }))
    }

    fn bond_req() -> RatesPriceRequest {
        request(rates_instrument::Instrument::Bond(BondInstrument {
            coupon_rate: 0.06,
            coupon_frequency: WirePaymentFrequency::SemiAnnual as i32,
            day_count: WireAccrualBasis::Thirty360BondBasis as i32,
            maturity_date: Some(WireBrokenDate {
                year: 2031,
                month: 6,
                day: 25,
            }),
            redemption: 100.0,
            side: Side::Buy as i32,
            ..Default::default()
        }))
    }

    /// Assert two priced FI results are equal to the bit across every measure.
    fn assert_bit_identical(got: &RatesPricingResult, want: &RatesPricingResult, label: &str) {
        assert_eq!(got.pv.to_bits(), want.pv.to_bits(), "[{label}] pv drifted");
        assert_eq!(
            got.par_rate.to_bits(),
            want.par_rate.to_bits(),
            "[{label}] par_rate drifted"
        );
        assert_eq!(
            got.pv01.to_bits(),
            want.pv01.to_bits(),
            "[{label}] pv01 drifted"
        );
        assert_eq!(
            got.dv01.to_bits(),
            want.dv01.to_bits(),
            "[{label}] dv01 drifted"
        );
        assert_eq!(
            got.key_rate_ladder.len(),
            want.key_rate_ladder.len(),
            "[{label}] ladder length drifted"
        );
        for (i, (a, b)) in got
            .key_rate_ladder
            .iter()
            .zip(&want.key_rate_ladder)
            .enumerate()
        {
            assert_eq!(a.to_bits(), b.to_bits(), "[{label}] key_rate[{i}] drifted");
        }
    }

    /// VALIDATION (a): with NO house model registered, the pluggable dispatch is
    /// `to_bits`-identical to the direct native [`super::super::price_rates`]
    /// across OIS / IRS / FRA / bond — zero behaviour change on the default path.
    #[test]
    fn empty_registry_dispatch_is_native_bit_identical() {
        for (label, req) in [
            ("ois", ois_req()),
            ("irs", irs_req()),
            ("fra", fra_req()),
            ("bond", bond_req()),
        ] {
            let native = super::super::price_rates(&req).expect("native prices");
            let dispatched = dispatch_rates(&req, None).expect("dispatch prices");
            assert_bit_identical(&dispatched, &native, label);
        }
    }

    /// An empty-but-installed registry still routes every arm to native (the
    /// `active_rates_model` scan returns `None`), a second byte-identity guarantee.
    #[test]
    fn installed_empty_registry_stays_native() {
        let reg = ModelRegistry::new();
        for (label, req) in [("ois", ois_req()), ("bond", bond_req())] {
            let native = super::super::price_rates(&req).expect("native prices");
            let via = dispatch_rates(&req, Some(&reg)).expect("dispatch prices");
            assert_bit_identical(&via, &native, label);
        }
    }

    fn sentinel() -> RatesMeasures {
        RatesMeasures {
            pv: 424_242.0,
            par_rate: 0.099,
            pv01: -1.5,
            dv01: -2.5,
            key_rate_ladder: vec![7.0, 8.0],
        }
    }

    /// VALIDATION (b): a house model registered for ONE kind (OIS) serves that
    /// arm through the dispatch (its sentinel measures, proving the
    /// dispatch→registry edge), while every OTHER kind stays on the native path.
    #[test]
    fn registered_model_overrides_its_kind_only() {
        let mut reg = ModelRegistry::new();
        reg.register_native_rates(RatesProductKind::Ois, ConstantRatesModel::new(sentinel()))
            .expect("register");

        // OIS routes through the model: the sentinel measures, NOT the native PV.
        let ois = dispatch_rates(&ois_req(), Some(&reg)).expect("ois prices");
        let want = sentinel();
        assert_eq!(ois.pv.to_bits(), want.pv.to_bits());
        assert_eq!(ois.par_rate.to_bits(), want.par_rate.to_bits());
        assert_eq!(ois.pv01.to_bits(), want.pv01.to_bits());
        assert_eq!(ois.dv01.to_bits(), want.dv01.to_bits());
        assert_eq!(ois.key_rate_ladder, want.key_rate_ladder);
        // And it genuinely differs from the native OIS PV.
        let native_ois = super::super::price_rates(&ois_req()).expect("native ois");
        assert_ne!(ois.pv.to_bits(), native_ois.pv.to_bits());

        // Every other kind stays native (no model registered for it).
        for (label, req) in [("irs", irs_req()), ("fra", fra_req()), ("bond", bond_req())] {
            let native = super::super::price_rates(&req).expect("native prices");
            let via = dispatch_rates(&req, Some(&reg)).expect("dispatch prices");
            assert_bit_identical(&via, &native, label);
        }
    }

    /// The live [`dispatch_rates_live`] reads the worker's installed registry:
    /// nothing installed ⇒ native bit-for-bit; a model installed ⇒ its override.
    /// A drop guard resets the worker's registry even on panic so the install
    /// never leaks to sibling tests sharing this thread under `cargo test`.
    #[test]
    fn install_routes_live_dispatch() {
        struct ResetGuard;
        impl Drop for ResetGuard {
            fn drop(&mut self) {
                install_fi_house_models(ModelRegistry::new());
            }
        }
        let _reset = ResetGuard;

        // Nothing installed on this worker: the native path.
        let native = super::super::price_rates(&ois_req()).expect("native ois");
        let live_native = dispatch_rates_live(&ois_req()).expect("live ois");
        assert_bit_identical(&live_native, &native, "live-native-ois");

        // Install an OIS house model; the live path now routes through it.
        let mut reg = ModelRegistry::new();
        reg.register_native_rates(RatesProductKind::Ois, ConstantRatesModel::new(sentinel()))
            .expect("register");
        install_fi_house_models(reg);
        let live = dispatch_rates_live(&ois_req()).expect("live override ois");
        assert_eq!(live.pv.to_bits(), sentinel().pv.to_bits());

        // Reset to an empty registry: back to the native path, bit-for-bit.
        install_fi_house_models(ModelRegistry::new());
        let after = dispatch_rates_live(&ois_req()).expect("live reset ois");
        assert_bit_identical(&after, &native, "live-reset-ois");
    }
}
