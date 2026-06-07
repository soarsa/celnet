//! The typed surface vocabulary: market context, broker quote sets, calibrated
//! smiles, arbitrage reports, and scenario grids.
//!
//! These mirror the wire surface family ([`celnet_proto::Smile`],
//! [`celnet_proto::BrokerQuoteSet`], [`celnet_proto::ArbReport`],
//! [`celnet_proto::ScenarioResponse`]) but in celnet-domain form: a [`Smile`]
//! exposes its delta-axis points as typed [`SmilePoint`]s and an [`ArbReport`] as
//! a typed struct a quant can branch on, never `Option<…>`-wrapped proto.

use celnet_types::Greeks;

use crate::error::{ClientError, ClientResult};
use crate::vocab::{Calibration, Conventions};

/// Relative tolerance for matching a calibrated smile pillar by its signed
/// convention delta. Pillars sit at conventional deltas (0.10, 0.25, 0.50, …),
/// so a tight relative band cleanly distinguishes adjacent pillars while
/// absorbing the last-ULP wobble of a round-tripped wire value.
const DELTA_MATCH_REL: f64 = 1e-9;
/// Absolute tolerance for the delta-pillar match, covering a near-zero delta.
const DELTA_MATCH_ABS: f64 = 1e-9;

/// The market context a scenario / what-if grid is shocked around — spot, vol,
/// and the two continuous rates. The typed form of the wire `MarketContext`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketContext {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Annualized volatility (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Continuously-compounded domestic (quote) interest rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) interest rate.
    pub r_for: f64,
}

impl MarketContext {
    pub(crate) fn to_wire(self) -> celnet_proto::MarketContext {
        celnet_proto::MarketContext {
            spot: self.spot,
            vol: self.vol,
            r_dom: self.r_dom,
            r_for: self.r_for,
        }
    }

    pub(crate) fn from_wire(w: &celnet_proto::MarketContext) -> Self {
        Self {
            spot: w.spot,
            vol: w.vol,
            r_dom: w.r_dom,
            r_for: w.r_for,
        }
    }
}

/// The broker market-quote set for a single tenor — ATM plus the 25Δ and 10Δ
/// risk reversals and butterflies — the standard inputs a desk marks a smile
/// from. The typed form of the wire `BrokerQuoteSet`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrokerQuoteSet {
    /// The tenor (year fraction) this quote set marks.
    pub tenor_years: f64,
    /// At-the-money volatility (absolute).
    pub atm_vol: f64,
    /// 25-delta risk reversal (call vol − put vol).
    pub rr_25: f64,
    /// 25-delta butterfly (wing avg − ATM).
    pub bf_25: f64,
    /// 10-delta risk reversal.
    pub rr_10: f64,
    /// 10-delta butterfly.
    pub bf_10: f64,
    /// Whether the 10Δ wings are present (drive a five-point calibration). An
    /// explicit presence flag, not a float test on `rr_10`/`bf_10`, so a genuine
    /// zero-but-present 10Δ wing is still calibrated five-point.
    pub has_ten_delta: bool,
}

impl BrokerQuoteSet {
    /// A three-point quote set (ATM + 25Δ RR/BF), leaving the 10Δ wings absent.
    #[must_use]
    pub fn three_point(tenor_years: f64, atm_vol: f64, rr_25: f64, bf_25: f64) -> Self {
        Self {
            tenor_years,
            atm_vol,
            rr_25,
            bf_25,
            rr_10: 0.0,
            bf_10: 0.0,
            has_ten_delta: false,
        }
    }

    /// A five-point quote set (ATM + 25Δ and 10Δ RR/BF), marking the 10Δ wings as
    /// present so the surface calibrates a five-point smile.
    #[must_use]
    pub fn five_point(
        tenor_years: f64,
        atm_vol: f64,
        rr_25: f64,
        bf_25: f64,
        rr_10: f64,
        bf_10: f64,
    ) -> Self {
        Self {
            tenor_years,
            atm_vol,
            rr_25,
            bf_25,
            rr_10,
            bf_10,
            has_ten_delta: true,
        }
    }

    pub(crate) fn to_wire(self) -> celnet_proto::BrokerQuoteSet {
        celnet_proto::BrokerQuoteSet {
            tenor_years: self.tenor_years,
            atm_vol: self.atm_vol,
            rr_25: self.rr_25,
            bf_25: self.bf_25,
            rr_10: self.rr_10,
            bf_10: self.bf_10,
            has_ten_delta: self.has_ten_delta,
        }
    }

