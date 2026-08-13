//! The **SOFR STIR futures** arm of the LP-SIM feed: the listed `SR3` / `SR1` strip
//! from [`celnet_refdata::stir_futures_universe`], streamed as an exchange-convention
//! two-way on the same `(member, instrument_id)` path the bonds, the deliverable
//! futures and the OIS curve points use.
//!
//! # Why the sim must quote these
//!
//! The platform prices OIS off an internal curve and cash Treasuries off an
//! aggregated book. A STIR future is the listed instrument that connects them: it is
//! how a rates desk expresses a front-end view and how a short-dated hedge actually
//! executes. If the registry advertises the strip as tradeable but no LP quotes it,
//! no composite line exists, and every hedge routed at it silently backstops — the
//! same gap [`crate::futures`] closed for the deliverable complex.
//!
//! # What a contract is anchored to, and what is honestly modelled
//!
//! A STIR contract's index is `100 − R`, so its level is a **rate** observation, not
//! a price. The anchor is taken from the venue's real quoted OIS curve
//! ([`crate::ois::load_ois_universe`]) — the same curve the swap lines stream — so
//! the STIR strip and the swap strip move off one consistent view of USD rates rather
//! than two unrelated handles.
//!
//! **The honest boundary:** the platform's on-the-run swap tenors start at **2 years**
//! ([`celnet_refdata::ON_THE_RUN_SWAP_TENORS`]), and the STIR strip lives entirely
//! inside that first two years. There is therefore no *observed* curve point at a
//! STIR contract's own maturity. The front is a **modelled extension** of the real
//! 2-year observation: the shortest real point sets the level, and
//! [`FRONT_END_PREMIUM`] shapes the run-up to it, decaying linearly to zero at the
//! 2-year anchor so the strip meets the observed curve continuously. That is a
//! stated modelling choice, not an observation dressed as one — exactly the
//! distinction [`crate::futures`] draws when it says a futures price is *"a real
//! market observation at the matching point of the curve, not a chosen handle"*.
//! Here there is no matching point, so the extension is named rather than hidden.
//!
//! # Quotation
//!
//! The index is carried directly as the line's quoted mid, snapped to the contract's
//! outright tick (half a basis point) with the bid rounded down and the offer up, as
//! a listed market maker quotes.

use celnet_aggregation::Instrument;
use celnet_refdata::{CivilYmd, StirFutureSpec, listed_stir_on};
use celnet_types::{BrokenDate, Ccy, CommodityRef, Symbol, Tenor, Underlying};

use crate::ois::OisCurvePoint;
use crate::price::{MidSource, RateModel};
use crate::quoted::QuotedLine;

/// The premium, in decimal rate, of the very front of the strip over the shortest
/// observed curve point. The committed USD OIS curve is front-dipped (2y `4.05%`
/// through a `3.92%` belly), i.e. inverted at the front, so the sub-2y rates a STIR
/// strip references sit **above** the 2-year. 25 bp over ~2 years of run-off is a
/// front-end slope consistent with that shape.
pub const FRONT_END_PREMIUM: f64 = 0.002_5;

/// The tenor, in months, at which [`FRONT_END_PREMIUM`] has fully decayed — the
/// shortest genuinely observed point on the curve, so the modelled front meets the
/// real curve continuously there.
pub const FRONT_END_ANCHOR_MONTHS: f64 = 24.0;

/// The half-spread a member shows, in ticks.
const HALF_SPREAD_TICKS: f64 = 0.5;
/// The peak directional lean of the outermost member, in ticks.
const LEAN_TICKS: f64 = 0.1;
/// The peak starting-level dispersion of a member, in ticks.
const DISPERSION_TICKS: f64 = 0.1;
/// The panel's outermost member is this many `skew_step`s from the centre
/// (`(n-1)/2` for the deployed 5-member panel).
const PANEL_HALF_WIDTH_STEPS: f64 = 2.0;

/// One listed STIR contract the feed quotes, with the reference rate it is anchored
/// to resolved from the venue's own OIS curve.
#[derive(Debug, Clone, PartialEq)]
pub struct StirContract {
    /// The listed contract's reference-data specification.
    pub spec: StirFutureSpec,
    /// The anchor rate for this contract's reference period, as a **decimal** — the
    /// shortest observed curve point extended to the front (see the module note).
    pub reference_rate: f64,
}

impl StirContract {
    /// The contract's reference index price, `100 − R`.
    #[must_use]
    pub fn reference_index(&self) -> f64 {
        StirFutureSpec::index_for_rate(self.reference_rate)
    }

