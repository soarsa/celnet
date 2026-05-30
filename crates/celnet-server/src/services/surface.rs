//! The surface service: `GetSmile`, `MarkSurface`, and `Scenario`.
//!
//! * **`GetSmile`** reads the calibrated smile for one `(pair, tenor)` on the
//!   delta axis — but the smile must first exist. The edge marks an on-the-fly
//!   smile from the maker's live ATM/RR/BF when no broker set is supplied, so a
//!   `GetSmile` always returns a calibrated, arbitrage-checked
//!   [`celnet_proto::Smile`] rather than a stub.
//! * **`MarkSurface`** calibrates a smile per supplied broker quote set
//!   (ATM + 25Δ/10Δ risk-reversal/butterfly), returns the per-tenor calibrated
//!   smiles with their [`celnet_proto::ArbReport`], and stamps a monotonic
//!   `surface_version`.
//! * **`Scenario`** reprices a [`celnet_proto::Instrument`] across the Cartesian
//!   product of one or more [`celnet_proto::ShockAxis`]es applied to a base market
//!   context — the desk what-if grid — returning a repriced [`celnet_proto::Greeks`]
//!   set per node. The repricing routes through the same [`crate::pricer`] the RFQ
//!   and RFS paths use, so a scenario node is consistent with a live quote.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_proto::surface_service_server::SurfaceService;
use celnet_proto::{
    ArbReport, BrokerQuoteSet, BucketedRisk, Conventions, CrossGamma, GetSmileRequest,
    MarkSurfaceRequest, MarkSurfaceResponse, MarketContext as WireMarketContext, ScenarioPoint,
    ScenarioRequest, ScenarioResponse, Smile, SmilePoint, VegaBucket, shock_axis,
};
use tonic::{Request, Response, Status};

use celnet_conventions::ConventionRecord;
use celnet_surface::{
    ArbitrageReport, MarketContext as SurfaceContext, MarketHedgeSmile, MarketQuotes, build_smile,
    check_slice,
};
use celnet_types::{
    AtmConvention, Cut, DayCount, DeltaConvention, OptionType, PremiumStyle, Settlement,
};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::readiness::ReadinessGate;
use crate::surface_book::SurfaceBook;

/// The delta pillars (signed convention deltas) the smile is reported on: the
/// 10Δ and 25Δ wings plus the 50Δ (ATM) — the market-standard read axis.
const REPORT_DELTAS: [f64; 5] = [-0.10, -0.25, 0.50, 0.25, 0.10];

/// The absolute vol bump (1 vol point) used to finite-difference a bucketed-vega
/// pillar: the book's value sensitivity to a 1.0-absolute-vol move of one smile
/// pillar, holding the others still.
const VEGA_BUCKET_BUMP: f64 = 1e-4;

/// The relative spot bump and absolute vol bump used to finite-difference a
/// cross-gamma term d²V/(dx_a dx_b) by a 2-D central stencil.
const CROSS_SPOT_REL: f64 = 1e-4;
const CROSS_VOL_ABS: f64 = 1e-4;
const CROSS_RATE_ABS: f64 = 1e-4;

/// The surface service over the [`CoreLink`] and readiness gate.
#[derive(Debug)]
pub struct SurfaceEdge {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    clock: Clock,
    /// The shared versioned marked-surface registry. `MarkSurface` deposits its
    /// calibrated per-tenor smiles here under a fresh version (the book is the
    /// version authority) so the pricing / RFQ / RFS paths can pin a
    /// `surface_version` against the exact marked surface.
    surface_book: Arc<SurfaceBook>,
}

impl SurfaceEdge {
    /// Construct the surface service.
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
    ) -> Self {
        Self {
            link,
            gate,
            clock,
            surface_book,
        }
    }

    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ))
        }
    }
}