    pub(crate) fn from_wire(w: &celnet_proto::BrokerQuoteSet) -> Self {
        Self {
            tenor_years: w.tenor_years,
            atm_vol: w.atm_vol,
            rr_25: w.rr_25,
            bf_25: w.bf_25,
            rr_10: w.rr_10,
            bf_10: w.bf_10,
            has_ten_delta: w.has_ten_delta,
        }
    }
}

/// One quoted vol point on the calibrated smile, keyed by signed convention delta
/// and expiry. The typed form of the wire `SmilePoint`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmilePoint {
    /// Signed convention delta (e.g. `0.25` for a 25Δ call, `-0.25` for a 25Δ put).
    pub delta: f64,
    /// Expiry as a year fraction.
    pub tenor_years: f64,
    /// Absolute volatility at this `(delta, tenor)`.
    pub vol: f64,
}

/// The arbitrage status of a calibrated smile — the transparency a desk wants
/// when marking. The typed form of the wire `ArbReport`.
#[derive(Debug, Clone)]
pub struct ArbReport {
    /// True if the smile is free of butterfly (vertical) arbitrage.
    pub butterfly_arbitrage_free: bool,
    /// True if the surface is free of calendar (horizontal) arbitrage.
    pub calendar_arbitrage_free: bool,
    /// The worst negative density observed (`0.0` = none).
    pub worst_density: f64,
    /// A human-readable note on any repair applied during calibration. Still
    /// embeds a `model=<label>` token for human eyes, but [`ArbReport::model`] is
    /// the authoritative, typed provenance — read the calibration family from that
    /// field, never by parsing this string.
    pub note: String,
    /// The calibration family this smile was actually marked under — the typed,
    /// authoritative provenance the server stamps on every marked smile (the typed
    /// form of the wire `ArbReport.smile_model`). Read this instead of parsing the
    /// `model=` token from [`ArbReport::note`].
    pub model: Calibration,
}

impl ArbReport {
    /// True iff the smile passed both the butterfly and the calendar checks.
    #[must_use]
    pub fn is_arbitrage_free(&self) -> bool {
        self.butterfly_arbitrage_free && self.calendar_arbitrage_free
    }

    pub(crate) fn from_wire(w: &celnet_proto::ArbReport) -> Self {
        Self {
            butterfly_arbitrage_free: w.butterfly_arbitrage_free,
            calendar_arbitrage_free: w.calendar_arbitrage_free,
            worst_density: w.worst_density,
            note: w.note.clone(),
            model: crate::vocab::calibration_from_wire(w.smile_model),
        }
    }
}

/// A calibrated smile for one `(pair, tenor)`: the marked broker quote set, the
/// delta-axis vol points, the conventions, and the arbitrage report. The typed
/// form of the wire `Smile`.
#[derive(Debug, Clone)]
pub struct Smile {
    /// The tenor (year fraction).
    pub tenor_years: f64,
    /// The broker quote set the smile was marked from.
    pub broker_quotes: BrokerQuoteSet,
    /// The calibrated delta-axis vol points (10Δ / 25Δ wings + 50Δ ATM).
    pub points: Vec<SmilePoint>,
    /// The conventions the smile is expressed under.
    pub conventions: Conventions,
    /// The arbitrage status of the calibrated smile.
    pub arbitrage: ArbReport,
    /// Calibration time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl Smile {
    /// The calibrated vol at the signed convention delta `delta`, if a pillar at
    /// that delta is present (within a tight tolerance).
    #[must_use]
    pub fn vol_at_delta(&self, delta: f64) -> Option<f64> {
        self.points
            .iter()
            .find(|p| celnet_core::is_close(p.delta, delta, DELTA_MATCH_REL, DELTA_MATCH_ABS))
            .map(|p| p.vol)
    }

    /// The at-the-money (50Δ) calibrated vol, if present.
    #[must_use]
    pub fn atm_vol(&self) -> Option<f64> {
        self.vol_at_delta(0.50)
    }