    /// The aggregation engine's asset-agnostic key. Keyed by the contract code and
    /// expiring on the last trading date, so it is disjoint from every cash bond's
    /// key and from every deliverable future's — no two instruments consolidate onto
    /// one line.
    #[must_use]
    pub fn engine_instrument(&self) -> Instrument {
        let expiry = self.spec.last_trading_date;
        Instrument::new(
            Underlying::Commodity(CommodityRef::new(
                Symbol::new(self.spec.instrument_id.clone(), ""),
                Ccy::USD,
            )),
            Tenor::BrokenDate(BrokenDate::new(
                expiry.year,
                u8::try_from(expiry.month).unwrap_or(1),
                u8::try_from(expiry.day).unwrap_or(1),
            )),
        )
    }

    /// Build the quoted line for this contract.
    ///
    /// The index is carried by a [`MidSource::MeanRevertingRate`], whose sampled
    /// level *is* the quoted market (no price map) — the same treatment the OIS curve
    /// points get, and the right one here: a STIR index is quoted directly, not
    /// derived from a security's cash flows. The model works in decimals and quotes
    /// in percent, so the index is seeded as `index / 100`.
    #[must_use]
    pub fn to_line(
        &self,
        base_half_spread: f64,
        base_skew_step: f64,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> QuotedLine {
        let tick = self.spec.terms.tick_size_points;
        let ratio = |target: f64, base: f64| {
            if base.is_finite() && base > 0.0 {
                target / base
            } else {
                1.0
            }
        };
        // Displacements are budgeted in TICKS of index, then converted to the model's
        // natural units (the index over 100) so the panel's quoted width is set by the
        // contract's own grid rather than by a bond-shaped spread.
        let to_model = |ticks: f64| ticks * tick / 100.0;
        let lean_scale = if base_skew_step.is_finite() && base_skew_step > 0.0 {
            (LEAN_TICKS * tick) / PANEL_HALF_WIDTH_STEPS / base_skew_step
        } else {
            1.0
        };

        QuotedLine {
            instrument_id: self.spec.instrument_id.clone(),
            display_name: self.spec.name.clone(),
            identity: self.spec.instrument_id.clone(),
            instrument: self.engine_instrument(),
            mid: MidSource::MeanRevertingRate(RateModel {
                long_run_rate: self.reference_index() / 100.0,
                initial_rate: self.reference_index() / 100.0,
                reversion_per_sec,
                perturbation,
            }),
            spread_scale: ratio(HALF_SPREAD_TICKS * tick, base_half_spread),
            lean_scale,
            yield_dispersion: Some(to_model(DISPERSION_TICKS)),
            dealer_view: to_model(DISPERSION_TICKS),
            // A listed contract trades ON a published grid.
            tick: Some(tick),
        }
    }
}

/// Whole months from `from` to `to`, floored at zero.
fn months_between(from: CivilYmd, to: CivilYmd) -> f64 {
    let months = (to.year - from.year) * 12 + i32::try_from(to.month).unwrap_or(0)
        - i32::try_from(from.month).unwrap_or(0);
    f64::from(months.max(0))
}

/// The anchor rate for a contract whose reference period starts `months_out` from the
/// valuation date: the shortest observed curve point, plus the front-end premium
/// decaying linearly to zero at [`FRONT_END_ANCHOR_MONTHS`].
///
/// Separated out and public so the modelling choice is testable in isolation rather
/// than only observable through a streamed quote.
#[must_use]
pub fn anchor_rate(shortest_observed_rate: f64, months_out: f64) -> f64 {
    let decay = (1.0 - months_out / FRONT_END_ANCHOR_MONTHS).clamp(0.0, 1.0);
    shortest_observed_rate + FRONT_END_PREMIUM * decay
}

/// The shortest-tenor point of the venue's own OIS curve — the one real observation
/// the STIR strip is anchored to. `None` for an empty curve.
fn shortest_observed(points: &[OisCurvePoint]) -> Option<&OisCurvePoint> {
    points.iter().min_by_key(|p| p.tenor_years)
}

/// Load the listed STIR contracts still trading on `as_of`, each anchored to the
/// venue's own OIS curve. Returns empty when the curve carries no point to anchor to
/// — a feed that cannot source a real level must quote nothing rather than invent one.
#[must_use]
pub fn load_stir_universe(points: &[OisCurvePoint], as_of: CivilYmd) -> Vec<StirContract> {
    let Some(anchor) = shortest_observed(points) else {
        return Vec::new();
    };
    listed_stir_on(as_of)
        .into_iter()
        .map(|spec| {
            let months_out = months_between(as_of, spec.reference_period_start);
            StirContract {
                reference_rate: anchor_rate(anchor.reference_rate, months_out),
                spec,
            }
        })
        .collect()
}

/// The quoted lines for a set of STIR contracts.
#[must_use]
pub fn stir_lines(
    contracts: &[StirContract],
    base_half_spread: f64,
    base_skew_step: f64,
    reversion_per_sec: f64,
    perturbation: f64,
) -> Vec<QuotedLine> {
    contracts
        .iter()
        .map(|c| {
            c.to_line(
                base_half_spread,
                base_skew_step,
                reversion_per_sec,
                perturbation,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ois::load_ois_universe;

    fn as_of() -> CivilYmd {
        CivilYmd::new(2026, 4, 16)
    }

    #[test]
    fn every_listed_contract_is_quotable_and_anchored_to_the_real_curve() {
        let points = load_ois_universe();
        let contracts = load_stir_universe(&points, as_of());
        assert_eq!(
            contracts.len(),
            celnet_refdata::listed_stir_on(as_of()).len(),
            "a tradeable-but-unquotable contract never reaches an aggregated book"
        );
        let shortest = shortest_observed(&points).expect("curve").reference_rate;
        for c in &contracts {
            assert!(
                c.reference_rate.is_finite() && c.reference_rate > 0.0 && c.reference_rate < 0.20,
                "{}: implausible anchor {}",
                c.spec.instrument_id,
                c.reference_rate
            );
            // Front-dipped curve ⇒ the modelled front sits at or above the 2y point,
            // never below it.
            assert!(c.reference_rate >= shortest - 1e-12);
            assert!(c.reference_rate <= shortest + FRONT_END_PREMIUM + 1e-12);
            // The index is the quoted market and must be a sane sub-100 handle.
            assert!(c.reference_index() > 90.0 && c.reference_index() < 100.0);
        }
    }

    #[test]
    fn the_front_end_extension_decays_to_the_observed_point() {
        let base = 0.0405;
        // At the valuation date the whole premium applies…
        assert!((anchor_rate(base, 0.0) - (base + FRONT_END_PREMIUM)).abs() < 1e-12);
        // …halfway out, half of it…
        assert!((anchor_rate(base, 12.0) - (base + FRONT_END_PREMIUM / 2.0)).abs() < 1e-12);
        // …and at the observed anchor it is exactly the observation, so the modelled
        // front meets the real curve continuously rather than stepping onto it.
        assert!((anchor_rate(base, FRONT_END_ANCHOR_MONTHS) - base).abs() < 1e-12);
        // Beyond the anchor it never dips below the observation.
        assert!((anchor_rate(base, 60.0) - base).abs() < 1e-12);
    }

    #[test]
    fn the_strip_slopes_down_in_rate_so_the_index_rises_along_it() {
        let contracts = load_stir_universe(&load_ois_universe(), as_of());
        let sr3: Vec<_> = contracts
            .iter()
            .filter(|c| c.spec.terms.symbol == "SR3")
            .collect();
        assert!(sr3.len() >= 2, "need a strip to compare");
        for w in sr3.windows(2) {
            assert!(
                w[1].reference_rate <= w[0].reference_rate + 1e-12,
                "a further contract must not price ABOVE a nearer one on this curve"
            );
            assert!(
                w[1].reference_index() >= w[0].reference_index() - 1e-12,
                "index moves inversely to rate"
            );
        }
    }

    #[test]
    fn a_line_quotes_on_the_contract_grid_with_a_distinct_engine_key() {
        let contracts = load_stir_universe(&load_ois_universe(), as_of());
        let lines = stir_lines(&contracts, 0.02, 0.25, 0.05, 2e-4);
        assert_eq!(lines.len(), contracts.len());

        let mut keys = std::collections::BTreeSet::new();
        for (line, c) in lines.iter().zip(&contracts) {
            assert_eq!(line.instrument_id, c.spec.instrument_id);
            assert_eq!(
                line.tick,
                Some(c.spec.terms.tick_size_points),
                "a listed contract must quote ON its published grid"
            );
            // The mid the panel will quote is the contract's index, in index points.
            let mid = line.mid.mid_at(0, 0.0);
            assert!(
                (mid - c.reference_index()).abs() < 1e-9,
                "{}: mid {mid} should be the index {}",
                c.spec.instrument_id,
                c.reference_index()
            );
            assert!(keys.insert(format!("{:?}", line.instrument)));
        }
        assert_eq!(keys.len(), lines.len(), "engine keys must be injective");
    }

    #[test]
    fn an_empty_curve_yields_no_quotes_rather_than_an_invented_level() {
        assert!(load_stir_universe(&[], as_of()).is_empty());
    }
}