/// A resolved convention record built from the wire conventions, supplying the
/// delta/ATM/premium/settlement choices the surface calibration consults.
fn convention_record(conv: &ConventionSet) -> ConventionRecord {
    // The accrual day-counts default to the vol day-count (the surface layer only
    // consults the delta/ATM/premium choices for the smile build).
    ConventionRecord::new(
        conv.delta,
        conv.atm,
        conv.premium,
        conv.cut,
        conv.day_count,
        conv.day_count,
        conv.day_count,
        conv.settlement,
    )
}

/// Calibrate a Vanna-Volga smile for one tenor from a broker quote set against a
/// market context.
fn calibrate(
    broker: &BrokerQuoteSet,
    market: &WireMarketContext,
    conv: &ConventionSet,
) -> Result<MarketHedgeSmile, Status> {
    let record = convention_record(conv);
    let ctx = SurfaceContext::new(
        market.spot,
        market.r_dom,
        market.r_for,
        broker.tenor_years,
        record,
    );
    let quotes = quotes_from_broker(broker);
    build_smile(&ctx, &quotes)
        .map_err(|e| Status::invalid_argument(format!("calibration failed: {e:?}")))
}

/// Build the calibration quote set from a broker set, choosing five-point vs
/// three-point off the explicit `has_ten_delta` presence flag — never a float
/// compare on the wing values. So a genuine *zero-but-present* 10Δ wing
/// (`rr_10 == 0.0 && bf_10 == 0.0` with `has_ten_delta == true`) still drives a
/// five-point calibration, where a float test would have mis-classified it as
/// three-point.
fn quotes_from_broker(broker: &BrokerQuoteSet) -> MarketQuotes {
    if broker.has_ten_delta {
        MarketQuotes::five_point(
            broker.atm_vol,
            broker.rr_25,
            broker.bf_25,
            broker.rr_10,
            broker.bf_10,
        )
    } else {
        MarketQuotes::three_point(broker.atm_vol, broker.rr_25, broker.bf_25)
    }
}

/// Project a calibrated smile and its broker set onto the wire [`Smile`] message,
/// reporting the delta-axis vol points and the arbitrage status.
fn smile_to_wire(
    pair: Option<celnet_proto::CcyPair>,
    broker: &BrokerQuoteSet,
    smile: &MarketHedgeSmile,
    conv: &ConventionSet,
    market: &WireMarketContext,
    clock: &Clock,
) -> Smile {
    let forward = smile.forward();
    let t = broker.tenor_years;
    let record = convention_record(conv);
    let ctx = SurfaceContext::new(market.spot, market.r_dom, market.r_for, t, record);

    // Map each report delta to its strike (via the convention inversion) and read
    // the calibrated vol there.
    let mut points = Vec::with_capacity(REPORT_DELTAS.len());
    for &d in &REPORT_DELTAS {
        let (option, target) = if d >= 0.0 {
            (OptionType::Call, d)
        } else {
            (OptionType::Put, d)
        };
        let template = celnet_types::VanillaInputs::new(
            market.spot,
            forward,
            broker.atm_vol,
            t,
            market.r_dom,
            market.r_for,
        );
        // The 0.50 entry in `REPORT_DELTAS` is the ATM pillar (priced at the
        // forward, no delta inversion). Detect it via `celnet_core::is_close` with
        // explicit tolerances (the determinism guardrail forbids ad-hoc float
        // equality); the value is a fixed literal, so a tight tolerance is exact.
        let strike = if celnet_core::is_close(d, 0.50, 0.0, 1e-12) {
            forward
        } else {
            celnet_vanilla::strike_from_delta(ctx.delta_convention(), option, target, &template)
                .unwrap_or(forward)
        };
        let vol = celnet_core::Smile::implied_vol(smile, strike, forward, t).0;
        points.push(SmilePoint {
            delta: d,
            tenor_years: t,
            vol,
        });
    }

    // Arbitrage check across the calibrated benchmark strikes (widened grid).
    let arb = arb_report(smile, forward, t);

    Smile {
        pair,
        tenor_years: t,
        broker_quotes: Some(*broker),
        points,
        conventions: Some(conv_to_wire(conv)),
        arbitrage: Some(arb),
        epoch_nanos: clock.now_nanos(),
    }
}