    pub(crate) fn from_wire(w: celnet_proto::Smile) -> ClientResult<Self> {
        let broker_quotes = w
            .broker_quotes
            .as_ref()
            .map(BrokerQuoteSet::from_wire)
            .ok_or(ClientError::MissingField("Smile.broker_quotes"))?;
        let conventions = w
            .conventions
            .as_ref()
            .ok_or(ClientError::MissingField("Smile.conventions"))
            .and_then(Conventions::from_wire)?;
        let arbitrage = w
            .arbitrage
            .as_ref()
            .map(ArbReport::from_wire)
            .ok_or(ClientError::MissingField("Smile.arbitrage"))?;
        let points = w
            .points
            .iter()
            .map(|p| SmilePoint {
                delta: p.delta,
                tenor_years: p.tenor_years,
                vol: p.vol,
            })
            .collect();
        Ok(Self {
            tenor_years: w.tenor_years,
            broker_quotes,
            points,
            conventions,
            arbitrage,
            epoch_nanos: w.epoch_nanos,
        })
    }
}

/// The result of a surface mark: a server-assigned surface version and the
/// per-tenor calibrated smiles. The typed form of the wire `MarkSurfaceResponse`.
#[derive(Debug, Clone)]
pub struct MarkedSurface {
    /// A monotonic surface version id for this calibration (the handle a desk
    /// pins risk to between re-marks).
    pub surface_version: u64,
    /// The calibrated per-tenor smiles.
    pub smiles: Vec<Smile>,
    /// Mark time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl MarkedSurface {
    pub(crate) fn from_wire(w: celnet_proto::MarkSurfaceResponse) -> ClientResult<Self> {
        let mut smiles = Vec::with_capacity(w.smiles.len());
        for s in w.smiles {
            smiles.push(Smile::from_wire(s)?);
        }
        Ok(Self {
            surface_version: w.surface_version,
            smiles,
            epoch_nanos: w.epoch_nanos,
        })
    }
}

/// Which risk factor a scenario axis shocks — the typed form of the wire
/// `ShockAxis.Factor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShockFactor {
    /// Shock the spot FX rate.
    Spot,
    /// Shock the volatility.
    Vol,
    /// Shock the domestic rate.
    RateDom,
    /// Shock the foreign rate.
    RateFor,
    /// Roll calendar time forward (theta roll): each step is a number of years
    /// subtracted from time-to-expiry, holding the market still. Always an absolute
    /// (additive) shock to the year fraction.
    Time,
}

impl ShockFactor {
    fn to_wire(self) -> celnet_proto::shock_axis::Factor {
        match self {
            ShockFactor::Spot => celnet_proto::shock_axis::Factor::Spot,
            ShockFactor::Vol => celnet_proto::shock_axis::Factor::Vol,
            ShockFactor::RateDom => celnet_proto::shock_axis::Factor::RateDom,
            ShockFactor::RateFor => celnet_proto::shock_axis::Factor::RateFor,
            ShockFactor::Time => celnet_proto::shock_axis::Factor::Time,
        }
    }

    /// Decode a wire `ShockAxis.Factor` tag, mapping an out-of-range tag to a
    /// [`ClientError::Wire`].
    fn from_wire(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        Ok(match celnet_proto::shock_axis::Factor::try_from(tag) {
            Ok(celnet_proto::shock_axis::Factor::Spot) => ShockFactor::Spot,
            Ok(celnet_proto::shock_axis::Factor::Vol) => ShockFactor::Vol,
            Ok(celnet_proto::shock_axis::Factor::RateDom) => ShockFactor::RateDom,
            Ok(celnet_proto::shock_axis::Factor::RateFor) => ShockFactor::RateFor,
            Ok(celnet_proto::shock_axis::Factor::Time) => ShockFactor::Time,
            Err(_) => {
                return Err(WireError::UnknownEnum {
                    kind: "ShockAxis.Factor",
                    tag,
                }
                .into());
            }
        })
    }
}

/// One axis of a scenario grid: a swept perturbation of one risk factor. The
/// typed form of the wire `ShockAxis`. Steps are relative (multiplicative) or
/// absolute (additive).
#[derive(Debug, Clone, PartialEq)]
pub struct ShockAxis {
    /// The risk factor this axis shocks.
    pub factor: ShockFactor,
    /// True if each step is relative (`× (1 + step)`); false if absolute (`+ step`).
    pub relative: bool,
    /// The swept shock steps (e.g. `-0.2, -0.1, 0.0, 0.1, 0.2`).
    pub steps: Vec<f64>,
}

impl ShockAxis {
    /// A relative (multiplicative) shock axis.
    #[must_use]
    pub fn relative(factor: ShockFactor, steps: Vec<f64>) -> Self {
        Self {
            factor,
            relative: true,
            steps,
        }
    }

    /// An absolute (additive) shock axis.
    #[must_use]
    pub fn absolute(factor: ShockFactor, steps: Vec<f64>) -> Self {
        Self {
            factor,
            relative: false,
            steps,
        }
    }

