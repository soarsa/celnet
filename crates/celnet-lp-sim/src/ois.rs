//! The **swap / OIS curve points** the simulator streams a two-way for.
//!
//! Until this module existed the simulator's quotable set was cash bonds and listed
//! futures only, so no aggregated book ever carried a swap line. That is what forced
//! the venue's OIS arm to quote off its own internal curve: there was no composite to
//! price against, and a book-driven policy silently degraded to the curve fallback.
//! Feeding the OIS curve points through the SAME `(member, instrument_id)` streaming
//! path the bonds use is what makes an OIS RFS priceable off real, consolidated
//! multi-dealer liquidity — and therefore tierable by a client's pricing group.
//!
//! ## Quoting convention
//!
//! An OIS is quoted as a **par fixed rate**, not as a price per 100 face. The line's
//! mid is therefore a [`MidSource::MeanRevertingRate`] sampled directly (see
//! [`crate::price::RateModel`]) rather than a yield mapped through the bond analytics
//! leaf, and it is reported in **percent** (`4.25` = 4.25%) — the units the market
//! quotes and the units the venue's rates two-way carries on the wire.
//!
//! ## Identity
//!
//! The canonical server `instrument_id` of a curve point is
//! `<curve-symbol>-<n>Y` (e.g. `USD-OIS-5Y`), built by [`ois_instrument_id`]. The
//! tenor is part of the id precisely because a bare curve symbol cannot distinguish
//! the 2-year point from the 10-year one: an aggregated book keys on `instrument_id`,
//! so a tenor-less key would consolidate every point of the curve onto one line and
//! quote a 10-year request off 2-year liquidity.

use celnet_aggregation::Instrument;
use celnet_types::{Ccy, CommodityRef, Symbol, Tenor, Underlying};

use crate::price::{MidSource, RateModel};
use crate::quoted::QuotedLine;

/// The curve symbol the venue prices OIS on (the server's one known rates symbol).
pub const USD_OIS_CURVE: &str = "USD-OIS";

/// A typical dealer two-way on an on-the-run OIS point, expressed as a multiplier on
/// the fleet's base half-spread (`2 bp` of a `100.0` price handle). An OIS trades far
/// tighter than a cash bond in its own quoted units: `0.25` puts the panel's typical
/// half-spread at `0.005` percent, i.e. a **half-basis-point** market in rate terms.
const OIS_SPREAD_SCALE: f64 = 0.25;

/// Peak directional lean of the outermost member, as a fraction of the half-spread.
const LEAN_FRACTION: f64 = 0.15;
/// Peak fixed private view of a member, as a fraction of the half-spread.
const VIEW_FRACTION: f64 = 0.10;
/// Peak wandering private view of a member, as a fraction of the half-spread.
const WANDER_FRACTION: f64 = 0.15;
/// The outermost member of the deployed panel sits this many `skew_step`s from the
/// centre (`(n−1)/2`), so a peak-displacement budget converts to a per-step scale by
/// dividing by it. Mirrors the cash-bond budget in [`crate::universe`].
const PANEL_HALF_WIDTH_STEPS: f64 = 2.0;

/// One point of a swap/OIS curve the simulator can quote.
#[derive(Debug, Clone, PartialEq)]
pub struct OisCurvePoint {
    /// The curve symbol this point belongs to (e.g. `USD-OIS`) — the FIX `Symbol(55)`
    /// an inbound rates request names.
    pub curve_symbol: String,
    /// The settlement/pricing currency of the curve.
    pub currency: Ccy,
    /// The whole-year tenor of the point (e.g. `5` for the 5-year).
    pub tenor_years: u16,
    /// The reference par rate the seeded process reverts to, as a **decimal**
    /// (e.g. `0.0392` for 3.92%).
    pub reference_rate: f64,
}

/// The canonical server `instrument_id` for a `(curve symbol, whole-year tenor)`
/// curve point: `USD-OIS` + `5` ⇒ `USD-OIS-5Y`.
///
/// Delegates to [`celnet_refdata::swap_instrument_id`], which is the single definition
/// of the identity an aggregated book scopes on, an LP streams under, and the venue
/// resolves a composite by. The server derives the id from an inbound request's
/// `Symbol(55)` + tenor through that same function, so the two sides provably join.
#[must_use]
pub fn ois_instrument_id(curve_symbol: &str, tenor_years: u32) -> String {
    celnet_refdata::swap_instrument_id(curve_symbol, tenor_years)
}