/// Run the static no-arbitrage checks on a calibrated smile and project them onto
/// the wire [`ArbReport`].
fn arb_report(smile: &MarketHedgeSmile, forward: f64, t: f64) -> ArbReport {
    // A symmetric strike grid around the forward for the density / vertical checks.
    let grid: Vec<f64> = (1..=9)
        .map(|i| forward * (0.80 + 0.05 * f64::from(i - 1)))
        .collect();
    let h = forward * 0.01;
    let report: ArbitrageReport = check_slice(smile, &grid, forward, t, h);
    let bf_free = report.min_density >= -1e-6 && report.min_butterfly >= -1e-6;
    let cal_free = report.max_vertical_increase <= 1e-6;
    ArbReport {
        butterfly_arbitrage_free: bf_free,
        calendar_arbitrage_free: cal_free,
        worst_density: report.min_density.min(0.0),
        note: if report.is_arbitrage_free(1e-6) {
            "no repair applied".to_owned()
        } else {
            "static checks flagged; smile reported as calibrated".to_owned()
        },
    }
}

/// Re-encode a decoded convention set to the wire form.
fn conv_to_wire(c: &ConventionSet) -> Conventions {
    Conventions {
        delta_convention: celnet_proto::DeltaConvention::from(c.delta) as i32,
        atm_convention: celnet_proto::AtmConvention::from(c.atm) as i32,
        premium_style: celnet_proto::PremiumStyle::from(c.premium) as i32,
        cut: celnet_proto::Cut::from(c.cut) as i32,
        day_count: celnet_proto::DayCount::from(c.day_count) as i32,
        settlement: celnet_proto::Settlement::from(c.settlement) as i32,
    }
}

/// Decode the wire conventions, mapping a decode error to `invalid_argument`.
fn decode_conv(w: &Conventions) -> Result<ConventionSet, Status> {
    ConventionSet::decode(w).map_err(|e| Status::invalid_argument(e.to_string()))
}

/// Apply a single market-factor shock step to a market context along one axis.
///
/// The [`shock_axis::Factor::Time`] (theta-roll) axis does **not** shock the
/// market — it rolls the calendar forward (reduces time-to-expiry) — so it is a
/// no-op here and handled by the scenario loop's expiry roll instead.
fn apply_shock(
    base: &WireMarketContext,
    factor: shock_axis::Factor,
    relative: bool,
    step: f64,
) -> WireMarketContext {
    let mut m = *base;
    let adjust = |x: f64| if relative { x * (1.0 + step) } else { x + step };
    match factor {
        shock_axis::Factor::Spot => m.spot = adjust(m.spot),
        shock_axis::Factor::Vol => m.vol = adjust(m.vol),
        shock_axis::Factor::RateDom => m.r_dom = adjust(m.r_dom),
        shock_axis::Factor::RateFor => m.r_for = adjust(m.r_for),
        // The theta-roll axis rolls expiry, not the market context.
        shock_axis::Factor::Time => {}
    }
    m
}

/// Price an instrument at a (possibly time-rolled) expiry against a shocked market,
/// returning just the present value (`Greeks.price`). Used by the book-shaped risk
/// finite differences, which bump one factor and read the repriced value.
fn price_value_at(
    instrument: &celnet_proto::Instrument,
    market: &WireMarketContext,
    expiry_years: f64,
    conv: &ConventionSet,
) -> Result<f64, Status> {
    let mut rolled = instrument.clone();
    rolled.expiry_years = expiry_years;
    let priced = price_instrument(&rolled, market, conv)
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok(priced.greeks.price)
}