    pub(crate) fn to_wire(&self) -> celnet_proto::ShockAxis {
        celnet_proto::ShockAxis {
            factor: self.factor.to_wire() as i32,
            relative: self.relative,
            steps: self.steps.clone(),
        }
    }
}

/// One repriced node of a scenario grid: the applied shocks, the shocked market,
/// and the repriced Greek set. The typed form of the wire `ScenarioPoint`.
#[derive(Debug, Clone)]
pub struct ScenarioNode {
    /// The applied shock per axis, in axis order.
    pub applied_shocks: Vec<f64>,
    /// The shocked market this node was priced against.
    pub shocked_market: MarketContext,
    /// The full Greek set at this node.
    pub greeks: Greeks,
}

/// The repriced scenario grid — the typed form of the wire `ScenarioResponse`.
#[derive(Debug, Clone)]
pub struct ScenarioGrid {
    /// The repriced grid nodes (the Cartesian product of the request axes).
    pub nodes: Vec<ScenarioNode>,
}

impl ScenarioGrid {
    /// The node whose applied shocks exactly match `shocks`, if present (e.g. the
    /// unshocked base node `[0.0, 0.0]`).
    #[must_use]
    pub fn node_with_shocks(&self, shocks: &[f64]) -> Option<&ScenarioNode> {
        self.nodes.iter().find(|n| n.applied_shocks == shocks)
    }

    pub(crate) fn from_wire(w: celnet_proto::ScenarioResponse) -> ClientResult<Self> {
        let nodes = nodes_from_wire(w.points)?;
        Ok(Self { nodes })
    }
}

/// Decode the repriced scenario grid nodes from the wire points.
fn nodes_from_wire(points: Vec<celnet_proto::ScenarioPoint>) -> ClientResult<Vec<ScenarioNode>> {
    let mut nodes = Vec::with_capacity(points.len());
    for p in points {
        let shocked_market = p
            .shocked_market
            .as_ref()
            .map(MarketContext::from_wire)
            .ok_or(ClientError::MissingField("ScenarioPoint.shocked_market"))?;
        let greeks = p
            .greeks
            .as_ref()
            .map(crate::vocab::greeks_from_wire)
            .ok_or(ClientError::MissingField("ScenarioPoint.greeks"))?;
        nodes.push(ScenarioNode {
            applied_shocks: p.applied_shocks,
            shocked_market,
            greeks,
        });
    }
    Ok(nodes)
}

/// A request for the book-shaped risk decomposition a desk hedges against — the
/// typed form of the wire `RiskBucketRequest`. Each part is independently optional:
/// leave a field empty to omit that part of the decomposition.
///
/// * `vega_pillars` — the `(tenor, delta)` pillars to report bucketed vega for;
/// * `cross_gamma` — the factor pairs to report cross-gamma `d²V/(dx_a dx_b)` for;
/// * `roll_horizons_years` — the horizons (years rolled forward) for the theta roll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiskRequest {
    /// The `(tenor_years, signed-delta)` pillars to bucket vega across.
    pub vega_pillars: Vec<(f64, f64)>,
    /// The factor pairs to report cross-gamma for (each must name two distinct
    /// factors).
    pub cross_gamma: Vec<(ShockFactor, ShockFactor)>,
    /// The horizons (years rolled forward) the theta roll decays the book over.
    pub roll_horizons_years: Vec<f64>,
}

impl RiskRequest {
    /// A request for bucketed vega at the given `(tenor, delta)` pillars only.
    #[must_use]
    pub fn vega(pillars: Vec<(f64, f64)>) -> Self {
        Self {
            vega_pillars: pillars,
            ..Self::default()
        }
    }

    /// Builder: add the cross-gamma factor pairs to report.
    #[must_use]
    pub fn with_cross_gamma(mut self, pairs: Vec<(ShockFactor, ShockFactor)>) -> Self {
        self.cross_gamma = pairs;
        self
    }

    /// Builder: add the theta-roll horizons (years rolled forward).
    #[must_use]
    pub fn with_roll_horizons(mut self, horizons: Vec<f64>) -> Self {
        self.roll_horizons_years = horizons;
        self
    }