impl OisCurvePoint {
    /// The canonical server `instrument_id` of this point.
    #[must_use]
    pub fn instrument_id(&self) -> String {
        ois_instrument_id(&self.curve_symbol, u32::from(self.tenor_years))
    }

    /// A short human-friendly blotter/GUI name, e.g. `USD OIS 5Y`.
    #[must_use]
    pub fn display_name(&self) -> String {
        format!(
            "{} {}Y",
            self.curve_symbol.replace('-', " "),
            self.tenor_years
        )
    }

    /// The aggregation engine's asset-agnostic key. Injective over `instrument_id`
    /// (the id is carried in the symbol), so two curve points never consolidate onto
    /// one line.
    #[must_use]
    pub fn engine_instrument(&self) -> Instrument {
        Instrument::new(
            Underlying::Commodity(CommodityRef::new(
                Symbol::new(self.instrument_id(), ""),
                self.currency,
            )),
            Tenor::Years(self.tenor_years),
        )
    }

    /// Build the streamable [`QuotedLine`] for this curve point.
    ///
    /// The three per-member displacements (deliberate lean, fixed private view,
    /// wandering private view) are each budgeted as a fraction of the line's own
    /// half-spread — the same discipline the cash-bond lines use — so the panel's best
    /// bid can never print through its best offer and produce a crossed composite that
    /// the server's RFQ resolver would reject.
    #[must_use]
    pub fn to_line(
        &self,
        base_half_spread: f64,
        base_skew_step: f64,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> QuotedLine {
        // The effective half-spread of this line in its QUOTED units (percent).
        let half_spread = base_half_spread * OIS_SPREAD_SCALE;
        // Budgeted displacements are sized in percent, then converted to the model's
        // own natural units (decimal rate) by /100.
        let to_rate = |fraction: f64| fraction * half_spread / 100.0;
        let lean_scale = if base_skew_step.is_finite() && base_skew_step > 0.0 {
            LEAN_FRACTION * half_spread / PANEL_HALF_WIDTH_STEPS / base_skew_step
        } else {
            1.0
        };
        QuotedLine {
            instrument_id: self.instrument_id(),
            display_name: self.display_name(),
            identity: format!("{} {}Y OIS", self.currency, self.tenor_years),
            instrument: self.engine_instrument(),
            mid: MidSource::MeanRevertingRate(RateModel {
                long_run_rate: self.reference_rate,
                initial_rate: self.reference_rate,
                reversion_per_sec,
                perturbation,
            }),
            spread_scale: OIS_SPREAD_SCALE,
            lean_scale,
            yield_dispersion: Some(to_rate(VIEW_FRACTION)),
            dealer_view: to_rate(WANDER_FRACTION),
            // An OIS is an OTC market quoted off-grid.
            tick: None,
        }
    }
}

/// The venue's quotable USD OIS curve: the on-the-run whole-year points, seeded with a
/// realistically-shaped (dipped then rising) reference curve.
///
/// The tenor set matches the venue's default auto-quote tenors, so every request the
/// venue is willing to auto-quote has a book line behind it; a tenor outside this set
/// has no composite and is declined rather than quoted off a fabricated level.
#[must_use]
pub fn load_ois_universe() -> Vec<OisCurvePoint> {
    // The reference level of each on-the-run point, as a decimal par rate. Shaped like
    // a real front-dipped USD OIS curve (inverted through the belly, rising into the
    // long end) rather than a flat line, so a tiering/skew configuration has a genuine
    // term structure to act on.
    fn reference_rate_for(tenor_years: u32) -> f64 {
        match tenor_years {
            2 => 0.0405,
            3 => 0.0395,
            5 => 0.0392,
            7 => 0.0398,
            _ => 0.0408,
        }
    }
    // Driven off the SHARED on-the-run tenor set, so the panel quotes exactly the
    // points the venue is willing to auto-quote — no tenor the venue admits is left
    // without a book line, and no line is streamed for a tenor it would decline.
    celnet_refdata::ON_THE_RUN_SWAP_TENORS
        .iter()
        .map(|&tenor_years| OisCurvePoint {
            curve_symbol: USD_OIS_CURVE.to_string(),
            currency: Ccy::USD,
            tenor_years: u16::try_from(tenor_years).unwrap_or(u16::MAX),
            reference_rate: reference_rate_for(tenor_years),
        })
        .collect()
}

/// Build the [`QuotedLine`]s for a loaded OIS universe — the swap arm of
/// [`crate::quotable_lines`].
#[must_use]
pub fn ois_lines(
    points: &[OisCurvePoint],
    base_half_spread: f64,
    base_skew_step: f64,
    reversion_per_sec: f64,
    perturbation: f64,
) -> Vec<QuotedLine> {
    points
        .iter()
        .map(|p| {
            p.to_line(
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
    use crate::fleet::into_feeds;
    use crate::{LpSimConfig, build_fleet};

    fn cfg() -> LpSimConfig {
        LpSimConfig {
            members: 5,
            ..LpSimConfig::default()
        }
    }

    #[test]
    fn instrument_id_carries_the_tenor_so_curve_points_stay_distinct() {
        assert_eq!(ois_instrument_id("USD-OIS", 5), "USD-OIS-5Y");
        assert_eq!(ois_instrument_id("  USD-OIS  ", 10), "USD-OIS-10Y");
        let ids: Vec<String> = load_ois_universe()
            .iter()
            .map(OisCurvePoint::instrument_id)
            .collect();
        let distinct: std::collections::BTreeSet<&String> = ids.iter().collect();
        assert_eq!(
            distinct.len(),
            ids.len(),
            "curve-point ids must be distinct"
        );
    }

    #[test]
    fn engine_keys_are_injective_over_the_curve() {
        let points = load_ois_universe();
        let keys: std::collections::BTreeSet<String> = points
            .iter()
            .map(|p| p.engine_instrument().to_string())
            .collect();
        assert_eq!(
            keys.len(),
            points.len(),
            "two curve points must never consolidate onto one engine key"
        );
    }

    /// The whole point of the module: a multi-dealer panel quoting the OIS curve
    /// consolidates to a well-formed, UNCROSSED composite for every point — which is
    /// what the venue then prices an RFS off.
    #[test]
    fn the_panel_consolidates_every_curve_point_to_an_uncrossed_composite() {
        let cfg = cfg();
        let lines = ois_lines(
            &load_ois_universe(),
            cfg.half_spread,
            cfg.skew_step,
            cfg.reversion_per_sec,
            cfg.perturbation,
        );
        let feeds = into_feeds(build_fleet(&cfg, &lines));
        let consolidation = cfg.consolidation();

        for now in [0_i64, 7_000_000_000, 613_000_000_000] {
            for line in &lines {
                let composite = crate::composite_for(&feeds, line, now, &consolidation)
                    .unwrap_or_else(|e| panic!("no composite for {}: {e:?}", line.instrument_id));
                let book = &composite.book;
                assert!(
                    book.best_bid.is_finite() && book.best_offer.is_finite(),
                    "{} composite must be finite",
                    line.instrument_id
                );
                assert!(
                    book.best_bid <= book.best_offer,
                    "{} composite crossed at t={now}: {} > {}",
                    line.instrument_id,
                    book.best_bid,
                    book.best_offer
                );
                // Quoted in PERCENT — a plausible OIS level, never a price handle.
                assert!(
                    (1.0..10.0).contains(&book.best_bid),
                    "{} bid {} is not a percent-quoted OIS rate",
                    line.instrument_id,
                    book.best_bid
                );
            }
        }
    }

    /// The par rate is quoted DIRECTLY (percent), not mapped through a bond price —
    /// so the composite mid sits on the seeded reference curve.
    #[test]
    fn the_quoted_mid_tracks_the_seeded_reference_rate() {
        let cfg = cfg();
        for p in load_ois_universe() {
            let line = p.to_line(
                cfg.half_spread,
                cfg.skew_step,
                cfg.reversion_per_sec,
                cfg.perturbation,
            );
            let mid = line.mid.mid_at(0, 0.0);
            assert!(
                (mid - p.reference_rate * 100.0).abs() < 1e-9,
                "{} mid {mid} must be the reference rate in percent",
                line.instrument_id
            );
        }
    }
}