/// A shocked copy of a market context with one factor bumped (relative for spot,
/// absolute for vol/rates), used by the cross-gamma 2-D stencil.
fn bump_factor(
    m: &WireMarketContext,
    factor: shock_axis::Factor,
    sign: f64,
) -> (WireMarketContext, f64) {
    let mut out = *m;
    let h = match factor {
        shock_axis::Factor::Spot => {
            let h = m.spot * CROSS_SPOT_REL;
            out.spot += sign * h;
            h
        }
        shock_axis::Factor::Vol => {
            out.vol += sign * CROSS_VOL_ABS;
            CROSS_VOL_ABS
        }
        shock_axis::Factor::RateDom => {
            out.r_dom += sign * CROSS_RATE_ABS;
            CROSS_RATE_ABS
        }
        shock_axis::Factor::RateFor => {
            out.r_for += sign * CROSS_RATE_ABS;
            CROSS_RATE_ABS
        }
        // Time is rolled on expiry, not the market; the cross-gamma loop handles a
        // time factor by rolling expiry rather than the context (see below).
        shock_axis::Factor::Time => 0.0,
    };
    (out, h)
}

#[tonic::async_trait]
impl SurfaceService for SurfaceEdge {
    async fn get_smile(
        &self,
        request: Request<GetSmileRequest>,
    ) -> Result<Response<Smile>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;
        let conv = decode_conv(&wire_conv)?;