    pub(crate) fn to_wire(&self) -> celnet_proto::RiskBucketRequest {
        celnet_proto::RiskBucketRequest {
            vega_pillars: self
                .vega_pillars
                .iter()
                .map(|&(tenor_years, delta)| celnet_proto::VegaBucket {
                    tenor_years,
                    delta,
                    vega: 0.0, // ignored on the request; the pillar selects the bucket.
                })
                .collect(),
            cross_gamma_pairs: self
                .cross_gamma
                .iter()
                .map(|&(a, b)| celnet_proto::CrossGamma {
                    factor_a: a.to_wire() as i32,
                    factor_b: b.to_wire() as i32,
                    value: 0.0, // ignored on the request.
                })
                .collect(),
            roll_horizons_years: self.roll_horizons_years.clone(),
        }
    }
}

/// One bucketed-vega pillar: the book's value sensitivity to a 1.0-absolute-vol move
/// of the single smile pillar at `(tenor_years, delta)`, holding every other pillar
/// still. The typed form of the wire `VegaBucket`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegaPillar {
    /// The pillar tenor (year fraction) this bucket isolates.
    pub tenor_years: f64,
    /// The signed convention delta-pillar (e.g. `0.50` ATM, `±0.25`, `±0.10`).
    pub delta: f64,
    /// `dV/dσ` for a bump of this pillar alone (per 1.0 absolute vol).
    pub vega: f64,
}

/// One cross-gamma term: the mixed second-order sensitivity of value to a
/// simultaneous move of two distinct risk factors, `d²V/(dx_a dx_b)`. The typed form
/// of the wire `CrossGamma`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossGammaTerm {
    /// The first risk factor of the pair.
    pub factor_a: ShockFactor,
    /// The second risk factor of the pair (distinct from `factor_a`).
    pub factor_b: ShockFactor,
    /// The mixed second derivative `d²V/(dx_a dx_b)`.
    pub value: f64,
}

/// Book-shaped risk: the trader-facing decomposition a desk actually hedges against
/// — vega bucketed across the `(tenor, delta-pillar)` grid, the off-diagonal
/// cross-gamma terms, and the theta roll over a horizon. The typed form of the wire
/// `BucketedRisk`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BucketedRisk {
    /// Vega across the requested `(tenor, delta-pillar)` grid.
    pub vega_buckets: Vec<VegaPillar>,
    /// The off-diagonal cross-gamma terms requested.
    pub cross_gammas: Vec<CrossGammaTerm>,
    /// The theta roll: book value at each rolled horizon, holding the market still,
    /// parallel to `roll_horizons_years`.
    pub theta_roll: Vec<f64>,
    /// The horizons (years rolled forward) the `theta_roll` values correspond to.
    pub roll_horizons_years: Vec<f64>,
}

impl BucketedRisk {
    /// The bucketed vega at the `(tenor, delta)` pillar matching `tenor_years` and
    /// `delta` (within a tight tolerance), if present.
    #[must_use]
    pub fn vega_at(&self, tenor_years: f64, delta: f64) -> Option<f64> {
        self.vega_buckets
            .iter()
            .find(|b| {
                celnet_core::is_close(b.tenor_years, tenor_years, DELTA_MATCH_REL, DELTA_MATCH_ABS)
                    && celnet_core::is_close(b.delta, delta, DELTA_MATCH_REL, DELTA_MATCH_ABS)
            })
            .map(|b| b.vega)
    }

    /// The cross-gamma term for the unordered `{factor_a, factor_b}` pair, if present.
    #[must_use]
    pub fn cross_gamma(&self, factor_a: ShockFactor, factor_b: ShockFactor) -> Option<f64> {
        self.cross_gammas
            .iter()
            .find(|c| {
                (c.factor_a == factor_a && c.factor_b == factor_b)
                    || (c.factor_a == factor_b && c.factor_b == factor_a)
            })
            .map(|c| c.value)
    }

    fn from_wire(w: &celnet_proto::BucketedRisk) -> ClientResult<Self> {
        let vega_buckets = w
            .vega_buckets
            .iter()
            .map(|b| VegaPillar {
                tenor_years: b.tenor_years,
                delta: b.delta,
                vega: b.vega,
            })
            .collect();
        let mut cross_gammas = Vec::with_capacity(w.cross_gammas.len());
        for c in &w.cross_gammas {
            cross_gammas.push(CrossGammaTerm {
                factor_a: ShockFactor::from_wire(c.factor_a)?,
                factor_b: ShockFactor::from_wire(c.factor_b)?,
                value: c.value,
            });
        }
        Ok(Self {
            vega_buckets,
            cross_gammas,
            theta_roll: w.theta_roll.clone(),
            roll_horizons_years: w.roll_horizons_years.clone(),
        })
    }
}