        // Read the maker's live market and mark a smile on the fly at the requested
        // tenor from the live ATM vol (a neutral smile when no broker skew is
        // supplied: flat RR/BF), so GetSmile always returns a calibrated slice.
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let market = WireMarketContext {
            spot: snap.spot,
            vol: snap.atm_vol,
            r_dom: snap.r_dom,
            r_for: snap.r_for,
        };
        let broker = BrokerQuoteSet {
            tenor_years: req.tenor_years,
            atm_vol: snap.atm_vol,
            rr_25: 0.0,
            bf_25: 0.0010,
            rr_10: 0.0,
            bf_10: 0.0,
            // A neutral on-the-fly mark from live ATM only: three-point.
            has_ten_delta: false,
        };
        let smile = calibrate(&broker, &market, &conv)?;
        Ok(Response::new(smile_to_wire(
            req.pair,
            &broker,
            &smile,
            &conv,
            &market,
            &self.clock,
        )))
    }

    async fn mark_surface(
        &self,
        request: Request<MarkSurfaceRequest>,
    ) -> Result<Response<MarkSurfaceResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;
        let conv = decode_conv(&wire_conv)?;
        if req.broker_quotes.is_empty() {
            return Err(Status::invalid_argument(
                "mark_surface requires at least one broker quote set",
            ));
        }

        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let market = WireMarketContext {
            spot: snap.spot,
            vol: snap.atm_vol,
            r_dom: snap.r_dom,
            r_for: snap.r_for,
        };

        // Stamp one fresh version for this whole mark and deposit every calibrated
        // tenor slice under it, so a later request can pin this exact surface.
        let version = self.surface_book.next_version();
        let pair = req
            .pair
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("mark_surface requires a `pair`"))?;

        let mut smiles = Vec::with_capacity(req.broker_quotes.len());
        for broker in &req.broker_quotes {
            let smile = calibrate(broker, &market, &conv)?;
            // Deposit the calibrated slice into the versioned registry so a pinned
            // price against `version` resolves this exact marked smile.
            self.surface_book.deposit(
                version,
                &pair.base,
                &pair.quote,
                broker.tenor_years,
                smile.forward(),
                smile,
            );
            smiles.push(smile_to_wire(
                req.pair.clone(),
                broker,
                &smile,
                &conv,
                &market,
                &self.clock,
            ));
        }

        Ok(Response::new(MarkSurfaceResponse {
            pair: req.pair,
            surface_version: version,
            smiles,
            epoch_nanos: self.clock.now_nanos(),
        }))
    }

    async fn scenario(
        &self,
        request: Request<ScenarioRequest>,
    ) -> Result<Response<ScenarioResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let instrument = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
        let base_market = req
            .base_market
            .ok_or_else(|| Status::invalid_argument("missing `base_market`"))?;
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;
        let conv = decode_conv(&wire_conv)?;

        // Decode every axis up front (factor + relative + steps).
        struct Axis {
            factor: shock_axis::Factor,
            relative: bool,
            steps: Vec<f64>,
        }
        let mut axes = Vec::with_capacity(req.axes.len());
        for a in &req.axes {
            let factor = shock_axis::Factor::try_from(a.factor).map_err(|_| {
                Status::invalid_argument(format!("unknown ShockAxis.Factor tag {}", a.factor))
            })?;
            if a.steps.is_empty() {
                return Err(Status::invalid_argument("each shock axis needs ≥ 1 step"));
            }
            axes.push(Axis {
                factor,
                relative: a.relative,
                steps: a.steps.clone(),
            });
        }

        // The base year-fraction-to-expiry the theta-roll axis decays from. The
        // request carries it explicitly (`expiry_years`); fall back to the
        // instrument's own expiry when the request leaves it at the zero default.
        let base_expiry = if req.expiry_years > 0.0 {
            req.expiry_years
        } else {
            instrument.expiry_years
        };

        // Walk the Cartesian product of the axes (mixed-radix counter), repricing
        // the instrument at each shocked market context and (theta-) rolled expiry.
        let mut points = Vec::new();
        let total: usize = axes.iter().map(|a| a.steps.len()).product::<usize>().max(1);
        // Guard against a pathological grid blowing up memory.
        if total > 1_000_000 {
            return Err(Status::invalid_argument(
                "scenario grid too large (> 1,000,000 nodes)",
            ));
        }
        for node in 0..total {
            let mut shocked = base_market;
            let mut applied = Vec::with_capacity(axes.len());
            // The node's effective expiry: the base expiry minus any FACTOR_TIME
            // roll step (calendar rolled forward). Floored just above zero so a
            // roll that reaches expiry stays in the pricing domain.
            let mut expiry = base_expiry;
            let mut rem = node;
            for axis in &axes {
                let idx = rem % axis.steps.len();
                rem /= axis.steps.len();
                let step = axis.steps[idx];
                applied.push(step);
                if axis.factor == shock_axis::Factor::Time {
                    // Theta roll: subtract the (absolute) years rolled forward.
                    expiry = (expiry - step).max(f64::MIN_POSITIVE);
                } else {
                    shocked = apply_shock(&shocked, axis.factor, axis.relative, step);
                }
            }
            // Reprice at the node's shocked market and rolled expiry.
            let mut rolled = instrument.clone();
            rolled.expiry_years = expiry;
            let priced = price_instrument(&rolled, &shocked, &conv)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            points.push(ScenarioPoint {
                applied_shocks: applied,
                shocked_market: Some(shocked),
                greeks: Some(priced.greeks.into()),
                expiry_years: expiry,
            });
        }

        // The book-shaped risk decomposition at the base market, if requested:
        // bucketed vega per (tenor, delta) pillar, cross-gamma per factor pair, and
        // the theta roll over the requested horizons.
        let bucketed_risk = req
            .risk_buckets
            .as_ref()
            .map(|rb| bucketed_risk(&instrument, &base_market, base_expiry, &conv, rb))
            .transpose()?;

        Ok(Response::new(ScenarioResponse {
            points,
            bucketed_risk,
        }))
    }
}

/// Compute the book-shaped risk decomposition for an instrument at its base market:
///
/// * **bucketed vega** — for each requested `(tenor, delta)` pillar, the value
///   sensitivity to a 1.0-absolute-vol bump of that pillar alone. Under the
///   scenario's flat-vol pricer a pillar at a *different* tenor than the
///   instrument's expiry has no effect (its bucket is zero); a pillar matching the
///   instrument's expiry carries the full central-difference vega — the honest
///   decomposition for a single-expiry structure on a flat scenario vol.
/// * **cross-gamma** — the mixed second derivative d²V/(dx_a dx_b) of each
///   requested factor pair, by a 2-D central stencil (e.g. spot×vol is the
///   book-level vanna aggregate; rate×spot couples discounting and forward).
/// * **theta roll** — the value at each requested horizon (years rolled forward),
///   holding the market still: the overnight / weekend decay of the book.
fn bucketed_risk(
    instrument: &celnet_proto::Instrument,
    base: &WireMarketContext,
    base_expiry: f64,
    conv: &ConventionSet,
    request: &celnet_proto::RiskBucketRequest,
) -> Result<BucketedRisk, Status> {
    // ---- bucketed vega per (tenor, delta) pillar ---------------------------
    let mut vega_buckets = Vec::with_capacity(request.vega_pillars.len());
    for pillar in &request.vega_pillars {
        // A pillar at a different tenor than the instrument's expiry has no effect
        // on a single-expiry structure priced off a flat scenario vol.
        let vega = if celnet_core::is_close(pillar.tenor_years, base_expiry, 1e-9, 1e-9) {
            let h = VEGA_BUCKET_BUMP;
            let up = WireMarketContext {
                vol: base.vol + h,
                ..*base
            };
            let dn = WireMarketContext {
                vol: base.vol - h,
                ..*base
            };
            let v_up = price_value_at(instrument, &up, base_expiry, conv)?;
            let v_dn = price_value_at(instrument, &dn, base_expiry, conv)?;
            (v_up - v_dn) / (2.0 * h)
        } else {
            0.0
        };
        vega_buckets.push(VegaBucket {
            tenor_years: pillar.tenor_years,
            delta: pillar.delta,
            vega,
        });
    }

    // ---- cross-gamma per factor pair ---------------------------------------
    let mut cross_gammas = Vec::with_capacity(request.cross_gamma_pairs.len());
    for pair in &request.cross_gamma_pairs {
        let fa = shock_axis::Factor::try_from(pair.factor_a).map_err(|_| {
            Status::invalid_argument(format!("unknown cross-gamma factor_a {}", pair.factor_a))
        })?;
        let fb = shock_axis::Factor::try_from(pair.factor_b).map_err(|_| {
            Status::invalid_argument(format!("unknown cross-gamma factor_b {}", pair.factor_b))
        })?;
        if fa == fb {
            return Err(Status::invalid_argument(
                "a cross-gamma pair must name two distinct factors",
            ));
        }
        let value = cross_gamma_value(instrument, base, base_expiry, conv, fa, fb)?;
        cross_gammas.push(CrossGamma {
            factor_a: pair.factor_a,
            factor_b: pair.factor_b,
            value,
        });
    }

    // ---- theta roll over the requested horizons ----------------------------
    let mut theta_roll = Vec::with_capacity(request.roll_horizons_years.len());
    for &horizon in &request.roll_horizons_years {
        let rolled_expiry = (base_expiry - horizon).max(f64::MIN_POSITIVE);
        theta_roll.push(price_value_at(instrument, base, rolled_expiry, conv)?);
    }

    Ok(BucketedRisk {
        vega_buckets,
        cross_gammas,
        theta_roll,
        roll_horizons_years: request.roll_horizons_years.clone(),
    })
}