/// A scenario result carrying both the repriced shock grid and the book-shaped risk
/// decomposition — the typed form of a `ScenarioResponse` produced by
/// [`crate::Client::scenario_with_risk`].
#[derive(Debug, Clone)]
pub struct ScenarioRisk {
    /// The repriced scenario grid.
    pub grid: ScenarioGrid,
    /// The book-shaped risk decomposition (bucketed vega, cross-gamma, theta roll)
    /// at the base market.
    pub risk: BucketedRisk,
}

impl ScenarioRisk {
    pub(crate) fn from_wire(w: celnet_proto::ScenarioResponse) -> ClientResult<Self> {
        let risk = w
            .bucketed_risk
            .as_ref()
            .map(BucketedRisk::from_wire)
            .transpose()?
            .unwrap_or_default();
        let grid = ScenarioGrid {
            nodes: nodes_from_wire(w.points)?,
        };
        Ok(Self { grid, risk })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the float-`!=` discriminator: the SDK marks 10Δ-wing
    /// presence with an explicit flag, not a float test. A `five_point` set always
    /// reports the wings present — even when both wing values are a genuine zero —
    /// and round-trips that presence across the wire, so the server calibrates it
    /// five-point. A `three_point` set reports them absent.
    #[test]
    fn five_point_marks_ten_delta_present_even_with_zero_wings() {
        let five = BrokerQuoteSet::five_point(1.0, 0.10, -0.004, 0.002, 0.0, 0.0);
        assert!(five.has_ten_delta, "five-point marks the 10Δ wings present");
        let round = BrokerQuoteSet::from_wire(&five.to_wire());
        assert!(round.has_ten_delta, "presence survives the wire round-trip");

        let three = BrokerQuoteSet::three_point(1.0, 0.10, -0.004, 0.002);
        assert!(
            !three.has_ten_delta,
            "three-point marks the 10Δ wings absent"
        );
        assert!(!BrokerQuoteSet::from_wire(&three.to_wire()).has_ten_delta);
    }

    fn smile_with_points(points: Vec<SmilePoint>) -> Smile {
        Smile {
            tenor_years: 1.0,
            broker_quotes: BrokerQuoteSet::three_point(1.0, 0.10, -0.004, 0.002),
            points,
            conventions: Conventions::major_default(),
            arbitrage: ArbReport {
                butterfly_arbitrage_free: true,
                calendar_arbitrage_free: true,
                worst_density: 0.0,
                note: String::new(),
                model: Calibration::MarketHedge,
            },
            epoch_nanos: 0,
        }
    }

    /// Regression for the float-`<` discriminator: pillar lookup routes through
    /// `celnet_core::is_close` (the determinism rule), matches a pillar within the
    /// tolerance band, and crucially does *not* alias adjacent conventional pillars
    /// (0.25 must never resolve a 0.10 query, and vice versa).
    #[test]
    fn vol_at_delta_matches_within_tolerance_without_pillar_aliasing() {
        let smile = smile_with_points(vec![
            SmilePoint {
                delta: -0.10,
                tenor_years: 1.0,
                vol: 0.130,
            },
            SmilePoint {
                delta: -0.25,
                tenor_years: 1.0,
                vol: 0.115,
            },
            SmilePoint {
                delta: 0.50,
                tenor_years: 1.0,
                vol: 0.100,
            },
            SmilePoint {
                delta: 0.25,
                tenor_years: 1.0,
                vol: 0.108,
            },
            SmilePoint {
                delta: 0.10,
                tenor_years: 1.0,
                vol: 0.121,
            },
        ]);

        // Exact pillar hits.
        assert_eq!(smile.vol_at_delta(-0.25), Some(0.115));
        assert_eq!(smile.atm_vol(), Some(0.100));

        // A query a hair inside the tolerance band still resolves the pillar.
        assert_eq!(
            smile.vol_at_delta(0.25 + DELTA_MATCH_ABS / 2.0),
            Some(0.108),
            "a query within tolerance resolves the 25Δ pillar"
        );

        // A query between two pillars resolves neither (no aliasing/fallthrough).
        assert_eq!(
            smile.vol_at_delta(0.175),
            None,
            "a delta between the 10Δ and 25Δ pillars matches no pillar"
        );

        // A pillar absent from the smile yields None, never a neighbour's vol.
        assert_eq!(smile.vol_at_delta(0.40), None);
    }
}