/// The mixed second derivative d²V/(dx_a dx_b) at the base market by a 2-D central
/// stencil. A `FACTOR_TIME` factor rolls the expiry rather than the market context.
fn cross_gamma_value(
    instrument: &celnet_proto::Instrument,
    base: &WireMarketContext,
    base_expiry: f64,
    conv: &ConventionSet,
    fa: shock_axis::Factor,
    fb: shock_axis::Factor,
) -> Result<f64, Status> {
    // The signed bump along one factor: returns the (market, expiry, step-size)
    // triple, rolling expiry for the time factor and the market otherwise.
    let perturb = |m: &WireMarketContext, e: f64, f: shock_axis::Factor, sign: f64| {
        if f == shock_axis::Factor::Time {
            let h = base_expiry * CROSS_SPOT_REL; // a small relative time step.
            ((*m), (e - sign * h).max(f64::MIN_POSITIVE), h)
        } else {
            let (mm, h) = bump_factor(m, f, sign);
            (mm, e, h)
        }
    };

    // (+,+), (+,-), (-,+), (-,-) corners.
    let (m_pp, e_pp, ha) = {
        let (m1, e1, ha) = perturb(base, base_expiry, fa, 1.0);
        let (m2, e2, _) = perturb(&m1, e1, fb, 1.0);
        (m2, e2, ha)
    };
    let (m_pm, e_pm, _) = {
        let (m1, e1, _) = perturb(base, base_expiry, fa, 1.0);
        perturb(&m1, e1, fb, -1.0)
    };
    let (m_mp, e_mp, _) = {
        let (m1, e1, _) = perturb(base, base_expiry, fa, -1.0);
        perturb(&m1, e1, fb, 1.0)
    };
    let (m_mm, e_mm, hb) = {
        let (m1, e1, _) = perturb(base, base_expiry, fa, -1.0);
        let (m2, e2, hb) = perturb(&m1, e1, fb, -1.0);
        (m2, e2, hb)
    };

    let v_pp = price_value_at(instrument, &m_pp, e_pp, conv)?;
    let v_pm = price_value_at(instrument, &m_pm, e_pm, conv)?;
    let v_mp = price_value_at(instrument, &m_mp, e_mp, conv)?;
    let v_mm = price_value_at(instrument, &m_mm, e_mm, conv)?;

    Ok((v_pp - v_pm - v_mp + v_mm) / (4.0 * ha * hb))
}

/// Silence unused-import warnings for the convention enum aliases when the
/// associated `From` impls are the only users (they are referenced via
/// `convention_record`); keeping the imports explicit documents the vocabulary.
const _: fn() = || {
    let _ = (
        DeltaConvention::SpotUnadjusted,
        AtmConvention::AtmForward,
        PremiumStyle::DomesticPips,
        Cut::NewYork1000,
        DayCount::Act365Fixed,
        Settlement::Deliverable,
    );
};

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    fn broker(has_ten_delta: bool, rr_10: f64, bf_10: f64) -> BrokerQuoteSet {
        BrokerQuoteSet {
            tenor_years: 1.0,
            atm_vol: 0.10,
            rr_25: -0.004,
            bf_25: 0.002,
            rr_10,
            bf_10,
            has_ten_delta,
        }
    }

    /// Regression for the float-`!=` discriminator bug: the five-point-vs-three-
    /// point choice is driven off the explicit `has_ten_delta` presence flag, not a
    /// float compare on the wing values. A genuine *zero-but-present* 10Δ wing
    /// (`rr_10 == 0.0 && bf_10 == 0.0`, `has_ten_delta == true`) is therefore still
    /// treated as five-point — the outer wing pillar is present.
    #[test]
    fn zero_but_present_ten_delta_wing_is_five_point() {
        let q = quotes_from_broker(&broker(true, 0.0, 0.0));
        assert!(
            q.outer.is_some(),
            "a present zero 10Δ wing must calibrate five-point (outer pillar present)"
        );
    }

    /// An absent 10Δ wing (the three-point case) yields no outer pillar, regardless
    /// of the (ignored) wing values.
    #[test]
    fn absent_ten_delta_wing_is_three_point() {
        let q = quotes_from_broker(&broker(false, 0.0, 0.0));
        assert!(q.outer.is_none(), "no presence flag ⇒ three-point");
        // Non-zero wing values are *ignored* when the flag is absent: still 3-point.
        let q2 = quotes_from_broker(&broker(false, -0.01, 0.02));
        assert!(
            q2.outer.is_none(),
            "the wing values never decide presence — only the flag does"
        );
    }

    /// A populated five-point set carries its 10Δ wing values through to the outer
    /// pillar.
    #[test]
    fn populated_five_point_carries_wings() {
        let q = quotes_from_broker(&broker(true, -0.012, 0.006));
        let outer = q.outer.expect("five-point has an outer pillar");
        assert!(is_close(outer.risk_reversal, -0.012, 1e-12, 1e-12));
        assert!(is_close(outer.butterfly, 0.006, 1e-12, 1e-12));
    }
}
